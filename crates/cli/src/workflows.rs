//! Guided marketplace workflows: the index's packs, and a pack's setup on one Runner. A setup
//! lives in local state and in the encrypted roster; its integrations stay the Runner's
//! installed plugins.

use std::collections::{BTreeMap, BTreeSet};
use std::sync::Arc;

use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};

use crate::app::App;
use crate::config::now_secs;
use crate::marketplace::{self, BotTemplate, Index};
use crate::model::{Author, Body, Bot, Job, JobCancel, Message, MessageState, PluginStatus, Routine};
use crate::runtime::{self, TurnOutcome};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Question {
    pub id: String,
    pub label: String,
    #[serde(default)]
    pub placeholder: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Requirement {
    pub service_id: String,
    pub name: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Specialist {
    pub id: String,
    pub template_id: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct PackRoutine {
    pub id: String,
    pub specialist_id: String,
    pub name: String,
    pub schedule: String,
    pub prompt: String,
}

/// A channel the workflow's bot listens on once the workflow is on: the account of one of its
/// connections, what it takes, and what the bot does with each message.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct PackChannel {
    pub id: String,
    pub specialist_id: String,
    /// A connection's service: `telegram` or `slack`.
    pub service_id: String,
    pub name: String,
    pub listen: crate::channels::Listen,
    pub task: String,
}

/// Optional, additive entries in the v1 index. A service a pack needs may arrive in a later
/// index; until then its setup waits on that account. A pack with channels is version 2, which
/// a build that does not know channels skips.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Pack {
    pub id: String,
    pub name: String,
    pub outcome: String,
    pub description: String,
    #[serde(default = "pack_symbol")]
    pub symbol_name: String,
    #[serde(default = "pack_version")]
    pub version: u64,
    pub questions: Vec<Question>,
    #[serde(default)]
    pub connections: Vec<Requirement>,
    pub specialists: Vec<Specialist>,
    pub routines: Vec<PackRoutine>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub channels: Vec<PackChannel>,
    pub sample_specialist: String,
    pub sample_prompt: String,
}

fn pack_version() -> u64 {
    1
}

fn pack_symbol() -> String {
    "sparkles".into()
}

impl Pack {
    pub fn parse(value: &Value, index: &Index) -> Result<Self, String> {
        let pack: Self = serde_json::from_value(value.clone()).map_err(|e| format!("Not a workflow pack: {e}"))?;
        if pack.version != if pack.channels.is_empty() { 1 } else { 2 }
            || !crate::plugins::is_id(&pack.id)
            || [&pack.name, &pack.outcome, &pack.description, &pack.sample_prompt].iter().any(|s| s.trim().is_empty())
        {
            return Err("A workflow pack needs a supported version, id, name, outcome, description and sample.".into());
        }
        if pack.specialists.is_empty() || pack.specialists.len() > 6 || pack.questions.len() > 12 || pack.connections.len() > 12 || pack.routines.len() > 20 || pack.channels.len() > 6 {
            return Err("A workflow pack exceeds its setup limits.".into());
        }
        unique_ids(pack.questions.iter().map(|q| q.id.as_str()))?;
        unique_ids(pack.specialists.iter().map(|s| s.id.as_str()))?;
        unique_ids(pack.connections.iter().map(|c| c.service_id.as_str()))?;
        unique_ids(pack.routines.iter().map(|r| r.id.as_str()))?;
        unique_ids(pack.channels.iter().map(|c| c.id.as_str()))?;
        for channel in &pack.channels {
            if !pack.specialists.iter().any(|s| s.id == channel.specialist_id)
                || !pack.connections.iter().any(|c| c.service_id == channel.service_id)
                || crate::channels::service_of(&channel.service_id).is_none()
                || channel.listen.is_empty()
                || channel.name.trim().is_empty()
                || channel.name.chars().count() > 60
                || channel.task.trim().is_empty()
            {
                return Err("A workflow channel needs a specialist, a Telegram or Slack connection, a filter, a name, and a task.".into());
            }
        }
        if pack.questions.iter().any(|q| q.label.trim().is_empty()) {
            return Err("A setup question needs a label.".into());
        }
        if pack.connections.iter().any(|c| c.name.trim().is_empty()) {
            return Err("A connection requirement needs a name.".into());
        }
        for specialist in &pack.specialists {
            if index.bot(&specialist.template_id).is_none() {
                return Err(format!("Unknown specialist template {}", specialist.template_id));
            }
        }
        if !pack.specialists.iter().any(|s| s.id == pack.sample_specialist) {
            return Err("Unknown sample specialist.".into());
        }
        for routine in &pack.routines {
            if !pack.specialists.iter().any(|s| s.id == routine.specialist_id) {
                return Err("Unknown routine specialist.".into());
            }
            if routine.name.trim().is_empty() || routine.name.chars().count() > crate::routines::MAX_NAME_CHARS || routine.prompt.trim().is_empty() {
                return Err("A workflow routine needs a name and prompt.".into());
            }
            crate::schedule::parse_repeating(&routine.schedule)?;
        }
        Ok(pack)
    }
}

fn unique_ids<'a>(ids: impl Iterator<Item = &'a str>) -> Result<(), String> {
    let mut seen = BTreeSet::new();
    for id in ids {
        if !crate::plugins::is_id(id) || !seen.insert(id) {
            return Err(format!("Invalid or repeated workflow id {id:?}"));
        }
    }
    Ok(())
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Sample {
    pub job_id: String,
    pub chat_id: String,
    pub bot_id: String,
    pub started_at: f64,
    /// running, ready, failed, reviewed. A cancelled generation is never reviewed.
    pub state: String,
    #[serde(default)]
    pub message_ids: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Setup {
    pub id: String,
    pub runner_id: String,
    /// When this Device last changed it; the newer copy wins a merge.
    #[serde(default)]
    pub updated_at: f64,
    /// The chosen pack and profiles are pinned so a marketplace update cannot change a
    /// partially completed setup's questions, instructions or schedules.
    pub pack: Pack,
    pub templates: BTreeMap<String, BotTemplate>,
    pub answers: BTreeMap<String, String>,
    pub bot_ids: BTreeMap<String, String>,
    pub routine_ids: BTreeMap<String, String>,
    pub owned_routine_ids: Vec<String>,
    /// References to the Runner's installed plugins, by service.
    pub connection_ids: BTreeMap<String, String>,
    /// questions, connections, sample, reviewed, enabled, cancelled.
    pub phase: String,
    pub sample: Option<Sample>,
    /// The channels the setup made on its Runner, by the pack's channel id.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub channel_ids: BTreeMap<String, String>,
}

/// The kind of a setup's sample Job.
pub const SAMPLE_JOB: &str = "workflow_sample";

fn stable_id(prefix: &str, parts: &[&str]) -> String {
    let hash = Sha256::digest(serde_json::to_vec(parts).unwrap());
    format!("{prefix}-{}", hash[..16].iter().map(|b| format!("{b:02x}")).collect::<String>())
}

fn get(app: &App, id: &str) -> Result<Setup, String> {
    app.state.lock().unwrap().workflows.iter().find(|w| w.id == id).cloned().ok_or_else(|| "Unknown workflow setup.".into())
}

/// Strict persistence: report a failed database write, leaving the previous record available
/// for a retry. The roster carries the change to the other Devices.
fn save(app: &App, setup: &mut Setup) -> Result<(), String> {
    persist(app, setup, None)
}

/// With `sample_job`, writes only while that sample is still the setup's running one, so a late
/// outcome cannot revive a cancelled or superseded generation.
fn persist(app: &App, setup: &mut Setup, sample_job: Option<&str>) -> Result<(), String> {
    let mut state = app.state.lock().unwrap();
    let held = state.workflows.iter().position(|w| w.id == setup.id);
    if let Some(job_id) = sample_job {
        let Some(current) = held.map(|i| &state.workflows[i]) else { return Ok(()) };
        if current.phase == "cancelled" || current.sample.as_ref().is_none_or(|s| s.job_id != job_id || s.state != "running") {
            return Ok(());
        }
    }
    let previous = state.workflows.clone();
    setup.updated_at = now_secs().max(held.map_or(0.0, |i| previous[i].updated_at + 0.000001));
    match held {
        Some(i) => state.workflows[i] = setup.clone(),
        None => state.workflows.push(setup.clone()),
    }
    if let Err(error) = app.store.save_state(&state) {
        state.workflows = previous;
        return Err(format!("Could not save workflow progress: {error}"));
    }
    drop(state);
    app.push_roster();
    app.emit(app.roster_summary());
    Ok(())
}

/// Keeps setups a roster leaves out, as one from a build that does not know workflows does;
/// cancellations remain records, so sync cannot resurrect a cancelled setup. Each setup merges
/// on its own, the newer copy winning. True when this Device's roster should go out again.
pub fn merge(current: &mut Vec<Setup>, incoming: Option<Vec<Setup>>) -> bool {
    let Some(incoming) = incoming else {
        return !current.is_empty();
    };
    let mut republish = false;
    for setup in &incoming {
        match current.iter_mut().find(|held| held.id == setup.id) {
            Some(held) if setup.updated_at > held.updated_at => *held = setup.clone(),
            Some(held) if setup.updated_at < held.updated_at => republish = true,
            Some(_) => {}
            None => current.push(setup.clone()),
        }
    }
    republish || current.iter().any(|held| !incoming.iter().any(|s| s.id == held.id))
}

fn all(app: &App) -> Vec<Setup> {
    app.state.lock().unwrap().workflows.clone()
}

fn str_param<'a>(params: &'a Value, key: &str) -> Result<&'a str, String> {
    params[key].as_str().filter(|s| !s.is_empty()).ok_or_else(|| format!("Missing {key}."))
}

/// Whether the setup's sample is still going: a turn here, or on another Runner within its
/// five-minute job-result window.
fn is_running(app: &App, setup: &Setup) -> bool {
    setup.sample.as_ref().is_some_and(|s| {
        s.state == "running"
            && (app.running_turns().iter().any(|turn| turn["job_id"].as_str() == Some(&s.job_id))
                || (setup.runner_id != app.this_device_id().unwrap_or_default() && now_secs() - s.started_at < 300.0))
    })
}

fn guard_edit(app: &App, setup: &Setup) -> Result<(), String> {
    if is_running(app, setup) {
        return Err("The sample is still running.".into());
    }
    if setup.phase == "enabled" {
        return Err("Turn the workflow off before changing it.".into());
    }
    Ok(())
}

/// Pauses the routines this setup added; a reused routine of the user's keeps its state.
fn pause_owned(app: &Arc<App>, setup: &Setup) -> Result<(), String> {
    for id in &setup.owned_routine_ids {
        if app.routine(id).is_some() {
            crate::routines::set_enabled(app, id, false)?;
        }
    }
    Ok(())
}

pub async fn handle(app: &Arc<App>, method: &str, params: &Value) -> Result<Value, String> {
    // One materialization/install per local CLI; stable IDs also recover interrupted steps.
    let _editing = app.workflow_editing.lock().await;
    if method == "workflows.start" {
        let runner_id = str_param(params, "runner_id")?;
        let runner = app.device(runner_id).filter(|d| d.is_runner()).ok_or("Choose a Runner for this workflow.")?;
        let pack_id = str_param(params, "pack_id")?;
        let id = stable_id("workflow", &[pack_id, runner_id]);
        if let Ok(mut setup) = get(app, &id) {
            if setup.phase == "cancelled" {
                setup.phase = if setup.bot_ids.is_empty() { "questions" } else { "connections" }.into();
                save(app, &mut setup)?;
            }
            return view(app, &setup);
        }
        let index = marketplace::index(app).await;
        let pack = index.pack(pack_id).cloned().ok_or("This workflow is no longer in the marketplace.")?;
        let templates = pack.specialists.iter().map(|s| (s.id.clone(), index.bot(&s.template_id).unwrap().clone())).collect();
        let mut setup = Setup {
            id,
            runner_id: runner.id,
            updated_at: 0.0,
            pack,
            templates,
            answers: BTreeMap::new(),
            bot_ids: BTreeMap::new(),
            routine_ids: BTreeMap::new(),
            owned_routine_ids: Vec::new(),
            connection_ids: BTreeMap::new(),
            phase: "questions".into(),
            sample: None,
            channel_ids: BTreeMap::new(),
        };
        save(app, &mut setup)?;
        return view(app, &setup);
    }
    let id = str_param(params, "id")?;
    let mut setup = get(app, id)?;
    let mut installed_status: Option<Value> = None;
    match method {
        "workflows.get" => {}
        "workflows.configure" => {
            guard_edit(app, &setup)?;
            let answers: BTreeMap<String, String> =
                serde_json::from_value(params["answers"].clone()).map_err(|_| "Pass the workflow's answers as text fields.")?;
            validate_answers(&setup.pack, &answers)?;
            let selected: BTreeMap<String, String> = serde_json::from_value(params.get("bot_ids").cloned().unwrap_or_else(|| json!({})))
                .map_err(|_| "Pass selected bot IDs by specialist.")?;
            materialize(app, &mut setup, answers, selected)?;
            save(app, &mut setup)?;
        }
        "workflows.connection" => {
            guard_edit(app, &setup)?;
            let service_id = str_param(params, "service_id")?;
            let requirement = setup.pack.connections.iter().find(|c| c.service_id == service_id).cloned().ok_or("This workflow does not need that integration.")?;
            let choices = choices(app, &setup.runner_id, service_id);
            let plugin_id = if let Some(id) = params["plugin_id"].as_str() {
                if !choices.iter().any(|p| p.id == id) {
                    return Err(format!("Choose a {} account on this Runner.", requirement.name));
                }
                id.to_string()
            } else if let Some(id) = setup.connection_ids.get(service_id) {
                // A lost installation response is recovered by its recorded instance, even while
                // the Runner's advertisement of it is still on its way.
                id.clone()
            } else {
                // Never pick one of several named accounts for the user.
                if !choices.is_empty() {
                    return Err(format!("Choose one of the {} accounts on this Runner.", requirement.name));
                }
                let index = marketplace::index(app).await;
                let manifest = index.plugin(service_id).cloned().ok_or_else(|| format!("{} isn't in the marketplace yet.", requirement.name))?;
                let account_name = params["account_name"].as_str().filter(|name| !name.trim().is_empty()).unwrap_or(&setup.pack.name);
                let status = crate::plugins::on_runner(app, &setup.runner_id, "plugins.install", json!({ "manifest": manifest, "source": "marketplace", "account_name": account_name })).await?;
                let id = str_param(&status, "id")?.to_string();
                installed_status = Some(status);
                id
            };
            if setup.connection_ids.get(service_id) != Some(&plugin_id) {
                pause_owned(app, &setup)?;
                setup.sample = None;
                if !setup.bot_ids.is_empty() {
                    setup.phase = "connections".into();
                }
            }
            setup.connection_ids.insert(service_id.into(), plugin_id);
            save(app, &mut setup)?;
        }
        "workflows.sample" => {
            if setup.phase == "cancelled" {
                return Err("Resume this workflow before running a sample.".into());
            }
            if is_running(app, &setup) {
                return view(app, &setup);
            }
            ready(app, &setup)?;
            pause_owned(app, &setup)?;
            let bot_id = setup.bot_ids.get(&setup.pack.sample_specialist).ok_or("Set up the sample specialist first.")?.clone();
            let dm = app.dm_with(&bot_id, None).map_err(|e| e.to_string())?;
            let message = Message::new(&dm.meta.id, Author::You, Body::text(format!("Run a sample of {} for me to review. {}\nKeep its schedules paused. Present a draft in this chat; ask no question about enabling schedules.", setup.pack.name, setup.pack.sample_prompt)));
            let job = Job {
                id: format!("job-{}", uuid::Uuid::new_v4()),
                chat_id: dm.meta.id.clone(),
                bot_id: bot_id.clone(),
                kind: SAMPLE_JOB.into(),
                trigger_message_id: message.id.clone(),
                routine_id: None,
                check: None,
                requested_by: app.this_device_id().unwrap_or_default(),
                from_bot_id: None,
                hops: 0,
                round: 0,
                is_winding_down: false,
                setup: None,
                task_id: None,
                handoff: None,
                task_context: None,
                created_at: now_secs(),
            };
            setup.sample = Some(Sample { job_id: job.id.clone(), chat_id: dm.meta.id, bot_id, started_at: job.created_at, state: "running".into(), message_ids: Vec::new() });
            setup.phase = "sample".into();
            save(app, &mut setup)?;
            app.upsert_message(message, true);
            runtime::start_turn(app, job);
        }
        "workflows.review" => {
            if setup.phase == "cancelled" {
                return Err("Resume setup and run a sample first.".into());
            }
            ready(app, &setup)?;
            let job_id = str_param(params, "job_id")?;
            let sample = setup.sample.as_mut().ok_or("Run a sample first.")?;
            if sample.job_id != job_id || !matches!(sample.state.as_str(), "ready" | "reviewed") || sample.message_ids.is_empty() {
                return Err("Review the completed result of the current sample first.".into());
            }
            if sample.message_ids.iter().any(|id| app.message(&sample.chat_id, id).is_none()) {
                return Err("The sample's result hasn't reached this Device yet. Try again in a moment.".into());
            }
            sample.state = "reviewed".into();
            setup.phase = "reviewed".into();
            save(app, &mut setup)?;
        }
        "workflows.enable" => {
            ready(app, &setup)?;
            if setup.phase == "cancelled" || setup.sample.as_ref().is_none_or(|s| s.state != "reviewed") {
                return Err("Run and review a sample before turning on its schedule.".into());
            }
            for id in setup.routine_ids.values() {
                crate::routines::set_enabled(app, id, true)?;
            }
            turn_on_channels(app, &mut setup).await?;
            setup.phase = "enabled".into();
            save(app, &mut setup)?;
        }
        "workflows.cancel" => {
            if let Some(sample) = &setup.sample {
                app.cancel_job(&sample.job_id);
                if setup.runner_id != app.this_device_id().unwrap_or_default() {
                    if let Some(runner) = app.device(&setup.runner_id) {
                        let ciphertext = crate::crypto::seal_json(&runner.box_pubkey, &JobCancel { job_id: sample.job_id.clone() }).map_err(|e| e.to_string())?;
                        app.push_blob("job_cancel", Some(runner.id), ciphertext);
                    }
                }
            }
            pause_owned(app, &setup)?;
            pause_channels(app, &setup).await;
            setup.phase = "cancelled".into();
            setup.sample = None;
            save(app, &mut setup)?;
        }
        _ => return Err(format!("Unknown workflow method {method}")),
    }
    let mut out = view(app, &setup)?;
    if let Some(status) = installed_status {
        // The Runner's answer, until its advertisement of the new plugin reaches this Device.
        if let Some(connection) = out["connections"].as_array_mut().and_then(|rows| rows.iter_mut().find(|c| c["selected_id"] == status["id"])) {
            let choices = connection["choices"].as_array_mut().unwrap();
            if !choices.iter().any(|choice| choice["id"] == status["id"]) {
                choices.push(status);
            }
        }
    }
    Ok(out)
}

/// Makes or updates the workflow's channels on its Runner and turns them on: the one recorded
/// when it is still there with the same bot and account, else the Runner's channel of that bot,
/// account, and name (a lost reply), else a new one. One whose account changed is replaced.
async fn turn_on_channels(app: &Arc<App>, setup: &mut Setup) -> Result<(), String> {
    if setup.pack.channels.is_empty() {
        return Ok(());
    }
    // The Runner's own list, not what it last advertised, so turning the workflow on again
    // finds the channel it made even before that reached this Device.
    let listed = crate::event_triggers::dispatch(app, "events.list", json!({ "runner_id": setup.runner_id })).await?;
    let channels: Vec<(String, crate::event_triggers::SubscriptionConfig)> = listed["subscriptions"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|s| Some((s["id"].as_str()?.to_string(), serde_json::from_value(s["config"].clone()).ok()?)))
        .filter(|(_, config): &(String, crate::event_triggers::SubscriptionConfig)| config.is_channel())
        .collect();
    for spec in setup.pack.channels.clone() {
        let bot_id = setup.bot_ids.get(&spec.specialist_id).cloned().ok_or("Set up the workflow's bot first.")?;
        let account_id = setup.connection_ids.get(&spec.service_id).cloned().ok_or("Choose the workflow's account first.")?;
        let same = |config: &crate::event_triggers::SubscriptionConfig| {
            config.bot_id == bot_id && config.channel.as_ref().is_some_and(|c| c.account_id == account_id)
        };
        let recorded = setup.channel_ids.get(&spec.id).and_then(|id| channels.iter().find(|(c, _)| c == id));
        let existing = recorded
            .filter(|(_, config)| same(config))
            .or_else(|| channels.iter().find(|(_, config)| same(config) && config.name == spec.name));
        if let Some((stale, _)) = recorded.filter(|(id, _)| existing.is_none_or(|(e, _)| e != id)) {
            let _ = crate::event_triggers::dispatch(app, "events.delete", json!({ "runner_id": setup.runner_id, "id": stale })).await;
        }
        // Chats the user narrowed the channel to stay narrowed.
        let chats = existing.and_then(|(_, config)| config.channel.as_ref()).map(|c| c.chats.clone()).unwrap_or_default();
        let channel = crate::channels::ChannelSpec { account_id: account_id.clone(), chats, listen: spec.listen.clone().normalized() };
        let config = crate::channels::config_for(&bot_id, &spec.service_id, &spec.name, &spec.task, channel);
        let id = match existing {
            Some((id, _)) => {
                crate::event_triggers::dispatch(app, "events.update", json!({ "runner_id": setup.runner_id, "id": id, "config": config })).await?;
                id.clone()
            }
            None => {
                let created = crate::event_triggers::dispatch(app, "events.create", json!({ "runner_id": setup.runner_id, "config": config })).await?;
                str_param(&created, "id")?.to_string()
            }
        };
        setup.channel_ids.insert(spec.id.clone(), id);
    }
    Ok(())
}

/// Turning a workflow off pauses its channels; their conversations stay.
async fn pause_channels(app: &Arc<App>, setup: &Setup) {
    for id in setup.channel_ids.values() {
        if let Err(error) = crate::event_triggers::dispatch(app, "events.pause", json!({ "runner_id": setup.runner_id, "id": id })).await {
            tracing::warn!(%error, "pausing a workflow's channel");
        }
    }
}

fn validate_answers(pack: &Pack, answers: &BTreeMap<String, String>) -> Result<(), String> {
    if answers.keys().any(|id| !pack.questions.iter().any(|q| &q.id == id)) {
        return Err("Pass only answers this workflow asks for.".into());
    }
    for question in &pack.questions {
        let answer = answers.get(&question.id).map(String::as_str).unwrap_or("").trim();
        if answer.is_empty() {
            return Err(format!("Fill in {}.", question.label));
        }
        if answer.chars().count() > 2000 {
            return Err(format!("Keep {} under 2,000 characters.", question.label));
        }
        if crate::memory::scrub(answer) != answer {
            return Err("Keep keys and passwords in the integration's own sign-in.".into());
        }
    }
    Ok(())
}

/// The bot setup uses for a specialist: the one picked, else the one it recorded, else the one it
/// added before, else a bot on the Runner with the template's description.
fn planned_bot(app: &App, setup: &Setup, role: &str, picked: Option<&String>) -> Option<Bot> {
    let on_runner = |id: &String| app.bot(id).filter(|b| b.runner_id == setup.runner_id);
    picked
        .or_else(|| setup.bot_ids.get(role))
        .and_then(on_runner)
        .or_else(|| on_runner(&stable_id("bot-workflow", &[&setup.id, role])))
        .or_else(|| {
            let description = &setup.templates[role].description;
            app.state.lock().unwrap().bots.iter().find(|b| b.runner_id == setup.runner_id && &b.description == description).cloned()
        })
}

fn materialize(app: &Arc<App>, setup: &mut Setup, answers: BTreeMap<String, String>, selected: BTreeMap<String, String>) -> Result<(), String> {
    if selected.keys().any(|role| !setup.templates.contains_key(role)) {
        return Err("Unknown workflow specialist.".into());
    }
    // Validate every explicit choice before creating resources.
    if selected.values().any(|id| app.bot(id).is_none_or(|b| b.runner_id != setup.runner_id)) {
        return Err("Choose a bot on this workflow's Runner.".into());
    }
    let answers: BTreeMap<String, String> = answers.into_iter().map(|(k, v)| (k, v.trim().to_string())).collect();
    if setup.answers != answers || selected.iter().any(|(role, id)| setup.bot_ids.get(role) != Some(id)) {
        pause_owned(app, setup)?;
        setup.sample = None;
    }
    setup.answers = answers;
    for specialist in &setup.pack.specialists {
        let template = &setup.templates[&specialist.id];
        let stable = stable_id("bot-workflow", &[&setup.id, &specialist.id]);
        let bot = if let Some(bot) = planned_bot(app, setup, &specialist.id, selected.get(&specialist.id)) {
            bot
        } else {
            if app.bot(&stable).is_some() {
                return Err("The bot this workflow added moved to another Runner. Choose another bot.".into());
            }
            let provider = app.credentials.lock().unwrap().statuses().iter().find(|p| p.is_connected).map(|p| p.kind.clone()).unwrap_or_else(|| "deepseek".into());
            let bot = Bot {
                id: stable.clone(),
                name: template.name.clone(),
                description: template.description.clone(),
                symbol_name: template.symbol_name.clone(),
                accent: template.accent.clone(),
                avatar: None,
                runner_id: setup.runner_id.clone(),
                provider,
                model: None,
                thinking: None,
                legacy_instructions: String::new(),
                workdir: None,
                permissions: None,
                created_at: 0.0,
            };
            app.create_bot_with_dm(bot, Some(stable_id("chat-workflow", &[&stable]))).map_err(|e| e.to_string())?.0
        };
        setup.bot_ids.insert(specialist.id.clone(), bot.id);
    }
    for spec in &setup.pack.routines {
        let bot_id = &setup.bot_ids[&spec.specialist_id];
        let stable = stable_id("routine-workflow", &[&setup.id, &spec.id, bot_id]);
        let schedule = crate::schedule::parse(&spec.schedule)?.canonical();
        let held = setup.routine_ids.get(&spec.id).and_then(|id| app.routine(id)).filter(|r| &r.bot_id == bot_id && r.schedule == schedule && r.prompt == spec.prompt);
        let suitable = app.routines_of(bot_id).into_iter().find(|r| r.name == spec.name && r.schedule == schedule && r.prompt == spec.prompt);
        let recovered = match app.routine(&stable) {
            Some(routine) if routine.prompt != spec.prompt || routine.schedule != schedule || &routine.bot_id != bot_id => {
                setup.sample = None;
                Some(
                    app.update_routine(&stable, |r| {
                        r.bot_id = bot_id.clone();
                        r.prompt = spec.prompt.clone();
                        r.schedule = schedule.clone();
                        r.is_enabled = false;
                    })
                    .map_err(|e| e.to_string())?,
                )
            }
            other => other,
        };
        let routine = if let Some(routine) = held.or(recovered).or(suitable) {
            routine
        } else {
            if app.routines_of(bot_id).len() >= crate::routines::MAX_PER_BOT {
                return Err("The chosen bot has no room for another routine. Choose another bot.".into());
            }
            let now = now_secs();
            let routine = Routine {
                id: stable.clone(),
                bot_id: bot_id.clone(),
                name: spec.name.clone(),
                prompt: spec.prompt.clone(),
                feedback_authorization_prompt: None,
                schedule,
                timezone: crate::schedule::local_timezone(),
                missed_run_policy: Default::default(),
                last_scheduled_at: None,
                health: None,
                is_enabled: false,
                enabled_at: now,
                last_run_at: None,
                last_outcome: None,
                paused_reason: None,
                check: None,
                pull_request: None,
                calendar: None,
                created_at: now,
            };
            app.insert_routine(routine).map_err(|e| e.to_string())?
        };
        // Also recovers a crash between the deterministic insert and the setup's own write.
        if routine.id == stable && !setup.owned_routine_ids.contains(&stable) {
            setup.owned_routine_ids.push(stable);
        }
        setup.routine_ids.insert(spec.id.clone(), routine.id);
    }
    // The one account the Runner has for a service is the one to use; of several, the user picks.
    for requirement in &setup.pack.connections {
        if !setup.connection_ids.contains_key(&requirement.service_id) {
            if let [only] = choices(app, &setup.runner_id, &requirement.service_id).as_slice() {
                setup.connection_ids.insert(requirement.service_id.clone(), only.id.clone());
            }
        }
    }
    setup.phase = "connections".into();
    Ok(())
}

/// The Runner's installed plugins for a service: its named accounts, or the singleton plugin of
/// that id. Servers from its `mcp.json` stay out.
fn choices(app: &App, runner_id: &str, service_id: &str) -> Vec<PluginStatus> {
    app.device(runner_id)
        .map(|d| d.plugins.into_iter().filter(|p| p.source.is_none() && p.service_id.as_deref().unwrap_or(&p.id) == service_id).collect())
        .unwrap_or_default()
}

fn ready(app: &App, setup: &Setup) -> Result<(), String> {
    validate_answers(&setup.pack, &setup.answers)?;
    for specialist in &setup.pack.specialists {
        let bot = setup.bot_ids.get(&specialist.id).and_then(|id| app.bot(id)).filter(|b| b.runner_id == setup.runner_id).ok_or("A bot of this workflow was removed or moved. Run the sample again to replace it.")?;
        let credentials = app.credentials.lock().unwrap();
        if !credentials.statuses().iter().any(|p| p.kind == bot.provider && p.is_connected) {
            return Err(format!("Connect {} in Settings for {} first.", credentials.label(&bot.provider), bot.name));
        }
    }
    for spec in &setup.pack.routines {
        let routine = setup.routine_ids.get(&spec.id).and_then(|id| app.routine(id)).ok_or("A routine of this workflow was removed. Run the sample again to restore it.")?;
        if setup.bot_ids.get(&spec.specialist_id) != Some(&routine.bot_id) || routine.prompt != spec.prompt || routine.schedule != crate::schedule::parse(&spec.schedule)?.canonical() {
            return Err("A routine of this workflow changed. Run the sample again.".into());
        }
    }
    for requirement in &setup.pack.connections {
        let id = setup.connection_ids.get(&requirement.service_id).ok_or_else(|| format!("Choose a {} account.", requirement.name))?;
        if !choices(app, &setup.runner_id, &requirement.service_id).iter().any(|p| &p.id == id && p.state == "ready") {
            return Err(format!("Finish connecting {}.", requirement.name));
        }
    }
    Ok(())
}

fn view(app: &App, setup: &Setup) -> Result<Value, String> {
    let index = marketplace::current(app);
    let connections: Vec<_> = setup
        .pack
        .connections
        .iter()
        .map(|r| json!({ "service_id": r.service_id, "name": r.name, "selected_id": setup.connection_ids.get(&r.service_id), "choices": choices(app, &setup.runner_id, &r.service_id), "available": index.plugin(&r.service_id).is_some() }))
        .collect();
    let candidates: Vec<Bot> = app.state.lock().unwrap().bots.iter().filter(|b| b.runner_id == setup.runner_id).cloned().collect();
    let specialists: Vec<_> = setup
        .pack
        .specialists
        .iter()
        .map(|s| json!({ "id": s.id, "name": setup.templates[&s.id].name, "selected_id": planned_bot(app, setup, &s.id, None).map(|b| b.id), "choices": candidates }))
        .collect();
    let routines: Vec<_> = setup.routine_ids.values().filter_map(|id| app.routine(id)).map(|r| app.routine_out(&r)).collect();
    // What the workflow listens to, and once it is on, the Runner's channel.
    let advertised = app.device(&setup.runner_id).map(|runner| runner.channels).unwrap_or_default();
    let channels: Vec<_> = setup
        .pack
        .channels
        .iter()
        .map(|c| json!({ "id": c.id, "name": c.name, "service_id": c.service_id, "listen": c.listen, "channel": setup.channel_ids.get(&c.id).and_then(|id| advertised.iter().find(|s| &s.id == id)) }))
        .collect();
    let sample_messages: Vec<_> =
        setup.sample.as_ref().into_iter().flat_map(|s| s.message_ids.iter().filter_map(|id| app.message(&s.chat_id, id))).map(|m| m.for_app()).collect();
    Ok(json!({ "setup": setup, "connections": connections, "specialists": specialists, "routines": routines, "channels": channels, "sample_messages": sample_messages, "is_running": is_running(app, setup) }))
}

/// Sets the result boundary only after obtaining the chat lock, so replies from a preceding
/// queued turn cannot become part of this sample's result.
pub fn sample_started(app: &App, job: &Job) {
    if job.kind != SAMPLE_JOB {
        return;
    }
    if let Some(mut setup) = all(app).into_iter().find(|s| s.sample.as_ref().is_some_and(|sample| sample.job_id == job.id)) {
        setup.sample.as_mut().unwrap().started_at = now_secs();
        if let Err(error) = persist(app, &mut setup, Some(&job.id)) {
            tracing::error!(%error, "saving workflow sample boundary");
        }
    }
}

/// Called on the executing Runner while the chat's turn lock is still held. Reads the current
/// generation before recording a result; a cancelled or retried generation cannot activate it.
pub fn sample_finished(app: &App, job: &Job, outcome: TurnOutcome) {
    if job.kind != SAMPLE_JOB {
        return;
    }
    let Some(mut setup) = all(app).into_iter().find(|s| s.phase != "cancelled" && s.sample.as_ref().is_some_and(|sample| sample.job_id == job.id && sample.state == "running")) else {
        return;
    };
    let sample = setup.sample.as_mut().unwrap();
    let messages = app.store.page(&job.chat_id, None, 100).map(|p| p.0).unwrap_or_default();
    sample.message_ids = messages
        .iter()
        .filter(|m| {
            m.created_at >= sample.started_at
                && m.author == Author::Bot { bot_id: job.bot_id.clone() }
                && matches!(&m.body, Body::Text { text, .. } if !text.trim().is_empty())
                && m.state == MessageState::Complete
        })
        .map(|m| m.id.clone())
        .collect();
    sample.state = if outcome == TurnOutcome::Sent && !sample.message_ids.is_empty() { "ready" } else { "failed" }.into();
    if let Err(error) = persist(app, &mut setup, Some(&job.id)) {
        tracing::error!(%error, "saving workflow sample outcome");
    }
}

/// Enforces the review gate for routines a workflow added, even if a bot or another UI tries to
/// resume them during setup. The user's own routines keep their own controls.
pub fn allow_enable(app: &App, routine_id: &str) -> Result<(), String> {
    let mut owned = false;
    for setup in all(app) {
        if setup.owned_routine_ids.iter().any(|id| id == routine_id) {
            owned = true;
            if setup.phase == "cancelled" || setup.sample.as_ref().is_none_or(|s| s.state != "reviewed") {
                return Err("Review this workflow's sample before turning on its routine.".into());
            }
            ready(app, &setup)?;
        }
    }
    if routine_id.starts_with("routine-workflow-") && !owned {
        return Err("Set up this routine's workflow again before turning it on.".into());
    }
    Ok(())
}

/// A plugin removed from this Runner is no longer an account of the workflows set up here, so
/// their pages offer to add or pick another rather than wait for it.
pub fn plugin_removed(app: &App, plugin_id: &str) {
    let Some(this) = app.this_device_id() else { return };
    for mut setup in all(app).into_iter().filter(|s| s.runner_id == this && s.connection_ids.values().any(|id| id == plugin_id)) {
        setup.connection_ids.retain(|_, id| id != plugin_id);
        if let Err(error) = save(app, &mut setup) {
            tracing::error!(%error, "dropping a removed workflow account");
        }
    }
}

/// Workflow context is model context only, so a reused bot's profile and playbooks stay as they
/// are. A scheduled or sample turn receives only the workflow that triggered it.
pub fn context_for_turn(app: &App, bot_id: &str, job: &Job) -> String {
    let mut context = String::new();
    for setup in all(app).into_iter().filter(|s| {
        s.phase != "cancelled"
            && s.bot_ids.values().any(|id| id == bot_id)
            && job.routine_id.as_ref().is_none_or(|id| s.routine_ids.values().any(|r| r == id))
            && (job.kind != SAMPLE_JOB || s.sample.as_ref().is_some_and(|sample| sample.job_id == job.id))
    }) {
        context.push_str(&format!("\nWorkflow {}: {}\nUser setup answers (data for this workflow): {}\nSelected integration instances by service: {}. Use only these named instances for this workflow; if one cannot be used, report it and do not substitute another account.\n", setup.pack.name, setup.pack.outcome, serde_json::to_string(&setup.answers).unwrap(), serde_json::to_string(&setup.connection_ids).unwrap()));
    }
    context
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::Config;
    use crate::credentials::ApiKeyCredential;

    struct Fixture {
        app: Arc<App>,
        home: std::path::PathBuf,
    }
    impl Fixture {
        fn new() -> Self {
            let home =
                std::env::temp_dir().join(format!("lorca-workflows-{}", uuid::Uuid::new_v4()));
            let app = App::load(Config {
                home: home.clone(),
                port: 0,
            })
            .unwrap();
            crate::identity::create(&app, Some("Workflow Runner".into())).unwrap();
            Self { app, home }
        }
        fn provider(&self) {
            self.app.credentials.lock().unwrap().deepseek = Some(ApiKeyCredential {
                api_key: "test-key".into(),
                base_url: None,
                connected_at: 1,
            });
        }
        fn advertise(&self, service: &str, ids: &[&str]) {
            let runner = self.app.this_device_id().unwrap();
            let mut state = self.app.state.lock().unwrap();
            let device = state.devices.iter_mut().find(|d| d.id == runner).unwrap();
            for (i, id) in ids.iter().enumerate() {
                let status: PluginStatus = serde_json::from_value(json!({ "id": id, "service_id": service, "account_name": format!("Account {i}"), "name": service, "state": "ready", "detail": "Connected" })).unwrap();
                device.plugins.push(status);
            }
        }
        async fn start(&self, pack: &str) -> Setup {
            let result = handle(
                &self.app,
                "workflows.start",
                &json!({"pack_id":pack,"runner_id":self.app.this_device_id()}),
            )
            .await
            .unwrap();
            serde_json::from_value(result["setup"].clone()).unwrap()
        }
        async fn configure(&self, setup: &Setup) -> Setup {
            let answers: BTreeMap<_, _> = setup
                .pack
                .questions
                .iter()
                .map(|q| (q.id.clone(), format!("scope-{}", q.id)))
                .collect();
            let result = handle(
                &self.app,
                "workflows.configure",
                &json!({"id":setup.id,"answers":answers}),
            )
            .await
            .unwrap();
            serde_json::from_value(result["setup"].clone()).unwrap()
        }
        async fn repository(&self) -> Setup {
            self.provider();
            self.advertise("github", &["github"]);
            let setup = self
                .configure(&self.start("repository-monitoring").await)
                .await;
            let result = handle(
                &self.app,
                "workflows.connection",
                &json!({"id":setup.id,"service_id":"github","plugin_id":"github"}),
            )
            .await
            .unwrap();
            serde_json::from_value(result["setup"].clone()).unwrap()
        }
    }
    impl Drop for Fixture {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.home);
        }
    }

    fn preview(app: &App, setup: &mut Setup, job_id: &str) -> Job {
        let bot_id = setup.bot_ids[&setup.pack.sample_specialist].clone();
        let dm = app.dm_with(&bot_id, None).unwrap();
        let job = Job {
            id: job_id.into(),
            chat_id: dm.meta.id.clone(),
            bot_id: bot_id.clone(),
            kind: "workflow_sample".into(),
            trigger_message_id: "sample-trigger".into(),
            routine_id: None,
            check: None,
            requested_by: app.this_device_id().unwrap(),
            from_bot_id: None,
            hops: 0,
            round: 0,
            is_winding_down: false,
            setup: None,
            task_id: None,
            handoff: None,
            task_context: None,
            created_at: now_secs() - 1.0,
        };
        setup.sample = Some(Sample {
            job_id: job.id.clone(),
            chat_id: dm.meta.id,
            bot_id,
            started_at: job.created_at,
            state: "running".into(),
            message_ids: Vec::new(),
        });
        setup.phase = "sample".into();
        save(app, setup).unwrap();
        job
    }

    #[test]
    fn packs_validate_references_ids_versions_and_schedules() {
        let index = marketplace::bundled();
        assert_eq!(index.packs.len(), 4);
        for pack in &index.packs {
            assert!(Pack::parse(&serde_json::to_value(pack).unwrap(), &index).is_ok());
        }
        // A pack with channels is version 2, and each channel names one of its connections.
        let feedback = serde_json::to_value(index.pack("feedback-collector").unwrap()).unwrap();
        for (pointer, value) in [("/version", json!(1)), ("/channels/0/service_id", json!("gmail")), ("/channels/0/listen", json!({})), ("/channels/0/specialist_id", json!("missing"))] {
            let mut invalid = feedback.clone();
            *invalid.pointer_mut(pointer).unwrap() = value;
            assert!(Pack::parse(&invalid, &index).is_err(), "{pointer}");
        }
        let original = serde_json::to_value(&index.packs[0]).unwrap();
        for (field, value) in [
            ("version", json!(2)),
            ("sample_specialist", json!("missing")),
            ("specialists", json!([])),
            (
                "routines",
                json!([{"id":"daily","specialist_id":"preparer","name":"x","schedule":"every 1m","prompt":"p"}]),
            ),
        ] {
            let mut invalid = original.clone();
            invalid[field] = value;
            assert!(Pack::parse(&invalid, &index).is_err(), "{field}");
        }
        let mut invalid = original.clone();
        invalid["questions"][0]["id"] = json!("Bad ID");
        assert!(Pack::parse(&invalid, &index).is_err());
        let mut repeated = original.clone();
        repeated["connections"] =
            json!([{"service_id":"gmail","name":"Gmail"},{"service_id":"gmail","name":"Again"}]);
        assert!(Pack::parse(&repeated, &index).is_err());
    }

    #[tokio::test]
    async fn resume_cancel_and_restart_reuse_resources_and_keep_imports_paused() {
        let fixture = Fixture::new();
        let setup = fixture.start("repository-monitoring").await;
        assert!(setup.bot_ids.is_empty());
        let configured = fixture.configure(&setup).await;
        let again = fixture.configure(&configured).await;
        assert_eq!(configured.bot_ids, again.bot_ids);
        assert_eq!(configured.routine_ids, again.routine_ids);
        assert_eq!(
            fixture.app.state.lock().unwrap().bots.len(),
            2,
            "Chef and one specialist"
        );
        let routine_id = configured.routine_ids.values().next().unwrap();
        assert!(!fixture.app.routine(routine_id).unwrap().is_enabled);
        assert!(crate::routines::set_enabled(&fixture.app, routine_id, true).is_err());
        handle(&fixture.app, "workflows.cancel", &json!({"id":setup.id}))
            .await
            .unwrap();
        let resumed = fixture.start("repository-monitoring").await;
        assert_eq!(resumed.bot_ids, configured.bot_ids);
        assert_eq!(resumed.routine_ids, configured.routine_ids);
        let reloaded = App::load(Config {
            home: fixture.home.clone(),
            port: 0,
        })
        .unwrap();
        assert_eq!(
            get(&reloaded, &setup.id).unwrap().bot_ids,
            configured.bot_ids
        );
        assert!(!reloaded.routine(routine_id).unwrap().is_enabled);
    }

    #[tokio::test]
    async fn suitable_bots_and_routines_are_reused_without_profile_changes() {
        let fixture = Fixture::new();
        let mut setup = fixture.start("repository-monitoring").await;
        let template = setup.templates.values().next().unwrap();
        let bot = Bot {
            id: "existing".into(),
            name: "My Watcher".into(),
            description: template.description.clone(),
            symbol_name: "eye".into(),
            accent: "red".into(),
            avatar: None,
            runner_id: setup.runner_id.clone(),
            provider: "deepseek".into(),
            model: None,
            thinking: None,
            legacy_instructions: String::new(),
            workdir: None,
            permissions: None,
            created_at: 1.0,
        };
        fixture.app.create_bot_with_dm(bot.clone(), None).unwrap();
        let spec = &setup.pack.routines[0];
        let routine = crate::routines::create(
            &fixture.app,
            &bot.id,
            &spec.name,
            &spec.schedule,
            &spec.prompt,
            None,
            false,
        )
        .unwrap();
        setup = fixture.configure(&setup).await;
        assert_eq!(setup.bot_ids.values().collect::<Vec<_>>(), vec![&bot.id]);
        assert_eq!(
            setup.routine_ids.values().collect::<Vec<_>>(),
            vec![&routine.id]
        );
        assert!(setup.owned_routine_ids.is_empty());
        assert_eq!(fixture.app.bot(&bot.id).unwrap(), bot);
    }

    #[tokio::test]
    async fn accepts_only_required_answers_and_bots_on_the_selected_runner() {
        let fixture = Fixture::new();
        let setup = fixture.start("repository-monitoring").await;
        for answers in [
            json!({}),
            json!({"repositories":"repo","unasked":"x"}),
            json!({"repositories":"API_KEY=secret-value"}),
        ] {
            assert!(handle(
                &fixture.app,
                "workflows.configure",
                &json!({"id":setup.id,"answers":answers})
            )
            .await
            .is_err());
        }
        assert_eq!(fixture.app.state.lock().unwrap().bots.len(), 1);
        assert!(handle(&fixture.app, "workflows.configure", &json!({"id":setup.id,"answers":{"repositories":"repo"},"bot_ids":{"monitor":"missing"}})).await.is_err());
    }

    #[tokio::test]
    async fn named_accounts_require_explicit_selection_and_retries_do_not_install_another() {
        let fixture = Fixture::new();
        let setup = fixture
            .configure(&fixture.start("inbox-triage").await)
            .await;
        fixture.advertise("gmail", &["gmail-work", "gmail-personal"]);
        let params = json!({"id":setup.id,"service_id":"gmail"});
        assert!(handle(&fixture.app, "workflows.connection", &params)
            .await
            .unwrap_err()
            .contains("Choose one of the"));
        let result = handle(
            &fixture.app,
            "workflows.connection",
            &json!({"id":setup.id,"service_id":"gmail","plugin_id":"gmail-personal"}),
        )
        .await
        .unwrap();
        assert_eq!(result["setup"]["connection_ids"]["gmail"], "gmail-personal");
        let result = handle(&fixture.app, "workflows.connection", &params)
            .await
            .unwrap();
        assert_eq!(result["setup"]["connection_ids"]["gmail"], "gmail-personal");
        assert!(handle(
            &fixture.app,
            "workflows.connection",
            &json!({"id":setup.id,"service_id":"gmail","plugin_id":"slack-work"})
        )
        .await
        .is_err());
        assert_eq!(choices(&fixture.app, &setup.runner_id, "gmail").len(), 2);
    }

    #[tokio::test]
    async fn a_missing_account_is_added_as_a_named_account_and_a_missing_service_waits() {
        let fixture = Fixture::new();
        let setup = fixture.configure(&fixture.start("meeting-preparation").await).await;
        let params = json!({"id":setup.id,"service_id":"google-calendar"});
        let added = handle(&fixture.app, "workflows.connection", &params).await.unwrap();
        let id = added["setup"]["connection_ids"]["google-calendar"].as_str().unwrap().to_string();
        assert!(id.starts_with("google-calendar-"), "a named account of its own: {id}");
        let account = &added["connections"][0]["choices"][0];
        assert_eq!(account["service_id"], "google-calendar");
        assert_eq!(account["account_name"], "Meeting preparation");
        // A retry keeps the recorded account rather than adding another.
        handle(&fixture.app, "workflows.connection", &params).await.unwrap();
        assert_eq!(fixture.app.plugins.lock().unwrap().instances("google-calendar").count(), 1);

        // A service this index lacks leaves the setup waiting, its bots kept.
        let mut waiting = get(&fixture.app, &setup.id).unwrap();
        waiting.pack.connections.push(Requirement { service_id: "not-yet-listed".into(), name: "Not Yet Listed".into() });
        save(&fixture.app, &mut waiting).unwrap();
        let error = handle(&fixture.app, "workflows.connection", &json!({"id":setup.id,"service_id":"not-yet-listed"})).await.unwrap_err();
        assert!(error.contains("isn't in the marketplace"), "{error}");
        assert_eq!(fixture.start("meeting-preparation").await.bot_ids, setup.bot_ids);
    }

    #[tokio::test]
    async fn setup_survives_a_restart_and_rides_in_the_roster() {
        let fixture = Fixture::new();
        let setup = fixture.repository().await;
        let reloaded = App::load(Config { home: fixture.home.clone(), port: 0 }).unwrap();
        assert_eq!(get(&reloaded, &setup.id).unwrap(), setup);
        let roster: crate::model::RosterBlob = serde_json::from_value(json!({ "bots": [], "chats": [], "updated_at": 1.0, "workflows": [setup] })).unwrap();
        assert_eq!(roster.workflows.unwrap()[0].connection_ids["github"], "github");
    }

    #[tokio::test]
    async fn accounts_can_be_chosen_first_and_a_lone_account_is_used() {
        let fixture = Fixture::new();
        fixture.provider();
        fixture.advertise("github", &["github"]);
        let setup = fixture.start("repository-monitoring").await;
        let progress = handle(&fixture.app, "workflows.get", &json!({"id":setup.id})).await.unwrap();
        assert!(progress["specialists"][0]["selected_id"].is_null(), "nothing to reuse, so setup adds the bot");
        let chosen = handle(&fixture.app, "workflows.connection", &json!({"id":setup.id,"service_id":"github","plugin_id":"github"})).await.unwrap();
        assert_eq!(chosen["setup"]["connection_ids"]["github"], "github");
        assert_eq!(chosen["setup"]["phase"], "questions");

        fixture.advertise("gmail", &["gmail-work"]);
        let inbox = fixture.configure(&fixture.start("inbox-triage").await).await;
        assert_eq!(inbox.connection_ids["gmail"], "gmail-work");
        let progress = handle(&fixture.app, "workflows.get", &json!({"id":inbox.id})).await.unwrap();
        assert_eq!(progress["specialists"][0]["selected_id"], json!(inbox.bot_ids["triager"]));

        plugin_removed(&fixture.app, "gmail-work");
        assert!(get(&fixture.app, &inbox.id).unwrap().connection_ids.is_empty(), "a removed plugin is no account");

        let several = Fixture::new();
        several.advertise("gmail", &["gmail-work", "gmail-personal"]);
        let inbox = several.configure(&several.start("inbox-triage").await).await;
        assert!(inbox.connection_ids.is_empty(), "one of several named accounts is the user's choice");
    }

    #[tokio::test]
    async fn only_the_current_completed_sample_can_be_reviewed_before_enable() {
        let fixture = Fixture::new();
        let mut setup = fixture.repository().await;
        assert!(
            handle(&fixture.app, "workflows.enable", &json!({"id":setup.id}))
                .await
                .is_err()
        );
        let job = preview(&fixture.app, &mut setup, "preview-current");
        assert!(handle(
            &fixture.app,
            "workflows.review",
            &json!({"id":setup.id,"job_id":job.id})
        )
        .await
        .is_err());
        let mut partial = Message::new(
            &job.chat_id,
            Author::Bot {
                bot_id: job.bot_id.clone(),
            },
            Body::text("unfinished"),
        );
        partial.state = MessageState::Streaming;
        fixture.app.upsert_message(partial, false);
        sample_finished(&fixture.app, &job, TurnOutcome::Skipped);
        assert_eq!(
            get(&fixture.app, &setup.id).unwrap().sample.unwrap().state,
            "failed"
        );
        let job = preview(&fixture.app, &mut setup, "preview-next");
        let result = Message::new(
            &job.chat_id,
            Author::Bot {
                bot_id: job.bot_id.clone(),
            },
            Body::text("Here is the sample briefing."),
        );
        fixture.app.upsert_message(result.clone(), true);
        sample_finished(&fixture.app, &job, TurnOutcome::Sent);
        assert_eq!(
            get(&fixture.app, &setup.id)
                .unwrap()
                .sample
                .unwrap()
                .message_ids,
            [result.id]
        );
        assert!(handle(
            &fixture.app,
            "workflows.review",
            &json!({"id":setup.id,"job_id":"preview-current"})
        )
        .await
        .is_err());
        handle(
            &fixture.app,
            "workflows.review",
            &json!({"id":setup.id,"job_id":job.id}),
        )
        .await
        .unwrap();
        handle(&fixture.app, "workflows.enable", &json!({"id":setup.id}))
            .await
            .unwrap();
        assert!(setup
            .routine_ids
            .values()
            .all(|id| fixture.app.routine(id).unwrap().is_enabled));
        handle(&fixture.app, "workflows.cancel", &json!({"id":setup.id}))
            .await
            .unwrap();
        assert!(setup
            .routine_ids
            .values()
            .all(|id| !fixture.app.routine(id).unwrap().is_enabled));
        sample_finished(&fixture.app, &job, TurnOutcome::Sent);
        assert!(get(&fixture.app, &setup.id).unwrap().sample.is_none());
        assert!(
            handle(&fixture.app, "workflows.enable", &json!({"id":setup.id}))
                .await
                .is_err()
        );
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn turning_a_workflow_on_starts_its_channel_and_turning_it_off_pauses_it() {
        let fixture = Fixture::new();
        fixture.provider();
        let app = &fixture.app;
        let manifest = marketplace::bundled().plugin("telegram").cloned().unwrap();
        let telegram = crate::plugins::accounts::install(app, manifest, "marketplace", Some("Community")).unwrap();
        crate::plugins::set_variables(app, &telegram.id, &[("TELEGRAM_BOT_TOKEN".to_string(), "1:abc".to_string())].into_iter().collect()).unwrap();
        let setup = fixture.configure(&fixture.start("feedback-collector").await).await;
        // A stand-in GitHub account, advertised once the Runner's own record is current.
        fixture.advertise("github", &["github"]);
        for (service, plugin) in [("telegram", telegram.id.as_str()), ("github", "github")] {
            handle(app, "workflows.connection", &json!({"id":setup.id,"service_id":service,"plugin_id":plugin})).await.unwrap();
        }
        let mut setup = get(app, &setup.id).unwrap();
        let viewed = handle(app, "workflows.get", &json!({"id":setup.id})).await.unwrap();
        assert_eq!(viewed["channels"][0]["name"], "Community feedback");
        assert!(viewed["channels"][0]["channel"].is_null(), "nothing listens before the workflow is on");
        assert!(app.channels.statuses().is_empty());
        let job = preview(app, &mut setup, "feedback-sample");
        let result = Message::new(&job.chat_id, Author::Bot { bot_id: job.bot_id.clone() }, Body::text("Today's digest: none yet."));
        app.upsert_message(result, true);
        sample_finished(app, &job, TurnOutcome::Sent);
        handle(app, "workflows.review", &json!({"id":setup.id,"job_id":job.id})).await.unwrap();
        handle(app, "workflows.enable", &json!({"id":setup.id})).await.unwrap();
        let channel = app.channels.statuses().remove(0);
        assert_eq!((channel.bot_id.as_str(), channel.account_id.as_str(), channel.state.as_str()), (setup.bot_ids["collector"].as_str(), telegram.id.as_str(), "listening"));
        assert_eq!(channel.listen.describe(), "mentions, replies, #feedback");
        assert_eq!(get(app, &setup.id).unwrap().channel_ids["community"], channel.id);
        let viewed = handle(app, "workflows.get", &json!({"id":setup.id})).await.unwrap();
        assert_eq!(viewed["channels"][0]["channel"]["id"], channel.id);
        // The user narrowed it to one chat. Turning it on again, even without the record of
        // its id, finds it on the Runner and keeps the chat.
        let chats = vec![crate::channels::ChannelChat { id: "-1001".into(), title: "Acme Community".into() }];
        let narrowed = crate::channels::config_for(&channel.bot_id, "telegram", &channel.name, &channel.task, crate::channels::ChannelSpec { account_id: telegram.id.clone(), chats, listen: channel.listen.clone() });
        crate::event_triggers::serve(app, "events.update", &json!({"id":channel.id,"config":narrowed})).unwrap();
        let mut again = get(app, &setup.id).unwrap();
        again.channel_ids.clear();
        turn_on_channels(app, &mut again).await.unwrap();
        let statuses = app.channels.statuses();
        assert_eq!((statuses.len(), statuses[0].id.as_str(), statuses[0].chats.len()), (1, channel.id.as_str(), 1));
        handle(app, "workflows.cancel", &json!({"id":setup.id})).await.unwrap();
        assert_eq!(app.channels.statuses()[0].state, "paused", "turning it off pauses the channel and keeps it");
    }

    #[tokio::test]
    async fn changes_to_answers_accounts_or_routines_require_another_sample() {
        let fixture = Fixture::new();
        let mut setup = fixture.repository().await;
        let job = preview(&fixture.app, &mut setup, "sample");
        fixture.app.upsert_message(
            Message::new(
                &job.chat_id,
                Author::Bot {
                    bot_id: job.bot_id.clone(),
                },
                Body::text("Sample."),
            ),
            false,
        );
        sample_finished(&fixture.app, &job, TurnOutcome::Sent);
        handle(
            &fixture.app,
            "workflows.review",
            &json!({"id":setup.id,"job_id":job.id}),
        )
        .await
        .unwrap();
        let mut answers = setup.answers.clone();
        answers.insert("repositories".into(), "another/repo".into());
        handle(
            &fixture.app,
            "workflows.configure",
            &json!({"id":setup.id,"answers":answers}),
        )
        .await
        .unwrap();
        assert!(get(&fixture.app, &setup.id).unwrap().sample.is_none());
        assert!(
            handle(&fixture.app, "workflows.enable", &json!({"id":setup.id}))
                .await
                .is_err()
        );
    }

    #[tokio::test]
    async fn a_pending_machine_advertisement_does_not_duplicate_a_bound_install() {
        let fixture = Fixture::new();
        let mut setup = fixture
            .configure(&fixture.start("inbox-triage").await)
            .await;
        setup
            .connection_ids
            .insert("gmail".into(), "gmail-stable-instance".into());
        save(&fixture.app, &mut setup).unwrap();
        let response = handle(
            &fixture.app,
            "workflows.connection",
            &json!({"id":setup.id,"service_id":"gmail"}),
        )
        .await
        .unwrap();
        assert_eq!(
            response["setup"]["connection_ids"]["gmail"],
            "gmail-stable-instance"
        );
        assert!(choices(&fixture.app, &setup.runner_id, "gmail").is_empty());
    }

    #[tokio::test]
    async fn sample_boundaries_and_late_outcomes_keep_the_current_generation() {
        let fixture = Fixture::new();
        let mut setup = fixture.repository().await;
        let job = preview(&fixture.app, &mut setup, "queued-sample");
        fixture.app.upsert_message(
            Message::new(
                &job.chat_id,
                Author::Bot {
                    bot_id: job.bot_id.clone(),
                },
                Body::text("Reply from a preceding queued turn."),
            ),
            false,
        );
        sample_started(&fixture.app, &job);
        let own = Message::new(
            &job.chat_id,
            Author::Bot {
                bot_id: job.bot_id.clone(),
            },
            Body::text("This sample's reply."),
        );
        fixture.app.upsert_message(own.clone(), false);
        sample_finished(&fixture.app, &job, TurnOutcome::Sent);
        assert_eq!(
            get(&fixture.app, &setup.id)
                .unwrap()
                .sample
                .unwrap()
                .message_ids,
            vec![own.id]
        );
        let stale_job = preview(&fixture.app, &mut setup, "cancelled-sample");
        let mut stale_completion = get(&fixture.app, &setup.id).unwrap();
        stale_completion.sample.as_mut().unwrap().state = "ready".into();
        handle(&fixture.app, "workflows.cancel", &json!({"id":setup.id}))
            .await
            .unwrap();
        // Simulates completion already read before cancellation persisted.
        persist(&fixture.app, &mut stale_completion, Some(&stale_job.id)).unwrap();
        assert_eq!(get(&fixture.app, &setup.id).unwrap().phase, "cancelled");
        assert!(get(&fixture.app, &setup.id).unwrap().sample.is_none());
    }

    #[cfg(all(feature = "runner", feature = "server"))]
    #[tokio::test]
    async fn a_real_sample_job_streams_a_result_and_then_allows_review_and_activation() {
        use crate::credentials::{CustomApi, CustomModel, CustomProvider};
        use axum::{routing::post, Router};
        let seen = Arc::new(std::sync::Mutex::new(None::<Value>));
        let capture = seen.clone();
        let router = Router::new().route("/chat/completions", post(move |axum::Json(body): axum::Json<Value>| {
            let capture = capture.clone();
            async move {
                *capture.lock().unwrap() = Some(body);
                ([ ("content-type", "text/event-stream") ], "data: {\"choices\":[{\"delta\":{\"role\":\"assistant\",\"content\":\"Sample: review the two open pull requests.\"},\"finish_reason\":null}]}\n\ndata: {\"choices\":[{\"delta\":{},\"finish_reason\":\"stop\"}]}\n\ndata: [DONE]\n\n")
            }
        }));
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let base_url = format!("http://{}", listener.local_addr().unwrap());
        let server = tokio::spawn(async move {
            axum::serve(listener, router).await.unwrap();
        });
        let fixture = Fixture::new();
        let setup = fixture.repository().await;
        let model: CustomModel =
            serde_json::from_value(json!({"id":"sample-model","context_window":128000})).unwrap();
        fixture.app.credentials.lock().unwrap().custom.insert(
            "custom:sample".into(),
            CustomProvider {
                name: "Sample server".into(),
                api: CustomApi::ChatCompletions,
                base_url,
                api_key: String::new(),
                models: vec![model],
                created_at: 1,
            },
        );
        for id in setup.bot_ids.values() {
            fixture
                .app
                .update_bot(id, |b| b.provider = "custom:sample".into())
                .unwrap();
        }
        // Install a fixture record through the same persisted Store projection the local
        // Runner advertises. No external MCP or OAuth server is contacted by this sample.
        crate::config::write_json_private(&fixture.app.config.plugins_dir().join("installed.json"), &json!({
            "plugins":[{"manifest":{"id":"github","name":"Test GitHub","servers":{"fixture":{"type":"http","url":"http://127.0.0.1:9/mcp"}}},"source":"inline","installed_at":1,"variables":{}}]
        })).unwrap();
        *fixture.app.plugins.lock().unwrap() = crate::plugins::Store::load(&fixture.app.config);
        let mut events = fixture.app.events.subscribe();
        let initial = handle(&fixture.app, "workflows.sample", &json!({"id":setup.id}))
            .await
            .unwrap();
        let job_id = initial["setup"]["sample"]["job_id"].as_str().unwrap();
        let replay = handle(&fixture.app, "workflows.sample", &json!({"id":setup.id}))
            .await
            .unwrap();
        assert_eq!(
            initial["setup"]["sample"]["job_id"],
            replay["setup"]["sample"]["job_id"]
        );
        tokio::time::timeout(std::time::Duration::from_secs(10), async {
            while get(&fixture.app, &setup.id).unwrap().sample.unwrap().state == "running" {
                let _ = events.recv().await;
            }
        })
        .await
        .unwrap();
        let completed = handle(&fixture.app, "workflows.get", &json!({"id":setup.id}))
            .await
            .unwrap();
        assert_eq!(completed["setup"]["sample"]["state"], "ready");
        assert_eq!(
            completed["sample_messages"][0]["body"]["text"],
            "Sample: review the two open pull requests."
        );
        let request = seen.lock().unwrap().take().unwrap();
        assert!(request["messages"]
            .to_string()
            .contains("scope-repositories"));
        assert!(request["messages"]
            .to_string()
            .contains("Selected integration instances"));
        let tools = request["tools"].as_array().unwrap();
        assert!(!tools.iter().any(|t| matches!(
            t["function"]["name"].as_str(),
            Some("routines" | "create_bot" | "edit_bot" | "install_plugin" | "connect_plugin" | "message_bot" | "propose_playbook")
        )));
        assert!(tools.iter().any(|t| t["function"]["name"] == "read_playbook"));
        handle(
            &fixture.app,
            "workflows.review",
            &json!({"id":setup.id,"job_id":job_id}),
        )
        .await
        .unwrap();
        handle(&fixture.app, "workflows.enable", &json!({"id":setup.id}))
            .await
            .unwrap();
        assert!(setup
            .routine_ids
            .values()
            .all(|id| fixture.app.routine(id).unwrap().is_enabled));
        server.abort();
    }

    #[tokio::test]
    async fn rolling_upgrade_and_independent_pack_updates_preserve_progress() {
        let fixture = Fixture::new();
        let mut first = fixture.start("repository-monitoring").await;
        first.updated_at = 1.0;
        let mut current = vec![first.clone()];
        assert!(merge(&mut current, None));
        assert!(merge(&mut current, Some(vec![])));
        let mut second = fixture.start("inbox-triage").await;
        second.updated_at = 1.0;
        assert!(merge(&mut current, Some(vec![second.clone()])));
        let newer = Setup { updated_at: 2.0, phase: "connections".into(), ..first };
        assert!(!merge(&mut current, Some(vec![newer.clone(), second])));
        assert_eq!(current[0], newer);
    }
}
