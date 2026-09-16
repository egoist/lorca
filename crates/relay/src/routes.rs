use axum::extract::{DefaultBodyLimit, Path, Query, State};
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};
use axum::{Json, Router};
use rand::RngCore;
use rusqlite::{params, OptionalExtension};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

use crate::auth::{b64url_decode, b64url_encode, issue_token, verify_signature, Auth, SignedRequest};
use crate::db::{self, now};
use crate::AppState;

const MAX_BLOB_BYTES: usize = 4 * 1024 * 1024;
/// `file` blobs carry attachments: an encrypted photo or document a Device sent with a message.
const MAX_FILE_BLOB_BYTES: usize = 24 * 1024 * 1024;
/// Room for a `file` blob as base64url inside its JSON body.
const MAX_BODY_BYTES: usize = 40 * 1024 * 1024;
const CHALLENGE_TTL: i64 = 120;
const PAIRING_TTL: i64 = 10 * 60;
const MAX_WAIT_SECONDS: u64 = 30;

#[derive(Debug)]
pub struct ApiError {
    status: StatusCode,
    message: String,
}

impl ApiError {
    pub fn bad_request(message: &str) -> Self {
        ApiError { status: StatusCode::BAD_REQUEST, message: message.into() }
    }
    pub fn unauthorized(message: &str) -> Self {
        ApiError { status: StatusCode::UNAUTHORIZED, message: message.into() }
    }
    pub fn forbidden(message: &str) -> Self {
        ApiError { status: StatusCode::FORBIDDEN, message: message.into() }
    }
    pub fn not_found(message: &str) -> Self {
        ApiError { status: StatusCode::NOT_FOUND, message: message.into() }
    }
    pub fn conflict(message: &str) -> Self {
        ApiError { status: StatusCode::CONFLICT, message: message.into() }
    }
}

impl From<rusqlite::Error> for ApiError {
    fn from(error: rusqlite::Error) -> Self {
        tracing::error!(%error, "database");
        ApiError { status: StatusCode::INTERNAL_SERVER_ERROR, message: "Database error".into() }
    }
}

impl IntoResponse for ApiError {
    fn into_response(self) -> Response {
        (self.status, Json(json!({ "error": self.message }))).into_response()
    }
}

type ApiResult<T> = Result<T, ApiError>;

pub fn router(state: AppState) -> Router {
    Router::new()
        .route("/v1/health", get(health))
        .route("/v1/identities", post(register_identity))
        .route("/v1/auth/challenge", post(auth_challenge))
        .route("/v1/auth/verify", post(auth_verify))
        .route("/v1/machines", get(list_machines))
        .route("/v1/blobs", get(list_blobs).put(put_blob))
        .route("/v1/blobs/{id}", get(get_blob).delete(delete_blob))
        .route("/v1/pair", post(create_pairing))
        .route("/v1/pair/{nonce}/request", post(post_pair_request).get(get_pair_request))
        .route("/v1/pair/{nonce}/reply", post(post_pair_reply).get(get_pair_reply))
        .layer(DefaultBodyLimit::max(MAX_BODY_BYTES))
        .with_state(state)
}

async fn health() -> Json<Value> {
    Json(json!({ "ok": true, "service": "tinybot-relay" }))
}

// MARK: - Identities and machines

#[derive(Debug, Deserialize)]
struct MachineKeys {
    machine_pubkey: String,
    box_pubkey: String,
}

#[derive(Debug, Deserialize)]
struct RegisterIdentity {
    #[allow(dead_code)]
    identity_pubkey: String,
    content_pubkey: String,
    machine: MachineKeys,
    #[allow(dead_code)]
    ts: i64,
}

/// Registers an identity (idempotent) and attests one machine for it. Signed by the identity
/// key. Used at identity creation, restore, and for each Device the identity pairs.
async fn register_identity(State(state): State<AppState>, Json(signed): Json<SignedRequest>) -> ApiResult<Json<Value>> {
    let (body, identity_pubkey): (RegisterIdentity, String) = signed.verify()?;
    crate::auth::verifying_key(&body.machine.machine_pubkey)?;
    if b64url_decode(&body.machine.box_pubkey)?.len() != 32 {
        return Err(ApiError::bad_request("box_pubkey must be 32 bytes"));
    }

    let db = state.db.lock().unwrap();
    let existing: Option<String> = db
        .query_row("SELECT content_pubkey FROM identities WHERE pubkey = ?1", params![identity_pubkey], |row| row.get(0))
        .optional()?;
    match existing {
        Some(content) if content != body.content_pubkey => {
            return Err(ApiError::conflict("Identity exists with a different content key"))
        }
        Some(_) => {}
        None => {
            db.execute(
                "INSERT INTO identities (pubkey, content_pubkey, created_at) VALUES (?1, ?2, ?3)",
                params![identity_pubkey, body.content_pubkey, now()],
            )?;
        }
    }

    let attestation = serde_json::to_string(&json!({ "payload": signed.payload, "signature": signed.signature })).unwrap();
    db.execute(
        "INSERT INTO machines (machine_pubkey, identity_pubkey, box_pubkey, attestation, last_seen, created_at)
         VALUES (?1, ?2, ?3, ?4, ?5, ?5)
         ON CONFLICT(machine_pubkey) DO UPDATE SET box_pubkey = excluded.box_pubkey, attestation = excluded.attestation",
        params![body.machine.machine_pubkey, identity_pubkey, body.machine.box_pubkey, attestation, now()],
    )?;

    Ok(Json(json!({ "identity_pubkey": identity_pubkey, "machine_pubkey": body.machine.machine_pubkey })))
}

#[derive(Debug, Deserialize)]
struct ChallengeRequest {
    machine_pubkey: String,
}

async fn auth_challenge(State(state): State<AppState>, Json(body): Json<ChallengeRequest>) -> ApiResult<Json<Value>> {
    let db = state.db.lock().unwrap();
    db::expire(&db)?;
    if db::machine(&db, &body.machine_pubkey)?.is_none() {
        return Err(ApiError::not_found("Unknown machine"));
    }
    let mut bytes = [0u8; 32];
    rand::thread_rng().fill_bytes(&mut bytes);
    let nonce = b64url_encode(&bytes);
    db.execute(
        "INSERT INTO challenges (nonce, machine_pubkey, expires_at) VALUES (?1, ?2, ?3)",
        params![nonce, body.machine_pubkey, now() + CHALLENGE_TTL],
    )?;
    Ok(Json(json!({ "nonce": nonce, "expires_in": CHALLENGE_TTL })))
}

#[derive(Debug, Deserialize)]
struct VerifyRequest {
    machine_pubkey: String,
    nonce: String,
    signature: String,
}

async fn auth_verify(State(state): State<AppState>, Json(body): Json<VerifyRequest>) -> ApiResult<Json<Value>> {
    let db = state.db.lock().unwrap();
    let row: Option<(String, i64)> = db
        .query_row("SELECT machine_pubkey, expires_at FROM challenges WHERE nonce = ?1", params![body.nonce], |row| {
            Ok((row.get(0)?, row.get(1)?))
        })
        .optional()?;
    let Some((machine_pubkey, expires_at)) = row else { return Err(ApiError::unauthorized("Unknown challenge")) };
    db.execute("DELETE FROM challenges WHERE nonce = ?1", params![body.nonce])?;
    if machine_pubkey != body.machine_pubkey || expires_at < now() {
        return Err(ApiError::unauthorized("Challenge expired"));
    }
    verify_signature(&body.machine_pubkey, body.nonce.as_bytes(), &body.signature)?;
    let machine = db::machine(&db, &body.machine_pubkey)?.ok_or_else(|| ApiError::not_found("Unknown machine"))?;
    db::touch_machine(&db, &machine.machine_pubkey)?;
    let (token, token_expires) = issue_token(&state.secret, &machine.identity_pubkey, &machine.machine_pubkey);
    Ok(Json(json!({
        "token": token,
        "expires_at": token_expires,
        "identity_pubkey": machine.identity_pubkey,
        "machine_pubkey": machine.machine_pubkey,
    })))
}

#[derive(Debug, Serialize)]
struct MachineOut {
    machine_pubkey: String,
    box_pubkey: String,
    last_seen: i64,
    created_at: i64,
}

async fn list_machines(State(state): State<AppState>, auth: Auth) -> ApiResult<Json<Value>> {
    let db = state.db.lock().unwrap();
    let machines: Vec<MachineOut> = db::machines_for(&db, &auth.identity_pubkey)?
        .into_iter()
        .map(|m| MachineOut {
            machine_pubkey: m.machine_pubkey,
            box_pubkey: m.box_pubkey,
            last_seen: m.last_seen,
            created_at: m.created_at,
        })
        .collect();
    Ok(Json(json!({ "machines": machines, "now": now() })))
}

// MARK: - Blobs

#[derive(Debug, Deserialize)]
struct PutBlob {
    #[serde(default)]
    id: Option<String>,
    kind: String,
    #[serde(default)]
    recipient_machine_pubkey: Option<String>,
    /// base64url ciphertext
    ciphertext: String,
}

async fn put_blob(State(state): State<AppState>, auth: Auth, Json(body): Json<PutBlob>) -> ApiResult<Json<Value>> {
    if !db::KINDS.contains(&body.kind.as_str()) {
        return Err(ApiError::bad_request("Unknown blob kind"));
    }
    let ciphertext = b64url_decode(&body.ciphertext)?;
    let max = if body.kind == "file" { MAX_FILE_BLOB_BYTES } else { MAX_BLOB_BYTES };
    if ciphertext.is_empty() || ciphertext.len() > max {
        return Err(ApiError::bad_request("Ciphertext size out of range"));
    }
    let id = body.id.unwrap_or_else(|| uuid::Uuid::new_v4().to_string());
    if id.len() > 64 {
        return Err(ApiError::bad_request("Blob id too long"));
    }

    let mut db = state.db.lock().unwrap();
    if let Some(recipient) = &body.recipient_machine_pubkey {
        match db::machine(&db, recipient)? {
            Some(machine) if machine.identity_pubkey == auth.identity_pubkey => {}
            _ => return Err(ApiError::bad_request("Recipient is not a machine of this identity")),
        }
    }

    let tx = db.transaction()?;
    let existing: Option<i64> = tx
        .query_row("SELECT seq FROM blobs WHERE id = ?1 AND identity_pubkey = ?2", params![id, auth.identity_pubkey], |row| {
            row.get(0)
        })
        .optional()?;
    if let Some(seq) = existing {
        tx.commit()?;
        return Ok(Json(json!({ "id": id, "seq": seq, "existing": true })));
    }
    let seq = db::next_seq(&tx, &auth.identity_pubkey)?;
    tx.execute(
        "INSERT INTO blobs (id, identity_pubkey, kind, recipient_machine_pubkey, seq, ciphertext, size, created_at)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
        params![id, auth.identity_pubkey, body.kind, body.recipient_machine_pubkey, seq, ciphertext, ciphertext.len() as i64, now()],
    )?;
    tx.commit()?;
    drop(db);

    state.notify.notify_waiters();
    Ok(Json(json!({ "id": id, "seq": seq })))
}

#[derive(Debug, Deserialize)]
struct ListBlobs {
    #[serde(default)]
    since: i64,
    #[serde(default)]
    kinds: Option<String>,
    #[serde(default)]
    wait: Option<u64>,
    #[serde(default)]
    limit: Option<i64>,
}

#[derive(Debug, Serialize)]
struct BlobOut {
    id: String,
    kind: String,
    recipient_machine_pubkey: Option<String>,
    seq: i64,
    ciphertext: String,
    created_at: i64,
}

async fn list_blobs(State(state): State<AppState>, auth: Auth, Query(query): Query<ListBlobs>) -> ApiResult<Json<Value>> {
    let kinds: Vec<String> = query
        .kinds
        .as_deref()
        .unwrap_or("")
        .split(',')
        .map(str::trim)
        .filter(|k| !k.is_empty())
        .map(str::to_string)
        .collect();
    let limit = query.limit.unwrap_or(200).clamp(1, 500);
    let wait = query.wait.unwrap_or(0).min(MAX_WAIT_SECONDS);
    let deadline = tokio::time::Instant::now() + std::time::Duration::from_secs(wait);

    loop {
        let (rows, head) = {
            let db = state.db.lock().unwrap();
            let rows = db::blobs_since(&db, &auth.identity_pubkey, &auth.machine_pubkey, query.since, &kinds, limit)?;
            (rows, db::current_seq(&db, &auth.identity_pubkey)?)
        };
        if !rows.is_empty() || wait == 0 || tokio::time::Instant::now() >= deadline {
            let blobs: Vec<BlobOut> = rows
                .into_iter()
                .map(|row| BlobOut {
                    id: row.id,
                    kind: row.kind,
                    recipient_machine_pubkey: row.recipient_machine_pubkey,
                    seq: row.seq,
                    ciphertext: b64url_encode(&row.ciphertext),
                    created_at: row.created_at,
                })
                .collect();
            return Ok(Json(json!({ "blobs": blobs, "seq": head })));
        }
        let remaining = deadline.saturating_duration_since(tokio::time::Instant::now());
        let _ = tokio::time::timeout(remaining.min(std::time::Duration::from_secs(2)), state.notify.notified()).await;
    }
}

/// One blob by id, for kinds a Device does not take in its poll: a `file` is fetched when a
/// transcript needs it, by the Runner that runs the turn and by Devices that show it.
async fn get_blob(State(state): State<AppState>, auth: Auth, Path(id): Path<String>) -> ApiResult<Json<BlobOut>> {
    let db = state.db.lock().unwrap();
    let row = db
        .query_row(
            "SELECT id, kind, recipient_machine_pubkey, seq, ciphertext, created_at FROM blobs
             WHERE id = ?1 AND identity_pubkey = ?2
               AND (recipient_machine_pubkey IS NULL OR recipient_machine_pubkey = ?3)",
            params![id, auth.identity_pubkey, auth.machine_pubkey],
            |row| {
                Ok(db::BlobRow {
                    id: row.get(0)?,
                    kind: row.get(1)?,
                    recipient_machine_pubkey: row.get(2)?,
                    seq: row.get(3)?,
                    ciphertext: row.get(4)?,
                    created_at: row.get(5)?,
                })
            },
        )
        .optional()?;
    let Some(row) = row else { return Err(ApiError::not_found("No such blob")) };
    Ok(Json(BlobOut {
        id: row.id,
        kind: row.kind,
        recipient_machine_pubkey: row.recipient_machine_pubkey,
        seq: row.seq,
        ciphertext: b64url_encode(&row.ciphertext),
        created_at: row.created_at,
    }))
}

async fn delete_blob(State(state): State<AppState>, auth: Auth, Path(id): Path<String>) -> ApiResult<StatusCode> {
    let db = state.db.lock().unwrap();
    let changed = db.execute("DELETE FROM blobs WHERE id = ?1 AND identity_pubkey = ?2", params![id, auth.identity_pubkey])?;
    if changed == 0 {
        return Err(ApiError::not_found("No such blob"));
    }
    Ok(StatusCode::NO_CONTENT)
}

// MARK: - Pairing mailbox

async fn create_pairing(State(state): State<AppState>, auth: Auth) -> ApiResult<Json<Value>> {
    let mut bytes = [0u8; 16];
    rand::thread_rng().fill_bytes(&mut bytes);
    let nonce = b64url_encode(&bytes);
    let db = state.db.lock().unwrap();
    db::expire(&db)?;
    db.execute(
        "INSERT INTO pairings (nonce, identity_pubkey, created_at, expires_at) VALUES (?1, ?2, ?3, ?4)",
        params![nonce, auth.identity_pubkey, now(), now() + PAIRING_TTL],
    )?;
    Ok(Json(json!({ "nonce": nonce, "expires_at": now() + PAIRING_TTL })))
}

#[derive(Debug, Deserialize)]
struct Ciphertext {
    ciphertext: String,
}

fn pairing_owner(db: &rusqlite::Connection, nonce: &str) -> ApiResult<String> {
    let row: Option<(String, i64)> = db
        .query_row("SELECT identity_pubkey, expires_at FROM pairings WHERE nonce = ?1", params![nonce], |row| {
            Ok((row.get(0)?, row.get(1)?))
        })
        .optional()?;
    match row {
        Some((identity, expires_at)) if expires_at >= now() => Ok(identity),
        _ => Err(ApiError::not_found("Unknown or expired pairing")),
    }
}

/// The joining Device posts its sealed request. No auth: it has no keys the relay knows yet.
async fn post_pair_request(
    State(state): State<AppState>,
    Path(nonce): Path<String>,
    Json(body): Json<Ciphertext>,
) -> ApiResult<StatusCode> {
    let ciphertext = b64url_decode(&body.ciphertext)?;
    if ciphertext.len() > 64 * 1024 {
        return Err(ApiError::bad_request("Pairing request too large"));
    }
    let db = state.db.lock().unwrap();
    pairing_owner(&db, &nonce)?;
    let changed = db.execute(
        "UPDATE pairings SET request = ?1 WHERE nonce = ?2 AND request IS NULL",
        params![ciphertext, nonce],
    )?;
    if changed == 0 {
        return Err(ApiError::conflict("Pairing already has a request"));
    }
    Ok(StatusCode::NO_CONTENT)
}

async fn get_pair_request(State(state): State<AppState>, auth: Auth, Path(nonce): Path<String>) -> ApiResult<Json<Value>> {
    let db = state.db.lock().unwrap();
    if pairing_owner(&db, &nonce)? != auth.identity_pubkey {
        return Err(ApiError::forbidden("Not your pairing"));
    }
    let request: Option<Vec<u8>> =
        db.query_row("SELECT request FROM pairings WHERE nonce = ?1", params![nonce], |row| row.get(0))?;
    Ok(Json(json!({ "ciphertext": request.map(|bytes| b64url_encode(&bytes)) })))
}

async fn post_pair_reply(
    State(state): State<AppState>,
    auth: Auth,
    Path(nonce): Path<String>,
    Json(body): Json<Ciphertext>,
) -> ApiResult<StatusCode> {
    let ciphertext = b64url_decode(&body.ciphertext)?;
    let db = state.db.lock().unwrap();
    if pairing_owner(&db, &nonce)? != auth.identity_pubkey {
        return Err(ApiError::forbidden("Not your pairing"));
    }
    db.execute("UPDATE pairings SET reply = ?1 WHERE nonce = ?2", params![ciphertext, nonce])?;
    Ok(StatusCode::NO_CONTENT)
}

/// The joining Device polls for the sealed reply. No auth; only its box key can open it.
async fn get_pair_reply(State(state): State<AppState>, Path(nonce): Path<String>) -> ApiResult<Json<Value>> {
    let db = state.db.lock().unwrap();
    pairing_owner(&db, &nonce)?;
    let reply: Option<Vec<u8>> =
        db.query_row("SELECT reply FROM pairings WHERE nonce = ?1", params![nonce], |row| row.get(0))?;
    Ok(Json(json!({ "ciphertext": reply.map(|bytes| b64url_encode(&bytes)) })))
}
