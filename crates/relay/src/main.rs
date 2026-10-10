//! Lorca relay: a zero-knowledge store-and-forward service.
//!
//! It stores identity and machine public keys, opaque ciphertext blobs with per-identity
//! sequence numbers, and a short-lived pairing mailbox. Auth is a signature challenge; every
//! mutating identity-level request is signed by the identity key.

mod auth;
mod db;
mod hub;
mod limit;
mod mail;
mod metrics;
mod push;
mod routes;
mod store;
mod sweep;

use std::net::SocketAddr;
use std::sync::Arc;

use rand::RngCore;

#[derive(usage::Cli, Debug)]
#[usage(bin = "lorca-relay", about = "Lorca relay server", unknown_flags = "error")]
struct Args {
    /// Address to listen on.
    #[usage(long, env = "LORCA_RELAY_BIND", default = "127.0.0.1:8787")]
    bind: SocketAddr,

    /// The database: a SQLite path, or a `postgres://` URL. SQLite belongs to one relay
    /// process. Postgres is shared by as many as run, so a deploy can overlap the old process
    /// with the new one; give them the same --secret and an S3 bucket for files.
    #[usage(long, env = "LORCA_RELAY_DB", default = "lorca-relay.db")]
    db: String,

    /// Secret used to sign bearer tokens. Random per boot when unset, which logs every
    /// client out on restart.
    #[usage(long, env = "LORCA_RELAY_SECRET")]
    secret: Option<String>,

    /// Stored ciphertext allowed per identity, in bytes; 5 GiB by default. 0 means no limit.
    #[usage(long, env = "LORCA_RELAY_QUOTA_BYTES", default = "5368709120")]
    quota_bytes: u64,

    /// Requests per minute one IP may make to the routes that need no token (registration,
    /// auth, pairing mailbox), with a burst of the same size. 0 disables the limit.
    #[usage(long, env = "LORCA_RELAY_IP_PER_MINUTE", default = "60")]
    ip_per_minute: u32,

    /// Requests per second one identity may make with a bearer token, across all its
    /// machines, with a burst of ten times that. 0 disables the limit.
    #[usage(long, env = "LORCA_RELAY_IDENTITY_PER_SECOND", default = "50")]
    identity_per_second: u32,

    /// Refuse clients that speak an older protocol than this with `426`, which their apps show
    /// as "update required". A client says which it speaks in `Lorca-Protocol`; one that says
    /// nothing speaks 0. Raise it when the relay stops serving what old clients rely on.
    #[usage(long, env = "LORCA_RELAY_MIN_PROTOCOL", default = "0")]
    min_protocol: u32,

    /// Delete an identity after this many days with no sign of life: no machine seen, no blob
    /// written, no socket open. Its Devices keep what they hold and register again if they
    /// come back. 0 keeps every identity.
    #[usage(long, env = "LORCA_RELAY_INACTIVE_DAYS", default = "365")]
    inactive_days: u32,

    /// Serve `GET /metrics` (Prometheus text: counts, bytes, sockets, pushes, sweeps) to
    /// whoever sends this as a bearer token. Unset, the route does not exist.
    #[usage(long, env = "LORCA_RELAY_METRICS_TOKEN", hide_env_values = true)]
    metrics_token: Option<String>,

    /// Uploads over 1 MiB handled at once. Each holds its ciphertext body in memory while
    /// it is sent to storage, so this bounds what attachments cost;
    /// one that waits 30 s for a place gets `503`. 0 disables the limit.
    #[usage(long, env = "LORCA_RELAY_CONCURRENT_UPLOADS", default = "3")]
    concurrent_uploads: usize,

    /// Take the client IP from the last `X-Forwarded-For` hop. Set it only behind a proxy
    /// that overwrites that header.
    #[usage(long, env = "LORCA_RELAY_TRUST_PROXY", default = "false")]
    trust_proxy: bool,

    /// Directory for `file` ciphertext. Defaults to the database path with a `.files`
    /// extension (`lorca-relay.files`).
    #[usage(long, env = "LORCA_RELAY_FILES_DIR", conflicts("--s3-bucket"))]
    files_dir: Option<std::path::PathBuf>,

    /// Keep `file` ciphertext in this S3-compatible bucket (AWS, R2, MinIO) instead of a
    /// directory. Needs --s3-endpoint and the access keys.
    #[usage(long, env = "LORCA_RELAY_S3_BUCKET", requires("--s3-endpoint"))]
    s3_bucket: Option<String>,

    /// `https://<account>.r2.cloudflarestorage.com`, `https://s3.us-east-1.amazonaws.com`, …
    #[usage(long, env = "LORCA_RELAY_S3_ENDPOINT")]
    s3_endpoint: Option<String>,

    /// SigV4 region. R2 takes `auto`.
    #[usage(long, env = "LORCA_RELAY_S3_REGION", default = "auto")]
    s3_region: String,

    /// Key prefix inside the bucket.
    #[usage(long, env = "LORCA_RELAY_S3_PREFIX", default = "")]
    s3_prefix: String,

    /// Falls back to AWS_ACCESS_KEY_ID.
    #[usage(long, env = "LORCA_RELAY_S3_ACCESS_KEY", hide_env_values = true)]
    s3_access_key: Option<String>,

    /// Falls back to AWS_SECRET_ACCESS_KEY.
    #[usage(long, env = "LORCA_RELAY_S3_SECRET_KEY", hide_env_values = true)]
    s3_secret_key: Option<String>,

    /// Apple's `.p8` push key, for pushes to iPhones: the key's text (`-----BEGIN PRIVATE
    /// KEY-----…`, with real or `\n` line breaks) or the path of the file. Needs --apns-key-id
    /// and --apns-team-id.
    #[usage(long, env = "LORCA_RELAY_APNS_KEY", hide_env_values = true, requires("--apns-key-id", "--apns-team-id"))]
    apns_key: Option<String>,

    #[usage(long, env = "LORCA_RELAY_APNS_KEY_ID")]
    apns_key_id: Option<String>,

    #[usage(long, env = "LORCA_RELAY_APNS_TEAM_ID")]
    apns_team_id: Option<String>,

    /// The phone app's bundle id.
    #[usage(long, env = "LORCA_RELAY_APNS_TOPIC", default = "app.lorca")]
    apns_topic: String,

    /// A Firebase service account, for pushes to Android phones: the JSON itself or the path
    /// of the file.
    #[usage(long, env = "LORCA_RELAY_FCM_SERVICE_ACCOUNT", hide_env_values = true)]
    fcm_service_account: Option<String>,

    /// The mail domain, `bots.lorca.app`: each account may take one address there, which its
    /// bots share. Needs --mail-token, the mail Worker's bearer. Unset, the relay has no email.
    #[usage(long, env = "LORCA_RELAY_MAIL_DOMAIN", requires("--mail-token"))]
    mail_domain: Option<String>,

    /// The token the mail Worker sends when it asks where mail goes and hands over sealed copies.
    #[usage(long, env = "LORCA_RELAY_MAIL_TOKEN", hide_env_values = true)]
    mail_token: Option<String>,

    /// The Cloudflare account whose Email Sending sends bots' mail. Needs --email-api-token.
    /// Unset, mail comes in and bots cannot send.
    #[usage(long, env = "LORCA_RELAY_EMAIL_ACCOUNT_ID", requires("--email-api-token", "--mail-domain"))]
    email_account_id: Option<String>,

    /// A Cloudflare API token that may send email for that account.
    #[usage(long, env = "LORCA_RELAY_EMAIL_API_TOKEN", hide_env_values = true)]
    email_api_token: Option<String>,

    /// Messages an account may send a day.
    #[usage(long, env = "LORCA_RELAY_MAIL_DAILY_SENDS", default = "100")]
    mail_daily_sends: i64,

    /// Messages an account may send a day in its first week.
    #[usage(long, env = "LORCA_RELAY_MAIL_NEW_DAILY_SENDS", default = "20")]
    mail_new_daily_sends: i64,

    /// Recipients one message may have, To and Cc together. Email Sending allows 50.
    #[usage(long, env = "LORCA_RELAY_MAIL_MAX_RECIPIENTS", default = "10")]
    mail_max_recipients: usize,

    /// Bounces in a day, permanent or to suppressed recipients, that suspend an address for a week.
    #[usage(long, env = "LORCA_RELAY_MAIL_BOUNCE_LIMIT", default = "5")]
    mail_bounce_limit: i64,
}

/// Email, when the operator set up a domain. `LORCA_RELAY_EMAIL_API_URL` points Email Sending
/// at a test server.
fn mail(args: &Args) -> anyhow::Result<Option<mail::MailConfig>> {
    let (Some(domain), Some(token)) = (&args.mail_domain, &args.mail_token) else { return Ok(None) };
    let sending = match (&args.email_account_id, &args.email_api_token) {
        (Some(account), Some(api_token)) => Some(mail::Sending::new(account.clone(), api_token.clone(), std::env::var("LORCA_RELAY_EMAIL_API_URL").ok())?),
        _ => None,
    };
    Ok(Some(mail::MailConfig {
        domain: domain.trim().to_ascii_lowercase(),
        worker_token: token.clone(),
        sending,
        daily_sends: args.mail_daily_sends,
        new_daily_sends: args.mail_new_daily_sends,
        max_recipients: args.mail_max_recipients.clamp(1, 50),
        bounce_limit: args.mail_bounce_limit.max(1),
    }))
}

/// A key given as its text or as the path of a file. The text form lets a deploy with no
/// volume carry the key in a variable.
fn key_text(value: &str) -> anyhow::Result<String> {
    let text = value.trim();
    if text.starts_with("-----BEGIN") {
        return Ok(text.replace("\\n", "\n"));
    }
    if text.starts_with('{') {
        return Ok(text.to_string());
    }
    std::fs::read_to_string(text).map_err(|e| anyhow::anyhow!("reading {text}: {e}"))
}

/// APNs and FCM, each when its key is given. `LORCA_RELAY_APNS_URL` and
/// `LORCA_RELAY_FCM_URL` point them at a test server.
fn pusher(args: &Args) -> anyhow::Result<push::Pusher> {
    let apns = match &args.apns_key {
        Some(key) => Some(push::Apns::new(
            &key_text(key)?,
            args.apns_key_id.clone().expect("the parser requires the key id"),
            args.apns_team_id.clone().expect("the parser requires the team id"),
            args.apns_topic.clone(),
            std::env::var("LORCA_RELAY_APNS_URL").ok(),
        )?),
        None => None,
    };
    let fcm = match &args.fcm_service_account {
        Some(account) => Some(push::Fcm::new(
            &key_text(account)?,
            std::env::var("LORCA_RELAY_FCM_URL").ok(),
        )?),
        None => None,
    };
    Ok(push::Pusher::new(apns, fcm))
}

fn file_store(args: &Args) -> anyhow::Result<store::FileStore> {
    let Some(bucket) = &args.s3_bucket else {
        let beside = if args.db.contains("://") { "lorca-relay.db" } else { args.db.as_str() };
        let dir = args.files_dir.clone().unwrap_or_else(|| std::path::PathBuf::from(beside).with_extension("files"));
        std::fs::create_dir_all(&dir)?;
        return Ok(store::FileStore::Local { dir });
    };
    let access_key = args
        .s3_access_key
        .clone()
        .or_else(|| std::env::var("AWS_ACCESS_KEY_ID").ok())
        .ok_or_else(|| anyhow::anyhow!("--s3-bucket needs LORCA_RELAY_S3_ACCESS_KEY or AWS_ACCESS_KEY_ID"))?;
    let secret_key = args
        .s3_secret_key
        .clone()
        .or_else(|| std::env::var("AWS_SECRET_ACCESS_KEY").ok())
        .ok_or_else(|| anyhow::anyhow!("--s3-bucket needs LORCA_RELAY_S3_SECRET_KEY or AWS_SECRET_ACCESS_KEY"))?;
    Ok(store::FileStore::S3(store::S3::new(
        args.s3_endpoint.clone().expect("the parser requires the endpoint"),
        bucket.clone(),
        args.s3_region.clone(),
        args.s3_prefix.clone(),
        access_key,
        secret_key,
    )))
}

#[derive(Clone)]
pub struct AppState {
    pub db: Arc<dyn db::Store>,
    pub secret: Arc<[u8; 32]>,
    /// This process's sync sockets and the keys of unpaired machines, whose tokens are refused.
    pub local: Arc<db::Local>,
    pub quota_bytes: u64,
    pub ip_limiter: Arc<limit::RateLimiter>,
    pub identity_limiter: Arc<limit::RateLimiter>,
    pub trust_proxy: bool,
    /// Places for uploads over `limit::LARGE_UPLOAD`; `None` when they are not limited.
    pub uploads: Option<Arc<tokio::sync::Semaphore>>,
    pub min_protocol: u32,
    pub metrics_token: Option<Arc<str>>,
    pub stats: Arc<metrics::StatsCache>,
    /// This process, as a label on what only it counted.
    pub instance: Arc<str>,
    /// Cancelled when the process is told to stop; the sync sockets end on it.
    pub stopping: tokio_util::sync::CancellationToken,
    /// Where `file` ciphertext goes. The database holds only the row.
    pub file_store: Arc<store::FileStore>,
    /// APNs and FCM, for the phones of an identity.
    pub pusher: Arc<push::Pusher>,
    /// The pushes on their way to APNs and FCM, which a stopping relay delivers before it exits.
    pub pushes: tokio_util::task::TaskTracker,
    /// Email, when the operator set up a domain for it.
    pub mail: Option<Arc<mail::MailConfig>>,
}

/// How long a stopping relay waits for APNs and FCM to take the pushes it already answered
/// for. Each takes one in well under a second.
const PUSH_GRACE: std::time::Duration = std::time::Duration::from_secs(5);

/// Stopping, once the server has answered its last request: the pushes it said it queued go to
/// APNs and FCM, for up to `grace`, while the database is still open to forget a token Apple
/// or Google calls dead. Then the database closes.
async fn stop(state: &AppState, grace: std::time::Duration) {
    state.pushes.close();
    if tokio::time::timeout(grace, state.pushes.wait()).await.is_err() {
        tracing::warn!(pushes = state.pushes.len(), "stopping before every push was delivered");
    }
    state.db.close().await;
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(tracing_subscriber::EnvFilter::try_from_default_env().unwrap_or_else(|_| "info".into()))
        .init();

    let args = Args::parse();
    let local = Arc::new(db::Local::default());
    let db = db::open(&args.db, local.clone()).await?;
    let file_store = Arc::new(file_store(&args)?);
    let pusher = Arc::new(pusher(&args)?);
    let mail = mail(&args)?.map(Arc::new);

    let mut secret = [0u8; 32];
    match &args.secret {
        Some(text) => {
            let digest = <sha2::Sha256 as sha2::Digest>::digest(text.as_bytes());
            secret.copy_from_slice(&digest);
        }
        None => rand::thread_rng().fill_bytes(&mut secret),
    }

    let state = AppState {
        db: db.clone(),
        secret: Arc::new(secret),
        local,
        quota_bytes: args.quota_bytes,
        ip_limiter: Arc::new(limit::RateLimiter::new(args.ip_per_minute as f64 / 60.0, args.ip_per_minute)),
        identity_limiter: Arc::new(limit::RateLimiter::new(
            args.identity_per_second as f64,
            args.identity_per_second.saturating_mul(10),
        )),
        trust_proxy: args.trust_proxy,
        min_protocol: args.min_protocol,
        metrics_token: args.metrics_token.as_deref().filter(|token| !token.is_empty()).map(Arc::from),
        stats: Arc::default(),
        instance: uuid::Uuid::new_v4().simple().to_string()[..8].into(),
        uploads: (args.concurrent_uploads > 0).then(|| Arc::new(tokio::sync::Semaphore::new(args.concurrent_uploads))),
        stopping: tokio_util::sync::CancellationToken::new(),
        file_store: file_store.clone(),
        pusher: pusher.clone(),
        pushes: tokio_util::task::TaskTracker::new(),
        mail: mail.clone(),
    };

    let ticking = db.clone();
    tokio::spawn(async move {
        let mut tick = tokio::time::interval(std::time::Duration::from_secs(60));
        loop {
            tick.tick().await;
            if let Err(error) = ticking.tick().await {
                tracing::warn!(?error, "housekeeping");
            }
        }
    });

    sweep::spawn(db.clone(), file_store.clone(), args.inactive_days);

    let stopping = state.stopping.clone();
    let app = routes::router(state.clone());
    let listener = tokio::net::TcpListener::bind(args.bind).await?;
    tracing::info!(
        bind = %args.bind,
        db = %db.describe(),
        quota_bytes = args.quota_bytes,
        files = %file_store.describe(),
        push = %pusher.describe(),
        mail = mail.as_ref().map(|mail| format!("{}{}", mail.domain, if mail.sending.is_some() { "" } else { " (receiving only)" })).unwrap_or_else(|| "off".into()),
        "lorca-relay listening"
    );
    let shared = args.db.contains("://");
    if shared && args.secret.is_none() {
        tracing::warn!("no --secret: a token from one relay process will not verify on another");
    }
    if shared && args.s3_bucket.is_none() {
        tracing::warn!("files are in a local directory: relay processes on other hosts will not find them");
    }
    // A deploy stops the process with SIGTERM. The sockets close, so the Devices connect to
    // the process that replaces this one, the pushes it took go out, and with Postgres this
    // one's presence rows go. Windows has no SIGTERM: a relay there stops on Ctrl-C.
    let serve = axum::serve(listener, app.into_make_service_with_connect_info::<SocketAddr>()).with_graceful_shutdown(async move {
        #[cfg(unix)]
        {
            let mut terminate = tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate()).expect("a SIGTERM handler");
            tokio::select! {
                _ = terminate.recv() => {}
                _ = tokio::signal::ctrl_c() => {}
            }
        }
        #[cfg(not(unix))]
        let _ = tokio::signal::ctrl_c().await;
        tracing::info!("stopping");
        stopping.cancel();
    });
    serve.await?;
    stop(&state, PUSH_GRACE).await;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::key_text;

    #[test]
    fn a_key_is_its_text_or_the_path_of_a_file() {
        assert_eq!(key_text("-----BEGIN PRIVATE KEY-----\\nabc\\n-----END PRIVATE KEY-----").unwrap(), "-----BEGIN PRIVATE KEY-----\nabc\n-----END PRIVATE KEY-----");
        assert_eq!(key_text(" {\"type\": \"service_account\"}\n").unwrap(), "{\"type\": \"service_account\"}");
        let path = std::env::temp_dir().join(format!("lorca-relay-key-{}", uuid::Uuid::new_v4()));
        std::fs::write(&path, "from a file").unwrap();
        assert_eq!(key_text(path.to_str().unwrap()).unwrap(), "from a file");
        assert!(key_text("/nonexistent/key.p8").is_err());
    }
}
