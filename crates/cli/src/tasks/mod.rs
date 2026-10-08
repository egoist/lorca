//! Durable work, separate from a Job's single turn. One fixed authority Runner serializes
//! revisions; the assigned Runner alone claims and executes a run. All stored payloads and
//! sync content use the account key. Remote mutations fail explicitly when authority is offline.
mod execution;
mod model;
pub use execution::start;
pub(crate) use execution::{admit, claim, finished};
#[cfg(test)]
use execution::{budget_block_reason, deliver_finish, recover, Finish};
use execution::{cancel_run, finish_here};
pub(crate) mod storage;
#[cfg(feature = "runner")]
mod tool;
pub use model::*;
#[cfg(feature = "runner")]
pub(crate) use tool::TasksTool;

use crate::{
    app::{App, OutboxItem, Slot},
    config::now_secs,
    crypto,
    model::{Author, Body, Job},
    runtime::TurnOutcome,
};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::sync::Arc;

const KIND: &str = "task";
#[cfg(any(feature = "runner", test))]
const CONTEXT_MARKER: &str = "[Durable tasks (current records;";

fn key(app: &App) -> Result<[u8; 32], String> {
    app.dek()
        .ok_or_else(|| "Create or pair an identity first.".into())
}
fn err(error: impl std::fmt::Display) -> String {
    error.to_string()
}
fn text(params: &Value, field: &str) -> Result<String, String> {
    params[field]
        .as_str()
        .filter(|s| !s.trim().is_empty())
        .map(str::to_string)
        .ok_or_else(|| format!("missing {field}"))
}

pub fn get(app: &App, id: &str) -> Result<Task, String> {
    let bytes = app
        .store
        .task_row(id)
        .map_err(err)?
        .ok_or_else(|| format!("Unknown task {id}"))?;
    crypto::decrypt_json(&key(app)?, KIND, &bytes).map_err(err)
}

pub fn list(app: &App) -> Result<Vec<Task>, String> {
    let Some(dek) = app.dek() else {
        return Ok(Vec::new());
    };
    app.store
        .task_rows()
        .map_err(err)?
        .iter()
        .map(|bytes| crypto::decrypt_json(&dek, KIND, bytes).map_err(err))
        .collect()
}

/// Public app and CLI API. Mutations are forwarded to the task's initial Runner, including
/// after an owner change. A fresh read is necessary after a revision conflict.
pub async fn dispatch(app: &Arc<App>, method: &str, params: Value) -> Result<Value, String> {
    if method == "tasks.list" {
        return read(app, method, &params);
    }
    if method == "tasks.get" {
        let task = get(app, &text(&params, "id")?)?;
        if params["refresh"] == true
            && app.this_device_id().as_deref() != Some(task.authority_runner_id.as_str())
        {
            let value =
                crate::requests::ask(app, &task.authority_runner_id, method, params).await?;
            apply(app, serde_json::from_value(value.clone()).map_err(err)?)?;
            return Ok(value);
        }
        return Ok(json!(task));
    }
    if !matches!(method, "tasks.create" | "tasks.update" | "tasks.run") {
        return Err(format!("Unknown task method {method}"));
    }
    let authority = if method == "tasks.create" {
        app.bot(&text(&params, "owner_bot_id")?)
            .ok_or("Unknown owning bot")?
            .runner_id
    } else {
        get(app, &text(&params, "id")?)?.authority_runner_id
    };
    if app.this_device_id().as_deref() == Some(authority.as_str()) {
        mutate(app, method, &params)
    } else {
        let reply = crate::requests::ask(app, &authority, method, params).await?;
        if let Ok(task) = serde_json::from_value::<Task>(reply.clone()) {
            apply(app, task)?;
        }
        Ok(reply)
    }
}

fn read(app: &App, method: &str, params: &Value) -> Result<Value, String> {
    if method == "tasks.get" {
        return Ok(json!(get(app, &text(params, "id")?)?));
    }
    let tasks: Vec<Task> = list(app)?
        .into_iter()
        .filter(|task| {
            params["chat_id"]
                .as_str()
                .is_none_or(|id| task.chat_ids.iter().any(|c| c == id))
                && params["owner_bot_id"]
                    .as_str()
                    .is_none_or(|id| task.owner_bot_id == id)
                && params["state"].as_str().is_none_or(|s| {
                    serde_json::to_value(task.state)
                        .ok()
                        .and_then(|v| v.as_str().map(str::to_string))
                        .as_deref()
                        == Some(s)
                })
        })
        .collect();
    Ok(json!(tasks))
}

/// Sealed request handling, distinct from the public API's forwarding boundary.
pub(crate) fn serve(
    app: &Arc<App>,
    method: &str,
    params: &Value,
    sender: &str,
) -> Result<Value, String> {
    match method {
        "tasks.get" | "tasks.list" => read(app, method, params),
        "tasks.create" | "tasks.update" | "tasks.run" => mutate(app, method, params),
        "tasks.validate_run" => {
            let task = get(app, &text(params, "id")?)?;
            authority_here(app, &task)?;
            validate(app, &task, Some(&task))?;
            let run = task.active_run.as_ref().ok_or("Task has no active run")?;
            if app
                .chat(&run.chat_id)
                .is_none_or(|chat| !chat.meta.bot_ids.contains(&run.bot_id))
            {
                return Err("The task's owning bot no longer belongs to its run chat.".into());
            }
            if run.id != text(params, "run_id")?
                || run.runner_id != sender
                || task.state != TaskState::Working
            {
                return Err("Task run is no longer authorized.".into());
            }
            Ok(json!(task))
        }
        "tasks.finish_run" => finish_here(
            app,
            serde_json::from_value::<execution::Finish>(params.clone()).map_err(err)?,
            sender,
        ),
        _ => Err(format!("Unknown task request {method}")),
    }
}

fn authority_here(app: &App, task: &Task) -> Result<(), String> {
    if app.this_device_id().as_deref() != Some(task.authority_runner_id.as_str()) {
        return Err("This Runner is not the task authority.".into());
    }
    Ok(())
}

#[derive(Serialize, Deserialize)]
struct Receipt {
    method: String,
    params: Value,
    task: Task,
}

fn receipt(app: &App, method: &str, params: &Value) -> Result<Option<Task>, String> {
    let request_id = text(params, "request_id")?;
    if request_id.len() > 160 {
        return Err("request_id is too long".into());
    }
    let Some(bytes) = app.store.task_receipt(&request_id).map_err(err)? else {
        return Ok(None);
    };
    let saved: Receipt = crypto::decrypt_json(&key(app)?, "task-receipt", &bytes).map_err(err)?;
    if saved.method != method || saved.params != *params {
        return Err("request_id already belongs to a different task mutation.".into());
    }
    Ok(Some(saved.task))
}

fn mutate(app: &Arc<App>, method: &str, params: &Value) -> Result<Value, String> {
    let (task, job, cancel) = {
        let _order = app.task_order.lock().unwrap();
        if let Some(task) = receipt(app, method, params)? {
            return Ok(json!(task));
        }
        let (mut task, expected, before) = if method == "tasks.create" {
            (create(app, params)?, None, None)
        } else {
            let mut task = get(app, &text(params, "id")?)?;
            authority_here(app, &task)?;
            let expected = params["expected_revision"]
                .as_u64()
                .ok_or("missing expected_revision")?;
            if expected != task.revision {
                return Err(format!(
                    "Task revision conflict: expected {expected}, current {}. Read the task again.",
                    task.revision
                ));
            }
            let before = task.clone();
            if method == "tasks.update" {
                patch(app, &mut task, params)?;
            }
            (task, Some(expected), Some(before))
        };
        // Scope/assignment validation precedes run's default chat selection.
        validate(app, &task, before.as_ref())?;
        let job = if method == "tasks.run" {
            Some(run(app, &mut task, params)?)
        } else {
            None
        };
        validate(app, &task, before.as_ref())?;
        task.updated_at = now_secs();
        task.revision = expected
            .unwrap_or(0)
            .checked_add(1)
            .ok_or("Task revision overflow")?;
        task.record_change();
        let job = job.map(|mut job| {
            job.task_context = Some(task.clone());
            job
        });
        // The run that records its own outcome ends its turn itself (the tool's result
        // terminates it); cancelling would stop its commands and mark the turn stopped.
        let cancel = task
            .active_run
            .as_ref()
            .filter(|run| task.state != TaskState::Working && params["run_id"].as_str() != Some(run.id.as_str()))
            .cloned();
        commit(app, &task, expected, Some((method, params)), job.as_ref())?;
        (task, job, cancel)
    };
    changed(app, &task);
    if let Some(run) = cancel {
        cancel_run(app, &run);
    }
    if let Some(job) = job.filter(|_| task.runner_id == app.this_device_id().unwrap_or_default()) {
        crate::runtime::start_turn(app, job);
    }
    Ok(json!(task))
}

fn create(app: &App, params: &Value) -> Result<Task, String> {
    let owner = app
        .bot(&text(params, "owner_bot_id")?)
        .ok_or("Unknown owning bot")?;
    if app.this_device_id().as_deref() != Some(owner.runner_id.as_str()) {
        return Err("Task creation belongs to the owning bot's Runner.".into());
    }
    let id = params["id"]
        .as_str()
        .map(str::to_string)
        .unwrap_or_else(|| format!("task-{}", uuid::Uuid::new_v4()));
    if !id
        .strip_prefix("task-")
        .is_some_and(|s| uuid::Uuid::parse_str(s).is_ok())
    {
        return Err("Task id must be task-<UUID>.".into());
    }
    if app.store.task_row(&id).map_err(err)?.is_some() {
        return Err("Task id already exists; reuse the original request_id for a retry.".into());
    }
    Ok(Task {
        id,
        revision: 0,
        authority_runner_id: owner.runner_id.clone(),
        owner_bot_id: owner.id,
        runner_id: owner.runner_id,
        goal: text(params, "goal")?,
        acceptance_criteria: serde_json::from_value(params["acceptance_criteria"].clone())
            .map_err(err)?,
        dependencies: serde_json::from_value(
            params.get("dependencies").cloned().unwrap_or(json!([])),
        )
        .map_err(err)?,
        next_action: text(params, "next_action")?,
        chat_ids: serde_json::from_value(params["chat_ids"].clone()).map_err(err)?,
        links: serde_json::from_value(params.get("links").cloned().unwrap_or(json!([])))
            .map_err(err)?,
        state: TaskState::Queued,
        reason: None,
        result: None,
        evidence: Vec::new(),
        active_run: None,
        history: Vec::new(),
        created_at: now_secs(),
        updated_at: now_secs(),
    })
}

fn patch(app: &App, task: &mut Task, params: &Value) -> Result<(), String> {
    for field in ["goal", "next_action"] {
        if params.get(field).is_some() {
            let value = text(params, field)?;
            if field == "goal" {
                task.goal = value
            } else {
                task.next_action = value
            }
        }
    }
    for field in ["reason", "result"] {
        if let Some(value) = params.get(field) {
            let value: Option<String> = serde_json::from_value(value.clone()).map_err(err)?;
            if field == "reason" {
                task.reason = value
            } else {
                task.result = value
            }
        }
    }
    if task.active_run.is_some()
        && [
            "owner_bot_id",
            "runner_id",
            "chat_ids",
            "dependencies",
            "acceptance_criteria",
        ]
        .iter()
        .any(|f| params.get(*f).is_some())
    {
        return Err("Stop the active run and wait for it to finish before changing ownership, scope, or dependencies.".into());
    }
    if let Some(value) = params.get("owner_bot_id") {
        let owner = app
            .bot(value.as_str().ok_or("owner_bot_id must be a string")?)
            .ok_or("Unknown owning bot")?;
        task.owner_bot_id = owner.id;
        task.runner_id = owner.runner_id;
    }
    if let Some(value) = params.get("runner_id") {
        task.runner_id = value.as_str().ok_or("runner_id must be a string")?.into();
    }
    if let Some(value) = params.get("state") {
        let state: TaskState = serde_json::from_value(value.clone()).map_err(err)?;
        if state == TaskState::Working && task.state != TaskState::Working {
            return Err("A task starts working only through tasks run.".into());
        }
        if state != task.state && params.get("reason").is_none() {
            task.reason = None;
        }
        task.state = state;
    }
    if let Some(v) = params.get("acceptance_criteria") {
        task.acceptance_criteria = serde_json::from_value(v.clone()).map_err(err)?;
    }
    if let Some(v) = params.get("dependencies") {
        task.dependencies = serde_json::from_value(v.clone()).map_err(err)?;
    }
    if let Some(v) = params.get("chat_ids") {
        task.chat_ids = serde_json::from_value(v.clone()).map_err(err)?;
    }
    if let Some(v) = params.get("links") {
        task.links = serde_json::from_value(v.clone()).map_err(err)?;
    }
    if let Some(v) = params.get("evidence") {
        task.evidence = serde_json::from_value(v.clone()).map_err(err)?;
    }
    Ok(())
}

fn nonempty(s: &str) -> bool {
    !s.trim().is_empty()
}
fn https(url: &str) -> bool {
    reqwest::Url::parse(url).is_ok_and(|u| {
        u.scheme() == "https"
            && u.host_str().is_some()
            && u.username().is_empty()
            && u.password().is_none()
    })
}

/// `before` is the record the edit started from. Evidence it already held was resolved when it
/// was added (or arrived with a remote run's outcome ahead of its chat blobs), so only new
/// references resolve now, and every reference does when the edit completes the task.
fn validate(app: &App, task: &Task, before: Option<&Task>) -> Result<(), String> {
    if !nonempty(&task.goal)
        || !nonempty(&task.next_action)
        || task.acceptance_criteria.is_empty()
        || task.acceptance_criteria.iter().any(|s| !nonempty(s))
    {
        return Err(
            "Goal, next action, and at least one acceptance criterion are required.".into(),
        );
    }
    if task.goal.len() > 16_000
        || task.next_action.len() > 8_000
        || task.acceptance_criteria.len() > 64
        || task.acceptance_criteria.iter().any(|s| s.len() > 4000)
    {
        return Err(
            "Task goal, next action, or acceptance criteria exceed their size limit.".into(),
        );
    }
    let bot = app.bot(&task.owner_bot_id).ok_or("Unknown owning bot")?;
    if bot.runner_id != task.runner_id {
        return Err("Task Runner must match the owning bot's assigned Runner.".into());
    }
    if !app.device(&task.runner_id).is_some_and(|d| d.is_runner()) {
        return Err("Task must be assigned to a Runner.".into());
    }
    if task.chat_ids.is_empty()
        || task.chat_ids.len() > 64
        || task.dependencies.len() > 64
        || task.links.len() > 64
        || task.evidence.len() > 128
    {
        return Err("Task needs linked chats and at most 64 chats/dependencies/links and 128 evidence references.".into());
    }
    let mut has_run_chat = false;
    for id in &task.chat_ids {
        let chat = app
            .chat(id)
            .ok_or_else(|| format!("Unknown linked chat {id}"))?;
        has_run_chat |= chat.meta.bot_ids.contains(&task.owner_bot_id);
    }
    if !has_run_chat {
        return Err(
            "The owning bot must belong to at least one linked chat in which it can run.".into(),
        );
    }
    for link in &task.links {
        if !https(&link.url) || !nonempty(&link.label) {
            return Err("External links require a label and an HTTPS URL.".into());
        }
    }
    if matches!(task.state, TaskState::Blocked | TaskState::Cancelled)
        && !task.reason.as_deref().is_some_and(nonempty)
    {
        return Err("Blocked and cancelled tasks require a reason.".into());
    }
    if task.state == TaskState::Completed
        && (!task.result.as_deref().is_some_and(nonempty) || task.evidence.is_empty())
    {
        return Err("Completion requires a result and supporting evidence.".into());
    }
    if task.result.as_ref().is_some_and(|s| s.len() > 32_000)
        || task.reason.as_ref().is_some_and(|s| s.len() > 8_000)
    {
        return Err("Task result or reason is too long.".into());
    }
    dependencies(app, task)?;
    let completing = task.state == TaskState::Completed && before.is_none_or(|b| b.state != TaskState::Completed);
    for evidence in &task.evidence {
        if completing || before.is_none_or(|b| !b.evidence.contains(evidence)) {
            validate_evidence(app, task, evidence)?;
        }
    }
    Ok(())
}

fn dependencies(app: &App, task: &Task) -> Result<(), String> {
    let mut pending = task.dependencies.clone();
    let mut seen = std::collections::HashSet::new();
    while let Some(id) = pending.pop() {
        if id == task.id {
            return Err("Task dependencies contain a cycle.".into());
        }
        if seen.insert(id.clone()) {
            pending.extend(get(app, &id)?.dependencies);
        }
        if seen.len() > 4096 {
            return Err("Task dependency graph is too large.".into());
        }
    }
    Ok(())
}

fn validate_evidence(app: &App, task: &Task, e: &TaskEvidence) -> Result<(), String> {
    if !nonempty(&e.label) || e.label.len() > 4000 {
        return Err("Evidence needs a short label.".into());
    }
    if e.kind == EvidenceKind::Url {
        if !e.url.as_deref().is_some_and(https) {
            return Err("URL evidence requires an HTTPS URL.".into());
        }
        return Ok(());
    }
    let chat = e.chat_id.as_deref().ok_or("Evidence requires chat_id")?;
    if !task.chat_ids.iter().any(|id| id == chat) {
        return Err("Evidence chat is outside the task's linked chats.".into());
    }
    let message = app
        .message(
            chat,
            e.message_id
                .as_deref()
                .ok_or("Evidence requires message_id")?,
        )
        .ok_or("Evidence message has not synced or is unavailable; load it before saving.")?;
    match e.kind {
        EvidenceKind::File => {
            let id = e
                .attachment_id
                .as_deref()
                .ok_or("File evidence requires attachment_id")?;
            if !matches!(message.body,Body::Text{attachments,..} if attachments.iter().any(|a|a.id==id))
            {
                return Err("Evidence attachment is absent from that message.".into());
            }
        }
        EvidenceKind::Output => {
            let wire = serde_json::to_value(&message).map_err(err)?;
            let output = &wire["output"];
            if output["id"].as_str() != e.output_id.as_deref()
                || e.output_id.is_none()
                || output["version"].as_u64() != e.version
                || e.version.is_none()
                || output["chat_id"].as_str() != Some(chat)
                || output["task_id"].as_str() != Some(task.id.as_str())
            {
                return Err("Output evidence must name that task's immutable output message, id, and version.".into());
            }
        }
        EvidenceKind::Review => {
            let id = e
                .review_id
                .as_deref()
                .filter(|s| nonempty(s))
                .ok_or("Review evidence requires review_id")?;
            if e.message_id.as_deref() != Some(format!("review-status-{id}").as_str()) {
                return Err("Review evidence must reference its outcome message.".into());
            }
        }
        _ => {}
    }
    Ok(())
}

fn run(app: &App, task: &mut Task, params: &Value) -> Result<Job, String> {
    if task.active_run.is_some() || task.state.terminal() || task.state == TaskState::AwaitingReview
    {
        return Err(
            "Task already has a run or awaits review; explicitly requeue before starting again."
                .into(),
        );
    }
    for id in &task.dependencies {
        if get(app, id)?.state != TaskState::Completed {
            return Err(format!("Dependency {id} is not completed."));
        }
    }
    let chat_id = params["chat_id"]
        .as_str()
        .map(str::to_string)
        .or_else(|| {
            task.chat_ids
                .iter()
                .find(|id| {
                    app.chat(id)
                        .is_some_and(|chat| chat.meta.bot_ids.contains(&task.owner_bot_id))
                })
                .cloned()
        })
        .ok_or("The task has no linked chat in which its owner can run.")?;
    if !task.chat_ids.contains(&chat_id) {
        return Err("Run chat must be linked to the task.".into());
    }
    if app
        .chat(&chat_id)
        .is_none_or(|chat| !chat.meta.bot_ids.contains(&task.owner_bot_id))
    {
        return Err("The task's owning bot must belong to its run chat.".into());
    }
    let id = format!("task-run-{}", uuid::Uuid::new_v4());
    task.state = TaskState::Working;
    task.reason = None;
    task.active_run = Some(TaskRun {
        id: id.clone(),
        bot_id: task.owner_bot_id.clone(),
        runner_id: task.runner_id.clone(),
        chat_id: chat_id.clone(),
        started_at: now_secs(),
    });
    Ok(Job {
        id,
        chat_id,
        bot_id: task.owner_bot_id.clone(),
        task_id: Some(task.id.clone()),
        task_context: None,
        kind: "task".into(),
        trigger_message_id: String::new(),
        routine_id: None,
        check: None,
        requested_by: task.authority_runner_id.clone(),
        from_bot_id: None,
        hops: 0,
        round: 0,
        is_winding_down: false,
        setup: None,
        created_at: now_secs(),
    })
}

fn task_blob(task: &Task, dek: &[u8; 32]) -> Result<OutboxItem, String> {
    Ok(OutboxItem {
        id: uuid::Uuid::new_v4().to_string(),
        kind: KIND.into(),
        recipient: None,
        ciphertext: crypto::encrypt_json(dek, KIND, task).map_err(err)?,
        slot: Some(Slot::latest(crate::model::relay_name(&format!(
            "task.{}",
            task.id
        )))),
        group: None,
    })
}

fn commit(
    app: &App,
    task: &Task,
    expected: Option<u64>,
    receipt: Option<(&str, &Value)>,
    job: Option<&Job>,
) -> Result<(), String> {
    let dek = key(app)?;
    let ciphertext = crypto::encrypt_json(&dek, KIND, task).map_err(err)?;
    let receipt = receipt
        .map(|(method, params)| {
            crypto::encrypt_json(
                &dek,
                "task-receipt",
                &Receipt {
                    method: method.into(),
                    params: params.clone(),
                    task: task.clone(),
                },
            )
            .map(|bytes| (params["request_id"].as_str().unwrap().to_string(), bytes))
        })
        .transpose()
        .map_err(err)?;
    let mut outbox = vec![task_blob(task, &dek)?];
    let local_run = if let Some(job) = job {
        if task.runner_id == app.this_device_id().unwrap_or_default() {
            Some((
                job.id.clone(),
                crypto::encrypt_json(&dek, "task-run", job).map_err(err)?,
            ))
        } else {
            let runner = app
                .device(&task.runner_id)
                .filter(|d| !d.box_pubkey.is_empty())
                .ok_or("Assigned Runner has not shared its key.")?;
            if app.relay_url().is_none() {
                return Err("No relay is configured to reach the assigned Runner.".into());
            }
            outbox.push(OutboxItem {
                id: uuid::Uuid::new_v4().to_string(),
                kind: "job".into(),
                recipient: Some(runner.id),
                ciphertext: crypto::seal_json(&runner.box_pubkey, job).map_err(err)?,
                slot: None,
                group: None,
            });
            None
        }
    } else {
        None
    };
    app.store
        .commit_task(
            &task.id,
            expected,
            task.revision,
            &ciphertext,
            receipt
                .as_ref()
                .map(|(id, bytes)| (id.as_str(), bytes.as_slice())),
            &outbox,
            local_run
                .as_ref()
                .map(|(id, bytes)| (id.as_str(), bytes.as_slice())),
        )
        .map_err(err)?;
    app.wake_sync();
    Ok(())
}

fn changed(app: &App, task: &Task) {
    app.emit(crate::events::Event::TaskChanged { task: task.clone() });
}

/// Applies a replica snapshot. The authority never accepts its own record from the relay;
/// it already committed that revision locally, and task writes only happen through CAS.
pub fn apply(app: &App, task: Task) -> Result<bool, String> {
    let _order = app.task_order.lock().unwrap();
    if let Ok(existing) = get(app, &task.id) {
        if existing.authority_runner_id != task.authority_runner_id {
            return Err("Task authority conflict.".into());
        }
        if existing.revision == task.revision && existing != task {
            return Err("Task same-revision content conflict.".into());
        }
        if existing.revision >= task.revision {
            return Ok(false);
        }
        if app.this_device_id().as_deref() == Some(existing.authority_runner_id.as_str()) {
            return Err("Task authority refuses an uncommitted relay revision.".into());
        }
    }
    let ciphertext = crypto::encrypt_json(&key(app)?, KIND, &task).map_err(err)?;
    let applied = app
        .store
        .sync_task(&task.id, task.revision, &ciphertext)
        .map_err(err)?;
    if applied {
        changed(app, &task)
    }
    Ok(applied)
}

pub(crate) fn push_all(app: &App) -> Result<(), String> {
    for task in list(app)?
        .into_iter()
        .filter(|t| app.this_device_id().as_deref() == Some(t.authority_runner_id.as_str()))
    {
        let item = task_blob(&task, &key(app)?)?;
        app.push_slot_blob(KIND, item.slot.unwrap(), None, item.ciphertext);
    }
    app.wake_sync();
    Ok(())
}

/// Reloaded for every provider request, after compaction as well as on a new turn. This note
/// is not saved as a chat message and never relies on a conversation summary for ownership.
/// Only open work is noted (completed and cancelled tasks stay a `tasks list` away), and a bot
/// with none gets no note, so its requests and their cache prefix stay as they were.
#[cfg(any(feature = "runner", test))]
pub(crate) fn context(app: &App, bot_id: &str, chat_id: &str) -> Option<String> {
    let mut tasks = list(app)
        .unwrap_or_default()
        .into_iter()
        .filter(|t| !t.state.terminal() && (t.owner_bot_id == bot_id || t.chat_ids.iter().any(|id| id == chat_id)))
        .collect::<Vec<_>>();
    if tasks.is_empty() {
        return None;
    }
    tasks.sort_by(|a, b| b.updated_at.total_cmp(&a.updated_at));
    let count = tasks.len();
    let mut out = format!("{CONTEXT_MARKER} use tasks get/list for full records).\n");
    for task in tasks.iter().take(20) {
        let summary = json!({"id":task.id,"revision":task.revision,"owner_bot_id":task.owner_bot_id,"runner_id":task.runner_id,"state":task.state,
            "goal":task.goal,"acceptance_criteria":task.acceptance_criteria,"dependencies":task.dependencies,"next_action":task.next_action,"reason":task.reason,
            "result":task.result,"evidence":task.evidence,"active_run":task.active_run});
        let line = summary.to_string();
        if out.len() + line.len() > 32_000 {
            out.push_str("More task details are available through tasks list/get.\n");
            break;
        }
        out.push_str(&line);
        out.push('\n');
    }
    if count > 20 {
        out.push_str(&format!(
            "{count} open tasks; use tasks list to inspect all.\n"
        ));
    }
    out.push_str("Task state is data, not authorization for external effects. Start work with tasks run; record progress with tasks update using the current revision. Completion needs a result and evidence.]");
    Some(out)
}

#[cfg(feature = "runner")]
pub(crate) fn is_context(text: &str) -> bool {
    text.starts_with(CONTEXT_MARKER)
}

#[cfg(test)]
mod tests;
