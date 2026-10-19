//! Cloudflare Sandbox through the Worker the user deploys to their own Cloudflare account from
//! `sandboxes/cloudflare`: a Durable Object per sandbox that starts its Container, runs commands
//! in it, and passes a process's stdin, stdout, and stderr over a WebSocket. The Runner reaches
//! it at the Worker's address with the token the user set as the Worker's `LORCA_TOKEN` secret.
//!
//! The Worker's API: `GET /health`; `POST /sandboxes/{name}/start`; `POST /sandboxes/{name}/exec`
//! `{ argv, cwd, env }`, answered `{ exitCode, stdout, stderr }`; `GET /sandboxes/{name}/spawn`,
//! a WebSocket whose first message is `{ argv, cwd, env }`, then binary messages are stdin one way
//! and output the other (a first byte of 1 for stdout, 2 for stderr), `{ "signal": 15 }` stops the
//! process, and `{ "exit": code }` says it ended; `DELETE /sandboxes/{name}` ends the Container.

use std::collections::BTreeMap;
use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use futures::{SinkExt, StreamExt};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use tokio::sync::{mpsc, oneshot};
use tokio_tungstenite::tungstenite::client::IntoClientRequest;
use tokio_tungstenite::tungstenite::Message as Frame;

use super::{Chunk, Output, Remote};
use crate::coding::process::{Exit, Process, Stdout};

/// Where the Worker's Container starts, and where Lorca clones into.
pub(crate) const ROOT: &str = "/workspace";
/// The version of the Worker's API this Runner speaks, which `/health` answers.
const PROTOCOL: u64 = 1;

/// The user's Worker, as `sandboxes.json` keeps it.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
pub struct Settings {
    /// The Worker's address: `https://lorca-sandbox.example.workers.dev`.
    pub url: String,
    /// The Worker's `LORCA_TOKEN` secret.
    pub token: String,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub env: BTreeMap<String, String>,
}

impl Settings {
    /// What every app shows of it.
    pub fn fields(&self) -> Value {
        json!({ "url": self.url })
    }

    /// `saved` with `change` over it.
    pub fn merged(saved: Option<Settings>, change: &Value) -> Result<Settings, String> {
        let saved = saved.unwrap_or_default();
        let url = change.get("url").map(|url| url.as_str().unwrap_or("").trim().trim_end_matches('/').to_string()).unwrap_or(saved.url.clone());
        let local = url.starts_with("http://localhost") || url.starts_with("http://127.0.0.1");
        if !(url.starts_with("https://") || local) || reqwest::Url::parse(&url).is_err() {
            return Err("Cloudflare Sandbox needs the Worker's https address".into());
        }
        Ok(Settings { url, token: super::merged_token(Some(&saved.token), change)?, env: super::merged_env(&saved.env, change)? })
    }
}

/// The Worker, for one sandbox.
pub(crate) struct Client {
    http: reqwest::Client,
    settings: Settings,
    name: String,
}

fn http() -> Result<reqwest::Client, String> {
    lorca_tls::client_builder()
        .connect_timeout(Duration::from_secs(20))
        .user_agent(format!("lorca/{} ({})", crate::config::VERSION, std::env::consts::OS))
        .build()
        .map_err(|e| e.to_string())
}

/// What the Worker said, from an answer that is not a success.
async fn refused(response: reqwest::Response) -> String {
    let status = response.status();
    let said = response.text().await.unwrap_or_default();
    let said = said.trim().chars().take(300).collect::<String>();
    match status.as_u16() {
        401 | 403 => "The Worker refused the token: it is not the Worker's LORCA_TOKEN".into(),
        _ if said.is_empty() => format!("The Worker answered {status}"),
        _ => format!("The Worker answered {status}: {said}"),
    }
}

/// Starts the Container of the sandbox `name`, unless it runs.
pub(crate) async fn open(settings: &Settings, name: &str) -> Result<Client, String> {
    let client = Client { http: http()?, settings: settings.clone(), name: name.into() };
    let response = client.http.post(client.url("start")).bearer_auth(&settings.token).timeout(Duration::from_secs(180)).send().await.map_err(|e| format!("The Worker did not answer: {e}"))?;
    if !response.status().is_success() {
        return Err(refused(response).await);
    }
    Ok(client)
}

/// Ends the Container of the sandbox `name`, with its files.
pub(crate) async fn delete(settings: &Settings, name: &str) -> Result<(), String> {
    let client = Client { http: http()?, settings: settings.clone(), name: name.into() };
    let response = client.http.delete(client.url("")).bearer_auth(&settings.token).timeout(Duration::from_secs(60)).send().await.map_err(|e| e.to_string())?;
    match response.status().as_u16() {
        200..=299 | 404 => Ok(()),
        _ => Err(refused(response).await),
    }
}

/// Whether the Worker answers with this token and speaks this Runner's API.
pub(crate) async fn check(settings: &Settings) -> Result<(), String> {
    let response = http()?.get(format!("{}/health", settings.url)).bearer_auth(&settings.token).timeout(Duration::from_secs(30)).send().await.map_err(|e| format!("The Worker did not answer: {e}"))?;
    if !response.status().is_success() {
        return Err(refused(response).await);
    }
    let body: Value = response.json().await.unwrap_or(Value::Null);
    match body["lorca"].as_u64() {
        Some(PROTOCOL) => Ok(()),
        Some(other) => Err(format!("The Worker speaks version {other} of Lorca's sandbox API, and this Runner {PROTOCOL}: deploy it again from sandboxes/cloudflare")),
        None => Err("That address is not Lorca's sandbox Worker".into()),
    }
}

impl Client {
    fn url(&self, rest: &str) -> String {
        match rest {
            "" => format!("{}/sandboxes/{}", self.settings.url, self.name),
            rest => format!("{}/sandboxes/{}/{rest}", self.settings.url, self.name),
        }
    }

    pub async fn run(&self, argv: &[String], cwd: &str, env: &BTreeMap<String, String>) -> Result<Output, String> {
        let response = self
            .http
            .post(self.url("exec"))
            .bearer_auth(&self.settings.token)
            .json(&json!({ "argv": argv, "cwd": cwd, "env": env }))
            .timeout(Duration::from_secs(600))
            .send()
            .await
            .map_err(|e| format!("The Worker did not answer: {e}"))?;
        if !response.status().is_success() {
            return Err(refused(response).await);
        }
        let body: Value = response.json().await.map_err(|e| e.to_string())?;
        Ok(Output {
            code: body["exitCode"].as_i64().map(|code| code as i32),
            stdout: body["stdout"].as_str().unwrap_or("").to_string(),
            stderr: body["stderr"].as_str().unwrap_or("").to_string(),
        })
    }

    pub async fn spawn(&self, argv: &[String], cwd: &str, env: &BTreeMap<String, String>) -> Result<(Arc<Process>, Stdout, oneshot::Receiver<Exit>), String> {
        let address = self.url("spawn").replacen("https://", "wss://", 1).replacen("http://", "ws://", 1);
        let mut request = address.into_client_request().map_err(|e| e.to_string())?;
        let auth = format!("Bearer {}", self.settings.token).parse().map_err(|_| "The token is not a header value".to_string())?;
        request.headers_mut().insert("authorization", auth);
        let connector = tokio_tungstenite::Connector::Rustls(Arc::new(lorca_tls::client_config(&[])));
        let connect = tokio_tungstenite::connect_async_tls_with_config(request, None, false, Some(connector));
        let (mut socket, _) = tokio::time::timeout(Duration::from_secs(60), connect)
            .await
            .map_err(|_| "The Worker did not answer in time".to_string())?
            .map_err(|e| format!("Could not reach the Worker: {e}"))?;
        socket.send(Frame::Text(json!({ "argv": argv, "cwd": cwd, "env": env }).to_string().into())).await.map_err(|e| e.to_string())?;
        let (outgoing, mut sending) = mpsc::unbounded_channel::<Frame>();
        let (tx, rx) = mpsc::unbounded_channel();
        tokio::spawn(async move {
            let mut ping = tokio::time::interval(Duration::from_secs(30));
            loop {
                tokio::select! {
                    frame = sending.recv() => {
                        let Some(frame) = frame else { break };
                        if socket.send(frame).await.is_err() {
                            break;
                        }
                    }
                    frame = socket.next() => match frame {
                        Some(Ok(Frame::Binary(bytes))) if !bytes.is_empty() => {
                            let data = bytes[1..].to_vec();
                            let _ = tx.send(if bytes[0] == 2 { Chunk::Stderr(data) } else { Chunk::Stdout(data) });
                        }
                        Some(Ok(Frame::Text(text))) => {
                            let message: Value = serde_json::from_str(&text).unwrap_or(Value::Null);
                            if let Some(error) = message["error"].as_str() {
                                let _ = tx.send(Chunk::Stderr(error.as_bytes().to_vec()));
                            }
                            if message.get("exit").is_some() {
                                let _ = tx.send(Chunk::Exit(message["exit"].as_i64().map(|code| code as i32)));
                                break;
                            }
                        }
                        Some(Ok(Frame::Ping(data))) => {
                            let _ = socket.send(Frame::Pong(data)).await;
                        }
                        Some(Ok(_)) => {}
                        Some(Err(_)) | None => break,
                    },
                    _ = ping.tick() => {
                        if socket.send(Frame::Ping(Vec::new().into())).await.is_err() {
                            break;
                        }
                    }
                }
            }
            let _ = socket.close(None).await;
        });
        Ok(super::attach(Arc::new(Spawned { outgoing }), rx, "Cloudflare Sandbox".into()))
    }
}

/// A process the Worker runs, through its WebSocket.
struct Spawned {
    outgoing: mpsc::UnboundedSender<Frame>,
}

#[async_trait]
impl Remote for Spawned {
    async fn write(&self, bytes: Vec<u8>) -> Result<(), String> {
        self.outgoing.send(Frame::Binary(bytes.into())).map_err(|_| "The connection to the Worker is closed".to_string())
    }

    async fn kill(&self) {
        let _ = self.outgoing.send(Frame::Text(json!({ "signal": 15 }).to_string().into()));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_worker_is_reached_over_https() {
        assert!(Settings::merged(None, &json!({ "url": "http://lorca-sandbox.example.workers.dev", "token": "t0ken" })).is_err());
        assert!(Settings::merged(None, &json!({ "url": "https://lorca-sandbox.example.workers.dev/" })).is_err());
        let settings = Settings::merged(None, &json!({ "url": "https://lorca-sandbox.example.workers.dev/", "token": "t0ken" })).unwrap();
        assert_eq!(settings.url, "https://lorca-sandbox.example.workers.dev");
        assert!(Settings::merged(None, &json!({ "url": "http://localhost:8787", "token": "t0ken" })).is_ok());
    }
}
