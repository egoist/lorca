use super::*;
use crate::config::Config;
use crate::keys::{b64, Machine};
use crate::model::Bot;
use crate::relay::BlobIn;

struct Fixture {
    source: Arc<App>,
    target: Arc<App>,
    chef: Bot,
    specialist: Bot,
    source_chat: String,
    target_chat: String,
    homes: Vec<std::path::PathBuf>,
}

impl Drop for Fixture {
    fn drop(&mut self) {
        for app in [&self.source, &self.target] {
            for job in app.running_jobs.lock().unwrap().values() {
                job.cancel.cancel();
            }
        }
        for home in &self.homes {
            let _ = std::fs::remove_dir_all(home);
        }
    }
}

const TASK: &str = "task-12345678-1234-1234-1234-123456789abc";

fn fixture(remote: bool) -> Fixture {
    let homes: Vec<_> = (0..2)
        .map(|_| std::env::temp_dir().join(format!("lorca-handoffs-{}", uuid::Uuid::new_v4())))
        .collect();
    let source = App::load(Config {
        home: homes[0].clone(),
        port: 0,
    })
    .unwrap();
    crate::identity::create(&source, Some("Source".into())).unwrap();
    source.settings.lock().unwrap().relay_url = Some("https://relay.invalid".into());
    let target = App::load(Config {
        home: homes[1].clone(),
        port: 0,
    })
    .unwrap();
    let mut paired = source.machine_file().unwrap();
    paired.machine_secret = b64(&Machine::generate().secret);
    paired.name = "Target".into();
    *target.machine.lock().unwrap() = Some(paired);
    target.save_machine().unwrap();
    let target_device = target.local_device().unwrap();
    source
        .state
        .lock()
        .unwrap()
        .devices
        .push(target_device.clone());
    let chef = source.state.lock().unwrap().bots[0].clone();
    let source_chat = source.dm_with(&chef.id, None).unwrap().meta.id;
    let specialist = Bot {
        id: "specialist".into(),
        name: "Specialist".into(),
        runner_id: if remote {
            target_device.id
        } else {
            chef.runner_id.clone()
        },
        ..chef.clone()
    };
    let (specialist, dm) = source.create_bot_with_dm(specialist, None).unwrap();
    *target.state.lock().unwrap() = source.state.lock().unwrap().clone();
    target.save_state_now();
    // The parent task the requesting chat works on; its authority is the requesting Runner.
    crate::tasks::apply(
        &source,
        crate::tasks::Task {
            id: TASK.into(),
            revision: 1,
            authority_runner_id: chef.runner_id.clone(),
            owner_bot_id: chef.id.clone(),
            runner_id: chef.runner_id.clone(),
            goal: "Ship the parser fix".into(),
            acceptance_criteria: vec!["Empty fields parse".into()],
            dependencies: Vec::new(),
            next_action: "Review the parser".into(),
            chat_ids: vec![source_chat.clone()],
            links: Vec::new(),
            state: crate::tasks::TaskState::Queued,
            reason: None,
            result: None,
            evidence: Vec::new(),
            active_run: None,
            history: Vec::new(),
            created_at: now_secs(),
            updated_at: now_secs(),
        },
    )
    .unwrap();
    Fixture {
        source,
        target,
        chef,
        specialist,
        source_chat,
        target_chat: dm.meta.id,
        homes,
    }
}

fn delegate_work(f: &Fixture) -> HandoffRequest {
    let value = delegate(
        &f.source,
        &f.chef.id,
        &f.source_chat,
        0,
        DelegateInput {
            bot_id: f.specialist.id.clone(),
            message: "Review the parser".into(),
            context: "The input is an offline export".into(),
            expected_output: "An annotated report".into(),
            acceptance_criteria: vec!["Include a failing input".into(), "Explain the fix".into()],
            task_id: Some(TASK.into()),
        },
    )
    .unwrap();
    get(&f.source, value["handoff_id"].as_str().unwrap())
        .unwrap()
        .current()
        .request
        .clone()
}

fn transfer(from: &App, to: &Arc<App>, kind: &str) {
    let file = to.machine_file().unwrap();
    for item in from
        .store
        .outbox()
        .unwrap()
        .into_iter()
        .filter(|item| item.kind == kind)
    {
        crate::sync::apply_blob(
            to,
            &file,
            &BlobIn {
                seq: 1,
                id: item.id,
                kind: item.kind,
                ciphertext: b64(&item.ciphertext),
                recipient_machine_pubkey: item.recipient,
                created_at: 1,
            },
        );
    }
}

async fn settle(app: &App) {
    tokio::time::timeout(std::time::Duration::from_secs(5), async {
        while !app.running_jobs.lock().unwrap().is_empty() {
            tokio::task::yield_now().await;
        }
    })
    .await
    .unwrap();
}

#[tokio::test]
async fn offline_admission_persists_contract_and_encrypted_job_atomically() {
    let f = fixture(true);
    let request = delegate_work(&f);
    let record = get(&f.source, &request.handoff_id).unwrap();
    assert_eq!(record.view()["delivery"], "waiting_for_runner");
    assert_eq!(request.attempt, 1);
    let outbox = f.source.store.outbox().unwrap();
    let update_blob = outbox.iter().find(|b| b.kind == "handoff").unwrap();
    let update: HandoffUpdate =
        crate::crypto::decrypt_json(&f.source.dek().unwrap(), "handoff", &update_blob.ciphertext)
            .unwrap();
    assert_eq!(update.request(), &request);
    let job_blob = outbox.iter().find(|b| b.kind == "job").unwrap();
    let machine = f.target.machine_file().unwrap().machine().unwrap();
    let job: Job = crate::crypto::unseal_json(&machine.box_secret, &job_blob.ciphertext).unwrap();
    assert_eq!(job.id, request.job_id);
    assert_eq!(job.task_id, request.task_id);
    assert_eq!(
        job.handoff,
        Some(HandoffJob::Request {
            request: request.clone()
        })
    );
    let bytes = f
        .source
        .store
        .handoff(&request.handoff_id)
        .unwrap()
        .unwrap();
    assert!(!bytes
        .windows(request.message.len())
        .any(|w| w == request.message.as_bytes()));
    assert!(
        crate::crypto::decrypt_json::<Handoff>(&f.source.dek().unwrap(), "handoff", &bytes)
            .is_err()
    );
    let incoming = f
        .source
        .message(&f.target_chat, &request.trigger_message_id)
        .unwrap();
    let Body::Handoff { reason, .. } = incoming.body else {
        panic!("missing marker")
    };
    assert!(
        reason.contains(&request.expected_output) && reason.contains("Include a failing input")
    );
    let restarted = App::load(Config {
        home: f.homes[0].clone(),
        port: 0,
    })
    .unwrap();
    assert_eq!(get(&restarted, &request.handoff_id).unwrap(), record);
    let listed = dispatch(
        &restarted,
        "handoffs.list",
        json!({"task_id": request.task_id, "outstanding": true}),
    )
    .unwrap();
    assert_eq!(listed["handoffs"].as_array().unwrap().len(), 1);
}

#[tokio::test]
async fn runner_queue_states_do_not_claim_execution() {
    let f = fixture(true);
    let target = f.target.this_device_id().unwrap();
    f.source
        .state
        .lock()
        .unwrap()
        .device_online
        .insert(target.clone());
    // Presence is usable only with this Device's relay connection.
    f.source
        .relay_connected
        .store(true, std::sync::atomic::Ordering::Relaxed);
    let first = delegate_work(&f);
    assert_eq!(
        get(&f.source, &first.handoff_id).unwrap().view()["delivery"],
        "queued_relay"
    );
    f.source.settings.lock().unwrap().relay_url = None;
    f.source.machine.lock().unwrap().as_mut().unwrap().relay_url = None;
    let next = delegate_work(&f);
    assert_eq!(
        get(&f.source, &next.handoff_id).unwrap().view()["delivery"],
        "waiting_for_relay"
    );
    assert!(get(&f.source, &next.handoff_id)
        .unwrap()
        .current()
        .report
        .is_none());
}

#[tokio::test]
async fn same_runner_and_cross_runner_failures_return_to_the_requesting_chat() {
    for remote in [false, true] {
        let f = fixture(remote);
        let request = delegate_work(&f);
        if remote {
            transfer(&f.source, &f.target, "handoff");
            transfer(&f.source, &f.target, "job");
            settle(&f.target).await;
            transfer(&f.target, &f.source, "handoff");
        }
        settle(&f.source).await;
        let record = get(&f.source, &request.handoff_id).unwrap();
        let report = record.current().outcome().unwrap();
        assert_eq!(report.status, HandoffStatus::Failed);
        assert!(report.summary.contains("Connect") || report.summary.contains("provider"));
        assert!(!report.result_links.is_empty());
        let marker = f
            .source
            .message(&f.source_chat, &format!("report-{}", request.job_id))
            .unwrap();
        assert!(matches!(marker.body, Body::Handoff { ref from, ref to, ref reason } if from == &f.specialist.id && to == &f.chef.id && reason == &report.summary));
        assert_eq!(record.current().result_delivery, ResultDelivery::Finished);
    }
}

#[tokio::test]
async fn explicit_blocker_reports_automatically_and_automatic_finish_preserves_it() {
    let f = fixture(true);
    let request = delegate_work(&f);
    let job = request_job(&request);
    stage_job(&f.target, &job).unwrap();
    assert!(begin_job(&f.target, &job).unwrap());
    let value = report(
        &f.target,
        &f.specialist.id,
        &request.handoff_id,
        &request.job_id,
        ReportInput {
            status: HandoffStatus::Blocked,
            summary: "Need the repository URL".into(),
            result_links: Vec::new(),
            evidence: vec!["No repository was supplied".into()],
        },
    )
    .unwrap();
    assert_eq!(value["status"], "blocked");
    finish_job(&f.target, &job, TurnOutcome::Pass, false).unwrap();
    transfer(&f.target, &f.source, "handoff");
    settle(&f.source).await;
    let result = get(&f.source, &request.handoff_id).unwrap();
    assert_eq!(
        result.current().outcome().unwrap().summary,
        "Need the repository URL"
    );
    assert_eq!(
        dispatch(&f.source, "handoffs.list", json!({"outstanding": true})).unwrap()["handoffs"]
            .as_array()
            .unwrap()
            .len(),
        1
    );
    assert!(!begin_job(&f.target, &job).unwrap());
    // It waits in the requester's Attention until the requester follows up.
    let items = crate::attention::view(&f.source).unwrap().items;
    assert_eq!(items.len(), 1);
    assert_eq!((items[0].category, items[0].summary.as_str(), items[0].coordinator_bot_id.as_str()), (crate::attention::Category::Blocker, "Need the repository URL", f.chef.id.as_str()));
    follow_up(&f.source, &f.chef.id, &request.handoff_id, &request.job_id, "Use repository egoist/lorca").unwrap();
    assert!(crate::attention::view(&f.source).unwrap().items.is_empty());
}

#[tokio::test]
async fn completion_carries_immutable_output_references_and_deduplicates_delivery() {
    let f = fixture(true);
    let request = delegate_work(&f);
    let job = request_job(&request);
    stage_job(&f.target, &job).unwrap();
    begin_job(&f.target, &job).unwrap();
    let output = Message::new(
        &request.target_chat_id,
        Author::Bot {
            bot_id: f.specialist.id.clone(),
        },
        Body::text("Parser report"),
    );
    f.target.upsert_message(output.clone(), true);
    let mut link = ResultLink::message(&output, "Report version 2".into());
    link.kind = "output".into();
    link.output_id = Some("output-stable".into());
    link.version = Some(2);
    report(
        &f.target,
        &f.specialist.id,
        &request.handoff_id,
        &request.job_id,
        ReportInput {
            status: HandoffStatus::Completed,
            summary: "Reviewed the parser".into(),
            result_links: vec![link.clone()],
            evidence: vec!["The failing input reproduced the bug".into()],
        },
    )
    .unwrap();
    transfer(&f.target, &f.source, "handoff");
    settle(&f.source).await;
    let record = get(&f.source, &request.handoff_id).unwrap();
    assert!(record
        .current()
        .outcome()
        .unwrap()
        .result_links
        .contains(&link));
    let count = f.source.store.all(&f.source_chat).unwrap().len();
    let terminal = record.current().outcome().unwrap().clone();
    apply_update(
        &f.source,
        HandoffUpdate::Report {
            request: request.clone(),
            report: terminal,
        },
    )
    .unwrap();
    settle(&f.source).await;
    assert_eq!(f.source.store.all(&f.source_chat).unwrap().len(), count);
    assert_eq!(
        dispatch(&f.source, "handoffs.list", json!({"outstanding": true})).unwrap()["handoffs"],
        json!([])
    );
}

#[tokio::test]
async fn automatic_completion_links_the_final_response() {
    let f = fixture(true);
    let request = delegate_work(&f);
    let job = request_job(&request);
    stage_job(&f.target, &job).unwrap();
    begin_job(&f.target, &job).unwrap();
    let output = Message::new(
        &request.target_chat_id,
        Author::Bot {
            bot_id: f.specialist.id.clone(),
        },
        Body::text("Found two issues; both fixed."),
    );
    f.target.upsert_message(output.clone(), true);
    // A notice the turn posted on the way, as a compaction does, is no failure.
    f.target.notice(&request.target_chat_id, "Compacted Specialist's context: 12k tokens summarized.");
    finish_job(&f.target, &job, TurnOutcome::Sent, false).unwrap();
    let record = get(&f.target, &request.handoff_id).unwrap();
    let report = record.current().outcome().unwrap();
    assert_eq!(report.status, HandoffStatus::Completed);
    assert_eq!(report.summary, "Found two issues; both fixed.");
    assert!(report
        .result_links
        .iter()
        .any(|link| link.message_id.as_deref() == Some(&output.id)));
}

#[tokio::test]
async fn cancellation_reaches_an_offline_target_before_delayed_admission() {
    let f = fixture(true);
    let request = delegate_work(&f);
    cancel(
        &f.source,
        &f.chef.id,
        &request.handoff_id,
        &request.job_id,
        "The request changed",
    )
    .unwrap();
    let record = get(&f.source, &request.handoff_id).unwrap();
    let update = HandoffUpdate::Cancel {
        request: request.clone(),
        report: record.current().cancellation.clone().unwrap(),
    };
    apply_update(&f.target, update).unwrap();
    transfer(&f.source, &f.target, "job");
    settle(&f.target).await;
    settle(&f.source).await;
    assert_eq!(
        get(&f.target, &request.handoff_id)
            .unwrap()
            .current()
            .outcome()
            .unwrap()
            .status,
        HandoffStatus::Cancelled
    );
    let cancels = f
        .source
        .store
        .outbox()
        .unwrap()
        .into_iter()
        .filter(|item| item.kind == "job_cancel")
        .collect::<Vec<_>>();
    assert_eq!(cancels.len(), 1);
    let machine = f.target.machine_file().unwrap().machine().unwrap();
    let control: JobCancel =
        crate::crypto::unseal_json(&machine.box_secret, &cancels[0].ciphertext).unwrap();
    assert_eq!(control.job_id, request.job_id);
    assert!(f
        .target
        .store
        .all(&f.target_chat)
        .unwrap()
        .iter()
        .all(|m| m.author
            != (Author::Bot {
                bot_id: f.specialist.id.clone()
            })));
}

#[tokio::test]
async fn follow_up_retains_the_contract_and_late_results_cannot_overwrite_it() {
    let f = fixture(true);
    let first = delegate_work(&f);
    cancel(
        &f.source,
        &f.chef.id,
        &first.handoff_id,
        &first.job_id,
        "Need more context",
    )
    .unwrap();
    settle(&f.source).await;
    let next = follow_up(
        &f.source,
        &f.chef.id,
        &first.handoff_id,
        &first.job_id,
        "Use repository egoist/lorca",
    )
    .unwrap();
    let second = get(&f.source, &first.handoff_id)
        .unwrap()
        .current()
        .request
        .clone();
    assert_eq!(second.handoff_id, first.handoff_id);
    assert_ne!(second.job_id, first.job_id);
    assert_eq!(second.attempt, 2);
    assert_eq!(second.expected_output, first.expected_output);
    assert_eq!(second.acceptance_criteria, first.acceptance_criteria);
    assert_eq!(next["job_id"], second.job_id);
    assert!(follow_up(
        &f.source,
        &f.chef.id,
        &first.handoff_id,
        &first.job_id,
        "Duplicate retry"
    )
    .unwrap_err()
    .contains("changed"));
    let stale = HandoffReport {
        status: HandoffStatus::Completed,
        summary: "Old result".into(),
        result_links: Vec::new(),
        evidence: Vec::new(),
        created_at: now_secs(),
        started_after: None,
    };
    apply_update(
        &f.source,
        HandoffUpdate::Report {
            request: first.clone(),
            report: stale,
        },
    )
    .unwrap();
    let record = get(&f.source, &first.handoff_id).unwrap();
    assert_eq!(record.current().request.job_id, second.job_id);
    assert!(record.current().report.is_none());
    assert_eq!(record.attempts.len(), 2);
}

#[tokio::test]
async fn requester_restart_preserves_outstanding_work_and_routes_a_late_result() {
    let f = fixture(true);
    let request = delegate_work(&f);
    let restarted = App::load(Config {
        home: f.homes[0].clone(),
        port: 0,
    })
    .unwrap();
    resume(&restarted).unwrap();
    assert!(get(&restarted, &request.handoff_id)
        .unwrap()
        .current()
        .report
        .is_none());
    let report = HandoffReport {
        status: HandoffStatus::Blocked,
        summary: "Need credentials connected in Settings".into(),
        result_links: Vec::new(),
        evidence: vec!["Provider is disconnected".into()],
        created_at: now_secs(),
        started_after: None,
    };
    apply_update(
        &restarted,
        HandoffUpdate::Report {
            request: request.clone(),
            report,
        },
    )
    .unwrap();
    settle(&restarted).await;
    assert!(restarted
        .message(&f.source_chat, &format!("report-{}", request.job_id))
        .is_some());
    assert_eq!(
        get(&restarted, &request.handoff_id)
            .unwrap()
            .current()
            .result_delivery,
        ResultDelivery::Finished
    );
}

#[tokio::test]
async fn recipient_restart_reports_interruption_without_replaying_effects() {
    let f = fixture(true);
    let request = delegate_work(&f);
    let job = request_job(&request);
    stage_job(&f.target, &job).unwrap();
    begin_job(&f.target, &job).unwrap();
    let restarted = App::load(Config {
        home: f.homes[1].clone(),
        port: 0,
    })
    .unwrap();
    resume(&restarted).unwrap();
    let record = get(&restarted, &request.handoff_id).unwrap();
    assert_eq!(
        record.current().outcome().unwrap().status,
        HandoffStatus::Failed
    );
    assert!(record
        .current()
        .outcome()
        .unwrap()
        .summary
        .contains("restarted"));
    assert!(!begin_job(&restarted, &job).unwrap());
    assert!(restarted.running_jobs.lock().unwrap().is_empty());
    transfer(&restarted, &f.source, "handoff");
    settle(&f.source).await;
    assert!(get(&f.source, &request.handoff_id)
        .unwrap()
        .current()
        .outcome()
        .unwrap()
        .summary
        .contains("restarted"));
}

#[tokio::test]
async fn queued_local_work_and_hard_stop_have_durable_cancel_reports() {
    let f = fixture(false);
    let lock = f.source.chat_lock(&f.target_chat);
    let guard = lock.lock().await;
    let request = delegate_work(&f);
    crate::runtime::cancel_chat(&f.source, &f.target_chat);
    drop(guard);
    settle(&f.source).await;
    assert_eq!(
        get(&f.source, &request.handoff_id)
            .unwrap()
            .current()
            .outcome()
            .unwrap()
            .status,
        HandoffStatus::Cancelled
    );
    assert!(f
        .source
        .message(&f.source_chat, &format!("report-{}", request.job_id))
        .is_some());
}

#[tokio::test]
async fn contracts_and_reports_enforce_participant_and_reference_rules() {
    let f = fixture(true);
    let request = delegate_work(&f);
    assert!(cancel(
        &f.source,
        &f.specialist.id,
        &request.handoff_id,
        &request.job_id,
        "Stop"
    )
    .is_err());
    assert!(delegate(
        &f.source,
        &f.chef.id,
        &f.source_chat,
        MAX_BOT_HOPS,
        DelegateInput {
            bot_id: f.specialist.id.clone(),
            message: "again".into(),
            ..Default::default()
        }
    )
    .is_err());
    assert!(delegate(
        &f.source,
        &f.chef.id,
        &f.source_chat,
        0,
        DelegateInput {
            bot_id: f.chef.id.clone(),
            message: "self".into(),
            ..Default::default()
        }
    )
    .is_err());
    stage_job(&f.target, &request_job(&request)).unwrap();
    begin_job(&f.target, &request_job(&request)).unwrap();
    assert!(report(
        &f.target,
        &f.specialist.id,
        &request.handoff_id,
        "wrong-job",
        ReportInput {
            status: HandoffStatus::Completed,
            summary: "Done".into(),
            result_links: Vec::new(),
            evidence: Vec::new()
        }
    )
    .is_err());
    let bad = ResultLink {
        kind: "url".into(),
        url: Some("file:///tmp/secret".into()),
        ..ResultLink::message(
            &Message::new("c", Author::You, Body::text("x")),
            "bad".into(),
        )
    };
    assert!(bad.validate().is_err());
    let future_report = HandoffReport {
        status: HandoffStatus::Cancelled,
        summary: "Cancelled".into(),
        result_links: Vec::new(),
        evidence: Vec::new(),
        created_at: now_secs(),
        started_after: None,
    };
    finish_job(
        &f.target,
        &request_job(&request),
        TurnOutcome::Skipped,
        true,
    )
    .unwrap();
    assert_eq!(
        get(&f.target, &request.handoff_id)
            .unwrap()
            .current()
            .outcome()
            .unwrap()
            .status,
        future_report.status
    );
    // The Job field is additive; envelopes from a build without handoffs still decode.
    let mut wire = serde_json::to_value(request_job(&request)).unwrap();
    wire.as_object_mut().unwrap().remove("handoff");
    wire.as_object_mut().unwrap().remove("task_id");
    let old: Job = serde_json::from_value(wire).unwrap();
    assert!(old.handoff.is_none() && old.task_id.is_none());
}

#[tokio::test]
async fn queued_local_requests_resume_and_interrupted_result_delivery_does_not_replay() {
    let f = fixture(false);
    let lock = f.source.chat_lock(&f.target_chat);
    let guard = lock.lock().await;
    let request = delegate_work(&f);
    let restarted = App::load(Config {
        home: f.homes[0].clone(),
        port: 0,
    })
    .unwrap();
    resume(&restarted).unwrap();
    settle(&restarted).await;
    assert_eq!(
        get(&restarted, &request.handoff_id)
            .unwrap()
            .current()
            .outcome()
            .unwrap()
            .status,
        HandoffStatus::Failed
    );
    drop(guard);
    settle(&f.source).await;

    let g = fixture(true);
    let request = delegate_work(&g);
    let source_lock = g.source.chat_lock(&g.source_chat);
    let guard = source_lock.lock().await;
    apply_update(
        &g.source,
        HandoffUpdate::Report {
            request: request.clone(),
            report: HandoffReport {
                status: HandoffStatus::Blocked,
                summary: "Need input".into(),
                result_links: vec![request_link(&request)],
                evidence: vec!["Input missing".into()],
                created_at: now_secs(),
                started_after: None,
            },
        },
    )
    .unwrap();
    assert!(begin_job(&g.source, &result_job(&request)).unwrap());
    let restarted = App::load(Config {
        home: g.homes[0].clone(),
        port: 0,
    })
    .unwrap();
    resume(&restarted).unwrap();
    assert_eq!(
        get(&restarted, &request.handoff_id)
            .unwrap()
            .current()
            .result_delivery,
        ResultDelivery::Finished
    );
    assert!(restarted.running_jobs.lock().unwrap().is_empty());
    assert!(restarted.store.all(&g.source_chat).unwrap().iter().all(|m| !matches!(m.body, Body::Notice { .. })));
    assert!(restarted.message(&g.source_chat, &format!("report-{}", request.job_id)).is_some());
    drop(guard);
    settle(&g.source).await;
}

#[tokio::test]
async fn duplicate_job_envelopes_do_not_replace_the_active_cancellation_token() {
    let f = fixture(true);
    let request = delegate_work(&f);
    let lock = f.target.chat_lock(&f.target_chat);
    let guard = lock.lock().await;
    crate::runtime::spawn_local_job(f.target.clone(), request_job(&request), None);
    let original = f.target.running_jobs.lock().unwrap()[&request.job_id]
        .cancel
        .clone();
    crate::runtime::spawn_local_job(f.target.clone(), request_job(&request), None);
    f.target.cancel_job(&request.job_id);
    assert!(original.is_cancelled());
    drop(guard);
    settle(&f.target).await;
    assert_eq!(
        get(&f.target, &request.handoff_id)
            .unwrap()
            .current()
            .outcome()
            .unwrap()
            .status,
        HandoffStatus::Cancelled
    );
}

#[tokio::test]
async fn contracts_and_reports_reload_into_their_turns_prompts() {
    let f = fixture(true);
    let request = delegate_work(&f);
    let job = request_job(&request);
    let prompt = prompt(&f.target, &job);
    assert!(prompt.contains(&request.expected_output));
    assert!(prompt.contains("Include a failing input"));
    assert!(prompt.contains(request.task_id.as_deref().unwrap()));
    assert!(super::prompt(&f.source, &crate::runtime::command_job(&f.source, &f.source_chat, &f.chef.id, "card")).is_empty());
    apply_update(
        &f.source,
        HandoffUpdate::Report {
            request: request.clone(),
            report: HandoffReport {
                status: HandoffStatus::Blocked,
                summary: "Need the schema version".into(),
                result_links: vec![request_link(&request)],
                evidence: vec!["The export has no version field".into()],
                created_at: now_secs(),
                started_after: None,
            },
        },
    )
    .unwrap();
    let continuation = super::prompt(&f.source, &result_job(&request));
    assert!(continuation.contains("\"blocked\""));
    assert!(continuation.contains("The export has no version field"));
    assert!(continuation.contains(&request.trigger_message_id));
    assert!(continuation.contains("An annotated report"));
    settle(&f.source).await;
}

#[tokio::test]
async fn delegating_for_a_task_checks_it_and_records_the_report_on_it() {
    let f = fixture(true);
    assert!(delegate(
        &f.source,
        &f.chef.id,
        &f.source_chat,
        0,
        DelegateInput {
            bot_id: f.specialist.id.clone(),
            message: "Review".into(),
            task_id: Some("task-00000000-0000-4000-8000-000000000000".into()),
            ..Default::default()
        }
    )
    .unwrap_err()
    .contains("Unknown task"));
    let request = delegate_work(&f);
    apply_update(
        &f.source,
        HandoffUpdate::Report {
            request: request.clone(),
            report: HandoffReport {
                status: HandoffStatus::Completed,
                summary: "Empty fields parse now.\nThe fix checks the field count.".into(),
                result_links: vec![request_link(&request)],
                evidence: Vec::new(),
                created_at: now_secs(),
                started_after: None,
            },
        },
    )
    .unwrap();
    let report = format!("report-{}", request.job_id);
    let recorded = tokio::time::timeout(std::time::Duration::from_secs(5), async {
        loop {
            let task = crate::tasks::get(&f.source, TASK).unwrap();
            if let Some(evidence) = task.evidence.iter().find(|e| e.message_id.as_deref() == Some(report.as_str())) {
                return evidence.clone();
            }
            tokio::task::yield_now().await;
        }
    })
    .await
    .unwrap();
    assert_eq!(recorded.label, "Specialist: Empty fields parse now.");
    assert_eq!(recorded.chat_id.as_deref(), Some(f.source_chat.as_str()));
    settle(&f.source).await;
}

#[tokio::test]
async fn resuming_a_stopped_delegated_turn_sends_the_next_attempt() {
    let f = fixture(true);
    let request = delegate_work(&f);
    let job = request_job(&request);
    stage_job(&f.target, &job).unwrap();
    assert!(begin_job(&f.target, &job).unwrap());
    finish_job(&f.target, &job, TurnOutcome::Skipped, false).unwrap();
    resume_stopped(&f.target, job.clone()).unwrap();
    let record = get(&f.target, &request.handoff_id).unwrap();
    let next = &record.current().request;
    assert_eq!((record.attempts.len(), next.attempt), (2, 2));
    assert_ne!(next.job_id, request.job_id);
    // The same request: no second marker in the recipient's DM.
    assert_eq!(next.trigger_message_id, request.trigger_message_id);
    assert_eq!((&next.message, &next.expected_output), (&request.message, &request.expected_output));
    assert!(resume_stopped(&f.target, job).unwrap_err().contains("nothing to resume"));
    settle(&f.target).await;
}
