//! SQLite: one file in WAL mode behind a single writer connection and a pool of readers.
//! Every query runs on tokio's blocking pool, so a large blob never stalls the async workers,
//! and reads run in parallel with each other and with the writer. One relay process owns the
//! file, so presence and events are this process's own.

use std::collections::HashSet;
use std::sync::{Arc, Mutex, MutexGuard};

use async_trait::async_trait;
use rusqlite::types::Value;
use rusqlite::{params, params_from_iter, Connection, OptionalExtension, TransactionBehavior};
use tokio::sync::Semaphore;

use super::{blocking, now, sealed_kinds_sql, BlobRow, DeletedIdentity, Event, Inserted, Local, Machine, NewBlob, Payload, PushToken, Slot, Stats, Store};
use crate::routes::{ApiError, ApiResult};

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
    CREATE TABLE IF NOT EXISTS revoked_machines (
        machine_pubkey TEXT PRIMARY KEY,
        identity_pubkey TEXT NOT NULL,
        revoked_at INTEGER NOT NULL
    );
    CREATE TABLE IF NOT EXISTS blobs (
        identity_pubkey TEXT NOT NULL,
        id TEXT NOT NULL,
        kind TEXT NOT NULL,
        recipient_machine_pubkey TEXT,
        seq INTEGER NOT NULL,
        ciphertext BLOB NOT NULL,
        size INTEGER NOT NULL,
        created_at INTEGER NOT NULL,
        slot TEXT,
        group_id TEXT,
        PRIMARY KEY (identity_pubkey, id)
    );
    CREATE TABLE IF NOT EXISTS deleted_groups (
        identity_pubkey TEXT NOT NULL,
        group_id TEXT NOT NULL,
        deleted_at INTEGER NOT NULL,
        PRIMARY KEY (identity_pubkey, group_id)
    );
    CREATE INDEX IF NOT EXISTS blobs_identity_seq ON blobs(identity_pubkey, seq);
    CREATE INDEX IF NOT EXISTS blobs_identity_slot ON blobs(identity_pubkey, slot) WHERE slot IS NOT NULL;
    CREATE INDEX IF NOT EXISTS blobs_identity_group ON blobs(identity_pubkey, group_id) WHERE group_id IS NOT NULL;
    CREATE INDEX IF NOT EXISTS blobs_sealed_created ON blobs(created_at) WHERE recipient_machine_pubkey IS NOT NULL;
    CREATE TABLE IF NOT EXISTS schema_version (
        version INTEGER PRIMARY KEY,
        applied_at INTEGER NOT NULL
    );
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
    );
    CREATE TABLE IF NOT EXISTS push_tokens (
        machine_pubkey TEXT PRIMARY KEY,
        identity_pubkey TEXT NOT NULL,
        platform TEXT NOT NULL,
        token TEXT NOT NULL,
        environment TEXT NOT NULL,
        updated_at INTEGER NOT NULL
    );
    CREATE INDEX IF NOT EXISTS push_tokens_identity ON push_tokens(identity_pubkey);";

/// Schema changes after `SCHEMA`, in order. `schema_version` records what was applied:
/// version 1 is `SCHEMA`, version `n + 2` is `MIGRATIONS[n]`, as in the Postgres backend. A
/// step only adds (a table, a nullable column, an index). Append; never edit a step that has
/// shipped.
const MIGRATIONS: &[&str] = &[];

pub struct Sqlite {
    path: String,
    local: Arc<Local>,
    writer: Arc<Mutex<Connection>>,
    readers: Arc<Mutex<Vec<Connection>>>,
    read_permits: Arc<Semaphore>,
}

impl Sqlite {
    pub fn open(path: &str, local: Arc<Local>) -> anyhow::Result<Sqlite> {
        let writer = Connection::open(path)?;
        writer.execute_batch(SCHEMA)?;
        migrate(&writer)?;

        let count = std::thread::available_parallelism().map(|n| n.get()).unwrap_or(4).clamp(4, 16);
        let mut readers = Vec::with_capacity(count);
        for _ in 0..count {
            let reader = Connection::open(path)?;
            reader.execute_batch("PRAGMA busy_timeout = 5000; PRAGMA query_only = ON;")?;
            readers.push(reader);
        }
        Ok(Sqlite {
            path: path.to_string(),
            local,
            writer: Arc::new(Mutex::new(writer)),
            readers: Arc::new(Mutex::new(readers)),
            read_permits: Arc::new(Semaphore::new(count)),
        })
    }

    /// Runs `f` on a reader connection from the pool.
    async fn read<T, F>(&self, f: F) -> ApiResult<T>
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
    async fn write<T, F>(&self, f: F) -> ApiResult<T>
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

/// Applies the `MIGRATIONS` this database has not had. `SCHEMA` ran already: it carries the
/// connection's pragmas, so it runs at every open, and nobody else has the file.
fn migrate(connection: &Connection) -> rusqlite::Result<()> {
    connection.execute("INSERT OR IGNORE INTO schema_version (version, applied_at) VALUES (1, ?1)", params![now()])?;
    let version: i64 = connection.query_row("SELECT MAX(version) FROM schema_version", [], |row| row.get(0))?;
    for (index, step) in MIGRATIONS.iter().enumerate().skip(version as usize - 1) {
        let tx = connection.unchecked_transaction()?;
        tx.execute_batch(step)?;
        tx.execute("INSERT INTO schema_version (version, applied_at) VALUES (?1, ?2)", params![index as i64 + 2, now()])?;
        tx.commit()?;
        tracing::info!(version = index + 2, "schema migrated");
    }
    Ok(())
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

// MARK: - Push tokens

pub fn set_push_token(connection: &Connection, identity_pubkey: &str, token: &PushToken) -> rusqlite::Result<()> {
    // A token belongs to one install. A phone that paired again has a new machine key and
    // the same token, so the old row goes.
    connection
        .prepare_cached("DELETE FROM push_tokens WHERE token = ?1 AND machine_pubkey != ?2")?
        .execute(params![token.token, token.machine_pubkey])?;
    connection
        .prepare_cached(
            "INSERT OR REPLACE INTO push_tokens (machine_pubkey, identity_pubkey, platform, token, environment, updated_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
        )?
        .execute(params![token.machine_pubkey, identity_pubkey, token.platform, token.token, token.environment, now()])?;
    Ok(())
}

pub fn delete_push_token(connection: &Connection, machine_pubkey: &str) -> rusqlite::Result<()> {
    connection.prepare_cached("DELETE FROM push_tokens WHERE machine_pubkey = ?1")?.execute(params![machine_pubkey])?;
    Ok(())
}

/// The identity's tokens, leaving out the machine that asks for the push.
pub fn push_tokens_for(connection: &Connection, identity_pubkey: &str, except_machine: &str) -> rusqlite::Result<Vec<PushToken>> {
    let mut statement = connection.prepare_cached(
        "SELECT machine_pubkey, platform, token, environment FROM push_tokens WHERE identity_pubkey = ?1 AND machine_pubkey != ?2",
    )?;
    let rows = statement.query_map(params![identity_pubkey, except_machine], |row| {
        Ok(PushToken { machine_pubkey: row.get(0)?, platform: row.get(1)?, token: row.get(2)?, environment: row.get(3)? })
    })?;
    rows.collect()
}

// MARK: - Identities and machines

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

/// True once a machine was unpaired. Its key never comes back: a Device that pairs again
/// generates a new one.
pub fn is_revoked(connection: &Connection, machine_pubkey: &str) -> rusqlite::Result<bool> {
    connection
        .prepare_cached("SELECT 1 FROM revoked_machines WHERE machine_pubkey = ?1")?
        .query_row(params![machine_pubkey], |_| Ok(()))
        .optional()
        .map(|row| row.is_some())
}

pub fn revoked_machines(connection: &Connection) -> rusqlite::Result<Vec<String>> {
    connection.prepare("SELECT machine_pubkey FROM revoked_machines")?.query_map([], |row| row.get(0))?.collect()
}

/// Unpairs a machine of this identity: its row goes, its key is remembered as revoked, and
/// the envelopes sealed to it (jobs, results, requests, responses nobody else can read) go
/// with it. False when the identity has no such machine.
pub fn revoke_machine(connection: &mut Connection, identity_pubkey: &str, machine_pubkey: &str) -> rusqlite::Result<bool> {
    let tx = connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
    let removed = tx
        .prepare_cached("DELETE FROM machines WHERE machine_pubkey = ?1 AND identity_pubkey = ?2")?
        .execute(params![machine_pubkey, identity_pubkey])?;
    if removed == 0 {
        return Ok(false);
    }
    tx.prepare_cached("INSERT OR REPLACE INTO revoked_machines (machine_pubkey, identity_pubkey, revoked_at) VALUES (?1, ?2, ?3)")?
        .execute(params![machine_pubkey, identity_pubkey, now()])?;
    tx.prepare_cached("DELETE FROM challenges WHERE machine_pubkey = ?1")?.execute(params![machine_pubkey])?;
    tx.prepare_cached("DELETE FROM push_tokens WHERE machine_pubkey = ?1")?.execute(params![machine_pubkey])?;
    let freed: i64 = tx
        .prepare_cached("SELECT COALESCE(SUM(size), 0) FROM blobs WHERE identity_pubkey = ?1 AND recipient_machine_pubkey = ?2")?
        .query_row(params![identity_pubkey, machine_pubkey], |row| row.get(0))?;
    tx.prepare_cached("DELETE FROM blobs WHERE identity_pubkey = ?1 AND recipient_machine_pubkey = ?2")?
        .execute(params![identity_pubkey, machine_pubkey])?;
    tx.prepare_cached("UPDATE usage SET bytes = MAX(bytes - ?1, 0) WHERE identity_pubkey = ?2")?
        .execute(params![freed, identity_pubkey])?;
    tx.commit()?;
    Ok(true)
}

/// Deletes the identity and every row it owns. The machines' keys stay in `revoked_machines`,
/// so their tokens die and a Device that still holds its attestation gets `410`.
pub fn delete_identity(connection: &mut Connection, identity_pubkey: &str) -> rusqlite::Result<DeletedIdentity> {
    let tx = connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
    let machines: Vec<String> = tx
        .prepare_cached("SELECT machine_pubkey FROM machines WHERE identity_pubkey = ?1")?
        .query_map(params![identity_pubkey], |row| row.get(0))?
        .collect::<rusqlite::Result<_>>()?;
    let files: Vec<String> = tx
        .prepare_cached("SELECT id FROM blobs WHERE identity_pubkey = ?1 AND kind = 'file'")?
        .query_map(params![identity_pubkey], |row| row.get(0))?
        .collect::<rusqlite::Result<_>>()?;
    tx.prepare_cached(
        "INSERT OR REPLACE INTO revoked_machines (machine_pubkey, identity_pubkey, revoked_at)
         SELECT machine_pubkey, identity_pubkey, ?2 FROM machines WHERE identity_pubkey = ?1",
    )?
    .execute(params![identity_pubkey, now()])?;
    tx.prepare_cached("DELETE FROM challenges WHERE machine_pubkey IN (SELECT machine_pubkey FROM machines WHERE identity_pubkey = ?1)")?
        .execute(params![identity_pubkey])?;
    for table in ["push_tokens", "pairings", "blobs", "deleted_groups", "sequences", "usage", "machines"] {
        tx.prepare_cached(&format!("DELETE FROM {table} WHERE identity_pubkey = ?1"))?.execute(params![identity_pubkey])?;
    }
    tx.prepare_cached("DELETE FROM identities WHERE pubkey = ?1")?.execute(params![identity_pubkey])?;
    tx.commit()?;
    Ok(DeletedIdentity { machines, files })
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
    if is_revoked(connection, machine_pubkey)? {
        return Err(ApiError::gone("Machine was unpaired"));
    }
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
        if is_revoked(connection, machine_pubkey)? {
            return Err(ApiError::gone("Machine was unpaired"));
        }
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

const BLOB_COLUMNS: &str = "id, kind, recipient_machine_pubkey, seq, ciphertext, created_at";

fn blob_row(row: &rusqlite::Row) -> rusqlite::Result<BlobRow> {
    blob_row_at(row, 0)
}

/// `BLOB_COLUMNS` starting at column `at`.
fn blob_row_at(row: &rusqlite::Row, at: usize) -> rusqlite::Result<BlobRow> {
    Ok(BlobRow {
        id: row.get(at)?,
        kind: row.get(at + 1)?,
        recipient_machine_pubkey: row.get(at + 2)?,
        seq: row.get(at + 3)?,
        ciphertext: row.get(at + 4)?,
        created_at: row.get(at + 5)?,
    })
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

/// Deletes the blobs `slot` supersedes and gives their bytes back to the identity's usage.
fn supersede(tx: &Connection, identity_pubkey: &str, slot: &Slot) -> rusqlite::Result<()> {
    let first: Option<i64> = tx
        .prepare_cached("SELECT MIN(seq) FROM blobs WHERE identity_pubkey = ?1 AND slot = ?2")?
        .query_row(params![identity_pubkey, slot.name], |row| row.get(0))?;
    let Some(first) = first else { return Ok(()) };
    let floor = if slot.keep_first { first } else { 0 };
    let freed: i64 = tx
        .prepare_cached("SELECT COALESCE(SUM(size), 0) FROM blobs WHERE identity_pubkey = ?1 AND slot = ?2 AND seq > ?3")?
        .query_row(params![identity_pubkey, slot.name, floor], |row| row.get(0))?;
    tx.prepare_cached("DELETE FROM blobs WHERE identity_pubkey = ?1 AND slot = ?2 AND seq > ?3")?
        .execute(params![identity_pubkey, slot.name, floor])?;
    tx.prepare_cached("UPDATE usage SET bytes = MAX(bytes - ?1, 0) WHERE identity_pubkey = ?2")?
        .execute(params![freed, identity_pubkey])?;
    Ok(())
}

/// Stores a blob under the identity's next sequence number. A known id returns its existing
/// seq. `quota_bytes` of 0 means unlimited.
pub fn insert_blob(connection: &mut Connection, blob: &NewBlob, quota_bytes: u64) -> ApiResult<Inserted> {
    let NewBlob { identity_pubkey, id, kind, recipient_machine_pubkey, slot, group, payload } = blob;
    let (identity_pubkey, id, kind) = (identity_pubkey.as_str(), id.as_str(), kind.as_str());
    let (recipient_machine_pubkey, group) = (recipient_machine_pubkey.as_deref(), group.as_deref());
    let ciphertext: &[u8] = match payload {
        Payload::Inline(bytes) => bytes,
        Payload::InFileStore { .. } => &[],
    };
    let size = payload.size();
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
    if let Some(group) = group {
        if group_deleted(&tx, identity_pubkey, group)? {
            return Err(ApiError::conflict("Group was deleted"));
        }
    }
    // Before the quota check, so a new version of a message fits where the old one was. A
    // refusal below rolls this back with the rest.
    if let Some(slot) = slot {
        supersede(&tx, identity_pubkey, slot)?;
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
        "INSERT INTO blobs (id, identity_pubkey, kind, recipient_machine_pubkey, seq, ciphertext, size, created_at, slot, group_id)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)",
    )?
    .execute(params![id, identity_pubkey, kind, recipient_machine_pubkey, seq, ciphertext, size, now(), slot.as_ref().map(|s| s.name.as_str()), group])?;
    tx.prepare_cached(
        "INSERT INTO usage (identity_pubkey, bytes) VALUES (?1, ?2)
         ON CONFLICT(identity_pubkey) DO UPDATE SET bytes = bytes + excluded.bytes",
    )?
    .execute(params![identity_pubkey, size])?;
    tx.commit()?;
    Ok(Inserted { seq, existing: false })
}

/// True once the identity deleted this group. It stays deleted: a blob that names it later (a
/// Runner finishing a turn in a chat another Device deleted) is refused.
pub fn group_deleted(connection: &Connection, identity_pubkey: &str, group: &str) -> rusqlite::Result<bool> {
    connection
        .prepare_cached("SELECT 1 FROM deleted_groups WHERE identity_pubkey = ?1 AND group_id = ?2")?
        .exists(params![identity_pubkey, group])
}

/// Deletes every blob of a group (a chat's messages, read marks, and attachments) and gives
/// their bytes back to the identity's usage. Returns the ids of the `file` blobs among them;
/// the caller removes their objects afterwards.
pub fn delete_group(connection: &mut Connection, identity_pubkey: &str, group: &str) -> rusqlite::Result<Vec<String>> {
    let tx = connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
    tx.prepare_cached("INSERT OR IGNORE INTO deleted_groups (identity_pubkey, group_id, deleted_at) VALUES (?1, ?2, ?3)")?
        .execute(params![identity_pubkey, group, now()])?;
    let rows: Vec<(String, String, i64)> = tx
        .prepare_cached("DELETE FROM blobs WHERE identity_pubkey = ?1 AND group_id = ?2 RETURNING id, kind, size")?
        .query_map(params![identity_pubkey, group], |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)))?
        .collect::<rusqlite::Result<_>>()?;
    let freed: i64 = rows.iter().map(|(_, _, size)| size).sum();
    tx.prepare_cached("UPDATE usage SET bytes = MAX(bytes - ?1, 0) WHERE identity_pubkey = ?2")?
        .execute(params![freed, identity_pubkey])?;
    tx.commit()?;
    Ok(rows.into_iter().filter(|(_, kind, _)| kind == "file").map(|(id, _, _)| id).collect())
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
/// A page ends at `limit` rows or before the row that would take it past `max_bytes`, and
/// always holds one row when there is one; the caller asks again from the last seq it got.
pub fn blobs_since(
    connection: &Connection,
    identity_pubkey: &str,
    machine_pubkey: &str,
    since: i64,
    kinds: &[String],
    limit: i64,
    max_bytes: i64,
) -> rusqlite::Result<Vec<BlobRow>> {
    let mut sql = format!(
        "SELECT size, {BLOB_COLUMNS} FROM blobs
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
    let mut statement = connection.prepare_cached(&sql)?;
    let mut rows = statement.query(params_from_iter(values))?;
    let (mut page, mut bytes) = (Vec::new(), 0i64);
    // The size is read before the ciphertext, so the row that ends the page is never loaded.
    while let Some(row) = rows.next()? {
        bytes += row.get::<_, i64>(0)?;
        if bytes > max_bytes && !page.is_empty() {
            break;
        }
        page.push(blob_row_at(row, 1)?);
    }
    Ok(page)
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

/// Drops the sealed envelopes nobody consumed and the marks of groups deleted long ago.
pub fn sweep(connection: &mut Connection, sealed_before: i64, groups_before: i64) -> rusqlite::Result<u64> {
    let stale = format!("recipient_machine_pubkey IS NOT NULL AND kind IN ({}) AND created_at < ?1", sealed_kinds_sql());
    let tx = connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
    tx.execute(
        &format!(
            "UPDATE usage SET bytes = MAX(bytes - (SELECT COALESCE(SUM(size), 0) FROM blobs WHERE blobs.identity_pubkey = usage.identity_pubkey AND {stale}), 0)
             WHERE identity_pubkey IN (SELECT identity_pubkey FROM blobs WHERE {stale})"
        ),
        params![sealed_before],
    )?;
    let gone = tx.execute(&format!("DELETE FROM blobs WHERE {stale}"), params![sealed_before])?;
    tx.execute("DELETE FROM deleted_groups WHERE deleted_at < ?1", params![groups_before])?;
    tx.commit()?;
    Ok(gone as u64)
}

pub fn stats(connection: &Connection) -> rusqlite::Result<Stats> {
    let count = |sql: &str| connection.query_row(sql, [], |row| row.get::<_, i64>(0));
    let seen_since = |seconds: i64| connection.query_row("SELECT COUNT(*) FROM machines WHERE last_seen > ?1", params![now() - seconds], |row| row.get::<_, i64>(0));
    Ok(Stats {
        identities: count("SELECT COUNT(*) FROM identities")?,
        machines: count("SELECT COUNT(*) FROM machines")?,
        active_machines: [seen_since(86_400)?, seen_since(7 * 86_400)?, seen_since(30 * 86_400)?],
        revoked_machines: count("SELECT COUNT(*) FROM revoked_machines")?,
        deleted_groups: count("SELECT COUNT(*) FROM deleted_groups")?,
        blobs: connection
            .prepare("SELECT kind, COUNT(*), COALESCE(SUM(size), 0) FROM blobs GROUP BY kind ORDER BY kind")?
            .query_map([], |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)))?
            .collect::<rusqlite::Result<_>>()?,
        usage_bytes: count("SELECT COALESCE(SUM(bytes), 0) FROM usage")?,
        largest_identity_bytes: count("SELECT COALESCE(MAX(bytes), 0) FROM usage")?,
        push_tokens: connection
            .prepare("SELECT platform, COUNT(*) FROM push_tokens GROUP BY platform ORDER BY platform")?
            .query_map([], |row| Ok((row.get(0)?, row.get(1)?)))?
            .collect::<rusqlite::Result<_>>()?,
    })
}

pub fn orphans(connection: &Connection, files: &[(String, String)]) -> rusqlite::Result<Vec<(String, String)>> {
    let mut known = connection.prepare_cached("SELECT 1 FROM identities WHERE pubkey = ?1")?;
    let mut has_row = connection.prepare_cached("SELECT 1 FROM blobs WHERE identity_pubkey = ?1 AND id = ?2")?;
    let mut orphans = Vec::new();
    for (identity, id) in files {
        if known.exists(params![identity])? && !has_row.exists(params![identity, id])? {
            orphans.push((identity.clone(), id.clone()));
        }
    }
    Ok(orphans)
}

// MARK: - Store

fn not_yours(owner: String, identity_pubkey: &str) -> ApiResult<()> {
    if owner != identity_pubkey {
        return Err(ApiError::forbidden("Not your pairing"));
    }
    Ok(())
}

#[async_trait]
impl Store for Sqlite {
    fn describe(&self) -> String {
        format!("sqlite {}", self.path)
    }

    async fn register_identity(&self, identity_pubkey: &str, content_pubkey: &str, machine_pubkey: &str, box_pubkey: &str, attestation: &str) -> ApiResult<()> {
        let args = [identity_pubkey, content_pubkey, machine_pubkey, box_pubkey, attestation].map(str::to_string);
        self.write(move |db| register_identity(db, &args[0], &args[1], &args[2], &args[3], &args[4])).await
    }

    async fn create_challenge(&self, nonce: &str, machine_pubkey: &str, expires_at: i64) -> ApiResult<()> {
        let (nonce, machine_pubkey) = (nonce.to_string(), machine_pubkey.to_string());
        self.write(move |db| create_challenge(db, &nonce, &machine_pubkey, expires_at)).await
    }

    async fn redeem_challenge(&self, nonce: &str, machine_pubkey: &str) -> ApiResult<Machine> {
        let (nonce, machine_pubkey) = (nonce.to_string(), machine_pubkey.to_string());
        self.write(move |db| {
            let Some((challenged, expires_at)) = take_challenge(db, &nonce)? else {
                return Err(ApiError::unauthorized("Unknown challenge"));
            };
            if challenged != machine_pubkey || expires_at < now() {
                return Err(ApiError::unauthorized("Challenge expired"));
            }
            let machine = machine(db, &machine_pubkey)?.ok_or_else(|| ApiError::not_found("Unknown machine"))?;
            touch_machine(db, &machine.machine_pubkey)?;
            Ok(machine)
        })
        .await
    }

    async fn machines_for(&self, identity_pubkey: &str) -> ApiResult<Vec<Machine>> {
        let identity_pubkey = identity_pubkey.to_string();
        self.read(move |db| Ok(machines_for(db, &identity_pubkey)?)).await
    }

    async fn touch_machine(&self, machine_pubkey: &str) -> ApiResult<()> {
        let machine_pubkey = machine_pubkey.to_string();
        self.write(move |db| Ok(touch_machine(db, &machine_pubkey)?)).await
    }

    async fn revoke_machine(&self, identity_pubkey: &str, machine_pubkey: &str) -> ApiResult<bool> {
        let (identity_pubkey, machine_pubkey) = (identity_pubkey.to_string(), machine_pubkey.to_string());
        self.write(move |db| Ok(revoke_machine(db, &identity_pubkey, &machine_pubkey)?)).await
    }

    async fn revoked_machines(&self) -> ApiResult<Vec<String>> {
        self.read(|db| Ok(revoked_machines(db)?)).await
    }

    async fn precheck_blob(&self, identity_pubkey: &str, id: &str, group: Option<&str>, size: i64, quota_bytes: u64) -> ApiResult<Option<Inserted>> {
        let (identity_pubkey, id, group) = (identity_pubkey.to_string(), id.to_string(), group.map(str::to_string));
        self.read(move |db| {
            if let Some(seq) = blob_seq(db, &identity_pubkey, &id)? {
                return Ok(Some(Inserted { seq, existing: true }));
            }
            if let Some(group) = &group {
                if group_deleted(db, &identity_pubkey, group)? {
                    return Err(ApiError::conflict("Group was deleted"));
                }
            }
            if quota_bytes > 0 && (usage(db, &identity_pubkey)? + size) as u64 > quota_bytes {
                return Err(ApiError::too_large("Storage quota exceeded"));
            }
            Ok(None)
        })
        .await
    }

    async fn insert_blob(&self, blob: NewBlob, quota_bytes: u64) -> ApiResult<Inserted> {
        self.write(move |db| insert_blob(db, &blob, quota_bytes)).await
    }

    async fn delete_identity(&self, identity_pubkey: &str) -> ApiResult<DeletedIdentity> {
        let identity_pubkey = identity_pubkey.to_string();
        self.write(move |db| Ok(delete_identity(db, &identity_pubkey)?)).await
    }

    async fn orphans(&self, files: &[(String, String)]) -> ApiResult<Vec<(String, String)>> {
        let files = files.to_vec();
        self.read(move |db| Ok(orphans(db, &files)?)).await
    }

    async fn stats(&self) -> ApiResult<Stats> {
        self.read(|db| Ok(stats(db)?)).await
    }

    async fn sweep(&self, sealed_before: i64, groups_before: i64) -> ApiResult<u64> {
        self.write(move |db| Ok(sweep(db, sealed_before, groups_before)?)).await
    }

    async fn blobs_since(&self, identity_pubkey: &str, machine_pubkey: &str, since: i64, kinds: &[String], limit: i64, max_bytes: i64) -> ApiResult<(Vec<BlobRow>, i64)> {
        let (identity_pubkey, machine_pubkey, kinds) = (identity_pubkey.to_string(), machine_pubkey.to_string(), kinds.to_vec());
        self.read(move |db| {
            let rows = blobs_since(db, &identity_pubkey, &machine_pubkey, since, &kinds, limit, max_bytes)?;
            Ok((rows, current_seq(db, &identity_pubkey)?))
        })
        .await
    }

    async fn blob(&self, identity_pubkey: &str, machine_pubkey: &str, id: &str) -> ApiResult<Option<BlobRow>> {
        let args = [identity_pubkey, machine_pubkey, id].map(str::to_string);
        self.read(move |db| Ok(blob(db, &args[0], &args[1], &args[2])?)).await
    }

    async fn delete_blob(&self, identity_pubkey: &str, id: &str) -> ApiResult<Option<String>> {
        let (identity_pubkey, id) = (identity_pubkey.to_string(), id.to_string());
        self.write(move |db| Ok(delete_blob(db, &identity_pubkey, &id)?)).await
    }

    async fn delete_group(&self, identity_pubkey: &str, group: &str) -> ApiResult<Vec<String>> {
        let (identity_pubkey, group) = (identity_pubkey.to_string(), group.to_string());
        self.write(move |db| Ok(delete_group(db, &identity_pubkey, &group)?)).await
    }

    async fn set_push_token(&self, identity_pubkey: &str, token: &PushToken) -> ApiResult<()> {
        let (identity_pubkey, token) = (identity_pubkey.to_string(), token.clone());
        self.write(move |db| Ok(set_push_token(db, &identity_pubkey, &token)?)).await
    }

    async fn delete_push_token(&self, machine_pubkey: &str) -> ApiResult<()> {
        let machine_pubkey = machine_pubkey.to_string();
        self.write(move |db| Ok(delete_push_token(db, &machine_pubkey)?)).await
    }

    async fn push_tokens_for(&self, identity_pubkey: &str, except_machine: &str) -> ApiResult<Vec<PushToken>> {
        let (identity_pubkey, except_machine) = (identity_pubkey.to_string(), except_machine.to_string());
        self.read(move |db| Ok(push_tokens_for(db, &identity_pubkey, &except_machine)?)).await
    }

    async fn create_pairing(&self, nonce: &str, identity_pubkey: &str, expires_at: i64) -> ApiResult<()> {
        let (nonce, identity_pubkey) = (nonce.to_string(), identity_pubkey.to_string());
        self.write(move |db| Ok(create_pairing(db, &nonce, &identity_pubkey, expires_at)?)).await
    }

    async fn delete_pairing(&self, nonce: &str, identity_pubkey: &str) -> ApiResult<()> {
        let (nonce, identity_pubkey) = (nonce.to_string(), identity_pubkey.to_string());
        self.write(move |db| {
            not_yours(pairing_owner(db, &nonce)?, &identity_pubkey)?;
            Ok(delete_pairing(db, &nonce)?)
        })
        .await
    }

    async fn post_pair_request(&self, nonce: &str, ciphertext: &[u8]) -> ApiResult<()> {
        let (nonce, ciphertext) = (nonce.to_string(), ciphertext.to_vec());
        self.write(move |db| {
            pairing_owner(db, &nonce)?;
            if !set_pairing_request(db, &nonce, &ciphertext)? {
                return Err(ApiError::conflict("Pairing already has a request"));
            }
            Ok(())
        })
        .await
    }

    async fn pair_request(&self, nonce: &str, identity_pubkey: &str) -> ApiResult<Option<Vec<u8>>> {
        let (nonce, identity_pubkey) = (nonce.to_string(), identity_pubkey.to_string());
        self.read(move |db| {
            not_yours(pairing_owner(db, &nonce)?, &identity_pubkey)?;
            Ok(pairing_request(db, &nonce)?)
        })
        .await
    }

    async fn post_pair_reply(&self, nonce: &str, identity_pubkey: &str, ciphertext: &[u8]) -> ApiResult<()> {
        let (nonce, identity_pubkey, ciphertext) = (nonce.to_string(), identity_pubkey.to_string(), ciphertext.to_vec());
        self.write(move |db| {
            not_yours(pairing_owner(db, &nonce)?, &identity_pubkey)?;
            Ok(set_pairing_reply(db, &nonce, &ciphertext)?)
        })
        .await
    }

    async fn pair_reply(&self, nonce: &str) -> ApiResult<Option<Vec<u8>>> {
        let nonce = nonce.to_string();
        self.read(move |db| {
            pairing_owner(db, &nonce)?;
            Ok(pairing_reply(db, &nonce)?)
        })
        .await
    }

    async fn tick(&self) -> ApiResult<()> {
        self.write(|db| Ok(expire(db)?)).await
    }

    async fn socket_opened(&self, _identity_pubkey: &str, _machine_pubkey: &str, _socket_id: u64, first_here: bool) -> ApiResult<bool> {
        Ok(first_here)
    }

    async fn socket_closed(&self, _identity_pubkey: &str, _machine_pubkey: &str, _socket_id: u64, last_here: bool) -> ApiResult<bool> {
        Ok(last_here)
    }

    async fn online(&self, identity_pubkey: &str) -> ApiResult<HashSet<String>> {
        Ok(self.local.hub.online(identity_pubkey))
    }

    async fn publish(&self, event: Event) {
        self.local.deliver(&event);
    }

    async fn close(&self) {}
}
