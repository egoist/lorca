//! Lorca's own subagent: the bot's agent loop run in this process on a task the bot hands it,
//! after Claude Code's and Grok Bot's subagents. It has a context of its own, the bot's provider
//! and model, and the file and shell tools in the folder it is given (only the reading ones when
//! it only explores); every command and change outside reading asks as a coding agent's does
//! (`Event::Approve`). It talks to nobody but the bot: its last message is its report, which
//! reaches the bot as a notification. A message while it works is taken up after the tool calls
//! under way (steering), one when it is done starts another run on the same context, and its
//! context is kept in `agents/<id>.context.json`, which resumes it after its run is gone.

use std::collections::VecDeque;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use async_trait::async_trait;
use lorca_agent::{
    AfterToolCallContext, AfterToolCallResult, AgentContext, AgentEvent, AgentLoopConfig, AgentMessage, AssistantPart, BeforeToolCallContext, BeforeToolCallResult, ContentPart,
    EventSink, LoopHooks, RetryPolicy, StopReason, Tool, ToolExecutionMode,
};
use serde_json::Value;
use tokio::sync::{mpsc, oneshot};
use tokio_util::sync::CancellationToken;

use super::driver::{Approval, Driver, Ended, Event, Events};
use super::transcript::{self, Item};
use crate::app::App;

/// The tools a subagent that only explores keeps.
const READING: &[&str] = &["read", "grep", "find", "ls"];

/// What a subagent runs with.
pub(crate) struct Launch<'a> {
    pub app: &'a Arc<App>,
    pub bot_id: &'a str,
    pub folder: &'a Path,
    /// Its context file, `agents/<id>.context.json`: its session.
    pub context: PathBuf,
    pub read_only: bool,
    /// The limits of the turn that gave it the work, which its model calls count toward.
    pub budget: Option<crate::budgets::BudgetContext>,
}

pub(crate) struct Lorca(Arc<Shared>);

struct Shared {
    app: Arc<App>,
    bot_id: String,
    folder: PathBuf,
    file: PathBuf,
    read_only: bool,
    budget: Option<crate::budgets::BudgetContext>,
    events: Events,
    /// Its context, all its runs' messages.
    messages: tokio::sync::Mutex<Vec<AgentMessage>>,
    state: Mutex<State>,
    /// The calls Lorca said no to, whose steps read as not allowed.
    denied: Mutex<std::collections::HashSet<String>>,
}

#[derive(Default)]
struct State {
    /// A run is under way, and what cancels it.
    run: Option<CancellationToken>,
    /// Messages it has yet to take up: at its next step while it runs, else in the next run.
    queue: VecDeque<String>,
    /// It is stopping: no run starts again, and its end follows.
    stopped: bool,
}

/// Starts a subagent on `prompt`, or resumes the one whose context `launch.context` holds.
pub(crate) async fn start(launch: Launch<'_>, prompt: &str, resume: bool, events: Events) -> Result<Arc<dyn Driver>, String> {
    let messages = if resume {
        let bytes = std::fs::read(&launch.context).map_err(|_| "Its context is gone, so it cannot go on. Start a new subagent.".to_string())?;
        serde_json::from_slice(&bytes).map_err(|error| format!("Its context could not be read: {error}"))?
    } else {
        Vec::new()
    };
    let _ = events.send(Event::Session(launch.context.display().to_string()));
    let shared = Arc::new(Shared {
        app: launch.app.clone(),
        bot_id: launch.bot_id.to_string(),
        folder: launch.folder.to_path_buf(),
        file: launch.context,
        read_only: launch.read_only,
        budget: launch.budget,
        events,
        messages: tokio::sync::Mutex::new(messages),
        state: Mutex::new(State::default()),
        denied: Mutex::new(Default::default()),
    });
    let driver = Lorca(shared);
    driver.send(prompt, false).await?;
    Ok(Arc::new(driver))
}

#[async_trait]
impl Driver for Lorca {
    async fn send(&self, text: &str, interrupt: bool) -> Result<(), String> {
        let shared = &self.0;
        let mut state = shared.state.lock().unwrap();
        if state.stopped {
            return Err("It is stopping.".into());
        }
        match state.run.clone() {
            // It works: the message waits for its next step, or cuts in first.
            Some(run) => {
                if interrupt {
                    state.queue.push_front(text.to_string());
                    run.cancel();
                } else {
                    state.queue.push_back(text.to_string());
                }
            }
            None => {
                let cancel = CancellationToken::new();
                state.run = Some(cancel.clone());
                tokio::spawn(runs(shared.clone(), text.to_string(), cancel));
            }
        }
        Ok(())
    }

    async fn stop(&self) {
        let shared = &self.0;
        let running = {
            let mut state = shared.state.lock().unwrap();
            state.stopped = true;
            state.queue.clear();
            state.run.as_ref().inspect(|run| run.cancel()).is_some()
        };
        // A run under way says it ended once it has; with none, it ends now.
        if !running {
            let _ = shared.events.send(Event::Ended(Ended { outcome: "Stopped".into() }));
        }
    }
}

/// Runs on `text`, then on each message that arrived as a run ended, until none waits.
async fn runs(shared: Arc<Shared>, mut text: String, mut cancel: CancellationToken) {
    loop {
        let run = run(&shared, &text, &cancel);
        let outcome = match &shared.budget {
            Some(budget) => budget.scope(run).await,
            None => run.await,
        };
        let next = {
            let mut state = shared.state.lock().unwrap();
            if state.stopped {
                state.run = None;
                drop(state);
                let _ = shared.events.send(Event::Ended(Ended { outcome: "Stopped".into() }));
                return;
            }
            let next = state.queue.pop_front();
            match &next {
                Some(_) => {
                    cancel = CancellationToken::new();
                    state.run = Some(cancel.clone());
                }
                None => state.run = None,
            }
            next
        };
        match next {
            Some(next) => text = next,
            None => {
                let _ = shared.events.send(outcome);
                return;
            }
        }
    }
}

/// One run of its loop on `text`: what it ends as, done with what it said last, or failed.
async fn run(shared: &Arc<Shared>, text: &str, cancel: &CancellationToken) -> Event {
    let app = &shared.app;
    let Some(bot) = app.bot(&shared.bot_id) else { return Event::Failed("Its bot is gone.".into()) };
    crate::catalog::check_for_model(app, &bot.provider, bot.model.as_deref()).await;
    let provider = match crate::providers::provider_for(app, &bot.provider, bot.model.as_deref(), crate::providers::thinking_level(&bot)) {
        Ok(provider) => provider,
        Err(error) => {
            let _ = shared.events.send(Event::Item(Item::text(uuid::Uuid::new_v4().to_string(), "error", &error)));
            return Event::Failed(error);
        }
    };
    let mut tools: Vec<Arc<dyn Tool>> = lorca_agent::tools::coding_tools(shared.folder.clone());
    if let Some(bash) = tools.iter().position(|tool| tool.name() == "bash") {
        tools[bash] = Arc::new(lorca_agent::tools::BashTool::new(shared.folder.clone()).with_extras(crate::shell::bot_shell_extras(app)));
    }
    if shared.read_only {
        tools.retain(|tool| READING.contains(&tool.name()));
    }
    let context = AgentContext { system_prompt: system_prompt(&bot, &shared.folder, shared.read_only), messages: shared.messages.lock().await.clone(), tools, cache_points: Vec::new() };
    let said = Arc::new(Mutex::new(None));
    let sink = Arc::new(Sink { shared: shared.clone(), said: said.clone(), steps: Mutex::new(Default::default()) });
    let hooks = Arc::new(Hooks { shared: shared.clone() });
    let config = AgentLoopConfig {
        provider,
        hooks,
        tool_execution: ToolExecutionMode::Sequential,
        sink: Some(sink),
        retry: Some(RetryPolicy::default()),
        request: lorca_agent::RequestOptions::default(),
        interrupt: None,
        stop_grace: lorca_agent::STOP_GRACE,
    };
    let (tx, _rx) = mpsc::channel::<AgentEvent>(1);
    drop(_rx);
    let _ = shared.events.send(Event::Working);
    let added = lorca_agent::run_agent_loop(vec![AgentMessage::user(text)], context, &config, &tx, cancel.clone()).await;
    let last = added.iter().rev().find_map(|message| match message {
        AgentMessage::Assistant(message) => Some(message.clone()),
        _ => None,
    });
    {
        let mut messages = shared.messages.lock().await;
        messages.extend(added);
        if let Err(error) = crate::config::write_json_private(&shared.file, &*messages) {
            tracing::warn!(%error, "saving a subagent's context");
        }
    }
    match last {
        Some(message) if message.stop_reason == StopReason::Aborted || cancel.is_cancelled() => {
            let _ = shared.events.send(Event::Item(Item::text(uuid::Uuid::new_v4().to_string(), "status", "Interrupted")));
            Event::Idle(said.lock().unwrap().take())
        }
        Some(message) if message.stop_reason == StopReason::Error => {
            let error = message.error_message.unwrap_or_else(|| "The model's request failed".into());
            let error = error.lines().next().unwrap_or(&error).to_string();
            let _ = shared.events.send(Event::Item(Item::text(uuid::Uuid::new_v4().to_string(), "error", &error)));
            Event::Failed(error)
        }
        _ => Event::Idle(said.lock().unwrap().take()),
    }
}

/// What it is told it is: the bot's subagent, its task, and that its report goes to the bot.
fn system_prompt(bot: &crate::model::Bot, folder: &Path, read_only: bool) -> String {
    let what = if read_only {
        "You only read and search: read, grep, find, and ls are your tools. Change nothing."
    } else {
        "Your tools work in the folder you are in; every command and every change outside it is reviewed before it runs, and one that is held comes back to you saying why: go on another way, or say in your report what you needed."
    };
    let mut prompt = format!(
        "You are a subagent of {name}, a Lorca bot, on a task it handed you. Do it on your own, then end with your report to {name}: what \
         you found or did, concisely, with the evidence it needs (file paths, the commands you ran and what they printed). You never \
         talk to the user: your last message goes only to {name}, which tells the user what matters. Do not ask questions back; when \
         something is unclear, make a reasonable choice and say which in your report. {name} may send you more messages; you keep \
         your whole context.\n\n{what}\n\nWorking directory: {}. Date: {}. Platform: {}.\n\n{}\n{}",
        folder.display(),
        crate::memory::local_time(crate::config::now_secs() as i64).date,
        std::env::consts::OS,
        lorca_agent::tools::coding_tools_snippet(),
        lorca_agent::tools::coding_tools_guidelines().iter().map(|line| format!("- {line}")).collect::<Vec<_>>().join("\n"),
        name = bot.name,
    );
    if read_only {
        prompt = prompt.replace("write: Create or overwrite files. edit: Make precise file edits with exact text replacement, including multiple disjoint edits in one call. bash: Execute bash commands (ls, grep, find, etc.). ", "");
    }
    prompt
}

/// Its steps as the transcript shows them, and what it said last.
struct Sink {
    shared: Arc<Shared>,
    said: Arc<Mutex<Option<String>>>,
    /// Its calls under way, by id.
    steps: Mutex<std::collections::HashMap<String, transcript::ToolStep>>,
}

#[async_trait]
impl EventSink for Sink {
    async fn on_event(&self, event: &AgentEvent) {
        let folder = &self.shared.folder;
        let item = |item: Item| {
            let _ = self.shared.events.send(Event::Item(item));
        };
        match event {
            AgentEvent::MessageEnd { message: AgentMessage::User(message) } => {
                let text = content_text(&message.content);
                item(Item::text(format!("prompt-{}", uuid::Uuid::new_v4()), "prompt", &text));
            }
            AgentEvent::MessageEnd { message: AgentMessage::Assistant(message) } => {
                let id = uuid::Uuid::new_v4();
                for (index, part) in message.content.iter().enumerate() {
                    match part {
                        AssistantPart::Thinking { thinking, .. } if !thinking.trim().is_empty() => item(Item::text(format!("{id}-{index}"), "thought", thinking)),
                        AssistantPart::Text { text } if !text.trim().is_empty() => {
                            *self.said.lock().unwrap() = Some(text.trim().to_string());
                            item(Item::text(format!("{id}-{index}"), "text", text));
                        }
                        _ => {}
                    }
                }
            }
            AgentEvent::ToolExecutionStart { tool_call_id, tool_name, args } => {
                let step = super::pi::step(tool_name, args, folder);
                self.steps.lock().unwrap().insert(tool_call_id.clone(), step.clone());
                item(Item::tool(tool_call_id.clone(), step));
            }
            AgentEvent::ToolExecutionEnd { tool_call_id, tool_name, result, is_error } => {
                let step = self.steps.lock().unwrap().remove(tool_call_id);
                let mut step = step.unwrap_or_else(|| super::pi::step(tool_name, &Value::Null, folder));
                let text = content_text(&result.content);
                step.status = if self.shared.denied.lock().unwrap().remove(tool_call_id) { "denied" } else if *is_error { "failed" } else { "done" }.into();
                step.output = transcript::output(&text);
                item(Item::tool(tool_call_id.clone(), step));
            }
            AgentEvent::Retry { attempt, max_attempts, error, .. } => {
                let line = error.lines().next().unwrap_or(error);
                item(Item::text(format!("retry-{}", uuid::Uuid::new_v4()), "status", &format!("Retrying ({attempt} of {max_attempts}): {line}")));
            }
            _ => {}
        }
    }
}

fn content_text(content: &[ContentPart]) -> String {
    content
        .iter()
        .filter_map(|part| match part {
            ContentPart::Text { text } => Some(text.as_str()),
            _ => None,
        })
        .collect::<Vec<_>>()
        .join("\n")
}

/// What it does goes through the bot's Access and Auto-review, as a coding agent's does; messages
/// sent while it works reach it at its next step.
struct Hooks {
    shared: Arc<Shared>,
}

#[async_trait]
impl LoopHooks for Hooks {
    async fn steering_messages(&self) -> Vec<AgentMessage> {
        self.shared.state.lock().unwrap().queue.drain(..).map(AgentMessage::user).collect()
    }

    async fn before_tool_call(&self, ctx: BeforeToolCallContext<'_>) -> Option<BeforeToolCallResult> {
        let approval = match ctx.tool_call.name.as_str() {
            "bash" => Approval::Command { command: ctx.args["command"].as_str().unwrap_or("").to_string() },
            "write" | "edit" => {
                let path = ctx.args["path"].as_str().unwrap_or("");
                Approval::Files { paths: vec![lorca_agent::tools::resolve_to_cwd(path, &self.shared.folder).display().to_string()] }
            }
            _ => return None,
        };
        let (tx, rx) = oneshot::channel();
        if self.shared.events.send(Event::Approve(approval, tx)).is_err() {
            return Some(BeforeToolCallResult { block: true, reason: Some("Stopped".into()), ..Default::default() });
        }
        let verdict = tokio::select! {
            verdict = rx => verdict.unwrap_or_else(|_| Err("Stopped".into())),
            _ = ctx.cancel.cancelled() => Err("Stopped".into()),
        };
        let reason = verdict.err()?;
        self.shared.denied.lock().unwrap().insert(ctx.tool_call.id.clone());
        Some(BeforeToolCallResult { block: true, reason: Some(reason), ..Default::default() })
    }

    async fn after_tool_call(&self, ctx: AfterToolCallContext<'_>) -> Option<AfterToolCallResult> {
        crate::secrets::scrub_result(&self.shared.app, ctx.result)
    }
}
