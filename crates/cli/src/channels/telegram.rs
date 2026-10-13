//! A Telegram account: a bot made with BotFather, its token in the account's setup. The Runner
//! reads the bot's messages with the Bot API's long poll (`getUpdates`), so no webhook or public
//! URL is involved, and the bot answers with `send_message` from the account's builtin server.
//! `LORCA_TELEGRAM_API_URL` points both at another Bot API server, such as a local stub in tests.

use std::sync::Arc;
use std::time::Duration;

use serde_json::{json, Value};
use tokio_util::sync::CancellationToken;

use super::{ask, clean, ingest, load_account, mentions, pause, set_identity, set_problem, update_account, withdraw, ChatRequest, Identity, Incoming, IncomingFile, TELEGRAM};
use crate::app::App;
use crate::plugins::builtin::{structured_result, text_result};

/// The setup variable that holds the bot's token.
pub const TOKEN: &str = "TELEGRAM_BOT_TOKEN";
/// The longest message Telegram sends.
const MAX_TEXT: usize = 4096;
/// How long a long poll waits for a message on Telegram's side.
const POLL_SECS: u64 = 25;
/// The largest file the Bot API lets a bot download.
const MAX_DOWNLOAD: u64 = 20 * 1024 * 1024;

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
        // `my_chat_member` says when someone adds the bot to a group, which then asks the user.
        let body = json!({ "offset": offset, "timeout": POLL_SECS, "allowed_updates": ["message", "my_chat_member"] });
        let polled = tokio::select! {
            polled = call(&app, &token, "getUpdates", body, Duration::from_secs(POLL_SECS + 15)) => polled,
            _ = cancel.cancelled() => return,
        };
        let mut updates = match polled {
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
        // An album's photos arrive a moment apart: the rest of one the poll ends in is read with it.
        let ends_in_album = updates.as_array().and_then(|updates| updates.last()).is_some_and(|update| update["message"]["media_group_id"].is_string());
        if ends_in_album && pause(&cancel, 1.0).await {
            let again = json!({ "offset": offset, "timeout": 0, "allowed_updates": ["message", "my_chat_member"] });
            if let Ok(more) = call(&app, &token, "getUpdates", again, Duration::from_secs(15)).await {
                updates = more;
            }
        }
        let mut stuck = None;
        let updates = updates.as_array().map(Vec::as_slice).unwrap_or_default();
        for (update_id, message) in gathered(&account_id, &me, updates) {
            if let Some(mut message) = message {
                if let Err(failure) = fetch_images(&app, &token, &mut message).await {
                    stuck = Some(if let Failure::Slow(seconds) = failure { seconds.max(1.0) } else { 5.0 });
                    break;
                }
                if let Err(error) = ingest(&app, &message) {
                    tracing::error!(%error, "keeping a Telegram message");
                    stuck = Some(5.0);
                    break;
                }
            }
            match updates.iter().find(|update| update["update_id"] == update_id).and_then(membership) {
                Some(Membership::Added(request)) => {
                    ask(&app, &account_id, request, None);
                }
                Some(Membership::Left(chat_id)) => withdraw(&app, &account_id, &chat_id),
                None => {}
            }
            if let Err(error) = update_account(&app, &account_id, |state| {
                state.offset = Some(update_id + 1);
                true
            }) {
                tracing::error!(%error, "saving a Telegram account's place");
                stuck = Some(5.0);
                break;
            }
        }
        // The update stays unconfirmed, and is read again after a pause.
        if let Some(wait) = stuck {
            if !pause(&cancel, wait).await {
                return;
            }
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

/// The images a message carries: a photo at its largest size, or an image sent as a file, when
/// the Bot API lets a bot download it.
fn images(message: &Value) -> Vec<IncomingFile> {
    let size = |file: &Value| file["file_size"].as_u64();
    let fits = |file: &Value| size(file).is_none_or(|bytes| bytes <= MAX_DOWNLOAD);
    let mut files = Vec::new();
    if let Some(photo) = message["photo"].as_array().and_then(|sizes| sizes.iter().filter(|file| fits(file)).max_by_key(|file| file["width"].as_u64().unwrap_or(0) * file["height"].as_u64().unwrap_or(0))) {
        files.push(IncomingFile { id: photo["file_id"].as_str().unwrap_or_default().into(), unique_id: photo["file_unique_id"].as_str().unwrap_or_default().into(), name: "photo.jpg".into(), mime: "image/jpeg".into(), size: size(photo) });
    }
    let document = &message["document"];
    if let Some(mime) = document["mime_type"].as_str().filter(|mime| mime.starts_with("image/") && fits(document)) {
        let name = document["file_name"].as_str().map(|name| clean(name, 80)).unwrap_or_else(|| "image".into());
        files.push(IncomingFile { id: document["file_id"].as_str().unwrap_or_default().into(), unique_id: document["file_unique_id"].as_str().unwrap_or_default().into(), name, mime: mime.into(), size: size(document) });
    }
    files.retain(|file| !file.id.is_empty() && !file.unique_id.is_empty());
    files
}

/// A Telegram update as the channels take it: a person's text message, or a photo or image with
/// its caption or without one.
pub fn incoming(account_id: &str, me: &Identity, update: &Value) -> Option<Incoming> {
    let message = update.get("message")?;
    let from = &message["from"];
    if from["is_bot"] == true {
        return None;
    }
    let files = images(message);
    let text = message["text"].as_str().or_else(|| message["caption"].as_str()).unwrap_or_default().to_string();
    if text.trim().is_empty() && files.is_empty() {
        return None;
    }
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
        files,
        attachments: Vec::new(),
        held: false,
    })
}

/// Tells a chat that asks that the bot answers once its owner allows it, as a reply to the
/// message that asked. Best effort: nothing else waits on it.
pub async fn notice(app: &App, token: &str, chat_id: &str, message_id: &str, thread_id: Option<&str>) {
    let number = |id: &str| id.parse::<i64>().map(Value::from).unwrap_or_else(|_| json!(id));
    let mut body = json!({ "chat_id": number(chat_id), "text": super::PENDING,
        "reply_parameters": { "message_id": number(message_id), "allow_sending_without_reply": true } });
    if let Some(thread_id) = thread_id {
        body["message_thread_id"] = number(thread_id);
    }
    if let Err(error) = call(app, token, "sendMessage", body, Duration::from_secs(30)).await {
        tracing::debug!(?error, "telling a Telegram chat it waits");
    }
}

/// How often the bot's typing is sent again: Telegram shows a chat action for about five seconds.
const TYPING_EVERY: Duration = if cfg!(test) { Duration::from_millis(100) } else { Duration::from_secs(4) };

/// Shows the bot typing in a chat (`sendChatAction`) until `stop`. Best effort: a network
/// failure tries again at the next refresh, a request to slow down waits as asked, and a refusal
/// (a wrong token, a chat the bot left) ends it.
pub async fn typing(app: Arc<App>, token: String, chat_id: String, thread_id: Option<String>, stop: CancellationToken) {
    let number = |id: &str| id.parse::<i64>().map(Value::from).unwrap_or_else(|_| json!(id));
    let mut body = json!({ "chat_id": number(&chat_id), "action": "typing" });
    if let Some(thread_id) = &thread_id {
        body["message_thread_id"] = number(thread_id);
    }
    loop {
        let wait = tokio::select! {
            shown = call(&app, &token, "sendChatAction", body.clone(), Duration::from_secs(10)) => match shown {
                Ok(_) | Err(Failure::Network) => TYPING_EVERY,
                Err(Failure::Slow(seconds)) => Duration::from_secs_f64(seconds.clamp(1.0, 3600.0)),
                Err(error) => {
                    tracing::debug!(?error, "showing the bot typing in a Telegram chat");
                    return;
                }
            },
            _ = stop.cancelled() => return,
        };
        tokio::select! {
            _ = tokio::time::sleep(wait) => {}
            _ = stop.cancelled() => return,
        }
    }
}

/// The bot joining or leaving a group, from a `my_chat_member` update.
#[derive(Debug, PartialEq)]
pub enum Membership {
    /// Someone added the bot to a group: the group asks to be allowed.
    Added(ChatRequest),
    /// The bot left the group or was removed: its request goes.
    Left(String),
}

pub fn membership(update: &Value) -> Option<Membership> {
    let change = update.get("my_chat_member")?;
    let chat = &change["chat"];
    // A Telegram channel's posts are not messages a channel reads, and a private chat's own
    // first message asks for it.
    if !matches!(chat["type"].as_str(), Some("group" | "supergroup")) {
        return None;
    }
    let chat_id = id_text(&chat["id"])?;
    let gone = |status: &Value| matches!(status.as_str(), Some("left" | "kicked"));
    let (old, new) = (&change["old_chat_member"], &change["new_chat_member"]);
    if gone(&new["status"]) || (new["status"] == "restricted" && new["is_member"] == false) {
        return Some(Membership::Left(chat_id));
    }
    let was_out = gone(&old["status"]) || (old["status"] == "restricted" && old["is_member"] == false);
    was_out.then(|| {
        Membership::Added(ChatRequest {
            id: chat_id,
            title: clean(chat["title"].as_str().unwrap_or("Telegram"), 80),
            private: false,
            from: clean(&person(&change["from"]), 64),
            at: crate::config::now_unix(),
        })
    })
}

/// The updates of one poll as messages, each with the last update it takes in: the photos of an
/// album, which Telegram sends as one message each, are one message, which their caption
/// describes.
pub(crate) fn gathered(account_id: &str, me: &Identity, updates: &[Value]) -> Vec<(i64, Option<Incoming>)> {
    let mut out: Vec<(i64, Option<Incoming>, Option<String>)> = Vec::new();
    for update in updates {
        let Some(update_id) = update["update_id"].as_i64() else { continue };
        let message = incoming(account_id, me, update);
        let album = update["message"]["media_group_id"].as_str().map(|group| format!("{}:{group}", update["message"]["chat"]["id"]));
        if let (Some(album), Some(next)) = (&album, &message) {
            if let Some((last, Some(first), _)) = out.last_mut().filter(|(_, _, group)| group.as_ref() == Some(album)) {
                *last = update_id;
                if first.text.trim().is_empty() {
                    first.text = next.text.clone();
                }
                first.files.extend(next.files.iter().cloned());
                first.mentions_bot |= next.mentions_bot;
                first.replies_to_bot |= next.replies_to_bot;
                first.reply_to = first.reply_to.take().or_else(|| next.reply_to.clone());
                continue;
            }
        }
        out.push((update_id, message, album));
    }
    out.into_iter().map(|(update_id, message, _)| (update_id, message)).collect()
}

/// Fetches the images of a message a channel takes and keeps them on this Runner, each under an
/// id made from the account and Telegram's id for the file, so a message read again uses the copy
/// already here. An image Telegram will not hand over is left out; a network failure is an error,
/// so the message is read again.
async fn fetch_images(app: &Arc<App>, token: &str, incoming: &mut Incoming) -> Result<(), Failure> {
    if incoming.files.is_empty() || !super::taken(app, incoming) {
        return Ok(());
    }
    for file in std::mem::take(&mut incoming.files) {
        let digest = <sha2::Sha256 as sha2::Digest>::digest(format!("{}\n{}", incoming.account_id, file.unique_id).as_bytes());
        let id = format!("att-tg-{}", digest.iter().take(12).map(|byte| format!("{byte:02x}")).collect::<String>());
        let bytes = match std::fs::read(crate::files::local_path(app, &id)) {
            Ok(bytes) => bytes,
            Err(_) => match download(app, token, &file.id).await {
                Ok(bytes) => bytes,
                Err(Failure::Network) => return Err(Failure::Network),
                Err(Failure::Slow(seconds)) => return Err(Failure::Slow(seconds)),
                Err(failure) => {
                    tracing::warn!(?failure, "fetching a Telegram image");
                    continue;
                }
            },
        };
        match crate::files::keep(app, &id, &file.name, &file.mime, &bytes) {
            Ok(attachment) => incoming.attachments.push(attachment),
            Err(error) => tracing::warn!(%error, "keeping a Telegram image"),
        }
    }
    Ok(())
}

/// A file's bytes: where Telegram keeps it (`getFile`), then the file itself. The token is in both
/// URLs, so errors never carry them.
async fn download(app: &App, token: &str, file_id: &str) -> Result<Vec<u8>, Failure> {
    let found = call(app, token, "getFile", json!({ "file_id": file_id }), Duration::from_secs(30)).await?;
    let path = found["file_path"].as_str().ok_or_else(|| Failure::Other("Telegram did not say where the file is.".into()))?;
    let response = app.http.get(format!("{}/file/bot{token}/{path}", base())).timeout(Duration::from_secs(60)).send().await.map_err(|error| {
        tracing::debug!(error = %error.without_url(), "downloading a Telegram file");
        Failure::Network
    })?;
    if !response.status().is_success() {
        return Err(Failure::Other(format!("Telegram answered {} for the file.", response.status().as_u16())));
    }
    let bytes = response.bytes().await.map_err(|_| Failure::Network)?;
    if bytes.len() as u64 > MAX_DOWNLOAD {
        return Err(Failure::Other("The file is larger than Telegram lets a bot download.".into()));
    }
    Ok(bytes.to_vec())
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
                        super::replied(app, context["lorca/chat_id"].as_str().unwrap_or_default(), &chat_id);
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
