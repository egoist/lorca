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
    for kind in ["roster","chat","job","job_cancel","job_result","request","response","machine","credentials","key","file","task","handoff","review","attention","project_context","playbook","event"] {
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

#[tokio::test]
async fn event_blobs_require_a_recipient_and_travel_as_opaque_ciphertext() {
    let relay = Relay::start(0).await;
    let item = OutboxItem { id: "event-envelope".into(), kind: "event".into(), recipient: Some("machine".into()), ciphertext: b"opaque encrypted event".to_vec(), slot: None, group: None };
    let seq = relay.client.put_blob(&relay.url, &relay.token, item.clone()).await.unwrap();
    assert_eq!(relay.client.put_blob(&relay.url, &relay.token, item).await.unwrap(), seq);
    let (blobs, _) = relay.client.list_blobs(&relay.url, &relay.token, 0, "event").await.unwrap();
    assert_eq!(blobs.len(), 1);
    assert_eq!(lorca::keys::unb64(&blobs[0].ciphertext).unwrap(), b"opaque encrypted event");
    let response = relay.http.put(format!("{}/v1/blobs", relay.url)).bearer_auth(&relay.token)
        .json(&json!({"id": "broadcast-event", "kind": "event", "ciphertext": lorca::keys::b64(b"ciphertext")})).send().await.unwrap();
    assert_eq!(response.status(), StatusCode::BAD_REQUEST);
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

#[tokio::test]
async fn shared_links_are_read_by_anyone_and_changed_by_their_owner() {
    let relay = Relay::start(0).await;
    let put = |token: String, id: &str, bytes: &[u8]| {
        let (client, url, id, bytes) = (&relay.client, relay.url.clone(), id.to_string(), bytes.to_vec());
        async move { client.put_share(&url, &token, &id, bytes).await }
    };
    put(relay.token.clone(), "link-1", b"sealed").await.unwrap();

    // A browser reads it with no token and no protocol, and may read the answer.
    let read = reqwest::get(format!("{}/v1/shares/link-1", relay.url)).await.unwrap();
    assert_eq!(read.status(), 200);
    assert_eq!(read.headers()["access-control-allow-origin"], "*");
    assert_eq!(read.bytes().await.unwrap().as_ref(), b"sealed");
    let missing = reqwest::get(format!("{}/v1/shares/nothing", relay.url)).await.unwrap();
    assert_eq!(missing.status(), 404);
    assert_eq!(missing.headers()["access-control-allow-origin"], "*");
    assert!(relay.client.get_share(&relay.url, "nothing").await.unwrap().is_none());

    // The owner replaces it under the same id; another identity can neither replace nor delete it.
    put(relay.token.clone(), "link-1", b"sealed again").await.unwrap();
    assert_eq!(relay.client.get_share(&relay.url, "link-1").await.unwrap().unwrap(), b"sealed again");
    relay.state.db.register_identity("other", "content-2", "machine-2", "box-2", "attestation").await.unwrap();
    let other = issue_token(&relay.state.secret, "other", "machine-2").0;
    assert_eq!(put(other.clone(), "link-1", b"taken").await.unwrap_err().status, Some(403));
    assert_eq!(relay.client.delete_share(&relay.url, &other, "link-1").await.unwrap_err().status, Some(404));
    assert_eq!(relay.client.get_share(&relay.url, "link-1").await.unwrap().unwrap(), b"sealed again");

    // A template's size at most, and so many links per identity.
    let too_large = vec![0u8; MAX_SHARE_BYTES + 1];
    assert_eq!(put(relay.token.clone(), "link-big", &too_large).await.unwrap_err().status, Some(413));
    for i in 1..MAX_SHARES {
        put(relay.token.clone(), &format!("link-q{i}"), b"q").await.unwrap();
    }
    assert_eq!(put(relay.token.clone(), "link-over", b"q").await.unwrap_err().status, Some(409));
    put(relay.token.clone(), "link-1", b"replacing is not another link").await.unwrap();

    relay.client.delete_share(&relay.url, &relay.token, "link-1").await.unwrap();
    assert!(relay.client.get_share(&relay.url, "link-1").await.unwrap().is_none());
    // An identity that goes takes its links along.
    relay.state.db.delete_identity("identity", false).await.unwrap();
    assert!(relay.client.get_share(&relay.url, "link-q1").await.unwrap().is_none());
}

#[tokio::test]
async fn attention_slots_round_trip_as_ciphertext_at_protocol_three() {
    assert_eq!(PROTOCOL, lorca::relay::PROTOCOL);
    let relay = Relay::start(1_000_000).await;
    let dek = lorca::keys::random_32();
    let payload = serde_json::json!({"source":{"chat_id":"private-chat"},"summary":"Private decision"});
    for (id, summary) in [("attention-first", "Private decision"), ("attention-latest", "Updated decision")] {
        let mut payload = payload.clone(); payload["summary"] = summary.into();
        relay.client.put_blob(&relay.url, &relay.token, OutboxItem {
            id: id.into(), kind: "attention".into(), recipient: None,
            ciphertext: lorca::crypto::encrypt_json(&dek, "attention", &payload).unwrap(),
            slot: Some(lorca::app::Slot::latest("opaque-attention-slot")), group: None,
        }).await.unwrap();
    }
    let (blobs, _) = relay.client.list_blobs(&relay.url, &relay.token, 0, "attention").await.unwrap();
    assert_eq!(blobs.len(), 1);
    assert_eq!(blobs[0].id, "attention-latest");
    let bytes = lorca::keys::unb64(&blobs[0].ciphertext).unwrap();
    let opened: serde_json::Value = lorca::crypto::decrypt_json(&dek, "attention", &bytes).unwrap();
    assert_eq!(opened["summary"], "Updated decision");
    assert!(lorca::crypto::decrypt_json::<serde_json::Value>(&lorca::keys::random_32(), "attention", &bytes).is_err());
    let response = relay.http.get(format!("{}/v1/blobs?kinds=attention", relay.url)).bearer_auth(&relay.token).header("lorca-protocol", "2").send().await.unwrap();
    assert_eq!(response.status(), reqwest::StatusCode::UPGRADE_REQUIRED);
}

#[tokio::test]
async fn handoff_request_report_and_cancellation_round_trip_in_independent_slots() {
    let relay = Relay::start(0).await;
    let key = [42; 32];
    let make = |id: &str, role: &str, text: &str| OutboxItem {
        id: id.into(), kind: "handoff".into(), recipient: None,
        ciphertext: lorca::crypto::encrypt(&key, "handoff", text.as_bytes()).unwrap(),
        slot: Some(lorca::app::Slot::latest(format!("handoff-test-1-{role}"))), group: None,
    };
    relay.client.put_blob(&relay.url, &relay.token, make("request", "request", "contract")).await.unwrap();
    relay.client.put_blob(&relay.url, &relay.token, make("started", "report", "running")).await.unwrap();
    relay.client.put_blob(&relay.url, &relay.token, make("finished", "report", "completed with evidence")).await.unwrap();
    relay.client.put_blob(&relay.url, &relay.token, make("cancel", "cancel", "requester cancelled")).await.unwrap();
    let (rows, _) = relay.client.list_blobs(&relay.url, &relay.token, 0, lorca::sync::POLL_KINDS).await.unwrap();
    assert_eq!(rows.iter().map(|r| r.id.as_str()).collect::<Vec<_>>(), ["request", "finished", "cancel"]);
    for (row, expected) in rows.iter().zip(["contract", "completed with evidence", "requester cancelled"]) {
        let ciphertext = lorca::keys::unb64(&row.ciphertext).unwrap();
        assert_ne!(ciphertext, expected.as_bytes());
        assert_eq!(lorca::crypto::decrypt(&key, "handoff", &ciphertext).unwrap(), expected.as_bytes());
    }
    let health: Value = relay.http.get(format!("{}/v1/health", relay.url)).send().await.unwrap().json().await.unwrap();
    assert_eq!(health["protocol"], 3);
    assert!(!db::SEALED_KINDS.contains(&"handoff"));
}

#[tokio::test]
async fn project_context_sync_is_encrypted_separate_from_transcript_paging_and_group_deleted() {
    let relay = Relay::start(0).await;
    let dek = [9; 32];
    let plaintext = serde_json::json!({ "chat_id": "project-a", "entry": { "text": "confidential project brief" } });
    let ciphertext = lorca::crypto::encrypt_json(&dek, "project_context", &plaintext).unwrap();
    let context = OutboxItem {
        id: "project-revision".into(), kind: "project_context".into(), recipient: None,
        ciphertext: ciphertext.clone(), slot: Some(lorca::app::Slot::latest("project-revision")), group: Some("project-a".into()),
    };
    relay.client.put_blob(&relay.url, &relay.token, context.clone()).await.unwrap();
    relay.client.put_blob(&relay.url, &relay.token, context).await.unwrap();
    for i in 0..410 {
        relay.client.put_blob(&relay.url, &relay.token, OutboxItem {
            id: format!("message-{i}"), kind: "chat".into(), recipient: None,
            ciphertext: vec![i as u8], slot: Some(lorca::app::Slot::latest(format!("message-{i}"))), group: Some("project-a".into()),
        }).await.unwrap();
    }
    let (blobs, _) = relay.client.list_blobs(&relay.url, &relay.token, 0, "project_context").await.unwrap();
    assert_eq!(blobs.len(), 1, "context is loaded independently of the newest transcript page");
    let downloaded = lorca::keys::unb64(&blobs[0].ciphertext).unwrap();
    assert_eq!(downloaded, ciphertext);
    assert_eq!(lorca::crypto::decrypt_json::<serde_json::Value>(&dek, "project_context", &downloaded).unwrap(), plaintext);
    let (page, has_more) = relay.client.group_page(&relay.url, &relay.token, "project-a", None, 400).await.unwrap();
    assert_eq!(page.len(), 400);
    assert!(has_more);
    assert!(page.iter().flat_map(|slot| &slot.blobs).all(|blob| blob.kind == "chat"));
    relay.client.delete_group(&relay.url, &relay.token, "project-a").await.unwrap();
    assert!(relay.client.list_blobs(&relay.url, &relay.token, 0, "project_context").await.unwrap().0.is_empty());
    let late = OutboxItem { id: "late-context".into(), kind: "project_context".into(), recipient: None, ciphertext, slot: None, group: Some("project-a".into()) };
    assert_eq!(relay.client.put_blob(&relay.url, &relay.token, late).await.unwrap_err().status, Some(409));
}
