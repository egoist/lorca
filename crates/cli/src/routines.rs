//! Routines: recurring tasks a bot runs on a schedule in its direct chat, after Grok Bot. The
//! roster carries them, so every Device lists them and can pause one; the bot's Runner runs
//! them, looking for due ones every half minute, and any Device can ask for a run now. A
//! routine with a check runs the check first, a script and no model, and the bot runs only on
//! what it found.

use std::sync::Arc;

use serde_json::Value;

use crate::app::App;
use crate::config::{now_secs, now_unix};
use crate::model::*;
use crate::runtime::{self, TurnOutcome};
use crate::schedule;

#[cfg(feature = "runner")]
use lorca_agent::codemode::{CodemodeOptions, CodemodeTool, HostFunction};
#[cfg(feature = "runner")]
use lorca_agent::{DirectRunner, Tool, ToolOutcome, ToolResult, ToolRunner};
#[cfg(feature = "runner")]
use tokio_util::sync::CancellationToken;

/// How often the Runner looks for due routines.
#[cfg(feature = "runner")]
const TICK: std::time::Duration = std::time::Duration::from_secs(30);

/// A week without a message from the user, and due routines are paused instead of run, so
/// they do not spend money on results nobody reads.
pub const AWAY_AFTER_SECS: i64 = 7 * 86_400;

pub const MAX_NAME_CHARS: usize = 60;
pub const MAX_PER_BOT: usize = 20;
/// The longest check a routine keeps: a check looks, and the run does the work.
pub const MAX_CHECK_CHARS: usize = 8_000;

// MARK: - Editing

/// Adds a routine for `bot_id`, checking the schedule, the name, and the check first.
pub fn create(app: &Arc<App>, bot_id: &str, name: &str, schedule_text: &str, prompt: &str, check: Option<&str>, enabled: bool) -> Result<Routine, String> {
    create_with_policy(app, bot_id, name, schedule_text, prompt, check, enabled, None, None)
}

/// `create` with a timezone (this Runner's when `None`) and a missed-run policy (one run when
/// `None`).
#[allow(clippy::too_many_arguments)]
pub fn create_with_policy(app: &Arc<App>, bot_id: &str, name: &str, schedule_text: &str, prompt: &str, check: Option<&str>, enabled: bool, timezone: Option<&str>, missed_run_policy: Option<&str>) -> Result<Routine, String> {
    let name = clean_name(name)?;
    let prompt = prompt.trim();
    if prompt.is_empty() {
        return Err("Say what the routine should do on each run.".into());
    }
    let check = check.map(clean_check).transpose()?.flatten();
    let schedule = schedule::parse(schedule_text)?;
    let timezone = match timezone {
        Some(zone) => schedule::timezone(zone)?.to_string(),
        None => schedule::local_timezone(),
    };
    let missed_run_policy = missed_run_policy.map(crate::routine_health::MissedRunPolicy::parse).transpose()?.unwrap_or_default();
    if app.bot(bot_id).is_none() {
        return Err("Unknown bot".into());
    }
    let existing = app.routines_of(bot_id);
    if existing.len() >= MAX_PER_BOT {
        return Err(format!("A bot keeps at most {MAX_PER_BOT} routines. Delete one first."));
    }
    if existing.iter().any(|r| r.name.eq_ignore_ascii_case(&name)) {
        return Err(format!("A routine named {name:?} already exists. Pick another name or edit that one."));
    }
    let now = now_secs();
    let routine = Routine {
        id: String::new(),
        bot_id: bot_id.to_string(),
        name,
        prompt: prompt.to_string(),
        feedback_authorization_prompt: None,
        schedule: schedule.canonical(),
        timezone,
        missed_run_policy,
        last_scheduled_at: None,
        health: None,
        is_enabled: enabled,
        enabled_at: now,
        last_run_at: None,
        last_outcome: None,
        paused_reason: None,
        check,
        created_at: now,
    };
    app.insert_routine(routine).map_err(|e| e.to_string())
}

/// Changes a routine's name, schedule, prompt, or check. Only the fields given change; an empty
/// check removes it, and a new schedule counts from now.
pub fn edit(app: &Arc<App>, id: &str, name: Option<&str>, schedule_text: Option<&str>, prompt: Option<&str>, check: Option<&str>) -> Result<Routine, String> {
    edit_with_policy(app, id, name, schedule_text, prompt, check, None, None)
}

/// `edit` with a new timezone or missed-run policy as well. A new zone counts from now.
#[allow(clippy::too_many_arguments)]
pub fn edit_with_policy(app: &Arc<App>, id: &str, name: Option<&str>, schedule_text: Option<&str>, prompt: Option<&str>, check: Option<&str>, timezone: Option<&str>, missed_run_policy: Option<&str>) -> Result<Routine, String> {
    let current = app.routine(id).ok_or("Unknown routine")?;
    let name = name.map(clean_name).transpose()?;
    if let Some(name) = &name {
        if app.routines_of(&current.bot_id).iter().any(|r| r.id != id && r.name.eq_ignore_ascii_case(name)) {
            return Err(format!("A routine named {name:?} already exists."));
        }
    }
    let schedule = schedule_text.map(schedule::parse).transpose()?;
    let prompt = prompt.map(str::trim).filter(|p| !p.is_empty()).map(str::to_string);
    let check = check.map(clean_check).transpose()?;
    let timezone = timezone.map(schedule::timezone).transpose()?.map(|zone| zone.to_string());
    let missed_run_policy = missed_run_policy.map(crate::routine_health::MissedRunPolicy::parse).transpose()?;
    if name.is_none() && schedule.is_none() && prompt.is_none() && check.is_none() && timezone.is_none() && missed_run_policy.is_none() {
        return Err("Pass a new name, schedule, prompt, check, timezone, or missed-run policy.".into());
    }
    app.update_routine(id, |routine| {
        if let Some(name) = name {
            routine.name = name;
        }
        if let Some(schedule) = schedule {
            routine.schedule = schedule.canonical();
            routine.enabled_at = now_secs();
        }
        if let Some(timezone) = timezone {
            routine.timezone = timezone;
            routine.enabled_at = now_secs();
        }
        if let Some(policy) = missed_run_policy {
            routine.missed_run_policy = policy;
        }
        if let Some(prompt) = prompt {
            // A task written here is the task the user asked for, feedback revisions included.
            if prompt != routine.prompt {
                routine.feedback_authorization_prompt = None;
            }
            routine.prompt = prompt;
        }
        if let Some(check) = check {
            routine.check = check;
            routine.health = None;
            routine.enabled_at = now_secs();
        }
    })
    .map_err(|e| e.to_string())
}

/// A check as a routine keeps it, or `None` for an empty one, which removes it. A Runner reads
/// its options line now, so a bad one is refused before it is saved.
fn clean_check(code: &str) -> Result<Option<String>, String> {
    let code = code.trim();
    if code.is_empty() {
        return Ok(None);
    }
    if code.chars().count() > MAX_CHECK_CHARS {
        return Err(format!("Keep the check under {MAX_CHECK_CHARS} characters: it only looks, and the run does the work."));
    }
    #[cfg(feature = "runner")]
    lorca_agent::codemode::parse_source(code)?;
    Ok(Some(code.to_string()))
}

/// A routine's check is its Runner's to change, and a build that does not know checks writes
/// the roster without them. When `incoming` leaves out the check that a routine of a bot on
/// `this_device` has in `current`, the check stays. True when one did, so the roster goes up
/// again with it. The same merge keeps a feedback revision's original task authority while the
/// task is still the revised one; a task edited on another Device is the user's own.
pub fn keep_checks(current: &[Routine], incoming: &mut [Routine], bots: &[Bot], this_device: &str) -> bool {
    let mut kept = false;
    for routine in incoming.iter_mut() {
        if !bots.iter().any(|bot| bot.id == routine.bot_id && bot.runner_id == this_device) {
            continue;
        }
        let Some(held) = current.iter().find(|held| held.id == routine.id) else { continue };
        if routine.check.is_none() && held.check.is_some() {
            routine.check = held.check.clone();
            kept = true;
        }
        if routine.feedback_authorization_prompt.is_none() && held.feedback_authorization_prompt.is_some() && routine.prompt == held.prompt {
            routine.feedback_authorization_prompt = held.feedback_authorization_prompt.clone();
            kept = true;
        }
        // A schedule/check edit or explicit resume re-arms it. Otherwise preserve this
        // Runner's newest checkpoint when a Device publishes a roster it read earlier.
        if routine.enabled_at <= held.enabled_at {
            if held.last_scheduled_at > routine.last_scheduled_at {
                routine.last_scheduled_at = held.last_scheduled_at;
                kept = true;
            }
            if held.health.as_ref().map(|health| health.updated_at) > routine.health.as_ref().map(|health| health.updated_at) {
                routine.health = held.health.clone();
                kept = true;
            }
            if held.paused_reason.as_deref() == Some("authentication") && routine.is_enabled {
                routine.is_enabled = false;
                routine.paused_reason = held.paused_reason.clone();
                kept = true;
            }
        }
    }
    kept
}

/// Pauses or resumes a routine. A resumed schedule counts from now.
pub fn set_enabled(app: &Arc<App>, id: &str, enabled: bool) -> Result<Routine, String> {
    if enabled { crate::workflows::allow_enable(app, id)?; }
    app.update_routine(id, |routine| {
        if enabled && !routine.is_enabled {
            routine.enabled_at = now_secs();
            if let Some(health) = routine.health.as_mut() {
                health.resume();
            }
        }
        routine.is_enabled = enabled;
        routine.paused_reason = None;
    })
    .map_err(|e| e.to_string())
}

pub fn delete(app: &Arc<App>, id: &str) -> Result<(), String> {
    app.delete_routine(id).map_err(|e| e.to_string())
}

fn clean_name(name: &str) -> Result<String, String> {
    let name: String = name.split_whitespace().collect::<Vec<_>>().join(" ");
    if name.is_empty() {
        return Err("Give the routine a name.".into());
    }
    if name.chars().count() > MAX_NAME_CHARS {
        return Err(format!("Keep the name under {MAX_NAME_CHARS} characters."));
    }
    Ok(name)
}

/// The schedule in words and the next firing from now, in `timezone` (this Runner's when
/// `None`), for a caller that wants to check a schedule and its policy before saving them.
pub fn describe(schedule_text: &str, timezone: Option<&str>, missed_run_policy: Option<&str>) -> Result<Value, String> {
    let schedule = schedule::parse(schedule_text)?;
    let timezone = match timezone {
        Some(zone) => schedule::timezone(zone)?.to_string(),
        None => schedule::local_timezone(),
    };
    let missed_run_policy = missed_run_policy.map(crate::routine_health::MissedRunPolicy::parse).transpose()?.unwrap_or_default();
    Ok(serde_json::json!({
        "schedule": schedule.canonical(),
        "text": schedule.describe(),
        "timezone": timezone,
        "missed_run_policy": missed_run_policy,
        "next_run_at": schedule.next_after(now_unix(), &timezone).map(|t| t as f64),
    }))
}

// MARK: - Running

/// Starts a run of the routine now, here or on the bot's Runner through the relay. Refuses
/// while a run is going on.
pub fn run_now(app: &Arc<App>, id: &str) -> Result<(), String> {
    app.budgets.admit(app, "routine", id)?;
    let routine = app.routine(id).ok_or("Unknown routine")?;
    if routine.paused_reason.as_deref() == Some("authentication") {
        return Err(signed_out_text(&routine));
    }
    if app.is_routine_running(&routine.id) {
        return Err(format!("{} is running right now.", routine.name));
    }
    let job = job_for(app, &routine)?;
    runtime::start_turn(app, job);
    Ok(())
}

/// Why a routine paused by failed sign-ins does not run, and what brings it back.
pub fn signed_out_text(routine: &Routine) -> String {
    format!("{} is paused after three failed sign-ins in a row. Reconnect, then resume it.", routine.name)
}

/// The job that runs a routine: a `routine` turn in the bot's direct chat.
fn job_for(app: &Arc<App>, routine: &Routine) -> Result<Job, String> {
    let dm = app.dm_with(&routine.bot_id, None).map_err(|e| e.to_string())?;
    Ok(Job {
        id: format!("job-{}", uuid::Uuid::new_v4()),
        chat_id: dm.meta.id,
        bot_id: routine.bot_id.clone(),
        kind: "routine".into(),
        task_id: None,
        task_context: None,
        trigger_message_id: String::new(),
        handoff: None,
        routine_id: Some(routine.id.clone()),
        check: None,
        requested_by: app.this_device_id().unwrap_or_default(),
        from_bot_id: None,
        hops: 0,
        round: 0,
        is_winding_down: false,
        setup: None,
        created_at: now_secs(),
    })
}

/// A run began on this Runner: the schedule counts from now, so a slow run is not queued
/// again behind itself.
pub fn started(app: &Arc<App>, id: &str) {
    let _ = app.update_routine(id, |routine| routine.last_run_at = Some(now_secs()));
}

/// A run ended: what the panel shows as the last outcome.
pub fn finished(app: &Arc<App>, id: &str, outcome: TurnOutcome) {
    let outcome = match outcome {
        TurnOutcome::Sent => "sent",
        TurnOutcome::Pass => "pass",
        TurnOutcome::Skipped => "error",
    };
    let _ = app.update_routine(id, |routine| routine.last_outcome = Some(outcome.into()));
}

/// A run ended, after the provider's own retries: a failure to reach or sign in to the provider
/// counts in the runs' streak (never in the checks'), and three failed sign-ins in a row pause
/// the routine until it is resumed. Anything else leaves the streak as it was.
pub fn model_result(app: &App, id: &str, error: Option<&str>) {
    let Some(routine) = app.routine(id) else { return };
    let before = routine.health.as_ref().map(|health| health.model.clone()).unwrap_or_default();
    let mut model = before.clone();
    match error {
        Some(error) => model.record_failure(now_secs(), crate::routine_health::classify(error)),
        None => model = Default::default(),
    }
    if model == before {
        return;
    }
    let _ = app.update_routine(id, |routine| {
        let health = routine.health.get_or_insert_with(Default::default);
        health.model = model;
        health.updated_at = now_secs();
        if health.model.authentication_failures >= 3 {
            routine.is_enabled = false;
            routine.paused_reason = Some("authentication".into());
        }
    });
}

/// The scheduler: every half minute, run the routines of this Runner's bots that are due.
#[cfg(feature = "runner")]
pub async fn run(app: Arc<App>) {
    loop {
        tokio::time::sleep(TICK).await;
        crate::feedback::tick(&app);
        tick(&app);
    }
}

/// Starts every due routine of a bot on this Runner, or its check, unless the user has been
/// away, in which case the due ones are paused with a notice instead.
#[cfg(feature = "runner")]
pub fn tick(app: &Arc<App>) {
    let Some(this) = app.this_device_id() else { return };
    let now = now_unix();
    let enabled: Vec<Routine> = app.state.lock().unwrap().routines.iter().filter(|r| r.is_enabled).cloned().collect();
    let due: Vec<Routine> = enabled
        .into_iter()
        .filter(|r| r.next_run_at().is_some_and(|t| t <= now))
        .filter(|r| app.bot(&r.bot_id).is_some_and(|b| b.runner_id == this) && !app.is_routine_running(&r.id) && !app.routine_checks.is_running(&r.id))
        .filter(|r| app.budgets.admit(app, "routine", &r.id).is_ok())
        .collect();
    if due.is_empty() {
        return;
    }
    if user_away(app, now) {
        pause_while_away(app, &due);
        return;
    }
    for routine in due {
        // The due time is taken before any work starts, so a restart neither runs it again nor
        // runs it late; with `skip`, one more than a minute late is only taken.
        let due_at = routine.next_run_at().unwrap_or(now);
        if let Err(error) = app.record_routine(&routine.id, |routine| routine.last_scheduled_at = Some(now as f64)) {
            tracing::error!(%error, routine = %routine.name, "saving a routine's due time");
            continue;
        }
        if routine.missed_run_policy.should_skip(due_at, now) {
            continue;
        }
        if routine.check.is_some() {
            check_then_run(app, routine);
            continue;
        }
        started(app, &routine.id);
        match job_for(app, &routine) {
            Ok(job) => runtime::spawn_local_job(app.clone(), job, None),
            Err(error) => tracing::warn!(%error, routine = %routine.name, "starting a routine"),
        }
    }
}

// MARK: - Checks

/// How long a check may take, plugin servers starting included.
#[cfg(feature = "runner")]
const CHECK_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(5 * 60);
/// The most of what a check found that its run reads.
#[cfg(feature = "runner")]
const MAX_FOUND_CHARS: usize = 4_000;
/// The bot's file tools a check may call: the ones that only read.
#[cfg(feature = "runner")]
const CHECK_FILE_TOOLS: [&str; 4] = ["read", "grep", "find", "ls"];

/// In-flight check locks. Completed health and schedule anchors live in the durable roster.
#[cfg(feature = "runner")]
#[derive(Default)]
pub struct Checks(std::sync::Mutex<std::collections::HashMap<String, CheckState>>);

#[cfg(feature = "runner")]
#[derive(Default, Clone, Copy)]
struct CheckState {
    running: bool,
}

#[cfg(feature = "runner")]
impl Checks {
    pub fn is_running(&self, id: &str) -> bool {
        self.0.lock().unwrap().get(id).is_some_and(|state| state.running)
    }

    /// Marks the routine's check running; false when it already is.
    fn start(&self, id: &str) -> bool {
        let mut checks = self.0.lock().unwrap();
        let state = checks.entry(id.to_string()).or_default();
        !std::mem::replace(&mut state.running, true)
    }

    /// Marks the routine's check running once no other check of it runs; false when `cancel`
    /// stops the wait.
    async fn start_when_free(&self, id: &str, cancel: &CancellationToken) -> bool {
        while !self.start(id) {
            tokio::select! {
                _ = cancel.cancelled() => return false,
                _ = tokio::time::sleep(std::time::Duration::from_millis(250)) => {}
            }
        }
        true
    }

    fn finish(&self, id: &str) {
        self.0.lock().unwrap().remove(id);
    }
}

/// A check ended: its health is saved, so the schedule counts from it after a restart too. The
/// roster goes up when the check changed how the routine stands (it failed, or passed after a
/// failure); a quiet check after another changes nothing the other Devices show.
#[cfg(feature = "runner")]
fn checked(app: &App, routine: &Routine, run: &CheckRun) -> Result<(), String> {
    let failure = run.error.as_deref().map(crate::routine_health::classify);
    let mut changed = false;
    let saved = app.record_routine(&routine.id, |current| {
        if current.check != routine.check || current.enabled_at > routine.enabled_at {
            return;
        }
        let health = current.health.get_or_insert_with(Default::default);
        let before = health.status;
        health.record(now_secs(), run.found.is_some(), failure);
        changed = failure.is_some() || health.status != before;
        if health.authentication_failures >= 3 {
            current.is_enabled = false;
            current.paused_reason = Some("authentication".into());
        }
    });
    app.routine_checks.finish(&routine.id);
    if saved.is_ok() && changed {
        app.push_roster();
    }
    saved.map(|_| ()).map_err(|error| format!("Could not save the routine's check: {error}"))
}

/// When the routine runs next, as the apps and the bot say it. Its Runner takes a due time
/// within half a minute, so one past for longer is a Runner that is offline or, elsewhere, one
/// whose quiet checks went unannounced: the routine is shown due at its next time from now.
pub fn next_run_shown(routine: &Routine) -> Option<i64> {
    let next = routine.next_run_at()?;
    let now = now_unix();
    if next < now - 60 {
        return schedule::parse(&routine.schedule).ok().and_then(|schedule| schedule.next_after(now, &routine.timezone)).or(Some(next));
    }
    Some(next)
}

/// Runs a due routine's check, then the routine when the check found something or failed. A
/// quiet check publishes its completed health without a marker or model turn. Transport
/// failures back off; authentication failures pause after three attempts.
///
/// One check of a routine runs at a time. A run started by hand meanwhile waits for this check
/// before it runs its own (`check_now`), which no longer sees what this one found and stored as
/// seen; so what this one found still starts its run, after that one, as the chat runs one turn
/// at a time.
#[cfg(feature = "runner")]
fn check_then_run(app: &Arc<App>, routine: Routine) {
    if !app.routine_checks.start(&routine.id) {
        return;
    }
    let app = app.clone();
    tokio::spawn(async move {
        let found = run_check(&app, &routine, &CancellationToken::new()).await;
        if let Err(error) = checked(&app, &routine, &found) {
            tracing::error!(%error, routine = %routine.name, "persisting routine check health");
            return;
        }
        // A routine paused, deleted, or given another check meanwhile does not run on this one.
        let Some(current) = app.routine(&routine.id).filter(|current| current.is_enabled && current.check == routine.check) else { return };
        // Transport/auth failures retry with durable backoff instead of starting a model
        // turn on every failure. Script failures still hand their error to the bot to fix.
        if found.error.as_deref().is_some_and(|error| matches!(crate::routine_health::classify(error), crate::routine_health::Failure::Connection | crate::routine_health::Failure::Authentication | crate::routine_health::Failure::Blocked)) {
            return;
        }
        let Some(report) = found.report() else { return };
        started(&app, &current.id);
        match job_for(&app, &current) {
            Ok(mut job) => {
                job.check = Some(report);
                runtime::spawn_local_job(app.clone(), job, None);
            }
            Err(error) => tracing::warn!(%error, routine = %current.name, "starting a routine its check called for"),
        }
    });
}

/// How a check went.
#[cfg(feature = "runner")]
#[derive(Debug, Clone, PartialEq)]
pub struct CheckRun {
    /// What it returned, as its run reads it; `None` when it returned nothing to report.
    pub found: Option<String>,
    /// How it failed, when it did.
    pub error: Option<String>,
    /// Its whole result, what it printed and its calls included, for the bot that wrote it.
    pub result: String,
}

#[cfg(feature = "runner")]
impl CheckRun {
    /// What the run reads about the check, when the check calls for a run.
    pub fn report(&self) -> Option<CheckReport> {
        (self.found.is_some() || self.error.is_some()).then(|| CheckReport { found: self.found.clone().unwrap_or_default(), error: self.error.clone() })
    }
}

/// Runs a routine's check now, for a run started by hand or a check just saved: after a check of
/// the routine already running, and counted like a due one, so the schedule counts from it.
#[cfg(feature = "runner")]
pub async fn check_now(app: &Arc<App>, routine: &Routine, cancel: &CancellationToken) -> CheckRun {
    if !app.routine_checks.start_when_free(&routine.id, cancel).await {
        let stopped = "Stopped before the check ran.".to_string();
        return CheckRun { found: None, error: Some(stopped.clone()), result: stopped };
    }
    let found = run_check(app, routine, cancel).await;
    if let Err(error) = checked(app, routine, &found) {
        return CheckRun { found: None, error: Some(error.clone()), result: error };
    }
    found
}

/// Runs a routine's check: its script in a codemode sandbox of its own, with the bot's file
/// tools that only read, the read-only tools of this Runner's plugins, the values the bot's
/// scripts keep in its direct chat, and `models.ask`. No model of the bot's runs and nobody is
/// asked anything: a call that could change something ends the check.
#[cfg(feature = "runner")]
pub async fn run_check(app: &Arc<App>, routine: &Routine, cancel: &CancellationToken) -> CheckRun {
    let failed = |error: String| CheckRun { found: None, error: Some(error.clone()), result: error };
    let budget = match crate::budgets::for_routine(app, routine) { Ok(budget) => budget, Err(error) => return failed(error) };
    match budget.run(cancel, run_budgeted_check(app, routine, cancel)).await {
        Ok(run) => run,
        Err(error) => failed(error),
    }
}

#[cfg(feature = "runner")]
async fn run_budgeted_check(app: &Arc<App>, routine: &Routine, cancel: &CancellationToken) -> CheckRun {
    let failed = |error: String| CheckRun { found: None, error: Some(error.clone()), result: error };
    let Some(code) = routine.check.as_deref() else { return CheckRun { found: None, error: None, result: String::new() } };
    let Some(bot) = app.bot(&routine.bot_id) else { return failed("The routine's bot is gone.".into()) };
    let dm = match app.dm_with(&bot.id, None) {
        Ok(dm) => dm,
        Err(error) => return failed(error.to_string()),
    };
    let files: Vec<Arc<dyn Tool>> =
        lorca_agent::tools::coding_tools(bot.working_directory(&app.config.home)).into_iter().filter(|tool| CHECK_FILE_TOOLS.contains(&tool.name())).collect();
    let catalog = crate::plugins::mcp::bot_catalog(app, &bot, &dm.meta.id, files);
    let store = Arc::new(crate::scripts::ScriptStore { app: app.clone(), chat_id: dm.meta.id.clone(), bot_id: bot.id.clone() });
    let functions: Vec<Arc<dyn HostFunction>> =
        crate::scripts::ModelsAsk::new(app, &dm.meta.id, &bot.provider).map(|ask| Arc::new(ask) as Arc<dyn HostFunction>).into_iter().collect();
    let options = CodemodeOptions { mcp_types: !crate::plugins::mcp::plugin_briefs(app).is_empty(), timeout: CHECK_TIMEOUT, ..CodemodeOptions::default() };
    let codemode = CodemodeTool::new(catalog.clone(), options).with_store(store).with_functions(functions);
    let runner = CheckRunner { app: app.clone(), catalog, bot, chat_id: dm.meta.id.clone(), budget: crate::budgets::current() };
    match codemode.run_script(&format!("check-{}", routine.id), code, cancel.clone(), &runner).await {
        Err(error) => failed(error.0),
        Ok(run) => {
            let result = run.result.text_content();
            if run.result.is_error {
                CheckRun { found: None, error: Some(clipped(&result, MAX_FOUND_CHARS)), result }
            } else {
                CheckRun { found: run.returned.as_ref().and_then(found_text), error: None, result }
            }
        }
    }
}

/// What a check returned, as its run reads it: a string as it is, anything else as JSON.
/// Nothing, `null`, `false`, and an empty string, list, or object have nothing to report.
#[cfg(feature = "runner")]
fn found_text(value: &Value) -> Option<String> {
    let text = match value {
        Value::Null | Value::Bool(false) => return None,
        Value::Array(items) if items.is_empty() => return None,
        Value::Object(fields) if fields.is_empty() => return None,
        Value::String(text) => text.trim().to_string(),
        other => serde_json::to_string_pretty(other).unwrap_or_default(),
    };
    (!text.is_empty()).then(|| clipped(&text, MAX_FOUND_CHARS))
}

#[cfg(feature = "runner")]
fn clipped(text: &str, max: usize) -> String {
    if text.chars().count() > max {
        format!("{}…", text.chars().take(max).collect::<String>())
    } else {
        text.to_string()
    }
}

/// Runs a check's calls: the read-only ones the bot's Access allows, and no other. A refused
/// call ends the check.
#[cfg(feature = "runner")]
struct CheckRunner {
    app: Arc<App>,
    catalog: Arc<crate::plugins::mcp::PluginCatalog>,
    bot: Bot,
    chat_id: String,
    budget: Option<crate::budgets::BudgetContext>,
}

#[cfg(feature = "runner")]
#[async_trait::async_trait]
impl ToolRunner for CheckRunner {
    async fn run(&self, tool: Arc<dyn Tool>, tool_call_id: String, args: Value, cancel: CancellationToken) -> ToolOutcome {
        if let Some(reason) = self.budget.as_ref().and_then(|budget| budget.check().err()) {
            return ToolOutcome { result: ToolResult { is_error: true, ..ToolResult::text(reason) }, is_error: true, blocked: true };
        }
        let allowed = if CHECK_FILE_TOOLS.contains(&tool.name()) {
            crate::permissions::check_tool(&self.app, &self.bot, tool.name())
        } else {
            crate::plugins::mcp::authorize_catalog_tool(&self.app, &self.catalog, &self.bot, tool.name(), &cancel).await
        };
        if let Err(denied) = allowed {
            let refusal = crate::permissions::refuse(&self.app, &self.chat_id, &self.bot, denied);
            return ToolOutcome { result: ToolResult { is_error: true, ..ToolResult::text(refusal.reason.unwrap_or_default()) }, is_error: true, blocked: true };
        }
        let reads = CHECK_FILE_TOOLS.contains(&tool.name()) || crate::plugins::mcp::is_read_only(&self.app, &self.catalog, tool.name(), &cancel).await;
        if !reads {
            let refusal = format!("{} can change things, and a check only looks: leave it to the run the check starts.", tool.name());
            return ToolOutcome { result: ToolResult { is_error: true, ..ToolResult::text(refusal) }, is_error: true, blocked: true };
        }
        DirectRunner.run(tool, tool_call_id, args, cancel).await
    }
}

/// True when the user has not written in any chat for a week. A user who never wrote counts
/// from the newest routine, so a fresh account is never "away".
#[cfg(any(feature = "runner", test))]
fn user_away(app: &Arc<App>, now: i64) -> bool {
    let last_routine = app.state.lock().unwrap().routines.iter().map(|r| r.created_at.max(r.enabled_at) as i64).max();
    let last_message = app.store.last_user_at().unwrap_or(None);
    let last_activity = last_message.max(last_routine).unwrap_or(now);
    now - last_activity > AWAY_AFTER_SECS
}

/// Pauses the routines with a notice in each bot's chat, in Grok Bot's words.
#[cfg(feature = "runner")]
fn pause_while_away(app: &Arc<App>, routines: &[Routine]) {
    let mut by_bot: std::collections::BTreeMap<String, Vec<String>> = std::collections::BTreeMap::new();
    for routine in routines {
        let _ = app.update_routine(&routine.id, |r| {
            r.is_enabled = false;
            r.paused_reason = Some("away".into());
        });
        by_bot.entry(routine.bot_id.clone()).or_default().push(routine.name.clone());
    }
    for (bot_id, names) in by_bot {
        let Ok(dm) = app.dm_with(&bot_id, None) else { continue };
        let days = AWAY_AFTER_SECS / 86_400;
        app.notice(
            &dm.meta.id,
            format!(
                "Routines paused while you were away: {}. Nobody has written in {days} days, so they stopped to avoid wasted spend. Turn them back on from this chat's inspector, or ask here.",
                names.join(", ")
            ),
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    struct ScratchApp(Arc<App>, std::path::PathBuf);
    impl Drop for ScratchApp {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.1);
        }
    }

    /// An App with an identity on a scratch home, so this Device is a Runner, and one bot
    /// `b1` (Chef) assigned to it.
    fn scratch_app() -> ScratchApp {
        let home = std::env::temp_dir().join(format!("lorca-routines-{}", uuid::Uuid::new_v4()));
        let app = App::load(crate::config::Config { home: home.clone(), port: 0 }).unwrap();
        crate::identity::create(&app, Some("Workbench".into())).unwrap();
        let runner_id = app.this_device_id().unwrap();
        {
            let mut state = app.state.lock().unwrap();
            state.bots.clear();
            state.chats.clear();
            state.bots.push(Bot {
                id: "b1".into(),
                name: "Chef".into(),
                description: String::new(),
                symbol_name: String::new(),
                accent: String::new(),
                avatar: None,
                runner_id,
                provider: "deepseek".into(),
                model: None,
                thinking: None,
                legacy_instructions: String::new(),
                workdir: None,
                permissions: None,
                created_at: 0.0,
            });
        }
        ScratchApp(app, home)
    }

    #[cfg(feature = "runner")]
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn bot_permissions_apply_to_unattended_check_reads() {
        let scratch = scratch_app();
        let app = &scratch.0;
        let routine = create(app, "b1", "Inbox", "every 1h", "Report changes", Some("return await tools.ls({});"), true).unwrap();
        let policy = serde_json::from_value(serde_json::json!({"filesystem":"none","shell":false})).unwrap();
        app.update_bot("b1", |bot| bot.permissions = Some(policy)).unwrap();
        let result = run_check(app, &routine, &CancellationToken::new()).await;
        assert!(result.error.as_deref().is_some_and(|error| error.contains("reading files is off")), "{:?}", result.error);
        // The routine shows as blocked, not as a check its bot can fix.
        assert_eq!(crate::routine_health::classify(result.error.as_deref().unwrap()), crate::routine_health::Failure::Blocked);
    }

    #[test]
    fn routines_are_checked_and_named_once() {
        let scratch = scratch_app();
        let app = &scratch.0;
        assert!(create(app, "b1", "Brief", "every 2m", "x", None, true).unwrap_err().contains("too often"));
        assert!(create(app, "b1", "", "every 1h", "x", None, true).unwrap_err().contains("name"));
        assert!(create(app, "b1", "Brief", "every 1h", "  ", None, true).unwrap_err().contains("should do"));
        let brief = create(app, "b1", "  Morning   brief ", "0 9 * * 1-5", "Summarize the inbox.", None, true).unwrap();
        assert_eq!(brief.name, "Morning brief");
        assert_eq!(brief.schedule, "0 9 * * 1-5");
        assert!(brief.is_enabled && brief.next_run_at().is_some());
        assert!(create(app, "b1", "morning BRIEF", "every 1h", "x", None, true).unwrap_err().contains("already exists"));
        assert!(create(app, "b2", "Other", "every 1h", "x", None, true).unwrap_err().contains("Unknown bot"));

        let edited = edit(app, &brief.id, None, Some("every 2h"), None, None).unwrap();
        assert_eq!(edited.schedule, "every 2h");
        assert!(edited.enabled_at >= brief.enabled_at);
        assert!(edit(app, &brief.id, None, None, None, None).unwrap_err().contains("Pass a new"));

        let paused = set_enabled(app, &brief.id, false).unwrap();
        assert!(!paused.is_enabled && paused.next_run_at().is_none());
        let resumed = set_enabled(app, &brief.id, true).unwrap();
        assert!(resumed.is_enabled && resumed.paused_reason.is_none());
        assert_eq!(app.routines_of("b1").len(), 1);
        delete(app, &brief.id).unwrap();
        assert!(app.routines_of("b1").is_empty());
        assert!(delete(app, &brief.id).is_err());
    }

    #[test]
    fn the_next_run_counts_from_the_last_one() {
        let scratch = scratch_app();
        let app = &scratch.0;
        let hourly = create(app, "b1", "Hourly", "every 1h", "x", None, true).unwrap();
        assert_eq!(hourly.next_run_at(), Some(hourly.enabled_at as i64 + 3600));
        let ran = app.update_routine(&hourly.id, |r| r.last_run_at = Some(hourly.enabled_at + 7200.0)).unwrap();
        assert_eq!(ran.next_run_at(), Some(hourly.enabled_at as i64 + 10_800));
        // A resume after a long pause counts from the resume, not the old run.
        let paused = set_enabled(app, &hourly.id, false).unwrap();
        assert_eq!(paused.next_run_at(), None);
        let resumed = set_enabled(app, &hourly.id, true).unwrap();
        assert!(resumed.next_run_at().unwrap() >= resumed.enabled_at as i64 + 3600 - 1);
    }

    /// The scheduler fires a due routine once, counts the schedule from that run, and refuses
    /// to queue it again while the first run has not ended. Without a provider the run ends in
    /// a notice and an error outcome, which is what the panel would show.
    #[cfg(feature = "runner")]
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn the_tick_runs_a_due_routine_once() {
        let scratch = scratch_app();
        let app = &scratch.0;
        let hourly = create(app, "b1", "Hourly", "every 1h", "x", None, true).unwrap();
        tick(app);
        assert_eq!(app.routine(&hourly.id).unwrap().last_run_at, None, "not due yet");
        // Armed an hour and a bit ago: due now.
        let now = now_secs();
        app.update_routine(&hourly.id, |r| r.enabled_at = now - 3700.0).unwrap();
        tick(app);
        let ran = app.routine(&hourly.id).unwrap();
        let first_run = ran.last_run_at.expect("started");
        assert!(ran.next_run_at().unwrap() > now_unix() + 3500, "the next run counts from this one");
        tick(app);
        // The run itself stamps the start too, a moment after the tick did.
        let again = app.routine(&hourly.id).unwrap().last_run_at.unwrap();
        assert!((again - first_run).abs() < 5.0, "not started again: {again} vs {first_run}");
        // The run ends: no provider on this Runner, so the chat says so and the outcome is an error.
        let dm = app.dm_with("b1", None).unwrap();
        for _ in 0..100 {
            if app.routine(&hourly.id).unwrap().last_outcome.is_some() {
                break;
            }
            tokio::time::sleep(std::time::Duration::from_millis(50)).await;
        }
        assert_eq!(app.routine(&hourly.id).unwrap().last_outcome.as_deref(), Some("error"));
        let notices: Vec<String> = app.messages(&dm.meta.id).iter().filter_map(|m| match &m.body { Body::Notice { text, .. } => Some(text.clone()), _ => None }).collect();
        assert_eq!(notices[0], "Routine · Hourly", "the marker opens the run");
        assert!(notices[1].starts_with("Chef cannot run yet"), "{notices:?}");
        assert_eq!(notices.len(), 2, "one run, one marker: {notices:?}");
    }

    /// A due routine of a user who has been away for a week is paused with a notice, not run.
    #[cfg(feature = "runner")]
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn away_pauses_due_routines_with_a_notice() {
        let scratch = scratch_app();
        let app = &scratch.0;
        let brief = create(app, "b1", "Brief", "every 1h", "x", None, true).unwrap();
        let long_ago = now_secs() - (AWAY_AFTER_SECS as f64) - 7200.0;
        app.update_routine(&brief.id, |r| {
            r.created_at = long_ago;
            r.enabled_at = long_ago;
        })
        .unwrap();
        tick(app);
        let paused = app.routine(&brief.id).unwrap();
        assert!(!paused.is_enabled && paused.paused_reason.as_deref() == Some("away") && paused.last_run_at.is_none());
        let dm = app.dm_with("b1", None).unwrap();
        let messages = app.messages(&dm.meta.id);
        let Body::Notice { text, routine_id } = &messages[0].body else { panic!("a notice") };
        assert!(text.starts_with("Routines paused while you were away: Brief."), "{text}");
        assert_eq!(routine_id, &None);
        // Resuming arms it from now, so it is not due again at once.
        let resumed = set_enabled(app, &brief.id, true).unwrap();
        assert!(resumed.next_run_at().unwrap() > now_unix() + 3500);
        tick(app);
        assert_eq!(app.routine(&brief.id).unwrap().last_run_at, None);
    }

    #[test]
    fn a_week_of_silence_is_away() {
        let scratch = scratch_app();
        let app = &scratch.0;
        let now = now_unix();
        assert!(!user_away(app, now), "a fresh account is not away");
        create(app, "b1", "Hourly", "every 1h", "x", None, true).unwrap();
        assert!(!user_away(app, now));
        assert!(user_away(app, now + AWAY_AFTER_SECS + 60));
        let dm = app.dm_with("b1", None).unwrap();
        let mut message = Message::new(&dm.meta.id, Author::You, Body::text("hi"));
        message.created_at = (now + AWAY_AFTER_SECS) as f64;
        app.upsert_message(message, false);
        assert!(!user_away(app, now + AWAY_AFTER_SECS + 60));
    }

    fn plain(id: &str, bot_id: &str, check: Option<&str>) -> Routine {
        Routine {
            id: id.into(),
            bot_id: bot_id.into(),
            name: id.into(),
            prompt: "x".into(),
            feedback_authorization_prompt: None,
            schedule: "every 1h".into(),
            timezone: "UTC".into(),
            missed_run_policy: Default::default(),
            last_scheduled_at: None,
            health: None,
            is_enabled: true,
            enabled_at: 0.0,
            last_run_at: None,
            last_outcome: None,
            paused_reason: None,
            check: check.map(str::to_string),
            created_at: 0.0,
        }
    }

    #[test]
    fn a_check_is_kept_as_written_and_removed_when_empty() {
        let scratch = scratch_app();
        let app = &scratch.0;
        let watch = create(app, "b1", "Watch", "every 10m", "Tell me what is new.", Some("  return load('seen');  "), true).unwrap();
        assert_eq!(watch.check.as_deref(), Some("return load('seen');"));
        assert!(create(app, "b1", "Long", "every 1h", "x", Some(&"x".repeat(MAX_CHECK_CHARS + 1)), true).unwrap_err().contains("under"));
        #[cfg(feature = "runner")]
        assert!(create(app, "b1", "Bad", "every 1h", "x", Some("// @options: {\"retries\": 3}\nreturn 1"), true).unwrap_err().contains("@options"));
        assert_eq!(create(app, "b1", "Plain", "every 1h", "x", Some("   "), true).unwrap().check, None);
        assert_eq!(edit(app, &watch.id, None, None, None, Some("")).unwrap().check, None, "an empty check removes it");
    }

    /// Only a Runner writes its routines' checks, so one an incoming roster leaves out was
    /// dropped by a build that does not know them.
    #[test]
    fn a_roster_without_checks_keeps_this_runners_checks() {
        let scratch = scratch_app();
        let app = &scratch.0;
        let mut bots = app.state.lock().unwrap().bots.clone();
        let here = bots[0].runner_id.clone();
        let mut away = bots[0].clone();
        away.id = "b2".into();
        away.runner_id = "another-runner".into();
        bots.push(away);
        let current = vec![plain("r1", "b1", Some("return 1")), plain("r2", "b2", Some("return 2"))];
        let mut incoming = vec![plain("r1", "b1", None), plain("r2", "b2", None), plain("r3", "b1", None)];
        assert!(keep_checks(&current, &mut incoming, &bots, &here));
        assert_eq!(incoming[0].check.as_deref(), Some("return 1"));
        assert_eq!(incoming[1].check, None, "another Runner keeps its own");
        assert_eq!(incoming[2].check, None);
        // A check that arrives stands.
        let mut incoming = vec![plain("r1", "b1", Some("return 9"))];
        assert!(!keep_checks(&current, &mut incoming, &bots, &here));
        assert_eq!(incoming[0].check.as_deref(), Some("return 9"));
    }

    /// A due time past for more than a minute (an offline Runner, or quiet checks the Runner did
    /// not announce) is shown as the next time from now; one just due is shown as it is.
    #[test]
    fn a_routine_past_due_is_shown_due_from_now() {
        let mut routine = plain("r1", "b1", Some("return null"));
        routine.enabled_at = now_secs() - 7200.0;
        assert!(next_run_shown(&routine).unwrap() > now_unix());
        routine.check = None;
        assert!(next_run_shown(&routine).unwrap() > now_unix(), "with or without a check");
        routine.enabled_at = now_secs() - 3610.0;
        assert!(next_run_shown(&routine).unwrap() <= now_unix(), "one just due is about to run");
    }

    #[cfg(feature = "runner")]
    #[test]
    fn nothing_returned_is_nothing_found() {
        use serde_json::json;
        for quiet in [json!(null), json!(false), json!(""), json!("  "), json!([]), json!({})] {
            assert_eq!(found_text(&quiet), None, "{quiet}");
        }
        assert_eq!(found_text(&json!(" two new ")).as_deref(), Some("two new"));
        assert_eq!(found_text(&json!({ "new": [1] })).as_deref(), Some("{\n  \"new\": [\n    1\n  ]\n}"));
        assert!(found_text(&json!("y".repeat(MAX_FOUND_CHARS + 10))).unwrap().ends_with('…'));
    }

    /// A check reads with the bot's file tools, keeps values with store(), and returns what it
    /// found; a tool that writes is not there to call, and its failure is what the run reads.
    #[cfg(feature = "runner")]
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn a_check_looks_and_says_what_it_found() {
        let scratch = scratch_app();
        let app = &scratch.0;
        let bot = app.bot("b1").unwrap();
        let workdir = bot.working_directory(&app.config.home);
        std::fs::create_dir_all(&workdir).unwrap();
        std::fs::write(workdir.join("notes.txt"), "hello").unwrap();
        let looks = "const seen = load('runs') ?? 0;\nstore('runs', seen + 1);\nconst files = await tools.ls({});\nreturn files.includes('notes.txt') ? 'notes.txt is here' : null;";
        let watch = create(app, "b1", "Watch", "every 10m", "Tell me what is new.", Some(looks), true).unwrap();
        let checked = run_check(app, &watch, &CancellationToken::new()).await;
        assert_eq!((checked.found.as_deref(), checked.error.as_deref()), (Some("notes.txt is here"), None), "{}", checked.result);
        assert_eq!(checked.report(), Some(CheckReport { found: "notes.txt is here".into(), error: None }));
        let dm = app.dm_with("b1", None).unwrap();
        assert_eq!(app.store.codemode_values(&dm.meta.id, "b1").unwrap()["runs"], serde_json::json!(1));

        let writes = edit(app, &watch.id, None, None, None, Some("await tools.write({ path: 'x.txt', content: 'y' });")).unwrap();
        let checked = run_check(app, &writes, &CancellationToken::new()).await;
        assert!(checked.error.as_deref().is_some_and(|error| error.contains("Unknown tool")), "{checked:?}");
        assert!(checked.report().unwrap().error.is_some());
        assert!(!workdir.join("x.txt").exists());
    }

    /// A quiet due check records and publishes health without a model run. Findings start a run.
    #[cfg(feature = "runner")]
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn a_due_check_starts_the_run_only_when_it_finds_something() {
        let scratch = scratch_app();
        let app = &scratch.0;
        let quiet = create(app, "b1", "Quiet", "every 10m", "Tell me what is new.", Some("return null;"), true).unwrap();
        let now = now_secs();
        app.update_routine(&quiet.id, |r| r.enabled_at = now - 700.0).unwrap();
        let mut events = app.events.subscribe();
        tick(app);
        for _ in 0..200 {
            if app.routine(&quiet.id).unwrap().health.as_ref().and_then(|health| health.last_check_at).is_some() && !app.routine_checks.is_running(&quiet.id) {
                break;
            }
            tokio::time::sleep(std::time::Duration::from_millis(25)).await;
        }
        let checked_at = app.routine(&quiet.id).unwrap().health.unwrap().last_check_at.unwrap() as i64;
        let after = app.routine(&quiet.id).unwrap();
        assert_eq!((after.last_run_at, after.last_outcome.as_deref()), (None, None), "a check is not a run");
        assert_eq!(after.next_run_at(), after.next_run_after(checked_at));
        assert!(after.next_run_at().unwrap() > now_unix() + 500);
        let dm = app.dm_with("b1", None).unwrap();
        assert!(app.messages(&dm.meta.id).is_empty(), "no marker, no turn");
        // The roster event carries the persisted next check to apps.
        let mut heard = false;
        while let Ok(event) = events.try_recv() {
            if let crate::events::Event::RosterChanged { routines, .. } = event {
                heard |= routines.iter().any(|r| r["id"] == quiet.id.as_str() && r["next_run_at"].as_f64().is_some_and(|next| next > (now_unix() + 500) as f64));
            }
        }
        assert!(heard, "a roster event with the next check");

        let found = create(app, "b1", "Found", "every 10m", "Tell me what is new.", Some("return 'Two new pull requests';"), true).unwrap();
        app.update_routine(&found.id, |r| r.enabled_at = now - 700.0).unwrap();
        tick(app);
        for _ in 0..200 {
            if app.routine(&found.id).unwrap().last_outcome.is_some() {
                break;
            }
            tokio::time::sleep(std::time::Duration::from_millis(25)).await;
        }
        let ran = app.routine(&found.id).unwrap();
        assert!(ran.last_run_at.is_some());
        // No provider on this Runner: the run opens with its marker and ends in a notice.
        assert_eq!(ran.last_outcome.as_deref(), Some("error"));
        let notices: Vec<String> = app.messages(&dm.meta.id).iter().filter_map(|m| match &m.body { Body::Notice { text, .. } => Some(text.clone()), _ => None }).collect();
        assert_eq!(notices[0], "Routine · Found");
    }

    /// A check on save or by hand waits for a due check already running, and counts as the
    /// routine's last check, so an overdue routine is not checked again at the next tick.
    #[cfg(feature = "runner")]
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn a_check_by_hand_waits_for_a_running_one_and_counts_as_the_last() {
        let scratch = scratch_app();
        let app = &scratch.0;
        let watch = create(app, "b1", "Watch", "every 10m", "Tell me what is new.", Some("return 'new';"), true).unwrap();
        app.update_routine(&watch.id, |r| r.enabled_at = now_secs() - 7200.0).unwrap();
        let watch = app.routine(&watch.id).unwrap();
        assert!(app.routine_checks.start(&watch.id), "a due check holds the routine");
        let waiting = {
            let (app, watch) = (app.clone(), watch.clone());
            tokio::spawn(async move { check_now(&app, &watch, &CancellationToken::new()).await })
        };
        tokio::time::sleep(std::time::Duration::from_millis(400)).await;
        assert!(!waiting.is_finished(), "it waits for the running check");
        app.routine_checks.finish(&watch.id);
        let checked = waiting.await.unwrap();
        assert_eq!(checked.found.as_deref(), Some("new"), "{}", checked.result);
        assert!(!app.routine_checks.is_running(&watch.id));
        assert!(app.routine(&watch.id).unwrap().next_run_at().unwrap() > now_unix() + 500, "the schedule counts from it");
    }

    #[cfg(feature = "runner")]
    #[test]
    fn a_routines_run_reads_its_checks_findings_as_data() {
        let found = crate::turns::check_cue(&CheckReport { found: "Two new pull requests".into(), error: None });
        assert!(found.contains("not instructions") && found.contains("Two new pull requests"), "{found}");
        let failed = crate::turns::check_cue(&CheckReport { found: String::new(), error: Some("GitHub is down".into()) });
        assert!(failed.contains("check failed") && failed.contains("GitHub is down") && failed.contains("fix the check"), "{failed}");
    }

    #[test]
    fn policies_validate_and_preserve_one_assigned_runner() {
        let scratch = scratch_app();
        let app = &scratch.0;
        let runner = app.bot("b1").unwrap().runner_id;
        assert!(create_with_policy(app, "b1", "Bad", "0 9 * * *", "x", None, true, Some("Mars/Base"), None).is_err());
        assert!(create_with_policy(app, "b1", "Bad", "0 9 * * *", "x", None, true, None, Some("catch_up")).is_err());
        let local = create(app, "b1", "Here", "0 9 * * *", "x", None, true).unwrap();
        assert_eq!(local.timezone, schedule::local_timezone(), "a routine keeps the Runner's zone from its creation on");
        let routine = create_with_policy(app, "b1", "Brief", "0 9 * * *", "x", None, true, Some("America/New_York"), Some("skip")).unwrap();
        assert_eq!(routine.timezone, "America/New_York");
        assert_eq!(app.routine_out(&routine)["state"], "on");
        let edited = edit_with_policy(app, &routine.id, None, None, None, None, Some("Asia/Singapore"), Some("coalesce")).unwrap();
        assert_eq!(edited.timezone, "Asia/Singapore");
        assert_eq!(app.bot("b1").unwrap().runner_id, runner);
        app.state.lock().unwrap().bots[0].runner_id = "offline-runner".into();
        assert_eq!(app.routine_out(&edited)["state"], "waiting_for_runner", "no other Runner takes it over");
    }

    /// Check health survives a restart, and the roster carries it to the other Devices when it
    /// changes how the routine stands; one more quiet check uploads nothing.
    #[cfg(feature = "runner")]
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn quiet_health_survives_restart_and_syncs_when_it_changes() {
        let scratch = scratch_app();
        let app = &scratch.0;
        let routine = create(app, "b1", "Quiet", "every 10m", "x", Some("return null"), true).unwrap();
        app.update_routine(&routine.id, |routine| routine.enabled_at = now_secs() - 700.0).unwrap();
        let routine = app.routine(&routine.id).unwrap();
        check_now(app, &routine, &CancellationToken::new()).await;
        let after = app.routine(&routine.id).unwrap();
        let health = after.health.as_ref().unwrap();
        assert!(health.last_check_at.is_some() && health.last_success_at.is_some());
        assert_eq!(health.status, Some(crate::routine_health::CheckStatus::Quiet));
        assert_eq!((after.last_run_at, after.last_outcome.as_ref()), (None, None));
        let restarted = App::load(crate::config::Config { home: scratch.1.clone(), port: 0 }).unwrap();
        assert_eq!(restarted.routine(&routine.id).unwrap(), after);
        tick(&restarted);
        assert!(!restarted.routine_checks.is_running(&routine.id), "not checked again after restart");
        let roster = || app.store.outbox().unwrap().into_iter().find(|blob| blob.kind == "roster").unwrap().ciphertext;
        let uploaded: RosterBlob = crate::crypto::decrypt_json(&app.dek().unwrap(), "roster", &roster()).unwrap();
        assert_eq!(uploaded.routines[0].health, after.health, "the first quiet check goes up");
        let before = roster();
        check_now(app, &after, &CancellationToken::new()).await;
        assert!(app.routine(&routine.id).unwrap().health.unwrap().last_check_at >= after.health.unwrap().last_check_at);
        assert_eq!(roster(), before, "another quiet check uploads nothing");
        let failing = edit(app, &routine.id, None, None, None, Some("throw new Error('connection refused')")).unwrap();
        check_now(app, &failing, &CancellationToken::new()).await;
        assert_ne!(roster(), before, "a failed check goes up");
    }

    #[cfg(feature = "runner")]
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn missed_runs_skip_or_coalesce_without_a_burst() {
        let scratch = scratch_app();
        let app = &scratch.0;
        let skip = create_with_policy(app, "b1", "Skip", "every 10m", "x", Some("return 'new'"), true, None, Some("skip")).unwrap();
        let coalesce = create(app, "b1", "Coalesce", "every 10m", "x", Some("store('checks', (load('checks') ?? 0) + 1); return null"), true).unwrap();
        for routine in [&skip, &coalesce] {
            app.update_routine(&routine.id, |routine| routine.enabled_at = now_secs() - 5.0 * 86_400.0).unwrap();
        }
        tick(app);
        for _ in 0..200 {
            if app.routine(&coalesce.id).unwrap().health.is_some() { break; }
            tokio::time::sleep(std::time::Duration::from_millis(25)).await;
        }
        assert!(app.routine(&skip.id).unwrap().health.is_none());
        assert!(app.routine(&skip.id).unwrap().last_scheduled_at.is_some());
        for _ in 0..5 { tick(app); }
        let dm = app.dm_with("b1", None).unwrap();
        assert_eq!(app.store.codemode_values(&dm.meta.id, "b1").unwrap()["checks"], serde_json::json!(1));
        assert!(app.messages(&dm.meta.id).is_empty());
        for routine in [&skip, &coalesce] { assert!(app.routine(&routine.id).unwrap().next_run_at().unwrap() > now_unix()); }
    }

    #[cfg(feature = "runner")]
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn authentication_pause_and_connection_backoff_survive_restart() {
        let scratch = scratch_app();
        let app = &scratch.0;
        let watch = create(app, "b1", "Watch", "every 5m", "x", Some("throw new Error('HTTP 401 Unauthorized')"), true).unwrap();
        for _ in 0..3 {
            let current = app.routine(&watch.id).unwrap();
            check_now(app, &current, &CancellationToken::new()).await;
        }
        let paused = app.routine(&watch.id).unwrap();
        assert_eq!(paused.paused_reason.as_deref(), Some("authentication"));
        assert!(!paused.is_enabled);
        assert!(run_now(app, &watch.id).unwrap_err().contains("resume it"));
        assert_eq!(app.routine_out(&paused)["state"], "blocked");
        let restarted = App::load(crate::config::Config { home: scratch.1.clone(), port: 0 }).unwrap();
        assert_eq!(restarted.routine(&watch.id).unwrap(), paused);
        let resumed = set_enabled(&restarted, &watch.id, true).unwrap();
        assert!(resumed.is_enabled && resumed.paused_reason.is_none());
        assert_eq!(resumed.health.unwrap().authentication_failures, 0);
        let connection = edit(app, &watch.id, None, None, None, Some("throw new Error('connection refused')")).unwrap();
        for _ in 0..3 { check_now(app, &connection, &CancellationToken::new()).await; }
        let after = app.routine(&watch.id).unwrap();
        assert_eq!(after.health.as_ref().unwrap().connection_failures, 3);
        assert!(after.next_run_at().is_none(), "editing does not implicitly resume a paused routine");
        let mut current = after.clone();
        current.is_enabled = true;
        assert!(current.next_run_at().unwrap() >= now_unix() + 1190);
    }

    #[test]
    fn stale_roster_edits_preserve_runner_check_health_and_cursor() {
        let scratch = scratch_app();
        let app = &scratch.0;
        let current = create(app, "b1", "Watch", "every 10m", "x", Some("return null"), true).unwrap();
        let mut stale = current.clone();
        let held = app.update_routine(&current.id, |routine| {
            routine.last_scheduled_at = Some(now_secs());
            routine.health = Some(Default::default());
            routine.health.as_mut().unwrap().record(now_secs(), false, None);
        }).unwrap();
        stale.name = "Renamed elsewhere".into();
        let mut incoming = vec![stale];
        assert!(keep_checks(std::slice::from_ref(&held), &mut incoming, &app.state.lock().unwrap().bots, &app.this_device_id().unwrap()));
        assert_eq!(incoming[0].health, held.health);
        assert_eq!(incoming[0].last_scheduled_at, held.last_scheduled_at);
        assert_eq!(incoming[0].name, "Renamed elsewhere");
    }

    #[cfg(feature = "runner")]
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn a_failed_checkpoint_does_not_admit_scheduled_work() {
        let scratch = scratch_app();
        let app = &scratch.0;
        let routine = create(app, "b1", "Watch", "every 10m", "x", Some("store('ran', true); return null"), true).unwrap();
        let due = app.update_routine(&routine.id, |routine| routine.enabled_at = now_secs() - 700.0).unwrap();
        let connection = rusqlite::Connection::open(app.config.database_path()).unwrap();
        connection.execute_batch("CREATE TRIGGER reject_routines BEFORE UPDATE ON routines BEGIN SELECT RAISE(FAIL, 'routine writes unavailable'); END;").unwrap();
        tick(app);
        assert_eq!(app.routine(&routine.id).unwrap(), due, "failed checkpoint rolls back in-memory admission");
        assert!(!app.routine_checks.is_running(&routine.id));
        let dm = app.dm_with("b1", None).unwrap();
        assert!(!app.store.codemode_values(&dm.meta.id, "b1").unwrap().contains_key("ran"));
        connection.execute_batch("DROP TRIGGER reject_routines").unwrap();
        tick(app);
        for _ in 0..200 {
            if app.routine(&routine.id).unwrap().health.is_some() { break; }
            tokio::time::sleep(std::time::Duration::from_millis(25)).await;
        }
        assert_eq!(app.store.codemode_values(&dm.meta.id, "b1").unwrap()["ran"], serde_json::json!(true));
    }

    #[cfg(feature = "runner")]
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn a_job_queued_before_authentication_pause_requires_resume() {
        let scratch = scratch_app();
        let app = &scratch.0;
        let routine = create(app, "b1", "Brief", "every 1h", "x", None, true).unwrap();
        let queued = job_for(app, &routine).unwrap();
        for _ in 0..3 { model_result(app, &routine.id, Some("HTTP 401 Unauthorized")); }
        let outcome = crate::turns::run_job(app, &queued, CancellationToken::new()).await;
        assert_eq!(outcome, TurnOutcome::Skipped);
        let paused = app.routine(&routine.id).unwrap();
        assert_eq!(paused.last_run_at, None, "no model run started");
        assert_eq!(paused.health.as_ref().unwrap().last_check_at, None, "a provider failure is not a check");
        assert_eq!(paused.health.as_ref().unwrap().model.authentication_failures, 3);
        assert!(app.messages(&queued.chat_id).iter().any(|message| matches!(&message.body, Body::Notice { text, .. } if text.contains("then resume it"))));
    }
}
