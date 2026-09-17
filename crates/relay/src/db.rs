//! Storage: one SQLite file in WAL mode behind a single writer connection and a pool of
//! readers. Every query runs on tokio's blocking pool, so a 24 MB `file` blob never stalls
//! the async workers, and reads run in parallel with each other and with the writer.
//!
//! Every SQL statement lives here. `routes.rs` only decides what to ask for.

use std::collections::HashMap;
use std::sync::{Arc, Mutex, MutexGuard, Weak};

use rusqlite::types::Value;
use rusqlite::{params, params_from_iter, Connection, OptionalExtension, TransactionBehavior};
use tokio::sync::{Notify, Semaphore};

use crate::routes::{ApiError, ApiResult};

pub const KINDS: &[&str] = &["roster", "chat", "job", "job_result", "request", "response", "machine", "key", "file"];

pub fn now() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0)
}

const SCHEMA: &str = "
    PRAGMA journal_mode = WAL;
    PRAGMA synchronous = NORMAL;
    PRAGMA foreign_keys = ON;
    PRAGMA busy_timeout = 5000;
    CREATE TABLE IF NOT EXISTS identities (
        pubkey TEXT PRIMARY KEY,
        content_pubkey TEXT NOT NULL,
        created_at INTEGER NOT NULL
    );
    CREATE TABLE IF NOT EXISTS machines (
        machine_pubkey TEXT PRIMARY KEY,
        identity_pubkey TEXT NOT NULL,
        box_pubkey TEXT NOT NULL,
        attestation TEXT NOT NULL,
        last_seen INTEGER NOT NULL,
        created_at INTEGER NOT NULL
    );
    CREATE INDEX IF NOT EXISTS machines_identity ON machines(identity_pubkey);
    CREATE TABLE IF NOT EXISTS blobs (
        identity_pubkey TEXT NOT NULL,
        id TEXT NOT NULL,
        kind TEXT NOT NULL,
        recipient_machine_pubkey TEXT,
        seq INTEGER NOT NULL,
        ciphertext BLOB NOT NULL,
        size INTEGER NOT NULL,
        created_at INTEGER NOT NULL,
        PRIMARY KEY (identity_pubkey, id)
    );
    CREATE INDEX IF NOT EXISTS blobs_identity_seq ON blobs(identity_pubkey, seq);
    CREATE TABLE IF NOT EXISTS sequences (
        identity_pubkey TEXT PRIMARY KEY,
        seq INTEGER NOT NULL
    );
    CREATE TABLE IF NOT EXISTS usage (
        identity_pubkey TEXT PRIMARY KEY,
        bytes INTEGER NOT NULL
    );
    CREATE TABLE IF NOT EXISTS challenges (
        nonce TEXT PRIMARY KEY,
        machine_pubkey TEXT NOT NULL,
        expires_at INTEGER NOT NULL
    );
    CREATE TABLE IF NOT EXISTS pairings (
        nonce TEXT PRIMARY KEY,
        identity_pubkey TEXT NOT NULL,
        request BLOB,
        reply BLOB,
        created_at INTEGER NOT NULL,
        expires_at INTEGER NOT NULL
    );";

pub struct Db {
    writer: Arc<Mutex<Connection>>,
    readers: Arc<Mutex<Vec<Connection>>>,
    read_permits: Arc<Semaphore>,
}

impl Db {
    pub fn open(path: &str) -> anyhow::Result<Db> {
        let writer = Connection::open(path)?;
        writer.execute_batch(SCHEMA)?;

        let count = std::thread::available_parallelism().map(|n| n.get()).unwrap_or(4).clamp(4, 16);
        let mut readers = Vec::with_capacity(count);
        for _ in 0..count {
            let reader = Connection::open(path)?;
            reader.execute_batch("PRAGMA busy_timeout = 5000; PRAGMA query_only = ON;")?;
            readers.push(reader);
        }
        Ok(Db {
            writer: Arc::new(Mutex::new(writer)),
            readers: Arc::new(Mutex::new(readers)),
            read_permits: Arc::new(Semaphore::new(count)),
        })
    }

    /// Runs `f` on a reader connection from the pool.
    pub async fn read<T, F>(&self, f: F) -> ApiResult<T>
    where
        F: FnOnce(&Connection) -> ApiResult<T> + Send + 'static,
        T: Send + 'static,
    {
        let permit = self.read_permits.clone().acquire_owned().await.map_err(|_| ApiError::internal("Database closed"))?;
        let pool = self.readers.clone();
        blocking(move || {
            let _permit = permit;
            let reader = Reader::take(pool);
            f(&reader)
        })
        .await
    }

    /// Runs `f` on the writer connection. Writes are serialized here, never by SQLite's
    /// busy handler.
    pub async fn write<T, F>(&self, f: F) -> ApiResult<T>
    where
        F: FnOnce(&mut Connection) -> ApiResult<T> + Send + 'static,
        T: Send + 'static,
    {
        let writer = self.writer.clone();
        blocking(move || {
            let mut connection = unpoisoned(&writer);
            f(&mut connection)
        })
        .await
    }
}

/// Runs CPU- or disk-bound work off the async workers.
pub async fn blocking<T, F>(f: F) -> ApiResult<T>
where
    F: FnOnce() -> ApiResult<T> + Send + 'static,
    T: Send + 'static,
{
    tokio::task::spawn_blocking(f).await.map_err(|_| ApiError::internal("Database task failed"))?
}

fn unpoisoned<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(|poisoned| poisoned.into_inner())
}

/// A pooled reader that returns itself to the pool when dropped, even if the query panicked.
struct Reader {
    connection: Option<Connection>,
    pool: Arc<Mutex<Vec<Connection>>>,
}

impl Reader {
    fn take(pool: Arc<Mutex<Vec<Connection>>>) -> Reader {
        let connection = unpoisoned(&pool).pop().expect("one reader per permit");
        Reader { connection: Some(connection), pool }
    }
}

impl std::ops::Deref for Reader {
    type Target = Connection;
    fn deref(&self) -> &Connection {
        self.connection.as_ref().expect("reader held until drop")
    }
}

impl Drop for Reader {
    fn drop(&mut self) {
        if let Some(connection) = self.connection.take() {
            unpoisoned(&self.pool).push(connection);
        }
    }
}

// MARK: - Long-poll wakeups

/// One `Notify` per identity with a long-poll in flight, so a blob write wakes only the
/// machines of the identity it belongs to.
#[derive(Default)]
pub struct Wakers {
    map: Mutex<HashMap<String, Weak<Notify>>>,
}

impl Wakers {
    pub fn waiter(&self, identity_pubkey: &str) -> Arc<Notify> {
        let mut map = unpoisoned(&self.map);
        if let Some(notify) = map.get(identity_pubkey).and_then(Weak::upgrade) {
            return notify;
        }
        if map.len() >= 4096 {
            map.retain(|_, weak| weak.strong_count() > 0);
        }
        let notify = Arc::new(Notify::new());
        map.insert(identity_pubkey.to_string(), Arc::downgrade(&notify));
        notify
    }

    pub fn wake(&self, identity_pubkey: &str) {
        let notify = unpoisoned(&self.map).get(identity_pubkey).and_then(Weak::upgrade);
        if let Some(notify) = notify {
            notify.notify_waiters();
        }
    }
}

// MARK: - Presence

/// The online window is 150 s, so `last_seen` only needs a write every 30 s per machine
/// instead of one on every authenticated request.
pub const TOUCH_INTERVAL: i64 = 30;

#[derive(Default)]
pub struct Presence {
    touched: Mutex<HashMap<String, i64>>,
}

impl Presence {
    /// True when this machine's `last_seen` is due for a write.
    pub fn due(&self, machine_pubkey: &str) -> bool {
        let now = now();
        let mut touched = unpoisoned(&self.touched);
        if touched.get(machine_pubkey).is_some_and(|last| now - last < TOUCH_INTERVAL) {
            return false;
        }
        if touched.len() >= 65_536 {
            touched.retain(|_, last| now - *last < 10 * 60);
        }
        touched.insert(machine_pubkey.to_string(), now);
        true
    }
}

// MARK: - Identities and machines

pub struct Machine {
    pub machine_pubkey: String,
    pub identity_pubkey: String,
    pub box_pubkey: String,
    pub last_seen: i64,
    pub created_at: i64,
}

fn machine_row(row: &rusqlite::Row) -> rusqlite::Result<Machine> {
    Ok(Machine {
        machine_pubkey: row.get(0)?,
        identity_pubkey: row.get(1)?,
        box_pubkey: row.get(2)?,
        last_seen: row.get(3)?,
        created_at: row.get(4)?,
    })
}

pub fn machine(connection: &Connection, machine_pubkey: &str) -> rusqlite::Result<Option<Machine>> {
    connection
        .prepare_cached("SELECT machine_pubkey, identity_pubkey, box_pubkey, last_seen, created_at FROM machines WHERE machine_pubkey = ?1")?
        .query_row(params![machine_pubkey], machine_row)
        .optional()
}

pub fn machines_for(connection: &Connection, identity_pubkey: &str) -> rusqlite::Result<Vec<Machine>> {
    connection
        .prepare_cached(
            "SELECT machine_pubkey, identity_pubkey, box_pubkey, last_seen, created_at FROM machines
             WHERE identity_pubkey = ?1 ORDER BY created_at",
        )?
        .query_map(params![identity_pubkey], machine_row)?
        .collect()
}

pub fn touch_machine(connection: &Connection, machine_pubkey: &str) -> rusqlite::Result<()> {
    connection
        .prepare_cached("UPDATE machines SET last_seen = ?1 WHERE machine_pubkey = ?2")?
        .execute(params![now(), machine_pubkey])?;
    Ok(())
}

/// Registers the identity (idempotent) and attests one machine.
pub fn register_identity(
    connection: &Connection,
    identity_pubkey: &str,
    content_pubkey: &str,
    machine_pubkey: &str,
    box_pubkey: &str,
    attestation: &str,
) -> ApiResult<()> {
    let existing: Option<String> = connection
        .query_row("SELECT content_pubkey FROM identities WHERE pubkey = ?1", params![identity_pubkey], |row| row.get(0))
        .optional()?;
    match existing {
        Some(content) if content != content_pubkey => {
            return Err(ApiError::conflict("Identity exists with a different content key"))
        }
        Some(_) => {}
        None => {
            connection.execute(
                "INSERT INTO identities (pubkey, content_pubkey, created_at) VALUES (?1, ?2, ?3)",
                params![identity_pubkey, content_pubkey, now()],
            )?;
        }
    }
    connection.execute(
        "INSERT INTO machines (machine_pubkey, identity_pubkey, box_pubkey, attestation, last_seen, created_at)
         VALUES (?1, ?2, ?3, ?4, ?5, ?5)
         ON CONFLICT(machine_pubkey) DO UPDATE SET box_pubkey = excluded.box_pubkey, attestation = excluded.attestation",
        params![machine_pubkey, identity_pubkey, box_pubkey, attestation, now()],
    )?;
    Ok(())
}

// MARK: - Auth challenges

pub fn create_challenge(connection: &Connection, nonce: &str, machine_pubkey: &str, expires_at: i64) -> ApiResult<()> {
    if machine(connection, machine_pubkey)?.is_none() {
        return Err(ApiError::not_found("Unknown machine"));
    }
    connection.execute(
        "INSERT INTO challenges (nonce, machine_pubkey, expires_at) VALUES (?1, ?2, ?3)",
        params![nonce, machine_pubkey, expires_at],
    )?;
    Ok(())
}

/// Removes the challenge and returns its machine and expiry; a nonce is good once.
pub fn take_challenge(connection: &Connection, nonce: &str) -> rusqlite::Result<Option<(String, i64)>> {
    connection
        .query_row("DELETE FROM challenges WHERE nonce = ?1 RETURNING machine_pubkey, expires_at", params![nonce], |row| {
            Ok((row.get(0)?, row.get(1)?))
        })
        .optional()
}

// MARK: - Blobs

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

const BLOB_COLUMNS: &str = "id, kind, recipient_machine_pubkey, seq, ciphertext, created_at";

fn blob_row(row: &rusqlite::Row) -> rusqlite::Result<BlobRow> {
    Ok(BlobRow {
        id: row.get(0)?,
        kind: row.get(1)?,
        recipient_machine_pubkey: row.get(2)?,
        seq: row.get(3)?,
        ciphertext: row.get(4)?,
        created_at: row.get(5)?,
    })
}

pub struct Inserted {
    pub seq: i64,
    pub existing: bool,
}

/// The seq a blob id already has under this identity, if any.
pub fn blob_seq(connection: &Connection, identity_pubkey: &str, id: &str) -> rusqlite::Result<Option<i64>> {
    connection
        .prepare_cached("SELECT seq FROM blobs WHERE id = ?1 AND identity_pubkey = ?2")?
        .query_row(params![id, identity_pubkey], |row| row.get(0))
        .optional()
}

pub fn usage(connection: &Connection, identity_pubkey: &str) -> rusqlite::Result<i64> {
    Ok(connection
        .prepare_cached("SELECT bytes FROM usage WHERE identity_pubkey = ?1")?
        .query_row(params![identity_pubkey], |row| row.get(0))
        .optional()?
        .unwrap_or(0))
}

/// A blob's bytes: in the row, or (a `file`) in the file store with only the size recorded.
pub enum Payload<'a> {
    Inline(&'a [u8]),
    InFileStore { size: i64 },
}

/// Stores a blob under the identity's next sequence number. A known id returns its existing
/// seq. `quota_bytes` of 0 means unlimited.
pub fn insert_blob(
    connection: &mut Connection,
    identity_pubkey: &str,
    id: &str,
    kind: &str,
    recipient_machine_pubkey: Option<&str>,
    payload: Payload<'_>,
    quota_bytes: u64,
) -> ApiResult<Inserted> {
    let (ciphertext, size): (&[u8], i64) = match payload {
        Payload::Inline(bytes) => (bytes, bytes.len() as i64),
        Payload::InFileStore { size } => (&[], size),
    };
    let tx = connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
    if let Some(recipient) = recipient_machine_pubkey {
        match machine(&tx, recipient)? {
            Some(machine) if machine.identity_pubkey == identity_pubkey => {}
            _ => return Err(ApiError::bad_request("Recipient is not a machine of this identity")),
        }
    }
    if let Some(seq) = blob_seq(&tx, identity_pubkey, id)? {
        return Ok(Inserted { seq, existing: true });
    }
    if quota_bytes > 0 && (usage(&tx, identity_pubkey)? + size) as u64 > quota_bytes {
        return Err(ApiError::too_large("Storage quota exceeded"));
    }
    tx.prepare_cached(
        "INSERT INTO sequences (identity_pubkey, seq) VALUES (?1, 1)
         ON CONFLICT(identity_pubkey) DO UPDATE SET seq = seq + 1",
    )?
    .execute(params![identity_pubkey])?;
    let seq: i64 = tx
        .prepare_cached("SELECT seq FROM sequences WHERE identity_pubkey = ?1")?
        .query_row(params![identity_pubkey], |row| row.get(0))?;
    tx.prepare_cached(
        "INSERT INTO blobs (id, identity_pubkey, kind, recipient_machine_pubkey, seq, ciphertext, size, created_at)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
    )?
    .execute(params![id, identity_pubkey, kind, recipient_machine_pubkey, seq, ciphertext, size, now()])?;
    tx.prepare_cached(
        "INSERT INTO usage (identity_pubkey, bytes) VALUES (?1, ?2)
         ON CONFLICT(identity_pubkey) DO UPDATE SET bytes = bytes + excluded.bytes",
    )?
    .execute(params![identity_pubkey, size])?;
    tx.commit()?;
    Ok(Inserted { seq, existing: false })
}

pub fn current_seq(connection: &Connection, identity_pubkey: &str) -> rusqlite::Result<i64> {
    Ok(connection
        .prepare_cached("SELECT seq FROM sequences WHERE identity_pubkey = ?1")?
        .query_row(params![identity_pubkey], |row| row.get(0))
        .optional()?
        .unwrap_or(0))
}

/// Blobs after `since` that this machine may see: unaddressed ones and its own envelopes.
/// The kind filter is part of the query, so a poll that leaves `file` out never loads a file.
pub fn blobs_since(
    connection: &Connection,
    identity_pubkey: &str,
    machine_pubkey: &str,
    since: i64,
    kinds: &[String],
    limit: i64,
) -> rusqlite::Result<Vec<BlobRow>> {
    let mut sql = format!(
        "SELECT {BLOB_COLUMNS} FROM blobs
         WHERE identity_pubkey = ?1 AND seq > ?2
           AND (recipient_machine_pubkey IS NULL OR recipient_machine_pubkey = ?3)"
    );
    if !kinds.is_empty() {
        let marks: Vec<String> = (0..kinds.len()).map(|i| format!("?{}", i + 5)).collect();
        sql.push_str(&format!(" AND kind IN ({})", marks.join(", ")));
    }
    sql.push_str(" ORDER BY seq ASC LIMIT ?4");
    let mut values: Vec<Value> =
        vec![identity_pubkey.to_string().into(), since.into(), machine_pubkey.to_string().into(), limit.into()];
    values.extend(kinds.iter().map(|kind| Value::from(kind.clone())));
    connection.prepare_cached(&sql)?.query_map(params_from_iter(values), blob_row)?.collect()
}

pub fn blob(connection: &Connection, identity_pubkey: &str, machine_pubkey: &str, id: &str) -> rusqlite::Result<Option<BlobRow>> {
    connection
        .prepare_cached(&format!(
            "SELECT {BLOB_COLUMNS} FROM blobs
             WHERE id = ?1 AND identity_pubkey = ?2
               AND (recipient_machine_pubkey IS NULL OR recipient_machine_pubkey = ?3)"
        ))?
        .query_row(params![id, identity_pubkey, machine_pubkey], blob_row)
        .optional()
}

/// Deletes a blob's row and gives its bytes back to the identity's usage. Returns the blob's
/// kind, or `None` when there was none; the caller removes a `file`'s object afterwards.
pub fn delete_blob(connection: &mut Connection, identity_pubkey: &str, id: &str) -> rusqlite::Result<Option<String>> {
    let tx = connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
    let row: Option<(i64, String)> = tx
        .prepare_cached("DELETE FROM blobs WHERE id = ?1 AND identity_pubkey = ?2 RETURNING size, kind")?
        .query_row(params![id, identity_pubkey], |row| Ok((row.get(0)?, row.get(1)?)))
        .optional()?;
    let Some((size, kind)) = row else { return Ok(None) };
    tx.prepare_cached("UPDATE usage SET bytes = MAX(bytes - ?1, 0) WHERE identity_pubkey = ?2")?
        .execute(params![size, identity_pubkey])?;
    tx.commit()?;
    Ok(Some(kind))
}

// MARK: - Pairing mailbox

pub fn create_pairing(connection: &Connection, nonce: &str, identity_pubkey: &str, expires_at: i64) -> rusqlite::Result<()> {
    connection.execute(
        "INSERT INTO pairings (nonce, identity_pubkey, created_at, expires_at) VALUES (?1, ?2, ?3, ?4)",
        params![nonce, identity_pubkey, now(), expires_at],
    )?;
    Ok(())
}

/// The identity that opened a live pairing.
pub fn pairing_owner(connection: &Connection, nonce: &str) -> ApiResult<String> {
    let row: Option<(String, i64)> = connection
        .prepare_cached("SELECT identity_pubkey, expires_at FROM pairings WHERE nonce = ?1")?
        .query_row(params![nonce], |row| Ok((row.get(0)?, row.get(1)?)))
        .optional()?;
    match row {
        Some((identity, expires_at)) if expires_at >= now() => Ok(identity),
        _ => Err(ApiError::not_found("Unknown or expired pairing")),
    }
}

/// False when the pairing already holds a request.
pub fn set_pairing_request(connection: &Connection, nonce: &str, ciphertext: &[u8]) -> rusqlite::Result<bool> {
    let changed = connection
        .execute("UPDATE pairings SET request = ?1 WHERE nonce = ?2 AND request IS NULL", params![ciphertext, nonce])?;
    Ok(changed > 0)
}

pub fn pairing_request(connection: &Connection, nonce: &str) -> rusqlite::Result<Option<Vec<u8>>> {
    connection.query_row("SELECT request FROM pairings WHERE nonce = ?1", params![nonce], |row| row.get(0))
}

pub fn set_pairing_reply(connection: &Connection, nonce: &str, ciphertext: &[u8]) -> rusqlite::Result<()> {
    connection.execute("UPDATE pairings SET reply = ?1 WHERE nonce = ?2", params![ciphertext, nonce])?;
    Ok(())
}

pub fn pairing_reply(connection: &Connection, nonce: &str) -> rusqlite::Result<Option<Vec<u8>>> {
    connection.query_row("SELECT reply FROM pairings WHERE nonce = ?1", params![nonce], |row| row.get(0))
}

pub fn delete_pairing(connection: &Connection, nonce: &str) -> rusqlite::Result<()> {
    connection.execute("DELETE FROM pairings WHERE nonce = ?1", params![nonce])?;
    Ok(())
}

/// Drops expired challenges and pairings. A background task runs this once a minute.
pub fn expire(connection: &Connection) -> rusqlite::Result<()> {
    let now = now();
    connection.execute("DELETE FROM challenges WHERE expires_at < ?1", params![now])?;
    connection.execute("DELETE FROM pairings WHERE expires_at < ?1", params![now])?;
    Ok(())
}
