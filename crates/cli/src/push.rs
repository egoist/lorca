//! Pushes to the identity's phones when a bot has replied. The Runner seals a small notice
//! (who, where, the first words) and asks the relay to push it; the relay hands APNs and FCM
//! ciphertext under fixed words, and the phone opens it before the alert shows.
//!
//! The envelope is `nonce(12) || ciphertext` of ChaCha20-Poly1305 (the IETF one, which iOS's
//! CryptoKit opens in the notification service extension without the core) under a key
//! derived from the account DEK, with `push` as associated data.

use std::sync::Arc;

use chacha20poly1305::aead::{Aead, KeyInit, Payload};
use chacha20poly1305::{ChaCha20Poly1305, Nonce};
use rand::RngCore;
use serde::{Deserialize, Serialize};

use crate::app::App;
use crate::model::{Bot, Chat};

const NONCE_LEN: usize = 12;
const AAD: &[u8] = b"push";
/// The relay takes 2560 bytes of ciphertext, so the whole push fits APNs' 4 KB.
const MAX_SEALED_BYTES: usize = 2400;
const BODY_CHARS: usize = 280;

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
/// phones unless the user is looking at that chat in the app on this Runner. Best effort: a
/// relay without push keys answers that it queued nothing.
pub fn reply(app: &Arc<App>, chat: &Chat, bot: &Bot, text: &str) {
    if app.is_watching(&chat.meta.id) {
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
        let result = match crate::sync::token_or_register(&app, &url, &machine).await {
            Ok(token) => app.relay.push(&url, &token, &crate::keys::b64(&sealed)).await,
            Err(error) => Err(error),
        };
        match result {
            Ok(queued) => tracing::debug!(queued, "pushed a reply"),
            Err(error) => tracing::debug!(%error, "pushing a reply"),
        }
    });
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
}
