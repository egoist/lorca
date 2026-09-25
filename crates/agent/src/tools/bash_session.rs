//! Terminal sessions for `bash` on a host that keeps them ([`BashSessions`]), and the two tools
//! that reach a session after the call that started it returned: `bash_input` and
//! `bash_output`.
//!
//! This is where Lorca parts from pi's bash, which runs commands on pipes with nothing on stdin
//! and fails fast, because the person is at the terminal and answers there. A Lorca bot's
//! Runner is often a machine nobody is sitting at, so a command gets a pseudo-terminal of its
//! own: `sudo`, `ssh`, and `getpass` prompt on it, a `[Y/n]` waits on it, and the answer comes
//! from the model (`bash_input`) or from the user over the chat. A command that goes quiet
//! does not hold the turn: its call returns with the session id, and the session lives on with
//! the host, which decides when it ends.
//!
//! The pty is a Unix thing. On Windows `bash` keeps pi's pipes and no session ever starts.

#![cfg_attr(not(unix), allow(dead_code))]

use std::path::{Path, PathBuf};
use std::sync::{Arc, LazyLock, Mutex};
use std::time::Duration;

use async_trait::async_trait;
use regex::Regex;
use serde_json::{json, Value};
use tokio::sync::watch;
use tokio::time::Instant;
use tokio_util::sync::CancellationToken;

use super::sanitize::terminal_text;
use super::truncate::{format_size, truncate_tail, TruncatedBy, TruncationOptions, TruncationResult, DEFAULT_MAX_BYTES};
use crate::tool::{Tool, ToolError, ToolResult, ToolUpdateFn};

/// How long a running command may print nothing before its call returns with the session id.
/// A command blocked on something the terminal cannot show (a password prompt on
/// `/dev/tty` with echo off prints before it waits; a macOS permission dialog prints nothing)
/// would otherwise hold the turn with no sign of why.
pub const WAITING_AFTER: Duration = Duration::from_secs(20);
/// How long a command whose last line reads like a prompt ("Password:", "[Y/n]") may be quiet
/// before its call returns, so the question reaches the model and the user at once.
pub const PROMPT_QUIET: Duration = Duration::from_secs(2);
/// The longest `bash_output` waits in one call: a wait holds the turn, and with it the user's
/// next message.
pub const MAX_OUTPUT_WAIT: Duration = Duration::from_secs(300);
/// Raw output a session keeps in memory, its tail. What came before is in the spill file,
/// which starts once the output outgrows what a result shows.
const MEMORY_BYTES: usize = 4 * DEFAULT_MAX_BYTES;
/// Where the spill file stops growing. A command can keep printing long after its call
/// returned, with nobody reading it, on a Runner nobody watches; this keeps it from filling the
/// disk.
const SPILL_MAX_BYTES: u64 = 64 * 1024 * 1024;
/// The terminal's size. Programs that draw to the width wrap there.
const COLUMNS: u16 = 80;
const ROWS: u16 = 24;
/// After the shell exits, the terminal is read until it goes this quiet (or closes), for the
/// last output still on its way, and for at most `DRAIN_MAX`.
const DRAIN_QUIET: Duration = Duration::from_millis(100);
const DRAIN_MAX: Duration = Duration::from_secs(1);
/// How long typing into a command may wait for it to take the input.
const WRITE_TIMEOUT: Duration = Duration::from_secs(5);
/// How long a prompt hint may be, for the row that shows it.
const PROMPT_CHARS: usize = 120;

/// Where `bash` keeps the commands it runs in a terminal, so one can outlive the call that
/// started it: a later `bash_input` or `bash_output`, or the host's own UI, reaches it by id.
/// The host decides how long a session lives and ends it with [`BashSession::stop`] (a Stop,
/// a closed chat, its own limits); the tools start sessions, read them, and write to them.
pub trait BashSessions: Send + Sync {
    /// Keeps `session`, which the call `call_id` just started.
    fn insert(&self, call_id: &str, session: Arc<BashSession>);
    /// The session named `id`, while the host keeps it for the caller.
    fn get(&self, id: &str) -> Option<Arc<BashSession>>;
    /// The model has read how the session ended; the host may let it go.
    fn remove(&self, id: &str);
}

/// How a session ended.
#[derive(Debug, Clone, PartialEq)]
pub enum SessionEnd {
    /// The shell exited with this code.
    Exited(i32),
    /// A signal ended the shell (Ctrl-C's SIGINT is 2). As on pipes, this is not a failure.
    Signaled(i32),
    /// Lorca ended it, for this reason, as the model and the user read it: "Command aborted".
    Stopped(String),
}

impl SessionEnd {
    /// `exited`, `failed` (a nonzero code), or `stopped`.
    pub fn state(&self) -> &'static str {
        match self {
            SessionEnd::Exited(0) | SessionEnd::Signaled(_) => "exited",
            SessionEnd::Exited(_) => "failed",
            SessionEnd::Stopped(_) => "stopped",
        }
    }

    /// In words, for a status line: "Command exited with code 1".
    pub fn describe(&self) -> String {
        match self {
            SessionEnd::Exited(code) => format!("Command exited with code {code}"),
            SessionEnd::Signaled(signal) => format!("Command terminated by signal {signal}"),
            SessionEnd::Stopped(reason) => reason.clone(),
        }
    }

    /// The exit code, for a result's details.
    pub fn code(&self) -> Option<i32> {
        match self {
            SessionEnd::Exited(code) => Some(*code),
            _ => None,
        }
    }
}

/// A command running in a pseudo-terminal of its own. Its output drains into a bounded buffer
/// whether or not anyone reads it, so the command never blocks on a full terminal; the raw
/// bytes spill to a file once they outgrow a result. Dropping the last handle kills whatever
/// still runs.
pub struct BashSession {
    id: String,
    command: String,
    pid: u32,
    started: Instant,
    state: Mutex<SessionState>,
    changed: watch::Sender<u64>,
    #[cfg(unix)]
    master: Mutex<Option<Arc<tokio::io::unix::AsyncFd<std::os::fd::OwnedFd>>>>,
    /// Cancelled when the terminal closes: the reader lets go of it.
    closed: CancellationToken,
}

struct SessionState {
    output: Output,
    last_output: Instant,
    last_input: Instant,
    end: Option<SessionEnd>,
    /// How much of the output the model has seen, in raw bytes.
    read: u64,
    /// The reader reached the end of the terminal: nothing holds it open anymore.
    drained: bool,
}

/// The raw output: its last `MEMORY_BYTES`, with counts of what came before, and every byte in
/// the spill file once there is one.
struct Output {
    tail: std::collections::VecDeque<u8>,
    total: u64,
    /// Line breaks among the bytes no longer in `tail`.
    dropped_lines: u64,
    spill: Option<Spill>,
}

struct Spill {
    path: PathBuf,
    file: Option<std::fs::File>,
    /// A result named the file, so it stays after the session.
    shown: bool,
    /// Bytes in the file, up to `SPILL_MAX_BYTES`.
    written: u64,
}

impl Spill {
    fn write(&mut self, bytes: &[u8]) {
        use std::io::Write;
        let Some(file) = self.file.as_mut() else { return };
        if self.written >= SPILL_MAX_BYTES {
            return;
        }
        let room = (SPILL_MAX_BYTES - self.written) as usize;
        let part = &bytes[..bytes.len().min(room)];
        let _ = file.write_all(part);
        self.written += part.len() as u64;
        if part.len() < bytes.len() {
            let _ = file.write_all(format!("\n[Lorca stopped saving this output at {}.]\n", format_size(SPILL_MAX_BYTES as usize)).as_bytes());
            self.written = SPILL_MAX_BYTES;
        }
    }
}

impl Output {
    fn new() -> Self {
        Output { tail: std::collections::VecDeque::new(), total: 0, dropped_lines: 0, spill: None }
    }

    fn push(&mut self, bytes: &[u8], id: &str) {
        self.total += bytes.len() as u64;
        self.tail.extend(bytes);
        if self.tail.len() > MEMORY_BYTES {
            let excess = self.tail.len() - MEMORY_BYTES;
            self.dropped_lines += self.tail.drain(..excess).filter(|b| *b == b'\n').count() as u64;
        }
        if self.spill.is_none() && self.total > DEFAULT_MAX_BYTES as u64 {
            self.start_spill(id);
        } else if let Some(spill) = self.spill.as_mut() {
            spill.write(bytes);
        }
    }

    /// Opens the spill file with everything so far, which the tail still holds: the spill
    /// starts at `DEFAULT_MAX_BYTES`, well inside `MEMORY_BYTES`.
    fn start_spill(&mut self, id: &str) {
        let path = std::env::temp_dir().join(format!("lorca-{id}-{}.log", crate::now_ms()));
        let mut spill = Spill { file: std::fs::File::create(&path).ok(), path, shown: false, written: 0 };
        let (a, b) = self.tail.as_slices();
        spill.write(a);
        spill.write(b);
        self.spill = Some(spill);
    }

    /// The bytes from `from` on, as far back as memory reaches, with whether older ones are
    /// missing from memory and how many line breaks came before the returned slice. Cut by
    /// memory, the slice starts at a line, never inside a character or an escape sequence.
    fn since(&self, from: u64) -> (Vec<u8>, bool, u64) {
        let memory_start = self.total - self.tail.len() as u64;
        let dropped = from < memory_start;
        let mut skip = (from.max(memory_start) - memory_start) as usize;
        if dropped {
            if let Some(line_end) = self.tail.iter().skip(skip).position(|b| *b == b'\n') {
                skip += line_end + 1;
            }
        }
        let lines_before = self.dropped_lines + self.tail.iter().take(skip).filter(|b| **b == b'\n').count() as u64;
        (self.tail.iter().skip(skip).copied().collect(), dropped, lines_before)
    }

    /// The text of the last bytes, for the prompt check.
    fn tail_text(&self, bytes: usize) -> String {
        let skip = self.tail.len().saturating_sub(bytes);
        terminal_text(&self.tail.iter().skip(skip).copied().collect::<Vec<u8>>())
    }
}

/// When a wait gives up on silence.
#[derive(Clone, Copy)]
pub(crate) struct Idle {
    pub after: Duration,
    /// Only once the command printed something new: a wait the caller asked for runs its
    /// length through silence.
    pub needs_output: bool,
}

/// Why a wait returned.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) enum Stop {
    Ended,
    /// Alive and quiet: at a prompt, or silent past the idle limit.
    Waiting,
    /// The caller's deadline passed while the command ran on.
    Deadline,
    Cancelled,
}

impl BashSession {
    pub fn id(&self) -> &str {
        &self.id
    }

    /// The command as the model wrote it.
    pub fn command(&self) -> &str {
        &self.command
    }

    pub fn pid(&self) -> u32 {
        self.pid
    }

    /// How it ended, or `None` while it runs.
    pub fn end(&self) -> Option<SessionEnd> {
        self.state.lock().unwrap().end.clone()
    }

    /// When it last printed or was typed into, or started.
    pub fn last_activity(&self) -> Instant {
        let state = self.state.lock().unwrap();
        state.last_output.max(state.last_input)
    }

    /// When it last printed, or started.
    pub fn last_output(&self) -> Instant {
        self.state.lock().unwrap().last_output
    }

    /// How many bytes it has printed in all.
    pub fn total(&self) -> u64 {
        self.state.lock().unwrap().output.total
    }

    /// Changes whenever it prints or ends.
    pub fn changes(&self) -> watch::Receiver<u64> {
        self.changed.subscribe()
    }

    /// The line it asks with, when its output ends in one that nothing was typed after: "[sudo]
    /// password for ana:". With echo off, an answered question stays the last line until the
    /// command prints again.
    pub fn prompt(&self) -> Option<String> {
        let state = self.state.lock().unwrap();
        if state.last_input > state.last_output {
            return None;
        }
        prompt_hint(&state.output.tail_text(2048))
    }

    /// Whether it stopped at a question: its output ends on an open line that reads like one,
    /// and nothing was typed after it.
    pub fn asks(&self) -> bool {
        let state = self.state.lock().unwrap();
        if state.last_input > state.last_output {
            return false;
        }
        let text = state.output.tail_text(2048);
        !text.ends_with('\n') && prompt_hint(&text).is_some()
    }

    /// Whether a program in it has put the terminal in raw mode, as one does to read keys one at
    /// a time: a menu, a yes/no choice, a full-screen program. It shows even when the program
    /// prints into a pipe: the master reads the terminal's modes on macOS and Linux alike.
    pub fn reads_keys(&self) -> bool {
        #[cfg(unix)]
        {
            use std::os::fd::AsRawFd;
            let Some(fd) = self.master.lock().unwrap().clone() else { return false };
            let mut termios = std::mem::MaybeUninit::<libc::termios>::uninit();
            if unsafe { libc::tcgetattr(fd.get_ref().as_raw_fd(), termios.as_mut_ptr()) } != 0 {
                return false;
            }
            unsafe { termios.assume_init() }.c_lflag & libc::ICANON == 0
        }
        #[cfg(not(unix))]
        false
    }

    /// Its last `count` lines with anything on them, without escapes, each cut to a screen's
    /// width: what a terminal would show at the bottom.
    pub fn last_lines(&self, count: usize) -> Vec<String> {
        let text = self.state.lock().unwrap().output.tail_text(4096);
        let mut lines: Vec<String> = text
            .lines()
            .rev()
            .map(str::trim_end)
            .filter(|line| !line.trim().is_empty())
            .take(count)
            .map(|line| if line.chars().count() > 160 { format!("{}…", line.chars().take(160).collect::<String>()) } else { line.to_string() })
            .collect();
        lines.reverse();
        lines
    }

    /// Resolves once the session has ended.
    pub async fn ended(&self) {
        let mut changes = self.changed.subscribe();
        while self.end().is_none() {
            if changes.changed().await.is_err() {
                return;
            }
        }
    }

    /// Ends the session: kills the command's process group and closes the terminal, which
    /// hangs up what else holds it. `reason` is how the model and the user read the end. A
    /// session that already ended keeps its end.
    pub fn stop(&self, reason: impl Into<String>) {
        {
            let mut state = self.state.lock().unwrap();
            if state.end.is_some() {
                return;
            }
            state.end = Some(SessionEnd::Stopped(reason.into()));
        }
        super::bash::kill_group(self.pid);
        self.close();
        self.changed.send_modify(|version| *version += 1);
    }

    /// Types `text` into the command, as keys on its terminal. The input counts from before the
    /// first key, so whatever the command prints in reply comes after it.
    pub async fn write(&self, text: &[u8]) -> Result<(), String> {
        #[cfg(unix)]
        {
            let fd = self.master.lock().unwrap().clone().ok_or("The command has ended")?;
            self.state.lock().unwrap().last_input = Instant::now();
            // Whoever follows the session sees the question answered.
            self.changed.send_modify(|version| *version += 1);
            let mut written = 0;
            let result = tokio::time::timeout(WRITE_TIMEOUT, async {
                while written < text.len() {
                    let mut guard = fd.writable().await.map_err(|e| e.to_string())?;
                    match guard.try_io(|inner| {
                        use std::os::fd::AsRawFd;
                        let rest = &text[written..];
                        let n = unsafe { libc::write(inner.as_raw_fd(), rest.as_ptr().cast(), rest.len()) };
                        if n < 0 {
                            Err(std::io::Error::last_os_error())
                        } else {
                            Ok(n as usize)
                        }
                    }) {
                        Ok(Ok(n)) => written += n,
                        Ok(Err(error)) if error.kind() == std::io::ErrorKind::Interrupted => {}
                        Ok(Err(error)) => return Err(error.to_string()),
                        Err(_would_block) => {}
                    }
                }
                Ok(())
            })
            .await;
            result.map_err(|_| "The command is not taking input".to_string())?
        }
        #[cfg(not(unix))]
        {
            let _ = text;
            Err("Interactive input is not available on this platform".into())
        }
    }

    /// Waits until the command ends, goes quiet (at a prompt, or past `idle`), `until`
    /// passes, or `cancel` fires. `base` is how much output there was when the wait began and
    /// `since` when it began: silence counts from the later of `since` and the last output,
    /// and only output past `base` counts as new. `on_output` runs after each chunk.
    pub(crate) async fn wait(
        &self,
        base: u64,
        since: Instant,
        idle: Idle,
        until: Option<Instant>,
        cancel: &CancellationToken,
        mut on_output: impl FnMut(&BashSession),
    ) -> Stop {
        let mut changes = self.changed.subscribe();
        loop {
            let now = Instant::now();
            let (ended, fresh, quiet_since) = {
                let state = self.state.lock().unwrap();
                (state.end.is_some(), state.output.total > base, state.last_output.max(since))
            };
            if ended {
                return Stop::Ended;
            }
            if until.is_some_and(|until| now >= until) {
                return Stop::Deadline;
            }
            let prompt_due = (fresh && self.asks()).then(|| quiet_since + PROMPT_QUIET);
            let idle_due = (!idle.needs_output || fresh).then(|| quiet_since + idle.after);
            let due = [prompt_due, idle_due].into_iter().flatten().min();
            if due.is_some_and(|due| now >= due) {
                return Stop::Waiting;
            }
            let wake = [due, until].into_iter().flatten().min();
            tokio::select! {
                changed = changes.changed() => {
                    if changed.is_err() {
                        return Stop::Ended;
                    }
                    on_output(self);
                }
                _ = sleep_until_some(wake) => {}
                _ = cancel.cancelled() => return Stop::Cancelled,
            }
        }
    }

    /// The output from `from` on, for the model: without escapes, tail-truncated, with where
    /// the whole of it is when cut. Marks it read.
    fn read_from(&self, from: u64) -> Shown {
        let mut state = self.state.lock().unwrap();
        let (raw, dropped, lines_before) = state.output.since(from);
        let text = terminal_text(&raw);
        let truncation = truncate_tail(&text, TruncationOptions::default());
        let cut = truncation.truncated || dropped;
        if cut && state.output.spill.is_none() {
            state.output.start_spill(&self.id);
        }
        let full_output_path = match state.output.spill.as_mut() {
            Some(spill) if cut && spill.file.is_some() => {
                spill.shown = true;
                if let Some(file) = spill.file.as_mut() {
                    use std::io::Write;
                    let _ = file.flush();
                }
                Some(spill.path.clone())
            }
            _ => None,
        };
        state.read = state.output.total;
        let text = if cut { footer(&truncation, dropped, lines_before, full_output_path.as_deref()) } else { truncation.content.clone() };
        Shown { text, truncation, full_output_path }
    }

    /// The model's place in the output.
    fn read_mark(&self) -> u64 {
        self.state.lock().unwrap().read
    }

    /// The whole output so far as the model would read it, for progress updates.
    fn preview(&self) -> String {
        let state = self.state.lock().unwrap();
        let (raw, _, _) = state.output.since(0);
        truncate_tail(&terminal_text(&raw), TruncationOptions::default()).content
    }

    fn close(&self) {
        #[cfg(unix)]
        self.master.lock().unwrap().take();
        self.closed.cancel();
    }

    fn push_output(&self, bytes: &[u8]) {
        {
            let mut state = self.state.lock().unwrap();
            state.output.push(bytes, &self.id);
            state.last_output = Instant::now();
        }
        self.changed.send_modify(|version| *version += 1);
    }

    /// The shell exited: read what is still on its way, close the terminal, and record the
    /// end unless the session was stopped first.
    async fn exited(&self, end: SessionEnd) {
        let exited_at = Instant::now();
        let mut changes = self.changed.subscribe();
        loop {
            let (drained, last_output) = {
                let state = self.state.lock().unwrap();
                (state.drained, state.last_output)
            };
            let quiet_until = last_output.max(exited_at) + DRAIN_QUIET;
            if drained || Instant::now() >= quiet_until || exited_at.elapsed() >= DRAIN_MAX {
                break;
            }
            let _ = tokio::time::timeout_at(quiet_until.min(exited_at + DRAIN_MAX), changes.changed()).await;
        }
        self.close();
        let mut state = self.state.lock().unwrap();
        if state.end.is_none() {
            state.end = Some(end);
        }
        drop(state);
        self.changed.send_modify(|version| *version += 1);
    }
}

impl Drop for BashSession {
    fn drop(&mut self) {
        let state = self.state.get_mut().unwrap();
        if state.end.is_none() {
            super::bash::kill_group(self.pid);
        }
        self.closed.cancel();
        if let Some(spill) = state.output.spill.take().filter(|spill| !spill.shown) {
            drop(spill.file);
            let _ = std::fs::remove_file(&spill.path);
        }
    }
}

async fn sleep_until_some(deadline: Option<Instant>) {
    match deadline {
        Some(deadline) => tokio::time::sleep_until(deadline).await,
        None => std::future::pending().await,
    }
}

/// A new session id: `bash-` and six hex digits, short enough to read back.
fn new_id() -> String {
    format!("bash-{}", &uuid::Uuid::new_v4().simple().to_string()[..6])
}

#[cfg(unix)]
impl BashSession {
    /// Starts `command` under `shell -c` in `cwd`, on a new pseudo-terminal that is the
    /// command's controlling terminal. `timeout` (seconds) kills it when it runs that long.
    pub async fn spawn(shell: &str, command: &str, cwd: &Path, timeout: Option<f64>) -> std::io::Result<Arc<BashSession>> {
        use std::os::fd::AsRawFd;
        use std::process::Stdio;
        use tokio::io::unix::AsyncFd;
        use tokio::io::Interest;

        let (master, slave) = open_pty()?;
        let mut cmd = crate::login_shell::command(shell).await;
        cmd.arg("-c")
            .arg(command)
            .current_dir(cwd)
            // Programs draw for a terminal and page for a person. Nobody scrolls this one.
            .env("TERM", "xterm-256color")
            .env("PAGER", "cat")
            .env("GIT_PAGER", "cat")
            .env_remove("COLUMNS")
            .env_remove("LINES")
            .stdin(Stdio::from(slave.try_clone()?))
            .stdout(Stdio::from(slave.try_clone()?))
            .stderr(Stdio::from(slave));
        unsafe {
            cmd.pre_exec(|| {
                // A session of its own, whose controlling terminal is the pty: /dev/tty is this
                // terminal, so sudo, ssh, and getpass ask here and never on the terminal
                // `lorca serve` was started from. The shell leads the session and its process
                // group (pgid = pid), which is what stopping it kills.
                if libc::setsid() == -1 {
                    return Err(std::io::Error::last_os_error());
                }
                if libc::ioctl(0, libc::TIOCSCTTY as _, 0) == -1 {
                    return Err(std::io::Error::last_os_error());
                }
                // A controlling terminal sends SIGHUP to the command's process group when the
                // shell exits. On pipes nothing did, and `server > log 2>&1 &` outlived the
                // command; it still does, as under nohup. Stopping a session uses SIGKILL.
                libc::signal(libc::SIGHUP, libc::SIG_IGN);
                Ok(())
            });
        }
        let mut child = cmd.spawn()?;
        // The slave's last copies in this process go with the command.
        drop(cmd);
        let pid = child.id().unwrap_or(0);
        let fd = Arc::new(AsyncFd::with_interest(master, Interest::READABLE | Interest::WRITABLE)?);
        let now = Instant::now();
        let session = Arc::new(BashSession {
            id: new_id(),
            command: command.to_string(),
            pid,
            started: now,
            state: Mutex::new(SessionState { output: Output::new(), last_output: now, last_input: now, end: None, read: 0, drained: false }),
            changed: watch::channel(0).0,
            master: Mutex::new(Some(fd.clone())),
            closed: CancellationToken::new(),
        });

        // The reader drains the terminal for as long as it is open. A command that writes to a
        // terminal nobody reads blocks, and on macOS one that exits with output unread waits
        // in exit until someone reads it.
        let weak = Arc::downgrade(&session);
        let closed = session.closed.clone();
        tokio::spawn(async move {
            let mut buffer = vec![0u8; 16 * 1024];
            loop {
                let mut guard = tokio::select! {
                    guard = fd.readable() => match guard { Ok(guard) => guard, Err(_) => break },
                    _ = closed.cancelled() => break,
                };
                match guard.try_io(|inner| {
                    let n = unsafe { libc::read(inner.as_raw_fd(), buffer.as_mut_ptr().cast(), buffer.len()) };
                    if n < 0 {
                        Err(std::io::Error::last_os_error())
                    } else {
                        Ok(n as usize)
                    }
                }) {
                    // End of file on macOS, EIO on Linux: nothing holds the terminal anymore.
                    Ok(Ok(0)) => break,
                    Ok(Ok(n)) => match weak.upgrade() {
                        Some(session) => session.push_output(&buffer[..n]),
                        None => break,
                    },
                    Ok(Err(error)) if error.kind() == std::io::ErrorKind::Interrupted => {}
                    Ok(Err(_)) => break,
                    Err(_would_block) => {}
                }
            }
            if let Some(session) = weak.upgrade() {
                session.state.lock().unwrap().drained = true;
                session.changed.send_modify(|version| *version += 1);
            }
        });

        let weak = Arc::downgrade(&session);
        let deadline = timeout.map(|seconds| (now + Duration::from_secs_f64(seconds), seconds));
        tokio::spawn(async move {
            let status = match deadline {
                Some((at, seconds)) => tokio::select! {
                    status = child.wait() => status,
                    _ = tokio::time::sleep_until(at) => {
                        if let Some(session) = weak.upgrade() {
                            session.stop(format!("Command timed out after {seconds} seconds"));
                        }
                        child.wait().await
                    }
                },
                None => child.wait().await,
            };
            if let Some(session) = weak.upgrade() {
                use std::os::unix::process::ExitStatusExt;
                let end = match status {
                    Ok(status) => match (status.code(), status.signal()) {
                        (Some(code), _) => SessionEnd::Exited(code),
                        (None, Some(signal)) => SessionEnd::Signaled(signal),
                        (None, None) => SessionEnd::Exited(0),
                    },
                    Err(error) => SessionEnd::Stopped(format!("Lost track of the command: {error}")),
                };
                session.exited(end).await;
            }
        });
        Ok(session)
    }
}

#[cfg(not(unix))]
impl BashSession {
    pub async fn spawn(_shell: &str, _command: &str, _cwd: &Path, _timeout: Option<f64>) -> std::io::Result<Arc<BashSession>> {
        Err(std::io::Error::new(std::io::ErrorKind::Unsupported, "interactive terminals are not available on this platform"))
    }
}

/// A new pseudo-terminal pair, `COLUMNS` by `ROWS`, with echo off: what the user or the model
/// types (a `y`, a password) never comes back as output, so it cannot reach the transcript or
/// the model's context. This also hides the typing from the output shown on purpose; a program
/// that echoes for itself (readline, curses) still does, and prompts print as usual.
#[cfg(unix)]
fn open_pty() -> std::io::Result<(std::os::fd::OwnedFd, std::os::fd::OwnedFd)> {
    use std::os::fd::{AsRawFd, FromRawFd, OwnedFd};
    let (mut master, mut slave) = (-1, -1);
    let mut size = libc::winsize { ws_row: ROWS, ws_col: COLUMNS, ws_xpixel: 0, ws_ypixel: 0 };
    if unsafe { libc::openpty(&mut master, &mut slave, std::ptr::null_mut(), std::ptr::null_mut(), &mut size) } != 0 {
        return Err(std::io::Error::last_os_error());
    }
    let (master, slave) = unsafe { (OwnedFd::from_raw_fd(master), OwnedFd::from_raw_fd(slave)) };
    for fd in [&master, &slave] {
        // Other children must not inherit either end, or the terminal outlives the command.
        if unsafe { libc::fcntl(fd.as_raw_fd(), libc::F_SETFD, libc::FD_CLOEXEC) } == -1 {
            return Err(std::io::Error::last_os_error());
        }
    }
    let mut termios = std::mem::MaybeUninit::<libc::termios>::uninit();
    if unsafe { libc::tcgetattr(slave.as_raw_fd(), termios.as_mut_ptr()) } == 0 {
        let mut termios = unsafe { termios.assume_init() };
        termios.c_lflag &= !(libc::ECHO | libc::ECHONL);
        unsafe { libc::tcsetattr(slave.as_raw_fd(), libc::TCSANOW, &termios) };
    }
    let flags = unsafe { libc::fcntl(master.as_raw_fd(), libc::F_GETFL) };
    if flags == -1 || unsafe { libc::fcntl(master.as_raw_fd(), libc::F_SETFL, flags | libc::O_NONBLOCK) } == -1 {
        return Err(std::io::Error::last_os_error());
    }
    Ok((master, slave))
}

/// Words that mark a question on a terminal. A guess from the last line, never a parse.
static PROMPT_WORDS: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"(?i)password|passphrase|\[y/n\]|\(y/n\)|\(yes/no|\benter |\bpress |\bpin\b|\btoken\b|\bcode\b|\botp\b").unwrap()
});

/// The line a command asks with: the last line of `text` when it reads like a question, either
/// by its words or, left open with no line break after it, by ending as prompts do (`?`, `:`,
/// `>`, `)`, `]`, `$`, `#`).
pub fn prompt_hint(text: &str) -> Option<String> {
    let open = !text.ends_with('\n');
    let line = text.trim_end_matches('\n').rsplit('\n').next()?.trim();
    if line.is_empty() {
        return None;
    }
    let ends_like_prompt = line.ends_with(['?', ':', '>', ')', ']', '$', '#']);
    if !(PROMPT_WORDS.is_match(line) || (open && ends_like_prompt)) {
        return None;
    }
    Some(if line.chars().count() > PROMPT_CHARS { format!("{}…", line.chars().take(PROMPT_CHARS).collect::<String>()) } else { line.to_string() })
}

/// A key a model spelled out as its escape, the way JSON that escaped the backslash delivers
/// it: `\u0003` as six characters, `\x03`, or `\u001b[B` for Down. Only the whole text, and only
/// a control character with at most a key's tail after it, so code typed into a REPL that holds
/// an escape goes as written.
static ESCAPED_KEY: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"^\\(?:u00([01][0-9a-fA-F]|7[fF])|x([01][0-9a-fA-F]|7[fF]))(\[[0-9;]*[A-Za-z~]|O[A-Za-z])?$").unwrap());

/// What typing `text` sends: the text as written, or the key it spells out as an escape.
pub fn typed_keys(text: &str) -> std::borrow::Cow<'_, str> {
    let Some(parts) = ESCAPED_KEY.captures(text) else { return text.into() };
    let code = parts.get(1).or_else(|| parts.get(2)).map_or("", |m| m.as_str());
    let Ok(byte) = u8::from_str_radix(code, 16) else { return text.into() };
    format!("{}{}", char::from(byte), parts.get(3).map_or("", |m| m.as_str())).into()
}

/// Output shown to the model: the text, and how it was cut.
pub(crate) struct Shown {
    pub text: String,
    pub truncation: TruncationResult,
    pub full_output_path: Option<PathBuf>,
}

/// The truncation note, with line numbers that count from the start of the whole output, where
/// the full output file starts.
fn footer(truncation: &TruncationResult, dropped: bool, lines_before: u64, full: Option<&Path>) -> String {
    let full = full.map(|p| p.display().to_string()).unwrap_or_default();
    let end = lines_before + truncation.total_lines as u64;
    let start = end + 1 - (truncation.output_lines as u64).min(end);
    let mut out = truncation.content.clone();
    if truncation.last_line_partial {
        out.push_str(&format!("\n\n[Showing last {} of line {end}. Full output: {full}]", format_size(truncation.output_bytes)));
    } else if truncation.truncated_by == Some(TruncatedBy::Lines) || (!truncation.truncated && dropped) {
        out.push_str(&format!("\n\n[Showing lines {start}-{end} of {end}. Full output: {full}]"));
    } else {
        out.push_str(&format!("\n\n[Showing lines {start}-{end} of {end} ({} limit). Full output: {full}]", format_size(DEFAULT_MAX_BYTES)));
    }
    out
}

/// The status line of a session still running when a call returns. Silence is not a question:
/// a quiet command may be working, or asking where nobody sees it, since a question printed
/// into a pipe or a file never reaches the terminal. Raw mode is the one sign of that left.
fn running_note(session: &BashSession, stop: Stop, idle: Duration) -> String {
    let id = session.id();
    match (stop, session.prompt()) {
        (Stop::Waiting, Some(prompt)) => format!(
            "[Waiting for input: \"{prompt}\". The command is still running as session {id}: answer it with bash_input, or check on it with bash_output.]"
        ),
        (Stop::Waiting, None) if session.reads_keys() => format!(
            "[No output for {} seconds, and the command has put its terminal in raw mode, as a program does to read keys at a menu \
             or a yes/no choice: it is probably waiting for one. If its question is not above, it went into a pipe or a file (`| \
             tail`, `> log`) where nobody can see it: stop the command with bash_input \"\\u0003\" and run it again with its \
             non-interactive options (--yes, --no-interactive) or with stdin from /dev/null. Otherwise press keys with bash_input: \
             \"\" is Enter, and \"\\u001b[B\" and \"\\u001b[A\" with enter false move down and up. It is still running as session {id}.]",
            idle.as_secs_f64()
        ),
        (Stop::Waiting, None) => format!(
            "[No output for {} seconds. The command is still running as session {id}: it may be working quietly, or waiting for \
             input. A question it prints into a pipe or a file (`| tail`, `> log`) never shows here. Keep waiting with \
             bash_output, answer it with bash_input, or stop it with bash_input \"\\u0003\".]",
            idle.as_secs_f64()
        ),
        _ => format!("[Still running as session {id}. Wait for more with bash_output, or answer it with bash_input.]"),
    }
}

/// The result of a call on `session` once `wait` returned: output from `from`, and how the
/// command stands. `first` is the call that started it, whose result on a normal end reads
/// exactly like a command run on pipes.
pub(crate) fn session_result(session: &Arc<BashSession>, sessions: &dyn BashSessions, from: u64, stop: Stop, idle: Duration, first: bool) -> Result<ToolResult, ToolError> {
    if stop == Stop::Cancelled {
        session.stop("Command aborted");
    }
    let shown = session.read_from(from);
    let with_status = |status: &str| if shown.text.is_empty() { status.to_string() } else { format!("{}\n\n{status}", shown.text) };
    let summary = format!("$ {}", session.command().lines().next().unwrap_or("").chars().take(60).collect::<String>());
    let truncation = if shown.truncation.truncated { serde_json::to_value(&shown.truncation).unwrap_or(Value::Null) } else { Value::Null };
    let Some(end) = session.end() else {
        let prompt = if stop == Stop::Waiting { session.prompt() } else { None };
        let body = with_status(&running_note(session, stop, idle));
        return Ok(ToolResult::text(body).with_details(json!({
            "summary": if prompt.is_some() { "Waiting for input" } else { "Running" },
            "session_id": session.id(),
            "state": "waiting",
            "prompt": prompt,
            "truncation": truncation,
            "full_output_path": shown.full_output_path,
        })));
    };
    sessions.remove(session.id());
    match &end {
        SessionEnd::Stopped(reason) => Err(ToolError(with_status(reason))),
        SessionEnd::Exited(code) if *code != 0 => Err(ToolError(with_status(&end.describe()))),
        SessionEnd::Exited(_) | SessionEnd::Signaled(_) => {
            let body = if first {
                if shown.text.is_empty() { "(no output)".to_string() } else { shown.text.clone() }
            } else {
                with_status(&end.describe())
            };
            Ok(ToolResult::text(body).with_details(json!({
                "summary": summary,
                "exit_code": end.code(),
                "truncation": truncation,
                "full_output_path": shown.full_output_path,
            })))
        }
    }
}

/// Details for a call that did not end, so the host can tie its row to the session.
pub fn session_of(details: &Value) -> Option<&str> {
    details["session_id"].as_str()
}

// MARK: - Tools

/// `bash_input`: types into a command `bash` left running, then waits for what it does next.
pub struct BashInputTool {
    sessions: Arc<dyn BashSessions>,
    waiting_after: Duration,
}

impl BashInputTool {
    pub fn new(sessions: Arc<dyn BashSessions>) -> Self {
        BashInputTool { sessions, waiting_after: WAITING_AFTER }
    }

    /// How long the command may stay silent after the input before the call returns.
    pub fn waiting_after(mut self, after: Duration) -> Self {
        self.waiting_after = after;
        self
    }
}

#[async_trait]
impl Tool for BashInputTool {
    fn name(&self) -> &str {
        "bash_input"
    }
    fn description(&self) -> &str {
        "Type into a command that bash left running, by its session id: the text, then Enter unless enter is false. Returns \
         what the command printed since you last read it, once it ends, asks again, or goes quiet. Keys are control \
         characters: \\u0003 is Ctrl-C (interrupts the command), \\u0004 is Ctrl-D (ends input), \\u001b is Escape. What you \
         type is not echoed back."
    }
    fn parameters(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "session_id": { "type": "string", "description": "The session id bash returned" },
                "text": { "type": "string", "description": "What to type" },
                "enter": { "type": "boolean", "description": "Press Enter after the text (default true)" }
            },
            "required": ["session_id", "text"]
        })
    }
    async fn execute(&self, _id: &str, args: Value, cancel: CancellationToken, _on_update: ToolUpdateFn) -> Result<ToolResult, ToolError> {
        let id = args["session_id"].as_str().ok_or("session_id is required")?.trim();
        let text = typed_keys(args["text"].as_str().ok_or("text is required")?);
        let session = self.sessions.get(id).ok_or_else(|| unknown_session(id))?;
        let from = session.read_mark();
        if session.end().is_none() {
            let mut keys = text.as_bytes().to_vec();
            if args["enter"].as_bool().unwrap_or(true) {
                // What a terminal sends for Return; the line discipline makes it a line end.
                keys.push(b'\r');
            }
            // A command that ended meanwhile reports its end below.
            if let Err(error) = session.write(&keys).await {
                if session.end().is_none() {
                    return Err(ToolError(error));
                }
            }
        }
        let (base, since) = (session.total(), Instant::now());
        let idle = Idle { after: self.waiting_after, needs_output: false };
        let stop = session.wait(base, since, idle, None, &cancel, |_| {}).await;
        session_result(&session, self.sessions.as_ref(), from, stop, self.waiting_after, false)
    }
}

/// `bash_output`: what a command `bash` left running printed since the model last read it,
/// waiting for more when asked.
pub struct BashOutputTool {
    sessions: Arc<dyn BashSessions>,
    waiting_after: Duration,
}

impl BashOutputTool {
    pub fn new(sessions: Arc<dyn BashSessions>) -> Self {
        BashOutputTool { sessions, waiting_after: WAITING_AFTER }
    }

    /// How long the command may stay silent after printing something new before the call
    /// returns ahead of its wait.
    pub fn waiting_after(mut self, after: Duration) -> Self {
        self.waiting_after = after;
        self
    }
}

#[async_trait]
impl Tool for BashOutputTool {
    fn name(&self) -> &str {
        "bash_output"
    }
    fn description(&self) -> &str {
        "Read what a command that bash left running printed since you last read it, by its session id. With wait_seconds (at \
         most 300), wait until it ends, asks for input, or the time is up. Returns whether it still runs or how it ended."
    }
    fn parameters(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "session_id": { "type": "string", "description": "The session id bash returned" },
                "wait_seconds": { "type": "number", "description": "How long to wait for it to finish or print more (default 0: return at once)" }
            },
            "required": ["session_id"]
        })
    }
    async fn execute(&self, _id: &str, args: Value, cancel: CancellationToken, _on_update: ToolUpdateFn) -> Result<ToolResult, ToolError> {
        let id = args["session_id"].as_str().ok_or("session_id is required")?.trim();
        let wait = args["wait_seconds"].as_f64().filter(|w| w.is_finite() && *w > 0.0).map(Duration::from_secs_f64).unwrap_or_default();
        let session = self.sessions.get(id).ok_or_else(|| unknown_session(id))?;
        let from = session.read_mark();
        let (base, since) = (session.total(), Instant::now());
        let idle = Idle { after: self.waiting_after, needs_output: true };
        let stop = session.wait(base, since, idle, Some(since + wait.min(MAX_OUTPUT_WAIT)), &cancel, |_| {}).await;
        session_result(&session, self.sessions.as_ref(), from, stop, self.waiting_after, false)
    }
}

fn unknown_session(id: &str) -> ToolError {
    ToolError(format!("No session {id}: it ended and its output was read, or it was stopped. Run the command again with bash if needed."))
}

/// `bash` in a terminal session: starts the command, keeps it with the host, and waits for it
/// the way every call on a session does. `on_update` streams the output so far.
#[allow(clippy::too_many_arguments)]
pub(crate) async fn run(
    shell: &str,
    command: &str,
    cwd: &Path,
    timeout: Option<f64>,
    sessions: &Arc<dyn BashSessions>,
    call_id: &str,
    waiting_after: Duration,
    cancel: CancellationToken,
    on_update: ToolUpdateFn,
) -> Result<ToolResult, ToolError> {
    let session = BashSession::spawn(shell, command, cwd, timeout).await.map_err(|e| ToolError(format!("Failed to start {shell}: {e}")))?;
    sessions.insert(call_id, session.clone());
    let mut last_update = Instant::now() - Duration::from_secs(1);
    let idle = Idle { after: waiting_after, needs_output: false };
    let stop = session
        .wait(0, session.started, idle, None, &cancel, |session| {
            if last_update.elapsed() >= Duration::from_millis(super::bash::UPDATE_THROTTLE_MS) {
                last_update = Instant::now();
                on_update(ToolResult::text(session.preview()));
            }
        })
        .await;
    session_result(&session, sessions.as_ref(), 0, stop, waiting_after, true)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tools::BashTool;
    use std::collections::HashMap;

    /// Sessions kept in a map, as a host would.
    #[derive(Default)]
    struct Host(Mutex<HashMap<String, Arc<BashSession>>>);

    impl BashSessions for Host {
        fn insert(&self, _call_id: &str, session: Arc<BashSession>) {
            self.0.lock().unwrap().insert(session.id().to_string(), session);
        }
        fn get(&self, id: &str) -> Option<Arc<BashSession>> {
            self.0.lock().unwrap().get(id).cloned()
        }
        fn remove(&self, id: &str) {
            self.0.lock().unwrap().remove(id);
        }
    }

    struct Tools {
        host: Arc<Host>,
        bash: BashTool,
        input: BashInputTool,
        output: BashOutputTool,
    }

    fn tools(waiting_after: Duration) -> Tools {
        let host = Arc::new(Host::default());
        let sessions: Arc<dyn BashSessions> = host.clone();
        Tools {
            bash: BashTool::with_sessions(std::env::temp_dir(), sessions.clone()).waiting_after(waiting_after),
            input: BashInputTool::new(sessions.clone()).waiting_after(waiting_after),
            output: BashOutputTool::new(sessions).waiting_after(waiting_after),
            host,
        }
    }

    async fn call(tool: &dyn Tool, args: Value) -> Result<ToolResult, ToolError> {
        tool.execute("call", args, CancellationToken::new(), Arc::new(|_| {})).await
    }

    fn session_id(result: &ToolResult) -> String {
        session_of(&result.details).expect("a session id").to_string()
    }

    #[cfg(unix)]
    fn alive(pid: i32) -> bool {
        unsafe { libc::kill(pid, 0) == 0 }
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn results_read_as_they_did_on_pipes() {
        let t = tools(WAITING_AFTER);
        let ok = call(&t.bash, json!({"command": "printf 'hi\\nthere'"})).await.unwrap();
        assert_eq!(ok.text_content(), "hi\nthere", "line ends come back as \\n");
        let err = call(&t.bash, json!({"command": "echo boom >&2; exit 3"})).await.unwrap_err();
        assert!(err.0.contains("boom") && err.0.contains("exited with code 3"), "{}", err.0);
        let timeout = call(&t.bash, json!({"command": "sleep 5", "timeout": 0.2})).await.unwrap_err();
        assert!(timeout.0.contains("timed out after 0.2 seconds"), "{}", timeout.0);
        let quiet = call(&t.bash, json!({"command": "true"})).await.unwrap();
        assert_eq!(quiet.text_content(), "(no output)");
        assert!(t.host.0.lock().unwrap().is_empty(), "a command that ended is not kept");
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn commands_get_a_controlling_terminal() {
        let t = tools(WAITING_AFTER);
        let result = call(&t.bash, json!({"command": "[ -t 0 ] && echo tty; : </dev/tty && echo has-tty; stty size; echo \"$TERM $PAGER\""})).await.unwrap();
        assert_eq!(result.text_content(), "tty\nhas-tty\n24 80\nxterm-256color cat\n");
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn a_command_that_reads_waits_and_takes_an_answer() {
        let t = tools(Duration::from_millis(400));
        let waiting = call(&t.bash, json!({"command": "echo ready; read -r x; echo got:$x"})).await.unwrap();
        let text = waiting.text_content();
        assert!(text.starts_with("ready\n\n\n[No output for 0.4 seconds."), "{text}");
        assert_eq!(waiting.details["state"], "waiting");
        let id = session_id(&waiting);
        assert!(text.contains(&id));

        let answered = call(&t.input, json!({"session_id": id, "text": "hello"})).await.unwrap();
        assert_eq!(answered.text_content(), "got:hello\n\n\nCommand exited with code 0", "the answer is not echoed");
        assert!(t.host.0.lock().unwrap().is_empty(), "read to its end, the session goes");
        assert!(call(&t.output, json!({"session_id": id})).await.unwrap_err().0.starts_with("No session"));
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn a_prompt_on_dev_tty_returns_at_once_and_its_answer_stays_secret() {
        let t = tools(Duration::from_secs(30));
        let started = std::time::Instant::now();
        let waiting = call(&t.bash, json!({"command": "read -rs -p 'Password: ' p </dev/tty; echo; echo length:${#p}"})).await.unwrap();
        assert!(started.elapsed() < Duration::from_secs(10), "a prompt does not wait out the silence");
        assert_eq!(waiting.details["prompt"], "Password:");
        assert!(waiting.text_content().contains("[Waiting for input: \"Password:\"."), "{}", waiting.text_content());
        let id = session_id(&waiting);

        // The user answers on the side: nobody holds a call, and a later read has the reply.
        let session = t.host.get(&id).unwrap();
        session.write(b"hunter2\r").await.unwrap();
        session.ended().await;
        let done = call(&t.output, json!({"session_id": id})).await.unwrap();
        assert_eq!(done.text_content(), "\nlength:7\n\n\nCommand exited with code 0");
        assert!(!done.text_content().contains("hunter2"));
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn an_answered_question_is_not_asked_again() {
        let t = tools(Duration::from_secs(30));
        let waiting = call(&t.bash, json!({"command": "read -rs -p 'Password: ' p </dev/tty; sleep 60"})).await.unwrap();
        let session = t.host.get(&session_id(&waiting)).unwrap();
        assert!(session.asks());
        session.write(b"hunter2\r").await.unwrap();
        // With echo off the line still reads "Password: ", and the command has gone quiet.
        tokio::time::sleep(Duration::from_millis(300)).await;
        assert_eq!(session.last_lines(1), ["Password:"]);
        assert!(!session.asks() && session.prompt().is_none());
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn a_yes_no_question_is_answered_by_the_model() {
        let t = tools(Duration::from_secs(30));
        let waiting = call(&t.bash, json!({"command": "printf 'Continue? [y/N] '; read -r a; echo answer:$a; sleep 0.3; echo done"})).await.unwrap();
        assert_eq!(waiting.details["prompt"], "Continue? [y/N]");
        let id = session_id(&waiting);
        let still = call(&t.output, json!({"session_id": id})).await.unwrap();
        assert!(still.text_content().starts_with("[Still running as session"), "nothing new: {}", still.text_content());
        let answered = call(&t.input, json!({"session_id": id, "text": "y"})).await.unwrap();
        assert_eq!(answered.text_content(), "answer:y\ndone\n\n\nCommand exited with code 0");
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn ctrl_c_interrupts() {
        let t = tools(Duration::from_millis(400));
        let waiting = call(&t.bash, json!({"command": "sleep 60"})).await.unwrap();
        let interrupted = call(&t.input, json!({"session_id": session_id(&waiting), "text": "\u{3}", "enter": false})).await.unwrap();
        assert_eq!(interrupted.text_content(), "Command terminated by signal 2");
        // The whole foreground group hears it, as in a terminal: the script stops too.
        let script = call(&t.bash, json!({"command": "sleep 60; echo after"})).await.unwrap();
        let interrupted = call(&t.input, json!({"session_id": session_id(&script), "text": "\u{3}", "enter": false})).await.unwrap();
        assert_eq!(interrupted.text_content(), "Command terminated by signal 2");
        // Spelled out, as a model whose JSON escaped the backslash sends it.
        let spelled = call(&t.bash, json!({"command": "sleep 60"})).await.unwrap();
        let interrupted = call(&t.input, json!({"session_id": session_id(&spelled), "text": "\\u0003"})).await.unwrap();
        assert_eq!(interrupted.text_content(), "Command terminated by signal 2");
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn silence_is_not_taken_for_a_question() {
        let t = tools(Duration::from_millis(400));
        // Quiet on a normal terminal: working, or asking where nobody sees it.
        let quiet = call(&t.bash, json!({"command": "sleep 60"})).await.unwrap();
        assert_eq!((quiet.details["summary"].as_str(), quiet.details["prompt"].as_str()), (Some("Running"), None));
        assert!(quiet.text_content().contains("never shows here"), "{}", quiet.text_content());
        assert!(!t.host.get(&session_id(&quiet)).unwrap().reads_keys());

        // A menu whose output goes into a pipe, as `bun create vite … | tail -5`: the terminal
        // goes raw and stays blank.
        let menu = call(&t.bash, json!({"command": "(stty raw; printf 'Pick a framework? '; sleep 60) | tail -5"})).await.unwrap();
        let text = menu.text_content();
        assert!(text.starts_with("[No output for 0.4 seconds, and the command has put its terminal in raw mode"), "{text}");
        assert!(t.host.get(&session_id(&menu)).unwrap().reads_keys());
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn stop_kills_the_whole_process_group() {
        let t = tools(Duration::from_secs(30));
        let cancel = CancellationToken::new();
        // Stop once the grandchild runs and its pid is in the output. A Stop on a clock can land
        // before the command starts: the first command in a process waits for the login shell's
        // environment, a few hundred milliseconds, and a group stopped then has nothing in it.
        let (host, stopper) = (t.host.clone(), cancel.clone());
        tokio::spawn(async move {
            loop {
                let session = host.0.lock().unwrap().values().next().cloned();
                if session.is_some_and(|session| session.state.lock().unwrap().output.tail.contains(&b'\n')) {
                    return stopper.cancel();
                }
                tokio::time::sleep(Duration::from_millis(10)).await;
            }
        });
        let aborted = t.bash.execute("call", json!({"command": "sleep 300 & echo $!; wait"}), cancel, Arc::new(|_| {})).await.unwrap_err();
        assert!(aborted.0.ends_with("Command aborted"), "{}", aborted.0);
        let pid: i32 = aborted.0.lines().next().unwrap().trim().parse().unwrap();
        let gone = async {
            while alive(pid) {
                tokio::time::sleep(Duration::from_millis(20)).await;
            }
        };
        tokio::time::timeout(Duration::from_secs(5), gone).await.expect("the grandchild died with the group");
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn a_background_job_outlives_a_command_that_ends() {
        let t = tools(WAITING_AFTER);
        let started = call(&t.bash, json!({"command": "sleep 30 >/dev/null 2>&1 & echo $!"})).await.unwrap();
        let pid: i32 = started.text_content().trim().parse().unwrap();
        tokio::time::sleep(Duration::from_millis(300)).await;
        assert!(alive(pid), "as under nohup: the terminal's hangup does not reach it");
        unsafe { libc::kill(pid, libc::SIGKILL) };
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn the_model_reads_text_and_the_spill_file_keeps_the_raw_bytes() {
        let t = tools(WAITING_AFTER);
        let result = call(&t.bash, json!({"command": "for i in $(seq 1 2500); do printf '\\033[31mline %d\\033[0m\\n' $i; done"})).await.unwrap();
        let text = result.text_content();
        assert!(!text.contains('\u{1b}') && !text.contains("[31m"), "no escapes for the model");
        assert!(text.contains("line 2500\n\n[Showing lines 501-2500 of 2500. Full output: "), "{}", &text[text.len() - 200..]);
        let path = result.details["full_output_path"].as_str().unwrap().to_string();
        let raw = std::fs::read(&path).unwrap();
        assert!(raw.windows(5).any(|w| w == b"\x1b[31m"), "the file is what the terminal got");
        assert!(raw.windows(2).any(|w| w == b"\r\n"));
        std::fs::remove_file(path).unwrap();
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn an_unread_spill_file_goes_with_its_session() {
        let t = tools(Duration::from_millis(400));
        let waiting = call(&t.bash, json!({"command": "sleep 1; head -c 60000 /dev/zero | tr '\\0' x; sleep 60"})).await.unwrap();
        let session = t.host.get(&session_id(&waiting)).unwrap();
        tokio::time::timeout(Duration::from_secs(5), async {
            while session.total() < 60000 {
                tokio::time::sleep(Duration::from_millis(20)).await;
            }
        })
        .await
        .unwrap();
        let spill = session.state.lock().unwrap().output.spill.as_ref().map(|s| s.path.clone()).expect("spilled past 50 KB");
        assert!(spill.exists());
        session.stop("Stopped");
        t.host.0.lock().unwrap().clear();
        drop(session);
        assert!(!spill.exists(), "no result named it, so it goes");
    }

    #[test]
    fn a_window_cut_by_memory_starts_at_a_line() {
        let mut output = Output::new();
        let lines = MEMORY_BYTES / 11 + 100;
        for _ in 0..lines {
            output.push(b"abcdefghij\n", "bash-test01");
        }
        let (bytes, dropped, lines_before) = output.since(0);
        assert!(dropped);
        assert!(bytes.starts_with(b"abcdefghij\n"), "a whole line first");
        assert_eq!(lines_before + bytes.iter().filter(|b| **b == b'\n').count() as u64, lines as u64);
        let spill = output.spill.take().unwrap();
        assert_eq!(std::fs::metadata(&spill.path).unwrap().len(), (lines * 11) as u64, "the file has every byte");
        std::fs::remove_file(spill.path).unwrap();
    }

    #[test]
    fn the_spill_file_stops_growing_at_its_cap() {
        let path = std::env::temp_dir().join(format!("lorca-spill-cap-{}.log", uuid::Uuid::new_v4()));
        let mut spill = Spill { file: Some(std::fs::File::create(&path).unwrap()), path: path.clone(), shown: false, written: SPILL_MAX_BYTES - 4 };
        spill.write(b"123456789");
        spill.write(b"more");
        drop(spill);
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "1234\n[Lorca stopped saving this output at 64.0MB.]\n");
        std::fs::remove_file(path).unwrap();
    }

    #[test]
    fn keys_spelled_out_as_escapes_are_typed_as_keys() {
        assert_eq!(typed_keys("\\u0003"), "\u{3}");
        assert_eq!(typed_keys("\\x04"), "\u{4}");
        assert_eq!(typed_keys("\\u001B"), "\u{1b}");
        assert_eq!(typed_keys("\\u001b[B"), "\u{1b}[B");
        assert_eq!(typed_keys("\\u001b[3~"), "\u{1b}[3~");
        assert_eq!(typed_keys("y"), "y");
        assert_eq!(typed_keys("\\u0041"), "\\u0041", "only control characters");
        assert_eq!(typed_keys("print(\"\\x1b[31m\")"), "print(\"\\x1b[31m\")", "code that holds an escape goes as written");
    }

    #[test]
    fn prompts_are_the_last_line_when_it_asks() {
        assert_eq!(prompt_hint("[sudo] password for ana: ").as_deref(), Some("[sudo] password for ana:"));
        assert_eq!(prompt_hint("Do you want to continue? [Y/n] ").as_deref(), Some("Do you want to continue? [Y/n]"));
        assert_eq!(
            prompt_hint("Are you sure you want to continue connecting (yes/no/[fingerprint])? ").as_deref(),
            Some("Are you sure you want to continue connecting (yes/no/[fingerprint])?")
        );
        assert_eq!(prompt_hint("Need to install create-foo\nOk to proceed? (y) ").as_deref(), Some("Ok to proceed? (y)"));
        assert_eq!(prompt_hint("Enter the code we sent:\n").as_deref(), Some("Enter the code we sent:"), "the words count on a closed line");
        assert_eq!(prompt_hint("Compiling foo\n"), None);
        assert_eq!(prompt_hint("Building (3/7)\n"), None, "punctuation counts only on an open line");
        assert_eq!(prompt_hint("spinning up the pinball machine "), None, "whole words only");
        assert_eq!(prompt_hint(""), None);
    }
}
