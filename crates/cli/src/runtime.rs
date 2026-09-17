//! What a Device does with turns: sends the user's message, runs a group's room exchange,
//! starts jobs here or on the bot's Runner through the relay, and takes their results. The
//! turn itself, with its model, tools, and memory, lives in `turns` under the `runner` feature.

use std::sync::Arc;

use tokio_util::sync::CancellationToken;

use crate::app::App;
use crate::config::now_secs;
use crate::events::Event;
use crate::model::*;

/// How a chat is named in a bot's memory: `your chat with the user`, `group "Standup"`.
pub fn chat_source(chat: &Chat) -> String {
    if chat.meta.is_group() {
        let title = chat.meta.title.clone().filter(|t| !t.trim().is_empty()).unwrap_or_else(|| {
            chat.meta.bot_ids.iter().map(|id| name_of(chat, id)).collect::<Vec<_>>().join(", ")
        });
        format!("group \"{title}\"")
    } else {
        "your chat with the user".into()
    }
}

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
    #[cfg(feature = "runner")]
    let outcome = if cancel.is_cancelled() { TurnOutcome::Skipped } else { crate::turns::run_job(app, &job, cancel).await };
    #[cfg(not(feature = "runner"))]
    let outcome = {
        let _ = cancel;
        app.notice(&job.chat_id, "This Device does not run bots; assign the bot to a Runner.");
        TurnOutcome::Skipped
    };
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

/// Bot names are not in the chat struct; the lookup is primed from the roster and shared by
/// every worker thread that builds a transcript.
pub fn name_of(_chat: &Chat, bot_id: &str) -> String {
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
