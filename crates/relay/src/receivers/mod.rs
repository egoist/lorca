//! Receivers: services' requests the relay takes for routines that run on events. A Runner
//! subscribes one of its routines with a receiver for a subject (a pull request), handing the
//! relay its event subscription's id, generation, and signing secret. The receiver checks what
//! a service sends (GitHub's signature, a webhook's key), turns it into an event about the
//! subject, signs it as the Runner's event inbox expects ([the #84 delivery contract]), seals it
//! to the Runner's box key, and stores it as an `event` blob for that machine alone.
//!
//! A service sends its webhooks in plain text, so a receiver sees each request while it handles
//! it, and keeps none of it: only the subscription's row and the sealed envelope stay.
//!
//! Two receivers: `github`, the Lorca GitHub App, and `webhook`, a routine's own URL and key.
//! Each is on when its settings are given.

pub mod github;
pub mod webhook;

use std::sync::Arc;

use axum::extract::{DefaultBodyLimit, Path, State};
use axum::routing::{delete, get, post};
use axum::{Json, Router};
use hmac::{Hmac, Mac};
use serde::Deserialize;
use serde_json::{json, Value};
use sha2::Sha256;

use crate::auth::{b64url_decode, b64url_encode, Auth};
use crate::db::{self, now, ReceiverSub};
use crate::routes::{ApiError, ApiResult};
use crate::AppState;

/// The subscriptions an identity keeps with one receiver.
const MAX_PER_IDENTITY: i64 = 200;

#[derive(Default)]
pub struct Receivers {
    pub github: Option<github::GithubApp>,
    pub webhook: Option<webhook::Hooks>,
}

impl Receivers {
    pub fn describe(&self) -> String {
        let on: Vec<&str> = [self.github.as_ref().map(|_| "github"), self.webhook.as_ref().map(|_| "webhook")].into_iter().flatten().collect();
        if on.is_empty() { "none".into() } else { on.join(", ") }
    }
}

pub fn router(state: AppState) -> Router<AppState> {
    let public = Router::new()
        .route("/github/callback", get(github::callback))
        .route_layer(axum::middleware::from_fn_with_state(state.clone(), crate::limit::per_ip));
    Router::new()
        .route("/v1/receivers/{receiver}/setup", post(setup))
        .route("/v1/receivers/{receiver}/subscriptions", post(subscribe))
        .route("/v1/receivers/{receiver}/subscriptions/{id}", delete(unsubscribe))
        .route("/v1/receivers/{receiver}/subscriptions/{id}/key", post(set_key))
        // GitHub's deliveries come from its own addresses, many to one: no per-IP limit, and
        // its signature is checked before anything else.
        .route("/github/webhook", post(github::webhook).layer(DefaultBodyLimit::max(github::MAX_DELIVERY_BYTES)))
        .route("/r/{id}", post(webhook::receive).layer(DefaultBodyLimit::max(webhook::MAX_BODY_BYTES)))
        .merge(public)
}

fn unknown(receiver: &str) -> ApiError {
    ApiError::not_found(&format!("This relay has no {receiver} receiver"))
}

/// The link a user follows to set the receiver up for their account: the GitHub App's install.
async fn setup(State(state): State<AppState>, auth: Auth, Path(receiver): Path<String>) -> ApiResult<Json<Value>> {
    match (receiver.as_str(), &state.receivers.github) {
        ("github", Some(app)) => Ok(Json(json!({ "url": app.install_url(&state, &auth.identity_pubkey).await? }))),
        _ => Err(unknown(&receiver)),
    }
}

#[derive(Debug, Deserialize)]
pub struct Subscribe {
    #[serde(default)]
    pub subject: String,
    pub subscription_id: String,
    pub generation: i64,
    pub secret: String,
    #[serde(default)]
    pub key_hash: Option<String>,
}

impl Subscribe {
    fn check(&self) -> ApiResult<()> {
        let id_ok = !self.subscription_id.is_empty() && self.subscription_id.len() <= 128 && self.subscription_id.chars().all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_'));
        if !id_ok || self.generation < 1 || !(16..=128).contains(&self.secret.len()) || self.subject.chars().count() > 200 {
            return Err(ApiError::bad_request("A subscription needs its id, generation, and a secret of 16 to 128 characters"));
        }
        if self.key_hash.as_deref().is_some_and(|hash| !is_hash(hash)) {
            return Err(ApiError::bad_request("A key hash is SHA-256 in base64url"));
        }
        Ok(())
    }

    /// The row this request makes, for the calling machine.
    pub fn row(&self, receiver: &str, auth: &Auth, subject: String, account: Option<String>, state: Option<String>) -> ReceiverSub {
        ReceiverSub {
            id: random_id(),
            receiver: receiver.into(),
            identity_pubkey: auth.identity_pubkey.clone(),
            machine_pubkey: auth.machine_pubkey.clone(),
            subject,
            subscription_id: self.subscription_id.clone(),
            generation: self.generation,
            secret: self.secret.clone(),
            key_hash: self.key_hash.clone(),
            account,
            state,
            created_at: now(),
        }
    }
}

/// Subscribes the calling machine's routine with the receiver. Answers `subscribed` with the
/// row's id (and a webhook's URL), `needs_setup` with the link the user follows first, or
/// `refused` with why.
async fn subscribe(State(state): State<AppState>, auth: Auth, Path(receiver): Path<String>, Json(body): Json<Subscribe>) -> ApiResult<Json<Value>> {
    body.check()?;
    let answer = match receiver.as_str() {
        "github" => state.receivers.github.as_ref().ok_or_else(|| unknown(&receiver))?.subscribe(&state, &auth, &body).await?,
        "webhook" => state.receivers.webhook.as_ref().ok_or_else(|| unknown(&receiver))?.subscribe(&state, &auth, &body).await?,
        _ => return Err(unknown(&receiver)),
    };
    Ok(Json(answer))
}

/// Saves a subscription, within what an identity keeps.
pub async fn save(state: &AppState, sub: &ReceiverSub) -> ApiResult<()> {
    state.db.receiver_subscribe(sub, MAX_PER_IDENTITY).await
}

async fn unsubscribe(State(state): State<AppState>, auth: Auth, Path((receiver, id)): Path<(String, String)>) -> ApiResult<Json<Value>> {
    let removed = state.db.receiver_unsubscribe(&auth.identity_pubkey, &receiver, &id).await?;
    Ok(Json(json!({ "removed": removed })))
}

#[derive(Debug, Deserialize)]
struct SetKey {
    key_hash: String,
}

/// A new key for a webhook: its hash replaces the old one, which stops working at once.
async fn set_key(State(state): State<AppState>, auth: Auth, Path((receiver, id)): Path<(String, String)>, Json(body): Json<SetKey>) -> ApiResult<Json<Value>> {
    if receiver != "webhook" || state.receivers.webhook.is_none() {
        return Err(unknown(&receiver));
    }
    if !is_hash(&body.key_hash) {
        return Err(ApiError::bad_request("A key hash is SHA-256 in base64url"));
    }
    if !state.db.receiver_set_key(&auth.identity_pubkey, &receiver, &id, &body.key_hash).await? {
        return Err(ApiError::not_found("No such webhook"));
    }
    Ok(Json(json!({ "ok": true })))
}

/// SHA-256 in base64url.
fn is_hash(text: &str) -> bool {
    b64url_decode(text).is_ok_and(|bytes| bytes.len() == 32)
}

pub fn random_id() -> String {
    let mut bytes = [0u8; 16];
    rand::RngCore::fill_bytes(&mut rand::thread_rng(), &mut bytes);
    b64url_encode(&bytes)
}

/// The #84 delivery contract's signature: HMAC-SHA256 with the secret over the compact JSON of
/// `[version, subscription_id, generation, delivery_id, occurred_at, event_type, payload]`, in
/// unpadded base64url.
pub fn sign(secret: &str, fields: &Value) -> String {
    let mut mac = Hmac::<Sha256>::new_from_slice(secret.as_bytes()).expect("HMAC takes any key");
    mac.update(&serde_json::to_vec(fields).expect("serializable"));
    b64url_encode(&mac.finalize().into_bytes())
}

/// The envelope a Runner's inbox verifies, for one event about a subscription's subject.
pub fn envelope(sub: &ReceiverSub, delivery_id: &str, event_type: &str, payload: &Value) -> Value {
    let payload = payload.to_string();
    let occurred_at = now();
    let delivery_id: String = delivery_id.chars().take(256).collect();
    let fields = json!([1, sub.subscription_id, sub.generation, delivery_id, occurred_at, event_type, payload]);
    json!({
        "version": 1,
        "subscription_id": sub.subscription_id,
        "generation": sub.generation,
        "delivery_id": delivery_id,
        "occurred_at": occurred_at,
        "event_type": event_type,
        "payload": payload,
        "signature": sign(&sub.secret, &fields),
    })
}

/// Seals an event to the subscription's Runner and stores it as an `event` blob for that
/// machine alone. A subscription whose machine is gone (unpaired) goes with it. False when it
/// was not delivered.
pub async fn deliver(state: &AppState, sub: &ReceiverSub, delivery_id: &str, payload: &Value) -> ApiResult<bool> {
    let machines = state.db.machines_for(&sub.identity_pubkey).await?;
    let Some(machine) = machines.into_iter().find(|machine| machine.machine_pubkey == sub.machine_pubkey) else {
        state.db.receiver_unsubscribe(&sub.identity_pubkey, &sub.receiver, &sub.id).await?;
        return Ok(false);
    };
    let envelope = envelope(sub, delivery_id, &sub.receiver, payload);
    let box_key: [u8; 32] = b64url_decode(&machine.box_pubkey)?.try_into().map_err(|_| ApiError::internal("A machine's box key is 32 bytes"))?;
    let sealed = crypto_box::PublicKey::from_bytes(box_key)
        .seal(&mut rand::rngs::OsRng, &serde_json::to_vec(&envelope).expect("serializable"))
        .map_err(|_| ApiError::internal("Sealing failed"))?;
    let blob = db::NewBlob {
        identity_pubkey: sub.identity_pubkey.clone(),
        id: uuid::Uuid::new_v4().to_string(),
        kind: "event".into(),
        recipient_machine_pubkey: Some(sub.machine_pubkey.clone()),
        slot: None,
        group: None,
        payload: db::Payload::Inline(sealed),
    };
    let inserted = state.db.insert_blob(blob, state.quota_bytes).await?;
    if !inserted.existing {
        state.db.publish(db::Event::Blobs { identity: sub.identity_pubkey.clone(), recipient: Some(sub.machine_pubkey.clone()) }).await;
    }
    crate::metrics::METRICS.receiver_events.add(1);
    Ok(true)
}

/// Keeps the receivers' settings in one place for `AppState`.
pub fn shared(receivers: Receivers) -> Arc<Receivers> {
    Arc::new(receivers)
}

#[cfg(test)]
mod tests;
