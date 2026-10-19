//! Coding agents a bot hands jobs to on its Runner: Claude Code, Codex, OpenCode, pi, or fx, in a
//! folder there, as it is, on the user's request the bot passes on and under a title it gives the
//! job. The agent decides how to do the job, pushes it, and opens a pull request; the user reviews it.
//! The agent is a process of its own that outlives the turn that started it. The bot keeps its id
//! across turns, and when the user asks, reads its transcript, passes it what they say (queued, or
//! cutting in), and stops it. When the agent is done with what it was asked, what it said arrives
//! in the chat as its report to the user, and the bot gets a turn that only relays it: no checking,
//! building, testing, or new instructions; when it ends on its own, the bot gets a turn to tell
//! the user. Every command it runs
//! goes through the bot's Access and Auto-review, as the bot's own commands do, and what the
//! review asks the user shows on the row that started it.
//!
//! Every agent is driven through [`driver::Driver`] whatever it speaks (`claude`: stream-json,
//! `codex`: app-server, `acp`: the Agent Client Protocol for OpenCode and fx, `pi`: pi's RPC), and its
//! transcript is steps ([`transcript::Item`]) the apps draw the way Cursor draws an agent's run.

pub(crate) mod acp;
pub(crate) mod claude;
pub(crate) mod codex;
pub(crate) mod driver;
pub(crate) mod pi;
pub(crate) mod process;
pub mod settings;
pub(crate) mod tool;
pub mod transcript;
pub(crate) mod workspace;

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use tokio::sync::mpsc;
use tokio::time::Instant;

use crate::app::App;
use crate::model::{AgentChanges, AgentRun, Author, Body, Message};
use driver::{Approval, Driver, Event};
use transcript::{Item, Transcript};

pub use tool::CodingAgentTool;

pub use crate::model::CODING_AGENT_KINDS as KINDS;
/// An agent that is done with its work and hears nothing for this long stops; a follow-up starts
/// it again on its session.
pub const IDLE_LIMIT: Duration = Duration::from_secs(60 * 60);
/// The steps `coding.transcript` sends the apps, at most.
const APP_ITEMS: usize = 400;
/// What the bot reads at once, at most.
const READ_LINES: usize = 200;
/// How much of what an agent said its message in the chat carries.
const MESSAGE_CHARS: usize = 4000;
/// Ended agents are forgotten this long after they ended.
const FORGET_AFTER: f64 = 30.0 * 24.0 * 3600.0;

pub use crate::model::coding_agent_name as name;

/// `~/dev/lorca` for a folder in the home folder.
pub(crate) fn home_relative(path: &Path) -> String {
    match dirs::home_dir().and_then(|home| path.strip_prefix(&home).ok().map(Path::to_path_buf)) {
        Some(rest) if rest.as_os_str().is_empty() => "~".into(),
        Some(rest) => format!("~/{}", rest.display()),
        None => path.display().to_string(),
    }
}

/// An agent as it survives a restart, in `agents/agents.json`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub(crate) struct Record {
    pub id: String,
    pub chat_id: String,
    pub bot_id: String,
    /// The `coding_agent` row that started it, which carries its card.
    pub message_id: String,
    /// The message behind the turn that started it: what Auto-review weighs its commands against.
    pub trigger_message_id: String,
    pub kind: String,
    pub title: String,
    /// What resumes it: Claude Code's session, Codex's thread, OpenCode's or fx's session, pi's
    /// session file.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub session: Option<String>,
    pub folder: PathBuf,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub branch: Option<String>,
    /// The commit it started from, which its changes count from.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub base: Option<String>,
    pub task: String,
    pub state: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub outcome: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pull_request: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub changes: Option<AgentChanges>,
    pub started_at: f64,
    #[serde(default)]
    pub updated_at: f64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ended_at: Option<f64>,
    /// Its message in the chat the bot has yet to hear about: the turn it wakes for it, unless a
    /// turn read the agent meanwhile.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub unheard: Option<String>,
    /// The bot's command secrets it has in its environment, by name.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub secrets: Vec<String>,
    /// The event behind the turn that started it (a channel's message, a service's event): its
    /// commands are weighed against the owner's task for it, never against the event's text.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub event: Option<crate::event_triggers::EventTask>,
    /// A routine's run or an event started it, with nobody watching: what its review would ask
    /// goes back to it at once, as an unattended turn's does, instead of waiting on a card.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub unattended: bool,
}

impl Record {
    /// What Auto-review weighs the agent's actions against: the turn that started it.
    fn trigger(&self) -> crate::plugins::review::Trigger {
        crate::plugins::review::Trigger { message_id: self.trigger_message_id.clone(), routine: None, event: self.event.clone() }
    }

    fn set_state(&mut self, state: &str) {
        self.state = state.into();
        self.updated_at = crate::config::now_secs();
    }
}

/// `text` with this Runner's saved secrets replaced by their placeholders, as a tool's result
/// is: what an agent prints, shows, or is sent goes to its transcript, the chat, and the bot.
pub(crate) fn without_secrets(app: &App, text: String) -> String {
    crate::secrets::Redactions::load(app).text(&text).unwrap_or(text)
}

/// A step without the Runner's saved secrets in it.
fn item_without_secrets(app: &App, mut item: Item) -> Item {
    let redactions = crate::secrets::Redactions::load(app);
    let clean = |text: &mut String| {
        if let Some(clean) = redactions.text(text) {
            *text = clean;
        }
    };
    clean(&mut item.text);
    if let Some(step) = item.tool.as_mut() {
        clean(&mut step.title);
        if let Some(output) = step.output.as_mut() {
            clean(output);
        }
    }
    for entry in &mut item.plan {
        clean(&mut entry.text);
    }
    item
}

pub struct Agent {
    record: Mutex<Record>,
    driver: Mutex<Option<Arc<dyn Driver>>>,
    transcript: Mutex<Transcript>,
    /// A question of its waits for the user in the chat.
    asks: AtomicBool,
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
    fn new(record: Record, transcript: Transcript) -> Arc<Agent> {
        Arc::new(Agent {
            record: Mutex::new(record),
            driver: Mutex::new(None),
            transcript: Mutex::new(transcript),
            asks: AtomicBool::new(false),
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

    /// It runs: a process follows it.
    pub fn is_running(&self) -> bool {
        self.driver().is_some()
    }

    fn is_open(&self) -> bool {
        matches!(self.record.lock().unwrap().state.as_str(), "starting" | "working" | "idle")
    }

    /// Puts a step in its transcript and its file. True when it changed anything.
    fn add(&self, app: &App, item: Item) -> bool {
        let item = item_without_secrets(app, item);
        let pull_request = match &item.tool {
            Some(step) => step.output.as_deref().and_then(workspace::pull_request),
            None => workspace::pull_request(&item.text),
        };
        let changed = self.transcript.lock().unwrap().upsert(item.clone());
        if !changed {
            return false;
        }
        transcript::append(&app.config.home, &self.id(), &item);
        *self.active_at.lock().unwrap() = Instant::now();
        if let Some(link) = pull_request {
            self.record.lock().unwrap().pull_request = Some(link);
        }
        true
    }

    /// The transcript the bot has not read, up to `READ_LINES` of its end, or its last `lines`.
    fn read(&self, lines: Option<usize>) -> (String, usize) {
        self.transcript.lock().unwrap().read(lines, READ_LINES)
    }
}

/// The coding agents on this Runner.
#[derive(Default)]
pub struct Agents {
    agents: Mutex<Vec<Arc<Agent>>>,
    /// Held while a row is read and written, so the latest state lands last.
    rows: Mutex<()>,
    /// `coding_agent` calls that start an agent, and the rows that show them: chat, call, row.
    calls: Mutex<Vec<(String, String, String)>>,
}

impl Agents {
    /// A `coding_agent` call that starts an agent began, and row `message_id` carries its card.
    pub fn begin(&self, chat_id: &str, call_id: &str, message_id: &str) {
        self.calls.lock().unwrap().push((chat_id.to_string(), call_id.to_string(), message_id.to_string()));
    }

    /// The row that carries call `call_id`'s card.
    pub fn row(&self, chat_id: &str, call_id: &str) -> Option<String> {
        self.calls.lock().unwrap().iter().find(|(chat, call, _)| chat == chat_id && call == call_id).map(|(_, _, row)| row.clone())
    }

    /// Call `call_id` returned, and `finish` writes its result into row `message_id`, with the
    /// card as it stands in the same write. A call that started no agent says why on its card.
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
                    card.state = if is_error { "failed" } else { "stopped" }.into();
                    card.outcome = text.lines().next().map(|line| line.trim().to_string()).filter(|line| !line.is_empty());
                    card.updated_at = Some(crate::config::now_secs());
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
        let _row = self.rows.lock().unwrap();
        let Some(mut message) = app.message(chat_id, message_id) else { return };
        let Body::Tool { agent: Some(card), .. } = &mut message.body else { return };
        if card.is_open() {
            card.state = "stopped".into();
            card.outcome = Some("Stopped".into());
            card.updated_at = Some(crate::config::now_secs());
            app.upsert_message(message, true);
        }
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
        self.all()
            .into_iter()
            .filter(|agent| {
                let record = agent.record.lock().unwrap();
                record.chat_id == chat_id && record.bot_id == bot_id
            })
            .collect()
    }

    /// The card for `agent`, as it stands.
    fn card(&self, app: &App, agent: &Agent) -> AgentRun {
        let record = agent.record();
        let asks = agent.asks.load(Ordering::SeqCst) && (record.state == "working" || record.state == "starting");
        let device = app.bot(&record.bot_id).and_then(|bot| app.device(&bot.runner_id)).map(|device| device.name);
        // The card goes to every Device: what the agent was asked, asks, or ended on, without a
        // saved secret, as a command's card.
        let redactions = crate::secrets::Redactions::load(app);
        let clean = |text: String| redactions.text(&text).unwrap_or(text);
        AgentRun {
            id: record.id,
            kind: record.kind,
            title: clean(record.title),
            task: clean(record.task),
            folder: home_relative(&record.folder),
            branch: record.branch,
            state: if asks { "asking".into() } else { record.state },
            outcome: record.outcome.map(clean),
            pull_request: record.pull_request,
            changes: record.changes,
            device,
            started_at: Some(record.started_at),
            updated_at: Some(record.updated_at),
        }
    }

    /// Writes where `agent` stands to its card.
    fn sync(&self, app: &App, agent: &Agent) {
        let (chat_id, message_id) = {
            let record = agent.record.lock().unwrap();
            (record.chat_id.clone(), record.message_id.clone())
        };
        // Read under the lock, so a sync that read the agent earlier cannot write over a later one.
        let _row = self.rows.lock().unwrap();
        let card = self.card(app, agent);
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

    /// Lorca is quitting: every agent stops, and the card of one at work says so; one that was
    /// done waits to be resumed by a follow-up.
    pub async fn shutdown(&self, app: &App) {
        for agent in self.all() {
            let Some(driver) = agent.driver.lock().unwrap().take() else { continue };
            agent.stopping.store(true, Ordering::SeqCst);
            driver.stop().await;
            {
                let mut record = agent.record.lock().unwrap();
                if matches!(record.state.as_str(), "starting" | "working") {
                    record.set_state("stopped");
                    record.outcome = Some("Stopped when Lorca quit".into());
                    record.ended_at = Some(crate::config::now_secs());
                }
            }
            agent.asks.store(false, Ordering::SeqCst);
            self.sync(app, &agent);
        }
        save(app);
    }

    /// An agent is at work: a restart would cut it off.
    pub fn is_busy(&self) -> bool {
        self.all().iter().any(|agent| matches!(agent.record.lock().unwrap().state.as_str(), "starting" | "working"))
    }
}

/// The agents this Runner's last run left: their cards and handles stay. One that was at work
/// stopped with Lorca; one that was done waits for a follow-up, which resumes its session.
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
        if matches!(record.state.as_str(), "starting" | "working") {
            record.set_state("stopped");
            record.outcome = Some("Stopped when Lorca quit".into());
            record.ended_at = Some(now);
        }
        let transcript = Transcript::load(&transcript::file(&app.config.home, &record.id));
        let agent = Agent::new(record, transcript);
        app.coding_agents.agents.lock().unwrap().push(agent.clone());
        app.coding_agents.sync(app, &agent);
    }
    save(app);
}

fn records_path(app: &App) -> PathBuf {
    app.config.home.join("agents").join("agents.json")
}

/// Writes every agent's record.
pub(crate) fn save(app: &App) {
    let records: Vec<Record> = app.coding_agents.all().iter().map(|agent| agent.record()).collect();
    if let Err(error) = crate::config::write_json_private(&records_path(app), &records) {
        tracing::warn!(%error, "saving the coding agents");
    }
}

/// What starts an agent.
pub(crate) struct Start {
    pub chat_id: String,
    pub bot_id: String,
    pub message_id: String,
    pub trigger_message_id: String,
    pub kind: String,
    pub program: PathBuf,
    pub place: workspace::Place,
    pub title: String,
    pub task: String,
    pub prompt: String,
    /// The bot's command secrets to give it as environment variables, by name.
    pub secrets: Vec<String>,
    pub event: Option<crate::event_triggers::EventTask>,
    pub unattended: bool,
}

/// Starts an agent and follows it. It returns once the agent runs; its work goes on.
pub(crate) async fn start(app: &Arc<App>, start: Start) -> Result<Arc<Agent>, String> {
    let id = format!("agent-{}", &uuid::Uuid::new_v4().simple().to_string()[..8]);
    let now = crate::config::now_secs();
    let record = Record {
        id: id.clone(),
        chat_id: start.chat_id.clone(),
        bot_id: start.bot_id.clone(),
        message_id: start.message_id.clone(),
        trigger_message_id: start.trigger_message_id.clone(),
        kind: start.kind.clone(),
        title: without_secrets(app, start.title.clone()),
        session: None,
        folder: start.place.folder.clone(),
        branch: start.place.branch.clone(),
        base: start.place.base.clone(),
        task: without_secrets(app, start.task.clone()),
        state: "starting".into(),
        outcome: None,
        pull_request: None,
        changes: None,
        started_at: now,
        updated_at: now,
        ended_at: None,
        unheard: None,
        secrets: start.secrets.clone(),
        event: start.event.clone(),
        unattended: start.unattended,
    };
    let agent = Agent::new(record, Transcript::default());
    *agent.budget.lock().unwrap() = crate::budgets::current();
    app.coding_agents.agents.lock().unwrap().push(agent.clone());
    let place = match &start.place.branch {
        Some(branch) => format!("Working in {} on {branch}", home_relative(&start.place.folder)),
        None => format!("Working in {}", home_relative(&start.place.folder)),
    };
    agent.add(app, Item::text("place", "status", &place));
    save(app);
    app.coding_agents.sync(app, &agent);
    let launched = launch(app, &agent, &start.program, &start.prompt, None).await;
    if let Err(error) = &launched {
        let mut record = agent.record.lock().unwrap();
        record.set_state("failed");
        record.outcome = Some(error.clone());
        record.ended_at = Some(crate::config::now_secs());
    }
    app.coding_agents.sync(app, &agent);
    save(app);
    launched.map(|_| agent)
}

/// Starts the agent's process on `prompt` (resuming `session`), with a supervisor that follows it.
async fn launch(app: &Arc<App>, agent: &Arc<Agent>, program: &Path, prompt: &str, session: Option<&str>) -> Result<(), String> {
    use lorca_agent::tools::SecretVariables;
    let record = agent.record();
    // Looked up as it starts, so one the user deleted meanwhile is not given.
    let variables = match record.secrets.is_empty() {
        true => Vec::new(),
        false => crate::secrets::CommandSecrets { app: app.clone(), bot_id: record.bot_id.clone() }.variables(&record.secrets)?,
    };
    let prompt = without_secrets(app, prompt.to_string());
    let (tx, rx) = mpsc::unbounded_channel();
    let generation = agent.generation.fetch_add(1, Ordering::SeqCst) + 1;
    agent.stopping.store(false, Ordering::SeqCst);
    tokio::spawn(supervise(app.clone(), agent.clone(), rx, generation));
    let folder = &record.folder;
    let driver = match record.kind.as_str() {
        "codex" => codex::start(program, folder, &prompt, session, &variables, tx).await?,
        "opencode" => {
            let mut variables = variables;
            variables.push(("OPENCODE_CONFIG_CONTENT".into(), settings::OPENCODE_PERMISSIONS.into()));
            let launch = acp::Launch { name: "OpenCode", program, args: vec!["acp".into()], folder, variables };
            acp::start(launch, &prompt, session, tx).await?
        }
        "pi" => pi::start(&app.config.home, program, folder, &prompt, session, &variables, tx).await?,
        "fx" => {
            let mut variables = variables;
            variables.push(("FX_PERMISSION_MODE".into(), settings::FX_PERMISSION_MODE.into()));
            let launch = acp::Launch { name: "fx", program, args: vec!["acp".into()], folder, variables };
            acp::start(launch, &prompt, session, tx).await?
        }
        _ => claude::start(program, folder, &prompt, session, &variables, tx).await?,
    };
    *agent.driver.lock().unwrap() = Some(driver);
    Ok(())
}

/// Gives the agent a message from the bot: to its running process, or to a new process on its
/// session when its process is gone (it was done and stopped, or Lorca restarted).
pub(crate) async fn send(app: &Arc<App>, agent: &Arc<Agent>, text: &str, interrupt: bool) -> Result<String, String> {
    let text = &without_secrets(app, text.to_string());
    let record = agent.record();
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
    let program = settings::find(&record.kind).await.ok_or_else(|| format!("{} is no longer installed on this Runner.", name(&record.kind)))?;
    if !record.folder.is_dir() {
        return Err(format!("{} is gone, so it cannot go on there.", home_relative(&record.folder)));
    }
    {
        let mut kept = agent.record.lock().unwrap();
        kept.set_state("starting");
        kept.outcome = None;
        kept.ended_at = None;
    }
    app.coding_agents.sync(app, agent);
    match launch(app, agent, &program, text, record.session.as_deref()).await {
        Ok(()) => {
            save(app);
            Ok(format!("{} started again on its session and is working on it.", name(&record.kind)))
        }
        Err(error) => {
            {
                let mut kept = agent.record.lock().unwrap();
                kept.set_state("failed");
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
    agent.asks.store(false, Ordering::SeqCst);
    let driver = agent.driver.lock().unwrap().take();
    {
        let mut record = agent.record.lock().unwrap();
        if matches!(record.state.as_str(), "starting" | "working" | "idle") {
            record.set_state("stopped");
            record.outcome = Some(reason.to_string());
            record.ended_at = Some(crate::config::now_secs());
        }
    }
    agent.hold.lock().unwrap().take();
    if let Some(driver) = driver {
        tokio::spawn(async move { driver.stop().await });
    }
    app.coding_agents.sync(app, agent);
    save(app);
}

/// Follows one run of the agent: its events, and the idle limit.
async fn supervise(app: Arc<App>, agent: Arc<Agent>, mut events: mpsc::UnboundedReceiver<Event>, generation: u64) {
    loop {
        if agent.generation.load(Ordering::SeqCst) != generation {
            return;
        }
        let active_at = *agent.active_at.lock().unwrap();
        let idle = agent.record.lock().unwrap().state == "idle";
        // Done and quiet for long: it stops, and a follow-up resumes it.
        if idle && !agent.asks.load(Ordering::SeqCst) && Instant::now() >= active_at + IDLE_LIMIT {
            if let Some(driver) = agent.driver.lock().unwrap().take() {
                agent.stopping.store(true, Ordering::SeqCst);
                tokio::spawn(async move { driver.stop().await });
            }
        }
        let wake_at = if idle { active_at + IDLE_LIMIT } else { Instant::now() + Duration::from_secs(60) };
        tokio::select! {
            event = events.recv() => {
                let Some(event) = event else { return };
                if agent.generation.load(Ordering::SeqCst) != generation {
                    return;
                }
                handle(&app, &agent, event, generation).await;
            }
            _ = tokio::time::sleep_until(wake_at) => {}
        }
    }
}

/// One event of the agent's.
async fn handle(app: &Arc<App>, agent: &Arc<Agent>, event: Event, generation: u64) {
    let agents = &app.coding_agents;
    match event {
        Event::Session(session) => {
            agent.record.lock().unwrap().session = Some(session);
            save(app);
        }
        Event::Item(item) => {
            let had = agent.record.lock().unwrap().pull_request.clone();
            agent.add(app, item);
            if agent.record.lock().unwrap().pull_request != had {
                agents.sync(app, agent);
                save(app);
            }
        }
        Event::Working => {
            let was = {
                let mut record = agent.record.lock().unwrap();
                let was = record.state.clone();
                record.set_state("working");
                was
            };
            *agent.active_at.lock().unwrap() = Instant::now();
            if was == "working" {
                return;
            }
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
                        return;
                    }
                }
            }
            agents.sync(app, agent);
            save(app);
        }
        Event::Idle(said) => {
            {
                let mut record = agent.record.lock().unwrap();
                if !matches!(record.state.as_str(), "working" | "starting") {
                    return;
                }
                record.set_state("idle");
            }
            agent.hold.lock().unwrap().take();
            let (folder, base) = {
                let record = agent.record.lock().unwrap();
                (record.folder.clone(), record.base.clone())
            };
            if let Some(base) = base {
                // The agent may have moved to a branch of its own.
                let (changes, branch) = (workspace::changes(&folder, &base).await, workspace::branch(&folder).await);
                let mut record = agent.record.lock().unwrap();
                record.changes = changes;
                record.branch = branch.or(record.branch.take());
            }
            let said = said.or_else(|| agent.transcript.lock().unwrap().last_said()).unwrap_or_else(|| "Done.".into());
            tell(app, agent, &said, false);
        }
        // A turn that ended in an error ends the agent as one that ended on its own: its process
        // stops, a later message resumes its session, and its bot hears why.
        Event::Failed(reason) => {
            {
                let mut record = agent.record.lock().unwrap();
                if !matches!(record.state.as_str(), "working" | "starting") {
                    return;
                }
                record.set_state("failed");
                record.outcome = Some(reason.clone());
                record.ended_at = Some(crate::config::now_secs());
            }
            agent.hold.lock().unwrap().take();
            agent.asks.store(false, Ordering::SeqCst);
            if let Some(driver) = agent.driver.lock().unwrap().take() {
                agent.stopping.store(true, Ordering::SeqCst);
                tokio::spawn(async move { driver.stop().await });
            }
            tell(app, agent, &reason, true);
        }
        Event::Approve(approval, reply) => {
            let (app, agent) = (app.clone(), agent.clone());
            tokio::spawn(async move {
                let verdict = approve(&app, &agent, approval).await;
                let _ = reply.send(verdict);
            });
        }
        Event::Ended(end) => ended(app, agent, end, generation),
    }
}

/// Its process is gone. One that went on its own tells the bot; one the bot or the user stopped
/// says so on its card, and one that was done with its work stays idle, to resume.
fn ended(app: &Arc<App>, agent: &Arc<Agent>, end: driver::Ended, generation: u64) {
    if agent.generation.load(Ordering::SeqCst) != generation {
        return;
    }
    agent.driver.lock().unwrap().take();
    agent.hold.lock().unwrap().take();
    agent.asks.store(false, Ordering::SeqCst);
    let stopping = agent.stopping.load(Ordering::SeqCst);
    let tell_bot = {
        let mut record = agent.record.lock().unwrap();
        match record.state.as_str() {
            // Stopped already, by the bot or the user.
            "stopped" | "failed" => false,
            // Done: a follow-up resumes it, whether it stopped for being quiet or exited.
            "idle" => false,
            _ if stopping => {
                record.set_state("stopped");
                record.outcome = Some(end.outcome.clone());
                record.ended_at = Some(crate::config::now_secs());
                false
            }
            _ => {
                record.set_state("failed");
                record.outcome = Some(end.outcome.clone());
                record.ended_at = Some(crate::config::now_secs());
                true
            }
        }
    };
    if tell_bot {
        tell(app, agent, &end.outcome, true);
    } else {
        app.coding_agents.sync(app, agent);
        save(app);
    }
}

/// Puts what the agent said, or why it ended, in the chat as a message from it, and wakes its bot
/// to hear it. What it said when done is its report to the user, which alerts as a reply does,
/// and the bot's turn for it only relays it.
fn tell(app: &Arc<App>, agent: &Arc<Agent>, text: &str, failed: bool) {
    let record = agent.record();
    let mut text = without_secrets(app, text.trim().to_string());
    if text.chars().count() > MESSAGE_CHARS {
        text = format!("{}…", text.chars().take(MESSAGE_CHARS).collect::<String>());
    }
    let notice = format!("{}: {text}", record.title);
    let message = Message::new(
        &record.chat_id,
        Author::Bot { bot_id: record.bot_id.clone() },
        Body::AgentMessage { agent_id: record.id.clone(), agent_kind: record.kind.clone(), title: record.title.clone(), text, failed },
    );
    let message_id = message.id.clone();
    app.upsert_message(message, true);
    agent.record.lock().unwrap().unheard = Some(message_id.clone());
    app.coding_agents.sync(app, agent);
    save(app);
    // Done: the user reviews the work from its report, which alerts on its own.
    if !failed {
        if let (Some(chat), Some(bot)) = (app.chat(&record.chat_id), app.bot(&record.bot_id)) {
            crate::push::reply(app, &chat, &bot, &notice);
        }
    }
    let job = crate::runtime::agent_job(app, &record.chat_id, &record.bot_id, &message_id);
    let (app, agent) = (app.clone(), agent.clone());
    tokio::spawn(async move {
        let unheard = {
            let lock = app.chat_lock(&job.chat_id);
            let _turn = lock.lock().await;
            agent.record.lock().unwrap().unheard.as_deref() == Some(message_id.as_str())
        };
        if unheard {
            crate::runtime::start_turn(&app, job);
        }
    });
}

/// What an `agent` turn opens with, beside the message the bot reads in the chat. For an agent
/// that is done, to relay its report to the user and nothing more: the user reviews the work. For
/// one that ended on its own, why, and what to do. None when a turn heard it already.
pub fn wake_cue(app: &App, job: &crate::model::Job) -> Option<String> {
    let agent = app.coding_agents.all().into_iter().find(|agent| agent.record.lock().unwrap().unheard.as_deref() == Some(job.trigger_message_id.as_str()))?;
    let record = {
        let mut record = agent.record.lock().unwrap();
        record.unheard = None;
        record.clone()
    };
    save(app);
    // The message that woke it says whether the agent was done or ended on its own.
    let failed = match app.message(&job.chat_id, &job.trigger_message_id).map(|message| message.body) {
        Some(Body::AgentMessage { failed, .. }) => failed,
        _ => record.state == "failed",
    };
    Some(heard_cue(app, &record, &agent, failed))
}

/// What the bot hears about the agent, from the message that woke it: its report when done,
/// why it ended when `failed`.
pub(crate) fn heard_cue(app: &App, record: &Record, agent: &Agent, failed: bool) -> String {
    let who = format!("{} ({}), the coding agent you started on “{}”", name(&record.kind), record.id, record.title);
    // Why it ended, unless a message started it again before a turn heard it.
    if failed && record.state != "failed" {
        let cue = format!(
            "[{who}, ended on its own; its message in the chat says why. It has been started again since and is {}. Tell the user why it \
             ended in a line or two, unless you have already; do not start it again.]",
            match record.state.as_str() {
                "idle" => "done",
                "stopped" => "stopped again",
                state => state,
            }
        );
        return without_secrets(app, cue);
    }
    let cue = if failed {
        let tail = agent.transcript.lock().unwrap().last_lines(12);
        let tail = if tail.trim().is_empty() { String::new() } else { format!(" Its last lines:\n{tail}\n") };
        format!(
            "[{who}, has ended on its own: {}. Its last message in the chat says why.{tail} Tell the user why in a line or two. When it ended \
             for a passing reason, such as a lost connection, you may start it again on its session with coding_agent send and a message \
             that asks it to go on, adding nothing of your own; do not take the job over or change it.]",
            record.outcome.as_deref().unwrap_or("it exited")
        )
    } else {
        let pull_request = record.pull_request.as_ref().map(|link| format!(" Its pull request: {link}.")).unwrap_or_default();
        format!(
            "[{who}, is done; its message in the chat is its report to the user.{pull_request} Relay it to the user in your reply: its \
             conclusion, its pull request's link, and the evidence it brought back, the commands it ran and what they printed, as it \
             reported them. Say where its report leaves something out, such as no pull request or no command output, without filling it \
             in. This turn only relays: use no tools, and do not open its code, its diff, or its transcript, run a build or a test, judge \
             its work against the request, or send it a message. The user reviews the work and decides what comes next.]"
        )
    };
    without_secrets(app, cue)
}

/// What Auto-review hears about an agent's environment: the saved secrets in it.
pub(crate) fn holds_secrets(names: &[String]) -> String {
    format!(
        "It runs with the user's saved secrets {} in its environment, which the user gave this bot for its commands; sending one anywhere but the service it is for leaks it.",
        names.iter().map(|name| format!("${name}")).collect::<Vec<_>>().join(", ")
    )
}

/// Whether the agent may do what it asks: the bot's Access as it stands, then Auto-review, and
/// the user in the chat when the review asks.
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
    let (tool, description, args, shown) = match &approval {
        Approval::Command { command } => {
            // An agent holding saved secrets has each command reviewed, knowing which: any of
            // them can read its environment.
            if auto_review && record.secrets.is_empty() && crate::local_review::needs_no_review(app, command, &folder) {
                return Ok(());
            }
            let mut description = format!(
                "{who}, a coding agent this bot started, wants to run this shell command as the user on {runner_name}, with full filesystem, process, credential, and network access. Working directory: {}. Lorca asks every coding agent to push its work and open a pull request when it is done, unless the user's request says otherwise.",
                home_relative(&folder)
            );
            if !record.secrets.is_empty() {
                description.push_str(&format!(" {}", holds_secrets(&record.secrets)));
            }
            ("bash", description, json!({ "command": command }), command.chars().take(crate::model::APP_COMMAND_CHARS).collect::<String>())
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
            ("edit", description, json!({ "paths": paths }), format!("Change {shown}"))
        }
        Approval::Tool { name: tool, input } => {
            let description = format!("{who}, a coding agent this bot started in {}, wants to use its tool {tool} as the user on {runner_name}.", home_relative(&folder));
            let detail = input.as_object().and_then(|fields| fields.values().find_map(|value| value.as_str().filter(|text| text.len() < 120).map(str::to_string))).unwrap_or_default();
            let shown = if detail.is_empty() { tool.clone() } else { format!("{tool}({detail})") };
            ("coding_agent_tool", description, json!({ "tool": tool, "input": input }), shown)
        }
    };
    let trigger = record.trigger();
    let cancel = tokio_util::sync::CancellationToken::new();
    let propose_rule = matches!(approval, Approval::Command { .. });
    let facts = match &approval {
        Approval::Command { command } => crate::local_review::facts(command, &folder).await,
        _ => Vec::new(),
    };
    let action = crate::plugins::review::Action { target_name: &runner_name, tool, description: &description, args: &args, script: None, facts: &facts, propose_rule };
    let (reason, rule) = match crate::plugins::review::review(app, &bot, &record.chat_id, &trigger, action, &cancel).await {
        crate::plugins::review::Outcome::Allow => return Ok(()),
        // The agent reads why and goes on another way, or stops and says what it needed.
        crate::plugins::review::Outcome::Held { reason, .. } => return Err(crate::plugins::review::agent_note(&reason)),
        crate::plugins::review::Outcome::Ask { reason, rule } => (reason, rule),
    };
    // Nobody watches the run that started it, so a question would wait for no one.
    if record.unattended {
        let why = reason.as_deref().unwrap_or("Auto-review is off, so every command asks");
        return Err(format!(
            "This needs the user's permission ({why}), and nobody is watching the run that started you, so it did not run. Go on \
             with work that does not depend on it, or stop and name this action exactly in your last message, so the user \
             can decide."
        ));
    }
    // One question at a time.
    let _one = agent.asking.lock().await;
    if !agent.is_running() {
        return Err("Stopped".into());
    }
    let always_rule = rule.map(|text| crate::model::AutoReviewRule { id: uuid::Uuid::new_v4().to_string(), text, behavior: "allow".into(), tool: None });
    // The card is the bot's, so it says which of its agents asks.
    let asks = if tool == "bash" { "asks to run this." } else { "asks for this." };
    let who_asks = format!("{who}, the coding agent on “{}”, {asks}", without_secrets(app, record.title.clone()));
    let reason = Some(match reason {
        Some(reason) => format!("{who_asks} {reason}"),
        None => who_asks,
    });
    use crate::plugins::mcp::Decision;
    match ask_in_chat(app, agent, &bot.id, &runner_name, tool, &shown, reason, always_rule).await {
        Decision::Allowed | Decision::Always => Ok(()),
        Decision::Denied => Err("The user did not allow this in Lorca. Do not retry it; go on without it or stop and say what you needed.".into()),
        Decision::Expired => Err("Nobody answered in time, so it did not run. Go on without it or stop and say what you needed.".into()),
        Decision::Dismissed => Err("Stopped".into()),
    }
}

/// Asks the user whether the agent may do `shown`, on a card at the end of the chat as a command
/// of the bot's own asks, and waits for the answer from any Device (`chats.permission` names the
/// card). The agent's row reads `asking` meanwhile. The bot's turn does not wait on the card, so
/// the user's messages to the bot leave it up; stopping the agent dismisses it.
#[allow(clippy::too_many_arguments)]
async fn ask_in_chat(
    app: &Arc<App>,
    agent: &Arc<Agent>,
    bot_id: &str,
    runner_name: &str,
    tool: &str,
    shown: &str,
    reason: Option<String>,
    always_rule: Option<crate::model::AutoReviewRule>,
) -> crate::plugins::mcp::Decision {
    let chat_id = agent.record.lock().unwrap().chat_id.clone();
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
    agent.asks.store(true, Ordering::SeqCst);
    app.coding_agents.sync(app, agent);
    let summary = crate::local_review::command_summary(shown);
    let target = crate::local_review::LOCAL_TARGET_ID;
    let decision = crate::plugins::mcp::ask_aside(app, &chat_id, bot_id, target, runner_name, tool, &summary, json!({ "command": shown }), reason, always_rule, &cancel).await;
    cancel.cancel();
    watch.abort();
    agent.asks.store(false, Ordering::SeqCst);
    app.coding_agents.sync(app, agent);
    decision
}

/// `coding.transcript` and `coding.stop` for an agent on this Runner, from the local app or a
/// sealed request: `{ chat_id, message_id }` names the row that started it.
pub async fn serve(app: &Arc<App>, verb: &str, body: &Value) -> Result<Value, String> {
    let chat_id = body["chat_id"].as_str().ok_or("missing chat_id")?;
    let message_id = body["message_id"].as_str().ok_or("missing message_id")?;
    let message = app.message(chat_id, message_id).ok_or("Unknown message")?;
    let Author::Bot { .. } = &message.author else { return Err("That row has no coding agent".into()) };
    let agent = app.coding_agents.by_row(chat_id, message_id).ok_or("That coding agent never started")?;
    match verb {
        "coding.transcript" => {
            let items = agent.transcript.lock().unwrap().tail(APP_ITEMS);
            Ok(json!({ "items": items, "card": app.coding_agents.card(app, &agent) }))
        }
        "coding.stop" => {
            if !agent.is_open() {
                return Err("It has already ended".into());
            }
            stop(app, &agent, "Stopped");
            Ok(json!({ "stopped": true }))
        }
        other => Err(format!("Unknown request {other}")),
    }
}

#[cfg(test)]
mod tests;
