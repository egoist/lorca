//! Pushes to phones. A Runner hands the relay ciphertext only the identity's Devices can
//! open; the relay wraps it in an alert with fixed words and gives it to APNs or FCM. The
//! phone decrypts it before the alert shows, so the words Apple, Google, and the relay see
//! are always "New reply".
//!
//! APNs takes a provider token (an ES256 JWT from the team's `.p8` key) over HTTP/2. FCM
//! takes an OAuth access token minted from a service account (an RS256 JWT).

use std::sync::Mutex;

use base64::Engine;
use ring::rand::SystemRandom;
use ring::signature::{EcdsaKeyPair, RsaKeyPair, ECDSA_P256_SHA256_FIXED_SIGNING, RSA_PKCS1_SHA256};
use serde::Deserialize;
use serde_json::json;

use crate::auth::b64url_encode;
use crate::db::{now, PushToken};

const APNS_PRODUCTION: &str = "https://api.push.apple.com";
const APNS_SANDBOX: &str = "https://api.sandbox.push.apple.com";
const FCM_SCOPE: &str = "https://www.googleapis.com/auth/firebase.messaging";
/// APNs refuses a provider token older than an hour and one refreshed more than once in
/// twenty minutes.
const APNS_JWT_TTL: i64 = 40 * 60;
/// How long APNs and FCM keep a push for a phone that is off.
const EXPIRES_AFTER: i64 = 24 * 60 * 60;

/// What the phone shows when it cannot decrypt (no key yet, or a push from before it paired).
const FALLBACK_TITLE: &str = "Tinybot";
const FALLBACK_BODY: &str = "New reply";

pub enum Delivery {
    Sent,
    /// The token is dead (uninstalled, or for the other APNs environment): forget it.
    Gone,
    Failed(String),
}

pub struct Pusher {
    http: reqwest::Client,
    apns: Option<Apns>,
    fcm: Option<Fcm>,
}

impl Pusher {
    pub fn new(apns: Option<Apns>, fcm: Option<Fcm>) -> Self {
        Pusher { http: reqwest::Client::new(), apns, fcm }
    }

    pub fn describe(&self) -> String {
        match (&self.apns, &self.fcm) {
            (Some(_), Some(_)) => "apns+fcm".into(),
            (Some(_), None) => "apns".into(),
            (None, Some(_)) => "fcm".into(),
            (None, None) => "off".into(),
        }
    }

    pub fn takes(&self, platform: &str) -> bool {
        match platform {
            "apns" => self.apns.is_some(),
            "fcm" => self.fcm.is_some(),
            _ => false,
        }
    }

    pub async fn send(&self, token: &PushToken, ciphertext: &[u8]) -> Delivery {
        let sealed = b64url_encode(ciphertext);
        let result = match (token.platform.as_str(), &self.apns, &self.fcm) {
            ("apns", Some(apns), _) => apns.send(&self.http, token, &sealed).await,
            ("fcm", _, Some(fcm)) => fcm.send(&self.http, token, &sealed).await,
            _ => return Delivery::Failed(format!("{} is not configured", token.platform)),
        };
        result.unwrap_or_else(|error| Delivery::Failed(error.to_string()))
    }
}

fn jwt(header: serde_json::Value, claims: serde_json::Value, sign: impl FnOnce(&[u8]) -> anyhow::Result<Vec<u8>>) -> anyhow::Result<String> {
    let signing_input = format!("{}.{}", b64url_encode(header.to_string().as_bytes()), b64url_encode(claims.to_string().as_bytes()));
    let signature = sign(signing_input.as_bytes())?;
    Ok(format!("{signing_input}.{}", b64url_encode(&signature)))
}

/// The DER inside a PEM block, whatever its label.
fn pem_to_der(pem: &str) -> anyhow::Result<Vec<u8>> {
    let body: String = pem.lines().map(str::trim).filter(|line| !line.is_empty() && !line.starts_with("-----")).collect();
    Ok(base64::engine::general_purpose::STANDARD.decode(body)?)
}

// MARK: - APNs

pub struct Apns {
    key: EcdsaKeyPair,
    key_id: String,
    team_id: String,
    /// The app's bundle id.
    topic: String,
    /// Overrides both Apple hosts, for a test server.
    base_url: Option<String>,
    jwt: Mutex<Option<(String, i64)>>,
}

impl Apns {
    pub fn new(p8: &str, key_id: String, team_id: String, topic: String, base_url: Option<String>) -> anyhow::Result<Self> {
        let der = pem_to_der(p8)?;
        let key = EcdsaKeyPair::from_pkcs8(&ECDSA_P256_SHA256_FIXED_SIGNING, &der, &SystemRandom::new())
            .map_err(|e| anyhow::anyhow!("the APNs key is not a P-256 PKCS#8 key: {e}"))?;
        Ok(Apns { key, key_id, team_id, topic, base_url, jwt: Mutex::new(None) })
    }

    fn provider_token(&self) -> anyhow::Result<String> {
        let mut cached = self.jwt.lock().unwrap_or_else(|e| e.into_inner());
        if let Some((token, issued_at)) = cached.as_ref() {
            if now() - issued_at < APNS_JWT_TTL {
                return Ok(token.clone());
            }
        }
        let issued_at = now();
        let token = jwt(json!({ "alg": "ES256", "kid": self.key_id }), json!({ "iss": self.team_id, "iat": issued_at }), |input| {
            let signature = self.key.sign(&SystemRandom::new(), input).map_err(|_| anyhow::anyhow!("signing the APNs token failed"))?;
            Ok(signature.as_ref().to_vec())
        })?;
        *cached = Some((token.clone(), issued_at));
        Ok(token)
    }

    async fn send(&self, http: &reqwest::Client, token: &PushToken, sealed: &str) -> anyhow::Result<Delivery> {
        let host = match (&self.base_url, token.environment.as_str()) {
            (Some(url), _) => url.as_str(),
            (None, "sandbox") => APNS_SANDBOX,
            (None, _) => APNS_PRODUCTION,
        };
        // `mutable-content` hands the alert to the app's notification service extension,
        // which swaps the fixed words for the decrypted ones.
        let body = json!({
            "aps": {
                "alert": { "title": FALLBACK_TITLE, "body": FALLBACK_BODY },
                "sound": "default",
                "mutable-content": 1,
            },
            "c": sealed,
        });
        let response = http
            .post(format!("{host}/3/device/{}", token.token))
            .bearer_auth(self.provider_token()?)
            .header("apns-topic", &self.topic)
            .header("apns-push-type", "alert")
            .header("apns-priority", "10")
            .header("apns-expiration", (now() + EXPIRES_AFTER).to_string())
            .json(&body)
            .send()
            .await?;
        let status = response.status();
        if status.is_success() {
            return Ok(Delivery::Sent);
        }
        let reason = response.json::<serde_json::Value>().await.ok().and_then(|v| v["reason"].as_str().map(str::to_string)).unwrap_or_default();
        if status == reqwest::StatusCode::GONE || matches!(reason.as_str(), "BadDeviceToken" | "Unregistered" | "DeviceTokenNotForTopic") {
            return Ok(Delivery::Gone);
        }
        // A provider token Apple no longer takes is minted again on the next push.
        if reason == "ExpiredProviderToken" || reason == "InvalidProviderToken" {
            *self.jwt.lock().unwrap_or_else(|e| e.into_inner()) = None;
        }
        Ok(Delivery::Failed(format!("APNs {status}: {reason}")))
    }
}

// MARK: - FCM

#[derive(Deserialize)]
struct ServiceAccount {
    client_email: String,
    private_key: String,
    project_id: String,
    #[serde(default = "default_token_uri")]
    token_uri: String,
}

fn default_token_uri() -> String {
    "https://oauth2.googleapis.com/token".into()
}

pub struct Fcm {
    key: RsaKeyPair,
    account: ServiceAccount,
    /// Overrides the FCM host, for a test server.
    base_url: Option<String>,
    access: tokio::sync::Mutex<Option<(String, i64)>>,
}

impl Fcm {
    pub fn new(service_account_json: &str, base_url: Option<String>) -> anyhow::Result<Self> {
        let account: ServiceAccount = serde_json::from_str(service_account_json)?;
        let key = RsaKeyPair::from_pkcs8(&pem_to_der(&account.private_key)?).map_err(|e| anyhow::anyhow!("the service account key is not an RSA PKCS#8 key: {e}"))?;
        Ok(Fcm { key, account, base_url, access: tokio::sync::Mutex::new(None) })
    }

    async fn access_token(&self, http: &reqwest::Client) -> anyhow::Result<String> {
        let mut cached = self.access.lock().await;
        if let Some((token, expires_at)) = cached.as_ref() {
            if expires_at - 60 > now() {
                return Ok(token.clone());
            }
        }
        let issued_at = now();
        let claims = json!({ "iss": self.account.client_email, "scope": FCM_SCOPE, "aud": self.account.token_uri, "iat": issued_at, "exp": issued_at + 3600 });
        let assertion = jwt(json!({ "alg": "RS256", "typ": "JWT" }), claims, |input| {
            let mut signature = vec![0u8; self.key.public().modulus_len()];
            self.key.sign(&RSA_PKCS1_SHA256, &SystemRandom::new(), input, &mut signature).map_err(|_| anyhow::anyhow!("signing the FCM assertion failed"))?;
            Ok(signature)
        })?;
        let response = http
            .post(&self.account.token_uri)
            .form(&[("grant_type", "urn:ietf:params:oauth:grant-type:jwt-bearer"), ("assertion", assertion.as_str())])
            .send()
            .await?;
        let status = response.status();
        let value: serde_json::Value = response.json().await?;
        let Some(token) = value["access_token"].as_str() else { anyhow::bail!("FCM token exchange {status}: {value}") };
        *cached = Some((token.to_string(), issued_at + value["expires_in"].as_i64().unwrap_or(3600)));
        Ok(token.to_string())
    }

    async fn send(&self, http: &reqwest::Client, token: &PushToken, sealed: &str) -> anyhow::Result<Delivery> {
        let host = self.base_url.as_deref().unwrap_or("https://fcm.googleapis.com");
        // Data only: the app's messaging service decrypts it and posts the notification.
        let body = json!({
            "message": {
                "token": token.token,
                "data": { "c": sealed },
                "android": { "priority": "HIGH", "ttl": format!("{EXPIRES_AFTER}s") },
            }
        });
        let response = http
            .post(format!("{host}/v1/projects/{}/messages:send", self.account.project_id))
            .bearer_auth(self.access_token(http).await?)
            .json(&body)
            .send()
            .await?;
        let status = response.status();
        if status.is_success() {
            return Ok(Delivery::Sent);
        }
        let text = response.text().await.unwrap_or_default();
        if status == reqwest::StatusCode::NOT_FOUND || text.contains("UNREGISTERED") {
            return Ok(Delivery::Gone);
        }
        if status == reqwest::StatusCode::UNAUTHORIZED {
            *self.access.lock().await = None;
        }
        Ok(Delivery::Failed(format!("FCM {status}: {text}")))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // A throwaway P-256 key in PKCS#8, as Apple's `.p8` files are.
    const P8: &str = "-----BEGIN PRIVATE KEY-----
MIGHAgEAMBMGByqGSM49AgEGCCqGSM49AwEHBG0wawIBAQQggUoLEBNgg5MHzaFi
Z6lqVJhY2NcZwEuF4trWnvnw1AehRANCAATwsWtjWEGippjwrz56xcdYTXE0D0Ud
7cCVDDMUDHTutnz9PyZ5SsgkTGYeE+Kykce3DhJptdeq8K7snCdp8UuW
-----END PRIVATE KEY-----";

    #[test]
    fn the_apns_provider_token_is_a_signed_es256_jwt() {
        let apns = Apns::new(P8, "KEYID12345".into(), "TEAMID1234".into(), "dev.tinybot.app".into(), None).unwrap();
        let token = apns.provider_token().unwrap();
        let parts: Vec<&str> = token.split('.').collect();
        assert_eq!(parts.len(), 3);
        let header: serde_json::Value = serde_json::from_slice(&crate::auth::b64url_decode(parts[0]).unwrap()).unwrap();
        assert_eq!(header, json!({ "alg": "ES256", "kid": "KEYID12345" }));
        // r || s, the JOSE form, not DER.
        assert_eq!(crate::auth::b64url_decode(parts[2]).unwrap().len(), 64);
        // Minted once, reused until it ages out.
        assert_eq!(apns.provider_token().unwrap(), token);
    }
}
