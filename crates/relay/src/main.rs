//! Tinybot relay: a zero-knowledge store-and-forward service.
//!
//! It stores identity and machine public keys, opaque ciphertext blobs with per-identity
//! sequence numbers, and a short-lived pairing mailbox. Auth is a signature challenge; every
//! mutating identity-level request is signed by the identity key.

mod auth;
mod db;
mod routes;

use std::net::SocketAddr;
use std::sync::{Arc, Mutex};

use clap::Parser;
use rand::RngCore;
use tokio::sync::Notify;

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
}

#[derive(Clone)]
pub struct AppState {
    pub db: Arc<Mutex<rusqlite::Connection>>,
    pub secret: Arc<[u8; 32]>,
    /// Woken on every blob write so long-polls return early.
    pub notify: Arc<Notify>,
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(tracing_subscriber::EnvFilter::try_from_default_env().unwrap_or_else(|_| "info".into()))
        .init();

    let args = Args::parse();
    let connection = db::open(&args.db)?;

    let mut secret = [0u8; 32];
    match args.secret {
        Some(text) => {
            let digest = <sha2::Sha256 as sha2::Digest>::digest(text.as_bytes());
            secret.copy_from_slice(&digest);
        }
        None => rand::thread_rng().fill_bytes(&mut secret),
    }

    let state = AppState { db: Arc::new(Mutex::new(connection)), secret: Arc::new(secret), notify: Arc::new(Notify::new()) };

    let app = routes::router(state);
    let listener = tokio::net::TcpListener::bind(args.bind).await?;
    tracing::info!(bind = %args.bind, db = %args.db, "tinybot-relay listening");
    axum::serve(listener, app).await?;
    Ok(())
}
