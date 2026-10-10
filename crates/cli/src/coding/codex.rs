//! Codex without a terminal: `codex app-server`, the JSON-RPC protocol its own apps speak, over
//! stdio. A thread in the folder, one turn per message: a message sent while a turn runs steers
//! it (`turn/steer`), and one that interrupts stops it first (`turn/interrupt`). Commands run in
//! Codex's sandbox, which keeps writes to the folder and the network off (`workspace-write`);
//! what needs more, such as a push, asks (`on-request`), and Lorca decides.

use std::collections::HashMap;
use std::path::Path;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use async_trait::async_trait;
use serde_json::{json, Value};
use tokio::sync::oneshot;

use super::driver::{Approval, Driver, Ended, Event, Events};
use super::lines;
use super::process::{self, Process};

/// How long Codex has to answer a request.
const REQUEST_TIMEOUT: Duration = Duration::from_secs(60);

type Pending = Mutex<HashMap<u64, oneshot::Sender<Result<Value, String>>>>;

pub(crate) struct Codex {
    process: Arc<Process>,
    pending: Arc<Pending>,
    next_id: AtomicU64,
    thread: String,
    /// The turn it works on, while it does.
    turn: Arc<Mutex<Option<String>>>,
    /// A message is cutting in: the turn it interrupts ending is not the end of its work.
    cutting_in: Arc<AtomicBool>,
}

/// Starts Codex in `folder` on `prompt`, or resumes `thread` with it.
pub(crate) async fn start(program: &Path, folder: &Path, prompt: &str, thread: Option<&str>, variables: &[process::Variable], events: Events) -> Result<Arc<dyn Driver>, String> {
    let (process, stdout, exit) = process::spawn(program, &["app-server".to_string()], folder, variables).await?;
    let pending: Arc<Pending> = Arc::new(Mutex::new(HashMap::new()));
    let turn = Arc::new(Mutex::new(None));
    let cutting_in = Arc::new(AtomicBool::new(false));
    tokio::spawn(read(process.clone(), process::json_lines(stdout), exit, events, pending.clone(), turn.clone(), cutting_in.clone(), folder.to_path_buf()));
    let mut codex = Codex { process, pending, next_id: AtomicU64::new(1), thread: String::new(), turn, cutting_in };
    codex.request("initialize", json!({ "clientInfo": { "name": "lorca", "title": "Lorca", "version": env!("CARGO_PKG_VERSION") } })).await?;
    codex.process.send(&json!({ "jsonrpc": "2.0", "method": "initialized" })).await?;
    let folder = folder.to_string_lossy().to_string();
    let started = match thread {
        Some(thread) => codex.request("thread/resume", json!({ "threadId": thread, "cwd": folder, "approvalPolicy": "on-request", "sandbox": "workspace-write", "excludeTurns": true })).await?,
        None => codex.request("thread/start", json!({ "cwd": folder, "approvalPolicy": "on-request", "sandbox": "workspace-write" })).await?,
    };
    codex.thread = started["thread"]["id"].as_str().ok_or("Codex started no thread")?.to_string();
    let codex = Arc::new(codex);
    codex.start_turn(prompt).await?;
    Ok(codex)
}

impl Codex {
    async fn request(&self, method: &str, params: Value) -> Result<Value, String> {
        let id = self.next_id.fetch_add(1, Ordering::SeqCst);
        let (tx, rx) = oneshot::channel();
        self.pending.lock().unwrap().insert(id, tx);
        if let Err(error) = self.process.send(&json!({ "jsonrpc": "2.0", "id": id, "method": method, "params": params })).await {
            self.pending.lock().unwrap().remove(&id);
            return Err(error);
        }
        match tokio::time::timeout(REQUEST_TIMEOUT, rx).await {
            Ok(Ok(answer)) => answer,
            Ok(Err(_)) => Err("Codex is no longer running".into()),
            Err(_) => {
                self.pending.lock().unwrap().remove(&id);
                Err(format!("Codex did not answer {method}"))
            }
        }
    }

    async fn start_turn(&self, text: &str) -> Result<(), String> {
        let started = self.request("turn/start", json!({ "threadId": self.thread, "input": [{ "type": "text", "text": text }] })).await?;
        if let Some(id) = started["turn"]["id"].as_str() {
            *self.turn.lock().unwrap() = Some(id.to_string());
        }
        Ok(())
    }

    /// Waits a little for the turn it works on to end.
    async fn turn_ended(&self) -> bool {
        for _ in 0..100 {
            if self.turn.lock().unwrap().is_none() {
                return true;
            }
            tokio::time::sleep(Duration::from_millis(100)).await;
        }
        false
    }
}

#[async_trait]
impl Driver for Codex {
    async fn send(&self, text: &str, interrupt: bool) -> Result<(), String> {
        let turn = self.turn.lock().unwrap().clone();
        match turn {
            Some(turn) if interrupt => {
                self.cutting_in.store(true, Ordering::SeqCst);
                let interrupted = self.request("turn/interrupt", json!({ "threadId": self.thread, "turnId": turn })).await;
                let ended = interrupted.is_ok() && self.turn_ended().await;
                let started = if ended { self.start_turn(text).await } else { Err("Codex did not stop what it was doing".into()) };
                self.cutting_in.store(false, Ordering::SeqCst);
                started
            }
            Some(turn) => match self.request("turn/steer", json!({ "threadId": self.thread, "expectedTurnId": turn, "input": [{ "type": "text", "text": text }] })).await {
                Ok(_) => Ok(()),
                // The turn ended meanwhile: the message starts the next one.
                Err(_) => {
                    self.turn_ended().await;
                    self.start_turn(text).await
                }
            },
            None => self.start_turn(text).await,
        }
    }

    async fn stop(&self) {
        self.process.stop();
    }
}

async fn read(
    process: Arc<Process>,
    mut messages: tokio::sync::mpsc::UnboundedReceiver<Value>,
    exit: oneshot::Receiver<process::Exit>,
    events: Events,
    pending: Arc<Pending>,
    turn: Arc<Mutex<Option<String>>>,
    cutting_in: Arc<AtomicBool>,
    folder: std::path::PathBuf,
) {
    let mut last_said: Option<String> = None;
    // The files each change it is making touches, for its approval.
    let mut changes: HashMap<String, Vec<String>> = HashMap::new();
    let mut session_told = false;
    while let Some(message) = messages.recv().await {
        let method = message["method"].as_str();
        let id = message.get("id").filter(|id| !id.is_null()).cloned();
        match (method, id) {
            // An answer to one of Lorca's requests.
            (None, Some(id)) => {
                let Some(id) = id.as_u64() else { continue };
                let Some(tx) = pending.lock().unwrap().remove(&id) else { continue };
                let answer = match message.get("error") {
                    Some(error) => Err(error["message"].as_str().unwrap_or("Codex refused").to_string()),
                    None => Ok(message["result"].clone()),
                };
                if let (false, Ok(result)) = (session_told, &answer) {
                    if let Some(thread) = result["thread"]["id"].as_str() {
                        session_told = true;
                        let _ = events.send(Event::Session(thread.to_string()));
                    }
                }
                let _ = tx.send(answer);
            }
            // Codex asks Lorca.
            (Some(method), Some(id)) => {
                let params = &message["params"];
                let approval = match method {
                    "item/commandExecution/requestApproval" => {
                        let command = params["commandActions"][0]["command"].as_str().or(params["command"].as_str()).unwrap_or("").to_string();
                        Some(Approval::Command { command })
                    }
                    "item/fileChange/requestApproval" => {
                        let item = params["itemId"].as_str().unwrap_or("");
                        Some(Approval::Files { paths: changes.get(item).cloned().unwrap_or_default() })
                    }
                    _ => None,
                };
                let Some(approval) = approval else {
                    let answer = if method == "mcpServer/elicitation/request" {
                        json!({ "jsonrpc": "2.0", "id": id, "result": { "action": "decline" } })
                    } else {
                        json!({ "jsonrpc": "2.0", "id": id, "error": { "code": -32601, "message": "Lorca does not answer this" } })
                    };
                    let _ = process.send(&answer).await;
                    continue;
                };
                let (tx, rx) = oneshot::channel();
                if events.send(Event::Approve(approval, tx)).is_err() {
                    continue;
                }
                let process = process.clone();
                tokio::spawn(async move {
                    let allowed = rx.await.map(|verdict| verdict.is_ok()).unwrap_or(false);
                    let decision = if allowed { "accept" } else { "decline" };
                    let _ = process.send(&json!({ "jsonrpc": "2.0", "id": id, "result": { "decision": decision } })).await;
                });
            }
            (Some(method), None) => {
                let params = &message["params"];
                match method {
                    "turn/started" => {
                        *turn.lock().unwrap() = params["turn"]["id"].as_str().map(str::to_string);
                        let _ = events.send(Event::Working);
                    }
                    "turn/completed" => {
                        let ended = params["turn"]["id"].as_str().map(str::to_string);
                        {
                            let mut current = turn.lock().unwrap();
                            if *current == ended || current.is_none() {
                                *current = None;
                            }
                        }
                        match params["turn"]["status"].as_str() {
                            Some("interrupted") => {
                                let _ = events.send(Event::Lines(vec!["  ⎿ Interrupted".into()]));
                                if !cutting_in.load(Ordering::SeqCst) {
                                    let _ = events.send(Event::Idle(last_said.take()));
                                }
                            }
                            Some("failed") => {
                                let error = params["turn"]["error"]["message"].as_str().unwrap_or("Codex stopped with an error").to_string();
                                let _ = events.send(Event::Lines(vec![format!("Error: {error}")]));
                                let _ = events.send(Event::Idle(Some(error)));
                            }
                            _ => {
                                let _ = events.send(Event::Idle(last_said.take()));
                            }
                        }
                    }
                    "item/started" if params["item"]["type"] == "fileChange" => {
                        let item = &params["item"];
                        let paths = item["changes"].as_array().into_iter().flatten().filter_map(|change| change["path"].as_str().map(str::to_string)).collect();
                        if let Some(id) = item["id"].as_str() {
                            changes.insert(id.to_string(), paths);
                        }
                    }
                    "item/completed" => {
                        let item = &params["item"];
                        let out = item_lines(item, &mut last_said, &folder);
                        if let Some(id) = item["id"].as_str() {
                            changes.remove(id);
                        }
                        if !out.is_empty() {
                            let _ = events.send(Event::Lines(out));
                        }
                    }
                    "error" => {
                        if let Some(text) = params["error"]["message"].as_str() {
                            let _ = events.send(Event::Lines(vec![format!("Error: {text}")]));
                        }
                    }
                    _ => {}
                }
            }
            (None, None) => {}
        }
    }
    // Whatever Lorca still waits on will not be answered.
    pending.lock().unwrap().clear();
    let exit = exit.await.ok();
    let ended = match exit {
        Some(process::Exit { stopped: true, .. }) => Ended { outcome: "Stopped".into(), failed: false },
        Some(process::Exit { code: Some(0), .. }) => Ended { outcome: "Codex exited".into(), failed: false },
        Some(process::Exit { code, said, .. }) => {
            let code = code.map(|code| format!(" with code {code}")).unwrap_or_default();
            Ended { outcome: format!("Codex exited{code}{}", said.map(|said| format!(": {said}")).unwrap_or_default()), failed: true }
        }
        None => Ended { outcome: "Codex exited".into(), failed: true },
    };
    let _ = events.send(Event::Ended(ended));
}

/// One finished item of a turn, in the transcript, its paths from `folder`.
fn item_lines(item: &Value, last_said: &mut Option<String>, folder: &Path) -> Vec<String> {
    match item["type"].as_str().unwrap_or("") {
        "userMessage" => {
            let text = item["content"].as_array().into_iter().flatten().filter_map(|part| part["text"].as_str()).collect::<Vec<_>>().join("\n");
            lines::sent(&text)
        }
        "agentMessage" => {
            let text = item["text"].as_str().unwrap_or("");
            if !text.trim().is_empty() {
                *last_said = Some(text.trim().to_string());
            }
            lines::said(text)
        }
        "commandExecution" => {
            let command = item["commandActions"][0]["command"].as_str().or(item["command"].as_str()).unwrap_or("");
            let mut out = vec![lines::call("Bash", command)];
            let failed = item["exitCode"].as_i64().is_some_and(|code| code != 0) || item["status"] == "failed" || item["status"] == "declined";
            let output = item["aggregatedOutput"].as_str().unwrap_or("");
            if item["status"] == "declined" {
                out.push("  ⎿ Not allowed".into());
            } else {
                out.extend(lines::result(output, failed));
            }
            out
        }
        "fileChange" => item["changes"]
            .as_array()
            .into_iter()
            .flatten()
            .map(|change| {
                let name = match change["kind"]["type"].as_str() {
                    Some("add") => "Write",
                    Some("delete") => "Delete",
                    _ => "Edit",
                };
                lines::call(name, &lines::path(change["path"].as_str().unwrap_or(""), folder))
            })
            .collect(),
        "mcpToolCall" => vec![lines::call(&format!("{}.{}", item["server"].as_str().unwrap_or("mcp"), item["tool"].as_str().unwrap_or("tool")), "")],
        "webSearch" => vec![lines::call("WebSearch", item["query"].as_str().unwrap_or(""))],
        _ => Vec::new(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_turns_items_read_as_the_transcript() {
        let mut said = None;
        let command = json!({ "type": "commandExecution", "command": "/bin/zsh -lc 'git status --short'", "commandActions": [{ "command": "git status --short" }], "aggregatedOutput": "?? hi.txt\n", "exitCode": 0, "status": "completed" });
        assert_eq!(item_lines(&command, &mut said, Path::new("/tmp/x")), vec!["● Bash(git status --short)", "  ⎿ ?? hi.txt"]);
        let message = json!({ "type": "agentMessage", "text": "Done.\nAll tests pass." });
        assert_eq!(item_lines(&message, &mut said, Path::new("/tmp/x")), vec!["Done.", "All tests pass."]);
        assert_eq!(said.as_deref(), Some("Done.\nAll tests pass."));
        let change = json!({ "type": "fileChange", "changes": [{ "path": "/tmp/x/hi.txt", "kind": { "type": "add" } }] });
        assert_eq!(item_lines(&change, &mut said, Path::new("/tmp/x")), vec!["● Write(hi.txt)"]);
        assert!(item_lines(&json!({ "type": "reasoning" }), &mut said, Path::new("/tmp/x")).is_empty());
    }
}
