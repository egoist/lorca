//! What a Device does with turns: sends the user's message, runs a group's room exchange,
//! starts jobs here or on the bot's Runner through the relay, and takes their results. The
//! turn itself, with its model, tools, and memory, lives in `turns` under the `runner` feature.

use std::sync::Arc;

use tokio_util::sync::CancellationToken;

use crate::app::{App, RunningJob};
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

/// Appends the user's message and admits the turn it calls for. If another turn owns the chat
/// lock, that turn can consume the message as steering at its next safe model boundary; the
/// admitted replacement job then exits when it reaches the lock.
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
        if let Err(error) = crate::files::push_blob(&app, Some(chat_id), attachment) {
            tracing::warn!(%error, name = %attachment.name, "uploading an attachment");
        }
    }
    let mut message = Message::new(chat_id, Author::You, Body::Text { text: text.to_string(), attachments });
    if let Some(id) = message_id.filter(|id| !id.is_empty()) {
        message.id = id;
    }
    app.upsert_message(message.clone(), true);
    #[cfg(feature = "runner")]
    crate::turns::steer_message(&app, &message);

    if chat.meta.is_group() {
        let members = turn_order(&chat.meta, &app, text);
        start_room(app.clone(), chat_id.to_string(), message.id.clone(), members);
    } else if let Some(bot) = chat.meta.bot_ids.first().and_then(|id| app.bot(id)) {
        start_turn(&app, user_turn_job(&app, chat_id, &bot.id, &message.id));
    }
    Ok(message)
}

/// Stops work in a chat on this Device and forwards job-specific cancellations to every other
/// Runner currently doing that work. This is the hard Stop path; ordinary messages steer and
/// do not call it.
pub fn cancel_chat(app: &Arc<App>, chat_id: &str) {
    for (job_id, runner_id) in app.cancel_chat(chat_id) {
        let Some(runner) = app.device(&runner_id).filter(|runner| !runner.box_pubkey.is_empty()) else { continue };
        match crate::crypto::seal_json(&runner.box_pubkey, &JobCancel { job_id: job_id.clone() }) {
            Ok(ciphertext) => {
                app.push_blob("job_cancel", Some(runner.id), ciphertext);
            }
            Err(error) => tracing::warn!(%error, %job_id, "sealing job cancellation"),
        }
    }
}

fn user_turn_job(app: &Arc<App>, chat_id: &str, bot_id: &str, trigger_message_id: &str) -> Job {
    Job {
        id: format!("job-{}", uuid::Uuid::new_v4()),
        chat_id: chat_id.to_string(),
        bot_id: bot_id.to_string(),
        kind: "turn".into(),
        trigger_message_id: trigger_message_id.to_string(),
        requested_by: app.this_device_id().unwrap_or_default(),
        routine_id: None,
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

fn begin_job(
    app: &App,
    job_id: &str,
    chat_id: &str,
    bot_id: &str,
    routine_id: Option<String>,
    runner_id: Option<String>,
    cancel: CancellationToken,
) {
    app.running_jobs.lock().unwrap().insert(
        job_id.to_string(),
        RunningJob {
            chat_id: chat_id.to_string(),
            bot_id: bot_id.to_string(),
            routine_id: routine_id.clone(),
            runner_id,
            cancel,
        },
    );
    app.emit(Event::JobStarted {
        chat_id: chat_id.to_string(),
        bot_id: bot_id.to_string(),
        job_id: job_id.to_string(),
        routine_id,
    });
}

fn finish_job(app: &App, job_id: &str, chat_id: &str, bot_id: &str, routine_id: Option<String>) {
    app.running_jobs.lock().unwrap().remove(job_id);
    app.emit(Event::JobFinished {
        chat_id: chat_id.to_string(),
        bot_id: bot_id.to_string(),
        job_id: job_id.to_string(),
        routine_id,
    });
}

/// Registers a room before it waits for the chat lock. A later message is admitted behind it
/// and takes over at the next member boundary.
fn start_room(app: Arc<App>, chat_id: String, trigger: String, members: Vec<Bot>) {
    if members.is_empty() {
        return;
    }
    let room_id = format!("room-{}", uuid::Uuid::new_v4());
    let cancel = CancellationToken::new();
    begin_job(&app, &room_id, &chat_id, "", None, None, cancel.clone());
    tokio::spawn(run_room(app, chat_id, trigger, members, room_id, cancel));
}

/// Whether another user instruction follows this room's trigger in the durable transcript.
fn has_newer_user_message(app: &App, chat_id: &str, trigger: &str) -> bool {
    let Some(chat) = app.chat(chat_id) else { return false };
    let Some(index) = chat.messages.iter().position(|message| message.id == trigger) else { return false };
    chat.messages[index + 1..].iter().any(|message| message.author == Author::You && message.is_complete())
}

/// One group exchange: each member gets a turn in order and either speaks or passes; the room
/// goes another round while anyone spoke, and the last round is marked as winding down. The
/// room holds the chat lock, so turns in one group never overlap. Stop cancels it immediately;
/// a new user message lets the current member settle, then hands the lock to its replacement.
async fn run_room(
    app: Arc<App>,
    chat_id: String,
    trigger: String,
    members: Vec<Bot>,
    room_id: String,
    cancel: CancellationToken,
) {
    let lock = app.chat_lock(&chat_id);
    let _guard = lock.lock().await;
    if has_newer_user_message(&app, &chat_id, &trigger) {
        finish_job(&app, &room_id, &chat_id, "", None);
        return;
    }

    // What each member had seen from others when it last took a turn: a member with nothing
    // new is not offered another turn.
    let mut seen: std::collections::HashMap<String, usize> = std::collections::HashMap::new();
    'rounds: for round in 1..=MAX_ROOM_ROUNDS {
        let mut any_sent = false;
        for bot in &members {
            if cancel.is_cancelled() || has_newer_user_message(&app, &chat_id, &trigger) {
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
                routine_id: None,
                requested_by: app.this_device_id().unwrap_or_default(),
                from_bot_id: None,
                hops: 0,
                round,
                is_winding_down: round == MAX_ROOM_ROUNDS,
                created_at: now_secs(),
            };
            let outcome = run_member_turn(&app, job, &cancel).await;
            if has_newer_user_message(&app, &chat_id, &trigger) {
                break 'rounds;
            }
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

    finish_job(&app, &room_id, &chat_id, "", None);
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
    remote_turn(app, job, bot.runner_id, room_cancel.child_token()).await
}

/// Seals a job to the bot's Runner and waits for its `job_result`. The job counts as running
/// here meanwhile, so the app shows the bot at work and can steer or stop it.
async fn remote_turn(
    app: &Arc<App>,
    job: Job,
    runner_id: String,
    cancel: CancellationToken,
) -> TurnOutcome {
    begin_job(
        app,
        &job.id,
        &job.chat_id,
        &job.bot_id,
        job.routine_id.clone(),
        Some(runner_id),
        cancel.clone(),
    );
    remote_turn_started(app, job, cancel).await
}

/// A remote turn whose running record already exists. `start_turn` registers synchronously so
/// a hard Stop immediately afterwards can still cancel it before dispatch.
async fn remote_turn_started(
    app: &Arc<App>,
    job: Job,
    cancel: CancellationToken,
) -> TurnOutcome {
    let (tx, rx) = tokio::sync::oneshot::channel();
    app.pending_results.lock().unwrap().insert(job.id.clone(), tx);
    let job_id = job.id.clone();
    let chat_id = job.chat_id.clone();
    let bot_id = job.bot_id.clone();
    let routine_id = job.routine_id.clone();
    let outcome = if cancel.is_cancelled() {
        TurnOutcome::Skipped
    } else {
        match dispatch_job(app, job) {
            Dispatch::Sent => {
                tokio::select! {
                    result = rx => result.map(|text| TurnOutcome::parse(&text)).unwrap_or(TurnOutcome::Skipped),
                    _ = tokio::time::sleep(REMOTE_TURN_TIMEOUT) => TurnOutcome::Skipped,
                    _ = cancel.cancelled() => TurnOutcome::Skipped,
                }
            }
            Dispatch::Ran | Dispatch::Deferred => TurnOutcome::Skipped,
        }
    };
    app.pending_results.lock().unwrap().remove(&job_id);
    finish_job(app, &job_id, &chat_id, &bot_id, routine_id);
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
        let cancel = CancellationToken::new();
        begin_job(
            app,
            &job.id,
            &job.chat_id,
            &job.bot_id,
            job.routine_id.clone(),
            Some(bot.runner_id),
            cancel.clone(),
        );
        let app = app.clone();
        tokio::spawn(async move {
            remote_turn_started(&app, job, cancel).await;
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
/// outcome goes back to the requesting Device when the job came from another one. It registers
/// before taking the lock, so hard Stop also cancels jobs waiting behind another turn.
pub fn spawn_local_job(app: Arc<App>, job: Job, remote_blob_id: Option<String>) {
    let cancel = CancellationToken::new();
    begin_job(
        &app,
        &job.id,
        &job.chat_id,
        &job.bot_id,
        job.routine_id.clone(),
        None,
        cancel.clone(),
    );
    tokio::spawn(async move {
        let lock = app.chat_lock(&job.chat_id);
        let _guard = lock.lock().await;
        let outcome = run_job_started(&app, job.clone(), cancel).await;
        if let Some(id) = &job.routine_id {
            crate::routines::finished(&app, id, outcome);
        }
        if let Some(id) = remote_blob_id {
            crate::sync::delete_remote_blob(&app, &id).await;
        }
        if app.this_device_id().as_deref() != Some(job.requested_by.as_str()) {
            report_outcome(&app, &job, outcome);
        }
    });
}

/// Runs a member job inside an active room. The room already holds the chat lock.
async fn run_job_here(app: &Arc<App>, job: Job, cancel: CancellationToken) -> TurnOutcome {
    begin_job(
        app,
        &job.id,
        &job.chat_id,
        &job.bot_id,
        job.routine_id.clone(),
        None,
        cancel.clone(),
    );
    run_job_started(app, job, cancel).await
}

/// Runs a job with its working record already installed.
async fn run_job_started(app: &Arc<App>, job: Job, cancel: CancellationToken) -> TurnOutcome {
    #[cfg(feature = "runner")]
    let outcome = if cancel.is_cancelled()
        || (job.kind == "turn"
            && app.take_steering_message(&job.chat_id, &job.trigger_message_id))
    {
        TurnOutcome::Skipped
    } else {
        crate::turns::run_job(app, &job, cancel).await
    };
    #[cfg(not(feature = "runner"))]
    let outcome = {
        let _ = cancel;
        app.notice(&job.chat_id, "This Device does not run bots; assign the bot to a Runner.");
        TurnOutcome::Skipped
    };
    finish_job(app, &job.id, &job.chat_id, &job.bot_id, job.routine_id.clone());
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::Config;

    struct ScratchApp(Arc<App>, std::path::PathBuf);

    impl Drop for ScratchApp {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.1);
        }
    }

    fn scratch_app() -> ScratchApp {
        let home = std::env::temp_dir().join(format!("lorca-runtime-{}", uuid::Uuid::new_v4()));
        let app = App::load(Config { home: home.clone(), port: 0 }).unwrap();
        ScratchApp(app, home)
    }

    fn empty_chat(id: &str) -> Chat {
        Chat {
            meta: ChatMeta {
                id: id.into(),
                kind: "dm".into(),
                title: None,
                bot_ids: Vec::new(),
                owner_bot_id: None,
                is_pinned: false,
                created_at: 1.0,
            },
            messages: Vec::new(),
            unread_count: 0,
            usage: None,
            compactions: Vec::new(),
        }
    }

    #[test]
    fn a_new_user_message_does_not_abort_the_job_in_flight() {
        let scratch = scratch_app();
        let app = &scratch.0;
        app.state.lock().unwrap().chats.push(empty_chat("chat"));
        let cancel = CancellationToken::new();
        app.running_jobs.lock().unwrap().insert(
            "old".into(),
            RunningJob {
                chat_id: "chat".into(),
                bot_id: "bot".into(),
                routine_id: None,
                runner_id: None,
                cancel: cancel.clone(),
            },
        );

        let message =
            send_user_message(app.clone(), "chat", "change course", None, Vec::new()).unwrap();

        assert!(!cancel.is_cancelled());
        assert_eq!(message.author, Author::You);
        assert_eq!(
            app.chat("chat")
                .unwrap()
                .messages
                .last()
                .map(|message| message.id.as_str()),
            Some(message.id.as_str())
        );
    }

    #[test]
    fn cancellation_is_sealed_to_the_remote_runner() {
        let scratch = scratch_app();
        let app = &scratch.0;
        let runner_keys = crate::keys::Machine::generate();
        app.state.lock().unwrap().devices.push(Device {
            id: "runner".into(),
            name: "Runner".into(),
            model: String::new(),
            os: "macos".into(),
            os_version: String::new(),
            box_pubkey: runner_keys.box_pubkey(),
            plugins: Vec::new(),
            updated_at: 1,
        });
        let cancel = CancellationToken::new();
        app.running_jobs.lock().unwrap().insert(
            "remote-job".into(),
            RunningJob {
                chat_id: "chat".into(),
                bot_id: "bot".into(),
                routine_id: None,
                runner_id: Some("runner".into()),
                cancel: cancel.clone(),
            },
        );

        cancel_chat(app, "chat");

        assert!(cancel.is_cancelled());
        let queued = app.state.lock().unwrap().outbox.last().cloned().unwrap();
        assert_eq!(queued.kind, "job_cancel");
        assert_eq!(queued.recipient.as_deref(), Some("runner"));
        let ciphertext = crate::keys::unb64(&queued.ciphertext).unwrap();
        let payload: JobCancel =
            crate::crypto::unseal_json(&runner_keys.box_secret, &ciphertext).unwrap();
        assert_eq!(payload.job_id, "remote-job");
    }

    #[tokio::test]
    async fn a_replacement_job_exits_after_its_message_was_steered() {
        let scratch = scratch_app();
        let app = &scratch.0;
        let mut chat = empty_chat("chat");
        let mut message = Message::new("chat", Author::You, Body::text("steer"));
        message.id = "message".into();
        chat.messages.push(message);
        app.state.lock().unwrap().chats.push(chat);
        assert!(app.claim_steering_message("chat", "message"));
        let job = Job {
            id: "replacement".into(),
            chat_id: "chat".into(),
            bot_id: "bot".into(),
            kind: "turn".into(),
            trigger_message_id: "message".into(),
            routine_id: None,
            requested_by: String::new(),
            from_bot_id: None,
            hops: 0,
            round: 0,
            is_winding_down: false,
            created_at: 1.0,
        };

        let outcome = run_job_started(app, job, CancellationToken::new()).await;

        assert_eq!(outcome, TurnOutcome::Skipped);
        assert!(app.take_steering_message("chat", "message"));
    }
}
