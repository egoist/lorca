use super::*;
use crate::model::{Author, Message};
struct Scratch {
    app: Arc<App>,
    bot: String,
    origin: Origin,
}
impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.app.config.home);
    }
}
fn scratch() -> Scratch {
    let home = std::env::temp_dir().join(format!("lorca-feedback-{}", uuid::Uuid::new_v4()));
    let app = App::load(crate::config::Config { home, port: 0 }).unwrap();
    crate::identity::create(&app, Some("Workbench".into())).unwrap();
    let bot = app.state.lock().unwrap().bots[0].id.clone();
    let dm = app.dm_with(&bot, None).unwrap();
    let message = Message::new(
        &dm.meta.id,
        Author::You,
        Body::text("Please put the summary first."),
    );
    let origin = Origin {
        chat_id: dm.meta.id,
        message_id: message.id.clone(),
        routine_id: None,
        review_id: None,
        task_id: None,
    };
    app.upsert_message(message, false);
    Scratch { app, bot, origin }
}
fn input(s: &Scratch, kind: Kind) -> Record {
    Record {
        kind,
        origin: s.origin.clone(),
        note: "Put the summary first.".into(),
        before: None,
        after: None,
        target: None,
        excluded: false,
        event_id: None,
    }
}
async fn proposal(s: &Scratch) -> (crate::model::Routine, Proposal) {
    let r = crate::routines::create(
        &s.app,
        &s.bot,
        "Brief",
        "every 1h",
        "Summarize the inbox.",
        None,
        false,
    )
    .unwrap();
    let f = record(&s.app, &s.bot, input(s, Kind::Explicit))
        .await
        .unwrap();
    let p = propose(
        &s.app,
        &s.bot,
        Proposed {
            target: Target::RoutinePrompt { id: r.id.clone() },
            after: json!("Summarize the inbox. Put the summary first."),
            evidence: vec![f.id],
            explanation: "The user explicitly asked for the summary first in the linked work."
                .into(),
        },
    )
    .await
    .unwrap();
    (r, p)
}
#[tokio::test]
async fn explicit_outcomes_have_origins_scrubbed_examples_and_replay_ids() {
    let s = scratch();
    for kind in [
        Kind::Accepted,
        Kind::Rejected,
        Kind::Explicit,
        Kind::RoutineFailure,
        Kind::IgnoredAlert,
        Kind::Edited,
    ] {
        let mut r = input(&s, kind.clone());
        r.event_id = Some(format!("event-{kind:?}"));
        r.note = "api_key=very-secret-value".into();
        if kind == Kind::Edited {
            r.before = Some("long".into());
            r.after = Some("brief".into());
        }
        let a = record(&s.app, &s.bot, r.clone()).await.unwrap();
        let b = record(&s.app, &s.bot, r).await.unwrap();
        assert_eq!(a.id, b.id);
        assert_eq!(a.origin, s.origin);
        assert!(a.link.contains(&s.origin.message_id));
        assert!(!a.note.contains("very-secret-value"));
    }
    let loaded = store::load(&s.app, &s.bot).unwrap();
    assert_eq!(loaded.feedback.len(), 6);
    let disk = std::fs::read(store::path(&s.app, &s.bot).unwrap()).unwrap();
    assert!(!disk
        .windows(b"summary first".len())
        .any(|b| b == b"summary first"));
    assert!(crate::crypto::decrypt_json::<Store>(&s.app.dek().unwrap(), "chat", &disk).is_err());
}
#[tokio::test]
async fn silence_is_neutral_and_cannot_support_revisions() {
    let s = scratch();
    let r = crate::routines::create(
        &s.app,
        &s.bot,
        "Brief",
        "every 1h",
        "Summarize.",
        None,
        true,
    )
    .unwrap();
    let ignored = record(&s.app, &s.bot, input(&s, Kind::IgnoredAlert))
        .await
        .unwrap();
    assert!(!ignored.kind.actionable());
    let e = propose(
        &s.app,
        &s.bot,
        Proposed {
            target: Target::RoutinePrompt { id: r.id },
            after: json!("Stop the brief."),
            evidence: vec![ignored.id],
            explanation: "They did not answer".into(),
        },
    )
    .await
    .unwrap_err();
    assert!(e.contains("neutral"));
    #[cfg(feature = "runner")]
    assert_eq!(
        review::run(&s.app, &s.bot, false).await.unwrap(),
        json!({"proposals":[]})
    );
}
#[tokio::test]
async fn reviewed_apply_versions_and_guarded_rollback_preserve_authority_and_schedule() {
    let s = scratch();
    let (r, p) = proposal(&s).await;
    assert!(
        p.diff.contains("-Summarize the inbox.") && p.diff.contains("+Summarize the inbox. Put")
    );
    assert_eq!(
        s.app.routine(&r.id).unwrap().prompt,
        r.prompt,
        "proposal writes no workflow"
    );
    assert!(decide(&s.app, &s.bot, &p.id, "wrong", true, "device")
        .await
        .is_err());
    let saved = decide(&s.app, &s.bot, &p.id, &p.diff_hash, true, "device")
        .await
        .unwrap();
    let now = s.app.routine(&r.id).unwrap();
    assert_eq!(now.prompt, p.after.as_str().unwrap());
    assert_eq!(now.feedback_authorization_prompt, Some(r.prompt.clone()));
    assert_eq!(now.schedule, r.schedule);
    assert_eq!(now.is_enabled, r.is_enabled);
    assert_eq!(now.enabled_at, r.enabled_at);
    assert_eq!(saved["revision"]["version"], 1);
    let id = saved["revision"]["id"].as_str().unwrap();
    decide(&s.app, &s.bot, &p.id, &p.diff_hash, true, "device")
        .await
        .unwrap();
    assert_eq!(
        store::load(&s.app, &s.bot).unwrap().revisions.len(),
        1,
        "decision is replay safe"
    );
    rollback(
        &s.app,
        &s.bot,
        id,
        &memory::hash_text(&now.prompt),
        "device",
    )
    .await
    .unwrap();
    assert_eq!(s.app.routine(&r.id).unwrap().prompt, r.prompt);
    assert_eq!(s.app.routine(&r.id).unwrap().feedback_authorization_prompt, None, "the user's own task again");
    assert_eq!(store::load(&s.app, &s.bot).unwrap().revisions[1].version, 2);
}
#[tokio::test]
async fn a_users_own_edit_replaces_the_original_task_authority() {
    let s = scratch();
    let (r, p) = proposal(&s).await;
    decide(&s.app, &s.bot, &p.id, &p.diff_hash, true, "device").await.unwrap();
    let revised = s.app.routine(&r.id).unwrap();
    crate::routines::edit(&s.app, &r.id, None, Some("every 2h"), Some(&revised.prompt), None).unwrap();
    assert_eq!(s.app.routine(&r.id).unwrap().feedback_authorization_prompt, Some(r.prompt.clone()), "an unchanged task keeps it");
    // Another Device's edit arrives without the field; the Runner keeps none for a new task.
    let bots = s.app.state.lock().unwrap().bots.clone();
    let held = vec![s.app.routine(&r.id).unwrap()];
    let mut incoming = held.clone();
    incoming[0].prompt = "Archive newsletters.".into();
    incoming[0].feedback_authorization_prompt = None;
    crate::routines::keep_checks(&held, &mut incoming, &bots, &s.app.this_device_id().unwrap());
    assert_eq!(incoming[0].feedback_authorization_prompt, None);
    crate::routines::edit(&s.app, &r.id, None, None, Some("Archive newsletters."), None).unwrap();
    assert_eq!(s.app.routine(&r.id).unwrap().feedback_authorization_prompt, None);
}
#[tokio::test]
async fn later_user_edits_refuse_stale_apply_and_rollback() {
    let s = scratch();
    let (r, p) = proposal(&s).await;
    s.app
        .update_routine(&r.id, |r| r.prompt = "User changed the task.".into())
        .unwrap();
    assert!(decide(&s.app, &s.bot, &p.id, &p.diff_hash, true, "device")
        .await
        .unwrap_err()
        .contains("changed"));
    s.app
        .update_routine(&r.id, |r| {
            r.prompt = p.before.content.as_str().unwrap().into()
        })
        .unwrap();
    let saved = decide(&s.app, &s.bot, &p.id, &p.diff_hash, true, "device")
        .await
        .unwrap();
    s.app
        .update_routine(&r.id, |r| r.prompt = "A newer manual edit.".into())
        .unwrap();
    assert!(rollback(
        &s.app,
        &s.bot,
        saved["revision"]["id"].as_str().unwrap(),
        &memory::hash_text("A newer manual edit."),
        "device"
    )
    .await
    .unwrap_err()
    .contains("later edits"));
}
#[tokio::test]
async fn exclusion_erases_examples_invalidates_proposals_and_covers_future_outcomes() {
    let s = scratch();
    let (_, p) = proposal(&s).await;
    serve(
        &s.app,
        "feedback.exclude",
        &json!({"bot_id":s.bot,"chat_id":s.origin.chat_id}),
        "device",
    )
    .await
    .unwrap();
    let data = store::load(&s.app, &s.bot).unwrap();
    let f = &data.feedback[0];
    assert!(f.excluded && f.example.is_empty() && f.note.is_empty());
    assert_eq!(data.proposals[0].state, "excluded");
    assert!(data.proposals[0].diff.is_empty());
    assert!(decide(&s.app, &s.bot, &p.id, &p.diff_hash, true, "device")
        .await
        .is_err());
    let later = record(&s.app, &s.bot, input(&s, Kind::Explicit))
        .await
        .unwrap();
    assert!(later.excluded && later.note.is_empty());
    #[cfg(feature = "runner")]
    assert_eq!(
        review::run(&s.app, &s.bot, false).await.unwrap()["proposals"],
        json!([])
    );
}
#[tokio::test]
async fn strict_contracts_do_not_take_settings_or_broaden_control_fields() {
    let s = scratch();
    let (r, p) = proposal(&s).await;
    let bad: Result<Proposed, _> = serde_json::from_value(
        json!({"target":p.target,"after":"x","evidence":p.evidence,"explanation":"x","budget":1000,"enabled":true}),
    );
    assert!(bad.is_err());
    assert!(propose(
        &s.app,
        &s.bot,
        Proposed {
            target: Target::RoutinePrompt { id: r.id },
            after: json!({"prompt":"x","enabled":true}),
            evidence: p.evidence,
            explanation: "x".into()
        }
    )
    .await
    .unwrap_err()
    .contains("text only"));
    assert!(serve(
        &s.app,
        "feedback.settings",
        &json!({"bot_id":s.bot,"review_every_secs":30}),
        "device"
    )
    .await
    .is_err());
    assert!(store::load(&s.app, &s.bot)
        .unwrap()
        .settings
        .review_every_secs
        .is_none());
    let read = crate::api::dispatch(&s.app, "feedback.list", json!({"bot_id":s.bot}))
        .await
        .unwrap();
    assert_eq!(read["proposals"][0]["diff_hash"], p.diff_hash);
}
#[tokio::test]
async fn invalid_origins_and_wrong_runner_are_refused() {
    let s = scratch();
    let mut r = input(&s, Kind::Explicit);
    r.origin.message_id = "missing".into();
    assert!(record(&s.app, &s.bot, r).await.is_err());
    s.app.state.lock().unwrap().bots[0].runner_id = "elsewhere".into();
    assert!(record(&s.app, &s.bot, input(&s, Kind::Explicit))
        .await
        .unwrap_err()
        .contains("assigned Runner"));
}
#[tokio::test]
async fn simultaneous_decisions_apply_one_version() {
    let s = scratch();
    let (_, p) = proposal(&s).await;
    let (a, b) = tokio::join!(
        decide(&s.app, &s.bot, &p.id, &p.diff_hash, true, "a"),
        decide(&s.app, &s.bot, &p.id, &p.diff_hash, true, "b")
    );
    assert!(a.is_ok() && b.is_ok());
    assert_eq!(store::load(&s.app, &s.bot).unwrap().revisions.len(), 1);
}
#[tokio::test]
async fn recovery_resolves_applied_write_ahead_revision_without_reapplying() {
    let s = scratch();
    let (r, p) = proposal(&s).await;
    let mut data = store::load(&s.app, &s.bot).unwrap();
    data.revisions.push(Revision {
        id: "recovered".into(),
        version: 1,
        proposal_id: p.id.clone(),
        target: p.target.clone(),
        before: p.before.clone(),
        after: p.after.clone(),
        state: "applying".into(),
        created_at: 0.,
        actor_device_id: "device".into(),
        rollback_of: None,
    });
    store::save(&s.app, &s.bot, &data).unwrap();
    s.app
        .update_routine(&r.id, |r| r.prompt = p.after.as_str().unwrap().into())
        .unwrap();
    serve(&s.app, "feedback.list", &json!({"bot_id":s.bot}), "device")
        .await
        .unwrap();
    let data = store::load(&s.app, &s.bot).unwrap();
    assert_eq!(data.revisions[0].state, "applied");
    assert_eq!(data.proposals[0].state, "accepted");
}
#[tokio::test]
async fn the_list_stays_small_and_the_store_keeps_the_newest_feedback() {
    let s = scratch();
    let (_, p) = proposal(&s).await;
    for n in 0..310 {
        let mut r = input(&s, Kind::Accepted);
        r.note = format!("Good brief {n}");
        record(&s.app, &s.bot, r).await.unwrap();
    }
    let data = store::load(&s.app, &s.bot).unwrap();
    assert_eq!(data.feedback.len(), 300);
    assert!(data.feedback.iter().any(|f| f.id == p.evidence[0]), "a pending proposal keeps its evidence");
    let list = serve(&s.app, "feedback.list", &json!({"bot_id":s.bot}), "device").await.unwrap();
    assert_eq!(list["feedback_count"], 300);
    assert_eq!(list["feedback"].as_array().unwrap().len(), 31);
    assert_eq!(list["feedback"][0]["note"], "Good brief 309");
    assert_eq!(list["proposals"][0]["id"], p.id.as_str());
}
#[test]
fn diff_handles_unicode_empty_files_and_shared_suffix() {
    assert!(store::diff(&json!("hello\n尾巴"), &json!("changed\n尾巴")).contains("+changed"));
    assert!(store::diff(&json!(""), &json!("你好")).contains("+你好"));
    assert!(store::diff(&json!("你好"), &json!("")).contains("-你好"));
}

#[tokio::test]
async fn a_skill_revision_is_a_guarded_save_with_its_provenance_and_undoes_the_same_way() {
    let s = scratch();
    let scope = Scope::bot(&s.bot);
    let content = crate::playbooks::PlaybookContent { name: "launch-brief".into(), description: "How to write the launch brief".into(), instructions: "Summarize the inbox.".into(), ..Default::default() };
    let saved = crate::playbooks::save(&s.app, &scope, None, content, 0, "", Default::default()).unwrap();
    let id = saved["id"].as_str().unwrap().to_string();
    let target = Target::Playbook { scope: scope.clone(), id: id.clone() };
    assert!(skills(&s.app, &s.bot).iter().any(|(t, name)| *t == target && name == "launch-brief"));
    let f = record(&s.app, &s.bot, input(&s, Kind::Explicit)).await.unwrap();
    let proposed = |after: &str| Proposed { target: target.clone(), after: json!(after), evidence: vec![f.id.clone()], explanation: "Use the user's ordering.".into() };
    assert!(propose(&s.app, &s.bot, Proposed { after: json!({"instructions": "x"}), ..proposed("") }).await.is_err(), "text only");
    let p = propose(&s.app, &s.bot, proposed("Summarize the inbox. Put the summary first.")).await.unwrap();
    let applied = decide(&s.app, &s.bot, &p.id, &p.diff_hash, true, "device").await.unwrap();
    let view = crate::playbooks::get(&s.app, &scope, &id).unwrap();
    assert_eq!(view["content"]["instructions"], "Summarize the inbox. Put the summary first.");
    assert_eq!(view["content"]["name"], "launch-brief", "only the instructions change");
    assert_eq!(view["revision"], 2);
    assert_eq!(view["provenance"]["kind"], "workflow_feedback");
    assert_eq!(view["provenance"]["message_ids"][0], s.origin.message_id.as_str());

    // An edit the user made since refuses the undo, as it would refuse a stale suggestion.
    let current = read_target(&s.app, &s.bot, &target).await.unwrap();
    let mut edited: crate::playbooks::PlaybookContent = serde_json::from_value(view["content"].clone()).unwrap();
    edited.instructions = "Written by hand.".into();
    crate::playbooks::save(&s.app, &scope, Some(&id), edited, 2, &current.hash, Default::default()).unwrap();
    let revision = applied["revision"]["id"].as_str().unwrap();
    assert!(rollback(&s.app, &s.bot, revision, &current.hash, "device").await.is_err());
    let stale = propose(&s.app, &s.bot, proposed("Another change.")).await.unwrap();
    let hand = read_target(&s.app, &s.bot, &target).await.unwrap();
    let mut back: crate::playbooks::PlaybookContent = serde_json::from_value(crate::playbooks::get(&s.app, &scope, &id).unwrap()["content"].clone()).unwrap();
    back.instructions = "Summarize the inbox. Put the summary first.".into();
    crate::playbooks::save(&s.app, &scope, Some(&id), back, hand.revision, &hand.hash, Default::default()).unwrap();
    assert!(decide(&s.app, &s.bot, &stale.id, &stale.diff_hash, true, "device").await.unwrap_err().contains("changed"));
    let current = read_target(&s.app, &s.bot, &target).await.unwrap();
    rollback(&s.app, &s.bot, revision, &current.hash, "device").await.unwrap();
    assert_eq!(crate::playbooks::get(&s.app, &scope, &id).unwrap()["content"]["instructions"], "Summarize the inbox.");
    assert!(read_target(&s.app, &s.bot, &Target::Playbook { scope: Scope::bot("another-bot"), id }).await.is_err());
}
#[tokio::test]
async fn review_decisions_become_feedback_once_and_nothing_else_does() {
    let s = scratch();
    let actor = s.app.this_device_id().unwrap();
    let entry = |id: &str, change: &str, previous: Option<&str>, text: &str| json!({"id": id, "change": change, "version": 1, "actor_device_id": actor, "at": 1.0,
        "previous_payload": previous.map(|t| json!({"kind": "draft", "text": t})), "payload": {"kind": "draft", "text": text}});
    let item: crate::review_queue::ReviewItem = serde_json::from_value(json!({
        "id": "review-1", "runner_id": actor, "bot_id": s.bot, "request_hash": "h",
        "origin": {"chat_id": s.origin.chat_id, "message_id": s.origin.message_id, "routine_id": null, "task_id": null},
        "target": {"account": "Draft", "resource": "Launch note"}, "rationale": "", "payload": {"kind": "draft", "text": "Short note."},
        "version": 2, "revision": 4, "preconditions": {"authorization_hash": "", "workdir": "", "connection_hash": null, "tool_hash": null, "files": []},
        "state": "approved", "approval": null, "outcome": null, "created_at": 1.0, "updated_at": 1.0,
        "history": [entry("h1", "created", None, "Long note."), entry("h2", "edited", Some("Long note."), "Short note."), entry("h3", "approved", None, "Short note."), entry("h4", "executed", None, "Short note.")],
    })).unwrap();
    review_decided(&s.app, &item).await;
    review_decided(&s.app, &item).await;
    let data = store::load(&s.app, &s.bot).unwrap();
    let kinds: Vec<_> = data.feedback.iter().map(|f| f.kind.clone()).collect();
    assert_eq!(kinds, vec![Kind::Edited, Kind::Accepted], "one record per decision, none for the rest");
    assert_eq!(data.feedback[0].before.as_deref(), Some("Long note."));
    assert_eq!(data.feedback[0].after.as_deref(), Some("Short note."));
    assert_eq!(data.feedback[1].origin.review_id.as_deref(), Some("review-1"));
}
#[tokio::test]
async fn workflow_exclusion_blocks_proposals_and_future_recording() {
    let s = scratch();
    let (_, p) = proposal(&s).await;
    serve(
        &s.app,
        "feedback.exclude",
        &json!({"bot_id":s.bot,"target":p.target}),
        "device",
    )
    .await
    .unwrap();
    let mut r = input(&s, Kind::Explicit);
    r.target = Some(p.target.clone());
    let f = record(&s.app, &s.bot, r).await.unwrap();
    assert!(f.excluded && f.note.is_empty());
    assert!(propose(
        &s.app,
        &s.bot,
        Proposed {
            target: p.target,
            after: p.after,
            evidence: vec![f.id],
            explanation: "x".into()
        }
    )
    .await
    .unwrap_err()
    .contains("excluded"));
}
#[tokio::test]
async fn corrupted_encrypted_feedback_fails_closed() {
    let s = scratch();
    record(&s.app, &s.bot, input(&s, Kind::Explicit))
        .await
        .unwrap();
    let path = store::path(&s.app, &s.bot).unwrap();
    let mut bytes = std::fs::read(&path).unwrap();
    let last = bytes.len() - 1;
    bytes[last] ^= 1;
    std::fs::write(path, bytes).unwrap();
    assert!(record(&s.app, &s.bot, input(&s, Kind::Accepted))
        .await
        .unwrap_err()
        .contains("decryption"));
}

#[cfg(all(feature = "runner", feature = "server"))]
#[tokio::test]
async fn bounded_coordinator_stages_concrete_diff_then_skips_reviewed_evidence() {
    use axum::{routing::post, Json, Router};
    use std::sync::atomic::{AtomicUsize, Ordering};
    let s = scratch();
    let routine = crate::routines::create(
        &s.app,
        &s.bot,
        "Brief",
        "every 1h",
        "Summarize.",
        None,
        false,
    )
    .unwrap();
    let f = record(&s.app, &s.bot, input(&s, Kind::Explicit))
        .await
        .unwrap();
    let response=json!({"proposals":[{"target":{"kind":"routine_prompt","id":routine.id},"after":"Summarize. Put the summary first.","evidence":[f.id],"explanation":"The linked explicit request asks for the summary first."}]}).to_string();
    let calls = Arc::new(AtomicUsize::new(0));
    let seen = calls.clone();
    let server=Router::new().route("/v1/chat/completions",post(move |Json(body):Json<Value>| {
        let response=response.clone();let seen=seen.clone();
        async move {
            seen.fetch_add(1,Ordering::Relaxed);
            assert_eq!(body["max_tokens"],4096);assert!(body.get("tools").is_none());
            let chunk=json!({"id":"feedback-model","object":"chat.completion.chunk","model":"feedback-model","choices":[{"index":0,"delta":{"role":"assistant","content":response},"finish_reason":null}]});
            let done=json!({"id":"feedback-model","object":"chat.completion.chunk","model":"feedback-model","choices":[{"index":0,"delta":{},"finish_reason":"stop"}],"usage":{"prompt_tokens":100,"completion_tokens":50,"total_tokens":150}});
            ([("content-type","text/event-stream")],format!("data: {chunk}\n\ndata: {done}\n\ndata: [DONE]\n\n"))
        }
    }));
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let process = tokio::spawn(async move { axum::serve(listener, server).await.unwrap() });
    let custom = crate::credentials::CustomProvider {
        name: "Feedback model".into(),
        api: crate::credentials::CustomApi::ChatCompletions,
        base_url: format!("http://{addr}/v1"),
        api_key: String::new(),
        models: vec![
            serde_json::from_value(json!({"id":"feedback-model","context_window":64000})).unwrap(),
        ],
        created_at: 0,
    };
    s.app
        .credentials
        .lock()
        .unwrap()
        .custom
        .insert("custom:feedback".into(), custom);
    s.app.state.lock().unwrap().bots[0].provider = "custom:feedback".into();
    let result = tokio::time::timeout(
        std::time::Duration::from_secs(10),
        review::run(&s.app, &s.bot, false),
    )
    .await
    .unwrap()
    .unwrap();
    assert_eq!(result["proposals"].as_array().unwrap().len(), 1);
    assert!(result["proposals"][0]["diff"]
        .as_str()
        .unwrap()
        .contains("+Summarize. Put"));
    assert_eq!(
        s.app.routine(&routine.id).unwrap().prompt,
        "Summarize.",
        "coordinator never applies"
    );
    assert_eq!(
        review::run(&s.app, &s.bot, false).await.unwrap()["proposals"],
        json!([])
    );
    assert_eq!(calls.load(Ordering::Relaxed), 1);
    process.abort();
}

#[tokio::test]
async fn a_long_failed_routine_keeps_the_exact_job_source_and_replay_id() {
    let s = scratch();
    let r = crate::routines::create(
        &s.app,
        &s.bot,
        "Brief",
        "every 1h",
        "Summarize.",
        None,
        false,
    )
    .unwrap();
    let job:Job=serde_json::from_value(json!({"id":"long-run","chat_id":s.origin.chat_id,"bot_id":s.bot,"kind":"routine","trigger_message_id":"","routine_id":r.id,"requested_by":s.app.this_device_id().unwrap(),"created_at":0})).unwrap();
    let mut marker = Message::new(
        &job.chat_id,
        Author::System,
        Body::Notice {
            text: "Routine · Brief".into(),
            routine_id: Some(r.id.clone()),
        },
    );
    marker.id = "routine-run-long-run".into();
    s.app.upsert_message(marker, false);
    for _ in 0..30 {
        s.app.upsert_message(
            Message::new(
                &job.chat_id,
                Author::Bot {
                    bot_id: s.bot.clone(),
                },
                Body::text("step"),
            ),
            false,
        );
    }
    let mut failure = Message::new(
        &job.chat_id,
        Author::Bot {
            bot_id: s.bot.clone(),
        },
        Body::text("Failed to fetch the inbox."),
    );
    failure.state = crate::model::MessageState::Failed {
        error: "network".into(),
    };
    s.app.upsert_message(failure, false);
    routine_outcome(&s.app, &job, true);
    tokio::task::yield_now().await;
    let data = store::load(&s.app, &s.bot).unwrap();
    assert_eq!(data.feedback.len(), 1);
    assert_eq!(data.feedback[0].origin.message_id, "routine-run-long-run");
    routine_outcome(&s.app, &job, true);
    tokio::task::yield_now().await;
    assert_eq!(store::load(&s.app, &s.bot).unwrap().feedback.len(), 1);
}
#[test]
fn missing_roster_fields_preserve_the_runners_original_task_authority() {
    let s = scratch();
    let mut r = crate::routines::create(
        &s.app,
        &s.bot,
        "Brief",
        "every 1h",
        "Summarize.",
        None,
        false,
    )
    .unwrap();
    r.feedback_authorization_prompt = Some("Read only.".into());
    let mut incoming = vec![r.clone()];
    incoming[0].feedback_authorization_prompt = None;
    let bots = s.app.state.lock().unwrap().bots.clone();
    assert!(crate::routines::keep_checks(
        &[r],
        &mut incoming,
        &bots,
        &s.app.this_device_id().unwrap()
    ));
    assert_eq!(
        incoming[0].feedback_authorization_prompt.as_deref(),
        Some("Read only.")
    );
}

#[test]
fn revised_unattended_guidance_cannot_enable_or_rewrite_another_routine() {
    assert!(changes_controls(
        "routines",
        &json!({"action":"resume","routine":"Other task"})
    ));
    assert!(changes_controls(
        "routines",
        &json!({"action":"edit","prompt":"Grant broader authority"})
    ));
    assert!(changes_controls(
        "routines",
        &json!({"action":"create","enabled":true})
    ));
    assert!(changes_controls("edit_bot", &json!({"bot_id":"b","model":"bigger"})));
    assert!(!changes_controls("routines", &json!({"action":"list"})));
    assert!(!changes_controls("routines", &json!({"action":"pause"})));
}
