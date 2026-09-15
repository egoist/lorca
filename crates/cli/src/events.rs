//! Events the CLI pushes to the app over the local websocket.

use serde::Serialize;
use serde_json::Value;

use crate::model::{Bot, ChatMeta, Message};

#[derive(Debug, Clone, Serialize)]
#[serde(tag = "event", content = "data")]
pub enum Event {
    #[serde(rename = "snapshot")]
    Snapshot(Value),
    #[serde(rename = "roster.changed")]
    RosterChanged { devices: Vec<Value>, bots: Vec<Bot>, chats: Vec<ChatSummary> },
    #[serde(rename = "message.added")]
    MessageAdded { chat_id: String, message: Message },
    #[serde(rename = "message.updated")]
    MessageUpdated { chat_id: String, message: Message },
    #[serde(rename = "message.removed")]
    MessageRemoved { chat_id: String, message_id: String },
    #[serde(rename = "chat.removed")]
    ChatRemoved { chat_id: String },
    #[serde(rename = "job.started")]
    JobStarted { chat_id: String, bot_id: String, job_id: String },
    #[serde(rename = "job.finished")]
    JobFinished { chat_id: String, bot_id: String, job_id: String },
    #[serde(rename = "relay.status")]
    RelayStatus { connected: bool, url: Option<String> },
    #[serde(rename = "pair.completed")]
    PairCompleted { nonce: String, device: Value },
    #[serde(rename = "identity.changed")]
    IdentityChanged { has_identity: bool },
}

/// Chat metadata plus per-device state, without messages.
#[derive(Debug, Clone, Serialize)]
pub struct ChatSummary {
    #[serde(flatten)]
    pub meta: ChatMeta,
    pub unread_count: u32,
}
