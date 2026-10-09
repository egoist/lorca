//! Durable delegation contracts. Requests, Runner reports, and requester cancellations merge
//! independently per attempt; encrypted records and their relay outbox share one transaction.

use std::sync::Arc;

use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

use crate::app::{App, OutboxItem, Slot};
use crate::config::now_secs;
use crate::model::{Author, Body, Job, JobCancel, Message, MessageState, MAX_BOT_HOPS};
use crate::runtime::TurnOutcome;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct HandoffRequest {
    pub handoff_id: String,
    pub attempt: u32,
    pub job_id: String,
    pub from_bot_id: String,
    pub source_chat_id: String,
    pub source_runner_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub task_id: Option<String>,
    pub target_bot_id: String,
    pub target_chat_id: String,
    pub target_runner_id: String,
    pub trigger_message_id: String,
    pub message: String,
    #[serde(default)]
    pub context: String,
    #[serde(default)]
    pub expected_output: String,
    #[serde(default)]
    pub acceptance_criteria: Vec<String>,
    pub hops: u32,
    pub created_at: f64,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum HandoffJob {
    Request {
        request: HandoffRequest,
    },
    Result {
        handoff_id: String,
        request_job_id: String,
    },
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum HandoffStatus {
    Running,
    Completed,
    Failed,
    Blocked,
    Cancelled,
}

impl HandoffStatus {
    pub fn terminal(self) -> bool {
        self != Self::Running
    }
}

/// Wire-compatible with canonical TaskEvidence and immutable output-version references.
/// These are references to supporting records, never another output or task store.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ResultLink {
    pub kind: String,
    pub label: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub chat_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub message_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub attachment_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub url: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub output_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub version: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub review_id: Option<String>,
}

impl ResultLink {
    fn message(message: &Message, label: String) -> Self {
        Self {
            kind: "message".into(),
            label,
            chat_id: Some(message.chat_id.clone()),
            message_id: Some(message.id.clone()),
            attachment_id: None,
            url: None,
            output_id: None,
            version: None,
            review_id: None,
        }
    }

    fn validate(&self) -> Result<(), String> {
        if self.label.trim().is_empty() {
            return Err("A result link needs a label".into());
        }
        let has_message = self.chat_id.as_ref().is_some_and(|s| !s.is_empty())
            && self.message_id.as_ref().is_some_and(|s| !s.is_empty());
        let valid = match self.kind.as_str() {
            "message" => has_message,
            "output" => {
                has_message
                    && self.output_id.as_ref().is_some_and(|s| !s.is_empty())
                    && self.version.is_some_and(|v| v > 0)
            }
            "review" => has_message && self.review_id.as_ref().is_some_and(|s| !s.is_empty()),
            "file" => has_message && self.attachment_id.as_ref().is_some_and(|s| !s.is_empty()),
            "url" => self.url.as_ref().is_some_and(|s| {
                reqwest::Url::parse(s)
                    .is_ok_and(|url| url.scheme() == "https" && url.host_str().is_some())
            }),
            _ => false,
        };
        if !valid {
            return Err(format!("Invalid {} result reference", self.kind));
        }
        Ok(())
    }
}

fn request_link(request: &HandoffRequest) -> ResultLink {
    ResultLink {
        kind: "message".into(),
        label: "Delegated request".into(),
        chat_id: Some(request.target_chat_id.clone()),
        message_id: Some(request.trigger_message_id.clone()),
        attachment_id: None,
        url: None,
        output_id: None,
        version: None,
        review_id: None,
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct HandoffReport {
    pub status: HandoffStatus,
    pub summary: String,
    #[serde(default)]
    pub result_links: Vec<ResultLink>,
    /// Bot claims and observed turn/tool outcomes; the linked rows provide the underlying record.
    #[serde(default)]
    pub evidence: Vec<String>,
    pub created_at: f64,
    /// The last row before execution, so automatic reporting captures only this turn.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub started_after: Option<String>,
}

#[derive(Debug, Clone, Copy, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
enum ResultDelivery {
    #[default]
    Pending,
    Started,
    Finished,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct HandoffAttempt {
    pub request: HandoffRequest,
    pub delivery: String,
    pub report: Option<HandoffReport>,
    /// A requester cancellation is independent of Runner progress and wins a concurrent finish.
    pub cancellation: Option<HandoffReport>,
    #[serde(default)]
    result_delivery: ResultDelivery,
}

impl HandoffAttempt {
    pub fn outcome(&self) -> Option<&HandoffReport> {
        self.cancellation.as_ref().or(self.report.as_ref())
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Handoff {
    pub id: String,
    pub attempts: Vec<HandoffAttempt>,
}

impl Handoff {
    pub fn current(&self) -> &HandoffAttempt {
        self.attempts.last().expect("a handoff has an attempt")
    }
    pub fn view(&self) -> Value {
        let attempt = self.current();
        json!({ "handoff_id": self.id, "job_id": attempt.request.job_id, "request": attempt.request,
            "target_runner_id": attempt.request.target_runner_id, "delivery": match attempt.outcome() {
                Some(report) if report.status == HandoffStatus::Running => "running",
                Some(_) => "finished",
                None => &attempt.delivery,
            },
            "status": attempt.outcome().map(|r| r.status), "report": attempt.outcome(), "attempts": self.attempts })
    }
}

/// Each role has its own relay slot, so origin admission/cancellation cannot replace Runner
/// reports. Every update carries its request and can arrive before the request's own blob.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum HandoffUpdate {
    Request {
        request: HandoffRequest,
        delivery: String,
    },
    Report {
        request: HandoffRequest,
        report: HandoffReport,
    },
    Cancel {
        request: HandoffRequest,
        report: HandoffReport,
    },
}

impl HandoffUpdate {
    fn request(&self) -> &HandoffRequest {
        match self {
            Self::Request { request, .. }
            | Self::Report { request, .. }
            | Self::Cancel { request, .. } => request,
        }
    }
    fn slot(&self) -> Slot {
        let kind = match self {
            Self::Request { .. } => "request",
            Self::Report { .. } => "report",
            Self::Cancel { .. } => "cancel",
        };
        let request = self.request();
        Slot::latest(format!("{}-{}-{kind}", request.handoff_id, request.attempt))
    }
}

fn key(app: &App) -> Result<[u8; 32], String> {
    app.dek().ok_or_else(|| "No account key".into())
}

pub fn get(app: &App, id: &str) -> Result<Handoff, String> {
    let bytes = app
        .store
        .handoff(id)
        .map_err(|e| e.to_string())?
        .ok_or("Unknown handoff")?;
    crate::crypto::decrypt_json(&key(app)?, "handoff_state", &bytes).map_err(|e| e.to_string())
}

pub fn list(app: &App) -> Result<Vec<Handoff>, String> {
    let dek = key(app)?;
    let mut records: Vec<Handoff> = app
        .store
        .handoffs()
        .map_err(|e| e.to_string())?
        .iter()
        .map(|bytes| {
            crate::crypto::decrypt_json(&dek, "handoff_state", bytes).map_err(|e| e.to_string())
        })
        .collect::<Result<_, _>>()?;
    records.sort_by(|a, b| {
        a.current()
            .request
            .created_at
            .total_cmp(&b.current().request.created_at)
    });
    Ok(records)
}

fn save(
    app: &App,
    record: &Handoff,
    update: Option<&HandoffUpdate>,
    mut outbox: Vec<OutboxItem>,
) -> Result<(), String> {
    let dek = key(app)?;
    let bytes =
        crate::crypto::encrypt_json(&dek, "handoff_state", record).map_err(|e| e.to_string())?;
    if let Some(update) = update {
        outbox.insert(
            0,
            OutboxItem {
                id: uuid::Uuid::new_v4().to_string(),
                kind: "handoff".into(),
                recipient: None,
                ciphertext: crate::crypto::encrypt_json(&dek, "handoff", update)
                    .map_err(|e| e.to_string())?,
                slot: Some(update.slot()),
                group: None,
            },
        );
    }
    app.store
        .save_handoff(&record.id, &bytes, &outbox)
        .map_err(|e| e.to_string())?;
    if !outbox.is_empty() {
        app.outbox_notify.notify_waiters();
    }
    Ok(())
}

fn merge(app: &App, update: &HandoffUpdate) -> Result<Handoff, String> {
    let request = update.request();
    if request.attempt == 0 || request.hops > MAX_BOT_HOPS {
        return Err("Invalid handoff attempt".into());
    }
    let mut record = match app
        .store
        .handoff(&request.handoff_id)
        .map_err(|e| e.to_string())?
    {
        Some(bytes) => crate::crypto::decrypt_json::<Handoff>(&key(app)?, "handoff_state", &bytes)
            .map_err(|e| e.to_string())?,
        None => Handoff {
            id: request.handoff_id.clone(),
            attempts: Vec::new(),
        },
    };
    if let Some(first) = record.attempts.first() {
        let original = &first.request;
        if original.from_bot_id != request.from_bot_id
            || original.source_chat_id != request.source_chat_id
            || original.target_bot_id != request.target_bot_id
            || original.task_id != request.task_id
        {
            return Err("Handoff ownership and destination cannot change".into());
        }
    }
    let index = if let Some(index) = record
        .attempts
        .iter()
        .position(|a| a.request.attempt == request.attempt)
    {
        if record.attempts[index].request != *request {
            return Err("Conflicting handoff attempt".into());
        }
        index
    } else {
        record.attempts.push(HandoffAttempt {
            request: request.clone(),
            delivery: "queued".into(),
            report: None,
            cancellation: None,
            result_delivery: ResultDelivery::Pending,
        });
        record.attempts.sort_by_key(|a| a.request.attempt);
        record
            .attempts
            .iter()
            .position(|a| a.request.attempt == request.attempt)
            .unwrap()
    };
    let attempt = &mut record.attempts[index];
    match update {
        HandoffUpdate::Request { delivery, .. } => attempt.delivery = delivery.clone(),
        HandoffUpdate::Report { report, .. } => {
            // Terminal reports never regress to running or get replaced by another finish.
            if !attempt.report.as_ref().is_some_and(|r| r.status.terminal()) {
                attempt.report = Some(report.clone());
            }
        }
        HandoffUpdate::Cancel { report, .. } => {
            attempt.cancellation = Some(report.clone());
        }
    }
    Ok(record)
}

pub fn apply_update(app: &Arc<App>, update: HandoffUpdate) -> Result<(), String> {
    {
        let _guard = app.handoff_lock.lock().unwrap();
        let record = merge(app, &update)?;
        save(app, &record, None, Vec::new())?;
    }
    if let HandoffUpdate::Cancel { request, .. } = &update {
        app.cancel_job(&request.job_id);
    }
    route_report(app, &update.request().handoff_id)
}

#[derive(Debug, Clone, Default, Deserialize)]
pub struct DelegateInput {
    pub bot_id: String,
    pub message: String,
    #[serde(default)]
    pub context: String,
    #[serde(default)]
    pub expected_output: String,
    #[serde(default)]
    pub acceptance_criteria: Vec<String>,
    #[serde(default)]
    pub task_id: Option<String>,
}

pub fn delegate(
    app: &Arc<App>,
    from_bot_id: &str,
    chat_id: &str,
    hops: u32,
    input: DelegateInput,
) -> Result<Value, String> {
    let from = app.bot(from_bot_id).ok_or("Requesting bot is gone")?;
    require_runner(app, &from.runner_id)?;
    let target = app.bot(input.bot_id.trim()).ok_or("Unknown target bot")?;
    let chat = app.chat(chat_id).ok_or("Chat is gone")?;
    if !chat.meta.bot_ids.contains(&from.id) {
        return Err("Requesting bot is not in this chat".into());
    }
    if target.id == from.id {
        return Err("You cannot message yourself".into());
    }
    if chat.meta.is_group() && chat.meta.bot_ids.contains(&target.id) {
        return Err(format!(
            "{} is in this chat and reads it. Say it here instead.",
            target.name
        ));
    }
    if hops >= MAX_BOT_HOPS {
        return Err(
            "Bots have passed this along eight times without the user. Answer the user instead."
                .into(),
        );
    }
    if let Some(task) = &input.task_id {
        crate::tasks::get(app, task)?;
    }
    if input.message.trim().is_empty() {
        return Err("message is required".into());
    }
    if input.message.len()
        + input.context.len()
        + input.expected_output.len()
        + input
            .acceptance_criteria
            .iter()
            .map(String::len)
            .sum::<usize>()
        > 64 * 1024
    {
        return Err("A handoff contract is limited to 64 KiB".into());
    }
    let dm = app.dm_with(&target.id, None).map_err(|e| e.to_string())?;
    let request = HandoffRequest {
        handoff_id: format!("handoff-{}", uuid::Uuid::new_v4()),
        attempt: 1,
        job_id: format!("job-{}", uuid::Uuid::new_v4()),
        from_bot_id: from.id,
        source_chat_id: chat_id.into(),
        source_runner_id: from.runner_id,
        task_id: input.task_id,
        target_bot_id: target.id,
        target_chat_id: dm.meta.id,
        target_runner_id: target.runner_id,
        trigger_message_id: format!("msg-{}", uuid::Uuid::new_v4()),
        message: input.message.trim().into(),
        context: input.context,
        expected_output: input.expected_output,
        acceptance_criteria: input.acceptance_criteria,
        hops: hops + 1,
        created_at: now_secs(),
    };
    admit(app, request, None)
}

fn require_runner(app: &App, runner_id: &str) -> Result<(), String> {
    if app.this_device_id().as_deref() != Some(runner_id) {
        return Err("This operation belongs to the bot's assigned Runner".into());
    }
    Ok(())
}

fn request_job(request: &HandoffRequest) -> Job {
    Job {
        id: request.job_id.clone(),
        chat_id: request.target_chat_id.clone(),
        bot_id: request.target_bot_id.clone(),
        kind: "message".into(),
        trigger_message_id: request.trigger_message_id.clone(),
        task_id: request.task_id.clone(),
        task_context: None,
        handoff: Some(HandoffJob::Request {
            request: request.clone(),
        }),
        routine_id: None,
        check: None,
        requested_by: request.source_runner_id.clone(),
        from_bot_id: Some(request.from_bot_id.clone()),
        hops: request.hops,
        round: 0,
        is_winding_down: false,
        setup: None,
        created_at: request.created_at,
    }
}

fn incoming(app: &App, request: &HandoffRequest) {
    if app
        .message(&request.target_chat_id, &request.trigger_message_id)
        .is_some()
    {
        return;
    }
    let mut marker = Message::new(
        &request.target_chat_id,
        Author::Bot {
            bot_id: request.from_bot_id.clone(),
        },
        Body::Handoff {
            from: request.from_bot_id.clone(),
            to: request.target_bot_id.clone(),
            reason: request_text(request),
        },
    );
    marker.id = request.trigger_message_id.clone();
    marker.created_at = request.created_at;
    app.upsert_message(marker, true);
}

/// The request as its "Message from" marker in the recipient's DM shows it, and as the
/// recipient reads it: the message, then what was supplied with it.
pub fn request_text(request: &HandoffRequest) -> String {
    let mut text = request.message.clone();
    if !request.context.trim().is_empty() {
        text.push_str(&format!("\n\nContext:\n{}", request.context.trim()));
    }
    if !request.expected_output.trim().is_empty() {
        text.push_str(&format!(
            "\n\nExpected output:\n{}",
            request.expected_output.trim()
        ));
    }
    if !request.acceptance_criteria.is_empty() {
        text.push_str(&format!(
            "\n\nAcceptance criteria:\n{}",
            request
                .acceptance_criteria
                .iter()
                .map(|s| format!("- {s}"))
                .collect::<Vec<_>>()
                .join("\n")
        ));
    }
    text
}

fn admit(app: &Arc<App>, request: HandoffRequest, replaces: Option<&str>) -> Result<Value, String> {
    let local = app.this_device_id().as_deref() == Some(&request.target_runner_id);
    let mut jobs = Vec::new();
    let delivery = if local {
        "queued_local"
    } else {
        let runner = app
            .device(&request.target_runner_id)
            .filter(|d| d.is_runner() && !d.box_pubkey.is_empty())
            .ok_or("Target Runner is unknown or has no encryption key")?;
        let job = request_job(&request);
        jobs.push(OutboxItem {
            id: request.job_id.clone(),
            kind: "job".into(),
            recipient: Some(runner.id.clone()),
            ciphertext: crate::crypto::seal_json(&runner.box_pubkey, &job)
                .map_err(|e| e.to_string())?,
            slot: None,
            group: None,
        });
        if app.relay_url().is_none() {
            "waiting_for_relay"
        } else if app.device_is_online(&runner.id) {
            "queued_relay"
        } else {
            "waiting_for_runner"
        }
    }
    .to_string();
    let update = HandoffUpdate::Request {
        request: request.clone(),
        delivery,
    };
    let record = {
        let _guard = app.handoff_lock.lock().unwrap();
        if let Some(previous) = replaces {
            let current = get(app, &request.handoff_id)?;
            if current.current().request.job_id != previous {
                return Err("Handoff changed; inspect it and use its current job_id".into());
            }
        }
        let record = merge(app, &update)?;
        save(app, &record, Some(&update), jobs)?;
        record
    };
    incoming(app, &request);
    if local {
        crate::runtime::spawn_local_job(app.clone(), request_job(&request), None);
    }
    Ok(record.view())
}

/// Claims execution before effects start. A duplicate job or a cancelled/superseded attempt
/// never starts another turn; a running claim from a previous process is recovered as failure.
pub fn begin_job(app: &Arc<App>, job: &Job) -> Result<bool, String> {
    let Some(binding) = &job.handoff else {
        return Ok(true);
    };
    let _guard = app.handoff_lock.lock().unwrap();
    match binding {
        HandoffJob::Request { request } => {
            require_runner(app, &request.target_runner_id)?;
            if app
                .bot(&request.target_bot_id)
                .is_some_and(|b| b.runner_id != request.target_runner_id)
            {
                return Err(
                    "The recipient moved to another Runner; inspect and create a new handoff"
                        .into(),
                );
            }
            let update = HandoffUpdate::Request {
                request: request.clone(),
                delivery: "queued_local".into(),
            };
            let mut record = merge(app, &update)?;
            if record.current().request.job_id != job.id || record.current().outcome().is_some() {
                return Ok(false);
            }
            incoming(app, request);
            let started_after = app
                .store
                .page(&job.chat_id, None, 1)
                .map_err(|e| e.to_string())?
                .0
                .last()
                .map(|m| m.id.clone());
            let report = HandoffReport {
                status: HandoffStatus::Running,
                summary: "Runner started the delegated turn".into(),
                result_links: Vec::new(),
                evidence: Vec::new(),
                created_at: now_secs(),
                started_after,
            };
            record.attempts.last_mut().unwrap().report = Some(report.clone());
            save(
                app,
                &record,
                Some(&HandoffUpdate::Report {
                    request: request.clone(),
                    report,
                }),
                Vec::new(),
            )?;
        }
        HandoffJob::Result {
            handoff_id,
            request_job_id,
        } => {
            let mut record = get(app, handoff_id)?;
            let attempt = record
                .attempts
                .iter_mut()
                .find(|a| &a.request.job_id == request_job_id)
                .ok_or("Unknown handoff attempt")?;
            if attempt.result_delivery != ResultDelivery::Pending {
                return Ok(false);
            }
            attempt.result_delivery = ResultDelivery::Started;
            save(app, &record, None, Vec::new())?;
        }
    }
    Ok(true)
}

/// Persist a queued recipient job before it waits for the chat lock. The relay may replay
/// the envelope after process loss; the encrypted claim decides whether it can run again.
pub fn stage_job(app: &App, job: &Job) -> Result<(), String> {
    if let Some(HandoffJob::Request { request }) = &job.handoff {
        let _guard = app.handoff_lock.lock().unwrap();
        let update = HandoffUpdate::Request {
            request: request.clone(),
            delivery: "queued_local".into(),
        };
        let record = merge(app, &update)?;
        save(app, &record, None, Vec::new())?;
    }
    Ok(())
}

pub fn finish_job(
    app: &Arc<App>,
    job: &Job,
    outcome: TurnOutcome,
    cancelled: bool,
) -> Result<(), String> {
    match &job.handoff {
        Some(HandoffJob::Request { request }) => {
            let record = get(app, &request.handoff_id)?;
            let attempt = record
                .attempts
                .iter()
                .find(|a| a.request.job_id == job.id)
                .ok_or("Unknown handoff attempt")?;
            if attempt.outcome().is_some_and(|r| r.status.terminal()) {
                return route_report(app, &record.id);
            }
            let (links, evidence, said, failure) = turn_evidence(app, attempt);
            // A turn that failed ends Skipped (`turns::run_job`); a notice it posted on the way,
            // such as a compaction, is no failure by itself. One its limits stopped waits on the
            // user, who can resume it in Limits (`resume_stopped`).
            let limited = outcome == TurnOutcome::Skipped && stopped_at_limits(app, job);
            let status = if cancelled {
                HandoffStatus::Cancelled
            } else if limited {
                HandoffStatus::Blocked
            } else if outcome == TurnOutcome::Skipped {
                HandoffStatus::Failed
            } else if outcome == TurnOutcome::Sent {
                HandoffStatus::Completed
            } else {
                HandoffStatus::Blocked
            };
            // The first line is what the requesting chat's marker shows.
            let summary = match status {
                HandoffStatus::Cancelled => "Stopped before finishing.".into(),
                HandoffStatus::Failed => failure.unwrap_or_else(|| "Couldn't finish.".into()),
                HandoffStatus::Blocked if limited => failure.unwrap_or_else(|| "Stopped at its limits.".into()),
                HandoffStatus::Blocked => "Ended the turn without a reply.".into(),
                _ => said.unwrap_or_else(|| "Done.".into()),
            };
            let report = HandoffReport {
                status,
                summary,
                result_links: links,
                evidence,
                created_at: now_secs(),
                started_after: attempt
                    .report
                    .as_ref()
                    .and_then(|r| r.started_after.clone()),
            };
            publish_report(app, request.clone(), report)
        }
        Some(HandoffJob::Result {
            handoff_id,
            request_job_id,
        }) => {
            let _guard = app.handoff_lock.lock().unwrap();
            let mut record = get(app, handoff_id)?;
            if let Some(attempt) = record
                .attempts
                .iter_mut()
                .find(|a| a.request.job_id == *request_job_id)
            {
                attempt.result_delivery = ResultDelivery::Finished;
                save(app, &record, None, Vec::new())?;
            }
            Ok(())
        }
        None => Ok(()),
    }
}

/// The turn's own limits, or its task's, stopped it on this Runner.
fn stopped_at_limits(app: &App, job: &Job) -> bool {
    app.budgets.local_snapshots(app).iter().any(|budget| {
        matches!(budget.state.as_str(), "budget_exhausted" | "interrupted")
            && ((budget.kind == "job" && budget.id == job.id)
                || (budget.kind == "task" && job.task_id.as_ref() == Some(&budget.id)))
    })
}

/// Resuming a delegated turn in Limits after its limits stopped it. That attempt already
/// reported back as blocked, and a report never changes, so the work goes on as the handoff's
/// next attempt, with the same request and no new marker in the recipient's DM; its result
/// reaches the requesting bot like any other. Any other turn goes on as it is.
pub fn resume_stopped(app: &Arc<App>, job: Job) -> Result<(), String> {
    let Some(HandoffJob::Request { request }) = &job.handoff else {
        crate::runtime::start_turn(app, job);
        return Ok(());
    };
    let next = {
        let _guard = app.handoff_lock.lock().unwrap();
        let record = get(app, &request.handoff_id)?;
        let attempt = record.current();
        if attempt.request.job_id != job.id || attempt.cancellation.is_some() {
            return Err("This handoff was cancelled or sent again since, so there is nothing to resume.".into());
        }
        if !attempt.report.as_ref().is_some_and(|r| r.status.terminal()) {
            drop(_guard);
            crate::runtime::start_turn(app, job);
            return Ok(());
        }
        let mut next = attempt.request.clone();
        next.attempt = next.attempt.checked_add(1).ok_or("Too many handoff attempts")?;
        next.job_id = format!("job-{}", uuid::Uuid::new_v4());
        next.created_at = now_secs();
        next
    };
    admit(app, next, Some(&job.id)).map(|_| ())
}

fn turn_evidence(
    app: &App,
    attempt: &HandoffAttempt,
) -> (Vec<ResultLink>, Vec<String>, Option<String>, Option<String>) {
    let request = &attempt.request;
    let after = attempt
        .report
        .as_ref()
        .and_then(|r| r.started_after.as_deref())
        .unwrap_or(&request.trigger_message_id);
    let messages = app
        .store
        .messages_after(&request.target_chat_id, after)
        .unwrap_or_default();
    let (mut links, mut evidence, mut said, mut failure, mut notice) = (Vec::new(), Vec::new(), None, None, None);
    for message in messages {
        // Why a turn that never ran stopped ("cannot run yet: … Connect it in Settings").
        if let (Author::System, Body::Notice { text, .. }) = (&message.author, &message.body) {
            notice = Some(text.clone());
        }
        if message.author
            != (Author::Bot {
                bot_id: request.target_bot_id.clone(),
            })
        {
            continue;
        }
        if let MessageState::Failed { error } = &message.state {
            failure = Some(error.clone());
        }
        match &message.body {
            Body::Text { text, .. } if !text.trim().is_empty() && message.is_complete() => {
                said = Some(text.chars().take(8000).collect::<String>());
                links.push(ResultLink::message(&message, "Recipient response".into()));
            }
            Body::Tool {
                name,
                summary,
                is_running: false,
                is_error,
                ..
            } => {
                evidence.push(format!(
                    "{}: {} ({}, message {})",
                    name,
                    summary,
                    if *is_error { "error" } else { "returned" },
                    message.id
                ));
            }
            _ => {}
        }
        // A published output version is its own message; the report names that version.
        if let Some(output) = &message.output {
            let mut link = ResultLink::message(&message, output.name.clone());
            link.kind = "output".into();
            link.output_id = Some(output.id.clone());
            link.version = Some(output.version);
            links.push(link);
        }
    }
    if links.is_empty() {
        links.push(request_link(request));
    }
    links.truncate(40);
    evidence.truncate(40);
    (links, evidence, said, failure.or(notice))
}

fn publish_report(
    app: &Arc<App>,
    request: HandoffRequest,
    report: HandoffReport,
) -> Result<(), String> {
    {
        let _guard = app.handoff_lock.lock().unwrap();
        let update = HandoffUpdate::Report {
            request: request.clone(),
            report,
        };
        let existing = get(app, &request.handoff_id)?;
        if !existing
            .attempts
            .iter()
            .find(|a| a.request.job_id == request.job_id)
            .and_then(|a| a.report.as_ref())
            .is_some_and(|r| r.status.terminal())
        {
            let record = merge(app, &update)?;
            save(app, &record, Some(&update), Vec::new())?;
        }
    }
    route_report(app, &request.handoff_id)
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ReportInput {
    pub status: HandoffStatus,
    pub summary: String,
    #[serde(default)]
    pub result_links: Vec<ResultLink>,
    #[serde(default)]
    pub evidence: Vec<String>,
}

pub fn report(
    app: &Arc<App>,
    bot_id: &str,
    handoff_id: &str,
    job_id: &str,
    input: ReportInput,
) -> Result<Value, String> {
    if serde_json::to_vec(&input).map_err(|e| e.to_string())?.len() > 64 * 1024 {
        return Err("Handoff report is limited to 64 KiB".into());
    }
    let record = get(app, handoff_id)?;
    let attempt = record.current();
    let request = &attempt.request;
    if request.target_bot_id != bot_id || request.job_id != job_id {
        return Err("Only the recipient's active attempt can report this handoff".into());
    }
    require_runner(app, &request.target_runner_id)?;
    if !input.status.terminal() || input.summary.trim().is_empty() {
        return Err("A terminal status and summary are required".into());
    }
    if attempt.outcome().is_some_and(|r| r.status.terminal()) {
        return Err("This attempt already has a terminal report".into());
    }
    if input.summary.len() + input.evidence.iter().map(String::len).sum::<usize>() > 32 * 1024
        || input.result_links.len() > 40
    {
        return Err("Handoff report is too large".into());
    }
    for link in &input.result_links {
        link.validate()?;
    }
    let (automatic, observed, _, _) = turn_evidence(app, attempt);
    let mut links = input.result_links;
    for link in automatic {
        if !links.contains(&link) && links.len() < 40 {
            links.push(link);
        }
    }
    let mut evidence = input.evidence;
    evidence.extend(observed);
    publish_report(
        app,
        request.clone(),
        HandoffReport {
            status: input.status,
            summary: input.summary.trim().into(),
            result_links: links,
            evidence,
            created_at: now_secs(),
            started_after: attempt
                .report
                .as_ref()
                .and_then(|r| r.started_after.clone()),
        },
    )?;
    Ok(get(app, handoff_id)?.view())
}

/// The report in the requesting chat: a "Message from ◉ Specialist" marker whose text is the
/// summary, which the requesting bot reads as "[Message from Specialist]: …". Its links and
/// evidence reach that bot in the continuation's system prompt (`prompt`).
fn result_message(request: &HandoffRequest, report: &HandoffReport) -> Message {
    let mut message = Message::new(
        &request.source_chat_id,
        Author::Bot {
            bot_id: request.target_bot_id.clone(),
        },
        Body::Handoff {
            from: request.target_bot_id.clone(),
            to: request.from_bot_id.clone(),
            reason: report.summary.clone(),
        },
    );
    message.id = format!("report-{}", request.job_id);
    message.created_at = report.created_at;
    message
}

fn result_job(request: &HandoffRequest) -> Job {
    let mut job = request_job(request);
    job.id = format!("result-{}", request.job_id);
    job.chat_id = request.source_chat_id.clone();
    job.bot_id = request.from_bot_id.clone();
    job.kind = "handoff_result".into();
    job.trigger_message_id = format!("report-{}", request.job_id);
    job.from_bot_id = None;
    job.handoff = Some(HandoffJob::Result {
        handoff_id: request.handoff_id.clone(),
        request_job_id: request.job_id.clone(),
    });
    job
}

fn route_report(app: &Arc<App>, id: &str) -> Result<(), String> {
    let record = get(app, id)?;
    let attempt = record.current();
    let request = &attempt.request;
    // Only the origin Runner wakes its coordinator. Other paired Devices retain the report.
    if app.this_device_id().as_deref() != Some(request.source_runner_id.as_str()) {
        return Ok(());
    }
    // The requesting bot cancelled this attempt itself: there is nothing to tell it.
    if attempt.cancellation.is_some() {
        return Ok(());
    }
    let Some(report) = attempt.outcome().filter(|r| r.status.terminal()) else {
        return Ok(());
    };
    if app.chat(&request.source_chat_id).is_none() {
        return Ok(());
    }
    let message = result_message(request, report);
    app.upsert_message(message.clone(), true);
    if attempt.result_delivery == ResultDelivery::Pending
        && app
            .bot(&request.from_bot_id)
            .is_some_and(|b| b.runner_id == request.source_runner_id)
    {
        let job = result_job(request);
        if !app.running_jobs.lock().unwrap().contains_key(&job.id) {
            if let Some(task) = request.task_id.clone() {
                let (app, name) = (app.clone(), crate::runtime::name_of(app, &request.target_bot_id));
                tokio::spawn(async move { record_on_task(&app, &task, &message, &name).await });
            }
            crate::runtime::spawn_local_job(app.clone(), job, None);
        }
    }
    Ok(())
}

/// A report for a task's handoff joins the task's evidence as the report message, through the
/// task's own revision check, as a review's outcome does. Retried once when the task moved
/// meanwhile; a task whose linked chats leave out the requesting chat, or that refuses the
/// change, keeps the report in the chat only.
async fn record_on_task(app: &Arc<App>, task_id: &str, report: &Message, name: &str) {
    let Body::Handoff { reason, .. } = &report.body else { return };
    let first = reason.lines().map(str::trim).find(|line| !line.is_empty()).unwrap_or_default();
    let label: String = format!("{name}: {first}").chars().take(200).collect();
    for _ in 0..2 {
        let Ok(task) = crate::tasks::get(app, task_id) else { return };
        if !task.chat_ids.contains(&report.chat_id)
            || task.evidence.iter().any(|e| e.message_id.as_deref() == Some(report.id.as_str()))
        {
            return;
        }
        let mut evidence = serde_json::to_value(&task.evidence).unwrap_or_else(|_| json!([]));
        if let Some(list) = evidence.as_array_mut() {
            list.push(json!({ "kind": "message", "label": label, "chat_id": report.chat_id, "message_id": report.id }));
        }
        let update = json!({ "id": task_id, "expected_revision": task.revision,
            "request_id": format!("{}-task-{}", report.id, task.revision), "evidence": evidence });
        match Box::pin(crate::tasks::dispatch(app, "tasks.update", update)).await {
            Ok(_) => return,
            Err(error) if error.contains("revision conflict") => continue,
            Err(error) => {
                tracing::warn!(%error, message_id = %report.id, "recording a handoff report on its task");
                return;
            }
        }
    }
}

pub fn follow_up(
    app: &Arc<App>,
    bot_id: &str,
    id: &str,
    expected_job_id: &str,
    message: &str,
) -> Result<Value, String> {
    let _guard = app.handoff_lock.lock().unwrap();
    let record = get(app, id)?;
    let attempt = record.current();
    let mut request = attempt.request.clone();
    if request.from_bot_id != bot_id {
        return Err("Only the requesting bot can follow up".into());
    }
    require_runner(app, &request.source_runner_id)?;
    if request.job_id != expected_job_id {
        return Err("Handoff changed; inspect it and use its current job_id".into());
    }
    if !attempt.outcome().is_some_and(|r| r.status.terminal()) {
        return Err(
            "This handoff is still outstanding; cancel it before replacing the request".into(),
        );
    }
    if message.trim().is_empty() || message.len() > 32 * 1024 {
        return Err("A follow-up message of at most 32 KiB is required".into());
    }
    request.attempt = request
        .attempt
        .checked_add(1)
        .ok_or("Too many handoff attempts")?;
    request.job_id = format!("job-{}", uuid::Uuid::new_v4());
    request.trigger_message_id = format!("msg-{}", uuid::Uuid::new_v4());
    request.message = message.trim().into();
    request.created_at = now_secs();
    drop(_guard);
    admit(app, request, Some(expected_job_id))
}

pub fn cancel(
    app: &Arc<App>,
    bot_id: &str,
    id: &str,
    expected_job_id: &str,
    reason: &str,
) -> Result<Value, String> {
    let request;
    {
        let _guard = app.handoff_lock.lock().unwrap();
        let record = get(app, id)?;
        let attempt = record.current();
        request = attempt.request.clone();
        if request.from_bot_id != bot_id {
            return Err("Only the requesting bot can cancel".into());
        }
        require_runner(app, &request.source_runner_id)?;
        if request.job_id != expected_job_id {
            return Err("Handoff changed; inspect it and use its current job_id".into());
        }
        if reason.trim().is_empty() || reason.len() > 32 * 1024 {
            return Err("Cancellation needs a reason of at most 32 KiB".into());
        }
        if attempt.outcome().is_some_and(|r| r.status.terminal()) {
            return Ok(record.view());
        }
        let report = HandoffReport {
            status: HandoffStatus::Cancelled,
            summary: reason.into(),
            result_links: vec![request_link(&request)],
            evidence: vec!["Cancelled by the requesting bot".into()],
            created_at: now_secs(),
            started_after: None,
        };
        let update = HandoffUpdate::Cancel {
            request: request.clone(),
            report,
        };
        let record = merge(app, &update)?;
        let mut outbox = Vec::new();
        if request.target_runner_id != request.source_runner_id {
            let runner = app
                .device(&request.target_runner_id)
                .ok_or("Target Runner is unknown")?;
            outbox.push(OutboxItem {
                id: uuid::Uuid::new_v4().to_string(),
                kind: "job_cancel".into(),
                recipient: Some(runner.id),
                ciphertext: crate::crypto::seal_json(
                    &runner.box_pubkey,
                    &JobCancel {
                        job_id: request.job_id.clone(),
                    },
                )
                .map_err(|e| e.to_string())?,
                slot: None,
                group: None,
            });
        }
        save(app, &record, Some(&update), outbox)?;
    }
    app.cancel_job(&request.job_id);
    route_report(app, id)?;
    Ok(get(app, id)?.view())
}

/// Restarts queued local work and delivery of reports. A recorded running turn becomes an
/// explicit failure after process loss, so side effects are never automatically replayed.
pub fn resume(app: &Arc<App>) -> Result<(), String> {
    let Some(device) = app.this_device_id() else {
        return Ok(());
    };
    for record in list(app)? {
        let attempt = record.current();
        let request = &attempt.request;
        if request.target_runner_id == device {
            if attempt.outcome().is_none() {
                crate::runtime::spawn_local_job(app.clone(), request_job(request), None);
            } else if attempt
                .outcome()
                .is_some_and(|r| r.status == HandoffStatus::Running)
            {
                let report = HandoffReport { status: HandoffStatus::Failed, summary: "Stopped when Lorca restarted on its Runner.".into(),
                    result_links: turn_evidence(app, attempt).0, evidence: vec!["The turn was running when the Runner's Lorca restarted; what it did before is in the recipient's chat and is not run again".into()], created_at: now_secs(), started_after: None };
                publish_report(app, request.clone(), report)?;
            }
        }
        // A continuation that started is not run again: like any turn cut off by a restart, it
        // may have acted already, and its report stays in the chat.
        if request.source_runner_id == device && attempt.result_delivery == ResultDelivery::Started
        {
            let _guard = app.handoff_lock.lock().unwrap();
            let mut current = get(app, &record.id)?;
            if let Some(attempt) = current
                .attempts
                .iter_mut()
                .find(|a| a.request.job_id == request.job_id)
            {
                attempt.result_delivery = ResultDelivery::Finished;
            }
            save(app, &current, None, Vec::new())?;
        }
        route_report(app, &record.id)?;
    }
    Ok(())
}

pub fn dispatch(app: &Arc<App>, method: &str, params: Value) -> Result<Value, String> {
    let required = |key: &str| {
        params[key]
            .as_str()
            .filter(|s| !s.is_empty())
            .ok_or_else(|| format!("missing {key}"))
    };
    match method {
        "handoffs.list" => {
            let records: Vec<_> = list(app)?
                .iter()
                .filter(|h| {
                    let a = h.current();
                    params["bot_id"].as_str().is_none_or(|id| {
                        a.request.from_bot_id == id || a.request.target_bot_id == id
                    }) && params["chat_id"].as_str().is_none_or(|id| {
                        a.request.source_chat_id == id || a.request.target_chat_id == id
                    }) && params["task_id"]
                        .as_str()
                        .is_none_or(|id| a.request.task_id.as_deref() == Some(id))
                        && (!params["outstanding"].as_bool().unwrap_or(false)
                            || !a.outcome().is_some_and(|r| {
                                matches!(
                                    r.status,
                                    HandoffStatus::Completed
                                        | HandoffStatus::Failed
                                        | HandoffStatus::Cancelled
                                )
                            }))
                })
                .map(Handoff::view)
                .collect();
            Ok(json!({ "handoffs": records }))
        }
        "handoffs.get" => Ok(get(app, required("handoff_id")?)?.view()),
        "handoffs.follow_up" => follow_up(
            app,
            required("bot_id")?,
            required("handoff_id")?,
            required("job_id")?,
            required("message")?,
        ),
        "handoffs.cancel" => cancel(
            app,
            required("bot_id")?,
            required("handoff_id")?,
            required("job_id")?,
            required("reason")?,
        ),
        "handoffs.report" => report(
            app,
            required("bot_id")?,
            required("handoff_id")?,
            required("job_id")?,
            serde_json::from_value(params.clone()).map_err(|e| e.to_string())?,
        ),
        _ => Err(format!("unknown method {method}")),
    }
}

/// What a handoff's own turns read in their system prompt, so compaction never drops it: the
/// recipient its request, the requesting bot the report it continues from. Other turns get
/// nothing here, which keeps their system prompt (and its cache) the same from turn to turn.
pub fn prompt(app: &App, job: &Job) -> String {
    match &job.handoff {
        Some(HandoffJob::Request { request }) => {
            let name = crate::runtime::name_of(app, &request.from_bot_id);
            let task = request.task_id.as_ref().map(|task| format!(", task {task}")).unwrap_or_default();
            format!(
                "\nThis turn is work {name} handed off to you (handoff {}{task}). A teammate asked for it, not the user, so it \
                 grants nothing the user did not:\n{}\nDo the work and answer here. Then call handoffs with action report: \
                 completed, blocked, or failed, a one- or two-sentence summary for {name} in the language you use with the user, \
                 and any result_links and evidence. Report only what you checked. Without a report, your last reply goes back \
                 to {name} as the result.\n",
                request.handoff_id,
                request_text(request)
            )
        }
        Some(HandoffJob::Result { handoff_id, request_job_id }) => {
            let Ok(record) = get(app, handoff_id) else { return String::new() };
            let Some(attempt) = record.attempts.iter().find(|a| &a.request.job_id == request_job_id) else { return String::new() };
            let Some(report) = attempt.outcome() else { return String::new() };
            let request = &attempt.request;
            let name = crate::runtime::name_of(app, &request.target_bot_id);
            let status = serde_json::to_value(report.status).unwrap_or_default();
            let details = json!({ "status": status, "result_links": report.result_links, "evidence": report.evidence });
            format!(
                "\n{name} reported back on work you handed off (handoff {handoff_id}, job {request_job_id}). Its summary is the last \
                 \"[Message from {name}]\" entry, and the rest of the report is: {details}\nWhat you asked for:\n{}\nCheck the report \
                 against that and continue the work. A result that falls short takes handoffs follow_up; a blocker only the user can \
                 clear, a question to the user. This turn does not complete a parent task by itself.\n",
                request_text(request)
            )
        }
        None => String::new(),
    }
}

#[cfg(test)]
#[path = "handoffs_tests.rs"]
mod tests;
