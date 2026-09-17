//! Process-wide state: keys, the plaintext store, the outbox to the relay, and the event bus.

use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use tokio::sync::{broadcast, Notify};
use tokio_util::sync::CancellationToken;

use crate::config::{self, Config, Settings};
use crate::events::{ChatSummary, Event};
use crate::keys::{self, IdentityFile, MachineFile};
use crate::model::*;
use crate::credentials::Credentials;
use crate::relay::RelayClient;

pub const ONLINE_WINDOW_SECS: i64 = 150;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OutboxItem {
    pub id: String,
    pub kind: String,
    #[serde(default)]
    pub recipient: Option<String>,
    /// base64url
    pub ciphertext: String,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct State {
    #[serde(default)]
    pub devices: Vec<Device>,
    #[serde(default)]
    pub bots: Vec<Bot>,
    #[serde(default)]
    pub chats: Vec<Chat>,
    #[serde(default)]
    pub last_seq: i64,
    #[serde(default)]
    pub outbox: Vec<OutboxItem>,
    #[serde(default)]
    pub machine_blob_hash: Option<String>,
    /// machine pubkey → last seen (unix), from the relay's machine list.
    #[serde(default)]
    pub device_seen: HashMap<String, i64>,
    /// Relay blob ids this device produced or already applied, so its own echoes are no-ops.
    #[serde(default)]
    pub applied_blob_ids: Vec<String>,
}

/// A pairing this identity device is waiting on.
pub struct PendingPairing {
    pub ephemeral: crypto_box::SecretKey,
    pub status: PairingStatus,
    pub cancel: CancellationToken,
}

#[derive(Debug, Clone, Serialize)]
#[serde(tag = "state", rename_all = "snake_case")]
pub enum PairingStatus {
    Waiting,
    Completed { device: Value },
    Failed { error: String },
}

pub struct App {
    pub config: Config,
    pub settings: Mutex<Settings>,
    pub identity: Mutex<Option<IdentityFile>>,
    pub machine: Mutex<Option<MachineFile>>,
    pub credentials: Mutex<Credentials>,
    pub state: Mutex<State>,
    pub events: broadcast::Sender<Event>,
    pub relay: RelayClient,
    pub outbox_notify: Notify,
    pub relay_connected: AtomicBool,
    pub pairings: Mutex<HashMap<String, PendingPairing>>,
    /// The pairing this Device is joining, while `pair.accept` waits for the reply.
    pub accepting: Mutex<Option<CancellationToken>>,
    /// job id → (chat id, cancel)
    pub running_jobs: Mutex<HashMap<String, (String, String, CancellationToken)>>,
    pub chat_locks: Mutex<HashMap<String, Arc<tokio::sync::Mutex<()>>>>,
    /// `room_turn` jobs sent to other Runners, waiting for their `job_result`.
    pub pending_results: Mutex<HashMap<String, tokio::sync::oneshot::Sender<String>>>,
    /// Requests sent to other Runners, waiting for their `response`.
    pub pending_responses: Mutex<HashMap<String, tokio::sync::oneshot::Sender<Response>>>,
    pub http: reqwest::Client,
}

impl App {
    pub fn load(config: Config) -> anyhow::Result<Arc<App>> {
        config.ensure_home()?;
        let settings = Settings::load(&config);
        let identity: Option<IdentityFile> = config::read_json(&config.identity_path());
        let machine: Option<MachineFile> = config::read_json(&config.machine_path());
        let credentials = Credentials::load(&config);
        let mut state: State = config::read_json(&config.state_path()).unwrap_or_default();
        // Upload this Device's metadata once per launch: a relay that changed or was reset
        // since the last upload has no copy, and every newly paired Device needs one.
        state.machine_blob_hash = None;
        let (events, _) = broadcast::channel(512);
        let http = reqwest::Client::builder().timeout(std::time::Duration::from_secs(60)).build()?;

        Ok(Arc::new(App {
            config,
            settings: Mutex::new(settings),
            identity: Mutex::new(identity),
            machine: Mutex::new(machine),
            credentials: Mutex::new(credentials),
            state: Mutex::new(state),
            events,
            relay: RelayClient::new(http.clone()),
            outbox_notify: Notify::new(),
            relay_connected: AtomicBool::new(false),
            pairings: Mutex::new(HashMap::new()),
            accepting: Mutex::new(None),
            running_jobs: Mutex::new(HashMap::new()),
            chat_locks: Mutex::new(HashMap::new()),
            pending_results: Mutex::new(HashMap::new()),
            pending_responses: Mutex::new(HashMap::new()),
            http,
        }))
    }

    // MARK: - Persistence

    pub fn save_state(&self) {
        let snapshot = self.state.lock().unwrap().clone();
        if let Err(error) = config::write_json_private(&self.config.state_path(), &snapshot) {
            tracing::error!(%error, "saving state");
        }
    }

    pub fn save_machine(&self) -> anyhow::Result<()> {
        let machine = self.machine.lock().unwrap().clone();
        match machine {
            Some(machine) => config::write_json_private(&self.config.machine_path(), &machine),
            None => Ok(()),
        }
    }

    pub fn save_identity(&self) -> anyhow::Result<()> {
        let identity = self.identity.lock().unwrap().clone();
        match identity {
            Some(identity) => config::write_json_private(&self.config.identity_path(), &identity),
            None => Ok(()),
        }
    }

    pub fn save_credentials(&self) -> anyhow::Result<()> {
        let credentials = self.credentials.lock().unwrap().clone();
        credentials.save(&self.config)
    }

    pub fn emit(&self, event: Event) {
        let _ = self.events.send(event);
    }

    // MARK: - Keys

    pub fn has_identity(&self) -> bool {
        self.machine.lock().unwrap().is_some()
    }

    pub fn is_identity_device(&self) -> bool {
        self.identity.lock().unwrap().is_some()
    }

    pub fn machine_file(&self) -> Option<MachineFile> {
        self.machine.lock().unwrap().clone()
    }

    pub fn this_device_id(&self) -> Option<String> {
        self.machine_file().and_then(|m| m.machine().ok()).map(|m| m.pubkey())
    }

    pub fn dek(&self) -> Option<[u8; 32]> {
        self.machine_file().and_then(|m| m.dek().ok())
    }

    /// Settings, then `TINYBOT_RELAY_URL`, then the URL pairing handed this Device, then, in
    /// dev, the relay the dev loop runs on this machine.
    pub fn relay_url(&self) -> Option<String> {
        let from_settings = self.settings.lock().unwrap().effective_relay_url();
        from_settings
            .or_else(|| std::env::var("TINYBOT_RELAY_URL").ok().filter(|s| !s.is_empty()))
            .or_else(|| self.machine_file().and_then(|m| m.relay_url))
            .or_else(config::dev_relay_url)
    }

    pub fn set_relay_url(&self, url: Option<String>) -> anyhow::Result<()> {
        let mut settings = self.settings.lock().unwrap();
        settings.relay_url = url.map(|u| u.trim().trim_end_matches('/').to_string()).filter(|u| !u.is_empty());
        settings.save(&self.config)?;
        drop(settings);
        self.relay.forget_token();
        self.outbox_notify.notify_waiters();
        Ok(())
    }

    /// Renames this Device; the roster hears through the machine blob.
    pub fn rename_device(&self, name: &str) -> anyhow::Result<()> {
        let name = name.trim();
        if name.is_empty() {
            anyhow::bail!("Give the Device a name");
        }
        {
            let mut machine = self.machine.lock().unwrap();
            let Some(file) = machine.as_mut() else { anyhow::bail!("No identity on this Device") };
            file.name = name.to_string();
        }
        self.save_machine()?;
        if let Some(device) = self.local_device() {
            upsert_device(&mut self.state.lock().unwrap().devices, device);
        }
        self.save_state();
        self.push_machine_blob_if_changed();
        self.emit(self.roster_summary());
        Ok(())
    }

    /// Forgets the identity on this Device: keys, credentials, and everything synced. The
    /// relay keeps the account; another Device or the backup phrase brings it back.
    pub fn forget_identity(&self) -> anyhow::Result<()> {
        for (_, _, cancel) in self.running_jobs.lock().unwrap().values() {
            cancel.cancel();
        }
        *self.identity.lock().unwrap() = None;
        *self.machine.lock().unwrap() = None;
        *self.credentials.lock().unwrap() = Credentials::default();
        *self.state.lock().unwrap() = State::default();
        self.settings.lock().unwrap().relay_url = None;
        self.relay.forget_token();
        for path in [self.config.identity_path(), self.config.machine_path(), self.config.credentials_path(), self.config.state_path(), self.config.settings_path()] {
            if path.exists() {
                std::fs::remove_file(&path)?;
            }
        }
        let files = self.config.files_dir();
        if files.is_dir() {
            std::fs::remove_dir_all(&files)?;
        }
        self.emit(Event::IdentityChanged { has_identity: false });
        self.emit(Event::Snapshot(self.snapshot()));
        Ok(())
    }

    /// This Device as the roster sees it.
    pub fn local_device(&self) -> Option<Device> {
        let machine = self.machine_file()?;
        let keys = machine.machine().ok()?;
        Some(Device {
            id: keys.pubkey(),
            name: machine.name.clone(),
            model: machine.model.clone(),
            os: machine.os.clone(),
            os_version: machine.os_version.clone(),
            box_pubkey: keys.box_pubkey(),
            providers_connected: self.credentials.lock().unwrap().connected_kinds(),
            updated_at: config::now_unix(),
        })
    }

    // MARK: - Outbox

    pub fn push_blob(&self, kind: &str, recipient: Option<String>, ciphertext: Vec<u8>) -> String {
        self.push_blob_as(uuid::Uuid::new_v4().to_string(), kind, recipient, ciphertext)
    }

    /// Queues a blob under a chosen id: a `file` blob carries its attachment's id so any
    /// Device can fetch it by that id later.
    pub fn push_blob_as(&self, id: String, kind: &str, recipient: Option<String>, ciphertext: Vec<u8>) -> String {
        {
            let mut state = self.state.lock().unwrap();
            state.outbox.push(OutboxItem { id: id.clone(), kind: kind.to_string(), recipient, ciphertext: keys::b64(&ciphertext) });
            remember_applied(&mut state, &id);
        }
        self.save_state();
        self.outbox_notify.notify_waiters();
        id
    }

    pub fn push_roster(&self) {
        let Some(dek) = self.dek() else { return };
        let roster = {
            let state = self.state.lock().unwrap();
            RosterBlob {
                bots: state.bots.clone(),
                chats: state.chats.iter().map(|c| c.meta.clone()).collect(),
                updated_at: config::now_secs(),
            }
        };
        match crate::crypto::encrypt_json(&dek, "roster", &roster) {
            Ok(ciphertext) => {
                self.push_blob("roster", None, ciphertext);
            }
            Err(error) => tracing::error!(%error, "encrypting roster"),
        }
    }

    pub fn push_chat_op(&self, op: &ChatBlob) {
        let Some(dek) = self.dek() else { return };
        match crate::crypto::encrypt_json(&dek, "chat", op) {
            Ok(ciphertext) => {
                self.push_blob("chat", None, ciphertext);
            }
            Err(error) => tracing::error!(%error, "encrypting chat op"),
        }
    }

    /// Uploads this Device's metadata when it changed since the last upload.
    pub fn push_machine_blob_if_changed(&self) {
        let (Some(dek), Some(device)) = (self.dek(), self.local_device()) else { return };
        let fingerprint = format!(
            "{}|{}|{}|{}|{}|{:?}",
            device.id, device.name, device.model, device.os, device.os_version, device.providers_connected
        );
        let hash = keys::b64(&<sha2::Sha256 as sha2::Digest>::digest(fingerprint.as_bytes()));
        let changed = {
            let mut state = self.state.lock().unwrap();
            upsert_device(&mut state.devices, device.clone());
            if state.machine_blob_hash.as_deref() == Some(&hash) {
                false
            } else {
                state.machine_blob_hash = Some(hash);
                true
            }
        };
        if !changed {
            return;
        }
        match crate::crypto::encrypt_json(&dek, "machine", &MachineBlob { device }) {
            Ok(ciphertext) => {
                self.push_blob("machine", None, ciphertext);
            }
            Err(error) => tracing::error!(%error, "encrypting machine blob"),
        }
    }

    // MARK: - Lookup

    pub fn bot(&self, id: &str) -> Option<Bot> {
        self.state.lock().unwrap().bots.iter().find(|b| b.id == id).cloned()
    }

    pub fn chat(&self, id: &str) -> Option<Chat> {
        self.state.lock().unwrap().chats.iter().find(|c| c.meta.id == id).cloned()
    }

    pub fn device(&self, id: &str) -> Option<Device> {
        self.state.lock().unwrap().devices.iter().find(|d| d.id == id).cloned()
    }

    pub fn device_is_online(&self, id: &str) -> bool {
        if self.this_device_id().as_deref() == Some(id) {
            return true;
        }
        let state = self.state.lock().unwrap();
        state.device_seen.get(id).map(|seen| config::now_unix() - seen < ONLINE_WINDOW_SECS).unwrap_or(false)
    }

    pub fn chat_lock(&self, chat_id: &str) -> Arc<tokio::sync::Mutex<()>> {
        self.chat_locks.lock().unwrap().entry(chat_id.to_string()).or_default().clone()
    }

    /// Every turn in flight, for the app's working indicators: `(chat id, bot id)`; a group
    /// exchange between member turns has an empty bot id.
    pub fn running_turns(&self) -> Vec<Value> {
        self.running_jobs
            .lock()
            .unwrap()
            .iter()
            .map(|(job_id, (chat, bot, _))| json!({ "job_id": job_id, "chat_id": chat, "bot_id": bot }))
            .collect()
    }

    pub fn running_chat_ids(&self) -> Vec<String> {
        let mut ids: Vec<String> = self.running_jobs.lock().unwrap().values().map(|(chat, _, _)| chat.clone()).collect();
        ids.sort();
        ids.dedup();
        ids
    }

    // MARK: - Roster mutations (local + roster upload)

    pub fn roster_summary(&self) -> Event {
        let state = self.state.lock().unwrap();
        Event::RosterChanged {
            devices: self.devices_out(&state),
            bots: state.bots.clone(),
            chats: state.chats.iter().map(|c| ChatSummary { meta: c.meta.clone(), unread_count: c.unread_count, usage: c.usage.clone() }).collect(),
        }
    }

    pub fn roster_changed(&self, upload: bool) {
        self.save_state();
        if upload {
            self.push_roster();
        }
        self.emit(self.roster_summary());
    }

    /// Creates the bot and its direct chat in one roster change, so other Devices (and the
    /// app's optimistic rows) never see a bot without its DM.
    pub fn create_bot_with_dm(&self, bot: Bot, chat_id: Option<String>) -> anyhow::Result<(Bot, Chat)> {
        let bot = self.insert_bot(bot)?;
        let chat = self.dm_with(&bot.id, chat_id)?;
        Ok((bot, chat))
    }

    /// Applies a change to a bot's profile and publishes the roster. The next turn of that bot
    /// reads the new profile.
    pub fn update_bot(&self, id: &str, update: impl FnOnce(&mut Bot)) -> anyhow::Result<Bot> {
        let bot = {
            let mut state = self.state.lock().unwrap();
            let bot = state.bots.iter_mut().find(|b| b.id == id).ok_or_else(|| anyhow::anyhow!("Unknown bot"))?;
            update(bot);
            bot.clone()
        };
        self.roster_changed(true);
        crate::runtime::prime_names(self);
        Ok(bot)
    }

    fn insert_bot(&self, mut bot: Bot) -> anyhow::Result<Bot> {
        let runner = self.device(&bot.runner_id).ok_or_else(|| anyhow::anyhow!("Unknown Runner"))?;
        if !runner.is_runner() {
            anyhow::bail!("{} runs {} and cannot run bots", runner.name, runner.os);
        }
        if bot.id.is_empty() {
            bot.id = format!("bot-{}", &uuid::Uuid::new_v4().to_string()[..8]);
        }
        if bot.created_at == 0.0 {
            bot.created_at = config::now_secs();
        }
        {
            let mut state = self.state.lock().unwrap();
            if state.bots.iter().any(|b| b.id == bot.id) {
                anyhow::bail!("Bot id already exists");
            }
            state.bots.push(bot.clone());
        }
        Ok(bot)
    }

    pub fn create_chat(&self, mut meta: ChatMeta) -> anyhow::Result<Chat> {
        if meta.id.is_empty() {
            meta.id = format!("chat-{}", &uuid::Uuid::new_v4().to_string()[..8]);
        }
        if meta.created_at == 0.0 {
            meta.created_at = config::now_secs();
        }
        let ids: Vec<String> = if meta.kind == "dm" {
            meta.bot_ids.iter().take(1).cloned().collect()
        } else {
            meta.kind = "group".into();
            meta.bot_ids.iter().take(MAX_GROUP_BOTS).cloned().collect()
        };
        meta.bot_ids = ids;
        if meta.bot_ids.is_empty() {
            anyhow::bail!("A chat needs at least one bot");
        }
        if meta.owner_bot_id.as_ref().map(|id| !meta.bot_ids.contains(id)).unwrap_or(true) {
            meta.owner_bot_id = meta.bot_ids.first().cloned();
        }
        {
            let state = self.state.lock().unwrap();
            for id in &meta.bot_ids {
                if !state.bots.iter().any(|b| &b.id == id) {
                    anyhow::bail!("Unknown bot {id}");
                }
            }
        }
        let chat = Chat { meta, messages: Vec::new(), unread_count: 0, usage: None, compactions: Vec::new() };
        {
            let mut state = self.state.lock().unwrap();
            if let Some(existing) = state.chats.iter().find(|c| c.meta.id == chat.meta.id) {
                return Ok(existing.clone());
            }
            state.chats.insert(0, chat.clone());
        }
        self.roster_changed(true);
        Ok(chat)
    }

    /// The one DM per bot.
    pub fn dm_with(&self, bot_id: &str, preferred_id: Option<String>) -> anyhow::Result<Chat> {
        if let Some(existing) = self
            .state
            .lock()
            .unwrap()
            .chats
            .iter()
            .find(|c| c.meta.kind == "dm" && c.meta.bot_ids == vec![bot_id.to_string()])
            .cloned()
        {
            return Ok(existing);
        }
        self.create_chat(ChatMeta {
            id: preferred_id.unwrap_or_default(),
            kind: "dm".into(),
            title: None,
            bot_ids: vec![bot_id.to_string()],
            owner_bot_id: Some(bot_id.to_string()),
            is_pinned: false,
            created_at: 0.0,
        })
    }

    pub fn delete_chat(&self, chat_id: &str) {
        self.cancel_chat(chat_id);
        {
            let mut state = self.state.lock().unwrap();
            state.chats.retain(|c| c.meta.id != chat_id);
        }
        self.emit(Event::ChatRemoved { chat_id: chat_id.to_string() });
        self.roster_changed(true);
    }

    pub fn update_chat_meta(&self, chat_id: &str, update: impl FnOnce(&mut ChatMeta)) -> anyhow::Result<()> {
        {
            let mut state = self.state.lock().unwrap();
            let chat = state.chats.iter_mut().find(|c| c.meta.id == chat_id).ok_or_else(|| anyhow::anyhow!("Unknown chat"))?;
            update(&mut chat.meta);
        }
        self.roster_changed(true);
        Ok(())
    }

    pub fn mark_read(&self, chat_id: &str) {
        let changed = {
            let mut state = self.state.lock().unwrap();
            match state.chats.iter_mut().find(|c| c.meta.id == chat_id) {
                Some(chat) if chat.unread_count > 0 => {
                    chat.unread_count = 0;
                    true
                }
                _ => false,
            }
        };
        if changed {
            self.save_state();
            self.emit(self.roster_summary());
        }
    }

    // MARK: - Messages

    /// Insert or replace a message locally, emit, and optionally upload.
    pub fn upsert_message(&self, message: Message, upload: bool) {
        let (added, changed) = {
            let mut state = self.state.lock().unwrap();
            let Some(chat) = state.chats.iter_mut().find(|c| c.meta.id == message.chat_id) else { return };
            match chat.messages.iter_mut().find(|m| m.id == message.id) {
                Some(existing) => {
                    let changed = *existing != message;
                    *existing = message.clone();
                    (false, changed)
                }
                None => {
                    chat.messages.push(message.clone());
                    (true, true)
                }
            }
        };
        if added {
            self.emit(Event::MessageAdded { chat_id: message.chat_id.clone(), message: message.clone() });
        } else if changed {
            self.emit(Event::MessageUpdated { chat_id: message.chat_id.clone(), message: message.clone() });
        }
        if upload {
            self.save_state();
            self.push_chat_op(&ChatBlob::Upsert { message });
        }
    }

    pub fn remove_message(&self, chat_id: &str, message_id: &str, upload: bool) {
        let removed = {
            let mut state = self.state.lock().unwrap();
            let Some(chat) = state.chats.iter_mut().find(|c| c.meta.id == chat_id) else { return };
            let before = chat.messages.len();
            chat.messages.retain(|m| m.id != message_id);
            before != chat.messages.len()
        };
        if removed {
            self.emit(Event::MessageRemoved { chat_id: chat_id.to_string(), message_id: message_id.to_string() });
            self.save_state();
            if upload {
                self.push_chat_op(&ChatBlob::Remove { chat_id: chat_id.to_string(), message_id: message_id.to_string() });
            }
        }
    }

    pub fn message(&self, chat_id: &str, message_id: &str) -> Option<Message> {
        let state = self.state.lock().unwrap();
        state.chats.iter().find(|c| c.meta.id == chat_id)?.messages.iter().find(|m| m.id == message_id).cloned()
    }

    pub fn notice(&self, chat_id: &str, text: impl Into<String>) {
        let message = Message::new(chat_id, Author::System, Body::Notice { text: text.into() });
        self.upsert_message(message, true);
    }

    /// Adds a finished turn's usage to the chat's and tells the app.
    #[cfg(feature = "runner")]
    pub fn record_usage(&self, chat_id: &str, model: &str, usage: &tinybot_agent::Usage, context_window: u64) {
        let updated = {
            let mut state = self.state.lock().unwrap();
            let Some(chat) = state.chats.iter_mut().find(|c| c.meta.id == chat_id) else { return };
            let entry = chat.usage.get_or_insert_with(ChatUsage::default);
            entry.context_tokens = tinybot_agent::estimate::context_tokens(usage);
            entry.context_window = context_window;
            entry.input_tokens += usage.input + usage.cache_read + usage.cache_write;
            entry.output_tokens += usage.output;
            entry.cache_read_tokens += usage.cache_read;
            entry.cost_usd += usage.cost.total;
            entry.turns += 1;
            entry.model = model.to_string();
            entry.updated_at = crate::config::now_secs();
            entry.clone()
        };
        self.save_state();
        self.emit(Event::ChatUsageChanged { chat_id: chat_id.to_string(), usage: updated });
    }

    /// Replaces the bot's summary of the chat so far, or clears it.
    pub fn set_compaction(&self, chat_id: &str, bot_id: &str, compaction: Option<Compaction>) {
        {
            let mut state = self.state.lock().unwrap();
            let Some(chat) = state.chats.iter_mut().find(|c| c.meta.id == chat_id) else { return };
            chat.compactions.retain(|c| c.bot_id != bot_id);
            if let Some(compaction) = compaction {
                chat.compactions.push(compaction);
            }
        }
        self.save_state();
    }

    // MARK: - Jobs

    pub fn cancel_chat(&self, chat_id: &str) {
        let jobs = self.running_jobs.lock().unwrap();
        for (chat, _, cancel) in jobs.values() {
            if chat == chat_id {
                cancel.cancel();
            }
        }
    }

    // MARK: - Snapshot

    fn devices_out(&self, state: &State) -> Vec<Value> {
        let this_id = self.this_device_id();
        let now = config::now_unix();
        let mut devices: Vec<&Device> = state.devices.iter().collect();
        devices.sort_by_key(|d| (Some(d.id.clone()) != this_id, d.name.to_lowercase()));
        devices
            .into_iter()
            .map(|device| {
                let is_this = Some(device.id.clone()) == this_id;
                let seen = state.device_seen.get(&device.id).copied().unwrap_or(device.updated_at);
                let status = if is_this || now - seen < ONLINE_WINDOW_SECS { "online" } else { "offline" };
                let providers: Vec<ProviderStatus> = if is_this {
                    self.credentials.lock().unwrap().statuses()
                } else if device.is_runner() {
                    crate::credentials::PROVIDER_KINDS
                        .iter()
                        .map(|kind| ProviderStatus {
                            kind: kind.to_string(),
                            is_connected: device.providers_connected.iter().any(|k| k == kind),
                            detail: if device.providers_connected.iter().any(|k| k == kind) {
                                format!("Connected on {}", device.name)
                            } else {
                                "Not connected".into()
                            },
                            base_url: None,
                        })
                        .collect()
                } else {
                    Vec::new()
                };
                json!({
                    "id": device.id,
                    "name": device.name,
                    "model": device.model,
                    "os": device.os,
                    "os_version": device.os_version,
                    "machine_key": short_key(&device.id),
                    "is_this_device": is_this,
                    "status": status,
                    "last_seen": seen as f64,
                    "providers": providers,
                })
            })
            .collect()
    }

    pub fn snapshot(&self) -> Value {
        let state = self.state.lock().unwrap();
        let identity_id = self.machine_file().map(|m| keys::identity_id(&m.identity_pubkey));
        json!({
            "version": config::VERSION,
            "has_identity": self.has_identity(),
            "is_identity_device": self.is_identity_device(),
            "identity_id": identity_id,
            "this_device_id": self.this_device_id(),
            "relay_url": self.relay_url(),
            "relay_connected": self.relay_connected.load(Ordering::Relaxed),
            "devices": self.devices_out(&state),
            "bots": state.bots,
            "chats": state.chats,
            "running_chat_ids": self.running_chat_ids(),
            "running_turns": self.running_turns(),
        })
    }
}

pub fn short_key(key: &str) -> String {
    if key.len() > 10 {
        format!("mk_{}…{}", &key[..4], &key[key.len() - 4..])
    } else {
        key.to_string()
    }
}

pub fn upsert_device(devices: &mut Vec<Device>, device: Device) -> bool {
    match devices.iter_mut().find(|d| d.id == device.id) {
        Some(existing) => {
            if *existing == device {
                false
            } else {
                *existing = device;
                true
            }
        }
        None => {
            devices.push(device);
            true
        }
    }
}

pub fn remember_applied(state: &mut State, id: &str) {
    state.applied_blob_ids.push(id.to_string());
    if state.applied_blob_ids.len() > 2000 {
        let excess = state.applied_blob_ids.len() - 2000;
        state.applied_blob_ids.drain(..excess);
    }
}
