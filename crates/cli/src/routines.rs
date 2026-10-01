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
    let name = clean_name(name)?;
    let prompt = prompt.trim();
    if prompt.is_empty() {
        return Err("Say what the routine should do on each run.".into());
    }
    let check = check.map(clean_check).transpose()?.flatten();
    let schedule = schedule::parse(schedule_text)?;
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
        schedule: schedule.canonical(),
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
    if name.is_none() && schedule.is_none() && prompt.is_none() && check.is_none() {
        return Err("Pass a new name, schedule, prompt, or check.".into());
    }
    app.update_routine(id, |routine| {
        if let Some(name) = name {
            routine.name = name;
        }
        if let Some(schedule) = schedule {
            routine.schedule = schedule.canonical();
            routine.enabled_at = now_secs();
        }
        if let Some(prompt) = prompt {
            routine.prompt = prompt;
        }
        if let Some(check) = check {
            routine.check = check;
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
/// again with it.
pub fn keep_checks(current: &[Routine], incoming: &mut [Routine], bots: &[Bot], this_device: &str) -> bool {
    let mut kept = false;
    for routine in incoming.iter_mut().filter(|routine| routine.check.is_none()) {
        if !bots.iter().any(|bot| bot.id == routine.bot_id && bot.runner_id == this_device) {
            continue;
        }
        if let Some(check) = current.iter().find(|held| held.id == routine.id).and_then(|held| held.check.clone()) {
            routine.check = Some(check);
            kept = true;
        }
    }
    kept
}

/// Pauses or resumes a routine. A resumed schedule counts from now.
pub fn set_enabled(app: &Arc<App>, id: &str, enabled: bool) -> Result<Routine, String> {
    app.update_routine(id, |routine| {
        if enabled && !routine.is_enabled {
            routine.enabled_at = now_secs();
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

/// The schedule in words and the next firing from now, for a caller that wants to check a
/// schedule before saving it.
pub fn describe(schedule_text: &str) -> Result<Value, String> {
    let schedule = schedule::parse(schedule_text)?;
    let next = schedule.next_after(now_unix());
    Ok(serde_json::json!({
        "schedule": schedule.canonical(),
        "text": schedule.describe(),
        "next_run_at": next.map(|t| t as f64),
    }))
}

// MARK: - Running

/// Starts a run of the routine now, here or on the bot's Runner through the relay. Refuses
/// while a run is going on.
pub fn run_now(app: &Arc<App>, id: &str) -> Result<(), String> {
    let routine = app.routine(id).ok_or("Unknown routine")?;
    if app.is_routine_running(&routine.id) {
        return Err(format!("{} is running right now.", routine.name));
    }
    let job = job_for(app, &routine)?;
    runtime::start_turn(app, job);
    Ok(())
}

/// The job that runs a routine: a `routine` turn in the bot's direct chat.
fn job_for(app: &Arc<App>, routine: &Routine) -> Result<Job, String> {
    let dm = app.dm_with(&routine.bot_id, None).map_err(|e| e.to_string())?;
    Ok(Job {
        id: format!("job-{}", uuid::Uuid::new_v4()),
        chat_id: dm.meta.id,
        bot_id: routine.bot_id.clone(),
        kind: "routine".into(),
        trigger_message_id: String::new(),
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

/// The scheduler: every half minute, run the routines of this Runner's bots that are due.
#[cfg(feature = "runner")]
pub async fn run(app: Arc<App>) {
    loop {
        tokio::time::sleep(TICK).await;
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
        .filter(|r| due_at(app, r).is_some_and(|t| t <= now))
        .filter(|r| app.bot(&r.bot_id).is_some_and(|b| b.runner_id == this) && !app.is_routine_running(&r.id) && !app.routine_checks.is_running(&r.id))
        .collect();
    if due.is_empty() {
        return;
    }
    if user_away(app, now) {
        pause_while_away(app, &due);
        return;
    }
    for routine in due {
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

/// The checks of this Runner's routines: when each last ran here, and which run now. Kept in
/// memory: a check that found nothing changes nothing else, and after a restart a due check
/// runs once more.
#[cfg(feature = "runner")]
#[derive(Default)]
pub struct Checks(std::sync::Mutex<std::collections::HashMap<String, CheckState>>);

#[cfg(feature = "runner")]
#[derive(Default, Clone, Copy)]
struct CheckState {
    last_at: Option<i64>,
    running: bool,
}

#[cfg(feature = "runner")]
impl Checks {
    /// When the routine's check last ran here.
    pub fn last_at(&self, id: &str) -> Option<i64> {
        self.0.lock().unwrap().get(id).and_then(|state| state.last_at)
    }

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

    fn finish(&self, id: &str, at: i64) {
        self.0.lock().unwrap().insert(id.to_string(), CheckState { last_at: Some(at), running: false });
    }
}

/// A check of the routine ended now: the schedule counts from it, and the apps hear of the
/// next check, which no roster change tells them.
#[cfg(feature = "runner")]
fn checked(app: &App, id: &str) {
    app.routine_checks.finish(id, now_unix());
    app.emit(app.roster_summary());
}

/// When this Runner runs the routine next: its schedule from the last run, or from the last
/// check here, which counts as a run whether or not it started one.
#[cfg(feature = "runner")]
fn due_at(app: &App, routine: &Routine) -> Option<i64> {
    match app.routine_checks.last_at(&routine.id) {
        Some(at) => routine.next_run_after(at),
        None => routine.next_run_at(),
    }
}

/// When the routine runs next, as the apps and the bot say it. Only the Runner knows when a
/// routine's check last ran; elsewhere a routine with a check that is past due is due at its
/// next schedule time, since its Runner has been checking in between.
pub fn next_run_shown(app: &App, routine: &Routine) -> Option<i64> {
    #[cfg(feature = "runner")]
    if let Some(at) = app.routine_checks.last_at(&routine.id) {
        return routine.next_run_after(at);
    }
    #[cfg(not(feature = "runner"))]
    let _ = app;
    let next = routine.next_run_at()?;
    let now = now_unix();
    if routine.check.is_some() && next <= now {
        return schedule::parse(&routine.schedule).ok().and_then(|schedule| schedule.next_after(now)).or(Some(next));
    }
    Some(next)
}

/// Runs a due routine's check, then the routine when the check found something or failed. A
/// check that found nothing leaves no trace but the time it ran, which the schedule counts
/// from: no marker, no turn, and no roster change.
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
        checked(&app, &routine.id);
        // A routine paused, deleted, or given another check meanwhile does not run on this one.
        let Some(current) = app.routine(&routine.id).filter(|current| current.is_enabled && current.check == routine.check) else { return };
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
    checked(app, &routine.id);
    found
}

/// Runs a routine's check: its script in a codemode sandbox of its own, with the bot's file
/// tools that only read, the read-only tools of this Runner's plugins, the values the bot's
/// scripts keep in its direct chat, and `models.ask`. No model of the bot's runs and nobody is
/// asked anything: a call that could change something ends the check.
#[cfg(feature = "runner")]
pub async fn run_check(app: &Arc<App>, routine: &Routine, cancel: &CancellationToken) -> CheckRun {
    let failed = |error: String| CheckRun { found: None, error: Some(error.clone()), result: error };
    let Some(code) = routine.check.as_deref() else { return CheckRun { found: None, error: None, result: String::new() } };
    let Some(bot) = app.bot(&routine.bot_id) else { return failed("The routine's bot is gone.".into()) };
    let dm = match app.dm_with(&bot.id, None) {
        Ok(dm) => dm,
        Err(error) => return failed(error.to_string()),
    };
    let files: Vec<Arc<dyn Tool>> =
        lorca_agent::tools::coding_tools(bot.working_directory(&app.config.home)).into_iter().filter(|tool| CHECK_FILE_TOOLS.contains(&tool.name())).collect();
    let catalog = crate::plugins::mcp::turn_catalog(app, files);
    let store = Arc::new(crate::scripts::ScriptStore { app: app.clone(), chat_id: dm.meta.id.clone(), bot_id: bot.id.clone() });
    let functions: Vec<Arc<dyn HostFunction>> =
        crate::scripts::ModelsAsk::new(app, &dm.meta.id, &bot.provider).map(|ask| Arc::new(ask) as Arc<dyn HostFunction>).into_iter().collect();
    let options = CodemodeOptions { mcp_types: !crate::plugins::mcp::plugin_briefs(app).is_empty(), timeout: CHECK_TIMEOUT, ..CodemodeOptions::default() };
    let codemode = CodemodeTool::new(catalog.clone(), options).with_store(store).with_functions(functions);
    let runner = CheckRunner { app: app.clone(), catalog };
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

/// Runs a check's calls: the read-only ones, and no other. A refused call ends the check.
#[cfg(feature = "runner")]
struct CheckRunner {
    app: Arc<App>,
    catalog: Arc<crate::plugins::mcp::PluginCatalog>,
}

#[cfg(feature = "runner")]
#[async_trait::async_trait]
impl ToolRunner for CheckRunner {
    async fn run(&self, tool: Arc<dyn Tool>, tool_call_id: String, args: Value, cancel: CancellationToken) -> ToolOutcome {
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
                created_at: 0.0,
            });
        }
        ScratchApp(app, home)
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
            schedule: "every 1h".into(),
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

    /// A Device other than the Runner never hears of the checks that found nothing, so a
    /// routine with a check that is past due is shown due at its next schedule time.
    #[test]
    fn a_routine_with_a_check_is_shown_due_from_now() {
        let scratch = scratch_app();
        let app = &scratch.0;
        let mut routine = plain("r1", "b1", Some("return null"));
        routine.enabled_at = now_secs() - 7200.0;
        assert!(next_run_shown(app, &routine).unwrap() > now_unix());
        routine.check = None;
        assert!(next_run_shown(app, &routine).unwrap() <= now_unix(), "a routine without one shows when it was due");
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

    /// A due check that finds nothing runs no turn: no marker, no run counted, no roster
    /// change, and the next check counts from this one. One that finds something starts the run.
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
            if app.routine_checks.last_at(&quiet.id).is_some() && !app.routine_checks.is_running(&quiet.id) {
                break;
            }
            tokio::time::sleep(std::time::Duration::from_millis(25)).await;
        }
        let checked_at = app.routine_checks.last_at(&quiet.id).expect("the check ran");
        let after = app.routine(&quiet.id).unwrap();
        assert_eq!((after.last_run_at, after.last_outcome.as_deref()), (None, None), "a check is not a run");
        assert_eq!(due_at(app, &after), after.next_run_after(checked_at));
        assert!(due_at(app, &after).unwrap() > now_unix() + 500);
        let dm = app.dm_with("b1", None).unwrap();
        assert!(app.messages(&dm.meta.id).is_empty(), "no marker, no turn");
        // The local app hears the next check, which no roster change carries.
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
        app.routine_checks.finish(&watch.id, now_unix() - 7000);
        let checked = waiting.await.unwrap();
        assert_eq!(checked.found.as_deref(), Some("new"), "{}", checked.result);
        assert!(!app.routine_checks.is_running(&watch.id));
        assert!(due_at(app, &watch).unwrap() > now_unix() + 500, "the schedule counts from it");
    }

    #[cfg(feature = "runner")]
    #[test]
    fn a_routines_run_reads_its_checks_findings_as_data() {
        let found = crate::turns::check_cue(&CheckReport { found: "Two new pull requests".into(), error: None });
        assert!(found.contains("not instructions") && found.contains("Two new pull requests"), "{found}");
        let failed = crate::turns::check_cue(&CheckReport { found: String::new(), error: Some("GitHub is down".into()) });
        assert!(failed.contains("check failed") && failed.contains("GitHub is down") && failed.contains("fix the check"), "{failed}");
    }
}
