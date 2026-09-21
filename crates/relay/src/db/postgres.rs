//! Postgres: one database shared by every relay process, so a deploy overlaps the old process
//! with the new one and nothing is down in between. What a lone process keeps in memory goes
//! through the database here: events travel by `NOTIFY` to every process's sockets, presence
//! is the `relay_sockets` table, and each process proves it is alive in `relay_instances`, so
//! the sockets of one that died stop counting.
//!
//! Writes of one identity are serialized with a transaction-scoped advisory lock, which gives
//! them the order SQLite's single writer gives: a seq is visible before the next one exists.

use std::collections::HashSet;
use std::sync::Arc;

use async_trait::async_trait;
use deadpool_postgres::{Manager, ManagerConfig, Object, Pool, RecyclingMethod};
use futures::StreamExt;
use tokio_postgres::types::ToSql;
use tokio_postgres::{AsyncMessage, Row, Transaction};

use super::{now, sealed_kinds_sql, BlobRow, DeletedIdentity, Event, Inserted, Local, Machine, NewBlob, Payload, PushToken, Stats, Store};
use crate::routes::{ApiError, ApiResult};

const CHANNEL: &str = "lorca_relay";
/// A process beats once a minute (`tick`); one silent for this long is gone.
const INSTANCE_TTL: i64 = 150;
/// Held while the schema is made, so two processes starting together do not collide.
const SCHEMA_LOCK: i64 = 0x10ca_5c4e;

const SCHEMA: &str = "
    CREATE TABLE IF NOT EXISTS identities (
        pubkey TEXT PRIMARY KEY,
        content_pubkey TEXT NOT NULL,
        created_at BIGINT NOT NULL
    );
    CREATE TABLE IF NOT EXISTS machines (
        machine_pubkey TEXT PRIMARY KEY,
        identity_pubkey TEXT NOT NULL,
        box_pubkey TEXT NOT NULL,
        attestation TEXT NOT NULL,
        last_seen BIGINT NOT NULL,
        created_at BIGINT NOT NULL
    );
    CREATE INDEX IF NOT EXISTS machines_identity ON machines(identity_pubkey);
    CREATE TABLE IF NOT EXISTS revoked_machines (
        machine_pubkey TEXT PRIMARY KEY,
        identity_pubkey TEXT NOT NULL,
        revoked_at BIGINT NOT NULL
    );
    CREATE TABLE IF NOT EXISTS blobs (
        identity_pubkey TEXT NOT NULL,
        id TEXT NOT NULL,
        kind TEXT NOT NULL,
        recipient_machine_pubkey TEXT,
        seq BIGINT NOT NULL,
        ciphertext BYTEA NOT NULL,
        size BIGINT NOT NULL,
        created_at BIGINT NOT NULL,
        slot TEXT,
        group_id TEXT,
        PRIMARY KEY (identity_pubkey, id)
    );
    CREATE INDEX IF NOT EXISTS blobs_identity_seq ON blobs(identity_pubkey, seq);
    CREATE INDEX IF NOT EXISTS blobs_sealed_created ON blobs(created_at) WHERE recipient_machine_pubkey IS NOT NULL;
    CREATE INDEX IF NOT EXISTS blobs_identity_slot ON blobs(identity_pubkey, slot) WHERE slot IS NOT NULL;
    CREATE INDEX IF NOT EXISTS blobs_identity_group ON blobs(identity_pubkey, group_id) WHERE group_id IS NOT NULL;
    CREATE TABLE IF NOT EXISTS deleted_groups (
        identity_pubkey TEXT NOT NULL,
        group_id TEXT NOT NULL,
        deleted_at BIGINT NOT NULL,
        PRIMARY KEY (identity_pubkey, group_id)
    );
    CREATE TABLE IF NOT EXISTS sequences (
        identity_pubkey TEXT PRIMARY KEY,
        seq BIGINT NOT NULL
    );
    CREATE TABLE IF NOT EXISTS usage (
        identity_pubkey TEXT PRIMARY KEY,
        bytes BIGINT NOT NULL
    );
    CREATE TABLE IF NOT EXISTS challenges (
        nonce TEXT PRIMARY KEY,
        machine_pubkey TEXT NOT NULL,
        expires_at BIGINT NOT NULL
    );
    CREATE TABLE IF NOT EXISTS pairings (
        nonce TEXT PRIMARY KEY,
        identity_pubkey TEXT NOT NULL,
        request BYTEA,
        reply BYTEA,
        created_at BIGINT NOT NULL,
        expires_at BIGINT NOT NULL
    );
    CREATE TABLE IF NOT EXISTS push_tokens (
        machine_pubkey TEXT PRIMARY KEY,
        identity_pubkey TEXT NOT NULL,
        platform TEXT NOT NULL,
        token TEXT NOT NULL,
        environment TEXT NOT NULL,
        updated_at BIGINT NOT NULL
    );
    CREATE INDEX IF NOT EXISTS push_tokens_identity ON push_tokens(identity_pubkey);
    CREATE TABLE IF NOT EXISTS relay_instances (
        id TEXT PRIMARY KEY,
        beat_at BIGINT NOT NULL
    );
    CREATE TABLE IF NOT EXISTS relay_sockets (
        instance_id TEXT NOT NULL,
        socket_id BIGINT NOT NULL,
        identity_pubkey TEXT NOT NULL,
        machine_pubkey TEXT NOT NULL,
        PRIMARY KEY (instance_id, socket_id)
    );
    CREATE INDEX IF NOT EXISTS relay_sockets_identity ON relay_sockets(identity_pubkey);";

/// Schema changes after `SCHEMA`, in order. `schema_version` records what was applied:
/// version 1 is `SCHEMA`, version `n + 2` is `MIGRATIONS[n]`. A deploy runs the old process
/// beside the new one, so a step only adds (a table, a nullable column, an index). Append;
/// never edit a step that has shipped.
const MIGRATIONS: &[&str] = &[];

/// Applies the steps this database has not had. DDL locks whole tables, and the process this
/// one replaces is writing to them meanwhile, so a step runs once and not at every start, and
/// one that lost a deadlock to a write runs again.
async fn migrate(client: &mut Object) -> Result<(), tokio_postgres::Error> {
    client.batch_execute("CREATE TABLE IF NOT EXISTS schema_version (version BIGINT PRIMARY KEY, applied_at BIGINT NOT NULL)").await?;
    let version: i64 = client.query_one("SELECT COALESCE(MAX(version), 0) FROM schema_version", &[]).await?.get(0);
    for (index, step) in std::iter::once(&SCHEMA).chain(MIGRATIONS).enumerate().skip(version as usize) {
        let mut attempt = 0;
        loop {
            attempt += 1;
            let tx = client.transaction().await?;
            let applied = async {
                tx.batch_execute(step).await?;
                tx.execute("INSERT INTO schema_version (version, applied_at) VALUES ($1, $2)", &[&(index as i64 + 1), &now()]).await
            }
            .await;
            match applied {
                Ok(_) => break tx.commit().await?,
                Err(error) if error.code() == Some(&tokio_postgres::error::SqlState::T_R_DEADLOCK_DETECTED) && attempt < 5 => {
                    tracing::warn!(version = index + 1, "schema step lost a deadlock; again");
                    drop(tx);
                }
                Err(error) => return Err(error),
            }
        }
        tracing::info!(version = index + 1, "schema migrated");
    }
    Ok(())
}

impl From<tokio_postgres::Error> for ApiError {
    fn from(error: tokio_postgres::Error) -> Self {
        tracing::error!(%error, "database");
        ApiError::internal("Database error")
    }
}

impl From<deadpool_postgres::PoolError> for ApiError {
    fn from(error: deadpool_postgres::PoolError) -> Self {
        tracing::error!(%error, "database pool");
        ApiError::internal("Database error")
    }
}

pub struct Postgres {
    pool: Pool,
    /// This process, in `relay_instances` and `relay_sockets`.
    instance: String,
    host: String,
}

/// TLS for `sslmode=require` and `prefer`, with the provider named: the build links both ring
/// and aws-lc-rs, and rustls picks neither by itself. Certificates are checked against the
/// web's roots; a database on a private network takes `sslmode=disable`.
fn tls() -> tokio_postgres_rustls::MakeRustlsConnect {
    let roots = rustls::RootCertStore { roots: webpki_roots::TLS_SERVER_ROOTS.to_vec() };
    let config = rustls::ClientConfig::builder_with_provider(Arc::new(rustls::crypto::ring::default_provider()))
        .with_safe_default_protocol_versions()
        .expect("ring supports the default TLS versions")
        .with_root_certificates(roots)
        .with_no_client_auth();
    tokio_postgres_rustls::MakeRustlsConnect::new(config)
}

impl Postgres {
    pub async fn open(url: &str, local: Arc<Local>) -> anyhow::Result<Postgres> {
        let config: tokio_postgres::Config = url.parse()?;
        let host = match config.get_hosts().first() {
            Some(tokio_postgres::config::Host::Tcp(name)) => name.clone(),
            Some(tokio_postgres::config::Host::Unix(path)) => path.display().to_string(),
            None => String::new(),
        };
        let manager = Manager::from_config(config.clone(), tls(), ManagerConfig { recycling_method: RecyclingMethod::Fast });
        let pool = Pool::builder(manager).max_size(16).build()?;

        let mut client = pool.get().await?;
        client.execute("SELECT pg_advisory_lock($1)", &[&SCHEMA_LOCK]).await?;
        let made = migrate(&mut client).await;
        client.execute("SELECT pg_advisory_unlock($1)", &[&SCHEMA_LOCK]).await?;
        made?;

        let store = Postgres { pool, instance: uuid::Uuid::new_v4().to_string(), host };
        store.beat(&client).await?;
        drop(client);
        tokio::spawn(listen(config, local, store.pool.clone()));
        Ok(store)
    }

    async fn client(&self) -> ApiResult<Object> {
        Ok(self.pool.get().await?)
    }

    async fn beat(&self, client: &Object) -> Result<(), tokio_postgres::Error> {
        client
            .execute(
                "INSERT INTO relay_instances (id, beat_at) VALUES ($1, $2) ON CONFLICT (id) DO UPDATE SET beat_at = EXCLUDED.beat_at",
                &[&self.instance, &now()],
            )
            .await?;
        Ok(())
    }

    /// True when a process that is alive holds a socket of this machine.
    async fn has_socket(&self, client: &Object, machine_pubkey: &str) -> ApiResult<bool> {
        let row = client
            .query_one(
                "SELECT EXISTS (
                     SELECT 1 FROM relay_sockets s JOIN relay_instances i ON i.id = s.instance_id
                     WHERE s.machine_pubkey = $1 AND i.beat_at > $2)",
                &[&machine_pubkey, &(now() - INSTANCE_TTL)],
            )
            .await?;
        Ok(row.get(0))
    }
}

/// Hears every process's events, this one's among them, and hands them to the local sockets.
/// A listener that lost its connection may have missed some, so after it is back every local
/// socket is told to look again and the revoked keys are read afresh.
async fn listen(config: tokio_postgres::Config, local: Arc<Local>, pool: Pool) {
    let mut first = true;
    loop {
        match config.connect(tls()).await {
            Ok((client, mut connection)) => {
                let mut messages = futures::stream::poll_fn(move |cx| connection.poll_message(cx));
                let (sender, mut notifications) = tokio::sync::mpsc::unbounded_channel();
                let pump = tokio::spawn(async move {
                    while let Some(message) = messages.next().await {
                        match message {
                            Ok(AsyncMessage::Notification(notification)) => {
                                let _ = sender.send(notification.payload().to_string());
                            }
                            Ok(_) => {}
                            Err(error) => {
                                tracing::warn!(%error, "event listener lost its connection");
                                break;
                            }
                        }
                    }
                });
                if let Err(error) = client.batch_execute(&format!("LISTEN {CHANNEL}")).await {
                    tracing::warn!(%error, "LISTEN");
                } else {
                    if !first {
                        if let Ok(client) = pool.get().await {
                            if let Ok(rows) = client.query("SELECT machine_pubkey FROM revoked_machines", &[]).await {
                                rows.iter().for_each(|row| local.revoked.insert(row.get(0)));
                            }
                        }
                        local.hub.everyone();
                    }
                    first = false;
                    while let Some(payload) = notifications.recv().await {
                        match serde_json::from_str::<Event>(&payload) {
                            Ok(event) => local.deliver(&event),
                            Err(error) => tracing::warn!(%error, "event payload"),
                        }
                    }
                }
                pump.abort();
            }
            Err(error) => tracing::warn!(%error, "event listener cannot connect"),
        }
        tokio::time::sleep(std::time::Duration::from_secs(1)).await;
    }
}

/// Serializes this identity's writes until the transaction ends.
async fn lock_identity(tx: &Transaction<'_>, identity_pubkey: &str) -> Result<(), tokio_postgres::Error> {
    tx.execute("SELECT pg_advisory_xact_lock(hashtextextended($1, 0))", &[&identity_pubkey]).await?;
    Ok(())
}

async fn give_back(tx: &Transaction<'_>, identity_pubkey: &str, bytes: i64) -> Result<(), tokio_postgres::Error> {
    tx.execute("UPDATE usage SET bytes = GREATEST(bytes - $1, 0) WHERE identity_pubkey = $2", &[&bytes, &identity_pubkey]).await?;
    Ok(())
}

const MACHINE_COLUMNS: &str = "machine_pubkey, identity_pubkey, box_pubkey, last_seen, created_at";

fn machine_row(row: &Row) -> Machine {
    Machine { machine_pubkey: row.get(0), identity_pubkey: row.get(1), box_pubkey: row.get(2), last_seen: row.get(3), created_at: row.get(4) }
}

const BLOB_COLUMNS: &str = "id, kind, recipient_machine_pubkey, seq, ciphertext, created_at";

fn blob_row(row: &Row) -> BlobRow {
    BlobRow { id: row.get(0), kind: row.get(1), recipient_machine_pubkey: row.get(2), seq: row.get(3), ciphertext: row.get(4), created_at: row.get(5) }
}

/// The identity that opened a live pairing.
async fn pairing_owner(client: &Object, nonce: &str) -> ApiResult<String> {
    let row = client.query_opt("SELECT identity_pubkey, expires_at FROM pairings WHERE nonce = $1", &[&nonce]).await?;
    match row {
        Some(row) if row.get::<_, i64>(1) >= now() => Ok(row.get(0)),
        _ => Err(ApiError::not_found("Unknown or expired pairing")),
    }
}

async fn own_pairing(client: &Object, nonce: &str, identity_pubkey: &str) -> ApiResult<()> {
    if pairing_owner(client, nonce).await? != identity_pubkey {
        return Err(ApiError::forbidden("Not your pairing"));
    }
    Ok(())
}

#[async_trait]
impl Store for Postgres {
    fn describe(&self) -> String {
        format!("postgres {}", self.host)
    }

    async fn register_identity(&self, identity_pubkey: &str, content_pubkey: &str, machine_pubkey: &str, box_pubkey: &str, attestation: &str) -> ApiResult<()> {
        let mut client = self.client().await?;
        let tx = client.transaction().await?;
        lock_identity(&tx, identity_pubkey).await?;
        if tx.query_opt("SELECT 1 FROM revoked_machines WHERE machine_pubkey = $1", &[&machine_pubkey]).await?.is_some() {
            return Err(ApiError::gone("Machine was unpaired"));
        }
        match tx.query_opt("SELECT content_pubkey FROM identities WHERE pubkey = $1", &[&identity_pubkey]).await? {
            Some(row) if row.get::<_, String>(0) != content_pubkey => return Err(ApiError::conflict("Identity exists with a different content key")),
            Some(_) => {}
            None => {
                tx.execute("INSERT INTO identities (pubkey, content_pubkey, created_at) VALUES ($1, $2, $3)", &[&identity_pubkey, &content_pubkey, &now()]).await?;
            }
        }
        tx.execute(
            "INSERT INTO machines (machine_pubkey, identity_pubkey, box_pubkey, attestation, last_seen, created_at)
             VALUES ($1, $2, $3, $4, $5, $5)
             ON CONFLICT (machine_pubkey) DO UPDATE SET box_pubkey = EXCLUDED.box_pubkey, attestation = EXCLUDED.attestation",
            &[&machine_pubkey, &identity_pubkey, &box_pubkey, &attestation, &now()],
        )
        .await?;
        tx.commit().await?;
        Ok(())
    }

    async fn create_challenge(&self, nonce: &str, machine_pubkey: &str, expires_at: i64) -> ApiResult<()> {
        let client = self.client().await?;
        if client.query_opt("SELECT 1 FROM machines WHERE machine_pubkey = $1", &[&machine_pubkey]).await?.is_none() {
            if client.query_opt("SELECT 1 FROM revoked_machines WHERE machine_pubkey = $1", &[&machine_pubkey]).await?.is_some() {
                return Err(ApiError::gone("Machine was unpaired"));
            }
            return Err(ApiError::not_found("Unknown machine"));
        }
        client.execute("INSERT INTO challenges (nonce, machine_pubkey, expires_at) VALUES ($1, $2, $3)", &[&nonce, &machine_pubkey, &expires_at]).await?;
        Ok(())
    }

    async fn redeem_challenge(&self, nonce: &str, machine_pubkey: &str) -> ApiResult<Machine> {
        let client = self.client().await?;
        let Some(challenge) = client.query_opt("DELETE FROM challenges WHERE nonce = $1 RETURNING machine_pubkey, expires_at", &[&nonce]).await? else {
            return Err(ApiError::unauthorized("Unknown challenge"));
        };
        if challenge.get::<_, String>(0) != machine_pubkey || challenge.get::<_, i64>(1) < now() {
            return Err(ApiError::unauthorized("Challenge expired"));
        }
        let row = client
            .query_opt(&format!("UPDATE machines SET last_seen = $1 WHERE machine_pubkey = $2 RETURNING {MACHINE_COLUMNS}"), &[&now(), &machine_pubkey])
            .await?
            .ok_or_else(|| ApiError::not_found("Unknown machine"))?;
        Ok(machine_row(&row))
    }

    async fn machines_for(&self, identity_pubkey: &str) -> ApiResult<Vec<Machine>> {
        let rows = self
            .client()
            .await?
            .query(&format!("SELECT {MACHINE_COLUMNS} FROM machines WHERE identity_pubkey = $1 ORDER BY created_at"), &[&identity_pubkey])
            .await?;
        Ok(rows.iter().map(machine_row).collect())
    }

    async fn touch_machine(&self, machine_pubkey: &str) -> ApiResult<()> {
        self.client().await?.execute("UPDATE machines SET last_seen = $1 WHERE machine_pubkey = $2", &[&now(), &machine_pubkey]).await?;
        Ok(())
    }

    async fn revoke_machine(&self, identity_pubkey: &str, machine_pubkey: &str) -> ApiResult<bool> {
        let mut client = self.client().await?;
        let tx = client.transaction().await?;
        lock_identity(&tx, identity_pubkey).await?;
        let removed = tx.execute("DELETE FROM machines WHERE machine_pubkey = $1 AND identity_pubkey = $2", &[&machine_pubkey, &identity_pubkey]).await?;
        if removed == 0 {
            return Ok(false);
        }
        tx.execute(
            "INSERT INTO revoked_machines (machine_pubkey, identity_pubkey, revoked_at) VALUES ($1, $2, $3)
             ON CONFLICT (machine_pubkey) DO UPDATE SET revoked_at = EXCLUDED.revoked_at",
            &[&machine_pubkey, &identity_pubkey, &now()],
        )
        .await?;
        tx.execute("DELETE FROM challenges WHERE machine_pubkey = $1", &[&machine_pubkey]).await?;
        tx.execute("DELETE FROM push_tokens WHERE machine_pubkey = $1", &[&machine_pubkey]).await?;
        let freed: i64 = tx
            .query_one(
                "WITH gone AS (DELETE FROM blobs WHERE identity_pubkey = $1 AND recipient_machine_pubkey = $2 RETURNING size)
                 SELECT COALESCE(SUM(size), 0)::BIGINT FROM gone",
                &[&identity_pubkey, &machine_pubkey],
            )
            .await?
            .get(0);
        give_back(&tx, identity_pubkey, freed).await?;
        tx.commit().await?;
        Ok(true)
    }

    async fn delete_identity(&self, identity_pubkey: &str) -> ApiResult<DeletedIdentity> {
        let mut client = self.client().await?;
        let tx = client.transaction().await?;
        lock_identity(&tx, identity_pubkey).await?;
        let machines: Vec<String> =
            tx.query("SELECT machine_pubkey FROM machines WHERE identity_pubkey = $1", &[&identity_pubkey]).await?.iter().map(|row| row.get(0)).collect();
        let files: Vec<String> =
            tx.query("SELECT id FROM blobs WHERE identity_pubkey = $1 AND kind = 'file'", &[&identity_pubkey]).await?.iter().map(|row| row.get(0)).collect();
        tx.execute(
            "INSERT INTO revoked_machines (machine_pubkey, identity_pubkey, revoked_at)
             SELECT machine_pubkey, identity_pubkey, $2 FROM machines WHERE identity_pubkey = $1
             ON CONFLICT (machine_pubkey) DO UPDATE SET revoked_at = EXCLUDED.revoked_at",
            &[&identity_pubkey, &now()],
        )
        .await?;
        tx.execute("DELETE FROM challenges WHERE machine_pubkey IN (SELECT machine_pubkey FROM machines WHERE identity_pubkey = $1)", &[&identity_pubkey]).await?;
        for table in ["push_tokens", "pairings", "blobs", "deleted_groups", "sequences", "usage", "machines"] {
            tx.execute(&format!("DELETE FROM {table} WHERE identity_pubkey = $1"), &[&identity_pubkey]).await?;
        }
        tx.execute("DELETE FROM identities WHERE pubkey = $1", &[&identity_pubkey]).await?;
        tx.commit().await?;
        Ok(DeletedIdentity { machines, files })
    }

    async fn orphans(&self, files: &[(String, String)]) -> ApiResult<Vec<(String, String)>> {
        let (identities, ids): (Vec<&str>, Vec<&str>) = files.iter().map(|(identity, id)| (identity.as_str(), id.as_str())).unzip();
        let rows = self
            .client()
            .await?
            .query(
                "SELECT f.identity, f.id FROM unnest($1::text[], $2::text[]) AS f(identity, id)
                 WHERE EXISTS (SELECT 1 FROM identities i WHERE i.pubkey = f.identity)
                   AND NOT EXISTS (SELECT 1 FROM blobs b WHERE b.identity_pubkey = f.identity AND b.id = f.id)",
                &[&identities, &ids],
            )
            .await?;
        Ok(rows.iter().map(|row| (row.get(0), row.get(1))).collect())
    }

    async fn stats(&self) -> ApiResult<Stats> {
        let client = self.client().await?;
        let totals = client
            .query_one(
                "SELECT (SELECT COUNT(*) FROM identities), (SELECT COUNT(*) FROM machines),
                        (SELECT COUNT(*) FROM machines WHERE last_seen > $1), (SELECT COUNT(*) FROM machines WHERE last_seen > $2),
                        (SELECT COUNT(*) FROM machines WHERE last_seen > $3), (SELECT COUNT(*) FROM revoked_machines),
                        (SELECT COUNT(*) FROM deleted_groups), (SELECT COALESCE(SUM(bytes), 0)::BIGINT FROM usage),
                        (SELECT COALESCE(MAX(bytes), 0) FROM usage)",
                &[&(now() - 86_400), &(now() - 7 * 86_400), &(now() - 30 * 86_400)],
            )
            .await?;
        let blobs = client.query("SELECT kind, COUNT(*), COALESCE(SUM(size), 0)::BIGINT FROM blobs GROUP BY kind ORDER BY kind", &[]).await?;
        let push_tokens = client.query("SELECT platform, COUNT(*) FROM push_tokens GROUP BY platform ORDER BY platform", &[]).await?;
        Ok(Stats {
            identities: totals.get(0),
            machines: totals.get(1),
            active_machines: [totals.get(2), totals.get(3), totals.get(4)],
            revoked_machines: totals.get(5),
            deleted_groups: totals.get(6),
            usage_bytes: totals.get(7),
            largest_identity_bytes: totals.get(8),
            blobs: blobs.iter().map(|row| (row.get(0), row.get(1), row.get(2))).collect(),
            push_tokens: push_tokens.iter().map(|row| (row.get(0), row.get(1))).collect(),
        })
    }

    async fn sweep(&self, sealed_before: i64, groups_before: i64) -> ApiResult<u64> {
        let client = self.client().await?;
        // One statement, so the bytes go back with the rows. `usage` takes a relative update,
        // which needs no identity lock.
        let gone = client
            .query_one(
                &format!(
                    "WITH gone AS (
                         DELETE FROM blobs WHERE recipient_machine_pubkey IS NOT NULL AND kind IN ({}) AND created_at < $1
                         RETURNING identity_pubkey, size),
                     freed AS (SELECT identity_pubkey, SUM(size)::BIGINT AS bytes FROM gone GROUP BY identity_pubkey),
                     given AS (
                         UPDATE usage SET bytes = GREATEST(usage.bytes - freed.bytes, 0) FROM freed
                         WHERE usage.identity_pubkey = freed.identity_pubkey)
                     SELECT COUNT(*) FROM gone",
                    sealed_kinds_sql()
                ),
                &[&sealed_before],
            )
            .await?
            .get::<_, i64>(0);
        client.execute("DELETE FROM deleted_groups WHERE deleted_at < $1", &[&groups_before]).await?;
        Ok(gone as u64)
    }

    async fn revoked_machines(&self) -> ApiResult<Vec<String>> {
        let rows = self.client().await?.query("SELECT machine_pubkey FROM revoked_machines", &[]).await?;
        Ok(rows.iter().map(|row| row.get(0)).collect())
    }

    async fn precheck_blob(&self, identity_pubkey: &str, id: &str, group: Option<&str>, size: i64, quota_bytes: u64) -> ApiResult<Option<Inserted>> {
        let client = self.client().await?;
        if let Some(row) = client.query_opt("SELECT seq FROM blobs WHERE id = $1 AND identity_pubkey = $2", &[&id, &identity_pubkey]).await? {
            return Ok(Some(Inserted { seq: row.get(0), existing: true }));
        }
        if let Some(group) = group {
            if client.query_opt("SELECT 1 FROM deleted_groups WHERE identity_pubkey = $1 AND group_id = $2", &[&identity_pubkey, &group]).await?.is_some() {
                return Err(ApiError::conflict("Group was deleted"));
            }
        }
        if quota_bytes > 0 {
            let used: i64 = client.query_opt("SELECT bytes FROM usage WHERE identity_pubkey = $1", &[&identity_pubkey]).await?.map(|row| row.get(0)).unwrap_or(0);
            if (used + size) as u64 > quota_bytes {
                return Err(ApiError::too_large("Storage quota exceeded"));
            }
        }
        Ok(None)
    }

    async fn insert_blob(&self, blob: NewBlob, quota_bytes: u64) -> ApiResult<Inserted> {
        let NewBlob { identity_pubkey, id, kind, recipient_machine_pubkey, slot, group, payload } = &blob;
        let size = payload.size();
        let ciphertext: &[u8] = match payload {
            Payload::Inline(bytes) => bytes,
            Payload::InFileStore { .. } => &[],
        };
        let mut client = self.client().await?;
        let tx = client.transaction().await?;
        lock_identity(&tx, identity_pubkey).await?;
        if let Some(recipient) = recipient_machine_pubkey {
            let owner = tx.query_opt("SELECT identity_pubkey FROM machines WHERE machine_pubkey = $1", &[recipient]).await?;
            if owner.map(|row| row.get::<_, String>(0)).as_ref() != Some(identity_pubkey) {
                return Err(ApiError::bad_request("Recipient is not a machine of this identity"));
            }
        }
        if let Some(row) = tx.query_opt("SELECT seq FROM blobs WHERE id = $1 AND identity_pubkey = $2", &[id, identity_pubkey]).await? {
            return Ok(Inserted { seq: row.get(0), existing: true });
        }
        if let Some(group) = group {
            if tx.query_opt("SELECT 1 FROM deleted_groups WHERE identity_pubkey = $1 AND group_id = $2", &[identity_pubkey, group]).await?.is_some() {
                return Err(ApiError::conflict("Group was deleted"));
            }
        }
        // Before the quota check, so a new version of a message fits where the old one was. A
        // refusal below rolls this back with the rest.
        if let Some(slot) = slot {
            let first: Option<i64> = tx.query_one("SELECT MIN(seq) FROM blobs WHERE identity_pubkey = $1 AND slot = $2", &[identity_pubkey, &slot.name]).await?.get(0);
            if let Some(first) = first {
                let floor = if slot.keep_first { first } else { 0 };
                let freed: i64 = tx
                    .query_one(
                        "WITH gone AS (DELETE FROM blobs WHERE identity_pubkey = $1 AND slot = $2 AND seq > $3 RETURNING size)
                         SELECT COALESCE(SUM(size), 0)::BIGINT FROM gone",
                        &[identity_pubkey, &slot.name, &floor],
                    )
                    .await?
                    .get(0);
                give_back(&tx, identity_pubkey, freed).await?;
            }
        }
        if quota_bytes > 0 {
            let used: i64 = tx.query_opt("SELECT bytes FROM usage WHERE identity_pubkey = $1", &[identity_pubkey]).await?.map(|row| row.get(0)).unwrap_or(0);
            if (used + size) as u64 > quota_bytes {
                return Err(ApiError::too_large("Storage quota exceeded"));
            }
        }
        let seq: i64 = tx
            .query_one(
                "INSERT INTO sequences (identity_pubkey, seq) VALUES ($1, 1)
                 ON CONFLICT (identity_pubkey) DO UPDATE SET seq = sequences.seq + 1 RETURNING seq",
                &[identity_pubkey],
            )
            .await?
            .get(0);
        tx.execute(
            "INSERT INTO blobs (id, identity_pubkey, kind, recipient_machine_pubkey, seq, ciphertext, size, created_at, slot, group_id)
             VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10)",
            &[id, identity_pubkey, kind, recipient_machine_pubkey, &seq, &ciphertext, &size, &now(), &slot.as_ref().map(|s| s.name.as_str()), group],
        )
        .await?;
        tx.execute(
            "INSERT INTO usage (identity_pubkey, bytes) VALUES ($1, $2)
             ON CONFLICT (identity_pubkey) DO UPDATE SET bytes = usage.bytes + EXCLUDED.bytes",
            &[identity_pubkey, &size],
        )
        .await?;
        tx.commit().await?;
        Ok(Inserted { seq, existing: false })
    }

    async fn blobs_since(&self, identity_pubkey: &str, machine_pubkey: &str, since: i64, kinds: &[String], limit: i64, max_bytes: i64) -> ApiResult<(Vec<BlobRow>, i64)> {
        let client = self.client().await?;
        // The window runs over the sizes alone; only the rows that fit are read in full.
        let kind_filter = if kinds.is_empty() { "" } else { "AND kind = ANY($6)" };
        let sql = format!(
            "WITH page AS (
                 SELECT id, (SUM(size) OVER (ORDER BY seq))::BIGINT AS bytes, ROW_NUMBER() OVER (ORDER BY seq) AS place
                 FROM (SELECT id, seq, size FROM blobs
                       WHERE identity_pubkey = $1 AND seq > $2
                         AND (recipient_machine_pubkey IS NULL OR recipient_machine_pubkey = $3) {kind_filter}
                       ORDER BY seq LIMIT $4) candidates)
             SELECT {BLOB_COLUMNS} FROM blobs
             WHERE identity_pubkey = $1 AND id IN (SELECT id FROM page WHERE bytes <= $5 OR place = 1)
             ORDER BY seq"
        );
        let mut values: Vec<&(dyn ToSql + Sync)> = vec![&identity_pubkey, &since, &machine_pubkey, &limit, &max_bytes];
        if !kinds.is_empty() {
            values.push(&kinds);
        }
        let rows = client.query(&sql, &values).await?;
        let head: i64 = client.query_opt("SELECT seq FROM sequences WHERE identity_pubkey = $1", &[&identity_pubkey]).await?.map(|row| row.get(0)).unwrap_or(0);
        Ok((rows.iter().map(blob_row).collect(), head))
    }

    async fn blob(&self, identity_pubkey: &str, machine_pubkey: &str, id: &str) -> ApiResult<Option<BlobRow>> {
        let row = self
            .client()
            .await?
            .query_opt(
                &format!(
                    "SELECT {BLOB_COLUMNS} FROM blobs
                     WHERE id = $1 AND identity_pubkey = $2 AND (recipient_machine_pubkey IS NULL OR recipient_machine_pubkey = $3)"
                ),
                &[&id, &identity_pubkey, &machine_pubkey],
            )
            .await?;
        Ok(row.as_ref().map(blob_row))
    }

    async fn delete_blob(&self, identity_pubkey: &str, id: &str) -> ApiResult<Option<String>> {
        let mut client = self.client().await?;
        let tx = client.transaction().await?;
        lock_identity(&tx, identity_pubkey).await?;
        let Some(row) = tx.query_opt("DELETE FROM blobs WHERE id = $1 AND identity_pubkey = $2 RETURNING size, kind", &[&id, &identity_pubkey]).await? else {
            return Ok(None);
        };
        give_back(&tx, identity_pubkey, row.get(0)).await?;
        tx.commit().await?;
        Ok(Some(row.get(1)))
    }

    async fn delete_group(&self, identity_pubkey: &str, group: &str) -> ApiResult<Vec<String>> {
        let mut client = self.client().await?;
        let tx = client.transaction().await?;
        lock_identity(&tx, identity_pubkey).await?;
        tx.execute(
            "INSERT INTO deleted_groups (identity_pubkey, group_id, deleted_at) VALUES ($1, $2, $3) ON CONFLICT DO NOTHING",
            &[&identity_pubkey, &group, &now()],
        )
        .await?;
        let rows = tx.query("DELETE FROM blobs WHERE identity_pubkey = $1 AND group_id = $2 RETURNING id, kind, size", &[&identity_pubkey, &group]).await?;
        give_back(&tx, identity_pubkey, rows.iter().map(|row| row.get::<_, i64>(2)).sum()).await?;
        tx.commit().await?;
        Ok(rows.iter().filter(|row| row.get::<_, String>(1) == "file").map(|row| row.get(0)).collect())
    }

    async fn set_push_token(&self, identity_pubkey: &str, token: &PushToken) -> ApiResult<()> {
        let mut client = self.client().await?;
        let tx = client.transaction().await?;
        // A token belongs to one install. A phone that paired again has a new machine key and
        // the same token, so the old row goes.
        tx.execute("DELETE FROM push_tokens WHERE token = $1 AND machine_pubkey != $2", &[&token.token, &token.machine_pubkey]).await?;
        tx.execute(
            "INSERT INTO push_tokens (machine_pubkey, identity_pubkey, platform, token, environment, updated_at)
             VALUES ($1, $2, $3, $4, $5, $6)
             ON CONFLICT (machine_pubkey) DO UPDATE SET identity_pubkey = EXCLUDED.identity_pubkey, platform = EXCLUDED.platform,
                 token = EXCLUDED.token, environment = EXCLUDED.environment, updated_at = EXCLUDED.updated_at",
            &[&token.machine_pubkey, &identity_pubkey, &token.platform, &token.token, &token.environment, &now()],
        )
        .await?;
        tx.commit().await?;
        Ok(())
    }

    async fn delete_push_token(&self, machine_pubkey: &str) -> ApiResult<()> {
        self.client().await?.execute("DELETE FROM push_tokens WHERE machine_pubkey = $1", &[&machine_pubkey]).await?;
        Ok(())
    }

    async fn push_tokens_for(&self, identity_pubkey: &str, except_machine: &str) -> ApiResult<Vec<PushToken>> {
        let rows = self
            .client()
            .await?
            .query(
                "SELECT machine_pubkey, platform, token, environment FROM push_tokens WHERE identity_pubkey = $1 AND machine_pubkey != $2",
                &[&identity_pubkey, &except_machine],
            )
            .await?;
        Ok(rows.iter().map(|row| PushToken { machine_pubkey: row.get(0), platform: row.get(1), token: row.get(2), environment: row.get(3) }).collect())
    }

    async fn create_pairing(&self, nonce: &str, identity_pubkey: &str, expires_at: i64) -> ApiResult<()> {
        self.client()
            .await?
            .execute("INSERT INTO pairings (nonce, identity_pubkey, created_at, expires_at) VALUES ($1, $2, $3, $4)", &[&nonce, &identity_pubkey, &now(), &expires_at])
            .await?;
        Ok(())
    }

    async fn delete_pairing(&self, nonce: &str, identity_pubkey: &str) -> ApiResult<()> {
        let client = self.client().await?;
        own_pairing(&client, nonce, identity_pubkey).await?;
        client.execute("DELETE FROM pairings WHERE nonce = $1", &[&nonce]).await?;
        Ok(())
    }

    async fn post_pair_request(&self, nonce: &str, ciphertext: &[u8]) -> ApiResult<()> {
        let client = self.client().await?;
        pairing_owner(&client, nonce).await?;
        if client.execute("UPDATE pairings SET request = $1 WHERE nonce = $2 AND request IS NULL", &[&ciphertext, &nonce]).await? == 0 {
            return Err(ApiError::conflict("Pairing already has a request"));
        }
        Ok(())
    }

    async fn pair_request(&self, nonce: &str, identity_pubkey: &str) -> ApiResult<Option<Vec<u8>>> {
        let client = self.client().await?;
        own_pairing(&client, nonce, identity_pubkey).await?;
        Ok(client.query_one("SELECT request FROM pairings WHERE nonce = $1", &[&nonce]).await?.get(0))
    }

    async fn post_pair_reply(&self, nonce: &str, identity_pubkey: &str, ciphertext: &[u8]) -> ApiResult<()> {
        let client = self.client().await?;
        own_pairing(&client, nonce, identity_pubkey).await?;
        client.execute("UPDATE pairings SET reply = $1 WHERE nonce = $2", &[&ciphertext, &nonce]).await?;
        Ok(())
    }

    async fn pair_reply(&self, nonce: &str) -> ApiResult<Option<Vec<u8>>> {
        let client = self.client().await?;
        pairing_owner(&client, nonce).await?;
        Ok(client.query_one("SELECT reply FROM pairings WHERE nonce = $1", &[&nonce]).await?.get(0))
    }

    async fn tick(&self) -> ApiResult<()> {
        let client = self.client().await?;
        client.execute("DELETE FROM challenges WHERE expires_at < $1", &[&now()]).await?;
        client.execute("DELETE FROM pairings WHERE expires_at < $1", &[&now()]).await?;
        self.beat(&client).await?;
        // A process that stopped beating died with its sockets open. They stop counting, and
        // the Devices of those identities hear that presence changed.
        let orphaned = client
            .query(
                "WITH dead AS (DELETE FROM relay_instances WHERE beat_at <= $1 RETURNING id)
                 DELETE FROM relay_sockets WHERE instance_id IN (SELECT id FROM dead) RETURNING identity_pubkey",
                &[&(now() - INSTANCE_TTL)],
            )
            .await?;
        let identities: HashSet<String> = orphaned.iter().map(|row| row.get(0)).collect();
        for identity in identities {
            self.publish(Event::Machines { identity }).await;
        }
        Ok(())
    }

    async fn socket_opened(&self, identity_pubkey: &str, machine_pubkey: &str, socket_id: u64, _first_here: bool) -> ApiResult<bool> {
        let client = self.client().await?;
        let had = self.has_socket(&client, machine_pubkey).await?;
        client
            .execute(
                "INSERT INTO relay_sockets (instance_id, socket_id, identity_pubkey, machine_pubkey) VALUES ($1, $2, $3, $4) ON CONFLICT DO NOTHING",
                &[&self.instance, &(socket_id as i64), &identity_pubkey, &machine_pubkey],
            )
            .await?;
        Ok(!had)
    }

    async fn socket_closed(&self, _identity_pubkey: &str, machine_pubkey: &str, socket_id: u64, _last_here: bool) -> ApiResult<bool> {
        let client = self.client().await?;
        client.execute("DELETE FROM relay_sockets WHERE instance_id = $1 AND socket_id = $2", &[&self.instance, &(socket_id as i64)]).await?;
        Ok(!self.has_socket(&client, machine_pubkey).await?)
    }

    async fn online(&self, identity_pubkey: &str) -> ApiResult<HashSet<String>> {
        let rows = self
            .client()
            .await?
            .query(
                "SELECT DISTINCT s.machine_pubkey FROM relay_sockets s JOIN relay_instances i ON i.id = s.instance_id
                 WHERE s.identity_pubkey = $1 AND i.beat_at > $2",
                &[&identity_pubkey, &(now() - INSTANCE_TTL)],
            )
            .await?;
        Ok(rows.iter().map(|row| row.get(0)).collect())
    }

    async fn publish(&self, event: Event) {
        let payload = serde_json::to_string(&event).expect("an event is plain JSON");
        let sent = async { self.client().await?.execute("SELECT pg_notify($1, $2)", &[&CHANNEL, &payload]).await.map_err(ApiError::from) }.await;
        if sent.is_err() {
            tracing::warn!(?event, "event not published");
        }
    }

    async fn close(&self) {
        let Ok(client) = self.client().await else { return };
        let gone = client
            .query(
                "WITH me AS (DELETE FROM relay_instances WHERE id = $1)
                 DELETE FROM relay_sockets WHERE instance_id = $1 RETURNING identity_pubkey",
                &[&self.instance],
            )
            .await
            .unwrap_or_default();
        let identities: HashSet<String> = gone.iter().map(|row| row.get(0)).collect();
        for identity in identities {
            self.publish(Event::Machines { identity }).await;
        }
    }
}
