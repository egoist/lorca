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
    /// The relay no longer serves the protocol this build speaks. Trying again changes
    /// nothing; a newer Lorca does.
    pub fn is_update_required(&self) -> bool {
        self.status == Some(426)
    }
    /// The relay refused the request, and the same request would be refused again.
    pub fn is_client_error(&self) -> bool {
        matches!(self.status, Some(400..=499)) && !self.is_rate_limited()
    }
    /// Over the relay's rate limit: the same request goes through after a wait.
    pub fn is_rate_limited(&self) -> bool {
        self.status == Some(429)
    }
}

impl From<reqwest::Error> for RelayError {
    fn from(error: reqwest::Error) -> Self {
        // reqwest's own words name the request ("error sending request for url (…)"); what went
        // wrong with it is the last cause in the chain ("Connection refused (os error 61)").
        let mut cause: &dyn std::error::Error = &error;
        while let Some(source) = cause.source() {
            cause = source;
        }
        RelayError { status: error.status().map(|s| s.as_u16()), message: format!("relay unreachable: {cause}") }
    }
}

pub type RelayResult<T> = Result<T, RelayError>;

/// One message of a chat as the relay pages it: where it sits in the log, and its first and
/// latest version (or its removal).
#[derive(Debug, Clone, Deserialize)]
pub struct GroupSlot {
    pub place: i64,
    pub blobs: Vec<BlobIn>,
}

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
    pub last_seen: i64,
    pub created_at: i64,
    /// The machine has a sync socket open on the relay.
    #[serde(default)]
    pub online: bool,
}

/// What the relay said over the sync socket since the caller last looked. It carries no data:
/// a Device pulls after it.
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct Signals {
    /// The identity has a blob this machine may read.
    pub blobs: bool,
    /// The machine list or a machine's presence changed.
    pub machines: bool,
}

/// This machine's sync socket. It is online on the relay while this is open. A task of its own
/// reads it, so the relay's pings are answered and its close is noticed while a sync round runs:
/// a busy turn writes a blob a second, the relay signals each one, and a socket read once per
/// round fell behind until the relay heard no pong and showed this Device offline. Dropping it
/// closes the socket.
pub struct SyncSocket {
    shared: Arc<SocketShared>,
    reader: tokio::task::JoinHandle<()>,
}

#[derive(Default)]
struct SocketShared {
    heard: Mutex<Heard>,
    /// Something was heard, or the socket ended.
    changed: tokio::sync::Notify,
    /// The caller asked for a probe.
    probe: tokio::sync::Notify,
}

#[derive(Default)]
struct Heard {
    signals: Signals,
    ended: Option<String>,
}

type SocketStream = tokio_tungstenite::WebSocketStream<tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>>;

/// The relay pings every 25 s. A socket silent for this long is dead: the computer slept, the
/// phone changed networks.
const SOCKET_SILENCE: std::time::Duration = std::time::Duration::from_secs(70);
/// How long the relay has to answer a probe.
const PROBE_WAIT: std::time::Duration = std::time::Duration::from_secs(10);

impl SyncSocket {
    fn start(stream: SocketStream) -> SyncSocket {
        let shared = Arc::new(SocketShared::default());
        let reader = tokio::spawn(read_socket(stream, shared.clone()));
        SyncSocket { shared, reader }
    }

    /// Every signal since the last call, once there is one. An error means the socket is gone
    /// and the caller connects again.
    pub async fn next(&mut self) -> RelayResult<Signals> {
        loop {
            let changed = self.shared.changed.notified();
            tokio::pin!(changed);
            changed.as_mut().enable();
            {
                let mut heard = self.shared.heard.lock().unwrap();
                let signals = std::mem::take(&mut heard.signals);
                if signals != Signals::default() {
                    return Ok(signals);
                }
                if let Some(message) = &heard.ended {
                    return Err(RelayError { status: None, message: message.clone() });
                }
            }
            changed.await;
        }
    }

    /// Pings the relay, which has `PROBE_WAIT` to answer before `next` gives the socket up: a
    /// phone's may have died while the app was suspended, with nothing said on it since.
    pub fn probe(&self) {
        self.shared.probe.notify_one();
    }
}

impl Drop for SyncSocket {
    fn drop(&mut self) {
        self.reader.abort();
    }
}

/// Reads the socket until it ends, answering pings as they come.
async fn read_socket(mut stream: SocketStream, shared: Arc<SocketShared>) {
    let ended = read_until_end(&mut stream, &shared).await;
    shared.heard.lock().unwrap().ended = Some(ended);
    shared.changed.notify_one();
}

/// Why the socket ended.
async fn read_until_end(stream: &mut SocketStream, shared: &SocketShared) -> String {
    use futures::{SinkExt, StreamExt};
    use tokio_tungstenite::tungstenite::Message;
    // When the relay has to have said something, a ping of its own or the answer to a probe.
    let mut due = tokio::time::Instant::now() + SOCKET_SILENCE;
    loop {
        let message = tokio::select! {
            message = tokio::time::timeout_at(due, stream.next()) => message,
            _ = shared.probe.notified() => {
                due = due.min(tokio::time::Instant::now() + PROBE_WAIT);
                match tokio::time::timeout_at(due, stream.send(Message::Ping(Default::default()))).await {
                    Ok(Ok(())) => continue,
                    Ok(Err(error)) => return socket_error(error).message,
                    Err(_) => return "the sync socket took no ping".into(),
                }
            }
        };
        let message = match message {
            Err(_) => return "the sync socket went silent".into(),
            Ok(None) => return "the relay closed the sync socket".into(),
            Ok(Some(Err(error))) => return socket_error(error).message,
            Ok(Some(Ok(message))) => message,
        };
        due = tokio::time::Instant::now() + SOCKET_SILENCE;
        let signal = match message {
            Message::Text(text) => serde_json::from_str::<Value>(&text).ok().and_then(|v| v["type"].as_str().map(str::to_string)),
            // The pong has to be flushed by hand when nothing else is written.
            Message::Ping(_) => {
                if let Err(error) = stream.flush().await {
                    return socket_error(error).message;
                }
                None
            }
            Message::Close(_) => return "the relay closed the sync socket".into(),
            _ => None,
        };
        let mut heard = shared.heard.lock().unwrap();
        match signal.as_deref() {
            Some("blobs") => heard.signals.blobs = true,
            Some("machines") => heard.signals.machines = true,
            _ => continue,
        }
        drop(heard);
        shared.changed.notify_one();
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

/// TLS for `wss://`: an upgrade is an HTTP/1.1 request, so it offers no ALPN.
fn tls() -> Arc<rustls::ClientConfig> {
    Arc::new(lorca_tls::client_config(&[]))
}

/// The relay protocol this client speaks, sent as `Lorca-Protocol` with every request. A
/// relay may refuse one it no longer serves with `426`. 1: group paging, `DELETE /v1/identity`.
/// 2: `POST /v1/machines`. 3: durable subject records (tasks, reviews, attention, shared project context) and sealed event envelopes.
/// 4: signed envelopes between Devices and generations of the account key, which a Device before
/// them can neither open nor read.
pub const PROTOCOL: u32 = 4;

/// What a relay's auth challenge looks like: base64url, as long as a nonce is. Nothing else is
/// signed in answer, so the challenge cannot get the machine key to sign an envelope or anything
/// else that begins with a `/` or a NUL.
pub fn is_challenge_nonce(nonce: &str) -> bool {
    (16..=128).contains(&nonce.len()) && nonce.bytes().all(|byte| byte.is_ascii_alphanumeric() || byte == b'-' || byte == b'_')
}

const FILE_TRANSFER_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(10 * 60);

pub struct RelayClient {
    /// Replaced by `reset_connections`, and its pooled connections with it.
    http: Mutex<reqwest::Client>,
    token: Mutex<Option<(String, i64)>>,
}

impl RelayClient {
    pub fn new() -> anyhow::Result<Self> {
        Ok(RelayClient { http: Mutex::new(Self::http_client()?), token: Mutex::new(None) })
    }

    /// The relay's own client: what it says about this build goes to the relay and never to
    /// a provider.
    fn http_client() -> anyhow::Result<reqwest::Client> {
        let mut headers = reqwest::header::HeaderMap::new();
        headers.insert("lorca-protocol", reqwest::header::HeaderValue::from(PROTOCOL));
        Ok(lorca_tls::client_builder()
            .timeout(std::time::Duration::from_secs(60))
            .user_agent(format!("lorca/{} ({})", crate::config::VERSION, std::env::consts::OS))
            .default_headers(headers)
            .build()?)
    }

    fn http(&self) -> reqwest::Client {
        self.http.lock().unwrap().clone()
    }

    /// Requests from here on open new connections. A phone's pooled ones may have died while
    /// the app was suspended, some without a word (a VPN on the phone keeps its end open), and
    /// a request sent on one of those waits out its whole timeout.
    pub fn reset_connections(&self) {
        match Self::http_client() {
            Ok(http) => *self.http.lock().unwrap() = http,
            Err(error) => tracing::warn!(%error, "building the relay client"),
        }
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

    /// Answers the protocol the relay speaks; 0 from one that does not say.
    pub async fn health(&self, url: &str) -> RelayResult<u32> {
        let value = Self::check(self.http().get(format!("{url}/v1/health")).send().await?).await?;
        Ok(value["protocol"].as_u64().unwrap_or(0) as u32)
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
        Self::check(self.http().post(format!("{url}/v1/identities")).json(&body).send().await?).await?;
        Ok(())
    }

    /// The machine behind the bearer attests another for its identity: how a Device without
    /// the identity key pairs one. Protocol 2.
    pub async fn attest(&self, url: &str, token: &str, machine_pubkey: &str, box_pubkey: &str) -> RelayResult<()> {
        let body = json!({ "machine_pubkey": machine_pubkey, "box_pubkey": box_pubkey });
        Self::check(self.http().post(format!("{url}/v1/machines")).bearer_auth(token).json(&body).send().await?).await?;
        Ok(())
    }

    pub async fn authenticate(&self, url: &str, machine: &Machine) -> RelayResult<String> {
        let challenge = Self::check(
            self.http()
                .post(format!("{url}/v1/auth/challenge"))
                .json(&json!({ "machine_pubkey": machine.pubkey() }))
                .send()
                .await?,
        )
        .await?;
        let nonce = challenge["nonce"].as_str().ok_or_else(|| RelayError { status: None, message: "no nonce".into() })?;
        // The relay picks what the machine key signs here, and that key signs envelopes too.
        if !is_challenge_nonce(nonce) {
            return Err(RelayError { status: None, message: "the relay's challenge is not a nonce".into() });
        }
        let verified = Self::check(
            self.http()
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

    pub async fn put_blob(&self, url: &str, token: &str, item: crate::app::OutboxItem) -> RelayResult<i64> {
        if item.kind == "file" {
            let mut request = self.http().put(format!("{url}/v1/files/{}", item.id))
                .bearer_auth(token)
                .header(reqwest::header::CONTENT_TYPE, "application/octet-stream")
                .timeout(FILE_TRANSFER_TIMEOUT);
            if let Some(group) = &item.group {
                request = request.query(&[("group", group)]);
            }
            let value = Self::check(request.body(item.ciphertext).send().await?).await?;
            return Ok(value["seq"].as_i64().unwrap_or(0));
        }
        let mut body = json!({ "id": item.id, "kind": item.kind, "recipient_machine_pubkey": item.recipient, "ciphertext": b64(&item.ciphertext) });
        if let Some(slot) = &item.slot {
            body["slot"] = json!(slot.name);
            body["keep_first"] = json!(slot.keep_first);
        }
        if let Some(group) = &item.group {
            body["group"] = json!(group);
        }
        let value = Self::check(
            self.http()
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
        request.headers_mut().insert("lorca-protocol", PROTOCOL.into());
        let connector = tokio_tungstenite::Connector::Rustls(tls());
        let connect = tokio_tungstenite::connect_async_tls_with_config(request, None, false, Some(connector));
        let (stream, _) = tokio::time::timeout(std::time::Duration::from_secs(20), connect)
            .await
            .map_err(|_| RelayError { status: None, message: "the sync socket timed out connecting".into() })?
            .map_err(socket_error)?;
        Ok(SyncSocket::start(stream))
    }

    pub async fn list_blobs(&self, url: &str, token: &str, since: i64, kinds: &str) -> RelayResult<(Vec<BlobIn>, i64)> {
        let value = Self::check(
            self.http()
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

    /// A chat backwards: the `limit` messages placed below `before` (the newest without it),
    /// oldest first, and whether older ones remain.
    pub async fn group_page(&self, url: &str, token: &str, group: &str, before: Option<i64>, limit: usize) -> RelayResult<(Vec<GroupSlot>, bool)> {
        let mut query = vec![("limit", limit.to_string())];
        if let Some(before) = before {
            query.push(("before", before.to_string()));
        }
        let value = Self::check(
            self.http().get(format!("{url}/v1/groups/{group}/blobs")).bearer_auth(token).query(&query).timeout(std::time::Duration::from_secs(60)).send().await?,
        )
        .await?;
        let slots = serde_json::from_value(value["slots"].clone()).map_err(|e| RelayError { status: None, message: format!("group page: {e}") })?;
        Ok((slots, value["has_more"].as_bool().unwrap_or(false)))
    }

    /// An attachment's encrypted bytes; `None` when it is no longer stored for this identity.
    pub async fn get_file(&self, url: &str, token: &str, id: &str) -> RelayResult<Option<Vec<u8>>> {
        let mut response = self.http().get(format!("{url}/v1/files/{id}")).bearer_auth(token).timeout(FILE_TRANSFER_TIMEOUT).send().await?;
        if response.status() == reqwest::StatusCode::NOT_FOUND {
            return Ok(None);
        }
        if !response.status().is_success() {
            Self::check(response).await?;
            unreachable!();
        }
        if response.headers().get(reqwest::header::CONTENT_TYPE).and_then(|v| v.to_str().ok()) != Some("application/octet-stream") {
            return Err(RelayError { status: None, message: "Relay returned an invalid attachment content type".into() });
        }
        let max = crate::files::MAX_ATTACHMENT_BYTES as usize + crate::crypto::ENVELOPE_OVERHEAD;
        let too_large = || RelayError { status: None, message: "Relay attachment exceeds the file size limit".into() };
        if response.content_length().is_some_and(|size| size > max as u64) {
            return Err(too_large());
        }
        let mut bytes = Vec::with_capacity(response.content_length().unwrap_or(0) as usize);
        while let Some(chunk) = response.chunk().await? {
            if chunk.len() > max - bytes.len() {
                return Err(too_large());
            }
            bytes.extend_from_slice(&chunk);
        }
        Ok(Some(bytes))
    }

    pub async fn delete_blob(&self, url: &str, token: &str, id: &str) -> RelayResult<()> {
        Self::check(self.http().delete(format!("{url}/v1/blobs/{id}")).bearer_auth(token).send().await?).await?;
        Ok(())
    }

    /// Deletes every blob of a group. Good to repeat.
    pub async fn delete_group(&self, url: &str, token: &str, group: &str) -> RelayResult<()> {
        Self::check(self.http().delete(format!("{url}/v1/groups/{group}")).bearer_auth(token).send().await?).await?;
        Ok(())
    }

    // MARK: - Shared links

    /// Puts a shared link's ciphertext under `id`, replacing what the id held.
    pub async fn put_share(&self, url: &str, token: &str, id: &str, ciphertext: Vec<u8>) -> RelayResult<()> {
        let request = self.http().put(format!("{url}/v1/shares/{id}")).bearer_auth(token);
        Self::check(request.header(reqwest::header::CONTENT_TYPE, "application/octet-stream").body(ciphertext).send().await?).await?;
        Ok(())
    }

    /// A shared link's ciphertext, which anyone may read; `None` when the relay holds nothing
    /// under `id`.
    pub async fn get_share(&self, url: &str, id: &str) -> RelayResult<Option<Vec<u8>>> {
        let mut response = self.http().get(format!("{url}/v1/shares/{id}")).send().await?;
        if !response.status().is_success() {
            let error = Self::check(response).await.err().unwrap_or_else(|| RelayError { status: None, message: "Unexpected answer".into() });
            // A relay older than shared links has no such route and says so without a word.
            return if error.status == Some(404) && error.message == "No such link" { Ok(None) } else { Err(error) };
        }
        let max = crate::templates::format::MAX_BYTES + crate::crypto::ENVELOPE_OVERHEAD;
        let too_large = || RelayError { status: None, message: "The shared link is larger than a template".into() };
        let mut bytes = Vec::new();
        while let Some(chunk) = response.chunk().await? {
            if chunk.len() > max - bytes.len() {
                return Err(too_large());
            }
            bytes.extend_from_slice(&chunk);
        }
        Ok(Some(bytes))
    }

    pub async fn delete_share(&self, url: &str, token: &str, id: &str) -> RelayResult<()> {
        Self::check(self.http().delete(format!("{url}/v1/shares/{id}")).bearer_auth(token).send().await?).await?;
        Ok(())
    }

    /// The link a user follows to set a receiver up for this account: where the GitHub App
    /// installs, or where they authorize it when it is installed on `subject`'s repository. A
    /// relay without the receiver answers 404.
    pub async fn receiver_setup(&self, url: &str, token: &str, receiver: &str, subject: Option<&str>) -> RelayResult<Value> {
        Self::check(self.http().post(format!("{url}/v1/receivers/{receiver}/setup")).bearer_auth(token).json(&json!({ "subject": subject })).send().await?).await
    }

    /// Asks a receiver to seal its events about `subject` to this machine under a subscription's
    /// secret. Answers `status`: `subscribed`, `needs_setup` (with `setup_url`), or `refused`
    /// (with `message`). A relay without the receiver answers 404.
    pub async fn receiver_subscribe(&self, url: &str, token: &str, receiver: &str, body: &Value) -> RelayResult<Value> {
        Self::check(self.http().post(format!("{url}/v1/receivers/{receiver}/subscriptions")).bearer_auth(token).json(body).send().await?).await
    }

    /// Replaces the hash of the key a receiver's senders include.
    pub async fn receiver_key(&self, url: &str, token: &str, receiver: &str, id: &str, key_hash: &str) -> RelayResult<()> {
        Self::check(self.http().post(format!("{url}/v1/receivers/{receiver}/subscriptions/{id}/key")).bearer_auth(token).json(&json!({ "key_hash": key_hash })).send().await?).await.map(|_| ())
    }

    pub async fn receiver_unsubscribe(&self, url: &str, token: &str, receiver: &str, id: &str) -> RelayResult<()> {
        Self::check(self.http().delete(format!("{url}/v1/receivers/{receiver}/subscriptions/{id}")).bearer_auth(token).send().await?).await.map(|_| ())
    }

    pub async fn machines(&self, url: &str, token: &str) -> RelayResult<(Vec<MachineIn>, i64)> {
        let value = Self::check(self.http().get(format!("{url}/v1/machines")).bearer_auth(token).send().await?).await?;
        let machines: Vec<MachineIn> = serde_json::from_value(value["machines"].clone()).unwrap_or_default();
        Ok((machines, value["now"].as_i64().unwrap_or(now_unix())))
    }

    // MARK: - Push

    /// Where this phone takes pushes: its APNs or FCM device token.
    pub async fn put_push_token(&self, url: &str, token: &str, platform: &str, device_token: &str, environment: Option<&str>) -> RelayResult<()> {
        Self::check(
            self.http()
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
        Self::check(self.http().delete(format!("{url}/v1/push/token")).bearer_auth(token).send().await?).await?;
        Ok(())
    }

    /// Asks the relay to push this ciphertext to the identity's phones; answers how many it queued.
    pub async fn push(&self, url: &str, token: &str, ciphertext_b64: &str) -> RelayResult<u64> {
        let value = Self::check(self.http().post(format!("{url}/v1/push")).bearer_auth(token).json(&json!({ "ciphertext": ciphertext_b64 })).send().await?).await?;
        Ok(value["queued"].as_u64().unwrap_or(0))
    }

    // MARK: - Pairing mailbox

    /// Unpairs a machine of this identity, this one included.
    pub async fn revoke_machine(&self, url: &str, token: &str, machine_pubkey: &str) -> RelayResult<()> {
        Self::check(self.http().delete(format!("{url}/v1/machines/{machine_pubkey}")).bearer_auth(token).send().await?).await?;
        Ok(())
    }

    /// Deletes the identity and everything the relay holds for it; every Device gets `410`.
    pub async fn delete_identity(&self, url: &str, token: &str) -> RelayResult<()> {
        Self::check(self.http().delete(format!("{url}/v1/identity")).bearer_auth(token).send().await?).await?;
        Ok(())
    }

    pub async fn pair_create(&self, url: &str, token: &str) -> RelayResult<String> {
        let value = Self::check(self.http().post(format!("{url}/v1/pair")).bearer_auth(token).send().await?).await?;
        Ok(value["nonce"].as_str().unwrap_or_default().to_string())
    }

    pub async fn pair_post_request(&self, url: &str, nonce: &str, ciphertext: &[u8]) -> RelayResult<()> {
        Self::check(
            self.http()
                .post(format!("{url}/v1/pair/{nonce}/request"))
                .json(&json!({ "ciphertext": b64(ciphertext) }))
                .send()
                .await?,
        )
        .await?;
        Ok(())
    }

    pub async fn pair_get_request(&self, url: &str, token: &str, nonce: &str) -> RelayResult<Option<Vec<u8>>> {
        let value = Self::check(self.http().get(format!("{url}/v1/pair/{nonce}/request")).bearer_auth(token).send().await?).await?;
        decode_optional(&value)
    }

    pub async fn pair_post_reply(&self, url: &str, token: &str, nonce: &str, ciphertext: &[u8]) -> RelayResult<()> {
        Self::check(
            self.http()
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
        Self::check(self.http().delete(format!("{url}/v1/pair/{nonce}")).bearer_auth(token).send().await?).await?;
        Ok(())
    }

    pub async fn pair_get_reply(&self, url: &str, nonce: &str) -> RelayResult<Option<Vec<u8>>> {
        let value = Self::check(self.http().get(format!("{url}/v1/pair/{nonce}/reply")).send().await?).await?;
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

#[cfg(test)]
mod tests {
    use super::*;
    use futures::{SinkExt, StreamExt};
    use tokio_tungstenite::tungstenite::Message;

    /// A relay that floods the socket with signals while the sync round is busy elsewhere still
    /// gets its pong, and the round then takes every signal at once.
    #[tokio::test]
    async fn a_flooded_socket_answers_pings_while_the_round_is_busy() {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let relay = tokio::spawn(async move {
            let (tcp, _) = listener.accept().await.unwrap();
            let mut socket = tokio_tungstenite::accept_async(tcp).await.unwrap();
            for _ in 0..2000 {
                socket.send(Message::Text(r#"{"type":"blobs"}"#.into())).await.unwrap();
            }
            socket.send(Message::Text(r#"{"type":"machines"}"#.into())).await.unwrap();
            socket.send(Message::Ping(Default::default())).await.unwrap();
            let mut ponged = false;
            let mut probed = false;
            let waited = tokio::time::timeout(std::time::Duration::from_secs(5), async {
                while let Some(Ok(message)) = socket.next().await {
                    match message {
                        Message::Pong(_) => ponged = true,
                        // The Device's probe.
                        Message::Ping(_) => probed = true,
                        _ => {}
                    }
                    if ponged && probed {
                        break;
                    }
                }
            })
            .await;
            assert!(waited.is_ok(), "pong {ponged}, probe {probed}");
            socket.close(None).await.unwrap();
        });

        let (stream, _) = tokio_tungstenite::connect_async(format!("ws://{address}/v1/sync")).await.unwrap();
        let mut socket = SyncSocket::start(stream);
        // The round is busy and reads no signal until the relay has said all it had to.
        while !socket.shared.heard.lock().unwrap().signals.machines {
            tokio::time::sleep(std::time::Duration::from_millis(10)).await;
        }
        socket.probe();
        assert_eq!(socket.next().await.unwrap(), Signals { blobs: true, machines: true });
        relay.await.unwrap();
        assert!(socket.next().await.is_err());
    }
}
