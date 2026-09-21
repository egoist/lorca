use axum::extract::ws::{Message, WebSocket, WebSocketUpgrade};
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
/// The relay pings each sync socket this often, and drops one that has been silent for two
/// rounds: a Mac that went to sleep, a phone that left the network.
const PING_SECONDS: u64 = 25;
/// Ciphertext in one page of `GET /v1/blobs`. A Device that replays a long history gets it in
/// pages this size, so the relay never builds a response out of hundreds of 4 MiB blobs.
const MAX_PAGE_BYTES: i64 = 8 * 1024 * 1024;
/// APNs takes 4 KB in all; the ciphertext rides in it as base64url beside the fixed alert.
const MAX_PUSH_BYTES: usize = 2560;

#[derive(Debug)]
pub struct ApiError {
    status: StatusCode,
    message: String,
    /// Seconds a rate-limited caller should wait; becomes the `Retry-After` header.
    retry_after: Option<u64>,
}

impl ApiError {
    fn new(status: StatusCode, message: &str) -> Self {
        ApiError { status, message: message.into(), retry_after: None }
    }
    pub fn bad_request(message: &str) -> Self {
        Self::new(StatusCode::BAD_REQUEST, message)
    }
    pub fn unauthorized(message: &str) -> Self {
        Self::new(StatusCode::UNAUTHORIZED, message)
    }
    pub fn forbidden(message: &str) -> Self {
        Self::new(StatusCode::FORBIDDEN, message)
    }
    pub fn not_found(message: &str) -> Self {
        Self::new(StatusCode::NOT_FOUND, message)
    }
    pub fn conflict(message: &str) -> Self {
        Self::new(StatusCode::CONFLICT, message)
    }
    /// The machine was unpaired: its key is dead for good, unlike an unknown one.
    pub fn gone(message: &str) -> Self {
        Self::new(StatusCode::GONE, message)
    }
    pub fn too_large(message: &str) -> Self {
        Self::new(StatusCode::PAYLOAD_TOO_LARGE, message)
    }
    pub fn too_many(retry_after: u64) -> Self {
        ApiError { retry_after: Some(retry_after), ..Self::new(StatusCode::TOO_MANY_REQUESTS, "Too many requests") }
    }
    pub fn internal(message: &str) -> Self {
        Self::new(StatusCode::INTERNAL_SERVER_ERROR, message)
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
        let mut response = (self.status, Json(json!({ "error": self.message }))).into_response();
        if let Some(seconds) = self.retry_after {
            response.headers_mut().insert(axum::http::header::RETRY_AFTER, seconds.into());
        }
        response
    }
}

pub type ApiResult<T> = Result<T, ApiError>;

pub fn router(state: AppState) -> Router {
    // Routes anyone can call are limited per IP; the rest are limited per identity in `Auth`.
    let public = Router::new()
        .route("/v1/identities", post(register_identity))
        .route("/v1/auth/challenge", post(auth_challenge))
        .route("/v1/auth/verify", post(auth_verify))
        .route("/v1/pair/{nonce}/request", post(post_pair_request))
        .route("/v1/pair/{nonce}/reply", get(get_pair_reply))
        .route_layer(axum::middleware::from_fn_with_state(state.clone(), crate::limit::per_ip));
    Router::new()
        .route("/", get(root))
        .route("/v1/health", get(health))
        .route("/v1/sync", get(sync_socket))
        .route("/v1/machines", get(list_machines))
        .route("/v1/machines/{machine_pubkey}", axum::routing::delete(revoke_machine))
        .route("/v1/blobs", get(list_blobs).put(put_blob))
        .route("/v1/blobs/{id}", get(get_blob).delete(delete_blob))
        .route("/v1/groups/{group}", axum::routing::delete(delete_group))
        .route("/v1/push", post(send_push))
        .route("/v1/push/token", axum::routing::put(put_push_token).delete(delete_push_token))
        .route("/v1/pair", post(create_pairing))
        .route("/v1/pair/{nonce}", axum::routing::delete(delete_pairing))
        .route("/v1/pair/{nonce}/request", get(get_pair_request))
        .route("/v1/pair/{nonce}/reply", post(post_pair_reply))
        .merge(public)
        .layer(DefaultBodyLimit::max(MAX_BODY_BYTES))
        .with_state(state)
}

/// What a browser opening the relay's address sees.
async fn root() -> &'static str {
    "Lorca Relay is running..."
}

async fn health() -> Json<Value> {
    Json(json!({ "ok": true, "service": "lorca-relay" }))
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
    state.db.register_identity(&identity_pubkey, &body.content_pubkey, &body.machine.machine_pubkey, &body.machine.box_pubkey, &attestation).await?;
    Ok(Json(json!({ "identity_pubkey": identity_pubkey, "machine_pubkey": machine_pubkey })))
}

#[derive(Debug, Deserialize)]
struct ChallengeRequest {
    machine_pubkey: String,
}

async fn auth_challenge(State(state): State<AppState>, Json(body): Json<ChallengeRequest>) -> ApiResult<Json<Value>> {
    let nonce = random_nonce(32);
    state.db.create_challenge(&nonce, &body.machine_pubkey, now() + CHALLENGE_TTL).await?;
    Ok(Json(json!({ "nonce": nonce, "expires_in": CHALLENGE_TTL })))
}

#[derive(Debug, Deserialize)]
struct VerifyRequest {
    machine_pubkey: String,
    nonce: String,
    signature: String,
}

async fn auth_verify(State(state): State<AppState>, Json(body): Json<VerifyRequest>) -> ApiResult<Json<Value>> {
    // Checked before the writer is taken: when every Device signs in again after a restart,
    // the signature checks run side by side instead of one at a time.
    verify_signature(&body.machine_pubkey, body.nonce.as_bytes(), &body.signature)?;
    let machine = state.db.redeem_challenge(&body.nonce, &body.machine_pubkey).await?;
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
    /// The machine has a sync socket open.
    online: bool,
}

async fn list_machines(State(state): State<AppState>, auth: Auth) -> ApiResult<Json<Value>> {
    let online = state.db.online(&auth.identity_pubkey).await?;
    let machines: Vec<MachineOut> = state
        .db
        .machines_for(&auth.identity_pubkey)
        .await?
        .into_iter()
        .map(|m| MachineOut {
            box_pubkey: m.box_pubkey,
            last_seen: m.last_seen,
            created_at: m.created_at,
            online: online.contains(&m.machine_pubkey),
            machine_pubkey: m.machine_pubkey,
        })
        .collect();
    Ok(Json(json!({ "machines": machines, "now": now() })))
}

/// Unpairs one machine of the caller's identity, the caller's own included. Its key never
/// authenticates again, its pending envelopes go, and its sync socket closes. The other
/// Devices are told to refresh their machine list now.
async fn revoke_machine(State(state): State<AppState>, auth: Auth, Path(machine_pubkey): Path<String>) -> ApiResult<StatusCode> {
    if !state.db.revoke_machine(&auth.identity_pubkey, &machine_pubkey).await? {
        return Err(ApiError::not_found("Not a machine of this identity"));
    }
    state.db.publish(db::Event::Revoked { identity: auth.identity_pubkey.clone(), machine: machine_pubkey }).await;
    Ok(StatusCode::NO_CONTENT)
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
    /// See `db::Slot`.
    #[serde(default)]
    slot: Option<String>,
    #[serde(default)]
    keep_first: bool,
    /// What the blob belongs to, a chat to the Devices: `DELETE /v1/groups/{group}` takes
    /// every blob of it.
    #[serde(default)]
    group: Option<String>,
}

/// Ids are client-chosen (uuids, `msg-<uuid>`) and become object keys, so only a plain charset.
fn valid_id(id: &str) -> bool {
    !id.is_empty()
        && id.len() <= 64
        && id != "."
        && id != ".."
        && id.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_' || b == b'.')
}

async fn put_blob(State(state): State<AppState>, auth: Auth, Json(body): Json<PutBlob>) -> ApiResult<Json<Value>> {
    if !db::KINDS.contains(&body.kind.as_str()) {
        return Err(ApiError::bad_request("Unknown blob kind"));
    }
    let id = body.id.unwrap_or_else(|| uuid::Uuid::new_v4().to_string());
    if !valid_id(&id) {
        return Err(ApiError::bad_request("Blob id must be 1–64 characters of [A-Za-z0-9._-]"));
    }
    if let Some(slot) = &body.slot {
        if !valid_id(slot) {
            return Err(ApiError::bad_request("Slot must be 1–64 characters of [A-Za-z0-9._-]"));
        }
        // A superseded `file` would leave its object behind.
        if body.kind == "file" {
            return Err(ApiError::bad_request("A file blob takes no slot"));
        }
    }
    if body.group.as_deref().is_some_and(|group| !valid_id(group)) {
        return Err(ApiError::bad_request("Group must be 1–64 characters of [A-Za-z0-9._-]"));
    }
    let max = if body.kind == "file" { MAX_FILE_BLOB_BYTES } else { MAX_BLOB_BYTES };
    // Decoding a 24 MB attachment is work for the blocking pool, and it happens before the
    // writer is taken so other writes are not held up by it.
    let ciphertext = db::blocking(move || b64url_decode(&body.ciphertext)).await?;
    if ciphertext.is_empty() || ciphertext.len() > max {
        return Err(ApiError::bad_request("Ciphertext size out of range"));
    }

    let quota = state.quota_bytes;
    let blob = |payload| db::NewBlob {
        identity_pubkey: auth.identity_pubkey.clone(),
        id: id.clone(),
        kind: body.kind.clone(),
        recipient_machine_pubkey: body.recipient_machine_pubkey.clone(),
        slot: body.slot.clone().map(|name| db::Slot { name, keep_first: body.keep_first }),
        group: body.group.clone(),
        payload,
    };

    let inserted = match body.kind.as_str() {
        "file" => {
            // The object goes up before the row. A cheap check first saves an upload the row
            // would refuse; the insert decides for real.
            let store = &state.file_store;
            let size = ciphertext.len() as i64;
            if let Some(existing) = state.db.precheck_blob(&auth.identity_pubkey, &id, body.group.as_deref(), size, quota).await? {
                return Ok(Json(json!({ "id": id, "seq": existing.seq, "existing": true })));
            }
            let key = crate::store::key(&auth.identity_pubkey, &id);
            store.put(&key, ciphertext).await?;
            match state.db.insert_blob(blob(db::Payload::InFileStore { size }), quota).await {
                Ok(inserted) => inserted,
                Err(error) => {
                    // The row was refused, so the object is an orphan. A concurrent put of the
                    // same id would have returned `existing` instead, so it is only ours.
                    if let Err(cleanup) = store.delete(&key).await {
                        tracing::warn!(?cleanup, key, "removing an orphaned file object");
                    }
                    return Err(error);
                }
            }
        }
        _ => state.db.insert_blob(blob(db::Payload::Inline(ciphertext)), quota).await?,
    };
    if inserted.existing {
        return Ok(Json(json!({ "id": id, "seq": inserted.seq, "existing": true })));
    }
    state.db.publish(db::Event::Blobs { identity: auth.identity_pubkey.clone(), recipient: body.recipient_machine_pubkey.clone() }).await;
    Ok(Json(json!({ "id": id, "seq": inserted.seq })))
}

/// Fills in the bytes of `file` rows from the file store.
async fn load_files(state: &AppState, identity_pubkey: &str, rows: &mut [db::BlobRow]) -> ApiResult<()> {
    for row in rows.iter_mut().filter(|row| row.in_file_store()) {
        let key = crate::store::key(identity_pubkey, &row.id);
        row.ciphertext = state.file_store.get(&key).await?.ok_or_else(|| {
            tracing::error!(key, "file blob's object is missing");
            ApiError::internal("File object missing")
        })?;
    }
    Ok(())
}

#[derive(Debug, Deserialize)]
struct ListBlobs {
    #[serde(default)]
    since: i64,
    #[serde(default)]
    kinds: Option<String>,
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

/// A page of the identity's log after `since`. A Device pulls pages until one comes back
/// empty, and again whenever its sync socket says `blobs`.
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
    let (mut rows, head) = state.db.blobs_since(&auth.identity_pubkey, &auth.machine_pubkey, query.since, &kinds, limit, MAX_PAGE_BYTES).await?;
    load_files(&state, &auth.identity_pubkey, &mut rows).await?;
    let blobs: Vec<BlobOut> = rows.into_iter().map(BlobOut::from).collect();
    Ok(Json(json!({ "blobs": blobs, "seq": head })))
}

// MARK: - Sync socket

/// The machine's sync socket. It is online while this is open; `last_seen` is written when it
/// comes and when it goes, and the identity's other sockets hear `machines` both times.
async fn sync_socket(State(state): State<AppState>, auth: Auth, upgrade: WebSocketUpgrade) -> Response {
    upgrade.on_upgrade(move |socket| serve_socket(state, auth, socket))
}

async fn touch(state: &AppState, machine_pubkey: &str) {
    if let Err(error) = state.db.touch_machine(machine_pubkey).await {
        tracing::warn!(?error, "writing last_seen");
    }
}

async fn serve_socket(state: AppState, auth: Auth, mut socket: WebSocket) {
    let (identity, machine) = (&auth.identity_pubkey, &auth.machine_pubkey);
    let mut seat = state.local.hub.join(identity, machine);
    // A backend that cannot answer leaves presence to what this process knows.
    let came_online = state.db.socket_opened(identity, machine, seat.id, seat.came_online).await.unwrap_or(seat.came_online);
    if came_online {
        touch(&state, machine).await;
        state.db.publish(db::Event::Machines { identity: identity.clone() }).await;
    }
    let mut ping = tokio::time::interval(std::time::Duration::from_secs(PING_SECONDS));
    ping.tick().await;
    let mut heard = tokio::time::Instant::now();
    loop {
        tokio::select! {
            // The process is stopping: the Device connects to the one that replaces it.
            _ = state.stopping.cancelled() => break,
            signal = seat.signals.recv() => {
                // The hub dropped this seat: the machine was unpaired.
                let Some(signal) = signal else { break };
                if socket.send(Message::Text(signal.json().into())).await.is_err() {
                    break;
                }
            }
            _ = ping.tick() => {
                if heard.elapsed().as_secs() > PING_SECONDS * 2 || socket.send(Message::Ping(Default::default())).await.is_err() {
                    break;
                }
            }
            message = socket.recv() => match message {
                Some(Ok(Message::Close(_))) | Some(Err(_)) | None => break,
                Some(Ok(_)) => heard = tokio::time::Instant::now(),
            },
        }
    }
    let last_here = state.local.hub.leave(identity, seat.id);
    let went_offline = state.db.socket_closed(identity, machine, seat.id, last_here).await.unwrap_or(last_here);
    // A process that is stopping says so once per identity in `Store::close`, and its Devices
    // are about to connect to the one that replaces it.
    if went_offline && !state.local.revoked.contains(machine) && !state.stopping.is_cancelled() {
        touch(&state, machine).await;
        state.db.publish(db::Event::Machines { identity: identity.clone() }).await;
    }
}

/// One blob by id, for kinds a Device does not take in its poll: a `file` is fetched when a
/// transcript needs it, by the Runner that runs the turn and by Devices that show it.
async fn get_blob(State(state): State<AppState>, auth: Auth, Path(id): Path<String>) -> ApiResult<Json<BlobOut>> {
    let row = state.db.blob(&auth.identity_pubkey, &auth.machine_pubkey, &id).await?;
    let Some(row) = row else { return Err(ApiError::not_found("No such blob")) };
    let mut rows = [row];
    load_files(&state, &auth.identity_pubkey, &mut rows).await?;
    let [row] = rows;
    Ok(Json(BlobOut::from(row)))
}

async fn delete_blob(State(state): State<AppState>, auth: Auth, Path(id): Path<String>) -> ApiResult<StatusCode> {
    let deleted = state.db.delete_blob(&auth.identity_pubkey, &id).await?;
    let Some(kind) = deleted else { return Err(ApiError::not_found("No such blob")) };
    if kind == "file" {
        // The row is gone either way; a leftover object is logged, not surfaced.
        let key = crate::store::key(&auth.identity_pubkey, &id);
        if let Err(error) = state.file_store.delete(&key).await {
            tracing::warn!(?error, key, "deleting a file object");
        }
    }
    Ok(StatusCode::NO_CONTENT)
}

/// Deletes a group: a chat's messages, read marks, and attachments. The group stays deleted,
/// so the call is good to repeat and a late blob of the chat is refused. The Devices learn of
/// the deletion from the roster; nothing here wakes them.
async fn delete_group(State(state): State<AppState>, auth: Auth, Path(group): Path<String>) -> ApiResult<StatusCode> {
    if !valid_id(&group) {
        return Err(ApiError::bad_request("Group must be 1–64 characters of [A-Za-z0-9._-]"));
    }
    let files = state.db.delete_group(&auth.identity_pubkey, &group).await?;
    for id in files {
        // The rows are gone either way; a leftover object is logged, not surfaced.
        let key = crate::store::key(&auth.identity_pubkey, &id);
        if let Err(error) = state.file_store.delete(&key).await {
            tracing::warn!(?error, key, "deleting a file object");
        }
    }
    Ok(StatusCode::NO_CONTENT)
}

// MARK: - Push

#[derive(Debug, Deserialize)]
struct PutPushToken {
    /// `apns` or `fcm`.
    platform: String,
    token: String,
    /// `sandbox` for a development build's APNs token.
    #[serde(default)]
    environment: Option<String>,
}

/// A phone says where its pushes go. The token names an app install to Apple or Google and
/// nothing about the account.
async fn put_push_token(State(state): State<AppState>, auth: Auth, Json(body): Json<PutPushToken>) -> ApiResult<StatusCode> {
    if !matches!(body.platform.as_str(), "apns" | "fcm") {
        return Err(ApiError::bad_request("platform must be apns or fcm"));
    }
    let valid = match body.platform.as_str() {
        "apns" => !body.token.is_empty() && body.token.len() <= 200 && body.token.bytes().all(|b| b.is_ascii_hexdigit()),
        _ => !body.token.is_empty() && body.token.len() <= 4096 && body.token.bytes().all(|b| b.is_ascii_graphic()),
    };
    if !valid {
        return Err(ApiError::bad_request("Not a device token"));
    }
    let environment = match body.environment.as_deref() {
        Some("sandbox") => "sandbox",
        _ => "production",
    };
    let token = db::PushToken { machine_pubkey: auth.machine_pubkey.clone(), platform: body.platform, token: body.token, environment: environment.into() };
    state.db.set_push_token(&auth.identity_pubkey, &token).await?;
    Ok(StatusCode::NO_CONTENT)
}

async fn delete_push_token(State(state): State<AppState>, auth: Auth) -> ApiResult<StatusCode> {
    state.db.delete_push_token(&auth.machine_pubkey).await?;
    Ok(StatusCode::NO_CONTENT)
}

/// A Device asks for a push to the identity's phones. The relay forwards ciphertext it
/// cannot read; delivery happens after the answer, so a slow APNs never holds a Runner.
async fn send_push(State(state): State<AppState>, auth: Auth, Json(body): Json<Ciphertext>) -> ApiResult<Json<Value>> {
    let ciphertext = b64url_decode(&body.ciphertext)?;
    if ciphertext.is_empty() || ciphertext.len() > MAX_PUSH_BYTES {
        return Err(ApiError::bad_request("Push size out of range"));
    }
    let tokens: Vec<db::PushToken> = state
        .db
        .push_tokens_for(&auth.identity_pubkey, &auth.machine_pubkey)
        .await?
        .into_iter()
        .filter(|token| state.pusher.takes(&token.platform))
        .collect();
    let queued = tokens.len();
    tokio::spawn(async move {
        for token in tokens {
            match state.pusher.send(&token, &ciphertext).await {
                crate::push::Delivery::Sent => {}
                crate::push::Delivery::Gone => {
                    if let Err(error) = state.db.delete_push_token(&token.machine_pubkey).await {
                        tracing::warn!(?error, "forgetting a dead push token");
                    }
                }
                crate::push::Delivery::Failed(error) => tracing::warn!(%error, platform = %token.platform, "push"),
            }
        }
    });
    Ok(Json(json!({ "queued": queued })))
}

// MARK: - Pairing mailbox

async fn create_pairing(State(state): State<AppState>, auth: Auth) -> ApiResult<Json<Value>> {
    let nonce = random_nonce(16);
    let expires_at = now() + PAIRING_TTL;
    state.db.create_pairing(&nonce, &auth.identity_pubkey, expires_at).await?;
    Ok(Json(json!({ "nonce": nonce, "expires_at": expires_at })))
}

/// The identity retires a pairing it no longer waits on. The mailbox goes, so a Device still
/// polling it learns the code is dead instead of waiting out the TTL.
async fn delete_pairing(State(state): State<AppState>, auth: Auth, Path(nonce): Path<String>) -> ApiResult<StatusCode> {
    state.db.delete_pairing(&nonce, &auth.identity_pubkey).await?;
    Ok(StatusCode::NO_CONTENT)
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
    state.db.post_pair_request(&nonce, &ciphertext).await?;
    Ok(StatusCode::NO_CONTENT)
}

async fn get_pair_request(State(state): State<AppState>, auth: Auth, Path(nonce): Path<String>) -> ApiResult<Json<Value>> {
    let request = state.db.pair_request(&nonce, &auth.identity_pubkey).await?;
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
    state.db.post_pair_reply(&nonce, &auth.identity_pubkey, &ciphertext).await?;
    Ok(StatusCode::NO_CONTENT)
}

/// The joining Device polls for the sealed reply. No auth; only its box key can open it.
async fn get_pair_reply(State(state): State<AppState>, Path(nonce): Path<String>) -> ApiResult<Json<Value>> {
    let reply = state.db.pair_reply(&nonce).await?;
    Ok(Json(json!({ "ciphertext": reply.map(|bytes| b64url_encode(&bytes)) })))
}
