//! A headless coding agent's process, spoken to with one JSON value per line on its stdin and
//! stdout wherever it runs ([`Host`]): on this Runner, started with the login shell's environment
//! in a process group of its own (a job object on Windows), so Stop ends what it started too; or
//! in a cloud sandbox the user set up (`super::sandbox`), whose streams come over the network.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use process_wrap::tokio::{ChildWrapper, CommandWrap};
use serde_json::Value;
use tokio::io::{AsyncBufReadExt, AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt, BufReader};
use tokio::sync::{oneshot, Mutex};
use tokio_util::sync::CancellationToken;

/// What a Claude Code session started from another one would inherit: they mark the parent's
/// session, which the agent Lorca starts is not part of.
const INHERITED_MARKERS: [&str; 10] = [
    "CLAUDECODE",
    "CLAUDE_PID",
    "CLAUDE_EFFORT",
    "CLAUDE_CODE_ENTRYPOINT",
    "CLAUDE_CODE_EXECPATH",
    "CLAUDE_CODE_SESSION_ID",
    "CLAUDE_CODE_CHILD_SESSION",
    "CLAUDE_CODE_SESSION_ATTENDED",
    "CLAUDE_CODE_MESSAGING_SOCKET",
    "CLAUDE_CODE_MESSAGING_TOKEN",
];

/// How a process ended: its exit code, the last thing it said on stderr, and whether Lorca
/// stopped it.
#[derive(Debug, Clone)]
pub(crate) struct Exit {
    pub code: Option<i32>,
    pub said: Option<String>,
    pub stopped: bool,
}

/// What a process prints, line by line.
pub(crate) type Stdout = Box<dyn AsyncRead + Send + Unpin>;
type Stdin = Box<dyn AsyncWrite + Send + Unpin>;

/// Where an agent's process runs.
#[derive(Clone)]
pub(crate) enum Host {
    /// This Runner.
    Local,
    /// A cloud sandbox the user set up, with the repository cloned into it.
    Sandbox(Arc<super::sandbox::Sandbox>),
}

pub(crate) struct Process {
    stdin: Mutex<Option<Stdin>>,
    kill: CancellationToken,
}

impl Process {
    /// A process whose stdin is `stdin`, and which ends when `kill` is cancelled.
    pub fn new(stdin: impl AsyncWrite + Send + Unpin + 'static, kill: CancellationToken) -> Arc<Process> {
        Arc::new(Process { stdin: Mutex::new(Some(Box::new(stdin))), kill })
    }

    /// Writes one JSON value and a line break.
    pub async fn send(&self, value: &Value) -> Result<(), String> {
        let mut line = serde_json::to_vec(value).map_err(|e| e.to_string())?;
        line.push(b'\n');
        let mut stdin = self.stdin.lock().await;
        let pipe = stdin.as_mut().ok_or("It is no longer running")?;
        pipe.write_all(&line).await.map_err(|e| format!("It is no longer running: {e}"))?;
        pipe.flush().await.map_err(|e| format!("It is no longer running: {e}"))
    }

    /// Ends the process and everything it started.
    pub fn stop(&self) {
        self.kill.cancel();
    }
}

/// An environment variable the agent is given: a saved secret the bot named.
pub(crate) type Variable = (std::ffi::OsString, std::ffi::OsString);

/// Starts `program` in `cwd` on `host`. Its stdout goes to the caller line by line; how it ended
/// arrives once it has.
pub(crate) async fn spawn(host: &Host, program: &Path, args: &[String], cwd: &Path, variables: &[Variable]) -> Result<(Arc<Process>, Stdout, oneshot::Receiver<Exit>), String> {
    match host {
        Host::Local => spawn_here(program, args, cwd, variables).await,
        Host::Sandbox(sandbox) => sandbox.spawn(program, args, cwd, variables).await,
    }
}

async fn spawn_here(program: &Path, args: &[String], cwd: &Path, variables: &[Variable]) -> Result<(Arc<Process>, Stdout, oneshot::Receiver<Exit>), String> {
    let mut command = lorca_agent::login_shell::command(program).await;
    command.args(args).current_dir(cwd).stdin(std::process::Stdio::piped()).stdout(std::process::Stdio::piped()).stderr(std::process::Stdio::piped());
    for name in INHERITED_MARKERS {
        command.env_remove(name);
    }
    command.envs(variables.iter().map(|(name, value)| (name, value)));
    let mut wrapped = CommandWrap::from(command);
    #[cfg(unix)]
    wrapped.wrap(process_wrap::tokio::ProcessGroup::leader());
    #[cfg(windows)]
    wrapped.wrap(process_wrap::tokio::JobObject);
    wrapped.wrap(process_wrap::tokio::KillOnDrop);
    let name = program.file_name().map(|name| name.to_string_lossy().to_string()).unwrap_or_default();
    let mut child: Box<dyn ChildWrapper> = wrapped.spawn().map_err(|e| format!("Cannot start {name}: {e}"))?;
    let stdin = child.stdin().take().ok_or("No stdin")?;
    let stdout = child.stdout().take().ok_or("No stdout")?;
    let stderr = child.stderr().take();
    let kill = CancellationToken::new();
    let (tx, rx) = oneshot::channel();
    let token = kill.clone();
    tokio::spawn(async move {
        // The last lines it wrote to stderr usually say why it stopped.
        let said = tokio::spawn(async move {
            let mut tail = String::new();
            if let Some(mut stderr) = stderr {
                let mut buffer = vec![0u8; 8192];
                while let Ok(read) = stderr.read(&mut buffer).await {
                    if read == 0 {
                        break;
                    }
                    tail.push_str(&String::from_utf8_lossy(&buffer[..read]));
                    if tail.len() > 16_384 {
                        tail = tail.split_off(tail.len() - 8192);
                    }
                }
            }
            tail.lines().map(str::trim).rfind(|line| !line.is_empty()).map(|line| line.chars().take(300).collect::<String>())
        });
        let (status, stopped) = tokio::select! {
            status = child.wait() => (status.ok(), false),
            _ = token.cancelled() => {
                let _ = child.start_kill();
                (child.wait().await.ok(), true)
            }
        };
        let said = tokio::time::timeout(std::time::Duration::from_secs(1), said).await.ok().and_then(Result::ok).flatten();
        let _ = tx.send(Exit { code: status.and_then(|status| status.code()), said, stopped });
    });
    Ok((Process::new(stdin, kill), Box::new(stdout), rx))
}

/// The lines of `stdout`, as JSON values; a line that is not JSON is skipped.
pub(crate) fn json_lines(stdout: Stdout) -> tokio::sync::mpsc::UnboundedReceiver<Value> {
    let (tx, rx) = tokio::sync::mpsc::unbounded_channel();
    tokio::spawn(async move {
        let mut lines = BufReader::new(stdout).lines();
        while let Ok(Some(line)) = lines.next_line().await {
            let line = line.trim();
            if line.is_empty() {
                continue;
            }
            match serde_json::from_str::<Value>(line) {
                Ok(value) => {
                    if tx.send(value).is_err() {
                        break;
                    }
                }
                Err(_) => tracing::debug!(line = %line.chars().take(200).collect::<String>(), "a coding agent printed a line that is not JSON"),
            }
        }
    });
    rx
}

/// Where `name` is installed, on the login shell's `PATH`: what a terminal would run.
pub(crate) async fn find(name: &str) -> Option<PathBuf> {
    #[cfg(test)]
    if let Some(path) = tests_support::overridden(name) {
        return Some(path);
    }
    let environment = lorca_agent::login_shell::environment().await;
    let path = environment.iter().rev().find(|(key, _)| key.to_str().is_some_and(|key| key.eq_ignore_ascii_case("PATH"))).map(|(_, value)| value.clone()).or_else(|| std::env::var_os("PATH"))?;
    let names: Vec<String> = if cfg!(windows) { [".exe", ".cmd", ".bat"].iter().map(|extension| format!("{name}{extension}")).collect() } else { vec![name.to_string()] };
    std::env::split_paths(&path).filter(|folder| folder.is_absolute()).flat_map(|folder| names.iter().map(move |file| folder.join(file))).find(|file| is_program(file))
}

fn is_program(path: &Path) -> bool {
    let Ok(metadata) = std::fs::metadata(path) else { return false };
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        metadata.is_file() && metadata.permissions().mode() & 0o111 != 0
    }
    #[cfg(not(unix))]
    metadata.is_file()
}

#[cfg(test)]
pub(crate) mod tests_support {
    use std::path::PathBuf;
    use std::sync::Mutex;

    static PROGRAMS: Mutex<Vec<(String, PathBuf)>> = Mutex::new(Vec::new());

    /// Has [`super::find`] answer `path` for `name`, as if it were installed.
    pub fn install(name: &str, path: PathBuf) {
        let mut programs = PROGRAMS.lock().unwrap();
        programs.retain(|(known, _)| known != name);
        programs.push((name.to_string(), path));
    }

    /// Has [`super::find`] look on `PATH` for `name` again.
    pub fn uninstall(name: &str) {
        PROGRAMS.lock().unwrap().retain(|(known, _)| known != name);
    }

    pub fn overridden(name: &str) -> Option<PathBuf> {
        PROGRAMS.lock().unwrap().iter().find(|(known, _)| known == name).map(|(_, path)| path.clone())
    }
}
