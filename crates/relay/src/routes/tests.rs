use std::sync::Arc;

use super::*;
use lorca::app::OutboxItem;
use lorca::relay::RelayClient;

#[tokio::test]
async fn encrypted_task_records_round_trip_and_replace_only_their_own_slot() {
    fn task_outbox(app: &lorca::app::App) -> OutboxItem {
        loop {
            let item = app.store.first_outbox().unwrap().unwrap();
            let state = app.state.lock().unwrap().clone();
            app.store.remove_outbox_with_state(&item.id, &state).unwrap();
            if item.kind == "task" { return item; }
        }
    }
    let relay = Relay::start(0).await;
    let app = lorca::app::App::load(lorca::config::Config { home: relay.home.join("source"), port: 0 }).unwrap();
    lorca::identity::create(&app, Some("Source".into())).unwrap();
    let bot = app.state.lock().unwrap().bots[0].clone();
    let chat_id = app.state.lock().unwrap().chats[0].meta.id.clone();
    let created = lorca::api::dispatch(&app, "tasks.create", serde_json::json!({
        "request_id":"create","owner_bot_id":bot.id,"chat_ids":[chat_id],
        "goal":"Encrypted durable work","acceptance_criteria":["Verified"],"next_action":"Inspect"
    })).await.unwrap();
    let id = created["id"].as_str().unwrap();
    let first = task_outbox(&app);
    assert!(!String::from_utf8_lossy(&first.ciphertext).contains("Encrypted durable work"));
    let uploaded = relay.client.put_blob(&relay.url, &relay.token, first.clone()).await.unwrap();
    assert_eq!(relay.client.put_blob(&relay.url, &relay.token, first).await.unwrap(), uploaded);
    let (page, _) = relay.client.list_blobs(&relay.url, &relay.token, 0, "task").await.unwrap();
    assert_eq!(page.len(), 1);
    let replica = lorca::app::App::load(lorca::config::Config { home: relay.home.join("replica"), port: 0 }).unwrap();
    lorca::identity::create(&replica, Some("Replica".into())).unwrap();
    replica.machine.lock().unwrap().as_mut().unwrap().account_dek = app.machine_file().unwrap().account_dek;
    let machine = replica.machine_file().unwrap();
    lorca::sync::apply_blob(&replica, &machine, &page[0]);
    assert_eq!(lorca::tasks::get(&replica, id).unwrap().revision, 1);
    lorca::api::dispatch(&app, "tasks.update", serde_json::json!({
        "id":id,"request_id":"progress","expected_revision":1,"next_action":"Verify"
    })).await.unwrap();
    let newest = task_outbox(&app);
    relay.client.put_blob(&relay.url, &relay.token, newest).await.unwrap();
    let (new_page, _) = relay.client.list_blobs(&relay.url, &relay.token, 0, "task").await.unwrap();
    assert_eq!(new_page.len(), 1);
    lorca::sync::apply_blob(&replica, &machine, &new_page[0]);
    assert_eq!(lorca::tasks::get(&replica, id).unwrap().next_action, "Verify");
    lorca::sync::apply_blob(&replica, &machine, &page[0]);
    assert_eq!(lorca::tasks::get(&replica, id).unwrap().revision, 2);
    for kind in ["roster","chat","job","job_cancel","job_result","request","response","machine","credentials","key","file","task","handoff","review","attention","project_context","event"] {
        assert!(db::KINDS.contains(&kind));
    }
    assert!(db::SEALED_KINDS.contains(&"event"));
    assert_eq!(PROTOCOL, lorca::relay::PROTOCOL);
}

struct Relay {
    state: AppState,
    url: String,
    token: String,
    client: RelayClient,
    http: reqwest::Client,
    home: std::path::PathBuf,
    task: tokio::task::JoinHandle<()>,
}

impl Relay {
    async fn start(quota_bytes: u64) -> Self {
        Self::start_with(quota_bytes, crate::push::Pusher::new(None, None)).await
    }

    async fn start_with(quota_bytes: u64, pusher: crate::push::Pusher) -> Self {
        let home = std::env::temp_dir().join(format!("lorca-binary-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&home).unwrap();
        let local = Arc::new(db::Local::default());
        let db = db::open(home.join("relay.db").to_str().unwrap(), local.clone()).await.unwrap();
        db.register_identity("identity", "content", "machine", "box", "attestation").await.unwrap();
        let state = AppState {
            db,
            local,
            secret: Arc::new([7; 32]),
            quota_bytes,
            ip_limiter: Arc::new(crate::limit::RateLimiter::new(0.0, 1)),
            identity_limiter: Arc::new(crate::limit::RateLimiter::new(0.0, 1)),
            trust_proxy: false,
            uploads: Some(Arc::new(tokio::sync::Semaphore::new(1))),
            min_protocol: PROTOCOL,
            metrics_token: None,
            stats: Arc::new(crate::metrics::StatsCache::default()),
            instance: "test".into(),
            stopping: tokio_util::sync::CancellationToken::new(),
            file_store: Arc::new(crate::store::FileStore::Local { dir: home.join("files") }),
            pusher: Arc::new(pusher),
            pushes: tokio_util::task::TaskTracker::new(),
        };
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let url = format!("http://{}", listener.local_addr().unwrap());
        let app = router(state.clone());
        let task = tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
        let token = issue_token(&state.secret, "identity", "machine").0;
        let mut headers = reqwest::header::HeaderMap::new();
        headers.insert("lorca-protocol", PROTOCOL.into());
        let http = reqwest::Client::builder().default_headers(headers).build().unwrap();
        Self {
            state,
            url,
            token,
            client: RelayClient::new().unwrap(),
            http,
            home,
            task,
        }
    }

    fn put(&self, id: &str) -> reqwest::RequestBuilder {
        self.http
            .put(format!("{}/v1/files/{id}", self.url))
            .bearer_auth(&self.token)
            .header(header::CONTENT_TYPE, "application/octet-stream")
    }

    async fn upload(&self, id: &str, group: Option<&str>, ciphertext: Vec<u8>) -> i64 {
        self.client.put_blob(&self.url, &self.token, file(id, group, ciphertext)).await.unwrap()
    }
}

impl Drop for Relay {
    fn drop(&mut self) {
        self.task.abort();
        let _ = std::fs::remove_dir_all(&self.home);
    }
}

fn file(id: &str, group: Option<&str>, ciphertext: Vec<u8>) -> OutboxItem {
    OutboxItem {
        id: id.into(),
        kind: "file".into(),
        recipient: None,
        ciphertext,
        slot: None,
        group: group.map(str::to_string),
    }
}

#[tokio::test]
async fn binary_attachment_round_trip_stays_encrypted_and_idempotent() {
    let relay = Relay::start(0).await;
    let plaintext = b"attachment\0\xff\xfe\x80\r\n";
    let ciphertext = lorca::crypto::encrypt(&[9; 32], "file", plaintext).unwrap();
    assert_eq!(ciphertext.len(), plaintext.len() + lorca::crypto::ENVELOPE_OVERHEAD);
    let seq = relay.upload("att-file", Some("chat"), ciphertext.clone()).await;
    assert_eq!(relay.upload("att-file", Some("chat"), vec![0; 10]).await, seq);
    let downloaded = relay.client.get_file(&relay.url, &relay.token, "att-file").await.unwrap().unwrap();
    assert_eq!(downloaded, ciphertext);
    assert_eq!(lorca::crypto::decrypt(&[9; 32], "file", &downloaded).unwrap(), plaintext);
    assert_eq!(std::fs::read(relay.home.join("files/identity/att-file")).unwrap(), ciphertext);
    let row = relay.state.db.blob("identity", "machine", "att-file").await.unwrap().unwrap();
    assert!(row.ciphertext.is_empty());
    let page = relay.client.list_blobs(&relay.url, &relay.token, 0, "").await.unwrap();
    assert!(page.0.is_empty(), "JSON sync pages contain no attachment payloads");
    let chat = OutboxItem {
        id: "message".into(),
        kind: "chat".into(),
        recipient: None,
        ciphertext: vec![0, 255, 128],
        slot: None,
        group: Some("chat".into()),
    };
    relay.client.put_blob(&relay.url, &relay.token, chat).await.unwrap();
    let (blobs, _) = relay.client.list_blobs(&relay.url, &relay.token, 0, "").await.unwrap();
    assert_eq!(blobs.len(), 1);
    assert_eq!(lorca::keys::unb64(&blobs[0].ciphertext).unwrap(), [0, 255, 128]);
    assert_eq!(relay.client.get_file(&relay.url, &relay.token, "message").await.unwrap(), None);

    let other = issue_token(&relay.state.secret, "other-identity", "other-machine").0;
    assert!(relay.client.get_file(&relay.url, &other, "att-file").await.unwrap().is_none());
    relay.client.delete_group(&relay.url, &relay.token, "chat").await.unwrap();
    assert!(relay.client.get_file(&relay.url, &relay.token, "att-file").await.unwrap().is_none());
    assert!(!relay.home.join("files/identity/att-file").exists());
    let error = relay
        .client
        .put_blob(&relay.url, &relay.token, file("att-late", Some("chat"), vec![1]))
        .await
        .unwrap_err();
    assert_eq!(error.status, Some(409));
    assert!(!relay.home.join("files/identity/att-late").exists());
}

#[tokio::test]
async fn encrypted_review_blobs_round_trip_and_keep_the_latest_version() {
    let relay = Relay::start(0).await;
    let make = |id: &str, version: u64| OutboxItem { id: id.into(), kind: "review".into(), recipient: None,
        ciphertext: lorca::crypto::encrypt_json(&[9; 32], "review", &json!({ "id": "review-one", "version": version, "draft": "private draft" })).unwrap(),
        slot: Some(lorca::app::Slot::latest(lorca::model::relay_name("review/review-one"))), group: None };
    let first = make("review-first", 1);
    relay.client.put_blob(&relay.url, &relay.token, first.clone()).await.unwrap();
    relay.client.put_blob(&relay.url, &relay.token, first).await.unwrap();
    let latest = make("review-last", 2);
    relay.client.put_blob(&relay.url, &relay.token, latest.clone()).await.unwrap();
    let (blobs, _) = relay.client.list_blobs(&relay.url, &relay.token, 0, "review").await.unwrap();
    assert_eq!(blobs.len(), 1);
    assert_eq!(blobs[0].id, "review-last");
    let ciphertext = lorca::keys::unb64(&blobs[0].ciphertext).unwrap();
    assert_eq!(ciphertext, latest.ciphertext);
    let content: Value = lorca::crypto::decrypt_json(&[9; 32], "review", &ciphertext).unwrap();
    assert_eq!(content["version"], 2);
}

#[tokio::test]
async fn binary_files_enforce_auth_metadata_quota_and_missing_objects() {
    let relay = Relay::start(4).await;
    let unauthenticated = relay
        .http
        .put(format!("{}/v1/files/att-file", relay.url))
        .header(header::CONTENT_TYPE, "application/octet-stream")
        .body(vec![1])
        .send()
        .await
        .unwrap();
    assert_eq!(unauthenticated.status(), StatusCode::UNAUTHORIZED);
    assert_eq!(relay.put("att-empty").body(Vec::new()).send().await.unwrap().status(), StatusCode::BAD_REQUEST);
    assert_eq!(
        relay
            .http
            .put(format!("{}/v1/files/att-file", relay.url))
            .bearer_auth(&relay.token)
            .header(header::CONTENT_TYPE, "application/json")
            .body(vec![1])
            .send()
            .await
            .unwrap()
            .status(),
        StatusCode::UNSUPPORTED_MEDIA_TYPE
    );
    assert_eq!(
        relay.put("att-file").query(&[("group", "../bad")]).body(vec![1]).send().await.unwrap().status(),
        StatusCode::BAD_REQUEST
    );
    assert_eq!(
        relay
            .put("att-file")
            .query(&[("slot", "not-allowed")])
            .body(vec![1])
            .send()
            .await
            .unwrap()
            .status(),
        StatusCode::BAD_REQUEST
    );
    let error = relay
        .client
        .put_blob(&relay.url, &relay.token, file("att-large", None, vec![1; 5]))
        .await
        .unwrap_err();
    assert_eq!(error.status, Some(413));
    assert!(!relay.home.join("files/identity/att-large").exists());

    relay.upload("att-avatar", None, vec![0, 255, 128, 1]).await;
    std::fs::remove_file(relay.home.join("files/identity/att-avatar")).unwrap();
    assert!(relay.client.get_file(&relay.url, &relay.token, "att-avatar").await.unwrap().is_none());
    assert!(relay.state.db.blob("identity", "machine", "att-avatar").await.unwrap().is_some());
    relay.client.delete_blob(&relay.url, &relay.token, "att-avatar").await.unwrap();
    relay.upload("att-replacement", None, vec![2; 4]).await;

    let response = relay
        .http
        .put(format!("{}/v1/blobs", relay.url))
        .bearer_auth(&relay.token)
        .json(&json!({ "id": "att-json", "kind": "file", "ciphertext": "AQ" }))
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::BAD_REQUEST);
    let response = relay.put("att-old").header("lorca-protocol", "0").body(vec![1]).send().await.unwrap();
    assert_eq!(response.status(), StatusCode::UPGRADE_REQUIRED);
}

#[tokio::test]
async fn binary_file_limit_includes_the_encryption_envelope_and_caps_chunked_bodies() {
    let relay = Relay::start(0).await;
    assert_eq!(
        MAX_FILE_BLOB_BYTES,
        lorca::files::MAX_ATTACHMENT_BYTES as usize + lorca::crypto::ENVELOPE_OVERHEAD
    );
    relay.upload("att-boundary", None, vec![0x80; MAX_FILE_BLOB_BYTES]).await;
    let downloaded = relay.client.get_file(&relay.url, &relay.token, "att-boundary").await.unwrap().unwrap();
    assert_eq!(downloaded.len(), MAX_FILE_BLOB_BYTES);
    assert!(downloaded.iter().all(|&byte| byte == 0x80));
    drop(downloaded);
    let chunks = futures::stream::iter((0..101).map(|_| Ok::<_, std::io::Error>(Bytes::from(vec![0; 1024 * 1024]))));
    let response = relay.put("att-overflow").body(reqwest::Body::wrap_stream(chunks)).send().await.unwrap();
    assert_eq!(response.status(), StatusCode::PAYLOAD_TOO_LARGE);
    assert!(!relay.home.join("files/identity/att-overflow").exists());
}

/// A relay told to stop still hands APNs the pushes it answered `queued` for, and stops waiting
/// for one APNs never takes once the grace is up.
#[tokio::test]
async fn a_stopping_relay_delivers_the_pushes_it_took() {
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::time::{Duration, Instant};

    /// A relay whose identity has a phone that takes pushes at `phone_token`.
    async fn relay_with_phone(apns_url: &str, phone_token: &str) -> Relay {
        let apns = crate::push::Apns::new(crate::push::tests::P8, "KEYID12345".into(), "TEAMID1234".into(), "app.lorca".into(), Some(apns_url.into())).unwrap();
        let relay = Relay::start_with(0, crate::push::Pusher::new(Some(apns), None)).await;
        relay.state.db.register_identity("identity", "content", "phone", "phone-box", "attestation").await.unwrap();
        let token = db::PushToken { machine_pubkey: "phone".into(), platform: "apns".into(), token: phone_token.into(), environment: "sandbox".into() };
        relay.state.db.set_push_token("identity", &token).await.unwrap();
        relay
    }

    // APNs takes a while over the token `slow`, and never answers for `stuck`.
    let delivered = Arc::new(AtomicUsize::new(0));
    let apns = Router::new().route("/3/device/{token}", post({
        let delivered = delivered.clone();
        move |Path(token): Path<String>| async move {
            if token == "stuck" {
                std::future::pending::<()>().await;
            }
            tokio::time::sleep(Duration::from_millis(300)).await;
            delivered.fetch_add(1, Ordering::Relaxed);
            StatusCode::OK
        }
    }));
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let apns_url = format!("http://{}", listener.local_addr().unwrap());
    let apns = tokio::spawn(async move { axum::serve(listener, apns).await.unwrap() });

    let relay = relay_with_phone(&apns_url, "slow").await;
    assert_eq!(relay.client.push(&relay.url, &relay.token, &b64url_encode(b"notice")).await.unwrap(), 1);
    crate::stop(&relay.state, Duration::from_secs(5)).await;
    assert_eq!(delivered.load(Ordering::Relaxed), 1, "the push reached APNs before the relay stopped");

    let relay = relay_with_phone(&apns_url, "stuck").await;
    assert_eq!(relay.client.push(&relay.url, &relay.token, &b64url_encode(b"notice")).await.unwrap(), 1);
    let started = Instant::now();
    crate::stop(&relay.state, Duration::from_millis(300)).await;
    let waited = started.elapsed();
    assert!(waited >= Duration::from_millis(300) && waited < Duration::from_secs(3), "waited {waited:?}");
    assert_eq!(delivered.load(Ordering::Relaxed), 1);
    apns.abort();
}

/// A computer that joined by pairing pairs the next one: the identity device attests the first
/// with the identity key, that one attests the next with its machine key, and the newest gets
/// the account. Once unpaired, a computer pairs nobody.
#[tokio::test]
async fn a_paired_computer_pairs_another() {
    let relay = Relay::start(0).await;
    let app = |name: &str| lorca::app::App::load(lorca::config::Config { home: relay.home.join(name), port: 0 }).unwrap();
    let (first, second, third) = (app("first"), app("second"), app("third"));
    lorca::identity::create(&first, Some("First".into())).unwrap();
    first.set_relay_url(Some(relay.url.clone())).unwrap();

    let (_, code) = lorca::pairing::start(first.clone()).await.unwrap();
    lorca::pairing::accept(second.clone(), &code, Some("Second".into())).await.unwrap();
    assert!(!second.is_identity_device());

    let (nonce, code) = lorca::pairing::start(second.clone()).await.unwrap();
    lorca::pairing::accept(third.clone(), &code, Some("Third".into())).await.unwrap();
    let (account, joined) = (first.machine_file().unwrap(), third.machine_file().unwrap());
    assert_eq!(joined.identity_pubkey, account.identity_pubkey);
    assert_eq!(joined.account_dek, account.account_dek);
    assert_eq!(relay.state.db.machines_for(&account.identity_pubkey).await.unwrap().len(), 3);
    third.relay.token(&relay.url, &joined.machine().unwrap()).await.unwrap();
    for _ in 0..50 {
        if lorca::pairing::status(&second, &nonce)["state"] == "completed" {
            break;
        }
        tokio::time::sleep(std::time::Duration::from_millis(100)).await;
    }
    assert_eq!(lorca::pairing::status(&second, &nonce)["device"]["name"], "Third");

    lorca::sync::unpair_device(&first, &second.this_device_id().unwrap()).await.unwrap();
    let error = lorca::pairing::start(second.clone()).await.unwrap_err();
    assert!(error.to_string().contains("unpaired"), "{error}");
}

/// `POST /v1/machines` takes a paired machine's bearer and new keys, refuses a key paired with
/// other keys, and answers `410` for a revoked one.
#[tokio::test]
async fn attesting_a_machine_takes_a_bearer_and_keeps_keys_apart() {
    let relay = Relay::start(0).await;
    let key = |seed: u8| b64url_encode(ed25519_dalek::SigningKey::from_bytes(&[seed; 32]).verifying_key().as_bytes());
    let (laptop, box_key) = (key(1), b64url_encode(&[2; 32]));
    relay.client.attest(&relay.url, &relay.token, &laptop, &box_key).await.unwrap();
    relay.client.attest(&relay.url, &relay.token, &laptop, &box_key).await.unwrap();
    assert_eq!(relay.state.db.machines_for("identity").await.unwrap().len(), 2);

    let refused = relay.client.attest(&relay.url, &relay.token, &laptop, &b64url_encode(&[3; 32])).await.unwrap_err();
    assert_eq!(refused.status, Some(409));
    let refused = relay.client.attest(&relay.url, "not-a-token", &key(4), &box_key).await.unwrap_err();
    assert_eq!(refused.status, Some(401));
    let refused = relay.client.attest(&relay.url, &relay.token, "not-a-key", &box_key).await.unwrap_err();
    assert_eq!(refused.status, Some(400));

    assert!(relay.state.db.revoke_machine("identity", &laptop).await.unwrap());
    let refused = relay.client.attest(&relay.url, &relay.token, &laptop, &box_key).await.unwrap_err();
    assert_eq!(refused.status, Some(410));
}
