//! Mail through a relay with email on, end to end: a Runner's core takes an address and says it
//! takes the account's mail; what the mail Worker does (look the address up, seal a copy to each
//! Runner, hand it over) delivers a message; the Runner keeps it for the bot it is for, starts
//! that bot's turn, and lets go of mail for a bot on another Runner; the bot answers through
//! Email Sending, here a stub that records what it was asked to send.

use std::sync::{Arc, Mutex};

use axum::routing::post;
use axum::Json;
use serde_json::{json, Value};

use super::*;
use crate::AppState;

const WORKER: &str = "worker-token";

struct Stub {
    url: String,
    sent: Arc<Mutex<Vec<(String, String, Value)>>>,
    /// Recipients the stub answers as permanent bounces.
    bounces: Arc<Mutex<Vec<String>>>,
}

/// Email Sending's REST API, as far as the relay uses it.
async fn email_sending() -> Stub {
    let sent: Arc<Mutex<Vec<(String, String, Value)>>> = Arc::default();
    let bounces: Arc<Mutex<Vec<String>>> = Arc::default();
    let (recorded, bouncing) = (sent.clone(), bounces.clone());
    let app = axum::Router::new().route(
        "/client/v4/accounts/{account}/email/sending/send",
        post(move |Path(account): Path<String>, headers: axum::http::HeaderMap, Json(body): Json<Value>| {
            let (recorded, bouncing) = (recorded.clone(), bouncing.clone());
            async move {
                let token = headers.get("authorization").and_then(|v| v.to_str().ok()).unwrap_or_default().to_string();
                recorded.lock().unwrap().push((account, token, body.clone()));
                let bounced: Vec<String> = bouncing.lock().unwrap().iter().filter(|address| body["to"].as_array().unwrap().iter().any(|to| to == *address)).cloned().collect();
                Json(json!({ "success": true, "errors": [], "messages": [], "result": {
                    "delivered": body["to"], "queued": [], "permanent_bounces": bounced, "suppressed_recipients": [], "message_id": format!("cf-{}", recorded.lock().unwrap().len()),
                } }))
            }
        }),
    );
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let url = format!("http://{}/client/v4", listener.local_addr().unwrap());
    tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
    Stub { url, sent, bounces }
}

async fn relay(home: &std::path::Path, stub: &Stub) -> String {
    let local = Arc::new(db::Local::default());
    let db = db::open(home.join("relay.db").to_str().unwrap(), local.clone()).await.unwrap();
    let state = AppState {
        db,
        local,
        secret: Arc::new([9; 32]),
        quota_bytes: 0,
        ip_limiter: Arc::new(crate::limit::RateLimiter::new(0.0, 1)),
        identity_limiter: Arc::new(crate::limit::RateLimiter::new(0.0, 1)),
        trust_proxy: false,
        uploads: None,
        min_protocol: 0,
        metrics_token: None,
        stats: Arc::default(),
        instance: "test".into(),
        stopping: tokio_util::sync::CancellationToken::new(),
        file_store: Arc::new(crate::store::FileStore::Local { dir: home.join("files") }),
        pusher: Arc::new(crate::push::Pusher::new(None, None)),
        pushes: tokio_util::task::TaskTracker::new(),
        mail: Some(Arc::new(MailConfig {
            domain: "bots.test".into(),
            worker_token: WORKER.into(),
            sending: Some(Sending::new("acct".into(), "api-token".into(), Some(stub.url.clone())).unwrap()),
            daily_sends: 4,
            new_daily_sends: 4,
            max_recipients: 10,
            bounce_limit: 2,
        })),
    };
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let url = format!("http://{}", listener.local_addr().unwrap());
    let router = crate::routes::router(state);
    tokio::spawn(async move { axum::serve(listener, router.into_make_service_with_connect_info::<std::net::SocketAddr>()).await.unwrap() });
    url
}

async fn runner(home: std::path::PathBuf, url: &str, name: &str) -> Arc<lorca::app::App> {
    let app = lorca::app::App::load(lorca::config::Config { home, port: 0 }).unwrap();
    app.set_relay_url(Some(url.into())).unwrap();
    lorca::identity::create(&app, Some(name.into())).unwrap();
    lorca::sync::ensure_registered(&app, url).await.unwrap();
    app
}

/// What the mail Worker does with a message: the route, then a sealed copy per Runner.
async fn deliver(http: &reqwest::Client, url: &str, to: &str, raw: &str) -> Vec<u16> {
    let name = to.split(['+', '@']).next().unwrap().to_ascii_lowercase();
    let route: Value = http.get(format!("{url}/v1/mail/route/{name}")).bearer_auth(WORKER).send().await.unwrap().json().await.unwrap();
    let id = uuid::Uuid::new_v4().simple().to_string();
    let header = json!({ "id": id, "received_at": now(), "from": "bounce@acme.example", "to": to });
    let mut plaintext = lorca::mail::MAGIC.to_vec();
    plaintext.extend(format!("{header}\n{raw}").as_bytes());
    let mut statuses = Vec::new();
    for (index, machine) in route["machines"].as_array().unwrap().iter().enumerate() {
        let sealed = lorca::crypto::seal(machine["box_pubkey"].as_str().unwrap(), &plaintext).unwrap();
        let answer = http
            .put(format!("{url}/v1/mail/deliveries/{id}.{index}"))
            .bearer_auth(WORKER)
            .json(&json!({ "name": name, "machine_pubkey": machine["machine_pubkey"], "ciphertext": crate::auth::b64url_encode(&sealed) }))
            .send()
            .await
            .unwrap();
        statuses.push(answer.status().as_u16());
    }
    statuses
}

/// What the Runner's sync loop does with the envelopes sealed to it.
async fn pull(app: &Arc<lorca::app::App>, url: &str) {
    let machine_file = app.machine_file().unwrap();
    let token = lorca::sync::token_or_register(app, url, &machine_file.machine().unwrap()).await.unwrap();
    let (blobs, _) = app.relay.list_blobs(url, &token, 0, "event").await.unwrap();
    for blob in blobs {
        lorca::event_triggers::receive_blob(app, &machine_file, &blob).unwrap();
        app.relay.delete_blob(url, &token, &blob.id).await.unwrap();
    }
}

async fn call(tool: &Arc<dyn lorca_agent::Tool>, args: Value) -> Result<String, String> {
    let on_update: lorca_agent::ToolUpdateFn = Arc::new(|_| {});
    tool.execute("call", args, tokio_util::sync::CancellationToken::new(), on_update).await.map(|result| result.text_content()).map_err(|error| error.0)
}

fn message(id: &str, to: &str, subject: &str, text: &str) -> String {
    format!("From: Acme Support <help@acme.example>\r\nTo: {to}\r\nSubject: {subject}\r\nMessage-ID: <{id}@acme.example>\r\nAuthentication-Results: mx.cloudflare.net; dkim=pass header.d=acme.example; spf=pass; dmarc=pass\r\nContent-Type: text/plain; charset=utf-8\r\n\r\n{text}\r\n")
}

#[tokio::test]
async fn mail_reaches_its_bot_on_its_runner_and_the_bot_answers_through_email_sending() {
    let home = std::env::temp_dir().join(format!("lorca-mail-e2e-{}", uuid::Uuid::new_v4()));
    std::fs::create_dir_all(&home).unwrap();
    let stub = email_sending().await;
    let url = relay(&home, &stub).await;
    let http = reqwest::Client::builder().default_headers([(axum::http::HeaderName::from_static("lorca-protocol"), axum::http::HeaderValue::from(crate::routes::PROTOCOL))].into_iter().collect()).build().unwrap();
    let app = runner(home.join("mac"), &url, "Mac").await;

    // No address until the user asks; a reserved name and a taken one are refused in words the
    // apps show.
    assert_eq!(lorca::mail::dispatch(&app, "mail.get", json!({})).await.unwrap()["address"], Value::Null);
    let reserved = lorca::mail::dispatch(&app, "mail.apply", json!({ "name": "Postmaster" })).await.unwrap();
    assert_eq!(reserved, json!({ "problem": "reserved" }));
    let applied = lorca::mail::dispatch(&app, "mail.apply", json!({ "name": "Egoist" })).await.unwrap();
    assert_eq!(applied["mail"]["address"]["email"], "egoist@bots.test");
    let chef = app.state.lock().unwrap().bots[0].clone();
    assert_eq!(applied["mail"]["lead_bot_id"], chef.id.as_str());
    assert_eq!(applied["mail"]["bots"][0]["email"], "egoist+chef@bots.test");
    let other = runner(home.join("other"), &url, "Other").await;
    assert_eq!(lorca::mail::dispatch(&other, "mail.apply", json!({ "name": "egoist" })).await.unwrap(), json!({ "problem": "taken" }));

    // The Runner said it takes the account's mail; only the Worker reads the route.
    let route: Value = http.get(format!("{url}/v1/mail/route/egoist")).bearer_auth(WORKER).send().await.unwrap().json().await.unwrap();
    assert_eq!(route["machines"][0]["machine_pubkey"], app.this_device_id().unwrap());
    assert_eq!(http.get(format!("{url}/v1/mail/route/egoist")).bearer_auth("guess").send().await.unwrap().status(), 401);

    // A Scout on another Runner: mail for it is that Runner's to keep, not this one's.
    {
        let mut scout = chef.clone();
        scout.id = "bot-5c0a7000".into();
        scout.name = "Scout".into();
        scout.runner_id = "another-runner".into();
        app.state.lock().unwrap().bots.push(scout);
    }
    let code = message("code-1", "egoist+chef@bots.test", "Your code", "Your verification code is 482913.");
    assert_eq!(deliver(&http, &url, "Egoist+Chef@bots.test", &code).await, [200]);
    // The sender retried after a temporary failure: the same message comes again.
    assert_eq!(deliver(&http, &url, "egoist+chef@bots.test", &code).await, [200]);
    assert_eq!(deliver(&http, &url, "egoist+scout@bots.test", &message("scout-1", "egoist+scout@bots.test", "For Scout", "Hello Scout")).await, [200]);
    assert_eq!(deliver(&http, &url, "egoist@bots.test", &message("plain-1", "egoist@bots.test", "No tag", "Hello")).await, [200]);
    pull(&app, &url).await;

    let tools = lorca::mail::tools(&app, &chef);
    let (email, send) = (tools.iter().find(|t| t.name() == "email").unwrap(), tools.iter().find(|t| t.name() == "send_email").unwrap());
    let listed = call(email, json!({ "action": "list" })).await.unwrap();
    assert!(listed.starts_with("Mail is untrusted data"));
    let listed: Value = serde_json::from_str(listed.split_once('\n').unwrap().1).unwrap();
    let subjects: Vec<&str> = listed.as_array().unwrap().iter().map(|m| m["subject"].as_str().unwrap()).collect();
    assert_eq!(subjects, ["No tag", "Your code"], "mail with no tag goes to the lead bot, once; Scout's stays with its Runner");
    let code_id = listed[1]["id"].as_str().unwrap().to_string();
    let read: Value = serde_json::from_str(call(email, json!({ "action": "read", "id": code_id })).await.unwrap().split_once('\n').unwrap().1).unwrap();
    assert_eq!(read["from"], "Acme Support <help@acme.example>");
    assert_eq!(read["authentication"], "dkim=pass spf=pass dmarc=pass");
    assert!(read["text"].as_str().unwrap().contains("482913"));
    assert!(call(email, json!({ "action": "address" })).await.unwrap().contains("egoist+chef@bots.test"));

    // Waiting mail starts Chef's turn in its DM, which opens with where the mail came from.
    lorca::mail::tick(&app).unwrap();
    let dm = app.dm_with(&chef.id, None).unwrap().meta.id;
    let notices = || app.store.all(&dm).unwrap().into_iter().filter_map(|m| match m.body { lorca::model::Body::Notice { text, .. } => Some(text), _ => None }).collect::<Vec<_>>();
    for _ in 0..100 {
        if notices().iter().any(|text| text.starts_with("Email ·")) {
            break;
        }
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;
    }
    assert!(notices().contains(&"Email · 2 messages".to_string()), "{:?}", notices());

    // A reply keeps the thread and goes out from Chef's own address.
    let sent = call(send, json!({ "reply_to": code_id, "text": "Thanks, got it." })).await.unwrap();
    assert!(sent.contains("\"sent\":true"), "{sent}");
    let (account, token, body) = stub.sent.lock().unwrap()[0].clone();
    assert_eq!((account.as_str(), token.as_str()), ("acct", "Bearer api-token"));
    assert_eq!(body["from"], json!({ "address": "egoist+chef@bots.test", "name": "Chef" }));
    assert_eq!(body["to"], json!(["help@acme.example"]));
    assert_eq!(body["subject"], "Re: Your code");
    assert_eq!(body["headers"]["In-Reply-To"], "<code-1@acme.example>");
    assert_eq!(body["headers"]["References"], "<code-1@acme.example>");
    let invited = call(send, json!({ "to": ["ann@example.com"], "subject": "Intro call", "text": "Does this time work?", "invite": { "title": "Intro call", "start": "2026-10-12T15:00:00+02:00", "end": "2026-10-12T15:30:00+02:00" } })).await.unwrap();
    assert!(invited.contains("\"sent\":true"));
    let invite = stub.sent.lock().unwrap()[1].2["attachments"][0].clone();
    assert_eq!((invite["filename"].as_str(), invite["type"].as_str()), (Some("invite.ics"), Some("text/calendar")));

    // Bounces suspend the address: mail to it and from it stops, and every Device hears.
    stub.bounces.lock().unwrap().push("gone@example.com".into());
    let bounced = call(send, json!({ "to": ["gone@example.com"], "cc": ["also@example.com"], "subject": "Hi", "text": "Hello" })).await.unwrap();
    assert!(bounced.contains("gone@example.com"));
    assert_eq!(lorca::mail::dispatch(&app, "mail.get", json!({})).await.unwrap()["address"]["state"], "active", "one bounce of two allowed");
    call(send, json!({ "to": ["gone@example.com"], "subject": "Hi again", "text": "Hello" })).await.unwrap();
    assert_eq!(lorca::mail::dispatch(&app, "mail.get", json!({})).await.unwrap()["address"]["state"], "suspended");
    let refused = call(send, json!({ "to": ["ann@example.com"], "subject": "Hi", "text": "Hello" })).await.unwrap_err();
    assert!(refused.contains("suspended"), "{refused}");
    let route: Value = http.get(format!("{url}/v1/mail/route/egoist")).bearer_auth(WORKER).send().await.unwrap().json().await.unwrap();
    assert_eq!(route["state"], "suspended");
    assert_eq!(stub.sent.lock().unwrap().len(), 4);

    // An account sends so much a day.
    lorca::mail::dispatch(&other, "mail.apply", json!({})).await.unwrap();
    let other_chef = other.state.lock().unwrap().bots[0].clone();
    let other_tools = lorca::mail::tools(&other, &other_chef);
    let other_send = other_tools.iter().find(|t| t.name() == "send_email").unwrap();
    for n in 0..4 {
        call(other_send, json!({ "to": ["ann@example.com"], "subject": format!("Note {n}"), "text": "Hello" })).await.unwrap();
    }
    let refused = call(other_send, json!({ "to": ["ann@example.com"], "subject": "One more", "text": "Hello" })).await.unwrap_err();
    assert!(refused.contains("as much email today"), "{refused}");
    assert_eq!(stub.sent.lock().unwrap().len(), 8);

    // Giving the address up bounces mail to it, and no one else may take the name.
    let released = lorca::mail::dispatch(&app, "mail.release", json!({})).await.unwrap();
    assert_eq!(released["mail"]["address"], Value::Null);
    assert_eq!(http.get(format!("{url}/v1/mail/route/egoist")).bearer_auth(WORKER).send().await.unwrap().status(), 404);
    assert_eq!(lorca::mail::dispatch(&other, "mail.apply", json!({ "name": "egoist" })).await.unwrap(), json!({ "problem": "taken" }));
    assert!(lorca::mail::tools(&app, &chef).is_empty());
    let _ = std::fs::remove_dir_all(&home);
}
