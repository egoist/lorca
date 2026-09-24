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
    /// What the bot does and how it should work, shown in its profile and used in its prompt.
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
    /// Compatibility with profiles written before Description became the single behavioral
    /// field. It remains on the wire for one-version rolling upgrades, but is folded into
    /// `description` as soon as the profile is read.
    #[serde(default, rename = "instructions")]
    pub legacy_instructions: String,
    /// Working directory for the coding tools on the Runner. Defaults to
    /// `<LORCA_HOME>/workspaces/<bot id>`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub workdir: Option<String>,
    pub created_at: f64,
}

/// One Auto-review rule: what a bot wants to do, in the user's words, and whether that runs
/// on its own or asks first. Always allow on a shell command's card adds the rule Auto-review
/// proposed in plain language; on a plugin tool's card it adds a rule for that exact tool,
/// matched without another model review.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct AutoReviewRule {
    pub id: String,
    pub text: String,
    /// `allow` (runs automatically) or `ask` (asks first; wins when rules conflict).
    pub behavior: String,
    /// The exact plugin tool (`github/create_issue`) a card's Always allow saved this rule for.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tool: Option<String>,
}

/// Auto-review, after Grok Bot: with it on, a Runner checks effectful plugin actions and shell
/// commands before they run and asks the user only when needed; off, every such action asks.
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
    /// Folds the old Instructions field into Description once. Existing paired Devices may
    /// still send the old field during a rolling upgrade, so an empty compatibility value is
    /// kept on the wire while current clients expose only Description.
    pub fn normalize_description(&mut self) -> bool {
        let legacy = self.legacy_instructions.trim();
        if legacy.is_empty() {
            return false;
        }
        let description = self.description.trim();
        if description.is_empty() {
            self.description = legacy.to_string();
        } else if description != legacy && !description.ends_with(&format!("\n\n{legacy}")) {
            self.description = format!("{description}\n\n{legacy}");
        } else {
            self.description = description.to_string();
        }
        self.legacy_instructions.clear();
        true
    }

    /// Where this bot's tools run, keyed by id so renames never move files. Created on first use.
    pub fn working_directory(&self, lorca_home: &std::path::Path) -> std::path::PathBuf {
        let home = dirs::home_dir().unwrap_or_else(|| std::path::PathBuf::from("."));
        match self.workdir.as_deref().map(str::trim).filter(|w| !w.is_empty()) {
            Some(dir) if dir.starts_with('~') => home.join(dir.trim_start_matches('~').trim_start_matches('/')),
            Some(dir) => std::path::PathBuf::from(dir),
            None => lorca_home.join("workspaces").join(&self.id),
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
/// encrypted with the account key; Devices keep a copy under `~/.lorca/files/<id>`.
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
        /// The bots the user's `@Name`s address, by id, in the order the user picked them: a
        /// bot reads "@Scout (id bot-1a2b3c4d)", and a group offers them the first turns.
        #[serde(default, skip_serializing_if = "Vec::is_empty")]
        mentions: Vec<String>,
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
        /// What the call does, in the bot's words: a shell command's `description`, which the
        /// status line reads as "Running command: Install dependencies…".
        #[serde(default, skip_serializing_if = "Option::is_none")]
        description: Option<String>,
        /// The bot a message_bot call goes to, by id: the apps read "Messaging Scout…" while it
        /// runs and show the finished row as "Messaged ◉ Scout".
        #[serde(default, skip_serializing_if = "Option::is_none")]
        target_bot_id: Option<String>,
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
    /// The bot asks before a reviewed plugin or shell action (or before installing a
    /// plugin, with `tool` = `install`). The turn waits for `decision`: `pending`, `allowed`
    /// (once), `always`, `denied`, or `expired`.
    Permission {
        plugin_id: String,
        plugin_name: String,
        tool: String,
        /// One line about the call: "create_issue · repo: lorca, title: …".
        summary: String,
        #[serde(default)]
        arguments: serde_json::Value,
        decision: String,
        /// Why Auto-review paused the action, when it did.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        reason: Option<String>,
        /// The plain-language rule Always allow adds, which Auto-review proposed for a shell
        /// command. A shell card without one offers only Allow once and Deny.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        rule: Option<String>,
        /// A shell card's whole command, for the apps, which never get `arguments`: `for_app`
        /// fills it with the first `APP_COMMAND_CHARS` characters.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        command: Option<String>,
        /// A sign-in card mid-flow: where to go and the code to enter there (device flow).
        #[serde(default, skip_serializing_if = "Option::is_none")]
        link: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        code: Option<String>,
    },
}

impl Body {
    pub fn text(text: impl Into<String>) -> Self {
        Body::Text { text: text.into(), attachments: Vec::new(), mentions: Vec::new() }
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
    /// A user message typed during a turn becomes model-visible at the next safe boundary.
    /// The apps keep showing when it was typed; transcript rebuilding uses this later time.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub promoted_at: Option<f64>,
}

/// How much of a tool call's detail the apps get: enough for the "Messaged ◉ X" marker.
const APP_TOOL_DETAIL_CHARS: usize = 400;
/// How much of a shell command a permission card carries to the apps.
const APP_COMMAND_CHARS: usize = 8000;
/// How many of a chat's newest messages a snapshot carries; older ones are asked for by page.
pub const SNAPSHOT_MESSAGES: usize = 60;

impl Message {
    /// The message as the apps get it. A tool row keeps what they show (the name, the summary,
    /// whether it runs) and drops what only a later turn's context needs: the arguments and
    /// the result, which run to hundreds of kilobytes for a file read or a command's output.
    /// Permission cards likewise keep their summary and decision, not their reviewed payload,
    /// except a shell command's text, which the card shows in full on request.
    pub fn for_app(&self) -> Message {
        let mut message = self.clone();
        match &mut message.body {
            Body::Tool { detail, arguments, result, .. } => {
                *arguments = serde_json::Value::Null;
                *result = None;
                if detail.chars().count() > APP_TOOL_DETAIL_CHARS {
                    *detail = detail.chars().take(APP_TOOL_DETAIL_CHARS).collect();
                }
            }
            Body::Permission { plugin_id, arguments, command, .. } => {
                if let Some(text) = arguments.get("command").and_then(serde_json::Value::as_str).filter(|_| plugin_id == "computer") {
                    *command = Some(text.chars().take(APP_COMMAND_CHARS).collect());
                }
                *arguments = serde_json::Value::Null;
            }
            _ => {}
        }
        message
    }

    pub fn new(chat_id: &str, author: Author, body: Body) -> Self {
        Message {
            id: format!("msg-{}", uuid::Uuid::new_v4()),
            chat_id: chat_id.to_string(),
            author,
            body,
            state: MessageState::Complete,
            created_at: crate::config::now_secs(),
            promoted_at: None,
        }
    }

    pub fn is_complete(&self) -> bool {
        matches!(self.state, MessageState::Complete)
    }

    /// A reply, failure, or pending confirmation the user has not seen. Tool activity and
    /// updates to an answered permission card do not add to the count.
    pub fn counts_unread(&self) -> bool {
        if !matches!(self.author, Author::Bot { .. }) {
            return false;
        }
        match &self.body {
            Body::Text { .. } => matches!(self.state, MessageState::Complete | MessageState::Failed { .. }),
            Body::Permission { decision, .. } => self.is_complete() && decision == "pending",
            _ => false,
        }
    }
}

/// Roster-level chat description. Messages live beside it in the store.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ChatMeta {
    pub id: String,
    /// `dm` or `group`.
    pub kind: String,
    /// A group's optional custom title. A direct chat is named after its bot.
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
    /// Why Lorca paused it, when it did: `away`.
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

impl ChatBlob {
    /// Versions of a message share a slot, and so does its removal, which leaves the relay
    /// nothing of the message but the removal. A chat's read marks share another. A tool row
    /// goes up while its call runs, for the status line on other Devices, and the finished row
    /// replaces it outright, so the relay keeps one version of each call.
    pub fn slot(&self) -> crate::app::Slot {
        match self {
            ChatBlob::Upsert { message } if matches!(message.body, Body::Tool { .. }) => crate::app::Slot::latest(relay_name(&message.id)),
            ChatBlob::Upsert { message } => crate::app::Slot::first_and_latest(relay_name(&message.id)),
            ChatBlob::Remove { message_id, .. } => crate::app::Slot::latest(relay_name(message_id)),
            ChatBlob::ClearUnread { chat_id } => crate::app::Slot::latest(relay_name(&format!("read-{chat_id}"))),
        }
    }

    /// The chat, as the relay's group: deleting the chat deletes the group.
    pub fn group(&self) -> String {
        match self {
            ChatBlob::Upsert { message } => relay_name(&message.chat_id),
            ChatBlob::Remove { chat_id, .. } | ChatBlob::ClearUnread { chat_id } => relay_name(chat_id),
        }
    }
}

/// A slot or group name. The relay takes 1–64 characters of `[A-Za-z0-9._-]`; any other name
/// goes up as its hash.
pub fn relay_name(name: &str) -> String {
    let plain = !name.is_empty()
        && name.len() <= 64
        && name != "."
        && name != ".."
        && name.bytes().all(|b| b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_' | b'.'));
    if plain {
        return name.to_string();
    }
    <sha2::Sha256 as sha2::Digest>::digest(name.as_bytes()).iter().map(|b| format!("{b:02x}")).collect()
}

/// `kind = machine`: a Device's own metadata and presence claims.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct MachineBlob {
    pub device: Device,
    /// The turns in flight on this Device, so every other Device shows the bots at work.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub turns: Vec<LiveTurn>,
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
    /// The first turn of a bot added from a marketplace template: what it sets up before it
    /// answers.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub setup: Option<TemplateSetup>,
    pub created_at: f64,
}

/// What a bot added from a marketplace template sets up on its first turn, as Grok Bot's
/// template import asks its new bot to.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
pub struct TemplateSetup {
    /// The template's name, such as "Researcher".
    pub template: String,
    /// The plugins it works with. The Runner checks which it has installed.
    #[serde(default)]
    pub plugins: Vec<SetupPlugin>,
    /// Its routines, added paused, by name.
    #[serde(default)]
    pub routines: Vec<String>,
    /// Facts it saves to its memory.
    #[serde(default)]
    pub memory: Vec<String>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
pub struct SetupPlugin {
    pub id: String,
    pub name: String,
    #[serde(default)]
    pub description: String,
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

/// A turn in flight on the Device whose machine blob lists it: a bot's turn on its Runner, or
/// a group exchange on the Device that offers its members their turns.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct LiveTurn {
    pub job_id: String,
    pub chat_id: String,
    /// Empty while a group exchange is between member turns.
    pub bot_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub routine_id: Option<String>,
    /// What the turn is doing that no message says yet. The bot's next message ends it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub activity: Option<JobActivity>,
}

/// What a turn is doing between its messages: the working row's "Thinking…" and "Retrying…".
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum JobActivity {
    /// The model started thinking; the bot's next message is what came of it.
    Thinking,
    /// A model call failed in a way worth another try; the turn waits `delay_ms` and asks again.
    Retry { attempt: u32, max_attempts: u32, delay_ms: u64, error: String },
}

impl JobActivity {
    /// The event an app hears for it.
    pub fn event(&self, chat_id: &str, bot_id: &str) -> crate::events::Event {
        let (chat_id, bot_id) = (chat_id.to_string(), bot_id.to_string());
        match self {
            JobActivity::Thinking => crate::events::Event::JobThinking { chat_id, bot_id },
            JobActivity::Retry { attempt, max_attempts, delay_ms, error } => crate::events::Event::JobRetry {
                chat_id,
                bot_id,
                attempt: *attempt,
                max_attempts: *max_attempts,
                delay_ms: *delay_ms,
                error: error.clone(),
            },
        }
    }
}

/// `kind = job_cancel`, sealed to the Runner's box key: the hard Stop control reaches a job
/// that another Device dispatched there. The job registers before waiting for its chat lock,
/// so Stop also removes admitted work that has not begun yet.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct JobCancel {
    pub job_id: String,
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

/// Machine facts for this host: what the app set, or probed from the system once per process.
pub fn host_facts() -> (String, String, String, String) {
    static PROBED: std::sync::OnceLock<(String, String, String, String)> = std::sync::OnceLock::new();
    HOST_FACTS.get().unwrap_or_else(|| PROBED.get_or_init(probe_host)).clone()
}

/// A Mac's marketing name with its chip, "MacBook Air (M5)". `hw.model` is an identifier such as
/// `Mac17,3` on current Macs, which says nothing a person or an icon can use.
fn mac_model_name() -> Option<String> {
    let output = std::process::Command::new("system_profiler").args(["SPHardwareDataType", "-json"]).output().ok()?;
    let json: serde_json::Value = serde_json::from_slice(&output.stdout).ok()?;
    let hardware = json.get("SPHardwareDataType")?.get(0)?;
    let name = hardware.get("machine_name")?.as_str()?.trim();
    if name.is_empty() {
        return None;
    }
    let chip = hardware.get("chip_type").and_then(|c| c.as_str()).map(|c| c.trim_start_matches("Apple ").trim()).filter(|c| !c.is_empty());
    Some(match chip {
        Some(chip) => format!("{name} ({chip})"),
        None => name.to_string(),
    })
}

fn probe_host() -> (String, String, String, String) {
    let name = std::process::Command::new("scutil")
        .args(["--get", "ComputerName"])
        .output()
        .ok()
        .and_then(|o| String::from_utf8(o.stdout).ok())
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
        .or_else(|| std::env::var("HOSTNAME").ok())
        .unwrap_or_else(|| "This Computer".to_string());
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
    let model = mac_model_name()
        .or_else(|| {
            std::process::Command::new("sysctl")
                .args(["-n", "hw.model"])
                .output()
                .ok()
                .and_then(|o| String::from_utf8(o.stdout).ok())
                .map(|s| s.trim().to_string())
                .filter(|s| !s.is_empty())
        })
        .unwrap_or_else(|| std::env::consts::ARCH.to_string());
    (name, os.to_string(), os_version, model)
}

#[cfg(test)]
mod app_view_tests {
    use super::*;

    #[test]
    fn old_bot_instructions_become_part_of_the_description_once() {
        let mut bot: Bot = serde_json::from_value(serde_json::json!({
            "id": "bot", "name": "Scout", "description": "Find sources.",
            "symbol_name": "binoculars", "accent": "teal", "runner_id": "runner",
            "provider": "deepseek", "instructions": "Cite every claim.", "created_at": 1.0
        }))
        .unwrap();

        assert!(bot.normalize_description());
        assert_eq!(bot.description, "Find sources.\n\nCite every claim.");
        assert!(bot.legacy_instructions.is_empty());
        assert!(!bot.normalize_description());
        let value = serde_json::to_value(bot).unwrap();
        assert!(value.get("label").is_none());
        assert_eq!(value["instructions"], "");
    }

    #[test]
    fn the_apps_get_a_tool_row_without_its_payload() {
        let tool = Message::new("c", Author::Bot { bot_id: "b".into() }, Body::Tool {
            name: "read".into(), summary: "Read a file".into(), detail: "x".repeat(5000), is_running: false,
            call_id: "call".into(), arguments: serde_json::json!({ "path": "big" }), result: Some("y".repeat(100_000)), is_error: false, description: None, target_bot_id: None,
        });
        let Body::Tool { detail, arguments, result, summary, .. } = tool.for_app().body else { panic!() };
        assert_eq!((detail.len(), arguments.is_null(), result, summary.as_str()), (400, true, None, "Read a file"));
    }

    #[test]
    fn permission_cards_drop_the_reviewed_payload() {
        let permission = Message::new("c", Author::Bot { bot_id: "b".into() }, Body::Permission {
            plugin_id: "computer".into(), plugin_name: "Mac".into(), tool: "bash".into(), summary: "Run a command".into(),
            arguments: serde_json::json!({ "command": "secret" }), decision: "pending".into(), reason: None, rule: None, command: None, link: None, code: None,
        });
        let app = permission.for_app();
        let Body::Permission { arguments, summary, command, .. } = &app.body else { panic!() };
        assert!(arguments.is_null());
        assert_eq!(summary, "Run a command");
        assert_eq!(command.as_deref(), Some("secret"));
        // A phone stores the app view; reading it again keeps the command.
        let Body::Permission { command, .. } = app.for_app().body else { panic!() };
        assert_eq!(command.as_deref(), Some("secret"));
    }
}
