//! Runs bot turns on this Runner: builds the model context from a chat, wires the tools, and
//! turns agent events into transcript messages and app events.

use std::sync::Arc;

use async_trait::async_trait;
use serde_json::{json, Value};
use tinybot_agent::agent_loop::{run_agent_loop_continue, AgentContext, AgentLoopConfig, EventSink, ToolExecutionMode};
use tinybot_agent::provider::{is_server_tool, AssistantEvent, WEB_FETCH_TOOL};
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

/// Appends the user's message, uploads it, and starts the turns it calls for.
///
/// A direct chat's bot always answers. A group runs a room exchange: every member is offered a
/// turn in order and sends or passes, in rounds, until a round goes by with nobody speaking.
pub fn send_user_message(app: Arc<App>, chat_id: &str, text: &str, message_id: Option<String>, attachments: Vec<Attachment>) -> anyhow::Result<Message> {
    let text = text.trim();
    if text.is_empty() && attachments.is_empty() {
        anyhow::bail!("Empty message");
    }
    let chat = app.chat(chat_id).ok_or_else(|| anyhow::anyhow!("Unknown chat"))?;
    // The bytes go out ahead of the message that names them.
    for attachment in &attachments {
        if let Err(error) = crate::files::push_blob(&app, attachment) {
            tracing::warn!(%error, name = %attachment.name, "uploading an attachment");
        }
    }
    let mut message = Message::new(chat_id, Author::You, Body::Text { text: text.to_string(), attachments });
    if let Some(id) = message_id.filter(|id| !id.is_empty()) {
        message.id = id;
    }
    app.upsert_message(message.clone(), true);

    if chat.meta.is_group() {
        let members = turn_order(&chat.meta, &app, text);
        tokio::spawn(run_room(app.clone(), chat_id.to_string(), message.id.clone(), members));
    } else if let Some(bot) = chat.meta.bot_ids.first().and_then(|id| app.bot(id)) {
        start_turn(&app, user_turn_job(&app, chat_id, &bot.id, &message.id));
    }
    Ok(message)
}

fn user_turn_job(app: &Arc<App>, chat_id: &str, bot_id: &str, trigger_message_id: &str) -> Job {
    Job {
        id: format!("job-{}", uuid::Uuid::new_v4()),
        chat_id: chat_id.to_string(),
        bot_id: bot_id.to_string(),
        kind: "turn".into(),
        trigger_message_id: trigger_message_id.to_string(),
        requested_by: app.this_device_id().unwrap_or_default(),
        from_bot_id: None,
        hops: 0,
        round: 0,
        is_winding_down: false,
        created_at: now_secs(),
    }
}

/// Members in chat order, with the ones the message names by `@` first.
pub fn turn_order(chat: &ChatMeta, app: &Arc<App>, text: &str) -> Vec<Bot> {
    let members: Vec<Bot> = chat.bot_ids.iter().filter_map(|id| app.bot(id)).collect();
    let lowered = text.to_lowercase();
    let (mentioned, rest): (Vec<Bot>, Vec<Bot>) =
        members.into_iter().partition(|bot| lowered.contains(&format!("@{}", bot.name.to_lowercase())));
    mentioned.into_iter().chain(rest).collect()
}

// MARK: - Rooms

/// Rounds of turns after one user message before the exchange winds down.
pub const MAX_ROOM_ROUNDS: u32 = 4;
/// How long a member's turn on another Runner may take before the room moves on.
const REMOTE_TURN_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(300);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TurnOutcome {
    /// The member posted at least one message.
    Sent,
    /// The member had nothing to add.
    Pass,
    /// The turn could not run (offline Runner, no provider) or failed.
    Skipped,
}

impl TurnOutcome {
    fn as_str(self) -> &'static str {
        match self {
            TurnOutcome::Sent => "sent",
            TurnOutcome::Pass => "pass",
            TurnOutcome::Skipped => "error",
        }
    }
    fn parse(text: &str) -> TurnOutcome {
        match text {
            "sent" => TurnOutcome::Sent,
            "pass" => TurnOutcome::Pass,
            _ => TurnOutcome::Skipped,
        }
    }
}

/// One group exchange: each member gets a turn in order and either speaks or passes; the room
/// goes another round while anyone spoke, and the last round is marked as winding down. The
/// room holds the chat lock, so turns in one group never overlap; `chats.stop` cancels it.
async fn run_room(app: Arc<App>, chat_id: String, trigger: String, members: Vec<Bot>) {
    if members.is_empty() {
        return;
    }
    let lock = app.chat_lock(&chat_id);
    let _guard = lock.lock().await;

    let cancel = CancellationToken::new();
    let room_id = format!("room-{}", uuid::Uuid::new_v4());
    app.running_jobs.lock().unwrap().insert(room_id.clone(), (chat_id.clone(), String::new(), cancel.clone()));
    app.emit(Event::JobStarted { chat_id: chat_id.clone(), bot_id: String::new(), job_id: room_id.clone() });

    // What each member had seen from others when it last took a turn: a member with nothing
    // new is not offered another turn.
    let mut seen: std::collections::HashMap<String, usize> = std::collections::HashMap::new();
    'rounds: for round in 1..=MAX_ROOM_ROUNDS {
        let mut any_sent = false;
        for bot in &members {
            if cancel.is_cancelled() {
                break 'rounds;
            }
            let Some(chat) = app.chat(&chat_id) else { break 'rounds };
            if !chat.meta.bot_ids.contains(&bot.id) {
                continue;
            }
            if round > 1 && seen.get(&bot.id) == Some(&heard_count(&chat, &bot.id)) {
                continue;
            }
            let job = Job {
                id: format!("job-{}", uuid::Uuid::new_v4()),
                chat_id: chat_id.clone(),
                bot_id: bot.id.clone(),
                kind: "room_turn".into(),
                trigger_message_id: trigger.clone(),
                requested_by: app.this_device_id().unwrap_or_default(),
                from_bot_id: None,
                hops: 0,
                round,
                is_winding_down: round == MAX_ROOM_ROUNDS,
                created_at: now_secs(),
            };
            let outcome = run_member_turn(&app, job, &cancel).await;
            if let Some(chat) = app.chat(&chat_id) {
                seen.insert(bot.id.clone(), heard_count(&chat, &bot.id));
            }
            if outcome == TurnOutcome::Sent {
                any_sent = true;
            }
        }
        if !any_sent {
            break;
        }
    }

    app.running_jobs.lock().unwrap().remove(&room_id);
    app.emit(Event::JobFinished { chat_id, bot_id: String::new(), job_id: room_id });
}

/// Messages in the chat that `bot_id` did not write itself.
fn heard_count(chat: &Chat, bot_id: &str) -> usize {
    chat.messages
        .iter()
        .filter(|m| m.is_complete())
        .filter(|m| matches!(m.body, Body::Text { .. } | Body::Handoff { .. }))
        .filter(|m| !matches!(&m.author, Author::Bot { bot_id: id } if id == bot_id))
        .count()
}

/// Runs one member's turn here or on its Runner and waits for the outcome.
async fn run_member_turn(app: &Arc<App>, job: Job, room_cancel: &CancellationToken) -> TurnOutcome {
    let Some(bot) = app.bot(&job.bot_id) else { return TurnOutcome::Skipped };
    if app.this_device_id().as_deref() == Some(bot.runner_id.as_str()) {
        return run_job_here(app, job, room_cancel.child_token()).await;
    }
    remote_turn(app, job, room_cancel.child_token()).await
}

/// Seals a job to the bot's Runner and waits for its `job_result`. The job counts as running
/// here meanwhile, so the app shows the bot at work and `chats.stop` can drop the wait.
async fn remote_turn(app: &Arc<App>, job: Job, cancel: CancellationToken) -> TurnOutcome {
    let (tx, rx) = tokio::sync::oneshot::channel();
    app.pending_results.lock().unwrap().insert(job.id.clone(), tx);
    let job_id = job.id.clone();
    let chat_id = job.chat_id.clone();
    let bot_id = job.bot_id.clone();
    let outcome = match dispatch_job(app, job) {
        Dispatch::Sent => {
            app.running_jobs.lock().unwrap().insert(job_id.clone(), (chat_id.clone(), bot_id.clone(), cancel.clone()));
            app.emit(Event::JobStarted { chat_id: chat_id.clone(), bot_id: bot_id.clone(), job_id: job_id.clone() });
            let outcome = tokio::select! {
                result = rx => result.map(|text| TurnOutcome::parse(&text)).unwrap_or(TurnOutcome::Skipped),
                _ = tokio::time::sleep(REMOTE_TURN_TIMEOUT) => TurnOutcome::Skipped,
                _ = cancel.cancelled() => TurnOutcome::Skipped,
            };
            app.running_jobs.lock().unwrap().remove(&job_id);
            app.emit(Event::JobFinished { chat_id, bot_id, job_id: job_id.clone() });
            outcome
        }
        Dispatch::Ran | Dispatch::Deferred => TurnOutcome::Skipped,
    };
    app.pending_results.lock().unwrap().remove(&job_id);
    outcome
}

/// A `job_result` from another Runner reached this Device.
pub fn deliver_job_result(app: &Arc<App>, result: JobResult) {
    if let Some(tx) = app.pending_results.lock().unwrap().remove(&result.job_id) {
        let _ = tx.send(result.outcome);
    }
}

// MARK: - Jobs

/// Starts a turn wherever the bot runs: here in the background, or on its Runner with the
/// wait for the result tracked here.
pub fn start_turn(app: &Arc<App>, job: Job) {
    let Some(bot) = app.bot(&job.bot_id) else { return };
    if app.this_device_id().as_deref() == Some(bot.runner_id.as_str()) {
        spawn_local_job(app.clone(), job, None);
    } else {
        let app = app.clone();
        tokio::spawn(async move {
            remote_turn(&app, job, CancellationToken::new()).await;
        });
    }
}

pub enum Dispatch {
    /// Runs on this Device.
    Ran,
    /// Sealed to an online Runner through the relay.
    Sent,
    /// Parked on the relay for an offline Runner, or could not be addressed (a notice says why).
    Deferred,
}

/// Runs the job here when the bot's Runner is this Device; otherwise seals it to that Runner.
pub fn dispatch_job(app: &Arc<App>, job: Job) -> Dispatch {
    let Some(bot) = app.bot(&job.bot_id) else { return Dispatch::Deferred };
    if app.this_device_id().as_deref() == Some(bot.runner_id.as_str()) {
        spawn_local_job(app.clone(), job, None);
        return Dispatch::Ran;
    }
    match app.device(&bot.runner_id) {
        Some(runner) if !runner.box_pubkey.is_empty() => match crate::crypto::seal_json(&runner.box_pubkey, &job) {
            Ok(ciphertext) => {
                app.push_blob("job", Some(runner.id.clone()), ciphertext);
                if app.relay_url().is_none() {
                    app.notice(&job.chat_id, format!("{} runs on {}, but no relay is configured, so this turn cannot leave this Device.", bot.name, runner.name));
                    Dispatch::Deferred
                } else if !app.device_is_online(&runner.id) {
                    app.notice(&job.chat_id, format!("{} runs on {}, which is offline. This turn waits on the relay until it reconnects.", bot.name, runner.name));
                    Dispatch::Deferred
                } else {
                    Dispatch::Sent
                }
            }
            Err(error) => {
                app.notice(&job.chat_id, format!("Could not address the job to {}: {error}", runner.name));
                Dispatch::Deferred
            }
        },
        _ => {
            app.notice(&job.chat_id, format!("{} is assigned to a Runner this Device does not know yet.", bot.name));
            Dispatch::Deferred
        }
    }
}

/// Runs a job on this Runner in the background. Turns in one chat run one at a time; the
/// outcome goes back to the requesting Device when the job came from another one.
pub fn spawn_local_job(app: Arc<App>, job: Job, remote_blob_id: Option<String>) {
    tokio::spawn(async move {
        let lock = app.chat_lock(&job.chat_id);
        let _guard = lock.lock().await;
        let outcome = run_job_here(&app, job.clone(), CancellationToken::new()).await;
        if let Some(id) = remote_blob_id {
            crate::sync::delete_remote_blob(&app, &id).await;
        }
        if app.this_device_id().as_deref() != Some(job.requested_by.as_str()) {
            report_outcome(&app, &job, outcome);
        }
    });
}

/// Runs a job now, registered so `chats.stop` can cancel it. The caller holds any lock needed.
async fn run_job_here(app: &Arc<App>, job: Job, cancel: CancellationToken) -> TurnOutcome {
    app.running_jobs.lock().unwrap().insert(job.id.clone(), (job.chat_id.clone(), job.bot_id.clone(), cancel.clone()));
    app.emit(Event::JobStarted { chat_id: job.chat_id.clone(), bot_id: job.bot_id.clone(), job_id: job.id.clone() });
    let outcome = if cancel.is_cancelled() { TurnOutcome::Skipped } else { run_job(app, &job, cancel).await };
    app.running_jobs.lock().unwrap().remove(&job.id);
    app.emit(Event::JobFinished { chat_id: job.chat_id.clone(), bot_id: job.bot_id.clone(), job_id: job.id.clone() });
    outcome
}

fn report_outcome(app: &Arc<App>, job: &Job, outcome: TurnOutcome) {
    let Some(requester) = app.device(&job.requested_by).filter(|d| !d.box_pubkey.is_empty()) else { return };
    let result = JobResult { job_id: job.id.clone(), chat_id: job.chat_id.clone(), bot_id: job.bot_id.clone(), outcome: outcome.as_str().into() };
    match crate::crypto::seal_json(&requester.box_pubkey, &result) {
        Ok(ciphertext) => {
            app.push_blob("job_result", Some(requester.id.clone()), ciphertext);
        }
        Err(error) => tracing::warn!(%error, "sealing the job result"),
    }
}

// MARK: - The turn

async fn run_job(app: &Arc<App>, job: &Job, cancel: CancellationToken) -> TurnOutcome {
    let Some(bot) = app.bot(&job.bot_id) else { return TurnOutcome::Skipped };
    let Some(chat) = app.chat(&job.chat_id) else { return TurnOutcome::Skipped };

    let provider = match providers::provider_for(app, &bot.provider, bot.model.as_deref()) {
        Ok(provider) => provider,
        Err(reason) => {
            let runner = app.device(&bot.runner_id).map(|d| d.name).unwrap_or_else(|| "its Runner".into());
            app.notice(&job.chat_id, format!("{} cannot run yet: {reason}. Connect {} on {runner}.", bot.name, provider_label(&bot.provider)));
            return TurnOutcome::Skipped;
        }
    };

    let workdir = bot.working_directory(&app.config.home);
    if let Err(error) = std::fs::create_dir_all(&workdir) {
        tracing::warn!(%error, dir = %workdir.display(), "creating the bot's working directory");
    }
    // Attachments another Device sent are fetched before the transcript names them.
    let attachments: Vec<Attachment> = chat
        .messages
        .iter()
        .rev()
        .take(MAX_CONTEXT_MESSAGES)
        .filter_map(|m| match (&m.author, &m.body) {
            (Author::You, Body::Text { attachments, .. }) => Some(attachments.clone()),
            _ => None,
        })
        .flatten()
        .collect();
    crate::files::prefetch(app, &attachments).await;

    let system_prompt = system_prompt(app, &chat, &bot, job);
    let mut messages = transcript_for(app, &chat, &bot, &workdir);
    if job.kind == "room_turn" {
        messages.push(AgentMessage::User(UserMessage::text(room_turn_cue(&chat, &bot, job))));
    } else if messages.last().map(AgentMessage::is_assistant).unwrap_or(true) {
        messages.push(AgentMessage::User(UserMessage::text("Continue.")));
    }
    let mut tools: Vec<Arc<dyn Tool>> = vec![
        Arc::new(ListTeammates { app: app.clone(), chat_id: chat.meta.id.clone() }),
        Arc::new(MessageBot { app: app.clone(), chat_id: chat.meta.id.clone(), bot: bot.clone(), hops: job.hops }),
        Arc::new(CreateBot { app: app.clone(), chat_id: chat.meta.id.clone(), bot: bot.clone() }),
        Arc::new(EditBot { app: app.clone(), bot: bot.clone() }),
        Arc::new(Remember { path: workdir.join("MEMORY.md") }),
    ];
    tools.extend(tinybot_agent::tools::coding_tools(workdir));

    let context = AgentContext { system_prompt, messages, tools };
    let sink = Arc::new(TurnSink(std::sync::Mutex::new(TurnState {
        app: app.clone(),
        chat_id: chat.meta.id.clone(),
        bot_id: bot.id.clone(),
        current: None,
        done_parts: 0,
        tool_messages: Vec::new(),
        sent: false,
        failed: false,
        shown_len: 0,
        last_flush: std::time::Instant::now(),
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
    let mut failed = false;
    if let Err(error) = run_agent_loop_continue(context, &config, &tx, cancel).await {
        tracing::error!(%error, "agent loop");
        failed = true;
    }
    let mut state = sink.0.lock().unwrap();
    state.finish();
    if state.sent {
        TurnOutcome::Sent
    } else if failed || state.failed {
        TurnOutcome::Skipped
    } else {
        TurnOutcome::Pass
    }
}

/// The ephemeral note that opens a member's turn in a group. It is not stored, so the next
/// turn rebuilds it from the transcript.
fn room_turn_cue(chat: &Chat, bot: &Bot, job: &Job) -> String {
    let last_own = chat
        .messages
        .iter()
        .rposition(|m| matches!(&m.author, Author::Bot { bot_id } if bot_id == &bot.id) && matches!(m.body, Body::Text { .. }));
    let new_count = chat.messages[last_own.map(|i| i + 1).unwrap_or(0)..]
        .iter()
        .filter(|m| m.is_complete() && matches!(m.body, Body::Text { .. } | Body::Handoff { .. }))
        .count();
    let mut cue = format!(
        "[Your turn in the group, round {}. {new_count} new message(s) since you last spoke. Reply to the group, or answer with exactly PASS to stay silent.",
        job.round
    );
    if job.is_winding_down {
        cue.push_str(" This exchange is wrapping up: PASS unless something essential is missing.");
    }
    cue.push(']');
    cue
}

/// The text blocks of a reply, in order, skipping thinking and tool calls.
fn text_parts(assistant: &AssistantMessage) -> Vec<&str> {
    assistant
        .content
        .iter()
        .filter_map(|part| match part {
            AssistantPart::Text { text } => Some(text.as_str()),
            _ => None,
        })
        .collect()
}

/// `PASS`, alone, is how a member stays silent on its turn.
fn is_pass(text: &str) -> bool {
    let cleaned: String = text.chars().filter(|c| c.is_alphanumeric()).collect();
    cleaned.eq_ignore_ascii_case("pass")
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
    /// The transcript message showing the text part being streamed. Each text part of a
    /// reply is its own message, so text on either side of a tool call reads as two bubbles.
    current: Option<Message>,
    /// How many text parts of the reply being generated are already complete messages.
    done_parts: usize,
    /// (tool call id, message id)
    tool_messages: Vec<(String, String)>,
    /// A text message reached the chat.
    sent: bool,
    /// The provider or loop reported an error.
    failed: bool,
    /// How much of the reply being generated the chat already shows.
    shown_len: usize,
    last_flush: std::time::Instant,
}

/// Wait this long before showing a reply up to a sentence end rather than a paragraph end.
const SENTENCE_FLUSH_AFTER: std::time::Duration = std::time::Duration::from_millis(1500);

/// Where the visible part of a growing reply may end: after the last completed paragraph, or,
/// once `since_flush` has passed, after the last completed sentence. `None` keeps the shown
/// text as it is.
fn chunk_boundary(text: &str, shown_len: usize, since_flush: std::time::Duration) -> Option<usize> {
    let fresh = text.get(shown_len..)?;
    if let Some(pos) = fresh.rfind("\n\n") {
        let cut = shown_len + pos;
        if cut > shown_len {
            return Some(cut);
        }
    }
    if since_flush < SENTENCE_FLUSH_AFTER {
        return None;
    }
    let mut cut = None;
    let mut iter = fresh.char_indices().peekable();
    while let Some((i, c)) = iter.next() {
        let ends = matches!(c, '.' | '!' | '?' | '。' | '！' | '？');
        let followed_by_space = iter.peek().map(|(_, n)| n.is_whitespace()).unwrap_or(false);
        if ends && followed_by_space {
            cut = Some(shown_len + i + c.len_utf8());
        }
    }
    cut.filter(|&c| c > shown_len)
}

impl TurnState {
    fn handle(&mut self, event: AgentEvent) {
        match event {
            // Replies arrive in chunks, not tokens (after Grok Bot, whose server re-sends the
            // whole message as it grows): the reply so far is shown at paragraph boundaries, or
            // at a sentence boundary once a while has passed, never mid-word. A turn that ends
            // in PASS never shows up at all.
            AgentEvent::MessageStart { message: AgentMessage::Assistant(_) } => {
                self.current = None;
                self.done_parts = 0;
                self.shown_len = 0;
                self.last_flush = std::time::Instant::now();
            }
            // A tool the provider ran on its side (web search): a tool row like any other, so
            // the status line reads "Searching the web…" while it runs, but it is activity only
            // and never rebuilt into a later turn's context.
            AgentEvent::MessageUpdate { assistant_message_event: AssistantEvent::ServerToolStart { id, name, detail }, .. } => {
                let mut message = Message::new(
                    &self.chat_id,
                    Author::Bot { bot_id: self.bot_id.clone() },
                    Body::Tool {
                        name: name.clone(),
                        summary: format!("{}…", server_tool_label(&name)),
                        detail: detail.clone(),
                        is_running: true,
                        call_id: id.clone(),
                        arguments: json!({ "detail": detail }),
                        result: None,
                        is_error: false,
                    },
                );
                message.state = MessageState::Streaming;
                self.app.upsert_message(message.clone(), false);
                self.tool_messages.push((id, message.id));
            }
            AgentEvent::MessageUpdate { assistant_message_event: AssistantEvent::ServerToolEnd { id, name, detail, summary }, .. } => {
                let Some((_, message_id)) = self.tool_messages.iter().find(|(call, _)| *call == id).cloned() else { return };
                let Some(mut message) = self.app.message(&self.chat_id, &message_id) else { return };
                if let Body::Tool { name: n, summary: s, detail: d, is_running, arguments, result, .. } = &mut message.body {
                    *n = name;
                    *s = summary.clone();
                    *d = detail.clone();
                    *is_running = false;
                    *arguments = json!({ "detail": detail });
                    *result = Some(summary);
                }
                message.state = MessageState::Complete;
                self.app.upsert_message(message, true);
            }
            AgentEvent::MessageUpdate { message: AgentMessage::Assistant(assistant), .. } => {
                if "PASS".starts_with(assistant.text().trim()) {
                    return;
                }
                let parts = text_parts(&assistant);
                let Some((last, earlier)) = parts.split_last() else { return };
                // A tool call (or thinking) closed the part before this one: it is a bubble of
                // its own, shown whole.
                for text in &earlier[self.done_parts.min(earlier.len())..] {
                    let message = self.current.take().unwrap_or_else(|| self.new_text_message());
                    self.complete(message, text);
                    self.done_parts += 1;
                    self.shown_len = 0;
                }
                let Some(cut) = chunk_boundary(last, self.shown_len, self.last_flush.elapsed()) else { return };
                let mut current = self.current.take().unwrap_or_else(|| self.new_text_message());
                current.body = Body::text(last[..cut].trim_end());
                current.state = MessageState::Streaming;
                // Uploaded too, so every paired Device watches the reply grow.
                self.app.upsert_message(current.clone(), true);
                self.current = Some(current);
                self.shown_len = cut;
                self.last_flush = std::time::Instant::now();
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
                let text = result.details["message"].as_str().map(str::to_string).unwrap_or_else(|| result.text_content());
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

    fn new_text_message(&self) -> Message {
        Message::new(&self.chat_id, Author::Bot { bot_id: self.bot_id.clone() }, Body::text(String::new()))
    }

    /// A finished text part: a bubble unless it is empty or a pass.
    fn complete(&mut self, mut message: Message, text: &str) {
        let text = text.trim();
        if text.is_empty() || is_pass(text) {
            return;
        }
        message.created_at = now_secs();
        message.body = Body::text(text);
        message.state = MessageState::Complete;
        self.app.upsert_message(message, true);
        self.sent = true;
    }

    fn end_assistant(&mut self, assistant: &AssistantMessage) {
        let parts = text_parts(assistant);
        let whole = assistant.text();
        match assistant.stop_reason {
            StopReason::Error => {
                // Earlier parts stand; the last one (or a fresh bubble) carries the error.
                let (last, earlier) = parts.split_last().map(|(l, e)| (*l, e)).unwrap_or(("", &[]));
                for text in &earlier[self.done_parts.min(earlier.len())..] {
                    let message = self.current.take().unwrap_or_else(|| self.new_text_message());
                    self.complete(message, text);
                }
                let error = assistant.error_message.clone().unwrap_or_else(|| "The provider returned an error".into());
                let mut current = self.current.take().unwrap_or_else(|| self.new_text_message());
                current.created_at = now_secs();
                current.body = Body::text(if last.trim().is_empty() { error.clone() } else { last.trim().to_string() });
                current.state = MessageState::Failed { error };
                self.app.upsert_message(current, true);
                self.failed = true;
            }
            _ => {
                // An empty text is a tool-only turn; a pass says nothing. A stopped reply keeps
                // the text so far.
                if is_pass(&whole) {
                    self.current = None;
                    return;
                }
                for text in &parts[self.done_parts.min(parts.len())..] {
                    let message = self.current.take().unwrap_or_else(|| self.new_text_message());
                    self.complete(message, text);
                }
                self.current = None;
            }
        }
        self.done_parts = parts.len();
    }

    fn finish(&mut self) {
        self.current = None;
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

/// The status line for a server-side tool while it runs, in Grok Bot's words.
fn server_tool_label(name: &str) -> &str {
    if name == WEB_FETCH_TOOL {
        "Reading the web"
    } else {
        "Searching the web"
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

fn system_prompt(app: &Arc<App>, chat: &Chat, bot: &Bot, job: &Job) -> String {
    let members: Vec<Bot> = chat.meta.bot_ids.iter().filter_map(|id| app.bot(id)).collect();
    let runner = app.device(&bot.runner_id);
    let workdir = bot.working_directory(&app.config.home);
    let mut prompt = String::new();
    prompt.push_str(&format!("You are {}, a bot in Tinybot. {}\n", bot.name, bot.label));
    if !bot.description.trim().is_empty() {
        prompt.push_str(&format!("{}\n", bot.description.trim()));
    }
    if !bot.instructions.trim().is_empty() {
        prompt.push_str(&format!("\nInstructions from your owner:\n{}\n", bot.instructions.trim()));
    }

    if chat.meta.is_group() {
        let title = chat.meta.title.clone().unwrap_or_else(|| members.iter().map(|b| b.name.clone()).collect::<Vec<_>>().join(", "));
        prompt.push_str(&format!("\nThis is the group chat \"{title}\" between the user and these bots, in turn order:\n"));
        for member in &members {
            let host = app.device(&member.runner_id).map(|d| d.name).unwrap_or_else(|| "unassigned".into());
            let marker = if member.id == bot.id { " (you)" } else { "" };
            let owner = if chat.meta.owner_bot_id.as_deref() == Some(member.id.as_str()) { " · owner" } else { "" };
            prompt.push_str(&format!("- {}{marker}{owner}: {} · runs on {host}\n", member.name, member.label));
        }
        prompt.push_str(
            "\nEveryone here, including the user, reads every message. After each new message the bots take turns in that \
             order, and a turn is yours now. Other bots' messages appear as \"[Name]: …\".\n\
             - Speak when the new messages ask something of you, name you with @, or need what only you know. \
             Otherwise answer with exactly PASS and nothing else.\n\
             - When a message names other bots with @ and not you, PASS.\n\
             - One message per turn, short, addressed to the group. Do not narrate or repeat what others said.\n\
             - Teammates in this chat read it: talk to them here. message_bot is only for bots outside this chat.\n\
             - The owner holds the work; when it is unclear who should act, leave it to them.\n",
        );
        if job.is_winding_down {
            prompt.push_str("\nThis exchange is wrapping up: PASS unless something essential is missing.\n");
        }
    } else {
        prompt.push_str(
            "\nThis is your direct chat with the user. You always answer here. When the user mentions another bot with @, \
             or a task belongs to a teammate, call message_bot: it delivers your message to that bot, who answers the user \
             in their own chat and can message you back. Then tell the user briefly what you passed on.\n",
        );
    }

    if let Some(from) = job.from_bot_id.as_ref().and_then(|id| app.bot(id)) {
        prompt.push_str(&format!(
            "\nThis turn was started by a message from {} (the last \"[Message from {}]\" entry). Handle their request \
             for the user, and use message_bot to reply to {} only when they need something back.\n",
            from.name, from.name, from.name
        ));
    }

    prompt.push_str(
        "\nTeam: call list_teammates to see every bot. If the right teammate does not exist yet, propose one and create it \
         with create_bot once the user agrees; keep every bot to one clear job. When the user wants a bot, including you, \
         to behave differently, change its profile with edit_bot.\n",
    );
    prompt.push_str(
        "\nMemory: call remember for stable facts, preferences, and summaries worth keeping across chats. Do not store \
         secrets. Memory is not an authoritative source; verify current data before acting on it.\n",
    );
    match std::fs::read_to_string(workdir.join("MEMORY.md")) {
        Ok(memory) if !memory.trim().is_empty() => {
            let shown: String = memory.chars().take(6000).collect();
            prompt.push_str(&format!("\nYour memory:\n{shown}\n"));
        }
        _ => {}
    }

    prompt.push_str(
        "\nWrite like a teammate in a chat app: short and direct, usually one to three sentences, and one line when one \
         line answers it. No preamble, no restating the question, no sign-off. Use a list or code only when it carries \
         the answer; headings are for long reports the user asked for. Ask one question when something is unclear. \
         Markdown renders. Do not invent APIs, files, or results.\n",
    );
    prompt.push_str(&format!("\nTools on your Runner: {}\n", tinybot_agent::tools::coding_tools_snippet()));
    for guideline in tinybot_agent::tools::coding_tools_guidelines() {
        prompt.push_str(&format!("- {guideline}\n"));
    }
    prompt.push_str(&format!(
        "Relative paths resolve against your working directory {}. Work there unless the user names another path. \
         Commands run as the user on that machine, so treat destructive commands with care and say what you ran.\n",
        workdir.display()
    ));
    if let Some(runner) = runner {
        prompt.push_str(&format!("\nYou run on the Runner \"{}\" ({}).", runner.name, runner.os_version));
    }
    prompt
}

/// The chat as `bot` should see it. Other bots' text becomes user messages tagged with their
/// name; this bot's tool rows become tool call and tool result pairs.
pub fn transcript_for(app: &App, chat: &Chat, bot: &Bot, workdir: &std::path::Path) -> Vec<AgentMessage> {
    let mut out = Vec::new();
    let pixels = providers::supports_vision(&bot.provider, bot.model.as_deref());
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
            (Author::You, Body::Text { text, attachments }) if attachments.is_empty() => out.push(user(text, timestamp)),
            (Author::You, Body::Text { text, attachments }) => {
                // A file is named by its path in the workspace; an image is shown as well.
                let mut content = Vec::new();
                if !text.is_empty() {
                    content.push(ContentPart::text(text));
                }
                for attachment in attachments {
                    content.extend(crate::files::content_parts(app, attachment, workdir, pixels));
                }
                out.push(AgentMessage::User(UserMessage { content, timestamp }));
            }
            (Author::Bot { bot_id }, Body::Text { text, .. }) if bot_id == &bot.id => {
                let mut assistant = AssistantMessage::empty("", "");
                assistant.content = vec![AssistantPart::Text { text: text.clone() }];
                assistant.timestamp = timestamp;
                out.push(AgentMessage::Assistant(assistant));
            }
            (Author::Bot { bot_id }, Body::Text { text, .. }) => {
                out.push(user(&format!("[{}]: {text}", name_of(chat, bot_id)), timestamp));
            }
            // Server-side tool rows are a record of activity, not calls to replay.
            (Author::Bot { .. }, Body::Tool { name, .. }) if is_server_tool(name) => {}
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
                    out.push(user(&format!("[Message from {from}]: {reason}"), timestamp));
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

/// Bot names are not in the chat struct; the lookup is primed from the roster and shared by
/// every worker thread that builds a transcript.
fn name_of(_chat: &Chat, bot_id: &str) -> String {
    NAME_CACHE.read().unwrap().get(bot_id).cloned().unwrap_or_else(|| bot_id.to_string())
}

static NAME_CACHE: std::sync::LazyLock<std::sync::RwLock<std::collections::HashMap<String, String>>> =
    std::sync::LazyLock::new(|| std::sync::RwLock::new(std::collections::HashMap::new()));

/// Refreshes the bot-name lookup used while building transcripts.
pub fn prime_names(app: &App) {
    let names: std::collections::HashMap<String, String> =
        app.state.lock().unwrap().bots.iter().map(|b| (b.id.clone(), b.name.clone())).collect();
    *NAME_CACHE.write().unwrap() = names;
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
                    "label": bot.label,
                    "description": bot.description,
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
    hops: u32,
}

#[async_trait]
impl Tool for MessageBot {
    fn name(&self) -> &str {
        "message_bot"
    }
    fn description(&self) -> &str {
        "Send a message to a bot that is not in this chat. It lands in that bot's own chat with the user, where it \
         answers and can message you back. Include the context they need; they do not see this conversation."
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
        if self.hops >= MAX_BOT_HOPS {
            return Err(ToolError(format!(
                "Bots have passed this along {} times without the user. Answer the user instead of messaging another bot.",
                self.hops
            )));
        }
        let all: Vec<Bot> = self.app.state.lock().unwrap().bots.clone();
        let target = all
            .iter()
            .find(|b| b.name.eq_ignore_ascii_case(&name))
            .cloned()
            .ok_or_else(|| ToolError(format!("No bot named {name}. Bots: {}", all.iter().map(|b| b.name.clone()).collect::<Vec<_>>().join(", "))))?;
        if target.id == self.bot.id {
            return Err("You cannot message yourself".into());
        }
        let chat = self.app.chat(&self.chat_id).ok_or("Chat is gone")?;
        if chat.meta.is_group() && chat.meta.bot_ids.contains(&target.id) {
            return Err(ToolError(format!("{} is in this chat and reads it. Say it here instead.", target.name)));
        }

        // Delivered into the target's own chat with the user, as a message from this bot.
        let dm = self.app.dm_with(&target.id, None).map_err(|e| ToolError(e.to_string()))?;
        let incoming = Message::new(
            &dm.meta.id,
            Author::Bot { bot_id: self.bot.id.clone() },
            Body::Handoff { from: self.bot.id.clone(), to: target.id.clone(), reason: message.clone() },
        );
        self.app.upsert_message(incoming.clone(), true);

        let job = Job {
            id: format!("job-{}", uuid::Uuid::new_v4()),
            chat_id: dm.meta.id.clone(),
            bot_id: target.id.clone(),
            kind: "message".into(),
            trigger_message_id: incoming.id,
            requested_by: self.app.this_device_id().unwrap_or_default(),
            from_bot_id: Some(self.bot.id.clone()),
            hops: self.hops + 1,
            round: 0,
            is_winding_down: false,
            created_at: now_secs(),
        };
        start_turn(&self.app, job);

        Ok(ToolResult::text(format!("Messaged {}. They will answer the user in their own chat and can message you back.", target.name))
            .with_details(json!({ "summary": format!("Messaged {}", target.name), "bot_id": target.id, "message": message })))
    }
}

/// Appends a note to the bot's memory file, which the next turns read.
struct Remember {
    path: std::path::PathBuf,
}

#[async_trait]
impl Tool for Remember {
    fn name(&self) -> &str {
        "remember"
    }
    fn description(&self) -> &str {
        "Save a short note to your memory: a stable preference, an important fact, or a summary of work worth keeping \
         across chats. Your memory is shown to you at the start of every turn. Never store secrets."
    }
    fn parameters(&self) -> Value {
        json!({
            "type": "object",
            "properties": { "note": { "type": "string", "description": "One line, in the third person about the user or the work" } },
            "required": ["note"],
            "additionalProperties": false
        })
    }
    async fn execute(&self, _id: &str, args: Value, _cancel: CancellationToken, _on_update: ToolUpdateFn) -> Result<ToolResult, ToolError> {
        let note = args["note"].as_str().unwrap_or("").trim().replace('\n', " ");
        if note.is_empty() {
            return Err("note is required".into());
        }
        if let Some(parent) = self.path.parent() {
            std::fs::create_dir_all(parent).map_err(|e| ToolError(e.to_string()))?;
        }
        let existing = std::fs::read_to_string(&self.path).unwrap_or_default();
        if existing.lines().any(|line| line.trim_start_matches("- ").trim() == note) {
            return Ok(ToolResult::text("Already remembered.").with_details(json!({ "summary": "Already remembered" })));
        }
        let mut lines: Vec<String> = existing.lines().map(str::to_string).collect();
        lines.push(format!("- {note}"));
        while lines.len() > 200 {
            lines.remove(0);
        }
        std::fs::write(&self.path, lines.join("\n") + "\n").map_err(|e| ToolError(e.to_string()))?;
        Ok(ToolResult::text(format!("Remembered: {note}")).with_details(json!({ "summary": "Remembered a note" })))
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
                "label": { "type": "string", "description": "One short line under the name: what it is for" },
                "description": { "type": "string", "description": "A sentence or two about what it does, shown in its profile" },
                "instructions": { "type": "string", "description": "How it should work: scope, tone, what to ask before acting" },
                "provider": { "type": "string", "enum": ["deepseek", "chatgpt"], "description": "Defaults to your own provider" },
                "workdir": { "type": "string", "description": "Working directory for its tools. Defaults to a private workspace under the CLI home; give it your own path to share files" }
            },
            "required": ["name", "label", "instructions"],
            "additionalProperties": false
        })
    }
    fn execution_mode(&self) -> Option<ToolExecutionMode> {
        Some(ToolExecutionMode::Sequential)
    }
    async fn execute(&self, _id: &str, args: Value, _cancel: CancellationToken, _on_update: ToolUpdateFn) -> Result<ToolResult, ToolError> {
        let name = args["name"].as_str().unwrap_or("").trim().trim_start_matches('@').to_string();
        let label = args["label"].as_str().unwrap_or("").trim().to_string();
        let description = args["description"].as_str().unwrap_or("").trim().to_string();
        let instructions = args["instructions"].as_str().unwrap_or("").trim().to_string();
        if name.is_empty() || label.is_empty() {
            return Err("name and label are required".into());
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
            label,
            description,
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
            format!("Created {} on {runner}. They are in this chat now and take turns after you.", created.name)
        } else {
            format!(
                "Created {} on {runner} with their own direct chat. To work with them together, the user can add them to a group chat.",
                created.name
            )
        };
        Ok(ToolResult::text(text).with_details(json!({ "summary": format!("Created {}", created.name), "bot_id": created.id })))
    }
}

/// Changes a teammate's profile (or the caller's own). The new profile applies from that bot's next turn.
struct EditBot {
    app: Arc<App>,
    bot: Bot,
}

#[async_trait]
impl Tool for EditBot {
    fn name(&self) -> &str {
        "edit_bot"
    }
    fn description(&self) -> &str {
        "Change a teammate's profile: name, label, description, instructions, provider, or working directory. Only the fields you \
         pass change. Instructions replace the old ones in full, so include everything the bot should keep. You can edit \
         yourself. Changes apply from that bot's next turn. Edit only when the user asks or agrees."
    }
    fn parameters(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "bot": { "type": "string", "description": "The teammate's current name" },
                "name": { "type": "string", "description": "New name, one or two words" },
                "label": { "type": "string", "description": "New short line under the name: what it is for" },
                "description": { "type": "string", "description": "New sentence or two about what it does" },
                "instructions": { "type": "string", "description": "New instructions, complete: they replace the old ones" },
                "provider": { "type": "string", "enum": ["deepseek", "chatgpt"] },
                "workdir": { "type": "string", "description": "New working directory for its tools" }
            },
            "required": ["bot"],
            "additionalProperties": false
        })
    }
    fn execution_mode(&self) -> Option<ToolExecutionMode> {
        Some(ToolExecutionMode::Sequential)
    }
    async fn execute(&self, _id: &str, args: Value, _cancel: CancellationToken, _on_update: ToolUpdateFn) -> Result<ToolResult, ToolError> {
        let name = args["bot"].as_str().unwrap_or("").trim().trim_start_matches('@').to_string();
        if name.is_empty() {
            return Err("bot is required".into());
        }
        let all: Vec<Bot> = self.app.state.lock().unwrap().bots.clone();
        let target = all
            .iter()
            .find(|b| b.name.eq_ignore_ascii_case(&name))
            .cloned()
            .ok_or_else(|| ToolError(format!("No bot named {name}. Bots: {}", all.iter().map(|b| b.name.clone()).collect::<Vec<_>>().join(", "))))?;

        let field = |key: &str| args[key].as_str().map(str::trim).filter(|v| !v.is_empty()).map(str::to_string);
        let new_name = field("name").map(|n| n.trim_start_matches('@').to_string());
        let label = field("label");
        let description = field("description");
        let instructions = field("instructions");
        let provider = field("provider");
        let workdir = field("workdir");
        if let Some(n) = &new_name {
            if n.chars().count() > 24 {
                return Err("Keep the name under 24 characters".into());
            }
            if all.iter().any(|b| b.id != target.id && b.name.eq_ignore_ascii_case(n)) {
                return Err(ToolError(format!("A bot named {n} already exists. Pick another name.")));
            }
        }
        if let Some(p) = &provider {
            if !matches!(p.as_str(), "deepseek" | "chatgpt") {
                return Err(ToolError(format!("Unknown provider {p}. Use deepseek or chatgpt.")));
            }
        }
        let changed: Vec<&str> = [
            ("name", new_name.is_some()),
            ("label", label.is_some()),
            ("description", description.is_some()),
            ("instructions", instructions.is_some()),
            ("provider", provider.is_some()),
            ("working directory", workdir.is_some()),
        ]
        .into_iter()
        .filter_map(|(label, set)| set.then_some(label))
        .collect();
        if changed.is_empty() {
            return Err("Pass at least one field to change: name, label, description, instructions, provider, or workdir".into());
        }

        let updated = self
            .app
            .update_bot(&target.id, |bot| {
                if let Some(v) = new_name {
                    bot.name = v;
                }
                if let Some(v) = label {
                    bot.label = v;
                }
                if let Some(v) = description {
                    bot.description = v;
                }
                if let Some(v) = instructions {
                    bot.instructions = v;
                }
                if let Some(v) = provider {
                    bot.provider = v;
                }
                if let Some(v) = workdir {
                    bot.workdir = Some(v);
                }
            })
            .map_err(|e| ToolError(e.to_string()))?;

        let what = changed.join(", ");
        let text = if target.id == self.bot.id {
            format!("Updated your own profile ({what}). The new profile applies from your next turn; finish this one as you are.")
        } else {
            format!("Updated {} ({what}). The new profile applies from their next turn.", updated.name)
        };
        Ok(ToolResult::text(text).with_details(json!({ "summary": format!("Updated {}", updated.name), "bot_id": updated.id, "changed": changed })))
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

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    #[test]
    fn chunks_end_at_paragraphs_first() {
        let text = "First para.\n\nSecond para that is still";
        assert_eq!(chunk_boundary(text, 0, Duration::ZERO), Some(11));
        // Nothing new to show until the second paragraph completes or time passes.
        assert_eq!(chunk_boundary(text, 11, Duration::ZERO), None);
        assert_eq!(chunk_boundary(text, 11, Duration::from_secs(2)), None);
        let text = "First para.\n\nSecond para done. Third starts";
        assert_eq!(chunk_boundary(text, 11, Duration::from_secs(2)), Some("First para.\n\nSecond para done.".len()));
    }

    #[test]
    fn a_pass_never_shows() {
        assert!(is_pass("PASS"));
        assert!(is_pass(" pass. "));
        assert!(!is_pass("Pass the salt"));
    }
}
