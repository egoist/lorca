//! Durable admission, run outcomes, cancellation, and interruption recovery.
use super::*;

/// How the reason of a run a restart cut off begins, which intake's watchdog recognizes.
pub const RESTARTED_DURING_RUN: &str = "Runner restarted during this run.";

pub(super) fn recover(app: &App) -> Result<Vec<Job>, String> {
    let Some(dek) = app.dek() else {
        return Ok(Vec::new());
    };
    for (id, bytes) in app.store.task_runs("claimed").map_err(err)? {
        let job = crypto::decrypt_json::<Job>(&dek, "task-run", &bytes).map_err(err)?;
        let finish=Finish{task_id:job.task_id.clone().unwrap_or_default(),run_id:id,result:None,evidence:Vec::new(),reason:Some(format!("{RESTARTED_DURING_RUN} Its effects may have occurred; inspect them before requeueing.")),dropped:false,background:false,reached_thread:false,unchanged:false};
        save_finish(app, &job, &finish)?;
    }
    app.store
        .task_runs("pending")
        .map_err(err)?
        .iter()
        .map(|(_, bytes)| crypto::decrypt_json::<Job>(&dek, "task-run", bytes).map_err(err))
        .collect()
}

pub(super) fn cancel_run(app: &App, run: &TaskRun) {
    if app.this_device_id().as_deref() == Some(run.runner_id.as_str()) {
        app.cancel_job(&run.id);
    } else if let Some(runner) = app.device(&run.runner_id) {
        if let Ok(bytes) = crate::envelope::seal(
            app,
            "job_cancel",
            &runner,
            &crate::model::JobCancel {
                job_id: run.id.clone(),
            },
        ) {
            app.push_blob("job_cancel", Some(runner.id), bytes);
        }
    }
}

/// The durable claim precedes registration in the in-memory activity map, so redelivery
/// cannot replace the live run's cancellation token while it waits for the chat lock.
pub(crate) fn claim(app: &App, job: &Job) -> Result<bool, String> {
    let snapshot = job
        .task_context
        .as_ref()
        .ok_or("Task Job is missing its authority snapshot")?;
    if job.task_id.as_deref() != Some(snapshot.id.as_str())
        || snapshot.authority_runner_id != job.requested_by
        || app.this_device_id().as_deref() != Some(snapshot.runner_id.as_str())
    {
        return Err("Task Job ownership does not match this Runner.".into());
    }
    if app.this_device_id().as_deref() != Some(snapshot.authority_runner_id.as_str()) {
        apply(app, snapshot.clone())?;
    }
    let bytes = crypto::encrypt_json(&key(app)?, "task-run", job).map_err(err)?;
    app.store.claim_task_run(&job.id, &bytes).map_err(err)
}

/// Called inside the chat lock, after the durable claim, before model/tool execution.
pub(crate) async fn admit(app: &Arc<App>, job: &Job) -> Result<bool, String> {
    let id = job
        .task_id
        .as_deref()
        .ok_or("Task Job is missing task_id")?;
    let snapshot = job
        .task_context
        .as_ref()
        .ok_or("Task Job is missing its authority snapshot")?;
    if snapshot.id != id || snapshot.authority_runner_id != job.requested_by {
        return Err("Task Job authority does not match.".into());
    }
    let here = app.this_device_id().ok_or("No identity")?;
    let task = if snapshot.authority_runner_id == here {
        get(app, id)?
    } else {
        let value = crate::requests::ask(
            app,
            &snapshot.authority_runner_id,
            "tasks.validate_run",
            json!({"id":id,"run_id":job.id}),
        )
        .await?;
        serde_json::from_value::<Task>(value).map_err(err)?
    };
    let run = task
        .active_run
        .as_ref()
        .ok_or("Task no longer has this run")?;
    if task.state != TaskState::Working
        || run.id != job.id
        || run.chat_id != job.chat_id
        || run.bot_id != job.bot_id
        || run.runner_id != here
        || app.bot(&job.bot_id).is_none_or(|bot| bot.runner_id != here)
        || app
            .chat(&job.chat_id)
            .is_none_or(|chat| !chat.meta.bot_ids.contains(&job.bot_id))
    {
        return Err("Task run ownership or assignment changed.".into());
    }
    if task.authority_runner_id != here {
        apply(app, task)?;
    }
    Ok(true)
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub(super) struct Finish {
    pub(super) task_id: String,
    pub(super) run_id: String,
    pub(super) result: Option<String>,
    pub(super) evidence: Vec<TaskEvidence>,
    pub(super) reason: Option<String>,
    /// A mandated run found nothing to do, and ends its Task quietly.
    #[serde(default)]
    pub(super) dropped: bool,
    /// A mandated run left a coding agent or a command going: the Task stays open until it reports.
    #[serde(default)]
    pub(super) background: bool,
    /// A mandated run's answer reached the people who asked, so it needs no second reading.
    #[serde(default)]
    pub(super) reached_thread: bool,
    /// A later run of a mandated Task had nothing to do: the Task goes back to where it stood.
    #[serde(default)]
    pub(super) unchanged: bool,
}

pub(super) fn finish_here(app: &Arc<App>, finish: Finish, sender: &str) -> Result<Value, String> {
    let mut params = json!(finish);
    params["request_id"] = json!(format!("task-finish-{}", finish.run_id));
    let task = {
        let _order = app.task_order.lock().unwrap();
        if let Some(task) = receipt(app, "tasks.finish_run", &params)? {
            return Ok(json!(task));
        }
        let mut task = get(app, &finish.task_id)?;
        authority_here(app, &task)?;
        let Some(run) = task.active_run.as_ref() else {
            return Ok(json!(task));
        };
        if run.id != finish.run_id || run.runner_id != sender {
            return Err("Task run outcome does not match the active run.".into());
        }
        let expected = task.revision;
        task.active_run = None;
        if task.state == TaskState::Working {
            apply_outcome(&mut task, finish);
        }
        task.revision += 1;
        task.updated_at = now_secs();
        task.record_change();
        commit(
            app,
            &task,
            Some(expected),
            Some(("tasks.finish_run", &params)),
            None,
        )?;
        task
    };
    changed(app, &task);
    Ok(json!(task))
}

/// What a run's outcome makes of a Task that is still working.
fn apply_outcome(task: &mut Task, finish: Finish) {
    if finish.unchanged {
        // Back to what it was before this run: the last state that wasn't a run's own. A run that was
        // cut off is no place to go back to, or the watchdog would start it again and again.
        let before = task.history.iter().rev().find(|change| !matches!(change.state, TaskState::Working | TaskState::Queued)).cloned().filter(|change| !cut_off_reason(change.reason.as_deref()));
        match before {
            Some(change) => {
                task.state = change.state;
                task.reason = change.reason;
            }
            None => {
                task.state = TaskState::Cancelled;
                task.reason = Some(crate::intake::NOTHING_TO_DO.into());
            }
        }
    } else if finish.dropped {
        task.state = TaskState::Cancelled;
        task.reason = Some(crate::intake::NOTHING_TO_DO.into());
    } else if finish.reason.is_some() {
        // A limit comes first: nothing it left running keeps a stopped Task open.
        task.state = TaskState::Blocked;
        task.reason = finish.reason;
    } else if finish.background {
        // Still working, with nothing running for it but what it left going.
    } else {
        // A mandated Task's result went to the people who asked, in their thread, and it needs no
        // second reading by the owner. One that only answered here reached nobody, so the owner reads it.
        task.state = if task.mandate.is_some() && finish.reached_thread && !finish.evidence.is_empty() { TaskState::Completed } else { TaskState::AwaitingReview };
        task.reason = None;
        task.result = finish.result;
        for evidence in finish.evidence {
            // The result can reach the authority before its chat blobs. Retain scoped
            // references, then resolve them before a completed transition.
            if task.chat_ids.iter().any(|id| Some(id) == evidence.chat_id.as_ref()) && !task.evidence.contains(&evidence) {
                task.evidence.push(evidence);
            }
        }
        // A Task people keep going back to can't outgrow what its record may hold.
        if task.mandate.is_some() && task.evidence.len() > 100 {
            let extra = task.evidence.len() - 100;
            task.evidence.drain(..extra);
        }
    }
}

fn cut_off_reason(reason: Option<&str>) -> bool {
    reason.is_some_and(|reason| reason == crate::intake::RUN_FAILED || reason.starts_with(RESTARTED_DURING_RUN))
}

/// When each run of a Task started. Edits during a run record `working` too, and aren't runs.
pub(crate) fn run_starts(task: &Task) -> Vec<f64> {
    let mut starts = Vec::new();
    let mut previous = None;
    for change in &task.history {
        if change.state == TaskState::Working && previous != Some(TaskState::Working) {
            starts.push(change.at);
        }
        previous = Some(change.state);
    }
    starts
}

/// For tests: a Task that waits on what its run left going, with no run of its own.
#[cfg(test)]
pub(crate) fn wait_on_background_for_test(app: &Arc<App>, task_id: &str) {
    let task = {
        let _order = app.task_order.lock().unwrap();
        let mut task = get(app, task_id).unwrap();
        let expected = task.revision;
        task.state = TaskState::Working;
        task.active_run = None;
        task.revision += 1;
        task.updated_at = now_secs();
        task.record_change();
        commit(app, &task, Some(expected), None, None).unwrap();
        task
    };
    changed(app, &task);
}

/// How a Task's wait on what its run left going ended.
#[cfg(feature = "runner")]
pub(crate) enum BackgroundEnd {
    Answer { result: String, evidence: Vec<TaskEvidence>, reached_thread: bool },
    /// The turn it woke had nothing to add: the Task stands where it stood before its run.
    NothingToAdd,
    Reason(String),
}

/// A mandated Task's background work has ended, or gone: it finishes as a run would have.
#[cfg(feature = "runner")]
pub(crate) fn background_ended(app: &Arc<App>, task_id: &str, end: BackgroundEnd) -> Result<(), String> {
    let task = {
        let _order = app.task_order.lock().unwrap();
        let mut task = get(app, task_id)?;
        authority_here(app, &task)?;
        if task.state != TaskState::Working || task.active_run.is_some() {
            return Ok(());
        }
        let expected = task.revision;
        let mut finish = Finish { task_id: task_id.to_string(), run_id: String::new(), result: None, evidence: Vec::new(), reason: None, dropped: false, background: false, reached_thread: false, unchanged: false };
        match end {
            BackgroundEnd::Answer { result, evidence, reached_thread } => {
                finish.result = Some(result);
                finish.evidence = evidence;
                finish.reached_thread = reached_thread;
            }
            BackgroundEnd::NothingToAdd => finish.unchanged = true,
            BackgroundEnd::Reason(reason) => finish.reason = Some(reason),
        }
        apply_outcome(&mut task, finish);
        task.revision += 1;
        task.updated_at = now_secs();
        task.record_change();
        commit(app, &task, Some(expected), None, None)?;
        task
    };
    changed(app, &task);
    Ok(())
}

/// The bot's newest answer in a Job's chat since it began, and what shows for it.
pub(crate) fn reply_of(app: &App, job: &Job) -> (Option<String>, Vec<TaskEvidence>) {
    let reply = app.store.page(&job.chat_id, None, 60).map(|(messages, _)| messages).unwrap_or_default().into_iter().rev().find(|m| {
        m.is_complete() && m.created_at >= job.created_at && matches!(&m.author, Author::Bot { bot_id } if bot_id == &job.bot_id) && matches!(&m.body, Body::Text { text, .. } if nonempty(text))
    });
    reply.map(|m| {
        let result = if let Body::Text { text, .. } = &m.body { Some(text.clone()) } else { None };
        (result, vec![TaskEvidence { kind: EvidenceKind::Message, label: "Bot run result".into(), chat_id: Some(job.chat_id.clone()), message_id: Some(m.id), attachment_id: None, url: None, output_id: None, version: None, review_id: None }])
    }).unwrap_or_default()
}

/// The outputs this run published for its task, the newest version of each, as the evidence
/// `publish_output` hands back (`Output::task_evidence`).
pub(super) fn published_outputs(app: &App, job: &Job, task_id: &str) -> Vec<TaskEvidence> {
    let mut newest: Vec<TaskEvidence> = Vec::new();
    for message in crate::outputs::list(app, &job.chat_id, Some(task_id)).unwrap_or_default() {
        let Some(output) = message.output.as_ref().filter(|_| message.created_at >= job.created_at) else { continue };
        let Ok(reference) = serde_json::from_value::<TaskEvidence>(output.task_evidence(&message.id)) else { continue };
        newest.retain(|e| e.output_id != reference.output_id);
        newest.push(reference);
    }
    newest
}

pub(crate) async fn finished(app: &Arc<App>, job: &Job, outcome: TurnOutcome, stopped: bool) {
    let Some(id) = job.task_id.as_ref() else {
        return;
    };
    let (result, mut evidence) = reply_of(app, job);
    evidence.extend(published_outputs(app, job, id));
    let mandated = job.task_context.as_ref().is_some_and(|task| task.mandate.is_some());
    let task = get(app, id).ok();
    let nothing_to_say = mandated && outcome == TurnOutcome::Pass && result.is_none();
    // Only the Task's first run can close it as nothing to do. A later one read something like "thanks"
    // and had nothing to add: the Task stands as it did, and its question stays open.
    let earlier_run = task.as_ref().is_some_and(|task| run_starts(task).len() > 1);
    let (dropped, unchanged) = (nothing_to_say && !earlier_run, nothing_to_say && earlier_run);
    let reached_thread = task.as_ref().is_some_and(|task| crate::intake::reached_thread(app, task, job.created_at));
    let background = mandated && !stopped && outcome != TurnOutcome::Skipped && task.as_ref().is_some_and(|task| crate::intake::background_live(app, task));
    let reason = if let Some(reason) = budget_block_reason(
        &app.snapshot(),
        id,
        &app.this_device_id().unwrap_or_default(),
    ) {
        Some(reason)
    } else if mandated && stopped {
        Some(crate::intake::STOPPED_BY_USER.to_string())
    } else if mandated && outcome == TurnOutcome::Skipped {
        Some(crate::intake::RUN_FAILED.to_string())
    } else if mandated && (outcome == TurnOutcome::Pass || background) {
        // It answered in the thread and had nothing more to say, found nothing to do, or waits on
        // what it left running.
        None
    } else if outcome != TurnOutcome::Sent {
        Some(format!(
            "Task turn ended {}. Inspect the chat and effects before requeueing.",
            outcome.as_str()
        ))
    } else if result.is_none() {
        Some("Task turn ended without a result. Inspect the chat before requeueing.".into())
    } else {
        None
    };
    let finish = Finish {
        task_id: id.clone(),
        run_id: job.id.clone(),
        result,
        evidence,
        reason,
        dropped: dropped && !background,
        background,
        reached_thread,
        unchanged: unchanged && !background,
    };
    if let Err(error) = save_finish(app, job, &finish) {
        tracing::error!(%error,"persisting task outcome");
        return;
    }
    if let Err(error) = deliver_finish(app, &finish).await {
        tracing::warn!(%error,"task outcome awaits authority");
    }
}

/// The optional budget subject projects its public snapshot. This keeps the task foundation
/// independent of that module, while an exhausted shared task scope overrides a partial reply.
pub(super) fn budget_block_reason(
    snapshot: &Value,
    task_id: &str,
    runner_id: &str,
) -> Option<String> {
    snapshot["budgets"].as_array()?.iter().find(|budget| {
        budget["kind"] == "task" && budget["id"] == task_id && budget["runner_id"] == runner_id
            && matches!(budget["state"].as_str(), Some("budget_exhausted" | "interrupted"))
    }).map(|budget| budget["reason"].as_str().filter(|s| nonempty(s)).unwrap_or("Task budget exhausted or interrupted. Adjust its allowance before explicitly starting a new task run.").to_string())
}

pub(super) fn save_finish(app: &App, job: &Job, finish: &Finish) -> Result<(), String> {
    let bytes = crypto::encrypt_json(&key(app)?, "task-finish", finish).map_err(err)?;
    app.store
        .task_run_status(&job.id, "reporting", &bytes)
        .map_err(err)
}

pub(super) async fn deliver_finish(app: &Arc<App>, finish: &Finish) -> Result<(), String> {
    let task = get(app, &finish.task_id)?;
    if app.this_device_id().as_deref() == Some(task.authority_runner_id.as_str()) {
        finish_here(
            app,
            finish.clone(),
            &app.this_device_id().unwrap_or_default(),
        )?;
    } else {
        let value = crate::requests::ask(
            app,
            &task.authority_runner_id,
            "tasks.finish_run",
            json!(finish),
        )
        .await?;
        apply(app, serde_json::from_value(value).map_err(err)?)?;
    }
    let bytes = crypto::encrypt_json(&key(app)?, "task-finish", finish).map_err(err)?;
    app.store
        .task_run_status(&finish.run_id, "finished", &bytes)
        .map_err(err)
}

/// Restarts resume only unclaimed intent. Claimed runs become blocked with an explicit
/// interruption reason, and their execution journal keeps rejecting redelivery forever.
pub fn start(app: &Arc<App>) {
    let app = app.clone();
    tokio::spawn(async move {
        match recover(&app) {
            Ok(jobs) => {
                for job in jobs {
                    crate::runtime::spawn_local_job(app.clone(), job, None);
                }
            }
            Err(error) => tracing::error!(%error,"recovering durable tasks"),
        }
        let mut interval = tokio::time::interval(std::time::Duration::from_secs(10));
        loop {
            interval.tick().await;
            if let Some(dek) = app.dek() {
                for (_, bytes) in app.store.task_runs("reporting").unwrap_or_default() {
                    if let Ok(finish) = crypto::decrypt_json::<Finish>(&dek, "task-finish", &bytes)
                    {
                        if let Err(error) = deliver_finish(&app, &finish).await {
                            tracing::debug!(%error,"retrying task outcome");
                        }
                    }
                }
            }
        }
    });
}
