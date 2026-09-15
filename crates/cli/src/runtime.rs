//! Runs bot turns on this Runner: builds the model context from a chat, wires the tools, and
//! turns agent events into transcript messages and app events.

use std::sync::Arc;

use async_trait::async_trait;
use serde_json::{json, Value};
use tinybot_agent::agent_loop::{run_agent_loop_continue, AgentContext, AgentLoopConfig, EventSink, ToolExecutionMode};
use tinybot_agent::{
    AgentEvent, AgentMessage, AssistantMessage, AssistantPart, ContentPart, StopReason, Tool, ToolCall, ToolError,
    ToolResult, ToolResultMessage, ToolUpdateFn, UserMessage,
};
use tokio::sync::mpsc;
use tokio_util::sync::CancellationToken;

use crate::app::App;
use crate::config::now_secs;
use crate::events::Event;
use crate::model::*;
use crate::providers;

const MAX_CONTEXT_MESSAGES: usize = 80;

// MARK: - Sending

/// Appends the user's message, uploads it, and starts a job for every bot that should answer.
pub fn send_user_message(app: &Arc<App>, chat_id: &str, text: &str, message_id: Option<String>) -> anyhow::Result<Message> {
    let text = text.trim();
    if text.is_empty() {
        anyhow::bail!("Empty message");
    }
    let chat = app.chat(chat_id).ok_or_else(|| anyhow::anyhow!("Unknown chat"))?;
    let mut message = Message::new(chat_id, Author::You, Body::Text { text: text.to_string() });
    if let Some(id) = message_id.filter(|id| !id.is_empty()) {
        message.id = id;
    }
    app.upsert_message(message.clone(), true);

    let members: Vec<Bot> = chat.meta.bot_ids.iter().filter_map(|id| app.bot(id)).collect();
    for bot in responders(&chat.meta, &members, text) {
        let job = Job {
            id: format!("job-{}", uuid::Uuid::new_v4()),
            chat_id: chat_id.to_string(),
            bot_id: bot.id.clone(),
            kind: "turn".into(),
            trigger_message_id: message.id.clone(),
            requested_by: app.this_device_id().unwrap_or_default(),
            created_at: now_secs(),
        };
        dispatch_job(app, job);
    }
    Ok(message)
}

/// DM: the one bot. Group: `@everyone`, else the bots mentioned, else the first bot.
pub fn responders(chat: &ChatMeta, members: &[Bot], text: &str) -> Vec<Bot> {
    if members.is_empty() {
        return Vec::new();
    }
    if !chat.is_group() {
        return vec![members[0].clone()];
    }
    let lowered = text.to_lowercase();
    if lowered.contains("@everyone") {
        return members.to_vec();
    }
    let mentioned: Vec<Bot> = members
        .iter()
        .filter(|bot| lowered.contains(&format!("@{}", bot.name.to_lowercase())))
        .cloned()
        .collect();
    if mentioned.is_empty() {
        vec![members[0].clone()]
    } else {
        mentioned
    }
}

/// Runs the job here when the bot's Runner is this Device; otherwise seals it to that Runner.
pub fn dispatch_job(app: &Arc<App>, job: Job) {
    let Some(bot) = app.bot(&job.bot_id) else { return };
    if app.this_device_id().as_deref() == Some(bot.runner_id.as_str()) {
        spawn_local_job(app.clone(), job, None);
        return;
    }
    match app.device(&bot.runner_id) {
        Some(runner) if !runner.box_pubkey.is_empty() => match crate::crypto::seal_json(&runner.box_pubkey, &job) {
            Ok(ciphertext) => {
                app.push_blob("job", Some(runner.id.clone()), ciphertext);
                if app.relay_url().is_none() {
                    app.notice(&job.chat_id, format!("{} runs on {}, but no relay is configured, so this turn cannot leave this Device.", bot.name, runner.name));
                } else if !app.device_is_online(&runner.id) {
                    app.notice(&job.chat_id, format!("{} runs on {}, which is offline. This turn waits on the relay until it reconnects.", bot.name, runner.name));
                }
            }
            Err(error) => app.notice(&job.chat_id, format!("Could not address the job to {}: {error}", runner.name)),
        },
        _ => app.notice(&job.chat_id, format!("{} is assigned to a Runner this Device does not know yet.", bot.name)),
    }
}

pub fn spawn_local_job(app: Arc<App>, job: Job, remote_blob_id: Option<String>) {
    let cancel = CancellationToken::new();
    app.running_jobs.lock().unwrap().insert(job.id.clone(), (job.chat_id.clone(), job.bot_id.clone(), cancel.clone()));
    app.emit(Event::JobStarted { chat_id: job.chat_id.clone(), bot_id: job.bot_id.clone(), job_id: job.id.clone() });

    tokio::spawn(async move {
        let lock = app.chat_lock(&job.chat_id);
        let _guard = lock.lock().await;
        if !cancel.is_cancelled() {
            run_job(&app, &job, cancel.clone()).await;
        }
        app.running_jobs.lock().unwrap().remove(&job.id);
        app.emit(Event::JobFinished { chat_id: job.chat_id.clone(), bot_id: job.bot_id.clone(), job_id: job.id.clone() });
        if let Some(id) = remote_blob_id {
            crate::sync::delete_remote_blob(&app, &id).await;
        }
    });
}

// MARK: - The turn

async fn run_job(app: &Arc<App>, job: &Job, cancel: CancellationToken) {
    let Some(bot) = app.bot(&job.bot_id) else { return };
    let Some(chat) = app.chat(&job.chat_id) else { return };

    let provider = match providers::provider_for(app, &bot.provider, bot.model.as_deref()) {
        Ok(provider) => provider,
        Err(reason) => {
            let runner = app.device(&bot.runner_id).map(|d| d.name).unwrap_or_else(|| "its Runner".into());
            app.notice(&job.chat_id, format!("{} cannot run yet: {reason}. Connect {} on {runner}.", bot.name, provider_label(&bot.provider)));
            return;
        }
    };

    let system_prompt = system_prompt(app, &chat, &bot);
    let mut messages = transcript_for(&chat, &bot);
    if messages.last().map(AgentMessage::is_assistant).unwrap_or(true) {
        messages.push(AgentMessage::User(UserMessage::text("Continue.")));
    }

    let workdir = bot.working_directory(&app.config.home);
    if let Err(error) = std::fs::create_dir_all(&workdir) {
        tracing::warn!(%error, dir = %workdir.display(), "creating the bot's working directory");
    }
    let mut tools: Vec<Arc<dyn Tool>> = vec![
        Arc::new(ListTeammates { app: app.clone(), chat_id: chat.meta.id.clone() }),
        Arc::new(MessageBot { app: app.clone(), chat_id: chat.meta.id.clone(), bot: bot.clone() }),
        Arc::new(CreateBot { app: app.clone(), chat_id: chat.meta.id.clone(), bot: bot.clone() }),
    ];
    tools.extend(tinybot_agent::tools::coding_tools(workdir));

    let context = AgentContext { system_prompt, messages, tools };
    let sink = Arc::new(TurnSink(std::sync::Mutex::new(TurnState {
        app: app.clone(),
        chat_id: chat.meta.id.clone(),
        bot_id: bot.id.clone(),
        current: None,
        tool_messages: Vec::new(),
    })));
    let config = AgentLoopConfig {
        provider,
        hooks: Arc::new(tinybot_agent::NoHooks),
        tool_execution: ToolExecutionMode::Sequential,
        sink: Some(sink.clone()),
    };

    // Events reach the transcript through the sink, in order with the tools' own writes.
    let (tx, _rx) = mpsc::channel::<AgentEvent>(1);
    drop(_rx);
    if let Err(error) = run_agent_loop_continue(context, &config, &tx, cancel).await {
        tracing::error!(%error, "agent loop");
    }
    sink.0.lock().unwrap().finish();
}

struct TurnSink(std::sync::Mutex<TurnState>);

#[async_trait]
impl EventSink for TurnSink {
    async fn on_event(&self, event: &AgentEvent) {
        self.0.lock().unwrap().handle(event.clone());
    }
}

fn provider_label(kind: &str) -> &str {
    match kind {
        "deepseek" => "DeepSeek",
        "chatgpt" => "ChatGPT",
        other => other,
    }
}

/// Reduces agent events into transcript messages.
struct TurnState {
    app: Arc<App>,
    chat_id: String,
    bot_id: String,
    /// The assistant message being streamed, as the app sees it.
    current: Option<Message>,
    /// (tool call id, message id)
    tool_messages: Vec<(String, String)>,
}

impl TurnState {
    fn handle(&mut self, event: AgentEvent) {
        match event {
            AgentEvent::MessageStart { message: AgentMessage::Assistant(_) } => {
                let mut message = Message::new(&self.chat_id, Author::Bot { bot_id: self.bot_id.clone() }, Body::Text { text: String::new() });
                message.state = MessageState::Thinking;
                self.app.upsert_message(message.clone(), false);
                self.current = Some(message);
            }
            AgentEvent::MessageUpdate { message: AgentMessage::Assistant(assistant), .. } => {
                let text = assistant.text();
                if text.is_empty() {
                    return;
                }
                if let Some(current) = self.current.as_mut() {
                    current.body = Body::Text { text };
                    current.state = MessageState::Streaming;
                    self.app.upsert_message(current.clone(), false);
                }
            }
            AgentEvent::MessageEnd { message: AgentMessage::Assistant(assistant) } => {
                self.end_assistant(&assistant);
            }
            AgentEvent::ToolExecutionStart { tool_call_id, tool_name, args } => {
                let mut message = Message::new(
                    &self.chat_id,
                    Author::Bot { bot_id: self.bot_id.clone() },
                    Body::Tool {
                        name: tool_name.clone(),
                        summary: format!("Running {}…", tool_label(&tool_name)),
                        detail: serde_json::to_string_pretty(&args).unwrap_or_default(),
                        is_running: true,
                        call_id: tool_call_id.clone(),
                        arguments: args,
                        result: None,
                        is_error: false,
                    },
                );
                message.state = MessageState::Streaming;
                self.app.upsert_message(message.clone(), false);
                self.tool_messages.push((tool_call_id, message.id));
            }
            AgentEvent::ToolExecutionEnd { tool_call_id, tool_name, result, is_error } => {
                let Some((_, message_id)) = self.tool_messages.iter().find(|(id, _)| *id == tool_call_id).cloned() else { return };
                let Some(mut message) = self.app.message(&self.chat_id, &message_id) else { return };
                let text = result.text_content();
                let summary = result.details["summary"]
                    .as_str()
                    .map(str::to_string)
                    .unwrap_or_else(|| first_line(&text, 80).unwrap_or_else(|| format!("{} finished", tool_label(&tool_name))));
                if let Body::Tool { summary: s, detail, is_running, result: r, is_error: e, .. } = &mut message.body {
                    *s = if is_error { format!("{} failed", tool_label(&tool_name)) } else { summary };
                    *detail = text.clone();
                    *is_running = false;
                    *r = Some(text);
                    *e = is_error;
                }
                message.state = MessageState::Complete;
                self.app.upsert_message(message, true);
            }
            _ => {}
        }
    }

    fn end_assistant(&mut self, assistant: &AssistantMessage) {
        let Some(mut current) = self.current.take() else { return };
        let text = assistant.text();
        match assistant.stop_reason {
            StopReason::Error => {
                let error = assistant.error_message.clone().unwrap_or_else(|| "The provider returned an error".into());
                current.body = Body::Text { text: if text.is_empty() { error.clone() } else { text } };
                current.state = MessageState::Failed { error };
                self.app.upsert_message(current, true);
            }
            StopReason::Aborted => {
                if text.is_empty() {
                    self.app.remove_message(&self.chat_id, &current.id, false);
                } else {
                    current.body = Body::Text { text };
                    current.state = MessageState::Complete;
                    self.app.upsert_message(current, true);
                }
            }
            _ => {
                if text.is_empty() {
                    // Tool-only turn: the tool rows carry it.
                    self.app.remove_message(&self.chat_id, &current.id, false);
                } else {
                    current.body = Body::Text { text };
                    current.state = MessageState::Complete;
                    self.app.upsert_message(current, true);
                }
            }
        }
    }

    fn finish(&mut self) {
        if let Some(current) = self.current.take() {
            self.app.remove_message(&self.chat_id, &current.id, false);
        }
        // A tool that never reported back (cancelled) should not stay spinning.
        for (_, message_id) in self.tool_messages.drain(..) {
            if let Some(mut message) = self.app.message(&self.chat_id, &message_id) {
                if let Body::Tool { is_running, summary, .. } = &mut message.body {
                    if *is_running {
                        *is_running = false;
                        *summary = "Stopped".into();
                        message.state = MessageState::Complete;
                        self.app.upsert_message(message, true);
                    }
                }
            }
        }
    }
}

fn tool_label(name: &str) -> &str {
    match name {
        "message_bot" => "message_bot",
        "list_teammates" => "list_teammates",
        other => other,
    }
}

fn first_line(text: &str, max: usize) -> Option<String> {
    let line = text.lines().next()?.trim();
    if line.is_empty() {
        return None;
    }
    Some(if line.chars().count() > max { format!("{}…", line.chars().take(max).collect::<String>()) } else { line.to_string() })
}

// MARK: - Context

fn system_prompt(app: &Arc<App>, chat: &Chat, bot: &Bot) -> String {
    let members: Vec<Bot> = chat.meta.bot_ids.iter().filter_map(|id| app.bot(id)).collect();
    let runner = app.device(&bot.runner_id);
    let mut prompt = String::new();
    prompt.push_str(&format!("You are {}, a bot in Tinybot. {}\n", bot.name, bot.tagline));
    if !bot.instructions.trim().is_empty() {
        prompt.push_str(&format!("\nInstructions from your owner:\n{}\n", bot.instructions.trim()));
    }
    if chat.meta.is_group() {
        let title = chat.meta.title.clone().unwrap_or_else(|| members.iter().map(|b| b.name.clone()).collect::<Vec<_>>().join(", "));
        prompt.push_str(&format!("\nThis is the group chat \"{title}\" between the user and these bots:\n"));
        for member in &members {
            let host = app.device(&member.runner_id).map(|d| d.name).unwrap_or_else(|| "unassigned".into());
            let marker = if member.id == bot.id { " (you)" } else { "" };
            prompt.push_str(&format!("- {}{marker}: {} · runs on {host}\n", member.name, member.tagline));
        }
        prompt.push_str(
            "\nMessages from other bots appear as \"[Name]: …\". The user reads everything. Answer the user directly; \
             do not narrate what other bots said unless it adds something. When another bot is better placed for a task, \
             call message_bot to hand it off with a clear ask, then stop and let them answer. Call list_teammates to see who is available. \
             If the right teammate does not exist yet, create one with create_bot; it joins this group chat.\n",
        );
    } else {
        prompt.push_str(
            "\nThis is a direct chat with the user. Call list_teammates to see other bots. Teammates answer in group chats: \
             to involve one, ask the user to add it to a group chat with you, or create a new teammate with create_bot when a \
             specialist is clearly missing. Do not hand off from a direct chat.\n",
        );
    }
    prompt.push_str(
        "\nBuilding a team: keep every bot to one job with a short, concrete description. Propose the team before creating it, \
         and create bots only when the user agrees or has asked you to set things up.\n",
    );
    prompt.push_str("\nBe concise and concrete. Markdown renders. Do not invent APIs, files, or results.\n");
    prompt.push_str(&format!("\nTools on your Runner: {}\n", tinybot_agent::tools::coding_tools_snippet()));
    for guideline in tinybot_agent::tools::coding_tools_guidelines() {
        prompt.push_str(&format!("- {guideline}\n"));
    }
    prompt.push_str(&format!(
        "Relative paths resolve against your working directory {}. Work there unless the user names another path. \
         Commands run as the user on that machine, so treat destructive commands with care and say what you ran.\n",
        bot.working_directory(&app.config.home).display()
    ));
    if let Some(runner) = runner {
        prompt.push_str(&format!("\nYou run on the Runner \"{}\" ({}).", runner.name, runner.os_version));
    }
    prompt
}

/// The chat as `bot` should see it. Other bots' text becomes user messages tagged with their
/// name; this bot's tool rows become tool call and tool result pairs.
pub fn transcript_for(chat: &Chat, bot: &Bot) -> Vec<AgentMessage> {
    let mut out = Vec::new();
    let start = chat.messages.len().saturating_sub(MAX_CONTEXT_MESSAGES);
    for message in &chat.messages[start..] {
        if !message.is_complete() {
            if let MessageState::Failed { .. } = message.state {
                continue;
            }
            continue;
        }
        let timestamp = (message.created_at * 1000.0) as u64;
        match (&message.author, &message.body) {
            (Author::You, Body::Text { text }) => out.push(user(text, timestamp)),
            (Author::Bot { bot_id }, Body::Text { text }) if bot_id == &bot.id => {
                let mut assistant = AssistantMessage::empty("", "");
                assistant.content = vec![AssistantPart::Text { text: text.clone() }];
                assistant.timestamp = timestamp;
                out.push(AgentMessage::Assistant(assistant));
            }
            (Author::Bot { bot_id }, Body::Text { text }) => {
                out.push(user(&format!("[{}]: {text}", name_of(chat, bot_id)), timestamp));
            }
            (Author::Bot { bot_id }, Body::Tool { name, call_id, arguments, result, is_error, .. }) if bot_id == &bot.id => {
                let call_id = if call_id.is_empty() { message.id.clone() } else { call_id.clone() };
                let mut assistant = AssistantMessage::empty("", "");
                assistant.content = vec![AssistantPart::ToolCall(ToolCall { id: call_id.clone(), name: name.clone(), arguments: arguments.clone() })];
                assistant.stop_reason = StopReason::ToolUse;
                assistant.timestamp = timestamp;
                out.push(AgentMessage::Assistant(assistant));
                out.push(AgentMessage::ToolResult(ToolResultMessage {
                    tool_call_id: call_id,
                    tool_name: name.clone(),
                    content: vec![ContentPart::text(result.clone().unwrap_or_default())],
                    details: Value::Null,
                    is_error: *is_error,
                    timestamp,
                }));
            }
            (Author::Bot { bot_id }, Body::Handoff { to, reason, .. }) if bot_id != &bot.id => {
                let from = name_of(chat, bot_id);
                if to == &bot.id {
                    out.push(user(&format!("[{from} handed this to you]: {reason}"), timestamp));
                } else {
                    out.push(user(&format!("[{from} → {}]: {reason}", name_of(chat, to)), timestamp));
                }
            }
            _ => {}
        }
    }
    // Drop a leading tool result with no call, which a truncated window can produce.
    while matches!(out.first(), Some(AgentMessage::ToolResult(_))) {
        out.remove(0);
    }
    out
}

fn user(text: &str, timestamp: u64) -> AgentMessage {
    AgentMessage::User(UserMessage { content: vec![ContentPart::text(text)], timestamp })
}

/// Bot names are not in the chat struct; the caller resolves through the app when it can.
fn name_of(_chat: &Chat, bot_id: &str) -> String {
    NAME_CACHE.with(|cache| cache.borrow().get(bot_id).cloned()).unwrap_or_else(|| bot_id.to_string())
}

thread_local! {
    static NAME_CACHE: std::cell::RefCell<std::collections::HashMap<String, String>> = std::cell::RefCell::new(std::collections::HashMap::new());
}

/// Refreshes the bot-name lookup used while building transcripts.
pub fn prime_names(app: &Arc<App>) {
    let names: std::collections::HashMap<String, String> =
        app.state.lock().unwrap().bots.iter().map(|b| (b.id.clone(), b.name.clone())).collect();
    NAME_CACHE.with(|cache| *cache.borrow_mut() = names);
}

// MARK: - Tools

struct ListTeammates {
    app: Arc<App>,
    chat_id: String,
}

#[async_trait]
impl Tool for ListTeammates {
    fn name(&self) -> &str {
        "list_teammates"
    }
    fn description(&self) -> &str {
        "List the other bots on this account: their name, what they are good at, which Runner they run on, and whether that Runner is online."
    }
    fn parameters(&self) -> Value {
        json!({ "type": "object", "properties": {}, "additionalProperties": false })
    }
    async fn execute(&self, _id: &str, _args: Value, _cancel: CancellationToken, _on_update: ToolUpdateFn) -> Result<ToolResult, ToolError> {
        let chat = self.app.chat(&self.chat_id);
        let bots = self.app.state.lock().unwrap().bots.clone();
        let rows: Vec<Value> = bots
            .iter()
            .map(|bot| {
                let runner = self.app.device(&bot.runner_id);
                json!({
                    "name": bot.name,
                    "tagline": bot.tagline,
                    "runner": runner.as_ref().map(|d| d.name.clone()).unwrap_or_else(|| "unassigned".into()),
                    "provider": bot.provider,
                    "online": self.app.device_is_online(&bot.runner_id),
                    "in_this_chat": chat.as_ref().map(|c| c.meta.bot_ids.contains(&bot.id)).unwrap_or(false),
                })
            })
            .collect();
        let text = serde_json::to_string_pretty(&json!({ "teammates": rows })).unwrap_or_default();
        Ok(ToolResult::text(text).with_details(json!({ "summary": format!("Listed {} teammates", rows.len()) })))
    }
}

struct MessageBot {
    app: Arc<App>,
    chat_id: String,
    bot: Bot,
}

#[async_trait]
impl Tool for MessageBot {
    fn name(&self) -> &str {
        "message_bot"
    }
    fn description(&self) -> &str {
        "Hand work to another bot in this chat. The message is shown in the transcript and the other bot answers in this chat on its own Runner. After calling this, stop; do not answer for them."
    }
    fn parameters(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "bot": { "type": "string", "description": "The teammate's name" },
                "message": { "type": "string", "description": "What you want them to do, with the context they need" }
            },
            "required": ["bot", "message"],
            "additionalProperties": false
        })
    }
    fn execution_mode(&self) -> Option<ToolExecutionMode> {
        Some(ToolExecutionMode::Sequential)
    }
    async fn execute(&self, _id: &str, args: Value, _cancel: CancellationToken, _on_update: ToolUpdateFn) -> Result<ToolResult, ToolError> {
        let name = args["bot"].as_str().unwrap_or("").trim().trim_start_matches('@').to_string();
        let message = args["message"].as_str().unwrap_or("").trim().to_string();
        if name.is_empty() || message.is_empty() {
            return Err("bot and message are required".into());
        }
        let chat = self.app.chat(&self.chat_id).ok_or("Chat is gone")?;
        let members: Vec<Bot> = chat.meta.bot_ids.iter().filter_map(|id| self.app.bot(id)).collect();
        let target = members
            .iter()
            .find(|b| b.name.eq_ignore_ascii_case(&name))
            .cloned()
            .ok_or_else(|| ToolError(format!("{name} is not in this chat. Members: {}", members.iter().map(|b| b.name.clone()).collect::<Vec<_>>().join(", "))))?;
        if target.id == self.bot.id {
            return Err("You cannot hand off to yourself".into());
        }

        let handoff = Message::new(
            &self.chat_id,
            Author::Bot { bot_id: self.bot.id.clone() },
            Body::Handoff { from: self.bot.id.clone(), to: target.id.clone(), reason: message.clone() },
        );
        self.app.upsert_message(handoff.clone(), true);

        let job = Job {
            id: format!("job-{}", uuid::Uuid::new_v4()),
            chat_id: self.chat_id.clone(),
            bot_id: target.id.clone(),
            kind: "handoff".into(),
            trigger_message_id: handoff.id.clone(),
            requested_by: self.app.this_device_id().unwrap_or_default(),
            created_at: now_secs(),
        };
        dispatch_job(&self.app, job);

        Ok(ToolResult::text(format!("Handed off to {}. Their reply will appear in this chat.", target.name))
            .with_details(json!({ "summary": format!("Handed off to {}", target.name) }))
            .terminating())
    }
}

struct CreateBot {
    app: Arc<App>,
    chat_id: String,
    bot: Bot,
}

#[async_trait]
impl Tool for CreateBot {
    fn name(&self) -> &str {
        "create_bot"
    }
    fn description(&self) -> &str {
        "Create a new teammate bot on your Runner with one clear job. In a group chat the new bot joins it right away; \
         in a direct chat it gets its own direct chat and can be added to a group later. Propose the team first and create \
         bots only once the user agrees."
    }
    fn parameters(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "name": { "type": "string", "description": "Short name, one or two words" },
                "tagline": { "type": "string", "description": "One line: what it is good at" },
                "instructions": { "type": "string", "description": "How it should work: scope, tone, what to ask before acting" },
                "provider": { "type": "string", "enum": ["deepseek", "chatgpt"], "description": "Defaults to your own provider" },
                "workdir": { "type": "string", "description": "Working directory for its tools. Defaults to a private workspace under the CLI home; give it your own path to share files" }
            },
            "required": ["name", "tagline", "instructions"],
            "additionalProperties": false
        })
    }
    fn execution_mode(&self) -> Option<ToolExecutionMode> {
        Some(ToolExecutionMode::Sequential)
    }
    async fn execute(&self, _id: &str, args: Value, _cancel: CancellationToken, _on_update: ToolUpdateFn) -> Result<ToolResult, ToolError> {
        let name = args["name"].as_str().unwrap_or("").trim().trim_start_matches('@').to_string();
        let tagline = args["tagline"].as_str().unwrap_or("").trim().to_string();
        let instructions = args["instructions"].as_str().unwrap_or("").trim().to_string();
        if name.is_empty() || tagline.is_empty() {
            return Err("name and tagline are required".into());
        }
        if name.chars().count() > 24 {
            return Err("Keep the name under 24 characters".into());
        }
        if self.app.state.lock().unwrap().bots.iter().any(|b| b.name.eq_ignore_ascii_case(&name)) {
            return Err(ToolError(format!("A bot named {name} already exists. Pick another name or use message_bot.")));
        }
        let provider = args["provider"].as_str().map(str::to_string).unwrap_or_else(|| self.bot.provider.clone());
        let (symbol_name, accent) = look_for(&name);
        let bot = Bot {
            id: String::new(),
            name: name.clone(),
            tagline,
            symbol_name,
            accent,
            runner_id: self.bot.runner_id.clone(),
            provider,
            model: None,
            instructions,
            workdir: args["workdir"].as_str().map(|w| w.trim().to_string()).filter(|w| !w.is_empty()),
            created_at: 0.0,
        };
        let (created, _dm) = self.app.create_bot_with_dm(bot, None).map_err(|e| ToolError(e.to_string()))?;
        prime_names(&self.app);

        let mut joined_here = false;
        if let Some(chat) = self.app.chat(&self.chat_id) {
            if chat.meta.is_group() && chat.meta.bot_ids.len() < MAX_GROUP_BOTS {
                let id = created.id.clone();
                self.app
                    .update_chat_meta(&self.chat_id, |meta| {
                        if !meta.bot_ids.contains(&id) {
                            meta.bot_ids.push(id.clone());
                        }
                    })
                    .map_err(|e| ToolError(e.to_string()))?;
                self.app.notice(&self.chat_id, format!("{} created {} and added them to the chat.", self.bot.name, created.name));
                joined_here = true;
            }
        }

        let runner = self.app.device(&created.runner_id).map(|d| d.name).unwrap_or_else(|| "this Runner".into());
        let text = if joined_here {
            format!("Created {} on {runner}. They are in this chat now; hand work to them with message_bot.", created.name)
        } else {
            format!(
                "Created {} on {runner} with their own direct chat. To work with them together, the user can add them to a group chat.",
                created.name
            )
        };
        Ok(ToolResult::text(text).with_details(json!({ "summary": format!("Created {}", created.name), "bot_id": created.id })))
    }
}

/// A stable look for a bot the model named, so teammates are told apart in the sidebar.
fn look_for(name: &str) -> (String, String) {
    const LOOKS: &[(&str, &str)] = &[
        ("chevron.left.forwardslash.chevron.right", "blue"),
        ("binoculars.fill", "teal"),
        ("pencil.and.scribble", "pink"),
        ("bolt.horizontal.fill", "orange"),
        ("leaf.fill", "green"),
        ("wand.and.stars", "purple"),
        ("flame.fill", "red"),
        ("sparkles", "indigo"),
    ];
    let hash = name.to_lowercase().bytes().fold(7u32, |acc, b| acc.wrapping_mul(31).wrapping_add(b as u32));
    let (symbol, accent) = LOOKS[(hash as usize) % LOOKS.len()];
    (symbol.to_string(), accent.to_string())
}
