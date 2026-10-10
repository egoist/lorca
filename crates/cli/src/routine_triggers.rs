//! What places a routine's runs besides a repeating schedule: one date and time, after which the
//! routine is done; a pull request it watches until it merges or closes; and calendar events it
//! runs a set time before or after. The Runner reads GitHub and Calendar through their plugins
//! the way a routine's check reads: under the bot's Access, with calls that only read, counted
//! in the routine's health and limits.

use serde::{Deserialize, Serialize};

use crate::config::now_unix;
use crate::model::Routine;
use crate::schedule::{EventOffset, Schedule};

#[cfg(feature = "runner")]
use std::sync::Arc;
#[cfg(feature = "runner")]
use {
    crate::app::App,
    crate::config::now_secs,
    crate::routines::CheckRun,
    serde_json::{json, Value},
    tokio_util::sync::CancellationToken,
};

/// How often a Runner reads the calendar of a routine timed by events.
pub const CALENDAR_SYNC_SECS: i64 = 15 * 60;
/// How far ahead a Runner reads a calendar, past the routine's own offset.
#[cfg(feature = "runner")]
const CALENDAR_AHEAD_SECS: i64 = 26 * 3600;
/// The most events a routine keeps from one read of its calendar.
#[cfg(feature = "runner")]
const MAX_UPCOMING: usize = 50;
/// The poll interval of a watch whose bot gave none.
pub const DEFAULT_WATCH_SCHEDULE: &str = "every 10m";

/// A pull request a routine watches through the GitHub plugin on its Runner.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct PullRequestWatch {
    /// `owner/name`.
    pub repo: String,
    pub number: u64,
    /// The plugin that reads it.
    pub plugin_id: String,
    /// What the last read found; the next read reports what changed since.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub seen: Option<PullRequestState>,
    /// When a read found it merged or closed: its last run reports that, and the routine ends.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ended_at: Option<f64>,
}

/// What a watch keeps of a pull request between reads.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
pub struct PullRequestState {
    #[serde(default)]
    pub title: String,
    /// `open` or `closed`.
    #[serde(default)]
    pub state: String,
    #[serde(default)]
    pub merged: bool,
    #[serde(default)]
    pub draft: bool,
    #[serde(default)]
    pub head_sha: String,
    #[serde(default)]
    pub commits: u64,
    #[serde(default)]
    pub comments: u64,
    #[serde(default)]
    pub review_comments: u64,
    #[serde(default)]
    pub updated_at: String,
    #[serde(default)]
    pub url: String,
}

impl PullRequestState {
    pub fn is_closed(&self) -> bool {
        self.merged || self.state.eq_ignore_ascii_case("closed")
    }
}

/// The Calendar account whose events place a routine's runs, and what its Runner last read.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct CalendarTrigger {
    /// The Calendar account's plugin id.
    pub plugin_id: String,
    /// Its name as the apps show it: "Google Calendar · Work".
    #[serde(default)]
    pub account: String,
    /// Words a matching event has in its title, description, place, or guests; every event
    /// when `None`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub matching: Option<String>,
    /// The matching events of the next day, as the last read found them.
    #[serde(default)]
    pub upcoming: Vec<CalendarEvent>,
    /// The events whose runs were taken, by `event_key`, so each runs once.
    #[serde(default)]
    pub done: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub synced_at: Option<f64>,
}

/// One event a routine runs around. Its title and link came from the calendar: data for the
/// run, never instructions.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct CalendarEvent {
    pub id: String,
    #[serde(default)]
    pub title: String,
    pub start: f64,
    pub end: f64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub url: Option<String>,
}

impl CalendarEvent {
    /// The same event moved to another time is another run.
    pub fn key(&self) -> String {
        format!("{}@{}", self.id, self.start as i64)
    }
}

/// `owner/name#42`, `owner/name/pull/42`, or the pull request's GitHub link, as `(repo, number)`.
pub fn parse_pull_request(text: &str) -> Result<(String, u64), String> {
    let problem = || format!("Could not read the pull request {text:?}. Give it as owner/repo#42 or its GitHub link.");
    let text = text.trim().trim_end_matches('/');
    let path = text.strip_prefix("https://").or_else(|| text.strip_prefix("http://")).map(|rest| rest.strip_prefix("www.").unwrap_or(rest));
    let (repo, number) = match path {
        Some(path) => {
            let path = path.strip_prefix("github.com/").ok_or_else(problem)?;
            let parts: Vec<&str> = path.split('/').collect();
            match parts.as_slice() {
                [owner, name, "pull" | "pulls", number, ..] => (format!("{owner}/{name}"), *number),
                _ => return Err(problem()),
            }
        }
        None => match text.split_once('#') {
            Some((repo, number)) => (repo.to_string(), number),
            None => {
                let parts: Vec<&str> = text.split('/').collect();
                match parts.as_slice() {
                    [owner, name, "pull" | "pulls", number] => (format!("{owner}/{name}"), *number),
                    _ => return Err(problem()),
                }
            }
        },
    };
    let number: u64 = number.parse().ok().filter(|n| *n > 0).ok_or_else(problem)?;
    let valid = |part: &str| !part.is_empty() && part.chars().all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.'));
    match repo.split_once('/') {
        Some((owner, name)) if valid(owner) && valid(name) => Ok((repo, number)),
        _ => Err(problem()),
    }
}

/// Whether a one-time routine due at `at` has run: its Runner took the time, or a run started
/// at or after it.
pub fn once_taken(routine: &Routine, at: i64) -> bool {
    let at = at as f64;
    routine.last_scheduled_at.is_some_and(|taken| taken >= at) || routine.last_run_at.is_some_and(|ran| ran >= at)
}

/// The events whose runs are still to come or are due, earliest first, with each one's due time.
/// An event due before the routine was set up or resumed is not its to run.
pub fn pending_events(routine: &Routine, offset: EventOffset) -> Vec<(i64, &CalendarEvent)> {
    let Some(calendar) = &routine.calendar else { return Vec::new() };
    let armed = routine.enabled_at as i64;
    let mut pending: Vec<(i64, &CalendarEvent)> = calendar
        .upcoming
        .iter()
        .filter(|event| !calendar.done.contains(&event.key()))
        .map(|event| (offset.due(event.start as i64, event.end as i64), event))
        .filter(|(due, _)| *due > armed)
        .collect();
    pending.sort_by_key(|(due, event)| (*due, event.start as i64));
    pending
}

/// The next run of a routine timed by events, and its event.
pub fn next_event_run(routine: &Routine, offset: EventOffset) -> Option<(i64, &CalendarEvent)> {
    pending_events(routine, offset).into_iter().next()
}

/// A one-time routine whose time was taken, or a watch whose pull request merged or closed and
/// whose last run started or was lost to a restart: the routine is done, and its Runner
/// removes it.
pub fn is_finished(routine: &Routine) -> bool {
    if let Some(watch) = &routine.pull_request {
        return watch.ended_at.is_some_and(|ended| routine.last_run_at.is_some_and(|ran| ran >= ended) || (now_unix() as f64) - ended > 600.0);
    }
    match crate::schedule::parse(&routine.schedule) {
        Ok(schedule @ Schedule::Once(_)) => schedule.once_instant(&routine.timezone).is_some_and(|at| once_taken(routine, at)),
        _ => false,
    }
}

/// Whether two copies of a routine look at the same thing: the same check, pull request, and
/// calendar events. A check, read, or sync of one copy says nothing of the other.
pub fn same_target(a: &Routine, b: &Routine) -> bool {
    let watch = |routine: &Routine| routine.pull_request.as_ref().map(|watch| (watch.repo.clone(), watch.number, watch.plugin_id.clone()));
    let calendar = |routine: &Routine| routine.calendar.as_ref().map(|calendar| (calendar.plugin_id.clone(), calendar.matching.clone()));
    a.check == b.check && watch(a) == watch(b) && calendar(a) == calendar(b) && a.schedule == b.schedule
}

/// Whether the Runner looks before it runs: a check, or a watch's read of its pull request.
pub fn looks_first(routine: &Routine) -> bool {
    routine.check.is_some() || routine.pull_request.is_some()
}

/// The routine's schedule in words: what starts it, as the apps and the bot say it.
pub fn describe(routine: &Routine) -> String {
    if let Some(watch) = &routine.pull_request {
        return format!("Watches {}#{}", watch.repo, watch.number);
    }
    match crate::schedule::parse(&routine.schedule) {
        Ok(Schedule::Events(offset)) => crate::schedule::describe_events(offset, routine.calendar.as_ref().and_then(|calendar| calendar.matching.as_deref())),
        Ok(schedule) => schedule.describe(),
        Err(_) => routine.schedule.clone(),
    }
}

/// When a routine timed by events should read its calendar again: never read, past the sync
/// interval, or after a failure's backoff.
pub fn calendar_sync_due(routine: &Routine, now: i64) -> bool {
    let Some(calendar) = &routine.calendar else { return false };
    let retry = routine.health.as_ref().and_then(|health| health.retry_at()).unwrap_or(0.0) as i64;
    let next = calendar.synced_at.map(|at| at as i64 + CALENDAR_SYNC_SECS).unwrap_or(0).max(retry);
    now >= next
}

/// One event as its run reads it: its title, when it is on the routine's clock, and its link.
pub fn event_text(routine: &Routine, event: &CalendarEvent) -> String {
    let now = now_unix();
    let start = crate::schedule::when_label(event.start as i64, now, &routine.timezone);
    let end = crate::schedule::when_label(event.end as i64, event.start as i64, &routine.timezone);
    let end = end.strip_prefix("today ").unwrap_or(&end);
    let title = if event.title.trim().is_empty() { "(no title)" } else { event.title.trim() };
    let mut text = format!("Event: {title}\nWhen: {start} to {end} ({})", routine.timezone);
    if let Some(url) = &event.url {
        text.push_str(&format!("\nLink: {url}"));
    }
    if let Some(calendar) = &routine.calendar {
        text.push_str(&format!("\nCalendar: {} (account id {}), event id {}", calendar.account, calendar.plugin_id, event.id));
    }
    text
}

/// Keeps what the Runner last read for its own routines when another Device writes the roster
/// from an older copy: a watch's last look and end, a calendar's events and the runs taken. True
/// when it kept any, so the roster goes up again with it.
pub fn keep_reads(held: &Routine, incoming: &mut Routine) -> bool {
    let mut kept = false;
    if let (Some(held), Some(watch)) = (&held.pull_request, incoming.pull_request.as_mut()) {
        if (held.repo.as_str(), held.number) == (watch.repo.as_str(), watch.number) && (held.seen != watch.seen || held.ended_at != watch.ended_at) && held.seen.is_some() {
            watch.seen = held.seen.clone();
            watch.ended_at = watch.ended_at.or(held.ended_at);
            kept = true;
        }
    }
    if let (Some(held), Some(calendar)) = (&held.calendar, incoming.calendar.as_mut()) {
        if (&held.plugin_id, &held.matching) == (&calendar.plugin_id, &calendar.matching) && held.synced_at > calendar.synced_at {
            calendar.upcoming = held.upcoming.clone();
            calendar.synced_at = held.synced_at;
            kept = true;
        }
        for key in &held.done {
            if !calendar.done.contains(key) {
                calendar.done.push(key.clone());
                kept = true;
            }
        }
    }
    kept
}

// MARK: - Reading GitHub and Calendar on the Runner

/// The plugin on this Runner that reads `repo`'s pull requests for a bot: GitHub from the
/// marketplace, else any plugin whose tools read a pull request. The bot's Access must allow it.
#[cfg(feature = "runner")]
pub fn github_plugin(app: &App, bot: &crate::model::Bot) -> Result<String, String> {
    let store = app.plugins.lock().unwrap();
    let allowed = |id: &str| bot.permissions.as_ref().is_none_or(|policy| policy.allows_connection(id));
    let installed = store.installed();
    let github = installed.iter().find(|plugin| plugin.manifest.id == "github" || plugin.service_id.as_deref() == Some("github")).or_else(|| {
        installed.iter().find(|plugin| crate::plugins::mcp::saved_tools(app, plugin).iter().any(|tool| matches!(tool["name"].as_str(), Some("pull_request_read" | "get_pull_request"))))
    });
    match github {
        Some(plugin) if allowed(&plugin.manifest.id) => Ok(plugin.manifest.id.clone()),
        Some(plugin) => Err(format!("{}'s Access leaves out {}. Ask the user to allow it in your Access first.", bot.name, plugin.display_name())),
        None => Err("GitHub is not installed on this Runner. Install it (install_plugin github) and sign in first.".into()),
    }
}

/// The Calendar account on this Runner that `name` names (its id, its name, or its account
/// label), or the only one there is.
#[cfg(feature = "runner")]
pub fn calendar_account(app: &App, bot: &crate::model::Bot, name: Option<&str>) -> Result<(String, String), String> {
    let store = app.plugins.lock().unwrap();
    let accounts: Vec<&crate::plugins::Installed> =
        store.installed().iter().filter(|plugin| plugin.manifest.id == "google-calendar" || plugin.service_id.as_deref() == Some("google-calendar")).collect();
    if accounts.is_empty() {
        return Err("Google Calendar is not connected on this Runner. Install it (install_plugin google-calendar) and sign in first.".into());
    }
    let chosen = match name.map(str::trim).filter(|name| !name.is_empty()) {
        Some(name) => accounts.iter().find(|plugin| {
            plugin.manifest.id == name || plugin.display_name().eq_ignore_ascii_case(name) || plugin.account_name.as_deref().is_some_and(|label| label.eq_ignore_ascii_case(name))
        }),
        None if accounts.len() == 1 => accounts.first(),
        None => {
            let names: Vec<String> = accounts.iter().map(|plugin| plugin.display_name()).collect();
            return Err(format!("Say which calendar: {}.", crate::schedule::join_words(&names)));
        }
    };
    let Some(plugin) = chosen else {
        let names: Vec<String> = accounts.iter().map(|plugin| plugin.display_name()).collect();
        return Err(format!("No Calendar account named {:?}. This Runner has {}.", name.unwrap_or_default(), crate::schedule::join_words(&names)));
    };
    if !bot.permissions.as_ref().is_none_or(|policy| policy.allows_connection(&plugin.manifest.id)) {
        return Err(format!("{}'s Access leaves out {}. Ask the user to allow it in your Access first.", bot.name, plugin.display_name()));
    }
    Ok((plugin.manifest.id.clone(), plugin.display_name()))
}

/// Calls one of a plugin's tools as a check would, through `runner`: the bot's Access and the
/// read-only rule apply. The first of `tools` the plugin has answers. A plugin that needs a
/// sign-in or a setup says so in words `routine_health::classify` reads.
#[cfg(feature = "runner")]
async fn call(
    app: &Arc<App>,
    catalog: &Arc<crate::plugins::mcp::PluginCatalog>,
    runner: &dyn lorca_agent::ToolRunner,
    plugin_id: &str,
    tools: &[(&str, Value)],
    cancel: &CancellationToken,
) -> Result<Value, String> {
    use lorca_agent::codemode::Catalog;
    let status = app.plugins.lock().unwrap().status(plugin_id);
    let Some(status) = status else { return Err(format!("{plugin_id} is not installed on this Runner: blocked")) };
    match status.state.as_str() {
        "needs_auth" | "insufficient_access" => return Err(format!("{} needs a sign-in: {}", status.name, status.detail)),
        "needs_setup" => return Err(format!("{} needs setup before its tools work: blocked", status.name)),
        _ => {}
    }
    for (tool, args) in tools {
        let name = crate::plugins::mcp::tool_name(plugin_id, tool);
        let Some(entry) = catalog.find(&name, cancel).await else { continue };
        let outcome = runner.run(entry.tool, format!("trigger-{}", uuid::Uuid::new_v4()), args.clone(), cancel.clone()).await;
        if outcome.is_error || outcome.result.is_error {
            return Err(outcome.result.text_content());
        }
        let text = outcome.result.text_content();
        return Ok(outcome.result.structured.unwrap_or_else(|| json!({ "content": [{ "type": "text", "text": text }] })));
    }
    let _ = app;
    Err(format!("{} has no tool to read this with ({}).", status.name, tools.iter().map(|(tool, _)| *tool).collect::<Vec<_>>().join(", ")))
}

/// The JSON a tool returned: its structured content, else the first text part that is JSON.
#[cfg(feature = "runner")]
fn returned(value: &Value) -> Option<Value> {
    if let Some(structured) = value.get("structuredContent").filter(|v| !v.is_null()) {
        return Some(structured.clone());
    }
    value["content"].as_array()?.iter().filter_map(|part| part["text"].as_str()).find_map(|text| serde_json::from_str::<Value>(text.trim()).ok())
}

/// The first object in `value`, itself included, that `is` accepts.
#[cfg(feature = "runner")]
fn find_object<'a>(value: &'a Value, is: &dyn Fn(&Value) -> bool) -> Option<&'a Value> {
    if is(value) {
        return Some(value);
    }
    match value {
        Value::Object(fields) => fields.values().find_map(|v| find_object(v, is)),
        Value::Array(items) => items.iter().find_map(|v| find_object(v, is)),
        _ => None,
    }
}

/// A pull request as GitHub's MCP server returns it (its trimmed form or the REST one).
#[cfg(feature = "runner")]
pub fn pull_request_state(value: &Value) -> Option<PullRequestState> {
    let found = returned(value)?;
    let pr = find_object(&found, &|v| v.get("number").is_some_and(Value::is_u64) && v.get("state").is_some_and(Value::is_string))?;
    let count = |key: &str| pr[key].as_u64().unwrap_or(0);
    Some(PullRequestState {
        title: pr["title"].as_str().unwrap_or_default().to_string(),
        state: pr["state"].as_str().unwrap_or_default().to_lowercase(),
        merged: pr["merged"].as_bool().unwrap_or(false) || pr["merged_at"].as_str().is_some_and(|at| !at.is_empty()),
        draft: pr["draft"].as_bool().unwrap_or(false),
        head_sha: pr["head"]["sha"].as_str().or(pr["head_sha"].as_str()).unwrap_or_default().to_string(),
        commits: count("commits"),
        comments: count("comments"),
        review_comments: count("review_comments"),
        updated_at: pr["updated_at"].as_str().unwrap_or_default().to_string(),
        url: pr["html_url"].as_str().unwrap_or_default().to_string(),
    })
}

/// What changed between two reads of a pull request, one line each; none when nothing did.
pub fn pull_request_changes(before: &PullRequestState, now: &PullRequestState) -> Vec<String> {
    let mut changes = Vec::new();
    let plural = |n: u64, one: &str, many: &str| if n == 1 { format!("1 {one}") } else { format!("{n} {many}") };
    if now.merged && !before.merged {
        changes.push("Merged.".to_string());
    } else if now.is_closed() && !before.is_closed() {
        changes.push("Closed without merging.".to_string());
    } else if !now.is_closed() && before.is_closed() {
        changes.push("Reopened.".to_string());
    }
    if before.draft && !now.draft && !now.is_closed() {
        changes.push("Marked ready for review.".to_string());
    } else if !before.draft && now.draft {
        changes.push("Turned back into a draft.".to_string());
    }
    if now.head_sha != before.head_sha && !now.head_sha.is_empty() {
        let added = now.commits.saturating_sub(before.commits);
        changes.push(if added > 0 { format!("{} pushed.", plural(added, "new commit", "new commits")) } else { "The branch was pushed again.".to_string() });
    }
    if now.comments > before.comments {
        changes.push(format!("{}.", plural(now.comments - before.comments, "new comment", "new comments")));
    }
    if now.review_comments > before.review_comments {
        changes.push(format!("{}.", plural(now.review_comments - before.review_comments, "new review comment", "new review comments")));
    }
    if now.title != before.title && !now.title.is_empty() {
        changes.push(format!("Renamed to “{}”.", now.title));
    }
    if changes.is_empty() && now.updated_at != before.updated_at && !now.updated_at.is_empty() {
        changes.push("Other activity, such as a review, a label, or a reviewer request.".to_string());
    }
    changes
}

/// Reads a watch's pull request and says what changed since the last read. The first read only
/// records it; a merged or closed one ends the watch after the run its news starts.
#[cfg(feature = "runner")]
pub async fn look_at_pull_request(
    app: &Arc<App>,
    routine: &Routine,
    catalog: &Arc<crate::plugins::mcp::PluginCatalog>,
    runner: &dyn lorca_agent::ToolRunner,
    cancel: &CancellationToken,
) -> CheckRun {
    let failed = |error: String| CheckRun { found: None, error: Some(error.clone()), result: error };
    let Some(watch) = routine.pull_request.clone() else { return failed("The routine watches no pull request.".into()) };
    let Some((owner, name)) = watch.repo.split_once('/') else { return failed(format!("{} is not a repository.", watch.repo)) };
    let tools = [
        ("pull_request_read", json!({ "method": "get", "owner": owner, "repo": name, "pullNumber": watch.number })),
        ("get_pull_request", json!({ "owner": owner, "repo": name, "pullNumber": watch.number })),
    ];
    let value = match call(app, catalog, runner, &watch.plugin_id, &tools, cancel).await {
        Ok(value) => value,
        Err(error) => return failed(error),
    };
    let Some(now) = pull_request_state(&value) else {
        return failed(format!("GitHub's answer for {}#{} did not read as a pull request.", watch.repo, watch.number));
    };
    let label = format!("{}#{}", watch.repo, watch.number);
    let link = if now.url.is_empty() { format!("https://github.com/{}/pull/{}", watch.repo, watch.number) } else { now.url.clone() };
    let changes = watch.seen.as_ref().map(|seen| pull_request_changes(seen, &now)).unwrap_or_default();
    let ended = now.is_closed();
    let first = watch.seen.is_none();
    let saved = app.record_routine(&routine.id, |current| {
        let Some(current_watch) = current.pull_request.as_mut().filter(|current_watch| (current_watch.repo.as_str(), current_watch.number) == (watch.repo.as_str(), watch.number)) else { return };
        current_watch.seen = Some(now.clone());
        if ended && current_watch.ended_at.is_none() {
            current_watch.ended_at = Some(now_secs());
        }
    });
    if let Err(error) = saved {
        return failed(format!("Could not save what the watch saw: {error}"));
    }
    let state = if now.merged { "merged" } else if now.is_closed() { "closed" } else if now.draft { "a draft" } else { "open" };
    let summary = format!("{label} “{}” is {state}: {link}", now.title);
    if first {
        // A pull request already merged or closed has nothing left to watch; the tool says so.
        return CheckRun { found: None, error: None, result: summary };
    }
    if changes.is_empty() {
        return CheckRun { found: None, error: None, result: format!("Nothing new. {summary}") };
    }
    let mut found = format!("Pull request {label} “{}”: {link}\nSince the last look:\n{}", now.title, changes.iter().map(|line| format!("- {line}")).collect::<Vec<_>>().join("\n"));
    if ended {
        found.push_str("\nThis is the watch's last run: it ends now that the pull request is closed.");
    }
    CheckRun { found: Some(found.clone()), error: None, result: found }
}

/// Events in what a Calendar tool returned: each object with a start and end time. All-day
/// events, cancelled ones, ones the user declined, and time held for nobody else (out of office,
/// focus time, a working place) are left out.
#[cfg(feature = "runner")]
pub fn calendar_events(value: &Value) -> Vec<(CalendarEvent, String)> {
    let Some(found) = returned(value) else { return Vec::new() };
    let mut items: Vec<&Value> = Vec::new();
    fn collect<'a>(value: &'a Value, items: &mut Vec<&'a Value>) {
        match value {
            Value::Object(fields) if fields.get("start").is_some_and(|start| start.is_object()) && fields.get("end").is_some() => items.push(value),
            Value::Object(fields) => fields.values().for_each(|v| collect(v, items)),
            Value::Array(values) => values.iter().for_each(|v| collect(v, items)),
            _ => {}
        }
    }
    collect(&found, &mut items);
    let time = |side: &Value| side["dateTime"].as_str().or(side["date_time"].as_str()).and_then(|text| chrono::DateTime::parse_from_rfc3339(text).ok()).map(|at| at.timestamp() as f64);
    items
        .into_iter()
        .filter_map(|item| {
            let kind = item["eventType"].as_str().or(item["event_type"].as_str()).unwrap_or("default").to_lowercase().replace('_', "");
            if matches!(kind.as_str(), "outofoffice" | "focustime" | "workinglocation" | "birthday") || item["status"].as_str() == Some("cancelled") {
                return None;
            }
            let declined = item["attendees"].as_array().is_some_and(|guests| guests.iter().any(|guest| guest["self"].as_bool() == Some(true) && guest["responseStatus"].as_str() == Some("declined")));
            if declined {
                return None;
            }
            let (start, end) = (time(&item["start"])?, time(&item["end"])?);
            let id = item["id"].as_str().filter(|id| !id.is_empty())?.to_string();
            let mut haystack = vec![item["summary"].as_str().unwrap_or_default().to_string(), item["description"].as_str().unwrap_or_default().to_string(), item["location"].as_str().unwrap_or_default().to_string()];
            for guest in item["attendees"].as_array().into_iter().flatten() {
                haystack.push(guest["email"].as_str().unwrap_or_default().to_string());
                haystack.push(guest["displayName"].as_str().unwrap_or_default().to_string());
            }
            let event = CalendarEvent { id, title: item["summary"].as_str().unwrap_or_default().to_string(), start, end: end.max(start), url: item["htmlLink"].as_str().map(str::to_string) };
            Some((event, haystack.join("\n").to_lowercase()))
        })
        .collect()
}

/// Reads a routine's Calendar account for the matching events of the next day and keeps them
/// on the routine. A read finds nothing to run by itself: its events' times start the runs.
#[cfg(feature = "runner")]
pub async fn read_calendar(
    app: &Arc<App>,
    routine: &Routine,
    catalog: &Arc<crate::plugins::mcp::PluginCatalog>,
    runner: &dyn lorca_agent::ToolRunner,
    cancel: &CancellationToken,
) -> CheckRun {
    let failed = |error: String| CheckRun { found: None, error: Some(error.clone()), result: error };
    let Some(calendar) = routine.calendar.clone() else { return failed("The routine follows no calendar.".into()) };
    let Ok(Schedule::Events(offset)) = crate::schedule::parse(&routine.schedule) else { return failed("The routine's schedule is not around events.".into()) };
    let now = now_unix();
    let reach = offset.minutes as i64 * 60;
    // An event a run comes after may have started a day before it ends.
    let from = if offset.after { now - 86_400 - reach } else { now - 3600 };
    let to = now + CALENDAR_AHEAD_SECS + if offset.after { 0 } else { reach };
    let iso = |at: i64| chrono::DateTime::from_timestamp(at, 0).map(|at| at.to_rfc3339_opts(chrono::SecondsFormat::Secs, true)).unwrap_or_default();
    let mut args = json!({ "startTime": iso(from), "endTime": iso(to), "orderBy": "startTime", "pageSize": 100 });
    if let Some(words) = &calendar.matching {
        args["fullText"] = json!(words);
    }
    let value = match call(app, catalog, runner, &calendar.plugin_id, &[("list_events", args)], cancel).await {
        Ok(value) => value,
        Err(error) => return failed(error),
    };
    let words = calendar.matching.as_deref().map(str::to_lowercase);
    let mut events: Vec<CalendarEvent> = calendar_events(&value)
        .into_iter()
        .filter(|(_, haystack)| words.as_deref().is_none_or(|words| haystack.contains(words)))
        .map(|(event, _)| event)
        .filter(|event| (event.end as i64) >= from && (event.start as i64) <= to)
        .collect();
    events.sort_by(|a, b| a.start.total_cmp(&b.start));
    events.dedup_by(|a, b| a.key() == b.key());
    events.truncate(MAX_UPCOMING);
    let count = events.len();
    let changed = calendar.upcoming != events;
    let saved = app.record_routine(&routine.id, |current| {
        let Some(current) = current.calendar.as_mut().filter(|current| (&current.plugin_id, &current.matching) == (&calendar.plugin_id, &calendar.matching)) else { return };
        // A run taken is remembered while its event is still in reach.
        current.done.retain(|key| events.iter().any(|event| &event.key() == key));
        current.upcoming = events;
        current.synced_at = Some(now_secs());
    });
    if let Err(error) = saved {
        return failed(format!("Could not save the calendar's events: {error}"));
    }
    if changed {
        app.push_roster();
    }
    let result = match count {
        0 => "No matching events in the next day.".to_string(),
        1 => "1 matching event in the next day.".to_string(),
        n => format!("{n} matching events in the next day."),
    };
    CheckRun { found: None, error: None, result }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pull_requests_read_from_their_usual_spellings() {
        assert_eq!(parse_pull_request("acme/project#42").unwrap(), ("acme/project".into(), 42));
        assert_eq!(parse_pull_request("https://github.com/acme/project/pull/42/files").unwrap(), ("acme/project".into(), 42));
        assert_eq!(parse_pull_request("github.com/acme/pro.ject/pull/7".replace("github.com", "https://github.com").as_str()).unwrap(), ("acme/pro.ject".into(), 7));
        assert_eq!(parse_pull_request("acme/project/pull/9").unwrap(), ("acme/project".into(), 9));
        assert!(parse_pull_request("acme#42").is_err());
        assert!(parse_pull_request("https://gitlab.com/acme/project/pull/42").is_err());
        assert!(parse_pull_request("acme/project#0").is_err());
        assert!(parse_pull_request("acme/pro ject#4").is_err());
    }

    fn state(state: &str, merged: bool, sha: &str, commits: u64, comments: u64, updated: &str) -> PullRequestState {
        PullRequestState { title: "Add login".into(), state: state.into(), merged, draft: false, head_sha: sha.into(), commits, comments, review_comments: 0, updated_at: updated.into(), url: String::new() }
    }

    #[test]
    fn a_watch_says_what_changed() {
        let before = state("open", false, "a", 2, 1, "t1");
        assert!(pull_request_changes(&before, &before).is_empty());
        assert_eq!(pull_request_changes(&before, &state("open", false, "b", 4, 3, "t2")), vec!["2 new commits pushed.", "2 new comments."]);
        assert_eq!(pull_request_changes(&before, &state("closed", true, "a", 2, 1, "t2")), vec!["Merged."]);
        assert_eq!(pull_request_changes(&before, &state("closed", false, "a", 2, 1, "t2")), vec!["Closed without merging."]);
        assert_eq!(pull_request_changes(&before, &state("open", false, "a", 2, 1, "t2")), vec!["Other activity, such as a review, a label, or a reviewer request."]);
        let mut draft = before.clone();
        draft.draft = true;
        assert_eq!(pull_request_changes(&draft, &before), vec!["Marked ready for review."]);
    }

    #[cfg(feature = "runner")]
    #[test]
    fn pull_requests_read_from_either_server_shape() {
        let trimmed = json!({ "content": [{ "type": "text", "text": "{\"number\":42,\"title\":\"Add login\",\"state\":\"open\",\"draft\":false,\"merged\":false,\"html_url\":\"https://github.com/acme/project/pull/42\",\"head\":{\"ref\":\"login\",\"sha\":\"abc\"},\"commits\":3,\"comments\":1,\"updated_at\":\"2026-10-10T10:00:00Z\"}" }] });
        let read = pull_request_state(&trimmed).unwrap();
        assert_eq!((read.state.as_str(), read.head_sha.as_str(), read.commits, read.comments), ("open", "abc", 3, 1));
        let rest = json!({ "structuredContent": { "pullRequest": { "number": 42, "title": "Add login", "state": "closed", "merged_at": "2026-10-10T11:00:00Z", "head": { "sha": "def" }, "review_comments": 2 } } });
        let read = pull_request_state(&rest).unwrap();
        assert!(read.merged && read.is_closed());
        assert_eq!(read.review_comments, 2);
        assert!(pull_request_state(&json!({ "content": [{ "type": "text", "text": "Not Found" }] })).is_none());
    }

    #[cfg(feature = "runner")]
    #[test]
    fn calendar_events_leave_out_what_nobody_meets_in() {
        let value = json!({ "structuredContent": { "events": [
            { "id": "a", "summary": "Acme sync", "start": { "dateTime": "2026-10-12T10:00:00-04:00" }, "end": { "dateTime": "2026-10-12T10:30:00-04:00" }, "htmlLink": "https://calendar.google.com/a", "attendees": [{ "email": "pat@acme.com" }] },
            { "id": "b", "summary": "Holiday", "start": { "date": "2026-10-12T00:00:00Z" }, "end": { "date": "2026-10-13T00:00:00Z" } },
            { "id": "c", "summary": "Gone", "status": "cancelled", "start": { "dateTime": "2026-10-12T11:00:00Z" }, "end": { "dateTime": "2026-10-12T12:00:00Z" } },
            { "id": "d", "summary": "Declined", "start": { "dateTime": "2026-10-12T11:00:00Z" }, "end": { "dateTime": "2026-10-12T12:00:00Z" }, "attendees": [{ "self": true, "responseStatus": "declined" }] },
            { "id": "e", "summary": "Focus", "eventType": "FOCUS_TIME", "start": { "dateTime": "2026-10-12T13:00:00Z" }, "end": { "dateTime": "2026-10-12T14:00:00Z" } }
        ] } });
        let events = calendar_events(&value);
        assert_eq!(events.len(), 1);
        let (event, haystack) = &events[0];
        assert_eq!((event.id.as_str(), event.start as i64, event.end as i64), ("a", 1_791_813_600, 1_791_815_400));
        assert!(haystack.contains("pat@acme.com"));
    }
}
