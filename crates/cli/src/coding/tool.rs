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
}

const DESCRIPTION: &str = "Hand a coding job to Claude Code or Codex on your Runner and supervise it. `start` runs one on `prompt` \
in a git worktree of `folder` (default: your working directory): a new worktree on a branch of its own, or the one `worktree` names, \
so it never edits the user's checkout; a folder that is not a repository is worked in as it is. Say in `proof` what you expect back \
(test output, screenshots, a pull request). It returns the agent's id at once and works on its own; you get a turn when it is done, \
stalls, or exits, with its last message and the outputs Lorca published from its work (its diff, its proof files, its pull request), \
whose references you can cite as task evidence. `read` shows its transcript since you last read it (or its last `lines`), `send` gives \
it `message`, taken up once it is done with what it does now, or at once with `interrupt: true`, which stops that first; a stopped \
agent starts again on its session. `stop` ends it, `list` shows your agents in this chat. Every command it runs goes through Auto-review \
as yours do, and its card in the chat shows the user its progress and anything it asks them.";

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
            Some("start") => self.start(call_id, &args).await.map_err(ToolError)?,
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
    async fn start(&self, call_id: &str, args: &Value) -> Result<String, String> {
        let app = &self.app;
        let prompt = args["prompt"].as_str().map(str::trim).filter(|prompt| !prompt.is_empty()).ok_or("start needs a prompt")?;
        let message_id = app.coding_agents.row(&self.chat_id, call_id).ok_or("This call has no card in the chat")?;
        let (kind, program) = pick(args["agent"].as_str()).await?;
        let folder = args["folder"].as_str();
        let worktree = args["worktree"].as_str().map(str::trim).filter(|name| !name.is_empty());
        let place = workspace::prepare(&app.config.home, &self.workdir, folder, worktree, prompt).await?;
        let host = hosts::detect().await;
        let task: String = prompt.lines().map(str::trim).find(|line| !line.is_empty()).unwrap_or(prompt).chars().take(TASK_CHARS).collect();
        let full = brief(prompt, args["proof"].as_str(), &place);
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
fn brief(prompt: &str, proof: Option<&str>, place: &workspace::Place) -> String {
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
    brief
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
    update(&|card| {
        card.state = "checking".into();
        card.device = Some(runner_name.clone());
    });
    let who = match ctx.args["agent"].as_str() {
        Some(kind) if KINDS.contains(&kind) => name(kind).to_string(),
        _ => "Claude Code or Codex".to_string(),
    };
    let folder = ctx.args["folder"].as_str().filter(|folder| !folder.trim().is_empty()).unwrap_or("the bot's working directory");
    let description = format!(
        "Start {who}, a coding agent that edits files and runs commands as the user on {runner_name}, in {folder} (a new git worktree of it when it is a repository). It works on the prompt on its own until it is done; each command it runs is reviewed again before it runs."
    );
    let args = json!({ "agent": ctx.args["agent"], "folder": ctx.args["folder"], "worktree": ctx.args["worktree"], "prompt": ctx.args["prompt"], "proof": ctx.args["proof"] });
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
