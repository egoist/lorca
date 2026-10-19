//! pi without a terminal: `pi --mode rpc`, the JSONL protocol its own integrations speak. A
//! message is a `prompt`, or while it works a `steer`, which it takes up before its next model
//! call; an interrupt is `abort` first. pi asks nobody before a tool runs, so Lorca loads an
//! extension of its own (`-e`) whose `tool_call` handler asks Lorca, through the protocol's
//! extension dialogs, before each tool that does more than read: what Lorca does not allow is
//! blocked with why.

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use async_trait::async_trait;
use serde_json::{json, Value};
use tokio::sync::oneshot;
use tokio::time::Instant;

use super::driver::{Approval, Driver, Event, Events};
use super::process::{self, Process};
use super::transcript::{self, Item, ToolStep};

/// The extension that asks Lorca before a tool runs. pi's own tools that only read run at once.
const REVIEW_EXTENSION: &str = r#"// Written by Lorca: asks Lorca before each of pi's tool calls that does more than read.
const READS = new Set(["read", "grep", "find", "ls"]);
export default function (pi) {
  pi.on("tool_call", async (event, ctx) => {
    if (READS.has(event.toolName)) return undefined;
    const title = JSON.stringify({ lorca: "review", id: event.toolCallId, tool: event.toolName, input: event.input });
    const answer = await ctx.ui.input(title, "");
    if (answer === "allow") return undefined;
    return { block: true, reason: answer || "Lorca did not allow it" };
  });
}
"#;

/// How long pi has to answer a command.
const REQUEST_TIMEOUT: Duration = Duration::from_secs(60);

type Pending = Mutex<HashMap<String, oneshot::Sender<Result<Value, String>>>>;

pub(crate) struct Pi {
    process: Arc<Process>,
    pending: Arc<Pending>,
    next_id: AtomicU64,
    /// It works on something: a message steers it.
    busy: Arc<AtomicBool>,
}

/// Where Lorca keeps the review extension, written again as each agent starts.
fn extension(home: &Path) -> Result<PathBuf, String> {
    let path = home.join("agents").join("lorca-review.ts");
    std::fs::create_dir_all(path.parent().unwrap_or(home)).map_err(|e| e.to_string())?;
    std::fs::write(&path, REVIEW_EXTENSION).map_err(|e| format!("Could not write pi's review extension: {e}"))?;
    Ok(path)
}

/// Starts pi in `folder` on `prompt`, or resumes `session`, its session file, with it.
pub(crate) async fn start(home: &Path, program: &Path, folder: &Path, prompt: &str, session: Option<&str>, variables: &[process::Variable], events: Events) -> Result<Arc<dyn Driver>, String> {
    let mut args: Vec<String> = vec!["--mode".into(), "rpc".into(), "-e".into(), extension(home)?.to_string_lossy().to_string()];
    if let Some(session) = session {
        args.push("--session".into());
        args.push(session.into());
    }
    let (process, stdout, exit) = process::spawn(program, &args, folder, variables).await?;
    let pending: Arc<Pending> = Arc::new(Mutex::new(HashMap::new()));
    let busy = Arc::new(AtomicBool::new(false));
    tokio::spawn(read(process.clone(), process::json_lines(stdout), exit, events.clone(), pending.clone(), busy.clone(), folder.to_path_buf()));
    let pi = Arc::new(Pi { process, pending, next_id: AtomicU64::new(1), busy });
    pi.send(prompt, false).await?;
    // Its session file resumes it once its process is gone. pi answers once it can, which may be
    // after a tool waiting on Lorca has its answer.
    let asking = pi.clone();
    tokio::spawn(async move {
        if let Ok(state) = asking.command(json!({ "type": "get_state" })).await {
            if let Some(file) = state["sessionFile"].as_str() {
                let _ = events.send(Event::Session(file.to_string()));
            }
        }
    });
    Ok(pi)
}

impl Pi {
    async fn command(&self, mut command: Value) -> Result<Value, String> {
        let id = format!("lorca-{}", self.next_id.fetch_add(1, Ordering::SeqCst));
        command["id"] = json!(id);
        let (tx, rx) = oneshot::channel();
        self.pending.lock().unwrap().insert(id.clone(), tx);
        if let Err(error) = self.process.send(&command).await {
            self.pending.lock().unwrap().remove(&id);
            return Err(error);
        }
        match tokio::time::timeout(REQUEST_TIMEOUT, rx).await {
            Ok(Ok(answer)) => answer,
            Ok(Err(_)) => Err("pi is no longer running".into()),
            Err(_) => {
                self.pending.lock().unwrap().remove(&id);
                Err(format!("pi did not answer {}", command["type"].as_str().unwrap_or("a command")))
            }
        }
    }
}

#[async_trait]
impl Driver for Pi {
    async fn send(&self, text: &str, interrupt: bool) -> Result<(), String> {
        if interrupt && self.busy.load(Ordering::SeqCst) {
            self.command(json!({ "type": "abort" })).await?;
        }
        // While it works, the message goes in before its next model call.
        let command = if self.busy.load(Ordering::SeqCst) { json!({ "type": "steer", "message": text }) } else { json!({ "type": "prompt", "message": text, "streamingBehavior": "steer" }) };
        self.command(command).await.map(|_| ())
    }

    async fn stop(&self) {
        self.process.stop();
    }
}

/// One of pi's tool calls as a step, its paths from `folder`.
fn step(name: &str, args: &Value, folder: &Path) -> ToolStep {
    let field = |key: &str| args[key].as_str().unwrap_or("").to_string();
    let path = || Some(field("path")).filter(|path| !path.is_empty()).map(|path| transcript::path(&path, folder));
    let (kind, title, file) = match name {
        "read" => ("read", path().unwrap_or_default(), path()),
        "grep" | "find" => ("search", field("pattern"), None),
        "ls" => ("search", path().unwrap_or_else(|| ".".into()), None),
        "edit" | "write" => ("edit", path().unwrap_or_default(), path()),
        "bash" => ("execute", field("command"), None),
        "web_search" | "web_fetch" | "fetch" => ("fetch", args["url"].as_str().or(args["query"].as_str()).or(args["queries"][0].as_str()).unwrap_or("").to_string(), None),
        _ => ("other", name.strip_prefix("mcp__").map(|rest| rest.replacen("__", ": ", 1)).unwrap_or_else(|| name.to_string()), None),
    };
    let (added, removed) = match name {
        "write" => (Some(transcript::line_count(&field("content"))), None),
        "edit" => {
            let edits: Vec<Value> = match args["edits"].as_array() {
                Some(edits) => edits.clone(),
                None => vec![args.clone()],
            };
            let count = |keys: [&str; 2]| edits.iter().map(|edit| transcript::line_count(edit[keys[0]].as_str().or(edit[keys[1]].as_str()).unwrap_or(""))).sum::<u32>();
            (Some(count(["newText", "new_string"])), Some(count(["oldText", "old_string"])))
        }
        _ => (None, None),
    };
    ToolStep { kind: kind.into(), title: transcript::title(&title), path: file, added, removed, status: "running".into(), output: None }
}

/// What a tool's call asks Lorca, from the extension's dialog.
fn approval(name: &str, input: &Value) -> Approval {
    match name {
        "bash" => Approval::Command { command: input["command"].as_str().unwrap_or("").to_string() },
        "edit" | "write" => Approval::Files { paths: input["path"].as_str().map(|path| vec![path.to_string()]).unwrap_or_default() },
        _ => Approval::Tool { name: name.to_string(), input: input.clone() },
    }
}

/// The text of a message's content: a string, or text blocks.
fn message_text(content: &Value) -> String {
    match content {
        Value::String(text) => text.clone(),
        _ => content.as_array().into_iter().flatten().filter(|part| part["type"] == "text").filter_map(|part| part["text"].as_str()).collect::<Vec<_>>().join("\n"),
    }
}

async fn read(process: Arc<Process>, mut messages: tokio::sync::mpsc::UnboundedReceiver<Value>, exit: oneshot::Receiver<process::Exit>, events: Events, pending: Arc<Pending>, busy: Arc<AtomicBool>, folder: PathBuf) {
    let item = |item: Item| {
        let _ = events.send(Event::Item(item));
    };
    let mut steps: HashMap<String, ToolStep> = HashMap::new();
    let denied: Arc<Mutex<HashSet<String>>> = Arc::new(Mutex::new(HashSet::new()));
    let mut last_said: Option<String> = None;
    // Why its run gave up, once its retries ran out.
    let mut failed: Option<String> = None;
    // Which assistant message it is on, for its steps' ids, and when its thinking began.
    let mut message = 0u64;
    let mut thinking: HashMap<u64, Instant> = HashMap::new();
    while let Some(record) = messages.recv().await {
        match record["type"].as_str().unwrap_or("") {
            "response" => {
                let Some(id) = record["id"].as_str() else { continue };
                let Some(tx) = pending.lock().unwrap().remove(id) else { continue };
                let answer = if record["success"].as_bool().unwrap_or(false) { Ok(record["data"].clone()) } else { Err(record["error"].as_str().unwrap_or("pi refused").to_string()) };
                let _ = tx.send(answer);
            }
            "agent_start" => {
                if !busy.swap(true, Ordering::SeqCst) {
                    let _ = events.send(Event::Working);
                }
            }
            "message_start" if record["message"]["role"] == "assistant" => {
                message += 1;
                thinking.clear();
            }
            "message_end" if record["message"]["role"] == "user" => {
                let text = message_text(&record["message"]["content"]);
                item(Item::text(format!("prompt-{}", uuid::Uuid::new_v4()), "prompt", &text));
            }
            "message_update" => {
                let update = &record["assistantMessageEvent"];
                let index = update["contentIndex"].as_u64().unwrap_or(0);
                match update["type"].as_str() {
                    Some("thinking_start") => {
                        thinking.insert(index, Instant::now());
                    }
                    Some("thinking_end") => {
                        let seconds = thinking.remove(&index).map(|began| began.elapsed().as_secs_f64().round());
                        let text = update["content"].as_str().unwrap_or("");
                        item(Item { seconds, ..Item::text(format!("m{message}-{index}"), "thought", text) });
                    }
                    Some("text_end") => {
                        let text = update["content"].as_str().unwrap_or("");
                        if !text.trim().is_empty() {
                            last_said = Some(text.trim().to_string());
                            item(Item::text(format!("m{message}-{index}"), "text", text));
                        }
                    }
                    _ => {}
                }
            }
            "tool_execution_start" => {
                let call = record["toolCallId"].as_str().unwrap_or("").to_string();
                let step = step(record["toolName"].as_str().unwrap_or("tool"), &record["args"], &folder);
                steps.insert(call.clone(), step.clone());
                item(Item::tool(call, step));
            }
            "tool_execution_end" => {
                let call = record["toolCallId"].as_str().unwrap_or("").to_string();
                let mut step = steps.remove(&call).unwrap_or_else(|| step(record["toolName"].as_str().unwrap_or("tool"), &json!({}), &folder));
                let failed = record["isError"].as_bool().unwrap_or(false);
                step.status = if denied.lock().unwrap().remove(&call) { "denied" } else if failed { "failed" } else { "done" }.into();
                step.output = transcript::output(&transcript::content_text(&record["result"]["content"]));
                item(Item::tool(call, step));
            }
            "auto_retry_end" if record["success"] == false => {
                let error = record["finalError"].as_str().unwrap_or("pi stopped with an error").to_string();
                item(Item::text(uuid::Uuid::new_v4().to_string(), "error", &error));
                failed = Some(error);
            }
            "agent_settled" => {
                for (call, mut step) in steps.drain() {
                    step.status = "failed".into();
                    item(Item::tool(call, step));
                }
                if record["aborted"].as_bool().unwrap_or(false) {
                    item(Item::text(uuid::Uuid::new_v4().to_string(), "status", "Interrupted"));
                }
                busy.store(false, Ordering::SeqCst);
                let _ = events.send(match failed.take() {
                    Some(error) if !record["aborted"].as_bool().unwrap_or(false) => Event::Failed(error),
                    _ => Event::Idle(last_said.take()),
                });
            }
            "extension_ui_request" => {
                let Some(id) = record["id"].as_str().map(str::to_string) else { continue };
                let asked: Option<Value> = record["title"].as_str().and_then(|title| serde_json::from_str(title).ok());
                let review = asked.filter(|asked| asked["lorca"] == "review");
                match (record["method"].as_str(), review) {
                    (Some("input"), Some(asked)) => {
                        let approval = approval(asked["tool"].as_str().unwrap_or("tool"), &asked["input"]);
                        let call = asked["id"].as_str().map(str::to_string);
                        let (tx, rx) = oneshot::channel();
                        if events.send(Event::Approve(approval, tx)).is_err() {
                            continue;
                        }
                        let (process, denied) = (process.clone(), denied.clone());
                        tokio::spawn(async move {
                            let verdict = rx.await.unwrap_or_else(|_| Err("Lorca stopped before deciding".into()));
                            let value = match verdict {
                                Ok(()) => "allow".to_string(),
                                Err(reason) => {
                                    denied.lock().unwrap().extend(call);
                                    reason
                                }
                            };
                            let _ = process.send(&json!({ "type": "extension_ui_response", "id": id, "value": value })).await;
                        });
                    }
                    // Another extension's dialog: nobody is at pi's screen to answer it.
                    (Some("select" | "confirm" | "input" | "editor"), _) => {
                        let _ = process.send(&json!({ "type": "extension_ui_response", "id": id, "cancelled": true })).await;
                    }
                    _ => {}
                }
            }
            _ => {}
        }
    }
    pending.lock().unwrap().clear();
    let _ = events.send(Event::Ended(super::driver::ended("pi", exit.await.ok())));
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn its_tools_read_as_steps_and_ask_as_lorca_reviews_them() {
        let folder = Path::new("/work/shop");
        let edit = step("edit", &json!({ "path": "src/a.rs", "edits": [{ "oldText": "a", "newText": "b\nc" }] }), folder);
        assert_eq!((edit.kind.as_str(), edit.title.as_str(), edit.added, edit.removed), ("edit", "src/a.rs", Some(2), Some(1)));
        assert_eq!(step("bash", &json!({ "command": "date -u" }), folder).title, "date -u");
        assert_eq!(step("grep", &json!({ "pattern": "fn main" }), folder).kind, "search");
        assert!(matches!(approval("bash", &json!({ "command": "git push" })), Approval::Command { command } if command == "git push"));
        assert!(matches!(approval("write", &json!({ "path": "b.txt" })), Approval::Files { paths } if paths == vec!["b.txt".to_string()]));
        assert_eq!(message_text(&json!([{ "type": "text", "text": "Fix it" }])), "Fix it");
    }
}
