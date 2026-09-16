use axum::extract::FromRequestParts;
use axum::http::request::Parts;
use base64::Engine;
use ed25519_dalek::{Signature, Verifier, VerifyingKey};
use hmac::{Hmac, Mac};
use serde::Deserialize;
use sha2::Sha256;

use crate::routes::ApiError;
use crate::AppState;

pub const TOKEN_TTL: i64 = 60 * 60;
pub const SIGNED_REQUEST_SKEW: i64 = 5 * 60;

pub fn b64url_decode(text: &str) -> Result<Vec<u8>, ApiError> {
    base64::engine::general_purpose::URL_SAFE_NO_PAD
        .decode(text.trim_end_matches('='))
        .map_err(|_| ApiError::bad_request("Invalid base64url"))
}

pub fn b64url_encode(bytes: &[u8]) -> String {
    base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(bytes)
}

pub fn verifying_key(pubkey: &str) -> Result<VerifyingKey, ApiError> {
    let bytes = b64url_decode(pubkey)?;
    let array: [u8; 32] = bytes.try_into().map_err(|_| ApiError::bad_request("Public key must be 32 bytes"))?;
    VerifyingKey::from_bytes(&array).map_err(|_| ApiError::bad_request("Invalid Ed25519 public key"))
}

pub fn verify_signature(pubkey: &str, message: &[u8], signature: &str) -> Result<(), ApiError> {
    let key = verifying_key(pubkey)?;
    let bytes = b64url_decode(signature)?;
    let signature = Signature::from_slice(&bytes).map_err(|_| ApiError::bad_request("Invalid signature"))?;
    key.verify(message, &signature).map_err(|_| ApiError::unauthorized("Signature does not verify"))
}

/// `{ payload: base64url(json), signature: base64url(ed25519(payload)) }`. The payload carries
/// `identity_pubkey` and `ts`; the identity key must have produced the signature.
#[derive(Debug, Deserialize)]
pub struct SignedRequest {
    pub payload: String,
    pub signature: String,
}

impl SignedRequest {
    pub fn verify<T: serde::de::DeserializeOwned>(&self) -> Result<(T, String), ApiError> {
        let bytes = b64url_decode(&self.payload)?;
        let value: serde_json::Value =
            serde_json::from_slice(&bytes).map_err(|_| ApiError::bad_request("Payload is not JSON"))?;
        let identity_pubkey = value["identity_pubkey"]
            .as_str()
            .ok_or_else(|| ApiError::bad_request("Payload has no identity_pubkey"))?
            .to_string();
        let ts = value["ts"].as_i64().ok_or_else(|| ApiError::bad_request("Payload has no ts"))?;
        if (crate::db::now() - ts).abs() > SIGNED_REQUEST_SKEW {
            return Err(ApiError::unauthorized("Signed request is too old"));
        }
        verify_signature(&identity_pubkey, &bytes, &self.signature)?;
        let typed: T = serde_json::from_slice(&bytes).map_err(|e| ApiError::bad_request(&format!("Payload: {e}")))?;
        Ok((typed, identity_pubkey))
    }
}

type HmacSha256 = Hmac<Sha256>;

pub fn issue_token(secret: &[u8; 32], identity_pubkey: &str, machine_pubkey: &str) -> (String, i64) {
    let expires_at = crate::db::now() + TOKEN_TTL;
    let body = format!("{identity_pubkey}|{machine_pubkey}|{expires_at}");
    let mut mac = HmacSha256::new_from_slice(secret).expect("hmac key");
    mac.update(body.as_bytes());
    let tag = mac.finalize().into_bytes();
    (format!("{}.{}", b64url_encode(body.as_bytes()), b64url_encode(&tag)), expires_at)
}

pub fn parse_token(secret: &[u8; 32], token: &str) -> Result<Auth, ApiError> {
    let (body, tag) = token.split_once('.').ok_or_else(|| ApiError::unauthorized("Malformed token"))?;
    let body_bytes = b64url_decode(body).map_err(|_| ApiError::unauthorized("Malformed token"))?;
    let tag_bytes = b64url_decode(tag).map_err(|_| ApiError::unauthorized("Malformed token"))?;
    let mut mac = HmacSha256::new_from_slice(secret).expect("hmac key");
    mac.update(&body_bytes);
    mac.verify_slice(&tag_bytes).map_err(|_| ApiError::unauthorized("Token does not verify"))?;

    let body = String::from_utf8(body_bytes).map_err(|_| ApiError::unauthorized("Malformed token"))?;
    let mut parts = body.split('|');
    let identity_pubkey = parts.next().unwrap_or("").to_string();
    let machine_pubkey = parts.next().unwrap_or("").to_string();
    let expires_at: i64 = parts.next().and_then(|s| s.parse().ok()).unwrap_or(0);
    if expires_at < crate::db::now() {
        return Err(ApiError::unauthorized("Token expired"));
    }
    Ok(Auth { identity_pubkey, machine_pubkey })
}

/// The authenticated machine. Extracting it also bumps the machine's `last_seen`, at most
/// once every `db::TOUCH_INTERVAL` seconds per machine.
#[derive(Debug, Clone)]
pub struct Auth {
    pub identity_pubkey: String,
    pub machine_pubkey: String,
}

impl FromRequestParts<AppState> for Auth {
    type Rejection = ApiError;

    async fn from_request_parts(parts: &mut Parts, state: &AppState) -> Result<Self, Self::Rejection> {
        let header = parts
            .headers
            .get(axum::http::header::AUTHORIZATION)
            .and_then(|v| v.to_str().ok())
            .ok_or_else(|| ApiError::unauthorized("Missing bearer token"))?;
        let token = header.strip_prefix("Bearer ").ok_or_else(|| ApiError::unauthorized("Missing bearer token"))?;
        let auth = parse_token(&state.secret, token)?;
        if let Err(retry_after) = state.identity_limiter.check(&auth.identity_pubkey) {
            return Err(ApiError::too_many(retry_after));
        }
        if state.presence.due(&auth.machine_pubkey) {
            let machine_pubkey = auth.machine_pubkey.clone();
            state.db.write(move |db| Ok(crate::db::touch_machine(db, &machine_pubkey)?)).await?;
        }
        Ok(auth)
    }
}
