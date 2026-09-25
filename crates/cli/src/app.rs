//! Process-wide state: keys, the in-memory view of the local SQLite store, and the event bus.

use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};

use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use tokio::sync::{broadcast, Notify};
use tokio_util::sync::CancellationToken;

use crate::config::{self, Config, Settings};
use crate::events::{ChatSummary, Event};
use crate::keys::{self, IdentityFile, MachineFile};
use crate::model::*;
use crate::local_store::LocalStore;
use crate::credentials::Credentials;
use crate::relay::RelayClient;


#[derive(Debug, Clone)]
pub struct OutboxItem {
    pub id: String,
    pub kind: String,
    pub recipient: Option<String>,
    /// Encrypted bytes, stored as a SQLite BLOB until upload.
    pub ciphertext: Vec<u8>,
    /// What this blob is a version of. The relay drops the versions it supersedes.
    pub slot: Option<Slot>,
    /// The chat this blob belongs to. The relay deletes a group's blobs in one call.
    pub group: Option<String>,
}

/// A blob's place among the versions of one thing: a message, the roster, this Device's
/// metadata. The relay keeps the latest blob of a slot, and with `keep_first` the oldest too:
/// its seq holds a message's place in the log for a Device that replays it from the start.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Slot {
    pub name: String,
    #[serde(default)]
    pub keep_first: bool,
}

impl Slot {
    pub fn latest(name: impl Into<String>) -> Slot {
        Slot { name: name.into(), keep_first: false }
    }

    pub fn first_and_latest(name: impl Into<String>) -> Slot {
        Slot { name: name.into(), keep_first: true }
    }
}

/// The process's in-memory projection of the local SQLite tables.
#[derive(Debug, Clone, Default)]
pub struct State {
    pub devices: Vec<Device>,
    pub bots: Vec<Bot>,
    pub chats: Vec<Chat>,
    pub routines: Vec<Routine>,
    pub auto_review: AutoReview,
    pub last_seq: i64,
    /// Chats deleted here whose blobs the relay still has to drop.
    pub group_deletes: Vec<String>,
    /// Avatars dropped here whose `file` blobs the relay still has to drop.
    pub blob_deletes: Vec<String>,
    pub machine_blob_hash: Option<String>,
    /// Whether the relay has had this Device's credentials since they last changed here.
    pub credentials_uploaded: bool,
    /// machine pubkey → last seen (unix), from the relay's machine list.
    pub device_seen: HashMap<String, i64>,
    /// The machines with a sync socket open on the relay, as of the last machine list. Not
    /// kept across runs: it is only good while this Device's own socket is open.
    pub device_online: std::collections::HashSet<String>,
    /// The same machines, kept while this Device's own socket is down: the turns they list
    /// keep showing until the relay says otherwise, instead of ending and starting again
    /// whenever this Device reconnects. Not kept across runs.
    pub turns_online: std::collections::HashSet<String>,
    /// The turns each other Device's latest machine blob lists, by machine pubkey.
    pub device_turns: HashMap<String, Vec<LiveTurn>>,
    /// Relay blob ids this device produced or already applied, so its own echoes are no-ops.
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

/// A turn in flight on this Device, or one it sent to another Runner and waits on.
#[derive(Debug, Clone)]
pub struct RunningJob {
    pub chat_id: String,
    /// Empty while a group exchange is between member turns.
    pub bot_id: String,
    /// Set when the turn is a run of a routine.
    pub routine_id: Option<String>,
    /// The other Runner executing this job, when this Device sent it there.
    pub runner_id: Option<String>,
    pub cancel: CancellationToken,
    /// What a turn running here is doing between its messages, for every app's working row.
    pub activity: Option<JobActivity>,
}

impl RunningJob {
    fn turn(&self, job_id: &str) -> LiveTurn {
        LiveTurn {
            job_id: job_id.to_string(),
            chat_id: self.chat_id.clone(),
            bot_id: self.bot_id.clone(),
            routine_id: self.routine_id.clone(),
            activity: self.activity.clone(),
        }
    }
}

/// A job this Device sealed to another Runner and still waits on, as SQLite keeps it: the wait
/// outlives the process, so a restarted CLI or phone core still shows the bot at work.
#[derive(Debug, Clone, PartialEq)]
pub struct SentJob {
    pub id: String,
    pub chat_id: String,
    pub bot_id: String,
    pub routine_id: Option<String>,
    pub runner_id: String,
    /// When the wait began, unix seconds.
    pub sent_at: f64,
}

pub struct App {
    pub config: Config,
    pub settings: Mutex<Settings>,
    pub identity: Mutex<Option<IdentityFile>>,
    pub machine: Mutex<Option<MachineFile>>,
    pub credentials: Mutex<Credentials>,
    pub state: Mutex<State>,
    pub store: LocalStore,
    pub events: broadcast::Sender<Event>,
    pub relay: RelayClient,
    pub outbox_notify: Notify,
    /// Counts `wake_sync` calls, so the sync loop can tell that one came while it was busy.
    pub sync_wakes: AtomicU64,
    /// Held from a message's local write to its outbox enqueue, so a chat's `position` order
    /// and the order its messages reach the relay log are the same on every Device.
    message_order: Mutex<()>,
    pub relay_connected: AtomicBool,
    /// The relay answered `426`: it no longer serves the protocol this build speaks.
    pub relay_update_required: AtomicBool,
    /// A `machine` blob named a key the last presence refresh did not list: a Device that
    /// just paired, or one unpaired since. The cycle refreshes presence again to tell.
    pub presence_stale: AtomicBool,
    /// Set while the sync loop applies a backlog of blobs (a fresh pair replays the whole
    /// history). Message and roster events are held back and state is not written per
    /// message; the cycle saves once and emits one snapshot when the page is applied.
    pub bulk_sync: AtomicBool,
    pub pairings: Mutex<HashMap<String, PendingPairing>>,
    /// The pairing this Device is joining, while `pair.accept` waits for the reply.
    pub accepting: Mutex<Option<CancellationToken>>,
    /// The provider browser sign-in in flight on this Device. A newer one, or the browser
    /// closing on a phone, cancels the old wait.
    #[cfg(feature = "provider-auth")]
    pub provider_auth: Mutex<CancellationToken>,
    /// By job id.
    pub running_jobs: Mutex<HashMap<String, RunningJob>>,
    /// The turns the local app was last told about, by job id: `turns_changed` tells it what
    /// changed since.
    announced_turns: Mutex<std::collections::BTreeMap<String, LiveTurn>>,
    /// The direct-chat agent loop that currently owns each chat lock: `(job id, queue)`.
    #[cfg(feature = "runner")]
    pub steering_queues: Mutex<HashMap<String, (String, lorca_agent::AgentMessageQueue)>>,
    pub chat_locks: Mutex<HashMap<String, Arc<tokio::sync::Mutex<()>>>>,
    /// The chat on screen in the local app while it is frontmost; a reply there needs no push.
    pub watched_chat: Mutex<Option<String>>,
    /// Jobs sent to other Runners, waiting for their `job_result`.
    pub pending_results: Mutex<HashMap<String, tokio::sync::oneshot::Sender<String>>>,
    /// Requests sent to other Runners, waiting for their `response`.
    pub pending_responses: Mutex<HashMap<String, tokio::sync::oneshot::Sender<Response>>>,
    /// Questions waiting for the user's answer, by the id of the row that asks: its chat, and
    /// where the answer goes.
    #[cfg(feature = "runner")]
    pub pending_permissions: Mutex<HashMap<String, (String, tokio::sync::oneshot::Sender<crate::plugins::mcp::Decision>)>>,
    /// Commands `bash` left running in their terminals, waiting for input.
    #[cfg(feature = "runner")]
    pub shell_sessions: crate::shell::Sessions,
    /// What this Runner has installed, with the secrets kept apart.
    pub plugins: Mutex<crate::plugins::Store>,
    /// The index fetched from `marketplace_url`: (fetched at, index).
    pub marketplace_cache: Mutex<Option<(f64, crate::marketplace::Index)>>,
    /// Connected MCP servers.
    #[cfg(feature = "runner")]
    pub mcp: crate::plugins::mcp::Pool,
    pub http: reqwest::Client,
}

impl App {
    pub fn load(config: Config) -> anyhow::Result<Arc<App>> {
        config.ensure_home()?;
        // This storage layout intentionally has no migration path. Remove the two superseded
        // local stores so plaintext account data is not left behind beside the database.
        for name in ["state.json", "transcript.sqlite3", "transcript.sqlite3-wal", "transcript.sqlite3-shm"] {
            let path = config.home.join(name);
            if path.is_file() {
                std::fs::remove_file(path)?;
            }
        }
        let settings = Settings::load(&config);
        let identity: Option<IdentityFile> = config::read_json(&config.identity_path());
        let machine: Option<MachineFile> = config::read_json(&config.machine_path());
        let credentials = Credentials::load(&config);
        let plugins = crate::plugins::Store::load(&config);
        let store = LocalStore::open(&config.database_path())?;
        let mut state = store.load_state()?;
        for bot in &mut state.bots {
            bot.normalize_description();
        }
        let chat_ids: Vec<String> = state.chats.iter().map(|chat| chat.meta.id.clone()).collect();
        store.retain_chats(&chat_ids)?;
        // Upload this Device's metadata once per launch: a relay that changed or was reset
        // since the last upload has no copy, and every newly paired Device needs one.
        state.machine_blob_hash = None;
        let (events, _) = broadcast::channel(512);
        let http = reqwest::Client::builder().timeout(std::time::Duration::from_secs(60)).build()?;

        let app = Arc::new(App {
            config,
            settings: Mutex::new(settings),
            identity: Mutex::new(identity),
            machine: Mutex::new(machine),
            credentials: Mutex::new(credentials),
            state: Mutex::new(state),
            store,
            events,
            relay: RelayClient::new()?,
            outbox_notify: Notify::new(),
            sync_wakes: AtomicU64::new(0),
            message_order: Mutex::new(()),
            relay_connected: AtomicBool::new(false),
            relay_update_required: AtomicBool::new(false),
            presence_stale: AtomicBool::new(false),
            bulk_sync: AtomicBool::new(false),
            pairings: Mutex::new(HashMap::new()),
            accepting: Mutex::new(None),
            #[cfg(feature = "provider-auth")]
            provider_auth: Mutex::new(CancellationToken::new()),
            running_jobs: Mutex::new(HashMap::new()),
            announced_turns: Mutex::new(std::collections::BTreeMap::new()),
            #[cfg(feature = "runner")]
            steering_queues: Mutex::new(HashMap::new()),
            chat_locks: Mutex::new(HashMap::new()),
            watched_chat: Mutex::new(None),
            pending_results: Mutex::new(HashMap::new()),
            pending_responses: Mutex::new(HashMap::new()),
            #[cfg(feature = "runner")]
            pending_permissions: Mutex::new(HashMap::new()),
            #[cfg(feature = "runner")]
            shell_sessions: crate::shell::Sessions::default(),
            plugins: Mutex::new(plugins),
            marketplace_cache: Mutex::new(None),
            #[cfg(feature = "runner")]
            mcp: crate::plugins::mcp::Pool::new(),
            http,
        });
        // Normalize and persist the in-memory view before background work begins.
        app.save_state_now();
        Ok(app)
    }

    // MARK: - Persistence

    pub fn save_state(&self) {
        // A backlog is saved once at the end of the page, not once per message.
        if self.bulk_sync.load(Ordering::Relaxed) {
            return;
        }
        self.save_state_now();
    }

    pub fn save_state_now(&self) {
        let snapshot = self.state.lock().unwrap().clone();
        if let Err(error) = self.store.save_state(&snapshot) {
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

    /// Starts one provider browser sign-in, cancelling a previous wait if there was one.
    #[cfg(feature = "provider-auth")]
    pub fn begin_provider_auth(&self) -> CancellationToken {
        let next = CancellationToken::new();
        let previous = std::mem::replace(&mut *self.provider_auth.lock().unwrap(), next.clone());
        previous.cancel();
        next
    }

    #[cfg(feature = "provider-auth")]
    pub fn cancel_provider_auth(&self) {
        self.provider_auth.lock().unwrap().cancel();
    }

    /// Changes the account's credential of `kind`: saved here, sent to the other Devices, and
    /// shown by the apps.
    pub fn update_credentials(&self, kind: &str, update: impl FnOnce(&mut Credentials)) -> anyhow::Result<()> {
        {
            let mut credentials = self.credentials.lock().unwrap();
            update(&mut credentials);
            credentials.touch(kind);
        }
        self.save_credentials()?;
        self.push_credentials();
        self.emit(self.roster_summary());
        Ok(())
    }

    /// Queues the account's credentials for the other Devices.
    pub fn push_credentials(&self) {
        let Some(dek) = self.dek() else { return };
        let credentials = self.credentials.lock().unwrap().clone();
        match crate::crypto::encrypt_json(&dek, "credentials", &credentials) {
            Ok(ciphertext) => {
                self.state.lock().unwrap().credentials_uploaded = true;
                self.push_slot_blob("credentials", Slot::latest("credentials"), None, ciphertext);
            }
            Err(error) => tracing::error!(%error, "encrypting credentials"),
        }
    }

    /// Sends the credentials this Device holds when the relay never had them: the ones
    /// connected here before the account carried them, or a relay that was reset. Called once
    /// a pull has merged what the relay holds, so an older set never replaces a newer one.
    pub fn push_credentials_if_owed(&self) {
        let is_owed = !self.state.lock().unwrap().credentials_uploaded && !self.credentials.lock().unwrap().is_empty();
        if is_owed {
            self.push_credentials();
        }
    }

    /// Another Device's credentials arrived: the later change of each kind wins, and a set
    /// that lacks a change made here gets this Device's in return.
    pub fn apply_credentials(&self, incoming: &Credentials) {
        let merge = self.credentials.lock().unwrap().merge(incoming);
        if !merge.taken.is_empty() {
            if let Err(error) = self.save_credentials() {
                tracing::error!(%error, "saving credentials");
            }
            self.emit(self.roster_summary());
        }
        if merge.is_ahead {
            self.push_credentials();
        }
    }

    pub fn emit(&self, event: Event) {
        if self.bulk_sync.load(Ordering::Relaxed)
            && matches!(
                event,
                Event::MessageAdded { .. }
                    | Event::MessageUpdated { .. }
                    | Event::MessageRemoved { .. }
                    | Event::ChatRemoved { .. }
                    | Event::RosterChanged { .. }
                    | Event::ChatUsageChanged { .. }
            )
        {
            // The snapshot at the end of the page carries all of this at once.
            return;
        }
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

    /// Settings, then `LORCA_RELAY_URL`, then the URL pairing handed this Device, then, in
    /// dev, the relay the dev loop runs on this machine, then `LORCA_DEFAULT_RELAY_URL`, the
    /// relay the app that launched this CLI ships with.
    pub fn relay_url(&self) -> Option<String> {
        let from_settings = self.settings.lock().unwrap().effective_relay_url();
        from_settings
            .or_else(|| std::env::var("LORCA_RELAY_URL").ok().filter(|s| !s.is_empty()))
            .or_else(|| self.machine_file().and_then(|m| m.relay_url))
            .or_else(config::dev_relay_url)
            .or_else(config::default_relay_url)
    }

    pub fn set_relay_url(&self, url: Option<String>) -> anyhow::Result<()> {
        let mut settings = self.settings.lock().unwrap();
        settings.relay_url = url.map(|u| u.trim().trim_end_matches('/').to_string()).filter(|u| !u.is_empty());
        settings.save(&self.config)?;
        drop(settings);
        self.relay.forget_token();
        // Another relay may serve this build; the sync loop wakes and finds out.
        if self.relay_update_required.swap(false, Ordering::Relaxed) {
            self.emit_relay_status();
        }
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
        for job in self.running_jobs.lock().unwrap().values() {
            job.cancel.cancel();
        }
        #[cfg(feature = "provider-auth")]
        self.cancel_provider_auth();
        #[cfg(feature = "runner")]
        self.steering_queues.lock().unwrap().clear();
        *self.identity.lock().unwrap() = None;
        *self.machine.lock().unwrap() = None;
        *self.credentials.lock().unwrap() = Credentials::default();
        *self.state.lock().unwrap() = State::default();
        self.store.clear()?;
        self.settings.lock().unwrap().relay_url = None;
        self.relay.forget_token();
        // The sync session ends on this instead of waiting for its socket to say something.
        self.outbox_notify.notify_waiters();
        for path in [self.config.identity_path(), self.config.machine_path(), self.config.credentials_path(), self.config.settings_path()] {
            if path.exists() {
                std::fs::remove_file(&path)?;
            }
        }
        let files = self.config.files_dir();
        if files.is_dir() {
            std::fs::remove_dir_all(&files)?;
        }
        // The other Devices' turns went with the account.
        self.turns_changed();
        self.emit(Event::IdentityChanged { has_identity: false });
        self.emit(Event::Snapshot(self.snapshot()));
        Ok(())
    }

    /// This Device as the roster sees it.
    pub fn local_device(&self) -> Option<Device> {
        let machine = self.machine_file()?;
        let keys = machine.machine().ok()?;
        // The model and the OS version are this host's as it is now, so an OS update reaches the
        // roster; the name is the user's to change, and `os` decided the Device's role at pairing.
        let (_, _, os_version, model) = host_facts();
        Some(Device {
            id: keys.pubkey(),
            name: machine.name.clone(),
            model,
            os: machine.os.clone(),
            os_version,
            box_pubkey: keys.box_pubkey(),
            plugins: self.plugins.lock().unwrap().statuses(),
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
        self.queue_blob(OutboxItem { id: id.clone(), kind: kind.to_string(), recipient, ciphertext, slot: None, group: None });
        id
    }

    /// Queues a `file` blob under its attachment's id. A message's attachment goes with its
    /// chat; a bot's avatar belongs to none.
    pub fn push_file_blob(&self, id: String, chat_id: Option<&str>, ciphertext: Vec<u8>) {
        let group = chat_id.map(crate::model::relay_name);
        self.queue_blob(OutboxItem { id, kind: "file".into(), recipient: None, ciphertext, slot: None, group });
    }

    /// Queues a version of `slot`. A version still waiting in the outbox gives way to this
    /// one, in its place in the queue, so a Device that was offline uploads each message once.
    pub fn push_slot_blob(&self, kind: &str, slot: Slot, group: Option<String>, ciphertext: Vec<u8>) {
        let id = uuid::Uuid::new_v4().to_string();
        self.queue_blob(OutboxItem { id, kind: kind.to_string(), recipient: None, ciphertext, slot: Some(slot), group });
    }

    fn queue_blob(&self, item: OutboxItem) {
        let snapshot = {
            let mut state = self.state.lock().unwrap();
            remember_applied(&mut state, &item.id);
            state.clone()
        };
        if let Err(error) = self.store.queue_outbox_with_state(&item, &snapshot) {
            self.state.lock().unwrap().applied_blob_ids.retain(|id| id != &item.id);
            tracing::error!(%error, kind = %item.kind, "queueing relay blob");
            return;
        }
        self.outbox_notify.notify_waiters();
    }

    pub fn push_roster(&self) {
        let Some(dek) = self.dek() else { return };
        let roster = {
            let state = self.state.lock().unwrap();
            RosterBlob {
                bots: state.bots.clone(),
                chats: state.chats.iter().map(|c| c.meta.clone()).collect(),
                routines: state.routines.clone(),
                auto_review: state.auto_review.clone(),
                updated_at: config::now_secs(),
            }
        };
        match crate::crypto::encrypt_json(&dek, "roster", &roster) {
            Ok(ciphertext) => {
                self.push_slot_blob("roster", Slot::latest("roster"), None, ciphertext);
            }
            Err(error) => tracing::error!(%error, "encrypting roster"),
        }
    }

    pub fn push_chat_op(&self, op: &ChatBlob) {
        let Some(dek) = self.dek() else { return };
        match crate::crypto::encrypt_json(&dek, "chat", op) {
            Ok(ciphertext) => {
                self.push_slot_blob("chat", op.slot(), Some(op.group()), ciphertext);
            }
            Err(error) => tracing::error!(%error, "encrypting chat op"),
        }
    }

    /// Uploads this Device's metadata, with the turns in flight on it, when either changed
    /// since the last upload.
    pub fn push_machine_blob_if_changed(&self) {
        let (Some(dek), Some(device)) = (self.dek(), self.local_device()) else { return };
        let turns = self.turns_here();
        let fingerprint = format!(
            "{}|{}|{}|{}|{}|{}|{}",
            device.id,
            device.name,
            device.model,
            device.os,
            device.os_version,
            serde_json::to_string(&device.plugins).unwrap_or_default(),
            serde_json::to_string(&turns).unwrap_or_default()
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
        let device_id = device.id.clone();
        match crate::crypto::encrypt_json(&dek, "machine", &MachineBlob { device, turns }) {
            Ok(ciphertext) => {
                self.push_slot_blob("machine", Slot::latest(format!("machine-{}", device_id)), None, ciphertext);
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

    pub fn routine(&self, id: &str) -> Option<Routine> {
        self.state.lock().unwrap().routines.iter().find(|r| r.id == id).cloned()
    }

    /// A bot's routines, oldest first.
    pub fn routines_of(&self, bot_id: &str) -> Vec<Routine> {
        let mut routines: Vec<Routine> = self.state.lock().unwrap().routines.iter().filter(|r| r.bot_id == bot_id).cloned().collect();
        routines.sort_by(|a, b| a.created_at.partial_cmp(&b.created_at).unwrap_or(std::cmp::Ordering::Equal));
        routines
    }

    /// The routine a running job belongs to, if any.
    pub fn is_routine_running(&self, routine_id: &str) -> bool {
        self.running_jobs.lock().unwrap().values().any(|job| job.routine_id.as_deref() == Some(routine_id))
    }

    pub fn device_is_online(&self, id: &str) -> bool {
        if self.this_device_id().as_deref() == Some(id) {
            return true;
        }
        let state = self.state.lock().unwrap();
        state.device_online.contains(id)
    }

    pub fn chat_lock(&self, chat_id: &str) -> Arc<tokio::sync::Mutex<()>> {
        self.chat_locks.lock().unwrap().entry(chat_id.to_string()).or_default().clone()
    }

    /// Every turn in flight this Device knows of, for the apps' working indicators; a group
    /// exchange between member turns has an empty bot id.
    pub fn running_turns(&self) -> Vec<Value> {
        self.known_turns()
            .into_values()
            .map(|turn| json!({ "job_id": turn.job_id, "chat_id": turn.chat_id, "bot_id": turn.bot_id, "routine_id": turn.routine_id }))
            .collect()
    }

    pub fn running_chat_ids(&self) -> Vec<String> {
        let mut ids: Vec<String> = self.known_turns().into_values().map(|turn| turn.chat_id).collect();
        ids.sort();
        ids.dedup();
        ids
    }

    /// The turns in flight on this Device, as its machine blob lists them: the turns it runs
    /// and the group exchanges it holds. A job it sent to another Runner is that Runner's to
    /// list.
    pub fn turns_here(&self) -> Vec<LiveTurn> {
        let mut turns: Vec<LiveTurn> =
            self.running_jobs.lock().unwrap().iter().filter(|(_, job)| job.runner_id.is_none()).map(|(id, job)| job.turn(id)).collect();
        turns.sort_by(|a, b| a.job_id.cmp(&b.job_id));
        turns
    }

    /// Every turn in flight this Device knows of, by job id: what the other Devices list while
    /// the relay last saw them online, then this Device's own jobs, including one it sent to a
    /// Runner that has not listed it yet.
    fn known_turns(&self) -> std::collections::BTreeMap<String, LiveTurn> {
        let mut turns = std::collections::BTreeMap::new();
        {
            let state = self.state.lock().unwrap();
            for (device_id, listed) in &state.device_turns {
                if state.turns_online.contains(device_id) {
                    for turn in listed {
                        turns.insert(turn.job_id.clone(), turn.clone());
                    }
                }
            }
        }
        for (id, job) in self.running_jobs.lock().unwrap().iter() {
            turns.entry(id.clone()).or_insert_with(|| job.turn(id));
        }
        turns
    }

    /// Tells the local app which turns started and ended since it last heard, and what each is
    /// doing. Every change to the turns this Device knows of ends here; a backlog is told once,
    /// after its snapshot. Call it with no state or job lock held.
    pub fn turns_changed(&self) {
        if self.bulk_sync.load(Ordering::Relaxed) {
            return;
        }
        let mut announced = self.announced_turns.lock().unwrap();
        let known = self.known_turns();
        for (id, turn) in announced.iter() {
            if !known.contains_key(id) {
                self.emit(Event::JobFinished { chat_id: turn.chat_id.clone(), bot_id: turn.bot_id.clone(), job_id: id.clone(), routine_id: turn.routine_id.clone() });
            }
        }
        for (id, turn) in &known {
            let before = announced.get(id);
            if before.is_none() {
                self.emit(Event::JobStarted { chat_id: turn.chat_id.clone(), bot_id: turn.bot_id.clone(), job_id: id.clone(), routine_id: turn.routine_id.clone() });
            }
            if let Some(activity) = &turn.activity {
                if before.and_then(|seen| seen.activity.as_ref()) != Some(activity) {
                    self.emit(activity.event(&turn.chat_id, &turn.bot_id));
                }
            }
        }
        *announced = known;
    }

    /// A turn here started, ended, or changed what it is doing: the other Devices hear it
    /// through this Device's machine blob, and the local app through `turns_changed`.
    pub fn local_turns_changed(&self) {
        self.push_machine_blob_if_changed();
        self.turns_changed();
    }

    /// What a turn running here is doing between its messages.
    pub fn set_job_activity(&self, job_id: &str, activity: JobActivity) {
        let changed = match self.running_jobs.lock().unwrap().get_mut(job_id) {
            Some(job) if job.activity.as_ref() != Some(&activity) => {
                job.activity = Some(activity);
                true
            }
            _ => false,
        };
        if changed {
            self.local_turns_changed();
        }
    }

    /// A bot's new message says what its turn was doing, so whatever the turn reported before
    /// it is over, as it is in the apps.
    fn end_turn_activity(&self, chat_id: &str, bot_id: &str) {
        let changed = self.running_jobs.lock().unwrap().values_mut().fold(false, |changed, job| {
            let ends = job.runner_id.is_none() && job.chat_id == chat_id && job.bot_id == bot_id && job.activity.is_some();
            if ends {
                job.activity = None;
            }
            changed || ends
        });
        if changed {
            self.local_turns_changed();
        }
    }

    /// The turns another Device's latest machine blob lists.
    pub fn set_device_turns(&self, device_id: &str, turns: Vec<LiveTurn>) {
        {
            let mut state = self.state.lock().unwrap();
            if state.device_turns.get(device_id).map(Vec::as_slice).unwrap_or_default() == turns.as_slice() {
                return;
            }
            if turns.is_empty() {
                state.device_turns.remove(device_id);
            } else {
                state.device_turns.insert(device_id.to_string(), turns.clone());
            }
        }
        if let Err(error) = self.store.set_device_turns(device_id, &turns) {
            tracing::error!(%error, "keeping another Device's turns");
        }
        self.turns_changed();
    }

    // MARK: - Roster mutations (local + roster upload)

    pub fn roster_summary(&self) -> Event {
        let state = self.state.lock().unwrap();
        Event::RosterChanged {
            devices: self.devices_out(&state),
            bots: state.bots.clone(),
            chats: state.chats.iter().map(|c| ChatSummary { meta: c.meta.clone(), unread_count: c.unread_count, usage: c.usage.clone() }).collect(),
            routines: self.routines_out(&state),
            auto_review: state.auto_review.clone(),
            providers: self.credentials.lock().unwrap().statuses(),
        }
    }

    pub fn auto_review(&self) -> AutoReview {
        self.state.lock().unwrap().auto_review.clone()
    }

    /// Replaces the Auto-review setting and publishes the roster.
    pub fn set_auto_review(&self, auto_review: AutoReview) {
        self.state.lock().unwrap().auto_review = auto_review;
        self.roster_changed(true);
    }

    /// Adds a rule, replacing one for the same exact tool or with the same words.
    pub fn add_auto_review_rule(&self, rule: AutoReviewRule) {
        {
            let mut state = self.state.lock().unwrap();
            state.auto_review.rules.retain(|r| match &rule.tool {
                Some(tool) => r.tool.as_deref() != Some(tool.as_str()),
                None => r.tool.is_some() || !r.text.eq_ignore_ascii_case(&rule.text),
            });
            state.auto_review.rules.push(rule);
        }
        self.roster_changed(true);
    }

    /// Asks the relay again now: a phone came back to the foreground, or pulled to refresh.
    /// Its connections may have died while the app was suspended, so the next requests open
    /// new ones, and the sync loop checks the socket and opens a new one at once when it did
    /// (`sync::run`). The bearer lasts an hour and outlives a suspension, so none of that waits
    /// on signing in again.
    pub fn wake_sync(&self) {
        self.relay.reset_connections();
        self.sync_wakes.fetch_add(1, Ordering::Relaxed);
        self.outbox_notify.notify_waiters();
    }

    /// Tells the app where the relay connection stands.
    pub fn emit_relay_status(&self) {
        self.emit(Event::RelayStatus {
            connected: self.relay_connected.load(Ordering::Relaxed),
            url: self.relay_url(),
            update_required: self.relay_update_required.load(Ordering::Relaxed),
        });
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
            let old_avatar = bot.avatar.as_ref().map(|avatar| avatar.id.clone());
            update(bot);
            bot.normalize_description();
            let bot = bot.clone();
            if let Some(old) = old_avatar.filter(|old| bot.avatar.as_ref().map(|avatar| &avatar.id) != Some(old)) {
                self.drop_avatar(&mut state, &old);
            }
            bot
        };
        self.roster_changed(true);
        crate::runtime::prime_names(self);
        Ok(bot)
    }

    /// Deletes a bot, its direct chat and routines, and its memberships in group chats. A
    /// group whose last bot was deleted goes with it; the other groups keep their transcript.
    pub fn delete_bot(&self, id: &str) -> anyhow::Result<()> {
        let removed_chat_ids = {
            let mut state = self.state.lock().unwrap();
            let deleted_bot = state.bots.iter().find(|bot| bot.id == id).cloned().ok_or_else(|| anyhow::anyhow!("Unknown bot"))?;

            state.bots.retain(|bot| bot.id != id);
            state.routines.retain(|routine| routine.bot_id != id);
            if let Some(avatar) = &deleted_bot.avatar {
                self.drop_avatar(&mut state, &avatar.id);
            }

            let mut removed = Vec::new();
            for chat in &mut state.chats {
                if !chat.meta.bot_ids.iter().any(|bot_id| bot_id == id) {
                    continue;
                }
                // A DM belongs to its bot. Groups survive with their remaining members, and
                // move ownership when the deleted bot held it.
                if !chat.meta.is_group() {
                    removed.push(chat.meta.id.clone());
                    continue;
                }
                chat.meta.bot_ids.retain(|bot_id| bot_id != id);
                chat.compactions.retain(|compaction| compaction.bot_id != id);
                if chat.meta.owner_bot_id.as_deref() == Some(id) {
                    chat.meta.owner_bot_id = chat.meta.bot_ids.first().cloned();
                }
                if chat.meta.bot_ids.is_empty() {
                    removed.push(chat.meta.id.clone());
                }
            }

            state.chats.retain(|chat| !removed.contains(&chat.meta.id));
            queue_chat_deletes(&mut state, &removed);
            removed
        };
        let snapshot = self.state.lock().unwrap().clone();
        self.store.save_state_deleting_chats(&snapshot, &removed_chat_ids)?;

        // Stop this bot after the roster mutation is committed. A room job has no bot id and
        // keeps going when its group survives; it reads the changed membership before offering
        // another member a turn. A room whose last bot was deleted is cancelled with its chat.
        for job in self.running_jobs.lock().unwrap().values() {
            if job.bot_id == id || removed_chat_ids.contains(&job.chat_id) {
                job.cancel.cancel();
            }
        }
        for chat_id in &removed_chat_ids {
            self.emit(Event::ChatRemoved { chat_id: chat_id.clone() });
        }
        #[cfg(feature = "runner")]
        self.shell_sessions.close_orphans(self);
        self.roster_changed(true);
        crate::runtime::prime_names(self);
        Ok(())
    }

    /// An avatar no bot shows any more: its local copy goes, and its `file` blob is queued for
    /// deletion on the relay. It belongs to no group, so no chat deletion takes it along.
    fn drop_avatar(&self, state: &mut State, attachment_id: &str) {
        let _ = std::fs::remove_file(self.config.files_dir().join(attachment_id));
        if !state.blob_deletes.iter().any(|id| id == attachment_id) {
            state.blob_deletes.push(attachment_id.to_string());
        }
    }

    fn insert_bot(&self, mut bot: Bot) -> anyhow::Result<Bot> {
        bot.normalize_description();
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
            meta.title = None;
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
        let chat = Chat { meta, unread_count: 0, usage: None, compactions: Vec::new() };
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
            // The relay drops the chat's messages, read marks, and attachments in one call,
            // made by the sync cycle and retried until it lands. What was still waiting to go
            // up goes nowhere.
            queue_chat_deletes(&mut state, &[chat_id.to_string()]);
        }
        let snapshot = self.state.lock().unwrap().clone();
        if let Err(error) = self.store.save_state_deleting_chats(&snapshot, &[chat_id.to_string()]) {
            tracing::error!(%error, %chat_id, "deleting chat state");
        }
        self.emit(Event::ChatRemoved { chat_id: chat_id.to_string() });
        #[cfg(feature = "runner")]
        self.shell_sessions.close_orphans(self);
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

    /// Sets the optional title of a group. A direct chat is always named after its one bot.
    pub fn rename_chat(&self, chat_id: &str, title: Option<String>) -> anyhow::Result<()> {
        {
            let mut state = self.state.lock().unwrap();
            let chat = state.chats.iter_mut().find(|c| c.meta.id == chat_id).ok_or_else(|| anyhow::anyhow!("Unknown chat"))?;
            if !chat.meta.is_group() {
                anyhow::bail!("Only group chats can be renamed");
            }
            chat.meta.title = title;
        }
        self.roster_changed(true);
        Ok(())
    }

    // MARK: - Routines

    /// Adds a routine and publishes the roster.
    pub fn insert_routine(&self, mut routine: Routine) -> anyhow::Result<Routine> {
        if self.bot(&routine.bot_id).is_none() {
            anyhow::bail!("Unknown bot");
        }
        if routine.id.is_empty() {
            routine.id = format!("rt-{}", &uuid::Uuid::new_v4().to_string()[..8]);
        }
        let now = config::now_secs();
        if routine.created_at == 0.0 {
            routine.created_at = now;
        }
        if routine.enabled_at == 0.0 {
            routine.enabled_at = now;
        }
        {
            let mut state = self.state.lock().unwrap();
            if state.routines.iter().any(|r| r.id == routine.id) {
                anyhow::bail!("Routine id already exists");
            }
            state.routines.push(routine.clone());
        }
        self.roster_changed(true);
        Ok(routine)
    }

    /// Changes a routine and publishes the roster.
    pub fn update_routine(&self, id: &str, update: impl FnOnce(&mut Routine)) -> anyhow::Result<Routine> {
        let routine = {
            let mut state = self.state.lock().unwrap();
            let routine = state.routines.iter_mut().find(|r| r.id == id).ok_or_else(|| anyhow::anyhow!("Unknown routine"))?;
            update(routine);
            routine.clone()
        };
        self.roster_changed(true);
        Ok(routine)
    }

    pub fn delete_routine(&self, id: &str) -> anyhow::Result<()> {
        {
            let mut state = self.state.lock().unwrap();
            let before = state.routines.len();
            state.routines.retain(|r| r.id != id);
            if state.routines.len() == before {
                anyhow::bail!("Unknown routine");
            }
        }
        self.roster_changed(true);
        Ok(())
    }

    /// Routines as the apps see them: the stored fields plus the schedule in words, when the
    /// next run is due, and whether a run is going on right now.
    fn routines_out(&self, state: &State) -> Vec<Value> {
        state.routines.iter().map(|routine| self.routine_out(routine)).collect()
    }

    pub fn routine_out(&self, routine: &Routine) -> Value {
        let mut out = serde_json::to_value(routine).unwrap_or_default();
        out["schedule_text"] = json!(crate::schedule::parse(&routine.schedule).map(|s| s.describe()).unwrap_or_else(|_| routine.schedule.clone()));
        out["next_run_at"] = json!(routine.next_run_at().map(|t| t as f64));
        out["is_running"] = json!(self.is_routine_running(&routine.id));
        out
    }

    /// The local app says which chat the user is looking at (`None` when it is not frontmost).
    pub fn set_watched_chat(&self, chat_id: Option<String>) {
        *self.watched_chat.lock().unwrap() = chat_id;
    }

    pub fn is_watching(&self, chat_id: &str) -> bool {
        self.watched_chat.lock().unwrap().as_deref() == Some(chat_id)
    }

    /// Clears a chat's unread count. Read on this Device (`upload`), it clears on every other
    /// one through a `ClearUnread` blob.
    pub fn mark_read(&self, chat_id: &str, upload: bool) {
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
            if upload {
                self.push_chat_op(&ChatBlob::ClearUnread { chat_id: chat_id.to_string() });
            }
        }
    }

    // MARK: - Messages

    /// Insert or replace a message locally, emit, and optionally upload. A reply, failed
    /// response, or pending confirmation counts as unread once, locally or from the relay.
    /// One that finishes while the user looks at its chat is read here, and so everywhere.
    /// A message the user sends, from any Device, says they have read what came before it.
    pub fn upsert_message(&self, message: Message, upload: bool) {
        if !self.state.lock().unwrap().chats.iter().any(|chat| chat.meta.id == message.chat_id) {
            return;
        }
        let _order = self.message_order.lock().unwrap();
        // A phone never runs a bot and never rebuilds model context. Keeping only the app view
        // avoids duplicating large tool arguments and results on every mobile Device.
        #[cfg(feature = "runner")]
        let stored = message.clone();
        #[cfg(not(feature = "runner"))]
        let stored = message.for_app();
        let stored_upsert = match self.store.upsert(&stored) {
            Ok(upsert) => upsert,
            Err(error) => {
                tracing::error!(%error, message_id = %message.id, "storing transcript message");
                return;
            }
        };
        if !stored_upsert.changed {
            return;
        }
        let watching = self.is_watching(&message.chat_id);
        let outcome = {
            let mut state = self.state.lock().unwrap();
            match state.chats.iter_mut().find(|c| c.meta.id == message.chat_id) {
                Some(chat) => {
                    let added = stored_upsert.previous.is_none();
                    let changed = stored_upsert.changed;
                    let counted = stored_upsert.previous.as_ref().is_some_and(Message::counts_unread);
                    let finished = !counted && message.counts_unread();
                    if finished && !watching {
                        chat.unread_count += 1;
                    }
                    let read = added && message.author == Author::You && chat.unread_count > 0;
                    if read {
                        chat.unread_count = 0;
                    }
                    Some((added, changed, finished, read))
                }
                None => None,
            }
        };
        let Some((added, changed, finished, read)) = outcome else {
            // The chat was deleted between the optimistic existence check and the SQLite
            // write. Do not leave an unreachable row behind.
            let _ = self.store.remove(&message.chat_id, &message.id);
            return;
        };
        if added {
            self.emit(Event::MessageAdded { chat_id: message.chat_id.clone(), message: message.for_app() });
        } else if changed {
            self.emit(Event::MessageUpdated { chat_id: message.chat_id.clone(), message: message.for_app() });
        }
        if (finished && !watching) || read {
            self.emit(self.roster_summary());
        }
        if upload {
            self.push_chat_op(&ChatBlob::Upsert { message: message.clone() });
        }
        if let (true, Author::Bot { bot_id }) = (added, &message.author) {
            self.end_turn_activity(&message.chat_id, bot_id);
        }
        if finished && watching {
            self.push_chat_op(&ChatBlob::ClearUnread { chat_id: message.chat_id });
        }
    }

    pub fn remove_message(&self, chat_id: &str, message_id: &str, upload: bool) {
        let _order = self.message_order.lock().unwrap();
        let removed = match self.store.remove(chat_id, message_id) {
            Ok(removed) => removed,
            Err(error) => {
                tracing::error!(%error, %chat_id, %message_id, "removing transcript message");
                false
            }
        };
        if removed {
            self.emit(Event::MessageRemoved { chat_id: chat_id.to_string(), message_id: message_id.to_string() });
            if upload {
                self.push_chat_op(&ChatBlob::Remove { chat_id: chat_id.to_string(), message_id: message_id.to_string() });
            }
        }
    }

    pub fn message(&self, chat_id: &str, message_id: &str) -> Option<Message> {
        self.store.message(chat_id, message_id).map_err(|error| tracing::error!(%error, %chat_id, %message_id, "reading transcript message")).ok().flatten()
    }

    #[cfg(test)]
    pub fn messages(&self, chat_id: &str) -> Vec<Message> {
        self.store.all(chat_id).map_err(|error| tracing::error!(%error, %chat_id, "reading transcript")).unwrap_or_default()
    }

    /// True when the relay holds older messages of this chat than this Device has.
    pub fn history_is_partial(&self, chat_id: &str) -> bool {
        self.store.history_before(chat_id).ok().flatten().is_some()
    }

    pub fn message_page(&self, chat_id: &str, before: Option<&str>, limit: usize) -> (Vec<Message>, bool) {
        match self.store.page(chat_id, before, limit) {
            // More may be on the relay than here: `chats.messages` fetches it when asked.
            Ok((messages, more)) => (messages.into_iter().map(|message| message.for_app()).collect(), more || self.history_is_partial(chat_id)),
            Err(error) => {
                tracing::error!(%error, %chat_id, "paging transcript");
                (Vec::new(), false)
            }
        }
    }

    pub fn notice(&self, chat_id: &str, text: impl Into<String>) {
        let message = Message::new(chat_id, Author::System, Body::Notice { text: text.into(), routine_id: None });
        self.upsert_message(message, true);
    }

    /// Adds a finished turn's usage to the chat's and tells the app.
    #[cfg(feature = "runner")]
    pub fn record_usage(&self, chat_id: &str, model: &str, usage: &lorca_agent::Usage, context_window: u64) {
        let updated = {
            let mut state = self.state.lock().unwrap();
            let Some(chat) = state.chats.iter_mut().find(|c| c.meta.id == chat_id) else { return };
            let entry = chat.usage.get_or_insert_with(ChatUsage::default);
            entry.context_tokens = lorca_agent::estimate::context_tokens(usage);
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

    /// Cancels every active or waiting job in a chat and returns the chat's turns that another
    /// Device runs, with that Device, so the caller can forward the cancellation there too:
    /// jobs this Device sent to another Runner, and turns the other Devices list for the chat.
    pub fn cancel_chat(&self, chat_id: &str) -> Vec<(String, String)> {
        let mut remote = Vec::new();
        for (id, job) in self.running_jobs.lock().unwrap().iter() {
            if job.chat_id == chat_id {
                job.cancel.cancel();
                if let Some(runner_id) = &job.runner_id {
                    remote.push((id.clone(), runner_id.clone()));
                }
            }
        }
        let state = self.state.lock().unwrap();
        for (device_id, turns) in state.device_turns.iter().filter(|(id, _)| state.turns_online.contains(*id)) {
            for turn in turns.iter().filter(|turn| turn.chat_id == chat_id) {
                if !remote.iter().any(|(id, _)| id == &turn.job_id) {
                    remote.push((turn.job_id.clone(), device_id.clone()));
                }
            }
        }
        remote
    }

    /// Cancels one job on this Runner. A cancellation envelope can arrive while the job is
    /// waiting for the chat lock because jobs register before they begin.
    pub fn cancel_job(&self, job_id: &str) {
        if let Some(job) = self.running_jobs.lock().unwrap().get(job_id) {
            job.cancel.cancel();
        }
    }

    /// Marks a user message as consumed by the turn that was already running. Returns false
    /// when another active turn claimed it first.
    pub fn claim_steering_message(&self, chat_id: &str, message_id: &str) -> bool {
        let promoted = self.message(chat_id, message_id).and_then(|mut message| {
            if message.promoted_at.is_some() {
                return None;
            }
            message.promoted_at = Some(config::now_secs());
            Some(message)
        });
        let Some(message) = promoted else { return false };
        if let Err(error) = self.store.upsert(&message) {
            tracing::error!(%error, %chat_id, %message_id, "promoting steering message");
            return false;
        }
        self.push_chat_op(&ChatBlob::Upsert { message });
        true
    }

    /// A replacement user job calls this after it reaches the chat lock. True means an older
    /// turn already handled the message, so this job is only the durable wake-up and may exit.
    /// `promoted_at` preserves that decision across a process restart.
    pub fn take_steering_message(&self, chat_id: &str, message_id: &str) -> bool {
        self.message(chat_id, message_id).is_some_and(|message| message.promoted_at.is_some())
    }

    #[cfg(feature = "runner")]
    pub fn register_steering_queue(&self, chat_id: &str, job_id: &str, queue: lorca_agent::AgentMessageQueue) {
        self.steering_queues.lock().unwrap().insert(chat_id.to_string(), (job_id.to_string(), queue));
    }

    #[cfg(feature = "runner")]
    pub fn unregister_steering_queue(&self, chat_id: &str, job_id: &str) {
        let mut queues = self.steering_queues.lock().unwrap();
        if queues.get(chat_id).is_some_and(|(active, _)| active == job_id) {
            queues.remove(chat_id);
        }
    }

    #[cfg(feature = "runner")]
    pub fn steering_queue(&self, chat_id: &str) -> Option<lorca_agent::AgentMessageQueue> {
        self.steering_queues.lock().unwrap().get(chat_id).map(|(_, queue)| queue.clone())
    }

    // MARK: - Snapshot

    fn devices_out(&self, state: &State) -> Vec<Value> {
        let this_id = self.this_device_id();
        let mut devices: Vec<&Device> = state.devices.iter().collect();
        devices.sort_by_key(|d| (Some(d.id.clone()) != this_id, d.name.to_lowercase()));
        devices
            .into_iter()
            .map(|device| {
                let is_this = Some(device.id.clone()) == this_id;
                let seen = state.device_seen.get(&device.id).copied().unwrap_or(device.updated_at);
                let status = if is_this || state.device_online.contains(&device.id) { "online" } else { "offline" };
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
                    "plugins": device.plugins,
                })
            })
            .collect()
    }

    /// A chat as a snapshot carries it: its newest messages in the apps' form, and `has_more`
    /// when older ones remain in SQLite for `chats.messages` to page.
    fn chat_for_app(&self, chat: &Chat) -> Value {
        let (messages, has_more) = self.message_page(&chat.meta.id, None, SNAPSHOT_MESSAGES);
        let mut out = serde_json::to_value(ChatSummary {
            meta: chat.meta.clone(),
            unread_count: chat.unread_count,
            usage: chat.usage.clone(),
        })
        .unwrap_or_default();
        out["messages"] = json!(messages);
        out["has_more"] = json!(has_more);
        out
    }

    pub fn snapshot(&self) -> Value {
        // Do not hold the metadata lock while paging SQLite: message writes take the locks in
        // the opposite order when they update unread state.
        let state = self.state.lock().unwrap().clone();
        let identity_id = self.machine_file().map(|m| keys::identity_id(&m.identity_pubkey));
        json!({
            "version": config::VERSION,
            "has_identity": self.has_identity(),
            "is_identity_device": self.is_identity_device(),
            "identity_id": identity_id,
            "this_device_id": self.this_device_id(),
            "relay_url": self.relay_url(),
            "relay_connected": self.relay_connected.load(Ordering::Relaxed),
            "relay_update_required": self.relay_update_required.load(Ordering::Relaxed),
            "devices": self.devices_out(&state),
            "bots": state.bots,
            "chats": state.chats.iter().map(|chat| self.chat_for_app(chat)).collect::<Vec<_>>(),
            "routines": self.routines_out(&state),
            "auto_review": state.auto_review,
            "providers": self.credentials.lock().unwrap().statuses(),
            "running_chat_ids": self.running_chat_ids(),
            "running_turns": self.running_turns(),
        })
    }
}

/// Drops everything still waiting to upload for these chats and queues their relay groups for
/// deletion. The sync cycle retries each group until the relay accepts it.
fn queue_chat_deletes(state: &mut State, chat_ids: &[String]) {
    for chat_id in chat_ids {
        let group = crate::model::relay_name(chat_id);
        if !state.group_deletes.contains(&group) {
            state.group_deletes.push(group);
        }
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
    state.applied_blob_ids.retain(|existing| existing != id);
    state.applied_blob_ids.push(id.to_string());
    if state.applied_blob_ids.len() > 2000 {
        let excess = state.applied_blob_ids.len() - 2000;
        state.applied_blob_ids.drain(..excess);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    struct ScratchApp(Arc<App>, std::path::PathBuf);

    impl Drop for ScratchApp {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.1);
        }
    }

    fn scratch_app() -> ScratchApp {
        let home = std::env::temp_dir().join(format!("lorca-app-{}", uuid::Uuid::new_v4()));
        let app = App::load(Config { home: home.clone(), port: 0 }).unwrap();
        ScratchApp(app, home)
    }

    fn bot(id: &str) -> Bot {
        Bot {
            id: id.into(),
            name: id.into(),
            description: String::new(),
            symbol_name: "sparkles".into(),
            accent: "indigo".into(),
            avatar: None,
            runner_id: "runner".into(),
            provider: "deepseek".into(),
            model: None,
            thinking: None,
            legacy_instructions: String::new(),
            workdir: None,
            created_at: 1.0,
        }
    }

    fn chat(id: &str, kind: &str, bot_ids: &[&str], owner: Option<&str>) -> Chat {
        Chat {
            meta: ChatMeta {
                id: id.into(),
                kind: kind.into(),
                title: None,
                bot_ids: bot_ids.iter().map(|id| id.to_string()).collect(),
                owner_bot_id: owner.map(str::to_string),
                is_pinned: false,
                created_at: 1.0,
            },
            unread_count: 0,
            usage: None,
            compactions: Vec::new(),
        }
    }

    fn routine(id: &str, bot_id: &str) -> Routine {
        Routine {
            id: id.into(),
            bot_id: bot_id.into(),
            name: id.into(),
            prompt: String::new(),
            schedule: "every 1h".into(),
            is_enabled: true,
            enabled_at: 1.0,
            last_run_at: None,
            last_outcome: None,
            paused_reason: None,
            created_at: 1.0,
        }
    }

    fn queued_message(chat_id: &str) -> OutboxItem {
        OutboxItem {
            id: format!("out-{chat_id}"),
            kind: "chat".into(),
            recipient: None,
            ciphertext: Vec::new(),
            slot: None,
            group: Some(crate::model::relay_name(chat_id)),
        }
    }

    #[test]
    fn superseded_local_stores_are_discarded() {
        let home = std::env::temp_dir().join(format!("lorca-old-store-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&home).unwrap();
        for name in ["state.json", "transcript.sqlite3", "transcript.sqlite3-wal", "transcript.sqlite3-shm"] {
            std::fs::write(home.join(name), b"obsolete").unwrap();
        }

        let scratch = ScratchApp(App::load(Config { home: home.clone(), port: 0 }).unwrap(), home.clone());

        for name in ["state.json", "transcript.sqlite3", "transcript.sqlite3-wal", "transcript.sqlite3-shm"] {
            assert!(!home.join(name).exists());
        }
        assert!(scratch.0.config.database_path().is_file());
    }

    #[test]
    fn durable_state_and_transcripts_live_in_one_database() {
        let scratch = scratch_app();
        let app = &scratch.0;
        {
            let mut state = app.state.lock().unwrap();
            state.bots.push(bot("b1"));
            state.chats.push(chat("chat", "dm", &["b1"], Some("b1")));
            state.routines.push(routine("routine", "b1"));
            state.last_seq = 42;
            state.group_deletes.push("deleted-chat".into());
            state.blob_deletes.push("old-avatar".into());
            state.machine_blob_hash = Some("machine-hash".into());
            state.credentials_uploaded = true;
            state.device_seen.insert("device".into(), 99);
            state.applied_blob_ids = vec!["blob-1".into(), "blob-2".into()];
        }
        let message = Message::new("chat", Author::You, Body::text("kept in SQLite"));
        let id = message.id.clone();
        app.upsert_message(message, false);
        app.save_state_now();

        assert!(!scratch.1.join("state.json").exists());
        assert!(app.config.database_path().is_file());

        let reloaded = App::load(Config { home: scratch.1.clone(), port: 0 }).unwrap();
        assert!(reloaded.chat("chat").is_some());
        let state = reloaded.state.lock().unwrap();
        assert_eq!(state.bots.iter().map(|bot| bot.id.as_str()).collect::<Vec<_>>(), vec!["b1"]);
        assert_eq!(state.routines.iter().map(|routine| routine.id.as_str()).collect::<Vec<_>>(), vec!["routine"]);
        assert_eq!(state.last_seq, 42);
        assert_eq!(state.group_deletes, vec!["deleted-chat"]);
        assert_eq!(state.blob_deletes, vec!["old-avatar"]);
        assert!(state.credentials_uploaded);
        assert_eq!(state.device_seen.get("device"), Some(&99));
        assert_eq!(state.applied_blob_ids, vec!["blob-1", "blob-2"]);
        drop(state);
        assert_eq!(reloaded.message("chat", &id).and_then(|message| match message.body {
            Body::Text { text, .. } => Some(text),
            _ => None,
        }).as_deref(), Some("kept in SQLite"));
    }

    #[test]
    fn deleting_a_bot_removes_its_dm_routines_and_group_memberships() {
        let scratch = scratch_app();
        let app = &scratch.0;
        {
            let mut state = app.state.lock().unwrap();
            state.bots = vec![bot("b1"), bot("b2")];
            state.chats = vec![
                chat("dm-b1", "dm", &["b1"], Some("b1")),
                chat("dm-b2", "dm", &["b2"], Some("b2")),
                chat("shared", "group", &["b1", "b2"], Some("b1")),
                chat("solo", "group", &["b1"], Some("b1")),
            ];
            state.routines = vec![routine("r1", "b1"), routine("r2", "b2")];
        }
        for chat_id in ["dm-b1", "dm-b2", "shared", "solo"] {
            app.store.queue_outbox(&queued_message(chat_id)).unwrap();
        }

        app.delete_bot("b1").unwrap();

        let state = app.state.lock().unwrap();
        assert_eq!(state.bots.iter().map(|bot| bot.id.as_str()).collect::<Vec<_>>(), vec!["b2"]);
        assert_eq!(state.routines.iter().map(|routine| routine.id.as_str()).collect::<Vec<_>>(), vec!["r2"]);
        assert_eq!(state.chats.iter().map(|chat| chat.meta.id.as_str()).collect::<Vec<_>>(), vec!["dm-b2", "shared"]);
        let shared = state.chats.iter().find(|chat| chat.meta.id == "shared").unwrap();
        assert_eq!(shared.meta.bot_ids, vec!["b2"]);
        assert_eq!(shared.meta.owner_bot_id.as_deref(), Some("b2"));
        assert!(state.group_deletes.contains(&"dm-b1".into()));
        assert!(state.group_deletes.contains(&"solo".into()));
        drop(state);
        assert_eq!(
            app.store.outbox().unwrap().iter().filter_map(|item| item.group.as_deref()).collect::<Vec<_>>(),
            vec!["dm-b2", "shared"]
        );
    }

    #[test]
    fn a_dropped_avatar_is_queued_for_deletion_on_the_relay() {
        let scratch = scratch_app();
        let app = &scratch.0;
        let avatar = |id: &str| crate::model::Attachment { id: id.into(), name: "a.png".into(), mime: "image/png".into(), size: 1, width: None, height: None };
        let mut b1 = bot("b1");
        b1.avatar = Some(avatar("first"));
        app.state.lock().unwrap().bots.push(b1);
        std::fs::create_dir_all(app.config.files_dir()).unwrap();
        for id in ["first", "second"] {
            std::fs::write(app.config.files_dir().join(id), b"png").unwrap();
        }

        app.update_bot("b1", |bot| bot.name = "Renamed".into()).unwrap();
        assert!(app.state.lock().unwrap().blob_deletes.is_empty());

        app.update_bot("b1", |bot| bot.avatar = Some(avatar("second"))).unwrap();
        assert_eq!(app.state.lock().unwrap().blob_deletes, vec!["first"]);
        assert!(!app.config.files_dir().join("first").exists());
        assert!(app.config.files_dir().join("second").exists());

        app.delete_bot("b1").unwrap();
        assert_eq!(app.state.lock().unwrap().blob_deletes, vec!["first", "second"]);
        assert!(!app.config.files_dir().join("second").exists());
    }

    #[test]
    fn deleting_a_group_does_not_delete_its_bots() {
        let scratch = scratch_app();
        let app = &scratch.0;
        {
            let mut state = app.state.lock().unwrap();
            state.bots = vec![bot("b1"), bot("b2")];
            state.chats = vec![
                chat("dm-b1", "dm", &["b1"], Some("b1")),
                chat("group", "group", &["b1", "b2"], Some("b1")),
            ];
            state.routines = vec![routine("r1", "b1")];
        }
        let message = Message::new("group", Author::You, Body::text("remove with the group"));
        let message_id = message.id.clone();
        app.upsert_message(message, false);

        app.delete_chat("group");

        let state = app.state.lock().unwrap();
        assert_eq!(state.bots.len(), 2);
        assert_eq!(state.routines.len(), 1);
        assert_eq!(state.chats.iter().map(|chat| chat.meta.id.as_str()).collect::<Vec<_>>(), vec!["dm-b1"]);
        assert!(state.group_deletes.contains(&"group".into()));
        drop(state);
        assert!(app.message("group", &message_id).is_none());
    }

    #[test]
    fn only_group_chats_can_be_renamed() {
        let scratch = scratch_app();
        let app = &scratch.0;
        {
            let mut state = app.state.lock().unwrap();
            state.chats = vec![
                chat("dm", "dm", &["b1"], Some("b1")),
                chat("group", "group", &["b1"], Some("b1")),
            ];
        }

        assert_eq!(app.rename_chat("dm", Some("Alias".into())).unwrap_err().to_string(), "Only group chats can be renamed");
        app.rename_chat("group", Some("Standup".into())).unwrap();

        let state = app.state.lock().unwrap();
        assert_eq!(state.chats[0].meta.title, None);
        assert_eq!(state.chats[1].meta.title.as_deref(), Some("Standup"));
    }

    #[test]
    fn concurrent_messages_queue_in_transcript_order() {
        let scratch = scratch_app();
        let app = &scratch.0;
        crate::identity::create(app, Some("Workbench".into())).unwrap();
        app.state.lock().unwrap().chats.push(chat("chat", "dm", &["b1"], Some("b1")));

        std::thread::scope(|scope| {
            for thread in 0..8 {
                scope.spawn(move || {
                    for index in 0..40 {
                        let author = if thread % 2 == 0 { Author::You } else { Author::System };
                        let mut message = Message::new("chat", author, Body::text(format!("{thread}-{index}")));
                        app.upsert_message(message.clone(), true);
                        // A streaming update takes the queued version's place.
                        message.body = Body::text(format!("{thread}-{index} done"));
                        app.upsert_message(message, true);
                    }
                });
            }
        });

        let stored: Vec<String> = app.messages("chat").into_iter().map(|message| message.id).collect();
        let queued: Vec<String> = app
            .store
            .outbox()
            .unwrap()
            .into_iter()
            .filter(|item| item.kind == "chat" && item.slot.as_ref().is_some_and(|slot| slot.keep_first))
            .map(|item| item.slot.unwrap().name)
            .collect();
        assert_eq!(stored.len(), 320);
        assert_eq!(queued, stored);
    }
}
