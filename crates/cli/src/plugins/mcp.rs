//! The MCP side of plugins on this Runner: a pool of connected servers (`rmcp`, stdio or
//! streamable HTTP), the catalog a turn's codemode scripts call plugin tools from, the OAuth
//! sign-in for a remote server, and Auto-review (`review.rs`) at the loop's
//! `before_tool_call` boundary. A read-only tool runs; anything else may ask the user in the
//! chat first, after Grok Bot.

use std::collections::{BTreeMap, HashMap};
use std::sync::{Arc, Mutex};

use async_trait::async_trait;
use lorca_agent::codemode::{self, Entry, Exposure, Namespace, NamespaceDetails};
use lorca_agent::{BeforeToolCallContext, BeforeToolCallResult};
use rmcp::model::{CallToolRequestParams, ClientConfig, ContentBlock, Implementation};
use rmcp::service::RunningService;
use rmcp::transport::auth::{AuthClient, AuthorizationManager, AuthorizationRequest, OAuthState};
use rmcp::transport::streamable_http_client::StreamableHttpClientTransportConfig;
use rmcp::transport::{StreamableHttpClientTransport, TokioChildProcess};
use rmcp::{RoleClient, ServiceExt};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use lorca_agent::agent_loop::ToolExecutionMode;
use lorca_agent::{ContentPart, Tool, ToolError, ToolResult, ToolUpdateFn};
use tokio_util::sync::CancellationToken;

use super::{fill, fill_if_set, pattern_matches, AuthSpec, Installed, ServerSpec};
use crate::app::App;
use crate::config::now_secs;
use crate::model::*;

/// How long the user has to answer a permission card before the call is refused.
pub const PERMISSION_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(10 * 60);
/// How long a server may take to start and answer the MCP handshake. A package runner such as
/// `npx` may download the server first.
const CONNECT_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(2 * 60);
/// Reconnect a device-flow server before its bearer expires. A turn that starts inside this
/// window gets a fresh token, so the static bearer cannot expire midway through ordinary work.
const DEVICE_TOKEN_REFRESH_BUFFER_SECS: f64 = 5.0 * 60.0;
/// The most text a tool result carries to the model.
const MAX_RESULT_CHARS: usize = 50_000;
/// How much of a tool's description, schema, and server instructions `searchTools()` ranks by.
const MAX_SEARCH_INDEX_DESCRIPTION_BYTES: usize = 2 * 1024;
const MAX_SEARCH_SCHEMA_BYTES: usize = 4 * 1024;
const MAX_SERVER_INSTRUCTIONS_BYTES: usize = 2 * 1024;
/// How much of a plugin's server instructions the codemode description carries.
const MAX_NAMESPACE_INSTRUCTIONS_BYTES: usize = 1024;

// MARK: - Pool

/// One connected MCP server.
pub struct Server {
    pub plugin_id: String,
    pub name: String,
    service: RunningService<RoleClient, Client>,
    /// What the server offers, which it may change while connected (`tools/list_changed`).
    tools: std::sync::RwLock<Vec<rmcp::model::Tool>>,
    pub instructions: Option<String>,
    /// Whether it offers resources, which scripts list and read through its resource tools.
    pub resources: bool,
    /// The OAuth manager behind the transport, to persist tokens it refreshed.
    auth: Option<Arc<tokio::sync::Mutex<AuthorizationManager>>>,
    /// Device-flow tokens are plain bearers. Drop this pooled server before its bearer expires;
    /// the next connection refreshes and persists the rotating token pair itself.
    bearer_expires_at: Option<f64>,
    generation: u64,
}

impl Server {
    pub fn tools(&self) -> Vec<rmcp::model::Tool> {
        self.tools.read().unwrap().clone()
    }

    /// Whether the connection is gone: the server's process ended, or its transport closed.
    pub fn is_closed(&self) -> bool {
        self.service.is_closed() || self.service.peer().is_transport_closed()
    }

    /// Ends the connection, and with a stdio server its process.
    pub fn stop(&self) { self.service.cancellation_token().cancel(); }

    /// Whether this is still the installed plugin's server: not one from before an update or a removal.
    pub fn is_current(&self, app: &App) -> bool {
        app.plugins.lock().unwrap().get(&self.plugin_id).is_some()
            && self.generation == app.mcp.generation(&self.plugin_id)
    }

    /// Waits up to ten seconds for a stopped server's transport to close.
    pub async fn wait_stopped(&self) -> Result<(), String> {
        tokio::time::timeout(std::time::Duration::from_secs(10), async {
            while !self.service.peer().is_transport_closed() {
                tokio::time::sleep(std::time::Duration::from_millis(20)).await;
            }
        }).await.map_err(|_| "The browser didn't close.".to_string())
    }

    /// A call Lorca makes itself (opening, closing, a screenshot), its error result as an error.
    pub async fn browser_call(&self, name: &str, args: Value) -> Result<rmcp::model::CallToolResult, String> {
        let mut params = CallToolRequestParams::default();
        params.name = name.to_string().into();
        params.arguments = args.as_object().cloned();
        let result = tokio::time::timeout(std::time::Duration::from_secs(120), self.service.peer().call_tool(params)).await
            .map_err(|_| { self.stop(); "The browser didn't answer in time and was closed.".to_string() })?
            .map_err(|e| e.to_string())?;
        if result.is_error == Some(true) {
            return Err(result.content.iter().filter_map(|block| match block { ContentBlock::Text(text) => Some(text.text.as_str()), _ => None }).collect::<Vec<_>>().join("\n"));
        }
        Ok(result)
    }

    /// Starts the browser if it isn't running yet and brings its current tab forward.
    pub async fn open_visible(&self) -> Result<(), String> {
        let tabs = self.browser_call("browser_tabs", json!({ "action": "list" })).await?;
        // Playwright lists tabs as `- 0: (current) [title] (url)`; selecting one fronts it.
        let current = tabs.content.iter().filter_map(|block| match block { ContentBlock::Text(text) => Some(text.text.as_str()), _ => None })
            .flat_map(str::lines).find_map(|line| line.trim().strip_prefix("- ")?.split_once(": (current)")?.0.parse::<usize>().ok());
        if let Some(current) = current {
            self.browser_call("browser_tabs", json!({ "action": "select", "index": current })).await?;
        }
        Ok(())
    }
}

/// A headed Browser server for one of a bot's profiles, a process of its own with the profile's
/// folder. The plugin's shared headless server still answers what the catalog lists.
pub async fn visible_browser(app: &Arc<App>, session_id: &str) -> Result<Arc<Server>, String> {
    let (plugin, values) = {
        let store = app.plugins.lock().unwrap();
        (store.get(crate::browser::PLUGIN_ID).cloned().ok_or("Install the Browser plugin on this Runner first.")?, store.values(crate::browser::PLUGIN_ID))
    };
    let Some(ServerSpec::Stdio { command, args, env, cwd, timeout }) = plugin.manifest.servers.get("browser") else {
        return Err("This Browser plugin doesn't run Playwright on the Runner, so its profiles can't open.".into());
    };
    // The profile's folder is the one Lorca gives it: a config, CDP, or extension override could
    // attach it to another browser.
    if args.iter().any(|arg| ["--config", "--cdp-endpoint", "--endpoint", "--extension", "--isolated", "--storage-state", "--user-data-dir", "--port"].iter().any(|flag| arg == flag || arg.starts_with(&format!("{flag}=")))) {
        return Err("This Browser plugin sets its own profile or connection, so its profiles can't open.".into());
    }
    if env.keys().any(|key| matches!(key.as_str(), "PLAYWRIGHT_MCP_CONFIG" | "PLAYWRIGHT_MCP_CDP_ENDPOINT" | "PLAYWRIGHT_MCP_ENDPOINT" | "PLAYWRIGHT_MCP_CDP_HEADERS" | "PLAYWRIGHT_MCP_EXTENSION" | "PLAYWRIGHT_MCP_ISOLATED" | "PLAYWRIGHT_MCP_STORAGE_STATE" | "PLAYWRIGHT_MCP_PORT")) {
        return Err("This Browser plugin sets its own profile or connection, so its profiles can't open.".into());
    }
    let dir = app.config.home.join("browser/profiles").join(session_id);
    let output = app.config.home.join("browser/output").join(session_id);
    std::fs::create_dir_all(&output).map_err(|e| e.to_string())?;
    crate::config::set_private(output.parent().unwrap()).map_err(|e| e.to_string())?;
    crate::config::set_private(&output).map_err(|e| e.to_string())?;
    let mut args: Vec<String> = args.iter().filter(|arg| *arg != "--headless" && !arg.starts_with("--headless=")).cloned().collect();
    args.extend(["--user-data-dir".into(), dir.display().to_string(), "--output-dir".into(), output.display().to_string(), "--image-responses".into(), "allow".into()]);
    let mut env = env.clone();
    env.insert("PLAYWRIGHT_MCP_HEADLESS".into(), "false".into());
    let spec = ServerSpec::Stdio { command: command.clone(), args, env, cwd: cwd.clone(), timeout: *timeout };
    let values = template_values(&plugin, values).await;
    let generation = app.mcp.generation(crate::browser::PLUGIN_ID);
    let server = tokio::time::timeout(CONNECT_TIMEOUT, connect(app, &plugin, "browser", &spec, &values, generation)).await.map_err(|_| "The Browser server did not start in time.".to_string())??;
    save_catalog(app, &plugin.manifest.id, "browser", SavedServer { instructions: server.instructions.clone(), tools: server.tools(), resources: server.resources });
    Ok(Arc::new(server))
}

/// Connected servers by `plugin/server`, connected on first use and dropped when the plugin
/// changes. Held by the App.
pub struct Pool {
    servers: Mutex<HashMap<String, Arc<Server>>>,
    /// Bumped by `forget`, per plugin: a connection that started before a change of the
    /// plugin's settings is used for nothing once it answers.
    generations: Mutex<HashMap<String, u64>>,
    /// One connection at a time per server, so two calls never start one server twice, while
    /// a server that is slow to start (`npx` downloading it) holds up no other.
    connecting: Mutex<HashMap<String, Arc<tokio::sync::Mutex<()>>>>,
    /// The HTTP client the MCP transports and the OAuth flow use (rmcp's reqwest, not the
    /// App's).
    http: mcp_http::Client,
    /// Browser sign-ins another Device finishes, by plugin, each with an id of its own.
    sign_ins: Mutex<HashMap<String, (String, Pending)>>,
}

impl Default for Pool {
    fn default() -> Self {
        Self::new()
    }
}

impl Pool {
    pub fn new() -> Self {
        let http = mcp_http::Client::builder()
            .tls_backend_preconfigured(lorca_tls::client_config(&["h2", "http/1.1"]))
            .timeout(std::time::Duration::from_secs(600))
            .build()
            .unwrap_or_default();
        Pool { servers: Mutex::new(HashMap::new()), generations: Mutex::new(HashMap::new()), connecting: Mutex::new(HashMap::new()), http, sign_ins: Mutex::new(HashMap::new()) }
    }

    /// Drops every connection of a plugin, so the next use reconnects with fresh settings.
    pub fn forget(&self, plugin_id: &str) {
        *self.generations.lock().unwrap().entry(plugin_id.to_string()).or_default() += 1;
        self.servers.lock().unwrap().retain(|key, _| !key.starts_with(&format!("{plugin_id}/")));
    }

    pub(super) fn generation(&self, plugin_id: &str) -> u64 {
        self.generations.lock().unwrap().get(plugin_id).copied().unwrap_or_default()
    }

    fn cached_server(&self, key: &str) -> Option<Arc<Server>> {
        let mut servers = self.servers.lock().unwrap();
        // A server whose process ended or whose connection dropped starts again, and one whose
        // device-flow bearer is about to expire connects with a fresh one.
        let stale = servers.get(key).is_some_and(|server| {
            server.is_closed() || server.bearer_expires_at.is_some_and(|expires_at| expires_at <= now_secs() + DEVICE_TOKEN_REFRESH_BUFFER_SECS)
        });
        if stale {
            servers.remove(key);
            None
        } else {
            servers.get(key).cloned()
        }
    }

    /// The connected server, connecting it first when needed.
    pub async fn server(&self, app: &Arc<App>, plugin_id: &str, name: &str) -> Result<Arc<Server>, String> {
        let key = format!("{plugin_id}/{name}");
        if let Some(server) = self.cached_server(&key) {
            return Ok(server);
        }
        let gate = self.connecting.lock().unwrap().entry(key.clone()).or_default().clone();
        let _guard = gate.lock().await;
        if let Some(server) = self.cached_server(&key) {
            return Ok(server);
        }
        let (plugin, values, generation) = {
            let store = app.plugins.lock().unwrap();
            let plugin = store.get(plugin_id).cloned().ok_or_else(|| format!("{plugin_id} is not installed on this Runner"))?;
            (plugin, store.values(plugin_id), self.generation(plugin_id))
        };
        let values = template_values(&plugin, values).await;
        let spec = plugin.manifest.servers.get(name).cloned().ok_or_else(|| format!("{} has no server {name}", plugin.manifest.name))?;
        super::note(app, plugin_id, Some(("connecting", "Connecting…")));
        let connected = tokio::time::timeout(CONNECT_TIMEOUT, connect(app, &plugin, name, &spec, &values, generation))
            .await
            .unwrap_or_else(|_| Err(format!("{} did not start within {} minutes", plugin.manifest.name, CONNECT_TIMEOUT.as_secs() / 60)));
        match connected {
            // The plugin changed (new settings, a sign-in, an uninstall) while it connected.
            Ok(_) if self.generation(plugin_id) != generation => {
                Err(format!("{}'s settings changed while it connected. Try again.", plugin.manifest.name))
            }
            Ok(server) => {
                save_catalog(app, plugin_id, name, SavedServer { instructions: server.instructions.clone(), tools: server.tools(), resources: server.resources });
                let server = Arc::new(server);
                self.servers.lock().unwrap().insert(key, server.clone());
                super::note(app, plugin_id, None);
                Ok(server)
            }
            Err(error) => {
                if self.generation(plugin_id) != generation { return Err("The account's settings changed while connecting. Try again.".into()); }
                if let Some((scope, challenge)) = insufficient_scope(&std::io::Error::other(error.clone())) {
                    needs_more_access(app, plugin_id, name, &scope, &challenge);
                } else if error.contains("sign-in needs more access") {
                    needs_more_access(app, plugin_id, name, "", "");
                }
                // A named account whose authorization expired or was revoked reads Sign in.
                if plugin.service_id.is_some() && authorization_expired(&error) {
                    let _ = super::set_oauth(app, plugin_id, name, None);
                }
                // A server that answered that it needs a sign-in reads Sign in, not an error.
                let signs_in = {
                    let store = app.plugins.lock().unwrap();
                    store.needs_sign_in(plugin_id, name, &spec, &store.values(plugin_id))
                };
                super::note(app, plugin_id, (!signs_in).then_some(("error", error.as_str())));
                Err(error)
            }
        }
    }

    /// Every server of a plugin, connected; a server that fails is left out with a warning.
    pub async fn servers_of(&self, app: &Arc<App>, plugin_id: &str) -> Vec<Arc<Server>> {
        let names: Vec<String> = app.plugins.lock().unwrap().get(plugin_id).map(|p| p.manifest.servers.keys().cloned().collect()).unwrap_or_default();
        let mut servers = Vec::new();
        for name in names {
            match self.server(app, plugin_id, &name).await {
                Ok(server) => servers.push(server),
                // A server that asked for a sign-in did what it should.
                Err(error) if app.plugins.lock().unwrap().status(plugin_id).is_some_and(|status| status.state == "needs_auth") => {
                    tracing::info!(%error, plugin = plugin_id, server = %name, "a plugin server needs a sign-in")
                }
                Err(error) => tracing::warn!(%error, plugin = plugin_id, server = %name, "connecting a plugin server"),
            }
        }
        servers
    }
}

async fn connect(app: &Arc<App>, plugin: &Installed, name: &str, spec: &ServerSpec, values: &BTreeMap<String, String>, generation: u64) -> Result<Server, String> {
    let mut implementation = Implementation::default();
    implementation.name = "Lorca".into();
    implementation.version = crate::config::VERSION.into();
    let mut info = ClientConfig::default();
    info.client_info = implementation;
    // A client per try, since a handshake takes the one it is given.
    let client = || Client { info: info.clone(), app: Arc::downgrade(app), plugin_id: plugin.manifest.id.clone(), server: name.to_string(), generation };
    let mut auth = None;
    let mut bearer_expires_at = None;
    let service = match spec {
        ServerSpec::Stdio { command, args, env, cwd, .. } => {
            let command = expand_home(&fill(command, values));
            // The login shell's environment, so `npx` or `uvx` resolve from the user's PATH, on
            // Windows as files the way a terminal finds them (`npx` is npm's `npx.cmd`).
            let mut cmd = lorca_agent::login_shell::command(&command).await;
            if plugin.manifest.id == crate::browser::PLUGIN_ID && args.iter().any(|arg| arg == "--user-data-dir") {
                // Nor may the login shell's environment attach a profile to another browser.
                for key in ["PLAYWRIGHT_MCP_CONFIG", "PLAYWRIGHT_MCP_CDP_ENDPOINT", "PLAYWRIGHT_MCP_ENDPOINT", "PLAYWRIGHT_MCP_CDP_HEADERS", "PLAYWRIGHT_MCP_EXTENSION", "PLAYWRIGHT_MCP_ISOLATED", "PLAYWRIGHT_MCP_STORAGE_STATE", "PLAYWRIGHT_MCP_PORT"] {
                    cmd.env_remove(key);
                }
            }
            cmd.args(args.iter().map(|a| expand_home(&fill(a, values))));
            // A variable naming an optional key the user left unset is left out.
            for (key, value) in env {
                if let Some(value) = fill_if_set(value, values) {
                    cmd.env(key, value);
                }
            }
            let dir = match cwd {
                Some(cwd) => std::path::PathBuf::from(expand_home(&fill(cwd, values))),
                None => app.config.plugins_dir().join(&plugin.manifest.id),
            };
            if cwd.is_none() {
                // A server from mcp.json has no folder until it first starts.
                let _ = std::fs::create_dir_all(&dir);
            } else if !dir.is_dir() {
                return Err(format!("{} cannot start in {}: there is no such folder.", plugin.manifest.name, dir.display()));
            }
            cmd.current_dir(dir);
            // On Windows a bare name starts a file found on PATH (npx.cmd), which the error names:
            // a batch file refuses an argument with a line break.
            let program = std::path::Path::new(cmd.as_std().get_program()).display().to_string();
            let starting = if program == command { program } else { format!("{command} ({program})") };
            // The server and what it starts in turn (npx starts node, uvx starts python) stop
            // together: a process group on macOS and Linux, a job object on Windows.
            let mut wrapped = process_wrap::tokio::CommandWrap::from(cmd);
            #[cfg(unix)]
            wrapped.wrap(process_wrap::tokio::ProcessGroup::leader());
            #[cfg(windows)]
            wrapped.wrap(process_wrap::tokio::JobObject);
            wrapped.wrap(process_wrap::tokio::KillOnDrop);
            let (transport, stderr) =
                TokioChildProcess::builder(wrapped).stderr(std::process::Stdio::piped()).spawn().map_err(|e| format!("Cannot start {starting}: {e}"))?;
            let said = Stderr::follow(stderr, &plugin.manifest.id);
            match client().serve(transport).await {
                Ok(service) => service,
                // What a server printed before it stopped usually says why.
                Err(error) => {
                    let words = said.last_words().await.unwrap_or_else(|| error.to_string());
                    return Err(format!("{command} did not answer the MCP handshake: {words}"));
                }
            }
        }
        ServerSpec::Http { url, headers, auth: auth_spec, .. } => {
            // `${VAR}` in an mcp.json server's URL is the environment's.
            let url = &fill(url, values);
            let mut config = StreamableHttpClientTransportConfig::with_uri(url.as_str());
            let mut custom = HashMap::new();
            // A header naming an optional key the user left unset is left out: Context7 without its
            // key works on the free limits, and with the placeholder refuses every call.
            for (key, value) in headers {
                let Some(value) = fill_if_set(value, values) else { continue };
                let key = mcp_http::header::HeaderName::from_bytes(key.as_bytes()).map_err(|e| format!("Bad header {key}: {e}"))?;
                let value = mcp_http::header::HeaderValue::from_str(&value).map_err(|e| format!("Bad header value: {e}"))?;
                custom.insert(key, value);
            }
            config = config.custom_headers(custom);
            let pasted = match auth_spec {
                Some(AuthSpec::Bearer { variable }) => values.get(variable).cloned(),
                Some(AuthSpec::Oauth { token_variable: Some(variable), .. }) => values.get(variable).cloned(),
                _ => None,
            };
            let tokens = {
                let store = app.plugins.lock().unwrap();
                if app.mcp.generation(&plugin.manifest.id) != generation { return Err("The account's settings changed while connecting. Try again.".into()); }
                store.sign_in_secret(&plugin.manifest.id, "oauth", name)
            };
            let http = crate::connector_limits::LimitedHttpClient::new(app, app.mcp.http.clone(), &plugin.manifest.id);
            let plain = |config: StreamableHttpClientTransportConfig| {
                let http = http.clone();
                move || StreamableHttpClientTransport::with_client(http.clone(), config.clone())
            };
            match (pasted, auth_spec, tokens) {
                (Some(token), _, _) => serve_retrying(&client, plain(config.auth_header(token))).await.map_err(|e| describe_connect_error(&e.to_string(), url))?,
                (None, Some(oauth @ AuthSpec::Oauth { .. }), Some(stored)) => {
                    let metadata_url = match oauth {
                        AuthSpec::Oauth { auth_server_metadata_url, .. } => auth_server_metadata_url.as_deref(),
                        _ => None,
                    };
                    if stored["native_flow"].as_bool() == Some(true) {
                        let token_endpoint = match oauth {
                            AuthSpec::Oauth { token_endpoint: Some(endpoint), .. } => endpoint,
                            _ => return Err("The saved sign-in cannot be refreshed. Sign in again.".into()),
                        };
                        let refreshed = super::oauth::refresh(&app.http, token_endpoint, &stored).await?;
                        let saved = refreshed.as_ref().unwrap_or(&stored);
                        if let Some(refreshed) = &refreshed { super::set_oauth_at_generation(app, &plugin.manifest.id, name, refreshed.clone(), generation)?; }
                        let token = saved["tokens"]["access_token"].as_str().ok_or("The saved sign-in has no access token")?.to_string();
                        bearer_expires_at = token_expires_at(saved);
                        serve_retrying(&client, plain(config.auth_header(token))).await.map_err(|e| describe_connect_error(&e.to_string(), url))?
                    } else if stored["device_flow"].as_bool() == Some(true) {
                        // GitHub's refresh endpoint has its own contract: no `scope` or MCP
                        // `resource`. Refresh it here, then give the transport a plain bearer.
                        let token_endpoint = match oauth {
                            AuthSpec::Oauth { token_endpoint: Some(endpoint), .. } => endpoint,
                            _ => return Err(format!("{}'s saved sign-in cannot be refreshed. Sign in again.", plugin.manifest.name)),
                        };
                        let bearer = device_bearer(app, &plugin.manifest.id, name, &plugin.manifest.name, token_endpoint, &stored).await?;
                        bearer_expires_at = bearer.expires_at;
                        serve_retrying(&client, plain(config.auth_header(bearer.access_token))).await.map_err(|e| describe_connect_error(&e.to_string(), url))?
                    } else {
                        // A token whose server metadata cannot be found again, or which has no
                        // refresh token, is sent as a plain bearer.
                        let refreshable = stored["tokens"]["refresh_token"].as_str().is_some_and(|r| !r.is_empty());
                        let restored = if refreshable { restore_manager(app, url, &stored, metadata_url).await } else { Err("no refresh token".into()) };
                        match restored {
                            Ok(manager) => {
                                let signed_in = AuthClient::new(http.clone(), manager);
                                auth = Some(signed_in.auth_manager.clone());
                                let transport = || StreamableHttpClientTransport::with_client(signed_in.clone(), config.clone());
                                serve_retrying(&client, transport).await.map_err(|e| describe_connect_error(&e.to_string(), url))?
                            }
                            Err(why) => {
                                tracing::debug!(%why, plugin = %plugin.manifest.id, "using the saved access token as a bearer");
                                let token = stored["tokens"]["access_token"].as_str().ok_or("The saved sign-in has no access token")?.to_string();
                                serve_retrying(&client, plain(config.auth_header(token))).await.map_err(|e| describe_connect_error(&e.to_string(), url))?
                            }
                        }
                    }
                }
                // A server that signs in only when asked is tried as it is first.
                (None, Some(AuthSpec::Oauth { optional: true, .. }), None) => {
                    match serve_retrying(&client, plain(config)).await {
                        Ok(service) => {
                            // A server that once asked for a sign-in and now lets Lorca in no
                            // longer reads Sign in.
                            app.plugins.lock().unwrap().forget_challenge(&app.config, &plugin.manifest.id, name);
                            service
                        }
                        Err(error) => match auth_challenge(&error) {
                            Some(challenge) => {
                                app.plugins.lock().unwrap().note_challenge(&app.config, &plugin.manifest.id, name, &challenge);
                                return Err(format!("{} needs a sign-in.", plugin.manifest.name));
                            }
                            None => return Err(describe_connect_error(&error.to_string(), url)),
                        },
                    }
                }
                (None, Some(AuthSpec::Oauth { .. }), None) => return Err(format!("Sign in to {} first.", plugin.manifest.name)),
                (None, Some(AuthSpec::Bearer { variable }), _) => return Err(format!("Set {variable} first.")),
                (None, None, _) => serve_retrying(&client, plain(config)).await.map_err(|e| describe_connect_error(&e.to_string(), url))?,
            }
        }
    };
    if let Some(manager) = &auth {
        // The handshake itself may have refreshed and rotated the pair. Save it before the
        // next request can fail, the connection can sit unused, or the process can exit.
        persist_refreshed(app, &plugin.manifest.id, name, manager, generation).await;
    }
    let instructions = service.peer_info().and_then(|i| i.instructions.clone());
    let resources = service.peer_info().is_some_and(|info| info.capabilities.resources.is_some());
    // Every tool the server offers: which ones bots see is read as they are listed, so showing a
    // hidden one needs no reconnecting.
    let tools = all_tools(service.peer()).await.map_err(|e| format!("{} could not list its tools: {e}", plugin.manifest.name))?;
    tracing::info!(plugin = %plugin.manifest.id, server = name, tools = tools.len(), resources, "connected an MCP server");
    Ok(Server { plugin_id: plugin.manifest.id.clone(), name: name.to_string(), service, tools: std::sync::RwLock::new(tools), instructions, resources, auth, bearer_expires_at, generation })
}

/// Lorca as an MCP client: its name and version, and what it does when a connected server says
/// its tools changed.
pub struct Client {
    info: ClientConfig,
    app: std::sync::Weak<App>,
    plugin_id: String,
    server: String,
    /// The plugin's generation when the connection began: a connection from before a change of
    /// its settings refreshes nothing.
    generation: u64,
}

impl rmcp::ClientHandler for Client {
    fn get_info(&self) -> ClientConfig {
        self.info.clone()
    }

    async fn on_tool_list_changed(&self, context: rmcp::service::NotificationContext<RoleClient>) {
        let Some(app) = self.app.upgrade() else { return };
        let (plugin_id, server, generation) = (self.plugin_id.clone(), self.server.clone(), self.generation);
        // From a task of its own: the connection answers the listing while this is handled.
        tokio::spawn(async move { refresh_tools(&app, &plugin_id, &server, generation, &context.peer).await });
    }
}

/// Lists a connected server's tools again after it said they changed, for the pooled connection
/// and the next turns' listing.
async fn refresh_tools(app: &Arc<App>, plugin_id: &str, server: &str, generation: u64, peer: &rmcp::service::Peer<RoleClient>) {
    let installed = app.plugins.lock().unwrap().get(plugin_id).is_some();
    if !installed || app.mcp.generation(plugin_id) != generation {
        return;
    }
    let tools = match all_tools(peer).await {
        Ok(tools) => tools,
        Err(error) => {
            tracing::warn!(%error, plugin = plugin_id, server, "listing a server's changed tools");
            return;
        }
    };
    tracing::info!(plugin = plugin_id, server, tools = tools.len(), "an MCP server changed its tools");
    let (instructions, resources) = match app.mcp.cached_server(&format!("{plugin_id}/{server}")) {
        Some(pooled) => {
            *pooled.tools.write().unwrap() = tools.clone();
            (pooled.instructions.clone(), pooled.resources)
        }
        None => match peer.peer_info() {
            Some(info) => (info.instructions.clone(), info.capabilities.resources.is_some()),
            None => (None, false),
        },
    };
    save_catalog(app, plugin_id, server, SavedServer { instructions, tools, resources });
}

/// Pages of a server's tool list one connection reads at most.
const MAX_TOOL_PAGES: usize = 100;

/// Every tool a server lists, page by page. A list ends at a page with no cursor, an empty one,
/// or one the server handed out before: following those would ask for the same page forever.
async fn all_tools(peer: &rmcp::service::Peer<RoleClient>) -> Result<Vec<rmcp::model::Tool>, rmcp::ServiceError> {
    let mut tools = Vec::new();
    let mut seen = std::collections::HashSet::new();
    let mut cursor = None;
    for _ in 0..MAX_TOOL_PAGES {
        let page = peer.list_tools(Some(rmcp::model::PaginatedRequestParams::default().with_cursor(cursor))).await?;
        tools.extend(page.tools);
        match page.next_cursor.filter(|next| !next.is_empty() && seen.insert(next.clone())) {
            Some(next) => cursor = Some(next),
            None => return Ok(tools),
        }
    }
    tracing::warn!(tools = tools.len(), "an MCP server's tool list ran past {MAX_TOOL_PAGES} pages; keeping what it listed");
    Ok(tools)
}

/// What a stdio server writes to stderr: read for as long as it runs, so it never blocks on a
/// full pipe, logged, and its last lines kept for when it stops before the handshake.
struct Stderr {
    lines: Arc<Mutex<std::collections::VecDeque<String>>>,
    reading: Option<tokio::task::JoinHandle<()>>,
}

impl Stderr {
    const KEPT: usize = 40;

    fn follow(stderr: Option<tokio::process::ChildStderr>, plugin_id: &str) -> Stderr {
        let lines = Arc::new(Mutex::new(std::collections::VecDeque::new()));
        let reading = stderr.map(|stderr| {
            let (lines, plugin_id) = (lines.clone(), plugin_id.to_string());
            tokio::spawn(async move {
                use tokio::io::AsyncBufReadExt;
                let mut reader = tokio::io::BufReader::new(stderr);
                let mut line = Vec::new();
                loop {
                    line.clear();
                    match reader.read_until(b'\n', &mut line).await {
                        Ok(0) | Err(_) => break,
                        Ok(_) => {
                            let text = String::from_utf8_lossy(&line).trim_end().to_string();
                            tracing::debug!(plugin = %plugin_id, "{text}");
                            let mut lines = lines.lock().unwrap();
                            if lines.len() == Self::KEPT {
                                lines.pop_front();
                            }
                            lines.push_back(text);
                        }
                    }
                }
            })
        });
        Stderr { lines, reading }
    }

    /// The server's last lines, once it has had a moment to finish writing them.
    async fn last_words(self) -> Option<String> {
        if let Some(reading) = self.reading {
            let _ = tokio::time::timeout(std::time::Duration::from_millis(500), reading).await;
        }
        let lines = self.lines.lock().unwrap();
        last_words(lines.iter().map(String::as_str))
    }
}

/// The last few lines a program wrote before it stopped, on one line, as a plugin's state shows
/// it: blank lines, stack frames, and the caret under a source line left out, and a long tail cut
/// from the front, since the cause usually comes last.
fn last_words<'a>(lines: impl DoubleEndedIterator<Item = &'a str>) -> Option<String> {
    const MAX_CHARS: usize = 400;
    let mut words: Vec<&str> = lines.rev().map(str::trim).filter(|line| !line.starts_with("at ") && !line.trim_start_matches('^').trim().is_empty()).take(4).collect();
    words.reverse();
    let words = words.join(" · ");
    match words.chars().count().checked_sub(MAX_CHARS) {
        None | Some(0) => (!words.is_empty()).then_some(words),
        Some(over) => Some(format!("…{}", words.chars().skip(over + 1).collect::<String>())),
    }
}

/// The handshake with a remote server, tried again after a moment when its failure is one a
/// moment may fix (`transient`): twice, after a quarter of a second and then a second, as pi
/// does. A tool call is never tried again, since the server may have done it.
// The error is rmcp's own, as its `serve` gives it.
#[allow(clippy::result_large_err)]
async fn serve_retrying<T, E, A>(client: &impl Fn() -> Client, transport: impl Fn() -> T) -> Result<RunningService<RoleClient, Client>, rmcp::service::ClientInitializeError>
where
    T: rmcp::transport::IntoTransport<RoleClient, E, A>,
    E: std::error::Error + Send + Sync + 'static,
{
    let mut waits = [250, 1000].into_iter();
    loop {
        match client().serve(transport()).await {
            Err(error) if transient(&error) => match waits.next() {
                Some(wait) => {
                    tracing::info!(%error, "trying a server's handshake again");
                    tokio::time::sleep(std::time::Duration::from_millis(wait)).await;
                }
                None => return Err(error),
            },
            done => return done,
        }
    }
}

/// Whether a failed handshake is the network's or the server's passing trouble (408, 429, or a 5xx
/// other than 501) rather than a refusal.
fn transient(error: &rmcp::service::ClientInitializeError) -> bool {
    use rmcp::transport::streamable_http_client::StreamableHttpError;
    let passing = |status: u16| status == 408 || status == 429 || ((500..600).contains(&status) && status != 501);
    let mut text = error.to_string();
    if let rmcp::service::ClientInitializeError::TransportError { error, .. } = error {
        let mut cause: Option<&(dyn std::error::Error + 'static)> = Some(error.error.as_ref());
        while let Some(current) = cause {
            if let Some(StreamableHttpError::Client(client)) = current.downcast_ref::<StreamableHttpError<mcp_http::Error>>() {
                if client.is_connect() || client.is_timeout() {
                    return true;
                }
                if let Some(status) = client.status() {
                    return passing(status.as_u16());
                }
            }
            text.push('\n');
            text.push_str(&current.to_string());
            cause = current.source();
        }
    }
    // What rmcp says of a status it did not expect, `HTTP 503 Service Unavailable: …`, and a
    // signed-in client's failure to reach the server at all.
    text.split("HTTP ").skip(1).filter_map(|rest| rest.get(..3)?.parse::<u16>().ok()).any(passing) || text.contains("error sending request")
}

/// Why a remote server did not connect, naming it by its origin alone: the state goes into the
/// machine blob, and a URL's path or query can hold a key (Zapier's does).
fn describe_connect_error(error: &str, url: &str) -> String {
    let shown = shown_url(url);
    let error = match reqwest::Url::parse(url) {
        Ok(parsed) => error.replace(parsed.as_str(), &shown).replace(url, &shown),
        Err(_) => error.replace(url, &shown),
    };
    let lower = error.to_lowercase();
    if error.contains("401") || lower.contains("unauthorized") || lower.contains("auth required") {
        format!("{shown} refused the credentials. Sign in again.")
    } else if reqwest::Url::parse(url).is_ok_and(|parsed| parsed.path().trim_end_matches('/').ends_with("/sse")) {
        // The older HTTP+SSE transport, which rmcp's client does not speak.
        format!("Could not connect to {shown}: {error}. Lorca speaks MCP's streamable HTTP; a server that offers it too usually has it at /mcp rather than /sse.")
    } else {
        format!("Could not connect to {shown}: {error}")
    }
}

fn authorization_expired(error: &str) -> bool {
    let error = error.to_lowercase();
    ["invalid_token", "invalid_grant", "token_expired", "token_revoked", "invalid_auth", "not_authed", "bad_refresh_token", "401 unauthorized", "authentication_required", "sign-in expired", "refused the credentials", "another authorization server"].iter().any(|marker| error.contains(marker))
}

/// `https://mcp.example.com` for any URL there.
fn shown_url(url: &str) -> String {
    match reqwest::Url::parse(url) {
        Ok(parsed) => match (parsed.host_str(), parsed.port()) {
            (Some(host), Some(port)) => format!("{}://{host}:{port}", parsed.scheme()),
            (Some(host), None) => format!("{}://{host}", parsed.scheme()),
            _ => "the server".into(),
        },
        Err(_) => "the server".into(),
    }
}

/// The `WWW-Authenticate` challenge of a server that answered the handshake with a 401, or an
/// empty one when it gave none: the server wants a sign-in.
fn auth_challenge(error: &rmcp::service::ClientInitializeError) -> Option<String> {
    use rmcp::transport::streamable_http_client::StreamableHttpError;
    if let rmcp::service::ClientInitializeError::TransportError { error, .. } = error {
        let mut cause: Option<&(dyn std::error::Error + 'static)> = Some(error.error.as_ref());
        while let Some(current) = cause {
            if let Some(http) = current.downcast_ref::<StreamableHttpError<mcp_http::Error>>() {
                if let Some(challenge) = http.auth_challenge() {
                    return Some(challenge.to_string());
                }
                if let StreamableHttpError::Client(client) = http {
                    if client.status().is_some_and(|status| status.as_u16() == 401) {
                        return Some(String::new());
                    }
                }
            }
            cause = current.source();
        }
    }
    // A transport that ended on it says so only in words.
    let text = error.to_string().to_lowercase();
    (text.contains("auth required") || text.contains("401 unauthorized")).then(String::new)
}

/// The values a server's `${VAR}` reads: the plugin's variables and secrets, and for a server
/// from `mcp.json`, the login shell's environment, as Claude's and Cursor's files mean them.
async fn template_values(plugin: &Installed, own: BTreeMap<String, String>) -> BTreeMap<String, String> {
    if plugin.source != super::mcp_json::SOURCE {
        return own;
    }
    let mut values: BTreeMap<String, String> = std::env::vars_os().map(|(name, value)| (name.to_string_lossy().into_owned(), value.to_string_lossy().into_owned())).collect();
    for (name, value) in lorca_agent::login_shell::environment().await {
        values.insert(name.to_string_lossy().into_owned(), value.to_string_lossy().into_owned());
    }
    values.extend(own);
    values
}

/// `~/x` as a path in the home folder.
fn expand_home(path: &str) -> String {
    match (path.strip_prefix("~/").or_else(|| path.strip_prefix("~\\")).or((path == "~").then_some("")), dirs::home_dir()) {
        (Some(rest), Some(home)) => home.join(rest).display().to_string(),
        _ => path.to_string(),
    }
}

/// The authorization server's metadata from the document a server's sign-in names, taken as it
/// is: for a server that advertises the wrong authorization server, or none.
async fn auth_server_metadata(app: &Arc<App>, metadata_url: &str, name: &str) -> Result<rmcp::transport::auth::AuthorizationMetadata, String> {
    let response = app.mcp.http.get(metadata_url).header("accept", "application/json").send().await.map_err(|e| format!("{name}'s authorization server metadata did not load: {e}"))?;
    if !response.status().is_success() {
        return Err(format!("{name}'s authorization server metadata answered {}.", response.status()));
    }
    let body = response.bytes().await.map_err(|e| e.to_string())?;
    serde_json::from_slice(&body).map_err(|e| format!("{name}'s authorization server metadata does not read: {e}"))
}

/// A token response without the optional fields a server sent empty or null, as some send
/// `"scope": ""` for a token with no scope. Kept, an empty scope is asked for again at every
/// refresh.
pub(super) fn tidy_tokens(tokens: &Value) -> Value {
    let mut tokens = tokens.clone();
    if let Some(fields) = tokens.as_object_mut() {
        fields.retain(|key, value| {
            let optional = matches!(key.as_str(), "scope" | "refresh_token" | "id_token" | "expires_in" | "refresh_token_expires_in");
            !(optional && (value.is_null() || value.as_str().is_some_and(|text| text.trim().is_empty())))
        });
    }
    tokens
}

/// An authorization manager holding tokens saved by an earlier sign-in. With `metadata_url`, the
/// authorization server is the one that document describes, never one found by discovery.
async fn restore_manager(app: &Arc<App>, url: &str, stored: &Value, metadata_url: Option<&str>) -> Result<AuthorizationManager, String> {
    let client_id = stored["client_id"].as_str().ok_or("The saved sign-in has no client id")?;
    if let Some(metadata_url) = metadata_url {
        use rmcp::transport::auth::CredentialStore;
        let mut manager = AuthorizationManager::new(url).await.map_err(|e| e.to_string())?;
        manager.with_client(app.mcp.http.clone()).map_err(|e| e.to_string())?;
        manager.set_metadata(auth_server_metadata(app, metadata_url, url).await?);
        let tokens = serde_json::from_value(tidy_tokens(&stored["tokens"])).map_err(|e| format!("The saved sign-in is unreadable: {e}"))?;
        let saved = rmcp::transport::auth::InMemoryCredentialStore::new();
        saved.save(rmcp::transport::auth::StoredCredentials::new(client_id.to_string(), Some(tokens), Vec::new(), None)).await.map_err(|e| e.to_string())?;
        manager.set_credential_store(saved);
        return match manager.initialize_from_store().await.map_err(|e| e.to_string())? {
            true => Ok(manager),
            false => Err("Restoring the sign-in".into()),
        };
    }
    let tokens = serde_json::from_value(tidy_tokens(&stored["tokens"])).map_err(|e| format!("The saved sign-in is unreadable: {e}"))?;
    let mut state = OAuthState::new(url, Some(app.mcp.http.clone())).await.map_err(|e| e.to_string())?;
    state.set_credentials(client_id, tokens).await.map_err(|e| format!("Restoring the sign-in: {e}"))?;
    state.into_authorization_manager().ok_or_else(|| "Restoring the sign-in".to_string())
}

#[derive(Debug)]
struct DeviceBearer {
    access_token: String,
    expires_at: Option<f64>,
}

fn token_expires_at(stored: &Value) -> Option<f64> {
    let received_at = stored["signed_in_at"].as_f64()?;
    let expires_in = stored["tokens"]["expires_in"].as_f64()?;
    (received_at.is_finite() && expires_in.is_finite() && expires_in > 0.0).then_some(received_at + expires_in)
}

/// GitHub normally honors `Accept: application/json`, but its OAuth endpoint's default wire
/// format is form-encoded. Accept both so a successful rotating refresh is never discarded just
/// because the response format changed.
fn parse_token_response(body: &[u8]) -> Option<Value> {
    if let Ok(json) = serde_json::from_slice(body) {
        return Some(json);
    }
    let text = std::str::from_utf8(body).ok()?;
    if !text.contains('=') {
        return None;
    }
    let mut url = reqwest::Url::parse("http://localhost/").ok()?;
    url.set_query(Some(text));
    let mut object = serde_json::Map::new();
    for (key, value) in url.query_pairs() {
        let value = if matches!(key.as_ref(), "expires_in" | "refresh_token_expires_in") {
            value.parse::<u64>().map(|number| json!(number)).unwrap_or_else(|_| json!(value.as_ref()))
        } else {
            json!(value.as_ref())
        };
        object.insert(key.into_owned(), value);
    }
    (!object.is_empty()).then_some(Value::Object(object))
}

/// Return a device-flow bearer, refreshing an expiring one with the provider's device-flow
/// client. The updated value is returned separately so callers can persist a rotated pair before
/// using it.
async fn refresh_device_bearer(http: &reqwest::Client, token_endpoint: &str, name: &str, stored: &Value) -> Result<(DeviceBearer, Option<Value>), String> {
    let access_token = stored["tokens"]["access_token"].as_str().ok_or("The saved sign-in has no access token")?.to_string();
    let expires_at = token_expires_at(stored);
    if !expires_at.is_some_and(|expires_at| expires_at <= now_secs() + DEVICE_TOKEN_REFRESH_BUFFER_SECS) {
        return Ok((DeviceBearer { access_token, expires_at }, None));
    }

    let client_id = stored["client_id"].as_str().ok_or("The saved sign-in has no client id")?;
    let refresh_token = stored["tokens"]["refresh_token"]
        .as_str()
        .filter(|token| !token.is_empty())
        .ok_or_else(|| format!("The {name} sign-in expired. Sign in again."))?;
    let response = http
        .post(token_endpoint)
        .header("accept", "application/json")
        .form(&[("client_id", client_id), ("grant_type", "refresh_token"), ("refresh_token", refresh_token)])
        .send()
        .await
        .map_err(|e| format!("{name} could not refresh its sign-in: {e}"))?;
    let status = response.status();
    let body = response.bytes().await.map_err(|e| format!("{name} sent an unreadable token refresh response: {e}"))?;
    let refreshed = parse_token_response(&body).ok_or_else(|| format!("{name} sent an unreadable token refresh response."))?;
    if !status.is_success() || refreshed["access_token"].as_str().is_none() {
        let reason = refreshed["error_description"].as_str().or(refreshed["error"].as_str()).unwrap_or("the refresh was refused");
        return Err(if refreshed["error"].as_str() == Some("bad_refresh_token") {
            format!("The {name} sign-in expired. Sign in again.")
        } else {
            format!("{name} could not refresh its sign-in: {reason}")
        });
    }

    let mut saved = stored.as_object().cloned().ok_or("The saved sign-in is unreadable")?;
    let mut tokens = stored["tokens"].as_object().cloned().ok_or("The saved sign-in tokens are unreadable")?;
    tokens.insert("access_token".into(), refreshed["access_token"].clone());
    for key in ["token_type", "refresh_token", "scope"] {
        if refreshed.get(key).is_some_and(|value| !value.is_null()) {
            tokens.insert(key.into(), refreshed[key].clone());
        }
    }
    // These durations are relative to this response. An omitted duration means the new token has
    // no advertised expiry; retaining the old duration would invent one from the wrong epoch.
    for key in ["expires_in", "refresh_token_expires_in"] {
        if refreshed.get(key).is_some_and(|value| !value.is_null()) {
            tokens.insert(key.into(), refreshed[key].clone());
        } else {
            tokens.remove(key);
        }
    }
    saved.insert("tokens".into(), Value::Object(tokens));
    saved.insert("signed_in_at".into(), json!(now_secs()));
    let saved = Value::Object(saved);
    let bearer = DeviceBearer {
        access_token: saved["tokens"]["access_token"].as_str().unwrap_or_default().to_string(),
        expires_at: token_expires_at(&saved),
    };
    Ok((bearer, Some(saved)))
}

async fn device_bearer(app: &Arc<App>, plugin_id: &str, server: &str, name: &str, token_endpoint: &str, stored: &Value) -> Result<DeviceBearer, String> {
    let generation = app.mcp.generation(plugin_id);
    let (bearer, refreshed) = refresh_device_bearer(&app.http, token_endpoint, name, stored).await?;
    if let Some(refreshed) = refreshed {
        super::set_oauth_at_generation(app, plugin_id, server, refreshed, generation)?;
    }
    Ok(bearer)
}

// MARK: - Sign-in

/// Signs in to an OAuth server of a plugin on this Runner: the MCP authorization flow with a
/// loopback callback. The browser opens here, or on the Device that asked from `elsewhere`,
/// which sends back where it landed (`finish_sign_in`). Runs in the background; the plugin's
/// state says how it goes and the tokens land in the secrets file.
pub async fn connect_oauth(app: &Arc<App>, plugin_id: &str, server: &str, elsewhere: Option<Elsewhere>) -> Result<SignInStart, String> {
    connect_oauth_for_card(app, plugin_id, server, None, elsewhere).await
}

/// The Device that asked for a sign-in from elsewhere: its name, and the loopback redirect it
/// listens on.
pub struct Elsewhere {
    pub device: String,
    pub redirect_uri: String,
}

/// How a sign-in started: what to tell the user, and for the Device that asked, the page it
/// opens and the id it finishes the sign-in with.
pub struct SignInStart {
    pub message: String,
    pub url: Option<String>,
    pub id: Option<String>,
    /// How a sign-in that finishes here ended, once it has, for a caller that waits on it.
    pub done: Option<tokio::sync::oneshot::Receiver<Result<(), String>>>,
}

/// A browser sign-in another Device finishes: the authorization this Runner holds until that
/// Device sends back where the browser landed, and what to update when it ends.
struct Pending {
    state: BrowserState,
    server: String,
    name: String,
    card: Option<(String, String)>,
    generation: u64,
}

enum BrowserState {
    Mcp(OAuthState),
    Native(super::oauth::Pending),
}

/// The OAuth server of a plugin, when it has one.
pub fn oauth_server(app: &App, plugin_id: &str) -> Option<String> {
    let store = app.plugins.lock().unwrap();
    let plugin = store.get(plugin_id)?;
    plugin
        .manifest
        .servers
        .iter()
        .find(|(_, spec)| matches!(spec, ServerSpec::Http { auth: Some(AuthSpec::Oauth { .. }), .. }))
        .map(|(name, _)| name.clone())
}

/// Posts a sign-in card in the chat, as Grok Bot does: "Sign in" on it starts the OAuth flow
/// on the Runner and the card says how it went. Nothing waits on it. A chat holds one waiting
/// card per plugin and Runner: asked again before the user writes, this answers with the card
/// already up; asked after, the new card replaces the old one, which reads dismissed.
pub fn post_sign_in_card(app: &Arc<App>, chat_id: &str, bot_id: &str, plugin_id: &str) -> Result<Message, String> {
    let (name, state) = {
        let store = app.plugins.lock().unwrap();
        let status = store.status(plugin_id).ok_or_else(|| format!("{plugin_id} is not installed on this Runner"))?;
        (status.name, status.state)
    };
    oauth_server(app, plugin_id).ok_or_else(|| format!("{name} has nothing to sign in to."))?;
    if state == "ready" {
        return Err(format!("{name} is already signed in."));
    }
    let runner_of = |bot_id: &str| app.bot(bot_id).map(|bot| bot.runner_id);
    let here = runner_of(bot_id);
    let mut waiting: Vec<(Message, bool)> = app
        .store
        .sign_in_cards(Some(chat_id), plugin_id)
        .unwrap_or_else(|error| {
            tracing::error!(%error, %chat_id, "reading sign-in cards");
            Vec::new()
        })
        .into_iter()
        .filter(|(card, _)| matches!(&card.body, Body::Permission { decision, .. } if decision == "pending"))
        .filter(|(card, _)| matches!(&card.author, Author::Bot { bot_id } if runner_of(bot_id) == here))
        .collect();
    let current = match waiting.last() {
        Some((_, true)) => waiting.pop().map(|(card, _)| card),
        _ => None,
    };
    for (card, _) in &waiting {
        set_card(app, chat_id, &card.id, "dismissed", None, None, None);
    }
    if let Some(card) = current {
        return Ok(card);
    }
    let runner = app.this_device_id().and_then(|id| app.device(&id)).map(|d| d.name).unwrap_or_else(|| "its Runner".into());
    let message = Message::new(
        chat_id,
        Author::Bot { bot_id: bot_id.to_string() },
        Body::Permission {
            plugin_id: plugin_id.to_string(),
            plugin_name: name.clone(),
            tool: "connect".into(),
            summary: format!("Sign in to {name} on {runner}."),
            arguments: Value::Null,
            decision: "pending".into(),
            reason: None,
            rule: None,
            command: None,
            link: None,
            code: None,
        },
    );
    app.upsert_message(message.clone(), true);
    crate::push::permission(app, &message);
    Ok(message)
}

/// Marks a card with how something went, and with a link and code while a device flow waits.
fn set_card(app: &Arc<App>, chat_id: &str, message_id: &str, decision: &str, summary: Option<String>, link: Option<String>, code: Option<String>) {
    if let Some(mut message) = app.message(chat_id, message_id) {
        if let Body::Permission { decision: d, summary: s, link: l, code: c, .. } = &mut message.body {
            *d = decision.to_string();
            if let Some(summary) = summary {
                *s = summary;
            }
            *l = link;
            *c = code;
        }
        app.upsert_message(message, true);
    }
}

/// `connect_oauth` with a chat card (chat id, message id) to update as the sign-in goes.
pub async fn connect_oauth_for_card(app: &Arc<App>, plugin_id: &str, server: &str, card: Option<(String, String)>, elsewhere: Option<Elsewhere>) -> Result<SignInStart, String> {
    let (plugin, spec) = {
        let store = app.plugins.lock().unwrap();
        let plugin = store.get(plugin_id).cloned().ok_or("Unknown plugin")?;
        let spec = plugin.manifest.servers.get(server).cloned().ok_or_else(|| format!("{} has no server {server}", plugin.manifest.name))?;
        (plugin, spec)
    };
    let own = app.plugins.lock().unwrap().values(plugin_id);
    let values = template_values(&plugin, own).await;
    let (url, scopes, client, device) = match &spec {
        ServerSpec::Http {
            url,
            auth:
                Some(AuthSpec::Oauth {
                    scopes,
                    token_variable,
                    client_id_variable,
                    client_secret_variable,
                    client_id,
                    client_secret,
                    device_authorization_endpoint,
                    token_endpoint,
                    authorization_endpoint,
                    authorization_params,
                    client_name,
                    callback_port,
                    callback_url,
                    auth_server_metadata_url,
                    ..
                }),
            ..
        } => {
            let url = fill(url, &values);
            // A fixed value may name the environment (`${CLIENT_SECRET}`), as an mcp.json entry's does.
            let set = |fixed: &Option<String>, variable: &Option<String>| {
                fixed.as_deref().map(|value| fill(value, &values)).filter(|v| !v.trim().is_empty()).or_else(|| variable.as_ref().and_then(|v| values.get(v).cloned())).filter(|v| !v.trim().is_empty())
            };
            let hint = ClientHint {
                id: set(client_id, client_id_variable),
                secret: set(client_secret, client_secret_variable),
                token_variable: token_variable.clone(),
                client_id_variable: client_id_variable.clone(),
                client_secret_variable: client_secret_variable.clone(),
                name: client_name.clone(),
                callback_port: *callback_port,
                callback_url: callback_url.clone(),
                metadata_url: auth_server_metadata_url.clone(),
                authorization_endpoint: authorization_endpoint.clone(),
                token_endpoint: token_endpoint.clone(),
                authorization_params: authorization_params.clone(),
            };
            let device = match (device_authorization_endpoint, token_endpoint, &hint.id) {
                (Some(device_endpoint), Some(token_endpoint), Some(_)) => Some((device_endpoint.clone(), token_endpoint.clone())),
                _ => None,
            };
            // What a server asked for more of since (`insufficient_scope`) is asked for too.
            let more = app.plugins.lock().unwrap().secret(plugin_id, &format!("scope:{server}"));
            let mut scopes = scopes.clone();
            for scope in more.as_ref().and_then(|more| more["scope"].as_str()).unwrap_or_default().split_whitespace() {
                if !scopes.iter().any(|known| known == scope) {
                    scopes.push(scope.to_string());
                }
            }
            (url, scopes, hint, device)
        }
        _ => return Err(format!("{server} does not sign in with OAuth.")),
    };
    // A redirect the client was registered with is this Runner's own, so its browser opens here.
    if client.authorization_endpoint.is_some() && elsewhere.as_ref().is_some_and(|device| client.has_fixed_redirect() && !client.matches_redirect(&device.redirect_uri)) {
        return Err("The sign-in needs the registered loopback callback on this Device. Check the integration's callback settings.".into());
    }
    let elsewhere = elsewhere.filter(|device| !client.has_fixed_redirect() || client.matches_redirect(&device.redirect_uri));
    let name = plugin.display_name();
    // A newer consent attempt supersedes an older one for this account. Other accounts keep
    // their generations and tokens.
    app.mcp.forget(plugin_id);
    let generation = app.mcp.generation(plugin_id);
    // A device code works on any Device, so it is the sign-in wherever the user asked.
    let opens_on = match (&device, &elsewhere) {
        (Some(_), _) => None,
        (None, Some(elsewhere)) => Some(elsewhere.device.clone()),
        (None, None) => Some(this_runner(app)),
    };
    match &opens_on {
        Some(device) => super::note(app, plugin_id, Some(("connecting", &format!("Finish signing in in the browser on {device}")))),
        None => super::note(app, plugin_id, Some(("connecting", "Getting a sign-in code…"))),
    }
    if let Some((chat_id, message_id)) = &card {
        let summary = opens_on.as_ref().map(|device| format!("Finish signing in in the browser on {device}.")).unwrap_or_else(|| "Getting a code…".into());
        set_card(app, chat_id, message_id, "allowed", Some(summary), None, None);
    }
    if let (Some(elsewhere), None) = (elsewhere, &device) {
        return match begin_sign_in(app, &url, &scopes, &name, &client, elsewhere.redirect_uri).await {
            Ok((state, page)) => {
                let id = hold_sign_in(app, plugin_id, Pending { state, server: server.to_string(), name: name.clone(), card, generation });
                Ok(SignInStart { message: format!("Open the {name} sign-in page on {}.", elsewhere.device), url: Some(page), id: Some(id), done: None })
            }
            Err(error) => {
                let _ = end_sign_in(app, plugin_id, server, &name, card, generation, Err(error.clone()));
                Err(error)
            }
        };
    }
    let message = if device.is_some() { format!("Getting a {name} sign-in code.") } else { format!("Opened the {name} sign-in page in the browser on this Runner.") };
    let (app, plugin_id, server) = (app.clone(), plugin_id.to_string(), server.to_string());
    let (ended, done) = tokio::sync::oneshot::channel();
    tokio::spawn(async move {
        let flow = match &device {
            Some((device_endpoint, token_endpoint)) => device_sign_in(&app, &plugin_id, &server, device_endpoint, token_endpoint, &scopes, &name, client.id.as_deref().unwrap_or(""), card.as_ref()).await,
            None => sign_in(&app, &url, &scopes, &name, &client).await,
        };
        let outcome = end_sign_in(&app, &plugin_id, &server, &name, card, generation, flow);
        let _ = ended.send(outcome);
    });
    Ok(SignInStart { message, url: None, id: None, done: Some(done) })
}

fn this_runner(app: &App) -> String {
    app.this_device_id().and_then(|id| app.device(&id)).map(|d| d.name).unwrap_or_else(|| "this Runner".into())
}

/// Keeps a sign-in for the Device that opened its page, in place of an older one of the
/// plugin, whose card asks again, and ends it as timed out if that Device never comes back.
/// Answers with its id.
fn hold_sign_in(app: &Arc<App>, plugin_id: &str, pending: Pending) -> String {
    let id = uuid::Uuid::new_v4().to_string();
    let replaced = app.mcp.sign_ins.lock().unwrap().insert(plugin_id.to_string(), (id.clone(), pending));
    if let Some((_, Pending { name, card: Some((chat_id, message_id)), .. })) = replaced {
        set_card_while(app, &chat_id, &message_id, "allowed", "pending", format!("Sign in to {name} on {}.", this_runner(app)));
    }
    let (app, plugin_id, held) = (app.clone(), plugin_id.to_string(), id.clone());
    tokio::spawn(async move {
        tokio::time::sleep(super::sign_in::TIMEOUT).await;
        if let Some(pending) = take_sign_in(&app, &plugin_id, &held) {
            let _ = end_sign_in(&app, &plugin_id, &pending.server, &pending.name, pending.card, pending.generation, Err("Timed out waiting for the browser".into()));
        }
    });
    id
}

/// The held sign-in `id` of a plugin, unless it ended or a newer one replaced it.
fn take_sign_in(app: &App, plugin_id: &str, id: &str) -> Option<Pending> {
    let mut held = app.mcp.sign_ins.lock().unwrap();
    if held.get(plugin_id).is_some_and(|(held_id, _)| held_id == id) { held.remove(plugin_id).map(|(_, pending)| pending) } else { None }
}

/// The Device that opened the page sent back where the browser landed: its code becomes the
/// tokens here.
pub async fn finish_sign_in(app: &Arc<App>, plugin_id: &str, id: &str, callback: &str) -> Result<Value, String> {
    let pending = take_sign_in(app, plugin_id, id).ok_or("That sign-in is over. Start it again.")?;
    if app.mcp.generation(plugin_id) != pending.generation { return Err("That account's settings changed. Start the sign-in again.".into()); }
    let flow = complete_sign_in(app, pending.state, callback, &pending.name).await;
    end_sign_in(app, plugin_id, &pending.server, &pending.name, pending.card, pending.generation, flow)?;
    Ok(json!({ "signed_in": true }))
}

/// The page closed before the sign-in finished: the plugin waits for a sign-in again, and its
/// card offers Sign in again.
pub fn cancel_sign_in(app: &Arc<App>, plugin_id: &str, id: &str) -> Result<Value, String> {
    let Some(pending) = take_sign_in(app, plugin_id, id) else { return Ok(Value::Null) };
    super::note(app, plugin_id, None);
    if let Some((chat_id, message_id)) = pending.card {
        let summary = format!("Sign in to {} on {}.", pending.name, this_runner(app));
        set_card_while(app, &chat_id, &message_id, "allowed", "pending", summary);
    }
    Ok(Value::Null)
}

/// Ends a sign-in: the tokens saved and every card that asks for the plugin's sign-in reads
/// Signed in, or the plugin and the card say why it failed. A failure leaves alone a plugin
/// another sign-in got ready, and a card that already says how it went.
fn end_sign_in(app: &Arc<App>, plugin_id: &str, server: &str, name: &str, card: Option<(String, String)>, generation: u64, flow: Result<Value, String>) -> Result<(), String> {
    if app.mcp.generation(plugin_id) != generation {
        if let Some((chat_id, message_id)) = card {
            set_card_while(app, &chat_id, &message_id, "allowed", "dismissed", "The account changed while the sign-in was open. Start it again.".into());
        }
        return Err("The account changed while the sign-in was open. Start it again.".into());
    }
    match flow.and_then(|saved| super::set_oauth_at_generation(app, plugin_id, server, saved, generation)) {
        Ok(()) => {
            app.mcp.forget(plugin_id);
            super::note(app, plugin_id, None);
            prefetch_tools(app, plugin_id);
            if let Some((chat_id, message_id)) = card {
                set_card(app, &chat_id, &message_id, "connected", Some(format!("Signed in to {name}.")), None, None);
            }
            settle_sign_in_cards(app, plugin_id, name);
            Ok(())
        }
        Err(error) => {
            if app.mcp.generation(plugin_id) != generation { return Err(error); }
            if error.contains("sign-in needs more access") {
                needs_more_access(app, plugin_id, server, "", "");
            } else if !is_ready(app, plugin_id) {
                super::note(app, plugin_id, Some(("error", &error)));
            }
            if let Some((chat_id, message_id)) = card {
                set_card_while(app, &chat_id, &message_id, "allowed", "failed", format!("Sign-in failed: {error}"));
            }
            Err(error)
        }
    }
}

fn is_ready(app: &App, plugin_id: &str) -> bool {
    app.plugins.lock().unwrap().status(plugin_id).is_some_and(|status| status.state == "ready")
}

/// The plugin is signed in, whichever card or sheet did it: every card of a bot here that asks
/// for its sign-in, or follows one, reads Signed in.
pub fn settle_sign_in_cards(app: &Arc<App>, plugin_id: &str, name: &str) {
    let here = app.this_device_id();
    let cards = app.store.sign_in_cards(None, plugin_id).unwrap_or_else(|error| {
        tracing::error!(%error, plugin = %plugin_id, "reading sign-in cards");
        Vec::new()
    });
    for (card, _) in cards {
        let Body::Permission { decision, .. } = &card.body else { continue };
        let ours = matches!(&card.author, Author::Bot { bot_id } if app.bot(bot_id).map(|bot| bot.runner_id) == here);
        if ours && (decision == "pending" || decision == "allowed") {
            set_card(app, &card.chat_id, &card.id, "connected", Some(format!("Signed in to {name}.")), None, None);
        }
    }
}

/// `set_card` for a card that still reads `expected`, with no link or code.
fn set_card_while(app: &Arc<App>, chat_id: &str, message_id: &str, expected: &str, decision: &str, summary: String) {
    let current = app.message(chat_id, message_id).and_then(|message| match message.body {
        Body::Permission { decision, .. } => Some(decision),
        _ => None,
    });
    if current.as_deref() == Some(expected) {
        set_card(app, chat_id, message_id, decision, Some(summary), None, None);
    }
}

/// How long a device flow waits for the user to enter the code.
const DEVICE_FLOW_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(15 * 60);

/// The device flow (RFC 8628), as GitHub's own CLI signs in: ask for a code, show it with the
/// link on the card and in the plugin's detail, poll the token endpoint until the user has
/// entered it. Answers with the tokens in the shape the browser flow saves.
#[allow(clippy::too_many_arguments)]
async fn device_sign_in(app: &Arc<App>, plugin_id: &str, server: &str, device_endpoint: &str, token_endpoint: &str, scopes: &[String], name: &str, client_id: &str, card: Option<&(String, String)>) -> Result<Value, String> {
    // The App's client: plain form posts, no MCP transport involved.
    let http = app.http.clone();
    let scope = scopes.join(" ");
    let started: Value = http
        .post(device_endpoint)
        .header("accept", "application/json")
        .form(&[("client_id", client_id), ("scope", &scope)])
        .send()
        .await
        .map_err(|e| format!("{name} did not answer the code request: {e}"))?
        .json()
        .await
        .map_err(|e| format!("{name} sent an unreadable code response: {e}"))?;
    let device_code = started["device_code"].as_str().ok_or_else(|| format!("{name} refused the sign-in: {}", started["error_description"].as_str().or(started["error"].as_str()).unwrap_or("no device code")))?.to_string();
    let user_code = started["user_code"].as_str().unwrap_or("").to_string();
    let link = started["verification_uri_complete"].as_str().or(started["verification_uri"].as_str()).unwrap_or("").to_string();
    let shown = started["verification_uri"].as_str().unwrap_or(&link).to_string();
    let interval = started["interval"].as_u64().unwrap_or(5).max(1);
    if let Some((chat_id, message_id)) = card {
        set_card(app, chat_id, message_id, "allowed", Some(format!("Enter the code {user_code} at {shown}.")), Some(link.clone()), Some(user_code.clone()));
    }
    // The note's new words move the machine blob, so a plugin sheet open on any Device
    // reloads the detail that now carries the code.
    let host = reqwest::Url::parse(&shown).ok().and_then(|url| url.host_str().map(str::to_string)).unwrap_or_else(|| shown.clone());
    app.plugins.lock().unwrap().codes.insert(plugin_id.to_string(), super::SignInCode { server: server.to_string(), code: user_code.clone(), link: link.clone() });
    super::note(app, plugin_id, Some(("connecting", &format!("Enter the code at {host}"))));
    let polled = poll_device_token(app, token_endpoint, name, client_id, &device_code, interval, started["expires_in"].as_u64()).await;
    // Spent either way; a newer flow's code stays.
    let mut store = app.plugins.lock().unwrap();
    if store.codes.get(plugin_id).is_some_and(|waiting| waiting.code == user_code) {
        store.codes.remove(plugin_id);
    }
    polled
}

/// Polls the token endpoint until the user has entered the code, it expires, or the sign-in
/// is refused.
async fn poll_device_token(app: &Arc<App>, token_endpoint: &str, name: &str, client_id: &str, device_code: &str, interval: u64, expires_in: Option<u64>) -> Result<Value, String> {
    let http = app.http.clone();
    let deadline = std::time::Instant::now() + DEVICE_FLOW_TIMEOUT.min(std::time::Duration::from_secs(expires_in.unwrap_or(900)));
    let mut wait = interval;
    loop {
        tokio::time::sleep(std::time::Duration::from_secs(wait)).await;
        if std::time::Instant::now() > deadline {
            return Err("The code expired before it was entered.".into());
        }
        let polled: Value = http
            .post(token_endpoint)
            .header("accept", "application/json")
            .form(&[("client_id", client_id), ("device_code", device_code), ("grant_type", "urn:ietf:params:oauth:grant-type:device_code")])
            .send()
            .await
            .map_err(|e| format!("{name} did not answer: {e}"))?
            .json()
            .await
            .map_err(|e| format!("{name} sent an unreadable token response: {e}"))?;
        if let Some(access_token) = polled["access_token"].as_str() {
            let tokens = json!({
                "access_token": access_token,
                "token_type": polled["token_type"].as_str().unwrap_or("bearer"),
                "refresh_token": polled["refresh_token"],
                "expires_in": polled["expires_in"],
                "refresh_token_expires_in": polled["refresh_token_expires_in"],
                "scope": polled["scope"],
            });
            return Ok(json!({ "client_id": client_id, "tokens": tokens, "signed_in_at": now_secs(), "device_flow": true }));
        }
        match polled["error"].as_str().unwrap_or("") {
            "authorization_pending" => {}
            "slow_down" => wait += 5,
            "expired_token" => return Err("The code expired before it was entered.".into()),
            "access_denied" => return Err("The sign-in was denied.".into()),
            other => return Err(format!("{name} refused the sign-in: {}", polled["error_description"].as_str().unwrap_or(other))),
        }
    }
}

/// An answer to a sign-in card: Sign in starts the flow and the card follows it, in this
/// Runner's browser or on the Device that asked from `elsewhere`; Not now closes the card. A
/// plugin signed in meanwhile, from another card or its sheet, reads Signed in at once.
/// `answered` is false when the card was not waiting; `url` is the page the Device that asked
/// opens, and `sign_in` the id it finishes the sign-in with.
pub async fn answer_sign_in(app: &Arc<App>, chat_id: &str, message_id: &str, decision: Decision, elsewhere: Option<Elsewhere>) -> Result<Value, String> {
    let Some(message) = app.message(chat_id, message_id) else { return Ok(json!({ "answered": false })) };
    let Body::Permission { plugin_id, plugin_name, tool, decision: current, .. } = &message.body else { return Ok(json!({ "answered": false })) };
    if tool != "connect" || current != "pending" {
        return Ok(json!({ "answered": false }));
    }
    match decision {
        Decision::Allowed | Decision::Always if is_ready(app, plugin_id) => {
            set_card(app, chat_id, message_id, "connected", Some(format!("Signed in to {plugin_name}.")), None, None);
            Ok(json!({ "answered": true }))
        }
        Decision::Allowed | Decision::Always => {
            let server = oauth_server(app, plugin_id).ok_or("Nothing to sign in to")?;
            let card = Some((chat_id.to_string(), message_id.to_string()));
            let started = connect_oauth_for_card(app, plugin_id, &server, card, elsewhere).await?;
            Ok(json!({ "answered": true, "url": started.url, "sign_in": started.id }))
        }
        Decision::Denied | Decision::Expired | Decision::Dismissed => {
            set_card(app, chat_id, message_id, "denied", None, None, None);
            Ok(json!({ "answered": true }))
        }
    }
}

/// What the manifest and the user supplied for a server that needs a preregistered client.
struct ClientHint {
    id: Option<String>,
    secret: Option<String>,
    token_variable: Option<String>,
    client_id_variable: Option<String>,
    client_secret_variable: Option<String>,
    /// The name to register under; Lorca when unset.
    name: Option<String>,
    /// The redirect a preregistered client was registered with.
    callback_port: Option<u16>,
    callback_url: Option<String>,
    /// The authorization server's metadata document, in place of discovery.
    metadata_url: Option<String>,
    authorization_endpoint: Option<String>,
    token_endpoint: Option<String>,
    authorization_params: BTreeMap<String, String>,
}

impl ClientHint {
    /// A redirect the client was registered with, which only this Runner's own loopback can take.
    fn has_fixed_redirect(&self) -> bool {
        self.callback_port.is_some() || self.callback_url.is_some()
    }

    fn matches_redirect(&self, redirect: &str) -> bool {
        let Ok(url) = reqwest::Url::parse(redirect) else { return false };
        if let Some(fixed) = &self.callback_url {
            let Ok(expected) = reqwest::Url::parse(fixed) else { return false };
            return url.scheme() == expected.scheme() && url.host_str() == expected.host_str() && url.path() == expected.path()
                && expected.port().or(self.callback_port).is_none_or(|port| url.port() == Some(port));
        }
        self.callback_port.is_none_or(|port| url.port() == Some(port))
    }
}

impl ClientHint {
    /// What to tell the user when the server registers no clients on the fly.
    fn no_registration_advice(&self, name: &str) -> String {
        let mut ways = Vec::new();
        if let Some(token) = &self.token_variable {
            ways.push(format!("paste a personal access token as {token} in the plugin's setup"));
        }
        if let Some(id) = &self.client_id_variable {
            let secret = self.client_secret_variable.as_deref().map(|s| format!(" and {s}")).unwrap_or_default();
            ways.push(format!("create an OAuth app at {name} with the callback http://127.0.0.1 and paste its client id{secret} as {id}{secret} in the plugin's setup"));
        }
        if ways.is_empty() {
            format!("{name} does not register apps on the fly and the plugin names no client id.")
        } else {
            format!("{name} does not register apps on the fly. To sign in, {}.", ways.join(", or "))
        }
    }
}

/// The server's own `WWW-Authenticate` challenge, which names its resource metadata (GitHub
/// keeps it under the server's path, where a blind probe never looks).
async fn challenge_of(app: &Arc<App>, url: &str) -> Option<String> {
    let probe = json!({ "jsonrpc": "2.0", "id": 0, "method": "initialize", "params": { "protocolVersion": "2025-06-18", "capabilities": {}, "clientInfo": { "name": "Lorca", "version": crate::config::VERSION } } });
    let response = app.mcp.http.post(url).header("accept", "application/json, text/event-stream").json(&probe).send().await.ok()?;
    if response.status().as_u16() != 401 {
        return None;
    }
    response.headers().get("www-authenticate").and_then(|v| v.to_str().ok()).map(str::to_string)
}

/// Starts the authorization against a server: discovery from its own challenge, the client
/// (registered on the fly as a native app, or the preregistered one), PKCE. Answers with what
/// finishes it and the page to open, for the browser to come back to `redirect`. The server
/// has `sign_in::SETUP_TIMEOUT` for all of it, so a Device waiting on the start hears how it went.
async fn begin_sign_in(app: &Arc<App>, url: &str, scopes: &[String], name: &str, client: &ClientHint, redirect: String) -> Result<(BrowserState, String), String> {
    if let Some(endpoint) = &client.authorization_endpoint {
        let id = client.id.as_deref().ok_or_else(|| client.no_registration_advice(name))?;
        let token = client.token_endpoint.as_deref().ok_or("The integration has no token endpoint.")?;
        let (pending, page) = super::oauth::Pending::begin(endpoint, token, id, client.secret.as_deref(), &redirect, scopes, &client.authorization_params)?;
        return Ok((BrowserState::Native(pending), page));
    }
    let setup = async {
        let mut state = OAuthState::new(url, Some(app.mcp.http.clone())).await.map_err(|e| format!("{name}: {e}"))?;
        let client_name = client.name.as_deref().unwrap_or("Lorca");
        let mut request = AuthorizationRequest::new(redirect).with_scopes(scopes.iter().cloned()).with_client_name(client_name).with_application_type("native");
        if let Some(challenge) = challenge_of(app, url).await {
            request = request.with_challenge(challenge);
        }
        if let Some(id) = &client.id {
            request = request.with_preregistered_client(id.clone());
            if let Some(secret) = &client.secret {
                request = request.with_client_secret(secret.clone());
            }
        }
        let refused = |e: rmcp::transport::auth::AuthError| match e {
            rmcp::transport::auth::AuthError::RegistrationFailed(_) => client.no_registration_advice(name),
            other => format!("{name} does not offer a sign-in: {other}"),
        };
        match &client.metadata_url {
            // The document the entry names, taken as it is, in place of discovery.
            Some(metadata_url) => {
                let metadata = auth_server_metadata(app, metadata_url, name).await?;
                let OAuthState::Unauthorized(mut manager) = state else { return Err(format!("{name}'s sign-in has already begun.")) };
                manager.set_metadata(metadata);
                state = OAuthState::Session(rmcp::transport::auth::AuthorizationSession::new(manager, request).await.map_err(|(_, e)| refused(e))?);
            }
            None => state.start_authorization(request).await.map_err(refused)?,
        }
        let page = state.get_authorization_url().await.map_err(|e| e.to_string())?;
        Ok((BrowserState::Mcp(state), page))
    };
    tokio::time::timeout(super::sign_in::SETUP_TIMEOUT, setup).await.map_err(|_| format!("{name} did not answer the sign-in in time."))?
}

/// Finishes an authorization with where the browser landed: its code for the tokens.
async fn complete_sign_in(app: &Arc<App>, state: BrowserState, callback: &str, name: &str) -> Result<Value, String> {
    let mut state = match state {
        BrowserState::Native(pending) => return pending.finish(&app.http, callback).await,
        BrowserState::Mcp(state) => state,
    };
    if super::sign_in::denied(callback) {
        return Err("The sign-in was denied.".into());
    }
    state.handle_callback_url(callback).await.map_err(|e| format!("{name} rejected the sign-in: {e}"))?;
    let (client_id, tokens) = state.get_credentials().await.map_err(|e| e.to_string())?;
    let tokens = tokens.ok_or("The sign-in produced no tokens")?;
    Ok(json!({ "client_id": client_id, "tokens": tidy_tokens(&json!(tokens)), "signed_in_at": now_secs() }))
}

/// A sign-in in this Runner's own browser, back to its own loopback.
async fn sign_in(app: &Arc<App>, url: &str, scopes: &[String], name: &str, client: &ClientHint) -> Result<Value, String> {
    let callback = if client.has_fixed_redirect() {
        super::sign_in::Callback::bind_fixed(client.callback_port, client.callback_url.as_deref()).await?
    } else {
        super::sign_in::Callback::bind().await?
    };
    let (state, page) = begin_sign_in(app, url, scopes, name, client, callback.redirect_uri()).await?;
    open_browser(app, &page)?;
    let landed = callback.wait(name, super::sign_in::TIMEOUT).await?;
    complete_sign_in(app, state, &landed, name).await
}

/// Opens a sign-in page in this computer's browser. `LORCA_OAUTH_NO_BROWSER=1` fetches it
/// instead, for tests against a fake server that redirects straight to the callback.
pub fn open_browser(app: &Arc<App>, url: &str) -> Result<(), String> {
    if std::env::var("LORCA_OAUTH_NO_BROWSER").ok().as_deref() == Some("1") {
        let (http, url) = (app.mcp.http.clone(), url.to_string());
        tokio::spawn(async move {
            if let Err(error) = http.get(&url).send().await {
                tracing::warn!(%error, "fetching the sign-in page");
            }
        });
        return Ok(());
    }
    open::that(url).map_err(|e| format!("Cannot open the browser: {e}"))
}

// MARK: - Tools for a turn

/// The name a script calls a plugin tool by, a JavaScript identifier: `github__create_issue`,
/// `google_drive__search` for the `google-drive` plugin.
pub fn tool_name(plugin_id: &str, tool: &str) -> String {
    let raw = format!("{plugin_id}__{tool}");
    let cleaned: String = raw.chars().map(|c| if c.is_ascii_alphanumeric() || c == '_' { c } else { '_' }).collect();
    cleaned.chars().take(64).collect()
}

/// What a plugin's servers offered the last time each connected, in `catalog.json` in the
/// plugin's folder: a turn lists the tools from it without starting a server.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
struct SavedCatalog {
    #[serde(default)]
    servers: BTreeMap<String, SavedServer>,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
struct SavedServer {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    instructions: Option<String>,
    #[serde(default)]
    tools: Vec<rmcp::model::Tool>,
    /// Whether it offers resources, so a turn lists its resource tools too.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    resources: bool,
}

/// What a plugin server offered when it last connected: its name, its instructions, its tools,
/// and whether it offers resources.
type Offered = (String, Option<String>, Vec<rmcp::model::Tool>, bool);

fn catalog_path(app: &App, plugin_id: &str) -> std::path::PathBuf {
    app.config.plugins_dir().join(plugin_id).join("catalog.json")
}

/// Keeps what a server just offered, for the next turns' listings.
fn save_catalog(app: &App, plugin_id: &str, server: &str, fresh: SavedServer) {
    let path = catalog_path(app, plugin_id);
    let mut saved: SavedCatalog = crate::config::read_json(&path).unwrap_or_default();
    if saved.servers.get(server) != Some(&fresh) {
        saved.servers.insert(server.to_string(), fresh);
        if let Err(error) = crate::config::write_json_private(&path, &saved) {
            tracing::warn!(%error, plugin = %plugin_id, "saving a plugin's tool list");
        }
    }
}

/// Connects a plugin that is ready and has no saved tool list, once, in the background, so the
/// next turn lists its tools: after an install, a setup, or a sign-in.
pub fn prefetch_tools(app: &Arc<App>, plugin_id: &str) {
    let Ok(runtime) = tokio::runtime::Handle::try_current() else { return };
    let ready = app.plugins.lock().unwrap().status(plugin_id).is_some_and(|status| status.state == "ready");
    if !ready || catalog_path(app, plugin_id).exists() {
        return;
    }
    let (app, plugin_id) = (app.clone(), plugin_id.to_string());
    runtime.spawn(async move {
        app.mcp.servers_of(&app, &plugin_id).await;
    });
}

/// The saved tool lists of a plugin's current servers, every tool they offered.
fn offered(app: &App, plugin: &Installed) -> Vec<Offered> {
    let saved: SavedCatalog = crate::config::read_json(&catalog_path(app, &plugin.manifest.id)).unwrap_or_default();
    saved.servers.into_iter().filter(|(name, _)| plugin.manifest.servers.contains_key(name)).map(|(name, server)| (name, server.instructions, server.tools, server.resources)).collect()
}

/// The saved tool lists of a plugin's current servers, without the tools it hides.
fn saved_servers(app: &App, plugin: &Installed) -> Vec<Offered> {
    offered(app, plugin)
        .into_iter()
        .map(|(name, instructions, tools, resources)| (name, instructions, tools.into_iter().filter(|tool| !plugin.manifest.tools.hides(&tool.name)).collect(), resources))
        .collect()
}

/// The tools a plugin's servers offered when they last connected, for the apps' MCP server sheet
/// and `lorca mcp get`: each one's name, the first line of what it does, whether its server
/// marked it read-only then, and whether it is hidden from bots, so a hidden one can be shown again.
pub fn saved_tools(app: &App, plugin: &Installed) -> Vec<Value> {
    offered(app, plugin)
        .into_iter()
        .flat_map(|(_, _, tools, _)| tools)
        .map(|tool| {
            let about = tool.description.as_deref().and_then(|text| text.lines().map(str::trim).find(|line| !line.is_empty())).unwrap_or_default();
            json!({
                "name": tool.name,
                "title": tool.title.clone().or_else(|| tool.annotations.as_ref().and_then(|a| a.title.clone())),
                "description": utf8_prefix(about, 300),
                "read_only": tool.annotations.as_ref().and_then(|a| a.read_only_hint).unwrap_or(false),
                "capability": if tool.annotations.as_ref().and_then(|a| a.read_only_hint).unwrap_or(false)
                    || plugin.manifest.tools.readonly.iter().any(|pattern| pattern_matches(pattern, &tool.name)) { "read" }
                    else if plugin.manifest.tools.draft.iter().any(|pattern| pattern_matches(pattern, &tool.name)) { "draft" } else { "write" },
                "hidden": plugin.manifest.tools.hides(&tool.name),
            })
        })
        .collect()
}

/// How many tools a plugin's servers offered when they last connected; none before they have.
pub fn saved_tool_count(app: &App, plugin: &Installed) -> Option<usize> {
    catalog_path(app, &plugin.manifest.id).is_file().then(|| saved_servers(app, plugin).iter().map(|(_, _, tools, _)| tools.len()).sum())
}

/// A tool of a plugin server, which scripts call as `tools.github__create_issue(args)`. The
/// server connects on the first call. Permission is asked before the call, at the loop's
/// `before_tool_call` boundary ([`review_call`]).
pub struct PluginTool {
    app: Arc<App>,
    budget: Option<crate::budgets::BudgetContext>,
    /// Bound for a bot turn or routine check. Catalogs used only for inspection omit it.
    policy_context: Option<(Bot, String)>,
    plugin_id: String,
    plugin_name: String,
    server_name: String,
    tool: rmcp::model::Tool,
    name: String,
    description: String,
    read_only: bool,
    kind: ToolKind,
    /// How long a call may go without an answer or progress: its server's `timeout`, or ten minutes.
    timeout: std::time::Duration,
}

/// Resolves the exact original tool on its original server from the live connection for a
/// durable review. A cached tool name never supplies authority or execution preconditions, and
/// the call checks the bot's Access in the originating chat again right before it goes out, as
/// a turn's calls do.
pub async fn reviewed_tool(app: &Arc<App>, bot: &Bot, chat_id: &str, plugin_id: &str, server_name: &str, name: &str, cancel: &CancellationToken) -> Result<Arc<dyn Tool>, String> {
    let plugin = app.plugins.lock().unwrap().get(plugin_id).cloned().ok_or("The reviewed connection was removed.")?;
    if plugin.manifest.tools.hides(name) { return Err("The reviewed tool is hidden.".into()); }
    let server = tokio::select! {
        result = app.mcp.server(app, plugin_id, server_name) => result?,
        _ = cancel.cancelled() => return Err("Stopped".into()),
    };
    let tool = server.tools().into_iter().find(|tool| tool.name.as_ref() == name).ok_or("The server no longer offers the reviewed tool.")?;
    let description = tool.description.as_deref().unwrap_or("").to_string();
    Ok(Arc::new(PluginTool { app: app.clone(), budget: crate::budgets::current(), plugin_id: plugin_id.into(), plugin_name: plugin.display_name(),
        server_name: server_name.into(), tool, name: tool_name(plugin_id, name), description, read_only: false, kind: ToolKind::Call,
        timeout: plugin.manifest.servers.get(server_name).map(ServerSpec::call_timeout).unwrap_or(super::CALL_TIMEOUT),
        policy_context: Some((bot.clone(), chat_id.into())) }))
}

/// What calling a plugin tool asks its server: one of its own tools, or its resources.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ToolKind {
    Call,
    ListResources,
    ListResourceTemplates,
    ReadResource,
}

/// The tools a server that offers resources lists and reads them with, named as Codex and pi
/// name them. Reading a resource changes nothing, so they never ask.
fn resource_tools(plugin_name: &str) -> Vec<(rmcp::model::Tool, ToolKind)> {
    let paged = json!({ "type": "object", "properties": { "cursor": { "type": "string", "description": "nextCursor from the page before, for the next page" } } });
    let listing = |item: Value, key: &str| json!({ "type": "object", "properties": { key: { "type": "array", "items": item }, "nextCursor": { "type": "string" } } });
    let resource = json!({ "type": "object", "properties": { "uri": { "type": "string" }, "name": { "type": "string" }, "title": { "type": "string" }, "description": { "type": "string" }, "mimeType": { "type": "string" }, "size": { "type": "number" } } });
    let template = json!({ "type": "object", "properties": { "uriTemplate": { "type": "string" }, "name": { "type": "string" }, "title": { "type": "string" }, "description": { "type": "string" }, "mimeType": { "type": "string" } } });
    let contents = json!({ "type": "object", "properties": { "contents": { "type": "array", "items": { "type": "object", "properties": { "uri": { "type": "string" }, "mimeType": { "type": "string" }, "text": { "type": "string" }, "blob": { "type": "string", "description": "base64" }, "path": { "type": "string", "description": "where binary contents were saved" } } } } } });
    let tool = |name: &str, description: String, input: Value, output: Value| -> rmcp::model::Tool {
        serde_json::from_value(json!({ "name": name, "description": description, "inputSchema": input, "outputSchema": output, "annotations": { "readOnlyHint": true } })).expect("a resource tool's definition")
    };
    vec![
        (tool("list_mcp_resources", format!("List the resources {plugin_name} offers: files, records, and other data to read by URI with read_mcp_resource. A long list comes in pages."), paged.clone(), listing(resource, "resources")), ToolKind::ListResources),
        (tool("list_mcp_resource_templates", format!("List the URI templates of resources {plugin_name} can read but does not list, such as file:///{{path}}. Fill one in and read it with read_mcp_resource."), paged, listing(template, "resourceTemplates")), ToolKind::ListResourceTemplates),
        (
            tool(
                "read_mcp_resource",
                format!("Read one of {plugin_name}'s resources by its URI. Text comes back as text and images as images; other binary contents are saved to a file, whose path the result names."),
                json!({ "type": "object", "properties": { "uri": { "type": "string", "description": "The resource's URI" } }, "required": ["uri"] }),
                contents,
            ),
            ToolKind::ReadResource,
        ),
    ]
}

/// One plugin tool in the turn's catalog, with the bounded text search ranks it by.
struct CatalogTool {
    name: String,
    original_name: String,
    plugin_id: String,
    plugin_name: String,
    server_name: String,
    description: String,
    search_schema: String,
    server_instructions: String,
    tool: Arc<PluginTool>,
}

/// A plugin as the codemode description names it.
struct PluginGroup {
    id: String,
    description: String,
    /// The description without its servers' instructions, which `describeNamespace()` gives
    /// whole beside it.
    about: String,
}

#[derive(Default)]
struct CatalogState {
    tools: BTreeMap<String, Arc<CatalogTool>>,
    /// Plugins whose servers connected during this turn, so their live tool lists are in.
    connected: std::collections::HashSet<String>,
    /// Each server's instructions, whole, by plugin id and server name.
    instructions: BTreeMap<(String, String), String>,
}

/// What a turn's codemode scripts can call: the bot's own read and write tools, and every tool
/// of the plugins installed on this Runner. A plugin's tools come from what its servers offered
/// last time, so listing them starts nothing; a plugin never connected yet is named, and a
/// search or a call by name connects it.
pub struct PluginCatalog {
    app: Arc<App>,
    budget: Option<crate::budgets::BudgetContext>,
    policy_context: Option<(Bot, String)>,
    local: Vec<Arc<dyn Tool>>,
    groups: Vec<PluginGroup>,
    state: Mutex<CatalogState>,
}

impl PluginCatalog {
    fn new(app: Arc<App>, local: Vec<Arc<dyn Tool>>) -> Self {
        Self::for_context(app, local, None)
    }

    fn for_context(app: Arc<App>, local: Vec<Arc<dyn Tool>>, policy_context: Option<(Bot, String)>) -> Self {
        let mut catalog = PluginCatalog { app, budget: crate::budgets::current(), policy_context, local, groups: Vec::new(), state: Mutex::new(CatalogState::default()) };
        for plugin in &catalog.installed() {
            let saved = saved_servers(&catalog.app, plugin);
            catalog.groups.push(plugin_group(&catalog.app, plugin, &saved));
            for (server, instructions, tools, resources) in &saved {
                catalog.add_tools(plugin, server, instructions.as_deref(), tools, *resources);
            }
        }
        catalog
    }

    fn add_tools(&self, plugin: &Installed, server_name: &str, instructions: Option<&str>, tools: &[rmcp::model::Tool], resources: bool) {
        let mut state = self.state.lock().unwrap();
        let key = (plugin.manifest.id.clone(), server_name.to_string());
        match instructions.map(str::trim).filter(|text| !text.is_empty()) {
            Some(text) => state.instructions.insert(key, text.to_string()),
            None => state.instructions.remove(&key),
        };
        let server_instructions = instructions.map(|text| utf8_prefix(text, MAX_SERVER_INSTRUCTIONS_BYTES).to_string()).unwrap_or_default();
        // A server that offers resources lists and reads them through three tools of its own.
        let resource_tools: Vec<(rmcp::model::Tool, ToolKind)> = if resources { resource_tools(&plugin.manifest.name) } else { Vec::new() };
        let all = tools.iter().map(|tool| (tool, ToolKind::Call)).chain(resource_tools.iter().map(|(tool, kind)| (tool, *kind)));
        for (tool, kind) in all {
            if kind == ToolKind::Call && plugin.manifest.tools.hides(&tool.name) {
                continue;
            }
            let original_name = tool.name.to_string();
            let existing = state.tools.values().find(|known| known.plugin_id == plugin.manifest.id && known.server_name == server_name && known.original_name == original_name).map(|known| known.name.clone());
            let name = existing.unwrap_or_else(|| unique_tool_name(&tool_name(&plugin.manifest.id, &original_name), &state.tools));
            let description = tool
                .description
                .as_deref()
                .map(str::trim)
                .filter(|text| !text.is_empty())
                .map(str::to_string)
                .or_else(|| tool.title.clone())
                .unwrap_or_else(|| format!("{} tool {original_name}", plugin.manifest.name));
            let read_only = kind != ToolKind::Call
                || tool.annotations.as_ref().and_then(|annotations| annotations.read_only_hint).unwrap_or(false)
                || plugin.manifest.tools.readonly.iter().any(|pattern| pattern_matches(pattern, &original_name));
            let executable = Arc::new(PluginTool {
                budget: self.budget.clone(),
                app: self.app.clone(),
                policy_context: self.policy_context.clone(),
                plugin_id: plugin.manifest.id.clone(),
                plugin_name: plugin.display_name(),
                server_name: server_name.to_string(),
                tool: tool.clone(),
                name: name.clone(),
                description: description.clone(),
                read_only,
                kind,
                timeout: plugin.manifest.servers.get(server_name).map(ServerSpec::call_timeout).unwrap_or(super::CALL_TIMEOUT),
            });
            let raw_search_schema = serde_json::to_string(&Value::Object((*tool.input_schema).clone())).unwrap_or_default();
            state.tools.insert(
                name.clone(),
                Arc::new(CatalogTool {
                    name,
                    original_name,
                    plugin_id: plugin.manifest.id.clone(),
                    plugin_name: plugin.display_name(),
                    server_name: server_name.to_string(),
                    description: utf8_prefix(&description, MAX_SEARCH_INDEX_DESCRIPTION_BYTES).to_string(),
                    search_schema: utf8_prefix(&raw_search_schema, MAX_SEARCH_SCHEMA_BYTES).to_string(),
                    server_instructions: server_instructions.clone(),
                    tool: executable,
                }),
            );
        }
    }

    /// Connects a plugin's servers and takes their live tool lists. The problems are why a
    /// server could not connect.
    async fn connect_plugin(&self, plugin: &Installed, cancel: &CancellationToken) -> Vec<String> {
        // Access taken away during the turn: the plugin stays dormant, and nobody is asked.
        if !self.allows(&plugin.manifest.id) {
            return vec![format!("{} is off for this bot", plugin.manifest.name)];
        }
        if self.state.lock().unwrap().connected.contains(&plugin.manifest.id) {
            return Vec::new();
        }
        let status = self.app.plugins.lock().unwrap().status(&plugin.manifest.id);
        if let Some(status) = status.filter(|status| matches!(status.state.as_str(), "needs_setup" | "needs_auth" | "insufficient_access")) {
            return vec![status.detail];
        }
        let mut problems = Vec::new();
        for server_name in plugin.manifest.servers.keys() {
            let server = tokio::select! {
                result = self.app.mcp.server(&self.app, &plugin.manifest.id, server_name) => result,
                _ = cancel.cancelled() => return problems,
            };
            match server {
                Ok(server) => self.add_tools(plugin, &server.name, server.instructions.as_deref(), &server.tools(), server.resources),
                Err(error) => {
                    tracing::warn!(%error, plugin = %plugin.manifest.id, server = %server_name, "connecting a plugin server for a script");
                    problems.push(error);
                }
            }
        }
        if problems.is_empty() {
            self.state.lock().unwrap().connected.insert(plugin.manifest.id.clone());
        }
        problems
    }

    /// Whether the bot whose turn this is may use the plugin now. A catalog with no bot, for
    /// a look at what is installed, takes them all.
    fn allows(&self, plugin_id: &str) -> bool {
        let Some((bot, _)) = &self.policy_context else { return true };
        self.app.bot(&bot.id).is_some_and(|bot| bot.permissions.as_ref().is_none_or(|policy| policy.allows_connection(plugin_id)))
    }

    /// The installed plugins this catalog offers: those the bot may use.
    fn installed(&self) -> Vec<Installed> {
        self.app.plugins.lock().unwrap().installed().iter().filter(|plugin| self.allows(&plugin.manifest.id)).cloned().collect()
    }

    fn plugin_tool(&self, name: &str) -> Option<Arc<PluginTool>> {
        self.state.lock().unwrap().tools.get(name).map(|tool| tool.tool.clone())
    }

    pub fn is_plugin_tool(&self, name: &str) -> bool { self.plugin_tool(name).is_some() }

    /// The plugin a tool belongs to, by name, for the working row's "Using GitHub…".
    pub fn plugin_name(&self, tool_name: &str) -> Option<String> {
        self.state.lock().unwrap().tools.get(tool_name).map(|tool| tool.plugin_name.clone())
    }

    fn entry(tool: &CatalogTool) -> Entry {
        Entry::new(tool.tool.clone(), Exposure::Listed).in_namespace(tool.plugin_id.clone())
    }

    /// A catalog tool by its name or its identifier.
    fn lookup(&self, name: &str) -> Option<Arc<CatalogTool>> {
        let state = self.state.lock().unwrap();
        state.tools.get(name).cloned().or_else(|| state.tools.values().find(|tool| codemode::to_identifier(&tool.name) == name).cloned())
    }
}

/// How the codemode description names a plugin: its id, its name and what it does, what it
/// still needs, and the start of its servers' own instructions, pointing at `describeNamespace()`
/// for the rest. Only the needs that last are named, so the description stays the same from
/// turn to turn.
fn plugin_group(app: &App, plugin: &Installed, saved: &[Offered]) -> PluginGroup {
    let manifest = &plugin.manifest;
    let mut description = plugin.display_name();
    if let Some(service) = &plugin.service_id {
        description.push_str(&format!(" (service {service}, account id {}). Use this namespace to explicitly select this account; ask the user when the intended account is unclear.", manifest.id));
    }
    if let Some(about) = manifest.description.lines().map(str::trim).find(|line| !line.is_empty()) {
        description.push_str(&format!(": {about}"));
    }
    let status = app.plugins.lock().unwrap().status(&manifest.id);
    match status.as_ref().map(|status| status.state.as_str()) {
        Some("needs_auth" | "insufficient_access") => description.push_str("\nNeeds a sign-in before its tools work: call connect_plugin with this account id."),
        Some("needs_setup") => description.push_str(&format!("\nNot set up yet ({}): the user sets it up in the plugin's settings.", status.map(|status| status.detail).unwrap_or_default())),
        _ => {}
    }
    let about = description.clone();
    for (_, instructions, _, _) in saved {
        if let Some(instructions) = instructions.as_deref().map(str::trim).filter(|text| !text.is_empty()) {
            let shown = utf8_prefix(instructions, MAX_NAMESPACE_INSTRUCTIONS_BYTES);
            let rest = if shown.len() < instructions.len() { format!("… (describeNamespace(\"{}\") has the rest)", manifest.id) } else { String::new() };
            description.push_str(&format!("\nServer instructions: {shown}{rest}"));
        }
    }
    PluginGroup { id: manifest.id.clone(), description, about }
}

#[async_trait]
impl codemode::Catalog for PluginCatalog {
    fn entries(&self) -> Vec<Entry> {
        let mut entries: Vec<Entry> = self.local.iter().map(|tool| Entry::new(tool.clone(), Exposure::Direct)).collect();
        entries.extend(self.state.lock().unwrap().tools.values().map(|tool| PluginCatalog::entry(tool)));
        entries
    }

    fn namespaces(&self) -> Vec<Namespace> {
        self.groups.iter().map(|group| Namespace { name: group.id.clone(), description: group.description.clone() }).collect()
    }

    async fn find(&self, name: &str, cancel: &CancellationToken) -> Option<Entry> {
        if let Some(tool) = self.local.iter().find(|tool| tool.name() == name) {
            return Some(Entry::new(tool.clone(), Exposure::Direct));
        }
        if let Some(tool) = self.lookup(name) {
            return Some(PluginCatalog::entry(&tool));
        }
        // A tool of a plugin that has not connected yet: connect it and look again.
        let (prefix, _) = name.split_once("__")?;
        let plugin = self.installed().into_iter().find(|plugin| plugin.manifest.id == prefix || codemode::to_identifier(&plugin.manifest.id) == prefix)?;
        self.connect_plugin(&plugin, cancel).await;
        self.lookup(name).map(|tool| PluginCatalog::entry(&tool))
    }

    /// A plugin by its id or identifier: what it is, its servers' instructions whole, and its
    /// tools. One with no saved tool list connects first, as a search does.
    async fn describe_namespace(&self, name: &str, cancel: &CancellationToken) -> Option<NamespaceDetails> {
        let plugin = self.installed().into_iter().find(|plugin| plugin.manifest.id == name || codemode::to_identifier(&plugin.manifest.id) == name)?;
        let id = plugin.manifest.id.clone();
        let known = self.state.lock().unwrap().tools.values().any(|tool| tool.plugin_id == id);
        if !known {
            self.connect_plugin(&plugin, cancel).await;
        }
        let about = self.groups.iter().find(|group| group.id == id).map(|group| group.about.clone()).unwrap_or_else(|| plugin.display_name());
        let state = self.state.lock().unwrap();
        let instructions: Vec<&str> = state.instructions.iter().filter(|((plugin_id, _), _)| *plugin_id == id).map(|(_, text)| text.as_str()).collect();
        let tools = state.tools.values().filter(|tool| tool.plugin_id == id).map(|tool| tool.name.clone()).collect();
        Some(NamespaceDetails { name: id, description: about, instructions: instructions.join("\n\n"), tools })
    }

    async fn search(&self, query: &str, namespace: Option<&str>, limit: usize, cancel: &CancellationToken) -> Result<Vec<Entry>, String> {
        if query.trim().is_empty() {
            return Err("searchTools() needs a non-empty query".into());
        }
        let installed = self.installed();
        let plugins: Vec<Installed> = match namespace {
            Some(namespace) => {
                let plugins: Vec<Installed> = installed.into_iter().filter(|plugin| plugin.manifest.id == namespace || codemode::to_identifier(&plugin.manifest.id) == namespace).collect();
                if plugins.is_empty() {
                    return Err(format!("No plugin \"{namespace}\" is installed on this Runner"));
                }
                plugins
            }
            None => installed,
        };
        // A plugin with no saved tool list connects now, so its tools can match.
        for plugin in &plugins {
            let known = self.state.lock().unwrap().tools.values().any(|tool| tool.plugin_id == plugin.manifest.id);
            if !known {
                self.connect_plugin(plugin, cancel).await;
            }
        }
        let candidates: Vec<Arc<CatalogTool>> =
            self.state.lock().unwrap().tools.values().filter(|tool| plugins.iter().any(|plugin| plugin.manifest.id == tool.plugin_id)).cloned().collect();
        Ok(rank_tools(query, &candidates, namespace.is_some()).into_iter().take(limit).map(|tool| PluginCatalog::entry(&tool)).collect())
    }
}

/// The catalog behind a turn's `codemode` tool, with `local` (the bot's own read and write
/// tools) callable from scripts too. No MCP process or HTTP connection starts here.
pub fn turn_catalog(app: &Arc<App>, local: Vec<Arc<dyn Tool>>) -> Arc<PluginCatalog> {
    Arc::new(PluginCatalog::new(app.clone(), local))
}

/// Execution catalogs bind every MCP call to the bot and chat for a final policy check.
pub fn bot_catalog(app: &Arc<App>, bot: &Bot, chat_id: &str, local: Vec<Arc<dyn Tool>>) -> Arc<PluginCatalog> {
    Arc::new(PluginCatalog::for_context(app.clone(), local, Some((bot.clone(), chat_id.into()))))
}

/// The installed plugins as the system prompt names them.
pub fn plugin_briefs(app: &App) -> Vec<PluginBrief> {
    let store = app.plugins.lock().unwrap();
    store
        .installed()
        .iter()
        .map(|plugin| {
            let status = store.status(&plugin.manifest.id);
            PluginBrief {
                id: plugin.manifest.id.clone(),
                name: plugin.display_name(),
                state: status.as_ref().map(|status| status.state.clone()).unwrap_or_else(|| "ready".into()),
                detail: status.map(|status| status.detail).unwrap_or_default(),
                skills: plugin
                    .manifest
                    .skills
                    .iter()
                    .map(|skill| {
                        (
                            skill.name.clone(),
                            skill.description.clone(),
                            app.config.plugins_dir().join(&plugin.manifest.id).join("skills").join(format!("{}.md", super::slug(&skill.name))),
                        )
                    })
                    .collect(),
            }
        })
        .collect()
}

/// What the system prompt says about one installed plugin. Its tools are in the codemode
/// tool's description.
pub struct PluginBrief {
    pub id: String,
    pub name: String,
    pub state: String,
    pub detail: String,
    /// (name, description, path)
    pub skills: Vec<(String, String, std::path::PathBuf)>,
}

/// What a plugin tool may do.
enum Access {
    ReadOnly,
    /// It may change things. `description` is its live server's, for the review.
    Changes { description: String, capability: crate::permissions::Capability },
    Stopped,
}

/// Whether a plugin tool only reads: its manifest lists it so, or its server marks it
/// `readOnlyHint` now. What the server says now counts, never what the saved list says, since
/// the bot can write that file; a server that cannot be reached says nothing, and the tool may
/// change things.
async fn access(app: &Arc<App>, tool: &PluginTool, cancel: &CancellationToken) -> Access {
    // Listing and reading resources is the protocol's, and changes nothing.
    if tool.kind != ToolKind::Call {
        return Access::ReadOnly;
    }
    let name = tool.tool.name.to_string();
    let manifest_read_only =
        app.plugins.lock().unwrap().get(&tool.plugin_id).is_some_and(|plugin| plugin.manifest.tools.readonly.iter().any(|pattern| pattern_matches(pattern, &name)));
    if manifest_read_only {
        return Access::ReadOnly;
    }
    let live = tokio::select! {
        biased;
        _ = cancel.cancelled() => return Access::Stopped,
        server = app.mcp.server(app, &tool.plugin_id, &tool.server_name) => server.ok(),
    };
    let live_tool = live.as_ref().and_then(|server| server.tools().into_iter().find(|candidate| candidate.name == tool.tool.name));
    if live_tool.as_ref().and_then(|live| live.annotations.as_ref()).and_then(|annotations| annotations.read_only_hint) == Some(true) {
        return Access::ReadOnly;
    }
    let draft = app.plugins.lock().unwrap().get(&tool.plugin_id)
        .is_some_and(|plugin| plugin.manifest.tools.draft.iter().any(|pattern| pattern_matches(pattern, &name)));
    Access::Changes {
        description: live_tool.and_then(|live| live.description.map(|description| description.to_string())).unwrap_or_default(),
        capability: if draft { crate::permissions::Capability::Draft } else { crate::permissions::Capability::Write },
    }
}

/// Whether a script's call to `name` reaches a plugin tool that only reads (see `access`).
pub async fn is_read_only(app: &Arc<App>, catalog: &PluginCatalog, name: &str, cancel: &CancellationToken) -> bool {
    match catalog.plugin_tool(name) {
        Some(tool) => matches!(access(app, &tool, cancel).await, Access::ReadOnly),
        None => false,
    }
}

async fn authorize_plugin(app: &Arc<App>, bot: &Bot, tool: &PluginTool, cancel: &CancellationToken) -> Result<(), crate::permissions::AccessDenied> {
    use crate::permissions::{self, Capability};
    let name = tool.tool.name.as_ref();
    permissions::check_connection(app, bot, &tool.plugin_id, name, None)?;
    let capability = match access(app, tool, cancel).await {
        Access::ReadOnly => Capability::Read,
        Access::Changes { capability, .. } => capability,
        Access::Stopped => {
            return Err(permissions::AccessDenied { tool: name.into(), connection_id: Some(tool.plugin_id.clone()), capability: None, reason: "the call was stopped".into(), grantable: false })
        }
    };
    permissions::check_connection(app, bot, &tool.plugin_id, name, Some(capability))
}

/// The bot's Access for a script's call by name, with the live or trusted classification of
/// what the tool does: a routine check's.
pub async fn authorize_catalog_tool(app: &Arc<App>, catalog: &PluginCatalog, bot: &Bot, name: &str, cancel: &CancellationToken) -> Result<(), crate::permissions::AccessDenied> {
    let tool = catalog.plugin_tool(name).ok_or_else(|| crate::permissions::AccessDenied {
        tool: name.into(), connection_id: None, capability: None, reason: "the tool is no longer available".into(), grantable: false,
    })?;
    authorize_plugin(app, bot, &tool, cancel).await
}

/// Auto-review, and the user's answer on a card when it asks, for a plugin call a script makes:
/// at the loop's `before_tool_call` boundary, so a call that is not allowed never reaches the
/// server and ends the script. A read-only tool runs at once. `None` lets the call run.
#[allow(clippy::too_many_arguments)]
pub async fn review_call(
    app: &Arc<App>,
    catalog: &PluginCatalog,
    chat_id: &str,
    trigger: &super::review::Trigger,
    bot: &Bot,
    unattended: bool,
    ctx: &BeforeToolCallContext<'_>,
) -> Option<BeforeToolCallResult> {
    let tool = catalog.plugin_tool(&ctx.tool_call.name)?;
    // Nothing to ask about while the user has the bot's browser.
    if tool.plugin_id == crate::browser::PLUGIN_ID {
        if let Err(error) = app.browser_sessions.wait_if_taken_over(&bot.id, ctx.cancel).await {
            return Some(crate::local_review::blocked(error));
        }
    }
    let name = tool.tool.name.to_string();
    if let Err(denied) = crate::permissions::check_connection(app, bot, &tool.plugin_id, &name, None) {
        return Some(crate::permissions::refuse(app, chat_id, bot, denied));
    }
    let (capability, review_description) = match access(app, &tool, ctx.cancel).await {
        Access::ReadOnly => (crate::permissions::Capability::Read, String::new()),
        Access::Stopped => return Some(crate::local_review::blocked("Stopped".into())),
        Access::Changes { description, capability } => (capability, description),
    };
    if let Err(denied) = crate::permissions::check_connection(app, bot, &tool.plugin_id, &name, Some(capability)) {
        return Some(crate::permissions::refuse(app, chat_id, bot, denied));
    }
    if capability == crate::permissions::Capability::Read { return None; }
    // The script the call comes from says what the whole batch is for.
    let script = ctx.parent.filter(|parent| parent.name == codemode::CODEMODE_TOOL_NAME).and_then(|parent| parent.arguments["code"].as_str());
    let outcome = super::review::decide(app, bot, chat_id, trigger, &tool.plugin_id, &tool.plugin_name, &name, &review_description, ctx.args, script, ctx.cancel).await;
    let super::review::Outcome::Ask { reason, .. } = outcome else { return None };
    if unattended {
        let staged = crate::review_execution::stage_call(app, bot, chat_id, trigger, &ctx.tool_call.id,
            crate::review_queue::ReviewPayload::Plugin { plugin_id: tool.plugin_id.clone(), server_name: tool.server_name.clone(), tool: name.clone(), arguments: ctx.args.clone() },
            crate::review_queue::ReviewTarget { account: tool.plugin_name.clone(), resource: call_summary(&name, ctx.args) }, reason.as_deref()).await;
        let status = match staged {
            Ok(item) => format!("Staged review {} (version {}). The user can edit and approve it later; the exact call resumes on this Runner. Do not retry it now.", item.id, item.version),
            Err(error) => format!("Could not stage the action for review: {error}. Report the proposed action."),
        };
        return Some(crate::local_review::blocked(format!(
            "{name} needs the user's permission ({}). {status}",
            reason.as_deref().unwrap_or("Auto-review is off, so every change asks")
        )));
    }
    let summary = call_summary(&name, ctx.args);
    match ask(app, chat_id, &bot.id, &tool.plugin_id, &tool.plugin_name, &name, &summary, ctx.args.clone(), reason, ctx.cancel).await {
        Decision::Allowed | Decision::Always => None,
        Decision::Denied => Some(crate::local_review::blocked(format!("The user did not allow {name}. Do not retry it; ask what they want instead."))),
        Decision::Expired => Some(crate::local_review::blocked(format!("Nobody answered the permission request for {name} in time. Say what you needed and stop."))),
        Decision::Dismissed => Some(crate::local_review::dismissed(&format!("The user sent a new message instead of answering, so {name} did not run. Follow that message."))),
    }
}

fn unique_tool_name(base: &str, catalog: &BTreeMap<String, Arc<CatalogTool>>) -> String {
    if !catalog.contains_key(base) {
        return base.to_string();
    }
    for index in 2.. {
        let suffix = format!("_{index}");
        let keep = 64usize.saturating_sub(suffix.len()).min(base.len());
        let candidate = format!("{}{}", &base[..keep], suffix);
        if !catalog.contains_key(&candidate) {
            return candidate;
        }
    }
    unreachable!()
}

fn utf8_prefix(text: &str, max_bytes: usize) -> &str {
    if text.len() <= max_bytes {
        return text;
    }
    let mut end = max_bytes;
    while end > 0 && !text.is_char_boundary(end) {
        end -= 1;
    }
    &text[..end]
}

fn query_tokens(query: &str) -> Vec<String> {
    let mut tokens = Vec::new();
    let mut start = None;
    for (index, byte) in query.bytes().enumerate() {
        if byte.is_ascii_alphanumeric() {
            start.get_or_insert(index);
        } else if let Some(token_start) = start.take() {
            let token = query[token_start..index].to_ascii_lowercase();
            if !tokens.contains(&token) {
                tokens.push(token);
            }
        }
    }
    if let Some(token_start) = start {
        let token = query[token_start..].to_ascii_lowercase();
        if !tokens.contains(&token) {
            tokens.push(token);
        }
    }
    tokens
}

fn term_frequency(text: &str, token: &str) -> usize {
    if token.is_empty() || token.len() > text.len() {
        return 0;
    }
    let text = text.as_bytes();
    let token = token.as_bytes();
    let mut count = 0;
    let mut start = 0;
    while start + token.len() <= text.len() {
        let end = start + token.len();
        let equal = text[start..end].iter().zip(token).all(|(left, right)| left.eq_ignore_ascii_case(right));
        let left_boundary = start == 0 || !text[start - 1].is_ascii_alphanumeric();
        let right_boundary = end == text.len() || !text[end].is_ascii_alphanumeric();
        if equal && left_boundary && right_boundary {
            count += 1;
            start = end;
        } else {
            start += 1;
        }
    }
    count
}

fn contains_complete_identity(query: &str, identity: &str) -> bool {
    if identity.is_empty() || identity.len() > query.len() {
        return false;
    }
    let query = query.as_bytes();
    let identity = identity.as_bytes();
    for start in 0..=query.len() - identity.len() {
        let end = start + identity.len();
        let equal = query[start..end].iter().zip(identity).all(|(left, right)| left.eq_ignore_ascii_case(right));
        let identity_byte = |byte: u8| byte.is_ascii_alphanumeric() || byte == b'_' || byte == b'-';
        if equal && (start == 0 || !identity_byte(query[start - 1])) && (end == query.len() || !identity_byte(query[end])) {
            return true;
        }
    }
    false
}

fn rank_tools(query: &str, candidates: &[Arc<CatalogTool>], scoped: bool) -> Vec<Arc<CatalogTool>> {
    let tokens = query_tokens(query);
    let mut document_frequencies = vec![0usize; tokens.len()];
    for (token_index, token) in tokens.iter().enumerate() {
        document_frequencies[token_index] = candidates
            .iter()
            .filter(|tool| {
                let primary = [&tool.plugin_id, &tool.plugin_name, &tool.original_name, &tool.name];
                let secondary = [&tool.description, &tool.search_schema, &tool.server_instructions];
                primary.iter().any(|field| term_frequency(field, token) > 0)
                    || secondary.iter().any(|field| term_frequency(field, token) > 0)
            })
            .count();
    }

    let mut ranked: Vec<(Arc<CatalogTool>, bool, f64, usize)> = Vec::new();
    for tool in candidates {
        let exact = [&tool.original_name, &tool.name, &tool.plugin_id]
            .iter()
            .any(|identity| contains_complete_identity(query, identity));
        let primary = [&tool.plugin_id, &tool.plugin_name, &tool.original_name, &tool.name];
        let secondary = [&tool.description, &tool.search_schema, &tool.server_instructions];
        let mut primary_hits = 0usize;
        let mut secondary_hits = 0usize;
        let mut score = 0.0;
        for (index, token) in tokens.iter().enumerate() {
            let primary_frequency: usize = primary.iter().map(|field| term_frequency(field, token)).sum();
            let secondary_frequency: usize = secondary.iter().map(|field| term_frequency(field, token)).sum();
            if primary_frequency > 0 {
                primary_hits += 1;
            }
            let secondary_is_evidence = secondary_frequency > 0
                && (scoped || (token.len() >= 4 && document_frequencies[index] < candidates.len()));
            if secondary_is_evidence {
                secondary_hits += 1;
            }
            if primary_frequency > 0 || secondary_frequency > 0 {
                let count = candidates.len().max(1) as f64;
                let frequency = document_frequencies[index] as f64;
                let idf = (1.0 + (count - frequency + 0.5) / (frequency + 0.5)).ln();
                score += idf * (3.0 * primary_frequency as f64 + secondary_frequency as f64);
            }
        }
        let clear_match = exact || (tokens.len() == 1 && primary_hits > 0) || primary_hits + secondary_hits >= 2;
        if clear_match {
            ranked.push((tool.clone(), exact, score, primary_hits));
        }
    }
    ranked.sort_by(|left, right| {
        right
            .1
            .cmp(&left.1)
            .then_with(|| right.2.partial_cmp(&left.2).unwrap_or(std::cmp::Ordering::Equal))
            .then_with(|| right.3.cmp(&left.3))
            .then_with(|| left.0.name.to_ascii_lowercase().cmp(&right.0.name.to_ascii_lowercase()))
    });
    ranked.into_iter().map(|(tool, _, _, _)| tool).collect()
}

#[async_trait]
impl Tool for PluginTool {
    fn name(&self) -> &str {
        &self.name
    }
    fn label(&self) -> &str {
        &self.plugin_name
    }
    fn description(&self) -> &str {
        &self.description
    }
    /// The server's input schema, as an object schema with properties, which providers and the
    /// schema check both need.
    fn parameters(&self) -> Value {
        let mut schema = Value::Object((*self.tool.input_schema).clone());
        if schema.get("type").is_none() {
            schema["type"] = json!("object");
        }
        if schema.get("properties").is_none() {
            schema["properties"] = json!({});
        }
        schema
    }
    /// A script gets the whole `CallToolResult`, with the tool's own output schema as its
    /// `structuredContent`.
    fn output_schema(&self) -> Option<Value> {
        let mut properties = json!({ "content": { "type": "array", "items": { "type": "object" } }, "isError": { "type": "boolean" } });
        if let Some(schema) = &self.tool.output_schema {
            properties["structuredContent"] = Value::Object((**schema).clone());
        }
        Some(json!({ "type": "object", "properties": properties, "required": ["content"] }))
    }
    /// A call that may ask the user runs alone, so two of a script's cards never ask at once.
    fn execution_mode(&self) -> Option<ToolExecutionMode> {
        (!self.read_only).then_some(ToolExecutionMode::Sequential)
    }
    async fn execute(&self, _id: &str, mut args: Value, cancel: CancellationToken, _on_update: ToolUpdateFn) -> Result<ToolResult, ToolError> {
        self.check_policy(&cancel).await?;
        if self.kind != ToolKind::Call {
            return self.resources(args, cancel).await;
        }
        let tool = self.tool.name.to_string();
        // A bot's Browser calls go to its open profile, and wait while the user has it.
        let mut browser_input = None;
        if self.plugin_id == crate::browser::PLUGIN_ID {
            if let Some((bot, _)) = &self.policy_context {
                browser_input = self.app.browser_sessions.input(&self.app, &bot.id, &cancel).await.map_err(ToolError)?;
            }
        }
        if browser_input.is_some() && tool == "browser_take_screenshot" {
            args["type"] = json!("png");
            if let Some(args) = args.as_object_mut() {
                args.remove("filename");
            }
        }
        let server = match &browser_input {
            Some(input) => input.server.clone(),
            None => tokio::select! {
                server = self.app.mcp.server(&self.app, &self.plugin_id, &self.server_name) => server.map_err(ToolError)?,
                _ = cancel.cancelled() => return Err(ToolError("Stopped".into())),
            },
        };
        let mut params = CallToolRequestParams::default();
        // A sign-in, connection or review may have awaited while the user revoked access.
        self.check_policy(&cancel).await?;
        let _permit = self.admit(&cancel).await?;
        params.name = tool.clone().into();
        params.arguments = args.as_object().cloned();
        // A call the user stops, or one that runs out of time, is called off at the server too
        // (`notifications/cancelled`), so it stops the work rather than finishing it unseen.
        let request = rmcp::model::ClientRequest::CallToolRequest(rmcp::model::CallToolRequest::new(params));
        let options = rmcp::service::PeerRequestOptions::with_timeout(self.timeout).reset_timeout_on_progress();
        let call = server.service.send_cancellable_request(request, options);
        let handle = tokio::select! {
            handle = call => handle.map_err(|e| ToolError(format!("{tool} failed: {e}")))?,
            _ = cancel.cancelled() => return Err(ToolError("Stopped".into())),
        };
        let (peer, id) = (handle.peer.clone(), handle.id.clone());
        let mut answer = Box::pin(handle.await_response());
        let response = tokio::select! {
            response = &mut answer => response,
            _ = cancel.cancelled() => {
                // A profile's browser stays the bot's until the server has answered, so a takeover
                // never meets a click still on its way. Called off, the call would be forgotten here
                // while the server went on, so it finishes; one that takes over ten seconds closes
                // its browser instead.
                if let Some(input) = browser_input {
                    let app = self.app.clone();
                    tokio::spawn(async move {
                        if tokio::time::timeout(std::time::Duration::from_secs(10), answer).await.is_err() {
                            input.interrupted(&app);
                        }
                    });
                    return Err(ToolError("Stopped".into()));
                }
                let cancelled = rmcp::model::CancelledNotification::new(rmcp::model::CancelledNotificationParam::new(Some(id), Some("Stopped".into())));
                let _ = peer.send_notification(cancelled.into()).await;
                return Err(ToolError("Stopped".into()));
            }
        };
        if let Some(auth) = &server.auth {
            persist_refreshed(&self.app, &self.plugin_id, &server.name, auth, server.generation).await;
        }
        let result = match response {
            Ok(rmcp::model::ServerResult::CallToolResult(result)) => result,
            Ok(_) => return Err(ToolError(format!("{tool} answered with something other than a result"))),
            Err(rmcp::ServiceError::Timeout { .. }) => {
                if let Some(input) = &browser_input { input.interrupted(&self.app); }
                return Err(ToolError(format!("{tool} took too long")));
            }
            Err(error) => {
                if let rmcp::ServiceError::McpError(error) = &error {
                    self.app.connector_limits.observe_result(&self.app, &self.plugin_id, &serde_json::to_value(error).unwrap_or_default());
                }
                if let Some((scope, challenge)) = insufficient_scope(&error) {
                    needs_more_access(&self.app, &self.plugin_id, &self.server_name, &scope, &challenge);
                    return Err(ToolError(format!("{} needs more access for {tool}. The user signs in to it again to grant it.", self.plugin_name)));
                }
                if self.is_named_account() && authorization_expired(&error.to_string()) {
                    let _ = super::set_oauth(&self.app, &self.plugin_id, &self.server_name, None);
                    self.app.mcp.forget(&self.plugin_id);
                    super::announce(&self.app);
                    return Err(ToolError(format!("{} needs a new sign-in. Use connect_plugin with account id {}.", self.plugin_name, self.plugin_id)));
                }
                // The next call starts the server again.
                if server.is_closed() {
                    return Err(ToolError(format!("{} stopped running: {error}", self.plugin_name)));
                }
                return Err(ToolError(format!("{tool} failed: {error}")));
            }
        };
        let is_error = result.is_error.unwrap_or(false);
        if is_error {
            self.app.connector_limits.observe_result(&self.app, &self.plugin_id, &serde_json::to_value(&result).unwrap_or_default());
        }
        // Slack and Google answer a revoked or narrowed authorization with an error result.
        if is_error && self.is_named_account() {
            let failure = serde_json::to_string(&result).unwrap_or_default();
            if let Some((scope, challenge)) = insufficient_scope(&std::io::Error::other(failure.clone())) {
                needs_more_access(&self.app, &self.plugin_id, &self.server_name, &scope, &challenge);
            } else if authorization_expired(&failure) {
                let _ = super::set_oauth(&self.app, &self.plugin_id, &self.server_name, None);
                self.app.mcp.forget(&self.plugin_id);
                super::announce(&self.app);
            }
        }
        // A screenshot of the bot's profile is published in the chat too.
        if !is_error && tool == "browser_take_screenshot" {
            if let (Some(input), Some((bot, chat_id))) = (&browser_input, &self.policy_context) {
                crate::browser::publish_image(&self.app, &bot.id, chat_id, &input.name, &result).map_err(ToolError)?;
            }
        }
        // Off the async threads: making a large image one a model takes takes a moment.
        let (result, mut content) = tokio::task::spawn_blocking(move || {
            let content = model_content(&result);
            (result, content)
        })
        .await
        .map_err(|e| ToolError(format!("{tool} failed: {e}")))?;
        if is_error && content.iter().all(|part| part.as_text().is_none_or(|text| text.trim().is_empty())) {
            content = vec![ContentPart::text(format!("{} {tool} reported an error", self.plugin_name))];
        }
        let mut structured = serde_json::to_value(&result).unwrap_or_else(|_| json!({ "content": [] }));
        if let Some(fields) = structured.as_object_mut() {
            fields.remove("_meta");
            fields.remove("resultType");
        }
        Ok(ToolResult { content, details: json!({ "plugin_id": self.plugin_id, "tool": tool }), structured: Some(structured), is_error, terminate: false })
    }
}

impl PluginTool {
    /// A call waits for the account's shared capacity, then counts toward the turn's limits.
    /// A turn out of plugin calls doesn't take a slot from the other bots.
    async fn admit(&self, cancel: &CancellationToken) -> Result<crate::connector_limits::CallPermit, ToolError> {
        if let Some(budget) = &self.budget { budget.check().map_err(ToolError)?; }
        let permit = crate::connector_limits::ConnectorLimits::acquire(&self.app, &self.plugin_id, cancel).await.map_err(ToolError)?;
        if let Some(budget) = &self.budget { budget.connector_call().map_err(ToolError)?; }
        Ok(permit)
    }

    fn describe_call_error(&self, error: &rmcp::ServiceError) -> String {
        if let rmcp::ServiceError::McpError(data) = error {
            self.app.connector_limits.observe_result(&self.app, &self.plugin_id, &serde_json::to_value(data).unwrap_or_default());
        }
        error.to_string()
    }

    fn is_named_account(&self) -> bool {
        self.app.plugins.lock().unwrap().get(&self.plugin_id).is_some_and(|plugin| plugin.service_id.is_some())
    }

    async fn check_policy(&self, cancel: &CancellationToken) -> Result<(), ToolError> {
        let Some((bot, chat_id)) = &self.policy_context else { return Ok(()) };
        match authorize_plugin(&self.app, bot, self, cancel).await {
            Ok(()) => Ok(()),
            Err(denied) => {
                let result = crate::permissions::refuse(&self.app, chat_id, bot, denied);
                Err(ToolError(result.reason.unwrap_or_default()))
            }
        }
    }

    /// A resource tool's call: a page of the server's resources or templates, or one resource's
    /// contents, shaped as a tool's result, so a script reads `structuredContent` either way.
    async fn resources(&self, args: Value, cancel: CancellationToken) -> Result<ToolResult, ToolError> {
        let tool = self.tool.name.to_string();
        let server = tokio::select! {
            server = self.app.mcp.server(&self.app, &self.plugin_id, &self.server_name) => server.map_err(ToolError)?,
            _ = cancel.cancelled() => return Err(ToolError("Stopped".into())),
        };
        let peer = server.service.peer();
        self.check_policy(&cancel).await?;
        let _permit = self.admit(&cancel).await?;
        let page = args["cursor"].as_str().and_then(|cursor| serde_json::from_value::<rmcp::model::PaginatedRequestParams>(json!({ "cursor": cursor })).ok());
        let asked = async {
            Ok::<Value, String>(match self.kind {
                ToolKind::ListResources => listed(serde_json::to_value(peer.list_resources(page).await.map_err(|e| self.describe_call_error(&e))?).unwrap_or_default(), "resources"),
                ToolKind::ListResourceTemplates => {
                    listed(serde_json::to_value(peer.list_resource_templates(page).await.map_err(|e| self.describe_call_error(&e))?).unwrap_or_default(), "resourceTemplates")
                }
                ToolKind::ReadResource | ToolKind::Call => {
                    let uri = args["uri"].as_str().filter(|uri| !uri.trim().is_empty()).ok_or("read_mcp_resource needs the resource's uri")?;
                    let params = serde_json::from_value::<rmcp::model::ReadResourceRequestParams>(json!({ "uri": uri })).map_err(|e| e.to_string())?;
                    serde_json::to_value(peer.read_resource(params).await.map_err(|e| self.describe_call_error(&e))?).unwrap_or_default()
                }
            })
        };
        let structured = tokio::select! {
            answer = tokio::time::timeout(self.timeout, asked) => answer.map_err(|_| ToolError(format!("{tool} took too long")))?.map_err(|e| ToolError(format!("{tool} failed: {e}")))?,
            _ = cancel.cancelled() => return Err(ToolError("Stopped".into())),
        };
        let (content, structured) = match self.kind {
            ToolKind::ReadResource => tokio::task::spawn_blocking(move || read_contents(structured)).await.map_err(|e| ToolError(format!("{tool} failed: {e}")))?,
            _ => (vec![ContentPart::text(serde_json::to_string_pretty(&structured).unwrap_or_default())], structured),
        };
        let blocks: Vec<Value> = content
            .iter()
            .map(|part| match part {
                ContentPart::Image { data, mime_type } => json!({ "type": "image", "data": data, "mimeType": mime_type }),
                other => json!({ "type": "text", "text": other.as_text().unwrap_or_default() }),
            })
            .collect();
        let result = json!({ "content": blocks, "structuredContent": structured, "isError": false });
        Ok(ToolResult { content, details: json!({ "plugin_id": self.plugin_id, "tool": tool }), structured: Some(result), is_error: false, terminate: false })
    }
}

/// A page of resources or resource templates, each with what says what it is, and without the
/// interfaces of MCP Apps (`ui://`), which Lorca does not show, or icons and `_meta`.
fn listed(page: Value, key: &str) -> Value {
    let keep = ["uri", "uriTemplate", "name", "title", "description", "mimeType", "size"];
    let items: Vec<Value> = page[key]
        .as_array()
        .into_iter()
        .flatten()
        .filter(|item| !item["uri"].as_str().is_some_and(|uri| uri.starts_with("ui://")) && !item["mimeType"].as_str().is_some_and(|mime| mime.contains("profile=mcp-app")))
        .map(|item| Value::Object(keep.iter().filter_map(|field| item.get(*field).map(|value| (field.to_string(), value.clone()))).collect()))
        .collect();
    let mut out = json!({ key: items });
    if let Some(cursor) = page["nextCursor"].as_str().filter(|cursor| !cursor.is_empty()) {
        out["nextCursor"] = json!(cursor);
    }
    out
}

/// The scope a signed-in server said a call needs, and its whole challenge, when it refused the
/// call for want of it: a 403 whose `WWW-Authenticate` says `error="insufficient_scope"`.
fn insufficient_scope(error: &(dyn std::error::Error + 'static)) -> Option<(String, String)> {
    let mut text = String::new();
    let mut cause = Some(error);
    while let Some(current) = cause {
        text.push_str(&current.to_string());
        text.push('\n');
        cause = current.source();
    }
    if !text.contains("insufficient_scope") && !text.to_ascii_lowercase().contains("insufficient scope") {
        return None;
    }
    let challenge = text.lines().find(|line| line.contains("insufficient_scope")).unwrap_or_default();
    let challenge = challenge.split_once("Bearer").map(|(_, rest)| format!("Bearer{rest}")).unwrap_or_else(|| challenge.to_string());
    let scope = challenge.split("scope=\"").nth(1).and_then(|rest| rest.split('"').next()).unwrap_or_default().to_string();
    Some((scope, challenge))
}

/// A server refused a call for want of access the sign-in lacks, as pi signs in again on
/// `insufficient_scope`: the scope it named joins those the sign-in had, for the next sign-in to ask
/// for (`scope:<server>`), and the sign-in is forgotten, so the plugin reads Sign in.
fn needs_more_access(app: &Arc<App>, plugin_id: &str, server: &str, required: &str, challenge: &str) {
    {
        let mut store = app.plugins.lock().unwrap();
        let granted = store.sign_in_secret(plugin_id, "oauth", server).and_then(|saved| saved["tokens"]["scope"].as_str().map(str::to_string)).unwrap_or_default();
        let earlier = store.secret(plugin_id, &format!("scope:{server}")).and_then(|saved| saved["scope"].as_str().map(str::to_string)).unwrap_or_default();
        let mut scopes: Vec<&str> = Vec::new();
        for scope in earlier.split_whitespace().chain(granted.split_whitespace()).chain(required.split_whitespace()) {
            if !scopes.contains(&scope) {
                scopes.push(scope);
            }
        }
        // An object, so `values` never takes it for a variable.
        let wanted = json!({ "scope": scopes.join(" ") });
        store.set_secret(plugin_id, &format!("scope:{server}"), Some(wanted));
        store.set_secret(plugin_id, &format!("oauth:{server}"), None);
        // One that signs in only when asked reads Sign in from the challenge it answered with.
        store.note_challenge(&app.config, plugin_id, server, challenge);
        if let Err(error) = store.save(&app.config) {
            tracing::warn!(%error, "forgetting a sign-in that needs more access");
        }
    }
    tracing::info!(plugin = plugin_id, server, required, "a server needs more access than its sign-in has");
    app.mcp.forget(plugin_id);
    super::note(app, plugin_id, None);
}

/// A read resource for the model and for scripts: text as text, images a model takes as images,
/// and other binary contents saved to a private file the result names, never carried as base64.
fn read_contents(read: Value) -> (Vec<ContentPart>, Value) {
    let mut content = Vec::new();
    let mut text_len = 0;
    let mut contents = Vec::new();
    for item in read["contents"].as_array().into_iter().flatten() {
        let uri = item["uri"].as_str().unwrap_or_default();
        let mime = item["mimeType"].as_str().unwrap_or("application/octet-stream");
        match (item["text"].as_str(), item["blob"].as_str()) {
            (Some(text), _) => {
                push_text(&mut content, &mut text_len, text.to_string());
                contents.push(json!({ "uri": uri, "mimeType": mime, "text": text }));
            }
            (None, Some(blob)) => match mime.starts_with("image/").then(|| model_image(blob).ok()).flatten() {
                Some(image) => {
                    content.extend(image);
                    contents.push(json!({ "uri": uri, "mimeType": mime, "blob": blob }));
                }
                None => match save_blob(uri, blob) {
                    Ok((path, size)) => {
                        push_text(&mut content, &mut text_len, format!("[{uri}: {mime}, {size} bytes, saved to {}]", path.display()));
                        contents.push(json!({ "uri": uri, "mimeType": mime, "path": path.display().to_string() }));
                    }
                    Err(error) => push_text(&mut content, &mut text_len, format!("[{uri}: {mime}, which could not be saved: {error}]")),
                },
            },
            (None, None) => {}
        }
    }
    if content.is_empty() {
        content.push(ContentPart::text("(empty)"));
    }
    (content, json!({ "contents": contents }))
}

/// Saves a resource's binary contents to a private file of its own, for the bot's other tools.
fn save_blob(uri: &str, blob: &str) -> Result<(std::path::PathBuf, usize), String> {
    use base64::Engine;
    let bytes = base64::engine::general_purpose::STANDARD.decode(blob.trim()).map_err(|e| format!("its base64 does not read: {e}"))?;
    let dir = std::env::temp_dir().join("lorca-resources");
    std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    let name: String = uri.rsplit(['/', '\\', ':']).next().unwrap_or_default().chars().filter(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '-' | '_')).take(80).collect();
    let path = dir.join(format!("{}-{}", uuid::Uuid::new_v4().simple(), if name.trim_matches('.').is_empty() { "resource" } else { &name }));
    crate::config::write_private(&path, &bytes).map_err(|e| e.to_string())?;
    Ok((path, bytes.len()))
}

/// A plugin result as text and images: text, embedded text, and resource links as text cut at
/// `MAX_RESULT_CHARS` in all; images a model takes, embedded ones too, as images; other images,
/// audio, and other binary data named, never carried as base64; and the structured result when
/// there are no blocks. What an error says, and what hooks read.
fn model_content(result: &rmcp::model::CallToolResult) -> Vec<ContentPart> {
    use rmcp::model::ResourceContents;
    let mut content: Vec<ContentPart> = Vec::new();
    let mut text_len = 0;
    for block in &result.content {
        match block {
            ContentBlock::Text(text) => push_text(&mut content, &mut text_len, text.text.clone()),
            ContentBlock::Image(image) => match model_image(&image.data) {
                Ok(parts) => content.extend(parts),
                Err(why) => push_text(&mut content, &mut text_len, format!("[{} image, left out: {why}]", image.mime_type)),
            },
            ContentBlock::Audio(audio) => push_text(&mut content, &mut text_len, format!("[{} audio, left out]", audio.mime_type)),
            ContentBlock::Resource(embedded) => match &embedded.resource {
                ResourceContents::TextResourceContents { text, .. } => push_text(&mut content, &mut text_len, text.clone()),
                ResourceContents::BlobResourceContents { uri, mime_type: Some(mime), blob, .. } if mime.starts_with("image/") => match model_image(blob) {
                    Ok(parts) => content.extend(parts),
                    Err(why) => push_text(&mut content, &mut text_len, format!("[{uri}: {mime}, left out: {why}]")),
                },
                ResourceContents::BlobResourceContents { uri, mime_type, .. } => {
                    push_text(&mut content, &mut text_len, format!("[{uri}: {}, left out]", mime_type.as_deref().unwrap_or("binary data")))
                }
                other => push_text(&mut content, &mut text_len, serde_json::to_string(other).unwrap_or_default()),
            },
            ContentBlock::ResourceLink(link) => push_text(&mut content, &mut text_len, format!("[{} ({})]", link.uri, link.name)),
            #[allow(unreachable_patterns)]
            other => push_text(&mut content, &mut text_len, serde_json::to_string(other).unwrap_or_default()),
        }
    }
    if content.is_empty() {
        match &result.structured_content {
            Some(structured) => content.push(ContentPart::text(serde_json::to_string_pretty(structured).unwrap_or_default())),
            None => content.push(ContentPart::text("(no output)")),
        }
    }
    content
}

/// An image of a result as a model takes it (`images::prepare`), with a note when it changed on
/// the way, or why it cannot go: providers refuse a whole request over an image they do not
/// take, and the chat would fail at every later turn. A large one takes a moment, so results
/// are made off the async threads.
fn model_image(data: &str) -> Result<Vec<ContentPart>, String> {
    lorca_agent::images::prepare_base64(data).map(lorca_agent::images::Inline::into_parts)
}

/// Adds a result's text, cut where the result's text reaches `MAX_RESULT_CHARS`.
fn push_text(content: &mut Vec<ContentPart>, text_len: &mut usize, mut text: String) {
    if *text_len + text.len() > MAX_RESULT_CHARS {
        let mut cut = MAX_RESULT_CHARS.saturating_sub(*text_len);
        while cut > 0 && !text.is_char_boundary(cut) {
            cut -= 1;
        }
        text.truncate(cut);
        text.push_str("\n[truncated]");
    }
    *text_len += text.len();
    content.push(ContentPart::text(text));
}

/// Saves tokens the transport refreshed, so the next connection does not start from a stale
/// refresh token.
async fn persist_refreshed(app: &Arc<App>, plugin_id: &str, server: &str, auth: &Arc<tokio::sync::Mutex<AuthorizationManager>>, generation: u64) {
    let credentials = auth.lock().await.get_credentials().await;
    if let Ok((client_id, Some(tokens))) = credentials {
        let key = format!("oauth:{server}");
        let saved = app.plugins.lock().unwrap().secret(plugin_id, &key);
        let fresh = json!({ "client_id": client_id, "tokens": tidy_tokens(&json!(tokens)), "signed_in_at": now_secs() });
        if saved.as_ref().map(|s| s["tokens"] != fresh["tokens"]).unwrap_or(true) {
            let _ = super::set_oauth_at_generation(app, plugin_id, server, fresh, generation);
        }
    }
}

/// `create_issue · repo: lorca, title: Fix the relay`
fn call_summary(tool: &str, args: &Value) -> String {
    let mut parts = Vec::new();
    if let Some(object) = args.as_object() {
        for (key, value) in object.iter().take(4) {
            let text = match value {
                Value::String(s) => s.clone(),
                other => other.to_string(),
            };
            let text: String = text.split_whitespace().collect::<Vec<_>>().join(" ");
            let text = if text.chars().count() > 60 { format!("{}…", text.chars().take(60).collect::<String>()) } else { text };
            parts.push(format!("{key}: {text}"));
        }
    }
    if parts.is_empty() {
        tool.to_string()
    } else {
        format!("{tool} · {}", parts.join(", "))
    }
}

// MARK: - Permissions

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Decision {
    Allowed,
    Always,
    Denied,
    Expired,
    /// The user wrote in the chat instead of answering (`dismiss_questions`).
    Dismissed,
}

impl Decision {
    pub fn parse(text: &str) -> Option<Decision> {
        match text {
            "allowed" | "allow" | "once" => Some(Decision::Allowed),
            "always" => Some(Decision::Always),
            "denied" | "deny" => Some(Decision::Denied),
            _ => None,
        }
    }
    pub fn as_str(self) -> &'static str {
        match self {
            Decision::Allowed => "allowed",
            Decision::Always => "always",
            Decision::Denied => "denied",
            Decision::Expired => "expired",
            Decision::Dismissed => "dismissed",
        }
    }
}

/// Posts a permission card in the chat and waits for the user's answer, from this Device or
/// any paired one. `reason` is why Auto-review paused the action. `always` adds an
/// Auto-review rule for the exact tool before answering.
#[allow(clippy::too_many_arguments)]
pub async fn ask(app: &Arc<App>, chat_id: &str, bot_id: &str, plugin_id: &str, plugin_name: &str, tool: &str, summary: &str, arguments: Value, reason: Option<String>, cancel: &CancellationToken) -> Decision {
    let always_rule = (tool != "install").then(|| AutoReviewRule {
        id: uuid::Uuid::new_v4().to_string(),
        text: format!("use {plugin_name} {tool}"),
        behavior: "allow".into(),
        tool: Some(format!("{plugin_id}/{tool}")),
    });
    ask_with_rule(app, chat_id, bot_id, plugin_id, plugin_name, tool, summary, arguments, reason, always_rule, cancel).await
}

/// A permission card whose Always allow choice saves `always_rule`. A plain-language rule (the
/// one Auto-review proposed for a shell command) is shown on the card; without a rule, Always
/// allow is not offered and allows once.
#[allow(clippy::too_many_arguments)]
pub async fn ask_with_rule(
    app: &Arc<App>,
    chat_id: &str,
    bot_id: &str,
    plugin_id: &str,
    plugin_name: &str,
    tool: &str,
    summary: &str,
    arguments: Value,
    reason: Option<String>,
    always_rule: Option<AutoReviewRule>,
    cancel: &CancellationToken,
) -> Decision {
    let message = Message::new(
        chat_id,
        Author::Bot { bot_id: bot_id.to_string() },
        Body::Permission {
            plugin_id: plugin_id.to_string(),
            plugin_name: plugin_name.to_string(),
            tool: tool.to_string(),
            summary: summary.to_string(),
            arguments,
            decision: "pending".into(),
            reason,
            rule: always_rule.as_ref().filter(|rule| rule.tool.is_none()).map(|rule| rule.text.clone()),
            command: None,
            link: None,
            code: None,
        },
    );
    let decision = await_answer(app, chat_id, &message.id, always_rule, cancel, || {
        app.upsert_message(message.clone(), true);
        crate::push::permission(app, &message);
    })
    .await;
    if let Some(mut message) = app.message(chat_id, &message.id) {
        if let Body::Permission { decision: d, .. } = &mut message.body {
            // A question the turn or script stopped waiting on was never answered: it reads as
            // dismissed, not as the user's no, which Auto-review would learn from.
            *d = if cancel.is_cancelled() && decision == Decision::Denied { Decision::Dismissed } else { decision }.as_str().into();
        }
        app.upsert_message(message, true);
    }
    decision
}

/// Waits for the answer to the question row `message_id` asks in `chat_id`, from this Device
/// or any paired one (`chats.permission` names the row), for `PERMISSION_TIMEOUT` at most.
/// `ask` puts the question up once an answer can arrive; Stop answers Denied. A question never
/// holds back what the user wrote: one that would go up while a direct turn has a message of
/// theirs still to read is dismissed without asking, and a message that arrives while it waits
/// dismisses it (`dismiss_questions`). An Always allow adds `always_rule` before this returns.
pub async fn await_answer(app: &Arc<App>, chat_id: &str, message_id: &str, always_rule: Option<AutoReviewRule>, cancel: &CancellationToken, ask: impl FnOnce()) -> Decision {
    // A call stopped before its question went up puts nothing in the chat.
    if cancel.is_cancelled() {
        return Decision::Denied;
    }
    let (tx, rx) = tokio::sync::oneshot::channel();
    app.pending_permissions.lock().unwrap().insert(message_id.to_string(), (chat_id.to_string(), tx));
    // Checked once the question is registered, so a message landing in between still reaches it.
    if app.steering_queue(chat_id).is_some_and(|queue| !queue.is_empty()) {
        app.pending_permissions.lock().unwrap().remove(message_id);
        return Decision::Dismissed;
    }
    ask();
    let decision = tokio::select! {
        answer = rx => answer.unwrap_or(Decision::Denied),
        _ = tokio::time::sleep(PERMISSION_TIMEOUT) => Decision::Expired,
        _ = cancel.cancelled() => Decision::Denied,
    };
    app.pending_permissions.lock().unwrap().remove(message_id);
    if decision == Decision::Always {
        if let Some(rule) = always_rule {
            app.add_auto_review_rule(rule);
        }
    }
    decision
}

/// `chats.permission` reached the Runner: hands the answer to the waiting tool. False when
/// nothing waits on that message (answered already, or expired).
pub fn answer(app: &Arc<App>, message_id: &str, decision: Decision) -> bool {
    match app.pending_permissions.lock().unwrap().remove(message_id) {
        Some((_, tx)) => tx.send(decision).is_ok(),
        None => false,
    }
}

/// The user wrote in `chat_id` while questions there waited for their answer: each is
/// dismissed, so its action does not run and the turn goes on to read what they wrote.
pub fn dismiss_questions(app: &App, chat_id: &str) {
    let dismissed: Vec<_> = app.pending_permissions.lock().unwrap().extract_if(|_, (chat, _)| chat == chat_id).collect();
    for (_, (_, tx)) in dismissed {
        let _ = tx.send(Decision::Dismissed);
    }
}

/// What a call returns when the user left its question for a new message: the action did not
/// happen, and the turn stops after this batch unless that message is there to read next, as a
/// direct chat's steering puts it. A group member's turn ends, and the room that message started
/// answers it.
pub fn dismissed_call(reason: String) -> ToolResult {
    ToolResult { terminate: true, ..ToolResult::text(reason) }
}

#[cfg(test)]
mod review_tests;

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use tokio::io::{AsyncReadExt, AsyncWriteExt};

    /// An App over a scratch home, removed when the test ends.
    struct ScratchApp(Arc<App>, std::path::PathBuf);
    impl Drop for ScratchApp {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.1);
        }
    }

    fn scratch_app() -> ScratchApp {
        let home = std::env::temp_dir().join(format!("lorca-mcp-{}", uuid::Uuid::new_v4()));
        let app = App::load(crate::config::Config { home: home.clone(), port: 0 }).unwrap();
        ScratchApp(app, home)
    }

    fn mcp_tool(name: &str, description: &str, schema: &str) -> rmcp::model::Tool {
        serde_json::from_value(json!({ "name": name, "description": description, "inputSchema": serde_json::from_str::<Value>(schema).unwrap() })).unwrap()
    }

    /// A real MCP child records every tools/call. Policy errors cannot reach it, even when
    /// the account saved Always allow or a codemode script directly executes the bound tool.
    #[cfg(unix)]
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn bot_permissions_gate_real_mcp_calls_and_ignore_forged_cached_readonly() {
        use crate::permissions::{BotPermissions, Capability};
        use lorca_agent::{AgentContext, DirectRunner};
        use lorca_agent::types::{AssistantMessage, ToolCall};
        let scratch = scratch_app();
        let app = &scratch.0;
        crate::identity::create(app, Some("Runner".into())).unwrap();
        let bot = app.state.lock().unwrap().bots[0].clone();
        let chat_id = app.dm_with(&bot.id, None).unwrap().meta.id;
        let calls_path = scratch.1.join("mcp-calls");
        let script = r#"import json, sys
for line in sys.stdin:
    req = json.loads(line)
    if 'id' not in req: continue
    method = req['method']
    if method == 'initialize':
        result = {'protocolVersion':'2025-06-18','capabilities':{'tools':{}},'serverInfo':{'name':'mail','version':'1'}}
    elif method == 'tools/list':
        result = {'tools':[{'name':name,'inputSchema':{'type':'object','properties':{}},'annotations':{'readOnlyHint':name=='list_messages'}} for name in ['list_messages','create_draft','send_message','cached_readonly']]}
    elif method == 'tools/call':
        name = req['params']['name']
        with open(sys.argv[1],'a') as file: file.write(name+'\n')
        result = {'content':[{'type':'text','text':name+' ran'}]}
    else: result = {}
    print(json.dumps({'jsonrpc':'2.0','id':req['id'],'result':result}), flush=True)
"#;
        let mut cached: Vec<_> = ["list_messages", "create_draft", "send_message", "cached_readonly"].iter().map(|name| mcp_tool(name, name, r#"{"type":"object","properties":{}}"#)).collect();
        for tool in &mut cached {
            // The disk claims all calls only read; only the live child's mark is authority.
            tool.annotations = Some(serde_json::from_value(json!({"readOnlyHint": true})).unwrap());
        }
        crate::config::write_json_private(&catalog_path(app, "mail-work"), &SavedCatalog { servers: BTreeMap::from([("api".into(), SavedServer { instructions: None, tools: cached, resources: true })]) }).unwrap();
        let manifest = super::super::Manifest::parse(&json!({
            "id": "mail-work", "name": "Mail Work", "servers": {"api": {"type":"stdio","command":"python3","args":["-u","-c",script,calls_path]}},
            "tools": {"draft": ["create_draft"]}
        })).unwrap();
        super::super::install(app, manifest, "inline").unwrap();
        let policy: BotPermissions = serde_json::from_value(json!({"connections":{"mail-work":{"capabilities":["read","draft"]}},"shell":false,"filesystem":"none"})).unwrap();
        app.update_bot(&bot.id, |bot| bot.permissions = Some(policy.clone())).unwrap();
        // The Access sheet lists the instance by its name with the tools it last offered.
        let listed = crate::api::dispatch(app, "bots.permissions", json!({"id": bot.id})).await.unwrap();
        let connection = listed["connections"].as_array().unwrap().iter().find(|entry| entry["id"] == "mail-work").unwrap();
        assert_eq!(connection["name"], "Mail Work");
        assert_eq!(connection["tools"].as_array().unwrap().len(), 4);
        app.add_auto_review_rule(AutoReviewRule { id: "always".into(), text: "Always send".into(), behavior: "allow".into(), tool: Some("mail-work/send_message".into()) });
        let catalog = bot_catalog(app, &bot, &chat_id, Vec::new());
        let cancel = CancellationToken::new();
        let assistant = AssistantMessage::empty("test", "test");
        let context = AgentContext { system_prompt: String::new(), messages: vec![], tools: vec![], cache_points: vec![] };
        let args = json!({});
        for name in ["send_message", "cached_readonly"] {
            let name = tool_name("mail-work", name);
            let call = ToolCall { id: name.clone(), name: name.clone(), arguments: args.clone() };
            let parent = ToolCall { id: "script".into(), name: "codemode".into(), arguments: json!({"code":"await tools.mail_work__send_message({})"}) };
            let ctx = BeforeToolCallContext { assistant_message: &assistant, tool_call: &call, args: &args, context: &context, cancel: &cancel, parent: Some(&parent) };
            let refused = review_call(app, &catalog, &chat_id, &super::super::review::Trigger::default(), &bot, false, &ctx).await.unwrap();
            assert!(refused.block && refused.reason.unwrap().contains("write access"));
            assert!(catalog.plugin_tool(&name).unwrap().execute("direct", args.clone(), cancel.clone(), Arc::new(|_| {})).await.is_err());
        }
        assert!(!calls_path.exists(), "no denied MCP call ran");
        assert!(authorize_catalog_tool(app, &catalog, &bot, "mail_work__create_draft", &cancel).await.is_ok());
        assert!(matches!(access(app, &catalog.plugin_tool("mail_work__create_draft").unwrap(), &cancel).await, Access::Changes { capability: Capability::Draft, .. }));
        let script = "const result = await tools.mail_work__list_messages({}); text(result);";
        let codemode = lorca_agent::codemode::CodemodeTool::new(catalog.clone(), Default::default());
        let run = codemode.run_script("read", script, cancel.clone(), &DirectRunner).await.unwrap();
        assert!(!run.result.is_error, "{}", run.result.text_content());
        assert_eq!(std::fs::read_to_string(&calls_path).unwrap(), "list_messages\n");
        // A resource tool shares its connection's grants too. Revoking during the turn wins.
        app.update_bot(&bot.id, |bot| bot.permissions = Some(BotPermissions { connections: Some(BTreeMap::new()), ..policy })).unwrap();
        for name in ["list_messages", "create_draft", "read_mcp_resource"] {
            let tool = catalog.plugin_tool(&tool_name("mail-work", name)).unwrap();
            assert!(tool.execute("revoked", args.clone(), cancel.clone(), Arc::new(|_| {})).await.is_err());
        }
        assert_eq!(std::fs::read_to_string(&calls_path).unwrap(), "list_messages\n", "revoked calls reach no service");
        // The next turn's catalog leaves the plugin out, so a search starts nothing.
        let next = bot_catalog(app, &bot, &chat_id, Vec::new());
        assert!(next.plugin_tool(&tool_name("mail-work", "list_messages")).is_none());
        assert!(codemode::Catalog::search(&*next, "messages", None, 10, &cancel).await.unwrap().is_empty());
    }

    fn catalog_tool(app: &Arc<App>, plugin_id: &str, plugin_name: &str, original_name: &str, description: &str, schema: &str) -> Arc<CatalogTool> {
        let name = tool_name(plugin_id, original_name);
        let tool = mcp_tool(original_name, description, schema);
        Arc::new(CatalogTool {
            name: name.clone(),
            original_name: original_name.into(),
            plugin_id: plugin_id.into(),
            plugin_name: plugin_name.into(),
            server_name: "test".into(),
            description: description.into(),
            search_schema: schema.into(),
            server_instructions: String::new(),
            tool: Arc::new(PluginTool {
                budget: None,
                app: app.clone(),
                policy_context: None,
                plugin_id: plugin_id.into(),
                plugin_name: plugin_name.into(),
                server_name: "test".into(),
                tool,
                name,
                description: description.into(),
                read_only: true,
                kind: ToolKind::Call,
                timeout: super::super::CALL_TIMEOUT,
            }),
        })
    }

    async fn token_endpoint(body: &str) -> (String, tokio::sync::oneshot::Receiver<String>) {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let response_body = body.to_string();
        let (seen_tx, seen_rx) = tokio::sync::oneshot::channel();
        tokio::spawn(async move {
            let (mut socket, _) = listener.accept().await.unwrap();
            let mut request = Vec::new();
            let mut buffer = [0_u8; 4096];
            loop {
                let count = socket.read(&mut buffer).await.unwrap();
                if count == 0 {
                    break;
                }
                request.extend_from_slice(&buffer[..count]);
                let Some(header_end) = request.windows(4).position(|part| part == b"\r\n\r\n").map(|position| position + 4) else { continue };
                let headers = String::from_utf8_lossy(&request[..header_end]);
                let content_length = headers
                    .lines()
                    .find_map(|line| {
                        let (name, value) = line.split_once(':')?;
                        name.eq_ignore_ascii_case("content-length").then(|| value.trim().parse::<usize>().ok()).flatten()
                    })
                    .unwrap_or(0);
                if request.len() >= header_end + content_length {
                    break;
                }
            }
            let _ = seen_tx.send(String::from_utf8_lossy(&request).into_owned());
            let reply = format!("HTTP/1.1 200 OK\r\nContent-Type: application/x-www-form-urlencoded\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{response_body}", response_body.len());
            socket.write_all(reply.as_bytes()).await.unwrap();
        });
        (format!("http://{address}/token"), seen_rx)
    }

    #[test]
    fn a_program_that_stopped_says_why_in_its_last_lines() {
        let node = "/srv/index.js:3\n    throw new Error(\"GITHUB_TOKEN is not set\");\n    ^\n\nError: GITHUB_TOKEN is not set\n    at Object.<anonymous> (/srv/index.js:3:11)\n    at Module._compile (node:internal/modules/cjs/loader:1554:14)\n\nNode.js v22.14.0";
        assert_eq!(last_words(node.lines()).as_deref(), Some("/srv/index.js:3 · throw new Error(\"GITHUB_TOKEN is not set\"); · Error: GITHUB_TOKEN is not set · Node.js v22.14.0"));
        let python = "Traceback (most recent call last):\n  File \"/srv/server.py\", line 1, in <module>\n    import httpx\nModuleNotFoundError: No module named 'httpx'";
        assert!(last_words(python.lines()).unwrap().ends_with("import httpx · ModuleNotFoundError: No module named 'httpx'"));
        let chatty: Vec<String> = (1..=10).map(|n| format!("line {n}")).collect();
        assert_eq!(last_words(chatty.iter().map(String::as_str)).as_deref(), Some("line 7 · line 8 · line 9 · line 10"));
        let long = last_words([format!("Error: {}", "x".repeat(500)).as_str()].into_iter()).unwrap();
        assert!(long.starts_with('…') && long.ends_with('x') && long.chars().count() == 400, "a long tail keeps its end");
        assert_eq!(last_words(["", "  ", "    at frame (x.js:1:1)"].into_iter()), None);
    }

    /// A 1×1 PNG, and text labeled an image, which no system reads as one, in base64.
    const PNG: &str = "iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAYAAAAfFcSJAAAADUlEQVR42mNkYPhfDwAChwGA60e6kgAAAABJRU5ErkJggg==";
    const NOT_AN_IMAGE: &str = "bm90IGFuIGltYWdl";

    /// A Browser server over an in-memory MCP transport: every call answers at once with text, a
    /// screenshot with a PNG, and `browser_wait_for` only after 300 ms. It records the calls.
    pub(crate) async fn fake_browser(app: &Arc<App>) -> (Arc<Server>, Arc<Mutex<Vec<String>>>) {
        use tokio::io::{AsyncBufReadExt, AsyncWriteExt};
        let (client_io, server_io) = tokio::io::duplex(16384);
        let calls = Arc::new(Mutex::new(Vec::new()));
        let recorded = calls.clone();
        let (read, mut write) = tokio::io::split(server_io);
        let (answers, mut outgoing) = tokio::sync::mpsc::unbounded_channel::<String>();
        tokio::spawn(async move {
            while let Some(line) = outgoing.recv().await {
                if write.write_all(line.as_bytes()).await.is_err() {
                    break;
                }
            }
        });
        tokio::spawn(async move {
            let mut lines = tokio::io::BufReader::new(read).lines();
            while let Ok(Some(line)) = lines.next_line().await {
                let request: Value = serde_json::from_str(&line).unwrap();
                let Some(id) = request.get("id").cloned() else { continue };
                let name = request["params"]["name"].as_str().unwrap_or_default().to_string();
                let result = match request["method"].as_str() {
                    Some("initialize") => json!({ "protocolVersion": request["params"]["protocolVersion"], "capabilities": { "tools": {} }, "serverInfo": { "name": "fake-browser", "version": "1" } }),
                    Some("tools/list") => json!({ "tools": [] }),
                    Some("tools/call") => {
                        recorded.lock().unwrap().push(name.clone());
                        if name == "browser_take_screenshot" {
                            json!({ "content": [{ "type": "image", "data": PNG, "mimeType": "image/png" }] })
                        } else {
                            json!({ "content": [{ "type": "text", "text": "ok" }] })
                        }
                    }
                    _ => json!({}),
                };
                let line = format!("{}\n", json!({ "jsonrpc": "2.0", "id": id, "result": result }));
                let answers = answers.clone();
                tokio::spawn(async move {
                    if name == "browser_wait_for" {
                        tokio::time::sleep(std::time::Duration::from_millis(300)).await;
                    }
                    let _ = answers.send(line);
                });
            }
        });
        let service = Client { info: ClientConfig::default(), app: Arc::downgrade(app), plugin_id: crate::browser::PLUGIN_ID.into(), server: "browser".into(), generation: app.mcp.generation(crate::browser::PLUGIN_ID) }.serve(client_io).await.unwrap();
        (Arc::new(Server {
            plugin_id: crate::browser::PLUGIN_ID.into(), name: "browser".into(), service,
            tools: std::sync::RwLock::new(Vec::new()), instructions: None, resources: false, auth: None, bearer_expires_at: None,
            generation: app.mcp.generation(crate::browser::PLUGIN_ID),
        }), calls)
    }

    /// One of the Browser plugin's tools as a bot's turn has it.
    pub(crate) fn browser_tool(app: &Arc<App>, bot: &Bot, chat_id: &str, name: &str) -> PluginTool {
        PluginTool {
            app: app.clone(),
            budget: crate::budgets::current(),
            plugin_id: crate::browser::PLUGIN_ID.into(),
            plugin_name: "Browser".into(),
            server_name: "browser".into(),
            tool: mcp_tool(name, "", r#"{"type":"object"}"#),
            name: tool_name(crate::browser::PLUGIN_ID, name),
            description: String::new(),
            read_only: false,
            kind: ToolKind::Call,
            timeout: super::super::CALL_TIMEOUT,
            policy_context: Some((bot.clone(), chat_id.into())),
        }
    }

    /// A small JPEG, in base64.
    fn jpeg() -> String {
        let mut jpeg = Vec::new();
        image::DynamicImage::ImageRgb8(image::ImageBuffer::from_pixel(8, 8, image::Rgb([20, 120, 220]))).write_to(&mut std::io::Cursor::new(&mut jpeg), image::ImageFormat::Jpeg).unwrap();
        base64::Engine::encode(&base64::engine::general_purpose::STANDARD, jpeg)
    }

    #[test]
    fn results_reach_the_model_without_binary_data() {
        let result: rmcp::model::CallToolResult = serde_json::from_value(json!({ "content": [
            { "type": "text", "text": "hello" },
            { "type": "audio", "data": "QVVESU8=", "mimeType": "audio/wav" },
            { "type": "resource", "resource": { "uri": "file:///a.txt", "mimeType": "text/plain", "text": "embedded" } },
            { "type": "resource", "resource": { "uri": "file:///a.pdf", "mimeType": "application/pdf", "blob": "JVBERi0=" } },
            { "type": "resource", "resource": { "uri": "file:///a.png", "mimeType": "image/png", "blob": PNG } },
            { "type": "resource_link", "uri": "file:///b.md", "name": "b.md" },
            { "type": "image", "data": NOT_AN_IMAGE, "mimeType": "image/svg+xml" },
            { "type": "image", "data": jpeg(), "mimeType": "image/png" },
            { "type": "resource", "resource": { "uri": "file:///c.png", "mimeType": "image/png", "blob": "iVBORw0=" } }
        ] }))
        .unwrap();
        let content = model_content(&result);
        let texts: Vec<&str> = content.iter().filter_map(|part| part.as_text()).collect();
        assert_eq!(
            texts,
            [
                "hello",
                "[audio/wav audio, left out]",
                "embedded",
                "[file:///a.pdf: application/pdf, left out]",
                "[file:///b.md (b.md)]",
                "[image/svg+xml image, left out: it is not an image Lorca can read]",
                "[file:///c.png: image/png, left out: it is not an image Lorca can read]"
            ]
        );
        let types: Vec<&str> = content.iter().filter_map(|part| if let ContentPart::Image { mime_type, .. } = part { Some(mime_type.as_str()) } else { None }).collect();
        assert_eq!(types, ["image/png", "image/jpeg"], "an embedded image is an image, and an image has the type its bytes say");
        let shown = format!("{content:?}");
        assert!(!shown.contains("QVVESU8=") && !shown.contains("JVBERi0=") && !shown.contains(NOT_AN_IMAGE), "audio and other binary data never reach the model as text");
    }

    /// A server at the other end of `io` that answers the handshake and lists one tool a page,
    /// with the next cursor `next` gives for the cursor it was asked with. Counts its pages.
    fn paging_server(io: tokio::io::DuplexStream, next: fn(Option<&str>) -> Option<String>, pages: Arc<std::sync::atomic::AtomicUsize>) {
        tokio::spawn(async move {
            use tokio::io::AsyncBufReadExt;
            let (read, mut write) = tokio::io::split(io);
            let mut lines = tokio::io::BufReader::new(read).lines();
            while let Ok(Some(line)) = lines.next_line().await {
                let message: Value = serde_json::from_str(&line).unwrap();
                let result = match message["method"].as_str() {
                    Some("initialize") => {
                        json!({ "protocolVersion": message["params"]["protocolVersion"], "capabilities": { "tools": {} }, "serverInfo": { "name": "pager", "version": "1" } })
                    }
                    Some("tools/list") => {
                        let page = pages.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
                        let mut result = json!({ "tools": [{ "name": format!("tool{page}"), "inputSchema": { "type": "object" } }] });
                        if let Some(cursor) = next(message["params"]["cursor"].as_str()) {
                            result["nextCursor"] = json!(cursor);
                        }
                        result
                    }
                    _ => continue,
                };
                if write.write_all(format!("{}\n", json!({ "jsonrpc": "2.0", "id": message["id"], "result": result })).as_bytes()).await.is_err() {
                    return;
                }
            }
        });
    }

    #[tokio::test]
    async fn a_tool_list_ends_where_its_server_stops_paging() {
        use rmcp::ServiceExt;
        async fn list(next: fn(Option<&str>) -> Option<String>) -> (usize, usize) {
            let (client, server) = tokio::io::duplex(64 * 1024);
            let pages = Arc::new(std::sync::atomic::AtomicUsize::new(0));
            paging_server(server, next, pages.clone());
            let service = ().serve(client).await.unwrap();
            let tools = tokio::time::timeout(std::time::Duration::from_secs(20), all_tools(service.peer())).await.expect("the listing ends").unwrap();
            (tools.len(), pages.load(std::sync::atomic::Ordering::SeqCst))
        }
        assert_eq!(list(|_| None).await, (1, 1));
        assert_eq!(list(|cursor| (cursor.is_none()).then(|| "2".into())).await, (2, 2), "a list of two pages");
        assert_eq!(list(|_| Some(String::new())).await, (1, 1), "an empty cursor ends the list");
        assert_eq!(list(|_| Some("again".into())).await, (2, 2), "a cursor handed out before ends it too");
        let endless = list(|cursor| Some(format!("{}", cursor.and_then(|cursor| cursor.parse::<u32>().ok()).unwrap_or(0) + 1))).await;
        assert_eq!(endless, (MAX_TOOL_PAGES, MAX_TOOL_PAGES), "a server that pages forever is read so far");
    }

    #[test]
    fn empty_token_fields_are_left_out() {
        let tidy = tidy_tokens(&json!({ "access_token": "a", "token_type": "Bearer", "scope": "", "refresh_token": " ", "id_token": null, "expires_in": null, "refresh_token_expires_in": 60 }));
        assert_eq!(tidy, json!({ "access_token": "a", "token_type": "Bearer", "refresh_token_expires_in": 60 }));
        let tokens: rmcp::transport::auth::OAuthTokenResponse = serde_json::from_value(tidy).expect("rmcp reads what is left");
        assert!(serde_json::to_value(tokens).unwrap().get("scope").is_none(), "no empty scope to ask for again at a refresh");
        assert_eq!(tidy_tokens(&json!({ "access_token": "a", "scope": "repo read:org" }))["scope"], json!("repo read:org"));
    }

    #[tokio::test]
    async fn a_server_that_stops_before_the_handshake_says_why() {
        let scratch = scratch_app();
        let app = &scratch.0;
        let (command, args) = if cfg!(windows) {
            ("cmd", vec!["/d", "/c", "echo Error: GITHUB_TOKEN is not set 1>&2 & exit 1"])
        } else {
            ("sh", vec!["-c", "echo 'Error: GITHUB_TOKEN is not set' >&2; exit 1"])
        };
        let manifest = super::super::Manifest::parse(&json!({ "id": "boom", "name": "Boom", "servers": { "main": { "type": "stdio", "command": command, "args": args } } })).unwrap();
        super::super::install(app, manifest, "inline").unwrap();
        let error = app.mcp.server(app, "boom", "main").await.err().unwrap();
        assert!(error.contains("did not answer the MCP handshake: Error: GITHUB_TOKEN is not set"), "{error}");
    }

    #[test]
    fn a_server_that_wants_more_access_asks_for_it_at_the_next_sign_in() {
        let refusal = std::io::Error::other("Transport send error: insufficient scope: Bearer error=\"insufficient_scope\", scope=\"read:org\", resource_metadata=\"https://hub.test/.well-known/x\"");
        let (scope, challenge) = insufficient_scope(&refusal).unwrap();
        assert_eq!(scope, "read:org");
        assert!(challenge.starts_with("Bearer error=\"insufficient_scope\""), "{challenge}");
        assert!(insufficient_scope(&std::io::Error::other("connection refused")).is_none());

        let scratch = scratch_app();
        let app = &scratch.0;
        let manifest = super::super::Manifest::parse(&json!({ "id": "hub", "name": "Hub", "servers": { "api": { "type": "http", "url": "https://hub.test/mcp", "auth": { "type": "oauth" } } } })).unwrap();
        super::super::install(app, manifest, "inline").unwrap();
        super::super::set_oauth(app, "hub", "api", Some(json!({ "client_id": "c", "tokens": { "access_token": "a", "scope": "repo read:org" } }))).unwrap();
        assert_eq!(app.plugins.lock().unwrap().status("hub").unwrap().state, "ready");
        needs_more_access(app, "hub", "api", "admin:org read:org", &challenge);
        let store = app.plugins.lock().unwrap();
        assert_eq!(store.secret("hub", "scope:api").unwrap()["scope"], json!("repo read:org admin:org"), "what it had and what it needs, once each");
        assert!(store.sign_in_secret("hub", "oauth", "api").is_none());
        assert_eq!(store.status("hub").unwrap().state, "insufficient_access");
    }

    #[test]
    fn a_server_that_offers_resources_gets_tools_to_read_them() {
        let scratch = scratch_app();
        let app = &scratch.0;
        for id in ["docs", "plain"] {
            let manifest = super::super::Manifest::parse(&json!({ "id": id, "name": id, "servers": { "api": { "type": "stdio", "command": "false" } } })).unwrap();
            super::super::install(app, manifest, "inline").unwrap();
            let saved = SavedCatalog { servers: BTreeMap::from([("api".to_string(), SavedServer { instructions: None, tools: vec![mcp_tool("search", "Search", r#"{"type":"object"}"#)], resources: id == "docs" })]) };
            crate::config::write_json_private(&catalog_path(app, id), &saved).unwrap();
        }
        let catalog = turn_catalog(app, Vec::new());
        let names: Vec<String> = codemode::Catalog::entries(catalog.as_ref()).iter().map(|entry| entry.tool.name().to_string()).collect();
        assert_eq!(names, ["docs__list_mcp_resource_templates", "docs__list_mcp_resources", "docs__read_mcp_resource", "docs__search", "plain__search"]);
        let read = catalog.plugin_tool("docs__read_mcp_resource").unwrap();
        assert!(read.read_only && read.kind == ToolKind::ReadResource, "reading a resource never asks");
        assert_eq!(read.parameters()["required"], json!(["uri"]));
        assert_eq!(saved_tool_count(app, &app.plugins.lock().unwrap().get("docs").unwrap().clone()), Some(1), "the server's own tools are what it counts");
    }

    #[test]
    fn resources_read_as_text_images_and_saved_files() {
        let page = listed(
            json!({ "resources": [
                { "uri": "file:///a.md", "name": "a.md", "mimeType": "text/markdown", "icons": [{ "src": "x" }], "_meta": { "x": 1 } },
                { "uri": "ui://widget", "name": "widget" },
                { "uri": "app://b", "name": "b", "mimeType": "text/html;profile=mcp-app" }
            ], "nextCursor": "2" }),
            "resources",
        );
        assert_eq!(page, json!({ "resources": [{ "uri": "file:///a.md", "name": "a.md", "mimeType": "text/markdown" }], "nextCursor": "2" }));
        use base64::Engine;
        let pdf = base64::engine::general_purpose::STANDARD.encode(b"%PDF-1.7 tiny");
        let (content, structured) = read_contents(json!({ "contents": [
            { "uri": "file:///a.md", "mimeType": "text/markdown", "text": "# A" },
            { "uri": "file:///b.png", "mimeType": "image/png", "blob": PNG },
            { "uri": "file:///docs/report.pdf", "mimeType": "application/pdf", "blob": pdf },
            { "uri": "file:///logo.svg", "mimeType": "image/svg+xml", "blob": NOT_AN_IMAGE }
        ] }));
        assert_eq!(content[0].as_text(), Some("# A"));
        assert!(matches!(&content[1], ContentPart::Image { mime_type, .. } if mime_type == "image/png"));
        let saved = structured["contents"][2]["path"].as_str().unwrap().to_string();
        assert!(saved.ends_with("-report.pdf") && content[2].as_text().unwrap().contains("13 bytes, saved to"), "{:?}", content[2]);
        assert_eq!(std::fs::read(&saved).unwrap(), b"%PDF-1.7 tiny");
        assert!(structured["contents"][2].get("blob").is_none(), "scripts get the file, not the base64");
        let logo = structured["contents"][3]["path"].as_str().unwrap().to_string();
        assert!(logo.ends_with("-logo.svg") && content[3].as_text().unwrap().contains("saved to"), "an image no model takes is saved like other binary contents: {:?}", content[3]);
        let _ = std::fs::remove_file(saved);
        let _ = std::fs::remove_file(logo);
    }

    /// A server at a local port that answers every request with `status` and no body, counting
    /// the POSTs it got.
    async fn answering(status: &'static str) -> (String, Arc<std::sync::atomic::AtomicUsize>) {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let posts = Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let counted = posts.clone();
        tokio::spawn(async move {
            while let Ok((mut socket, _)) = listener.accept().await {
                let counted = counted.clone();
                tokio::spawn(async move {
                    let mut request = Vec::new();
                    let mut buffer = [0_u8; 4096];
                    loop {
                        let Ok(count) = socket.read(&mut buffer).await else { return };
                        if count == 0 {
                            break;
                        }
                        request.extend_from_slice(&buffer[..count]);
                        let Some(end) = request.windows(4).position(|part| part == b"\r\n\r\n").map(|at| at + 4) else { continue };
                        let headers = String::from_utf8_lossy(&request[..end]).to_ascii_lowercase();
                        let length = headers.lines().find_map(|line| line.strip_prefix("content-length:")?.trim().parse::<usize>().ok()).unwrap_or(0);
                        if request.len() >= end + length {
                            break;
                        }
                    }
                    if request.starts_with(b"POST") {
                        counted.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
                    }
                    let _ = socket.write_all(format!("HTTP/1.1 {status}\r\nContent-Length: 0\r\nConnection: close\r\n\r\n").as_bytes()).await;
                });
            }
        });
        (format!("http://{address}/mcp"), posts)
    }

    #[tokio::test]
    async fn a_server_in_passing_trouble_is_tried_again_and_a_refusal_is_not() {
        let scratch = scratch_app();
        let app = &scratch.0;
        for (id, status, tries) in [("busy", "503 Service Unavailable", 3), ("refusing", "403 Forbidden", 1)] {
            let (url, posts) = answering(status).await;
            let manifest = super::super::Manifest::parse(&json!({ "id": id, "name": id, "servers": { "api": { "type": "http", "url": url } } })).unwrap();
            // Not `install`, whose background connection would try too.
            app.plugins.lock().unwrap().installed.push(super::super::Installed { manifest, source: "inline".into(), installed_at: 0.0, variables: BTreeMap::new(), service_id: None, account_name: None });
            assert!(app.mcp.server(app, id, "api").await.is_err());
            assert_eq!(posts.load(std::sync::atomic::Ordering::SeqCst), tries, "{status}");
        }
    }

    #[test]
    fn tool_names_fit_the_providers() {
        assert_eq!(tool_name("github", "create_issue"), "github__create_issue");
        assert_eq!(tool_name("my-server", "weird.name/x"), "my_server__weird_name_x");
        assert_eq!(tool_name("p", "a-b"), tool_name("p", "a_b"), "names are identifiers, so a collision gets a suffix in the catalog");
        assert_eq!(tool_name("p", &"x".repeat(100)).len(), 64);
        // serde_json keeps object keys sorted, so the summary lists them alphabetically.
        assert_eq!(call_summary("create_issue", &json!({ "repo": "lorca", "title": "Fix   the relay", "body": "x".repeat(80) })), format!("create_issue · body: {}…, repo: lorca, title: Fix the relay", "x".repeat(60)));
        assert_eq!(call_summary("get_me", &json!({})), "get_me");
        assert_eq!(Decision::parse("always"), Some(Decision::Always));
        assert_eq!(Decision::parse("nope"), None);
    }

    #[test]
    fn plugin_search_ranks_intent_and_exact_identity() {
        let scratch = scratch_app();
        let app = &scratch.0;
        let tools = vec![
            catalog_tool(
                app,
                "github",
                "GitHub",
                "create_issue",
                "Create an issue in a GitHub repository",
                r#"{"type":"object","properties":{"repo":{"type":"string"},"title":{"type":"string"}}}"#,
            ),
            catalog_tool(
                app,
                "github",
                "GitHub",
                "list_pull_requests",
                "List pull requests in a repository",
                r#"{"type":"object","properties":{"repo":{"type":"string"}}}"#,
            ),
            catalog_tool(
                app,
                "linear",
                "Linear",
                "create_issue",
                "Create an issue in a Linear team",
                r#"{"type":"object","properties":{"team":{"type":"string"},"title":{"type":"string"}}}"#,
            ),
        ];

        let ranked = rank_tools("create a GitHub issue", &tools, false);
        assert_eq!(ranked.first().map(|tool| tool.name.as_str()), Some("github__create_issue"));

        let exact = rank_tools("use github__list_pull_requests", &tools, false);
        assert_eq!(exact.first().map(|tool| tool.name.as_str()), Some("github__list_pull_requests"));
        assert!(rank_tools("send a calendar invitation", &tools, false).is_empty());
    }

    #[test]
    fn search_text_bounds_utf8_and_names_without_retargeting() {
        let scratch = scratch_app();
        assert_eq!(utf8_prefix("aéz", 2), "a");
        assert_eq!(query_tokens("GitHub github issue"), vec!["github", "issue"]);

        let first = catalog_tool(&scratch.0, "github", "GitHub", "create.issue", "Create", r#"{"type":"object"}"#);
        let mut catalog = BTreeMap::new();
        catalog.insert(first.name.clone(), first.clone());
        assert_eq!(unique_tool_name(&first.name, &catalog), "github__create_issue_2");
    }

    /// A turn lists a plugin's tools from what its server offered last time, without starting
    /// it, and names a plugin that never connected.
    #[test]
    fn a_turns_catalog_lists_saved_tools_without_connecting() {
        use lorca_agent::codemode::{Catalog, CodemodeOptions, CodemodeTool};
        let scratch = scratch_app();
        let app = &scratch.0;
        for (id, name) in [("linear", "Linear"), ("notion", "Notion")] {
            let manifest = super::super::Manifest::parse(&json!({
                "id": id, "name": name, "description": format!("{name} for the team.\nMore."),
                "servers": { "api": { "type": "stdio", "command": "false" } },
                "tools": { "readonly": ["list_*"], "hide": ["secret_*"] }
            }))
            .unwrap();
            super::super::install(app, manifest, "inline").unwrap();
        }
        let issues = mcp_tool("list_issues", "List issues in a team", r#"{"type":"object","properties":{"team":{"type":"string","description":"Team key"}},"required":["team"]}"#);
        let comment = mcp_tool("create_comment", "Comment on an issue", r#"{"type":"object","properties":{"issue":{"type":"string"},"body":{"type":"string"}}}"#);
        let hidden = mcp_tool("secret_admin", "Never offered", r#"{"type":"object"}"#);
        let instructions = format!("Use team keys like ENG.{}", " Cycles start on Monday.".repeat(60));
        let saved = SavedCatalog {
            servers: BTreeMap::from([
                ("api".to_string(), SavedServer { instructions: Some(instructions.clone()), tools: vec![issues, comment, hidden], resources: false }),
                ("gone".to_string(), SavedServer { instructions: None, tools: vec![mcp_tool("old", "A server the manifest no longer has", r#"{}"#)], resources: false }),
            ]),
        };
        crate::config::write_json_private(&catalog_path(app, "linear"), &saved).unwrap();

        let mut local: Vec<Arc<dyn Tool>> = lorca_agent::tools::coding_tools(scratch.1.clone()).into_iter().filter(|tool| tool.name() == "read").collect();
        // The scripts' own bash: one command at a time, resolving to its output and exit code.
        let bash = crate::shell::script_bash(app, &scratch.1);
        assert_eq!(bash.execution_mode(), Some(lorca_agent::agent_loop::ToolExecutionMode::Sequential));
        assert!(bash.output_schema().is_some() && !bash.description().contains("session id"), "{}", bash.description());
        local.push(bash);
        let catalog = turn_catalog(app, local);
        assert_eq!(plugin_briefs(app).len(), 2);
        let names: Vec<String> = catalog.entries().iter().map(|entry| entry.tool.name().to_string()).collect();
        assert_eq!(names, vec!["read", "bash", "linear__create_comment", "linear__list_issues"]);
        assert_eq!(catalog.plugin_name("linear__list_issues").as_deref(), Some("Linear"));
        assert!(catalog.plugin_tool("linear__list_issues").unwrap().read_only, "the manifest's readonly pattern applies");
        assert!(!catalog.plugin_tool("linear__create_comment").unwrap().read_only);
        assert_eq!(catalog.lookup("linear__list_issues").map(|tool| tool.original_name.clone()).as_deref(), Some("list_issues"));

        let codemode = CodemodeTool::new(catalog.clone(), CodemodeOptions::default());
        let description = codemode.description().to_string();
        assert!(description.contains("## linear\nLinear: Linear for the team.\nServer instructions: Use team keys like ENG."), "{description}");
        assert!(description.contains("linear__list_issues(args: {\n  // Team key\n  team: string;\n}): Promise<CallToolResult>;"), "{description}");
        assert!(description.contains("## notion (tools not known yet; describeNamespace() lists them)\nNotion: Notion for the team."), "{description}");
        assert!(
            description.contains(
                "Your own tools `read`, `bash` are callable here too, with the same arguments. `bash` resolves to \
                 `{ exit_code, full_output_path?, output, truncated, wall_time_seconds }`; the others resolve to their text output."
            ),
            "{description}"
        );
        assert!(!description.contains("secret_admin") && !description.contains("linear__old"), "{description}");
        assert!(description.contains("… (describeNamespace(\"linear\") has the rest)"), "long instructions are cut: {description}");
        assert_eq!(CodemodeTool::new(catalog.clone(), CodemodeOptions::default()).description(), description, "the same listing every time");

        // describeNamespace() gives the instructions whole, apart from what the plugin is.
        let runtime = tokio::runtime::Builder::new_current_thread().enable_all().build().unwrap();
        let details = runtime.block_on(catalog.describe_namespace("linear", &CancellationToken::new())).unwrap();
        assert_eq!(details.description, "Linear: Linear for the team.");
        assert_eq!(details.instructions, instructions);
        assert_eq!(details.tools, vec!["linear__create_comment", "linear__list_issues"]);
        assert!(runtime.block_on(catalog.describe_namespace("jira", &CancellationToken::new())).is_none());
    }

    #[tokio::test]
    async fn device_bearer_refreshes_with_githubs_request_shape_and_rotates_tokens() {
        let (endpoint, seen) = token_endpoint(
            "access_token=new-access&expires_in=28800&refresh_token=new-refresh&refresh_token_expires_in=15897600&scope=repo%2Cread%3Auser&token_type=bearer",
        )
        .await;
        let stored = json!({
            "client_id": "client-id",
            "device_flow": true,
            "signed_in_at": 1,
            "tokens": {
                "access_token": "old-access",
                "expires_in": 1,
                "refresh_token": "old-refresh",
                "scope": "repo,read:user",
                "token_type": "bearer"
            }
        });

        let (bearer, saved) = refresh_device_bearer(&reqwest::Client::new(), &endpoint, "GitHub", &stored).await.unwrap();
        let saved = saved.expect("an expired bearer is refreshed");
        assert_eq!(bearer.access_token, "new-access");
        assert!(bearer.expires_at.is_some_and(|expires_at| expires_at > now_secs() + 28_000.0));
        assert_eq!(saved["tokens"]["refresh_token"], json!("new-refresh"));
        assert_eq!(saved["tokens"]["refresh_token_expires_in"], json!(15_897_600));
        assert_eq!(saved["tokens"]["scope"], json!("repo,read:user"));

        let request = seen.await.unwrap();
        let lower = request.to_ascii_lowercase();
        assert!(lower.contains("accept: application/json"));
        let body = request.split("\r\n\r\n").nth(1).unwrap_or_default();
        assert!(body.contains("client_id=client-id"));
        assert!(body.contains("grant_type=refresh_token"));
        assert!(body.contains("refresh_token=old-refresh"));
        assert!(!body.contains("scope=") && !body.contains("resource="));
    }

    #[tokio::test]
    async fn fresh_device_bearer_needs_no_refresh_endpoint() {
        let stored = json!({
            "client_id": "client-id",
            "device_flow": true,
            "signed_in_at": now_secs(),
            "tokens": { "access_token": "access", "expires_in": 28_800, "refresh_token": "refresh" }
        });
        let (bearer, saved) = refresh_device_bearer(&reqwest::Client::new(), "not a URL", "GitHub", &stored).await.unwrap();
        assert_eq!(bearer.access_token, "access");
        assert!(saved.is_none());
    }

    #[tokio::test]
    async fn rejected_device_refresh_asks_for_a_new_sign_in() {
        let (endpoint, _) = token_endpoint("error=bad_refresh_token&error_description=expired").await;
        let stored = json!({
            "client_id": "client-id",
            "device_flow": true,
            "signed_in_at": 1,
            "tokens": { "access_token": "access", "expires_in": 1, "refresh_token": "refresh" }
        });
        let error = refresh_device_bearer(&reqwest::Client::new(), &endpoint, "GitHub", &stored).await.unwrap_err();
        assert_eq!(error, "The GitHub sign-in expired. Sign in again.");
    }

    #[tokio::test]
    async fn a_chat_holds_one_waiting_sign_in_card_per_plugin_and_runner() {
        let scratch = scratch_app();
        let app = &scratch.0;
        let bot = |id: &str, runner_id: &str| -> Bot {
            serde_json::from_value(json!({ "id": id, "name": id, "description": "", "symbol_name": "", "accent": "", "runner_id": runner_id, "provider": "deepseek", "created_at": 0.0 })).unwrap()
        };
        {
            let mut state = app.state.lock().unwrap();
            state.bots.extend([bot("maid", "here"), bot("cook", "here"), bot("scout", "elsewhere")]);
            state.chats.push(serde_json::from_value(json!({ "id": "chat", "kind": "group", "bot_ids": ["maid", "cook", "scout"], "created_at": 0.0 })).unwrap());
        }
        let manifest = crate::plugins::Manifest::parse(&json!({ "id": "docs", "name": "Docs", "servers": { "api": { "type": "http", "url": "https://docs.test/mcp", "auth": { "type": "oauth" } } } })).unwrap();
        assert_eq!(crate::plugins::install(app, manifest, "marketplace").unwrap().state, "needs_auth");
        let waiting = || {
            let cards = app.store.sign_in_cards(Some("chat"), "docs").unwrap().into_iter();
            cards.filter(|(card, _)| matches!(&card.body, Body::Permission { decision, .. } if decision == "pending")).map(|(card, _)| card.id).collect::<Vec<_>>()
        };
        let decision = |id: &str| match app.message("chat", id).unwrap().body {
            Body::Permission { decision, .. } => decision,
            _ => unreachable!(),
        };

        app.upsert_message(Message::new("chat", Author::You, Body::text("add Docs")), false);
        // The install puts the card up, then the bot asks for it again in the same turn.
        let first = post_sign_in_card(app, "chat", "maid", "docs").unwrap();
        assert_eq!(post_sign_in_card(app, "chat", "maid", "docs").unwrap().id, first.id);
        assert_eq!(post_sign_in_card(app, "chat", "cook", "docs").unwrap().id, first.id, "a bot on the same Runner");
        let elsewhere = post_sign_in_card(app, "chat", "scout", "docs").unwrap();
        assert_ne!(elsewhere.id, first.id, "another Runner signs in on its own");
        assert_eq!(waiting(), [first.id.clone(), elsewhere.id.clone()]);

        // The user wrote without signing in: asked again, the card goes up after their message.
        app.upsert_message(Message::new("chat", Author::You, Body::text("not yet")), false);
        let again = post_sign_in_card(app, "chat", "maid", "docs").unwrap();
        assert_ne!(again.id, first.id);
        assert_eq!(decision(&first.id), "dismissed");
        assert_eq!(waiting(), [elsewhere.id, again.id]);
    }

    #[tokio::test]
    async fn signed_in_once_every_card_here_says_so() {
        let scratch = scratch_app();
        let app = &scratch.0;
        crate::identity::create(app, Some("Workbench".into())).unwrap();
        let here = app.this_device_id().unwrap();
        let bot = |id: &str, runner_id: &str| -> Bot {
            serde_json::from_value(json!({ "id": id, "name": id, "description": "", "symbol_name": "", "accent": "", "runner_id": runner_id, "provider": "deepseek", "created_at": 0.0 })).unwrap()
        };
        {
            let mut state = app.state.lock().unwrap();
            state.bots.extend([bot("maid", &here), bot("scout", "elsewhere")]);
            for (chat, bots) in [("one", json!(["maid"])), ("two", json!(["maid", "scout"]))] {
                state.chats.push(serde_json::from_value(json!({ "id": chat, "kind": "group", "bot_ids": bots, "created_at": 0.0 })).unwrap());
            }
        }
        let manifest = crate::plugins::Manifest::parse(&json!({ "id": "docs", "name": "Docs", "servers": { "api": { "type": "http", "url": "https://docs.test/mcp", "auth": { "type": "oauth", "token_variable": "DOCS_TOKEN" } } }, "variables": [{ "name": "DOCS_TOKEN", "secret": true }] })).unwrap();
        crate::plugins::install(app, manifest, "marketplace").unwrap();
        let decision = |chat: &str, id: &str| match app.message(chat, id).unwrap().body {
            Body::Permission { decision, .. } => decision,
            _ => unreachable!(),
        };
        let first = post_sign_in_card(app, "one", "maid", "docs").unwrap();
        let second = post_sign_in_card(app, "two", "maid", "docs").unwrap();
        set_card(app, "two", &second.id, "allowed", Some("Finish signing in in the browser on Phone.".into()), None, None);
        let theirs = post_sign_in_card(app, "two", "scout", "docs").unwrap();

        // A pasted token stands in for the sign-in.
        let token = BTreeMap::from([("DOCS_TOKEN".to_string(), "secret".to_string())]);
        assert_eq!(crate::plugins::set_variables(app, "docs", &token).unwrap().state, "ready");
        settle_sign_in_cards(app, "docs", "Docs");
        assert_eq!((decision("one", &first.id), decision("two", &second.id)), ("connected".into(), "connected".into()));
        assert_eq!(decision("two", &theirs.id), "pending", "another Runner signs in on its own");

        // A card that waited through it reads Signed in when tapped, and starts nothing.
        let mut late = first.clone();
        late.id = "late".into();
        late.body = theirs.body.clone();
        late.author = Author::Bot { bot_id: "maid".into() };
        app.upsert_message(late, false);
        let answered = answer_sign_in(app, "one", "late", Decision::Allowed, None).await.unwrap();
        assert_eq!(answered, json!({ "answered": true }));
        assert_eq!(decision("one", "late"), "connected");
        assert!(app.mcp.sign_ins.lock().unwrap().is_empty());
    }
}
