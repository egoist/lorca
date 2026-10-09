use super::*;
use crate::config::Config;
use crate::review_queue as queue;

struct Scratch {
    app: Arc<App>,
    home: std::path::PathBuf,
}
impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.home);
    }
}

fn scratch() -> Scratch {
    let home = std::env::temp_dir().join(format!("lorca-review-queue-{}", uuid::Uuid::new_v4()));
    let app = App::load(Config {
        home: home.clone(),
        port: 0,
    })
    .unwrap();
    crate::identity::create(&app, Some("Review Runner".into())).unwrap();
    Scratch { app, home }
}

async fn proposed(app: &Arc<App>, payload: ReviewPayload, guards: Vec<String>) -> ReviewItem {
    let bot = app.state.lock().unwrap().bots[0].clone();
    let dm = app.dm_with(&bot.id, None).unwrap();
    mutate(app, "reviews.create", &json!({ "bot_id": bot.id, "request_id": uuid::Uuid::new_v4().to_string(),
        "origin": { "chat_id": dm.meta.id }, "payload": payload, "target": { "account": "test account", "resource": "test resource" },
        "rationale": "Send the reviewed draft", "guarded_paths": guards }), &bot.runner_id).await.unwrap()
}

async fn draft(app: &Arc<App>) -> ReviewItem {
    proposed(
        app,
        ReviewPayload::Draft {
            text: "original draft".into(),
        },
        Vec::new(),
    )
    .await
}

async fn approve(app: &Arc<App>, item: &ReviewItem) -> ReviewItem {
    mutate(
        app,
        "reviews.approve",
        &json!({ "id": item.id, "expected_version": item.version }),
        &app.this_device_id().unwrap(),
    )
    .await
    .unwrap()
}

#[tokio::test]
async fn encrypted_records_and_outbox_survive_a_restart() {
    let scratch = scratch();
    let item = draft(&scratch.app).await;
    let raw = scratch.app.store.review(&item.id).unwrap().unwrap();
    assert!(!String::from_utf8_lossy(&raw).contains("original draft"));
    let outbox = scratch.app.store.outbox().unwrap();
    let review_blob = outbox.iter().find(|blob| blob.kind == "review").unwrap();
    assert_eq!(
        review_blob.ciphertext, raw,
        "state and upload commit together"
    );
    assert_eq!(
        review_blob.slot.as_ref().unwrap().name,
        crate::model::relay_name(&format!("review/{}", item.id))
    );
    let reopened = App::load(Config {
        home: scratch.home.clone(),
        port: 0,
    })
    .unwrap();
    assert_eq!(queue::get(&reopened, &item.id).unwrap(), item);
    assert_eq!(reopened.snapshot()["reviews"][0]["id"], item.id);
}

#[tokio::test]
async fn editing_an_approved_payload_requires_approval_of_the_new_version() {
    let scratch = scratch();
    let first = draft(&scratch.app).await;
    let approved = approve(&scratch.app, &first).await;
    assert_eq!(
        approve(&scratch.app, &first).await,
        approved,
        "a repeated approval keeps one history event"
    );
    assert_eq!(
        approved.approval.as_ref().unwrap().digest,
        approved.reviewed_digest()
    );
    let edited = mutate(
        &scratch.app,
        "reviews.edit",
        &json!({ "id": first.id, "expected_version": first.version,
        "payload": { "kind": "draft", "text": "corrected draft" } }),
        &first.runner_id,
    )
    .await
    .unwrap();
    assert_eq!(edited.version, first.version + 1);
    assert_eq!(edited.state, ReviewState::Pending);
    assert!(edited.approval.is_none());
    let event = edited.history.last().unwrap();
    assert_eq!(event.previous_payload, Some(first.payload.clone()));
    assert_eq!(event.payload, edited.payload);
    assert!(mutate(
        &scratch.app,
        "reviews.approve",
        &json!({ "id": first.id, "expected_version": first.version }),
        &first.runner_id
    )
    .await
    .is_err());
    execute_approved(&scratch.app, &first.id).await.unwrap();
    assert_eq!(
        queue::get(&scratch.app, &first.id).unwrap().state,
        ReviewState::Pending
    );
    approve(&scratch.app, &edited).await;
    execute_approved(&scratch.app, &first.id).await.unwrap();
    let finished = queue::get(&scratch.app, &first.id).unwrap();
    assert_eq!(finished.state, ReviewState::Succeeded);
    assert_eq!(
        finished.outcome_evidence()["message_id"],
        finished.message_id()
    );
    let status = scratch
        .app
        .message(&first.origin.chat_id, &finished.message_id())
        .unwrap();
    assert!(
        matches!(&status.body, crate::model::Body::Notice { text, routine_id: None } if text.starts_with("Accepted · Draft: ") && text.ends_with(&format!("\n{}", match &finished.payload { ReviewPayload::Draft { text } => text.as_str(), _ => "" })))
    );
}

#[tokio::test]
async fn a_changed_guarded_file_requires_review_again_before_approval() {
    let scratch = scratch();
    let path = scratch.home.join("draft.txt");
    std::fs::write(&path, "before").unwrap();
    let item = proposed(
        &scratch.app,
        ReviewPayload::Draft {
            text: "a draft".into(),
        },
        vec![path.display().to_string()],
    )
    .await;
    std::fs::write(&path, "after").unwrap();
    let error = mutate(
        &scratch.app,
        "reviews.approve",
        &json!({ "id": item.id, "expected_version": item.version }),
        &item.runner_id,
    )
    .await
    .unwrap_err();
    assert!(error.contains("depends on changed"));
    let refreshed = queue::get(&scratch.app, &item.id).unwrap();
    assert_eq!(refreshed.version, 2);
    assert_eq!(refreshed.state, ReviewState::Pending);
    assert!(refreshed.approval.is_none());
    assert_ne!(refreshed.preconditions.files, item.preconditions.files);
}

#[tokio::test]
async fn authorization_changes_after_approval_invalidate_execution() {
    let scratch = scratch();
    let item = draft(&scratch.app).await;
    approve(&scratch.app, &item).await;
    let mut policy = scratch.app.auto_review();
    policy.is_enabled = !policy.is_enabled;
    scratch.app.set_auto_review(policy);
    execute_approved(&scratch.app, &item.id).await.unwrap();
    let refreshed = queue::get(&scratch.app, &item.id).unwrap();
    assert_eq!(refreshed.state, ReviewState::Pending);
    assert_eq!(refreshed.version, 2);
    assert!(refreshed.approval.is_none());
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn two_processes_cannot_execute_the_same_approved_call_twice() {
    let scratch = scratch();
    let item = proposed(&scratch.app, ReviewPayload::Shell { arguments: json!({ "command": "printf 'sent\\n' >> receipt.txt", "description": "Record a send" }) }, Vec::new()).await;
    approve(&scratch.app, &item).await;
    let another = App::load(Config {
        home: scratch.home.clone(),
        port: 0,
    })
    .unwrap();
    let (a, b) = tokio::join!(
        execute_approved(&scratch.app, &item.id),
        execute_approved(&another, &item.id)
    );
    a.unwrap();
    b.unwrap();
    let receipt = std::path::Path::new(&item.preconditions.workdir).join("receipt.txt");
    assert_eq!(std::fs::read_to_string(receipt).unwrap(), "sent\n");
    execute_approved(&another, &item.id).await.unwrap();
    assert_eq!(
        queue::get(&another, &item.id).unwrap().state,
        ReviewState::Succeeded
    );
}

#[tokio::test]
async fn restart_retains_approval_but_never_replays_an_interrupted_claim() {
    let scratch = scratch();
    let item = draft(&scratch.app).await;
    approve(&scratch.app, &item).await;
    let restarted = App::load(Config {
        home: scratch.home.clone(),
        port: 0,
    })
    .unwrap();
    recover_interrupted(&restarted).unwrap();
    assert_eq!(
        queue::get(&restarted, &item.id).unwrap().state,
        ReviewState::Approved
    );
    {
        let _lock = restarted.review_lock.lock().unwrap();
        let (mut claimed, raw) = queue::load(&restarted, &item.id).unwrap();
        claimed.state = ReviewState::Executing;
        claimed.revision += 1;
        queue::save(&restarted, &claimed, Some(&raw), ReviewChange::Approved).unwrap();
    }
    let restarted = App::load(Config {
        home: scratch.home.clone(),
        port: 0,
    })
    .unwrap();
    recover_interrupted(&restarted).unwrap();
    execute_approved(&restarted, &item.id).await.unwrap();
    let uncertain = queue::get(&restarted, &item.id).unwrap();
    assert_eq!(uncertain.state, ReviewState::Uncertain);
    assert!(uncertain
        .outcome
        .unwrap()
        .summary
        .contains("Check whether it finished"));
}

#[tokio::test]
async fn reject_and_cancel_do_not_need_the_originating_turn_or_bot_to_stay_alive() {
    let scratch = scratch();
    let item = draft(&scratch.app).await;
    let rejected = queue::serve(
        &scratch.app,
        "reviews.reject",
        &json!({ "id": item.id, "expected_version": item.version }),
        &item.runner_id,
    )
    .await
    .unwrap();
    assert_eq!(rejected["state"], "rejected");
    assert!(scratch.app.pending_permissions.lock().unwrap().is_empty());
    let other = draft(&scratch.app).await;
    scratch.app.state.lock().unwrap().bots.clear();
    let cancelled = queue::serve(
        &scratch.app,
        "reviews.cancel",
        &json!({ "id": other.id, "expected_version": other.version }),
        &other.runner_id,
    )
    .await
    .unwrap();
    assert_eq!(cancelled["state"], "cancelled");
}

#[tokio::test]
async fn paired_devices_read_encrypted_projections_and_decide_on_the_owner() {
    let scratch = scratch();
    let item = draft(&scratch.app).await;
    let phone = self::scratch();
    let mut machine = scratch.app.machine_file().unwrap();
    machine.machine_secret = crate::keys::b64(&crate::keys::random_32());
    machine.os = "ios".into();
    *phone.app.machine.lock().unwrap() = Some(machine);
    let actor = phone.app.this_device_id().unwrap();
    scratch
        .app
        .state
        .lock()
        .unwrap()
        .devices
        .push(phone.app.local_device().unwrap());
    let raw = scratch.app.store.review(&item.id).unwrap().unwrap();
    queue::apply(&phone.app, &raw).unwrap();
    assert_eq!(queue::get(&phone.app, &item.id).unwrap(), item);
    assert_eq!(
        crate::api::dispatch(&phone.app, "reviews.list", json!({}))
            .await
            .unwrap()[0]["id"],
        item.id
    );
    assert!(mutate(
        &phone.app,
        "reviews.approve",
        &json!({ "id": item.id, "expected_version": 1 }),
        &actor
    )
    .await
    .is_err());
    let approved = queue::serve(
        &scratch.app,
        "reviews.approve",
        &json!({ "id": item.id, "expected_version": 1 }),
        &actor,
    )
    .await
    .unwrap();
    assert_eq!(approved["approval"]["device_id"], actor);
    let newest = scratch.app.store.review(&item.id).unwrap().unwrap();
    queue::apply(&phone.app, &newest).unwrap();
    queue::apply(&phone.app, &raw).unwrap();
    assert_eq!(
        queue::get(&phone.app, &item.id).unwrap().state,
        ReviewState::Approved,
        "an old blob cannot undo a decision"
    );
    assert!(queue::serve(
        &scratch.app,
        "reviews.reject",
        &json!({ "id": item.id, "expected_version": 1 }),
        "unpaired-device"
    )
    .await
    .is_err());
}

#[tokio::test]
async fn a_routine_stages_a_held_call_without_a_live_permission_wait() {
    let scratch = scratch();
    let bot = scratch.app.state.lock().unwrap().bots[0].clone();
    let dm = scratch.app.dm_with(&bot.id, None).unwrap();
    let routine = crate::routines::create(
        &scratch.app,
        &bot.id,
        "Send",
        "every 1h",
        "Draft a report",
        None,
        true,
    )
    .unwrap();
    let trigger = Trigger {
        message_id: "routine-marker".into(),
        routine: Some(routine.clone()),
    };
    let mut auto = scratch.app.auto_review();
    auto.is_enabled = false;
    scratch.app.set_auto_review(auto);
    let workdir = bot.working_directory(&scratch.app.config.home);
    std::fs::create_dir_all(&workdir).unwrap();
    let args = json!({ "command": "printf sent > proposed.txt", "description": "Send a report" });
    let call = lorca_agent::ToolCall {
        id: "held-call".into(),
        name: "bash".into(),
        arguments: args.clone(),
    };
    let assistant = lorca_agent::AssistantMessage::empty("test", "test");
    let context = lorca_agent::AgentContext {
        system_prompt: String::new(),
        messages: Vec::new(),
        tools: Vec::new(),
        cache_points: Vec::new(),
    };
    let cancel = CancellationToken::new();
    let ctx = lorca_agent::BeforeToolCallContext {
        assistant_message: &assistant,
        tool_call: &call,
        args: &args,
        context: &context,
        cancel: &cancel,
        parent: None,
    };
    let held = crate::local_review::before_tool_call(
        &scratch.app,
        &dm.meta.id,
        &trigger,
        &bot,
        &workdir,
        true,
        ctx,
    )
    .await
    .unwrap();
    assert!(held.block && held.reason.unwrap().contains("Staged review"));
    assert!(scratch.app.pending_permissions.lock().unwrap().is_empty());
    let items = queue::list(&scratch.app).unwrap();
    assert_eq!(items.len(), 1);
    assert_eq!(
        items[0].origin.routine_id.as_deref(),
        Some(routine.id.as_str())
    );
    assert_eq!(items[0].payload, ReviewPayload::Shell { arguments: args });
    assert!(!workdir.join("proposed.txt").exists());
}

#[tokio::test]
async fn create_receipts_are_idempotent_and_reject_different_proposals() {
    let scratch = scratch();
    let bot = scratch.app.state.lock().unwrap().bots[0].clone();
    let dm = scratch.app.dm_with(&bot.id, None).unwrap();
    let mut params = json!({ "bot_id": bot.id, "request_id": "stable-proposal", "origin": { "chat_id": dm.meta.id },
        "target": { "account": "mail account", "resource": "draft" }, "rationale": "Review this message", "payload": { "kind": "draft", "text": "one draft" } });
    let first = mutate(&scratch.app, "reviews.create", &params, &bot.runner_id)
        .await
        .unwrap();
    let repeated = mutate(&scratch.app, "reviews.create", &params, &bot.runner_id)
        .await
        .unwrap();
    assert_eq!(first, repeated);
    assert!(uuid::Uuid::parse_str(first.id.strip_prefix("review-").unwrap()).is_ok());
    params["payload"]["text"] = json!("different draft");
    assert!(
        mutate(&scratch.app, "reviews.create", &params, &bot.runner_id)
            .await
            .unwrap_err()
            .contains("already used")
    );
    assert_eq!(queue::list(&scratch.app).unwrap().len(), 1);
}

#[tokio::test]
async fn a_changed_runner_or_approving_device_cannot_execute_a_saved_approval() {
    let scratch = scratch();
    let item = draft(&scratch.app).await;
    approve(&scratch.app, &item).await;
    scratch.app.state.lock().unwrap().bots[0].runner_id = "another-runner".into();
    execute_approved(&scratch.app, &item.id).await.unwrap();
    assert_eq!(
        queue::get(&scratch.app, &item.id).unwrap().state,
        ReviewState::Pending
    );
    scratch.app.state.lock().unwrap().bots[0].runner_id = item.runner_id.clone();
    let refreshed = queue::get(&scratch.app, &item.id).unwrap();
    approve(&scratch.app, &refreshed).await;
    {
        let _lock = scratch.app.review_lock.lock().unwrap();
        let (mut approved, previous) = queue::load(&scratch.app, &item.id).unwrap();
        approved.approval.as_mut().unwrap().device_id = "unpaired-device".into();
        queue::save(
            &scratch.app,
            &approved,
            Some(&previous),
            ReviewChange::Approved,
        )
        .unwrap();
    }
    execute_approved(&scratch.app, &item.id).await.unwrap();
    assert_eq!(
        queue::get(&scratch.app, &item.id).unwrap().state,
        ReviewState::Pending
    );
}

#[tokio::test]
async fn size_admission_reserves_room_for_approval_and_the_terminal_outcome() {
    let scratch = scratch();
    let bot = scratch.app.state.lock().unwrap().bots[0].clone();
    let dm = scratch.app.dm_with(&bot.id, None).unwrap();
    let mut params = json!({ "bot_id": bot.id, "request_id": "sized-proposal", "origin": { "chat_id": dm.meta.id },
        "target": { "account": "mail", "resource": "draft" }, "rationale": "r".repeat(300_000),
        "payload": { "kind": "draft", "text": "x".repeat(32_000) } });
    let item = mutate(&scratch.app, "reviews.create", &params, &bot.runner_id)
        .await
        .unwrap();
    approve(&scratch.app, &item).await;
    execute_approved(&scratch.app, &item.id).await.unwrap();
    assert_eq!(
        queue::get(&scratch.app, &item.id).unwrap().state,
        ReviewState::Succeeded
    );
    params["request_id"] = json!("too-large");
    params["payload"]["text"] = json!("x".repeat(33 * 1024));
    assert!(
        mutate(&scratch.app, "reviews.create", &params, &bot.runner_id)
            .await
            .unwrap_err()
            .contains("32 KiB")
    );
    assert_eq!(
        queue::list(&scratch.app).unwrap().len(),
        1,
        "failed admission writes no proposal"
    );
}

#[tokio::test]
async fn an_item_staged_for_a_task_records_its_outcome_there() {
    let scratch = scratch();
    let app = &scratch.app;
    let bot = app.state.lock().unwrap().bots[0].clone();
    let dm = app.dm_with(&bot.id, None).unwrap();
    let task: crate::tasks::Task = serde_json::from_value(
        crate::tasks::dispatch(app, "tasks.create", json!({ "request_id": "create", "owner_bot_id": bot.id, "chat_ids": [dm.meta.id],
            "goal": "Ship the release", "acceptance_criteria": ["Tagged"], "next_action": "Tag it" })).await.unwrap(),
    ).unwrap();
    let staged = mutate(app, "reviews.create", &json!({ "bot_id": bot.id, "request_id": "tag", "origin": { "chat_id": dm.meta.id, "task_id": task.id },
        "payload": { "kind": "draft", "text": "Release notes" }, "target": { "account": "GitHub", "resource": "v1.4.0" }, "rationale": "Publishes the notes" }), &bot.runner_id).await.unwrap();
    queue::serve(app, "reviews.reject", &json!({ "id": staged.id, "expected_version": staged.version }), &staged.runner_id).await.unwrap();
    let task = crate::tasks::get(app, &task.id).unwrap();
    let evidence = task.evidence.iter().find(|evidence| evidence.review_id.as_deref() == Some(staged.id.as_str())).expect("review evidence");
    assert_eq!(evidence.message_id.as_deref(), Some(staged.message_id().as_str()));
    // Recording again changes nothing.
    queue::record_on_task(app, &queue::get(app, &staged.id).unwrap()).await;
    assert_eq!(crate::tasks::get(app, &task.id).unwrap().revision, task.revision);
    let unknown = mutate(app, "reviews.create", &json!({ "bot_id": bot.id, "request_id": "nowhere", "origin": { "chat_id": dm.meta.id, "task_id": "task-00000000-0000-4000-8000-000000000000" },
        "payload": { "kind": "draft", "text": "x" }, "target": { "account": "a", "resource": "b" }, "rationale": "c" }), &bot.runner_id).await;
    assert!(unknown.unwrap_err().contains("no task"));
}

#[tokio::test]
async fn a_pending_item_waits_in_attention_until_it_is_decided() {
    let scratch = scratch();
    let app = &scratch.app;
    let attention = || crate::attention::view(app).unwrap().items;
    let first = draft(app).await;
    let items = attention();
    assert_eq!(items.len(), 1);
    assert_eq!(
        (items[0].category, items[0].title.as_str(), items[0].summary.as_str()),
        (crate::attention::Category::Review, "Draft: test resource", "Send the reviewed draft")
    );
    assert_eq!(items[0].sources[0].review_id.as_deref(), Some(first.id.as_str()));
    approve(app, &first).await;
    assert!(attention().is_empty(), "an approval settles it");
    // An edit after the approval needs approving again: it is back, under its new version.
    let edited = mutate(app, "reviews.edit", &json!({ "id": first.id, "expected_version": first.version,
        "payload": { "kind": "draft", "text": "corrected draft" } }), &first.runner_id).await.unwrap();
    assert_eq!(attention().len(), 1);
    queue::serve(app, "reviews.reject", &json!({ "id": edited.id, "expected_version": edited.version }), &edited.runner_id)
        .await
        .unwrap();
    assert!(attention().is_empty());
}
