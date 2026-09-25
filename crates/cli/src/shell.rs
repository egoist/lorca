//! The `bash` calls on this Runner and the terminal sessions they run in
//! (`lorca_agent::tools::bash_session`): which chat and bot each belongs to, the card that shows
//! it (the call's row, `Body::Tool.run`), and what ends it. A card follows its call from the
//! start: Auto-review's question, the command's output while it runs, what it asks, and how it
//! ended. A command waiting for input outlives the turn that started it, so the bot can answer it
//! in a later turn and the user can answer it from any Device (`bash.stdin`). What the user types
//! goes to the command and nowhere else: not the card, not a log, not the bot.
//!
//! A pty that outlives its turn is a leak unless something ends it: the command exiting, Stop
//! (in the chat or on the card), deleting the chat or its bot, Lorca quitting, the idle limit,
//! and the count limit.

use std::sync::{Arc, Mutex};
use std::time::Duration;

use lorca_agent::tools::bash_session::WAITING_AFTER;
use lorca_agent::tools::{BashSession, BashSessions, SessionEnd};
use serde_json::{json, Value};
use tokio::time::Instant;

use crate::app::App;
use crate::model::{Author, Body, CommandRun, Message};

/// A session with no output and no input for this long is stopped. Something that waited this
/// long for an answer is not getting one, and one blocked on something outside its terminal (a
/// macOS permission dialog) never ends on its own; half an hour leaves time to come back to a
/// phone.
pub const IDLE_LIMIT: Duration = Duration::from_secs(30 * 60);
/// At most this many commands run in terminals at once on a Runner; the oldest one stops when
/// another starts. Each holds a pty, a process group, and up to a few hundred KB of output.
pub const MAX_SESSIONS: usize = 8;
/// How long an ended session stays for the bot to read how it ended. Its card says so anyway.
const ENDED_RETENTION: Duration = Duration::from_secs(30 * 60);
/// A running command's card catches up with its output once the output pauses this long, and
/// at least this often while it streams; each change goes to every Device.
const ROW_SETTLE: Duration = Duration::from_millis(500);
const ROW_EVERY: Duration = Duration::from_secs(2);
/// The lines of output a card carries. The apps show them in a block that scrolls.
const ROW_LINES: usize = 20;

/// When sessions stop by themselves.
#[derive(Debug, Clone, Copy)]
pub struct Limits {
    pub idle: Duration,
    pub max: usize,
}

impl Default for Limits {
    fn default() -> Self {
        Limits { idle: IDLE_LIMIT, max: MAX_SESSIONS }
    }
}

/// Every `bash` call on this Runner whose card is open, and every session, oldest first.
#[derive(Default)]
pub struct Sessions {
    entries: Mutex<Vec<Entry>>,
    /// Held while a card is read and written, so the latest state lands last.
    rows: Mutex<()>,
    limits: Mutex<Limits>,
}

/// One `bash` call: the row that shows its card, and the session running its command once one
/// does.
struct Entry {
    chat_id: String,
    bot_id: String,
    call_id: String,
    /// None for a session a call started without a row, which no card shows.
    message_id: Option<String>,
    session: Option<Arc<BashSession>>,
    /// The call returned while the command still ran, and the turn went on without it.
    left_running: bool,
}

impl Sessions {
    /// Changes when sessions stop by themselves, for tests.
    pub fn set_limits(&self, limits: Limits) {
        *self.limits.lock().unwrap() = limits;
    }

    fn limits(&self) -> Limits {
        *self.limits.lock().unwrap()
    }

    /// A `bash` call started, and row `message_id` shows its card.
    pub fn begin(&self, chat_id: &str, bot_id: &str, call_id: &str, message_id: &str) {
        self.entries.lock().unwrap().push(Entry {
            chat_id: chat_id.to_string(),
            bot_id: bot_id.to_string(),
            call_id: call_id.to_string(),
            message_id: Some(message_id.to_string()),
            session: None,
            left_running: false,
        });
    }

    /// The call's command started in `session`: its card follows the session from now on.
    fn insert(&self, app: &Arc<App>, chat_id: &str, bot_id: &str, call_id: &str, session: Arc<BashSession>) {
        let max = self.limits().max;
        let oldest = {
            let mut entries = self.entries.lock().unwrap();
            match entries.iter_mut().find(|e| e.chat_id == chat_id && e.bot_id == bot_id && e.call_id == call_id && e.session.is_none()) {
                Some(entry) => entry.session = Some(session.clone()),
                None => entries.push(Entry {
                    chat_id: chat_id.to_string(),
                    bot_id: bot_id.to_string(),
                    call_id: call_id.to_string(),
                    message_id: None,
                    session: Some(session.clone()),
                    left_running: false,
                }),
            }
            let live: Vec<Arc<BashSession>> = entries.iter().filter_map(|e| e.session.clone()).filter(|s| s.end().is_none()).collect();
            live.len().checked_sub(max).map(|over| live[..over].to_vec()).unwrap_or_default()
        };
        for session in oldest {
            session.stop(format!("Stopped to make room for a newer command ({max} at most)"));
        }
        self.sync_row(app, session.id());
        tokio::spawn(watch(app.clone(), session));
    }

    /// The session `id`, when it belongs to this bot in this chat.
    pub(crate) fn find(&self, chat_id: &str, bot_id: &str, id: &str) -> Option<Arc<BashSession>> {
        let entries = self.entries.lock().unwrap();
        entries.iter().filter_map(|e| e.session.as_ref().filter(|s| s.id() == id && e.chat_id == chat_id && e.bot_id == bot_id)).next().cloned()
    }

    /// Whether session `id` is still kept: it runs, or it ended and its bot has not read how.
    pub fn contains(&self, id: &str) -> bool {
        self.entries.lock().unwrap().iter().any(|e| e.session.as_ref().is_some_and(|s| s.id() == id))
    }

    /// Whether a call this CLI still keeps shows in row `message_id`.
    fn contains_row(&self, message_id: &str) -> bool {
        self.entries.lock().unwrap().iter().any(|e| e.message_id.as_deref() == Some(message_id))
    }

    /// The row that shows call `call_id`'s card, for Auto-review's question.
    pub fn card(&self, chat_id: &str, call_id: &str) -> Option<String> {
        self.entries.lock().unwrap().iter().find(|e| e.chat_id == chat_id && e.call_id == call_id).and_then(|e| e.message_id.clone())
    }

    /// The command session `id` runs, for Auto-review of what is typed into it.
    pub fn command(&self, chat_id: &str, bot_id: &str, id: &str) -> Option<String> {
        self.find(chat_id, bot_id, id).map(|session| session.command().to_string())
    }

    /// The bot read how `id` ended: its card shows the end, and it goes.
    fn remove(&self, app: &App, id: &str) {
        self.sync_row(app, id);
        self.forget(id);
    }

    fn forget(&self, id: &str) {
        self.entries.lock().unwrap().retain(|e| e.session.as_ref().is_none_or(|s| s.id() != id));
    }

    /// Stop in the chat: every session there ends, and its card says so.
    pub fn stop_chat(&self, chat_id: &str) {
        let sessions: Vec<Arc<BashSession>> = self.entries.lock().unwrap().iter().filter(|e| e.chat_id == chat_id).filter_map(|e| e.session.clone()).collect();
        for session in sessions {
            session.stop("Stopped");
        }
    }

    /// Ends and forgets the calls whose chat is gone, or whose bot is no longer in it: after a
    /// chat or a bot is deleted, here or on another Device.
    pub fn close_orphans(&self, app: &App) {
        let members: Vec<(String, Vec<String>)> = app.state.lock().unwrap().chats.iter().map(|chat| (chat.meta.id.clone(), chat.meta.bot_ids.clone())).collect();
        let closed: Vec<Entry> = {
            let mut entries = self.entries.lock().unwrap();
            let (kept, closed) = std::mem::take(&mut *entries)
                .into_iter()
                .partition(|e| members.iter().any(|(chat_id, bot_ids)| *chat_id == e.chat_id && bot_ids.contains(&e.bot_id)));
            *entries = kept;
            closed
        };
        for session in closed.into_iter().filter_map(|e| e.session) {
            session.stop("Stopped: its chat or bot was deleted");
        }
    }

    /// Changes call `message_id`'s card and puts it up.
    pub fn update_card(&self, app: &App, chat_id: &str, message_id: &str, change: impl FnOnce(&mut CommandRun)) {
        let _row = self.rows.lock().unwrap();
        let Some(mut message) = app.message(chat_id, message_id) else { return };
        let Body::Tool { run: Some(run), .. } = &mut message.body else { return };
        change(run);
        app.upsert_message(message, true);
    }

    /// Call `call_id` returned, and `finish` writes its result into row `message_id`. The result
    /// and where the command stands go up in one write, so no Device sees the call returned
    /// beside a card from before. A command still running goes on in its session and its card
    /// follows it; one that ended says how. A call no session ran (not allowed, or run on pipes)
    /// ends its card here, from `text`, its result.
    pub fn call_ended(&self, app: &App, chat_id: &str, call_id: &str, message_id: &str, is_error: bool, text: &str, finish: impl FnOnce(&mut Message)) {
        let session = {
            let mut entries = self.entries.lock().unwrap();
            let entry = entries.iter_mut().find(|e| e.chat_id == chat_id && e.call_id == call_id);
            let session = entry.and_then(|e| {
                let session = e.session.clone()?;
                e.left_running = session.end().is_none();
                Some(session)
            });
            if session.is_none() {
                entries.retain(|e| !(e.chat_id == chat_id && e.call_id == call_id));
            }
            session
        };
        let _row = self.rows.lock().unwrap();
        let Some(mut message) = app.message(chat_id, message_id) else { return };
        finish(&mut message);
        if let Body::Tool { summary, run: Some(run), .. } = &mut message.body {
            match &session {
                Some(session) => follow(run, summary, session),
                None if run.is_open() => {
                    run.state = if is_error { "failed" } else { "exited" }.into();
                    run.prompt = None;
                    let lines = last_lines(text, ROW_LINES);
                    run.output = (!lines.is_empty()).then(|| lines.join("\n"));
                    run.outcome = is_error.then(|| text.lines().next().unwrap_or("").trim().to_string()).filter(|line| !line.is_empty());
                }
                None => {}
            }
        }
        app.upsert_message(message, true);
    }

    /// The turn ended before call `call_id` returned. A command already running goes on, and
    /// its card with it; a card still waiting for Auto-review or for the user's answer stops.
    pub fn abandon_call(&self, app: &App, chat_id: &str, call_id: &str, message_id: &str) {
        let running = {
            let mut entries = self.entries.lock().unwrap();
            let running = entries.iter().any(|e| e.chat_id == chat_id && e.call_id == call_id && e.session.is_some());
            if !running {
                entries.retain(|e| !(e.chat_id == chat_id && e.call_id == call_id));
            }
            running
        };
        if running {
            return;
        }
        self.update_card(app, chat_id, message_id, |run| {
            if run.is_open() {
                run.state = "stopped".into();
                run.prompt = None;
                run.outcome = Some("Stopped".into());
            }
        });
    }

    /// Writes where session `id` stands to its card. False when no card shows it.
    fn sync_row(&self, app: &App, id: &str) -> bool {
        let found = self.entries.lock().unwrap().iter().find(|e| e.session.as_ref().is_some_and(|s| s.id() == id)).and_then(|e| {
            let message_id = e.message_id.clone()?;
            Some((e.chat_id.clone(), message_id, e.session.clone()?))
        });
        let Some((chat_id, message_id, session)) = found else { return false };
        let _row = self.rows.lock().unwrap();
        let Some(mut message) = app.message(&chat_id, &message_id) else { return false };
        let Body::Tool { summary, run: Some(run), .. } = &mut message.body else { return false };
        follow(run, summary, &session);
        app.upsert_message(message, true);
        true
    }

    /// Lorca is quitting: every command still running stops, and its card says so. The cards
    /// go up to the relay on the next start.
    pub fn shutdown(&self, app: &App) {
        let sessions: Vec<Arc<BashSession>> = self.entries.lock().unwrap().iter().filter_map(|e| e.session.clone()).collect();
        for session in sessions {
            session.stop("Stopped when Lorca quit");
            self.sync_row(app, session.id());
        }
    }
}

/// A command its call left running ended by itself, and its bot has not read how: a turn for
/// the bot to hear it and carry on (`kind` `command`). Not after a Stop, the idle limit, or
/// Lorca quitting, which the user brought about.
pub(crate) fn wake_job(app: &App, sessions: &Sessions, id: &str) -> Option<crate::model::Job> {
    let (chat_id, bot_id, message_id) = {
        let entries = sessions.entries.lock().unwrap();
        let entry = entries.iter().find(|e| e.left_running && e.session.as_ref().is_some_and(|s| s.id() == id))?;
        let session = entry.session.as_ref()?;
        if !matches!(session.end(), Some(SessionEnd::Exited(_) | SessionEnd::Signaled(_))) {
            return None;
        }
        (entry.chat_id.clone(), entry.bot_id.clone(), entry.message_id.clone()?)
    };
    Some(crate::runtime::command_job(app, &chat_id, &bot_id, &message_id))
}

/// The last `count` lines of `text` with something on them.
fn last_lines(text: &str, count: usize) -> Vec<String> {
    let mut lines: Vec<String> = text.lines().rev().map(str::trim_end).filter(|line| !line.trim().is_empty()).take(count).map(str::to_string).collect();
    lines.reverse();
    lines
}

/// Where `session` stands, on its card and in its row's summary: waiting or running, with its
/// last lines, or how it ended.
fn follow(run: &mut CommandRun, summary: &mut String, session: &BashSession) {
    let end = session.end();
    let state = end.as_ref().map(SessionEnd::state).unwrap_or_else(|| live_state(session));
    *summary = match &end {
        None if state == "waiting" => "Waiting for input".into(),
        None => "Running".into(),
        Some(SessionEnd::Exited(0)) => command_summary(session),
        Some(end) => end.describe(),
    };
    let lines = session.last_lines(ROW_LINES);
    run.session_id = Some(session.id().to_string());
    run.command = session.command().chars().take(crate::model::APP_COMMAND_CHARS).collect();
    run.state = state.into();
    run.prompt = if end.is_none() { session.prompt() } else { None };
    run.output = (!lines.is_empty()).then(|| lines.join("\n"));
    run.outcome = end.map(|end| end.describe());
}

/// `waiting` when the command stopped at a question or has been silent a while, else `running`.
fn live_state(session: &BashSession) -> &'static str {
    if session.asks() || session.last_output().elapsed() >= WAITING_AFTER {
        "waiting"
    } else {
        "running"
    }
}

fn command_summary(session: &BashSession) -> String {
    format!("$ {}", session.command().lines().next().unwrap_or("").chars().take(60).collect::<String>())
}

/// "Stopped after 30 minutes without output".
fn idle_reason(idle: Duration) -> String {
    match idle.as_secs() / 60 {
        0 => format!("Stopped after {} seconds without output", idle.as_secs_f64()),
        1 => "Stopped after 1 minute without output".into(),
        minutes => format!("Stopped after {minutes} minutes without output"),
    }
}

/// Follows one session for its whole life: stops it at the idle limit, keeps its card in step
/// with its output, and lets it go some time after it ended.
async fn watch(app: Arc<App>, session: Arc<BashSession>) {
    let sessions = &app.shell_sessions;
    let id = session.id().to_string();
    let mut changes = session.changes();
    let mut shown = (session.total(), live_state(&session));
    let mut last_sync = Instant::now();
    while session.end().is_none() {
        let now = Instant::now();
        let idle = sessions.limits().idle;
        let idle_until = session.last_activity() + idle;
        if now >= idle_until {
            session.stop(idle_reason(idle));
            break;
        }
        let current = (session.total(), live_state(&session));
        let mut wake = idle_until;
        if current != shown {
            let due = (session.last_output() + ROW_SETTLE).min(last_sync + ROW_EVERY);
            if now >= due {
                sessions.sync_row(&app, &id);
                shown = current;
                last_sync = now;
                continue;
            }
            wake = wake.min(due);
        }
        // A command printing now reads as waiting once it has been silent a while.
        if current.1 == "running" {
            wake = wake.min(session.last_output() + WAITING_AFTER);
        }
        tokio::select! {
            changed = changes.changed() => {
                if changed.is_err() {
                    break;
                }
            }
            _ = tokio::time::sleep_until(wake) => {}
        }
    }
    sessions.sync_row(&app, &id);
    if let Some(job) = wake_job(&app, sessions, &id) {
        // A turn running in the chat may read how it ended itself (bash_input, bash_output),
        // which drops the session: a bot that heard it then gets no other turn for it.
        let heard = {
            let lock = app.chat_lock(&job.chat_id);
            let _turn = lock.lock().await;
            !sessions.contains(&id)
        };
        if !heard {
            crate::runtime::start_turn(&app, job);
        }
    }
    drop(changes);
    drop(session);
    if sessions.contains(&id) {
        tokio::time::sleep(ENDED_RETENTION).await;
        sessions.forget(&id);
    }
}

/// `bash.stdin` and `bash.stop` for a command's card on this Runner, from the local app or a
/// sealed request: `{ chat_id, message_id, text?, enter? }`. The text is written to the command
/// and dropped; nothing records it.
pub async fn serve(app: &Arc<App>, verb: &str, body: &Value) -> Result<Value, String> {
    let chat_id = body["chat_id"].as_str().ok_or("missing chat_id")?;
    let message_id = body["message_id"].as_str().ok_or("missing message_id")?;
    let message = app.message(chat_id, message_id).ok_or("Unknown message")?;
    let (Author::Bot { bot_id }, Body::Tool { run: Some(run), .. }) = (&message.author, &message.body) else {
        return Err("That card has no command running".into());
    };
    let session_id = run.session_id.as_deref().ok_or("The command has not started")?;
    let session = app.shell_sessions.find(chat_id, bot_id, session_id).filter(|s| s.end().is_none()).ok_or("The command has already ended")?;
    match verb {
        "bash.stdin" => {
            let mut keys = body["text"].as_str().unwrap_or("").as_bytes().to_vec();
            if body["enter"].as_bool().unwrap_or(true) {
                // What a terminal sends for Return.
                keys.push(b'\r');
            }
            session.write(&keys).await?;
            Ok(json!({ "sent": true }))
        }
        "bash.stop" => {
            session.stop("Stopped");
            Ok(json!({ "stopped": true }))
        }
        other => Err(format!("Unknown request {other}")),
    }
}

/// A card left open (Auto-review checking, a question, a command running) by a Lorca that quit
/// before it could say so: the call went with it. Cards of this Runner's own bots only; another
/// Runner's are its own to keep.
pub fn close_stale_rows(app: &App) {
    let Some(this_device) = app.this_device_id() else { return };
    let rows = match app.store.command_rows() {
        Ok(rows) => rows,
        Err(error) => {
            tracing::warn!(%error, "reading command cards");
            return;
        }
    };
    for mut message in rows {
        let Author::Bot { bot_id } = &message.author else { continue };
        if app.bot(bot_id).is_none_or(|bot| bot.runner_id != this_device) || app.shell_sessions.contains_row(&message.id) {
            continue;
        }
        let Body::Tool { summary, run: Some(run), .. } = &mut message.body else { continue };
        if !run.is_open() {
            continue;
        }
        let reason = "Stopped when Lorca quit".to_string();
        run.state = "stopped".into();
        run.prompt = None;
        run.outcome = Some(reason.clone());
        *summary = reason;
        app.upsert_message(message, true);
    }
}

#[cfg(test)]
impl Sessions {
    /// Every session kept, oldest first.
    pub fn entries_for_test(&self) -> Vec<Arc<BashSession>> {
        self.entries.lock().unwrap().iter().filter_map(|e| e.session.clone()).collect()
    }
}

/// A turn's reach into the sessions: a bot reaches the ones it started, in the chat it runs in.
pub struct TurnSessions {
    app: Arc<App>,
    chat_id: String,
    bot_id: String,
}

impl TurnSessions {
    pub fn new(app: &Arc<App>, chat_id: &str, bot_id: &str) -> Self {
        TurnSessions { app: app.clone(), chat_id: chat_id.to_string(), bot_id: bot_id.to_string() }
    }
}

impl BashSessions for TurnSessions {
    fn insert(&self, call_id: &str, session: Arc<BashSession>) {
        self.app.shell_sessions.insert(&self.app, &self.chat_id, &self.bot_id, call_id, session);
    }

    fn get(&self, id: &str) -> Option<Arc<BashSession>> {
        self.app.shell_sessions.find(&self.chat_id, &self.bot_id, id)
    }

    fn remove(&self, id: &str) {
        self.app.shell_sessions.remove(&self.app, id);
    }
}
