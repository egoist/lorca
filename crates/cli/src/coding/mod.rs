//! Coding agents a bot runs on its Runner and supervises: Claude Code or Codex, in a git
//! worktree of a repository there, on a prompt and with the proof the bot expects back. The
//! agent is a process of its own that outlives the turn that started it. The bot keeps its id
//! across turns, reads its transcript, sends it follow-ups (queued, or cutting in), and stops it;
//! it gets a turn of its own when the agent is done, stalls, or exits, with the outputs Lorca
//! published from its work. Every command the agent runs goes through the bot's Access and
//! Auto-review, as the bot's own commands do, and its card in the chat shows the user how it
//! stands and what it asks.
//!
//! Where a terminal host runs on the Runner (Herdr, else Luvus), the agent runs in a pane of it,
//! which the user can watch and type into there; otherwise Lorca runs it on its own. Both are
//! driven through [`driver::Driver`], so the bot's tools and the card are the same either way.

pub(crate) mod claude;
pub(crate) mod codex;
pub(crate) mod driver;
pub(crate) mod hosts;
pub(crate) mod lines;
pub(crate) mod process;
pub(crate) mod proof;
pub(crate) mod screen;
pub(crate) mod tool;
pub(crate) mod workspace;

use std::collections::VecDeque;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use tokio::sync::{mpsc, oneshot};
use tokio::time::Instant;

use crate::app::App;
use crate::model::{AgentQuestion, AgentRun, Author, Body, Message};
use driver::{Answer, Approval, Driver, Event};

pub use hosts::{hook_answer, hook_command};
pub use tool::{review_start, CodingAgentTool};

/// The coding agents a bot can run, by the name the bot and the apps use.
pub const KINDS: [&str; 2] = ["claude", "codex"];
/// Working with nothing new for this long reads as stalled: the bot hears it, once.
pub const STALL_AFTER: Duration = Duration::from_secs(10 * 60);
/// An agent Lorca runs that is done with its work and hears nothing for this long stops; a
/// follow-up starts it again on its session.
pub const IDLE_LIMIT: Duration = Duration::from_secs(60 * 60);
/// How long a question on its card waits for the user.
const QUESTION_TIMEOUT: Duration = Duration::from_secs(10 * 60);
/// The card catches up with the transcript once it pauses this long, and at least this often.
const ROW_SETTLE: Duration = Duration::from_millis(500);
const ROW_EVERY: Duration = Duration::from_secs(2);
/// The lines the card carries.
const ROW_LINES: usize = 20;
/// The lines of transcript kept in memory; the file keeps them all.
const KEEP_LINES: usize = 5000;
/// What `coding.transcript` sends the apps, at most.
const APP_LINES: usize = 2000;
/// What the bot reads at once, at most.
const READ_LINES: usize = 200;
/// How often the pane of an agent in a terminal host is read while it works.
const SCREEN_EVERY: Duration = Duration::from_secs(3);
/// Ended agents are forgotten this long after they ended.
const FORGET_AFTER: f64 = 30.0 * 24.0 * 3600.0;

/// The agent's name in what Lorca writes: "Claude Code", "Codex".
pub fn name(kind: &str) -> &'static str {
    if kind == "codex" { "Codex" } else { "Claude Code" }
}

/// `~/dev/lorca` for a folder in the home folder.
pub(crate) fn home_relative(path: &Path) -> String {
    match dirs::home_dir().and_then(|home| path.strip_prefix(&home).ok().map(Path::to_path_buf)) {
        Some(rest) if rest.as_os_str().is_empty() => "~".into(),
        Some(rest) => format!("~/{}", rest.display()),
        None => path.display().to_string(),
    }
}

/// The user's answer to what an agent's pane asks: one of its choices, or text to type.
#[derive(Debug, Clone, PartialEq)]
pub(crate) enum Reply {
    Choice(usize),
    Text(String),
}

/// What the bot has yet to hear about its agent.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub(crate) enum News {
    /// It is done with what it was asked, and Lorca published these outputs from its work.
    Finished { said: Option<String>, outputs: Vec<proof::Reference> },
    /// It has shown nothing new for a while.
    Stalled,
    /// It exited on its own.
    Ended { outcome: String },
}

/// An agent as it survives a restart, in `agents/agents.json`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub(crate) struct Record {
    pub id: String,
    pub chat_id: String,
    pub bot_id: String,
    /// The `coding_agent` row that started it, which shows its card.
    pub message_id: String,
    /// The message behind the turn that started it: what Auto-review weighs its commands against.
    pub trigger_message_id: String,
    pub kind: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub host: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub target: Option<hosts::Target>,
    /// What resumes it: Claude Code's session, Codex's thread.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub session: Option<String>,
    pub folder: PathBuf,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub branch: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub base: Option<String>,
    pub task: String,
    pub state: String,
    #[serde(default)]
    pub stalled: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub outcome: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pull_request: Option<String>,
    pub started_at: f64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ended_at: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub news: Option<News>,
    #[serde(default)]
    pub published: proof::Published,
    /// The bot's command secrets it has in its environment, by name.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub secrets: Vec<String>,
    /// The event behind the turn that started it (a channel's message, a service's event): its
    /// commands are weighed against the owner's task for it, never against the event's text.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub event: Option<crate::event_triggers::EventTask>,
}

impl Record {
    /// What Auto-review weighs the agent's actions against: the turn that started it.
    fn trigger(&self) -> crate::plugins::review::Trigger {
        crate::plugins::review::Trigger { message_id: self.trigger_message_id.clone(), routine: None, event: self.event.clone() }
    }
}

/// `text` with this Runner's saved secrets replaced by their placeholders, as a tool's result
/// is: what an agent prints, shows, or is sent goes to the card, its log, the bot, and outputs.
pub(crate) fn without_secrets(app: &App, text: String) -> String {
    crate::secrets::Redactions::load(app).text(&text).unwrap_or(text)
}

/// The transcript: the lines kept, how many went before them, and how far the bot has read.
#[derive(Default)]
struct Transcript {
    lines: VecDeque<String>,
    dropped: usize,
    read: usize,
    /// What the pane of an agent in a terminal host shows, which stands for its lines.
    screen: Option<String>,
}

impl Transcript {
    fn total(&self) -> usize {
        self.dropped + self.lines.len()
    }

    fn tail(&self, count: usize) -> Vec<String> {
        match &self.screen {
            Some(screen) => {
                let screen = screen::body(screen);
                let lines: Vec<&str> = screen.lines().collect();
                let start = lines.len().saturating_sub(count);
                lines[start..].iter().map(|line| line.to_string()).collect()
            }
            None => self.lines.iter().skip(self.lines.len().saturating_sub(count)).cloned().collect(),
        }
    }
}

pub struct Agent {
    record: Mutex<Record>,
    driver: Mutex<Option<Arc<dyn Driver>>>,
    transcript: Mutex<Transcript>,
    /// What its card asks the user now.
    question: Mutex<Option<AgentQuestion>>,
    /// Where the user's answer to a question its pane asks goes.
    answer: Mutex<Option<oneshot::Sender<Reply>>>,
    /// Counts every question its pane asks and every time it stops asking.
    asked: AtomicU64,
    /// One question to the user at a time.
    asking: tokio::sync::Mutex<()>,
    /// When it last showed something new.
    active_at: Mutex<Instant>,
    /// Its work counts toward the limits of the turn that gave it the work, while it works.
    budget: Mutex<Option<crate::budgets::BudgetContext>>,
    hold: Mutex<Option<crate::budgets::RuntimeHold>>,
    /// The user or the bot is stopping it.
    stopping: AtomicBool,
    /// Which run of its process the supervisor follows: a resumed agent is a new one.
    generation: AtomicU64,
}

impl Agent {
    fn new(record: Record) -> Arc<Agent> {
        Arc::new(Agent {
            record: Mutex::new(record),
            driver: Mutex::new(None),
            transcript: Mutex::new(Transcript::default()),
            question: Mutex::new(None),
            answer: Mutex::new(None),
            asked: AtomicU64::new(0),
            asking: tokio::sync::Mutex::new(()),
            active_at: Mutex::new(Instant::now()),
            budget: Mutex::new(None),
            hold: Mutex::new(None),
            stopping: AtomicBool::new(false),
            generation: AtomicU64::new(0),
        })
    }

    pub fn id(&self) -> String {
        self.record.lock().unwrap().id.clone()
    }

    pub(crate) fn record(&self) -> Record {
        self.record.lock().unwrap().clone()
    }

    fn driver(&self) -> Option<Arc<dyn Driver>> {
        self.driver.lock().unwrap().clone()
    }

    /// It runs: a process or a pane follows it.
    pub fn is_running(&self) -> bool {
        self.driver().is_some()
    }

    fn is_open(&self) -> bool {
        matches!(self.record.lock().unwrap().state.as_str(), "checking" | "asking" | "starting" | "working" | "idle")
    }

    fn add_lines(&self, app: &App, new: Vec<String>) {
        if new.is_empty() {
            return;
        }
        let redactions = crate::secrets::Redactions::load(app);
        let new: Vec<String> = new.into_iter().map(|line| redactions.text(&line).unwrap_or(line)).collect();
        let id = self.id();
        append_log(&app.config.home, &id, &new);
        let mut transcript = self.transcript.lock().unwrap();
        transcript.lines.extend(new);
        while transcript.lines.len() > KEEP_LINES {
            transcript.lines.pop_front();
            transcript.dropped += 1;
        }
        drop(transcript);
        *self.active_at.lock().unwrap() = Instant::now();
    }

    /// The transcript the bot has not read, up to `READ_LINES` of its end, or its last `lines`.
    /// An agent in a terminal host reads as its pane.
    fn read(&self, lines: Option<usize>) -> (String, usize) {
        let mut transcript = self.transcript.lock().unwrap();
        if let Some(screen) = transcript.screen.clone() {
            let count = lines.unwrap_or(READ_LINES);
            let all: Vec<&str> = screen.lines().collect();
            return (all[all.len().saturating_sub(count)..].join("\n"), 0);
        }
        let total = transcript.total();
        let from = match lines {
            Some(count) => total.saturating_sub(count),
            None => transcript.read.max(total.saturating_sub(READ_LINES)),
        };
        let skipped = if lines.is_none() { from.saturating_sub(transcript.read) } else { 0 };
        let start = from.saturating_sub(transcript.dropped);
        let text = transcript.lines.iter().skip(start).cloned().collect::<Vec<_>>().join("\n");
        transcript.read = total;
        (text, skipped)
    }
}

/// The coding agents on this Runner.
#[derive(Default)]
pub struct Agents {
    agents: Mutex<Vec<Arc<Agent>>>,
    /// Held while a card is read and written, so the latest state lands last.
    rows: Mutex<()>,
    /// `coding_agent` calls that start an agent, and the rows that show them: chat, call, row.
    calls: Mutex<Vec<(String, String, String)>>,
}

impl Agents {
    /// A `coding_agent` call that starts an agent began, and row `message_id` shows its card.
    pub fn begin(&self, chat_id: &str, call_id: &str, message_id: &str) {
        self.calls.lock().unwrap().push((chat_id.to_string(), call_id.to_string(), message_id.to_string()));
    }

    /// The row that shows call `call_id`'s card.
    pub fn row(&self, chat_id: &str, call_id: &str) -> Option<String> {
        self.calls.lock().unwrap().iter().find(|(chat, call, _)| chat == chat_id && call == call_id).map(|(_, _, row)| row.clone())
    }

    /// Changes the card in row `message_id` and puts it up.
    pub fn update_row(&self, app: &App, chat_id: &str, message_id: &str, change: impl FnOnce(&mut AgentRun)) {
        let _row = self.rows.lock().unwrap();
        let Some(mut message) = app.message(chat_id, message_id) else { return };
        let Body::Tool { agent: Some(card), .. } = &mut message.body else { return };
        change(card);
        app.upsert_message(message, true);
    }

    /// Call `call_id` returned, and `finish` writes its result into row `message_id`, with the
    /// card as it stands in the same write. A call that started no agent ends its card here.
    pub fn call_ended(&self, app: &App, chat_id: &str, call_id: &str, message_id: &str, is_error: bool, text: &str, finish: impl FnOnce(&mut Message)) {
        self.calls.lock().unwrap().retain(|(chat, call, _)| !(chat == chat_id && call == call_id));
        let started = self.by_row(chat_id, message_id);
        let _row = self.rows.lock().unwrap();
        let Some(mut message) = app.message(chat_id, message_id) else { return };
        finish(&mut message);
        if let Body::Tool { agent: Some(card), .. } = &mut message.body {
            match started {
                Some(agent) => *card = self.card(app, &agent),
                None if card.is_open() => {
                    if !matches!(card.state.as_str(), "denied" | "expired" | "dismissed" | "stopped") {
                        card.state = if is_error { "failed" } else { "stopped" }.into();
                        card.outcome = text.lines().next().map(|line| line.trim().to_string()).filter(|line| !line.is_empty());
                    }
                    card.question = None;
                }
                None => {}
            }
        }
        app.upsert_message(message, true);
    }

    /// The turn ended before call `call_id` returned: a card no agent took over stops.
    pub fn abandon_call(&self, app: &App, chat_id: &str, call_id: &str, message_id: &str) {
        self.calls.lock().unwrap().retain(|(chat, call, _)| !(chat == chat_id && call == call_id));
        if self.by_row(chat_id, message_id).is_some() {
            return;
        }
        self.update_row(app, chat_id, message_id, |card| {
            if card.is_open() {
                card.state = "stopped".into();
                card.question = None;
                card.outcome = Some("Stopped".into());
            }
        });
    }

    fn all(&self) -> Vec<Arc<Agent>> {
        self.agents.lock().unwrap().clone()
    }

    fn by_row(&self, chat_id: &str, message_id: &str) -> Option<Arc<Agent>> {
        self.all().into_iter().find(|agent| {
            let record = agent.record.lock().unwrap();
            record.chat_id == chat_id && record.message_id == message_id
        })
    }

    /// The agent `id`, when bot `bot_id` started it in chat `chat_id`.
    pub fn find(&self, chat_id: &str, bot_id: &str, id: &str) -> Option<Arc<Agent>> {
        let id = id.trim();
        self.all().into_iter().find(|agent| {
            let record = agent.record.lock().unwrap();
            record.id == id && record.chat_id == chat_id && record.bot_id == bot_id
        })
    }

    /// The agents bot `bot_id` started in chat `chat_id`, oldest first.
    pub fn of(&self, chat_id: &str, bot_id: &str) -> Vec<Arc<Agent>> {
        self.all().into_iter().filter(|agent| {
            let record = agent.record.lock().unwrap();
            record.chat_id == chat_id && record.bot_id == bot_id
        }).collect()
    }

    /// The card for `agent`, as it stands.
    fn card(&self, app: &App, agent: &Agent) -> AgentRun {
        let record = agent.record();
        let question = agent.question.lock().unwrap().clone();
        let output = agent.transcript.lock().unwrap().tail(ROW_LINES).join("\n");
        let device = app.bot(&record.bot_id).and_then(|bot| app.device(&bot.runner_id)).map(|device| device.name);
        let open = matches!(record.state.as_str(), "starting" | "working" | "idle");
        // The card goes to every Device: what the agent printed, asks, or ended on, without
        // a saved secret, as a command's card.
        let redactions = crate::secrets::Redactions::load(app);
        let clean = |text: String| redactions.text(&text).unwrap_or(text);
        let question = question.map(|question| AgentQuestion {
            text: clean(question.text),
            command: question.command.map(clean),
            choices: question.choices.into_iter().map(clean).collect(),
            ..question
        });
        let mut run = AgentRun {
            id: record.id,
            kind: record.kind,
            host: record.host,
            task: record.task,
            folder: home_relative(&record.folder),
            branch: record.branch,
            state: if question.is_some() && open { "asking".into() } else { record.state },
            stalled: record.stalled,
            question: question.filter(|_| open),
            output: (!output.trim().is_empty()).then_some(output),
            outcome: record.outcome,
            pull_request: record.pull_request,
            device,
            started_at: Some(record.started_at),
        };
        run.task = clean(run.task);
        redactions.option(&mut run.output);
        redactions.option(&mut run.outcome);
        run
    }

    /// Writes where `agent` stands to its card.
    fn sync(&self, app: &App, agent: &Agent) {
        let (chat_id, message_id) = {
            let record = agent.record.lock().unwrap();
            (record.chat_id.clone(), record.message_id.clone())
        };
        let card = self.card(app, agent);
        let _row = self.rows.lock().unwrap();
        let Some(mut message) = app.message(&chat_id, &message_id) else { return };
        let Body::Tool { agent: Some(current), .. } = &mut message.body else { return };
        if *current == card {
            return;
        }
        *current = card;
        app.upsert_message(message, true);
    }

    /// Stop in the chat: every agent there stops, and its card says so.
    pub fn stop_chat(&self, app: &Arc<App>, chat_id: &str) {
        for agent in self.all().into_iter().filter(|agent| agent.record.lock().unwrap().chat_id == chat_id) {
            stop(app, &agent, "Stopped");
        }
    }

    /// Stops and forgets the agents whose chat is gone, or whose bot is no longer in it.
    pub fn close_orphans(&self, app: &App) {
        let members: Vec<(String, Vec<String>)> = app.state.lock().unwrap().chats.iter().map(|chat| (chat.meta.id.clone(), chat.meta.bot_ids.clone())).collect();
        let orphans: Vec<Arc<Agent>> = self
            .all()
            .into_iter()
            .filter(|agent| {
                let record = agent.record.lock().unwrap();
                !members.iter().any(|(chat_id, bot_ids)| *chat_id == record.chat_id && bot_ids.contains(&record.bot_id))
            })
            .collect();
        if orphans.is_empty() {
            return;
        }
        for agent in &orphans {
            agent.stopping.store(true, Ordering::SeqCst);
            if let Some(driver) = agent.driver.lock().unwrap().take() {
                tokio::spawn(async move { driver.stop().await });
            }
        }
        self.agents.lock().unwrap().retain(|agent| !orphans.iter().any(|orphan| Arc::ptr_eq(agent, orphan)));
        save(app);
    }

    /// Lorca is quitting: an agent Lorca runs stops, and its card says so; one in a terminal
    /// host goes on in its pane, and Lorca follows it again when it starts.
    pub async fn shutdown(&self, app: &App) {
        for agent in self.all() {
            let host = agent.record.lock().unwrap().host.is_some();
            if host {
                continue;
            }
            let Some(driver) = agent.driver.lock().unwrap().take() else { continue };
            agent.stopping.store(true, Ordering::SeqCst);
            driver.stop().await;
            {
                let mut record = agent.record.lock().unwrap();
                if matches!(record.state.as_str(), "starting" | "working") {
                    record.state = "stopped".into();
                    record.outcome = Some("Stopped when Lorca quit".into());
                    record.ended_at = Some(crate::config::now_secs());
                }
            }
            *agent.question.lock().unwrap() = None;
            self.sync(app, &agent);
        }
        save(app);
    }

    /// An agent Lorca runs is at work: a restart would cut it off.
    pub fn is_busy(&self) -> bool {
        self.all().iter().any(|agent| {
            let record = agent.record.lock().unwrap();
            record.host.is_none() && matches!(record.state.as_str(), "starting" | "working" | "asking")
        })
    }
}

/// The agents this Runner's last run left: their cards and handles stay. One Lorca ran stopped
/// with it, unless it was done with its work, which a follow-up resumes; one in a terminal host
/// is followed again.
pub fn load(app: &Arc<App>) {
    let path = records_path(app);
    let records: Vec<Record> = std::fs::read(&path).ok().and_then(|bytes| serde_json::from_slice(&bytes).ok()).unwrap_or_default();
    let now = crate::config::now_secs();
    let this_device = app.this_device_id();
    for mut record in records {
        if record.ended_at.is_some_and(|ended| now - ended > FORGET_AFTER) {
            continue;
        }
        if app.bot(&record.bot_id).is_none_or(|bot| Some(&bot.runner_id) != this_device.as_ref()) {
            continue;
        }
        let reattach = record.host.is_some() && record.target.is_some() && matches!(record.state.as_str(), "starting" | "working" | "idle" | "asking");
        if !reattach && matches!(record.state.as_str(), "checking" | "asking" | "starting" | "working") {
            record.state = "stopped".into();
            record.outcome = Some("Stopped when Lorca quit".into());
            record.ended_at = Some(now);
        }
        let agent = Agent::new(record.clone());
        let lines = read_log(app, &record.id);
        {
            let mut transcript = agent.transcript.lock().unwrap();
            transcript.dropped = lines.len().saturating_sub(KEEP_LINES);
            transcript.lines = lines.into_iter().skip(transcript.dropped).collect();
            transcript.read = transcript.total();
        }
        app.coding_agents.agents.lock().unwrap().push(agent.clone());
        app.coding_agents.sync(app, &agent);
        if reattach {
            let app = app.clone();
            tokio::spawn(async move {
                if let Err(error) = hosts::attach(&app, &agent).await {
                    ended(&app, &agent, driver::Ended { outcome: error, failed: false }, agent.generation.load(Ordering::SeqCst));
                }
            });
        }
    }
    save(app);
}

fn records_path(app: &App) -> PathBuf {
    app.config.home.join("agents").join("agents.json")
}

fn log_path(home: &Path, id: &str) -> PathBuf {
    home.join("agents").join(format!("{id}.log"))
}

/// Writes every agent's record.
pub(crate) fn save(app: &App) {
    let records: Vec<Record> = app.coding_agents.all().iter().map(|agent| agent.record()).collect();
    if let Err(error) = crate::config::write_json_private(&records_path(app), &records) {
        tracing::warn!(%error, "saving the coding agents");
    }
}

fn append_log(home: &Path, id: &str, lines: &[String]) {
    use std::io::Write;
    let path = log_path(home, id);
    let _ = std::fs::create_dir_all(path.parent().unwrap_or(home));
    let mut options = std::fs::OpenOptions::new();
    options.create(true).append(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    if let Ok(mut file) = options.open(&path) {
        let _ = file.write_all(format!("{}\n", lines.join("\n")).as_bytes());
    }
}

fn read_log(app: &App, id: &str) -> Vec<String> {
    std::fs::read_to_string(log_path(&app.config.home, id)).map(|text| text.lines().map(str::to_string).collect()).unwrap_or_default()
}

/// What starts an agent.
pub(crate) struct Start {
    pub chat_id: String,
    pub bot_id: String,
    pub message_id: String,
    pub trigger_message_id: String,
    pub kind: String,
    pub program: PathBuf,
    pub host: Option<hosts::Host>,
    pub place: workspace::Place,
    pub task: String,
    pub prompt: String,
    /// The bot's command secrets to give it as environment variables, by name.
    pub secrets: Vec<String>,
    pub event: Option<crate::event_triggers::EventTask>,
}

/// Starts an agent and follows it. It returns once the agent runs; its work goes on.
pub(crate) async fn start(app: &Arc<App>, start: Start) -> Result<Arc<Agent>, String> {
    let id = format!("agent-{}", &uuid::Uuid::new_v4().simple().to_string()[..8]);
    let record = Record {
        id: id.clone(),
        chat_id: start.chat_id.clone(),
        bot_id: start.bot_id.clone(),
        message_id: start.message_id.clone(),
        trigger_message_id: start.trigger_message_id.clone(),
        kind: start.kind.clone(),
        host: start.host.as_ref().map(|host| host.name().to_string()),
        target: None,
        session: None,
        folder: start.place.folder.clone(),
        branch: start.place.branch.clone(),
        base: start.place.base.clone(),
        task: without_secrets(app, start.task.clone()),
        state: "starting".into(),
        stalled: false,
        outcome: None,
        pull_request: None,
        started_at: crate::config::now_secs(),
        ended_at: None,
        news: None,
        published: proof::Published::default(),
        secrets: start.secrets.clone(),
        event: start.event.clone(),
    };
    let agent = Agent::new(record);
    *agent.budget.lock().unwrap() = crate::budgets::current();
    app.coding_agents.agents.lock().unwrap().push(agent.clone());
    save(app);
    app.coding_agents.sync(app, &agent);
    let launched = launch(app, &agent, &start.program, start.host.as_ref(), &start.prompt, None).await;
    if let Err(error) = &launched {
        let mut record = agent.record.lock().unwrap();
        record.state = "failed".into();
        record.outcome = Some(error.clone());
        record.ended_at = Some(crate::config::now_secs());
    }
    app.coding_agents.sync(app, &agent);
    save(app);
    launched.map(|_| agent)
}

/// Starts the agent's process or pane, on `prompt` (resuming `session`), with a supervisor that
/// follows it. An agent given secrets runs as Lorca's own process, which has them in its
/// environment; a pane's command line and screen would show them.
async fn launch(app: &Arc<App>, agent: &Arc<Agent>, program: &Path, host: Option<&hosts::Host>, prompt: &str, session: Option<&str>) -> Result<(), String> {
    use lorca_agent::tools::SecretVariables;
    let record = agent.record();
    // Looked up as it starts, so one the user deleted meanwhile is not given.
    let variables = match record.secrets.is_empty() {
        true => Vec::new(),
        false => crate::secrets::CommandSecrets { app: app.clone(), bot_id: record.bot_id.clone() }.variables(&record.secrets)?,
    };
    let host = host.filter(|_| variables.is_empty());
    let prompt = without_secrets(app, prompt.to_string());
    let (tx, rx) = mpsc::unbounded_channel();
    let generation = agent.generation.fetch_add(1, Ordering::SeqCst) + 1;
    agent.stopping.store(false, Ordering::SeqCst);
    tokio::spawn(supervise(app.clone(), agent.clone(), rx, generation));
    let driver = match host {
        Some(host) => hosts::start(app, agent, host, program, &prompt, tx).await?,
        None if record.kind == "codex" => codex::start(program, &record.folder, &prompt, session, &variables, tx).await?,
        None => claude::start(program, &record.folder, &prompt, session, &variables, tx).await?,
    };
    *agent.driver.lock().unwrap() = Some(driver);
    Ok(())
}

/// Gives the agent a message from the bot: to its running process or pane, or to a new process
/// on its session when its process is gone (it was done and stopped, or Lorca restarted).
pub(crate) async fn send(app: &Arc<App>, agent: &Arc<Agent>, text: &str, interrupt: bool) -> Result<String, String> {
    let text = &without_secrets(app, text.to_string());
    let record = agent.record();
    {
        let mut kept = agent.record.lock().unwrap();
        kept.news = None;
        kept.stalled = false;
    }
    *agent.budget.lock().unwrap() = crate::budgets::current();
    if let Some(driver) = agent.driver() {
        driver.send(text, interrupt).await?;
        *agent.active_at.lock().unwrap() = Instant::now();
        let busy = matches!(record.state.as_str(), "working" | "starting");
        return Ok(match (busy, interrupt) {
            (true, false) => format!("Sent. {} takes it up once it is done with what it is doing.", name(&record.kind)),
            (true, true) => format!("Sent. {} stopped what it was doing to take it up.", name(&record.kind)),
            _ => format!("Sent. {} is working on it.", name(&record.kind)),
        });
    }
    if record.host.is_some() {
        return Err(format!("{} is no longer running in its pane. Start a new one.", name(&record.kind)));
    }
    let program = process::find(&record.kind).await.ok_or_else(|| format!("{} is no longer installed on this Runner.", name(&record.kind)))?;
    if !record.folder.is_dir() {
        return Err(format!("{} is gone, so it cannot go on there.", home_relative(&record.folder)));
    }
    {
        let mut kept = agent.record.lock().unwrap();
        kept.state = "starting".into();
        kept.outcome = None;
        kept.ended_at = None;
    }
    app.coding_agents.sync(app, agent);
    match launch(app, agent, &program, None, text, record.session.as_deref()).await {
        Ok(()) => {
            save(app);
            Ok(format!("{} started again on its session and is working on it.", name(&record.kind)))
        }
        Err(error) => {
            {
                let mut kept = agent.record.lock().unwrap();
                kept.state = "failed".into();
                kept.outcome = Some(error.clone());
                kept.ended_at = Some(crate::config::now_secs());
            }
            app.coding_agents.sync(app, agent);
            save(app);
            Err(error)
        }
    }
}

/// Stops the agent; its card says `reason`.
pub(crate) fn stop(app: &Arc<App>, agent: &Arc<Agent>, reason: &str) {
    agent.stopping.store(true, Ordering::SeqCst);
    *agent.question.lock().unwrap() = None;
    agent.answer.lock().unwrap().take();
    let driver = agent.driver.lock().unwrap().take();
    {
        let mut record = agent.record.lock().unwrap();
        if matches!(record.state.as_str(), "checking" | "asking" | "starting" | "working" | "idle") {
            record.state = "stopped".into();
            record.outcome = Some(reason.to_string());
            record.ended_at = Some(crate::config::now_secs());
            record.news = None;
        }
    }
    agent.hold.lock().unwrap().take();
    if let Some(driver) = driver {
        tokio::spawn(async move { driver.stop().await });
    }
    app.coding_agents.sync(app, agent);
    save(app);
}

/// Follows one run of the agent: its events, the card, a stall, and the idle limit.
async fn supervise(app: Arc<App>, agent: Arc<Agent>, mut events: mpsc::UnboundedReceiver<Event>, generation: u64) {
    let agents = &app.coding_agents;
    let mut dirty = false;
    let mut last_sync = Instant::now();
    let mut next_screen = Instant::now() + SCREEN_EVERY;
    loop {
        if agent.generation.load(Ordering::SeqCst) != generation {
            return;
        }
        let now = Instant::now();
        let active_at = *agent.active_at.lock().unwrap();
        let (state, stalled, host) = {
            let record = agent.record.lock().unwrap();
            (record.state.clone(), record.stalled, record.host.is_some())
        };
        // Working with nothing new for a while: the bot hears it once.
        if state == "working" && !stalled && agent.question.lock().unwrap().is_none() && now >= active_at + STALL_AFTER {
            {
                let mut record = agent.record.lock().unwrap();
                record.stalled = true;
                record.news = Some(News::Stalled);
            }
            agents.sync(&app, &agent);
            save(&app);
            wake(&app, &agent);
        }
        // Done and quiet for long: an agent Lorca runs stops, and a follow-up resumes it.
        if state == "idle" && !host && agent.question.lock().unwrap().is_none() && now >= active_at + IDLE_LIMIT {
            if let Some(driver) = agent.driver.lock().unwrap().take() {
                agent.stopping.store(true, Ordering::SeqCst);
                tokio::spawn(async move { driver.stop().await });
            }
        }
        if host && matches!(state.as_str(), "working" | "starting") && now >= next_screen {
            next_screen = now + SCREEN_EVERY;
            dirty |= refresh_screen(&app, &agent).await;
        }
        let mut wake_at = now + Duration::from_secs(60);
        if state == "working" && !stalled {
            wake_at = wake_at.min(active_at + STALL_AFTER);
        }
        if state == "idle" && !host {
            wake_at = wake_at.min(active_at + IDLE_LIMIT);
        }
        if host && matches!(state.as_str(), "working" | "starting") {
            wake_at = wake_at.min(next_screen);
        }
        if dirty {
            let due = (active_at + ROW_SETTLE).min(last_sync + ROW_EVERY);
            if now >= due {
                agents.sync(&app, &agent);
                dirty = false;
                last_sync = now;
            } else {
                wake_at = wake_at.min(due);
            }
        }
        tokio::select! {
            event = events.recv() => {
                let Some(event) = event else { return };
                if agent.generation.load(Ordering::SeqCst) != generation {
                    return;
                }
                dirty |= handle(&app, &agent, event, generation).await;
            }
            _ = tokio::time::sleep_until(wake_at) => {}
        }
    }
}

/// One event of the agent's. True when only the transcript changed, which the card catches up
/// with in a moment; a change of state goes up at once.
async fn handle(app: &Arc<App>, agent: &Arc<Agent>, event: Event, generation: u64) -> bool {
    let agents = &app.coding_agents;
    match event {
        Event::Session(session) => {
            agent.record.lock().unwrap().session = Some(session);
            save(app);
            false
        }
        Event::Lines(new) => {
            if let Some(link) = new.iter().find_map(|line| proof::pull_request(line)) {
                agent.record.lock().unwrap().pull_request = Some(link);
            }
            agent.add_lines(app, new);
            unstall(app, agent);
            true
        }
        Event::Screen(screen) => {
            let screen = without_secrets(app, screen);
            let previous = agent.transcript.lock().unwrap().screen.clone();
            let changed = previous.as_deref() != Some(screen.as_str());
            if changed {
                if let Some(link) = screen.lines().filter_map(proof::pull_request).next_back() {
                    agent.record.lock().unwrap().pull_request = Some(link);
                }
                // A spinner's timer ticking is not progress: only a change past its digits is.
                let progressed = previous.is_none_or(|previous| without_digits(&previous) != without_digits(&screen));
                agent.transcript.lock().unwrap().screen = Some(screen);
                if progressed {
                    *agent.active_at.lock().unwrap() = Instant::now();
                    unstall(app, agent);
                }
            }
            changed
        }
        Event::Working => {
            // Whatever its pane asked, it has moved on.
            agent.asked.fetch_add(1, Ordering::SeqCst);
            let was = {
                let mut record = agent.record.lock().unwrap();
                let was = record.state.clone();
                record.state = "working".into();
                record.stalled = false;
                was
            };
            *agent.active_at.lock().unwrap() = Instant::now();
            if was != "working" {
                // Its run time counts toward the limits of the turn that gave it the work.
                let budget = agent.budget.lock().unwrap().clone();
                if let Some(budget) = budget {
                    match budget.hold() {
                        Ok(hold) => {
                            let deadline = hold.deadline();
                            *agent.hold.lock().unwrap() = Some(hold);
                            if let Some(deadline) = deadline {
                                let (app, agent) = (app.clone(), agent.clone());
                                tokio::spawn(async move {
                                    tokio::time::sleep(deadline).await;
                                    if agent.generation.load(Ordering::SeqCst) == generation && agent.record.lock().unwrap().state == "working" {
                                        stop(&app, &agent, "Stopped at the bot's time limit");
                                    }
                                });
                            }
                        }
                        Err(reason) => {
                            stop(app, agent, &reason);
                            return false;
                        }
                    }
                }
                agents.sync(app, agent);
                save(app);
            }
            false
        }
        Event::Idle(said) => {
            agent.asked.fetch_add(1, Ordering::SeqCst);
            {
                let mut record = agent.record.lock().unwrap();
                if !matches!(record.state.as_str(), "working" | "starting") {
                    return false;
                }
                record.state = "idle".into();
                record.stalled = false;
            }
            agent.hold.lock().unwrap().take();
            agents.sync(app, agent);
            // What it made is published as outputs, and the bot hears it.
            let outputs = proof::publish(app, agent).await;
            agent.record.lock().unwrap().news = Some(News::Finished { said, outputs });
            agents.sync(app, agent);
            save(app);
            wake(app, agent);
            false
        }
        Event::Approve(approval, reply) => {
            let (app, agent) = (app.clone(), agent.clone());
            tokio::spawn(async move {
                let verdict = approve(&app, &agent, approval).await;
                let _ = reply.send(verdict);
            });
            false
        }
        Event::Blocked(screen) => {
            agent.asked.fetch_add(1, Ordering::SeqCst);
            let (app, agent) = (app.clone(), agent.clone());
            tokio::spawn(async move { answer_screen(&app, &agent, screen).await });
            false
        }
        Event::Unblocked => {
            agent.asked.fetch_add(1, Ordering::SeqCst);
            false
        }
        Event::Ended(end) => {
            ended(app, agent, end, generation);
            false
        }
    }
}

fn without_digits(text: &str) -> String {
    text.chars().filter(|c| !c.is_ascii_digit()).collect()
}

/// The agent shows something new: it is no longer stalled.
fn unstall(app: &App, agent: &Agent) {
    let was = std::mem::replace(&mut agent.record.lock().unwrap().stalled, false);
    if was {
        app.coding_agents.sync(app, agent);
    }
}

/// Its process or pane is gone. One that went on its own tells the bot; one the bot or the user
/// stopped says so on its card, and one that was done with its work stays idle, to resume.
fn ended(app: &Arc<App>, agent: &Arc<Agent>, end: driver::Ended, generation: u64) {
    if agent.generation.load(Ordering::SeqCst) != generation {
        return;
    }
    agent.driver.lock().unwrap().take();
    agent.hold.lock().unwrap().take();
    *agent.question.lock().unwrap() = None;
    agent.answer.lock().unwrap().take();
    let stopping = agent.stopping.load(Ordering::SeqCst);
    let tell = {
        let mut record = agent.record.lock().unwrap();
        let host = record.host.is_some();
        match record.state.as_str() {
            // Stopped already, by the bot or the user.
            "stopped" | "failed" | "exited" | "denied" | "expired" | "dismissed" => false,
            // Done, and stopped while it waited: a follow-up resumes it.
            "idle" if stopping && !host => false,
            _ if stopping => {
                record.state = "stopped".into();
                record.outcome = Some(end.outcome.clone());
                record.ended_at = Some(crate::config::now_secs());
                false
            }
            _ => {
                record.state = if end.failed { "failed" } else { "exited" }.into();
                record.outcome = Some(end.outcome.clone());
                record.ended_at = Some(crate::config::now_secs());
                record.news = Some(News::Ended { outcome: end.outcome.clone() });
                true
            }
        }
    };
    app.coding_agents.sync(app, agent);
    save(app);
    if tell {
        wake(app, agent);
    }
}

/// The bot hears what its agent did, in a turn of its own once no turn runs in the chat, unless
/// a turn read it meanwhile (`coding_agent read`).
fn wake(app: &Arc<App>, agent: &Arc<Agent>) {
    let record = agent.record();
    if record.news.is_none() {
        return;
    }
    let job = crate::runtime::agent_job(app, &record.chat_id, &record.bot_id, &record.message_id);
    let (app, agent) = (app.clone(), agent.clone());
    tokio::spawn(async move {
        let unheard = {
            let lock = app.chat_lock(&job.chat_id);
            let _turn = lock.lock().await;
            agent.record.lock().unwrap().news.is_some()
        };
        if unheard {
            crate::runtime::start_turn(&app, job);
        }
    });
}

/// What an `agent` turn opens with: what the agent did that the bot has not heard. None when it
/// heard it already.
pub fn wake_cue(app: &App, job: &crate::model::Job) -> Option<String> {
    let agent = app.coding_agents.by_row(&job.chat_id, &job.trigger_message_id)?;
    let (record, news) = {
        let mut record = agent.record.lock().unwrap();
        let news = record.news.take()?;
        (record.clone(), news)
    };
    save(app);
    let who = format!("{} ({})", name(&record.kind), record.id);
    let place = match &record.branch {
        Some(branch) => format!("in {} on branch {branch}", home_relative(&record.folder)),
        None => format!("in {}", home_relative(&record.folder)),
    };
    let tail = || {
        let lines = agent.transcript.lock().unwrap().tail(12).join("\n");
        if lines.trim().is_empty() { String::new() } else { format!(" Its last lines:\n{lines}\n") }
    };
    let cue = match news {
        News::Finished { said, outputs } => {
            let said = said.map(|said| format!(" It said:\n{}\n", said.chars().take(4000).collect::<String>())).unwrap_or_default();
            let outputs = if outputs.is_empty() {
                String::new()
            } else {
                let list = outputs.iter().map(|output| format!("{} (message {}, output {}, version {})", output.name, output.message_id, output.output_id, output.version)).collect::<Vec<_>>().join("; ");
                format!(" Lorca published from its work: {list}. Cite them as task evidence by those references.")
            };
            format!(
                "[{who}, which you started {place} on “{}”, is done.{said}{outputs} Check its work against what you asked, then send it a follow-up with coding_agent send, or tell the user how it went.]",
                record.task
            )
        }
        News::Stalled => format!(
            "[{who}, working {place}, has shown nothing new for {} minutes; it may be stuck.{} Read its transcript with coding_agent read, send it a message (interrupt: true cuts in), or stop it.]",
            STALL_AFTER.as_secs() / 60,
            tail()
        ),
        News::Ended { outcome } => format!(
            "[{who}, which you started {place}, has ended: {outcome}.{} Tell the user, or send it a message to start it again on its session.]",
            tail()
        ),
    };
    Some(without_secrets(app, cue))
}

/// What Auto-review hears about an agent's environment: the saved secrets in it.
pub(crate) fn holds_secrets(names: &[String]) -> String {
    format!(
        "It runs with the user's saved secrets {} in its environment, which the user gave this bot for its commands; sending one anywhere but the service it is for leaks it.",
        names.iter().map(|name| format!("${name}")).collect::<Vec<_>>().join(", ")
    )
}

/// Whether the agent may do what it asks: the bot's Access as it stands, then Auto-review, and
/// the user on its card when the review asks.
async fn approve(app: &Arc<App>, agent: &Arc<Agent>, approval: Approval) -> Result<(), String> {
    // Neither the review nor the card sees a saved secret the agent wrote out.
    let approval = match approval {
        Approval::Command { command } => Approval::Command { command: without_secrets(app, command) },
        Approval::Files { paths } => Approval::Files { paths: paths.into_iter().map(|path| without_secrets(app, path)).collect() },
        Approval::Tool { name, mut input } => {
            crate::secrets::Redactions::load(app).json(&mut input);
            Approval::Tool { name, input }
        }
    };
    let record = agent.record();
    let who = name(&record.kind);
    let Some(bot) = app.bot(&record.bot_id) else { return Err("Its bot is gone.".into()) };
    if crate::permissions::check_tool(app, &bot, "bash").is_err() {
        return Err("Shell commands are off for the bot that started you, so nothing may run. Stop and say what you needed.".into());
    }
    let folder = std::fs::canonicalize(&record.folder).unwrap_or(record.folder.clone());
    let runner_name = app.device(&bot.runner_id).map(|device| device.name).unwrap_or_else(|| "this Runner".into());
    let auto_review = app.auto_review().is_enabled;
    let (tool, description, args, question) = match &approval {
        Approval::Command { command } => {
            // An agent holding saved secrets has each command reviewed, knowing which: any of
            // them can read its environment.
            if auto_review && record.secrets.is_empty() && crate::local_review::needs_no_review(app, command, &folder) {
                return Ok(());
            }
            let mut description = format!(
                "{who}, a coding agent this bot started, wants to run this shell command as the user on {runner_name}, with full filesystem, process, credential, and network access. Working directory: {}.",
                home_relative(&folder)
            );
            if !record.secrets.is_empty() {
                description.push_str(&format!(" {}", holds_secrets(&record.secrets)));
            }
            let question = AgentQuestion { kind: "command".into(), command: Some(command.chars().take(crate::model::APP_COMMAND_CHARS).collect()), ..Default::default() };
            ("bash", description, json!({ "command": command }), question)
        }
        Approval::Files { paths } => {
            let inside = |path: &String| {
                let path = Path::new(path);
                let path = if path.is_absolute() { path.to_path_buf() } else { folder.join(path) };
                path.starts_with(&folder) || path.starts_with(&record.folder)
            };
            if !paths.is_empty() && paths.iter().all(inside) {
                return Ok(());
            }
            let description = format!("{who}, a coding agent this bot started in {}, wants to change files outside that folder, as the user on {runner_name}.", home_relative(&folder));
            let shown = paths.iter().map(|path| home_relative(Path::new(path))).collect::<Vec<_>>().join("\n");
            let question = AgentQuestion { kind: "command".into(), command: Some(format!("Change {shown}")), ..Default::default() };
            ("edit", description, json!({ "paths": paths }), question)
        }
        Approval::Tool { name: tool, input } => {
            let description = format!("{who}, a coding agent this bot started in {}, wants to use its tool {tool} as the user on {runner_name}.", home_relative(&folder));
            let detail = lines::claude_detail(tool, input, &folder);
            let question = AgentQuestion { kind: "command".into(), command: Some(lines::call(tool, &detail).trim_start_matches("● ").to_string()), ..Default::default() };
            ("coding_agent_tool", description, json!({ "tool": tool, "input": input }), question)
        }
    };
    let trigger = record.trigger();
    let cancel = tokio_util::sync::CancellationToken::new();
    let propose_rule = matches!(approval, Approval::Command { .. });
    let action = crate::plugins::review::Action { target_name: &runner_name, tool, description: &description, args: &args, script: None, propose_rule };
    let crate::plugins::review::Outcome::Ask { reason, rule } = crate::plugins::review::review(app, &bot, &record.chat_id, &trigger, action, &cancel).await else {
        return Ok(());
    };
    // One question at a time, on its card.
    let _one = agent.asking.lock().await;
    if !agent.is_running() {
        return Err("Stopped".into());
    }
    let always_rule = rule.clone().map(|text| crate::model::AutoReviewRule { id: uuid::Uuid::new_v4().to_string(), text, behavior: "allow".into(), tool: None });
    let question = AgentQuestion { reason, rule: always_rule.as_ref().map(|rule| rule.text.clone()), ..question };
    let decision = ask_on_card(app, agent, question, always_rule).await;
    use crate::plugins::mcp::Decision;
    match decision {
        Decision::Allowed | Decision::Always => Ok(()),
        Decision::Denied => Err("The user did not allow this in Lorca. Do not retry it; go on without it or stop and say what you needed.".into()),
        Decision::Expired => Err("Nobody answered in time, so it did not run. Go on without it or stop and say what you needed.".into()),
        Decision::Dismissed => Err("The user wrote to the bot instead of answering, so it did not run. Go on without it or stop and say what you needed.".into()),
    }
}

/// Puts `question` on the agent's card and waits for the user's Allow or Deny from any Device
/// (`chats.permission` names the card's row), as a command's card does.
async fn ask_on_card(app: &Arc<App>, agent: &Arc<Agent>, question: AgentQuestion, always_rule: Option<crate::model::AutoReviewRule>) -> crate::plugins::mcp::Decision {
    let (chat_id, message_id) = {
        let record = agent.record.lock().unwrap();
        (record.chat_id.clone(), record.message_id.clone())
    };
    let cancel = tokio_util::sync::CancellationToken::new();
    // Stopping the agent stops the question.
    let watch = {
        let (agent, cancel) = (agent.clone(), cancel.clone());
        tokio::spawn(async move {
            while !cancel.is_cancelled() {
                if !agent.is_running() || !agent.is_open() {
                    cancel.cancel();
                    break;
                }
                tokio::time::sleep(Duration::from_millis(500)).await;
            }
        })
    };
    let decision = crate::plugins::mcp::await_answer(app, &chat_id, &message_id, always_rule, &cancel, || {
        *agent.question.lock().unwrap() = Some(question);
        app.coding_agents.sync(app, agent);
        if let Some(message) = app.message(&chat_id, &message_id) {
            crate::push::permission(app, &message);
        }
    })
    .await;
    cancel.cancel();
    watch.abort();
    *agent.question.lock().unwrap() = None;
    app.coding_agents.sync(app, agent);
    decision
}

/// The agent's pane asks something. A menu whose "Yes" lets it go ahead once goes to Auto-review,
/// which may answer it; anything else, or what the review asks about, goes to the user on its
/// card, and only their answer is pressed or typed into the pane. The user can also answer in
/// the pane itself, which ends the question.
async fn answer_screen(app: &Arc<App>, agent: &Arc<Agent>, screen: String) {
    let _one = agent.asking.lock().await;
    let asked = agent.asked.load(Ordering::SeqCst);
    let still = || agent.asked.load(Ordering::SeqCst) == asked && agent.is_running();
    if !still() {
        return;
    }
    let Some(driver) = agent.driver() else { return };
    // A question can show before the agent reads keys: it is read again once it holds still.
    let mut question = settled(&*driver, screen).await;
    question.text = without_secrets(app, question.text);
    question.choices = question.choices.into_iter().map(|choice| without_secrets(app, choice)).collect();
    let record = agent.record();
    let who = name(&record.kind);
    if let (Some(index), Some(bot)) = (question.allow_once(), app.bot(&record.bot_id)) {
        if crate::permissions::check_tool(app, &bot, "bash").is_ok() && app.auto_review().is_enabled {
            let runner_name = app.device(&bot.runner_id).map(|device| device.name).unwrap_or_else(|| "this Runner".into());
            let description = format!(
                "{who}, a coding agent this bot started in {} on {runner_name}, asks this on its screen. Allowing answers it with “{}”, which lets it go ahead once.",
                home_relative(&record.folder),
                question.choices[index]
            );
            let args = json!({ "question": question.text, "answer": question.choices[index] });
            let trigger = record.trigger();
            let action = crate::plugins::review::Action { target_name: &runner_name, tool: "coding_agent_question", description: &description, args: &args, script: None, propose_rule: false };
            let cancel = tokio_util::sync::CancellationToken::new();
            if crate::plugins::review::review(app, &bot, &record.chat_id, &trigger, action, &cancel).await == crate::plugins::review::Outcome::Allow {
                if !still() {
                    return;
                }
                match driver.answer(&Answer::Keys(question.keys_for(index))).await {
                    Ok(()) => {
                        // An answer that took moves the pane on; one that did not goes to the user.
                        for _ in 0..20 {
                            tokio::time::sleep(Duration::from_millis(200)).await;
                            if !still() {
                                return;
                            }
                        }
                    }
                    Err(error) => tracing::warn!(%error, "answering a coding agent's pane"),
                }
            }
        }
    }
    if !still() {
        return;
    }
    let kind = if question.choices.is_empty() { "text" } else { "choices" };
    let card = AgentQuestion { kind: kind.into(), text: question.text.clone(), choices: question.choices.clone(), ..Default::default() };
    let (tx, mut rx) = oneshot::channel();
    *agent.answer.lock().unwrap() = Some(tx);
    *agent.question.lock().unwrap() = Some(card);
    app.coding_agents.sync(app, agent);
    if let Some(message) = app.message(&record.chat_id, &record.message_id) {
        crate::push::permission(app, &message);
    }
    let deadline = Instant::now() + QUESTION_TIMEOUT;
    let answer = loop {
        tokio::select! {
            answer = &mut rx => break answer.ok(),
            _ = tokio::time::sleep(Duration::from_millis(500)) => {
                if !still() || Instant::now() >= deadline {
                    break None;
                }
            }
        }
    };
    agent.answer.lock().unwrap().take();
    *agent.question.lock().unwrap() = None;
    app.coding_agents.sync(app, agent);
    if let Some(reply) = answer {
        let answer = match reply {
            Reply::Choice(index) if index < question.choices.len() => Answer::Keys(question.keys_for(index)),
            Reply::Choice(_) => return,
            Reply::Text(text) => Answer::Text(text),
        };
        if let Err(error) = driver.answer(&answer).await {
            tracing::warn!(%error, "answering a coding agent's pane");
        }
    }
}

/// What a pane asks once its screen has held still for a moment.
async fn settled(driver: &dyn Driver, mut screen: String) -> screen::Question {
    for _ in 0..10 {
        tokio::time::sleep(Duration::from_millis(700)).await;
        match driver.screen_now().await {
            Some(now) if now == screen => break,
            Some(now) => screen = now,
            None => break,
        }
    }
    screen::read(&screen)
}

/// Reads the pane of an agent in a terminal host. True when it shows something new.
async fn refresh_screen(app: &Arc<App>, agent: &Arc<Agent>) -> bool {
    let Some(driver) = agent.driver() else { return false };
    match driver.screen().await {
        Some(screen) => handle(app, agent, Event::Screen(screen), agent.generation.load(Ordering::SeqCst)).await,
        None => false,
    }
}

/// `coding.review`, from `lorca coding hook` in the pane of agent `id`: whether it may run
/// `command`, decided as for a headless agent. Only this CLI's own local clients ask.
pub async fn review_for_hook(app: &Arc<App>, id: &str, command: &str) -> Result<Value, String> {
    let agent = app.coding_agents.all().into_iter().find(|agent| agent.id() == id).ok_or("Lorca runs no such coding agent")?;
    let verdict = approve(app, &agent, Approval::Command { command: command.to_string() }).await;
    Ok(json!({ "allow": verdict.is_ok(), "reason": verdict.err() }))
}

/// `coding.transcript`, `coding.stop`, `coding.answer`, and `coding.show` for a card on this
/// Runner, from the local app or a sealed request: `{ chat_id, message_id, choice?, text? }`.
pub async fn serve(app: &Arc<App>, verb: &str, body: &Value) -> Result<Value, String> {
    let chat_id = body["chat_id"].as_str().ok_or("missing chat_id")?;
    let message_id = body["message_id"].as_str().ok_or("missing message_id")?;
    let message = app.message(chat_id, message_id).ok_or("Unknown message")?;
    let Author::Bot { .. } = &message.author else { return Err("That row has no coding agent".into()) };
    let agent = app.coding_agents.by_row(chat_id, message_id).ok_or("That coding agent never started")?;
    match verb {
        "coding.transcript" => {
            if let Some(driver) = agent.driver() {
                if let Some(screen) = driver.screen().await {
                    agent.transcript.lock().unwrap().screen = Some(without_secrets(app, screen));
                }
            }
            let text = {
                let transcript = agent.transcript.lock().unwrap();
                match &transcript.screen {
                    Some(screen) => screen.clone(),
                    None => transcript.tail(APP_LINES).join("\n"),
                }
            };
            Ok(json!({ "text": text, "card": app.coding_agents.card(app, &agent) }))
        }
        "coding.stop" => {
            if !agent.is_open() {
                return Err("It has already ended".into());
            }
            stop(app, &agent, "Stopped");
            Ok(json!({ "stopped": true }))
        }
        "coding.answer" => {
            let answer = match (body["choice"].as_u64(), body["text"].as_str()) {
                (Some(choice), _) => Reply::Choice(choice as usize),
                (None, Some(text)) => Reply::Text(text.to_string()),
                _ => return Err("An answer is a choice or text".into()),
            };
            let waiting = agent.answer.lock().unwrap().take().ok_or("It no longer asks")?;
            waiting.send(answer).map_err(|_| "It no longer asks")?;
            Ok(json!({ "answered": true }))
        }
        "coding.show" => {
            let driver = agent.driver().ok_or("It is no longer running")?;
            driver.focus().await?;
            Ok(json!({ "shown": true }))
        }
        other => Err(format!("Unknown request {other}")),
    }
}

#[cfg(test)]
mod tests;
