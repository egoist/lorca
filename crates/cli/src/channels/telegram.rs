//! A Telegram account: a bot made with BotFather, its token in the account's setup. The Runner
//! reads the bot's messages with the Bot API's long poll (`getUpdates`), so no webhook or public
//! URL is involved, and the bot answers with `send_message` from the account's builtin server.
//! `LORCA_TELEGRAM_API_URL` points both at another Bot API server, such as a local stub in tests.

use std::sync::Arc;
use std::time::Duration;

use serde_json::{json, Value};
use tokio_util::sync::CancellationToken;

use super::{clean, ingest, load_account, mentions, pause, save_account, set_identity, set_problem, Identity, Incoming, TELEGRAM};
use crate::app::App;
use crate::plugins::builtin::{structured_result, text_result};

/// The setup variable that holds the bot's token.
pub const TOKEN: &str = "TELEGRAM_BOT_TOKEN";
/// The longest message Telegram sends.
const MAX_TEXT: usize = 4096;
/// How long a long poll waits for a message on Telegram's side.
const POLL_SECS: u64 = 25;

const REFUSED: &str = "Telegram refused the bot’s token. Copy it again from BotFather.";
const CONFLICT: &str = "Something else reads this bot’s messages: another program, or a webhook. Stop it, then Lorca reads them again.";
const UNREACHABLE: &str = "Can’t reach Telegram.";

fn base() -> String {
    std::env::var("LORCA_TELEGRAM_API_URL").ok().filter(|url| !url.trim().is_empty()).unwrap_or_else(|| "https://api.telegram.org".into()).trim_end_matches('/').to_string()
}

#[derive(Debug)]
enum Failure {
    /// The token is wrong or revoked.
    Refused,
    /// A webhook or another reader holds the bot's updates.
    Conflict,
    /// Telegram asked to wait this many seconds.
    Slow(f64),
    Network,
    Other(String),
}

/// One Bot API call. The token is in the URL, so errors never carry the URL.
async fn call(app: &App, token: &str, method: &str, body: Value, timeout: Duration) -> Result<Value, Failure> {
    let url = format!("{}/bot{token}/{method}", base());
    let response = app.http.post(url).json(&body).timeout(timeout).send().await.map_err(|error| {
        tracing::debug!(error = %error.without_url(), method, "calling Telegram");
        Failure::Network
    })?;
    let status = response.status().as_u16();
    let value: Value = response.json().await.map_err(|_| Failure::Network)?;
    if value["ok"] == true {
        return Ok(value["result"].clone());
    }
    match value["error_code"].as_u64().unwrap_or(status as u64) {
        401 | 404 => Err(Failure::Refused),
        409 => Err(Failure::Conflict),
        429 => Err(Failure::Slow(value["parameters"]["retry_after"].as_f64().unwrap_or(1.0))),
        _ => Err(Failure::Other(value["description"].as_str().unwrap_or("Telegram refused the request.").to_string())),
    }
}

async fn identity(app: &App, token: &str) -> Result<Identity, Failure> {
    let me = call(app, token, "getMe", json!({}), Duration::from_secs(15)).await?;
    Ok(Identity {
        user_id: me["id"].as_i64().map(|id| id.to_string()).unwrap_or_default(),
        username: me["username"].as_str().map(|name| format!("@{name}")).unwrap_or_default(),
        reads_all: me["can_read_all_group_messages"].as_bool().unwrap_or(false),
    })
}

/// Checks a token as it is set, so the account's sheet says at once when Telegram refuses it.
pub async fn check(app: &Arc<App>, account_id: &str, token: &str) {
    match identity(app, token).await {
        Ok(found) => {
            set_identity(app, account_id, found);
            set_problem(app, account_id, None);
        }
        Err(Failure::Refused) => set_problem(app, account_id, Some(REFUSED)),
        Err(_) => {}
    }
}

/// Reads the account's messages until stopped: who the bot is first, then one long poll after
/// another. An update is confirmed to Telegram (the next poll's offset) only once its message is
/// in the inbox, so a restart or a storage failure reads it again, and the inbox keeps one copy.
pub async fn read(app: Arc<App>, account_id: String, token: String, cancel: CancellationToken) {
    let mut failures = 0u32;
    let me = loop {
        let found = tokio::select! {
            found = identity(&app, &token) => found,
            _ = cancel.cancelled() => return,
        };
        let wait = match found {
            Ok(found) => {
                set_identity(&app, &account_id, found.clone());
                set_problem(&app, &account_id, None);
                break found;
            }
            Err(failure) => trouble(&app, &account_id, failure, &mut failures),
        };
        if !pause(&cancel, wait).await {
            return;
        }
    };
    loop {
        let offset = load_account(&app, &account_id).offset;
        let body = json!({ "offset": offset, "timeout": POLL_SECS, "allowed_updates": ["message"] });
        let polled = tokio::select! {
            polled = call(&app, &token, "getUpdates", body, Duration::from_secs(POLL_SECS + 15)) => polled,
            _ = cancel.cancelled() => return,
        };
        let updates = match polled {
            Ok(updates) => updates,
            Err(failure) => {
                let wait = trouble(&app, &account_id, failure, &mut failures);
                if !pause(&cancel, wait).await {
                    return;
                }
                continue;
            }
        };
        // A poll that went through clears whatever the last one ran into.
        failures = 0;
        set_problem(&app, &account_id, None);
        let mut stuck = false;
        for update in updates.as_array().into_iter().flatten() {
            let Some(update_id) = update["update_id"].as_i64() else { continue };
            if let Some(message) = incoming(&account_id, &me, update) {
                if let Err(error) = ingest(&app, &message) {
                    tracing::error!(%error, "keeping a Telegram message");
                    stuck = true;
                    break;
                }
            }
            let mut state = load_account(&app, &account_id);
            state.offset = Some(update_id + 1);
            if let Err(error) = save_account(&app, &account_id, &state) {
                tracing::error!(%error, "saving a Telegram account's place");
                stuck = true;
                break;
            }
        }
        // The update stays unconfirmed, and is read again after a pause.
        if stuck && !pause(&cancel, 5.0).await {
            return;
        }
    }
}

/// What a failed call means for the account, and how long to wait before the next.
fn trouble(app: &Arc<App>, account_id: &str, failure: Failure, failures: &mut u32) -> f64 {
    match failure {
        Failure::Refused => {
            set_problem(app, account_id, Some(REFUSED));
            60.0
        }
        Failure::Conflict => {
            set_problem(app, account_id, Some(CONFLICT));
            30.0
        }
        Failure::Slow(seconds) => seconds.max(1.0),
        Failure::Network | Failure::Other(_) => {
            *failures += 1;
            if *failures >= 3 {
                set_problem(app, account_id, Some(UNREACHABLE));
            }
            f64::from(2u32.saturating_pow((*failures).min(6))).min(60.0)
        }
    }
}

fn person(user: &Value) -> String {
    let name = [user["first_name"].as_str(), user["last_name"].as_str()].into_iter().flatten().collect::<Vec<_>>().join(" ");
    if !name.trim().is_empty() {
        return name.trim().to_string();
    }
    user["username"].as_str().map(|name| format!("@{name}")).unwrap_or_else(|| "Someone".into())
}

fn id_text(value: &Value) -> Option<String> {
    value.as_i64().map(|id| id.to_string()).or_else(|| value.as_str().map(str::to_string))
}

/// A Telegram update as the channels take it: a text message (or a caption) from a person.
pub fn incoming(account_id: &str, me: &Identity, update: &Value) -> Option<Incoming> {
    let message = update.get("message")?;
    let from = &message["from"];
    if from["is_bot"] == true {
        return None;
    }
    let text = message["text"].as_str().or_else(|| message["caption"].as_str())?.to_string();
    let chat = &message["chat"];
    let private = chat["type"] == "private";
    let sender = clean(&person(from), 64);
    let reply = &message["reply_to_message"];
    // In a forum, a message that answers nobody points at its topic's opening message.
    let opens_topic = reply.get("forum_topic_created").is_some();
    let thread_id = (message["is_topic_message"] == true).then(|| id_text(&message["message_thread_id"])).flatten();
    let mut chat_title = if private { sender.clone() } else { clean(chat["title"].as_str().unwrap_or("Telegram"), 80) };
    if let (Some(_), Some(topic)) = (&thread_id, reply["forum_topic_created"]["name"].as_str()) {
        chat_title = format!("{chat_title} · {}", clean(topic, 60));
    }
    let mentions_bot = mentions(&text, &me.username)
        || message["entities"].as_array().into_iter().flatten().chain(message["caption_entities"].as_array().into_iter().flatten()).any(|entity| entity["type"] == "text_mention" && id_text(&entity["user"]["id"]).as_deref() == Some(me.user_id.as_str()));
    let replies_to_bot = !opens_topic && !me.user_id.is_empty() && id_text(&reply["from"]["id"]).as_deref() == Some(me.user_id.as_str());
    Some(Incoming {
        service: TELEGRAM,
        account_id: account_id.to_string(),
        chat_id: id_text(&chat["id"])?,
        chat_title,
        private,
        thread_id,
        message_id: id_text(&message["message_id"])?,
        reply_to: (!opens_topic).then(|| id_text(&reply["message_id"])).flatten(),
        sender,
        text,
        date: message["date"].as_i64().unwrap_or(0),
        mentions_bot,
        replies_to_bot,
    })
}

/// The account's builtin server: what the bot sends to Telegram.
pub struct Server;

#[async_trait::async_trait]
impl crate::plugins::builtin::Service for Server {
    fn name(&self) -> &'static str {
        TELEGRAM
    }

    fn instructions(&self) -> Option<String> {
        Some("Sends messages as this account's Telegram bot. In a channel's conversation, answer someone with send_message and the id of their message as reply_to_message_id, so the reply lands in their thread.".into())
    }

    fn tools(&self, _app: &Arc<App>, _plugin_id: &str) -> Vec<Value> {
        let id = json!({ "type": ["string", "integer"] });
        vec![
            json!({
                "name": "send_message",
                "description": "Send a text message as the bot to a Telegram chat it is in, as a reply to a message when reply_to_message_id is given. Plain text, at most 4096 characters.",
                "inputSchema": {
                    "type": "object",
                    "properties": {
                        "chat_id": { "type": ["string", "integer"], "description": "The chat's id, such as -1001234567890" },
                        "text": { "type": "string" },
                        "reply_to_message_id": { "type": ["string", "integer"], "description": "The id of the message this answers" },
                        "message_thread_id": id,
                    },
                    "required": ["chat_id", "text"],
                },
                "annotations": { "readOnlyHint": false, "destructiveHint": false, "openWorldHint": true },
            }),
            json!({
                "name": "get_me",
                "description": "Who the account's bot is: its id and @username, and whether it reads every message in its groups (privacy mode off) or only replies and commands.",
                "inputSchema": { "type": "object", "properties": {} },
                "annotations": { "readOnlyHint": true },
            }),
        ]
    }

    async fn call(&self, app: &Arc<App>, plugin_id: &str, tool: &str, args: Value, context: &Value) -> Value {
        let Some(token) = super::tokens(app, plugin_id, TELEGRAM).and_then(|tokens| tokens.into_iter().next()) else {
            return text_result("Add the bot’s token in this Telegram account’s setup first.", true);
        };
        match tool {
            "get_me" => match identity(app, &token).await {
                Ok(me) => structured_result(json!({ "id": me.user_id, "username": me.username, "reads_all_group_messages": me.reads_all })),
                Err(failure) => failed(failure),
            },
            "send_message" => {
                let Some(chat_id) = id_text(&args["chat_id"]).filter(|id| !id.trim().is_empty()) else {
                    return text_result("send_message needs chat_id.", true);
                };
                let text = args["text"].as_str().unwrap_or_default();
                if text.trim().is_empty() {
                    return text_result("send_message needs text.", true);
                }
                if text.chars().count() > MAX_TEXT {
                    return text_result("Telegram takes at most 4096 characters in a message. Send it in parts.", true);
                }
                if super::holds_secret(app, text) {
                    return text_result(super::SECRET_REFUSED, true);
                }
                let reply_to = id_text(&args["reply_to_message_id"]).filter(|id| !id.is_empty());
                let thread = id_text(&args["message_thread_id"]).filter(|id| !id.is_empty());
                let number = |id: &str| id.parse::<i64>().map(Value::from).unwrap_or_else(|_| json!(id));
                let mut body = json!({ "chat_id": number(&chat_id), "text": text });
                if let Some(reply_to) = &reply_to {
                    body["reply_parameters"] = json!({ "message_id": number(reply_to), "allow_sending_without_reply": true });
                }
                if let Some(thread) = &thread {
                    body["message_thread_id"] = number(thread);
                }
                match call(app, &token, "sendMessage", body, Duration::from_secs(30)).await {
                    Ok(sent) => {
                        let message_id = id_text(&sent["message_id"]).unwrap_or_default();
                        super::sent(app, plugin_id, &chat_id, thread.as_deref(), &message_id, reply_to.as_deref(), text, context);
                        structured_result(json!({ "message_id": message_id, "chat_id": chat_id }))
                    }
                    Err(failure) => failed(failure),
                }
            }
            _ => text_result(format!("Telegram has no tool {tool}."), true),
        }
    }
}

/// A failed call as the bot reads it; a request to slow down carries `retry_after`, which holds
/// the account's later calls ([`crate::connector_limits`]).
fn failed(failure: Failure) -> Value {
    match failure {
        Failure::Slow(seconds) => json!({
            "content": [{ "type": "text", "text": format!("Telegram asked to wait {} seconds before the next message.", seconds.ceil()) }],
            "structuredContent": { "retry_after": seconds },
            "isError": true,
        }),
        Failure::Refused => text_result(REFUSED, true),
        Failure::Conflict => text_result(CONFLICT, true),
        Failure::Network => text_result(UNREACHABLE, true),
        Failure::Other(description) => text_result(format!("Telegram: {description}"), true),
    }
}
