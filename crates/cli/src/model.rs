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
    /// Plugins installed on that Runner, with their setup state. Secrets stay on the Runner.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub plugins: Vec<PluginStatus>,
    #[serde(default)]
    pub updated_at: i64,
}

/// A plugin as its Runner advertises it in the machine blob: what is installed and whether it
/// is ready to use. Variables, tokens, and the package itself never leave the Runner.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct PluginStatus {
    pub id: String,
    pub name: String,
    #[serde(default)]
    pub description: String,
    #[serde(default)]
    pub version: String,
    /// An SF Symbol name for the apps.
    #[serde(default)]
    pub icon: String,
    /// `ready`, `needs_setup` (a required variable is missing), `needs_auth` (sign in on the
    /// Runner), `connecting`, or `error`.
    pub state: String,
    /// What the state means, for a row's subtitle.
    #[serde(default)]
    pub detail: String,
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
    /// SF Symbol drawn on the accent gradient; the look when there is no image.
    pub symbol_name: String,
    pub accent: String,
    /// A custom profile image, a `file` blob like a message attachment, shown in place of the
    /// symbol and accent wherever the bot's avatar appears.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub avatar: Option<Attachment>,
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

/// One Auto-review rule: what a bot wants to do, in the user's words, and whether that runs
/// on its own or asks first. A rule made from a card's Always allow also carries the exact
/// `plugin/tool`, matched without a review.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct AutoReviewRule {
    pub id: String,
    pub text: String,
    /// `allow` (runs automatically) or `ask` (asks first; wins when rules conflict).
    pub behavior: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tool: Option<String>,
}

/// Auto-review, after Grok Bot: with it on, a Runner checks each plugin action that changes
/// something before it runs and asks the user only when needed; off, every such action asks.
/// Shared by every Device through the roster.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct AutoReview {
    pub is_enabled: bool,
    #[serde(default)]
    pub rules: Vec<AutoReviewRule>,
}

impl Default for AutoReview {
    fn default() -> Self {
        AutoReview { is_enabled: true, rules: Vec::new() }
    }
}

impl AutoReview {
    /// The rule made for this exact tool, if any.
    pub fn rule_for(&self, plugin_id: &str, tool: &str) -> Option<&AutoReviewRule> {
        let key = format!("{plugin_id}/{tool}");
        self.rules.iter().find(|r| r.tool.as_deref() == Some(&key))
    }
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
        /// Set on the "Routine · Name" marker that opens a routine's run.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        routine_id: Option<String>,
    },
    /// The bot asks before using a plugin tool that is not read-only (or before installing a
    /// plugin, with `tool` = `install`). The turn waits for `decision`: `pending`, `allowed`
    /// (once), `always`, `denied`, or `expired`.
    Permission {
        plugin_id: String,
        plugin_name: String,
        tool: String,
        /// One line about the call: "create_issue · repo: tinybot, title: …".
        summary: String,
        #[serde(default)]
        arguments: serde_json::Value,
        decision: String,
        /// Why Auto-review paused the action, when it did.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        reason: Option<String>,
        /// A sign-in card mid-flow: where to go and the code to enter there (device flow).
        #[serde(default, skip_serializing_if = "Option::is_none")]
        link: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        code: Option<String>,
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

// MARK: - Routines

/// A recurring task a bot runs on a schedule, in its direct chat: Grok Bot's routine. It lives
/// in the roster, so every Device lists it and can pause it; the bot's Runner runs it.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Routine {
    pub id: String,
    pub bot_id: String,
    pub name: String,
    /// The task, written to the bot, handed to it on every run.
    pub prompt: String,
    /// `every 30m`, `every 2h`, `every 1d`, or five cron fields in the Runner's local time.
    pub schedule: String,
    pub is_enabled: bool,
    /// When the schedule started counting: creation, or the last resume.
    pub enabled_at: f64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_run_at: Option<f64>,
    /// How the last run ended: `sent`, `pass`, or `error`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_outcome: Option<String>,
    /// Why Tinybot paused it, when it did: `away`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub paused_reason: Option<String>,
    pub created_at: f64,
}

impl Routine {
    /// The time the next run counts from: the last run, else when the routine was armed.
    pub fn anchor(&self) -> i64 {
        self.last_run_at.unwrap_or(0.0).max(self.enabled_at) as i64
    }

    /// When the next run is due, or `None` when paused or the schedule is unreadable.
    pub fn next_run_at(&self) -> Option<i64> {
        if !self.is_enabled {
            return None;
        }
        crate::schedule::parse(&self.schedule).ok()?.next_after(self.anchor())
    }
}

// MARK: - Blob payloads

/// `kind = roster`: bots, chat metadata, and routines. Latest wins.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Default)]
pub struct RosterBlob {
    pub bots: Vec<Bot>,
    pub chats: Vec<ChatMeta>,
    #[serde(default)]
    pub routines: Vec<Routine>,
    #[serde(default)]
    pub auto_review: AutoReview,
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
    /// `message` for a teammate's message_bot, `routine` for a run of a routine.
    pub kind: String,
    pub trigger_message_id: String,
    /// `routine`: which routine is running.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub routine_id: Option<String>,
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

/// `kind = request`: a question for one Runner from another Device, sealed to the Runner's box
/// key. The verbs read and write a bot's memory, which lives on its Runner.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Request {
    pub id: String,
    /// `memory.read` or `memory.write`.
    pub verb: String,
    /// The Device asking; the answer is sealed to it.
    pub requested_by: String,
    #[serde(default)]
    pub body: serde_json::Value,
    pub created_at: f64,
}

/// `kind = response`: the Runner's answer to a `Request`, sealed to the Device that asked.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Response {
    pub request_id: String,
    #[serde(default)]
    pub body: serde_json::Value,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
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

/// Facts a host passes in when they cannot be probed: a phone names itself through the app.
static HOST_FACTS: std::sync::OnceLock<(String, String, String, String)> = std::sync::OnceLock::new();

/// Sets the name, os, os version, and model `host_facts` answers with, once per process.
pub fn set_host_facts(name: String, os: String, os_version: String, model: String) {
    let _ = HOST_FACTS.set((name, os, os_version, model));
}

/// Machine facts for this host: what the app set, or probed from the system.
pub fn host_facts() -> (String, String, String, String) {
    if let Some(facts) = HOST_FACTS.get() {
        return facts.clone();
    }
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
