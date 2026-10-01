//! The MCP side of plugins on this Runner: a pool of connected servers (`rmcp`, stdio or
//! streamable HTTP), the catalog a turn's codemode scripts call plugin tools from, the OAuth
//! sign-in for a remote server, and Auto-review (`review.rs`) at the loop's
//! `before_tool_call` boundary. A read-only tool runs; anything else may ask the user in the
//! chat first, after Grok Bot.

use std::collections::{BTreeMap, HashMap};
use std::sync::{Arc, Mutex};

use async_trait::async_trait;
use lorca_agent::codemode::{self, Entry, Exposure, Namespace};
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
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio_util::sync::CancellationToken;

use super::{fill, fill_if_set, pattern_matches, AuthSpec, Installed, ServerSpec};
use crate::app::App;
use crate::config::now_secs;
use crate::model::*;

/// How long the user has to answer a permission card before the call is refused.
pub const PERMISSION_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(10 * 60);
/// How long a tool call may run.
const CALL_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(10 * 60);
/// How long the sign-in page may take.
const SIGN_IN_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(5 * 60);
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
    service: RunningService<RoleClient, ClientConfig>,
    pub tools: Vec<rmcp::model::Tool>,
    pub instructions: Option<String>,
    /// The OAuth manager behind the transport, to persist tokens it refreshed.
    auth: Option<Arc<tokio::sync::Mutex<AuthorizationManager>>>,
    /// Device-flow tokens are plain bearers. Drop this pooled server before its bearer expires;
    /// the next connection refreshes and persists the rotating token pair itself.
    bearer_expires_at: Option<f64>,
}

/// Connected servers by `plugin/server`, connected on first use and dropped when the plugin
/// changes. Held by the App.
pub struct Pool {
    servers: Mutex<HashMap<String, Arc<Server>>>,
    /// Bumped by `forget`, per plugin: a connection that started before a change of the
    /// plugin's settings is used for nothing once it answers.
    generations: Mutex<HashMap<String, u64>>,
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
        Pool { servers: Mutex::new(HashMap::new()), generations: Mutex::new(HashMap::new()), connecting: tokio::sync::Mutex::new(()), http }
    }

    /// Drops every connection of a plugin, so the next use reconnects with fresh settings.
    pub fn forget(&self, plugin_id: &str) {
        *self.generations.lock().unwrap().entry(plugin_id.to_string()).or_default() += 1;
        self.servers.lock().unwrap().retain(|key, _| !key.starts_with(&format!("{plugin_id}/")));
    }

    fn generation(&self, plugin_id: &str) -> u64 {
        self.generations.lock().unwrap().get(plugin_id).copied().unwrap_or_default()
    }

    fn cached_server(&self, key: &str) -> Option<Arc<Server>> {
        let mut servers = self.servers.lock().unwrap();
        let expiring = servers
            .get(key)
            .and_then(|server| server.bearer_expires_at)
            .is_some_and(|expires_at| expires_at <= now_secs() + DEVICE_TOKEN_REFRESH_BUFFER_SECS);
        if expiring {
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
        let _guard = self.connecting.lock().await;
        if let Some(server) = self.cached_server(&key) {
            return Ok(server);
        }
        let (plugin, values) = {
            let store = app.plugins.lock().unwrap();
            let plugin = store.get(plugin_id).cloned().ok_or_else(|| format!("{plugin_id} is not installed on this Runner"))?;
            (plugin, store.values(plugin_id))
        };
        let spec = plugin.manifest.servers.get(name).cloned().ok_or_else(|| format!("{} has no server {name}", plugin.manifest.name))?;
        let generation = self.generation(plugin_id);
        super::note(app, plugin_id, Some(("connecting", "Connecting…")));
        let connected = tokio::time::timeout(CONNECT_TIMEOUT, connect(app, &plugin, name, &spec, &values))
            .await
            .unwrap_or_else(|_| Err(format!("{} did not start within {} minutes", plugin.manifest.name, CONNECT_TIMEOUT.as_secs() / 60)));
        match connected {
            // The plugin changed (new settings, a sign-in, an uninstall) while it connected.
            Ok(_) if self.generation(plugin_id) != generation => {
                super::note(app, plugin_id, None);
                Err(format!("{}'s settings changed while it connected. Try again.", plugin.manifest.name))
            }
            Ok(server) => {
                save_catalog(app, &server);
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
    let mut bearer_expires_at = None;
    let service = match spec {
        ServerSpec::Stdio { command, args, env } => {
            // The login shell's environment, so `npx` or `uvx` resolve from the user's PATH.
            let mut cmd = lorca_agent::login_shell::command(command).await;
            cmd.args(args.iter().map(|a| fill(a, values)));
            // A variable naming an optional key the user left unset is left out.
            for (key, value) in env {
                if let Some(value) = fill_if_set(value, values) {
                    cmd.env(key, value);
                }
            }
            cmd.current_dir(app.config.plugins_dir().join(&plugin.manifest.id));
            let transport = TokioChildProcess::new(cmd).map_err(|e| format!("Cannot start {command}: {e}"))?;
            info.serve(transport).await.map_err(|e| format!("{command} did not answer the MCP handshake: {e}"))?
        }
        ServerSpec::Http { url, headers, auth: auth_spec } => {
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
            let tokens = app.plugins.lock().unwrap().secret(&plugin.manifest.id, &format!("oauth:{name}"));
            match (pasted, auth_spec, tokens) {
                (Some(token), _, _) => {
                    let transport = StreamableHttpClientTransport::with_client(app.mcp.http.clone(), config.auth_header(token));
                    info.serve(transport).await.map_err(|e| describe_connect_error(&e.to_string(), url))?
                }
                (None, Some(oauth @ AuthSpec::Oauth { .. }), Some(stored)) => {
                    if stored["device_flow"].as_bool() == Some(true) {
                        // GitHub's refresh endpoint has its own contract: no `scope` or MCP
                        // `resource`. Refresh it here, then give the transport a plain bearer.
                        let token_endpoint = match oauth {
                            AuthSpec::Oauth { token_endpoint: Some(endpoint), .. } => endpoint,
                            _ => return Err(format!("{}'s saved sign-in cannot be refreshed. Sign in again.", plugin.manifest.name)),
                        };
                        let bearer = device_bearer(app, &plugin.manifest.id, name, &plugin.manifest.name, token_endpoint, &stored).await?;
                        bearer_expires_at = bearer.expires_at;
                        let transport = StreamableHttpClientTransport::with_client(app.mcp.http.clone(), config.auth_header(bearer.access_token));
                        info.serve(transport).await.map_err(|e| describe_connect_error(&e.to_string(), url))?
                    } else {
                        // A token whose server metadata cannot be found again, or which has no
                        // refresh token, is sent as a plain bearer.
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
    if let Some(manager) = &auth {
        // The handshake itself may have refreshed and rotated the pair. Save it before the
        // next request can fail, the connection can sit unused, or the process can exit.
        persist_refreshed(app, &plugin.manifest.id, name, manager).await;
    }
    let instructions = service.peer_info().and_then(|i| i.instructions.clone());
    let mut tools = service.list_all_tools().await.map_err(|e| format!("{} could not list its tools: {e}", plugin.manifest.name))?;
    tools.retain(|t| !plugin.manifest.tools.hide.iter().any(|h| pattern_matches(h, &t.name)));
    tracing::info!(plugin = %plugin.manifest.id, server = name, tools = tools.len(), "connected an MCP server");
    Ok(Server { plugin_id: plugin.manifest.id.clone(), name: name.to_string(), service, tools, instructions, auth, bearer_expires_at })
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
    let (bearer, refreshed) = refresh_device_bearer(&app.http, token_endpoint, name, stored).await?;
    if let Some(refreshed) = refreshed {
        super::set_oauth(app, plugin_id, server, Some(refreshed))?;
    }
    Ok(bearer)
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
    super::note(&app, &plugin_id, Some(("connecting", if device.is_some() { "Getting a sign-in code…" } else { "Finish signing in in the browser" })));
    let message = if device.is_some() { format!("Getting a {name} sign-in code.") } else { format!("Opened the {name} sign-in page in the browser on this Runner.") };
    tokio::spawn(async move {
        let flow = match &device {
            Some((device_endpoint, token_endpoint)) => device_sign_in(&app, &plugin_id, &server, device_endpoint, token_endpoint, &scopes, &name, client.id.as_deref().unwrap_or(""), card.as_ref()).await,
            None => sign_in(&app, &url, &scopes, &name, &client).await,
        };
        let outcome = match flow {
            Ok(saved) => match super::set_oauth(&app, &plugin_id, &server, Some(saved)) {
                Ok(()) => {
                    app.mcp.forget(&plugin_id);
                    super::note(&app, &plugin_id, None);
                    prefetch_tools(&app, &plugin_id);
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
        Decision::Denied | Decision::Expired | Decision::Dismissed => set_card(app, chat_id, message_id, "denied", None, None, None),
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

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
struct SavedServer {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    instructions: Option<String>,
    #[serde(default)]
    tools: Vec<rmcp::model::Tool>,
}

fn catalog_path(app: &App, plugin_id: &str) -> std::path::PathBuf {
    app.config.plugins_dir().join(plugin_id).join("catalog.json")
}

/// Keeps what a server just offered, for the next turns' listings.
fn save_catalog(app: &App, server: &Server) {
    let path = catalog_path(app, &server.plugin_id);
    let mut saved: SavedCatalog = crate::config::read_json(&path).unwrap_or_default();
    let fresh = SavedServer { instructions: server.instructions.clone(), tools: server.tools.clone() };
    let changed = saved.servers.get(&server.name).is_none_or(|old| old.instructions != fresh.instructions || old.tools != fresh.tools);
    if changed {
        saved.servers.insert(server.name.clone(), fresh);
        if let Err(error) = crate::config::write_json_private(&path, &saved) {
            tracing::warn!(%error, plugin = %server.plugin_id, "saving a plugin's tool list");
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

/// The saved tool lists of a plugin's current servers, without the tools its manifest hides.
fn saved_servers(app: &App, plugin: &Installed) -> Vec<(String, Option<String>, Vec<rmcp::model::Tool>)> {
    let saved: SavedCatalog = crate::config::read_json(&catalog_path(app, &plugin.manifest.id)).unwrap_or_default();
    saved
        .servers
        .into_iter()
        .filter(|(name, _)| plugin.manifest.servers.contains_key(name))
        .map(|(name, server)| {
            let tools = server.tools.into_iter().filter(|tool| !plugin.manifest.tools.hide.iter().any(|pattern| pattern_matches(pattern, &tool.name))).collect();
            (name, server.instructions, tools)
        })
        .collect()
}

/// A tool of a plugin server, which scripts call as `tools.github__create_issue(args)`. The
/// server connects on the first call. Permission is asked before the call, at the loop's
/// `before_tool_call` boundary ([`review_call`]).
pub struct PluginTool {
    app: Arc<App>,
    plugin_id: String,
    plugin_name: String,
    server_name: String,
    tool: rmcp::model::Tool,
    name: String,
    description: String,
    read_only: bool,
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
}

#[derive(Default)]
struct CatalogState {
    tools: BTreeMap<String, Arc<CatalogTool>>,
    /// Plugins whose servers connected during this turn, so their live tool lists are in.
    connected: std::collections::HashSet<String>,
}

/// What a turn's codemode scripts can call: the bot's own read and write tools, and every tool
/// of the plugins installed on this Runner. A plugin's tools come from what its servers offered
/// last time, so listing them starts nothing; a plugin never connected yet is named, and a
/// search or a call by name connects it.
pub struct PluginCatalog {
    app: Arc<App>,
    local: Vec<Arc<dyn Tool>>,
    groups: Vec<PluginGroup>,
    state: Mutex<CatalogState>,
}

impl PluginCatalog {
    fn new(app: Arc<App>, local: Vec<Arc<dyn Tool>>) -> Self {
        let installed = app.plugins.lock().unwrap().installed().to_vec();
        let mut catalog = PluginCatalog { app, local, groups: Vec::new(), state: Mutex::new(CatalogState::default()) };
        for plugin in &installed {
            let saved = saved_servers(&catalog.app, plugin);
            catalog.groups.push(plugin_group(&catalog.app, plugin, &saved));
            for (server, instructions, tools) in &saved {
                catalog.add_tools(plugin, server, instructions.as_deref(), tools);
            }
        }
        catalog
    }

    fn add_tools(&self, plugin: &Installed, server_name: &str, instructions: Option<&str>, tools: &[rmcp::model::Tool]) {
        let mut state = self.state.lock().unwrap();
        let server_instructions = instructions.map(|text| utf8_prefix(text, MAX_SERVER_INSTRUCTIONS_BYTES).to_string()).unwrap_or_default();
        for tool in tools {
            if plugin.manifest.tools.hide.iter().any(|pattern| pattern_matches(pattern, &tool.name)) {
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
            let read_only = tool.annotations.as_ref().and_then(|annotations| annotations.read_only_hint).unwrap_or(false)
                || plugin.manifest.tools.readonly.iter().any(|pattern| pattern_matches(pattern, &original_name));
            let executable = Arc::new(PluginTool {
                app: self.app.clone(),
                plugin_id: plugin.manifest.id.clone(),
                plugin_name: plugin.manifest.name.clone(),
                server_name: server_name.to_string(),
                tool: tool.clone(),
                name: name.clone(),
                description: description.clone(),
                read_only,
            });
            let raw_search_schema = serde_json::to_string(&Value::Object((*tool.input_schema).clone())).unwrap_or_default();
            state.tools.insert(
                name.clone(),
                Arc::new(CatalogTool {
                    name,
                    original_name,
                    plugin_id: plugin.manifest.id.clone(),
                    plugin_name: plugin.manifest.name.clone(),
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
        if self.state.lock().unwrap().connected.contains(&plugin.manifest.id) {
            return Vec::new();
        }
        let status = self.app.plugins.lock().unwrap().status(&plugin.manifest.id);
        if let Some(status) = status.filter(|status| status.state == "needs_setup" || status.state == "needs_auth") {
            return vec![status.detail];
        }
        let mut problems = Vec::new();
        for server_name in plugin.manifest.servers.keys() {
            let server = tokio::select! {
                result = self.app.mcp.server(&self.app, &plugin.manifest.id, server_name) => result,
                _ = cancel.cancelled() => return problems,
            };
            match server {
                Ok(server) => self.add_tools(plugin, &server.name, server.instructions.as_deref(), &server.tools),
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

    fn plugin_tool(&self, name: &str) -> Option<Arc<PluginTool>> {
        self.state.lock().unwrap().tools.get(name).map(|tool| tool.tool.clone())
    }

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
/// still needs, and the start of its servers' own instructions. Only the needs that last are
/// named, so the description stays the same from turn to turn.
fn plugin_group(app: &App, plugin: &Installed, saved: &[(String, Option<String>, Vec<rmcp::model::Tool>)]) -> PluginGroup {
    let manifest = &plugin.manifest;
    let mut description = manifest.name.clone();
    if let Some(about) = manifest.description.lines().map(str::trim).find(|line| !line.is_empty()) {
        description.push_str(&format!(": {about}"));
    }
    let status = app.plugins.lock().unwrap().status(&manifest.id);
    match status.as_ref().map(|status| status.state.as_str()) {
        Some("needs_auth") => description.push_str("\nNeeds a sign-in before its tools work: call connect_plugin."),
        Some("needs_setup") => description.push_str(&format!("\nNot set up yet ({}): the user sets it up in the plugin's settings.", status.map(|status| status.detail).unwrap_or_default())),
        _ => {}
    }
    for (_, instructions, _) in saved {
        if let Some(instructions) = instructions.as_deref().map(str::trim).filter(|text| !text.is_empty()) {
            let shown = utf8_prefix(instructions, MAX_NAMESPACE_INSTRUCTIONS_BYTES);
            description.push_str(&format!("\nServer instructions: {shown}{}", if shown.len() < instructions.len() { "…" } else { "" }));
        }
    }
    PluginGroup { id: manifest.id.clone(), description }
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
        let plugin = self.app.plugins.lock().unwrap().installed().iter().find(|plugin| plugin.manifest.id == prefix || codemode::to_identifier(&plugin.manifest.id) == prefix).cloned()?;
        self.connect_plugin(&plugin, cancel).await;
        self.lookup(name).map(|tool| PluginCatalog::entry(&tool))
    }

    async fn search(&self, query: &str, namespace: Option<&str>, limit: usize, cancel: &CancellationToken) -> Result<Vec<Entry>, String> {
        if query.trim().is_empty() {
            return Err("searchTools() needs a non-empty query".into());
        }
        let installed = self.app.plugins.lock().unwrap().installed().to_vec();
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
                name: plugin.manifest.name.clone(),
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
    Changes { description: String },
    Stopped,
}

/// Whether a plugin tool only reads: its manifest lists it so, or its server marks it
/// `readOnlyHint` now. What the server says now counts, never what the saved list says, since
/// the bot can write that file; a server that cannot be reached says nothing, and the tool may
/// change things.
async fn access(app: &Arc<App>, tool: &PluginTool, cancel: &CancellationToken) -> Access {
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
    let live_tool = live.as_ref().and_then(|server| server.tools.iter().find(|candidate| candidate.name == tool.tool.name).cloned());
    if live_tool.as_ref().and_then(|live| live.annotations.as_ref()).and_then(|annotations| annotations.read_only_hint) == Some(true) {
        return Access::ReadOnly;
    }
    Access::Changes { description: live_tool.and_then(|live| live.description.map(|description| description.to_string())).unwrap_or_default() }
}

/// Whether a script's call to `name` reaches a plugin tool that only reads (see `access`).
pub async fn is_read_only(app: &Arc<App>, catalog: &PluginCatalog, name: &str, cancel: &CancellationToken) -> bool {
    match catalog.plugin_tool(name) {
        Some(tool) => matches!(access(app, &tool, cancel).await, Access::ReadOnly),
        None => false,
    }
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
    let name = tool.tool.name.to_string();
    let review_description = match access(app, &tool, ctx.cancel).await {
        Access::ReadOnly => return None,
        Access::Stopped => return Some(crate::local_review::blocked("Stopped".into())),
        Access::Changes { description } => description,
    };
    // The script the call comes from says what the whole batch is for.
    let script = ctx.parent.filter(|parent| parent.name == codemode::CODEMODE_TOOL_NAME).and_then(|parent| parent.arguments["code"].as_str());
    let outcome = super::review::decide(app, bot, chat_id, trigger, &tool.plugin_id, &tool.plugin_name, &name, &review_description, ctx.args, script, ctx.cancel).await;
    let super::review::Outcome::Ask { reason, .. } = outcome else { return None };
    if unattended {
        return Some(crate::local_review::blocked(format!(
            "{name} needs the user's permission ({}), and nobody is here to give it. Report what you would do; the user can add an Auto-review rule allowing it.",
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
    async fn execute(&self, _id: &str, args: Value, cancel: CancellationToken, _on_update: ToolUpdateFn) -> Result<ToolResult, ToolError> {
        let tool = self.tool.name.to_string();
        let server = tokio::select! {
            server = self.app.mcp.server(&self.app, &self.plugin_id, &self.server_name) => server.map_err(ToolError)?,
            _ = cancel.cancelled() => return Err(ToolError("Stopped".into())),
        };
        let mut params = CallToolRequestParams::default();
        params.name = tool.clone().into();
        params.arguments = args.as_object().cloned();
        let call = server.service.call_tool(params);
        let result = tokio::select! {
            result = tokio::time::timeout(CALL_TIMEOUT, call) => result.map_err(|_| ToolError(format!("{tool} took too long")))?,
            _ = cancel.cancelled() => return Err(ToolError("Stopped".into())),
        };
        if let Some(auth) = &server.auth {
            persist_refreshed(&self.app, &self.plugin_id, &server.name, auth).await;
        }
        let result = result.map_err(|e| ToolError(format!("{tool} failed: {e}")))?;
        let is_error = result.is_error.unwrap_or(false);
        let mut content = model_content(&result);
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

/// A plugin result as text and images: text blocks cut at `MAX_RESULT_CHARS` in all, other
/// blocks as JSON, and the structured result when there are no blocks. What an error says, and
/// what hooks read.
fn model_content(result: &rmcp::model::CallToolResult) -> Vec<ContentPart> {
    let mut content: Vec<ContentPart> = Vec::new();
    let mut text_len = 0;
    for block in &result.content {
        match block {
            ContentBlock::Text(text) => {
                let mut text = text.text.clone();
                if text_len + text.len() > MAX_RESULT_CHARS {
                    let mut cut = MAX_RESULT_CHARS.saturating_sub(text_len);
                    while cut > 0 && !text.is_char_boundary(cut) {
                        cut -= 1;
                    }
                    text.truncate(cut);
                    text.push_str("\n[truncated]");
                }
                text_len += text.len();
                content.push(ContentPart::text(text));
            }
            ContentBlock::Image(image) => content.push(ContentPart::Image { data: image.data.clone(), mime_type: image.mime_type.clone() }),
            ContentBlock::Resource(resource) => {
                if let Ok(text) = serde_json::to_string(&resource.resource) {
                    content.push(ContentPart::text(text));
                }
            }
            other => {
                if let Ok(text) = serde_json::to_string(other) {
                    content.push(ContentPart::text(text));
                }
            }
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

/// Saves tokens the transport refreshed, so the next connection does not start from a stale
/// refresh token.
async fn persist_refreshed(app: &Arc<App>, plugin_id: &str, server: &str, auth: &Arc<tokio::sync::Mutex<AuthorizationManager>>) {
    let credentials = auth.lock().await.get_credentials().await;
    if let Ok((client_id, Some(tokens))) = credentials {
        let key = format!("oauth:{server}");
        let saved = app.plugins.lock().unwrap().secret(plugin_id, &key);
        let fresh = json!({ "client_id": client_id, "tokens": tokens, "signed_in_at": now_secs() });
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
mod tests {
    use super::*;

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
                app: app.clone(),
                plugin_id: plugin_id.into(),
                plugin_name: plugin_name.into(),
                server_name: "test".into(),
                tool,
                name,
                description: description.into(),
                read_only: true,
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
        let saved = SavedCatalog {
            servers: BTreeMap::from([
                ("api".to_string(), SavedServer { instructions: Some("Use team keys like ENG.".into()), tools: vec![issues, comment, hidden] }),
                ("gone".to_string(), SavedServer { instructions: None, tools: vec![mcp_tool("old", "A server the manifest no longer has", r#"{}"#)] }),
            ]),
        };
        crate::config::write_json_private(&catalog_path(app, "linear"), &saved).unwrap();

        let local: Vec<Arc<dyn Tool>> = lorca_agent::tools::coding_tools(scratch.1.clone()).into_iter().filter(|tool| tool.name() == "read").collect();
        let catalog = turn_catalog(app, local);
        assert_eq!(plugin_briefs(app).len(), 2);
        let names: Vec<String> = catalog.entries().iter().map(|entry| entry.tool.name().to_string()).collect();
        assert_eq!(names, vec!["read", "linear__create_comment", "linear__list_issues"]);
        assert_eq!(catalog.plugin_name("linear__list_issues").as_deref(), Some("Linear"));
        assert!(catalog.plugin_tool("linear__list_issues").unwrap().read_only, "the manifest's readonly pattern applies");
        assert!(!catalog.plugin_tool("linear__create_comment").unwrap().read_only);
        assert_eq!(catalog.lookup("linear__list_issues").map(|tool| tool.original_name.clone()).as_deref(), Some("list_issues"));

        let codemode = CodemodeTool::new(catalog.clone(), CodemodeOptions::default());
        let description = codemode.description().to_string();
        assert!(description.contains("## linear (2 tools)\nLinear: Linear for the team.\nServer instructions: Use team keys like ENG."), "{description}");
        assert!(description.contains("linear__list_issues(args: {\n  // Team key\n  team: string;\n}): Promise<CallToolResult>;"), "{description}");
        assert!(description.contains("## notion (tools not known yet; searchTools() finds them)\nNotion: Notion for the team."), "{description}");
        assert!(description.contains("Your own tools `read` are callable here too"), "{description}");
        assert!(!description.contains("secret_admin") && !description.contains("linear__old"), "{description}");
        assert_eq!(CodemodeTool::new(catalog, CodemodeOptions::default()).description(), description, "the same listing every time");
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
}
