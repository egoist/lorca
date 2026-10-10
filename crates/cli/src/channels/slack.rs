//! A Slack account's bot: the Slack app the account signs in with, given a bot user and Socket
//! Mode. The Runner holds its bot token and app-level token, reads its events over Socket
//! Mode's websocket, which needs no public URL, and the bot answers in the thread with
//! `post_message` from the account's builtin server. `LORCA_SLACK_API_URL` points the Web API
//! elsewhere, such as a local stub in tests.

use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;

use futures::{SinkExt, StreamExt};
use serde_json::{json, Value};
use tokio_tungstenite::tungstenite::Message as Frame;
use tokio_util::sync::CancellationToken;

use super::{clean, ingest, pause, set_identity, set_problem, Identity, Incoming, SLACK};
use crate::app::App;
use crate::plugins::builtin::{structured_result, text_result};

/// The setup variables: the bot user's token (`xoxb-`) and the app-level token with
/// `connections:write` (`xapp-`) that opens Socket Mode.
pub const BOT_TOKEN: &str = "SLACK_BOT_TOKEN";
pub const APP_TOKEN: &str = "SLACK_APP_TOKEN";
/// The longest message the bot sends at once; Slack truncates past 40,000 characters.
const MAX_TEXT: usize = 40_000;

const REFUSED: &str = "Slack refused the bot or app token. Copy them again from the app’s settings.";
const UNREACHABLE: &str = "Can’t reach Slack.";

fn base() -> String {
    std::env::var("LORCA_SLACK_API_URL").ok().filter(|url| !url.trim().is_empty()).unwrap_or_else(|| "https://slack.com/api".into()).trim_end_matches('/').to_string()
}

#[derive(Debug)]
enum Failure {
    Refused,
    Slow(f64),
    Network,
    Other(String),
}

/// One Web API call with a token.
async fn call(app: &App, token: &str, method: &str, body: Value) -> Result<Value, Failure> {
    let response = app
        .http
        .post(format!("{}/{method}", base()))
        .bearer_auth(token)
        .json(&body)
        .timeout(Duration::from_secs(20))
        .send()
        .await
        .map_err(|_| Failure::Network)?;
    let retry_after = response.headers().get("retry-after").and_then(|value| value.to_str().ok()?.parse::<f64>().ok());
    if response.status().as_u16() == 429 {
        return Err(Failure::Slow(retry_after.unwrap_or(1.0)));
    }
    let value: Value = response.json().await.map_err(|_| Failure::Network)?;
    if value["ok"] == true {
        return Ok(value);
    }
    match value["error"].as_str().unwrap_or_default() {
        "invalid_auth" | "not_authed" | "account_inactive" | "token_revoked" | "token_expired" => Err(Failure::Refused),
        "ratelimited" => Err(Failure::Slow(retry_after.unwrap_or(1.0))),
        error => Err(Failure::Other(if error.is_empty() { "Slack refused the request.".into() } else { error.to_string() })),
    }
}

/// Reads the account's events until stopped. Each event is acknowledged once its message is in
/// the inbox, so Slack sends one that failed again, and the inbox keeps one copy.
pub async fn read(app: Arc<App>, account_id: String, bot_token: String, app_token: String, cancel: CancellationToken) {
    let mut failures = 0u32;
    let mut names = Names::default();
    loop {
        let session = tokio::select! {
            session = session(&app, &account_id, &bot_token, &app_token, &mut names, &cancel) => session,
            _ = cancel.cancelled() => return,
        };
        let wait = match session {
            // Slack ends a session now and then and asks for a new one.
            Ok(()) => {
                failures = 0;
                0.5
            }
            Err(Failure::Refused) => {
                set_problem(&app, &account_id, Some(REFUSED));
                60.0
            }
            Err(Failure::Slow(seconds)) => seconds.max(1.0),
            Err(Failure::Network | Failure::Other(_)) => {
                failures += 1;
                if failures >= 3 {
                    set_problem(&app, &account_id, Some(UNREACHABLE));
                }
                f64::from(2u32.saturating_pow(failures.min(6))).min(60.0)
            }
        };
        if !pause(&cancel, wait).await {
            return;
        }
    }
}

async fn session(app: &Arc<App>, account_id: &str, bot_token: &str, app_token: &str, names: &mut Names, cancel: &CancellationToken) -> Result<(), Failure> {
    let me = call(app, bot_token, "auth.test", json!({})).await?;
    let me = Identity { user_id: me["user_id"].as_str().unwrap_or_default().to_string(), username: me["user"].as_str().map(|name| format!("@{name}")).unwrap_or_default(), reads_all: true };
    set_identity(app, account_id, me.clone());
    let opened = call(app, app_token, "apps.connections.open", json!({})).await?;
    let url = opened["url"].as_str().ok_or(Failure::Other("Slack gave no Socket Mode address.".into()))?;
    let connector = tokio_tungstenite::Connector::Rustls(Arc::new(lorca_tls::client_config(&[])));
    let connect = tokio_tungstenite::connect_async_tls_with_config(url, None, false, Some(connector));
    let (mut socket, _) = tokio::time::timeout(Duration::from_secs(20), connect).await.map_err(|_| Failure::Network)?.map_err(|_| Failure::Network)?;
    set_problem(app, account_id, None);
    loop {
        let frame = tokio::select! {
            frame = socket.next() => frame,
            _ = cancel.cancelled() => return Ok(()),
            // Slack pings well within this; silence means the connection is gone.
            _ = tokio::time::sleep(Duration::from_secs(120)) => return Err(Failure::Network),
        };
        let text = match frame {
            Some(Ok(Frame::Text(text))) => text.to_string(),
            Some(Ok(Frame::Ping(data))) => {
                let _ = socket.send(Frame::Pong(data)).await;
                continue;
            }
            Some(Ok(Frame::Close(_))) | None => return Ok(()),
            Some(Ok(_)) => continue,
            Some(Err(_)) => return Err(Failure::Network),
        };
        let Ok(envelope) = serde_json::from_str::<Value>(&text) else { continue };
        match envelope["type"].as_str().unwrap_or_default() {
            "disconnect" => return Ok(()),
            "events_api" => {
                let event = &envelope["payload"]["event"];
                if let Some(mut message) = incoming(account_id, &me, event) {
                    names.fill(app, bot_token, &mut message).await;
                    if let Err(error) = ingest(app, &message) {
                        // Unacknowledged, Slack sends it again.
                        tracing::error!(%error, "keeping a Slack message");
                        continue;
                    }
                }
                if let Some(id) = envelope["envelope_id"].as_str() {
                    let _ = socket.send(Frame::Text(json!({ "envelope_id": id }).to_string().into())).await;
                }
            }
            _ => {
                if let Some(id) = envelope["envelope_id"].as_str() {
                    let _ = socket.send(Frame::Text(json!({ "envelope_id": id }).to_string().into())).await;
                }
            }
        }
    }
}

/// A Slack event as the channels take it: a person's message in a channel, a private channel,
/// or a direct message, or a mention of the app. Edits, joins, and bots' messages are left out.
pub fn incoming(account_id: &str, me: &Identity, event: &Value) -> Option<Incoming> {
    let kind = event["type"].as_str()?;
    if !matches!(kind, "message" | "app_mention") || event.get("subtype").is_some_and(|subtype| subtype != "thread_broadcast") || event.get("bot_id").is_some() {
        return None;
    }
    let user = event["user"].as_str()?;
    if user == me.user_id {
        return None;
    }
    let text = event["text"].as_str()?.to_string();
    let chat_id = event["channel"].as_str()?.to_string();
    let ts = event["ts"].as_str()?.to_string();
    let private = event["channel_type"] == "im";
    let thread_ts = event["thread_ts"].as_str().map(str::to_string);
    let mention = format!("<@{}>", me.user_id);
    Some(Incoming {
        service: SLACK,
        account_id: account_id.to_string(),
        chat_id,
        chat_title: String::new(),
        private,
        // In a channel every exchange is a thread: a new message starts one, which the bot's
        // reply joins. A direct message is one conversation.
        thread_id: if private { None } else { Some(thread_ts.clone().unwrap_or_else(|| ts.clone())) },
        date: ts.split('.').next().and_then(|seconds| seconds.parse().ok()).unwrap_or(0),
        message_id: ts,
        reply_to: None,
        sender: user.to_string(),
        mentions_bot: kind == "app_mention" || (!me.user_id.is_empty() && text.contains(&mention)),
        replies_to_bot: thread_ts.is_some() && event["parent_user_id"].as_str() == Some(me.user_id.as_str()),
        text,
    })
}

/// People's and channels' names, looked up once per reader. A lookup that fails or is slow
/// leaves the id, since the event has to be acknowledged within three seconds.
#[derive(Default)]
struct Names {
    users: HashMap<String, String>,
    channels: HashMap<String, String>,
}

impl Names {
    async fn fill(&mut self, app: &App, token: &str, message: &mut Incoming) {
        if !self.users.contains_key(&message.sender) {
            let found = tokio::time::timeout(Duration::from_millis(1500), call(app, token, "users.info", json!({ "user": message.sender }))).await;
            if let Ok(Ok(found)) = found {
                let profile = &found["user"]["profile"];
                let name = [profile["display_name"].as_str(), profile["real_name"].as_str(), found["user"]["name"].as_str()].into_iter().flatten().find(|name| !name.trim().is_empty());
                if let Some(name) = name {
                    self.users.insert(message.sender.clone(), clean(name, 64));
                }
            }
        }
        if let Some(name) = self.users.get(&message.sender) {
            message.sender = name.clone();
        }
        if message.private {
            message.chat_title = message.sender.clone();
            return;
        }
        if !self.channels.contains_key(&message.chat_id) {
            let found = tokio::time::timeout(Duration::from_millis(1500), call(app, token, "conversations.info", json!({ "channel": message.chat_id }))).await;
            if let Ok(Ok(found)) = found {
                if let Some(name) = found["channel"]["name"].as_str() {
                    self.channels.insert(message.chat_id.clone(), format!("#{}", clean(name, 80)));
                }
            }
        }
        message.chat_title = self.channels.get(&message.chat_id).cloned().unwrap_or_else(|| message.chat_id.clone());
    }
}

/// The account's builtin server for its bot: what the bot posts to Slack. It offers nothing
/// until the account has a bot token, so an account used only through Slack's own server keeps
/// its tools as they are.
pub struct Server;

#[async_trait::async_trait]
impl crate::plugins::builtin::Service for Server {
    fn name(&self) -> &'static str {
        SLACK
    }

    fn instructions(&self) -> Option<String> {
        Some("Posts as this account's Slack app bot. In a channel's conversation, answer in the thread: post_message with the conversation's channel and thread_ts.".into())
    }

    fn tools(&self, app: &Arc<App>, plugin_id: &str) -> Vec<Value> {
        if !app.plugins.lock().unwrap().values(plugin_id).contains_key(BOT_TOKEN) {
            return Vec::new();
        }
        vec![json!({
            "name": "post_message",
            "description": "Post a message as the app's bot to a Slack channel or direct message it is in, in a thread when thread_ts is given. Slack's mrkdwn formatting.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "channel": { "type": "string", "description": "The channel's id, such as C0123456789" },
                    "text": { "type": "string" },
                    "thread_ts": { "type": "string", "description": "The ts of the thread's first message, to reply in that thread" },
                },
                "required": ["channel", "text"],
            },
            "annotations": { "readOnlyHint": false, "destructiveHint": false, "openWorldHint": true },
        })]
    }

    async fn call(&self, app: &Arc<App>, plugin_id: &str, tool: &str, args: Value, context: &Value) -> Value {
        if tool != "post_message" {
            return text_result(format!("Slack's bot has no tool {tool}."), true);
        }
        let Some(token) = app.plugins.lock().unwrap().values(plugin_id).get(BOT_TOKEN).cloned() else {
            return text_result("Add the bot token in this Slack account’s setup first.", true);
        };
        let channel = args["channel"].as_str().unwrap_or_default().trim().to_string();
        let text = args["text"].as_str().unwrap_or_default();
        if channel.is_empty() || text.trim().is_empty() {
            return text_result("post_message needs channel and text.", true);
        }
        if text.chars().count() > MAX_TEXT {
            return text_result("Slack takes at most 40,000 characters in a message. Post it in parts.", true);
        }
        if super::holds_secret(app, text) {
            return text_result(super::SECRET_REFUSED, true);
        }
        let thread_ts = args["thread_ts"].as_str().map(str::trim).filter(|ts| !ts.is_empty()).map(str::to_string);
        let mut body = json!({ "channel": channel, "text": text });
        if let Some(ts) = &thread_ts {
            body["thread_ts"] = json!(ts);
        }
        match call(app, token.trim(), "chat.postMessage", body).await {
            Ok(posted) => {
                let ts = posted["ts"].as_str().unwrap_or_default().to_string();
                super::sent(app, plugin_id, &channel, thread_ts.as_deref(), &ts, None, text, context);
                structured_result(json!({ "ts": ts, "channel": channel }))
            }
            Err(Failure::Slow(seconds)) => json!({
                "content": [{ "type": "text", "text": format!("Slack asked to wait {} seconds before the next message.", seconds.ceil()) }],
                "structuredContent": { "retry_after": seconds },
                "isError": true,
            }),
            Err(Failure::Refused) => text_result(REFUSED, true),
            Err(Failure::Network) => text_result(UNREACHABLE, true),
            Err(Failure::Other(error)) => text_result(format!("Slack: {error}"), true),
        }
    }
}
