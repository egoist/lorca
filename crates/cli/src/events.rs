//! Events the CLI pushes to the app over the local websocket.

use serde::Serialize;
use serde_json::Value;

use crate::model::{AutoReview, Bot, ChatMeta, ChatUsage, Message, ProviderStatus};

#[derive(Debug, Clone, Serialize)]
#[serde(tag = "event", content = "data")]
pub enum Event {
    #[serde(rename = "snapshot")]
    Snapshot(Value),
    #[serde(rename = "roster.changed")]
    RosterChanged { devices: Vec<Value>, bots: Vec<Bot>, chats: Vec<ChatSummary>, routines: Vec<Value>, auto_review: AutoReview, providers: Vec<ProviderStatus> },
    #[serde(rename = "message.added")]
    MessageAdded { chat_id: String, message: Message },
    #[serde(rename = "message.updated")]
    MessageUpdated { chat_id: String, message: Message },
    #[serde(rename = "message.removed")]
    MessageRemoved { chat_id: String, message_id: String },
    #[serde(rename = "chat.removed")]
    ChatRemoved { chat_id: String },
    /// `routine_id` is set when the turn is a run of a routine.
    #[serde(rename = "job.started")]
    JobStarted { chat_id: String, bot_id: String, job_id: String, #[serde(skip_serializing_if = "Option::is_none")] routine_id: Option<String> },
    #[serde(rename = "job.finished")]
    JobFinished { chat_id: String, bot_id: String, job_id: String, #[serde(skip_serializing_if = "Option::is_none")] routine_id: Option<String> },
    /// A model call failed in a way worth another try; the turn waits `delay_ms` and asks again.
    #[serde(rename = "job.retry")]
    JobRetry { chat_id: String, bot_id: String, attempt: u32, max_attempts: u32, delay_ms: u64, error: String },
    /// A turn finished and the chat's usage moved.
    #[serde(rename = "chat.usage")]
    ChatUsageChanged { chat_id: String, usage: ChatUsage },
    #[serde(rename = "relay.status")]
    /// `update_required`: the relay refused this build's protocol, and only a newer Lorca
    /// connects again.
    RelayStatus { connected: bool, url: Option<String>, update_required: bool },
    /// A Device opens this provider authorization URL while the core waits on its loopback
    /// callback. Runners open the same URL themselves.
    #[serde(rename = "provider.auth")]
    ProviderAuth { kind: String, url: String },
    /// This Device's pairing request reached the relay; the other Device has yet to accept.
    #[serde(rename = "pair.posted")]
    PairPosted { nonce: String },
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
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub usage: Option<ChatUsage>,
}
