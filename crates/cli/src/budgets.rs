//! Runner-owned limits. A DM's turns, a task's runs, and a routine's runs keep their existing
//! identities; this module owns only what they used, reservations, a stop at a limit, and resuming.

use std::collections::{BTreeMap, HashSet};
use std::sync::{Arc, Mutex};

use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

use crate::app::App;
use crate::config::now_secs;
use crate::model::Job;

const PURPOSE: &str = "runner_budgets_v1";

/// How long a resume's receipt answers a repeated delivery of the same request.
const RECEIPT_SECS: f64 = 7.0 * 24.0 * 3600.0;

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
#[serde(default, deny_unknown_fields)]
pub struct BudgetLimits {
    pub max_usd: Option<f64>,
    pub max_tokens: Option<u64>,
    pub max_runtime_secs: Option<u64>,
    pub max_retries: Option<u64>,
    pub max_connector_calls: Option<u64>,
}

impl BudgetLimits {
    pub fn validate(&self) -> Result<(), String> {
        if self.max_usd.is_some_and(|usd| !usd.is_finite() || usd < 0.0) {
            return Err("The spending limit must be a finite, nonnegative dollar amount.".into());
        }
        if self.max_runtime_secs.is_some_and(|seconds| seconds > 31_536_000) {
            return Err("A run time limit is at most one year; leave it empty for no limit.".into());
        }
        Ok(())
    }

    pub fn is_empty(&self) -> bool {
        *self == Self::default()
    }
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum Pricing {
    Api,
    SubscriptionEstimate,
    Unknown,
}

/// The limit work stopped at. Its wire name lets the apps say which in their own words; the
/// reason is the chat notice and what a refused model or tool call reads.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Limit {
    Usd,
    Tokens,
    Runtime,
    Retries,
    ConnectorCalls,
    UnknownPrice,
}

impl Limit {
    fn name(self) -> &'static str {
        match self {
            Limit::Usd => "usd",
            Limit::Tokens => "tokens",
            Limit::Runtime => "runtime",
            Limit::Retries => "retries",
            Limit::ConnectorCalls => "connector_calls",
            Limit::UnknownPrice => "unknown_price",
        }
    }

    fn reason(self) -> String {
        let what = match self {
            Limit::Usd => "spending",
            Limit::Tokens => "token",
            Limit::Runtime => "run time",
            Limit::Retries => "retry",
            Limit::ConnectorCalls => "plugin call",
            Limit::UnknownPrice => {
                return "Stopped: this model has no known price, so a spending limit can't cover it. Add a token or run time limit in Limits.".into()
            }
        };
        format!("Stopped at the {what} limit. Raise it in Limits to resume.")
    }
}

const INTERRUPTED: &str = "Interrupted when the Runner restarted. Check what it already did, then resume it in Limits.";

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct BudgetUsage {
    pub tokens: u64,
    pub api_cost_usd: f64,
    pub subscription_estimate_usd: f64,
    pub unknown_price_calls: u64,
    /// Requests without usage use input/received-output estimates; a restart retains
    /// outstanding reservations. These are estimates rather than a claim about the bill.
    pub estimated_calls: u64,
    pub model_calls: u64,
    pub runtime_secs: f64,
    pub retries: u64,
    pub connector_calls: u64,
}

impl BudgetUsage {
    fn usd(&self) -> f64 {
        self.api_cost_usd + self.subscription_estimate_usd
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct BudgetSnapshot {
    /// `chat` holds the limits each new turn in that chat starts with; `job` is one turn,
    /// `task` all runs of a task, `routine` all runs of a routine.
    pub kind: String,
    pub id: String,
    pub runner_id: String,
    pub bot_id: String,
    pub chat_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub job_kind: Option<String>,
    pub limits: BudgetLimits,
    pub usage: BudgetUsage,
    /// `ready`, `running`, `complete`, `budget_exhausted`, or `interrupted`.
    pub state: String,
    /// The limit a `budget_exhausted` scope stopped at: `usd`, `tokens`, `runtime`, `retries`,
    /// `connector_calls`, or `unknown_price`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reached: Option<String>,
    pub reason: Option<String>,
    pub updated_at: f64,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
struct Charge {
    tokens: u64,
    usd: f64,
    pricing: Option<Pricing>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
struct Record {
    view: BudgetSnapshot,
    #[serde(default)]
    pending: BTreeMap<String, Charge>,
    /// A turn's Job, so Resume can go on with it.
    job: Option<Job>,
    started_at: Option<f64>,
    #[serde(default)]
    active_runs: u64,
}

impl Record {
    fn is_stopped(&self) -> bool {
        matches!(self.view.state.as_str(), "budget_exhausted" | "interrupted")
    }

    fn is_active(&self) -> bool {
        self.started_at.is_some() || !self.pending.is_empty()
    }
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
struct Ledger {
    records: BTreeMap<String, Record>,
    #[serde(default)]
    remote: BTreeMap<String, Vec<BudgetSnapshot>>,
    #[serde(default)]
    receipts: BTreeMap<String, Value>,
}

/// Lazy loading also works when the user creates/pairs an identity after CLI startup.
#[derive(Default)]
pub struct BudgetStore(Mutex<Option<Ledger>>);

fn key(kind: &str, id: &str) -> String {
    format!("{kind}:{id}")
}

/// What still exists, read before the ledger's lock is taken. `tasks` is read only where the
/// limits change, not on every turn.
struct Live {
    chats: HashSet<String>,
    routines: HashSet<String>,
    devices: HashSet<String>,
    tasks: Option<HashSet<String>>,
}

fn live(app: &App, with_tasks: bool) -> Live {
    let tasks = with_tasks.then(|| crate::tasks::list(app).unwrap_or_default().into_iter().map(|task| task.id).collect());
    let state = app.state.lock().unwrap();
    Live {
        chats: state.chats.iter().map(|c| c.meta.id.clone()).collect(),
        routines: state.routines.iter().map(|r| r.id.clone()).collect(),
        devices: state.devices.iter().map(|d| d.id.clone()).collect(),
        tasks,
    }
}

/// Keeps only what someone may still look at or resume: limits the user set, a turn that
/// stopped or waits to go on, and work in flight. A finished turn, a chat or routine that is
/// gone, an unpaired Runner's projection, and old receipts go.
fn prune(ledger: &mut Ledger, live: &Live) {
    ledger.records.retain(|_, record| {
        if record.is_active() {
            return true;
        }
        let exists = match record.view.kind.as_str() {
            "routine" => live.routines.contains(&record.view.id),
            "task" => live.tasks.as_ref().is_none_or(|tasks| tasks.contains(&record.view.id)),
            _ => live.chats.contains(&record.view.chat_id),
        };
        exists
            && match record.view.kind.as_str() {
                // A resumed turn waits here for its run; a newer turn in the chat replaces it.
                "job" => record.view.state != "complete",
                _ => record.is_stopped() || !record.view.limits.is_empty(),
            }
    });
    ledger.remote.retain(|runner, _| live.devices.contains(runner));
    let cutoff = now_secs() - RECEIPT_SECS;
    ledger.receipts.retain(|_, receipt| receipt["updated_at"].as_f64().is_some_and(|at| at > cutoff));
}

impl BudgetStore {
    pub fn clear(&self) {
        *self.0.lock().unwrap() = None;
    }

    fn load<'a>(&self, app: &App, held: &'a mut Option<Ledger>) -> Result<&'a mut Ledger, String> {
        if held.is_none() {
            let dek = app.dek().ok_or("Create or pair an identity first.")?;
            let mut ledger: Ledger = match app.store.runner_limits(PURPOSE).map_err(|e| e.to_string())? {
                Some(bytes) => crate::crypto::decrypt_json(&dek, PURPOSE, &bytes)
                    .map_err(|e| format!("Cannot read Runner budgets: {e}"))?,
                None => Ledger::default(),
            };
            // A restart never replenishes a limit: open reservations count as used. A turn that
            // was running may have done part of its work, so it waits for the user to resume
            // it; a routine runs again on its schedule.
            for record in ledger.records.values_mut() {
                for (_, charge) in std::mem::take(&mut record.pending) {
                    apply_charge(&mut record.view.usage, &charge, true);
                }
                if let Some(started) = record.started_at.take() {
                    record.active_runs = 0;
                    record.view.usage.runtime_secs += (now_secs() - started).max(0.0);
                    if record.view.kind == "job" {
                        record.view.state = "interrupted".into();
                        record.view.reason = Some(INTERRUPTED.into());
                    } else if record.view.state == "running" {
                        record.view.state = "ready".into();
                    }
                }
            }
            self.save(app, &ledger)?;
            *held = Some(ledger);
        }
        Ok(held.as_mut().unwrap())
    }

    fn save(&self, app: &App, ledger: &Ledger) -> Result<(), String> {
        let dek = app.dek().ok_or("The account is no longer available.")?;
        let ciphertext = crate::crypto::encrypt_json(&dek, PURPOSE, ledger).map_err(|e| e.to_string())?;
        app.store
            .set_runner_limits(PURPOSE, &ciphertext)
            .map_err(|e| format!("Cannot persist Runner accounting: {e}"))
    }

    fn change<T>(&self, app: &App, f: impl FnOnce(&mut Ledger) -> Result<T, String>) -> Result<T, String> {
        let mut held = self.0.lock().unwrap();
        let current = self.load(app, &mut held)?;
        let mut changed = current.clone();
        let result = f(&mut changed);
        // Persist a stop even when admission failed. A storage failure prevents starting work,
        // and no in-memory grant becomes visible without its durable write.
        if changed != *current {
            self.save(app, &changed)?;
        }
        *current = changed;
        result
    }

    /// This Runner's limits and every paired Runner's, for the apps.
    pub fn snapshots(&self, app: &App) -> Vec<BudgetSnapshot> {
        if !app.has_identity() {
            return Vec::new();
        }
        let mut held = self.0.lock().unwrap();
        match self.load(app, &mut held) {
            Ok(ledger) => ledger
                .records
                .values()
                .map(|r| r.view.clone())
                .chain(ledger.remote.values().flatten().cloned())
                .collect(),
            Err(error) => {
                tracing::error!(%error, "reading budget snapshots");
                Vec::new()
            }
        }
    }

    /// This Runner's own, for its `machine` blob.
    pub fn local_snapshots(&self, app: &App) -> Vec<BudgetSnapshot> {
        let this = app.this_device_id().unwrap_or_default();
        self.snapshots(app).into_iter().filter(|s| s.runner_id == this).collect()
    }

    pub fn merge_remote(&self, app: &App, runner: &str, mut snapshots: Vec<BudgetSnapshot>) {
        snapshots.retain(|s| s.runner_id == runner);
        let mut changed = false;
        let result = self.change(app, |ledger| {
            if ledger.remote.get(runner).map(Vec::as_slice).unwrap_or_default() != snapshots.as_slice() {
                changed = true;
                if snapshots.is_empty() {
                    ledger.remote.remove(runner);
                } else {
                    ledger.remote.insert(runner.into(), snapshots);
                }
            }
            Ok(())
        });
        if let Err(error) = result {
            tracing::error!(%error, "keeping encrypted remote budget state");
        }
        if changed {
            self.notify(app);
        }
    }

    /// Tells this Device's app; other Devices hear it through [`Self::publish`].
    fn notify(&self, app: &App) {
        app.emit(crate::events::Event::BudgetsChanged { budgets: self.snapshots(app) });
    }

    /// Tells this Device's app and every paired Device: a limit, a stop, a start, or an end.
    fn publish(&self, app: &App) {
        self.notify(app);
        app.push_machine_blob_if_changed();
    }

    /// Whether the scope may start work. A scope this stops is published once.
    pub fn admit(&self, app: &App, kind: &str, id: &str) -> Result<(), String> {
        let mut stopped_now = false;
        let result = self.change(app, |ledger| match ledger.records.get_mut(&key(kind, id)) {
            Some(record) => {
                let was_stopped = record.is_stopped();
                let result = check(record);
                stopped_now = !was_stopped && result.is_err();
                result
            }
            None => Ok(()),
        });
        if stopped_now {
            self.publish(app);
        }
        result
    }
}

fn apply_charge(usage: &mut BudgetUsage, charge: &Charge, estimated: bool) {
    usage.tokens = usage.tokens.saturating_add(charge.tokens);
    match charge.pricing.unwrap_or(Pricing::Unknown) {
        Pricing::Api => usage.api_cost_usd += charge.usd,
        Pricing::SubscriptionEstimate => usage.subscription_estimate_usd += charge.usd,
        Pricing::Unknown => usage.unknown_price_calls += 1,
    }
    usage.estimated_calls += u64::from(estimated);
}

/// Stops the scope at a limit. It refuses work until the user resumes it.
fn exhaust(record: &mut Record, limit: Limit) -> String {
    let reason = limit.reason();
    record.view.state = "budget_exhausted".into();
    record.view.reached = Some(limit.name().into());
    record.view.reason = Some(reason.clone());
    record.view.updated_at = now_secs();
    reason
}

/// Refuses work for a stopped scope, and stops one that has used up a limit.
fn check(record: &mut Record) -> Result<(), String> {
    if record.is_stopped() {
        return Err(record.view.reason.clone().unwrap_or_else(|| Limit::Tokens.reason()));
    }
    let limits = &record.view.limits;
    let used = &record.view.usage;
    let running = record.started_at.map(|at| (now_secs() - at).max(0.0)).unwrap_or(0.0);
    let limit = if limits.max_tokens.is_some_and(|v| used.tokens >= v) {
        Some(Limit::Tokens)
    } else if limits.max_usd.is_some_and(|v| used.usd() >= v) {
        Some(Limit::Usd)
    } else if limits.max_runtime_secs.is_some_and(|v| used.runtime_secs + running >= v as f64) {
        Some(Limit::Runtime)
    } else if limits.max_connector_calls.is_some_and(|v| v > 0 && used.connector_calls >= v) {
        Some(Limit::ConnectorCalls)
    } else {
        None
    };
    match limit {
        Some(limit) => Err(exhaust(record, limit)),
        None => Ok(()),
    }
}

fn new_record(app: &App, kind: &str, id: &str, bot_id: &str, chat_id: &str, limits: BudgetLimits) -> Record {
    Record {
        view: BudgetSnapshot {
            kind: kind.into(),
            id: id.into(),
            runner_id: app.this_device_id().unwrap_or_default(),
            bot_id: bot_id.into(),
            chat_id: chat_id.into(),
            job_kind: None,
            limits,
            usage: BudgetUsage::default(),
            state: "ready".into(),
            reached: None,
            reason: None,
            updated_at: now_secs(),
        },
        pending: BTreeMap::new(),
        job: None,
        started_at: None,
        active_runs: 0,
    }
}

/// Every budget change goes to the bot's Runner through the existing request transport. A
/// chat's limits are what new turns start with, never a reset of what a turn used.
pub async fn dispatch(app: &Arc<App>, method: &str, params: &Value) -> Result<Value, String> {
    let runner = params["runner_id"]
        .as_str()
        .map(str::to_string)
        .or_else(|| params["bot_id"].as_str().and_then(|id| app.bot(id)).map(|b| b.runner_id))
        .or_else(|| app.this_device_id())
        .ok_or("missing runner_id")?;
    if app.this_device_id().as_deref() != Some(runner.as_str()) {
        return crate::requests::ask(app, &runner, method, params.clone()).await;
    }
    serve(app, method, params)
}

pub fn serve(app: &Arc<App>, method: &str, params: &Value) -> Result<Value, String> {
    let kind = params["kind"]
        .as_str()
        .filter(|kind| matches!(*kind, "job" | "task" | "routine" | "chat"))
        .ok_or("kind must be job, task, routine, or chat")?;
    let id = params["id"].as_str().filter(|id| !id.is_empty()).ok_or("missing id")?;
    let record_key = key(kind, id);
    if method == "budgets.set" {
        let limits: BudgetLimits = serde_json::from_value(params["limits"].clone()).map_err(|e| e.to_string())?;
        limits.validate()?;
        let task = (kind == "task").then(|| crate::tasks::get(app, id)).transpose()?;
        let bot_id = match kind {
            "routine" => app.routine(id).ok_or("Unknown routine")?.bot_id,
            "task" => task.as_ref().map(|task| task.owner_bot_id.clone()).unwrap_or_default(),
            "chat" => app.chat(id).and_then(|c| c.meta.bot_ids.first().cloned()).ok_or("Unknown chat")?,
            _ => params["bot_id"].as_str().ok_or("missing bot_id")?.to_string(),
        };
        let bot = app.bot(&bot_id).ok_or("Unknown bot")?;
        if app.this_device_id().as_deref() != Some(bot.runner_id.as_str()) {
            return Err("Budgets are managed on the bot's assigned Runner.".into());
        }
        let chat_id = match (kind, &task) {
            ("chat", _) => id.to_string(),
            (_, Some(task)) => task.chat_ids.first().cloned().unwrap_or_default(),
            _ => params["chat_id"].as_str().unwrap_or_default().to_string(),
        };
        let live = live(app, true);
        let snapshot = app.budgets.change(app, |ledger| {
            if kind == "job" && !ledger.records.contains_key(&record_key) {
                return Err("This turn has no limits to change.".into());
            }
            let record = ledger
                .records
                .entry(record_key.clone())
                .or_insert_with(|| new_record(app, kind, id, &bot_id, &chat_id, limits.clone()));
            record.view.limits = limits;
            record.view.updated_at = now_secs();
            let snapshot = record.view.clone();
            prune(ledger, &live);
            Ok(snapshot)
        })?;
        app.budgets.publish(app);
        return Ok(json!(snapshot));
    }
    if method == "budgets.resume" {
        let receipt = params["request_id"]
            .as_str()
            .filter(|id| !id.is_empty() && id.len() <= 128)
            .ok_or("Resume requires a unique request_id so a repeated delivery can't grant the limits twice.")?;
        let renew = params["renew"].as_bool().unwrap_or(false);
        let (snapshot, job, replay) = app.budgets.change(app, |ledger| {
            if let Some(saved) = ledger.receipts.get(receipt) {
                if saved["kind"] != kind || saved["id"] != id {
                    return Err("This request_id already belongs to another resume.".into());
                }
                return Ok((serde_json::from_value(saved.clone()).map_err(|e| e.to_string())?, None, true));
            }
            let record = ledger.records.get_mut(&record_key).ok_or("There is nothing to resume.")?;
            if record.is_active() {
                return Err("It's still running. Try again once it stops.".into());
            }
            if renew {
                record.view.usage = BudgetUsage::default();
            }
            record.view.state = "ready".into();
            record.view.reached = None;
            record.view.reason = None;
            check(record)?;
            record.view.updated_at = now_secs();
            let snapshot = record.view.clone();
            let job = record.job.clone();
            ledger.receipts.insert(receipt.into(), json!(snapshot));
            Ok((snapshot, job, false))
        })?;
        app.budgets.publish(app);
        #[cfg(feature = "runner")]
        if !replay && params["run"].as_bool().unwrap_or(false) {
            match (kind, job) {
                // A routine goes on with a new run, which checks it isn't running already.
                ("routine", _) => crate::routines::run_now(app, id)?,
                // A task goes on with a new run through its owner, which checks it may.
                ("task", _) => {
                    let task = crate::tasks::get(app, id)?;
                    let params = json!({ "id": id, "expected_revision": task.revision, "request_id": format!("budgets-{receipt}") });
                    let app = app.clone();
                    tokio::spawn(async move {
                        if let Err(error) = crate::tasks::dispatch(&app, "tasks.run", params).await {
                            tracing::warn!(%error, "starting a task again after its limits");
                        }
                    });
                }
                // An event's delivery goes back to its subscription's inbox, which admits it
                // again in its order, as the same Job.
                (_, Some(job)) if job.kind == "event" => {
                    crate::event_triggers::serve(app, "events.retry", &json!({ "id": job.trigger_message_id }))
                        .map_err(|error| error.to_string())?;
                }
                // A turn goes on from the transcript as it stands; a plugin call it made is
                // never sent again from here.
                (_, Some(mut job)) => {
                    job.check = job.check.or_else(|| Some(crate::model::CheckReport { found: String::new(), error: None }));
                    // A delegated turn goes on as its handoff's next attempt.
                    crate::handoffs::resume_stopped(app, job)?;
                }
                _ => {}
            }
        }
        #[cfg(not(feature = "runner"))]
        let _ = (job, replay);
        return Ok(json!(snapshot));
    }
    Err(format!("Unknown budget method {method}"))
}

#[cfg(feature = "runner")]
mod runtime;
#[cfg(feature = "runner")]
pub use runtime::{current, for_job, for_routine, wrap_provider, BudgetContext};
