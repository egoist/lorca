//! `bash`: run a shell command in the working directory, with the login shell's environment
//! ([`crate::login_shell`]). Output is tail-truncated to 2000 lines or 50KB; the full output is
//! saved to a temp file when truncated. Cancellation kills the whole process group (on Windows,
//! the process tree).
//!
//! Built with [`BashTool::new`], it is pi's bash: pipes, nothing on stdin, and the call lasts
//! as long as the command. Its results carry structured output for codemode scripts: the output
//! up to 1 MB, the exit code, and the wall time, also when the command fails. Built with
//! [`BashTool::with_sessions`], a command runs in a terminal of its own that can outlive the
//! call and take input ([`super::bash_session`]).

use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use serde_json::{json, Value};
use tokio::io::AsyncReadExt;
use tokio_util::sync::CancellationToken;

use super::bash_session::{BashSessions, WAITING_AFTER};
use super::truncate::{format_size, truncate_tail, TruncatedBy, TruncationOptions, DEFAULT_MAX_BYTES};
use crate::tool::{Tool, ToolError, ToolResult, ToolUpdateFn};

pub(crate) const UPDATE_THROTTLE_MS: u64 = 250;

/// The most of a command's output a script receives in `output`, as pi's bash gives it.
const SCRIPT_OUTPUT_MAX_BYTES: usize = 1024 * 1024;

const DESCRIPTION: &str = "Execute a bash command in the current working directory. Returns stdout and stderr. Output is truncated to last 2000 lines \
     or 50KB (whichever is hit first). If truncated, full output is saved to a temp file. Optionally provide a timeout in seconds.";

const TERMINAL_DESCRIPTION: &str = "Execute a bash command in the current working directory, in a terminal of its own. Returns its output, stdout and \
     stderr together, with colors and other terminal codes removed. Output is truncated to last 2000 lines or 50KB (whichever is hit \
     first). If truncated, full output is saved to a temp file. Optionally provide a timeout in seconds. A command that stops at \
     what looks like a prompt, or prints nothing for 20 seconds, returns while it still runs, with a session id: it may be waiting \
     for input, such as a password, a yes/no answer, or a key. Answer it with bash_input, or wait for more with bash_output. \
     For a server, a watcher, or a long build, set background: the call returns after 2 seconds with the session id while \
     the command runs on. Prefer this to & or nohup, which leave the command where its session cannot follow or stop it.";

const NO_INPUT_DESCRIPTION: &str = "Execute a bash command in the current working directory. Returns stdout and stderr. Output is truncated to last 2000 \
     lines or 50KB (whichever is hit first). If truncated, full output is saved to a temp file. Optionally provide a timeout in \
     seconds. Commands get no input: stdin is closed, and interactive input is not available on Windows, so pass answers as flags \
     (--yes, -y) or through files.";

const NO_SHELL: &str = "Commands run in Git for Windows' bash, and none was found: no bash.exe in Program Files, Program Files (x86), \
     %LOCALAPPDATA%\\Programs\\Git, or beside a git.exe on PATH. Install Git for Windows from https://git-scm.com/downloads/win, \
     and commands run from the next message on; or set LORCA_SHELL to the full path of a bash.exe and restart Lorca.";

pub struct BashTool {
    cwd: PathBuf,
    /// None on a Windows computer without Git for Windows' bash: every call then fails saying so.
    shell: Option<String>,
    sessions: Option<Arc<dyn BashSessions>>,
    waiting_after: Duration,
    extras: Arc<crate::login_shell::Extras>,
}

impl BashTool {
    pub fn new(cwd: PathBuf) -> Self {
        BashTool { cwd, shell: shell(), sessions: None, waiting_after: WAITING_AFTER, extras: Arc::default() }
    }

    /// Runs each command in a terminal session `sessions` keeps, so a command waiting for input
    /// returns with its session id and can be answered. On Windows commands still run on pipes,
    /// with no input.
    pub fn with_sessions(cwd: PathBuf, sessions: Arc<dyn BashSessions>) -> Self {
        BashTool { sessions: Some(sessions), ..BashTool::new(cwd) }
    }

    /// How long a command may print nothing before its call returns with the session id.
    /// Variables over the login shell's environment for every command, and folders first on
    /// its PATH (`login_shell::Extras`).
    pub fn with_extras(mut self, extras: crate::login_shell::Extras) -> Self {
        self.extras = Arc::new(extras);
        self
    }

    pub fn waiting_after(mut self, after: Duration) -> Self {
        self.waiting_after = after;
        self
    }

    /// Where commands run in terminals: a host that keeps sessions, on Unix.
    fn terminal(&self) -> Option<&Arc<dyn BashSessions>> {
        self.sessions.as_ref().filter(|_| cfg!(unix))
    }
}

/// The shell commands run in: `LORCA_SHELL` when set, else this platform's.
fn shell() -> Option<String> {
    #[cfg(unix)]
    let default = || Some(default_shell());
    #[cfg(windows)]
    let default = default_shell;
    std::env::var("LORCA_SHELL").ok().filter(|s| !s.is_empty()).or_else(default)
}

#[cfg(unix)]
fn default_shell() -> String {
    if std::path::Path::new("/bin/bash").exists() { "/bin/bash".into() } else { "/bin/sh".into() }
}

/// Git for Windows' bash ([`git_bash`]), or none when this computer has no Git.
#[cfg(windows)]
fn default_shell() -> Option<String> {
    git_bash(|name| std::env::var_os(name), Path::is_file).map(|path| path.display().to_string())
}

/// Git for Windows' bash: in `%ProgramFiles%`, `%ProgramFiles(x86)%`, or `%LOCALAPPDATA%\Programs`
/// (a per-user install), else at `..\..\bin\bash.exe` from a `git.exe` on PATH
/// (`…\Git\cmd\git.exe` → `…\Git\bin\bash.exe`), the first PATH entry whose Git has one. Never a
/// bare `bash`: `Command` looks in the system directories before PATH, and System32's `bash.exe`
/// is WSL's, which would run the command in Linux. So PATH is walked here, past the Windows
/// directory (`%SystemRoot%`, System32 included) and relative entries. `var` reads an environment
/// variable and `exists` says whether a file is there, so tests run the search on any platform.
#[cfg(any(windows, test))]
fn git_bash(var: impl Fn(&str) -> Option<std::ffi::OsString>, exists: impl Fn(&Path) -> bool) -> Option<PathBuf> {
    let dir = |name: &str| var(name).filter(|value| !value.is_empty()).map(PathBuf::from);
    let installs = [
        dir("ProgramFiles").map(|dir| dir.join("Git")),
        dir("ProgramFiles(x86)").map(|dir| dir.join("Git")),
        dir("LOCALAPPDATA").map(|dir| dir.join("Programs").join("Git")),
    ];
    let windows = dir("SystemRoot").or_else(|| dir("windir"));
    let path = var("PATH").unwrap_or_default();
    let on_path = std::env::split_paths(&path)
        .filter(|entry| entry.is_absolute() && !windows.as_deref().is_some_and(|windows| within(entry, windows)))
        .filter(|entry| exists(&entry.join("git.exe")))
        .filter_map(|entry| entry.parent().map(Path::to_path_buf));
    installs.into_iter().flatten().chain(on_path).map(|git| git.join("bin").join("bash.exe")).find(|bash| exists(bash))
}

/// Whether `path` lies in `dir`, ignoring case as Windows does (`C:\WINDOWS\system32` is in
/// `C:\Windows`).
#[cfg(any(windows, test))]
fn within(path: &Path, dir: &Path) -> bool {
    let lower = |path: &Path| PathBuf::from(path.to_string_lossy().to_lowercase());
    lower(path).starts_with(lower(dir))
}

/// Kills the process group the shell leads: `process_group(0)` on pipes, `setsid` in a terminal,
/// both make its pid the group's id.
#[cfg(unix)]
pub(crate) fn kill_group(pid: u32) {
    // Negative pid addresses the process group; -0 would be this process's own.
    if pid == 0 {
        return;
    }
    unsafe {
        libc::kill(-(pid as i32), libc::SIGKILL);
    }
}

/// Windows has no process group to signal: `taskkill /T` ends the shell and everything it started.
#[cfg(windows)]
pub(crate) fn kill_group(pid: u32) {
    if pid == 0 {
        return;
    }
    let _ = std::process::Command::new("taskkill")
        .args(["/F", "/T", "/PID", &pid.to_string()])
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status();
}

fn describe(text: &str, truncation: &super::truncate::TruncationResult, full_output_path: Option<&Path>) -> String {
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

/// The output a script receives: all of it up to `max` bytes, else its start and end around a
/// note of what was left out, cut between characters. The second value says whether it was cut.
fn script_output(output: &[u8], max: usize) -> (String, bool) {
    if output.len() <= max {
        return (String::from_utf8_lossy(output).into_owned(), false);
    }
    let continues = |index: usize| output.get(index).is_some_and(|byte| byte & 0xC0 == 0x80);
    let mut head = max / 2;
    while head > 0 && continues(head) {
        head -= 1;
    }
    let mut tail = output.len() - (max - max / 2);
    while continues(tail) {
        tail += 1;
    }
    let text = format!(
        "{}\n\n[... {} bytes omitted ...]\n\n{}",
        String::from_utf8_lossy(&output[..head]),
        tail - head,
        String::from_utf8_lossy(&output[tail..])
    );
    (text, true)
}

/// How a command with no exit code ended: killed by a signal, on Unix.
fn ended_without_code(status: Option<&std::process::ExitStatus>) -> String {
    #[cfg(unix)]
    {
        use std::os::unix::process::ExitStatusExt;
        if let Some(signal) = status.and_then(|status| status.signal()) {
            return format!("Command terminated by signal {signal}");
        }
    }
    let _ = status;
    "Command terminated without an exit code".into()
}

#[async_trait]
impl Tool for BashTool {
    fn name(&self) -> &str {
        "bash"
    }
    fn description(&self) -> &str {
        match (&self.sessions, self.terminal()) {
            (_, Some(_)) => TERMINAL_DESCRIPTION,
            (Some(_), None) => NO_INPUT_DESCRIPTION,
            (None, None) => DESCRIPTION,
        }
    }
    fn parameters(&self) -> Value {
        let mut parameters = json!({
            "type": "object",
            "properties": {
                "command": { "type": "string", "description": "Shell command to execute" },
                "description": {
                    "type": "string",
                    "description": "What the command does, in a few words and the language of the conversation, shown to the user while it runs: \"Install dependencies\", \"Run the tests\""
                },
                "timeout": { "type": "number", "description": "Timeout in seconds (optional, no default timeout)" }
            },
            "required": ["command", "description"]
        });
        if self.terminal().is_some() {
            parameters["properties"]["background"] = json!({
                "type": "boolean",
                "description": "Leave it running: return after 2 seconds with its session id, for a server, a watcher, or a long build (default false)"
            });
        }
        parameters
    }
    /// On pipes, after pi's: a nonzero exit is an error result for the model, and a script still
    /// receives this. A command in a terminal, which can return while it runs, has none.
    fn output_schema(&self) -> Option<Value> {
        self.terminal().is_none().then(|| {
            json!({
                "type": "object",
                "properties": {
                    "output": { "type": "string", "description": "stdout and stderr together, with the middle left out past 1 MB" },
                    "truncated": { "type": "boolean" },
                    "full_output_path": { "type": "string", "description": "The full output, when truncated" },
                    "exit_code": { "type": "number" },
                    "wall_time_seconds": { "type": "number" }
                },
                "required": ["output", "truncated", "exit_code", "wall_time_seconds"]
            })
        })
    }
    async fn execute(&self, id: &str, args: Value, cancel: CancellationToken, on_update: ToolUpdateFn) -> Result<ToolResult, ToolError> {
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
        let shell = self.shell.as_deref().ok_or_else(|| ToolError(NO_SHELL.into()))?;
        if let Some(sessions) = self.terminal() {
            let background = args["background"].as_bool().unwrap_or(false);
            return super::bash_session::run(shell, &command, &self.cwd, timeout, background, &self.extras, sessions, id, self.waiting_after, cancel, on_update).await;
        }

        let mut cmd = crate::login_shell::command(shell).await;
        self.extras.apply(&mut cmd);
        cmd.arg("-c").arg(&command).current_dir(&self.cwd).stdin(Stdio::null()).stdout(Stdio::piped()).stderr(Stdio::piped());
        #[cfg(unix)]
        {
            cmd.process_group(0);
        }
        let started = std::time::Instant::now();
        let mut child = cmd.spawn().map_err(|e| ToolError(format!("Failed to start {shell}: {e}")))?;
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
        let Some(code) = status.as_ref().and_then(|s| s.code()) else {
            return Err(ToolError(with_status(&ended_without_code(status.as_ref()))));
        };
        let (script_text, script_truncated) = script_output(&output, SCRIPT_OUTPUT_MAX_BYTES);
        let mut structured = json!({
            "output": script_text,
            "truncated": script_truncated,
            "exit_code": code,
            "wall_time_seconds": (started.elapsed().as_secs_f64() * 10.0).round() / 10.0,
        });
        if let Some(path) = full_output_path.as_ref().filter(|_| script_truncated) {
            structured["full_output_path"] = json!(path);
        }
        let details = json!({
            "summary": format!("$ {}", command.lines().next().unwrap_or("").chars().take(60).collect::<String>()),
            "exit_code": code,
            "truncation": if truncation.truncated { serde_json::to_value(&truncation).unwrap_or(Value::Null) } else { Value::Null },
            "full_output_path": full_output_path,
        });
        let result = if code != 0 {
            ToolResult { is_error: true, ..ToolResult::text(with_status(&format!("Command exited with code {code}"))) }
        } else {
            ToolResult::text(if shown.is_empty() { "(no output)".to_string() } else { shown })
        };
        Ok(ToolResult { structured: Some(structured), ..result.with_details(details) })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;

    #[tokio::test]
    async fn a_host_puts_its_own_command_first_and_its_variables_in() {
        let dir = std::env::temp_dir().join(format!("lorca-bash-extras-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let script = dir.join("lorca");
        std::fs::write(&script, "#!/bin/sh\necho \"this Runner's lorca on $LORCA_PORT\"\n").unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&script, std::fs::Permissions::from_mode(0o755)).unwrap();
        }
        let extras = crate::login_shell::Extras { variables: vec![("LORCA_PORT".into(), "4899".into())], path_first: vec![dir.clone()] };
        let tool = BashTool::new(std::env::temp_dir()).with_extras(extras);
        let result = tool.execute("1", json!({ "command": "lorca" }), CancellationToken::new(), Arc::new(|_| {})).await.unwrap();
        let text = result.text_content();
        assert!(text.contains("this Runner's lorca on 4899"), "{text}");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[tokio::test]
    async fn runs_and_reports_exit_code() {
        let tool = BashTool::new(std::env::temp_dir());
        let ok = tool.execute("1", json!({"command": "printf 'hi\\nthere'"}), CancellationToken::new(), Arc::new(|_| {})).await.unwrap();
        assert_eq!(ok.text_content(), "hi\nthere");
        assert!(!ok.is_error);
        let failed = tool.execute("2", json!({"command": "echo boom >&2; exit 3"}), CancellationToken::new(), Arc::new(|_| {})).await.unwrap();
        let text = failed.text_content();
        assert!(failed.is_error && text.contains("boom") && text.ends_with("Command exited with code 3"), "{text}");
        let timeout = tool.execute("3", json!({"command": "sleep 5", "timeout": 0.2}), CancellationToken::new(), Arc::new(|_| {})).await.unwrap_err();
        assert!(timeout.0.contains("timed out"));
    }

    /// What a codemode script receives: the output whole, the exit code, and the time, for a
    /// command that failed too.
    #[tokio::test]
    async fn gives_scripts_structured_output() {
        let tool = BashTool::new(std::env::temp_dir());
        assert!(tool.output_schema().is_some());
        let ok = tool.execute("1", json!({"command": "printf 'hi\\nthere'"}), CancellationToken::new(), Arc::new(|_| {})).await.unwrap();
        let structured = ok.structured.unwrap();
        assert_eq!((&structured["output"], &structured["truncated"], &structured["exit_code"]), (&json!("hi\nthere"), &json!(false), &json!(0)));
        assert!(structured["wall_time_seconds"].is_number() && structured.get("full_output_path").is_none(), "{structured}");

        let failed = tool.execute("2", json!({"command": "echo boom; exit 3"}), CancellationToken::new(), Arc::new(|_| {})).await.unwrap();
        let structured = failed.structured.unwrap();
        assert_eq!((&structured["output"], &structured["exit_code"]), (&json!("boom\n"), &json!(3)));

        // Past what the model reads, a script still gets the whole output.
        let long = tool.execute("3", json!({"command": "seq 1 5000"}), CancellationToken::new(), Arc::new(|_| {})).await.unwrap();
        assert!(long.text_content().contains("[Showing lines"), "the model's copy is cut");
        let output = long.structured.unwrap()["output"].as_str().unwrap().to_string();
        assert!(output.starts_with("1\n2\n") && output.ends_with("4999\n5000\n"), "{}", &output[..20]);
    }

    #[test]
    fn script_output_keeps_the_start_and_end_past_its_limit() {
        assert_eq!(script_output(b"short", 10), ("short".to_string(), false));
        let (text, truncated) = script_output("aaaaé€bbbbb".as_bytes(), 8);
        assert!(truncated);
        assert_eq!(text, "aaaa\n\n[... 6 bytes omitted ...]\n\nbbbb");
        // A cut inside a character moves to its edge instead of splitting it.
        let (text, _) = script_output("aaé€€bb".as_bytes(), 6);
        assert_eq!(text, "aa\n\n[... 8 bytes omitted ...]\n\nbb");
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn a_command_killed_by_a_signal_fails() {
        let tool = BashTool::new(std::env::temp_dir());
        let err = tool.execute("1", json!({"command": "echo before; kill -9 $$"}), CancellationToken::new(), Arc::new(|_| {})).await.unwrap_err();
        assert!(err.0.contains("before") && err.0.ends_with("Command terminated by signal 9"), "{}", err.0);
    }

    /// A drive root on Windows, `/` elsewhere, so the fixture paths are absolute where the test runs.
    fn drive() -> PathBuf {
        PathBuf::from(if cfg!(windows) { "C:\\" } else { "/" })
    }

    fn join(base: &Path, parts: &[&str]) -> PathBuf {
        parts.iter().fold(base.to_path_buf(), |path, part| path.join(part))
    }

    fn bash_in(git: &Path) -> PathBuf {
        join(git, &["bin", "bash.exe"])
    }

    /// Runs [`git_bash`] on a computer with these variables, these PATH entries, and only these files.
    fn find(vars: &[(&str, PathBuf)], path: &[PathBuf], files: &[PathBuf]) -> Option<PathBuf> {
        let path = std::env::join_paths(path).unwrap();
        git_bash(
            |name| match name {
                "PATH" => Some(path.clone()),
                _ => vars.iter().find(|(var, _)| *var == name).map(|(_, value)| value.clone().into_os_string()),
            },
            |file| files.iter().any(|f| f == file),
        )
    }

    #[test]
    fn git_bash_looks_in_install_folders_then_path() {
        let c = drive();
        let local = join(&c, &["Users", "me", "AppData", "Local"]);
        let custom = join(&c, &["Tools", "Git"]);
        let vars = [
            ("ProgramFiles", c.join("Program Files")),
            ("ProgramFiles(x86)", c.join("Program Files (x86)")),
            ("LOCALAPPDATA", local.clone()),
            ("SystemRoot", c.join("Windows")),
        ];
        let path = [join(&custom, &["cmd"])];
        let git = join(&custom, &["cmd", "git.exe"]);
        let in_order = [
            bash_in(&join(&c, &["Program Files", "Git"])),
            bash_in(&join(&c, &["Program Files (x86)", "Git"])),
            bash_in(&join(&local, &["Programs", "Git"])),
            bash_in(&custom),
        ];
        for (i, expected) in in_order.iter().enumerate() {
            let mut files = in_order[i..].to_vec();
            files.push(git.clone());
            assert_eq!(find(&vars, &path, &files).as_ref(), Some(expected));
        }
        assert_eq!(find(&vars, &path, &[git]), None);
        assert_eq!(find(&[], &[], &[]), None);
    }

    #[test]
    fn git_bash_derives_bash_from_git_on_path() {
        let c = drive();
        let shims = join(&c, &["Users", "me", "scoop", "shims"]);
        let portable = join(&c, &["Users", "me", "PortableGit"]);
        let bare = join(&c, &["msys64", "usr"]);
        let path = [shims.clone(), bare.clone(), join(&portable, &["cmd"])];
        let files = [
            // A shim's `..\bin\bash.exe` is not there, so the next Git on PATH is used.
            shims.join("git.exe"),
            // A bash with no git.exe beside it is not Git for Windows'.
            bash_in(&bare),
            join(&portable, &["cmd", "git.exe"]),
            bash_in(&portable),
        ];
        assert_eq!(find(&[], &path, &files), Some(bash_in(&portable)));
        // Git's own `bin` on PATH also leads to `bin\bash.exe`.
        assert_eq!(find(&[], &[portable.join("bin")], &[join(&portable, &["bin", "git.exe"]), bash_in(&portable)]), Some(bash_in(&portable)));
        assert_eq!(find(&[], &[PathBuf::from("relative").join("cmd")], &[join(Path::new("relative"), &["cmd", "git.exe"]), bash_in(Path::new("relative"))]), None);
    }

    #[test]
    fn git_bash_skips_the_windows_directory_on_path() {
        let c = drive();
        let system32 = join(&c, &["WINDOWS", "system32"]);
        let git = join(&c, &["Program Files", "Git"]);
        let vars = [("SystemRoot", c.join("Windows"))];
        // WSL's launcher, and a git.exe whose `..\..\bin\bash.exe` would be C:\WINDOWS\bin\bash.exe.
        let files = vec![system32.join("bash.exe"), system32.join("git.exe"), bash_in(&c.join("WINDOWS"))];
        assert_eq!(find(&vars, std::slice::from_ref(&system32), &files), None);
        assert_eq!(find(&[("windir", c.join("Windows"))], std::slice::from_ref(&system32), &files), None);

        let mut files = files;
        files.extend([join(&git, &["cmd", "git.exe"]), bash_in(&git)]);
        assert_eq!(find(&vars, &[system32, join(&git, &["cmd"])], &files), Some(bash_in(&git)));
    }

    /// The search on an environment as Windows writes it: drive letters, a PATH split on `;` with
    /// an empty, a relative, and a quoted entry, and System32 in another case than `%SystemRoot%`.
    #[cfg(windows)]
    #[test]
    fn git_bash_reads_a_windows_environment() {
        let path = r#"C:\WINDOWS\system32;C:\WINDOWS;;relative\cmd;"D:\Tools\Git\cmd";C:\Users\me\scoop\shims"#;
        let vars = [
            ("ProgramFiles", r"C:\Program Files"),
            ("ProgramFiles(x86)", r"C:\Program Files (x86)"),
            ("LOCALAPPDATA", r"C:\Users\me\AppData\Local"),
            ("SystemRoot", r"C:\Windows"),
            ("PATH", path),
        ];
        let search = |files: &[&str]| {
            git_bash(
                |name| vars.iter().find(|(var, _)| *var == name).map(|(_, value)| std::ffi::OsString::from(value)),
                |file| files.iter().any(|f| Path::new(f) == file),
            )
        };
        let on_path = [
            r"C:\WINDOWS\system32\bash.exe",
            r"C:\WINDOWS\system32\git.exe",
            r"C:\WINDOWS\bin\bash.exe",
            r"relative\cmd\git.exe",
            r"relative\bin\bash.exe",
            r"D:\Tools\Git\cmd\git.exe",
            r"D:\Tools\Git\bin\bash.exe",
            r"C:\Users\me\scoop\shims\git.exe",
        ];
        assert_eq!(search(&on_path), Some(PathBuf::from(r"D:\Tools\Git\bin\bash.exe")));

        let mut with_user_install = on_path.to_vec();
        with_user_install.push(r"C:\Users\me\AppData\Local\Programs\Git\bin\bash.exe");
        assert_eq!(search(&with_user_install), Some(PathBuf::from(r"C:\Users\me\AppData\Local\Programs\Git\bin\bash.exe")));

        with_user_install.push(r"C:\Program Files\Git\bin\bash.exe");
        assert_eq!(search(&with_user_install), Some(PathBuf::from(r"C:\Program Files\Git\bin\bash.exe")));
    }
}
