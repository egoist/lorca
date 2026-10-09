use super::*;

#[test]
fn task_budget_projection_keeps_runner_and_task_scope() {
    let snapshots = json!({"budgets":[
        {"kind":"job","id":"task-id","runner_id":"runner","state":"budget_exhausted","reason":"wrong scope"},
        {"kind":"task","id":"task-id","runner_id":"other","state":"budget_exhausted","reason":"wrong runner"},
        {"kind":"task","id":"task-id","runner_id":"runner","state":"budget_exhausted","reason":"Token allowance exhausted"}
    ]});
    assert_eq!(
        budget_block_reason(&snapshots, "task-id", "runner").as_deref(),
        Some("Token allowance exhausted")
    );
    assert!(budget_block_reason(&snapshots, "different-task", "runner").is_none());
}
use crate::{
    config::Config,
    model::{Device, Message},
};

#[tokio::test]
async fn finished_run_awaits_review_and_runtime_redelivery_preserves_live_cancellation() {
    let scratch = scratch_app();
    let app = &scratch.0;
    let task = make(app, "create");
    let (_, job) = launch(app, &task);
    assert!(claim(app, &job).unwrap());
    let cancel = tokio_util::sync::CancellationToken::new();
    app.running_jobs.lock().unwrap().insert(
        job.id.clone(),
        crate::app::RunningJob {
            chat_id: job.chat_id.clone(),
            bot_id: job.bot_id.clone(),
            routine_id: None,
            runner_id: None,
            cancel: cancel.clone(),
            activity: None,
        },
    );
    crate::runtime::spawn_local_job(app.clone(), job.clone(), None);
    assert_eq!(app.running_jobs.lock().unwrap().len(), 1);
    app.cancel_job(&job.id);
    assert!(cancel.is_cancelled());
    app.upsert_message(
        Message::new(
            &job.chat_id,
            Author::Bot {
                bot_id: job.bot_id.clone(),
            },
            Body::text("Verified result"),
        ),
        false,
    );
    finished(app, &job, TurnOutcome::Sent).await;
    let review = get(app, &task.id).unwrap();
    assert_eq!(review.state, TaskState::AwaitingReview);
    assert_eq!(review.result.as_deref(), Some("Verified result"));
    assert_eq!(review.evidence.len(), 1);
    assert!(review.active_run.is_none());
    let done = update(app, &review, json!({"state":"completed"}), "done").unwrap();
    assert_eq!(done.state, TaskState::Completed);
    assert!(!claim(app, &job).unwrap());
}

#[test]
fn concurrent_delivery_claims_exactly_one_execution() {
    let scratch = scratch_app();
    let app = &scratch.0;
    let task = make(app, "create");
    let (_, job) = launch(app, &task);
    let wins = std::sync::atomic::AtomicUsize::new(0);
    std::thread::scope(|scope| {
        for _ in 0..16 {
            let job = &job;
            let wins = &wins;
            scope.spawn(move || {
                if claim(app, job).unwrap() {
                    wins.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
                }
            });
        }
    });
    assert_eq!(wins.load(std::sync::atomic::Ordering::SeqCst), 1);
    let synchronous: i64 = app
        .store
        .connection
        .lock()
        .unwrap()
        .pragma_query_value(None, "synchronous", |r| r.get(0))
        .unwrap();
    assert_eq!(
        synchronous, 1,
        "Task transactions restore the database's NORMAL sync level"
    );
}

#[test]
fn authority_revalidates_chat_scope_before_remote_execution() {
    let scratch = scratch_app();
    let app = &scratch.0;
    let task = make(app, "create");
    let (_, job) = launch(app, &task);
    let params = json!({"id":task.id,"run_id":job.id});
    assert!(serve(app, "tasks.validate_run", &params, &task.runner_id).is_ok());
    app.state.lock().unwrap().chats[0].meta.bot_ids.clear();
    assert!(serve(app, "tasks.validate_run", &params, &task.runner_id)
        .unwrap_err()
        .contains("belong"));
}

#[test]
fn evidence_links_can_reference_a_teammates_chat_but_execution_stays_with_the_owner() {
    let scratch = scratch_app();
    let app = &scratch.0;
    let task = make(app, "create");
    let mut peer = app.bot(&task.owner_bot_id).unwrap();
    peer.id = "helper".into();
    let mut chat = app.state.lock().unwrap().chats[0].clone();
    chat.meta.id = "helper-chat".into();
    chat.meta.bot_ids = vec![peer.id.clone()];
    app.state.lock().unwrap().bots.push(peer);
    app.state.lock().unwrap().chats.push(chat);
    let task = update(
        app,
        &task,
        json!({"chat_ids":["helper-chat",task.chat_ids[0]]}),
        "link-helper",
    )
    .unwrap();
    let mut started = task.clone();
    let job = run(app, &mut started, &json!({})).unwrap();
    assert_eq!(job.chat_id, task.chat_ids[1]);
    assert!(
        run(app, &mut task.clone(), &json!({"chat_id":"helper-chat"}))
            .unwrap_err()
            .contains("belong")
    );
    let message = Message::new(
        "helper-chat",
        Author::Bot {
            bot_id: "helper".into(),
        },
        Body::text("Verified helper result"),
    );
    let mut evidence = message_evidence(app, &task);
    evidence.chat_id = Some("helper-chat".into());
    evidence.message_id = Some(message.id.clone());
    app.upsert_message(message, false);
    assert!(update(
        app,
        &task,
        json!({"evidence":[evidence]}),
        "helper-evidence"
    )
    .is_ok());
}

struct Scratch(Arc<App>, std::path::PathBuf);
impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.1);
    }
}
fn scratch_app() -> Scratch {
    let home = std::env::temp_dir().join(format!("lorca-tasks-{}", uuid::Uuid::new_v4()));
    let app = App::load(Config {
        home: home.clone(),
        port: 0,
    })
    .unwrap();
    crate::identity::create(&app, Some("Tasks".into())).unwrap();
    Scratch(app, home)
}
fn params(app: &App, request_id: &str) -> Value {
    let bot = app.state.lock().unwrap().bots[0].clone();
    let chat = app.state.lock().unwrap().chats[0].clone();
    json!({"request_id":request_id,"owner_bot_id":bot.id,"chat_ids":[chat.meta.id],"goal":"Deliver the fix","acceptance_criteria":["Regression check passes"],"next_action":"Inspect the failing case"})
}
fn make(app: &Arc<App>, request_id: &str) -> Task {
    serde_json::from_value(mutate(app, "tasks.create", &params(app, request_id)).unwrap()).unwrap()
}
fn update(app: &Arc<App>, task: &Task, fields: Value, request_id: &str) -> Result<Task, String> {
    let mut fields = fields;
    fields["id"] = json!(task.id);
    fields["expected_revision"] = json!(task.revision);
    fields["request_id"] = json!(request_id);
    serde_json::from_value(mutate(app, "tasks.update", &fields)?).map_err(err)
}
fn launch(app: &Arc<App>, task: &Task) -> (Task, Job) {
    let _order = app.task_order.lock().unwrap();
    let mut next = task.clone();
    validate(app, &next, Some(task)).unwrap();
    let mut job = run(app, &mut next, &json!({})).unwrap();
    next.revision += 1;
    next.record_change();
    job.task_context = Some(next.clone());
    commit(app, &next, Some(task.revision), None, Some(&job)).unwrap();
    (next, job)
}
fn message_evidence(app: &App, task: &Task) -> TaskEvidence {
    let message = Message::new(
        &task.chat_ids[0],
        Author::Bot {
            bot_id: task.owner_bot_id.clone(),
        },
        Body::text("Verified the fix"),
    );
    let evidence = TaskEvidence {
        kind: EvidenceKind::Message,
        label: "Regression result".into(),
        chat_id: Some(message.chat_id.clone()),
        message_id: Some(message.id.clone()),
        attachment_id: None,
        url: None,
        output_id: None,
        version: None,
        review_id: None,
    };
    app.upsert_message(message, false);
    evidence
}

#[test]
fn lifecycle_requires_reasons_and_completion_evidence() {
    let scratch = scratch_app();
    let app = &scratch.0;
    let task = make(app, "create");
    assert_eq!(task.state, TaskState::Queued);
    assert_eq!(task.revision, 1);
    for state in ["blocked", "cancelled"] {
        assert!(update(app, &task, json!({"state":state}), state)
            .unwrap_err()
            .contains("reason"));
    }
    assert!(update(
        app,
        &task,
        json!({"state":"completed","result":"done"}),
        "no-evidence"
    )
    .unwrap_err()
    .contains("evidence"));
    let task = update(
        app,
        &task,
        json!({"state":"blocked","reason":"Waiting for the dependency"}),
        "blocked",
    )
    .unwrap();
    let task = update(
        app,
        &task,
        json!({"state":"queued","next_action":"Verify the fix"}),
        "queue",
    )
    .unwrap();
    assert!(task.reason.is_none());
    let task = update(
        app,
        &task,
        json!({"state":"awaiting_review","result":"Fix ready"}),
        "review",
    )
    .unwrap();
    let evidence = message_evidence(app, &task);
    let task = update(
        app,
        &task,
        json!({"state":"completed","result":"Regression passed","evidence":[evidence]}),
        "complete",
    )
    .unwrap();
    assert_eq!(task.history.len(), 5);
    assert_eq!(task.state, TaskState::Completed);
    assert!(run(app, &mut task.clone(), &json!({})).is_err());
    assert_eq!(get(app, &task.id).unwrap(), task);
}

#[test]
fn request_receipts_survive_restart_and_reject_key_reuse() {
    let scratch = scratch_app();
    let app = &scratch.0;
    let first = make(app, "same-request");
    let second = make(app, "same-request");
    assert_eq!(first, second);
    let mut wrong = params(app, "same-request");
    wrong["goal"] = json!("Different work");
    assert!(mutate(app, "tasks.create", &wrong)
        .unwrap_err()
        .contains("different"));
    update(app, &first, json!({"next_action":"New step"}), "progress").unwrap();
    let reloaded = App::load(Config {
        home: scratch.1.clone(),
        port: 0,
    })
    .unwrap();
    assert_eq!(make(&reloaded, "same-request"), first);
    assert_eq!(get(&reloaded, &first.id).unwrap().revision, 2);
    assert_eq!(list(&reloaded).unwrap().len(), 1);
}

#[test]
fn ownership_edits_are_cas_and_history_keeps_the_winner() {
    let scratch = scratch_app();
    let app = &scratch.0;
    let task = make(app, "create");
    let mut peer = app.bot(&task.owner_bot_id).unwrap();
    peer.id = "peer".into();
    peer.name = "Peer".into();
    app.state.lock().unwrap().chats[0].meta.kind = "group".into();
    app.state.lock().unwrap().bots.push(peer.clone());
    app.state.lock().unwrap().chats[0]
        .meta
        .bot_ids
        .push(peer.id.clone());
    let winners = std::sync::atomic::AtomicUsize::new(0);
    std::thread::scope(|scope| {
        for i in 0..12 {
            let task = &task;
            let winners = &winners;
            scope.spawn(move || {
                if update(
                    app,
                    task,
                    json!({"owner_bot_id":"peer"}),
                    &format!("owner-{i}"),
                )
                .is_ok()
                {
                    winners.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
                }
            });
        }
    });
    assert_eq!(winners.load(std::sync::atomic::Ordering::SeqCst), 1);
    let saved = get(app, &task.id).unwrap();
    assert_eq!(saved.owner_bot_id, "peer");
    assert_eq!(saved.revision, 2);
    assert_eq!(saved.history[0].owner_bot_id, task.owner_bot_id);
    assert_eq!(saved.history[1].owner_bot_id, "peer");
}

#[test]
fn payloads_receipts_and_dispatch_intent_are_encrypted_and_atomic() {
    let scratch = scratch_app();
    let app = &scratch.0;
    let task = make(app, "create");
    let (_, job) = launch(app, &task);
    let dek = key(app).unwrap();
    for bytes in [
        app.store.task_row(&task.id).unwrap().unwrap(),
        app.store.task_receipt("create").unwrap().unwrap(),
        app.store.task_runs("pending").unwrap()[0].1.clone(),
    ] {
        assert!(!String::from_utf8_lossy(&bytes).contains("Deliver the fix"));
        assert!(crypto::decrypt(&crate::keys::random_32(), KIND, &bytes).is_err());
    }
    let outbox = app.store.outbox().unwrap();
    let queued = outbox.iter().find(|i| i.kind == KIND).unwrap();
    assert_eq!(queued.group, None);
    assert_eq!(
        crypto::decrypt_json::<Task>(&dek, KIND, &queued.ciphertext)
            .unwrap()
            .active_run
            .unwrap()
            .id,
        job.id
    );
    // A failed receipt insert rolls the attempted CAS and outbox write back with it.
    let current = get(app, &task.id).unwrap();
    let mut next = current.clone();
    next.revision += 1;
    next.next_action = "Not committed".into();
    assert!(commit(
        app,
        &next,
        Some(current.revision),
        Some(("tasks.create", &params(app, "create"))),
        None
    )
    .is_err());
    assert_eq!(get(app, &task.id).unwrap(), current);
}

#[test]
fn dependency_cycles_and_unfinished_dependencies_prevent_runs() {
    let scratch = scratch_app();
    let app = &scratch.0;
    let first = make(app, "first");
    let second = make(app, "second");
    let first = update(app, &first, json!({"dependencies":[second.id]}), "depend").unwrap();
    assert!(
        update(app, &second, json!({"dependencies":[first.id]}), "cycle")
            .unwrap_err()
            .contains("cycle")
    );
    assert!(run(app, &mut first.clone(), &json!({}))
        .unwrap_err()
        .contains("not completed"));
    let evidence = message_evidence(app, &second);
    update(
        app,
        &second,
        json!({"state":"completed","result":"dependency done","evidence":[evidence]}),
        "dependency-done",
    )
    .unwrap();
    assert!(run(app, &mut first.clone(), &json!({})).is_ok());
    assert!(update(app, &first, json!({"dependencies":["unknown"]}), "missing").is_err());
}

#[test]
fn completion_resolves_evidence_and_rejects_unrelated_chats() {
    let scratch = scratch_app();
    let app = &scratch.0;
    let task = make(app, "create");
    let mut evidence = message_evidence(app, &task);
    evidence.chat_id = Some("other-chat".into());
    assert!(update(app, &task, json!({"evidence":[evidence]}), "scope")
        .unwrap_err()
        .contains("outside"));
    let mut evidence = message_evidence(app, &task);
    evidence.message_id = Some("missing".into());
    assert!(update(
        app,
        &task,
        json!({"evidence":[evidence]}),
        "missing-message"
    )
    .is_err());
    let evidence = TaskEvidence {
        kind: EvidenceKind::Url,
        label: "Build result".into(),
        chat_id: None,
        message_id: None,
        attachment_id: None,
        url: Some("http://example.com".into()),
        output_id: None,
        version: None,
        review_id: None,
    };
    assert!(update(app, &task, json!({"evidence":[evidence]}), "bad-url").is_err());
    let mut evidence = message_evidence(app, &task);
    evidence.kind = EvidenceKind::Output;
    evidence.output_id = Some("out-series".into());
    evidence.version = Some(1);
    assert!(
        update(app, &task, json!({"evidence":[evidence]}), "wrong-output")
            .unwrap_err()
            .contains("immutable")
    );
}

#[test]
fn replicas_ignore_older_snapshots_and_surface_same_revision_conflicts() {
    let scratch = scratch_app();
    let app = &scratch.0;
    let task = make(app, "create");
    let remote = scratch_app();
    let replica = &remote.0;
    replica
        .machine
        .lock()
        .unwrap()
        .as_mut()
        .unwrap()
        .account_dek = app.machine_file().unwrap().account_dek;
    assert!(apply(replica, task.clone()).unwrap());
    assert!(!apply(replica, task.clone()).unwrap());
    let newer = update(app, &task, json!({"next_action":"Test next"}), "progress").unwrap();
    assert!(apply(replica, newer.clone()).unwrap());
    assert!(!apply(replica, task.clone()).unwrap());
    let mut conflict = newer.clone();
    conflict.owner_bot_id = "different-owner".into();
    assert!(apply(replica, conflict)
        .unwrap_err()
        .contains("same-revision"));
    let mut uncommitted = newer.clone();
    uncommitted.revision += 1;
    assert!(apply(app, uncommitted).unwrap_err().contains("uncommitted"));
    assert_eq!(get(replica, &task.id).unwrap(), newer);
}

#[tokio::test]
async fn run_claims_are_once_only_and_restart_requires_explicit_recovery() {
    let scratch = scratch_app();
    let app = &scratch.0;
    let task = make(app, "create");
    let (working, job) = launch(app, &task);
    assert!(claim(app, &job).unwrap());
    assert!(!claim(app, &job).unwrap());
    assert!(admit(app, &job).await.unwrap());
    assert!(update(
        app,
        &working,
        json!({"owner_bot_id":working.owner_bot_id}),
        "active-edit"
    )
    .is_err());
    let reloaded = App::load(Config {
        home: scratch.1.clone(),
        port: 0,
    })
    .unwrap();
    assert!(recover(&reloaded).unwrap().is_empty());
    let bytes = reloaded.store.task_runs("reporting").unwrap()[0].1.clone();
    let finish: Finish =
        crypto::decrypt_json(&key(&reloaded).unwrap(), "task-finish", &bytes).unwrap();
    deliver_finish(&reloaded, &finish).await.unwrap();
    let blocked = get(&reloaded, &task.id).unwrap();
    assert_eq!(blocked.state, TaskState::Blocked);
    assert!(blocked
        .reason
        .unwrap()
        .contains("effects may have occurred"));
    assert!(blocked.active_run.is_none());
    assert!(!claim(&reloaded, &job).unwrap());
    let (next, new_job) = launch(&reloaded, &get(&reloaded, &task.id).unwrap());
    assert_ne!(job.id, new_job.id);
    assert_eq!(next.state, TaskState::Working);
    // Lost finish response from an earlier attempt does not affect the replacement run.
    deliver_finish(&reloaded, &finish).await.unwrap();
    assert_eq!(
        get(&reloaded, &task.id).unwrap().active_run.unwrap().id,
        new_job.id
    );
}

#[test]
fn unclaimed_intent_survives_restart_and_wrong_runner_outcomes_are_refused() {
    let scratch = scratch_app();
    let app = &scratch.0;
    let task = make(app, "create");
    let (_, job) = launch(app, &task);
    let reloaded = App::load(Config {
        home: scratch.1.clone(),
        port: 0,
    })
    .unwrap();
    assert_eq!(recover(&reloaded).unwrap(), vec![job.clone()]);
    let finish = Finish {
        task_id: task.id.clone(),
        run_id: job.id,
        reason: None,
        result: Some("Done".into()),
        evidence: Vec::new(),
    };
    assert!(finish_here(&reloaded, finish, "wrong-runner").is_err());
    assert!(get(&reloaded, &task.id).unwrap().active_run.is_some());
}

#[test]
fn ownership_transfer_keeps_the_initial_authority_and_validates_assignment() {
    let scratch = scratch_app();
    let app = &scratch.0;
    let task = make(app, "create");
    let mut peer = app.bot(&task.owner_bot_id).unwrap();
    peer.id = "remote-bot".into();
    app.state.lock().unwrap().chats[0].meta.kind = "group".into();
    peer.runner_id = "remote-runner".into();
    app.state.lock().unwrap().devices.push(Device {
        id: "remote-runner".into(),
        os: "linux".into(),
        ..Default::default()
    });
    app.state.lock().unwrap().bots.push(peer.clone());
    app.state.lock().unwrap().chats[0]
        .meta
        .bot_ids
        .push(peer.id.clone());
    let next = update(app, &task, json!({"owner_bot_id":peer.id}), "transfer").unwrap();
    assert_eq!(next.runner_id, "remote-runner");
    assert_eq!(next.authority_runner_id, task.authority_runner_id);
    assert!(update(
        app,
        &next,
        json!({"runner_id":task.runner_id}),
        "bad-assignment"
    )
    .is_err());
    assert!(run(app, &mut next.clone(), &json!({"chat_id":"outside"})).is_err());
}

#[test]
fn task_context_reloads_durable_records_independently_of_compacted_transcripts() {
    let scratch = scratch_app();
    let app = &scratch.0;
    let task = make(app, "create");
    let first = context(app, &task.owner_bot_id, &task.chat_ids[0]).unwrap();
    assert!(first.contains("Inspect the failing case"));
    let updated=update(app,&task,json!({"next_action":"Verify final evidence","state":"blocked","reason":"Waiting for access"}),"progress").unwrap();
    let reloaded = App::load(Config {
        home: scratch.1.clone(),
        port: 0,
    })
    .unwrap();
    let current = context(&reloaded, &updated.owner_bot_id, &updated.chat_ids[0]).unwrap();
    assert!(current.contains("Verify final evidence") && current.contains("Waiting for access"));
    assert!(!current.contains("Inspect the failing case"));
    assert!(current.contains(&task.id));
    assert!(reloaded.snapshot()["tasks"]
        .as_array()
        .unwrap()
        .iter()
        .any(|t| t["id"] == task.id));
}

#[test]
fn only_open_tasks_reach_the_model_and_a_bot_without_any_gets_no_note() {
    let scratch = scratch_app();
    let app = &scratch.0;
    let bot = app.state.lock().unwrap().bots[0].id.clone();
    let chat = app.state.lock().unwrap().chats[0].meta.id.clone();
    assert!(context(app, &bot, &chat).is_none());
    let task = make(app, "create");
    assert!(context(app, &bot, &chat).unwrap().contains(&task.id));
    update(app, &task, json!({"state":"cancelled","reason":"No longer needed"}), "cancel").unwrap();
    assert!(context(app, &bot, &chat).is_none());
}

#[test]
fn working_starts_only_through_run() {
    let scratch = scratch_app();
    let app = &scratch.0;
    let task = make(app, "create");
    assert!(update(app, &task, json!({"state":"working"}), "by-hand")
        .unwrap_err()
        .contains("tasks run"));
}

#[test]
fn a_run_recording_its_own_outcome_is_not_cancelled_but_another_edit_cancels_it() {
    let scratch = scratch_app();
    let app = &scratch.0;
    let task = make(app, "create");
    let (working, job) = launch(app, &task);
    let token = tokio_util::sync::CancellationToken::new();
    app.running_jobs.lock().unwrap().insert(
        job.id.clone(),
        crate::app::RunningJob { chat_id: job.chat_id.clone(), bot_id: job.bot_id.clone(), routine_id: None, runner_id: None, cancel: token.clone(), activity: None },
    );
    let blocked = update(app, &working, json!({"state":"blocked","reason":"Needs access","run_id":job.id}), "own").unwrap();
    assert!(!token.is_cancelled());
    update(app, &blocked, json!({"state":"queued"}), "user").unwrap();
    assert!(token.is_cancelled());
}

#[test]
fn unrelated_edits_keep_unresolved_run_evidence_and_completion_resolves_it() {
    let scratch = scratch_app();
    let app = &scratch.0;
    let task = make(app, "create");
    let (_, job) = launch(app, &task);
    let mut evidence = message_evidence(app, &task);
    // The run's reply that the authority has not synced yet.
    evidence.message_id = Some("not-synced-yet".into());
    let finish = Finish { task_id: task.id.clone(), run_id: job.id.clone(), result: Some("Fix ready".into()), evidence: vec![evidence], reason: None };
    finish_here(app, finish, &task.runner_id).unwrap();
    let review = get(app, &task.id).unwrap();
    assert_eq!(review.state, TaskState::AwaitingReview);
    let edited = update(app, &review, json!({"next_action":"Ask for review"}), "edit").unwrap();
    assert!(update(app, &edited, json!({"state":"completed"}), "complete")
        .unwrap_err()
        .contains("synced"));
}

#[test]
fn outputs_published_for_the_task_are_its_evidence() {
    let scratch = scratch_app();
    let app = &scratch.0;
    let task = make(app, "create");
    let (_, job) = launch(app, &task);
    let publish = |task_id: &str, name: &str, replaces: Option<String>| {
        crate::outputs::publish(app, &task.chat_ids[0], &task.owner_bot_id, &scratch.1, crate::outputs::PublishOutput {
            name: name.into(),
            path: None,
            url: Some("https://docs.example.com/report".into()),
            mime: None,
            task_id: Some(task_id.into()),
            replaces,
            evidence: None,
        })
        .unwrap()
    };
    let first = publish(&task.id, "Report", None);
    let second = publish(&task.id, "Report", Some(first.id.clone()));
    let other = publish(&format!("task-{}", uuid::Uuid::new_v4()), "Other", None);
    // The run's outcome takes the newest version of each output it published for the task.
    let evidence = super::execution::published_outputs(app, &job, &task.id);
    assert_eq!(evidence.len(), 1);
    assert_eq!(evidence[0].kind, EvidenceKind::Output);
    assert_eq!(evidence[0].message_id.as_deref(), Some(second.id.as_str()));
    assert_eq!(evidence[0].output_id, first.output.as_ref().map(|o| o.id.clone()));
    assert_eq!(evidence[0].version, Some(2));
    let finish = Finish { task_id: task.id.clone(), run_id: job.id.clone(), result: Some("Report ready".into()), evidence, reason: None };
    finish_here(app, finish, &task.runner_id).unwrap();
    let review = get(app, &task.id).unwrap();
    let done = update(app, &review, json!({"state":"completed"}), "complete").unwrap();
    assert_eq!(done.state, TaskState::Completed);
    // What publish_output hands back for another task is not this task's evidence.
    let foreign = other.output.as_ref().unwrap().task_evidence(&other.id);
    assert!(update(app, &done, json!({"evidence":[foreign]}), "foreign").unwrap_err().contains("output"));
}
