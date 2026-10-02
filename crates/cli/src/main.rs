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
    /// Run the local API the app connects to.
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
        /// A pairing string from a paired Device. Omit to create one here.
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
    /// Manage this computer's MCP servers: mcp.json in Lorca's folder, which every bot on this
    /// computer uses.
    Mcp {
        #[usage(subcommand)]
        command: McpCommand,
    },
    /// The model catalog: the models Lorca offers, with their windows, thinking levels, and rates.
    Models {
        #[usage(subcommand)]
        command: ModelsCommand,
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
    /// Add a custom provider: any server that speaks OpenAI's Chat Completions or Responses, or
    /// Anthropic's Messages, such as a gateway or a model server on your network.
    Add {
        /// The name the apps show.
        name: String,
        /// The API root, such as https://openrouter.ai/api/v1 or http://localhost:11434/v1.
        base_url: String,
        /// The wire protocol it speaks.
        #[usage(long, choices("chat-completions", "responses", "messages"), default = "chat-completions")]
        api: String,
        /// A model id bots can pick; repeat for more. Omit to take every model the server lists.
        #[usage(long)]
        model: Vec<String>,
        /// Read an API key from stdin. Without it the server is called with no key.
        #[usage(long)]
        api_key_stdin: bool,
    },
    /// Disconnect a provider on every Device, or delete a custom one.
    Remove { kind: String },
    /// List the providers and what is connected.
    List,
}

#[derive(Subcommands, Debug)]
enum McpCommand {
    /// List the servers in mcp.json and how each one stands.
    List,
    /// Show a server's settings, connect it, and list the tools it offers.
    Get { name: String },
    /// Add a server: a command this computer runs, or the URL of a remote server.
    ///
    ///   lorca mcp add filesystem npx -y @modelcontextprotocol/server-filesystem ~/Documents
    ///   lorca mcp add linear https://mcp.linear.app/mcp
    ///   lorca mcp add github -e GITHUB_PERSONAL_ACCESS_TOKEN=ghp_… -- docker run -i --rm -e GITHUB_PERSONAL_ACCESS_TOKEN ghcr.io/github/github-mcp-server
    #[usage(verbatim_doc_comment)]
    Add {
        /// The server's name. Bots call its tools as name__tool.
        name: String,
        /// stdio runs a command and http connects to a URL; by default, a URL is http.
        #[usage(long, short = 't', choices("stdio", "http"))]
        transport: Option<String>,
        /// An environment variable for the command, as KEY=value. Repeat it for more.
        #[usage(long, short = 'e', var)]
        env: Vec<String>,
        /// A header for the URL, as "Name: value". Repeat it for more.
        #[usage(long, short = 'H', var)]
        header: Vec<String>,
        /// What the server is for, which bots read beside its tools.
        #[usage(long)]
        description: Option<String>,
        /// Seconds a call may go without an answer or progress; ten minutes when unset.
        #[usage(long)]
        timeout: Option<u64>,
        /// The command and its arguments, or the URL.
        #[usage(double_dash = "automatic")]
        target: Vec<String>,
    },
    /// Add a server from its JSON, as a README or another app's config writes it.
    AddJson {
        name: String,
        /// One server's JSON, such as {"command": "npx", "args": ["-y", "…"]}.
        json: String,
    },
    /// Remove a server, with its sign-in.
    Remove { name: String },
    /// Turn a server on.
    Enable { name: String },
    /// Turn a server off: bots no longer see it, and it never starts. Its settings stay.
    Disable { name: String },
    /// Sign in to a remote server that asks for it, in this computer's browser.
    SignIn { name: String },
    /// Forget the sign-in of a remote server. Its next use asks for a sign-in again.
    SignOut { name: String },
    /// Keep one of a server's tools from bots: its name, or a pattern ending in * (delete_*).
    Hide { name: String, tool: String },
    /// Offer a tool that was hidden to bots again.
    Show { name: String, tool: String },
    /// Read mcp.json again after editing it by hand, so the running lorca serve takes the change.
    Reload,
    /// Add the servers from another app's MCP config: Claude Desktop, Claude Code, Cursor,
    /// Windsurf, VS Code, or Gemini CLI. With no file, from every one of them this computer has.
    Import { file: Option<PathBuf> },
}

#[derive(Subcommands, Debug)]
enum ModelsCommand {
    /// Fetch the latest model catalog from lorca.app now, rather than at the next hourly check.
    Reload,
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let cli = Cli::parse();
    // Bare `lorca` lists the commands and touches nothing: the service is `lorca serve`, as the
    // apps start it, so `lorca` typed to see what it does, a bot's included, never starts a second
    // one or makes a data folder.
    let Some(command) = cli.command else {
        print!("{}", Cli::render_help(Cli::command(), false).unwrap_or_default());
        return Ok(());
    };
    // `lorca mcp` says how each step went in its own words; the log keeps to warnings.
    let quiet = matches!(command, Command::Mcp { .. } | Command::Models { .. });
    tracing_subscriber::fmt()
        .with_env_filter(tracing_subscriber::EnvFilter::try_from_default_env().unwrap_or_else(|_| if quiet { "lorca=warn,lorca_agent=warn".into() } else { "lorca=info,lorca_agent=info".into() }))
        .with_target(false)
        .with_writer(std::io::stderr)
        .init();

    let config = Config::load(cli.home, cli.port);
    let app = App::load(config)?;

    match command {
        Command::Serve { parent_pid, ready_stdout } => {
            runtime::resume_sent_jobs(&app);
            // A command a Lorca that quit left waiting went with it; its row says so now.
            {
                let app = app.clone();
                tokio::task::spawn_blocking(move || lorca::shell::close_stale_rows(&app));
            }
            #[cfg(unix)]
            tokio::spawn(stop_on_signal(app.clone()));
            // Installed marketplace plugins follow the index this build ships.
            lorca::plugins::refresh_installed(&app, &lorca::marketplace::bundled().plugins);
            // The servers in mcp.json, followed as the file changes.
            lorca::plugins::mcp_json::start(&app);
            lorca::catalog::enable(&app);
            lorca::catalog::check_in_background(&app);
            if let Some(pid) = parent_pid {
                tokio::spawn(watch_parent(app.clone(), pid));
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
                // The machine blob goes up now, so the other Devices learn what joined.
                flush_outbox_once(&app).await;
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
        // A server's connection runs deep: on a runtime thread, whose stack is twice the main
        // thread's megabyte on Windows.
        Command::Mcp { command } => {
            let app = app.clone();
            tokio::spawn(async move { mcp(&app, command).await }).await?
        }
        Command::Models { command: ModelsCommand::Reload } => {
            let (reply, live) = match serve_call(app.config.port, "models.reload", &serde_json::json!({})).await? {
                Some(reply) => (reply, true),
                None => {
                    lorca::catalog::enable(&app);
                    // Boxed, as `main`'s future lives on the main thread's stack, a megabyte on Windows.
                    (Box::pin(lorca::api::dispatch(&app, "models.reload", serde_json::json!({}))).await, false)
                }
            };
            let reply = reply.map_err(|message| anyhow::anyhow!(message))?;
            let updated = reply["updated"].as_str().unwrap_or_default();
            match (reply["changed"] == true, live) {
                (true, true) => println!("lorca serve now uses the model catalog of {updated}."),
                (true, false) => println!("Saved the model catalog of {updated}; lorca serve uses it when it starts."),
                (false, _) => println!("The model catalog of {updated} is the latest."),
            }
            Ok(())
        }
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
        ProviderCommand::Add { name, base_url, api, model, api_key_stdin } => {
            let api_key = if api_key_stdin { read_api_key()? } else { String::new() };
            let params = serde_json::json!({ "name": name, "base_url": base_url, "api": api, "models": model, "api_key": api_key });
            ("providers.connect_custom".to_string(), params)
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
            let result = Box::pin(lorca::api::dispatch(app, &method, params)).await;
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
        let mut detail = provider["detail"].as_str().unwrap_or_default().to_string();
        if let Some(name) = provider["name"].as_str() {
            let models: Vec<&str> = provider["models"].as_array().into_iter().flatten().filter_map(|m| m["id"].as_str()).collect();
            let mut shown = models.iter().take(5).copied().collect::<Vec<_>>().join(", ");
            if models.len() > 5 {
                shown.push_str(&format!(", and {} more", models.len() - 5));
            }
            detail = format!("{name} · {} · {detail} · {shown}", provider["api"].as_str().unwrap_or_default());
        }
        println!("{:<10} {detail}", provider["kind"].as_str().unwrap_or_default());
    }
}

/// An `mcp.*` request to the running `lorca serve` when there is one, so the app sees the change
/// at once and the server runs there; else here. Says which it was.
async fn mcp_call(app: &std::sync::Arc<App>, method: &str, params: serde_json::Value) -> anyhow::Result<(serde_json::Value, bool)> {
    let (result, live) = match serve_call(app.config.port, method, &params).await? {
        Some(result) => (result, true),
        // Boxed, as `main`'s future lives on the main thread's stack, a megabyte on Windows.
        None => (Box::pin(lorca::api::dispatch(app, method, params)).await, false),
    };
    Ok((result.map_err(|message| anyhow::anyhow!(message))?, live))
}

async fn mcp(app: &std::sync::Arc<App>, command: McpCommand) -> anyhow::Result<()> {
    use serde_json::json;
    match command {
        McpCommand::List => {
            let (mut list, live) = mcp_call(app, "mcp.list", json!({})).await?;
            if !live {
                // Without lorca serve nothing here is connected, so each server that is on connects
                // to say how it stands, side by side.
                let names: Vec<String> =
                    list["servers"].as_array().into_iter().flatten().filter(|s| s["enabled"] != false && !s["problem"].is_string()).filter_map(|s| s["name"].as_str().map(str::to_string)).collect();
                if !names.is_empty() {
                    println!("Connecting {}…", if names.len() == 1 { names[0].clone() } else { format!("{} servers", names.len()) });
                    futures::future::join_all(names.iter().map(|name| mcp_call(app, "mcp.reconnect", json!({ "name": name, "fresh": false })))).await;
                    list = mcp_call(app, "mcp.list", json!({})).await?.0;
                }
            }
            print_mcp_list(&list);
            // A script can tell that something needs looking at.
            if mcp_needs_attention(&list) {
                std::process::exit(1);
            }
            Ok(())
        }
        McpCommand::Get { name } => {
            let server = mcp_connect(app, &name).await?;
            print_mcp_server(&server);
            Ok(())
        }
        McpCommand::Add { name, transport, env, header, description, timeout, target } => {
            let config = mcp_config(transport.as_deref(), &env, &header, description, timeout, &target)?;
            let (saved, _) = mcp_call(app, "mcp.save", json!({ "name": name, "config": config })).await?;
            println!("Added {name} to {}.", saved["path"].as_str().unwrap_or("mcp.json"));
            let server = mcp_connect(app, &name).await?;
            println!("{}", mcp_outcome(&server));
            Ok(())
        }
        McpCommand::AddJson { name, json } => {
            let servers = lorca::plugins::mcp_json::parse_servers(&json).map_err(|e| anyhow::anyhow!(e))?;
            let entry = match servers.as_slice() {
                [(_, entry)] => entry.clone().map_err(|e| anyhow::anyhow!(e))?,
                many => anyhow::bail!("That JSON has {} servers. Give one server's JSON, or add them all with `lorca mcp import <file>`.", many.len()),
            };
            let config = serde_json::Value::from(&entry);
            let (saved, _) = mcp_call(app, "mcp.save", json!({ "name": name, "config": config })).await?;
            println!("Added {name} to {}.", saved["path"].as_str().unwrap_or("mcp.json"));
            let server = mcp_connect(app, &name).await?;
            println!("{}", mcp_outcome(&server));
            Ok(())
        }
        McpCommand::Remove { name } => {
            mcp_call(app, "mcp.remove", json!({ "name": name })).await?;
            println!("Removed {name}.");
            Ok(())
        }
        McpCommand::Enable { name } => {
            mcp_call(app, "mcp.set_enabled", json!({ "name": name, "enabled": true })).await?;
            println!("Turned {name} on.");
            let server = mcp_connect(app, &name).await?;
            println!("{}", mcp_outcome(&server));
            Ok(())
        }
        McpCommand::Disable { name } => {
            mcp_call(app, "mcp.set_enabled", json!({ "name": name, "enabled": false })).await?;
            println!("Turned {name} off. Bots no longer see it.");
            Ok(())
        }
        McpCommand::SignIn { name } => {
            eprintln!("Opening the {name} sign-in page in the browser. Finish signing in there…");
            // The sign-in ends in the process that started it, which answers once it has.
            mcp_call(app, "mcp.sign_in", json!({ "name": name, "wait": true })).await?;
            println!("✔ Signed in to {name}.");
            Ok(())
        }
        McpCommand::SignOut { name } => {
            mcp_call(app, "mcp.sign_out", json!({ "name": name })).await?;
            println!("Signed out of {name}. Its next use asks for a sign-in again.");
            Ok(())
        }
        McpCommand::Hide { name, tool } => {
            mcp_call(app, "mcp.hide_tool", json!({ "name": name, "tool": tool, "hidden": true })).await?;
            println!("Hid {tool} of {name}. Bots no longer see it.");
            Ok(())
        }
        McpCommand::Show { name, tool } => {
            mcp_call(app, "mcp.hide_tool", json!({ "name": name, "tool": tool, "hidden": false })).await?;
            println!("Bots see {name}'s {tool} again.");
            Ok(())
        }
        McpCommand::Reload => {
            let (list, live) = mcp_call(app, "mcp.reload", json!({})).await?;
            if live {
                println!("lorca serve read mcp.json again.");
            } else {
                println!("lorca serve is not running; it reads mcp.json when it starts.");
            }
            if let Some(error) = list["error"].as_str() {
                anyhow::bail!("{error}");
            }
            Ok(())
        }
        McpCommand::Import { file } => mcp_import(app, file).await,
    }
}

/// Connects a server, or takes the connection it has, and answers how it stands, with its tools.
/// Without a running `lorca serve` the server ran in this process, which stops it before leaving.
async fn mcp_connect(app: &std::sync::Arc<App>, name: &str) -> anyhow::Result<serde_json::Value> {
    eprintln!("Connecting {name}…");
    let (got, live) = mcp_call(app, "mcp.reconnect", serde_json::json!({ "name": name, "fresh": false })).await?;
    if !live {
        if let Some(id) = got["server"]["id"].as_str() {
            app.mcp.forget(id);
            tokio::time::sleep(std::time::Duration::from_millis(300)).await;
        }
    }
    Ok(got["server"].clone())
}

/// What `lorca mcp add` says once the server has had its first try.
fn mcp_outcome(server: &serde_json::Value) -> String {
    let name = server["name"].as_str().unwrap_or("The server");
    let status = &server["status"];
    match status["state"].as_str() {
        Some("ready") => match server["tool_count"].as_u64() {
            Some(count) => format!("✔ {name} is ready with {count} tool{}.", if count == 1 { "" } else { "s" }),
            None => format!("✔ {name} is ready."),
        },
        Some("needs_auth") => format!("! {name} needs a sign-in: run `lorca mcp sign-in {name}`, or sign in from Settings › Plugins in the app."),
        Some(_) => format!("✘ {name} did not connect: {}", status["detail"].as_str().unwrap_or("unknown error")),
        None => match server["problem"].as_str() {
            Some(problem) => format!("✘ {problem}"),
            None => format!("{name} is off."),
        },
    }
}

/// The entry `lorca mcp add` writes: a command with its environment, or a URL with its headers.
/// What follows a command is its arguments, flags included (`npx -y …`, `docker run -e …`); what
/// follows a URL is read as this command's own flags, which people write after it.
fn mcp_config(transport: Option<&str>, env: &[String], headers: &[String], description: Option<String>, timeout: Option<u64>, target: &[String]) -> anyhow::Result<serde_json::Value> {
    let Some(first) = target.first() else { anyhow::bail!("Give the command to run, or the server's URL, after the name.") };
    let is_url = first.starts_with("http://") || first.starts_with("https://");
    let (mut transport, mut env, mut headers, mut description, mut timeout) = (transport.map(str::to_string), env.to_vec(), headers.to_vec(), description, timeout);
    let mut target = target.to_vec();
    if is_url && transport.as_deref() != Some("stdio") {
        let mut rest = target.split_off(1).into_iter();
        while let Some(word) = rest.next() {
            let (flag, attached) = match word.split_once('=') {
                Some((flag, value)) if flag.starts_with("--") => (flag.to_string(), Some(value.to_string())),
                _ => (word.clone(), None),
            };
            let mut value = || attached.clone().or_else(|| rest.next()).ok_or_else(|| anyhow::anyhow!("{flag} needs a value."));
            match flag.as_str() {
                "-H" | "--header" => headers.push(value()?),
                "-e" | "--env" => env.push(value()?),
                "--description" => description = Some(value()?),
                "--timeout" => timeout = Some(value()?.parse().map_err(|_| anyhow::anyhow!("--timeout is a number of seconds."))?),
                "-t" | "--transport" => transport = Some(value()?),
                other => anyhow::bail!("A remote server is one URL, and {other:?} follows it. Flags go before or after the URL; a command's arguments follow the command."),
            }
        }
    }
    let transport = transport.as_deref();
    if transport.is_some_and(|transport| !["stdio", "http"].contains(&transport)) {
        anyhow::bail!("--transport is stdio or http.");
    }
    let remote = match transport {
        Some(transport) => transport != "stdio",
        None => target.len() == 1 && is_url,
    };
    let mut config = serde_json::Map::new();
    if remote {
        if target.len() > 1 {
            anyhow::bail!("A remote server is one URL; {} more words follow it.", target.len() - 1);
        }
        if !env.is_empty() {
            anyhow::bail!("-e sets a command's environment. A remote server takes headers: -H \"Name: value\".");
        }
        config.insert("type".into(), "http".into());
        config.insert("url".into(), first.clone().into());
        let mut map = serde_json::Map::new();
        for header in headers {
            let (name, value) = header.split_once(':').ok_or_else(|| anyhow::anyhow!("A header is \"Name: value\", not {header:?}."))?;
            map.insert(name.trim().to_string(), value.trim().into());
        }
        if !map.is_empty() {
            config.insert("headers".into(), map.into());
        }
    } else {
        if !headers.is_empty() {
            anyhow::bail!("-H sets a remote server's headers. A command takes its environment: -e KEY=value.");
        }
        config.insert("command".into(), first.clone().into());
        if target.len() > 1 {
            config.insert("args".into(), target[1..].to_vec().into());
        }
        let mut map = serde_json::Map::new();
        for variable in env {
            let (name, value) = variable.split_once('=').ok_or_else(|| anyhow::anyhow!("An environment variable is KEY=value, not {variable:?}."))?;
            map.insert(name.trim().to_string(), value.into());
        }
        if !map.is_empty() {
            config.insert("env".into(), map.into());
        }
    }
    if let Some(description) = description.filter(|d| !d.trim().is_empty()) {
        config.insert("description".into(), description.trim().into());
    }
    // The file reads a timeout of 1000 or more as milliseconds, as Gemini CLI writes it, so a long one
    // is written that way.
    if let Some(seconds) = timeout.filter(|seconds| *seconds > 0) {
        config.insert("timeout".into(), (if seconds >= 1000 { seconds * 1000 } else { seconds }).into());
    }
    Ok(serde_json::Value::Object(config))
}

/// Adds every server another app's config has, skipping names mcp.json already has.
async fn mcp_import(app: &std::sync::Arc<App>, file: Option<PathBuf>) -> anyhow::Result<()> {
    use serde_json::json;
    let named = file.is_some();
    let sources: Vec<(String, PathBuf)> = match file {
        Some(path) => vec![(path.display().to_string(), path)],
        None => lorca::plugins::mcp_json::known_sources().into_iter().map(|(app, path)| (app.to_string(), path)).collect(),
    };
    if sources.is_empty() {
        println!("No other app's MCP servers on this computer. Name a file: lorca mcp import <file>");
        return Ok(());
    }
    let (list, _) = mcp_call(app, "mcp.list", json!({})).await?;
    let mut names: Vec<String> = list["servers"].as_array().into_iter().flatten().filter_map(|s| s["name"].as_str().map(str::to_string)).collect();
    for (label, path) in sources {
        // A file named here may hold servers any way a README writes them; an app's settings file
        // holds them under its servers' object, or has none.
        let read = |text: String| if named { lorca::plugins::mcp_json::parse_servers(&text) } else { lorca::plugins::mcp_json::servers_in_app_config(&text) };
        let servers = match std::fs::read_to_string(&path).map_err(|e| e.to_string()).and_then(read) {
            Ok(servers) if servers.is_empty() => {
                println!("{label}: no MCP servers.");
                continue;
            }
            Ok(servers) => servers,
            Err(error) => {
                println!("{label}: {error}");
                continue;
            }
        };
        let (mut added, mut skipped) = (Vec::new(), Vec::new());
        for (name, entry) in servers {
            let Some(name) = name else {
                skipped.push("a server with no name (add it with `lorca mcp add-json <name> <json>`)".to_string());
                continue;
            };
            match entry {
                Err(problem) => skipped.push(format!("{name}: {problem}")),
                Ok(_) if names.contains(&name) => skipped.push(format!("{name}: mcp.json has it already")),
                Ok(entry) => match mcp_call(app, "mcp.save", json!({ "name": name, "config": serde_json::Value::from(&entry) })).await {
                    Ok(_) => {
                        names.push(name.clone());
                        added.push(name);
                    }
                    Err(error) => skipped.push(format!("{name}: {error}")),
                },
            }
        }
        match added.as_slice() {
            [] => println!("{label}: nothing to add."),
            added => println!("{label}: added {}.", added.join(", ")),
        }
        for line in skipped {
            println!("  skipped {line}");
        }
    }
    Ok(())
}

/// `npx -y "My Folder"`: a command and its arguments as a shell would take them.
fn command_line(server: &serde_json::Value) -> String {
    let config = &server["config"];
    if let Some(url) = config["url"].as_str() {
        return url.to_string();
    }
    let words = std::iter::once(config["command"].as_str().unwrap_or_default()).chain(config["args"].as_array().into_iter().flatten().filter_map(|arg| arg.as_str()));
    words
        .map(|word| if word.is_empty() || word.contains(|c: char| c.is_whitespace() || c == '"' || c == '\'') { format!("\"{}\"", word.replace('"', "\\\"")) } else { word.to_string() })
        .collect::<Vec<_>>()
        .join(" ")
}

/// A value whose name says it is a key, token, or password, cut to its last four characters.
fn masked(name: &str, value: &str) -> String {
    let lower = name.to_ascii_lowercase();
    let secret = ["key", "token", "secret", "password", "auth", "credential"].iter().any(|word| lower.contains(word));
    let count = value.chars().count();
    if !secret || value.contains("${") || count <= 8 {
        return value.to_string();
    }
    format!("••••{}", value.chars().skip(count - 4).collect::<String>())
}

/// How a server stands, in a few words.
fn mcp_state(server: &serde_json::Value) -> String {
    if let Some(problem) = server["problem"].as_str() {
        return format!("✘ {problem}");
    }
    if server["enabled"] == false {
        return "– off".into();
    }
    let status = &server["status"];
    match status["state"].as_str() {
        Some("ready") => match server["tool_count"].as_u64() {
            Some(count) => format!("✔ {count} tool{}", if count == 1 { "" } else { "s" }),
            None => "· not connected yet".into(),
        },
        Some("needs_auth") => "! needs a sign-in".into(),
        Some("connecting") => format!("… {}", status["detail"].as_str().unwrap_or("connecting")),
        Some(_) => format!("✘ {}", status["detail"].as_str().unwrap_or("error")),
        None => String::new(),
    }
}

/// Whether `mcp.json` does not read, or a server that is on cannot run, failed, or needs a
/// sign-in or setup.
fn mcp_needs_attention(list: &serde_json::Value) -> bool {
    list["error"].is_string()
        || list["servers"].as_array().into_iter().flatten().any(|server| {
            server["enabled"] != false && (server["problem"].is_string() || matches!(server["status"]["state"].as_str(), Some("error" | "needs_auth" | "needs_setup")))
        })
}

fn print_mcp_list(list: &serde_json::Value) {
    let path = list["path"].as_str().unwrap_or("mcp.json");
    if let Some(error) = list["error"].as_str() {
        println!("✘ {error}\n");
    }
    let servers = list["servers"].as_array().cloned().unwrap_or_default();
    if servers.is_empty() {
        println!("No MCP servers in {path} yet. Add one with `lorca mcp add <name> <command or URL>`.");
        return;
    }
    println!("{path}\n");
    let width = servers.iter().filter_map(|s| s["name"].as_str()).map(|n| n.chars().count()).max().unwrap_or(0);
    let states: Vec<String> = servers.iter().map(mcp_state).collect();
    let state_width = states.iter().map(|s| s.chars().count()).max().unwrap_or(0).min(40);
    for (server, state) in servers.iter().zip(&states) {
        let name = server["name"].as_str().unwrap_or_default();
        let target: String = command_line(server).chars().take(90).collect();
        println!("  {name:<width$}  {state:<state_width$}  {target}");
    }
}

fn print_mcp_server(server: &serde_json::Value) {
    let config = &server["config"];
    println!("{}", server["name"].as_str().unwrap_or_default());
    let row = |label: &str, value: &str| println!("  {label:<9} {value}");
    if config["url"].is_string() {
        row("URL", config["url"].as_str().unwrap_or_default());
        for (name, value) in config["headers"].as_object().into_iter().flatten() {
            row("Header", &format!("{name}: {}", masked(name, value.as_str().unwrap_or_default())));
        }
    } else {
        row("Command", &command_line(server));
        for (name, value) in config["env"].as_object().into_iter().flatten() {
            row("Env", &format!("{name}={}", masked(name, value.as_str().unwrap_or_default())));
        }
        if let Some(cwd) = config["cwd"].as_str() {
            row("Folder", cwd);
        }
    }
    if let Some(description) = config["description"].as_str() {
        row("About", description);
    }
    if let Some(timeout) = config["timeout"].as_u64() {
        // 1000 or more is milliseconds, as the file reads it.
        let seconds = if timeout >= 1000 { timeout.div_ceil(1000) } else { timeout };
        row("Timeout", &format!("{seconds} s without an answer or progress"));
    }
    row("State", &mcp_state(server));
    if server["status"]["state"] == "needs_auth" {
        row("", &format!("Run `lorca mcp sign-in {}`.", server["name"].as_str().unwrap_or_default()));
    }
    let tools = server["tools"].as_array().cloned().unwrap_or_default();
    if !tools.is_empty() {
        println!("  Tools");
        let width = tools.iter().filter_map(|t| t["name"].as_str()).map(|n| n.chars().count()).max().unwrap_or(0).min(36);
        for tool in tools {
            let about: String = tool["description"].as_str().unwrap_or_default().chars().take(100).collect();
            let hidden = if tool["hidden"] == true { " (hidden from bots)" } else { "" };
            println!("    {:<width$}  {about}{hidden}", tool["name"].as_str().unwrap_or_default());
        }
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
async fn watch_parent(app: std::sync::Arc<App>, pid: u32) {
    loop {
        tokio::time::sleep(std::time::Duration::from_secs(1)).await;
        if !process_alive(pid) {
            tracing::info!(pid, "parent exited; stopping");
            app.shell_sessions.shutdown(&app);
            std::process::exit(0);
        }
    }
}

/// Quitting (the app stopping its CLI, Ctrl-C, a closed terminal) first stops the commands bots
/// left running in their terminals, so none outlives Lorca and their rows say so; then the
/// signal ends the process as it would have.
#[cfg(unix)]
async fn stop_on_signal(app: std::sync::Arc<App>) {
    use tokio::signal::unix::{signal, SignalKind};
    let (Ok(mut terminate), Ok(mut interrupt), Ok(mut hangup)) = (signal(SignalKind::terminate()), signal(SignalKind::interrupt()), signal(SignalKind::hangup())) else {
        return;
    };
    let number = tokio::select! {
        _ = terminate.recv() => libc::SIGTERM,
        _ = interrupt.recv() => libc::SIGINT,
        _ = hangup.recv() => libc::SIGHUP,
    };
    app.shell_sessions.shutdown(&app);
    unsafe {
        libc::signal(number, libc::SIG_DFL);
        libc::raise(number);
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
    let (servers, problems, error) = {
        let store = app.plugins.lock().unwrap();
        let problems: Vec<String> = store.mcp.servers.iter().filter_map(|server| server.problem().map(|problem| format!("{}: {problem}", server.name))).collect();
        (store.mcp.servers.len(), problems, store.mcp.error.clone())
    };
    let path = app.config.mcp_path().display().to_string();
    match error {
        Some(error) => ok("mcp.json", false, error),
        None if problems.is_empty() => ok("mcp.json", true, format!("{servers} server{} in {path}", if servers == 1 { "" } else { "s" })),
        None => ok("mcp.json", false, problems.join("; ")),
    }
}
