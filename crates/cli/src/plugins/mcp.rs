//! The MCP side of plugins on this Runner: a pool of connected servers (`rmcp`, stdio or
//! streamable HTTP), the OAuth sign-in for a remote server, and the tools a turn gets from a
//! plugins installed on its Runner, each behind Auto-review (`review.rs`) and the permission gate: a read-only tool runs, anything
//! else asks the user in the chat first, after Grok Bot.

use std::collections::{BTreeMap, HashMap};
use std::sync::{Arc, Mutex};

use async_trait::async_trait;
use rmcp::model::{CallToolRequestParams, ClientConfig, ContentBlock, Implementation};
use rmcp::service::RunningService;
use rmcp::transport::auth::{AuthClient, AuthorizationManager, AuthorizationRequest, OAuthState};
use rmcp::transport::streamable_http_client::StreamableHttpClientTransportConfig;
use rmcp::transport::{StreamableHttpClientTransport, TokioChildProcess};
use rmcp::{RoleClient, ServiceExt};
use serde_json::{json, Value};
use lorca_agent::agent_loop::ToolExecutionMode;
use lorca_agent::{ContentPart, Tool, ToolError, ToolResult, ToolUpdateFn};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio_util::sync::CancellationToken;

use super::{fill, pattern_matches, AuthSpec, Installed, ServerSpec};
use crate::app::App;
use crate::config::now_secs;
use crate::model::*;

/// How long the user has to answer a permission card before the call is refused.
pub const PERMISSION_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(10 * 60);
/// How long a tool call may run.
const CALL_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(10 * 60);
/// How long the sign-in page may take.
const SIGN_IN_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(5 * 60);
/// The most text a tool result carries to the model.
const MAX_RESULT_CHARS: usize = 50_000;

// MARK: - Pool

/// One connected MCP server.
pub struct Server {
    pub plugin_id: String,
    pub name: String,
    service: RunningService<RoleClient, ClientConfig>,
    pub tools: Vec<rmcp::model::Tool>,
    pub instructions: Option<String>,
    /// The OAuth manager behind the transport, to persist tokens it refreshed.
    auth: Option<Arc<tokio::sync::Mutex<AuthorizationManager>>>,
}

/// Connected servers by `plugin/server`, connected on first use and dropped when the plugin
/// changes. Held by the App.
pub struct Pool {
    servers: Mutex<HashMap<String, Arc<Server>>>,
    connecting: tokio::sync::Mutex<()>,
    /// The HTTP client the MCP transports and the OAuth flow use (rmcp's reqwest, not the
    /// App's).
    http: mcp_http::Client,
}

impl Default for Pool {
    fn default() -> Self {
        Self::new()
    }
}

impl Pool {
    pub fn new() -> Self {
        let http = mcp_http::Client::builder().timeout(std::time::Duration::from_secs(600)).build().unwrap_or_default();
        Pool { servers: Mutex::new(HashMap::new()), connecting: tokio::sync::Mutex::new(()), http }
    }

    /// Drops every connection of a plugin, so the next use reconnects with fresh settings.
    pub fn forget(&self, plugin_id: &str) {
        self.servers.lock().unwrap().retain(|key, _| !key.starts_with(&format!("{plugin_id}/")));
    }

    /// The connected server, connecting it first when needed.
    pub async fn server(&self, app: &Arc<App>, plugin_id: &str, name: &str) -> Result<Arc<Server>, String> {
        let key = format!("{plugin_id}/{name}");
        if let Some(server) = self.servers.lock().unwrap().get(&key).cloned() {
            return Ok(server);
        }
        let _guard = self.connecting.lock().await;
        if let Some(server) = self.servers.lock().unwrap().get(&key).cloned() {
            return Ok(server);
        }
        let (plugin, values) = {
            let store = app.plugins.lock().unwrap();
            let plugin = store.get(plugin_id).cloned().ok_or_else(|| format!("{plugin_id} is not installed on this Runner"))?;
            (plugin, store.values(plugin_id))
        };
        let spec = plugin.manifest.servers.get(name).cloned().ok_or_else(|| format!("{} has no server {name}", plugin.manifest.name))?;
        super::note(app, plugin_id, Some(("connecting", "Connecting…")));
        match connect(app, &plugin, name, &spec, &values).await {
            Ok(server) => {
                let server = Arc::new(server);
                self.servers.lock().unwrap().insert(key, server.clone());
                super::note(app, plugin_id, None);
                Ok(server)
            }
            Err(error) => {
                super::note(app, plugin_id, Some(("error", &error)));
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
                Err(error) => tracing::warn!(%error, plugin = plugin_id, server = %name, "connecting a plugin server"),
            }
        }
        servers
    }
}

async fn connect(app: &Arc<App>, plugin: &Installed, name: &str, spec: &ServerSpec, values: &BTreeMap<String, String>) -> Result<Server, String> {
    let mut implementation = Implementation::default();
    implementation.name = "Lorca".into();
    implementation.version = crate::config::VERSION.into();
    let mut info = ClientConfig::default();
    info.client_info = implementation;
    let mut auth = None;
    let service = match spec {
        ServerSpec::Stdio { command, args, env } => {
            let mut cmd = tokio::process::Command::new(command);
            cmd.args(args.iter().map(|a| fill(a, values)));
            for (key, value) in env {
                cmd.env(key, fill(value, values));
            }
            cmd.current_dir(app.config.plugins_dir().join(&plugin.manifest.id));
            let transport = TokioChildProcess::new(cmd).map_err(|e| format!("Cannot start {command}: {e}"))?;
            info.serve(transport).await.map_err(|e| format!("{command} did not answer the MCP handshake: {e}"))?
        }
        ServerSpec::Http { url, headers, auth: auth_spec } => {
            let mut config = StreamableHttpClientTransportConfig::with_uri(url.as_str());
            let mut custom = HashMap::new();
            for (key, value) in headers {
                let key = mcp_http::header::HeaderName::from_bytes(key.as_bytes()).map_err(|e| format!("Bad header {key}: {e}"))?;
                let value = mcp_http::header::HeaderValue::from_str(&fill(value, values)).map_err(|e| format!("Bad header value: {e}"))?;
                custom.insert(key, value);
            }
            config = config.custom_headers(custom);
            let pasted = match auth_spec {
                Some(AuthSpec::Bearer { variable }) => values.get(variable).cloned(),
                Some(AuthSpec::Oauth { token_variable: Some(variable), .. }) => values.get(variable).cloned(),
                _ => None,
            };
            let tokens = app.plugins.lock().unwrap().secret(&plugin.manifest.id, &format!("oauth:{name}"));
            match (pasted, auth_spec, tokens) {
                (Some(token), _, _) => {
                    let transport = StreamableHttpClientTransport::with_client(app.mcp.http.clone(), config.auth_header(token));
                    info.serve(transport).await.map_err(|e| describe_connect_error(&e.to_string(), url))?
                }
                (None, Some(AuthSpec::Oauth { .. }), Some(stored)) => {
                    // A token that cannot be refreshed (a device-flow token, or a server whose
                    // metadata cannot be found again) is sent as a plain bearer.
                    let refreshable = stored["tokens"]["refresh_token"].as_str().is_some_and(|r| !r.is_empty());
                    let restored = if refreshable { restore_manager(app, url, &stored).await } else { Err("no refresh token".into()) };
                    match restored {
                        Ok(manager) => {
                            let client = AuthClient::new(app.mcp.http.clone(), manager);
                            auth = Some(client.auth_manager.clone());
                            let transport = StreamableHttpClientTransport::with_client(client, config);
                            info.serve(transport).await.map_err(|e| describe_connect_error(&e.to_string(), url))?
                        }
                        Err(why) => {
                            tracing::debug!(%why, plugin = %plugin.manifest.id, "using the saved access token as a bearer");
                            let token = stored["tokens"]["access_token"].as_str().ok_or("The saved sign-in has no access token")?.to_string();
                            let transport = StreamableHttpClientTransport::with_client(app.mcp.http.clone(), config.auth_header(token));
                            info.serve(transport).await.map_err(|e| describe_connect_error(&e.to_string(), url))?
                        }
                    }
                }
                (None, Some(AuthSpec::Oauth { .. }), None) => return Err(format!("Sign in to {} first.", plugin.manifest.name)),
                (None, Some(AuthSpec::Bearer { variable }), _) => return Err(format!("Set {variable} first.")),
                (None, None, _) => {
                    let transport = StreamableHttpClientTransport::with_client(app.mcp.http.clone(), config);
                    info.serve(transport).await.map_err(|e| describe_connect_error(&e.to_string(), url))?
                }
            }
        }
    };
    let instructions = service.peer_info().and_then(|i| i.instructions.clone());
    let mut tools = service.list_all_tools().await.map_err(|e| format!("{} could not list its tools: {e}", plugin.manifest.name))?;
    tools.retain(|t| !plugin.manifest.tools.hide.iter().any(|h| pattern_matches(h, &t.name)));
    tracing::info!(plugin = %plugin.manifest.id, server = name, tools = tools.len(), "connected an MCP server");
    Ok(Server { plugin_id: plugin.manifest.id.clone(), name: name.to_string(), service, tools, instructions, auth })
}

fn describe_connect_error(error: &str, url: &str) -> String {
    if error.contains("401") || error.to_lowercase().contains("unauthorized") {
        format!("{url} refused the credentials. Sign in again.")
    } else {
        format!("Could not connect to {url}: {error}")
    }
}

/// An authorization manager holding tokens saved by an earlier sign-in.
async fn restore_manager(app: &Arc<App>, url: &str, stored: &Value) -> Result<AuthorizationManager, String> {
    let client_id = stored["client_id"].as_str().ok_or("The saved sign-in has no client id")?;
    let tokens = serde_json::from_value(stored["tokens"].clone()).map_err(|e| format!("The saved sign-in is unreadable: {e}"))?;
    let mut state = OAuthState::new(url, Some(app.mcp.http.clone())).await.map_err(|e| e.to_string())?;
    state.set_credentials(client_id, tokens).await.map_err(|e| format!("Restoring the sign-in: {e}"))?;
    state.into_authorization_manager().ok_or_else(|| "Restoring the sign-in".to_string())
}

// MARK: - Sign-in

/// Signs in to an OAuth server on this Runner: the MCP authorization flow with a loopback
/// callback, the browser opened here. Runs in the background; the plugin's state says how it
/// goes and the tokens land in the secrets file.
pub fn connect_oauth(app: &Arc<App>, plugin_id: &str, server: &str) -> Result<String, String> {
    connect_oauth_for_card(app, plugin_id, server, None)
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
/// on the Runner and the card says how it went. Nothing waits on it.
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
    let runner = app.this_device_id().and_then(|id| app.device(&id)).map(|d| d.name).unwrap_or_else(|| "its Runner".into());
    let message = Message::new(
        chat_id,
        Author::Bot { bot_id: bot_id.to_string() },
        Body::Permission {
            plugin_id: plugin_id.to_string(),
            plugin_name: name.clone(),
            tool: "connect".into(),
            summary: format!("Sign in to {name}; the browser opens on {runner}."),
            arguments: Value::Null,
            decision: "pending".into(),
            reason: None,
            link: None,
            code: None,
        },
    );
    app.upsert_message(message.clone(), true);
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
pub fn connect_oauth_for_card(app: &Arc<App>, plugin_id: &str, server: &str, card: Option<(String, String)>) -> Result<String, String> {
    let (plugin, spec) = {
        let store = app.plugins.lock().unwrap();
        let plugin = store.get(plugin_id).cloned().ok_or("Unknown plugin")?;
        let spec = plugin.manifest.servers.get(server).cloned().ok_or_else(|| format!("{} has no server {server}", plugin.manifest.name))?;
        (plugin, spec)
    };
    let (url, scopes, client, device) = match &spec {
        ServerSpec::Http { url, auth: Some(AuthSpec::Oauth { scopes, token_variable, client_id_variable, client_secret_variable, client_id, client_secret, device_authorization_endpoint, token_endpoint }), .. } => {
            let values = app.plugins.lock().unwrap().values(plugin_id);
            let set = |fixed: &Option<String>, variable: &Option<String>| {
                fixed.clone().filter(|v| !v.trim().is_empty()).or_else(|| variable.as_ref().and_then(|v| values.get(v).cloned())).filter(|v| !v.trim().is_empty())
            };
            let hint = ClientHint {
                id: set(client_id, client_id_variable),
                secret: set(client_secret, client_secret_variable),
                token_variable: token_variable.clone(),
                client_id_variable: client_id_variable.clone(),
                client_secret_variable: client_secret_variable.clone(),
            };
            let device = match (device_authorization_endpoint, token_endpoint, &hint.id) {
                (Some(device_endpoint), Some(token_endpoint), Some(_)) => Some((device_endpoint.clone(), token_endpoint.clone())),
                _ => None,
            };
            (url.clone(), scopes.clone(), hint, device)
        }
        _ => return Err(format!("{server} does not sign in with OAuth.")),
    };
    let app = app.clone();
    let plugin_id = plugin_id.to_string();
    let server = server.to_string();
    let name = plugin.manifest.name.clone();
    super::note(&app, &plugin_id, Some(("connecting", if device.is_some() { "Enter the code from the chat" } else { "Finish signing in in the browser" })));
    let message = if device.is_some() { format!("Getting a {name} sign-in code.") } else { format!("Opened the {name} sign-in page in the browser on this Runner.") };
    tokio::spawn(async move {
        let flow = match &device {
            Some((device_endpoint, token_endpoint)) => device_sign_in(&app, device_endpoint, token_endpoint, &scopes, &name, client.id.as_deref().unwrap_or(""), card.as_ref()).await,
            None => sign_in(&app, &url, &scopes, &name, &client).await,
        };
        let outcome = match flow {
            Ok(saved) => match super::set_oauth(&app, &plugin_id, &server, Some(saved)) {
                Ok(()) => {
                    app.mcp.forget(&plugin_id);
                    super::note(&app, &plugin_id, None);
                    Ok(())
                }
                Err(error) => Err(error),
            },
            Err(error) => Err(error),
        };
        if let Err(error) = &outcome {
            super::note(&app, &plugin_id, Some(("error", error)));
        }
        if let Some((chat_id, message_id)) = card {
            match outcome {
                Ok(()) => set_card(&app, &chat_id, &message_id, "connected", Some(format!("Signed in to {name}.")), None, None),
                Err(error) => set_card(&app, &chat_id, &message_id, "failed", Some(format!("Sign-in failed: {error}")), None, None),
            }
        }
    });
    Ok(message)
}

/// How long a device flow waits for the user to enter the code.
const DEVICE_FLOW_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(15 * 60);

/// The device flow (RFC 8628), as GitHub's own CLI signs in: ask for a code, show it on the
/// card with the link, poll the token endpoint until the user has entered it. Answers with
/// the tokens in the shape the browser flow saves.
async fn device_sign_in(app: &Arc<App>, device_endpoint: &str, token_endpoint: &str, scopes: &[String], name: &str, client_id: &str, card: Option<&(String, String)>) -> Result<Value, String> {
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
    let deadline = std::time::Instant::now() + DEVICE_FLOW_TIMEOUT.min(std::time::Duration::from_secs(started["expires_in"].as_u64().unwrap_or(900)));
    let mut wait = interval;
    loop {
        tokio::time::sleep(std::time::Duration::from_secs(wait)).await;
        if std::time::Instant::now() > deadline {
            return Err("The code expired before it was entered.".into());
        }
        let polled: Value = http
            .post(token_endpoint)
            .header("accept", "application/json")
            .form(&[("client_id", client_id), ("device_code", &device_code), ("grant_type", "urn:ietf:params:oauth:grant-type:device_code")])
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

/// An answer to a sign-in card: Sign in starts the flow on this Runner and the card follows
/// it; Not now closes the card. True when the card was pending.
pub fn answer_sign_in(app: &Arc<App>, chat_id: &str, message_id: &str, decision: Decision) -> Result<bool, String> {
    let Some(message) = app.message(chat_id, message_id) else { return Ok(false) };
    let Body::Permission { plugin_id, tool, decision: current, .. } = &message.body else { return Ok(false) };
    if tool != "connect" || current != "pending" {
        return Ok(false);
    }
    match decision {
        Decision::Allowed | Decision::Always => {
            let server = oauth_server(app, plugin_id).ok_or("Nothing to sign in to")?;
            let runner = app.this_device_id().and_then(|id| app.device(&id)).map(|d| d.name).unwrap_or_else(|| "the Runner".into());
            let started = connect_oauth_for_card(app, plugin_id, &server, Some((chat_id.to_string(), message_id.to_string())))?;
            let summary = if started.starts_with("Getting a") { "Getting a code…".to_string() } else { format!("Finish signing in in the browser on {runner}.") };
            set_card(app, chat_id, message_id, "allowed", Some(summary), None, None);
        }
        Decision::Denied | Decision::Expired => set_card(app, chat_id, message_id, "denied", None, None, None),
    }
    Ok(true)
}

/// What the manifest and the user supplied for a server that needs a preregistered client.
struct ClientHint {
    id: Option<String>,
    secret: Option<String>,
    token_variable: Option<String>,
    client_id_variable: Option<String>,
    client_secret_variable: Option<String>,
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

async fn sign_in(app: &Arc<App>, url: &str, scopes: &[String], name: &str, client: &ClientHint) -> Result<Value, String> {
    let listener = tokio::net::TcpListener::bind(("127.0.0.1", 0)).await.map_err(|e| format!("Cannot listen for the callback: {e}"))?;
    let port = listener.local_addr().map_err(|e| e.to_string())?.port();
    let redirect = format!("http://127.0.0.1:{port}/callback");
    let mut state = OAuthState::new(url, Some(app.mcp.http.clone())).await.map_err(|e| format!("{name}: {e}"))?;
    let mut request = AuthorizationRequest::new(redirect).with_scopes(scopes.iter().cloned()).with_client_name("Lorca").with_application_type("native");
    if let Some(challenge) = challenge_of(app, url).await {
        request = request.with_challenge(challenge);
    }
    if let Some(id) = &client.id {
        request = request.with_preregistered_client(id.clone());
        if let Some(secret) = &client.secret {
            request = request.with_client_secret(secret.clone());
        }
    }
    state.start_authorization(request).await.map_err(|e| match e {
        rmcp::transport::auth::AuthError::RegistrationFailed(_) => client.no_registration_advice(name),
        other => format!("{name} does not offer a sign-in: {other}"),
    })?;
    let authorize_url = state.get_authorization_url().await.map_err(|e| e.to_string())?;
    if std::env::var("LORCA_OAUTH_NO_BROWSER").ok().as_deref() == Some("1") {
        // Tests: fetch the page ourselves; a fake server redirects straight to the callback.
        let http = app.mcp.http.clone();
        let url = authorize_url.clone();
        tokio::spawn(async move {
            if let Err(error) = http.get(&url).send().await {
                tracing::warn!(%error, "fetching the sign-in page");
            }
        });
    } else {
        open::that(&authorize_url).map_err(|e| format!("Cannot open the browser: {e}"))?;
    }
    let callback = tokio::time::timeout(SIGN_IN_TIMEOUT, wait_for_callback(listener, port, name))
        .await
        .map_err(|_| "Timed out waiting for the browser".to_string())??;
    state.handle_callback_url(&callback).await.map_err(|e| format!("{name} rejected the sign-in: {e}"))?;
    let (client_id, tokens) = state.get_credentials().await.map_err(|e| e.to_string())?;
    let tokens = tokens.ok_or("The sign-in produced no tokens")?;
    Ok(json!({ "client_id": client_id, "tokens": tokens, "signed_in_at": now_secs() }))
}

/// Answers the browser's redirect and returns the full callback URL.
async fn wait_for_callback(listener: tokio::net::TcpListener, port: u16, name: &str) -> Result<String, String> {
    loop {
        let (mut socket, _) = listener.accept().await.map_err(|e| e.to_string())?;
        let mut buffer = vec![0u8; 16384];
        let read = socket.read(&mut buffer).await.map_err(|e| e.to_string())?;
        let request = String::from_utf8_lossy(&buffer[..read]).into_owned();
        let path = request.lines().next().and_then(|line| line.split_whitespace().nth(1)).unwrap_or("/").to_string();
        if !path.starts_with("/callback") {
            let _ = socket.write_all(b"HTTP/1.1 404 Not Found\r\nContent-Length: 0\r\n\r\n").await;
            continue;
        }
        let failed = path.contains("error=");
        let body = if failed {
            "<html><body style=\"font-family:-apple-system\"><h2>Sign-in failed</h2><p>Go back to Lorca and try again.</p></body></html>".to_string()
        } else {
            format!("<html><body style=\"font-family:-apple-system\"><h2>Signed in to {name}</h2><p>You can close this window and return to Lorca.</p></body></html>")
        };
        let response = format!(
            "HTTP/1.1 {}\r\nContent-Type: text/html; charset=utf-8\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
            if failed { "400 Bad Request" } else { "200 OK" },
            body.len()
        );
        let _ = socket.write_all(response.as_bytes()).await;
        let _ = socket.shutdown().await;
        if failed {
            return Err("The sign-in was denied.".into());
        }
        return Ok(format!("http://127.0.0.1:{port}{path}"));
    }
}

// MARK: - Tools for a turn

/// The tool name the model sees: `github__create_issue`, within the providers' limits.
pub fn tool_name(plugin_id: &str, tool: &str) -> String {
    let raw = format!("{plugin_id}__{tool}");
    let cleaned: String = raw.chars().map(|c| if c.is_ascii_alphanumeric() || c == '_' || c == '-' { c } else { '_' }).collect();
    cleaned.chars().take(64).collect()
}

/// A tool of a plugin server, bound to the turn's chat and bot.
pub struct PluginTool {
    app: Arc<App>,
    chat_id: String,
    bot_id: String,
    /// A routine run has nobody to ask, so a tool that needs permission is refused.
    unattended: bool,
    plugin_id: String,
    plugin_name: String,
    server: Arc<Server>,
    tool: rmcp::model::Tool,
    name: String,
    description: String,
    read_only: bool,
}

/// The tools of every plugin installed on this Runner, connected now, for a bot that runs
/// here. A plugin that cannot connect is skipped with a warning; the bot is told in its prompt.
pub async fn tools_for(app: &Arc<App>, chat_id: &str, bot: &Bot, unattended: bool) -> (Vec<Arc<dyn Tool>>, Vec<PluginBrief>) {
    let mut tools: Vec<Arc<dyn Tool>> = Vec::new();
    let mut briefs = Vec::new();
    let installed: Vec<super::Installed> = app.plugins.lock().unwrap().installed().to_vec();
    for plugin in &installed {
        let plugin_id = &plugin.manifest.id;
        let status = app.plugins.lock().unwrap().status(plugin_id);
        let mut brief = PluginBrief {
            id: plugin_id.clone(),
            name: plugin.manifest.name.clone(),
            description: plugin.manifest.description.clone(),
            tools: Vec::new(),
            instructions: Vec::new(),
            skills: plugin.manifest.skills.iter().map(|s| (s.name.clone(), s.description.clone(), app.config.plugins_dir().join(plugin_id).join("skills").join(format!("{}.md", super::slug(&s.name))))).collect(),
            problem: None,
        };
        if let Some(status) = status.filter(|s| s.state == "needs_setup" || s.state == "needs_auth") {
            brief.problem = Some(status.detail.clone());
            briefs.push(brief);
            continue;
        }
        let servers = app.mcp.servers_of(app, plugin_id).await;
        if servers.is_empty() {
            brief.problem = Some(app.plugins.lock().unwrap().status(plugin_id).map(|s| s.detail).unwrap_or_else(|| "Could not connect".into()));
        }
        for server in servers {
            if let Some(instructions) = &server.instructions {
                brief.instructions.push(instructions.clone());
            }
            for tool in &server.tools {
                let name = tool_name(plugin_id, &tool.name);
                let read_only = tool.annotations.as_ref().and_then(|a| a.read_only_hint).unwrap_or(false)
                    || plugin.manifest.tools.readonly.iter().any(|p| pattern_matches(p, &tool.name));
                brief.tools.push((tool.name.to_string(), read_only));
                tools.push(Arc::new(PluginTool {
                    app: app.clone(),
                    chat_id: chat_id.to_string(),
                    bot_id: bot.id.clone(),
                    unattended,
                    plugin_id: plugin_id.clone(),
                    plugin_name: plugin.manifest.name.clone(),
                    server: server.clone(),
                    tool: tool.clone(),
                    description: tool.description.as_deref().unwrap_or("").to_string(),
                    name,
                    read_only,
                }));
            }
        }
        briefs.push(brief);
    }
    (tools, briefs)
}

/// What the system prompt says about one installed plugin.
pub struct PluginBrief {
    pub id: String,
    pub name: String,
    pub description: String,
    /// (tool name, read-only)
    pub tools: Vec<(String, bool)>,
    pub instructions: Vec<String>,
    /// (name, description, path)
    pub skills: Vec<(String, String, std::path::PathBuf)>,
    /// Why the plugin is unusable right now.
    pub problem: Option<String>,
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
    fn parameters(&self) -> Value {
        Value::Object((*self.tool.input_schema).clone())
    }
    fn execution_mode(&self) -> Option<ToolExecutionMode> {
        Some(ToolExecutionMode::Sequential)
    }
    async fn execute(&self, _id: &str, args: Value, cancel: CancellationToken, _on_update: ToolUpdateFn) -> Result<ToolResult, ToolError> {
        let tool = self.tool.name.to_string();
        if !self.read_only {
            let bot = self.app.bot(&self.bot_id).ok_or_else(|| ToolError("The bot is gone".into()))?;
            let outcome = super::review::decide(&self.app, &bot, &self.chat_id, &self.plugin_id, &self.plugin_name, &tool, &self.description, &args, &cancel).await;
            if let super::review::Outcome::Ask { reason } = outcome {
                if self.unattended {
                    return Err(ToolError(format!(
                        "{tool} needs the user's permission ({}), and nobody is here to give it. Report what you would do; the user can add an Auto-review rule allowing it.",
                        reason.as_deref().unwrap_or("Auto-review is off, so every change asks")
                    )));
                }
                let summary = call_summary(&tool, &args);
                match ask(&self.app, &self.chat_id, &self.bot_id, &self.plugin_id, &self.plugin_name, &tool, &summary, args.clone(), reason, &cancel).await {
                    Decision::Allowed | Decision::Always => {}
                    Decision::Denied => return Err(ToolError(format!("The user did not allow {tool}. Do not retry it; ask what they want instead."))),
                    Decision::Expired => return Err(ToolError(format!("Nobody answered the permission request for {tool} in time. Say what you needed and stop."))),
                }
            }
        }
        let mut params = CallToolRequestParams::default();
        params.name = tool.clone().into();
        params.arguments = args.as_object().cloned();
        let call = self.server.service.call_tool(params);
        let result = tokio::select! {
            result = tokio::time::timeout(CALL_TIMEOUT, call) => result.map_err(|_| ToolError(format!("{tool} took too long")))?,
            _ = cancel.cancelled() => return Err(ToolError("Stopped".into())),
        };
        if let Some(auth) = &self.server.auth {
            persist_refreshed(&self.app, &self.plugin_id, &self.server.name, auth).await;
        }
        let result = result.map_err(|e| ToolError(format!("{tool} failed: {e}")))?;
        let mut content: Vec<ContentPart> = Vec::new();
        let mut text_len = 0;
        for block in result.content {
            match block {
                ContentBlock::Text(text) => {
                    let mut text = text.text;
                    if text_len + text.len() > MAX_RESULT_CHARS {
                        let room = MAX_RESULT_CHARS.saturating_sub(text_len);
                        let mut cut = room;
                        while cut > 0 && !text.is_char_boundary(cut) {
                            cut -= 1;
                        }
                        text.truncate(cut);
                        text.push_str("\n[truncated]");
                    }
                    text_len += text.len();
                    content.push(ContentPart::text(text));
                }
                ContentBlock::Image(image) => content.push(ContentPart::Image { data: image.data, mime_type: image.mime_type }),
                ContentBlock::Resource(resource) => {
                    if let Ok(text) = serde_json::to_string(&resource.resource) {
                        content.push(ContentPart::text(text));
                    }
                }
                other => {
                    if let Ok(text) = serde_json::to_string(&other) {
                        content.push(ContentPart::text(text));
                    }
                }
            }
        }
        if content.is_empty() {
            if let Some(structured) = &result.structured_content {
                content.push(ContentPart::text(serde_json::to_string_pretty(structured).unwrap_or_default()));
            } else {
                content.push(ContentPart::text("(no output)"));
            }
        }
        let summary = format!("Used {}", self.plugin_name);
        if result.is_error.unwrap_or(false) {
            let text = content.iter().filter_map(ContentPart::as_text).collect::<Vec<_>>().join("\n");
            return Err(ToolError(format!("{tool} reported an error: {text}")));
        }
        Ok(ToolResult { content, details: json!({ "summary": summary, "plugin_id": self.plugin_id, "tool": tool }), terminate: false })
    }
}

/// Saves tokens the transport refreshed, so the next connection does not start from a stale
/// refresh token.
async fn persist_refreshed(app: &Arc<App>, plugin_id: &str, server: &str, auth: &Arc<tokio::sync::Mutex<AuthorizationManager>>) {
    let credentials = auth.lock().await.get_credentials().await;
    if let Ok((client_id, Some(tokens))) = credentials {
        let key = format!("oauth:{server}");
        let saved = app.plugins.lock().unwrap().secret(plugin_id, &key);
        let fresh = json!({ "client_id": client_id, "tokens": tokens, "signed_in_at": saved.as_ref().and_then(|s| s["signed_in_at"].as_f64()).unwrap_or_else(now_secs) });
        if saved.as_ref().map(|s| s["tokens"] != fresh["tokens"]).unwrap_or(true) {
            let _ = super::set_oauth(app, plugin_id, server, Some(fresh));
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
    fn as_str(self) -> &'static str {
        match self {
            Decision::Allowed => "allowed",
            Decision::Always => "always",
            Decision::Denied => "denied",
            Decision::Expired => "expired",
        }
    }
}

/// Posts a permission card in the chat and waits for the user's answer, from this Device or
/// any paired one. `reason` is why Auto-review paused the action. `always` adds an
/// Auto-review rule for the exact tool before answering.
#[allow(clippy::too_many_arguments)]
pub async fn ask(app: &Arc<App>, chat_id: &str, bot_id: &str, plugin_id: &str, plugin_name: &str, tool: &str, summary: &str, arguments: Value, reason: Option<String>, cancel: &CancellationToken) -> Decision {
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
            link: None,
            code: None,
        },
    );
    let (tx, rx) = tokio::sync::oneshot::channel();
    app.pending_permissions.lock().unwrap().insert(message.id.clone(), tx);
    app.upsert_message(message.clone(), true);
    let decision = tokio::select! {
        answer = rx => answer.unwrap_or(Decision::Denied),
        _ = tokio::time::sleep(PERMISSION_TIMEOUT) => Decision::Expired,
        _ = cancel.cancelled() => Decision::Denied,
    };
    app.pending_permissions.lock().unwrap().remove(&message.id);
    if decision == Decision::Always && tool != "install" {
        app.add_auto_review_rule(AutoReviewRule {
            id: uuid::Uuid::new_v4().to_string(),
            text: format!("use {plugin_name} {tool}"),
            behavior: "allow".into(),
            tool: Some(format!("{plugin_id}/{tool}")),
        });
    }
    if let Some(mut message) = app.message(chat_id, &message.id) {
        if let Body::Permission { decision: d, .. } = &mut message.body {
            *d = decision.as_str().into();
        }
        app.upsert_message(message, true);
    }
    decision
}

/// `chats.permission` reached the Runner: hands the answer to the waiting tool. False when
/// nothing waits on that message (answered already, or expired).
pub fn answer(app: &Arc<App>, message_id: &str, decision: Decision) -> bool {
    match app.pending_permissions.lock().unwrap().remove(message_id) {
        Some(tx) => tx.send(decision).is_ok(),
        None => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tool_names_fit_the_providers() {
        assert_eq!(tool_name("github", "create_issue"), "github__create_issue");
        assert_eq!(tool_name("my-server", "weird.name/x"), "my-server__weird_name_x");
        assert_eq!(tool_name("p", &"x".repeat(100)).len(), 64);
        // serde_json keeps object keys sorted, so the summary lists them alphabetically.
        assert_eq!(call_summary("create_issue", &json!({ "repo": "lorca", "title": "Fix   the relay", "body": "x".repeat(80) })), format!("create_issue · body: {}…, repo: lorca, title: Fix the relay", "x".repeat(60)));
        assert_eq!(call_summary("get_me", &json!({})), "get_me");
        assert_eq!(Decision::parse("always"), Some(Decision::Always));
        assert_eq!(Decision::parse("nope"), None);
    }
}
