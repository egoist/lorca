//! Tinybot relay: a zero-knowledge store-and-forward service.
//!
//! It stores identity and machine public keys, opaque ciphertext blobs with per-identity
//! sequence numbers, and a short-lived pairing mailbox. Auth is a signature challenge; every
//! mutating identity-level request is signed by the identity key.

mod auth;
mod db;
mod limit;
mod push;
mod routes;
mod store;

use std::net::SocketAddr;
use std::sync::Arc;

use clap::Parser;
use rand::RngCore;

#[derive(Parser, Debug)]
#[command(name = "tinybot-relay", about = "Tinybot relay server")]
struct Args {
    /// Address to listen on.
    #[arg(long, env = "TINYBOT_RELAY_BIND", default_value = "127.0.0.1:8787")]
    bind: SocketAddr,

    /// SQLite database path.
    #[arg(long, env = "TINYBOT_RELAY_DB", default_value = "tinybot-relay.db")]
    db: String,

    /// Secret used to sign bearer tokens. Random per boot when unset, which logs every
    /// client out on restart.
    #[arg(long, env = "TINYBOT_RELAY_SECRET")]
    secret: Option<String>,

    /// Stored ciphertext allowed per identity, in bytes. 0 means no limit.
    #[arg(long, env = "TINYBOT_RELAY_QUOTA_BYTES", default_value_t = 0)]
    quota_bytes: u64,

    /// Requests per minute one IP may make to the routes that need no token (registration,
    /// auth, pairing mailbox), with a burst of the same size. 0 disables the limit.
    #[arg(long, env = "TINYBOT_RELAY_IP_PER_MINUTE", default_value_t = 60)]
    ip_per_minute: u32,

    /// Requests per second one identity may make with a bearer token, across all its
    /// machines, with a burst of ten times that. 0 disables the limit.
    #[arg(long, env = "TINYBOT_RELAY_IDENTITY_PER_SECOND", default_value_t = 50)]
    identity_per_second: u32,

    /// Take the client IP from the last `X-Forwarded-For` hop. Set it only behind a proxy
    /// that overwrites that header.
    #[arg(long, env = "TINYBOT_RELAY_TRUST_PROXY", default_value_t = false)]
    trust_proxy: bool,

    /// Directory for `file` ciphertext. Defaults to the database path with a `.files`
    /// extension (`tinybot-relay.files`).
    #[arg(long, env = "TINYBOT_RELAY_FILES_DIR", conflicts_with = "s3_bucket")]
    files_dir: Option<std::path::PathBuf>,

    /// Keep `file` ciphertext in this S3-compatible bucket (AWS, R2, MinIO) instead of a
    /// directory. Needs --s3-endpoint and the access keys.
    #[arg(long, env = "TINYBOT_RELAY_S3_BUCKET", requires = "s3_endpoint")]
    s3_bucket: Option<String>,

    /// `https://<account>.r2.cloudflarestorage.com`, `https://s3.us-east-1.amazonaws.com`, …
    #[arg(long, env = "TINYBOT_RELAY_S3_ENDPOINT")]
    s3_endpoint: Option<String>,

    /// SigV4 region. R2 takes `auto`.
    #[arg(long, env = "TINYBOT_RELAY_S3_REGION", default_value = "auto")]
    s3_region: String,

    /// Key prefix inside the bucket.
    #[arg(long, env = "TINYBOT_RELAY_S3_PREFIX", default_value = "")]
    s3_prefix: String,

    /// Falls back to AWS_ACCESS_KEY_ID.
    #[arg(long, env = "TINYBOT_RELAY_S3_ACCESS_KEY", hide_env_values = true)]
    s3_access_key: Option<String>,

    /// Falls back to AWS_SECRET_ACCESS_KEY.
    #[arg(long, env = "TINYBOT_RELAY_S3_SECRET_KEY", hide_env_values = true)]
    s3_secret_key: Option<String>,

    /// Apple's `.p8` push key, for pushes to iPhones. Needs --apns-key-id and --apns-team-id.
    #[arg(long, env = "TINYBOT_RELAY_APNS_KEY", requires_all = ["apns_key_id", "apns_team_id"])]
    apns_key: Option<std::path::PathBuf>,

    #[arg(long, env = "TINYBOT_RELAY_APNS_KEY_ID")]
    apns_key_id: Option<String>,

    #[arg(long, env = "TINYBOT_RELAY_APNS_TEAM_ID")]
    apns_team_id: Option<String>,

    /// The phone app's bundle id.
    #[arg(long, env = "TINYBOT_RELAY_APNS_TOPIC", default_value = "dev.tinybot.app")]
    apns_topic: String,

    /// A Firebase service account JSON file, for pushes to Android phones.
    #[arg(long, env = "TINYBOT_RELAY_FCM_SERVICE_ACCOUNT")]
    fcm_service_account: Option<std::path::PathBuf>,
}

/// APNs and FCM, each when its key is given. `TINYBOT_RELAY_APNS_URL` and
/// `TINYBOT_RELAY_FCM_URL` point them at a test server.
fn pusher(args: &Args) -> anyhow::Result<push::Pusher> {
    let apns = match &args.apns_key {
        Some(path) => Some(push::Apns::new(
            &std::fs::read_to_string(path).map_err(|e| anyhow::anyhow!("reading {}: {e}", path.display()))?,
            args.apns_key_id.clone().expect("clap requires the key id"),
            args.apns_team_id.clone().expect("clap requires the team id"),
            args.apns_topic.clone(),
            std::env::var("TINYBOT_RELAY_APNS_URL").ok(),
        )?),
        None => None,
    };
    let fcm = match &args.fcm_service_account {
        Some(path) => Some(push::Fcm::new(
            &std::fs::read_to_string(path).map_err(|e| anyhow::anyhow!("reading {}: {e}", path.display()))?,
            std::env::var("TINYBOT_RELAY_FCM_URL").ok(),
        )?),
        None => None,
    };
    Ok(push::Pusher::new(apns, fcm))
}

fn file_store(args: &Args) -> anyhow::Result<store::FileStore> {
    let Some(bucket) = &args.s3_bucket else {
        let dir = args.files_dir.clone().unwrap_or_else(|| std::path::PathBuf::from(&args.db).with_extension("files"));
        std::fs::create_dir_all(&dir)?;
        return Ok(store::FileStore::Local { dir });
    };
    let access_key = args
        .s3_access_key
        .clone()
        .or_else(|| std::env::var("AWS_ACCESS_KEY_ID").ok())
        .ok_or_else(|| anyhow::anyhow!("--s3-bucket needs TINYBOT_RELAY_S3_ACCESS_KEY or AWS_ACCESS_KEY_ID"))?;
    let secret_key = args
        .s3_secret_key
        .clone()
        .or_else(|| std::env::var("AWS_SECRET_ACCESS_KEY").ok())
        .ok_or_else(|| anyhow::anyhow!("--s3-bucket needs TINYBOT_RELAY_S3_SECRET_KEY or AWS_SECRET_ACCESS_KEY"))?;
    Ok(store::FileStore::S3(store::S3::new(
        args.s3_endpoint.clone().expect("clap requires the endpoint"),
        bucket.clone(),
        args.s3_region.clone(),
        args.s3_prefix.clone(),
        access_key,
        secret_key,
    )))
}

#[derive(Clone)]
pub struct AppState {
    pub db: Arc<db::Db>,
    pub secret: Arc<[u8; 32]>,
    /// Woken per identity on every blob write so its long-polls return early.
    pub wakers: Arc<db::Wakers>,
    /// Throttles `last_seen` writes.
    pub presence: Arc<db::Presence>,
    /// Keys of unpaired machines; their tokens are refused.
    pub revoked: Arc<db::Revoked>,
    pub quota_bytes: u64,
    pub ip_limiter: Arc<limit::RateLimiter>,
    pub identity_limiter: Arc<limit::RateLimiter>,
    pub trust_proxy: bool,
    /// Where `file` ciphertext goes. The database holds only the row.
    pub file_store: Arc<store::FileStore>,
    /// APNs and FCM, for the phones of an identity.
    pub pusher: Arc<push::Pusher>,
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(tracing_subscriber::EnvFilter::try_from_default_env().unwrap_or_else(|_| "info".into()))
        .init();

    let args = Args::parse();
    let db = Arc::new(db::Db::open(&args.db)?);
    let file_store = Arc::new(file_store(&args)?);
    let pusher = Arc::new(pusher(&args)?);
    let revoked = Arc::new(db::Revoked::load(db.read(|db| Ok(db::revoked_machines(db)?)).await.map_err(|e| anyhow::anyhow!("{e:?}"))?));

    let mut secret = [0u8; 32];
    match args.secret {
        Some(text) => {
            let digest = <sha2::Sha256 as sha2::Digest>::digest(text.as_bytes());
            secret.copy_from_slice(&digest);
        }
        None => rand::thread_rng().fill_bytes(&mut secret),
    }

    let state = AppState {
        db: db.clone(),
        secret: Arc::new(secret),
        wakers: Arc::new(db::Wakers::default()),
        presence: Arc::new(db::Presence::default()),
        revoked,
        quota_bytes: args.quota_bytes,
        ip_limiter: Arc::new(limit::RateLimiter::new(args.ip_per_minute as f64 / 60.0, args.ip_per_minute)),
        identity_limiter: Arc::new(limit::RateLimiter::new(
            args.identity_per_second as f64,
            args.identity_per_second.saturating_mul(10),
        )),
        trust_proxy: args.trust_proxy,
        file_store: file_store.clone(),
        pusher: pusher.clone(),
    };

    tokio::spawn(async move {
        let mut tick = tokio::time::interval(std::time::Duration::from_secs(60));
        loop {
            tick.tick().await;
            if let Err(error) = db.write(|db| Ok(db::expire(db)?)).await {
                tracing::warn!(?error, "expiring challenges and pairings");
            }
        }
    });

    let app = routes::router(state);
    let listener = tokio::net::TcpListener::bind(args.bind).await?;
    tracing::info!(
        bind = %args.bind,
        db = %args.db,
        quota_bytes = args.quota_bytes,
        files = %file_store.describe(),
        push = %pusher.describe(),
        "tinybot-relay listening"
    );
    axum::serve(listener, app.into_make_service_with_connect_info::<SocketAddr>()).await?;
    Ok(())
}
