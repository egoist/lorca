use axum::extract::{DefaultBodyLimit, Path, Query, State};
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};
use axum::{Json, Router};
use rand::RngCore;
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
    pub fn too_large(message: &str) -> Self {
        ApiError { status: StatusCode::PAYLOAD_TOO_LARGE, message: message.into() }
    }
    pub fn internal(message: &str) -> Self {
        ApiError { status: StatusCode::INTERNAL_SERVER_ERROR, message: message.into() }
    }
}

impl From<rusqlite::Error> for ApiError {
    fn from(error: rusqlite::Error) -> Self {
        tracing::error!(%error, "database");
        ApiError::internal("Database error")
    }
}

impl IntoResponse for ApiError {
    fn into_response(self) -> Response {
        (self.status, Json(json!({ "error": self.message }))).into_response()
    }
}

pub type ApiResult<T> = Result<T, ApiError>;

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

fn random_nonce(len: usize) -> String {
    let mut bytes = vec![0u8; len];
    rand::thread_rng().fill_bytes(&mut bytes);
    b64url_encode(&bytes)
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
    let attestation = serde_json::to_string(&json!({ "payload": signed.payload, "signature": signed.signature })).unwrap();
    let machine_pubkey = body.machine.machine_pubkey.clone();
    let identity = identity_pubkey.clone();
    state
        .db
        .write(move |db| {
            db::register_identity(
                db,
                &identity,
                &body.content_pubkey,
                &body.machine.machine_pubkey,
                &body.machine.box_pubkey,
                &attestation,
            )
        })
        .await?;
    Ok(Json(json!({ "identity_pubkey": identity_pubkey, "machine_pubkey": machine_pubkey })))
}

#[derive(Debug, Deserialize)]
struct ChallengeRequest {
    machine_pubkey: String,
}

async fn auth_challenge(State(state): State<AppState>, Json(body): Json<ChallengeRequest>) -> ApiResult<Json<Value>> {
    let nonce = random_nonce(32);
    let stored = nonce.clone();
    state.db.write(move |db| db::create_challenge(db, &stored, &body.machine_pubkey, now() + CHALLENGE_TTL)).await?;
    Ok(Json(json!({ "nonce": nonce, "expires_in": CHALLENGE_TTL })))
}

#[derive(Debug, Deserialize)]
struct VerifyRequest {
    machine_pubkey: String,
    nonce: String,
    signature: String,
}

async fn auth_verify(State(state): State<AppState>, Json(body): Json<VerifyRequest>) -> ApiResult<Json<Value>> {
    let machine = state
        .db
        .write(move |db| {
            let Some((machine_pubkey, expires_at)) = db::take_challenge(db, &body.nonce)? else {
                return Err(ApiError::unauthorized("Unknown challenge"));
            };
            if machine_pubkey != body.machine_pubkey || expires_at < now() {
                return Err(ApiError::unauthorized("Challenge expired"));
            }
            verify_signature(&body.machine_pubkey, body.nonce.as_bytes(), &body.signature)?;
            let machine = db::machine(db, &body.machine_pubkey)?.ok_or_else(|| ApiError::not_found("Unknown machine"))?;
            db::touch_machine(db, &machine.machine_pubkey)?;
            Ok(machine)
        })
        .await?;
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
    let machines: Vec<MachineOut> = state
        .db
        .read(move |db| Ok(db::machines_for(db, &auth.identity_pubkey)?))
        .await?
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
    let id = body.id.unwrap_or_else(|| uuid::Uuid::new_v4().to_string());
    if id.len() > 64 {
        return Err(ApiError::bad_request("Blob id too long"));
    }
    let max = if body.kind == "file" { MAX_FILE_BLOB_BYTES } else { MAX_BLOB_BYTES };
    // Decoding a 24 MB attachment is work for the blocking pool, and it happens before the
    // writer is taken so other writes are not held up by it.
    let ciphertext = db::blocking(move || b64url_decode(&body.ciphertext)).await?;
    if ciphertext.is_empty() || ciphertext.len() > max {
        return Err(ApiError::bad_request("Ciphertext size out of range"));
    }

    let identity = auth.identity_pubkey.clone();
    let stored_id = id.clone();
    let quota = state.quota_bytes;
    let inserted = state
        .db
        .write(move |db| {
            db::insert_blob(db, &identity, &stored_id, &body.kind, body.recipient_machine_pubkey.as_deref(), &ciphertext, quota)
        })
        .await?;
    if inserted.existing {
        return Ok(Json(json!({ "id": id, "seq": inserted.seq, "existing": true })));
    }
    state.wakers.wake(&auth.identity_pubkey);
    Ok(Json(json!({ "id": id, "seq": inserted.seq })))
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

impl From<db::BlobRow> for BlobOut {
    fn from(row: db::BlobRow) -> Self {
        BlobOut {
            id: row.id,
            kind: row.kind,
            recipient_machine_pubkey: row.recipient_machine_pubkey,
            seq: row.seq,
            ciphertext: b64url_encode(&row.ciphertext),
            created_at: row.created_at,
        }
    }
}

/// Long-poll. The identity's waker is armed before each query, so a blob that lands between
/// the query and the wait still wakes this call; there is no periodic re-query.
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
    if kinds.iter().any(|kind| !db::KINDS.contains(&kind.as_str())) {
        return Err(ApiError::bad_request("Unknown blob kind"));
    }
    let limit = query.limit.unwrap_or(200).clamp(1, 500);
    let wait = query.wait.unwrap_or(0).min(MAX_WAIT_SECONDS);
    let deadline = tokio::time::Instant::now() + std::time::Duration::from_secs(wait);
    let waker = state.wakers.waiter(&auth.identity_pubkey);

    loop {
        let notified = waker.notified();
        tokio::pin!(notified);
        notified.as_mut().enable();

        let (identity, machine, kinds) = (auth.identity_pubkey.clone(), auth.machine_pubkey.clone(), kinds.clone());
        let since = query.since;
        let (blobs, head) = state
            .db
            .read(move |db| {
                let rows = db::blobs_since(db, &identity, &machine, since, &kinds, limit)?;
                let blobs: Vec<BlobOut> = rows.into_iter().map(BlobOut::from).collect();
                Ok((blobs, db::current_seq(db, &identity)?))
            })
            .await?;
        let timed_out = tokio::time::Instant::now() >= deadline;
        if !blobs.is_empty() || wait == 0 || timed_out {
            return Ok(Json(json!({ "blobs": blobs, "seq": head })));
        }
        if tokio::time::timeout_at(deadline, notified).await.is_err() {
            return Ok(Json(json!({ "blobs": blobs, "seq": head })));
        }
    }
}

/// One blob by id, for kinds a Device does not take in its poll: a `file` is fetched when a
/// transcript needs it, by the Runner that runs the turn and by Devices that show it.
async fn get_blob(State(state): State<AppState>, auth: Auth, Path(id): Path<String>) -> ApiResult<Json<BlobOut>> {
    let row = state
        .db
        .read(move |db| Ok(db::blob(db, &auth.identity_pubkey, &auth.machine_pubkey, &id)?.map(BlobOut::from)))
        .await?;
    row.map(Json).ok_or_else(|| ApiError::not_found("No such blob"))
}

async fn delete_blob(State(state): State<AppState>, auth: Auth, Path(id): Path<String>) -> ApiResult<StatusCode> {
    let deleted = state.db.write(move |db| Ok(db::delete_blob(db, &auth.identity_pubkey, &id)?)).await?;
    if !deleted {
        return Err(ApiError::not_found("No such blob"));
    }
    Ok(StatusCode::NO_CONTENT)
}

// MARK: - Pairing mailbox

async fn create_pairing(State(state): State<AppState>, auth: Auth) -> ApiResult<Json<Value>> {
    let nonce = random_nonce(16);
    let expires_at = now() + PAIRING_TTL;
    let stored = nonce.clone();
    state.db.write(move |db| Ok(db::create_pairing(db, &stored, &auth.identity_pubkey, expires_at)?)).await?;
    Ok(Json(json!({ "nonce": nonce, "expires_at": expires_at })))
}

#[derive(Debug, Deserialize)]
struct Ciphertext {
    ciphertext: String,
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
    state
        .db
        .write(move |db| {
            db::pairing_owner(db, &nonce)?;
            if !db::set_pairing_request(db, &nonce, &ciphertext)? {
                return Err(ApiError::conflict("Pairing already has a request"));
            }
            Ok(())
        })
        .await?;
    Ok(StatusCode::NO_CONTENT)
}

async fn get_pair_request(State(state): State<AppState>, auth: Auth, Path(nonce): Path<String>) -> ApiResult<Json<Value>> {
    let request = state
        .db
        .read(move |db| {
            if db::pairing_owner(db, &nonce)? != auth.identity_pubkey {
                return Err(ApiError::forbidden("Not your pairing"));
            }
            Ok(db::pairing_request(db, &nonce)?)
        })
        .await?;
    Ok(Json(json!({ "ciphertext": request.map(|bytes| b64url_encode(&bytes)) })))
}

async fn post_pair_reply(
    State(state): State<AppState>,
    auth: Auth,
    Path(nonce): Path<String>,
    Json(body): Json<Ciphertext>,
) -> ApiResult<StatusCode> {
    let ciphertext = b64url_decode(&body.ciphertext)?;
    if ciphertext.len() > 64 * 1024 {
        return Err(ApiError::bad_request("Pairing reply too large"));
    }
    state
        .db
        .write(move |db| {
            if db::pairing_owner(db, &nonce)? != auth.identity_pubkey {
                return Err(ApiError::forbidden("Not your pairing"));
            }
            Ok(db::set_pairing_reply(db, &nonce, &ciphertext)?)
        })
        .await?;
    Ok(StatusCode::NO_CONTENT)
}

/// The joining Device polls for the sealed reply. No auth; only its box key can open it.
async fn get_pair_reply(State(state): State<AppState>, Path(nonce): Path<String>) -> ApiResult<Json<Value>> {
    let reply = state
        .db
        .read(move |db| {
            db::pairing_owner(db, &nonce)?;
            Ok(db::pairing_reply(db, &nonce)?)
        })
        .await?;
    Ok(Json(json!({ "ciphertext": reply.map(|bytes| b64url_encode(&bytes)) })))
}
