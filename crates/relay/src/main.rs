//! Tinybot relay: a zero-knowledge store-and-forward service.
//!
//! It stores identity and machine public keys, opaque ciphertext blobs with per-identity
//! sequence numbers, and a short-lived pairing mailbox. Auth is a signature challenge; every
//! mutating identity-level request is signed by the identity key.

mod auth;
mod db;
mod routes;

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
}

#[derive(Clone)]
pub struct AppState {
    pub db: Arc<db::Db>,
    pub secret: Arc<[u8; 32]>,
    /// Woken per identity on every blob write so its long-polls return early.
    pub wakers: Arc<db::Wakers>,
    /// Throttles `last_seen` writes.
    pub presence: Arc<db::Presence>,
    pub quota_bytes: u64,
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(tracing_subscriber::EnvFilter::try_from_default_env().unwrap_or_else(|_| "info".into()))
        .init();

    let args = Args::parse();
    let db = Arc::new(db::Db::open(&args.db)?);

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
        quota_bytes: args.quota_bytes,
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
    tracing::info!(bind = %args.bind, db = %args.db, quota_bytes = args.quota_bytes, "tinybot-relay listening");
    axum::serve(listener, app).await?;
    Ok(())
}
