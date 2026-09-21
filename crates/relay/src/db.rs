//! Storage behind one trait, with two backends. SQLite (`db/sqlite.rs`) is one file beside
//! one relay process. Postgres (`db/postgres.rs`) is shared by any number of relay processes,
//! so a deploy can start the new one before the old one stops; what a single process keeps in
//! memory (who is online, whom to signal, which keys were unpaired) goes through the database
//! there.
//!
//! Every SQL statement lives in a backend. `routes.rs` only decides what to ask for.

mod postgres;
mod sqlite;

use std::collections::HashSet;
use std::sync::{Arc, Mutex};

use async_trait::async_trait;
use serde::{Deserialize, Serialize};

use crate::hub::Hub;
use crate::routes::{ApiError, ApiResult};

pub const KINDS: &[&str] = &[
    "roster",
    "chat",
    "job",
    "job_cancel",
    "job_result",
    "request",
    "response",
    "machine",
    "credentials",
    "key",
    "file",
];

/// Kinds sealed to one machine, which deletes what it consumed. One left behind (its Runner
/// never came back) is dropped by `Store::sweep` once it is stale.
pub const SEALED_KINDS: &[&str] = &["job", "job_cancel", "job_result", "request", "response"];

/// `'job', 'job_cancel', …` for an `IN (…)`.
fn sealed_kinds_sql() -> String {
    SEALED_KINDS.iter().map(|kind| format!("'{kind}'")).collect::<Vec<_>>().join(", ")
}

pub fn now() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0)
}

/// Runs CPU- or disk-bound work off the async workers.
pub async fn blocking<T, F>(f: F) -> ApiResult<T>
where
    F: FnOnce() -> ApiResult<T> + Send + 'static,
    T: Send + 'static,
{
    tokio::task::spawn_blocking(f).await.map_err(|_| ApiError::internal("Database task failed"))?
}

// MARK: - Rows

pub struct Machine {
    pub machine_pubkey: String,
    pub identity_pubkey: String,
    pub box_pubkey: String,
    pub last_seen: i64,
    pub created_at: i64,
}

pub struct BlobRow {
    pub id: String,
    pub kind: String,
    pub recipient_machine_pubkey: Option<String>,
    pub seq: i64,
    /// Empty for a `file`: its bytes are in the file store under `store::key`.
    pub ciphertext: Vec<u8>,
    pub created_at: i64,
}

impl BlobRow {
    pub fn in_file_store(&self) -> bool {
        self.kind == "file"
    }
}

pub struct Inserted {
    pub seq: i64,
    pub existing: bool,
}

/// A blob's bytes: in the row, or (a `file`) in the file store with only the size recorded.
pub enum Payload {
    Inline(Vec<u8>),
    InFileStore { size: i64 },
}

impl Payload {
    pub fn size(&self) -> i64 {
        match self {
            Payload::Inline(bytes) => bytes.len() as i64,
            Payload::InFileStore { size } => *size,
        }
    }
}

/// A blob's place among the versions of one thing: a message, the roster, a Device's
/// metadata. A new blob in a slot supersedes the earlier ones, so the log holds the latest
/// version instead of every version. `keep_first` spares the oldest, whose seq holds a
/// message's place in the log for a Device that replays it from the start.
pub struct Slot {
    pub name: String,
    pub keep_first: bool,
}

pub struct NewBlob {
    pub identity_pubkey: String,
    pub id: String,
    pub kind: String,
    pub recipient_machine_pubkey: Option<String>,
    pub slot: Option<Slot>,
    /// What the blob belongs to, a chat to the Devices. A deleted group takes no more blobs.
    pub group: Option<String>,
    pub payload: Payload,
}

/// What a deleted identity leaves for the caller to finish: its machines' tokens and sockets
/// to end, and its `file` objects to remove.
pub struct DeletedIdentity {
    pub machines: Vec<String>,
    pub files: Vec<String>,
}

/// Totals for `/metrics`. The relay reads no content, so these are all it knows.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Stats {
    pub identities: i64,
    pub machines: i64,
    /// Machines seen in the last day, seven days, and thirty days.
    pub active_machines: [i64; 3],
    pub revoked_machines: i64,
    pub deleted_groups: i64,
    /// `(kind, count, bytes)`.
    pub blobs: Vec<(String, i64, i64)>,
    pub usage_bytes: i64,
    pub largest_identity_bytes: i64,
    /// `(platform, count)`.
    pub push_tokens: Vec<(String, i64)>,
}

/// Where a phone takes pushes: its APNs or FCM device token, one per machine.
#[derive(Debug, Clone)]
pub struct PushToken {
    pub machine_pubkey: String,
    pub platform: String,
    pub token: String,
    /// `sandbox` or `production`; APNs keeps a host for each.
    pub environment: String,
}

// MARK: - What one process keeps in memory

/// The keys of unpaired machines, so a bearer token issued before the unpairing dies with
/// it. Loaded from the database at startup and kept current by `Event::Revoked`.
#[derive(Default)]
pub struct Revoked {
    keys: Mutex<HashSet<String>>,
}

impl Revoked {
    pub fn contains(&self, machine_pubkey: &str) -> bool {
        self.lock().contains(machine_pubkey)
    }

    pub fn insert(&self, machine_pubkey: &str) {
        self.lock().insert(machine_pubkey.to_string());
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, HashSet<String>> {
        self.keys.lock().unwrap_or_else(|poisoned| poisoned.into_inner())
    }
}

/// This process's sockets and revoked keys: where an `Event` lands.
#[derive(Default)]
pub struct Local {
    pub hub: Hub,
    pub revoked: Revoked,
}

/// Something every relay process has to hear about. With SQLite there is one process and the
/// event is delivered in place; with Postgres it goes through `NOTIFY` to all of them.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(tag = "t", rename_all = "snake_case")]
pub enum Event {
    /// The identity has a new blob. One sealed to a machine concerns that machine alone.
    Blobs { identity: String, recipient: Option<String> },
    /// The identity's machine list or a machine's presence changed.
    Machines { identity: String },
    /// A machine was unpaired: its tokens die and its sockets close.
    Revoked { identity: String, machine: String },
}

impl Local {
    pub fn deliver(&self, event: &Event) {
        match event {
            Event::Blobs { identity, recipient } => self.hub.blobs(identity, recipient.as_deref()),
            Event::Machines { identity } => self.hub.machines(identity),
            Event::Revoked { identity, machine } => {
                self.revoked.insert(machine);
                self.hub.kick(identity, machine);
                self.hub.machines(identity);
            }
        }
    }
}

// MARK: - The backend

#[async_trait]
pub trait Store: Send + Sync {
    fn describe(&self) -> String;

    // Identities and machines

    /// Registers the identity (idempotent) and attests one machine. `410` for a revoked key,
    /// `409` for an identity known under another content key.
    async fn register_identity(&self, identity_pubkey: &str, content_pubkey: &str, machine_pubkey: &str, box_pubkey: &str, attestation: &str) -> ApiResult<()>;
    /// `404` for an unknown machine, `410` for a revoked one.
    async fn create_challenge(&self, nonce: &str, machine_pubkey: &str, expires_at: i64) -> ApiResult<()>;
    /// Spends the challenge (a nonce is good once), checks it was this machine's and is still
    /// good, writes `last_seen`, and answers with the machine.
    async fn redeem_challenge(&self, nonce: &str, machine_pubkey: &str) -> ApiResult<Machine>;
    async fn machines_for(&self, identity_pubkey: &str) -> ApiResult<Vec<Machine>>;
    async fn touch_machine(&self, machine_pubkey: &str) -> ApiResult<()>;
    /// Unpairs a machine of this identity: its row goes, its key is remembered as revoked, and
    /// the envelopes sealed to it go with it. False when the identity has no such machine.
    async fn revoke_machine(&self, identity_pubkey: &str, machine_pubkey: &str) -> ApiResult<bool>;
    async fn revoked_machines(&self) -> ApiResult<Vec<String>>;
    /// Deletes the identity and everything the relay holds for it. Its machines' keys are
    /// remembered as revoked, so every Device gets `410` and forgets the identity.
    async fn delete_identity(&self, identity_pubkey: &str) -> ApiResult<DeletedIdentity>;

    // Blobs

    /// What `insert_blob` would say before a `file`'s object goes up: the seq of a known id,
    /// or a refusal (deleted group, quota). `None` means go ahead; the insert decides for real.
    async fn precheck_blob(&self, identity_pubkey: &str, id: &str, group: Option<&str>, size: i64, quota_bytes: u64) -> ApiResult<Option<Inserted>>;
    /// Stores a blob under the identity's next sequence number. A known id returns its
    /// existing seq. `quota_bytes` of 0 means unlimited.
    async fn insert_blob(&self, blob: NewBlob, quota_bytes: u64) -> ApiResult<Inserted>;
    /// A page of blobs after `since` that this machine may see (unaddressed ones and its own
    /// envelopes), and the identity's head seq. The page ends at `limit` rows or before the
    /// row that would take it past `max_bytes`, and always holds one row when there is one.
    async fn blobs_since(&self, identity_pubkey: &str, machine_pubkey: &str, since: i64, kinds: &[String], limit: i64, max_bytes: i64) -> ApiResult<(Vec<BlobRow>, i64)>;
    async fn blob(&self, identity_pubkey: &str, machine_pubkey: &str, id: &str) -> ApiResult<Option<BlobRow>>;
    /// Deletes a blob and gives its bytes back to the identity's usage. Returns its kind, or
    /// `None` when there was none; the caller removes a `file`'s object afterwards.
    async fn delete_blob(&self, identity_pubkey: &str, id: &str) -> ApiResult<Option<String>>;
    /// Deletes every blob of a group and marks the group deleted for good. Returns the ids of
    /// the `file` blobs among them; the caller removes their objects afterwards.
    async fn delete_group(&self, identity_pubkey: &str, group: &str) -> ApiResult<Vec<String>>;

    /// Of `(identity, blob id)` pairs found in the file store, those with no row although
    /// their identity is known here. An object of an identity this database never heard of is
    /// not an orphan: the file store may belong to another database.
    async fn orphans(&self, files: &[(String, String)]) -> ApiResult<Vec<(String, String)>>;

    // Push tokens

    async fn set_push_token(&self, identity_pubkey: &str, token: &PushToken) -> ApiResult<()>;
    async fn delete_push_token(&self, machine_pubkey: &str) -> ApiResult<()>;
    /// The identity's tokens, leaving out the machine that asks for the push.
    async fn push_tokens_for(&self, identity_pubkey: &str, except_machine: &str) -> ApiResult<Vec<PushToken>>;

    // Pairing mailbox. A nonce that is unknown or expired is `404`; one that belongs to
    // another identity is `403`.

    async fn create_pairing(&self, nonce: &str, identity_pubkey: &str, expires_at: i64) -> ApiResult<()>;
    async fn delete_pairing(&self, nonce: &str, identity_pubkey: &str) -> ApiResult<()>;
    /// `409` when the pairing already holds a request.
    async fn post_pair_request(&self, nonce: &str, ciphertext: &[u8]) -> ApiResult<()>;
    async fn pair_request(&self, nonce: &str, identity_pubkey: &str) -> ApiResult<Option<Vec<u8>>>;
    async fn post_pair_reply(&self, nonce: &str, identity_pubkey: &str, ciphertext: &[u8]) -> ApiResult<()>;
    async fn pair_reply(&self, nonce: &str) -> ApiResult<Option<Vec<u8>>>;

    /// Housekeeping, once a minute: expired challenges and pairings go, and with Postgres
    /// this process says it is alive and clears the sockets of processes that are not.
    async fn tick(&self) -> ApiResult<()>;
    /// Housekeeping, once an hour: sealed envelopes made before `sealed_before` that nobody
    /// consumed go, with their bytes given back, and so do the marks of groups deleted before
    /// `groups_before`. Returns how many envelopes went.
    async fn sweep(&self, sealed_before: i64, groups_before: i64) -> ApiResult<u64>;

    async fn stats(&self) -> ApiResult<Stats>;

    // Presence and events. `Local` has this process's answer; a shared backend widens it to
    // all of them.

    /// A machine's sync socket opened here. True when it had none anywhere: it came online.
    async fn socket_opened(&self, identity_pubkey: &str, machine_pubkey: &str, socket_id: u64, first_here: bool) -> ApiResult<bool>;
    /// True when that was the machine's last socket anywhere: it went offline.
    async fn socket_closed(&self, identity_pubkey: &str, machine_pubkey: &str, socket_id: u64, last_here: bool) -> ApiResult<bool>;
    async fn online(&self, identity_pubkey: &str) -> ApiResult<HashSet<String>>;
    async fn publish(&self, event: Event);
    /// The process is stopping: its sockets are no longer anyone's presence.
    async fn close(&self);
}

/// `postgres://…` or `postgresql://…` opens Postgres; anything else is a SQLite path.
pub async fn open(location: &str, local: Arc<Local>) -> anyhow::Result<Arc<dyn Store>> {
    let store: Arc<dyn Store> = if location.starts_with("postgres://") || location.starts_with("postgresql://") {
        Arc::new(postgres::Postgres::open(location, local.clone()).await?)
    } else {
        Arc::new(sqlite::Sqlite::open(location, local.clone())?)
    };
    for key in store.revoked_machines().await.map_err(|e| anyhow::anyhow!("{e:?}"))? {
        local.revoked.insert(&key);
    }
    Ok(store)
}

#[cfg(test)]
mod tests;
