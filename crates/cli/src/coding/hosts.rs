//! Terminal hosts that run coding agents in panes the user can watch and type into on the
//! Runner: Herdr (its socket API, newline-delimited JSON at `~/.config/herdr/herdr.sock` or
//! `HERDR_SOCKET_PATH`) and Luvus (its `luvus` command, which answers in JSON). When one runs,
//! an agent starts in a workspace of its own there; the host reports the agent's state
//! (`idle`, `working`, `blocked`, `done`), which Lorca turns into the same events a headless
//! agent sends. What the pane asks while blocked goes to Auto-review or the user
//! (`super::answer_screen`), and only that answer is pressed or typed into it. Claude Code's
//! commands also pass Lorca's review through a `PreToolUse` hook that calls `lorca coding hook`.

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use tokio::sync::mpsc;

use super::driver::{Answer, Driver, Ended, Event, Events};
use super::Agent;
use crate::app::App;

/// How much of an agent's pane is read for its transcript.
const SCREEN_LINES: u32 = 400;
/// How long an agent has to start in its pane.
const START_TIMEOUT: Duration = Duration::from_secs(120);
/// How long a hook waits for Lorca's decision, past the ten minutes a question waits.
const HOOK_TIMEOUT_SECS: u64 = 660;

/// A terminal host running on the Runner.
#[derive(Debug, Clone, PartialEq)]
pub(crate) enum Host {
    Herdr { socket: PathBuf },
    Luvus { program: PathBuf },
}

impl Host {
    pub fn name(&self) -> &'static str {
        match self {
            Host::Herdr { .. } => "herdr",
            Host::Luvus { .. } => "luvus",
        }
    }

    pub fn title(&self) -> &'static str {
        match self {
            Host::Herdr { .. } => "Herdr",
            Host::Luvus { .. } => "Luvus",
        }
    }
}

/// Where an agent runs in its host, kept across restarts.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub(crate) struct Target {
    pub workspace: String,
    pub pane: String,
    /// The agent's name in the host: its Lorca id.
    pub name: String,
}

/// The terminal host running on this Runner, Herdr first. None when neither runs, and in tests
/// unless one says otherwise.
pub(crate) async fn detect() -> Option<Host> {
    #[cfg(test)]
    return tests_support::host();
    #[cfg(not(test))]
    {
        if !cfg!(unix) {
            return None;
        }
        let socket = herdr_socket().await;
        if socket.exists() && tokio::time::timeout(Duration::from_secs(2), herdr(&socket, "ping", json!({}))).await.is_ok_and(|answer| answer.is_ok()) {
            return Some(Host::Herdr { socket });
        }
        let program = super::process::find("luvus").await?;
        let pinged = tokio::time::timeout(Duration::from_secs(3), luvus(&program, &["ping"])).await;
        pinged.is_ok_and(|answer| answer.is_ok()).then_some(Host::Luvus { program })
    }
}

async fn herdr_socket() -> PathBuf {
    let environment = lorca_agent::login_shell::environment().await;
    let named = environment.iter().rev().find(|(key, _)| key == "HERDR_SOCKET_PATH").map(|(_, value)| PathBuf::from(value)).or_else(|| std::env::var_os("HERDR_SOCKET_PATH").map(PathBuf::from));
    named.unwrap_or_else(|| dirs::home_dir().unwrap_or_default().join(".config").join("herdr").join("herdr.sock"))
}

/// One request to Herdr's socket, which has a while to answer: starting an agent takes long.
#[cfg(unix)]
async fn herdr(socket: &Path, method: &str, params: Value) -> Result<Value, String> {
    let limit = if method == "agent.start" { START_TIMEOUT + Duration::from_secs(10) } else { Duration::from_secs(15) };
    tokio::time::timeout(limit, herdr_once(socket, method, params)).await.unwrap_or_else(|_| Err(format!("Herdr did not answer {method}")))
}

#[cfg(unix)]
async fn herdr_once(socket: &Path, method: &str, params: Value) -> Result<Value, String> {
    use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
    let mut stream = tokio::net::UnixStream::connect(socket).await.map_err(|e| format!("Herdr is not running: {e}"))?;
    let request = json!({ "id": format!("lorca:{method}"), "method": method, "params": params });
    stream.write_all(format!("{request}\n").as_bytes()).await.map_err(|e| e.to_string())?;
    let mut line = String::new();
    BufReader::new(stream).read_line(&mut line).await.map_err(|e| e.to_string())?;
    let answer: Value = serde_json::from_str(&line).map_err(|_| "Herdr answered something else".to_string())?;
    match answer.get("error") {
        Some(error) => Err(error["message"].as_str().unwrap_or("Herdr refused").to_string()),
        None => Ok(answer["result"].clone()),
    }
}

#[cfg(not(unix))]
async fn herdr(_socket: &Path, _method: &str, _params: Value) -> Result<Value, String> {
    Err("Herdr runs on macOS and Linux".into())
}

/// One `luvus` command, which answers in JSON.
async fn luvus(program: &Path, args: &[&str]) -> Result<Value, String> {
    let mut command = lorca_agent::login_shell::command(program).await;
    command.args(args).stdin(std::process::Stdio::null());
    let output = command.output().await.map_err(|e| format!("Luvus did not start: {e}"))?;
    let text = String::from_utf8_lossy(&output.stdout);
    let answer: Option<Value> = serde_json::from_str(text.trim()).ok();
    match answer {
        Some(answer) if answer.get("error").is_some() => Err(answer["error"]["message"].as_str().unwrap_or("Luvus refused").to_string()),
        Some(answer) if output.status.success() => Ok(answer.get("result").cloned().unwrap_or(answer)),
        _ if output.status.success() => Ok(Value::Null),
        _ => {
            let said = String::from_utf8_lossy(&output.stderr);
            Err(said.lines().map(str::trim).find(|line| !line.is_empty()).unwrap_or("Luvus failed").to_string())
        }
    }
}

/// The agent a pane runs, as its host sees it.
struct Info {
    kind: String,
    ready: bool,
}

/// An agent's state as its host reports it.
#[derive(Debug, Clone, Copy, PartialEq)]
enum Status {
    Idle,
    Working,
    Blocked,
    Gone,
}

fn status(word: &str) -> Option<Status> {
    match word {
        "idle" | "done" => Some(Status::Idle),
        "working" => Some(Status::Working),
        "blocked" => Some(Status::Blocked),
        _ => None,
    }
}

/// An agent in a pane of a terminal host.
struct InPane {
    host: Host,
    target: Target,
    kind: String,
    /// The first prompt, typed in once the agent is ready for it.
    prompt: Mutex<Option<String>>,
    /// Lorca is closing its pane.
    closing: AtomicBool,
}

impl InPane {
    async fn read(&self, visible: bool) -> Option<String> {
        let text = match &self.host {
            // The pane's, since Herdr names the agent only once it is ready, past any question it
            // starts with.
            Host::Herdr { socket } => {
                let source = if visible { "visible" } else { "recent_unwrapped" };
                let read = herdr(socket, "pane.read", json!({ "pane_id": self.target.pane, "source": source, "lines": SCREEN_LINES, "strip_ansi": true })).await.ok()?;
                read["read"]["text"].as_str().or(read["text"].as_str())?.to_string()
            }
            Host::Luvus { program } => {
                let lines = if visible { "60".to_string() } else { SCREEN_LINES.to_string() };
                let source = if visible { "visible" } else { "recent" };
                let read = luvus(program, &["agent", "read", &self.target.name, "--lines", &lines, "--source", source]).await.ok()?;
                read["text"].as_str()?.to_string()
            }
        };
        Some(lorca_agent::tools::sanitize::terminal_text(text.as_bytes()).trim_end().to_string())
    }

    /// Presses `keys` one at a time: sent together, an arrow's escape sequence can read as Esc.
    async fn keys(&self, keys: &[String]) -> Result<(), String> {
        for (index, key) in keys.iter().enumerate() {
            if index > 0 {
                tokio::time::sleep(Duration::from_millis(150)).await;
            }
            match &self.host {
                Host::Herdr { socket } => herdr(socket, "pane.send_keys", json!({ "pane_id": self.target.pane, "keys": [key] })).await.map(drop)?,
                // Luvus's `enter` is a line feed; `return` is the key that sends.
                Host::Luvus { program } => {
                    let key = if key == "enter" { "return" } else { key.as_str() };
                    luvus(program, &["agent", "keys", self.target.name.as_str(), key]).await.map(drop)?
                }
            }
        }
        Ok(())
    }

    /// Types a message into the agent's input and sends it, on one line. Typed, not pasted:
    /// Claude Code holds back from acting on a message that is nothing but pasted text.
    async fn prompt(&self, text: &str) -> Result<(), String> {
        let line = text.split_whitespace().collect::<Vec<_>>().join(" ");
        match &self.host {
            Host::Herdr { socket } => {
                herdr(socket, "pane.send_text", json!({ "pane_id": self.target.pane, "text": line })).await?;
                tokio::time::sleep(Duration::from_millis(300)).await;
                herdr(socket, "pane.send_keys", json!({ "pane_id": self.target.pane, "keys": ["enter"] })).await.map(drop)
            }
            // `pane run` types the line and a line feed, which an agent's composer may take as a
            // new line: Return sends it.
            Host::Luvus { program } => {
                luvus(program, &["pane", "run", &self.target.pane, &line]).await?;
                tokio::time::sleep(Duration::from_millis(500)).await;
                luvus(program, &["agent", "keys", &self.target.name, "return"]).await.map(drop)
            }
        }
    }

    /// The agent its pane runs now, and whether it is ready for a prompt: started, and not
    /// asking. None when no agent runs there.
    async fn info(&self) -> Option<Info> {
        match &self.host {
            Host::Herdr { socket } => {
                let pane = herdr(socket, "pane.get", json!({ "pane_id": self.target.pane })).await.ok()?;
                let pane = if pane["pane"].is_object() { &pane["pane"] } else { &pane };
                let kind = pane["agent"].as_str()?.to_string();
                let agent = herdr(socket, "agent.get", json!({ "target": self.target.name })).await.ok();
                let ready = agent.is_some_and(|agent| agent["agent"]["interactive_ready"].as_bool().unwrap_or(false) && matches!(agent["agent"]["agent_status"].as_str(), Some("idle" | "done")));
                Some(Info { kind, ready })
            }
            Host::Luvus { program } => {
                let agent = luvus(program, &["agent", "get", &self.target.name]).await.ok()?;
                Some(Info { kind: agent["agent"].as_str()?.to_string(), ready: matches!(agent["status"].as_str(), Some("idle" | "done")) })
            }
        }
    }
}

#[async_trait]
impl Driver for InPane {
    async fn send(&self, text: &str, interrupt: bool) -> Result<(), String> {
        if self.prompt.lock().unwrap().is_some() {
            // Not ready for its first prompt yet: this one goes after it.
            let mut first = self.prompt.lock().unwrap();
            if let Some(prompt) = first.as_mut() {
                prompt.push_str("\n\n");
                prompt.push_str(text);
                return Ok(());
            }
        }
        if interrupt {
            self.keys(&["esc".into()]).await?;
            tokio::time::sleep(Duration::from_millis(1500)).await;
        }
        self.prompt(text).await
    }

    async fn answer(&self, answer: &Answer) -> Result<(), String> {
        match answer {
            Answer::Keys(keys) => self.keys(keys).await,
            Answer::Text(text) => {
                match &self.host {
                    Host::Herdr { socket } => herdr(socket, "pane.send_input", json!({ "pane_id": self.target.pane, "text": text, "keys": ["enter"] })).await.map(drop),
                    Host::Luvus { program } => {
                        luvus(program, &["pane", "send", &self.target.pane, text]).await?;
                        self.keys(&["enter".into()]).await
                    }
                }
            }
        }
    }

    async fn screen(&self) -> Option<String> {
        self.read(false).await
    }

    async fn screen_now(&self) -> Option<String> {
        self.read(true).await
    }

    async fn focus(&self) -> Result<(), String> {
        match &self.host {
            Host::Herdr { socket } => {
                herdr(socket, "workspace.focus", json!({ "workspace_id": self.target.workspace })).await?;
                herdr(socket, "agent.focus", json!({ "target": self.target.name })).await.map(drop)
            }
            Host::Luvus { program } => luvus(program, &["pane", "focus", &self.target.pane]).await.map(drop),
        }
    }

    async fn stop(&self) {
        self.closing.store(true, Ordering::SeqCst);
        let closed = match &self.host {
            Host::Herdr { socket } => herdr(socket, "workspace.close", json!({ "workspace_id": self.target.workspace })).await.map(drop),
            Host::Luvus { program } => luvus(program, &["pane", "close", &self.target.pane]).await.map(drop),
        };
        if let Err(error) = closed {
            tracing::warn!(%error, "closing a coding agent's pane");
        }
    }
}

/// Starts the agent in a new workspace of `host` in its folder, and follows it there.
pub(crate) async fn start(app: &Arc<App>, agent: &Arc<Agent>, host: &Host, program: &Path, prompt: &str, events: Events) -> Result<Arc<dyn Driver>, String> {
    let record = agent.record();
    let label = format!("{} · {}", super::name(&record.kind), record.branch.clone().unwrap_or_else(|| record.folder.file_name().map(|name| name.to_string_lossy().to_string()).unwrap_or_default()));
    let folder = record.folder.to_string_lossy().to_string();
    let args = agent_args(app, &record.kind, &record.id, program);
    let target = match host {
        Host::Herdr { socket } => {
            let created = herdr(socket, "workspace.create", json!({ "cwd": folder, "label": label, "focus": false })).await?;
            let workspace = created["workspace"]["workspace_id"].as_str().ok_or("Herdr made no workspace")?.to_string();
            let pane = created["root_pane"]["pane_id"].as_str().ok_or("Herdr made no pane")?.to_string();
            let target = Target { workspace: workspace.clone(), pane: pane.clone(), name: record.id.clone() };
            // A new pane's shell takes a moment to reach its prompt, before which Herdr finds the
            // pane busy.
            let mut started = Err(String::new());
            for _ in 0..20 {
                started = herdr(socket, "agent.start", json!({ "name": record.id, "kind": record.kind, "pane_id": pane, "args": args, "timeout_ms": START_TIMEOUT.as_millis() as u64 })).await;
                match &started {
                    Err(error) if error.contains("not an available shell") => tokio::time::sleep(Duration::from_millis(500)).await,
                    _ => break,
                }
            }
            if let Err(error) = started {
                let _ = herdr(socket, "workspace.close", json!({ "workspace_id": workspace })).await;
                return Err(format!("Herdr could not start {}: {error}", super::name(&record.kind)));
            }
            target
        }
        Host::Luvus { program: luvus_program } => {
            // Opening a workspace brings it forward; the one the user was in comes back.
            let before = luvus(luvus_program, &["workspace", "list"]).await?;
            let active = before["workspaces"].as_array().into_iter().flatten().position(|workspace| workspace["active"].as_bool().unwrap_or(false));
            luvus(luvus_program, &["workspace", "open", &folder]).await?;
            let after = luvus(luvus_program, &["workspace", "list"]).await?;
            let index = after["workspaces"]
                .as_array()
                .into_iter()
                .flatten()
                .position(|workspace| workspace["cwd"].as_str().map(|cwd| same_folder(cwd, &record.folder)).unwrap_or(false))
                .ok_or("Luvus opened no workspace there")?;
            let workspace = after["workspaces"][index]["display_position"].as_str().map(str::to_string).unwrap_or_else(|| index.to_string());
            if let Some(active) = active.filter(|active| *active != index) {
                let _ = luvus(luvus_program, &["workspace", "focus", &active.to_string()]).await;
            }
            let panes = luvus(luvus_program, &["pane", "list", "--all-tabs"]).await?;
            let pane = panes["panes"]
                .as_array()
                .into_iter()
                .flatten()
                .find(|pane| pane["workspace"].as_str() == Some(workspace.as_str()) && pane["cwd"].as_str().is_some_and(|cwd| same_folder(cwd, &record.folder)))
                .and_then(|pane| pane["pane"].as_str())
                .ok_or("Luvus has no pane there")?
                .to_string();
            let timeout = START_TIMEOUT.as_secs().to_string();
            let mut start = vec!["agent", "start", record.id.as_str(), "--kind", record.kind.as_str(), "--pane", pane.as_str(), "--timeout", timeout.as_str(), "--"];
            start.extend(args.iter().map(String::as_str));
            if let Err(error) = luvus(luvus_program, &start).await {
                let _ = luvus(luvus_program, &["pane", "close", &pane]).await;
                return Err(format!("Luvus could not start {}: {error}", super::name(&record.kind)));
            }
            Target { workspace, pane, name: record.id.clone() }
        }
    };
    agent.record.lock().unwrap().target = Some(target.clone());
    super::save(app);
    let pane = Arc::new(InPane { host: host.clone(), target, kind: record.kind.clone(), prompt: Mutex::new(Some(prompt.to_string())), closing: AtomicBool::new(false) });
    follow(pane.clone(), events);
    Ok(pane)
}

/// Follows an agent Lorca started in a terminal host before it restarted, when its pane is
/// still there.
pub(crate) async fn attach(app: &Arc<App>, agent: &Arc<Agent>) -> Result<(), String> {
    let record = agent.record();
    let target = record.target.clone().ok_or("It ran in no pane")?;
    let host = match record.host.as_deref() {
        Some("herdr") => Host::Herdr { socket: herdr_socket().await },
        Some("luvus") => Host::Luvus { program: super::process::find("luvus").await.ok_or("Luvus is no longer installed")? },
        _ => return Err("It ran in no terminal host".into()),
    };
    let pane = Arc::new(InPane { host, target, kind: record.kind.clone(), prompt: Mutex::new(None), closing: AtomicBool::new(false) });
    if pane.read(true).await.is_none() {
        return Err("Its pane closed while Lorca was not running".into());
    }
    let (tx, rx) = mpsc::unbounded_channel();
    let generation = agent.generation.fetch_add(1, std::sync::atomic::Ordering::SeqCst) + 1;
    tokio::spawn(super::supervise(app.clone(), agent.clone(), rx, generation));
    *agent.driver.lock().unwrap() = Some(pane.clone());
    follow(pane, tx);
    Ok(())
}

fn same_folder(path: &str, folder: &Path) -> bool {
    let path = std::fs::canonicalize(path).unwrap_or_else(|_| PathBuf::from(path));
    path == std::fs::canonicalize(folder).unwrap_or_else(|_| folder.to_path_buf())
}

/// What the agent starts with in a pane: Claude Code's Bash commands pass Lorca's review through
/// a hook; Codex runs commands in its sandbox and asks before anything past it.
fn agent_args(app: &App, kind: &str, id: &str, _program: &Path) -> Vec<String> {
    if kind == "codex" {
        return ["--ask-for-approval", "on-request", "--sandbox", "workspace-write", "-c", "check_for_update_on_startup=false"].iter().map(|arg| arg.to_string()).collect();
    }
    let lorca = std::env::current_exe().unwrap_or_else(|_| PathBuf::from("lorca"));
    let quote = |text: &str| format!("'{}'", text.replace('\'', r"'\''"));
    let command = format!(
        "LORCA_HOME={} LORCA_PORT={} {} coding hook {}",
        quote(&app.config.home.to_string_lossy()),
        app.config.port,
        quote(&lorca.to_string_lossy()),
        quote(id)
    );
    let settings = json!({ "hooks": { "PreToolUse": [{ "matcher": "Bash", "hooks": [{ "type": "command", "command": command, "timeout": HOOK_TIMEOUT_SECS }] }] } });
    vec!["--settings".into(), settings.to_string()]
}

/// Turns the host's reports on the agent into its events. The host's states can lag behind the
/// agent itself, so the agent is also looked for every few seconds: one that has exited (a
/// question it was answered no to, a crash, the user quitting it in the pane) ends.
fn follow(pane: Arc<InPane>, events: Events) {
    let (statuses_tx, mut statuses) = mpsc::unbounded_channel();
    watch(&pane, statuses_tx);
    tokio::spawn(async move {
        let mut worked = false;
        let mut blocked = false;
        let mut seen = false;
        let started = tokio::time::Instant::now();
        let mut next_look = started;
        loop {
            let status = match tokio::time::timeout(Duration::from_secs(1), statuses.recv()).await {
                Ok(Some(status)) => Some(status),
                Ok(None) => Some(Status::Gone),
                Err(_) => None,
            };
            if status == Some(Status::Gone) {
                let outcome = if pane.closing.load(Ordering::SeqCst) { "Stopped".to_string() } else { format!("Its {} pane closed", pane.host.title()) };
                let _ = events.send(Event::Ended(Ended { outcome, failed: false }));
                break;
            }
            // What it asked is answered, here or in the pane.
            if blocked && matches!(status, Some(Status::Working | Status::Idle)) {
                blocked = false;
                let _ = events.send(Event::Unblocked);
            }
            let waiting = pane.prompt.lock().unwrap().is_some();
            // Is the agent still there, and ready for its first prompt?
            let mut ready = false;
            if waiting || tokio::time::Instant::now() >= next_look {
                next_look = tokio::time::Instant::now() + Duration::from_secs(5);
                match pane.info().await {
                    Some(info) if info.kind == pane.kind => {
                        seen = true;
                        ready = info.ready;
                    }
                    _ if pane.closing.load(Ordering::SeqCst) => continue,
                    _ if seen || started.elapsed() > START_TIMEOUT => {
                        let outcome = if seen { format!("{} exited in its pane", super::name(&pane.kind)) } else { format!("{} did not start in its pane", super::name(&pane.kind)) };
                        let _ = events.send(Event::Ended(Ended { outcome, failed: !seen }));
                        break;
                    }
                    _ => {}
                }
            }
            if status == Some(Status::Blocked) && !blocked {
                blocked = true;
                let screen = pane.read(true).await.unwrap_or_default();
                let _ = events.send(Event::Blocked(screen));
                continue;
            }
            // Before its first prompt, what it does is getting ready, which may take a question
            // (a folder to trust, an update notice); work is not yet the bot's. A menu on its
            // screen asks even while the host reports it idle, and no prompt is typed over one.
            if waiting {
                let screen = pane.read(true).await.unwrap_or_default();
                let menu = super::screen::asks(&screen);
                if menu && !blocked {
                    blocked = true;
                    let _ = events.send(Event::Blocked(screen));
                    continue;
                }
                if !menu && blocked && status != Some(Status::Blocked) {
                    blocked = false;
                    let _ = events.send(Event::Unblocked);
                }
                if ready && !blocked {
                    tokio::time::sleep(Duration::from_millis(800)).await;
                    let prompt = pane.prompt.lock().unwrap().take();
                    if let Some(prompt) = prompt {
                        if let Err(error) = pane.prompt(&prompt).await {
                            let _ = events.send(Event::Ended(Ended { outcome: format!("Could not give it its prompt: {error}"), failed: true }));
                            break;
                        }
                        let _ = events.send(Event::Lines(super::lines::sent(&prompt)));
                    }
                }
                continue;
            }
            match status {
                Some(Status::Working) if !worked => {
                    worked = true;
                    let _ = events.send(Event::Working);
                }
                Some(Status::Idle) if worked => {
                    worked = false;
                    // A queued message may start more work at once: settle first.
                    tokio::time::sleep(Duration::from_millis(1500)).await;
                    if let Ok(Status::Working) = statuses.try_recv() {
                        worked = true;
                        continue;
                    }
                    let screen = pane.read(false).await;
                    if let Some(screen) = &screen {
                        let _ = events.send(Event::Screen(screen.clone()));
                    }
                    let said = screen.map(|screen| {
                        let body = super::screen::body(&screen);
                        let lines: Vec<&str> = body.lines().collect();
                        lines[lines.len().saturating_sub(30)..].join("\n")
                    });
                    let _ = events.send(Event::Idle(said));
                }
                _ => {}
            }
        }
    });
}

/// Subscribes to the host's events for the agent's pane, and reports its states on `tx`.
fn watch(pane: &Arc<InPane>, tx: mpsc::UnboundedSender<Status>) {
    let pane = pane.clone();
    tokio::spawn(async move {
        match &pane.host {
            Host::Herdr { socket } => herdr_events(socket, &pane.target, &tx).await,
            Host::Luvus { program } => luvus_events(program, &pane.target, &tx).await,
        }
        let _ = tx.send(Status::Gone);
    });
}

/// Herdr's events for one pane, until it closes. A dropped connection is made again while the
/// pane is there.
#[cfg(unix)]
async fn herdr_events(socket: &Path, target: &Target, tx: &mpsc::UnboundedSender<Status>) {
    use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
    loop {
        let Ok(mut stream) = tokio::net::UnixStream::connect(socket).await else { return };
        let subscribe = json!({
            "id": "lorca:events",
            "method": "events.subscribe",
            "params": { "subscriptions": [{ "type": "pane.agent_status_changed", "pane_id": target.pane }, { "type": "pane.exited" }, { "type": "pane.closed" }] },
        });
        if stream.write_all(format!("{subscribe}\n").as_bytes()).await.is_err() {
            return;
        }
        // Where it stands now, which no event may say.
        if let Ok(info) = herdr(socket, "agent.get", json!({ "target": target.name })).await {
            if let Some(status) = info["agent"]["agent_status"].as_str().and_then(status) {
                let _ = tx.send(status);
            }
        }
        let mut lines = BufReader::new(stream).lines();
        while let Ok(Some(line)) = lines.next_line().await {
            let Ok(message) = serde_json::from_str::<Value>(&line) else { continue };
            let data = &message["data"];
            let pane = data["pane_id"].as_str().unwrap_or("");
            if pane != target.pane {
                continue;
            }
            match message["event"].as_str().unwrap_or("") {
                "pane.agent_status_changed" | "pane_agent_status_changed" => {
                    if let Some(status) = data["agent_status"].as_str().and_then(status) {
                        if tx.send(status).is_err() {
                            return;
                        }
                    }
                }
                "pane.exited" | "pane_exited" | "pane.closed" | "pane_closed" => return,
                _ => {}
            }
        }
        if tx.is_closed() || herdr(socket, "pane.get", json!({ "pane_id": target.pane })).await.is_err() {
            return;
        }
        tokio::time::sleep(Duration::from_secs(2)).await;
    }
}

#[cfg(not(unix))]
async fn herdr_events(_socket: &Path, _target: &Target, _tx: &mpsc::UnboundedSender<Status>) {}

/// Luvus's event stream (`luvus events`), for one pane, until it closes.
async fn luvus_events(program: &Path, target: &Target, tx: &mpsc::UnboundedSender<Status>) {
    use tokio::io::{AsyncBufReadExt, BufReader};
    loop {
        let mut command = lorca_agent::login_shell::command(program).await;
        command.arg("events").stdin(std::process::Stdio::null()).stdout(std::process::Stdio::piped()).stderr(std::process::Stdio::null()).kill_on_drop(true);
        let Ok(mut child) = command.spawn() else { return };
        let Some(stdout) = child.stdout.take() else { return };
        if let Ok(info) = luvus(program, &["agent", "get", &target.name]).await {
            if let Some(status) = info["status"].as_str().and_then(status) {
                let _ = tx.send(status);
            }
        }
        let mut lines = BufReader::new(stdout).lines();
        while let Ok(Some(line)) = lines.next_line().await {
            let Ok(message) = serde_json::from_str::<Value>(&line) else { continue };
            let data = &message["data"];
            let pane = data["pane"].as_str().or(data["pane_id"].as_str()).unwrap_or("");
            if pane != target.pane {
                continue;
            }
            match message["event"].as_str().unwrap_or("") {
                "pane.agent_status_changed" => {
                    if let Some(status) = data["status"].as_str().and_then(status) {
                        if tx.send(status).is_err() {
                            return;
                        }
                    }
                }
                "pane.closed" | "pane.exited" => return,
                _ => {}
            }
        }
        if tx.is_closed() || luvus(program, &["agent", "get", &target.name]).await.is_err() {
            return;
        }
        tokio::time::sleep(Duration::from_secs(2)).await;
    }
}

/// The Bash command a `PreToolUse` hook's input (`lorca coding hook <agent>`, Claude Code's hook
/// in a pane) asks about; None for any other tool, which the hook lets through.
pub fn hook_command(input: &str) -> Option<String> {
    let input: Value = serde_json::from_str(input).ok()?;
    (input["tool_name"] == "Bash").then(|| input["tool_input"]["command"].as_str().unwrap_or("").to_string()).filter(|command| !command.is_empty())
}

/// What the hook prints for Claude Code: Lorca's decision.
pub fn hook_answer(verdict: Result<(), String>) -> String {
    let (decision, reason) = match verdict {
        Ok(()) => ("allow", "Lorca allowed it.".to_string()),
        Err(reason) => ("deny", reason),
    };
    json!({ "hookSpecificOutput": { "hookEventName": "PreToolUse", "permissionDecision": decision, "permissionDecisionReason": reason } }).to_string()
}

#[cfg(test)]
pub(crate) mod tests_support {
    use std::sync::Mutex;

    static HOST: Mutex<Option<super::Host>> = Mutex::new(None);

    pub fn host() -> Option<super::Host> {
        HOST.lock().unwrap().clone()
    }

    #[allow(dead_code)]
    pub fn set(host: Option<super::Host>) {
        *HOST.lock().unwrap() = host;
    }
}
