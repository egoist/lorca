//! Vercel Sandbox (vercel.com/docs/vercel-sandbox) over its REST API, with the user's own access
//! token, team, and project. An agent's sandbox is a persistent named one: stopping it keeps its
//! files in a snapshot, and getting it by name again resumes it from there. A command's stdin is
//! written in chunks at the offset the API counts, so a write sent again after a lost answer is
//! not written twice; its output streams as JSON lines, picked up again where it broke off.

use std::collections::BTreeMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use base64::Engine;
use futures::StreamExt;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use tokio::sync::{mpsc, oneshot};

use super::{Chunk, Output, Remote};
use crate::coding::process::{Exit, Process, Stdout};

/// Where a Vercel sandbox starts, and where Lorca clones into.
pub(crate) const ROOT: &str = "/vercel/sandbox";
const API: &str = "https://vercel.com/api";
/// How long a session runs at most when the user sets no limit: the Hobby plan's most.
const DEFAULT_TIMEOUT_MINUTES: u32 = 45;
/// What one stdin request carries at most, keeping its base64 body under the API's 1 MB.
const STDIN_CHUNK: usize = 512 * 1024;
const RETRIES: u32 = 5;
/// How long a new sandbox has to start running.
const START_TIMEOUT: Duration = Duration::from_secs(180);

/// The user's Vercel account, as `sandboxes.json` keeps it.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
pub struct Settings {
    /// An access token from the user's Vercel account settings.
    pub token: String,
    /// The team the project is in; empty for a personal account.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub team_id: String,
    /// The project the sandboxes belong to.
    pub project_id: String,
    /// The image to start from; empty for Vercel's own, which has Node.js, Python, and coding
    /// agents installed.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub image: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub vcpus: Option<u32>,
    /// How long a session runs at most, in minutes.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub timeout_minutes: Option<u32>,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub env: BTreeMap<String, String>,
}

impl Settings {
    /// What every app shows of it.
    pub fn fields(&self) -> Value {
        json!({ "team_id": self.team_id, "project_id": self.project_id, "image": self.image, "vcpus": self.vcpus, "timeout_minutes": self.timeout_minutes })
    }

    /// `saved` with `change` over it.
    pub fn merged(saved: Option<Settings>, change: &Value) -> Result<Settings, String> {
        let saved = saved.unwrap_or_default();
        let text = |key: &str, was: &str| change.get(key).map(|value| value.as_str().unwrap_or("").trim().to_string()).unwrap_or_else(|| was.to_string());
        let number = |key: &str, was: Option<u32>, range: std::ops::RangeInclusive<u32>, what: &str| -> Result<Option<u32>, String> {
            match change.get(key) {
                None => Ok(was),
                Some(Value::Null) => Ok(None),
                Some(value) => match value.as_u64().map(|value| value as u32).filter(|value| range.contains(value)) {
                    Some(value) => Ok(Some(value)),
                    None => Err(format!("{what} is from {} to {}", range.start(), range.end())),
                },
            }
        };
        let settings = Settings {
            token: super::merged_token(Some(&saved.token), change)?,
            team_id: text("team_id", &saved.team_id),
            project_id: text("project_id", &saved.project_id),
            image: text("image", &saved.image),
            vcpus: number("vcpus", saved.vcpus, 1..=8, "vCPUs")?,
            timeout_minutes: number("timeout_minutes", saved.timeout_minutes, 1..=24 * 60, "The time limit in minutes")?,
            env: super::merged_env(&saved.env, change)?,
        };
        if settings.project_id.is_empty() {
            return Err("Vercel Sandbox needs the project's id".into());
        }
        Ok(settings)
    }
}

struct Client {
    http: reqwest::Client,
    settings: Settings,
}

impl Client {
    fn new(settings: &Settings) -> Result<Client, String> {
        let http = lorca_tls::client_builder()
            .connect_timeout(Duration::from_secs(20))
            .user_agent(format!("lorca/{} ({})", crate::config::VERSION, std::env::consts::OS))
            .build()
            .map_err(|e| e.to_string())?;
        Ok(Client { http, settings: settings.clone() })
    }

    fn request(&self, method: reqwest::Method, path: &str, query: &[(&str, &str)]) -> reqwest::RequestBuilder {
        let mut query: Vec<(&str, &str)> = query.to_vec();
        if !self.settings.team_id.is_empty() {
            query.push(("teamId", &self.settings.team_id));
        }
        self.http.request(method, format!("{API}{path}")).bearer_auth(&self.settings.token).query(&query)
    }

    /// Sends `request` and reads its JSON, with what Vercel said when it refused.
    async fn json(&self, request: reqwest::RequestBuilder) -> Result<(reqwest::StatusCode, Value), String> {
        let response = request.timeout(Duration::from_secs(90)).send().await.map_err(|e| format!("Vercel did not answer: {e}"))?;
        let status = response.status();
        let body: Value = response.json().await.unwrap_or(Value::Null);
        Ok((status, body))
    }
}

/// What Vercel said, from an answer that is not a success.
fn refused(status: reqwest::StatusCode, body: &Value) -> String {
    let said = body["error"]["message"].as_str().or(body["message"].as_str()).unwrap_or("");
    match status.as_u16() {
        401 | 403 => format!("Vercel refused the token ({status}){}", if said.is_empty() { String::new() } else { format!(": {said}") }),
        _ if said.is_empty() => format!("Vercel answered {status}"),
        _ => format!("Vercel answered {status}: {said}"),
    }
}

/// An agent's sandbox, running: its session takes commands.
pub(crate) struct Session {
    client: Arc<Client>,
    id: String,
}

/// Resumes the sandbox `name`, or makes it.
pub(crate) async fn open(settings: &Settings, name: &str) -> Result<Session, String> {
    let client = Arc::new(Client::new(settings)?);
    let project = settings.project_id.as_str();
    let (status, body) = client.json(client.request(reqwest::Method::GET, &format!("/v2/sandboxes/{name}"), &[("projectId", project), ("resume", "true")])).await?;
    let body = match status.as_u16() {
        200..=299 => body,
        404 => {
            let mut create = json!({
                "projectId": project,
                "name": name,
                "persistent": true,
                "ports": [],
                "timeout": settings.timeout_minutes.unwrap_or(DEFAULT_TIMEOUT_MINUTES) as u64 * 60_000,
            });
            if let Some(vcpus) = settings.vcpus {
                create["resources"] = json!({ "vcpus": vcpus });
            }
            if !settings.image.is_empty() {
                create["image"] = json!(settings.image);
            }
            let (status, body) = client.json(client.request(reqwest::Method::POST, "/v3/sandboxes", &[]).json(&create)).await?;
            if !status.is_success() {
                return Err(refused(status, &body));
            }
            body
        }
        _ => return Err(refused(status, &body)),
    };
    let id = body["session"]["id"].as_str().ok_or("Vercel started no session")?.to_string();
    let session = Session { client, id };
    session.running(body["session"]["status"].as_str().unwrap_or("")).await?;
    Ok(session)
}

/// Deletes the sandbox `name` and its snapshots.
pub(crate) async fn delete(settings: &Settings, name: &str) -> Result<(), String> {
    let client = Client::new(settings)?;
    let (status, body) = client.json(client.request(reqwest::Method::DELETE, &format!("/v2/sandboxes/{name}"), &[("projectId", &settings.project_id), ("deleteOrphanSnapshots", "true")])).await?;
    match status.as_u16() {
        200..=299 | 404 => Ok(()),
        _ => Err(refused(status, &body)),
    }
}

/// Whether the token reaches the project's sandboxes.
pub(crate) async fn check(settings: &Settings) -> Result<(), String> {
    let client = Client::new(settings)?;
    let (status, body) = client.json(client.request(reqwest::Method::GET, "/v2/sandboxes", &[("project", &settings.project_id), ("limit", "1")])).await?;
    if status.is_success() {
        Ok(())
    } else {
        Err(refused(status, &body))
    }
}

impl Session {
    fn path(&self, rest: &str) -> String {
        format!("/v2/sandboxes/sessions/{}{rest}", self.id)
    }

    /// Waits for a session that is still starting.
    async fn running(&self, status: &str) -> Result<(), String> {
        let mut status = status.to_string();
        let started = tokio::time::Instant::now();
        while status == "pending" || status.is_empty() {
            if started.elapsed() > START_TIMEOUT {
                return Err("The Vercel sandbox did not start in time".into());
            }
            tokio::time::sleep(Duration::from_secs(1)).await;
            let (code, body) = self.client.json(self.client.request(reqwest::Method::GET, &self.path(""), &[])).await?;
            if !code.is_success() {
                return Err(refused(code, &body));
            }
            status = body["session"]["status"].as_str().unwrap_or("").to_string();
        }
        match status.as_str() {
            "running" => Ok(()),
            other => Err(format!("The Vercel sandbox is {other}")),
        }
    }

    /// Runs `argv` and waits for it.
    pub async fn run(&self, argv: &[String], cwd: &str, env: &BTreeMap<String, String>) -> Result<Output, String> {
        let body = json!({ "command": argv[0], "args": &argv[1..], "cwd": cwd, "env": env, "wait": true, "logs": true });
        let response = self
            .client
            .request(reqwest::Method::POST, &self.path("/cmd"), &[])
            .json(&body)
            .timeout(Duration::from_secs(600))
            .send()
            .await
            .map_err(|e| format!("Vercel did not answer: {e}"))?;
        if !response.status().is_success() {
            let status = response.status();
            return Err(refused(status, &response.json().await.unwrap_or(Value::Null)));
        }
        let mut output = Output::default();
        let mut lines = json_lines(response);
        while let Some(line) = lines.next().await {
            let line = line?;
            match line["stream"].as_str() {
                Some("stdout") => output.stdout.push_str(line["data"].as_str().unwrap_or("")),
                Some("stderr") => output.stderr.push_str(line["data"].as_str().unwrap_or("")),
                Some("error") => return Err(line["data"]["message"].as_str().unwrap_or("The command failed in the sandbox").to_string()),
                _ => {
                    if let Some(code) = line["command"]["exitCode"].as_i64() {
                        output.code = Some(code as i32);
                    }
                }
            }
        }
        Ok(output)
    }

    /// Starts `argv` with its stdin open, and follows it.
    pub async fn spawn(&self, argv: &[String], cwd: &str, env: &BTreeMap<String, String>) -> Result<(Arc<Process>, Stdout, oneshot::Receiver<Exit>), String> {
        let body = json!({ "command": argv[0], "args": &argv[1..], "cwd": cwd, "env": env, "attachStdin": true });
        let (status, answer) = self.client.json(self.client.request(reqwest::Method::POST, &self.path("/cmd"), &[]).json(&body)).await?;
        if !status.is_success() {
            return Err(refused(status, &answer));
        }
        let command = answer["command"]["id"].as_str().ok_or("Vercel started no command")?.to_string();
        let remote = Arc::new(Command { client: self.client.clone(), session: self.id.clone(), id: command, written: AtomicU64::new(0) });
        let (tx, rx) = mpsc::unbounded_channel();
        tokio::spawn(follow(remote.clone(), tx));
        Ok(super::attach(remote, rx, "Vercel Sandbox".into()))
    }

    /// Stops the session; the sandbox keeps its files for the next.
    pub async fn pause(&self) {
        let _ = self.client.json(self.client.request(reqwest::Method::POST, &self.path("/stop"), &[])).await;
    }
}

/// A command running in a session.
struct Command {
    client: Arc<Client>,
    session: String,
    id: String,
    /// How much of its stdin the sandbox has.
    written: AtomicU64,
}

impl Command {
    fn path(&self, rest: &str) -> String {
        format!("/v2/sandboxes/sessions/{}/cmd/{}{rest}", self.session, self.id)
    }
}

#[async_trait]
impl Remote for Command {
    async fn write(&self, bytes: Vec<u8>) -> Result<(), String> {
        for chunk in bytes.chunks(STDIN_CHUNK) {
            let offset = self.written.load(Ordering::SeqCst);
            let body = json!({ "data": base64::engine::general_purpose::STANDARD.encode(chunk), "offset": offset });
            let mut attempt = 0;
            loop {
                let sent = self.client.request(reqwest::Method::POST, &self.path("/stdin"), &[]).json(&body).timeout(Duration::from_secs(60)).send().await;
                // The offset makes a resend safe: the sandbox skips what it has.
                let retry = match &sent {
                    Err(_) => true,
                    Ok(response) => matches!(response.status().as_u16(), 429 | 502 | 503 | 504),
                };
                if retry && attempt < RETRIES {
                    tokio::time::sleep(Duration::from_millis(400 * 2u64.pow(attempt))).await;
                    attempt += 1;
                    continue;
                }
                let response = sent.map_err(|e| e.to_string())?;
                let status = response.status();
                let answer: Value = response.json().await.unwrap_or(Value::Null);
                if !status.is_success() {
                    return Err(refused(status, &answer));
                }
                let written = answer["bytesWritten"].as_u64().unwrap_or(offset + chunk.len() as u64);
                self.written.store(written, Ordering::SeqCst);
                break;
            }
        }
        Ok(())
    }

    async fn kill(&self) {
        let _ = self.client.json(self.client.request(reqwest::Method::POST, &self.path("/kill"), &[]).json(&json!({ "signal": 15 }))).await;
    }
}

/// Streams what the command prints to `chunks`, then how it ended. A stream that breaks off is
/// asked for again, skipping what came already.
async fn follow(command: Arc<Command>, chunks: mpsc::UnboundedSender<Chunk>) {
    // What came of each stream, as the API counts it (UTF-16 code units, as JavaScript does).
    let (mut stdout, mut stderr) = (0usize, 0usize);
    let mut broke = 0;
    loop {
        let response = command.client.request(reqwest::Method::GET, &command.path("/logs"), &[]).send().await;
        let response = match response {
            Ok(response) if response.status().is_success() => response,
            Ok(response) if response.status().as_u16() == 404 => break,
            _ if broke < 3 => {
                broke += 1;
                tokio::time::sleep(Duration::from_millis(500 * broke)).await;
                continue;
            }
            _ => {
                let _ = chunks.send(Chunk::Stderr(b"Lost the connection to the Vercel sandbox".to_vec()));
                return;
            }
        };
        let (mut skip_out, mut skip_err) = (stdout, stderr);
        let mut lines = json_lines(response);
        let mut ended = true;
        while let Some(line) = lines.next().await {
            let Ok(line) = line else {
                ended = false;
                break;
            };
            let data = line["data"].as_str().unwrap_or("");
            let (seen, skip, kind): (&mut usize, &mut usize, fn(Vec<u8>) -> Chunk) = match line["stream"].as_str() {
                Some("stdout") => (&mut stdout, &mut skip_out, Chunk::Stdout),
                Some("stderr") => (&mut stderr, &mut skip_err, Chunk::Stderr),
                Some("error") => {
                    let said = line["data"]["message"].as_str().unwrap_or("The sandbox stopped the command");
                    let _ = chunks.send(Chunk::Stderr(said.as_bytes().to_vec()));
                    continue;
                }
                _ => continue,
            };
            let units: Vec<u16> = data.encode_utf16().collect();
            let skipped = (*skip).min(units.len());
            *skip -= skipped;
            *seen += units.len() - skipped;
            if skipped < units.len() {
                let _ = chunks.send(kind(String::from_utf16_lossy(&units[skipped..]).into_bytes()));
            }
        }
        if ended {
            break;
        }
        broke += 1;
        if broke > 3 {
            let _ = chunks.send(Chunk::Stderr(b"Lost the connection to the Vercel sandbox".to_vec()));
            return;
        }
    }
    // The output is over: the command has ended, or is about to.
    for _ in 0..RETRIES {
        let answer = command.client.json(command.client.request(reqwest::Method::GET, &command.path(""), &[("wait", "true")])).await;
        if let Ok((status, body)) = answer {
            if status.is_success() {
                let _ = chunks.send(Chunk::Exit(body["command"]["exitCode"].as_i64().map(|code| code as i32)));
                return;
            }
            if status.as_u16() == 404 {
                break;
            }
        }
        tokio::time::sleep(Duration::from_secs(1)).await;
    }
    let _ = chunks.send(Chunk::Exit(None));
}

/// The JSON values of a response's lines; an error for a stream that broke off mid-line.
fn json_lines(response: reqwest::Response) -> impl futures::Stream<Item = Result<Value, String>> + Unpin {
    let bytes = response.bytes_stream();
    Box::pin(futures::stream::unfold((bytes, Vec::<u8>::new(), false), |(mut bytes, mut buffer, done)| async move {
        if done {
            return None;
        }
        loop {
            if let Some(end) = buffer.iter().position(|byte| *byte == b'\n') {
                let line: Vec<u8> = buffer.drain(..=end).collect();
                let text = String::from_utf8_lossy(&line);
                let text = text.trim();
                if text.is_empty() {
                    continue;
                }
                return Some((serde_json::from_str::<Value>(text).map_err(|e| e.to_string()), (bytes, buffer, false)));
            }
            match bytes.next().await {
                Some(Ok(chunk)) => buffer.extend_from_slice(&chunk),
                Some(Err(error)) => return Some((Err(error.to_string()), (bytes, buffer, true))),
                None => {
                    let rest = String::from_utf8_lossy(&buffer).trim().to_string();
                    if rest.is_empty() {
                        return None;
                    }
                    return Some((serde_json::from_str::<Value>(&rest).map_err(|_| "The stream broke off".to_string()), (bytes, Vec::new(), true)));
                }
            }
        }
    }))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_change_needs_a_project_and_keeps_the_saved_token() {
        assert!(Settings::merged(None, &json!({ "token": "t0ken" })).is_err());
        assert!(Settings::merged(None, &json!({ "project_id": "prj_1" })).is_err());
        let saved = Settings::merged(None, &json!({ "token": "t0ken", "project_id": "prj_1", "vcpus": 2 })).unwrap();
        let changed = Settings::merged(Some(saved), &json!({ "team_id": "team_1", "vcpus": null })).unwrap();
        assert_eq!((changed.token.as_str(), changed.team_id.as_str(), changed.vcpus), ("t0ken", "team_1", None));
        assert!(Settings::merged(Some(changed), &json!({ "vcpus": 64 })).is_err());
    }
}
