//! Durable admission, run outcomes, cancellation, and interruption recovery.
use super::*;

pub(super) fn recover(app: &App) -> Result<Vec<Job>, String> {
    let Some(dek) = app.dek() else {
        return Ok(Vec::new());
    };
    for (id, bytes) in app.store.task_runs("claimed").map_err(err)? {
        let job = crypto::decrypt_json::<Job>(&dek, "task-run", &bytes).map_err(err)?;
        let finish=Finish{task_id:job.task_id.clone().unwrap_or_default(),run_id:id,result:None,evidence:Vec::new(),reason:Some("Runner restarted during this run. Its effects may have occurred; inspect them before requeueing.".into())};
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
        if let Ok(bytes) = crypto::seal_json(
            &runner.box_pubkey,
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
            if finish.reason.is_some() {
                task.state = TaskState::Blocked;
                task.reason = finish.reason;
            } else {
                task.state = TaskState::AwaitingReview;
                task.reason = None;
                task.result = finish.result;
                for evidence in finish.evidence {
                    // The result can reach the authority before its chat blobs. Retain scoped
                    // references, then resolve them before a completed transition.
                    if task
                        .chat_ids
                        .iter()
                        .any(|id| Some(id) == evidence.chat_id.as_ref())
                        && !task.evidence.contains(&evidence)
                    {
                        task.evidence.push(evidence);
                    }
                }
            }
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

pub(crate) async fn finished(app: &Arc<App>, job: &Job, outcome: TurnOutcome) {
    let Some(id) = job.task_id.as_ref() else {
        return;
    };
    let reply = app
        .store
        .page(&job.chat_id, None, 60)
        .map(|(messages, _)| messages)
        .unwrap_or_default()
        .into_iter()
        .rev()
        .find(|m| {
            m.is_complete()
                && m.created_at >= job.created_at
                && matches!(&m.author,Author::Bot{bot_id} if bot_id==&job.bot_id)
                && matches!(&m.body,Body::Text{text,..} if nonempty(text))
        });
    let (result, mut evidence) = reply
        .map(|m| {
            let result = if let Body::Text { text, .. } = &m.body {
                Some(text.clone())
            } else {
                None
            };
            (
                result,
                vec![TaskEvidence {
                    kind: EvidenceKind::Message,
                    label: "Bot run result".into(),
                    chat_id: Some(job.chat_id.clone()),
                    message_id: Some(m.id),
                    attachment_id: None,
                    url: None,
                    output_id: None,
                    version: None,
                    review_id: None,
                }],
            )
        })
        .unwrap_or_default();
    evidence.extend(published_outputs(app, job, id));
    let reason = if let Some(reason) = budget_block_reason(
        &app.snapshot(),
        id,
        &app.this_device_id().unwrap_or_default(),
    ) {
        Some(reason)
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
