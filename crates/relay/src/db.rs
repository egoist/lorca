use rusqlite::{params, Connection, OptionalExtension};

pub const KINDS: &[&str] = &["roster", "chat", "job", "machine", "key"];

pub fn now() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0)
}

pub fn open(path: &str) -> anyhow::Result<Connection> {
    let connection = Connection::open(path)?;
    connection.execute_batch(
        "PRAGMA journal_mode = WAL;
         PRAGMA foreign_keys = ON;
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
             id TEXT PRIMARY KEY,
             identity_pubkey TEXT NOT NULL,
             kind TEXT NOT NULL,
             recipient_machine_pubkey TEXT,
             seq INTEGER NOT NULL,
             ciphertext BLOB NOT NULL,
             size INTEGER NOT NULL,
             created_at INTEGER NOT NULL
         );
         CREATE INDEX IF NOT EXISTS blobs_identity_seq ON blobs(identity_pubkey, seq);
         CREATE TABLE IF NOT EXISTS sequences (
             identity_pubkey TEXT PRIMARY KEY,
             seq INTEGER NOT NULL
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
         );",
    )?;
    Ok(connection)
}

pub struct Machine {
    pub machine_pubkey: String,
    pub identity_pubkey: String,
    pub box_pubkey: String,
    pub last_seen: i64,
    pub created_at: i64,
}

pub fn machine(connection: &Connection, machine_pubkey: &str) -> rusqlite::Result<Option<Machine>> {
    connection
        .query_row(
            "SELECT machine_pubkey, identity_pubkey, box_pubkey, last_seen, created_at FROM machines WHERE machine_pubkey = ?1",
            params![machine_pubkey],
            |row| {
                Ok(Machine {
                    machine_pubkey: row.get(0)?,
                    identity_pubkey: row.get(1)?,
                    box_pubkey: row.get(2)?,
                    last_seen: row.get(3)?,
                    created_at: row.get(4)?,
                })
            },
        )
        .optional()
}

pub fn machines_for(connection: &Connection, identity_pubkey: &str) -> rusqlite::Result<Vec<Machine>> {
    let mut statement = connection.prepare(
        "SELECT machine_pubkey, identity_pubkey, box_pubkey, last_seen, created_at FROM machines WHERE identity_pubkey = ?1 ORDER BY created_at",
    )?;
    let rows = statement.query_map(params![identity_pubkey], |row| {
        Ok(Machine {
            machine_pubkey: row.get(0)?,
            identity_pubkey: row.get(1)?,
            box_pubkey: row.get(2)?,
            last_seen: row.get(3)?,
            created_at: row.get(4)?,
        })
    })?;
    rows.collect()
}

pub fn touch_machine(connection: &Connection, machine_pubkey: &str) -> rusqlite::Result<()> {
    connection.execute("UPDATE machines SET last_seen = ?1 WHERE machine_pubkey = ?2", params![now(), machine_pubkey])?;
    Ok(())
}

pub fn next_seq(connection: &Connection, identity_pubkey: &str) -> rusqlite::Result<i64> {
    connection.execute(
        "INSERT INTO sequences (identity_pubkey, seq) VALUES (?1, 1)
         ON CONFLICT(identity_pubkey) DO UPDATE SET seq = seq + 1",
        params![identity_pubkey],
    )?;
    connection.query_row("SELECT seq FROM sequences WHERE identity_pubkey = ?1", params![identity_pubkey], |row| row.get(0))
}

pub fn current_seq(connection: &Connection, identity_pubkey: &str) -> rusqlite::Result<i64> {
    Ok(connection
        .query_row("SELECT seq FROM sequences WHERE identity_pubkey = ?1", params![identity_pubkey], |row| row.get(0))
        .optional()?
        .unwrap_or(0))
}

pub struct BlobRow {
    pub id: String,
    pub kind: String,
    pub recipient_machine_pubkey: Option<String>,
    pub seq: i64,
    pub ciphertext: Vec<u8>,
    pub created_at: i64,
}

pub fn blobs_since(
    connection: &Connection,
    identity_pubkey: &str,
    machine_pubkey: &str,
    since: i64,
    kinds: &[String],
    limit: i64,
) -> rusqlite::Result<Vec<BlobRow>> {
    let mut statement = connection.prepare(
        "SELECT id, kind, recipient_machine_pubkey, seq, ciphertext, created_at FROM blobs
         WHERE identity_pubkey = ?1 AND seq > ?2
           AND (recipient_machine_pubkey IS NULL OR recipient_machine_pubkey = ?3)
         ORDER BY seq ASC LIMIT ?4",
    )?;
    let rows = statement.query_map(params![identity_pubkey, since, machine_pubkey, limit], |row| {
        Ok(BlobRow {
            id: row.get(0)?,
            kind: row.get(1)?,
            recipient_machine_pubkey: row.get(2)?,
            seq: row.get(3)?,
            ciphertext: row.get(4)?,
            created_at: row.get(5)?,
        })
    })?;
    let mut out = Vec::new();
    for row in rows {
        let row = row?;
        if kinds.is_empty() || kinds.iter().any(|k| k == &row.kind) {
            out.push(row);
        }
    }
    Ok(out)
}

pub fn expire(connection: &Connection) -> rusqlite::Result<()> {
    let now = now();
    connection.execute("DELETE FROM challenges WHERE expires_at < ?1", params![now])?;
    connection.execute("DELETE FROM pairings WHERE expires_at < ?1", params![now])?;
    Ok(())
}
