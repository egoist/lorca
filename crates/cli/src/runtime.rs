//! What a Device does with turns: sends the user's message, runs a group's room exchange,
//! starts jobs here or on the bot's Runner through the relay, and takes their results. The
//! turn itself, with its model, tools, and memory, lives in `turns` under the `runner` feature.

use std::sync::Arc;

use tokio_util::sync::CancellationToken;

use crate::app::{App, RunningJob, SentJob};
use crate::config::now_secs;
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
pub fn send_user_message(
    app: Arc<App>,
    chat_id: &str,
    text: &str,
    message_id: Option<String>,
    attachments: Vec<Attachment>,
    mentions: Vec<String>,
) -> anyhow::Result<Message> {
    let text = text.trim();
    if text.is_empty() && attachments.is_empty() {
        anyhow::bail!("Empty message");
    }
    let mentions = resolve_mentions(&app, text, mentions);
    let chat = app.chat(chat_id).ok_or_else(|| anyhow::anyhow!("Unknown chat"))?;
    // The bytes go out ahead of the message that names them.
    for attachment in &attachments {
        if let Err(error) = crate::files::push_blob(&app, Some(chat_id), attachment) {
            tracing::warn!(%error, name = %attachment.name, "uploading an attachment");
        }
    }
    let mut message = Message::new(chat_id, Author::You, Body::Text { text: text.to_string(), attachments, mentions: mentions.clone() });
    if let Some(id) = message_id.filter(|id| !id.is_empty()) {
        message.id = id;
    }
    app.upsert_message(message.clone(), true);
    #[cfg(feature = "runner")]
    crate::turns::steer_message(&app, &message);

    if chat.meta.is_group() {
        let members = turn_order(&chat.meta, &app, &mentions);
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

/// Members in chat order, with the ones the message mentions first.
pub fn turn_order(chat: &ChatMeta, app: &Arc<App>, mentions: &[String]) -> Vec<Bot> {
    let members: Vec<Bot> = chat.bot_ids.iter().filter_map(|id| app.bot(id)).collect();
    let (mentioned, rest): (Vec<Bot>, Vec<Bot>) = members.into_iter().partition(|bot| mentions.contains(&bot.id));
    mentioned.into_iter().chain(rest).collect()
}

/// The bots a message mentions, by id: the ones the user picked from the `@` menu, then each
/// other `@Name` in the text that only one bot answers to. A name two bots share stays as typed
/// unless the user picked which.
fn resolve_mentions(app: &App, text: &str, picked: Vec<String>) -> Vec<String> {
    let bots = app.state.lock().unwrap().bots.clone();
    let mut mentions: Vec<String> = Vec::new();
    for id in picked {
        if bots.iter().any(|bot| bot.id == id) && !mentions.contains(&id) {
            mentions.push(id);
        }
    }
    for bot in &bots {
        let shared = bots.iter().filter(|other| other.name.eq_ignore_ascii_case(&bot.name)).count() > 1;
        if !shared && !mentions.contains(&bot.id) && text.match_indices('@').any(|(at, _)| mention_at(text, at, &bot.name)) {
            mentions.push(bot.id.clone());
        }
    }
    mentions
}

/// Whether `@name` starts at byte `at` of `text` as a word of its own, ignoring ASCII case: not
/// inside an email address, and not the start of a longer name.
pub(crate) fn mention_at(text: &str, at: usize, name: &str) -> bool {
    let before = text[..at].chars().next_back();
    let Some(rest) = text[at..].strip_prefix('@') else { return false };
    !name.is_empty()
        && !before.is_some_and(char::is_alphanumeric)
        && rest.get(..name.len()).is_some_and(|word| word.eq_ignore_ascii_case(name))
        && !rest[name.len()..].chars().next().is_some_and(|c| c.is_alphanumeric() || c == '_')
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
    // A job on another Runner outlives this process; `resume_sent_jobs` picks up its wait.
    if let Some(runner_id) = &runner_id {
        let sent = SentJob {
            id: job_id.to_string(),
            chat_id: chat_id.to_string(),
            bot_id: bot_id.to_string(),
            routine_id: routine_id.clone(),
            runner_id: runner_id.clone(),
            sent_at: now_secs(),
        };
        if let Err(error) = app.store.insert_sent_job(&sent) {
            tracing::error!(%error, %job_id, "keeping a job sent to another Runner");
        }
    }
    app.running_jobs.lock().unwrap().insert(
        job_id.to_string(),
        RunningJob {
            chat_id: chat_id.to_string(),
            bot_id: bot_id.to_string(),
            routine_id,
            runner_id,
            cancel,
            activity: None,
        },
    );
    app.local_turns_changed();
}

fn finish_job(app: &App, job_id: &str) {
    let job = app.running_jobs.lock().unwrap().remove(job_id);
    if job.is_some_and(|job| job.runner_id.is_some()) {
        if let Err(error) = app.store.remove_sent_job(job_id) {
            tracing::error!(%error, %job_id, "dropping a job sent to another Runner");
        }
    }
    app.local_turns_changed();
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
    if app.chat(chat_id).is_none() {
        return false;
    }
    app.store
        .messages_after(chat_id, trigger)
        .unwrap_or_default()
        .iter()
        .any(|message| message.author == Author::You && message.is_complete())
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
        finish_job(&app, &room_id);
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
            if round > 1 && seen.get(&bot.id) == Some(&heard_count(&app, &chat.meta.id, &bot.id)) {
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
                seen.insert(bot.id.clone(), heard_count(&app, &chat.meta.id, &bot.id));
            }
            if outcome == TurnOutcome::Sent {
                any_sent = true;
            }
        }
        if !any_sent {
            break;
        }
    }

    finish_job(&app, &room_id);
}

/// Messages in the chat that `bot_id` did not write itself.
fn heard_count(app: &App, chat_id: &str, bot_id: &str) -> usize {
    app.store.heard_count(chat_id, bot_id).unwrap_or(0)
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
    let result = expect_result(app, &job.id);
    let job_id = job.id.clone();
    let outcome = if cancel.is_cancelled() {
        TurnOutcome::Skipped
    } else {
        match dispatch_job(app, job) {
            Dispatch::Sent => wait_for_result(result, REMOTE_TURN_TIMEOUT, &cancel).await,
            Dispatch::Ran | Dispatch::Deferred => TurnOutcome::Skipped,
        }
    };
    app.pending_results.lock().unwrap().remove(&job_id);
    finish_job(app, &job_id);
    outcome
}

/// Registers the wait for a job's `job_result` before anything can deliver it.
fn expect_result(app: &App, job_id: &str) -> tokio::sync::oneshot::Receiver<String> {
    let (tx, rx) = tokio::sync::oneshot::channel();
    app.pending_results.lock().unwrap().insert(job_id.to_string(), tx);
    rx
}

/// How a job on another Runner ended: its `job_result`, or skipped when the wait runs out or
/// Stop cancels it.
async fn wait_for_result(
    result: tokio::sync::oneshot::Receiver<String>,
    timeout: std::time::Duration,
    cancel: &CancellationToken,
) -> TurnOutcome {
    tokio::select! {
        result = result => result.map(|text| TurnOutcome::parse(&text)).unwrap_or(TurnOutcome::Skipped),
        _ = tokio::time::sleep(timeout) => TurnOutcome::Skipped,
        _ = cancel.cancelled() => TurnOutcome::Skipped,
    }
}

/// Picks up the jobs this Device sent to other Runners before it last stopped: each counts as
/// running again until its `job_result` arrives, the rest of its wait runs out, or Stop cancels
/// it, so the apps show the bot at work across a restart. Called before the sync loop starts,
/// since the first pull may carry a result that landed meanwhile. A group exchange this Device
/// was running does not resume; only the member turn in flight shows.
pub fn resume_sent_jobs(app: &Arc<App>) {
    let jobs = match app.store.sent_jobs() {
        Ok(jobs) => jobs,
        Err(error) => {
            tracing::error!(%error, "reading the jobs sent to other Runners");
            return;
        }
    };
    let now = now_secs();
    for job in jobs {
        let left = job.sent_at + REMOTE_TURN_TIMEOUT.as_secs_f64() - now;
        if left <= 0.0 || app.chat(&job.chat_id).is_none() {
            if let Err(error) = app.store.remove_sent_job(&job.id) {
                tracing::error!(%error, job_id = %job.id, "dropping a job sent to another Runner");
            }
            continue;
        }
        let cancel = CancellationToken::new();
        app.running_jobs.lock().unwrap().insert(
            job.id.clone(),
            RunningJob {
                chat_id: job.chat_id.clone(),
                bot_id: job.bot_id.clone(),
                routine_id: job.routine_id.clone(),
                runner_id: Some(job.runner_id.clone()),
                cancel: cancel.clone(),
                activity: None,
            },
        );
        let result = expect_result(app, &job.id);
        let app = app.clone();
        tokio::spawn(async move {
            wait_for_result(result, std::time::Duration::from_secs_f64(left), &cancel).await;
            app.pending_results.lock().unwrap().remove(&job.id);
            finish_job(&app, &job.id);
        });
    }
    app.turns_changed();
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
    finish_job(app, &job.id);
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

/// What a turn running here is doing that no message says: it goes into this Device's machine
/// blob, so every app shows it, and the local app hears it at once.
pub fn report_activity(app: &App, job: &Job, activity: JobActivity) {
    app.set_job_activity(&job.id, activity);
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
    use crate::events::Event;

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
            unread_count: 0,
            usage: None,
            compactions: Vec::new(),
        }
    }

    fn bot(id: &str, name: &str) -> Bot {
        Bot {
            id: id.into(),
            name: name.into(),
            description: String::new(),
            symbol_name: String::new(),
            accent: String::new(),
            avatar: None,
            runner_id: "dev".into(),
            provider: "deepseek".into(),
            model: None,
            thinking: None,
            legacy_instructions: String::new(),
            workdir: None,
            created_at: 0.0,
        }
    }

    #[test]
    fn a_message_mentions_bots_by_id() {
        let scratch = scratch_app();
        let app = &scratch.0;
        app.state.lock().unwrap().bots = vec![bot("b1", "Chef"), bot("b2", "Chef"), bot("b3", "Scout"), bot("b4", "Sc")];

        // A typed name that one bot answers to resolves; a shared one only when the user picked
        // which. Neither an email address nor the start of a longer name is a mention.
        assert_eq!(resolve_mentions(app, "@chef and @Scout, mail x@sc.com", Vec::new()), ["b3"]);
        assert_eq!(resolve_mentions(app, "@Chef and @Scout", vec!["b2".into(), "gone".into(), "b2".into()]), ["b2", "b3"]);

        let group = ChatMeta { kind: "group".into(), bot_ids: vec!["b1".into(), "b2".into(), "b3".into()], ..empty_chat("g").meta };
        let order: Vec<String> = turn_order(&group, app, &["b2".into()]).into_iter().map(|bot| bot.id).collect();
        assert_eq!(order, ["b2", "b1", "b3"]);
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
                activity: None,
            },
        );

        let message =
            send_user_message(app.clone(), "chat", "change course", None, Vec::new(), Vec::new()).unwrap();

        assert!(!cancel.is_cancelled());
        assert_eq!(message.author, Author::You);
        assert_eq!(
            app.messages("chat")
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
                activity: None,
            },
        );

        cancel_chat(app, "chat");

        assert!(cancel.is_cancelled());
        let queued = app.store.last_outbox().unwrap().unwrap();
        assert_eq!(queued.kind, "job_cancel");
        assert_eq!(queued.recipient.as_deref(), Some("runner"));
        let ciphertext = queued.ciphertext;
        let payload: JobCancel =
            crate::crypto::unseal_json(&runner_keys.box_secret, &ciphertext).unwrap();
        assert_eq!(payload.job_id, "remote-job");
    }

    fn turn_on_mac(job_id: &str, chat_id: &str) -> LiveTurn {
        LiveTurn { job_id: job_id.into(), chat_id: chat_id.into(), bot_id: "bot".into(), routine_id: None, activity: Some(JobActivity::Thinking) }
    }

    #[test]
    fn another_devices_turns_show_while_the_relay_lists_it_online() {
        let scratch = scratch_app();
        let app = &scratch.0;
        let mut events = app.events.subscribe();
        let turn = turn_on_mac("mac-job", "chat");

        // Listed while the Mac is not online: nothing shows.
        app.set_device_turns("mac", vec![turn.clone()]);
        assert!(app.running_turns().is_empty());
        assert!(events.try_recv().is_err());

        app.state.lock().unwrap().turns_online.insert("mac".into());
        app.turns_changed();
        assert!(matches!(events.try_recv(), Ok(Event::JobStarted { job_id, chat_id, bot_id, .. }) if job_id == "mac-job" && chat_id == "chat" && bot_id == "bot"));
        assert!(matches!(events.try_recv(), Ok(Event::JobThinking { chat_id, bot_id }) if chat_id == "chat" && bot_id == "bot"));
        assert_eq!(app.running_turns().len(), 1);

        // The same list again says nothing new; another activity does.
        app.set_device_turns("mac", vec![turn.clone()]);
        assert!(events.try_recv().is_err());
        let retry = JobActivity::Retry { attempt: 1, max_attempts: 3, delay_ms: 2000, error: "overloaded".into() };
        app.set_device_turns("mac", vec![LiveTurn { activity: Some(retry), ..turn.clone() }]);
        assert!(matches!(events.try_recv(), Ok(Event::JobRetry { attempt: 1, max_attempts: 3, delay_ms: 2000, .. })));

        // It ends when the Mac lists it no more.
        app.set_device_turns("mac", Vec::new());
        assert!(matches!(events.try_recv(), Ok(Event::JobFinished { job_id, .. }) if job_id == "mac-job"));

        // The list outlives a restart, and shows again once the relay says the Mac is online.
        app.set_device_turns("mac", vec![turn.clone()]);
        let restarted = App::load(Config { home: scratch.1.clone(), port: 0 }).unwrap();
        assert!(restarted.running_turns().is_empty());
        restarted.state.lock().unwrap().turns_online.insert("mac".into());
        assert_eq!(restarted.running_turns().len(), 1);

        // Offline, it ends.
        while events.try_recv().is_ok() {}
        app.state.lock().unwrap().turns_online.clear();
        app.turns_changed();
        assert!(matches!(events.try_recv(), Ok(Event::JobFinished { job_id, .. }) if job_id == "mac-job"));
        assert!(app.running_turns().is_empty());
    }

    #[test]
    fn a_job_sent_to_a_runner_that_lists_it_is_one_turn_until_both_let_go() {
        let scratch = scratch_app();
        let app = &scratch.0;
        app.state.lock().unwrap().chats.push(empty_chat("chat"));
        let mut events = app.events.subscribe();
        begin_job(app, "job", "chat", "bot", None, Some("mac".into()), CancellationToken::new());
        assert!(matches!(events.try_recv(), Ok(Event::JobStarted { job_id, .. }) if job_id == "job"));
        // A job sent elsewhere is that Runner's to list.
        assert!(app.turns_here().is_empty());

        app.state.lock().unwrap().turns_online.insert("mac".into());
        app.set_device_turns("mac", vec![turn_on_mac("job", "chat")]);
        assert!(matches!(events.try_recv(), Ok(Event::JobThinking { .. })));
        assert_eq!(app.running_turns().len(), 1);

        // The wait here runs out while the Runner still lists the turn: it is still working.
        finish_job(app, "job");
        assert!(events.try_recv().is_err());
        assert_eq!(app.running_turns().len(), 1);
        app.set_device_turns("mac", Vec::new());
        assert!(matches!(events.try_recv(), Ok(Event::JobFinished { job_id, .. }) if job_id == "job"));
        assert!(app.running_turns().is_empty());
    }

    #[test]
    fn stop_is_sealed_to_the_device_that_lists_the_turn() {
        let scratch = scratch_app();
        let app = &scratch.0;
        let mac_keys = crate::keys::Machine::generate();
        app.state.lock().unwrap().devices.push(Device {
            id: "mac".into(),
            name: "Mac".into(),
            model: String::new(),
            os: "macos".into(),
            os_version: String::new(),
            box_pubkey: mac_keys.box_pubkey(),
            plugins: Vec::new(),
            updated_at: 1,
        });
        app.state.lock().unwrap().turns_online.insert("mac".into());
        app.set_device_turns("mac", vec![turn_on_mac("mac-job", "chat"), turn_on_mac("elsewhere", "other-chat")]);

        cancel_chat(app, "chat");

        let cancels: Vec<_> = app.store.outbox().unwrap().into_iter().filter(|item| item.kind == "job_cancel").collect();
        let [cancel] = cancels.try_into().unwrap();
        assert_eq!(cancel.recipient.as_deref(), Some("mac"));
        let payload: JobCancel = crate::crypto::unseal_json(&mac_keys.box_secret, &cancel.ciphertext).unwrap();
        assert_eq!(payload.job_id, "mac-job");
    }

    #[test]
    fn a_machine_blob_that_only_lists_other_turns_leaves_the_roster_alone() {
        let scratch = scratch_app();
        let app = &scratch.0;
        crate::identity::create(app, Some("Phone".into())).unwrap();
        let machine_file = app.machine_file().unwrap();
        let dek = machine_file.dek().unwrap();
        let mac = Device {
            id: "mac".into(),
            name: "Mac".into(),
            model: "MacBook Air (M5)".into(),
            os: "macos".into(),
            os_version: "26.0".into(),
            box_pubkey: String::new(),
            plugins: Vec::new(),
            updated_at: 1,
        };
        app.state.lock().unwrap().device_seen.insert("mac".into(), 1);
        app.state.lock().unwrap().turns_online.insert("mac".into());
        let blob = |seq: i64, device: Device, turns: Vec<LiveTurn>| crate::relay::BlobIn {
            id: format!("blob-{seq}"),
            kind: "machine".into(),
            recipient_machine_pubkey: None,
            seq,
            ciphertext: crate::keys::b64(&crate::crypto::encrypt_json(&dek, "machine", &MachineBlob { device, turns }).unwrap()),
            created_at: 0,
        };
        let mut events = app.events.subscribe();

        crate::sync::apply_blob(app, &machine_file, &blob(1, mac.clone(), Vec::new()));
        assert!(std::iter::from_fn(|| events.try_recv().ok()).any(|event| matches!(event, Event::RosterChanged { .. })));

        crate::sync::apply_blob(app, &machine_file, &blob(2, Device { updated_at: 2, ..mac.clone() }, vec![turn_on_mac("mac-job", "chat")]));
        let heard: Vec<Event> = std::iter::from_fn(|| events.try_recv().ok()).collect();
        assert!(!heard.iter().any(|event| matches!(event, Event::RosterChanged { .. })));
        assert!(heard.iter().any(|event| matches!(event, Event::JobStarted { job_id, .. } if job_id == "mac-job")));
    }

    #[tokio::test]
    async fn a_replacement_job_exits_after_its_message_was_steered() {
        let scratch = scratch_app();
        let app = &scratch.0;
        let chat = empty_chat("chat");
        let mut message = Message::new("chat", Author::You, Body::text("steer"));
        message.id = "message".into();
        app.state.lock().unwrap().chats.push(chat);
        app.upsert_message(message, false);
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

    #[tokio::test]
    async fn a_job_sent_to_another_runner_runs_again_after_a_restart() {
        let scratch = scratch_app();
        let app = &scratch.0;
        app.state.lock().unwrap().chats.push(empty_chat("chat"));
        app.save_state_now();
        begin_job(app, "remote-job", "chat", "bot", None, Some("runner".into()), CancellationToken::new());

        // The process ends with the job in flight, and the next one opens the same folder.
        let restarted = App::load(Config { home: scratch.1.clone(), port: 0 }).unwrap();
        assert!(restarted.running_turns().is_empty());
        resume_sent_jobs(&restarted);
        let turns = restarted.running_turns();
        assert_eq!(turns.len(), 1);
        assert_eq!((turns[0]["job_id"].as_str(), turns[0]["chat_id"].as_str(), turns[0]["bot_id"].as_str()), (Some("remote-job"), Some("chat"), Some("bot")));

        // The result that lands later ends it there, and nothing is left to resume.
        let mut events = restarted.events.subscribe();
        deliver_job_result(&restarted, JobResult { job_id: "remote-job".into(), chat_id: "chat".into(), bot_id: "bot".into(), outcome: "sent".into() });
        let finished = tokio::time::timeout(std::time::Duration::from_secs(5), events.recv()).await.unwrap().unwrap();
        assert!(matches!(finished, Event::JobFinished { job_id, .. } if job_id == "remote-job"));
        assert!(restarted.running_turns().is_empty());
        assert!(restarted.store.sent_jobs().unwrap().is_empty());
    }

    #[tokio::test]
    async fn a_sent_job_past_its_wait_or_chat_does_not_come_back() {
        let scratch = scratch_app();
        let app = &scratch.0;
        app.state.lock().unwrap().chats.push(empty_chat("chat"));
        let expired = SentJob {
            id: "expired".into(),
            chat_id: "chat".into(),
            bot_id: "bot".into(),
            routine_id: None,
            runner_id: "runner".into(),
            sent_at: now_secs() - REMOTE_TURN_TIMEOUT.as_secs_f64() - 1.0,
        };
        app.store.insert_sent_job(&expired).unwrap();
        app.store.insert_sent_job(&SentJob { id: "orphan".into(), chat_id: "deleted".into(), sent_at: now_secs(), ..expired.clone() }).unwrap();

        resume_sent_jobs(app);

        assert!(app.running_turns().is_empty());
        assert!(app.store.sent_jobs().unwrap().is_empty());
    }
}
