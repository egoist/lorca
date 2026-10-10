//! Claude Code without a terminal: `claude -p` with stream-json in and out, the protocol its
//! Agent SDK speaks. Lorca sends the bot's messages as user messages, reads the transcript as
//! it streams, and decides what Claude Code asks to do: every Bash call through a `PreToolUse`
//! hook, which runs whatever the user's own Claude Code settings allow, and any other tool that
//! would ask through the permission prompt (`--permission-prompt-tool stdio`). Edits in the
//! folder it works in run on their own (`acceptEdits`).

use std::path::Path;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;

use async_trait::async_trait;
use serde_json::{json, Value};
use tokio::sync::oneshot;

use super::driver::{Approval, Driver, Ended, Event, Events};
use super::lines;
use super::process::{self, Process};

/// The callback the Bash hook calls back with.
const BASH_HOOK: &str = "lorca-bash";

pub(crate) struct Claude {
    process: Arc<Process>,
    /// Messages sent that it has not taken up yet. It takes one up when it says so
    /// (`--replay-user-messages`): between turns, or at a step of the turn it is on.
    pending: Arc<AtomicUsize>,
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

async fn read(
    process: Arc<Process>,
    mut messages: tokio::sync::mpsc::UnboundedReceiver<Value>,
    exit: oneshot::Receiver<process::Exit>,
    events: Events,
    pending: Arc<AtomicUsize>,
    folder: std::path::PathBuf,
) {
    let mut session_told = false;
    let mut last_said: Option<String> = None;
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
                let mut out = Vec::new();
                for part in message["message"]["content"].as_array().into_iter().flatten() {
                    match part["type"].as_str() {
                        Some("text") => {
                            let text = part["text"].as_str().unwrap_or("");
                            if !text.trim().is_empty() {
                                last_said = Some(text.trim().to_string());
                            }
                            out.extend(lines::said(text));
                        }
                        Some("tool_use") => {
                            let name = part["name"].as_str().unwrap_or("Tool");
                            out.push(lines::call(name, &lines::claude_detail(name, &part["input"], &folder)));
                        }
                        _ => {}
                    }
                }
                if !out.is_empty() {
                    let _ = events.send(Event::Lines(out));
                }
            }
            "user" => {
                if !message["parent_tool_use_id"].is_null() {
                    continue;
                }
                let content = &message["message"]["content"];
                // One of the messages Lorca sent, as it takes it up.
                if message["isReplay"].as_bool().unwrap_or(false) || content.is_string() {
                    let text = lines::content_text(content);
                    if text.starts_with("[Request interrupted") {
                        let _ = events.send(Event::Lines(vec!["  ⎿ Interrupted".into()]));
                        continue;
                    }
                    let _ = pending.fetch_update(Ordering::SeqCst, Ordering::SeqCst, |count| Some(count.saturating_sub(1)));
                    let _ = events.send(Event::Lines(lines::sent(&text)));
                    let _ = events.send(Event::Working);
                    continue;
                }
                let mut out = Vec::new();
                for part in content.as_array().into_iter().flatten() {
                    if part["type"] == "tool_result" {
                        out.extend(lines::result(&lines::content_text(&part["content"]), part["is_error"].as_bool().unwrap_or(false)));
                    } else if part["text"].as_str().is_some_and(|text| text.starts_with("[Request interrupted")) {
                        out.push("  ⎿ Interrupted".into());
                    }
                }
                if !out.is_empty() {
                    let _ = events.send(Event::Lines(out));
                }
            }
            // The end of a turn. With a message still to take up, it goes on with that one.
            "result" => {
                if message["is_error"].as_bool().unwrap_or(false) && message["subtype"] != "error_during_execution" {
                    let reason = message["result"].as_str().or(message["subtype"].as_str()).unwrap_or("error");
                    let _ = events.send(Event::Lines(vec![format!("Error: {}", reason.lines().next().unwrap_or(reason))]));
                }
                if pending.load(Ordering::SeqCst) == 0 {
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
                        (input["tool_name"] == "Bash").then_some((Approval::Command { command }, true))
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
                        Some((approval, false))
                    }
                    _ => None,
                };
                let Some((approval, hook)) = approval else {
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
                tokio::spawn(async move {
                    let verdict = rx.await.unwrap_or_else(|_| Err("Lorca stopped before deciding".into()));
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
    let exit = exit.await.ok();
    let ended = match exit {
        Some(process::Exit { stopped: true, .. }) => Ended { outcome: "Stopped".into(), failed: false },
        Some(process::Exit { code: Some(0), .. }) => Ended { outcome: "Claude Code exited".into(), failed: false },
        Some(process::Exit { code, said, .. }) => {
            let code = code.map(|code| format!(" with code {code}")).unwrap_or_default();
            Ended { outcome: format!("Claude Code exited{code}{}", said.map(|said| format!(": {said}")).unwrap_or_default()), failed: true }
        }
        None => Ended { outcome: "Claude Code exited".into(), failed: true },
    };
    let _ = events.send(Event::Ended(ended));
}
