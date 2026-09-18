//! `bash`: run a shell command in the working directory. Output is tail-truncated to 2000 lines
//! or 50KB; the full output is saved to a temp file when truncated. Cancellation kills the
//! whole process group.

use std::path::PathBuf;
use std::process::Stdio;

use async_trait::async_trait;
use serde_json::{json, Value};
use tokio::io::AsyncReadExt;
use tokio_util::sync::CancellationToken;

use super::truncate::{format_size, truncate_tail, TruncatedBy, TruncationOptions, DEFAULT_MAX_BYTES};
use crate::tool::{Tool, ToolError, ToolResult, ToolUpdateFn};

const UPDATE_THROTTLE_MS: u64 = 250;

pub struct BashTool {
    cwd: PathBuf,
    shell: String,
}

impl BashTool {
    pub fn new(cwd: PathBuf) -> Self {
        let shell = std::env::var("LORCA_SHELL").ok().filter(|s| !s.is_empty()).unwrap_or_else(|| {
            if std::path::Path::new("/bin/bash").exists() { "/bin/bash".into() } else { "/bin/sh".into() }
        });
        BashTool { cwd, shell }
    }
}

fn kill_group(pid: u32) {
    // Negative pid addresses the process group the shell started with `process_group(0)`.
    unsafe {
        libc::kill(-(pid as i32), libc::SIGKILL);
    }
}

fn describe(text: &str, truncation: &super::truncate::TruncationResult, full_output_path: Option<&std::path::Path>) -> String {
    let mut out = text.to_string();
    if truncation.truncated {
        let start = truncation.total_lines - truncation.output_lines + 1;
        let end = truncation.total_lines;
        let full = full_output_path.map(|p| p.display().to_string()).unwrap_or_default();
        if truncation.last_line_partial {
            out.push_str(&format!("\n\n[Showing last {} of line {end}. Full output: {full}]", format_size(truncation.output_bytes)));
        } else if truncation.truncated_by == Some(TruncatedBy::Lines) {
            out.push_str(&format!("\n\n[Showing lines {start}-{end} of {}. Full output: {full}]", truncation.total_lines));
        } else {
            out.push_str(&format!(
                "\n\n[Showing lines {start}-{end} of {} ({} limit). Full output: {full}]",
                truncation.total_lines,
                format_size(DEFAULT_MAX_BYTES)
            ));
        }
    }
    out
}

#[async_trait]
impl Tool for BashTool {
    fn name(&self) -> &str {
        "bash"
    }
    fn description(&self) -> &str {
        "Execute a bash command in the current working directory. Returns stdout and stderr. Output is truncated to last 2000 lines \
         or 50KB (whichever is hit first). If truncated, full output is saved to a temp file. Optionally provide a timeout in seconds."
    }
    fn parameters(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "command": { "type": "string", "description": "Shell command to execute" },
                "timeout": { "type": "number", "description": "Timeout in seconds (optional, no default timeout)" }
            },
            "required": ["command"]
        })
    }
    async fn execute(&self, _id: &str, args: Value, cancel: CancellationToken, on_update: ToolUpdateFn) -> Result<ToolResult, ToolError> {
        let command = args["command"].as_str().ok_or("command is required")?.to_string();
        let timeout = args["timeout"].as_f64();
        if let Some(t) = timeout {
            if !t.is_finite() || t <= 0.0 {
                return Err("Invalid timeout: must be a finite number of seconds".into());
            }
        }
        if !self.cwd.exists() {
            return Err(ToolError(format!("Working directory does not exist: {}\nCannot execute bash commands.", self.cwd.display())));
        }

        let mut cmd = tokio::process::Command::new(&self.shell);
        cmd.arg("-c").arg(&command).current_dir(&self.cwd).stdin(Stdio::null()).stdout(Stdio::piped()).stderr(Stdio::piped());
        #[cfg(unix)]
        {
            cmd.process_group(0);
        }
        let mut child = cmd.spawn().map_err(|e| ToolError(format!("Failed to start {}: {e}", self.shell)))?;
        let pid = child.id().unwrap_or(0);

        let mut stdout = child.stdout.take();
        let mut stderr = child.stderr.take();
        let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel::<Vec<u8>>();
        let mut readers = Vec::new();
        for reader in [stdout.take().map(|s| Box::pin(s) as std::pin::Pin<Box<dyn tokio::io::AsyncRead + Send>>), stderr.take().map(|s| Box::pin(s) as _)].into_iter().flatten() {
            let tx = tx.clone();
            readers.push(tokio::spawn(async move {
                let mut reader = reader;
                let mut buf = vec![0u8; 8192];
                loop {
                    match reader.read(&mut buf).await {
                        Ok(0) | Err(_) => break,
                        Ok(n) => {
                            if tx.send(buf[..n].to_vec()).is_err() {
                                break;
                            }
                        }
                    }
                }
            }));
        }
        drop(tx);

        let mut output: Vec<u8> = Vec::new();
        let mut last_update = tokio::time::Instant::now() - std::time::Duration::from_secs(1);
        let deadline = timeout.map(|t| tokio::time::Instant::now() + std::time::Duration::from_secs_f64(t));
        let mut timed_out = false;
        let mut aborted = false;

        let status = loop {
            let sleep_until = deadline.unwrap_or_else(|| tokio::time::Instant::now() + std::time::Duration::from_secs(3600));
            tokio::select! {
                chunk = rx.recv() => {
                    match chunk {
                        Some(bytes) => {
                            output.extend_from_slice(&bytes);
                            if last_update.elapsed() >= std::time::Duration::from_millis(UPDATE_THROTTLE_MS) {
                                last_update = tokio::time::Instant::now();
                                let text = String::from_utf8_lossy(&output);
                                let snapshot = truncate_tail(&text, TruncationOptions::default());
                                on_update(ToolResult::text(snapshot.content));
                            }
                        }
                        None => break child.wait().await.ok(),
                    }
                }
                _ = cancel.cancelled() => { aborted = true; kill_group(pid); let _ = child.kill().await; break child.wait().await.ok(); }
                _ = tokio::time::sleep_until(sleep_until), if deadline.is_some() => { timed_out = true; kill_group(pid); let _ = child.kill().await; break child.wait().await.ok(); }
            }
        };
        for reader in readers {
            let _ = reader.await;
        }
        while let Ok(bytes) = rx.try_recv() {
            output.extend_from_slice(&bytes);
        }

        let text = String::from_utf8_lossy(&output).into_owned();
        let truncation = truncate_tail(&text, TruncationOptions::default());
        let full_output_path = if truncation.truncated {
            let path = std::env::temp_dir().join(format!("lorca-bash-{}-{}.log", std::process::id(), crate::now_ms()));
            let _ = std::fs::write(&path, &output);
            Some(path)
        } else {
            None
        };
        let shown = describe(&truncation.content, &truncation, full_output_path.as_deref());
        let with_status = |status: &str| if shown.is_empty() { status.to_string() } else { format!("{shown}\n\n{status}") };

        if aborted {
            return Err(ToolError(with_status("Command aborted")));
        }
        if timed_out {
            return Err(ToolError(with_status(&format!("Command timed out after {} seconds", timeout.unwrap_or(0.0)))));
        }
        let code = status.and_then(|s| s.code());
        if let Some(code) = code.filter(|c| *c != 0) {
            return Err(ToolError(with_status(&format!("Command exited with code {code}"))));
        }
        let body = if shown.is_empty() { "(no output)".to_string() } else { shown };
        let details = json!({
            "summary": format!("$ {}", command.lines().next().unwrap_or("").chars().take(60).collect::<String>()),
            "exit_code": code,
            "truncation": if truncation.truncated { serde_json::to_value(&truncation).unwrap_or(Value::Null) } else { Value::Null },
            "full_output_path": full_output_path,
        });
        Ok(ToolResult::text(body).with_details(details))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;

    #[tokio::test]
    async fn runs_and_reports_exit_code() {
        let tool = BashTool::new(std::env::temp_dir());
        let ok = tool.execute("1", json!({"command": "printf 'hi\\nthere'"}), CancellationToken::new(), Arc::new(|_| {})).await.unwrap();
        assert_eq!(ok.text_content(), "hi\nthere");
        let err = tool.execute("2", json!({"command": "echo boom >&2; exit 3"}), CancellationToken::new(), Arc::new(|_| {})).await.unwrap_err();
        assert!(err.0.contains("boom") && err.0.contains("exited with code 3"), "{}", err.0);
        let timeout = tool.execute("3", json!({"command": "sleep 5", "timeout": 0.2}), CancellationToken::new(), Arc::new(|_| {})).await.unwrap_err();
        assert!(timeout.0.contains("timed out"));
    }
}
