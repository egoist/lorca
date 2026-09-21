//! Routines: recurring tasks a bot runs on a schedule in its direct chat, after Grok Bot. The
//! roster carries them, so every Device lists them and can pause one; the bot's Runner runs
//! them, looking for due ones every half minute, and any Device can ask for a run now.

use std::sync::Arc;

use serde_json::Value;

use crate::app::App;
use crate::config::{now_secs, now_unix};
use crate::model::*;
use crate::runtime::{self, TurnOutcome};
use crate::schedule;

/// How often the Runner looks for due routines.
#[cfg(feature = "runner")]
const TICK: std::time::Duration = std::time::Duration::from_secs(30);

/// A week without a message from the user, and due routines are paused instead of run, so
/// they do not spend money on results nobody reads.
pub const AWAY_AFTER_SECS: i64 = 7 * 86_400;

pub const MAX_NAME_CHARS: usize = 60;
pub const MAX_PER_BOT: usize = 20;

// MARK: - Editing

/// Adds a routine for `bot_id`, checking the schedule and the name first.
pub fn create(app: &Arc<App>, bot_id: &str, name: &str, schedule_text: &str, prompt: &str, enabled: bool) -> Result<Routine, String> {
    let name = clean_name(name)?;
    let prompt = prompt.trim();
    if prompt.is_empty() {
        return Err("Say what the routine should do on each run.".into());
    }
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
        created_at: now,
    };
    app.insert_routine(routine).map_err(|e| e.to_string())
}

/// Changes a routine's name, schedule, or prompt. Only the fields given change; a new schedule
/// counts from now.
pub fn edit(app: &Arc<App>, id: &str, name: Option<&str>, schedule_text: Option<&str>, prompt: Option<&str>) -> Result<Routine, String> {
    let current = app.routine(id).ok_or("Unknown routine")?;
    let name = name.map(clean_name).transpose()?;
    if let Some(name) = &name {
        if app.routines_of(&current.bot_id).iter().any(|r| r.id != id && r.name.eq_ignore_ascii_case(name)) {
            return Err(format!("A routine named {name:?} already exists."));
        }
    }
    let schedule = schedule_text.map(schedule::parse).transpose()?;
    let prompt = prompt.map(str::trim).filter(|p| !p.is_empty()).map(str::to_string);
    if name.is_none() && schedule.is_none() && prompt.is_none() {
        return Err("Pass a new name, schedule, or prompt.".into());
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
    })
    .map_err(|e| e.to_string())
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
        requested_by: app.this_device_id().unwrap_or_default(),
        from_bot_id: None,
        hops: 0,
        round: 0,
        is_winding_down: false,
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

/// Starts every due routine of a bot on this Runner, unless the user has been away, in which
/// case the due ones are paused with a notice instead.
#[cfg(feature = "runner")]
pub fn tick(app: &Arc<App>) {
    let Some(this) = app.this_device_id() else { return };
    let now = now_unix();
    let due: Vec<Routine> = app
        .state
        .lock()
        .unwrap()
        .routines
        .iter()
        .filter(|r| r.is_enabled && r.next_run_at().is_some_and(|t| t <= now))
        .cloned()
        .collect();
    let due: Vec<Routine> = due
        .into_iter()
        .filter(|r| app.bot(&r.bot_id).is_some_and(|b| b.runner_id == this) && !app.is_routine_running(&r.id))
        .collect();
    if due.is_empty() {
        return;
    }
    if user_away(app, now) {
        pause_while_away(app, &due);
        return;
    }
    for routine in due {
        started(app, &routine.id);
        match job_for(app, &routine) {
            Ok(job) => runtime::spawn_local_job(app.clone(), job, None),
            Err(error) => tracing::warn!(%error, routine = %routine.name, "starting a routine"),
        }
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
        assert!(create(app, "b1", "Brief", "every 2m", "x", true).unwrap_err().contains("too often"));
        assert!(create(app, "b1", "", "every 1h", "x", true).unwrap_err().contains("name"));
        assert!(create(app, "b1", "Brief", "every 1h", "  ", true).unwrap_err().contains("should do"));
        let brief = create(app, "b1", "  Morning   brief ", "0 9 * * 1-5", "Summarize the inbox.", true).unwrap();
        assert_eq!(brief.name, "Morning brief");
        assert_eq!(brief.schedule, "0 9 * * 1-5");
        assert!(brief.is_enabled && brief.next_run_at().is_some());
        assert!(create(app, "b1", "morning BRIEF", "every 1h", "x", true).unwrap_err().contains("already exists"));
        assert!(create(app, "b2", "Other", "every 1h", "x", true).unwrap_err().contains("Unknown bot"));

        let edited = edit(app, &brief.id, None, Some("every 2h"), None).unwrap();
        assert_eq!(edited.schedule, "every 2h");
        assert!(edited.enabled_at >= brief.enabled_at);
        assert!(edit(app, &brief.id, None, None, None).unwrap_err().contains("Pass a new"));

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
        let hourly = create(app, "b1", "Hourly", "every 1h", "x", true).unwrap();
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
        let hourly = create(app, "b1", "Hourly", "every 1h", "x", true).unwrap();
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
        let brief = create(app, "b1", "Brief", "every 1h", "x", true).unwrap();
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
        create(app, "b1", "Hourly", "every 1h", "x", true).unwrap();
        assert!(!user_away(app, now));
        assert!(user_away(app, now + AWAY_AFTER_SECS + 60));
        let dm = app.dm_with("b1", None).unwrap();
        let mut message = Message::new(&dm.meta.id, Author::You, Body::text("hi"));
        message.created_at = (now + AWAY_AFTER_SECS) as f64;
        app.upsert_message(message, false);
        assert!(!user_away(app, now + AWAY_AFTER_SECS + 60));
    }
}
