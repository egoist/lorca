//! `tinybot`: keys, the local websocket for the app, the agent loop, and relay sync.

use std::path::PathBuf;

use clap::{Parser, Subcommand};

use tinybot::app::App;
use tinybot::config::Config;
use tinybot::{identity, keys, pairing, routines, runtime, sync, ws};

#[derive(Parser, Debug)]
#[command(name = "tinybot", version, about = "Tinybot CLI: identity, local API, agent loop, relay sync")]
struct Cli {
    /// Data directory (default ~/.tinybot).
    #[arg(long, env = "TINYBOT_HOME", global = true)]
    home: Option<PathBuf>,

    /// Local websocket port for the app.
    #[arg(long, env = "TINYBOT_PORT", global = true)]
    port: Option<u16>,

    #[command(subcommand)]
    command: Option<Command>,
}

#[derive(Subcommand, Debug)]
enum Command {
    /// Run the local API the app connects to (default).
    Serve {
        /// Exit when this process is gone. The app passes its own pid so a killed app never
        /// leaves a stale CLI holding the port.
        #[arg(long)]
        parent_pid: Option<u32>,
    },
    /// Manage this identity.
    Identity {
        #[command(subcommand)]
        command: IdentityCommand,
    },
    /// Pair another Device.
    Pair {
        /// A pairing string from the Device that holds the identity. Omit to create one here.
        pairing_string: Option<String>,
        /// Name for this Device when joining.
        #[arg(long)]
        name: Option<String>,
    },
    /// Show identity, Devices, bots, and relay state.
    Status,
    /// Check the local setup.
    Doctor,
}

#[derive(Subcommand, Debug)]
enum IdentityCommand {
    /// Create a new identity on this Device and print the backup phrase.
    New {
        #[arg(long)]
        name: Option<String>,
    },
    /// Restore from a backup phrase. Needs a relay.
    Restore {
        phrase: Vec<String>,
        #[arg(long)]
        name: Option<String>,
    },
    /// Print the identity id and public keys.
    Show,
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(tracing_subscriber::EnvFilter::try_from_default_env().unwrap_or_else(|_| "tinybot=info,tinybot_agent=info".into()))
        .with_target(false)
        .init();

    let cli = Cli::parse();
    let config = Config::load(cli.home, cli.port);
    let app = App::load(config)?;

    match cli.command.unwrap_or(Command::Serve { parent_pid: None }) {
        Command::Serve { parent_pid } => {
            runtime::prime_names(&app);
            // Installed marketplace plugins follow the index this build ships.
            tinybot::plugins::refresh_installed(&app, &tinybot::plugins::bundled());
            if let Some(pid) = parent_pid {
                tokio::spawn(watch_parent(pid));
            }
            tokio::spawn(sync::run(app.clone()));
            tokio::spawn(routines::run(app.clone()));
            ws::serve(app).await
        }
        Command::Identity { command } => match command {
            IdentityCommand::New { name } => {
                let phrase = identity::create(&app, name)?;
                println!("Identity created on this Device.\n");
                println!("Backup phrase (write it down; it is the identity):\n");
                println!("  {}\n", phrase.join(" "));
                if app.relay_url().is_none() {
                    println!("No relay configured. Set TINYBOT_RELAY_URL to sync with other Devices.");
                } else {
                    flush_outbox_once(&app).await;
                }
                Ok(())
            }
            IdentityCommand::Restore { phrase, name } => {
                identity::restore(&app, &phrase.join(" "), name).await?;
                println!("Identity restored. Run `tinybot serve` to sync.");
                Ok(())
            }
            IdentityCommand::Show => {
                match app.machine_file() {
                    Some(machine) => {
                        println!("identity id:   {}", keys::identity_id(&machine.identity_pubkey));
                        println!("identity key:  {}", machine.identity_pubkey);
                        println!("machine key:   {}", machine.machine()?.pubkey());
                        println!("device name:   {} ({})", machine.name, machine.os);
                        println!("holds master:  {}", app.is_identity_device());
                        println!("registered:    {}", machine.registered);
                    }
                    None => println!("No identity on this Device. Run `tinybot identity new`."),
                }
                Ok(())
            }
        },
        Command::Pair { pairing_string, name } => match pairing_string {
            Some(text) => {
                let device = pairing::accept(app.clone(), &text, name).await?;
                println!("Paired as {}. Run `tinybot serve` to sync.", device["name"].as_str().unwrap_or("this Device"));
                Ok(())
            }
            None => {
                let (nonce, pairing_string) = pairing::start(app.clone()).await?;
                println!("On the other Device run:\n\n  tinybot pair '{pairing_string}'\n\nWaiting…");
                loop {
                    tokio::time::sleep(std::time::Duration::from_secs(1)).await;
                    let status = pairing::status(&app, &nonce);
                    match status["state"].as_str() {
                        Some("completed") => {
                            println!("Paired {}.", status["device"]["name"].as_str().unwrap_or("a Device"));
                            flush_outbox_once(&app).await;
                            return Ok(());
                        }
                        Some("failed") => anyhow::bail!("{}", status["error"].as_str().unwrap_or("pairing failed")),
                        _ => {}
                    }
                }
            }
        },
        Command::Status => {
            let snapshot = app.snapshot();
            println!("{}", serde_json::to_string_pretty(&snapshot)?);
            Ok(())
        }
        Command::Doctor => {
            doctor(&app).await;
            Ok(())
        }
    }
}

/// Exits once the parent process is gone.
async fn watch_parent(pid: u32) {
    loop {
        tokio::time::sleep(std::time::Duration::from_secs(1)).await;
        let alive = unsafe { libc::kill(pid as i32, 0) } == 0 || std::io::Error::last_os_error().raw_os_error() == Some(libc::EPERM);
        if !alive {
            tracing::info!(pid, "parent exited; stopping");
            std::process::exit(0);
        }
    }
}

/// One sync pass so CLI-only flows upload what they queued.
async fn flush_outbox_once(app: &std::sync::Arc<App>) {
    let sync = sync::run(app.clone());
    let _ = tokio::time::timeout(std::time::Duration::from_secs(8), sync).await;
}

async fn doctor(app: &std::sync::Arc<App>) {
    let ok = |label: &str, good: bool, detail: String| println!("{} {label}: {detail}", if good { "✔" } else { "✘" });
    ok("home", app.config.home.is_dir(), app.config.home.display().to_string());
    ok("identity", app.has_identity(), if app.has_identity() { "present".into() } else { "run `tinybot identity new`".into() });
    let port_free = std::net::TcpListener::bind(("127.0.0.1", app.config.port)).is_ok();
    ok("port", port_free, if port_free { format!("{} free", app.config.port) } else { format!("{} busy (tinybot serve running?)", app.config.port) });
    match app.relay_url() {
        Some(url) => {
            let reachable = app.relay.health(&url).await.is_ok();
            ok("relay", reachable, if reachable { url } else { format!("{url} unreachable") });
        }
        None => ok("relay", false, "not configured (TINYBOT_RELAY_URL); single-Device mode".into()),
    }
    let credentials = app.credentials.lock().unwrap().connected_kinds();
    ok("providers", !credentials.is_empty(), if credentials.is_empty() { "none connected".into() } else { credentials.join(", ") });
}
