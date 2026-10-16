//! `coding_agent`: the bot's handle on the coding agents it runs.

use std::path::PathBuf;
use std::sync::Arc;

use async_trait::async_trait;
use lorca_agent::{Tool, ToolError, ToolResult, ToolUpdateFn};
use serde_json::{json, Value};
use tokio_util::sync::CancellationToken;

use super::{name, settings, workspace, Start, KINDS};
use crate::app::App;
use crate::model::Bot;

/// How much of the prompt the card shows as the task.
const TASK_CHARS: usize = 200;
/// How long a title may get.
const TITLE_CHARS: usize = 80;

pub struct CodingAgentTool {
    pub app: Arc<App>,
    pub bot: Bot,
    pub chat_id: String,
    pub workdir: PathBuf,
    /// The message behind the turn: what Auto-review weighs the agent's commands against.
    pub trigger_message_id: String,
    /// The event behind the turn, when one started it: the owner's task for it, whose text the
    /// review weighs the agent's commands against instead of the event's.
    pub event: Option<crate::event_triggers::EventTask>,
}

const DESCRIPTION: &str = "Hand a coding job to a coding agent on your Runner (Claude Code, Codex, OpenCode, pi, or fx, as the user set up \
in Settings → Coding Agents) and supervise it. `start` runs one on `prompt`, under a short `title` that names the job for the user \
(\"Docs: use mygo.json in the launcher examples\"), in `folder` (default: your working directory), which it works in as it is: \
it decides how, whether to make a branch or a worktree of its own, as when the user runs it in a terminal; say in the prompt if the \
user asked for one. `prompt` is what the user asked for, in their words, plus what only you can tell the agent: the folder, what you \
already found, how to check the work, and whether the user asked for a pull request. Do not replace their request with a plan, a \
checklist, or your guesses at the fix: the agent decides how to do the job. While an agent has a job, leave the job to it. \
`start` returns the agent's id at once and the agent works on its own: do not wait for it \
or check on it in the same turn, tell the user you started it. When it is done, or ends on its own, what it said arrives in the chat \
as a message from it and you get a turn: check its work, then send it a follow-up or tell the user how it went. `read` shows its \
transcript since you last read it (or its last `lines`), `send` gives it `message`, taken up once it is done with what it does now, \
or at once with `interrupt: true`, which stops that first; an agent whose process stopped starts again on its session. `stop` ends it, \
`list` shows your agents in this chat. Every command it runs goes through Auto-review as yours do. Saved secrets never go in a \
prompt or a message: for a job that needs one, such as publishing a package, name your command secrets in `secrets`, and the agent \
has each as $NAME in its environment.";

#[async_trait]
impl Tool for CodingAgentTool {
    fn name(&self) -> &str {
        "coding_agent"
    }
    fn description(&self) -> &str {
        DESCRIPTION
    }
    fn parameters(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "action": { "type": "string", "enum": ["start", "read", "send", "stop", "list"] },
                "title": { "type": "string", "description": "start: a short title for the job, a few words the user sees wherever the agent shows." },
                "prompt": { "type": "string", "description": "start: what the user asked for, in their words, with what only you can tell the agent." },
                "agent": { "type": "string", "enum": KINDS, "description": "start: which agent, only when the user asked for one. Default: the first the user turned on that this Runner has." },
                "folder": { "type": "string", "description": "start: the repository or folder to work in, absolute, ~/…, or relative to your working directory." },
                "secrets": { "type": "array", "items": { "type": "string" }, "description": "start: names of your saved command secrets the job needs, given to the agent as environment variables. Every command it runs is then reviewed knowing which." },
                "id": { "type": "string", "description": "read, send, stop: the agent's id." },
                "message": { "type": "string", "description": "send: the follow-up." },
                "interrupt": { "type": "boolean", "description": "send: stop what it does now and take this message at once." },
                "lines": { "type": "integer", "description": "read: its last lines instead of what is new." }
            },
            "required": ["action"]
        })
    }

    async fn execute(&self, call_id: &str, args: Value, cancel: CancellationToken, _on_update: ToolUpdateFn) -> Result<ToolResult, ToolError> {
        if cancel.is_cancelled() {
            return Err(ToolError("Stopped".into()));
        }
        let app = &self.app;
        let agents = &app.coding_agents;
        let agent = || {
            let id = args["id"].as_str().map(str::trim).filter(|id| !id.is_empty()).ok_or_else(|| ToolError("Name the agent by its id".into()))?;
            agents.find(&self.chat_id, &self.bot.id, id).ok_or_else(|| ToolError(format!("You have no coding agent {id} in this chat. coding_agent list shows yours.")))
        };
        let text = match args["action"].as_str() {
            Some("start") => self.start_unless_stopped(call_id, &args, &cancel).await.map_err(ToolError)?,
            Some("read") => {
                let agent = agent()?;
                let lines = args["lines"].as_u64().map(|lines| lines.clamp(1, 2000) as usize);
                let (text, skipped) = agent.read(lines);
                agent.record.lock().unwrap().unheard = None;
                let record = agent.record();
                let skipped = if skipped > 0 { format!("[{skipped} earlier lines left out]\n") } else { String::new() };
                let text = if text.trim().is_empty() { "(nothing new)".to_string() } else { text };
                format!("{} ({}) on “{}” is {}.\n{skipped}{text}", name(&record.kind), record.id, record.title, state_words(&record))
            }
            Some("send") => {
                let agent = agent()?;
                let message = args["message"].as_str().or(args["prompt"].as_str()).map(str::trim).filter(|text| !text.is_empty()).ok_or_else(|| ToolError("send needs a message".into()))?;
                super::send(app, &agent, message, args["interrupt"].as_bool().unwrap_or(false)).await.map_err(ToolError)?
            }
            Some("stop") => {
                let agent = agent()?;
                let record = agent.record();
                if !matches!(record.state.as_str(), "starting" | "working" | "idle") {
                    format!("{} ({}) had already ended: {}.", name(&record.kind), record.id, state_words(&record))
                } else {
                    super::stop(app, &agent, "Stopped by the bot");
                    format!("Stopped {} ({}).", name(&record.kind), record.id)
                }
            }
            Some("list") => {
                let rows: Vec<String> = agents
                    .of(&self.chat_id, &self.bot.id)
                    .iter()
                    .map(|agent| {
                        let record = agent.record();
                        let place = match &record.branch {
                            Some(branch) => format!("{} on {branch}", super::home_relative(&record.folder)),
                            None => super::home_relative(&record.folder),
                        };
                        let pull_request = record.pull_request.as_ref().map(|link| format!(", {link}")).unwrap_or_default();
                        format!("- {} {} “{}”: {} in {place}{pull_request}", record.id, name(&record.kind), record.title, state_words(&record))
                    })
                    .collect();
                if rows.is_empty() { "You have started no coding agents in this chat.".into() } else { rows.join("\n") }
            }
            _ => return Err(ToolError("action is start, read, send, stop, or list".into())),
        };
        Ok(ToolResult::text(text))
    }
}

impl CodingAgentTool {
    /// Starts the agent in a task of its own, since getting one ready can take a while and a call
    /// Stop cuts off is dropped: a start stopped part way finishes, and its agent stops as soon
    /// as it runs, so nothing is left running with no one following it.
    pub(super) async fn start_unless_stopped(&self, call_id: &str, args: &Value, cancel: &CancellationToken) -> Result<String, String> {
        let tool = CodingAgentTool {
            app: self.app.clone(),
            bot: self.bot.clone(),
            chat_id: self.chat_id.clone(),
            workdir: self.workdir.clone(),
            trigger_message_id: self.trigger_message_id.clone(),
            event: self.event.clone(),
        };
        let (call_id, args) = (call_id.to_string(), args.clone());
        let row = self.app.coding_agents.row(&self.chat_id, &call_id);
        let mut starting = tokio::spawn(async move { tool.start(&call_id, &args).await });
        tokio::select! {
            started = &mut starting => started.map_err(|error| error.to_string())?,
            _ = cancel.cancelled() => {
                let (app, chat_id) = (self.app.clone(), self.chat_id.clone());
                tokio::spawn(async move {
                    let _ = starting.await;
                    if let Some(agent) = row.and_then(|row| app.coding_agents.by_row(&chat_id, &row)) {
                        super::stop(&app, &agent, "Stopped");
                    }
                });
                Err("Stopped".into())
            }
        }
    }

    async fn start(&self, call_id: &str, args: &Value) -> Result<String, String> {
        let app = &self.app;
        let prompt = args["prompt"].as_str().map(str::trim).filter(|prompt| !prompt.is_empty()).ok_or("start needs a prompt")?;
        let task: String = prompt.lines().map(str::trim).find(|line| !line.is_empty()).unwrap_or(prompt).chars().take(TASK_CHARS).collect();
        let title = args["title"].as_str().map(super::transcript::title).filter(|title| !title.is_empty()).unwrap_or_else(|| task.clone());
        let title: String = title.chars().take(TITLE_CHARS).collect();
        let message_id = app.coding_agents.row(&self.chat_id, call_id).ok_or("This call has no row in the chat")?;
        if let Err(denied) = crate::permissions::check_tool(app, &self.bot, "bash") {
            return Err(format!("{denied}. A coding agent runs commands, so it needs Shell commands too."));
        }
        let (kind, program) = settings::pick(app, args["agent"].as_str()).await?;
        // Only the bot's own command secrets, which must be saved already.
        let secrets = secret_names(args)?;
        if !secrets.is_empty() {
            use lorca_agent::tools::SecretVariables;
            crate::secrets::CommandSecrets { app: app.clone(), bot_id: self.bot.id.clone() }.variables(&secrets)?;
        }
        let place = workspace::prepare(&self.workdir, args["folder"].as_str()).await?;
        let start = Start {
            chat_id: self.chat_id.clone(),
            bot_id: self.bot.id.clone(),
            message_id,
            trigger_message_id: self.trigger_message_id.clone(),
            kind: kind.clone(),
            program,
            place: place.clone(),
            title: title.clone(),
            task,
            prompt: brief(prompt, &place, &secrets),
            secrets,
            event: self.event.as_ref().map(|event| crate::event_triggers::EventTask { data: String::new(), ..event.clone() }),
        };
        let agent = super::start(app, start).await?;
        let id = agent.id();
        let place = match &place.branch {
            Some(branch) => format!("{} (on branch {branch})", super::home_relative(&place.folder)),
            None => super::home_relative(&place.folder),
        };
        Ok(format!(
            "Started {} ({id}) on “{title}” in {place}. It works on its own: when it is done, or ends, what it said arrives in the chat and you get a turn. Don't wait for it or check on it now; tell the user you started it. Meanwhile coding_agent read {id} shows its transcript, send gives it a follow-up, and stop ends it.",
            name(&kind)
        ))
    }
}

/// The job as the agent reads it: the bot's prompt, and where it works.
fn brief(prompt: &str, place: &workspace::Place, secrets: &[String]) -> String {
    let folder = place.folder.display();
    let mut brief = prompt.trim().to_string();
    brief.push_str(&format!("\n\nYou work in {folder}."));
    brief.push_str(" When you are done, end with a short summary of what you did and how you checked it. If you open a pull request, give its link.");
    if !secrets.is_empty() {
        let names = secrets.iter().map(|name| format!("${name}")).collect::<Vec<_>>().join(", ");
        brief.push_str(&format!(
            " Your environment holds {names}, saved secrets for your commands: use them only with the service each is for, and never print them, write them into a file, or put them in a commit."
        ));
    }
    brief
}

/// The saved secrets a start names, once each.
fn secret_names(args: &Value) -> Result<Vec<String>, String> {
    match &args["secrets"] {
        Value::Null => Ok(Vec::new()),
        Value::Array(names) => {
            let mut out: Vec<String> = Vec::new();
            for name in names {
                let name = name.as_str().map(str::trim).filter(|name| !name.is_empty()).ok_or("secrets is a list of names")?;
                if !out.iter().any(|known| known == name) {
                    out.push(name.to_string());
                }
            }
            Ok(out)
        }
        _ => Err("secrets is a list of names".into()),
    }
}

/// How an agent stands, in a few words for the bot.
fn state_words(record: &super::Record) -> String {
    match record.state.as_str() {
        "starting" => "starting".into(),
        "working" => "working".into(),
        "idle" => "done, waiting for a follow-up".into(),
        _ => format!("ended ({})", record.outcome.as_deref().unwrap_or(&record.state)),
    }
}
