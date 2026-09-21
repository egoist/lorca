//! HTTP client for the relay. Signs identity-level requests, authenticates machines with the
//! challenge, and moves ciphertext.

use std::sync::{Arc, Mutex};

use serde::Deserialize;
use serde_json::{json, Value};

use crate::config::now_unix;
use crate::keys::{b64, Identity, Machine};

#[derive(Debug, thiserror::Error)]
#[error("{message}")]
pub struct RelayError {
    pub status: Option<u16>,
    pub message: String,
}

impl RelayError {
    pub fn is_unauthorized(&self) -> bool {
        self.status == Some(401)
    }
    /// The relay does not know this machine: a different relay (or a reset one) than the one
    /// that attested it.
    pub fn is_unknown_machine(&self) -> bool {
        self.status == Some(404)
    }
    /// This machine was unpaired from another Device. Its key is dead for good.
    pub fn is_unpaired(&self) -> bool {
        self.status == Some(410)
    }
    pub fn is_client_error(&self) -> bool {
        matches!(self.status, Some(400..=499))
    }
}

impl From<reqwest::Error> for RelayError {
    fn from(error: reqwest::Error) -> Self {
        RelayError { status: error.status().map(|s| s.as_u16()), message: format!("relay unreachable: {error}") }
    }
}

pub type RelayResult<T> = Result<T, RelayError>;

#[derive(Debug, Clone, Deserialize)]
#[allow(dead_code)]
pub struct BlobIn {
    pub id: String,
    pub kind: String,
    pub recipient_machine_pubkey: Option<String>,
    pub seq: i64,
    pub ciphertext: String,
    pub created_at: i64,
}

#[derive(Debug, Clone, Deserialize)]
#[allow(dead_code)]
pub struct MachineIn {
    pub machine_pubkey: String,
    pub box_pubkey: String,
    pub last_seen: i64,
    pub created_at: i64,
    /// The machine has a sync socket open on the relay.
    #[serde(default)]
    pub online: bool,
}

/// What the relay says over the sync socket. It carries no data: a Device pulls after it.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Signal {
    /// The identity has a blob this machine may read.
    Blobs,
    /// The machine list or a machine's presence changed.
    Machines,
}

/// This machine's sync socket. It is online on the relay while this is open.
pub struct SyncSocket {
    stream: tokio_tungstenite::WebSocketStream<tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>>,
}

/// The relay pings every 25 s. A socket silent for this long is dead: the computer slept, the
/// phone changed networks.
const SOCKET_SILENCE: std::time::Duration = std::time::Duration::from_secs(70);

impl SyncSocket {
    /// The next signal. An error means the socket is gone and the caller connects again.
    pub async fn next(&mut self) -> RelayResult<Signal> {
        use futures::{SinkExt, StreamExt};
        use tokio_tungstenite::tungstenite::Message;
        loop {
            let message = tokio::time::timeout(SOCKET_SILENCE, self.stream.next())
                .await
                .map_err(|_| RelayError { status: None, message: "the sync socket went silent".into() })?
                .ok_or_else(|| RelayError { status: None, message: "the relay closed the sync socket".into() })?
                .map_err(socket_error)?;
            match message {
                Message::Text(text) => match serde_json::from_str::<Value>(&text).ok().as_ref().and_then(|v| v["type"].as_str()) {
                    Some("blobs") => return Ok(Signal::Blobs),
                    Some("machines") => return Ok(Signal::Machines),
                    _ => {}
                },
                // The pong has to be flushed by hand when nothing else is written.
                Message::Ping(_) => self.stream.flush().await.map_err(socket_error)?,
                Message::Close(_) => return Err(RelayError { status: None, message: "the relay closed the sync socket".into() }),
                _ => {}
            }
        }
    }
}

fn socket_error(error: tokio_tungstenite::tungstenite::Error) -> RelayError {
    use tokio_tungstenite::tungstenite::Error;
    match error {
        // The upgrade was refused: 401 for a stale token, 410 for an unpaired machine.
        Error::Http(response) => RelayError { status: Some(response.status().as_u16()), message: format!("sync socket refused ({})", response.status()) },
        other => RelayError { status: None, message: format!("sync socket: {other}") },
    }
}

/// TLS for `wss://`, with the provider named: the build links both ring and aws-lc-rs, and
/// rustls picks neither by itself.
fn tls() -> Arc<rustls::ClientConfig> {
    static CONFIG: std::sync::OnceLock<Arc<rustls::ClientConfig>> = std::sync::OnceLock::new();
    CONFIG
        .get_or_init(|| {
            let roots = rustls::RootCertStore { roots: webpki_roots::TLS_SERVER_ROOTS.to_vec() };
            let config = rustls::ClientConfig::builder_with_provider(Arc::new(rustls::crypto::ring::default_provider()))
                .with_safe_default_protocol_versions()
                .expect("ring supports the default TLS versions")
                .with_root_certificates(roots)
                .with_no_client_auth();
            Arc::new(config)
        })
        .clone()
}

pub struct RelayClient {
    http: reqwest::Client,
    token: Mutex<Option<(String, i64)>>,
}

impl RelayClient {
    pub fn new(http: reqwest::Client) -> Self {
        RelayClient { http, token: Mutex::new(None) }
    }

    pub fn forget_token(&self) {
        *self.token.lock().unwrap() = None;
    }

    async fn check(response: reqwest::Response) -> RelayResult<Value> {
        let status = response.status();
        if status == reqwest::StatusCode::NO_CONTENT {
            return Ok(Value::Null);
        }
        let text = response.text().await.unwrap_or_default();
        let value: Value = serde_json::from_str(&text).unwrap_or(Value::Null);
        if !status.is_success() {
            let message = value["error"].as_str().map(str::to_string).unwrap_or_else(|| format!("{status}: {text}"));
            return Err(RelayError { status: Some(status.as_u16()), message });
        }
        Ok(value)
    }

    pub async fn health(&self, url: &str) -> RelayResult<()> {
        Self::check(self.http.get(format!("{url}/v1/health")).send().await?).await?;
        Ok(())
    }

    /// Signed by the identity: registers the identity (idempotent) and attests one machine.
    pub async fn register(&self, url: &str, identity: &Identity, machine_pubkey: &str, box_pubkey: &str) -> RelayResult<()> {
        let payload = json!({
            "identity_pubkey": identity.pubkey(),
            "content_pubkey": identity.content_pubkey(),
            "machine": { "machine_pubkey": machine_pubkey, "box_pubkey": box_pubkey },
            "ts": now_unix(),
        });
        let bytes = serde_json::to_vec(&payload).unwrap();
        let body = json!({ "payload": b64(&bytes), "signature": identity.sign(&bytes) });
        Self::check(self.http.post(format!("{url}/v1/identities")).json(&body).send().await?).await?;
        Ok(())
    }

    pub async fn authenticate(&self, url: &str, machine: &Machine) -> RelayResult<String> {
        let challenge = Self::check(
            self.http
                .post(format!("{url}/v1/auth/challenge"))
                .json(&json!({ "machine_pubkey": machine.pubkey() }))
                .send()
                .await?,
        )
        .await?;
        let nonce = challenge["nonce"].as_str().ok_or_else(|| RelayError { status: None, message: "no nonce".into() })?;
        let verified = Self::check(
            self.http
                .post(format!("{url}/v1/auth/verify"))
                .json(&json!({
                    "machine_pubkey": machine.pubkey(),
                    "nonce": nonce,
                    "signature": machine.sign(nonce.as_bytes()),
                }))
                .send()
                .await?,
        )
        .await?;
        let token = verified["token"].as_str().ok_or_else(|| RelayError { status: None, message: "no token".into() })?.to_string();
        let expires_at = verified["expires_at"].as_i64().unwrap_or(now_unix() + 600);
        *self.token.lock().unwrap() = Some((token.clone(), expires_at));
        Ok(token)
    }

    pub async fn token(&self, url: &str, machine: &Machine) -> RelayResult<String> {
        if let Some((token, expires_at)) = self.token.lock().unwrap().clone() {
            if expires_at - 60 > now_unix() {
                return Ok(token);
            }
        }
        self.authenticate(url, machine).await
    }

    pub async fn put_blob(&self, url: &str, token: &str, item: &crate::app::OutboxItem) -> RelayResult<i64> {
        let mut body = json!({ "id": item.id, "kind": item.kind, "recipient_machine_pubkey": item.recipient, "ciphertext": item.ciphertext });
        if let Some(slot) = &item.slot {
            body["slot"] = json!(slot.name);
            body["keep_first"] = json!(slot.keep_first);
        }
        if let Some(group) = &item.group {
            body["group"] = json!(group);
        }
        let value = Self::check(
            self.http
                .put(format!("{url}/v1/blobs"))
                .bearer_auth(token)
                .json(&body)
                .send()
                .await?,
        )
        .await?;
        Ok(value["seq"].as_i64().unwrap_or(0))
    }

    /// Opens this machine's sync socket: `ws(s)://<relay>/v1/sync` with the bearer token.
    pub async fn sync_socket(&self, url: &str, token: &str) -> RelayResult<SyncSocket> {
        use tokio_tungstenite::tungstenite::client::IntoClientRequest;
        let address = match url.split_once("://") {
            Some(("https", rest)) => format!("wss://{rest}/v1/sync"),
            Some((_, rest)) => format!("ws://{rest}/v1/sync"),
            None => format!("ws://{url}/v1/sync"),
        };
        let mut request = address.into_client_request().map_err(socket_error)?;
        let bearer = format!("Bearer {token}").parse().map_err(|_| RelayError { status: None, message: "token is not a header value".into() })?;
        request.headers_mut().insert("authorization", bearer);
        let connector = tokio_tungstenite::Connector::Rustls(tls());
        let connect = tokio_tungstenite::connect_async_tls_with_config(request, None, false, Some(connector));
        let (stream, _) = tokio::time::timeout(std::time::Duration::from_secs(20), connect)
            .await
            .map_err(|_| RelayError { status: None, message: "the sync socket timed out connecting".into() })?
            .map_err(socket_error)?;
        Ok(SyncSocket { stream })
    }

    pub async fn list_blobs(&self, url: &str, token: &str, since: i64, kinds: &str) -> RelayResult<(Vec<BlobIn>, i64)> {
        let value = Self::check(
            self.http
                .get(format!("{url}/v1/blobs"))
                .bearer_auth(token)
                .query(&[("since", since.to_string()), ("kinds", kinds.to_string())])
                .timeout(std::time::Duration::from_secs(60))
                .send()
                .await?,
        )
        .await?;
        let blobs: Vec<BlobIn> = serde_json::from_value(value["blobs"].clone()).unwrap_or_default();
        Ok((blobs, value["seq"].as_i64().unwrap_or(since)))
    }

    /// One blob by id; `None` when the relay has no such blob for this identity.
    pub async fn get_blob(&self, url: &str, token: &str, id: &str) -> RelayResult<Option<BlobIn>> {
        let response = self.http.get(format!("{url}/v1/blobs/{id}")).bearer_auth(token).send().await?;
        if response.status() == reqwest::StatusCode::NOT_FOUND {
            return Ok(None);
        }
        let value = Self::check(response).await?;
        Ok(serde_json::from_value(value).ok())
    }

    pub async fn delete_blob(&self, url: &str, token: &str, id: &str) -> RelayResult<()> {
        Self::check(self.http.delete(format!("{url}/v1/blobs/{id}")).bearer_auth(token).send().await?).await?;
        Ok(())
    }

    /// Deletes every blob of a group. Good to repeat.
    pub async fn delete_group(&self, url: &str, token: &str, group: &str) -> RelayResult<()> {
        Self::check(self.http.delete(format!("{url}/v1/groups/{group}")).bearer_auth(token).send().await?).await?;
        Ok(())
    }

    pub async fn machines(&self, url: &str, token: &str) -> RelayResult<(Vec<MachineIn>, i64)> {
        let value = Self::check(self.http.get(format!("{url}/v1/machines")).bearer_auth(token).send().await?).await?;
        let machines: Vec<MachineIn> = serde_json::from_value(value["machines"].clone()).unwrap_or_default();
        Ok((machines, value["now"].as_i64().unwrap_or(now_unix())))
    }

    // MARK: - Push

    /// Where this phone takes pushes: its APNs or FCM device token.
    pub async fn put_push_token(&self, url: &str, token: &str, platform: &str, device_token: &str, environment: Option<&str>) -> RelayResult<()> {
        Self::check(
            self.http
                .put(format!("{url}/v1/push/token"))
                .bearer_auth(token)
                .json(&json!({ "platform": platform, "token": device_token, "environment": environment }))
                .send()
                .await?,
        )
        .await?;
        Ok(())
    }

    pub async fn delete_push_token(&self, url: &str, token: &str) -> RelayResult<()> {
        Self::check(self.http.delete(format!("{url}/v1/push/token")).bearer_auth(token).send().await?).await?;
        Ok(())
    }

    /// Asks the relay to push this ciphertext to the identity's phones; answers how many it queued.
    pub async fn push(&self, url: &str, token: &str, ciphertext_b64: &str) -> RelayResult<u64> {
        let value = Self::check(self.http.post(format!("{url}/v1/push")).bearer_auth(token).json(&json!({ "ciphertext": ciphertext_b64 })).send().await?).await?;
        Ok(value["queued"].as_u64().unwrap_or(0))
    }

    // MARK: - Pairing mailbox

    /// Unpairs a machine of this identity, this one included.
    pub async fn revoke_machine(&self, url: &str, token: &str, machine_pubkey: &str) -> RelayResult<()> {
        Self::check(self.http.delete(format!("{url}/v1/machines/{machine_pubkey}")).bearer_auth(token).send().await?).await?;
        Ok(())
    }

    pub async fn pair_create(&self, url: &str, token: &str) -> RelayResult<String> {
        let value = Self::check(self.http.post(format!("{url}/v1/pair")).bearer_auth(token).send().await?).await?;
        Ok(value["nonce"].as_str().unwrap_or_default().to_string())
    }

    pub async fn pair_post_request(&self, url: &str, nonce: &str, ciphertext: &[u8]) -> RelayResult<()> {
        Self::check(
            self.http
                .post(format!("{url}/v1/pair/{nonce}/request"))
                .json(&json!({ "ciphertext": b64(ciphertext) }))
                .send()
                .await?,
        )
        .await?;
        Ok(())
    }

    pub async fn pair_get_request(&self, url: &str, token: &str, nonce: &str) -> RelayResult<Option<Vec<u8>>> {
        let value = Self::check(self.http.get(format!("{url}/v1/pair/{nonce}/request")).bearer_auth(token).send().await?).await?;
        decode_optional(&value)
    }

    pub async fn pair_post_reply(&self, url: &str, token: &str, nonce: &str, ciphertext: &[u8]) -> RelayResult<()> {
        Self::check(
            self.http
                .post(format!("{url}/v1/pair/{nonce}/reply"))
                .bearer_auth(token)
                .json(&json!({ "ciphertext": b64(ciphertext) }))
                .send()
                .await?,
        )
        .await?;
        Ok(())
    }

    /// The identity retires a pairing: the mailbox goes and a Device polling it gets a 404.
    pub async fn pair_delete(&self, url: &str, token: &str, nonce: &str) -> RelayResult<()> {
        Self::check(self.http.delete(format!("{url}/v1/pair/{nonce}")).bearer_auth(token).send().await?).await?;
        Ok(())
    }

    pub async fn pair_get_reply(&self, url: &str, nonce: &str) -> RelayResult<Option<Vec<u8>>> {
        let value = Self::check(self.http.get(format!("{url}/v1/pair/{nonce}/reply")).send().await?).await?;
        decode_optional(&value)
    }
}

fn decode_optional(value: &Value) -> RelayResult<Option<Vec<u8>>> {
    match value["ciphertext"].as_str() {
        Some(text) => crate::keys::unb64(text)
            .map(Some)
            .map_err(|e| RelayError { status: None, message: format!("bad ciphertext: {e}") }),
        None => Ok(None),
    }
}
