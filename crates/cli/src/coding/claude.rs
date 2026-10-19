//! Claude Code without a terminal: `claude -p` with stream-json in and out, the protocol its
//! Agent SDK speaks. Lorca sends the bot's messages as user messages, reads the transcript as it
//! streams, and decides what Claude Code asks to do: every Bash call through a `PreToolUse` hook,
//! which runs whatever the user's own Claude Code settings allow, and any other tool that would
//! ask through the permission prompt (`--permission-prompt-tool stdio`). Edits in the folder it
//! works in run on their own (`acceptEdits`).

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};

use async_trait::async_trait;
use serde_json::{json, Value};
use tokio::sync::oneshot;
use tokio::time::Instant;

use super::driver::{Approval, Driver, Event, Events};
use super::process::{self, Process};
use super::transcript::{self, Item, PlanEntry, ToolStep};

/// The callback the Bash hook calls back with.
const BASH_HOOK: &str = "lorca-bash";

pub(crate) struct Claude {
    process: Arc<Process>,
    /// Messages sent that it has not taken up yet. It takes one up when it says so
    /// (`--replay-user-messages`): between turns, or at a step of the turn it is on.
    pending: Arc<AtomicUsize>,
}

/// Starts Claude Code in `folder` on `prompt`, or resumes `session` with it.
pub(crate) async fn start(host: &process::Host, program: &Path, folder: &Path, prompt: &str, session: Option<&str>, variables: &[process::Variable], events: Events) -> Result<Arc<dyn Driver>, String> {
    let mut args: Vec<String> = [
        "-p",
        "--input-format",
        "stream-json",
        "--output-format",
        "stream-json",
        "--verbose",
        "--replay-user-messages",
        "--permission-mode",
        "acceptEdits",
        "--permission-prompt-tool",
        "stdio",
    ]
    .iter()
    .map(|arg| arg.to_string())
    .collect();
    if let Some(session) = session {
        args.push("--resume".into());
        args.push(session.into());
    }
    let (process, stdout, exit) = process::spawn(host, program, &args, folder, variables).await?;
    let pending = Arc::new(AtomicUsize::new(0));
    tokio::spawn(read(process.clone(), process::json_lines(stdout), exit, events, pending.clone(), folder.to_path_buf()));
    process
        .send(&json!({
            "type": "control_request",
            "request_id": "lorca-initialize",
            "request": { "subtype": "initialize", "hooks": { "PreToolUse": [{ "matcher": "Bash", "hookCallbackIds": [BASH_HOOK] }] } },
        }))
        .await?;
    let driver = Arc::new(Claude { process, pending });
    driver.send(prompt, false).await?;
    Ok(driver)
}

#[async_trait]
impl Driver for Claude {
    async fn send(&self, text: &str, interrupt: bool) -> Result<(), String> {
        if interrupt {
            let id = format!("lorca-interrupt-{}", uuid::Uuid::new_v4());
            self.process.send(&json!({ "type": "control_request", "request_id": id, "request": { "subtype": "interrupt" } })).await?;
        }
        self.pending.fetch_add(1, Ordering::SeqCst);
        let sent = self.process.send(&json!({ "type": "user", "message": { "role": "user", "content": text } })).await;
        if sent.is_err() {
            self.pending.fetch_sub(1, Ordering::SeqCst);
        }
        sent
    }

    async fn stop(&self) {
        self.process.stop();
    }
}

/// One of Claude Code's tool calls as a step, its paths from `folder`; `TodoWrite` is its plan.
pub(crate) fn step(name: &str, input: &Value, folder: &Path) -> Option<ToolStep> {
    let field = |key: &str| input.get(key).and_then(Value::as_str).unwrap_or("").to_string();
    let path = |key: &str| Some(field(key)).filter(|path| !path.is_empty()).map(|path| transcript::path(&path, folder));
    let (kind, title, file) = match name {
        "Read" => ("read", path("file_path").unwrap_or_default(), path("file_path")),
        "Glob" | "Grep" => ("search", field("pattern"), None),
        "Edit" | "MultiEdit" | "Write" => ("edit", path("file_path").unwrap_or_default(), path("file_path")),
        "NotebookEdit" => ("edit", path("notebook_path").unwrap_or_default(), path("notebook_path")),
        "Bash" => ("execute", field("command"), None),
        "WebFetch" => ("fetch", field("url"), None),
        "WebSearch" => ("fetch", field("query"), None),
        "TodoWrite" => return None,
        "Task" | "Agent" => ("other", field("description"), None),
        _ => {
            // An MCP server's tool, `mcp__server__tool`, as "server: tool".
            let title = name.strip_prefix("mcp__").map(|rest| rest.replacen("__", ": ", 1)).unwrap_or_else(|| name.to_string());
            ("other", title, None)
        }
    };
    let (added, removed) = match name {
        "Edit" => (Some(transcript::line_count(&field("new_string"))), Some(transcript::line_count(&field("old_string")))),
        "MultiEdit" => {
            let edits = input["edits"].as_array().cloned().unwrap_or_default();
            let count = |key: &str| edits.iter().map(|edit| transcript::line_count(edit[key].as_str().unwrap_or(""))).sum::<u32>();
            (Some(count("new_string")), Some(count("old_string")))
        }
        "Write" => (Some(transcript::line_count(&field("content"))), None),
        _ => (None, None),
    };
    Some(ToolStep { kind: kind.into(), title: transcript::title(&title), path: file, added, removed, status: "running".into(), output: None })
}

/// Its to-do list as a plan.
pub(crate) fn plan(input: &Value) -> Vec<PlanEntry> {
    input["todos"]
        .as_array()
        .into_iter()
        .flatten()
        .map(|todo| PlanEntry { text: todo["content"].as_str().unwrap_or("").to_string(), status: todo["status"].as_str().unwrap_or("pending").to_string() })
        .collect()
}

async fn read(process: Arc<Process>, mut messages: tokio::sync::mpsc::UnboundedReceiver<Value>, exit: oneshot::Receiver<process::Exit>, events: Events, pending: Arc<AtomicUsize>, folder: PathBuf) {
    let mut session_told = false;
    let mut last_said: Option<String> = None;
    // The calls under way, and the ones Lorca said no to.
    let mut steps: HashMap<String, ToolStep> = HashMap::new();
    let denied: Arc<Mutex<HashSet<String>>> = Arc::new(Mutex::new(HashSet::new()));
    let mut sent = 0usize;
    let mut since = Instant::now();
    let item = |item: Item| {
        let _ = events.send(Event::Item(item));
    };
    while let Some(message) = messages.recv().await {
        match message["type"].as_str().unwrap_or("") {
            "system" if message["subtype"] == "init" => {
                if let (Some(id), false) = (message["session_id"].as_str(), session_told) {
                    session_told = true;
                    let _ = events.send(Event::Session(id.to_string()));
                }
            }
            "assistant" => {
                // A subagent's steps stay inside the call that started it.
                if !message["parent_tool_use_id"].is_null() {
                    continue;
                }
                let id = message["message"]["id"].as_str().map(str::to_string).unwrap_or_else(|| uuid::Uuid::new_v4().to_string());
                for (index, part) in message["message"]["content"].as_array().into_iter().flatten().enumerate() {
                    match part["type"].as_str() {
                        Some("thinking") => {
                            let text = part["thinking"].as_str().unwrap_or("");
                            item(Item { seconds: Some(since.elapsed().as_secs_f64().round()), ..Item::text(format!("{id}-{index}"), "thought", text) });
                        }
                        Some("text") => {
                            let text = part["text"].as_str().unwrap_or("");
                            if !text.trim().is_empty() {
                                last_said = Some(text.trim().to_string());
                                item(Item::text(format!("{id}-{index}"), "text", text));
                            }
                        }
                        Some("tool_use") => {
                            let call = part["id"].as_str().unwrap_or("").to_string();
                            let name = part["name"].as_str().unwrap_or("Tool");
                            if name == "TodoWrite" {
                                item(Item::plan("plan", plan(&part["input"])));
                                continue;
                            }
                            if let Some(step) = step(name, &part["input"], &folder) {
                                steps.insert(call.clone(), step.clone());
                                item(Item::tool(call, step));
                            }
                        }
                        _ => {}
                    }
                }
                since = Instant::now();
            }
            "user" => {
                if !message["parent_tool_use_id"].is_null() {
                    continue;
                }
                let content = &message["message"]["content"];
                // One of the messages Lorca sent, as it takes it up.
                if message["isReplay"].as_bool().unwrap_or(false) || content.is_string() {
                    let text = transcript::content_text(content);
                    if text.starts_with("[Request interrupted") {
                        item(Item::text(uuid::Uuid::new_v4().to_string(), "status", "Interrupted"));
                        continue;
                    }
                    let _ = pending.fetch_update(Ordering::SeqCst, Ordering::SeqCst, |count| Some(count.saturating_sub(1)));
                    sent += 1;
                    item(Item::text(format!("prompt-{}-{sent}", uuid::Uuid::new_v4()), "prompt", &text));
                    let _ = events.send(Event::Working);
                    since = Instant::now();
                    continue;
                }
                for part in content.as_array().into_iter().flatten() {
                    if part["type"] == "tool_result" {
                        let call = part["tool_use_id"].as_str().unwrap_or("").to_string();
                        let Some(mut step) = steps.remove(&call) else { continue };
                        let text = transcript::content_text(&part["content"]);
                        step.status = if denied.lock().unwrap().remove(&call) {
                            "denied"
                        } else if part["is_error"].as_bool().unwrap_or(false) {
                            "failed"
                        } else {
                            "done"
                        }
                        .into();
                        step.output = transcript::output(&text);
                        item(Item::tool(call, step));
                    } else if part["text"].as_str().is_some_and(|text| text.starts_with("[Request interrupted")) {
                        item(Item::text(uuid::Uuid::new_v4().to_string(), "status", "Interrupted"));
                    }
                }
                since = Instant::now();
            }
            // The end of a turn. With a message still to take up, it goes on with that one.
            "result" => {
                let mut failed = None;
                if message["is_error"].as_bool().unwrap_or(false) && message["subtype"] != "error_during_execution" {
                    let reason = message["result"].as_str().or(message["subtype"].as_str()).unwrap_or("error");
                    let reason = reason.lines().next().unwrap_or(reason).to_string();
                    item(Item::text(uuid::Uuid::new_v4().to_string(), "error", &reason));
                    failed = Some(reason);
                }
                // Calls the turn never finished, an interrupt's, end with it.
                for (call, mut step) in steps.drain() {
                    step.status = "failed".into();
                    item(Item::tool(call, step));
                }
                if pending.load(Ordering::SeqCst) == 0 {
                    if let Some(reason) = failed {
                        let _ = events.send(Event::Failed(reason));
                        continue;
                    }
                    let said = message["result"].as_str().filter(|text| !text.trim().is_empty()).map(str::to_string).or_else(|| last_said.take());
                    let _ = events.send(Event::Idle(said));
                }
            }
            "control_request" => {
                let Some(id) = message["request_id"].as_str().map(str::to_string) else { continue };
                let request = &message["request"];
                let approval = match request["subtype"].as_str() {
                    Some("hook_callback") => {
                        let input = &request["input"];
                        let command = input["tool_input"]["command"].as_str().unwrap_or("").to_string();
                        let call = input["tool_use_id"].as_str().or(request["tool_use_id"].as_str()).map(str::to_string);
                        (input["tool_name"] == "Bash").then_some((Approval::Command { command }, true, call))
                    }
                    Some("can_use_tool") => {
                        let name = request["tool_name"].as_str().unwrap_or("").to_string();
                        let input = request["input"].clone();
                        let approval = match name.as_str() {
                            "Bash" => Approval::Command { command: input["command"].as_str().unwrap_or("").to_string() },
                            "Write" | "Edit" | "MultiEdit" | "NotebookEdit" => {
                                let path = input["file_path"].as_str().or(input["notebook_path"].as_str()).unwrap_or("").to_string();
                                Approval::Files { paths: vec![path] }
                            }
                            _ => Approval::Tool { name, input: input.clone() },
                        };
                        Some((approval, false, request["tool_use_id"].as_str().map(str::to_string)))
                    }
                    _ => None,
                };
                let Some((approval, hook, call)) = approval else {
                    // Nothing else is Lorca's to answer; it is told so.
                    let _ = process.send(&json!({ "type": "control_response", "response": { "subtype": "error", "request_id": id, "error": "Not supported" } })).await;
                    continue;
                };
                let (tx, rx) = oneshot::channel();
                if events.send(Event::Approve(approval, tx)).is_err() {
                    continue;
                }
                let input = request["input"].clone();
                let process = process.clone();
                let denied = denied.clone();
                tokio::spawn(async move {
                    let verdict = rx.await.unwrap_or_else(|_| Err("Lorca stopped before deciding".into()));
                    if let (Err(_), Some(call)) = (&verdict, call) {
                        denied.lock().unwrap().insert(call);
                    }
                    let answer = match (hook, verdict) {
                        (true, Ok(())) => json!({ "hookSpecificOutput": { "hookEventName": "PreToolUse", "permissionDecision": "allow", "permissionDecisionReason": "Lorca allowed it." } }),
                        (true, Err(reason)) => json!({ "hookSpecificOutput": { "hookEventName": "PreToolUse", "permissionDecision": "deny", "permissionDecisionReason": reason } }),
                        (false, Ok(())) => json!({ "behavior": "allow", "updatedInput": input }),
                        (false, Err(reason)) => json!({ "behavior": "deny", "message": reason }),
                    };
                    let _ = process.send(&json!({ "type": "control_response", "response": { "subtype": "success", "request_id": id, "response": answer } })).await;
                });
            }
            _ => {}
        }
    }
    let _ = events.send(Event::Ended(super::driver::ended("Claude Code", exit.await.ok())));
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn its_tools_read_as_steps() {
        let folder = Path::new("/work/shop");
        let read = step("Read", &json!({ "file_path": "/work/shop/src/login.rs" }), folder).unwrap();
        assert_eq!((read.kind.as_str(), read.title.as_str(), read.path.as_deref()), ("read", "src/login.rs", Some("src/login.rs")));
        let edit = step("Edit", &json!({ "file_path": "/work/shop/a.rs", "old_string": "a", "new_string": "b\nc" }), folder).unwrap();
        assert_eq!((edit.kind.as_str(), edit.added, edit.removed), ("edit", Some(2), Some(1)));
        assert_eq!(step("Grep", &json!({ "pattern": "fn main" }), folder).unwrap().kind, "search");
        assert_eq!(step("mcp__github__create_issue", &json!({}), folder).unwrap().title, "github: create_issue");
        assert!(step("TodoWrite", &json!({}), folder).is_none());
        let plan = plan(&json!({ "todos": [{ "content": "Fix", "status": "in_progress" }] }));
        assert_eq!(plan, vec![PlanEntry { text: "Fix".into(), status: "in_progress".into() }]);
    }
}
