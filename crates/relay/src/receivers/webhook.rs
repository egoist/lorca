//! A routine's own webhook: a URL on the relay's public address (`https://relay.lorca.app/webhooks/<id>`)
//! and a key the Runner made, of which the relay keeps only the SHA-256. A request that carries
//! the key as `Authorization: Bearer <key>` runs the routine once, with its body as data from
//! outside: JSON as it came, any other text as a string, up to 64 KiB. An `Idempotency-Key`
//! makes a repeat one delivery. Each webhook takes a request a second, in bursts of up to 30.

use std::collections::HashMap;
use std::sync::Mutex;

use axum::body::Bytes;
use axum::extract::{Path, State};
use axum::http::{HeaderMap, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::Json;
use serde_json::{json, Value};
use sha2::{Digest, Sha256};

use super::Subscribe;
use crate::auth::{b64url_encode, Auth};
use crate::routes::{ApiError, ApiResult};
use crate::AppState;

/// The largest body a webhook takes.
pub const MAX_BODY_BYTES: usize = 64 * 1024;
/// How long this process remembers an `Idempotency-Key`. The Runner's inbox remembers a
/// delivery for thirty days, so a repeat after this still runs nothing.
const IDEMPOTENCY_SECS: i64 = 24 * 3600;

pub struct Hooks {
    /// The relay's public address, where the webhooks' URLs start: `https://relay.lorca.app`.
    pub base: String,
    limiter: crate::limit::RateLimiter,
    /// Idempotency keys seen lately, by webhook and key, with when.
    seen: Mutex<HashMap<String, i64>>,
}

impl Hooks {
    pub fn new(base: &str) -> Hooks {
        Hooks { base: base.trim_end_matches('/').to_string(), limiter: crate::limit::RateLimiter::new(1.0, 30), seen: Mutex::default() }
    }

    /// A URL for the calling machine's routine, checked against the key hash the Runner sent.
    pub async fn subscribe(&self, state: &AppState, auth: &Auth, body: &Subscribe) -> ApiResult<Value> {
        if body.key_hash.is_none() {
            return Err(ApiError::bad_request("A webhook needs its key's hash"));
        }
        let sub = body.row("webhook", auth, body.subject.clone(), None, None);
        super::save(state, &sub).await?;
        Ok(json!({ "status": "subscribed", "id": sub.id, "endpoint": format!("{}/webhooks/{}", self.base, sub.id), "name": "Webhook" }))
    }

    /// True the first time a webhook sees an idempotency key within a day.
    fn first(&self, hook: &str, key: &str) -> bool {
        let now = crate::db::now();
        let mut seen = self.seen.lock().unwrap();
        if seen.len() > 100_000 {
            seen.retain(|_, at| now - *at < IDEMPOTENCY_SECS);
        }
        let mark = format!("{hook}\u{0}{key}");
        match seen.get(&mark) {
            Some(at) if now - at < IDEMPOTENCY_SECS => false,
            _ => {
                seen.insert(mark, now);
                true
            }
        }
    }
}

/// SHA-256 in base64url, as the Runner hashes a key.
pub fn key_hash(key: &str) -> String {
    b64url_encode(&Sha256::digest(key.as_bytes()))
}

/// Compares two hashes in constant time.
fn same(a: &str, b: &str) -> bool {
    a.len() == b.len() && a.bytes().zip(b.bytes()).fold(0u8, |acc, (x, y)| acc | (x ^ y)) == 0
}

/// A request to a routine's webhook: `401` without its key, `413` over 64 KiB, `429` past its
/// rate, `202` once it is sealed to the routine's Runner (or seen already, with its
/// idempotency key).
pub async fn receive(State(state): State<AppState>, Path(id): Path<String>, headers: HeaderMap, body: Bytes) -> Response {
    match accept(&state, &id, &headers, &body).await {
        Ok(response) => response,
        Err(error) => error.into_response(),
    }
}

async fn accept(state: &AppState, id: &str, headers: &HeaderMap, body: &Bytes) -> ApiResult<Response> {
    let Some(hooks) = &state.receivers.webhook else { return Err(ApiError::not_found("No such webhook")) };
    let Some(sub) = state.db.receiver_sub("webhook", id).await? else { return Err(ApiError::not_found("No such webhook")) };
    let given = headers.get(axum::http::header::AUTHORIZATION).and_then(|value| value.to_str().ok()).and_then(|value| value.strip_prefix("Bearer ")).map(str::trim).unwrap_or_default();
    let expected = sub.key_hash.as_deref().unwrap_or_default();
    if given.is_empty() || expected.is_empty() || !same(&key_hash(given), expected) {
        return Err(ApiError::unauthorized("Include this routine's key as Authorization: Bearer <key>"));
    }
    if body.len() > MAX_BODY_BYTES {
        return Err(ApiError::too_large("A webhook's body is at most 64 KiB"));
    }
    if let Err(retry_after) = hooks.limiter.check(id) {
        return Err(ApiError::too_many(retry_after));
    }
    let text = std::str::from_utf8(body).map_err(|_| ApiError::bad_request("A webhook's body is JSON or UTF-8 text"))?;
    let data: Value = if text.trim().is_empty() { Value::Null } else { serde_json::from_str(text).unwrap_or_else(|_| Value::String(text.to_string())) };
    let idempotency = headers.get("idempotency-key").and_then(|value| value.to_str().ok()).map(str::trim).filter(|key| !key.is_empty()).map(|key| key.chars().take(200).collect::<String>());
    let delivery_id = match &idempotency {
        Some(key) => {
            if !hooks.first(id, key) {
                return Ok((StatusCode::OK, Json(json!({ "accepted": true, "duplicate": true }))).into_response());
            }
            format!("key-{}", b64url_encode(&Sha256::digest(format!("{id}\u{0}{key}").as_bytes())))
        }
        None => uuid::Uuid::new_v4().to_string(),
    };
    let mut payload = json!({ "kind": "request", "summary": "Webhook request", "data": data });
    if !sub.subject.is_empty() {
        payload["subject"] = json!(sub.subject);
    }
    if let Some(kind) = headers.get(axum::http::header::CONTENT_TYPE).and_then(|value| value.to_str().ok()) {
        payload["content_type"] = json!(kind.chars().take(100).collect::<String>());
    }
    if !super::deliver(state, &sub, &delivery_id, &payload).await? {
        return Err(ApiError::not_found("No such webhook"));
    }
    Ok((StatusCode::ACCEPTED, Json(json!({ "accepted": true }))).into_response())
}
