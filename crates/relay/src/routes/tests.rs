use std::sync::Arc;

use super::*;
use lorca::app::OutboxItem;
use lorca::relay::RelayClient;

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
            pusher: Arc::new(crate::push::Pusher::new(None, None)),
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
