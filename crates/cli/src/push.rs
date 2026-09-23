//! Pushes to the identity's phones for replies, failures, and pending confirmations. The Runner seals a small notice
//! (who, where, the first words) and asks the relay to push it; the relay hands APNs and FCM
//! ciphertext under fixed words, and the phone opens it before the alert shows.
//!
//! The envelope is `nonce(12) || ciphertext` of ChaCha20-Poly1305 (the IETF one, which iOS's
//! CryptoKit opens in the notification service extension without the core) under a key
//! derived from the account DEK, with `push` as associated data.

use std::{sync::Arc, time::Duration};

use chacha20poly1305::aead::{Aead, KeyInit, Payload};
use chacha20poly1305::{ChaCha20Poly1305, Nonce};
use rand::RngCore;
use serde::{Deserialize, Serialize};

use crate::app::App;
use crate::model::{Author, Body, Bot, Chat, Message};

const NONCE_LEN: usize = 12;
const AAD: &[u8] = b"push";
/// The relay takes 2560 bytes of ciphertext, so the whole push fits APNs' 4 KB.
const MAX_SEALED_BYTES: usize = 2400;
const BODY_CHARS: usize = 280;
/// Time for a paired Device displaying the reply to send its encrypted read mark back.
const READ_GRACE: Duration = Duration::from_secs(3);

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Notice {
    /// The bot's name.
    pub title: String,
    /// The group's title; absent in a direct chat.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub subtitle: Option<String>,
    pub body: String,
    pub chat_id: String,
}

pub fn seal(dek: &[u8; 32], notice: &Notice) -> anyhow::Result<Vec<u8>> {
    let cipher = ChaCha20Poly1305::new((&crate::keys::push_key(dek)).into());
    let mut nonce = [0u8; NONCE_LEN];
    rand::rngs::OsRng.fill_bytes(&mut nonce);
    let ciphertext = cipher
        .encrypt(Nonce::from_slice(&nonce), Payload { msg: &serde_json::to_vec(notice)?, aad: AAD })
        .map_err(|_| anyhow::anyhow!("encryption failed"))?;
    Ok([nonce.as_slice(), &ciphertext].concat())
}

pub fn open(dek: &[u8; 32], envelope: &[u8]) -> anyhow::Result<Notice> {
    if envelope.len() <= NONCE_LEN {
        anyhow::bail!("envelope too short");
    }
    let cipher = ChaCha20Poly1305::new((&crate::keys::push_key(dek)).into());
    let (nonce, ciphertext) = envelope.split_at(NONCE_LEN);
    let plaintext = cipher.decrypt(Nonce::from_slice(nonce), Payload { msg: ciphertext, aad: AAD }).map_err(|_| anyhow::anyhow!("decryption failed"))?;
    Ok(serde_json::from_slice(&plaintext)?)
}

fn excerpt(text: &str, max: usize) -> String {
    let one_line: String = text.split_whitespace().collect::<Vec<_>>().join(" ");
    if one_line.chars().count() > max {
        format!("{}…", one_line.chars().take(max).collect::<String>().trim_end())
    } else {
        one_line
    }
}

/// A bot finished a turn with `text` as the last thing it said. Pushes to the identity's
/// phones after a grace period for read marks from any Device. Best effort: a relay without
/// push keys answers that it queued nothing.
pub fn reply(app: &Arc<App>, chat: &Chat, bot: &Bot, text: &str) {
    send(app, chat, bot, text, None);
}

/// A terminal response error, after the turn has exhausted its retries and recovery.
pub fn failed(app: &Arc<App>, chat: &Chat, bot: &Bot, error: &str) {
    send(app, chat, bot, &format!("Reply failed: {error}"), None);
}

/// A new confirmation card must alert while the turn is still waiting for its answer.
pub fn permission(app: &Arc<App>, message: &Message) {
    let (Author::Bot { bot_id }, Body::Permission { summary, decision, .. }) = (&message.author, &message.body) else { return };
    if decision != "pending" {
        return;
    }
    let (Some(chat), Some(bot)) = (app.chat(&message.chat_id), app.bot(bot_id)) else { return };
    send(app, &chat, &bot, &format!("Confirmation needed: {summary}"), Some(message.id.clone()));
}

fn send(app: &Arc<App>, chat: &Chat, bot: &Bot, text: &str, permission_id: Option<String>) {
    if !should_notify(app, &chat.meta.id, permission_id.as_deref()) {
        return;
    }
    let (Some(dek), Some(url), Some(machine)) = (app.dek(), app.relay_url(), app.machine_file().and_then(|m| m.machine().ok())) else { return };
    let subtitle = chat.meta.is_group().then(|| {
        chat.meta.title.clone().filter(|t| !t.trim().is_empty()).unwrap_or_else(|| chat.meta.bot_ids.iter().map(|id| crate::runtime::name_of(chat, id)).collect::<Vec<_>>().join(", "))
    });
    let mut notice = Notice { title: bot.name.clone(), subtitle, body: excerpt(text, BODY_CHARS), chat_id: chat.meta.id.clone() };
    let mut sealed = match seal(&dek, &notice) {
        Ok(sealed) => sealed,
        Err(error) => return tracing::warn!(%error, "sealing a push"),
    };
    // Wide characters can outgrow the budget; halve the words until the envelope fits.
    while sealed.len() > MAX_SEALED_BYTES && notice.body.chars().count() > 20 {
        notice.body = excerpt(&notice.body, notice.body.chars().count() / 2);
        match seal(&dek, &notice) {
            Ok(shorter) => sealed = shorter,
            Err(_) => return,
        }
    }
    let app = app.clone();
    tokio::spawn(async move {
        tokio::time::sleep(READ_GRACE).await;
        // A read mark is about a reply that arrived, so a phone left open or disconnected
        // before the reply finished cannot suppress future notifications.
        if app.dek() != Some(dek) || !should_notify(&app, &notice.chat_id, permission_id.as_deref()) {
            return;
        }
        let result = match crate::sync::token_or_register(&app, &url, &machine).await {
            Ok(token) => {
                // Authentication may have taken longer than the read mark.
                if app.dek() != Some(dek) || !should_notify(&app, &notice.chat_id, permission_id.as_deref()) {
                    return;
                }
                app.relay.push(&url, &token, &crate::keys::b64(&sealed)).await
            }
            Err(error) => Err(error),
        };
        match result {
            Ok(queued) => tracing::debug!(queued, "pushed a chat notification"),
            Err(error) => tracing::debug!(%error, "pushing a chat notification"),
        }
    });
}

fn should_notify(app: &App, chat_id: &str, permission_id: Option<&str>) -> bool {
    !app.is_watching(chat_id) && app.chat(chat_id).is_some_and(|chat| chat.unread_count > 0)
        && permission_id.is_none_or(|id| {
            app.message(chat_id, id).is_some_and(|message| {
                matches!(message.body, Body::Permission { decision, .. } if decision == "pending")
            })
        })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_notice_round_trips_and_binds_the_key() {
        let dek = crate::keys::random_32();
        let notice = Notice { title: "Chef".into(), subtitle: Some("Standup".into()), body: "Sent the invoices.".into(), chat_id: "chat-1".into() };
        let sealed = seal(&dek, &notice).unwrap();
        assert_eq!(open(&dek, &sealed).unwrap(), notice);
        assert!(open(&crate::keys::random_32(), &sealed).is_err());
    }

    /// The vector the phone's notification service extension opens with CryptoKit
    /// (`mobile/targets/notify/PushEnvelope.swift`): same key derivation, same envelope.
    #[test]
    fn the_extension_vector_opens() {
        let dek = crate::keys::unb64_32("yMfGxcTDwsHAv769vLu6ubi3trW0s7KxsK-urayrqqk").unwrap();
        assert_eq!(crate::keys::b64(&crate::keys::push_key(&dek)), "unjACZuGtiiK-SdODWMW3_WnbcuTvn9uzRsw0n7qHus");
        let sealed = crate::keys::unb64("cPVJx5Kj3f-rxYhZNRuI61AeE75VVo89I2TVTkq372GepHEWAdS9QxGYMPLesVR2AjaBWI7r0-WzY4a4lXRbzHEIG1-8JjyetlxTUxPYBYTTzoOPGTq4Dfu-AIoi1lSRugJWU9eHX2KS_ybQSp8Tu2yiEZQLaMe22zjax9uk").unwrap();
        let notice = open(&dek, &sealed).unwrap();
        assert_eq!((notice.title.as_str(), notice.subtitle.as_deref(), notice.chat_id.as_str()), ("Chef", Some("Standup"), "chat-1"));
        assert_eq!(notice.body, "Sent the three flagged invoices.");
    }

    #[cfg(feature = "server")]
    #[tokio::test]
    async fn notifications_wait_for_reads_and_permission_answers_from_paired_devices() {
        use axum::{routing::post, Json, Router};
        use crate::{config::Config, keys::MachineFile, model::*};

        let (sent, mut pushes) = tokio::sync::mpsc::unbounded_channel();
        let server = Router::new()
            .route("/v1/auth/challenge", post(|| async { Json(serde_json::json!({ "nonce": "test" })) }))
            .route("/v1/auth/verify", post(|| async { Json(serde_json::json!({ "token": "test" })) }))
            .route("/v1/push", post(move |Json(body): Json<serde_json::Value>| {
                let sent = sent.clone();
                async move {
                    sent.send(body).unwrap();
                    Json(serde_json::json!({ "queued": 1 }))
                }
            }));
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let url = format!("http://{}", listener.local_addr().unwrap());
        let server = tokio::spawn(async move { axum::serve(listener, server).await.unwrap() });
        let home = std::env::temp_dir().join(format!("lorca-push-{}", uuid::Uuid::new_v4()));
        let app = App::load(Config { home: home.clone(), port: 0 }).unwrap();
        let dek = crate::keys::random_32();
        *app.machine.lock().unwrap() = Some(MachineFile {
            machine_secret: crate::keys::b64(&crate::keys::random_32()), identity_pubkey: "identity".into(),
            content_pubkey: "content".into(), account_dek: crate::keys::b64(&dek), name: "Runner".into(),
            os: "macos".into(), os_version: String::new(), model: String::new(), registered: true,
            relay_url: Some(url.clone()), created_at: 1,
        });
        app.settings.lock().unwrap().relay_url = Some(url);
        let bot = Bot {
            id: "bot".into(), name: "Chef".into(), description: String::new(), symbol_name: "sparkles".into(),
            accent: "indigo".into(), avatar: None, runner_id: "runner".into(), provider: "deepseek".into(),
            model: None, thinking: None, legacy_instructions: String::new(), workdir: None, created_at: 1.0,
        };
        app.state.lock().unwrap().bots.push(bot.clone());
        for id in ["read-on-phone", "unread", "deleted", "watching"] {
            let chat = Chat {
                meta: ChatMeta { id: id.into(), kind: "dm".into(), title: None, bot_ids: vec![bot.id.clone()],
                    owner_bot_id: None, is_pinned: false, created_at: 1.0 },
                unread_count: 0, usage: None, compactions: vec![],
            };
            app.state.lock().unwrap().chats.push(chat.clone());
            app.upsert_message(Message::new(id, Author::Bot { bot_id: bot.id.clone() }, Body::text("Done")), false);
            reply(&app, &chat, &bot, "Done");
        }
        assert!(tokio::time::timeout(Duration::from_millis(150), pushes.recv()).await.is_err(), "pushes must wait for read sync");
        // This is the same operation sync applies for another Device's ClearUnread blob.
        app.mark_read("read-on-phone", false);
        app.delete_chat("deleted");
        app.set_watched_chat(Some("watching".into()));
        let pushed = tokio::time::timeout(READ_GRACE + Duration::from_secs(2), pushes.recv()).await.unwrap().unwrap();
        let notice = open(&dek, &crate::keys::unb64(pushed["ciphertext"].as_str().unwrap()).unwrap()).unwrap();
        assert_eq!(notice.chat_id, "unread", "reading one chat must not suppress another");
        assert!(tokio::time::timeout(Duration::from_millis(150), pushes.recv()).await.is_err());

        // A prior read and an app that has since left the chat cannot suppress a new reply.
        app.set_watched_chat(None);
        let chat = app.chat("read-on-phone").unwrap();
        app.upsert_message(Message::new(&chat.meta.id, Author::Bot { bot_id: bot.id.clone() }, Body::text("Another reply")), false);
        reply(&app, &chat, &bot, "Another reply");
        let pushed = tokio::time::timeout(READ_GRACE + Duration::from_secs(2), pushes.recv()).await.unwrap().unwrap();
        let notice = open(&dek, &crate::keys::unb64(pushed["ciphertext"].as_str().unwrap()).unwrap()).unwrap();
        assert_eq!(notice.chat_id, "read-on-phone");
        assert_eq!(notice.body, "Another reply");

        // Pending confirmations and failed responses are unread even without a completed
        // text reply. Both travel through the same encrypted push envelope.
        let mut cards = Vec::new();
        for id in ["pending", "answered", "expired", "read-card", "failed", "read-error", "visible"] {
            let mut chat = chat.clone();
            chat.meta.id = id.into();
            chat.unread_count = 0;
            app.state.lock().unwrap().chats.push(chat.clone());
            if id == "visible" { app.set_watched_chat(Some(id.into())); }
            let mut message = Message::new(id, Author::Bot { bot_id: bot.id.clone() }, Body::Permission {
                plugin_id: "computer".into(), plugin_name: "Mac".into(), tool: "bash".into(),
                summary: "Deploy the app".into(), arguments: serde_json::Value::Null,
                decision: "pending".into(), reason: Some("Needs confirmation".into()), rule: None, command: None, link: None, code: None,
            });
            if id == "failed" || id == "read-error" {
                message.body = Body::text("Partial output");
                message.state = MessageState::Failed { error: "Provider connection lost".into() };
            }
            app.upsert_message(message.clone(), false);
            app.upsert_message(message.clone(), false);
            assert_eq!(app.chat(id).unwrap().unread_count, u32::from(id != "visible"), "duplicate versions count once");
            if id == "failed" || id == "read-error" {
                failed(&app, &chat, &bot, "Provider connection lost");
            } else {
                permission(&app, &message);
                cards.push(message);
            }
        }
        app.mark_read("read-card", false);
        app.mark_read("read-error", false);
        for mut message in cards {
            let decision = match message.chat_id.as_str() {
                "answered" => "allowed",
                "expired" => "expired",
                _ => continue,
            };
            if let Body::Permission { decision: current, .. } = &mut message.body { *current = decision.into(); }
            app.upsert_message(message, false);
        }
        let mut notices = Vec::new();
        for _ in 0..2 {
            let pushed = tokio::time::timeout(READ_GRACE + Duration::from_secs(2), pushes.recv()).await.unwrap().unwrap();
            notices.push(open(&dek, &crate::keys::unb64(pushed["ciphertext"].as_str().unwrap()).unwrap()).unwrap());
        }
        notices.sort_by(|a, b| a.chat_id.cmp(&b.chat_id));
        assert_eq!((notices[0].chat_id.as_str(), notices[0].body.as_str()), ("failed", "Reply failed: Provider connection lost"));
        assert_eq!((notices[1].chat_id.as_str(), notices[1].body.as_str()), ("pending", "Confirmation needed: Deploy the app"));
        assert!(tokio::time::timeout(Duration::from_millis(150), pushes.recv()).await.is_err());
        server.abort();
        drop(app);
        let _ = std::fs::remove_dir_all(home);
    }
}
