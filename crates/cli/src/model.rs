//! Plaintext domain model. Lives on Devices; crosses the network only inside encrypted blobs.
//! Field names match what the macOS app decodes.

use serde::{Deserialize, Serialize};

pub const MAX_GROUP_BOTS: usize = 6;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ProviderStatus {
    pub kind: String,
    pub is_connected: bool,
    pub detail: String,
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
    pub tagline: String,
    pub symbol_name: String,
    pub accent: String,
    pub runner_id: String,
    pub provider: String,
    /// Model id for the provider. `None` means the provider's default.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub model: Option<String>,
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

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Body {
    Text {
        text: String,
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
    /// `turn` for a user message, `handoff` for a teammate's message_bot.
    pub kind: String,
    pub trigger_message_id: String,
    pub requested_by: String,
    pub created_at: f64,
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
