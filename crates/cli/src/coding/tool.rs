//! `coding_agent`: the bot's handle on the coding agents it runs, and Auto-review of starting one.

use std::path::PathBuf;
use std::sync::Arc;

use async_trait::async_trait;
use lorca_agent::{BeforeToolCallContext, BeforeToolCallResult, Tool, ToolError, ToolResult, ToolUpdateFn};
use serde_json::{json, Value};
use tokio_util::sync::CancellationToken;

use super::{hosts, name, process, workspace, Start, KINDS};
use crate::app::App;
use crate::model::{AgentQuestion, Bot};
use crate::plugins::mcp::Decision;
use crate::plugins::review::{Action, Outcome, Trigger};

/// How much of the prompt the card shows as the task.
const TASK_CHARS: usize = 200;

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

const DESCRIPTION: &str = "Hand a coding job to Claude Code or Codex on your Runner and supervise it. `start` runs one on `prompt` \
in a git worktree of `folder` (default: your working directory): a new worktree on a branch of its own, or the one `worktree` names, \
so it never edits the user's checkout; a folder that is not a repository is worked in as it is. Say in `proof` what you expect back \
(test output, screenshots, a pull request). It returns the agent's id at once and works on its own; you get a turn when it is done, \
stalls, or exits, with its last message and the outputs Lorca published from its work (its diff, its proof files, its pull request), \
whose references you can cite as task evidence. `read` shows its transcript since you last read it (or its last `lines`), `send` gives \
it `message`, taken up once it is done with what it does now, or at once with `interrupt: true`, which stops that first; a stopped \
agent starts again on its session. `stop` ends it, `list` shows your agents in this chat. Every command it runs goes through Auto-review \
as yours do, and its card in the chat shows the user its progress and anything it asks them. Saved secrets never go in a prompt or a \
message: for a job that needs one, such as publishing a package, name your command secrets in `secrets`, and the agent has each as \
$NAME in its environment.";

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
                "agent": { "type": "string", "enum": KINDS, "description": "start: which agent. Default: Claude Code when it is installed, else Codex." },
                "prompt": { "type": "string", "description": "start: the job, as you would put it to an engineer." },
                "folder": { "type": "string", "description": "start: the repository or folder to work in, absolute, ~/…, or relative to your working directory." },
                "worktree": { "type": "string", "description": "start: the worktree to work in, by name, made from the repository's current commit if new. Default: a new one named after the job." },
                "proof": { "type": "string", "description": "start: the proof you expect back, such as \"the output of cargo test, and before/after screenshots of the settings page\"." },
                "secrets": { "type": "array", "items": { "type": "string" }, "description": "start: names of your saved command secrets the job needs, given to the agent as environment variables. Starting it and every command it runs are then reviewed knowing which, and it runs as Lorca's own process, not in a pane." },
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
                agent.record.lock().unwrap().news = None;
                let record = agent.record();
                let skipped = if skipped > 0 { format!("[{skipped} earlier lines left out]\n") } else { String::new() };
                let text = if text.trim().is_empty() { "(nothing new)".to_string() } else { text };
                format!("{} ({}) is {}.\n{skipped}{text}", name(&record.kind), record.id, state_words(&record))
            }
            Some("send") => {
                let agent = agent()?;
                let message = args["message"].as_str().or(args["prompt"].as_str()).map(str::trim).filter(|text| !text.is_empty()).ok_or_else(|| ToolError("send needs a message".into()))?;
                super::send(app, &agent, message, args["interrupt"].as_bool().unwrap_or(false)).await.map_err(ToolError)?
            }
            Some("stop") => {
                let agent = agent()?;
                let record = agent.record();
                if !matches!(record.state.as_str(), "starting" | "working" | "idle" | "asking") {
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
                        format!("- {} {}: {} in {place}: “{}”", record.id, name(&record.kind), state_words(&record), record.task)
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
    /// Starts the agent in a task of its own, since a host can take a while to get one ready and
    /// a call Stop cuts off is dropped: a start stopped part way finishes, and its agent stops as
    /// soon as it runs, so nothing is left running with no one following it.
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
        let message_id = app.coding_agents.row(&self.chat_id, call_id).ok_or("This call has no card in the chat")?;
        let (kind, program) = pick(args["agent"].as_str()).await?;
        // Only the bot's own command secrets, which must be saved already.
        let secrets = secret_names(args)?;
        if !secrets.is_empty() {
            use lorca_agent::tools::SecretVariables;
            crate::secrets::CommandSecrets { app: app.clone(), bot_id: self.bot.id.clone() }.variables(&secrets)?;
        }
        let folder = args["folder"].as_str();
        let worktree = args["worktree"].as_str().map(str::trim).filter(|name| !name.is_empty());
        let place = workspace::prepare(&app.config.home, &self.workdir, folder, worktree, prompt).await?;
        // A pane's command line and screen would show its secrets.
        let host = if secrets.is_empty() { hosts::detect().await } else { None };
        let task: String = prompt.lines().map(str::trim).find(|line| !line.is_empty()).unwrap_or(prompt).chars().take(TASK_CHARS).collect();
        let full = brief(prompt, args["proof"].as_str(), &place, &secrets);
        let start = Start {
            chat_id: self.chat_id.clone(),
            bot_id: self.bot.id.clone(),
            message_id,
            trigger_message_id: self.trigger_message_id.clone(),
            kind: kind.clone(),
            program,
            host: host.clone(),
            place: place.clone(),
            task,
            prompt: full,
            secrets,
            event: self.event.as_ref().map(|event| crate::event_triggers::EventTask { data: String::new(), ..event.clone() }),
        };
        let agent = super::start(app, start).await?;
        let id = agent.id();
        let place = match &place.branch {
            Some(branch) => format!("the worktree {} on branch {branch}", super::home_relative(&place.folder)),
            None => super::home_relative(&place.folder),
        };
        let pane = host.map(|host| format!(" It runs in a {} pane on this Runner, where the user can watch it.", host.title())).unwrap_or_default();
        Ok(format!(
            "Started {} ({id}) in {place}.{pane} It works on its own; you get a turn when it is done, stalls, or exits. Meanwhile coding_agent read {id} shows its transcript, send gives it a follow-up, and stop ends it.",
            name(&kind)
        ))
    }
}

/// The agent the bot asked for, or the one installed: Claude Code first.
async fn pick(asked: Option<&str>) -> Result<(String, PathBuf), String> {
    let asked = asked.map(str::trim).filter(|kind| !kind.is_empty());
    if let Some(kind) = asked {
        if !KINDS.contains(&kind) {
            return Err("agent is claude or codex".into());
        }
        if let Some(program) = process::find(kind).await {
            return Ok((kind.to_string(), program));
        }
    }
    for kind in KINDS {
        if let Some(program) = process::find(kind).await {
            return match asked {
                Some(asked) => Err(format!("{} is not installed on this Runner; {} is.", name(asked), name(kind))),
                None => Ok((kind.to_string(), program)),
            };
        }
    }
    Err("Neither Claude Code nor Codex is installed on this Runner. Ask the user to install one (claude.com/claude-code or developers.openai.com/codex) and sign in to it there.".into())
}

/// The job as the agent reads it: the bot's prompt, where it works, and where its proof goes.
fn brief(prompt: &str, proof: Option<&str>, place: &workspace::Place, secrets: &[String]) -> String {
    let folder = place.folder.display();
    let proof_folder = place.folder.join(workspace::PROOF_FOLDER);
    let mut brief = prompt.trim().to_string();
    match &place.branch {
        Some(branch) => brief.push_str(&format!("\n\nYou work in {folder}, a git worktree of its own on branch {branch}. Commit your work on that branch.")),
        None => brief.push_str(&format!("\n\nYou work in {folder}.")),
    }
    let proof = proof.map(str::trim).filter(|proof| !proof.is_empty());
    match proof {
        Some(proof) => brief.push_str(&format!(" When you are done, save the proof of your work in {}: {proof}.", proof_folder.display())),
        None => brief.push_str(&format!(" Save test output and screenshots that show your work in {}.", proof_folder.display())),
    }
    brief.push_str(
        " That folder stays out of git, so never commit it. Test output goes in .txt or .log files there, screenshots in .png files; name a screenshot before-… or after-… when it shows a change. If you open a pull request, give its link in your last message.",
    );
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
        "working" if record.stalled => "working, but has shown nothing new for a while".into(),
        "working" => "working".into(),
        "idle" => "done, waiting for a follow-up".into(),
        "asking" => "waiting for the user's answer on its card".into(),
        _ => format!("ended ({})", record.outcome.as_deref().unwrap_or(&record.state)),
    }
}

/// Starting a coding agent passes Auto-review as a command does, on the card of the call that
/// starts it: it edits files and runs commands as the user. Each command it runs later is
/// reviewed again (`super::approve`).
pub async fn review_start(app: &Arc<App>, bot: &Bot, chat_id: &str, trigger: &Trigger, unattended: bool, ctx: &BeforeToolCallContext<'_>) -> Option<BeforeToolCallResult> {
    if ctx.tool_call.name != "coding_agent" || ctx.args["action"] != "start" {
        return None;
    }
    let card = app.coding_agents.row(chat_id, &ctx.tool_call.id);
    let update = |change: &dyn Fn(&mut crate::model::AgentRun)| {
        if let Some(message_id) = &card {
            app.coding_agents.update_row(app, chat_id, message_id, change);
        }
    };
    let runner_name = app.device(&bot.runner_id).map(|device| device.name).unwrap_or_else(|| "this Runner".into());
    // The agent that will start: the one asked for, else the one installed. One that cannot start
    // is the call's to report.
    let Ok((kind, _)) = pick(ctx.args["agent"].as_str()).await else { return None };
    update(&|card| {
        card.state = "checking".into();
        card.kind = kind.clone();
        card.device = Some(runner_name.clone());
    });
    let who = name(&kind).to_string();
    let folder = ctx.args["folder"].as_str().filter(|folder| !folder.trim().is_empty()).unwrap_or("the bot's working directory");
    let mut description = format!(
        "Start {who}, a coding agent that edits files and runs commands as the user on {runner_name}, in {folder} (a new git worktree of it when it is a repository). It works on the prompt on its own until it is done; each command it runs is reviewed again before it runs."
    );
    let secrets = secret_names(ctx.args).unwrap_or_default();
    if !secrets.is_empty() {
        description.push_str(&format!(" {}", super::holds_secrets(&secrets)));
    }
    let args = json!({ "agent": ctx.args["agent"], "folder": ctx.args["folder"], "worktree": ctx.args["worktree"], "prompt": ctx.args["prompt"], "proof": ctx.args["proof"], "secrets": ctx.args["secrets"] });
    let action = Action { target_name: &runner_name, tool: "coding_agent", description: &description, args: &args, script: None, propose_rule: true };
    let Outcome::Ask { reason, rule } = crate::plugins::review::review(app, bot, chat_id, trigger, action, ctx.cancel).await else {
        update(&|card| card.state = "starting".into());
        return None;
    };
    let Some(message_id) = card.clone().filter(|_| !unattended) else {
        update(&|card| {
            card.state = "denied".into();
            card.outcome = Some("Needed the user's permission".into());
        });
        return Some(crate::local_review::blocked(format!(
            "Starting {who} needs the user's permission ({}), and nobody is here to give it. Say what you wanted it to do.",
            reason.as_deref().unwrap_or("Auto-review is off, so it asks")
        )));
    };
    let always_rule = rule.map(|text| crate::model::AutoReviewRule { id: uuid::Uuid::new_v4().to_string(), text, behavior: "allow".into(), tool: None });
    let question = AgentQuestion { kind: "start".into(), reason, rule: always_rule.as_ref().map(|rule| rule.text.clone()), ..Default::default() };
    let decision = crate::plugins::mcp::await_answer(app, chat_id, &message_id, always_rule, ctx.cancel, || {
        app.coding_agents.update_row(app, chat_id, &message_id, |card| {
            card.state = "asking".into();
            card.question = Some(question);
        });
        if let Some(message) = app.message(chat_id, &message_id) {
            crate::push::permission(app, &message);
        }
    })
    .await;
    let state = match decision {
        Decision::Allowed | Decision::Always => "starting",
        Decision::Denied if ctx.cancel.is_cancelled() => "stopped",
        Decision::Denied => "denied",
        Decision::Expired => "expired",
        Decision::Dismissed => "dismissed",
    };
    app.coding_agents.update_row(app, chat_id, &message_id, |card| {
        card.state = state.into();
        card.question = None;
    });
    match decision {
        Decision::Allowed | Decision::Always => None,
        Decision::Denied => Some(crate::local_review::blocked(format!("The user did not allow starting {who}. Do not retry it; ask what they want instead."))),
        Decision::Expired => Some(crate::local_review::blocked(format!("Nobody answered whether {who} may start. Say what you wanted it to do and stop."))),
        Decision::Dismissed => Some(crate::local_review::dismissed(&format!(
            "The user sent a new message instead of answering, so {who} did not start. Follow that message; start it again only if it still fits."
        ))),
    }
}
