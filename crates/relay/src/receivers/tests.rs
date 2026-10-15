//! The receivers end to end: a relay with both receivers, a stub of GitHub's API, and a Runner
//! (the `lorca` library) whose routines subscribe, take what the relay seals to them into their
//! event inbox, and run.

use std::sync::{Arc, Mutex};

use axum::routing::{get, post};
use axum::{Json, Router};
use serde_json::{json, Value};

use super::*;
use crate::routes::PROTOCOL;

const WEBHOOK_SECRET: &str = "a-github-webhook-secret-for-tests";

struct Relay {
    state: AppState,
    url: String,
    home: std::path::PathBuf,
    task: tokio::task::JoinHandle<()>,
}

impl Drop for Relay {
    fn drop(&mut self) {
        self.task.abort();
        let _ = std::fs::remove_dir_all(&self.home);
    }
}

/// GitHub's API as the App sees it: one installation on acme's project and secret
/// repositories, an open pull request 42 in each, and a user whose sign-in reaches that
/// installation but who can access only acme/project in it.
async fn github_stub(calls: Arc<Mutex<Vec<String>>>) -> String {
    let router = Router::new()
        .route(
            "/repos/acme/project/installation",
            get(move || {
                calls.lock().unwrap().push("installation".into());
                async { Json(json!({ "id": 77, "account": { "login": "acme" } })) }
            }),
        )
        .route("/repos/acme/secret/installation", get(|| async { Json(json!({ "id": 77, "account": { "login": "acme" } })) }))
        .route("/repos/acme/secret/pulls/42", get(|| async { Json(json!({ "number": 42, "title": "Rotate the keys", "state": "open", "head": { "sha": "s" } })) }))
        .route("/user/installations/77/repositories", get(|| async { Json(json!({ "repositories": [{ "full_name": "acme/project" }] })) }))
        .route("/app/installations/77/access_tokens", post(|| async { Json(json!({ "token": "installation-token" })) }))
        .route("/repos/acme/project/pulls/42", get(|| async { Json(json!({ "number": 42, "title": "Add passkey sign-in", "state": "open", "html_url": "https://github.com/acme/project/pull/42", "head": { "sha": "abc" } })) }))
        .route("/repos/acme/project/pulls/43", get(|| async { Json(json!({ "number": 43, "title": "Old", "state": "closed", "merged": true })) }))
        .route("/login/oauth/access_token", post(|| async { Json(json!({ "access_token": "user-token" })) }))
        .route("/user/installations", get(|| async { Json(json!({ "installations": [{ "id": 77, "account": { "login": "acme" } }] })) }));
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let url = format!("http://{}", listener.local_addr().unwrap());
    tokio::spawn(async move { axum::serve(listener, router).await.unwrap() });
    url
}

async fn relay(github: Option<&str>) -> Relay {
    let home = std::env::temp_dir().join(format!("lorca-receivers-{}", uuid::Uuid::new_v4()));
    std::fs::create_dir_all(&home).unwrap();
    let local = Arc::new(db::Local::default());
    let store = db::open(home.join("relay.db").to_str().unwrap(), local.clone()).await.unwrap();
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let url = format!("http://{}", listener.local_addr().unwrap());
    let mut receivers = Receivers { webhook: Some(webhook::Hooks::new(&url)), github: None };
    if let Some(stub) = github {
        let key = std::fs::read_to_string(concat!(env!("CARGO_MANIFEST_DIR"), "/src/receivers/testdata/github-app-test-key.pem")).unwrap();
        let mut app = github::GithubApp::new("1234", "lorca-test", "client", "client-secret", WEBHOOK_SECRET, &key, "https://lorca.app/github/connected").unwrap();
        app.api = stub.into();
        app.web = stub.into();
        receivers.github = Some(app);
    }
    let state = AppState {
        db: store,
        local,
        secret: Arc::new([7; 32]),
        quota_bytes: 0,
        ip_limiter: Arc::new(crate::limit::RateLimiter::new(100.0, 100)),
        identity_limiter: Arc::new(crate::limit::RateLimiter::new(100.0, 1000)),
        trust_proxy: false,
        uploads: None,
        min_protocol: 0,
        metrics_token: None,
        stats: Arc::new(crate::metrics::StatsCache::default()),
        instance: "test".into(),
        stopping: tokio_util::sync::CancellationToken::new(),
        file_store: Arc::new(crate::store::FileStore::Local { dir: home.join("files") }),
        pusher: Arc::new(crate::push::Pusher::new(None, None)),
        pushes: tokio_util::task::TaskTracker::new(),
        receivers: Arc::new(receivers),
    };
    let app = crate::routes::router(state.clone());
    let task = tokio::spawn(async move { axum::serve(listener, app.into_make_service_with_connect_info::<std::net::SocketAddr>()).await.unwrap() });
    Relay { state, url, home, task }
}

/// A Runner registered with the relay, with one bot.
async fn runner(relay: &Relay) -> (Arc<lorca::app::App>, String) {
    let app = lorca::app::App::load(lorca::config::Config { home: relay.home.join(format!("runner-{}", uuid::Uuid::new_v4())), port: 0 }).unwrap();
    lorca::identity::create(&app, Some("Workbench".into())).unwrap();
    app.set_relay_url(Some(relay.url.clone())).unwrap();
    let machine = app.machine_file().unwrap().machine().unwrap();
    let token = lorca::sync::token_or_register(&app, &relay.url, &machine).await.unwrap();
    (app, token)
}

/// Takes what the relay sealed to the Runner into its event inbox, as its sync does, and
/// deletes them from the relay. How many.
async fn pull(app: &Arc<lorca::app::App>, relay: &Relay, token: &str) -> usize {
    let (blobs, _) = app.relay.list_blobs(&relay.url, token, 0, "event").await.unwrap();
    let machine = app.machine_file().unwrap();
    for blob in &blobs {
        lorca::event_triggers::receive_blob(app, &machine, blob).unwrap();
        app.relay.delete_blob(&relay.url, token, &blob.id).await.unwrap();
    }
    blobs.len()
}

fn bot(app: &lorca::app::App) -> String {
    app.state.lock().unwrap().bots[0].id.clone()
}

#[track_caller]
fn until(mut done: impl FnMut() -> bool) -> impl std::future::Future<Output = ()> {
    let caller = std::panic::Location::caller();
    async move {
        for _ in 0..200 {
            if done() {
                return;
            }
            tokio::time::sleep(std::time::Duration::from_millis(25)).await;
        }
        panic!("timed out waiting at {caller}");
    }
}

fn notices(app: &lorca::app::App, bot_id: &str) -> Vec<String> {
    let dm = app.dm_with(bot_id, None).unwrap();
    app.message_page(&dm.meta.id, None, 500).0.iter().filter_map(|m| match &m.body { lorca::model::Body::Notice { text, .. } => Some(text.clone()), _ => None }).collect()
}

/// Settles a failed delivery, as the user's Discard does: with no model provider in a test,
/// every run fails, and a failed delivery holds the ones after it.
fn discard_failed(app: &Arc<lorca::app::App>) {
    let listed = lorca::event_triggers::serve(app, "events.list", &json!({})).unwrap();
    for sub in listed["subscriptions"].as_array().unwrap() {
        for delivery in sub["deliveries"].as_array().unwrap().iter().filter(|d| d["state"] == "failed") {
            lorca::event_triggers::serve(app, "events.discard", &json!({ "id": delivery["id"] })).unwrap();
        }
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_webhook_runs_its_routine_once_per_request_that_carries_its_key() {
    let relay = relay(None).await;
    let (app, token) = runner(&relay).await;
    let bot_id = bot(&app);
    let triggers = lorca::routines::Triggers { receiver: Some("webhook"), ..Default::default() };
    let routine = lorca::routines::create_routine(&app, &bot_id, "Deploys", "", "Tell me if a deploy failed.", None, true, None, None, triggers).unwrap();
    assert_eq!(lorca::routine_events::subscribe(&app, &routine.id).await, lorca::routine_events::Subscribed::Subscribed);
    let listed = lorca::turns::call_routines_tool(&app, &bot_id, json!({ "action": "list" })).await.unwrap();
    assert!(listed.contains("Deploys · When its webhook is called · listening at its webhook"), "{listed}");
    let events = app.routine(&routine.id).unwrap().events.unwrap();
    assert!(events.endpoint.starts_with(&format!("{}/webhooks/", relay.url)), "{events:?}");
    assert_eq!(events.key.len(), 43, "a key the Runner made");
    let row = relay.state.db.receiver_sub("webhook", events.endpoint.rsplit('/').next().unwrap()).await.unwrap().unwrap();
    assert_eq!(row.key_hash.as_deref(), Some(webhook::key_hash(&events.key).as_str()), "the relay keeps only the key's hash");

    let http = reqwest::Client::new();
    let post = |key: Option<&str>, idempotency: Option<&str>, body: Vec<u8>| {
        let mut request = http.post(&events.endpoint).header("content-type", "application/json").body(body);
        if let Some(key) = key {
            request = request.bearer_auth(key);
        }
        if let Some(idempotency) = idempotency {
            request = request.header("Idempotency-Key", idempotency);
        }
        request.send()
    };
    let body = br#"{"deploy":"web","status":"failed"}"#.to_vec();
    assert_eq!(post(None, None, body.clone()).await.unwrap().status(), 401, "no key");
    assert_eq!(post(Some("wrong"), None, body.clone()).await.unwrap().status(), 401, "a wrong key");
    assert_eq!(post(Some(&events.key), None, vec![b'x'; webhook::MAX_BODY_BYTES + 1]).await.unwrap().status(), 413, "over 64 KiB");
    assert_eq!(post(Some(&events.key), Some("deploy-7"), body.clone()).await.unwrap().status(), 202);
    assert_eq!(post(Some(&events.key), Some("deploy-7"), body.clone()).await.unwrap().status(), 200, "a repeat of an idempotency key");
    assert_eq!(pull(&app, &relay, &token).await, 1, "one sealed event");

    lorca::event_triggers::tick(&app).unwrap();
    until(|| app.routine(&routine.id).unwrap().last_run_at.is_some()).await;
    assert_eq!(notices(&app, &bot_id)[0], "Event · Deploys");
    assert_eq!(app.routine(&routine.id).unwrap().events.unwrap().last_event.unwrap().summary, "Webhook request");

    // A new key: the old one stops working at once.
    let fresh = lorca::routine_events::regenerate_key(&app, &routine.id).await.unwrap();
    assert_ne!(fresh, events.key);
    assert_eq!(post(Some(&events.key), None, body.clone()).await.unwrap().status(), 401);
    assert_eq!(post(Some(&fresh), None, body.clone()).await.unwrap().status(), 202);
    // Deleting the routine removes its URL.
    lorca::routines::delete(&app, &routine.id).unwrap();
    for _ in 0..200 {
        if post(Some(&fresh), None, body.clone()).await.unwrap().status() == 404 {
            return;
        }
        tokio::time::sleep(std::time::Duration::from_millis(25)).await;
    }
    panic!("the URL outlived its routine");
}

/// Signs a delivery as GitHub does and posts it to the App's webhook.
async fn deliver_github(relay: &Relay, event: &str, delivery: &str, payload: &Value, secret: &str) -> reqwest::Response {
    let body = serde_json::to_vec(payload).unwrap();
    let mut mac = <Hmac<Sha256> as Mac>::new_from_slice(secret.as_bytes()).unwrap();
    mac.update(&body);
    let signature = format!("sha256={}", mac.finalize().into_bytes().iter().map(|byte| format!("{byte:02x}")).collect::<String>());
    reqwest::Client::new()
        .post(format!("{}/webhooks/github", relay.url))
        .header("X-GitHub-Event", event)
        .header("X-GitHub-Delivery", delivery)
        .header("X-Hub-Signature-256", signature)
        .body(body)
        .send()
        .await
        .unwrap()
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_github_watch_binds_through_the_install_and_runs_on_each_event_until_merged() {
    let calls = Arc::new(Mutex::new(Vec::new()));
    let stub = github_stub(calls.clone()).await;
    let relay = relay(Some(&stub)).await;
    let (app, token) = runner(&relay).await;
    let bot_id = bot(&app);
    let triggers = lorca::routines::Triggers { receiver: Some("github"), subject: Some("Acme/Project#42"), ..Default::default() };
    let routine = lorca::routines::create_routine(&app, &bot_id, "Login PR", "", "Tell me what happened on the pull request.", None, true, None, None, triggers).unwrap();

    // Installed on the repository but not bound to this account yet: the relay hands the link
    // that authorizes the App; where it isn't installed, the install link.
    let lorca::routine_events::Subscribed::NeedsSetup(url) = lorca::routine_events::subscribe(&app, &routine.id).await else { panic!("needs setup") };
    // The bot reads that the watch waits on the user, and its list hands a fresh link, since the
    // one it gave may have expired.
    let listed = lorca::turns::call_routines_tool(&app, &bot_id, json!({ "action": "list" })).await.unwrap();
    assert!(listed.contains("Login PR · Watches Acme/Project#42 · waits for the user to finish setting up GitHub"), "{listed}");
    let fresh = listed.split("Setup link for the user, good for an hour: ").nth(1).unwrap().lines().next().unwrap();
    assert!(fresh.starts_with(&format!("{stub}/login/oauth/authorize?")) && fresh != url, "{fresh}");
    assert!(url.starts_with(&format!("{stub}/login/oauth/authorize?client_id=client&state=")), "{url}");
    let elsewhere = lorca::api::dispatch(&app, "receivers.setup", json!({ "receiver": "github", "subject": "acme/elsewhere#1" })).await.unwrap();
    assert!(elsewhere["url"].as_str().unwrap().starts_with(&format!("{stub}/apps/lorca-test/installations/new?state=")), "{elsewhere}");
    assert_eq!(app.routine(&routine.id).unwrap().events.unwrap().status, lorca::routine_events::Listening::NeedsSetup);
    let state = url.rsplit("state=").next().unwrap().to_string();
    let no_redirects = reqwest::Client::builder().redirect(reqwest::redirect::Policy::none()).build().unwrap();
    let callback = |installation: &str, state: &str| no_redirects.get(format!("{}/webhooks/github/callback?code=c&installation_id={installation}&setup_action=install&state={state}", relay.url)).send();
    // An installation the user's sign-in does not reach binds nothing.
    let other = lorca::api::dispatch(&app, "receivers.setup", json!({ "receiver": "github" })).await.unwrap()["url"].as_str().unwrap().rsplit("state=").next().unwrap().to_string();
    let refused = callback("99", &other).await.unwrap();
    assert_eq!(refused.headers()["location"], "https://lorca.app/github/connected?status=not_yours");
    let bound = callback("77", &state).await.unwrap();
    assert_eq!(bound.status(), 303);
    assert_eq!(bound.headers()["location"], "https://lorca.app/github/connected?status=connected&account=acme");
    assert_eq!(callback("77", &state).await.unwrap().headers()["location"], "https://lorca.app/github/connected?status=expired", "a state works once");
    // An authorization alone, from the authorize link, binds what the user reaches too.
    let again = lorca::api::dispatch(&app, "receivers.setup", json!({ "receiver": "github", "subject": "acme/project#42" })).await.unwrap()["url"].as_str().unwrap().rsplit("state=").next().unwrap().to_string();
    let authorized = no_redirects.get(format!("{}/webhooks/github/callback?code=c&state={again}", relay.url)).send().await.unwrap();
    assert_eq!(authorized.headers()["location"], "https://lorca.app/github/connected?status=connected&account=acme");
    // The installation also covers acme/secret, which this user can't access: no watch there,
    // only the link to authorize again.
    let secret = lorca::routines::create_routine(&app, &bot_id, "Secret PR", "", "x", None, true, None, None, lorca::routines::Triggers { receiver: Some("github"), subject: Some("acme/secret#42"), ..Default::default() }).unwrap();
    assert!(matches!(lorca::routine_events::subscribe(&app, &secret.id).await, lorca::routine_events::Subscribed::NeedsSetup(url) if url.contains("/login/oauth/authorize?")));
    assert!(relay.state.db.receiver_subs("github", "acme/secret#").await.unwrap().is_empty());
    // The app asks again when the user is back; it waits on the same setup.
    lorca::api::dispatch(&app, "routines.subscribe", json!({ "id": secret.id })).await.unwrap();
    assert_eq!(app.routine(&secret.id).unwrap().events.unwrap().status, lorca::routine_events::Listening::NeedsSetup);
    lorca::routines::delete(&app, &secret.id).unwrap();

    assert_eq!(lorca::routine_events::subscribe(&app, &routine.id).await, lorca::routine_events::Subscribed::Subscribed);
    let listed = lorca::turns::call_routines_tool(&app, &bot_id, json!({ "action": "list" })).await.unwrap();
    assert!(listed.contains("Login PR · Watches Acme/Project#42 · listening for GitHub events") && !listed.contains("Setup link"), "{listed}");
    let events = app.routine(&routine.id).unwrap().events.unwrap();
    assert_eq!((events.title.as_str(), events.source_name.as_str()), ("Add passkey sign-in", "GitHub"));
    // A closed pull request is refused.
    let closed = lorca::routines::create_routine(&app, &bot_id, "Old PR", "", "x", None, true, None, None, lorca::routines::Triggers { receiver: Some("github"), subject: Some("acme/project#43"), ..Default::default() }).unwrap();
    assert!(matches!(lorca::routine_events::subscribe(&app, &closed.id).await, lorca::routine_events::Subscribed::Refused(why) if why.contains("already merged")));
    // The bot's tool keeps no routine the receiver refused.
    lorca::routines::delete(&app, &closed.id).unwrap();

    let repo = json!({ "full_name": "acme/project" });
    let pr = json!({ "number": 42, "title": "Add passkey sign-in", "state": "open", "html_url": "https://github.com/acme/project/pull/42", "head": { "sha": "abc" } });
    assert_eq!(deliver_github(&relay, "pull_request", "d-0", &json!({ "action": "opened", "repository": repo, "pull_request": pr }), "not-the-secret").await.status(), 401, "GitHub's signature");
    let sealed = |response: reqwest::Response| async move { response.json::<Value>().await.unwrap()["sealed"].as_u64().unwrap() };
    let review = json!({ "action": "submitted", "repository": repo, "pull_request": pr, "sender": { "login": "kim" }, "review": { "state": "changes_requested", "user": { "login": "kim" }, "body": "Please add tests" } });
    assert_eq!(sealed(deliver_github(&relay, "pull_request_review", "d-1", &review, WEBHOOK_SECRET).await).await, 1);
    assert_eq!(sealed(deliver_github(&relay, "pull_request", "d-2", &json!({ "action": "labeled", "repository": repo, "pull_request": pr }), WEBHOOK_SECRET).await).await, 0, "a label starts no run");
    let check = json!({ "action": "completed", "repository": repo, "check_run": { "name": "test", "conclusion": "failure", "head_sha": "abc", "pull_requests": [{ "number": 42 }] } });
    assert_eq!(sealed(deliver_github(&relay, "check_run", "d-3", &check, WEBHOOK_SECRET).await).await, 1);
    // A commit status names a commit: the pull request's head, which the relay knows.
    assert_eq!(sealed(deliver_github(&relay, "status", "d-4", &json!({ "state": "failure", "sha": "abc", "context": "ci/circleci", "repository": repo }), WEBHOOK_SECRET).await).await, 1);
    assert_eq!(sealed(deliver_github(&relay, "status", "d-5", &json!({ "state": "failure", "sha": "zzz", "context": "ci", "repository": repo }), WEBHOOK_SECRET).await).await, 0);
    // GitHub redelivers: one run.
    assert_eq!(sealed(deliver_github(&relay, "pull_request_review", "d-1", &review, WEBHOOK_SECRET).await).await, 1);
    assert_eq!(pull(&app, &relay, &token).await, 4);

    // The first event runs the routine, in GitHub's words; the redelivery is the same delivery.
    lorca::event_triggers::tick(&app).unwrap();
    until(|| app.routine(&routine.id).unwrap().last_run_at.is_some()).await;
    assert_eq!(notices(&app, &bot_id)[0], "Event · Login PR");
    assert_eq!(app.routine(&routine.id).unwrap().events.unwrap().last_event.unwrap().summary, "Changes requested by kim");
    let listed = lorca::event_triggers::serve(&app, "events.list", &json!({})).unwrap();
    let watching = listed["subscriptions"].as_array().unwrap().iter().find(|sub| sub["config"]["routine_id"] == routine.id.as_str()).unwrap().clone();
    assert_eq!(watching["deliveries"].as_array().unwrap().len(), 3, "three deliveries, the repeat deduplicated");

    // It merges: that event's run is the last, and the routine, its subscription, and its row
    // on the relay go.
    let merged = json!({ "action": "closed", "repository": repo, "pull_request": { "number": 42, "merged": true, "title": "Add passkey sign-in" } });
    assert_eq!(sealed(deliver_github(&relay, "pull_request", "d-6", &merged, WEBHOOK_SECRET).await).await, 1);
    assert!(relay.state.db.receiver_subs("github", "acme/project#").await.unwrap().is_empty(), "the relay forgets the watch");
    pull(&app, &relay, &token).await;
    // The events after the first run one at a time, each settled as the user would; three runs
    // that can't reach a provider pause the routine until the user resumes it.
    for _ in 0..100 {
        let Some(current) = app.routine(&routine.id) else { break };
        if !current.is_enabled {
            lorca::routines::set_enabled(&app, &routine.id, true).unwrap();
        }
        discard_failed(&app);
        lorca::event_triggers::tick(&app).unwrap();
        tokio::time::sleep(std::time::Duration::from_millis(100)).await;
    }
    assert!(app.routine(&routine.id).is_none(), "the watch ended with its last run");
    until(|| lorca::event_triggers::receiver_subscriptions(&app).unwrap().is_empty()).await;
    assert!(notices(&app, &bot_id).iter().filter(|text| text.starts_with("Event · Login PR")).count() >= 2);
    assert!(calls.lock().unwrap().contains(&"installation".to_string()));
}

#[test]
fn github_events_say_what_happened_and_concern_their_pull_requests() {
    let repo = json!({ "full_name": "Acme/Project" });
    let review = github::normalize("pull_request_review", &json!({ "action": "submitted", "repository": repo, "pull_request": { "number": 42, "title": "T" }, "review": { "state": "APPROVED", "user": { "login": "kim" } } })).unwrap();
    assert_eq!((review.repo.as_str(), review.numbers.clone(), review.payload["summary"].as_str()), ("acme/project", vec![42], Some("Approved by kim")));
    let merged = github::normalize("pull_request", &json!({ "action": "closed", "repository": repo, "pull_request": { "number": 42, "merged": true } })).unwrap();
    assert!(merged.closes && merged.payload["ends"] == true && merged.payload["kind"] == "merged");
    assert!(github::normalize("pull_request", &json!({ "action": "assigned", "repository": repo, "pull_request": { "number": 42 } })).is_none());
    assert!(github::normalize("check_run", &json!({ "action": "completed", "repository": repo, "check_run": { "conclusion": "success", "pull_requests": [{ "number": 42 }] } })).is_none());
    let comment = github::normalize("issue_comment", &json!({ "action": "created", "repository": repo, "issue": { "number": 42, "pull_request": {} }, "comment": { "user": { "login": "sam" }, "body": "x".repeat(5000) } })).unwrap();
    assert_eq!(comment.payload["data"]["comment"]["body"].as_str().unwrap().len(), 2000, "a long body is clipped");
    assert!(github::normalize("issue_comment", &json!({ "action": "created", "repository": repo, "issue": { "number": 42 }, "comment": {} })).is_none(), "an issue's comment");
    assert_eq!(github::pull_request(" Acme/Project#42 "), Some(("acme/project".into(), 42)));
    assert!(github::pull_request("acme#42").is_none());
    let body = b"{\"zen\":\"x\"}";
    let mut mac = <Hmac<Sha256> as Mac>::new_from_slice(b"secret").unwrap();
    mac.update(body);
    let header = format!("sha256={}", mac.finalize().into_bytes().iter().map(|byte| format!("{byte:02x}")).collect::<String>());
    assert!(github::signed(b"secret", body, &header));
    assert!(!github::signed(b"secret", b"{\"zen\":\"y\"}", &header));
    assert!(!github::signed(b"secret", body, "sha1=00"));
    let _ = PROTOCOL;
}

#[test]
fn an_envelope_verifies_as_the_runners_inbox_expects() {
    let sub = ReceiverSub {
        id: "r".into(),
        receiver: "webhook".into(),
        identity_pubkey: "i".into(),
        machine_pubkey: "m".into(),
        subject: String::new(),
        subscription_id: "ev-1".into(),
        generation: 2,
        secret: "a-secret-of-sixteen".into(),
        key_hash: None,
        account: None,
        state: None,
        created_at: 0,
    };
    let sealed = envelope(&sub, "d-1", "webhook", &json!({ "summary": "Webhook request" }));
    let mut event: lorca::event_triggers::Envelope = serde_json::from_value(sealed.clone()).unwrap();
    let signature = event.signature.clone();
    event.sign(&sub.secret).unwrap();
    assert_eq!(event.signature, signature, "the relay signs as the CLI does");
}
