//! `lorca`: keys, the local websocket for the app, the agent loop, and relay sync.

use std::path::PathBuf;

use usage::Subcommands;

use lorca::app::App;
use lorca::config::Config;
use lorca::{identity, keys, pairing, routines, runtime, sync, ws};

#[derive(usage::Cli, Debug)]
#[usage(bin = "lorca", version, about = "Lorca CLI: identity, local API, agent loop, relay sync", unknown_flags = "error")]
struct Cli {
    /// Data directory (default ~/.lorca).
    #[usage(long, env = "LORCA_HOME", global)]
    home: Option<PathBuf>,

    /// Local websocket port for the app.
    #[usage(long, env = "LORCA_PORT", global)]
    port: Option<u16>,

    #[usage(subcommand)]
    command: Option<Command>,
}

#[derive(Subcommands, Debug)]
enum Command {
    /// Run the local API the app connects to (default).
    Serve {
        /// Exit when this process is gone. The app passes its own pid so a killed app never
        /// leaves a stale CLI holding the port.
        #[usage(long)]
        parent_pid: Option<u32>,
        /// Write a JSON readiness message to stdout once the local server is listening.
        #[usage(long)]
        ready_stdout: bool,
    },
    /// Manage this identity.
    Identity {
        #[usage(subcommand)]
        command: IdentityCommand,
    },
    /// Pair another Device.
    Pair {
        /// A pairing string from the Device that holds the identity. Omit to create one here.
        pairing_string: Option<String>,
        /// Name for this Device when joining.
        #[usage(long)]
        name: Option<String>,
    },
    /// Manage the account's provider credentials.
    Provider {
        #[usage(subcommand)]
        command: ProviderCommand,
    },
    /// Show identity, Devices, bots, and relay state.
    Status,
    /// Check the local setup.
    Doctor,
}

#[derive(Subcommands, Debug)]
enum IdentityCommand {
    /// Create a new identity on this Device and print the backup phrase.
    New {
        #[usage(long)]
        name: Option<String>,
    },
    /// Restore from a backup phrase. Needs a relay.
    Restore {
        phrase: Vec<String>,
        #[usage(long)]
        name: Option<String>,
    },
    /// Print the identity id and public keys.
    Show,
}

#[derive(Subcommands, Debug)]
enum ProviderCommand {
    /// Connect a provider: an API key for deepseek, anthropic, opencode, and opencode-go;
    /// a browser sign-in for chatgpt and grok.
    Set {
        /// deepseek, anthropic, opencode, opencode-go, chatgpt, or grok.
        kind: String,
        /// The API key. Omit to read it from stdin, which keeps it out of the shell history.
        api_key: Option<String>,
        /// The API root to call instead of the provider's own: a proxy or a compatible server.
        #[usage(long)]
        base_url: Option<String>,
    },
    /// Disconnect a provider on every Device.
    Remove { kind: String },
    /// List the providers and what is connected.
    List,
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(tracing_subscriber::EnvFilter::try_from_default_env().unwrap_or_else(|_| "lorca=info,lorca_agent=info".into()))
        .with_target(false)
        .with_writer(std::io::stderr)
        .init();

    let cli = Cli::parse();
    let config = Config::load(cli.home, cli.port);
    let app = App::load(config)?;

    match cli.command.unwrap_or(Command::Serve { parent_pid: None, ready_stdout: false }) {
        Command::Serve { parent_pid, ready_stdout } => {
            runtime::prime_names(&app);
            runtime::resume_sent_jobs(&app);
            // Installed marketplace plugins follow the index this build ships.
            lorca::plugins::refresh_installed(&app, &lorca::marketplace::bundled().plugins);
            if let Some(pid) = parent_pid {
                tokio::spawn(watch_parent(pid));
            }
            // Bots' commands and plugin servers start with it; read it while the rest starts.
            tokio::spawn(lorca_agent::login_shell::environment());
            tokio::spawn(sync::run(app.clone()));
            tokio::spawn(routines::run(app.clone()));
            ws::serve(app, ready_stdout).await
        }
        Command::Identity { command } => match command {
            IdentityCommand::New { name } => {
                let phrase = identity::create(&app, name)?;
                println!("Identity created on this Device.\n");
                println!("Backup phrase (write it down; it is the identity):\n");
                println!("  {}\n", phrase.join(" "));
                if app.relay_url().is_none() {
                    println!("No relay configured. Set LORCA_RELAY_URL to sync with other Devices.");
                } else {
                    flush_outbox_once(&app).await;
                }
                Ok(())
            }
            IdentityCommand::Restore { phrase, name } => {
                identity::restore(&app, &phrase.join(" "), name).await?;
                println!("Identity restored. Run `lorca serve` to sync.");
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
                    None => println!("No identity on this Device. Run `lorca identity new`."),
                }
                Ok(())
            }
        },
        Command::Pair { pairing_string, name } => match pairing_string {
            Some(text) => {
                let device = pairing::accept(app.clone(), &text, name).await?;
                println!("Paired as {}. Run `lorca serve` to sync.", device["name"].as_str().unwrap_or("this Device"));
                Ok(())
            }
            None => {
                let (nonce, pairing_string) = pairing::start(app.clone()).await?;
                println!("On the other Device run:\n\n  lorca pair '{pairing_string}'\n\nWaiting…");
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
        Command::Provider { command } => provider(&app, command).await,
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

/// Runs a provider command in the running `lorca serve` when there is one, so the app sees the
/// change at once; otherwise here, followed by one sync pass.
async fn provider(app: &std::sync::Arc<App>, command: ProviderCommand) -> anyhow::Result<()> {
    let (method, params) = match command {
        ProviderCommand::Set { kind, api_key, base_url } => {
            if !lorca::credentials::PROVIDER_KINDS.contains(&kind.as_str()) {
                anyhow::bail!("Unknown provider {kind}. Use one of: {}", lorca::credentials::PROVIDER_KINDS.join(", "));
            }
            let params = if matches!(kind.as_str(), "chatgpt" | "grok") {
                if api_key.is_some() || base_url.is_some() {
                    anyhow::bail!("{kind} connects with a browser sign-in and takes no API key");
                }
                println!("Finish the sign-in in the browser…");
                serde_json::json!({})
            } else {
                let api_key = match api_key {
                    Some(api_key) => api_key,
                    None => read_api_key()?,
                };
                serde_json::json!({ "api_key": api_key, "base_url": base_url })
            };
            let method_kind = if kind == "opencode-go" { "opencode_go" } else { &kind };
            (format!("providers.connect_{method_kind}"), params)
        }
        ProviderCommand::Remove { kind } => ("providers.disconnect".to_string(), serde_json::json!({ "kind": kind })),
        ProviderCommand::List => {
            // A running serve holds the same set: both load and save the one credentials file.
            print_providers(&serde_json::to_value(app.credentials.lock().unwrap().statuses())?);
            return Ok(());
        }
    };
    let result = match serve_call(app.config.port, &method, &params).await? {
        Some(result) => result,
        None => {
            let result = lorca::api::dispatch(app, &method, params).await;
            flush_outbox_once(app).await;
            result
        }
    };
    let result = result.map_err(|message| anyhow::anyhow!(message))?;
    print_providers(&result["providers"]);
    Ok(())
}

fn read_api_key() -> anyhow::Result<String> {
    use std::io::IsTerminal;
    if std::io::stdin().is_terminal() {
        eprint!("API key: ");
    }
    let mut line = String::new();
    std::io::stdin().read_line(&mut line)?;
    Ok(line.trim().to_string())
}

fn print_providers(providers: &serde_json::Value) {
    for provider in providers.as_array().into_iter().flatten() {
        println!("{:<10} {}", provider["kind"].as_str().unwrap_or_default(), provider["detail"].as_str().unwrap_or_default());
    }
}

/// One request to the `lorca serve` on `port`; `None` when nothing listens there.
async fn serve_call(port: u16, method: &str, params: &serde_json::Value) -> anyhow::Result<Option<Result<serde_json::Value, String>>> {
    use futures::{SinkExt, StreamExt};
    use tokio_tungstenite::tungstenite::Message;
    let Ok((mut socket, _)) = tokio_tungstenite::connect_async(format!("ws://127.0.0.1:{port}/ws")).await else { return Ok(None) };
    socket.send(Message::Text(serde_json::json!({ "id": 1, "method": method, "params": params }).to_string().into())).await?;
    // Events share the socket; the reply is the message with our id.
    while let Some(message) = socket.next().await {
        let Message::Text(text) = message? else { continue };
        let value: serde_json::Value = serde_json::from_str(&text)?;
        if value["id"] != 1 {
            continue;
        }
        return Ok(Some(match value.get("error") {
            Some(error) => Err(error["message"].as_str().unwrap_or("request failed").to_string()),
            None => Ok(value["result"].clone()),
        }));
    }
    anyhow::bail!("lorca serve closed the connection")
}

/// Exits once the parent process is gone.
async fn watch_parent(pid: u32) {
    loop {
        tokio::time::sleep(std::time::Duration::from_secs(1)).await;
        if !process_alive(pid) {
            tracing::info!(pid, "parent exited; stopping");
            std::process::exit(0);
        }
    }
}

#[cfg(unix)]
fn process_alive(pid: u32) -> bool {
    let signaled = unsafe { libc::kill(pid as i32, 0) } == 0;
    signaled || std::io::Error::last_os_error().raw_os_error() == Some(libc::EPERM)
}

/// A process handle is signaled once the process has exited.
#[cfg(windows)]
fn process_alive(pid: u32) -> bool {
    use windows_sys::Win32::Foundation::{CloseHandle, WAIT_TIMEOUT};
    use windows_sys::Win32::System::Threading::{OpenProcess, WaitForSingleObject, PROCESS_SYNCHRONIZE};
    unsafe {
        let handle = OpenProcess(PROCESS_SYNCHRONIZE, 0, pid);
        if handle.is_null() {
            return false;
        }
        let running = WaitForSingleObject(handle, 0) == WAIT_TIMEOUT;
        CloseHandle(handle);
        running
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
    ok("identity", app.has_identity(), if app.has_identity() { "present".into() } else { "run `lorca identity new`".into() });
    let port_free = std::net::TcpListener::bind(("127.0.0.1", app.config.port)).is_ok();
    ok("port", port_free, if port_free { format!("{} free", app.config.port) } else { format!("{} busy (lorca serve running?)", app.config.port) });
    match app.relay_url() {
        Some(url) => {
            let reachable = app.relay.health(&url).await.is_ok();
            ok("relay", reachable, if reachable { url } else { format!("{url} unreachable") });
        }
        None => ok("relay", false, "not configured (LORCA_RELAY_URL); single-Device mode".into()),
    }
    let credentials = app.credentials.lock().unwrap().connected_kinds();
    ok("providers", !credentials.is_empty(), if credentials.is_empty() { "none connected".into() } else { credentials.join(", ") });
}
