//! Plaintext domain model. Lives on Devices; crosses the network only inside encrypted blobs.
//! Field names match what the macOS app decodes.

use serde::{Deserialize, Serialize};

pub const MAX_GROUP_BOTS: usize = 6;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ProviderStatus {
    pub kind: String,
    pub is_connected: bool,
    pub detail: String,
    /// A custom API base URL, when the credential has one.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub base_url: Option<String>,
}

/// A paired machine or phone. `os` decides whether it is a Runner.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Device {
    /// The machine signing public key, base64url.
    pub id: String,
    pub name: String,
    pub model: String,
    pub os: String,
    pub os_version: String,
    pub box_pubkey: String,
    /// Providers connected on that Runner, by kind. Details stay on the Runner.
    #[serde(default)]
    pub providers_connected: Vec<String>,
    #[serde(default)]
    pub updated_at: i64,
}

impl Device {
    pub fn is_runner(&self) -> bool {
        matches!(self.os.as_str(), "macos" | "linux" | "windows")
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Bot {
    pub id: String,
    pub name: String,
    /// One short line under the name: what the bot is for.
    pub label: String,
    /// A sentence or two about the bot, shown in its profile and its prompt.
    pub description: String,
    pub symbol_name: String,
    pub accent: String,
    pub runner_id: String,
    pub provider: String,
    /// Model id for the provider. `None` means the provider's default.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub model: Option<String>,
    /// How much the model thinks: `off`, `minimal`, `low`, `medium`, `high`, `xhigh`, `max`.
    /// `None` means the provider's default.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub thinking: Option<String>,
    pub instructions: String,
    /// Working directory for the coding tools on the Runner. Defaults to
    /// `<TINYBOT_HOME>/workspaces/<bot id>`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub workdir: Option<String>,
    pub created_at: f64,
}

impl Bot {
    /// Where this bot's tools run, keyed by id so renames never move files. Created on first use.
    pub fn working_directory(&self, tinybot_home: &std::path::Path) -> std::path::PathBuf {
        let home = dirs::home_dir().unwrap_or_else(|| std::path::PathBuf::from("."));
        match self.workdir.as_deref().map(str::trim).filter(|w| !w.is_empty()) {
            Some(dir) if dir.starts_with('~') => home.join(dir.trim_start_matches('~').trim_start_matches('/')),
            Some(dir) => std::path::PathBuf::from(dir),
            None => tinybot_home.join("workspaces").join(&self.id),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Author {
    You,
    Bot { bot_id: String },
    System,
}

/// A file sent with a message. Its bytes travel as a `file` blob whose id is this id,
/// encrypted with the account key; Devices keep a copy under `~/.tinybot/files/<id>`.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Attachment {
    pub id: String,
    pub name: String,
    pub mime: String,
    pub size: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub width: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub height: Option<u32>,
}

impl Attachment {
    pub fn is_image(&self) -> bool {
        self.mime.starts_with("image/")
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Body {
    Text {
        text: String,
        #[serde(default, skip_serializing_if = "Vec::is_empty")]
        attachments: Vec<Attachment>,
    },
    Tool {
        name: String,
        summary: String,
        detail: String,
        is_running: bool,
        #[serde(default)]
        call_id: String,
        #[serde(default)]
        arguments: serde_json::Value,
        #[serde(default)]
        result: Option<String>,
        #[serde(default)]
        is_error: bool,
    },
    Handoff {
        from: String,
        to: String,
        reason: String,
    },
    Notice {
        text: String,
    },
}

impl Body {
    pub fn text(text: impl Into<String>) -> Self {
        Body::Text { text: text.into(), attachments: Vec::new() }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum MessageState {
    Thinking,
    Streaming,
    Complete,
    Failed { error: String },
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Message {
    pub id: String,
    pub chat_id: String,
    pub author: Author,
    pub body: Body,
    pub state: MessageState,
    /// Unix seconds.
    pub created_at: f64,
}

impl Message {
    pub fn new(chat_id: &str, author: Author, body: Body) -> Self {
        Message {
            id: format!("msg-{}", uuid::Uuid::new_v4()),
            chat_id: chat_id.to_string(),
            author,
            body,
            state: MessageState::Complete,
            created_at: crate::config::now_secs(),
        }
    }

    pub fn is_complete(&self) -> bool {
        matches!(self.state, MessageState::Complete)
    }
}

/// Roster-level chat description. Messages live beside it in the store.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ChatMeta {
    pub id: String,
    /// `dm` or `group`.
    pub kind: String,
    #[serde(default)]
    pub title: Option<String>,
    pub bot_ids: Vec<String>,
    /// The bot that owns the work in this chat right now. Unaddressed messages go to it; a
    /// handoff can pass it on. Empty means the members decide.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub owner_bot_id: Option<String>,
    #[serde(default)]
    pub is_pinned: bool,
    pub created_at: f64,
}

impl ChatMeta {
    pub fn is_group(&self) -> bool {
        self.kind == "group"
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Chat {
    #[serde(flatten)]
    pub meta: ChatMeta,
    #[serde(default)]
    pub messages: Vec<Message>,
    #[serde(default)]
    pub unread_count: u32,
    /// What the turns run here used. Kept on this Runner, never synced.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub usage: Option<ChatUsage>,
    /// Summaries standing in for the older part of the chat, one per bot. Kept on this Runner.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub compactions: Vec<Compaction>,
}

/// The tokens and money the turns in a chat used. `context_tokens` and `context_window` are
/// the last turn's; the rest accumulate.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
pub struct ChatUsage {
    pub context_tokens: u64,
    pub context_window: u64,
    pub input_tokens: u64,
    pub output_tokens: u64,
    pub cache_read_tokens: u64,
    pub cost_usd: f64,
    pub turns: u64,
    /// The model of the last turn.
    pub model: String,
    pub updated_at: f64,
}

/// A summary that stands in for everything a bot saw in the chat up to and including
/// `after_message_id`; the bot's context starts with it, then the messages after that one.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Compaction {
    pub bot_id: String,
    pub summary: String,
    pub after_message_id: String,
    pub tokens_before: u64,
    pub created_at: f64,
}

// MARK: - Blob payloads

/// `kind = roster`: bots and chat metadata. Latest wins.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Default)]
pub struct RosterBlob {
    pub bots: Vec<Bot>,
    pub chats: Vec<ChatMeta>,
    pub updated_at: f64,
}

/// `kind = chat`: one transcript operation.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(tag = "op", rename_all = "snake_case")]
pub enum ChatBlob {
    /// Insert or replace by message id.
    Upsert { message: Message },
    Remove { chat_id: String, message_id: String },
    ClearUnread { chat_id: String },
}

/// `kind = machine`: a Device's own metadata and presence claims.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct MachineBlob {
    pub device: Device,
}

/// `kind = job`, sealed to the Runner's box key: run one bot turn in one chat.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Job {
    pub id: String,
    pub chat_id: String,
    pub bot_id: String,
    /// `turn` for a user message in a DM, `room_turn` for one member's turn in a group,
    /// `message` for a teammate's message_bot.
    pub kind: String,
    pub trigger_message_id: String,
    /// The Device that created the job; a `room_turn` result goes back to it.
    pub requested_by: String,
    /// The bot that sent a `message` job, so the recipient knows who to answer.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub from_bot_id: Option<String>,
    /// Bot-to-bot hops since the last user message. Bounded so two bots cannot loop.
    #[serde(default)]
    pub hops: u32,
    /// `room_turn`: which round of the group exchange this is, from 1.
    #[serde(default)]
    pub round: u32,
    /// `room_turn`: the last round; the member should only add what is essential.
    #[serde(default)]
    pub is_winding_down: bool,
    pub created_at: f64,
}

pub const MAX_BOT_HOPS: u32 = 8;

/// `kind = job_result`, sealed to the requesting Device's box key: how a `room_turn` ended, so
/// the Device running the group exchange can offer the next turn.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct JobResult {
    pub job_id: String,
    pub chat_id: String,
    pub bot_id: String,
    /// `sent`, `pass`, or `error`.
    pub outcome: String,
}

/// Pairing handshake payloads (sealed boxes).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PairRequest {
    pub machine_pubkey: String,
    pub box_pubkey: String,
    pub device: Device,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PairReply {
    pub identity_pubkey: String,
    pub content_pubkey: String,
    pub account_dek: String,
    pub relay_url: String,
}

/// Machine facts for this host.
pub fn host_facts() -> (String, String, String, String) {
    let name = std::process::Command::new("scutil")
        .args(["--get", "ComputerName"])
        .output()
        .ok()
        .and_then(|o| String::from_utf8(o.stdout).ok())
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
        .or_else(|| std::env::var("HOSTNAME").ok())
        .unwrap_or_else(|| "This Mac".to_string());
    let os = if cfg!(target_os = "macos") {
        "macos"
    } else if cfg!(target_os = "linux") {
        "linux"
    } else if cfg!(target_os = "windows") {
        "windows"
    } else {
        "linux"
    };
    let os_version = std::process::Command::new("sw_vers")
        .args(["-productVersion"])
        .output()
        .ok()
        .and_then(|o| String::from_utf8(o.stdout).ok())
        .map(|s| format!("macOS {}", s.trim()))
        .unwrap_or_else(|| os.to_string());
    let model = std::process::Command::new("sysctl")
        .args(["-n", "hw.model"])
        .output()
        .ok()
        .and_then(|o| String::from_utf8(o.stdout).ok())
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| std::env::consts::ARCH.to_string());
    (name, os.to_string(), os_version, model)
}
