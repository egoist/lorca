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
    /// Nobody watches the turn (a routine's run or an event's): its agents' questions have
    /// nobody to answer them.
    pub unattended: bool,
}

const DESCRIPTION: &str = "Hand a coding job to a coding agent on your Runner (Claude Code, Codex, OpenCode, pi, or fx, as the user set up \
in Settings → Coding Agents). You pass the user's request on; the agent decides how to do the job and reports to the user. \
`start` runs one on `prompt`, under a short `title` that names the job for the user (\"Docs: use mygo.json in the launcher \
examples\"), in `folder` (default: your working directory), the repository or folder the job is in. `prompt` is what the user asked \
for, in their words, plus only facts you know and the agent cannot find: where the repository is, and what the user already said \
about the job earlier in the chat, in their words too. Add nothing of your own: no plan, checklist, acceptance criteria, guess at \
the fix, scope, way to build or check it, commands to run, or branch or worktree to use. The agent decides all of that, where in \
the folder it works and whether on a branch of its own included, and it pushes its work, opens a pull request, and ends with a \
report for the user: what it did, the commands it ran with their output, and the pull request's link. `start` returns the agent's \
id at once and the agent works in the background: tell the user you started it and end your turn. You will be notified when it \
is done: a turn on its own, whose \"<agent-notification>\" carries its status and its report. A turn of the agent that ends while \
it still waits on work of its own in the background (a command watching CI, a subagent) is not done; you hear only once it is. \
Until then you know nothing of its results: do not report, assume, or predict them, do not poll it or wait on it, and when the user \
asks how it goes, say it is still working, with where it is from `read`. Its report also lands in the chat as its message to the \
user; in your turn for it, relay it: its conclusion, its pull request, and the commands it ran with their output, without checking \
its code, building, testing, judging its work, or sending it anything. When it ends on its own, you get a turn to tell the user. \
`send` gives it `message`, which is what the user tells it, in their words; it keeps its whole context and takes the message up at \
its next step, or at once with `interrupt: true`, which stops what it does first; an agent whose process stopped starts again on \
its session. The user may also write to an agent on its page; you read that in the chat as \"[The user wrote to … directly]\", and \
the agent has it already, so do not pass it on again. `read` shows its history whenever the user asks about it, how it stands or \
what it did: what is new since you last read (its end again when nothing is), its last `lines`, or any part of its whole history \
from line `offset`. When it asks the user something, its question is a card in the chat the user answers. `stop` ends it. `list` \
shows your agents in this chat; with `scope: \"account\"` it shows every coding agent on the account, whichever bot, chat, and \
Runner started it, each with its bot, chat, Runner, folder, branch, state, and pull request, so you can tell whose a pull request \
or a job is (`query` narrows it: a pull request's link, a folder, a title, a bot). Only an agent's own bot reads, messages, or \
stops it, from its chat: for another bot's, message that bot. When `start` finds another agent already at work in the same folder \
or repository, it says whose and on what: tell the user. Every command it runs goes through Auto-review as yours do. Saved secrets never go in a prompt or \
a message: for a job that needs one, such as publishing a package, name your command secrets in `secrets`, and the agent has each \
as $NAME in its environment.";

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
                "scope": { "type": "string", "enum": ["chat", "account"], "description": "list: `chat` (default) for your agents in this chat, `account` for every coding agent on the account, any bot's, in any chat." },
                "query": { "type": "string", "description": "list: only the agents whose id, title, folder, branch, repository, pull request, bot, or chat contains this, such as a pull request's link." },
                "title": { "type": "string", "description": "start: a short title for the job, a few words the user sees wherever the agent shows." },
                "prompt": { "type": "string", "description": "start: what the user asked for, in their words, with only the facts you know and the agent cannot find, such as where the repository is." },
                "agent": { "type": "string", "enum": KINDS, "description": "start: which agent, only when the user asked for one. Default: the first the user turned on that this Runner has." },
                "folder": { "type": "string", "description": "start: the repository or folder the job is in, absolute, ~/…, or relative to your working directory. The agent decides where in it to work, and whether on a branch or worktree of its own." },
                "secrets": { "type": "array", "items": { "type": "string" }, "description": "start: names of your saved command secrets the job needs, given to the agent as environment variables. Every command it runs is then reviewed knowing which." },
                "id": { "type": "string", "description": "read, send, stop: the agent's id." },
                "message": { "type": "string", "description": "send: what the user tells the agent, in their words." },
                "interrupt": { "type": "boolean", "description": "send: stop what it does now and take this message at once." },
                "lines": { "type": "integer", "description": "read: its last lines instead of what is new; with offset, how many lines from there (default 200)." },
                "offset": { "type": "integer", "description": "read: the line of its whole history to read from, 1 for its start." }
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
            Some("start") => self.start_unless_stopped(call_id, &args, &cancel, false).await.map_err(ToolError)?,
            Some("read") => {
                let agent = agent()?;
                let lines = args["lines"].as_u64().map(|lines| lines.clamp(1, 2000) as usize);
                let offset = args["offset"].as_u64().map(|offset| offset.max(1) as usize);
                agent.record.lock().unwrap().unheard = None;
                let record = agent.record();
                let head = format!("{} ({}) on “{}” is {}.", name(&record.kind), record.id, record.title, state_words(&record));
                let mut transcript = agent.transcript.lock().unwrap();
                let total = transcript.line_count();
                // Any part of its history, from a line on.
                if let Some(offset) = offset {
                    let (text, first, total) = transcript.page(offset, lines.unwrap_or(super::READ_LINES));
                    let last = first + text.lines().count().max(1) - 1;
                    let text = if text.is_empty() { format!("(its history has {total} lines)") } else { text };
                    format!("{head} Lines {first}–{last} of {total}:\n{text}")
                } else {
                    let (text, skipped) = transcript.read(lines, super::READ_LINES);
                    // Nothing new since the last read: its end again, rather than nothing.
                    let (text, skipped, new) = if text.trim().is_empty() && lines.is_none() {
                        let (text, skipped) = transcript.read(Some(super::READ_LINES), super::READ_LINES);
                        (text, total.saturating_sub(super::READ_LINES).max(skipped), false)
                    } else {
                        (text, skipped, lines.is_none())
                    };
                    let what = match (new, skipped) {
                        (true, 0) => String::new(),
                        (false, 0) if lines.is_none() => "Nothing new since you last read; its history:\n".into(),
                        (false, 0) => String::new(),
                        (true, skipped) => format!("What is new since you last read, its last lines ({skipped} earlier lines left out; offset reads them):\n"),
                        (false, skipped) => format!("Nothing new since you last read; its last lines ({skipped} of {total} left out; offset reads them):\n"),
                    };
                    let text = if text.trim().is_empty() { "(no history yet)".to_string() } else { text };
                    format!("{head}\n{what}{text}")
                }
            }
            Some("send") => {
                let agent = agent()?;
                let message = args["message"].as_str().or(args["prompt"].as_str()).map(str::trim).filter(|text| !text.is_empty()).ok_or_else(|| ToolError("send needs a message".into()))?;
                super::send(app, &agent, message, args["interrupt"].as_bool().unwrap_or(false), super::Sender::Bot).await.map_err(ToolError)?
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
            Some("list") if args["scope"].as_str() == Some("account") => {
                let query = args["query"].as_str().map(str::trim).filter(|query| !query.is_empty()).map(str::to_lowercase);
                let listed: Vec<_> = crate::agent_roster::account(app).into_iter().filter(|listed| query.as_deref().is_none_or(|query| matches(listed, query))).collect();
                if listed.is_empty() {
                    match query {
                        Some(_) => "No coding agent on the account matches that.".into(),
                        None => "No coding agent on the account is open or changed in the last 30 days.".into(),
                    }
                } else {
                    let rows: Vec<String> = listed.iter().map(|listed| listed.line(&self.bot.id, &self.chat_id)).collect();
                    format!("Coding agents on the account, open ones first:\n{}", rows.join("\n"))
                }
            }
            Some("list") => {
                let rows: Vec<String> = agents
                    .of(&self.chat_id, &self.bot.id)
                    .iter()
                    .filter(|agent| agent.record.lock().unwrap().kind != super::SUBAGENT)
                    .map(|agent| {
                        let record = agent.record();
                        let place = match &record.branch {
                            Some(branch) => format!("{} on {branch}", super::home_relative(&record.folder)),
                            None => super::home_relative(&record.folder),
                        };
                        let pull_request = match (&record.pull_request, &record.pull_state) {
                            (Some(link), Some(state)) => format!(", {link} ({state})"),
                            (Some(link), None) => format!(", {link}"),
                            _ => String::new(),
                        };
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
    pub(super) async fn start_unless_stopped(&self, call_id: &str, args: &Value, cancel: &CancellationToken, subagent: bool) -> Result<String, String> {
        let tool = CodingAgentTool {
            app: self.app.clone(),
            bot: self.bot.clone(),
            chat_id: self.chat_id.clone(),
            workdir: self.workdir.clone(),
            trigger_message_id: self.trigger_message_id.clone(),
            event: self.event.clone(),
            unattended: self.unattended,
        };
        let (call_id, args) = (call_id.to_string(), args.clone());
        let row = self.app.coding_agents.row(&self.chat_id, &call_id);
        let mut starting = tokio::spawn(async move { if subagent { tool.start_subagent(&call_id, &args).await } else { tool.start(&call_id, &args).await } });
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
        let device_id = app.this_device_id();
        let folder = super::home_relative(&place.folder);
        let at = crate::agent_roster::Place { device_id: device_id.as_deref(), folder: &folder, repository: place.repository.as_deref() };
        let others = crate::agent_roster::working_at(app, &at, None);
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
            unattended: self.unattended,
            read_only: false,
        };
        let agent = super::start(app, start).await?;
        let id = agent.id();
        let place = match &place.branch {
            Some(branch) => format!("{} (on branch {branch})", super::home_relative(&place.folder)),
            None => super::home_relative(&place.folder),
        };
        let started = format!(
            "Started {} ({id}) on “{title}” in {place}. It works in the background. You will be notified when it is done, with its report, which also reaches the user in the chat, or why it ended on its own; until then you know nothing of its results, so do not report, assume, or predict them, and do not poll it. Tell the user you started it and end your turn. When the user asks, coding_agent read {id} shows its history, send passes it what they say, and stop ends it.",
            name(&kind)
        );
        Ok(match crate::agent_roster::warning(&others, &self.bot.id, &self.chat_id) {
            Some(warning) => format!("{started}\n\n{warning}"),
            None => started,
        })
    }
}

impl CodingAgentTool {
    /// Starts a subagent on the bot's own task, in `folder` (default: the bot's working
    /// directory), only reading there when its `type` is `explore`.
    async fn start_subagent(&self, call_id: &str, args: &Value) -> Result<String, String> {
        let app = &self.app;
        let prompt = args["prompt"].as_str().map(str::trim).filter(|prompt| !prompt.is_empty()).ok_or("start needs a prompt")?;
        let task: String = prompt.lines().map(str::trim).find(|line| !line.is_empty()).unwrap_or(prompt).chars().take(TASK_CHARS).collect();
        let title = args["title"].as_str().map(super::transcript::title).filter(|title| !title.is_empty()).unwrap_or_else(|| task.clone());
        let title: String = title.chars().take(TITLE_CHARS).collect();
        let message_id = app.coding_agents.row(&self.chat_id, call_id).ok_or("This call has no row in the chat")?;
        let read_only = args["type"].as_str() == Some("explore");
        if !read_only {
            if let Err(denied) = crate::permissions::check_tool(app, &self.bot, "bash") {
                return Err(format!("{denied}. A general subagent runs commands, so it needs Shell commands too; an explore one only reads."));
            }
        }
        let place = workspace::prepare(&self.workdir, args["folder"].as_str()).await?;
        let start = Start {
            chat_id: self.chat_id.clone(),
            bot_id: self.bot.id.clone(),
            message_id,
            trigger_message_id: self.trigger_message_id.clone(),
            kind: super::SUBAGENT.into(),
            program: PathBuf::new(),
            place: place.clone(),
            title: title.clone(),
            task,
            prompt: prompt.to_string(),
            secrets: Vec::new(),
            event: self.event.as_ref().map(|event| crate::event_triggers::EventTask { data: String::new(), ..event.clone() }),
            unattended: self.unattended,
            read_only,
        };
        let agent = super::start(app, start).await?;
        let id = agent.id();
        Ok(format!(
            "Started subagent {id} on “{title}” in {}. It works in the background with a context of its own. You will be notified with its report when it is done; until then you know nothing of its results, so do not report, assume, or predict them, do not poll it, and do not do its task yourself meanwhile. Go on with other work or end your turn. subagent send {id} gives it a follow-up with its context, read shows its history, and stop ends it.",
            super::home_relative(&place.folder)
        ))
    }
}

const SUBAGENT_DESCRIPTION: &str = "Hand a task to a subagent of yours: your own agent loop on your Runner, with a context of its own, your \
model, and the file and shell tools in `folder` (default: your working directory), every command and change reviewed as yours \
are. Use one for work that would fill your context or that runs while you go on: researching a codebase, a search across many \
files, a long check, independent pieces of work in parallel (several starts in one turn). `start` runs one on `prompt` under a short \
`title`; `type: \"explore\"` only reads and searches, `general` (default) also runs commands and changes files. It sees nothing of \
this chat, so brief it like a smart colleague who just walked in: the goal and why, what you know already, what to leave alone, \
and what to bring back (\"report in under 200 words\", \"list the files with line numbers\"). It answers only you, never the user, \
and asks nobody anything. `start` returns its id at once and it works in the background: you are notified with its report when it \
is done, as an \"<agent-notification>\" on a turn of its own, and its report never reaches the user unless you tell them. Until \
then you know nothing of its results: do not report, assume, or predict them, do not poll it, and do not do its task yourself \
meanwhile. `send` gives it a follow-up, taken up at its next step with its whole context, or at once with `interrupt: true`; one \
that is done starts again on its context. `read` shows its history (what is new, its last `lines`, or from line `offset`). `stop` \
ends it. `list` shows your subagents in this chat.";

/// `subagent`: the bot's handle on its own subagents, which run on the coding agents' machinery.
pub struct SubagentTool(pub CodingAgentTool);

#[async_trait]
impl Tool for SubagentTool {
    fn name(&self) -> &str {
        "subagent"
    }
    fn description(&self) -> &str {
        SUBAGENT_DESCRIPTION
    }
    fn parameters(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "action": { "type": "string", "enum": ["start", "read", "send", "stop", "list"] },
                "title": { "type": "string", "description": "start: a short title for its task, a few words." },
                "prompt": { "type": "string", "description": "start: the task, briefed in full: it sees nothing of this chat." },
                "type": { "type": "string", "enum": ["general", "explore"], "description": "start: explore only reads and searches; general (default) also runs commands and changes files." },
                "folder": { "type": "string", "description": "start: where it works, absolute, ~/…, or relative to your working directory." },
                "id": { "type": "string", "description": "read, send, stop: the subagent's id." },
                "message": { "type": "string", "description": "send: your follow-up." },
                "interrupt": { "type": "boolean", "description": "send: stop what it does now and take this at once." },
                "lines": { "type": "integer", "description": "read: its last lines instead of what is new; with offset, how many lines from there (default 200)." },
                "offset": { "type": "integer", "description": "read: the line of its whole history to read from, 1 for its start." }
            },
            "required": ["action"]
        })
    }

    async fn execute(&self, call_id: &str, args: Value, cancel: CancellationToken, on_update: ToolUpdateFn) -> Result<ToolResult, ToolError> {
        let tool = &self.0;
        match args["action"].as_str() {
            Some("start") => {
                if cancel.is_cancelled() {
                    return Err(ToolError("Stopped".into()));
                }
                let text = tool.start_unless_stopped(call_id, &args, &cancel, true).await.map_err(ToolError)?;
                Ok(ToolResult::text(text))
            }
            Some("list") => {
                let rows: Vec<String> = tool
                    .app
                    .coding_agents
                    .of(&tool.chat_id, &tool.bot.id)
                    .iter()
                    .map(|agent| agent.record())
                    .filter(|record| record.kind == super::SUBAGENT)
                    .map(|record| format!("- {} “{}”: {} in {}", record.id, record.title, state_words(&record), super::home_relative(&record.folder)))
                    .collect();
                Ok(ToolResult::text(if rows.is_empty() { "You have started no subagents in this chat.".into() } else { rows.join("\n") }))
            }
            _ => tool.execute(call_id, args, cancel, on_update).await,
        }
    }
}

/// The job as the agent reads it: the user's request the bot passed on, where it is, and what to
/// hand the user back, since the user reviews the work and nobody checks it before.
fn brief(prompt: &str, place: &workspace::Place, secrets: &[String]) -> String {
    let folder = place.folder.display();
    let mut brief = prompt.trim().to_string();
    brief.push_str(&format!("\n\nThe job is in {folder}."));
    brief.push_str(
        " How you do it is yours to decide: where you work, whether on a branch or a worktree of your own, and how you check it. \
         The user reviews your work, and nobody checks it before. Unless the request says otherwise, push your work and open a pull \
         request when you are done. End with your report to the user: what you did, each command you ran to check it with what it \
         printed, and the pull request's link, or why there is none.",
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

/// Whether an agent on the account answers `query`, lowercased.
fn matches(listed: &crate::agent_roster::Listed, query: &str) -> bool {
    let agent = &listed.agent;
    [Some(&agent.id), Some(&agent.title), Some(&agent.folder), agent.branch.as_ref(), agent.repository.as_ref(), agent.pull_request.as_ref(), Some(&listed.bot_name), Some(&listed.bot_id), Some(&listed.chat_title), Some(&listed.chat_id)]
        .into_iter()
        .flatten()
        .any(|field| field.to_lowercase().contains(query))
}

/// How an agent stands, in a few words for the bot.
fn state_words(record: &super::Record) -> String {
    match record.state.as_str() {
        "starting" => "starting".into(),
        "working" => "working".into(),
        "idle" => "done".into(),
        _ => format!("ended ({})", record.outcome.as_deref().unwrap_or(&record.state)),
    }
}
