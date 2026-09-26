//! The MCP side of plugins on this Runner: a pool of connected servers (`rmcp`, stdio or
//! streamable HTTP), on-demand tool discovery for a turn, the OAuth sign-in for a remote
//! server, and Auto-review (`review.rs`) at the execution boundary. A read-only tool runs;
//! anything else asks the user in the chat first, after Grok Bot.

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
/// Reconnect a device-flow server before its bearer expires. A turn that starts inside this
/// window gets a fresh token, so the static bearer cannot expire midway through ordinary work.
const DEVICE_TOKEN_REFRESH_BUFFER_SECS: f64 = 5.0 * 60.0;
/// The most text a tool result carries to the model.
const MAX_RESULT_CHARS: usize = 50_000;
/// fx-style discovery bounds: metadata stays small, while matching executable schemas are
/// admitted only for the next model step.
const MAX_CAPABILITY_QUERY_BYTES: usize = 4 * 1024;
const CAPABILITY_RESULT_LIMIT: usize = 5;
const MAX_SEARCH_IDENTITY_BYTES: usize = 256;
const MAX_SEARCH_DESCRIPTION_BYTES: usize = 1024;
const MAX_SEARCH_INDEX_DESCRIPTION_BYTES: usize = 2 * 1024;
const MAX_SEARCH_SCHEMA_BYTES: usize = 4 * 1024;
const MAX_SERVER_INSTRUCTIONS_BYTES: usize = 2 * 1024;
const MAX_SEARCH_RESULT_BYTES: usize = 16 * 1024;
const MCP_SCHEMA_LOAD_BUDGET_BYTES: usize = 64 * 1024;

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
    /// What started the turn, which the review reads as the request behind a call.
    trigger: super::review::Trigger,
    bot_id: String,
    /// A routine run has nobody to ask, so a tool that needs permission is refused.
    unattended: bool,
    plugin_id: String,
    plugin_name: String,
    server: Arc<Server>,
    tool: rmcp::model::Tool,
    name: String,
    /// The model also sees bounded server instructions in `description`; permission review
    /// keeps using the server's own tool description as before.
    review_description: String,
    description: String,
    read_only: bool,
}

/// One connected MCP tool in this turn's searchable catalog. Its executable schema stays out
/// of model context until discovery selects it.
struct CatalogTool {
    name: String,
    original_name: String,
    plugin_id: String,
    plugin_name: String,
    server_name: String,
    description: String,
    search_schema: String,
    server_instructions: String,
    read_only: bool,
    schema_bytes: usize,
    executable: Arc<dyn Tool>,
}

#[derive(Default)]
struct TurnToolsState {
    catalog: HashMap<String, Arc<CatalogTool>>,
    selected: BTreeMap<String, Arc<dyn Tool>>,
}

/// MCP discovery state for one agent turn. Installed plugin names are cheap prompt metadata;
/// servers connect only when `capability_search` or `mcp_select_tool` needs them, and selected
/// schemas join the agent loop on its next model step.
pub struct TurnTools {
    app: Arc<App>,
    chat_id: String,
    trigger: super::review::Trigger,
    bot_id: String,
    unattended: bool,
    state: Mutex<TurnToolsState>,
}

impl TurnTools {
    fn new(app: Arc<App>, chat_id: &str, trigger: &super::review::Trigger, bot: &Bot, unattended: bool) -> Self {
        TurnTools {
            app,
            chat_id: chat_id.to_string(),
            trigger: trigger.clone(),
            bot_id: bot.id.clone(),
            unattended,
            state: Mutex::new(TurnToolsState::default()),
        }
    }

    /// The two fixed, small schemas a model sees before it asks for any MCP capability.
    pub fn discovery_tools(self: &Arc<Self>) -> Vec<Arc<dyn Tool>> {
        vec![
            Arc::new(CapabilitySearch { turn: self.clone() }),
            Arc::new(McpSelectTool { turn: self.clone() }),
        ]
    }

    /// Adds every schema selected so far, preserving the fixed tool order and avoiding a
    /// duplicate when this is called on a context already updated by the loop hook.
    pub fn tools_with_selected(&self, base: &[Arc<dyn Tool>]) -> Vec<Arc<dyn Tool>> {
        let selected: Vec<Arc<dyn Tool>> = self.state.lock().unwrap().selected.values().cloned().collect();
        let mut tools = base.to_vec();
        for tool in selected {
            if !tools.iter().any(|candidate| candidate.name() == tool.name()) {
                tools.push(tool);
            }
        }
        tools
    }

    /// Selects a tool as `mcp_select_tool` does, for tests elsewhere in the crate.
    #[cfg(test)]
    pub(crate) fn select(&self, tool: Arc<dyn Tool>) {
        self.state.lock().unwrap().selected.insert(tool.name().to_string(), tool);
    }

    /// The label for a dynamic tool's working row.
    pub fn plugin_name(&self, tool_name: &str) -> Option<String> {
        self.state.lock().unwrap().catalog.get(tool_name).map(|tool| tool.plugin_name.clone())
    }

    async fn load_plugin(&self, plugin: &Installed, cancel: &CancellationToken) -> Result<Vec<String>, ToolError> {
        let status = self.app.plugins.lock().unwrap().status(&plugin.manifest.id);
        if let Some(status) = status.filter(|status| status.state == "needs_setup" || status.state == "needs_auth") {
            return Ok(vec![status.detail]);
        }

        let mut problems = Vec::new();
        for server_name in plugin.manifest.servers.keys() {
            let connection = self.app.mcp.server(&self.app, &plugin.manifest.id, server_name);
            let server = tokio::select! {
                result = connection => match result {
                    Ok(server) => server,
                    Err(error) => {
                        tracing::warn!(%error, plugin = %plugin.manifest.id, server = %server_name, "connecting a plugin server for capability discovery");
                        problems.push(error);
                        continue;
                    }
                },
                _ = cancel.cancelled() => return Err(ToolError("Stopped".into())),
            };
            self.catalog_server(plugin, server);
        }
        Ok(problems)
    }

    fn catalog_server(&self, plugin: &Installed, server: Arc<Server>) {
        let mut state = self.state.lock().unwrap();
        for tool in &server.tools {
            if plugin.manifest.tools.hide.iter().any(|pattern| pattern_matches(pattern, &tool.name)) {
                continue;
            }
            let original_name = tool.name.to_string();
            if state.catalog.values().any(|known| {
                known.plugin_id == plugin.manifest.id && known.server_name == server.name && known.original_name == original_name
            }) {
                continue;
            }

            let base_name = tool_name(&plugin.manifest.id, &original_name);
            let name = unique_tool_name(&base_name, &state.catalog);
            let server_instructions = server
                .instructions
                .as_deref()
                .map(|text| utf8_prefix(text, MAX_SERVER_INSTRUCTIONS_BYTES).to_string())
                .unwrap_or_default();
            let plain_description = tool.description.as_deref().unwrap_or("").to_string();
            let description = if server_instructions.is_empty() {
                plain_description.clone()
            } else if plain_description.is_empty() {
                format!("Server instructions: {server_instructions}")
            } else {
                format!("{plain_description}\n\nServer instructions: {server_instructions}")
            };
            let read_only = tool.annotations.as_ref().and_then(|annotations| annotations.read_only_hint).unwrap_or(false)
                || plugin.manifest.tools.readonly.iter().any(|pattern| pattern_matches(pattern, &original_name));
            let executable: Arc<dyn Tool> = Arc::new(PluginTool {
                app: self.app.clone(),
                chat_id: self.chat_id.clone(),
                trigger: self.trigger.clone(),
                bot_id: self.bot_id.clone(),
                unattended: self.unattended,
                plugin_id: plugin.manifest.id.clone(),
                plugin_name: plugin.manifest.name.clone(),
                server: server.clone(),
                tool: tool.clone(),
                name: name.clone(),
                review_description: plain_description.clone(),
                description,
                read_only,
            });
            let schema_bytes = serde_json::to_vec(&executable.spec()).map(|json| json.len()).unwrap_or(usize::MAX);
            let raw_search_schema = serde_json::to_string(&Value::Object((*tool.input_schema).clone())).unwrap_or_default();
            let search_schema = utf8_prefix(&raw_search_schema, MAX_SEARCH_SCHEMA_BYTES).to_string();
            let search_description = utf8_prefix(&plain_description, MAX_SEARCH_INDEX_DESCRIPTION_BYTES).to_string();
            state.catalog.insert(
                name.clone(),
                Arc::new(CatalogTool {
                    name,
                    original_name,
                    plugin_id: plugin.manifest.id.clone(),
                    plugin_name: plugin.manifest.name.clone(),
                    server_name: server.name.clone(),
                    description: search_description,
                    search_schema,
                    server_instructions,
                    read_only,
                    schema_bytes,
                    executable,
                }),
            );
        }
    }

    async fn search(&self, query: &str, plugin_id: Option<&str>, cancel: &CancellationToken) -> Result<Value, ToolError> {
        if query.is_empty() {
            return Err(ToolError("capability_search requires a non-empty query".into()));
        }
        if query.len() > MAX_CAPABILITY_QUERY_BYTES {
            return Err(ToolError(format!("capability_search query exceeds {MAX_CAPABILITY_QUERY_BYTES} bytes")));
        }

        let installed = self.app.plugins.lock().unwrap().installed().to_vec();
        let plugins: Vec<Installed> = match plugin_id {
            Some(id) => installed.into_iter().filter(|plugin| plugin.manifest.id == id).collect(),
            None => installed,
        };
        if plugin_id.is_some() && plugins.is_empty() {
            return Ok(json!({
                "tools": [],
                "count": 0,
                "total_matches": 0,
                "state": "plugin_not_found"
            }));
        }

        let mut problems = Vec::new();
        for plugin in &plugins {
            for problem in self.load_plugin(plugin, cancel).await? {
                if problems.len() < CAPABILITY_RESULT_LIMIT {
                    problems.push(json!({
                        "plugin": bounded_json_text(&plugin.manifest.id, MAX_SEARCH_IDENTITY_BYTES),
                        "message": bounded_json_text(&problem, 512)
                    }));
                }
            }
        }

        let candidates: Vec<Arc<CatalogTool>> = self
            .state
            .lock()
            .unwrap()
            .catalog
            .values()
            .filter(|tool| plugin_id.is_none_or(|id| tool.plugin_id == id))
            .cloned()
            .collect();
        let ranked = rank_tools(query, &candidates, plugin_id.is_some());
        let total_matches = ranked.len();
        let matches: Vec<Arc<CatalogTool>> = ranked.into_iter().take(CAPABILITY_RESULT_LIMIT).collect();
        let (loaded, rejected, schema_budget_exhausted) = self.select_search_matches(&matches);
        let rows: Vec<Value> = matches
            .iter()
            .map(|tool| {
                json!({
                    "name": tool.name,
                    "plugin": bounded_json_text(&tool.plugin_id, MAX_SEARCH_IDENTITY_BYTES),
                    "server": bounded_json_text(&tool.server_name, MAX_SEARCH_IDENTITY_BYTES),
                    "description": bounded_json_text(&tool.description, MAX_SEARCH_DESCRIPTION_BYTES),
                    "read_only": tool.read_only,
                })
            })
            .collect();
        let state = if total_matches == 0 {
            if candidates.is_empty() && !problems.is_empty() { "unavailable" } else { "no_match" }
        } else {
            "ready"
        };
        let value = json!({
            "tools": rows,
            "count": matches.len(),
            "total_matches": total_matches,
            "more_available": total_matches > matches.len(),
            "schemas_loaded": loaded,
            "schemas_rejected": rejected,
            "schema_budget_exhausted": schema_budget_exhausted,
            "problems": problems,
            "state": state,
        });
        let encoded = serde_json::to_vec(&value).unwrap_or_default();
        if encoded.len() > MAX_SEARCH_RESULT_BYTES {
            return Err(ToolError("capability_search result exceeded its context budget; use a narrower query or plugin".into()));
        }
        Ok(value)
    }

    fn select_search_matches(&self, matches: &[Arc<CatalogTool>]) -> (Vec<String>, Vec<String>, bool) {
        let mut state = self.state.lock().unwrap();
        let mut remaining = MCP_SCHEMA_LOAD_BUDGET_BYTES;
        let mut loaded = Vec::new();
        let mut rejected = Vec::new();
        let mut exhausted = false;
        for tool in matches {
            if state.selected.contains_key(&tool.name) {
                loaded.push(tool.name.clone());
                continue;
            }
            if tool.schema_bytes > MCP_SCHEMA_LOAD_BUDGET_BYTES {
                rejected.push(tool.name.clone());
                continue;
            }
            if tool.schema_bytes > remaining {
                exhausted = true;
                break;
            }
            remaining -= tool.schema_bytes;
            state.selected.insert(tool.name.clone(), tool.executable.clone());
            loaded.push(tool.name.clone());
        }
        (loaded, rejected, exhausted)
    }

    async fn select_exact(&self, name: &str, cancel: &CancellationToken) -> Result<String, ToolError> {
        if !self.state.lock().unwrap().catalog.contains_key(name) {
            if let Some((plugin_id, _)) = name.split_once("__") {
                let plugin = self
                    .app
                    .plugins
                    .lock()
                    .unwrap()
                    .installed()
                    .iter()
                    .find(|plugin| plugin.manifest.id == plugin_id)
                    .cloned();
                if let Some(plugin) = plugin {
                    let _ = self.load_plugin(&plugin, cancel).await?;
                }
            }
        }
        let mut state = self.state.lock().unwrap();
        let tool = state
            .catalog
            .get(name)
            .cloned()
            .ok_or_else(|| ToolError(format!("Dynamic MCP tool not found: {name}. Use capability_search and copy an exact returned name.")))?;
        if tool.schema_bytes > MCP_SCHEMA_LOAD_BUDGET_BYTES {
            return Err(ToolError(format!(
                "The schema for {name} is {} bytes, over the {}-byte MCP schema limit.",
                tool.schema_bytes, MCP_SCHEMA_LOAD_BUDGET_BYTES
            )));
        }
        state.selected.insert(tool.name.clone(), tool.executable.clone());
        Ok(tool.plugin_name.clone())
    }
}

/// Creates the cheap per-turn catalog plus the prompt metadata for installed plugins. No MCP
/// process or HTTP connection starts here.
pub fn turn_tools(app: &Arc<App>, chat_id: &str, trigger: &super::review::Trigger, bot: &Bot, unattended: bool) -> (Arc<TurnTools>, Vec<PluginBrief>) {
    let briefs = {
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
                                app.config
                                    .plugins_dir()
                                    .join(&plugin.manifest.id)
                                    .join("skills")
                                    .join(format!("{}.md", super::slug(&skill.name))),
                            )
                        })
                        .collect(),
                }
            })
            .collect()
    };
    (Arc::new(TurnTools::new(app.clone(), chat_id, trigger, bot, unattended)), briefs)
}

/// What the system prompt says about one installed plugin. Tool names, descriptions, schemas,
/// and server instructions arrive only through capability discovery.
pub struct PluginBrief {
    pub id: String,
    pub name: String,
    pub state: String,
    pub detail: String,
    /// (name, description, path)
    pub skills: Vec<(String, String, std::path::PathBuf)>,
}

struct CapabilitySearch {
    turn: Arc<TurnTools>,
}

#[async_trait]
impl Tool for CapabilitySearch {
    fn name(&self) -> &str {
        "capability_search"
    }
    fn description(&self) -> &str {
        "Find tools in installed plugins for a capability needed by the current task. Optionally restrict the search to one exact plugin id. Matching MCP schemas load automatically within a bounded budget and become callable on the next model step. Results describe only this query; no_match does not rule out a better query. Use exact returned names and never guess them."
    }
    fn parameters(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "query": { "type": "string", "description": "Natural-language capability needed for the current task", "minLength": 1, "maxLength": MAX_CAPABILITY_QUERY_BYTES },
                "plugin": { "type": "string", "description": "Optional exact installed plugin id from the prompt catalog", "minLength": 1 }
            },
            "required": ["query"],
            "additionalProperties": false
        })
    }
    fn execution_mode(&self) -> Option<ToolExecutionMode> {
        Some(ToolExecutionMode::Sequential)
    }
    async fn execute(&self, _id: &str, args: Value, cancel: CancellationToken, _on_update: ToolUpdateFn) -> Result<ToolResult, ToolError> {
        let query = args["query"].as_str().unwrap_or("").trim();
        let plugin = args["plugin"].as_str().map(str::trim).filter(|value| !value.is_empty());
        let result = self.turn.search(query, plugin, &cancel).await?;
        let count = result["count"].as_u64().unwrap_or(0);
        let text = serde_json::to_string(&result).unwrap_or_default();
        Ok(ToolResult::text(text).with_details(json!({ "summary": format!("Found {count} plugin tools") })))
    }
}

struct McpSelectTool {
    turn: Arc<TurnTools>,
}

#[async_trait]
impl Tool for McpSelectTool {
    fn name(&self) -> &str {
        "mcp_select_tool"
    }
    fn description(&self) -> &str {
        "Select one exact dynamic MCP tool name returned by capability_search so its executable schema is advertised on the next model step. Do not use partial names, guess a name, select built-in tools, or treat this as executing the selected tool."
    }
    fn parameters(&self) -> Value {
        json!({
            "type": "object",
            "properties": { "name": { "type": "string", "description": "Exact dynamic MCP tool name returned by capability_search", "minLength": 1 } },
            "required": ["name"],
            "additionalProperties": false
        })
    }
    fn execution_mode(&self) -> Option<ToolExecutionMode> {
        Some(ToolExecutionMode::Sequential)
    }
    async fn execute(&self, _id: &str, args: Value, cancel: CancellationToken, _on_update: ToolUpdateFn) -> Result<ToolResult, ToolError> {
        let name = args["name"].as_str().unwrap_or("").trim();
        if name.is_empty() {
            return Err(ToolError("mcp_select_tool requires an exact dynamic tool name".into()));
        }
        let plugin = self.turn.select_exact(name, &cancel).await?;
        Ok(ToolResult::text(format!(
            "Selected dynamic MCP tool `{name}` from {plugin}. Its schema is available on the next model step; call `{name}` then with arguments matching that schema."
        ))
        .with_details(json!({ "summary": format!("Selected {name}") })))
    }
}

fn unique_tool_name(base: &str, catalog: &HashMap<String, Arc<CatalogTool>>) -> String {
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

/// A JSON string scalar whose encoded representation fits the model-output budget. Control
/// characters expand when escaped, so a raw byte prefix alone is not enough.
fn bounded_json_text(text: &str, max_bytes: usize) -> String {
    if serde_json::to_string(text).map(|encoded| encoded.len()).unwrap_or(usize::MAX) <= max_bytes {
        return text.to_string();
    }
    let mut end = utf8_prefix(text, max_bytes).len();
    while end > 0 {
        let candidate = &text[..end];
        if serde_json::to_string(candidate).map(|encoded| encoded.len()).unwrap_or(usize::MAX) <= max_bytes {
            return candidate.to_string();
        }
        end -= 1;
        while end > 0 && !text.is_char_boundary(end) {
            end -= 1;
        }
    }
    String::new()
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
            let outcome = super::review::decide(
                &self.app,
                &bot,
                &self.chat_id,
                &self.trigger,
                &self.plugin_id,
                &self.plugin_name,
                &tool,
                &self.review_description,
                &args,
                &cancel,
            )
            .await;
            if let super::review::Outcome::Ask { reason, .. } = outcome {
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
                    Decision::Dismissed => return Ok(dismissed_call(format!("The user sent a new message instead of answering, so {tool} did not run. Follow that message."))),
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
            *d = decision.as_str().into();
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

    struct StubTool {
        name: String,
        description: String,
        parameters: Value,
    }

    #[async_trait::async_trait]
    impl Tool for StubTool {
        fn name(&self) -> &str {
            &self.name
        }
        fn description(&self) -> &str {
            &self.description
        }
        fn parameters(&self) -> Value {
            self.parameters.clone()
        }
        async fn execute(&self, _id: &str, _args: Value, _cancel: CancellationToken, _on_update: ToolUpdateFn) -> Result<ToolResult, ToolError> {
            Ok(ToolResult::text("ok"))
        }
    }

    fn catalog_tool(plugin_id: &str, plugin_name: &str, original_name: &str, description: &str, schema: &str) -> Arc<CatalogTool> {
        let name = tool_name(plugin_id, original_name);
        let executable: Arc<dyn Tool> = Arc::new(StubTool {
            name: name.clone(),
            description: description.into(),
            parameters: serde_json::from_str(schema).unwrap(),
        });
        Arc::new(CatalogTool {
            name,
            original_name: original_name.into(),
            plugin_id: plugin_id.into(),
            plugin_name: plugin_name.into(),
            server_name: "test".into(),
            description: description.into(),
            search_schema: schema.into(),
            server_instructions: String::new(),
            read_only: true,
            schema_bytes: serde_json::to_vec(&executable.spec()).unwrap().len(),
            executable,
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
        assert_eq!(tool_name("my-server", "weird.name/x"), "my-server__weird_name_x");
        assert_eq!(tool_name("p", &"x".repeat(100)).len(), 64);
        // serde_json keeps object keys sorted, so the summary lists them alphabetically.
        assert_eq!(call_summary("create_issue", &json!({ "repo": "lorca", "title": "Fix   the relay", "body": "x".repeat(80) })), format!("create_issue · body: {}…, repo: lorca, title: Fix the relay", "x".repeat(60)));
        assert_eq!(call_summary("get_me", &json!({})), "get_me");
        assert_eq!(Decision::parse("always"), Some(Decision::Always));
        assert_eq!(Decision::parse("nope"), None);
    }

    #[test]
    fn capability_search_ranks_intent_and_exact_identity() {
        let tools = vec![
            catalog_tool(
                "github",
                "GitHub",
                "create_issue",
                "Create an issue in a GitHub repository",
                r#"{"type":"object","properties":{"repo":{"type":"string"},"title":{"type":"string"}}}"#,
            ),
            catalog_tool(
                "github",
                "GitHub",
                "list_pull_requests",
                "List pull requests in a repository",
                r#"{"type":"object","properties":{"repo":{"type":"string"}}}"#,
            ),
            catalog_tool(
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
    fn discovery_bounds_utf8_and_names_without_retargeting() {
        assert_eq!(utf8_prefix("aéz", 2), "a");
        let bounded = bounded_json_text(&"\n".repeat(1_000), 128);
        assert!(serde_json::to_string(&bounded).unwrap().len() <= 128);
        assert_eq!(query_tokens("GitHub github issue"), vec!["github", "issue"]);

        let first = catalog_tool("github", "GitHub", "create.issue", "Create", r#"{"type":"object"}"#);
        let mut catalog = HashMap::new();
        catalog.insert(first.name.clone(), first.clone());
        assert_eq!(unique_tool_name(&first.name, &catalog), "github__create_issue_2");
    }

    #[test]
    fn selected_schemas_join_the_next_context_once() {
        let home = std::env::temp_dir().join(format!("lorca-mcp-turn-{}", uuid::Uuid::new_v4()));
        let app = App::load(crate::config::Config { home: home.clone(), port: 0 }).unwrap();
        let bot = Bot {
            id: "bot".into(),
            name: "Chef".into(),
            description: String::new(),
            symbol_name: String::new(),
            accent: String::new(),
            avatar: None,
            runner_id: "runner".into(),
            harness: crate::model::Harness::default(), codex_options: crate::model::CodexOptions::default(),
            provider: "deepseek".into(),
            model: None,
            thinking: None,
            legacy_instructions: String::new(),
            workdir: None,
            created_at: 0.0,
        };
        let turn = Arc::new(TurnTools::new(app.clone(), "chat", &Default::default(), &bot, false));
        let discovery = turn.discovery_tools();
        assert_eq!(discovery.iter().map(|tool| tool.name()).collect::<Vec<_>>(), vec!["capability_search", "mcp_select_tool"]);
        assert!(discovery.iter().map(|tool| serde_json::to_vec(&tool.spec()).unwrap().len()).sum::<usize>() < 4 * 1024);
        let base: Arc<dyn Tool> = Arc::new(StubTool {
            name: "base".into(),
            description: "base".into(),
            parameters: json!({ "type": "object" }),
        });
        let dynamic = catalog_tool("github", "GitHub", "create_issue", "Create an issue", r#"{"type":"object"}"#);
        turn.state.lock().unwrap().catalog.insert(dynamic.name.clone(), dynamic.clone());

        assert_eq!(turn.tools_with_selected(std::slice::from_ref(&base)).len(), 1);
        let (loaded, rejected, exhausted) = turn.select_search_matches(std::slice::from_ref(&dynamic));
        assert_eq!(loaded, vec!["github__create_issue"]);
        assert!(rejected.is_empty());
        assert!(!exhausted);
        let tools = turn.tools_with_selected(std::slice::from_ref(&base));
        assert_eq!(tools.iter().map(|tool| tool.name()).collect::<Vec<_>>(), vec!["base", "github__create_issue"]);
        assert_eq!(turn.tools_with_selected(&tools).len(), 2);

        drop(turn);
        drop(app);
        let _ = std::fs::remove_dir_all(home);
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
