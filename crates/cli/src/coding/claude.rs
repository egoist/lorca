//! Claude Code without a terminal: `claude -p` with stream-json in and out, the protocol its
//! Agent SDK speaks. Lorca sends the bot's messages as user messages, reads the transcript as it
//! streams, and decides what Claude Code asks to do: every Bash call through a `PreToolUse` hook,
//! which runs whatever the user's own Claude Code settings allow, and any other tool that would
//! ask through the permission prompt (`--permission-prompt-tool stdio`). Edits in the folder it
//! works in run on their own (`acceptEdits`).

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

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
    /// It takes a message at once when asked to (`interrupt_send_now_v1`): the turn under way
    /// makes way for it, its command moved to the background or ended.
    send_now: Arc<AtomicBool>,
}

/// Starts Claude Code in `folder` on `prompt`, or resumes `session` with it.
pub(crate) async fn start(program: &Path, folder: &Path, prompt: &str, session: Option<&str>, variables: &[process::Variable], events: Events) -> Result<Arc<dyn Driver>, String> {
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
    let (process, stdout, exit) = process::spawn(program, &args, folder, variables).await?;
    let pending = Arc::new(AtomicUsize::new(0));
    let send_now = Arc::new(AtomicBool::new(false));
    tokio::spawn(read(process.clone(), process::json_lines(stdout), exit, events, pending.clone(), send_now.clone(), folder.to_path_buf()));
    process
        .send(&json!({
            "type": "control_request",
            "request_id": "lorca-initialize",
            "request": { "subtype": "initialize", "hooks": { "PreToolUse": [{ "matcher": "Bash", "hookCallbackIds": [BASH_HOOK] }] } },
        }))
        .await?;
    let driver = Arc::new(Claude { process, pending, send_now });
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
        let uuid = uuid::Uuid::new_v4().to_string();
        let sent = self.process.send(&json!({ "type": "user", "message": { "role": "user", "content": text }, "uuid": uuid })).await;
        if sent.is_err() {
            self.pending.fetch_sub(1, Ordering::SeqCst);
            return sent;
        }
        // A message steers: the turn under way takes it up now rather than once it is done.
        // Between turns this does nothing, and the message starts the next.
        if !interrupt && self.send_now.load(Ordering::SeqCst) {
            let id = format!("lorca-send-now-{}", uuid::Uuid::new_v4());
            self.process.send(&json!({ "type": "control_request", "request_id": id, "request": { "subtype": "interrupt", "send_now": true, "message_uuid": uuid } })).await?;
        }
        Ok(())
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

/// `AskUserQuestion`'s questions, each known by its own text, which is how its answers name it.
pub(crate) fn questions(input: &Value) -> Vec<crate::model::Question> {
    use crate::model::{Question, QuestionOption};
    let text = |value: &Value| value.as_str().unwrap_or("").to_string();
    input["questions"]
        .as_array()
        .into_iter()
        .flatten()
        .map(|question| Question {
            id: text(&question["question"]),
            header: text(&question["header"]),
            question: text(&question["question"]),
            options: question["options"].as_array().into_iter().flatten().map(|option| QuestionOption { label: text(&option["label"]), description: text(&option["description"]) }).collect(),
            multi_select: question["multiSelect"].as_bool().unwrap_or(false),
        })
        .filter(|question| !question.question.is_empty())
        .collect()
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

/// How long Claude Code has, once its last background task settled after a turn that ended while
/// it ran, to start the turn that hears of it, before the turn's last message stands as its report.
const SETTLE: Duration = Duration::from_secs(30);

/// Turn ends (`terminal_reason`) that are failures, whatever the result's `subtype` says: its
/// last request to the model refused or failed, or it ran out of room or turns.
const FAILED_ENDS: &[&str] = &[
    "api_error",
    "model_error",
    "blocking_limit",
    "rapid_refill_breaker",
    "prompt_too_long",
    "image_error",
    "malformed_tool_use_exhausted",
    "budget_exhausted",
    "structured_output_retry_exhausted",
    "turn_setup_failed",
    "max_turns",
];

/// Why a turn's `result` failed, in words, or `None` when it did not: an interrupt's
/// (`error_during_execution` without a failed end) is no failure.
fn failure(result: &Value) -> Option<String> {
    let end = result["terminal_reason"].as_str();
    let failed_end = end.is_some_and(|end| FAILED_ENDS.contains(&end));
    let is_error = result["is_error"].as_bool().unwrap_or(false) && result["subtype"] != "error_during_execution";
    if !failed_end && !is_error {
        return None;
    }
    let text = result["result"].as_str().into_iter().chain(result["errors"].as_array().into_iter().flatten().filter_map(Value::as_str)).find(|text| !text.trim().is_empty());
    let reason = match (text, end) {
        (Some(text), _) => text.trim().lines().next().unwrap_or(text).to_string(),
        (None, Some(end)) if failed_end => format!("Claude Code's turn ended in an error ({end})"),
        _ => result["subtype"].as_str().unwrap_or("error").to_string(),
    };
    Some(reason)
}

async fn read(process: Arc<Process>, mut messages: tokio::sync::mpsc::UnboundedReceiver<Value>, exit: oneshot::Receiver<process::Exit>, events: Events, pending: Arc<AtomicUsize>, send_now: Arc<AtomicBool>, folder: PathBuf) {
    let mut session_told = false;
    let mut last_said: Option<String> = None;
    // The calls under way, and the ones Lorca said no to.
    let mut steps: HashMap<String, ToolStep> = HashMap::new();
    let denied: Arc<Mutex<HashSet<String>>> = Arc::new(Mutex::new(HashSet::new()));
    let mut sent = 0usize;
    let mut since = Instant::now();
    // A `result` ends a turn, not the work: Claude Code's background tasks (commands, subagents)
    // run on past it, and when one settles Claude Code takes the turn up again on its own to hear
    // of it. The background tasks live now, by id, with what each does (`background_tasks_changed`
    // holds the whole set); the calls that started them, by task, and their steps, which run
    // until the task settles.
    let mut background: Vec<(String, String)> = Vec::new();
    let mut calls: HashMap<String, String> = HashMap::new();
    let mut backgrounded: HashSet<String> = HashSet::new();
    let mut waiting: HashMap<String, ToolStep> = HashMap::new();
    // Whether a turn is under way, what the last one said when it ended while background tasks
    // ran, and when that stands as its report if no turn follows.
    let mut in_turn = false;
    let mut held: Option<Option<String>> = None;
    let mut settle_at: Option<Instant> = None;
    // Compactions so far, for their status lines' ids, and whether one is under way.
    let mut compactions = 0u32;
    let mut compacting = false;
    let item = |item: Item| {
        let _ = events.send(Event::Item(item));
    };
    loop {
        let message = match settle_at {
            Some(at) => tokio::select! {
                message = messages.recv() => message,
                _ = tokio::time::sleep_until(at) => {
                    settle_at = None;
                    if !in_turn && background.is_empty() {
                        if let Some(said) = held.take() {
                            let _ = events.send(Event::Idle(said));
                        }
                    }
                    continue;
                }
            },
            None => messages.recv().await,
        };
        let Some(message) = message else { break };
        match message["type"].as_str().unwrap_or("") {
            "system" if message["subtype"] == "init" => {
                let capabilities = message["capabilities"].as_array().into_iter().flatten();
                send_now.store(capabilities.into_iter().any(|capability| capability == "interrupt_send_now_v1"), Ordering::SeqCst);
                if let (Some(id), false) = (message["session_id"].as_str(), session_told) {
                    session_told = true;
                    let _ = events.send(Event::Session(id.to_string()));
                }
                // A turn begins, one Claude Code took up on its own (a background task settled)
                // as much as one for a message.
                if !in_turn {
                    in_turn = true;
                    settle_at = None;
                    let _ = events.send(Event::Working);
                }
            }
            "system" => match message["subtype"].as_str().unwrap_or("") {
                "background_tasks_changed" => {
                    background = message["tasks"]
                        .as_array()
                        .into_iter()
                        .flatten()
                        .map(|task| (task["task_id"].as_str().unwrap_or("").to_string(), task["description"].as_str().unwrap_or("").to_string()))
                        .collect();
                    // The last one settled after the turn ended: Claude Code goes on to hear of it.
                    settle_at = (background.is_empty() && !in_turn && held.is_some()).then(|| Instant::now() + SETTLE);
                }
                "task_started" => {
                    let (Some(task), Some(call)) = (message["task_id"].as_str(), message["tool_use_id"].as_str()) else { continue };
                    calls.insert(task.to_string(), call.to_string());
                    if message["is_backgrounded"].as_bool().unwrap_or(false) {
                        backgrounded.insert(call.to_string());
                    }
                }
                // A call under way moved to the background (`send_now`, or Claude Code's own).
                "task_updated" if message["patch"]["is_backgrounded"] == true => {
                    if let Some(call) = message["task_id"].as_str().and_then(|task| calls.get(task)) {
                        backgrounded.insert(call.clone());
                    }
                }
                "task_notification" => {
                    let task = message["task_id"].as_str().unwrap_or("");
                    let call = message["tool_use_id"].as_str().map(str::to_string).or_else(|| calls.get(task).cloned());
                    calls.remove(task);
                    let Some(call) = call else { continue };
                    backgrounded.remove(&call);
                    if let Some(mut step) = waiting.remove(&call) {
                        step.status = if message["status"] == "completed" { "done" } else { "failed" }.into();
                        if let Some(summary) = message["summary"].as_str().and_then(transcript::output) {
                            step.output = Some(summary);
                        }
                        item(Item::tool(call, step));
                    }
                }
                "status" if message["status"] == "compacting" => {
                    compactions += 1;
                    compacting = true;
                    item(Item::text(format!("compact-{compactions}"), "status", "Compacting the conversation"));
                }
                "status" if message["compact_result"] == "failed" => {
                    compacting = false;
                    let error = message["compact_error"].as_str().unwrap_or("it failed");
                    item(Item::text(format!("compact-{compactions}"), "status", &format!("Could not compact the conversation: {error}")));
                }
                "compact_boundary" => {
                    if !std::mem::replace(&mut compacting, false) {
                        compactions += 1;
                    }
                    item(Item::text(format!("compact-{compactions}"), "status", "Compacted the conversation"));
                }
                _ => {}
            },
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
                    in_turn = true;
                    settle_at = None;
                    let _ = events.send(Event::Working);
                    since = Instant::now();
                    continue;
                }
                for part in content.as_array().into_iter().flatten() {
                    if part["type"] == "tool_result" {
                        let call = part["tool_use_id"].as_str().unwrap_or("").to_string();
                        let Some(mut step) = steps.remove(&call) else { continue };
                        let text = transcript::content_text(&part["content"]);
                        // A call that went to the background runs on until its task settles.
                        if backgrounded.remove(&call) {
                            step.output = transcript::output(&text);
                            waiting.insert(call.clone(), step.clone());
                            item(Item::tool(call, step));
                            continue;
                        }
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
            // The end of a turn. With a message still to take up, it goes on with that one; with
            // background tasks still running, it is waiting on them, and Claude Code takes the
            // turn up again once one settles: only the turn that ends with none running is done.
            "result" => {
                in_turn = false;
                let failed = failure(&message);
                if let Some(reason) = &failed {
                    item(Item::text(uuid::Uuid::new_v4().to_string(), "error", reason));
                }
                // Calls the turn never finished, an interrupt's, end with it.
                for (call, mut step) in steps.drain() {
                    step.status = "failed".into();
                    item(Item::tool(call, step));
                }
                if pending.load(Ordering::SeqCst) == 0 {
                    if let Some(reason) = failed {
                        held = None;
                        let _ = events.send(Event::Failed(reason));
                        continue;
                    }
                    let said = message["result"].as_str().filter(|text| !text.trim().is_empty()).map(str::to_string).or_else(|| last_said.take());
                    if background.is_empty() {
                        held = None;
                        let _ = events.send(Event::Idle(said));
                    } else {
                        held = Some(said);
                        let tasks = background.iter().map(|(_, description)| description.as_str()).filter(|description| !description.is_empty()).collect::<Vec<_>>().join(", ");
                        let line = match background.len() {
                            1 => format!("Waiting on a background task: {tasks}"),
                            count => format!("Waiting on {count} background tasks: {tasks}"),
                        };
                        item(Item::text(uuid::Uuid::new_v4().to_string(), "status", line.trim_end_matches(": ")));
                    }
                }
            }
            // Its questions to the user come as a permission check of `AskUserQuestion`: the user
            // answers on a card, and the answers go back as the tool's input, by question.
            "control_request" if message["request"]["subtype"] == "can_use_tool" && message["request"]["tool_name"] == "AskUserQuestion" => {
                let Some(id) = message["request_id"].as_str().map(str::to_string) else { continue };
                let input = message["request"]["input"].clone();
                let (tx, rx) = oneshot::channel();
                if events.send(Event::Ask(questions(&input), tx)).is_err() {
                    continue;
                }
                let process = process.clone();
                tokio::spawn(async move {
                    let answer = match rx.await.unwrap_or_else(|_| Err("Lorca stopped before the user answered".into())) {
                        Ok(answers) => {
                            let mut input = input;
                            input["answers"] = json!(answers);
                            json!({ "behavior": "allow", "updatedInput": input })
                        }
                        Err(reason) => json!({ "behavior": "deny", "message": reason }),
                    };
                    let _ = process.send(&json!({ "type": "control_response", "response": { "subtype": "success", "request_id": id, "response": answer } })).await;
                });
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
