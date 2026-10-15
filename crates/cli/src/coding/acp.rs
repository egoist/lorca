//! An agent that speaks the Agent Client Protocol (agentclientprotocol.com) over stdio, as
//! OpenCode does (`opencode acp`): JSON-RPC, a session in the folder (`session/new`, or
//! `session/load` to resume one), and a prompt per message (`session/prompt`), which answers
//! once the turn is over. What it does arrives as `session/update` notifications; what needs a
//! decision comes as `session/request_permission`, which Lorca answers allow once or reject.
//! The protocol has no steering: a message sent while it works waits for the turn to end, and
//! one that interrupts cancels the turn first (`session/cancel`). Lorca offers it no file or
//! terminal methods, so it works with its own tools.

use std::collections::{HashMap, HashSet, VecDeque};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use async_trait::async_trait;
use serde_json::{json, Value};
use tokio::sync::oneshot;
use tokio::time::Instant;

use super::driver::{Approval, Driver, Event, Events};
use super::process::{self, Process};
use super::transcript::{self, Item, PlanEntry, ToolStep};

/// How long the agent has to answer a request other than a prompt.
const REQUEST_TIMEOUT: Duration = Duration::from_secs(120);

type Pending = Mutex<HashMap<u64, oneshot::Sender<Result<Value, String>>>>;

/// What starts one agent that speaks ACP.
pub(crate) struct Launch<'a> {
    /// Its name in what Lorca writes: "OpenCode".
    pub name: &'static str,
    pub program: &'a Path,
    pub args: Vec<String>,
    pub folder: &'a Path,
    /// Its environment beyond the login shell's: the bot's secrets, and its own settings.
    pub variables: Vec<process::Variable>,
}

pub(crate) struct Acp(Arc<Shared>);

struct Shared {
    name: &'static str,
    process: Arc<Process>,
    pending: Arc<Pending>,
    next_id: AtomicU64,
    session: String,
    events: Events,
    /// Messages waiting for the turn under way to end, and whether one is under way.
    queue: Mutex<(VecDeque<String>, bool)>,
    state: Arc<Mutex<Reader>>,
}

/// What the reader keeps between updates.
#[derive(Default)]
struct Reader {
    /// What it is saying or thinking, as it streams in, until another kind of update comes.
    text: Option<(String, String, String, Instant)>,
    /// Steps under way, by tool call id, as their updates fill them in.
    steps: HashMap<String, ToolStep>,
    /// Calls Lorca said no to.
    denied: HashSet<String>,
    last_said: Option<String>,
    /// While `session/load` replays an old session, its updates are not news.
    loading: bool,
    /// When it last showed something, for how long it thought.
    since: Option<Instant>,
}

/// Starts the agent in its folder on `prompt`, or resumes `session` with it.
pub(crate) async fn start(launch: Launch<'_>, prompt: &str, session: Option<&str>, events: Events) -> Result<Arc<dyn Driver>, String> {
    let name = launch.name;
    let (process, stdout, exit) = process::spawn(launch.program, &launch.args, launch.folder, &launch.variables).await?;
    let pending: Arc<Pending> = Arc::new(Mutex::new(HashMap::new()));
    let state = Arc::new(Mutex::new(Reader::default()));
    tokio::spawn(read(name, process.clone(), process::json_lines(stdout), exit, events.clone(), pending.clone(), state.clone(), launch.folder.to_path_buf()));
    let mut shared = Shared { name, process, pending, next_id: AtomicU64::new(1), session: String::new(), events, queue: Mutex::new((VecDeque::new(), false)), state };
    let initialized = shared
        .request(
            "initialize",
            json!({
                "protocolVersion": 1,
                "clientCapabilities": { "fs": { "readTextFile": false, "writeTextFile": false }, "terminal": false },
                "clientInfo": { "name": "lorca", "title": "Lorca", "version": env!("CARGO_PKG_VERSION") },
            }),
        )
        .await?;
    let folder = launch.folder.to_string_lossy().to_string();
    let can_load = initialized["agentCapabilities"]["loadSession"].as_bool().unwrap_or(false);
    shared.session = match session.filter(|_| can_load) {
        Some(session) => {
            shared.state.lock().unwrap().loading = true;
            let loaded = shared.request("session/load", json!({ "sessionId": session, "cwd": folder, "mcpServers": [] })).await;
            shared.state.lock().unwrap().loading = false;
            loaded.map_err(|error| format!("{name} could not go on with its session: {error}"))?;
            session.to_string()
        }
        None => {
            let created = shared.request("session/new", json!({ "cwd": folder, "mcpServers": [] })).await.map_err(|error| sign_in_hint(name, error))?;
            created["sessionId"].as_str().ok_or(format!("{name} started no session"))?.to_string()
        }
    };
    let _ = shared.events.send(Event::Session(shared.session.clone()));
    let acp = Acp(Arc::new(shared));
    acp.send(prompt, false).await?;
    Ok(Arc::new(acp))
}

/// What a refused `session/new` means: usually that the agent is not signed in.
fn sign_in_hint(name: &str, error: String) -> String {
    if error.to_ascii_lowercase().contains("auth") {
        format!("{name} needs signing in on this Runner: {error}")
    } else {
        format!("{name} did not start a session: {error}")
    }
}

impl Shared {
    async fn request(&self, method: &str, params: Value) -> Result<Value, String> {
        request(&self.process, &self.pending, &self.next_id, method, params, Some(REQUEST_TIMEOUT), self.name).await
    }
}

/// Follows turns to their ends, starting with `text`'s: then one for what waits, until nothing
/// does and it is done.
async fn prompts(shared: Arc<Shared>, mut text: String) {
    loop {
        let _ = shared.events.send(Event::Item(Item::text(format!("prompt-{}", uuid::Uuid::new_v4()), "prompt", &text)));
        let _ = shared.events.send(Event::Working);
        shared.state.lock().unwrap().since = Some(Instant::now());
        let params = json!({ "sessionId": shared.session, "prompt": [{ "type": "text", "text": text }] });
        let answer = request(&shared.process, &shared.pending, &shared.next_id, "session/prompt", params, None, shared.name).await;
        let said = {
            let mut state = shared.state.lock().unwrap();
            flush(&mut state, &shared.events);
            // Steps the turn never finished end with it.
            for (id, mut step) in state.steps.drain() {
                step.status = "failed".into();
                let _ = shared.events.send(Event::Item(Item::tool(id, step)));
            }
            state.last_said.take()
        };
        let failed = answer.as_ref().err().cloned();
        if let Some(error) = &failed {
            let _ = shared.events.send(Event::Item(Item::text(uuid::Uuid::new_v4().to_string(), "error", error)));
        }
        if answer.as_ref().is_ok_and(|answer| answer["stopReason"] == "cancelled") {
            let _ = shared.events.send(Event::Item(Item::text(uuid::Uuid::new_v4().to_string(), "status", "Interrupted")));
        }
        let next = {
            let mut queue = shared.queue.lock().unwrap();
            let next = queue.0.pop_front();
            queue.1 = next.is_some();
            next
        };
        match next {
            Some(next) => text = next,
            None => {
                let _ = shared.events.send(Event::Idle(failed.or(said)));
                return;
            }
        }
    }
}

#[async_trait]
impl Driver for Acp {
    async fn send(&self, text: &str, interrupt: bool) -> Result<(), String> {
        let shared = &self.0;
        let under_way = {
            let mut queue = shared.queue.lock().unwrap();
            if queue.1 {
                queue.0.push_back(text.to_string());
            }
            std::mem::replace(&mut queue.1, true)
        };
        if !under_way {
            tokio::spawn(prompts(shared.clone(), text.to_string()));
            return Ok(());
        }
        // The turn under way ends cancelled, and the message waiting starts the next.
        if interrupt {
            shared.process.send(&json!({ "jsonrpc": "2.0", "method": "session/cancel", "params": { "sessionId": shared.session } })).await?;
        }
        Ok(())
    }

    async fn stop(&self) {
        self.0.process.stop();
    }
}

async fn request(process: &Process, pending: &Pending, next_id: &AtomicU64, method: &str, params: Value, timeout: Option<Duration>, name: &str) -> Result<Value, String> {
    let id = next_id.fetch_add(1, Ordering::SeqCst);
    let (tx, rx) = oneshot::channel();
    pending.lock().unwrap().insert(id, tx);
    if let Err(error) = process.send(&json!({ "jsonrpc": "2.0", "id": id, "method": method, "params": params })).await {
        pending.lock().unwrap().remove(&id);
        return Err(error);
    }
    let answer = match timeout {
        Some(timeout) => match tokio::time::timeout(timeout, rx).await {
            Ok(answer) => answer,
            Err(_) => {
                pending.lock().unwrap().remove(&id);
                return Err(format!("{name} did not answer {method}"));
            }
        },
        None => rx.await,
    };
    answer.unwrap_or_else(|_| Err(format!("{name} is no longer running")))
}

/// Writes what it was saying or thinking as a step, once something else comes.
fn flush(state: &mut Reader, events: &Events) {
    let Some((id, kind, text, began)) = state.text.take() else { return };
    if text.trim().is_empty() {
        return;
    }
    let mut item = Item::text(id, &kind, &text);
    if kind == "thought" {
        item.seconds = Some(began.elapsed().as_secs_f64().round());
    } else {
        state.last_said = Some(text.trim().to_string());
    }
    let _ = events.send(Event::Item(item));
    state.since = Some(Instant::now());
}

/// A tool call, or an update to one, as a step: the fields it carries fill in what it had.
fn merge(step: &mut ToolStep, update: &Value, folder: &Path) {
    if let Some(kind) = update["kind"].as_str() {
        step.kind = match kind {
            "read" => "read",
            "search" => "search",
            "edit" | "delete" | "move" => "edit",
            "execute" => "execute",
            "fetch" => "fetch",
            _ => "other",
        }
        .into();
    }
    let location = update["locations"][0]["path"].as_str().map(|path| transcript::path(path, folder));
    if location.is_some() && matches!(step.kind.as_str(), "read" | "edit") {
        step.path = location.clone();
    }
    let input = &update["rawInput"];
    let title = match step.kind.as_str() {
        "execute" => input["command"].as_str().map(str::to_string),
        "read" | "edit" => location.or_else(|| input["path"].as_str().or(input["filePath"].as_str()).map(|path| transcript::path(path, folder))),
        "search" => input["pattern"].as_str().or(input["query"].as_str()).map(str::to_string),
        "fetch" => input["url"].as_str().or(input["query"].as_str()).map(str::to_string),
        _ => None,
    }
    .or_else(|| update["title"].as_str().map(str::to_string));
    if let Some(title) = title.filter(|title| !title.is_empty()) {
        step.title = transcript::title(&title);
    }
    if let Some(status) = update["status"].as_str() {
        step.status = match status {
            "completed" => "done",
            "failed" => "failed",
            _ => "running",
        }
        .into();
    }
    let mut text = Vec::new();
    for part in update["content"].as_array().into_iter().flatten() {
        match part["type"].as_str() {
            Some("diff") => {
                step.added = Some(transcript::line_count(part["newText"].as_str().unwrap_or("")));
                step.removed = Some(transcript::line_count(part["oldText"].as_str().unwrap_or("")));
            }
            Some("content") => text.extend(part["content"]["text"].as_str().map(str::to_string)),
            _ => {}
        }
    }
    if !text.is_empty() {
        step.output = transcript::output(&text.join("\n"));
    }
}

#[allow(clippy::too_many_arguments)]
async fn read(
    name: &'static str,
    process: Arc<Process>,
    mut messages: tokio::sync::mpsc::UnboundedReceiver<Value>,
    exit: oneshot::Receiver<process::Exit>,
    events: Events,
    pending: Arc<Pending>,
    state: Arc<Mutex<Reader>>,
    folder: PathBuf,
) {
    while let Some(message) = messages.recv().await {
        let method = message["method"].as_str();
        let id = message.get("id").filter(|id| !id.is_null()).cloned();
        match (method, id) {
            // An answer to one of Lorca's requests.
            (None, Some(id)) => {
                let Some(id) = id.as_u64() else { continue };
                let Some(tx) = pending.lock().unwrap().remove(&id) else { continue };
                let answer = match message.get("error") {
                    Some(error) => Err(error["message"].as_str().unwrap_or("refused").to_string()),
                    None => Ok(message["result"].clone()),
                };
                let _ = tx.send(answer);
            }
            (Some("session/request_permission"), Some(id)) => {
                let params = &message["params"];
                let call = &params["toolCall"];
                let call_id = call["toolCallId"].as_str().unwrap_or("").to_string();
                let input = &call["rawInput"];
                let approval = match call["kind"].as_str() {
                    Some("execute") => Approval::Command { command: input["command"].as_str().or(call["title"].as_str()).unwrap_or("").to_string() },
                    Some("edit" | "delete" | "move") => {
                        let mut paths: Vec<String> = call["locations"].as_array().into_iter().flatten().filter_map(|location| location["path"].as_str().map(str::to_string)).collect();
                        if paths.is_empty() {
                            paths.extend(input["path"].as_str().or(input["filePath"].as_str()).map(str::to_string));
                        }
                        Approval::Files { paths }
                    }
                    _ => Approval::Tool { name: call["title"].as_str().unwrap_or("tool").to_string(), input: input.clone() },
                };
                let option = |kind: &str| params["options"].as_array().into_iter().flatten().find(|option| option["kind"] == kind).and_then(|option| option["optionId"].as_str()).map(str::to_string);
                let (allow, reject) = (option("allow_once"), option("reject_once"));
                let (tx, rx) = oneshot::channel();
                if events.send(Event::Approve(approval, tx)).is_err() {
                    continue;
                }
                let (process, state) = (process.clone(), state.clone());
                tokio::spawn(async move {
                    let verdict = rx.await.unwrap_or_else(|_| Err("Lorca stopped before deciding".into()));
                    let outcome = match (verdict, allow, reject) {
                        (Ok(()), Some(allow), _) => json!({ "outcome": "selected", "optionId": allow }),
                        (Err(_), _, Some(reject)) => {
                            state.lock().unwrap().denied.insert(call_id);
                            json!({ "outcome": "selected", "optionId": reject })
                        }
                        _ => {
                            state.lock().unwrap().denied.insert(call_id);
                            json!({ "outcome": "cancelled" })
                        }
                    };
                    let _ = process.send(&json!({ "jsonrpc": "2.0", "id": id, "result": { "outcome": outcome } })).await;
                });
            }
            // Lorca offers no other methods: files and terminals are the agent's own.
            (Some(_), Some(id)) => {
                let _ = process.send(&json!({ "jsonrpc": "2.0", "id": id, "error": { "code": -32601, "message": "Lorca does not offer this" } })).await;
            }
            (Some("session/update"), None) => {
                let update = &message["params"]["update"];
                let mut state = state.lock().unwrap();
                if state.loading {
                    continue;
                }
                match update["sessionUpdate"].as_str().unwrap_or("") {
                    kind @ ("agent_message_chunk" | "agent_thought_chunk") => {
                        let kind = if kind == "agent_message_chunk" { "text" } else { "thought" };
                        let chunk = update["content"]["text"].as_str().unwrap_or("");
                        let message_id = update["messageId"].as_str().map(str::to_string);
                        let same = state.text.as_ref().is_some_and(|(id, current, _, _)| current == kind && message_id.as_ref().is_none_or(|message_id| id == message_id));
                        if !same {
                            flush(&mut state, &events);
                            let began = state.since.unwrap_or_else(Instant::now);
                            state.text = Some((message_id.unwrap_or_else(|| uuid::Uuid::new_v4().to_string()), kind.into(), String::new(), began));
                        }
                        if let Some((_, _, text, _)) = state.text.as_mut() {
                            text.push_str(chunk);
                        }
                    }
                    "tool_call" | "tool_call_update" => {
                        flush(&mut state, &events);
                        let call = update["toolCallId"].as_str().unwrap_or("").to_string();
                        let mut step = state.steps.get(&call).cloned().unwrap_or_else(|| ToolStep { kind: "other".into(), status: "running".into(), ..ToolStep::default() });
                        merge(&mut step, update, &folder);
                        if step.status != "running" {
                            if state.denied.remove(&call) {
                                step.status = "denied".into();
                            }
                            state.steps.remove(&call);
                        } else {
                            state.steps.insert(call.clone(), step.clone());
                        }
                        let _ = events.send(Event::Item(Item::tool(call, step)));
                        state.since = Some(Instant::now());
                    }
                    "plan" => {
                        flush(&mut state, &events);
                        let plan = update["entries"]
                            .as_array()
                            .into_iter()
                            .flatten()
                            .map(|entry| PlanEntry { text: entry["content"].as_str().unwrap_or("").to_string(), status: entry["status"].as_str().unwrap_or("pending").to_string() })
                            .collect();
                        let _ = events.send(Event::Item(Item::plan("plan", plan)));
                    }
                    _ => {}
                }
            }
            _ => {}
        }
    }
    // Whatever Lorca still waits on will not be answered.
    pending.lock().unwrap().clear();
    let _ = events.send(Event::Ended(super::driver::ended(name, exit.await.ok())));
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_tool_calls_updates_fill_in_its_step() {
        let folder = Path::new("/work/shop");
        let mut step = ToolStep { kind: "other".into(), status: "running".into(), ..ToolStep::default() };
        merge(&mut step, &json!({ "toolCallId": "c", "title": "shell", "kind": "execute", "status": "pending", "rawInput": {} }), folder);
        assert_eq!((step.kind.as_str(), step.title.as_str()), ("execute", "shell"));
        merge(&mut step, &json!({ "status": "in_progress", "rawInput": { "command": "date -u" }, "title": "date -u" }), folder);
        merge(&mut step, &json!({ "status": "completed", "content": [{ "type": "content", "content": { "type": "text", "text": "Thu Oct 15\n" } }] }), folder);
        assert_eq!((step.title.as_str(), step.status.as_str(), step.output.as_deref()), ("date -u", "done", Some("Thu Oct 15")));
        let mut edit = ToolStep { kind: "other".into(), status: "running".into(), ..ToolStep::default() };
        merge(&mut edit, &json!({ "kind": "edit", "locations": [{ "path": "/work/shop/b.txt" }], "content": [{ "type": "diff", "path": "/work/shop/b.txt", "oldText": null, "newText": "hi\nthere" }] }), folder);
        assert_eq!((edit.title.as_str(), edit.path.as_deref(), edit.added, edit.removed), ("b.txt", Some("b.txt"), Some(2), Some(0)));
    }
}
