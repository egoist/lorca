//! Plugins: MCP servers a bot can use, after Grok Bot's marketplace. A plugin is a manifest
//! (`plugin.json`: servers, variables, skills, tool hints) that a Runner installs; the Runner
//! keeps the package, the variables, the secrets, and the OAuth tokens, and advertises what it
//! has in its machine blob. Every bot on the Runner may use every plugin installed there.
//! The MCP side, with the permission gate, is `mcp` under the `runner` feature.

#[cfg(feature = "runner")]
pub mod mcp;
#[cfg(feature = "runner")]
pub mod review;

use std::collections::BTreeMap;
use std::sync::Arc;

use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

use crate::app::App;
use crate::config::{self, now_secs};
use crate::model::PluginStatus;

/// The bundled marketplace: first-party manifests.
const BUNDLED_INDEX: &str = include_str!("../../marketplace/index.json");

/// How long a fetched marketplace index is kept before it is asked for again.
const INDEX_TTL_SECS: f64 = 3600.0;

// MARK: - Manifest

/// `plugin.json`.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Manifest {
    /// Lowercase, `[a-z0-9-]`, unique on a Runner.
    pub id: String,
    pub name: String,
    #[serde(default)]
    pub description: String,
    #[serde(default)]
    pub version: String,
    /// An SF Symbol name for the apps.
    #[serde(default)]
    pub icon: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub homepage: Option<String>,
    /// Search words for the marketplace.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub tags: Vec<String>,
    #[serde(default)]
    pub servers: BTreeMap<String, ServerSpec>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub variables: Vec<VariableSpec>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub skills: Vec<SkillSpec>,
    #[serde(default, skip_serializing_if = "ToolHints::is_empty")]
    pub tools: ToolHints,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ServerSpec {
    /// A local process speaking MCP on stdio. `${VAR}` in `args` and `env` is filled from the
    /// plugin's variables and secrets.
    Stdio {
        command: String,
        #[serde(default)]
        args: Vec<String>,
        #[serde(default)]
        env: BTreeMap<String, String>,
    },
    /// A streamable-HTTP server. `${VAR}` in `headers` is filled the same way.
    Http {
        url: String,
        #[serde(default)]
        headers: BTreeMap<String, String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        auth: Option<AuthSpec>,
    },
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum AuthSpec {
    /// The MCP authorization flow (discovery, dynamic client registration, PKCE), signed in
    /// on the Runner. With `token_variable`, a token the user pasted is used instead when set.
    Oauth {
        #[serde(default)]
        scopes: Vec<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        token_variable: Option<String>,
        /// For a server that does not register clients on the fly (GitHub): the variables
        /// holding an OAuth app's client id and secret the user created, or fixed values.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        client_id_variable: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        client_secret_variable: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        client_id: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        client_secret: Option<String>,
        /// With a client id: sign in with the device flow (RFC 8628) instead of a browser
        /// callback. The card shows a code to enter at the link; nothing has to run on the
        /// Runner's screen, and no client secret ships.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        device_authorization_endpoint: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        token_endpoint: Option<String>,
    },
    /// `Authorization: Bearer <variable>`.
    Bearer { variable: String },
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct VariableSpec {
    pub name: String,
    #[serde(default)]
    pub description: String,
    /// Kept in the Runner's secrets file, never shown back.
    #[serde(default)]
    pub secret: bool,
    #[serde(default)]
    pub required: bool,
}

/// A note the bot reads when relevant, written to the plugin's folder at install.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct SkillSpec {
    pub name: String,
    #[serde(default)]
    pub description: String,
    pub content: String,
}

/// What the manifest says about tools the server does not annotate itself. Patterns are tool
/// names with an optional trailing `*`.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
pub struct ToolHints {
    /// Run without asking.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub readonly: Vec<String>,
    /// Never offered to the bot.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub hide: Vec<String>,
}

impl ToolHints {
    fn is_empty(&self) -> bool {
        self.readonly.is_empty() && self.hide.is_empty()
    }
}

/// `search_*` matches `search_issues`; `list_issues` matches itself only.
pub fn pattern_matches(pattern: &str, name: &str) -> bool {
    match pattern.strip_suffix('*') {
        Some(prefix) => name.starts_with(prefix),
        None => pattern == name,
    }
}

impl Manifest {
    /// Reads a manifest, refusing one that could not be installed.
    pub fn parse(value: &Value) -> Result<Manifest, String> {
        let manifest: Manifest = serde_json::from_value(value.clone()).map_err(|e| format!("Not a plugin manifest: {e}"))?;
        manifest.check()?;
        Ok(manifest)
    }

    pub fn check(&self) -> Result<(), String> {
        if self.id.is_empty() || !self.id.chars().all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-') {
            return Err(format!("A plugin id is lowercase letters, digits, and dashes; {:?} is not.", self.id));
        }
        if self.name.trim().is_empty() {
            return Err("The plugin has no name.".into());
        }
        if self.servers.is_empty() {
            return Err(format!("{} declares no MCP server.", self.name));
        }
        for (name, server) in &self.servers {
            if name.is_empty() {
                return Err("A server needs a name.".into());
            }
            match server {
                ServerSpec::Stdio { command, .. } if command.trim().is_empty() => return Err(format!("Server {name} has no command.")),
                ServerSpec::Http { url, .. } if !(url.starts_with("http://") || url.starts_with("https://")) => {
                    return Err(format!("Server {name} has no http(s) URL."))
                }
                _ => {}
            }
        }
        Ok(())
    }

    /// An inline MCP server the user pasted, as the app's "Add MCP Server" does: the usual
    /// `{ "mcpServers": { "name": { "command", "args", "env" } | { "url", "headers" } } }`,
    /// or one server object alone.
    pub fn from_mcp_json(name: &str, json: &Value) -> Result<Manifest, String> {
        let mut servers = BTreeMap::new();
        let entries: Vec<(String, Value)> = match json.get("mcpServers").and_then(Value::as_object) {
            Some(map) => map.iter().map(|(k, v)| (k.clone(), v.clone())).collect(),
            None => vec![(slug(name), json.clone())],
        };
        for (server_name, entry) in entries {
            let spec = if let Some(url) = entry.get("url").and_then(Value::as_str) {
                let headers = entry
                    .get("headers")
                    .and_then(Value::as_object)
                    .map(|h| h.iter().filter_map(|(k, v)| v.as_str().map(|v| (k.clone(), v.to_string()))).collect())
                    .unwrap_or_default();
                ServerSpec::Http { url: url.to_string(), headers, auth: None }
            } else if let Some(command) = entry.get("command").and_then(Value::as_str) {
                let args = entry
                    .get("args")
                    .and_then(Value::as_array)
                    .map(|a| a.iter().filter_map(Value::as_str).map(str::to_string).collect())
                    .unwrap_or_default();
                let env = entry
                    .get("env")
                    .and_then(Value::as_object)
                    .map(|e| e.iter().filter_map(|(k, v)| v.as_str().map(|v| (k.clone(), v.to_string()))).collect())
                    .unwrap_or_default();
                ServerSpec::Stdio { command: command.to_string(), args, env }
            } else {
                return Err(format!("Server {server_name} needs a command or a url."));
            };
            servers.insert(server_name, spec);
        }
        let name = name.trim();
        if name.is_empty() {
            return Err("Give the server a name.".into());
        }
        let manifest = Manifest {
            id: slug(name),
            name: name.to_string(),
            description: "An MCP server added by hand.".into(),
            version: String::new(),
            icon: "server.rack".into(),
            homepage: None,
            tags: Vec::new(),
            servers,
            variables: Vec::new(),
            skills: Vec::new(),
            tools: ToolHints::default(),
        };
        manifest.check()?;
        Ok(manifest)
    }
}

/// `My GitHub Server` → `my-github-server`.
pub fn slug(name: &str) -> String {
    let mut out = String::new();
    for c in name.trim().to_lowercase().chars() {
        if c.is_ascii_lowercase() || c.is_ascii_digit() {
            out.push(c);
        } else if !out.ends_with('-') && !out.is_empty() {
            out.push('-');
        }
    }
    out.trim_end_matches('-').to_string()
}

/// Fills `${VAR}` from the plugin's variables and secrets; an unknown name stays as it is.
pub fn fill(template: &str, values: &BTreeMap<String, String>) -> String {
    let mut out = String::new();
    let mut rest = template;
    while let Some(start) = rest.find("${") {
        out.push_str(&rest[..start]);
        match rest[start + 2..].find('}') {
            Some(end) => {
                let name = &rest[start + 2..start + 2 + end];
                match values.get(name) {
                    Some(value) => out.push_str(value),
                    None => out.push_str(&rest[start..start + 3 + end]),
                }
                rest = &rest[start + 3 + end..];
            }
            None => {
                out.push_str(&rest[start..]);
                rest = "";
            }
        }
    }
    out.push_str(rest);
    out
}

// MARK: - The Runner's store

/// One plugin as installed on this Runner.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Installed {
    pub manifest: Manifest,
    /// `marketplace`, `inline`, or the URL it came from.
    pub source: String,
    pub installed_at: f64,
    /// The plain variables. Secret ones live in `secrets.json`.
    #[serde(default)]
    pub variables: BTreeMap<String, String>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
struct InstalledFile {
    #[serde(default)]
    plugins: Vec<Installed>,
}

/// Secrets by plugin id: variable values, and `oauth:<server>` tokens as JSON.
type SecretsFile = BTreeMap<String, BTreeMap<String, Value>>;

/// What this Runner has installed, with the secrets kept apart. Held by the App.
#[derive(Debug, Default)]
pub struct Store {
    installed: Vec<Installed>,
    secrets: SecretsFile,
    /// Connection state the MCP side reports: `connecting`, or an error message.
    pub notes: BTreeMap<String, (String, String)>,
}

impl Store {
    pub fn load(config: &config::Config) -> Store {
        let dir = config.plugins_dir();
        let installed: InstalledFile = config::read_json(&dir.join("installed.json")).unwrap_or_default();
        let secrets: SecretsFile = config::read_json(&dir.join("secrets.json")).unwrap_or_default();
        Store { installed: installed.plugins, secrets, notes: BTreeMap::new() }
    }

    fn save(&self, config: &config::Config) -> anyhow::Result<()> {
        let dir = config.plugins_dir();
        std::fs::create_dir_all(&dir)?;
        config::set_private(&dir)?;
        config::write_json_private(&dir.join("installed.json"), &InstalledFile { plugins: self.installed.clone() })?;
        config::write_json_private(&dir.join("secrets.json"), &self.secrets)?;
        Ok(())
    }

    pub fn installed(&self) -> &[Installed] {
        &self.installed
    }

    pub fn get(&self, id: &str) -> Option<&Installed> {
        self.installed.iter().find(|p| p.manifest.id == id)
    }

    /// Every value a server template may use: plain variables and secret ones.
    pub fn values(&self, id: &str) -> BTreeMap<String, String> {
        let mut values: BTreeMap<String, String> = self.get(id).map(|p| p.variables.clone()).unwrap_or_default();
        if let Some(secrets) = self.secrets.get(id) {
            for (key, value) in secrets {
                if let Some(text) = value.as_str() {
                    values.insert(key.clone(), text.to_string());
                }
            }
        }
        values
    }

    pub fn secret(&self, id: &str, key: &str) -> Option<Value> {
        self.secrets.get(id).and_then(|s| s.get(key)).cloned()
    }

    fn set_secret(&mut self, id: &str, key: &str, value: Option<Value>) {
        let entry = self.secrets.entry(id.to_string()).or_default();
        match value {
            Some(value) => {
                entry.insert(key.to_string(), value);
            }
            None => {
                entry.remove(key);
            }
        }
    }

    /// The names of the variables that have a value, secrets included.
    pub fn set_variables(&self, id: &str) -> Vec<String> {
        self.values(id).keys().cloned().collect()
    }

    /// How each installed plugin stands, for the machine blob and the apps.
    pub fn statuses(&self) -> Vec<PluginStatus> {
        self.installed.iter().map(|p| self.status_of(p)).collect()
    }

    pub fn status(&self, id: &str) -> Option<PluginStatus> {
        self.get(id).map(|p| self.status_of(p))
    }

    fn status_of(&self, plugin: &Installed) -> PluginStatus {
        let manifest = &plugin.manifest;
        let values = self.values(&manifest.id);
        let missing: Vec<&str> = manifest
            .variables
            .iter()
            .filter(|v| v.required && !values.contains_key(&v.name))
            .map(|v| v.name.as_str())
            .collect();
        let (state, detail) = if let Some((state, detail)) = self.notes.get(&manifest.id) {
            (state.clone(), detail.clone())
        } else if !missing.is_empty() {
            ("needs_setup".to_string(), format!("Needs {}", missing.join(", ")))
        } else if let Some(server) = manifest.servers.iter().find(|(name, spec)| self.needs_sign_in(&manifest.id, name, spec, &values)).map(|(n, _)| n) {
            ("needs_auth".to_string(), if manifest.servers.len() > 1 { format!("Sign in to {server}") } else { "Sign in".to_string() })
        } else {
            ("ready".to_string(), "Ready".to_string())
        };
        PluginStatus {
            id: manifest.id.clone(),
            name: manifest.name.clone(),
            description: manifest.description.clone(),
            version: manifest.version.clone(),
            icon: manifest.icon.clone(),
            state,
            detail,
        }
    }

    /// An OAuth server with no tokens and no pasted token.
    pub fn needs_sign_in(&self, id: &str, server: &str, spec: &ServerSpec, values: &BTreeMap<String, String>) -> bool {
        match spec {
            ServerSpec::Http { auth: Some(AuthSpec::Oauth { token_variable, .. }), .. } => {
                let pasted = token_variable.as_ref().map(|v| values.contains_key(v)).unwrap_or(false);
                !pasted && self.secret(id, &format!("oauth:{server}")).is_none()
            }
            _ => false,
        }
    }
}

// MARK: - Installing

/// Installs or updates a plugin on this Runner and writes its skills to its folder.
pub fn install(app: &Arc<App>, manifest: Manifest, source: &str) -> Result<PluginStatus, String> {
    manifest.check()?;
    let dir = app.config.plugins_dir().join(&manifest.id);
    let skills = dir.join("skills");
    std::fs::create_dir_all(&skills).map_err(|e| e.to_string())?;
    for skill in &manifest.skills {
        let path = skills.join(format!("{}.md", slug(&skill.name)));
        std::fs::write(&path, skill.content.as_bytes()).map_err(|e| e.to_string())?;
    }
    let status = {
        let mut store = app.plugins.lock().unwrap();
        match store.installed.iter_mut().find(|p| p.manifest.id == manifest.id) {
            Some(existing) => {
                existing.manifest = manifest.clone();
                existing.source = source.to_string();
            }
            None => store.installed.push(Installed { manifest: manifest.clone(), source: source.to_string(), installed_at: now_secs(), variables: BTreeMap::new() }),
        }
        store.notes.remove(&manifest.id);
        store.save(&app.config).map_err(|e| e.to_string())?;
        store.status(&manifest.id).ok_or("installed but missing")?
    };
    #[cfg(feature = "runner")]
    app.mcp.forget(&manifest.id);
    announce(app);
    Ok(status)
}

/// Removes a plugin, its secrets, and its folder; every bot on this Runner loses it.
pub fn uninstall(app: &Arc<App>, id: &str) -> Result<(), String> {
    {
        let mut store = app.plugins.lock().unwrap();
        let before = store.installed.len();
        store.installed.retain(|p| p.manifest.id != id);
        if store.installed.len() == before {
            return Err("Unknown plugin".into());
        }
        store.secrets.remove(id);
        store.notes.remove(id);
        store.save(&app.config).map_err(|e| e.to_string())?;
    }
    #[cfg(feature = "runner")]
    app.mcp.forget(id);
    let dir = app.config.plugins_dir().join(id);
    if dir.is_dir() {
        let _ = std::fs::remove_dir_all(&dir);
    }
    let prefix = format!("{id}/");
    let mut auto_review = app.auto_review();
    if auto_review.rules.iter().any(|r| r.tool.as_deref().is_some_and(|t| t.starts_with(&prefix))) {
        auto_review.rules.retain(|r| !r.tool.as_deref().is_some_and(|t| t.starts_with(&prefix)));
        app.set_auto_review(auto_review);
    }
    announce(app);
    Ok(())
}

/// Sets variables on an installed plugin. Secret ones go to the secrets file; an empty value
/// clears the variable.
pub fn set_variables(app: &Arc<App>, id: &str, variables: &BTreeMap<String, String>) -> Result<PluginStatus, String> {
    let status = {
        let mut store = app.plugins.lock().unwrap();
        let manifest = store.get(id).map(|p| p.manifest.clone()).ok_or("Unknown plugin")?;
        for (name, value) in variables {
            let secret = manifest.variables.iter().find(|v| &v.name == name).map(|v| v.secret).unwrap_or(true);
            let value = value.trim();
            if secret {
                store.set_secret(id, name, (!value.is_empty()).then(|| json!(value)));
                if let Some(plugin) = store.installed.iter_mut().find(|p| p.manifest.id == id) {
                    plugin.variables.remove(name);
                }
            } else if let Some(plugin) = store.installed.iter_mut().find(|p| p.manifest.id == id) {
                if value.is_empty() {
                    plugin.variables.remove(name);
                } else {
                    plugin.variables.insert(name.clone(), value.to_string());
                }
            }
        }
        store.notes.remove(id);
        store.save(&app.config).map_err(|e| e.to_string())?;
        store.status(id).ok_or("Unknown plugin")?
    };
    #[cfg(feature = "runner")]
    app.mcp.forget(id);
    announce(app);
    Ok(status)
}

/// Keeps OAuth tokens for a server, or drops them.
pub fn set_oauth(app: &Arc<App>, id: &str, server: &str, tokens: Option<Value>) -> Result<(), String> {
    let mut store = app.plugins.lock().unwrap();
    if store.get(id).is_none() {
        return Err("Unknown plugin".into());
    }
    store.set_secret(id, &format!("oauth:{server}"), tokens);
    store.notes.remove(id);
    store.save(&app.config).map_err(|e| e.to_string())
}

/// Notes a connection state on a plugin (`connecting`, or `error` with the reason), cleared
/// by the next install, variable change, or successful connection.
pub fn note(app: &Arc<App>, id: &str, state: Option<(&str, &str)>) {
    {
        let mut store = app.plugins.lock().unwrap();
        match state {
            Some((state, detail)) => {
                store.notes.insert(id.to_string(), (state.to_string(), detail.to_string()));
            }
            None => {
                store.notes.remove(id);
            }
        }
    }
    announce(app);
}

/// The Runner's plugin list changed: the machine blob and the local app hear.
fn announce(app: &Arc<App>) {
    app.push_machine_blob_if_changed();
    app.emit(app.roster_summary());
}

/// What the apps show for one installed plugin: the manifest, which variables are set (never
/// their values), and each server's state.
pub fn detail(app: &Arc<App>, id: &str) -> Result<Value, String> {
    let store = app.plugins.lock().unwrap();
    let plugin = store.get(id).ok_or("Unknown plugin")?;
    let values = store.values(id);
    let status = store.status(id).ok_or("Unknown plugin")?;
    let servers: Vec<Value> = plugin
        .manifest
        .servers
        .iter()
        .map(|(name, spec)| {
            let (kind, auth) = match spec {
                ServerSpec::Stdio { command, .. } => ("stdio", json!({ "command": command })),
                ServerSpec::Http { url, auth, .. } => (
                    "http",
                    json!({
                        "url": url,
                        "oauth": matches!(auth, Some(AuthSpec::Oauth { .. })),
                        "signed_in": matches!(auth, Some(AuthSpec::Oauth { .. })) && !store.needs_sign_in(id, name, spec, &values),
                    }),
                ),
            };
            json!({ "name": name, "kind": kind, "auth": auth })
        })
        .collect();
    Ok(json!({
        "manifest": plugin.manifest,
        "source": plugin.source,
        "installed_at": plugin.installed_at,
        "status": status,
        "variables": plugin.manifest.variables.iter().map(|v| json!({
            "name": v.name, "description": v.description, "secret": v.secret, "required": v.required,
            "is_set": values.contains_key(&v.name),
            "value": if v.secret { Value::Null } else { json!(values.get(&v.name)) },
        })).collect::<Vec<_>>(),
        "servers": servers,
        "skills": plugin.manifest.skills.iter().map(|s| json!({ "name": s.name, "description": s.description })).collect::<Vec<_>>(),
    }))
}

/// Brings installed marketplace plugins up to the manifests the marketplace offers now, so a
/// plugin installed before an entry changed (a new sign-in method, a new server) gets the
/// change without a reinstall. Variables, secrets, and sign-ins stay; a changed server's
/// connection is dropped. `manifests` is the bundled index at startup and the full
/// marketplace when it loads.
pub fn refresh_installed(app: &Arc<App>, manifests: &[Manifest]) -> Vec<String> {
    let mut updated = Vec::new();
    {
        let mut store = app.plugins.lock().unwrap();
        for plugin in store.installed.iter_mut().filter(|p| p.source == "marketplace") {
            let Some(fresh) = manifests.iter().find(|m| m.id == plugin.manifest.id) else { continue };
            if *fresh != plugin.manifest {
                plugin.manifest = fresh.clone();
                updated.push(fresh.id.clone());
            }
        }
        if updated.is_empty() {
            return updated;
        }
        for id in &updated {
            store.notes.remove(id);
        }
        if let Err(error) = store.save(&app.config) {
            tracing::warn!(%error, "saving refreshed plugin manifests");
        }
    }
    for id in &updated {
        #[cfg(feature = "runner")]
        app.mcp.forget(id);
        let skills = app.config.plugins_dir().join(id).join("skills");
        if let Some(manifest) = manifests.iter().find(|m| &m.id == id) {
            let _ = std::fs::create_dir_all(&skills);
            for skill in &manifest.skills {
                let _ = std::fs::write(skills.join(format!("{}.md", slug(&skill.name))), skill.content.as_bytes());
            }
        }
        tracing::info!(plugin = %id, "refreshed the plugin's manifest from the marketplace");
    }
    announce(app);
    updated
}

/// The bundled manifests alone, for startup.
pub fn bundled() -> Vec<Manifest> {
    parse_index(BUNDLED_INDEX).unwrap_or_default()
}

// MARK: - Where a verb runs

/// Runs a plugin verb on `runner_id`: here when that is this Device, else as a request sealed
/// to that Runner (`crate::requests`), so a phone installs a plugin on a Mac without the relay
/// seeing the manifest or a secret.
pub async fn on_runner(app: &Arc<App>, runner_id: &str, verb: &str, body: Value) -> Result<Value, String> {
    if app.this_device_id().as_deref() == Some(runner_id) {
        #[cfg(feature = "runner")]
        {
            return serve_request(app, verb, &body);
        }
        #[cfg(not(feature = "runner"))]
        {
            let _ = (verb, body);
            return Err("This Device does not run bots, so it has no plugins.".into());
        }
    }
    crate::requests::ask(app, runner_id, verb, body).await
}

/// The plugin verbs this Runner answers, from the local app or a sealed request.
#[cfg(feature = "runner")]
pub fn serve_request(app: &Arc<App>, verb: &str, body: &Value) -> Result<Value, String> {
    let plugin_id = || body["plugin_id"].as_str().map(str::to_string).ok_or_else(|| "missing plugin_id".to_string());
    match verb {
        "plugins.install" => {
            let manifest = Manifest::parse(&body["manifest"])?;
            let source = body["source"].as_str().unwrap_or("inline");
            Ok(json!(install(app, manifest, source)?))
        }
        "plugins.uninstall" => {
            uninstall(app, &plugin_id()?)?;
            Ok(Value::Null)
        }
        "plugins.variables" => {
            let variables: BTreeMap<String, String> = serde_json::from_value(body["variables"].clone()).map_err(|e| format!("variables: {e}"))?;
            Ok(json!(set_variables(app, &plugin_id()?, &variables)?))
        }
        "plugins.connect" => {
            let id = plugin_id()?;
            let server = match body["server"].as_str() {
                Some(server) => server.to_string(),
                None => {
                    let store = app.plugins.lock().unwrap();
                    let plugin = store.get(&id).ok_or("Unknown plugin")?;
                    plugin
                        .manifest
                        .servers
                        .iter()
                        .find(|(_, spec)| matches!(spec, ServerSpec::Http { auth: Some(AuthSpec::Oauth { .. }), .. }))
                        .map(|(name, _)| name.clone())
                        .ok_or_else(|| format!("{} has nothing to sign in to.", plugin.manifest.name))?
                }
            };
            Ok(json!({ "message": mcp::connect_oauth(app, &id, &server)? }))
        }
        "plugins.detail" => detail(app, &plugin_id()?),
        "permission.answer" => {
            let message_id = body["message_id"].as_str().ok_or("missing message_id")?;
            let decision = body["decision"].as_str().and_then(mcp::Decision::parse).ok_or("decision is allow, always, or deny")?;
            if mcp::answer(app, message_id, decision) {
                return Ok(json!({ "answered": true }));
            }
            // Not a waiting tool: a sign-in card, answered by starting the flow here.
            let chat_id = body["chat_id"].as_str().ok_or("missing chat_id")?;
            Ok(json!({ "answered": mcp::answer_sign_in(app, chat_id, message_id, decision)? }))
        }
        other => Err(format!("Unknown request {other}")),
    }
}

// MARK: - Marketplace

/// The manifests on offer: the bundled index, plus the one at `marketplace_url` when set,
/// fetched at most once an hour. A fetch that fails leaves the bundled list.
pub async fn marketplace(app: &Arc<App>) -> Vec<Manifest> {
    let mut plugins: Vec<Manifest> = parse_index(BUNDLED_INDEX).unwrap_or_default();
    let url = app.settings.lock().unwrap().marketplace_url.clone().or_else(|| std::env::var("LORCA_MARKETPLACE_URL").ok()).filter(|u| !u.trim().is_empty());
    if let Some(url) = url {
        let cached = app.marketplace_cache.lock().unwrap().clone();
        let extra = match cached {
            Some((at, list)) if now_secs() - at < INDEX_TTL_SECS => list,
            _ => match fetch_index(app, &url).await {
                Ok(list) => {
                    *app.marketplace_cache.lock().unwrap() = Some((now_secs(), list.clone()));
                    list
                }
                Err(error) => {
                    tracing::warn!(%error, url, "fetching the marketplace index");
                    Vec::new()
                }
            },
        };
        for manifest in extra {
            match plugins.iter_mut().find(|p| p.id == manifest.id) {
                Some(existing) => *existing = manifest,
                None => plugins.push(manifest),
            }
        }
    }
    refresh_installed(app, &plugins);
    plugins
}

async fn fetch_index(app: &Arc<App>, url: &str) -> Result<Vec<Manifest>, String> {
    let text = app.http.get(url).send().await.map_err(|e| e.to_string())?.error_for_status().map_err(|e| e.to_string())?.text().await.map_err(|e| e.to_string())?;
    parse_index(&text)
}

fn parse_index(text: &str) -> Result<Vec<Manifest>, String> {
    let value: Value = serde_json::from_str(text).map_err(|e| e.to_string())?;
    let entries = value.get("plugins").and_then(Value::as_array).ok_or("The index has no plugins list")?;
    let mut plugins = Vec::new();
    for entry in entries {
        match Manifest::parse(entry) {
            Ok(manifest) => plugins.push(manifest),
            Err(error) => tracing::warn!(%error, "skipping a marketplace entry"),
        }
    }
    Ok(plugins)
}

/// Marketplace entries matching `query` (every word must appear in the id, name,
/// description, or tags), all of them for an empty query.
pub fn search<'a>(plugins: &'a [Manifest], query: &str) -> Vec<&'a Manifest> {
    let words: Vec<String> = query.split_whitespace().map(str::to_lowercase).collect();
    plugins
        .iter()
        .filter(|p| {
            let haystack = format!("{} {} {} {}", p.id, p.name, p.description, p.tags.join(" ")).to_lowercase();
            words.iter().all(|w| haystack.contains(w))
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_bundled_index_parses() {
        let plugins = parse_index(BUNDLED_INDEX).unwrap();
        assert!(plugins.iter().any(|p| p.id == "github"), "{:?}", plugins.iter().map(|p| &p.id).collect::<Vec<_>>());
        for plugin in &plugins {
            assert!(!plugin.icon.is_empty() && !plugin.description.is_empty(), "{} is incomplete", plugin.id);
        }
        assert_eq!(search(&plugins, "GIT hub").len(), 1);
        assert_eq!(search(&plugins, "").len(), plugins.len());
        assert!(search(&plugins, "nothing-like-this").is_empty());
    }

    #[test]
    fn manifests_are_checked_and_templates_filled() {
        assert!(Manifest::parse(&json!({ "id": "Bad Id", "name": "x", "servers": {} })).unwrap_err().contains("lowercase"));
        assert!(Manifest::parse(&json!({ "id": "x", "name": "x", "servers": {} })).unwrap_err().contains("no MCP server"));
        let inline = Manifest::from_mcp_json("My Server", &json!({ "mcpServers": { "fs": { "command": "npx", "args": ["-y", "server"], "env": { "TOKEN": "${TOKEN}" } } } })).unwrap();
        assert_eq!(inline.id, "my-server");
        assert!(matches!(inline.servers.get("fs"), Some(ServerSpec::Stdio { command, .. }) if command == "npx"));
        let one = Manifest::from_mcp_json("Remote", &json!({ "url": "https://example.com/mcp", "headers": { "X-Key": "${KEY}" } })).unwrap();
        assert!(matches!(one.servers.get("remote"), Some(ServerSpec::Http { .. })));
        assert!(Manifest::from_mcp_json("Nope", &json!({ "nothing": true })).is_err());

        let mut values = BTreeMap::new();
        values.insert("TOKEN".to_string(), "abc".to_string());
        assert_eq!(fill("Bearer ${TOKEN}", &values), "Bearer abc");
        assert_eq!(fill("${MISSING}/x${TOKEN}", &values), "${MISSING}/xabc");
        assert_eq!(fill("no vars", &values), "no vars");
        assert_eq!(slug("  GitHub  Server! "), "github-server");
        assert!(pattern_matches("search_*", "search_issues") && !pattern_matches("search_*", "create_issue") && pattern_matches("get_me", "get_me"));
    }

    struct ScratchApp(Arc<App>, std::path::PathBuf);
    impl Drop for ScratchApp {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.1);
        }
    }

    fn scratch_app() -> ScratchApp {
        let home = std::env::temp_dir().join(format!("lorca-plugins-{}", uuid::Uuid::new_v4()));
        let app = App::load(crate::config::Config { home: home.clone(), port: 0 }).unwrap();
        ScratchApp(app, home)
    }

    #[test]
    fn install_variables_and_status_round_trip() {
        let scratch = scratch_app();
        let app = &scratch.0;
        let manifest = Manifest::parse(&json!({
            "id": "acme", "name": "Acme", "description": "Test", "icon": "a",
            "servers": { "api": { "type": "http", "url": "https://acme.test/mcp", "auth": { "type": "oauth", "token_variable": "ACME_TOKEN" } } },
            "variables": [ { "name": "ACME_TOKEN", "secret": true }, { "name": "REGION", "required": true } ],
            "skills": [ { "name": "Ship it", "content": "# Ship" } ]
        }))
        .unwrap();
        let status = install(app, manifest, "marketplace").unwrap();
        assert_eq!((status.state.as_str(), status.detail.as_str()), ("needs_setup", "Needs REGION"));
        assert!(app.config.plugins_dir().join("acme/skills/ship-it.md").is_file());
        let mut vars = BTreeMap::new();
        vars.insert("REGION".to_string(), "eu".to_string());
        let status = set_variables(app, "acme", &vars).unwrap();
        assert_eq!(status.state, "needs_auth");
        vars.clear();
        vars.insert("ACME_TOKEN".to_string(), "secret-value".to_string());
        let status = set_variables(app, "acme", &vars).unwrap();
        assert_eq!(status.state, "ready");
        let detail = detail(app, "acme").unwrap();
        assert_eq!(detail["variables"][0]["is_set"], json!(true));
        assert_eq!(detail["variables"][0]["value"], Value::Null, "secrets are never read back");
        assert_eq!(detail["variables"][1]["value"], json!("eu"));
        let installed = std::fs::read_to_string(app.config.plugins_dir().join("installed.json")).unwrap();
        assert!(!installed.contains("secret-value"), "secrets stay out of installed.json");
        assert!(app.plugins.lock().unwrap().statuses().iter().any(|p| p.id == "acme" && p.state == "ready"));
        uninstall(app, "acme").unwrap();
        assert!(app.plugins.lock().unwrap().installed().is_empty());
        assert!(!app.config.plugins_dir().join("acme").exists());
        assert!(uninstall(app, "acme").is_err());
    }

    #[test]
    fn installed_marketplace_plugins_follow_the_index() {
        let scratch = scratch_app();
        let app = &scratch.0;
        let mut old = bundled().into_iter().find(|m| m.id == "github").unwrap();
        // As installed before the device flow existed: a bare OAuth entry.
        old.servers.insert("github".into(), ServerSpec::Http { url: "https://api.githubcopilot.com/mcp/".into(), headers: BTreeMap::new(), auth: Some(AuthSpec::Oauth { scopes: vec![], token_variable: Some("GITHUB_TOKEN".into()), client_id_variable: None, client_secret_variable: None, client_id: None, client_secret: None, device_authorization_endpoint: None, token_endpoint: None }) });
        install(app, old, "marketplace").unwrap();
        let mut vars = BTreeMap::new();
        vars.insert("GITHUB_TOKEN".to_string(), "ghp-secret".to_string());
        set_variables(app, "github", &vars).unwrap();
        assert_eq!(refresh_installed(app, &bundled()), vec!["github".to_string()]);
        assert!(refresh_installed(app, &bundled()).is_empty(), "already current");
        let store = app.plugins.lock().unwrap();
        let ServerSpec::Http { auth: Some(AuthSpec::Oauth { client_id, device_authorization_endpoint, .. }), .. } = store.get("github").unwrap().manifest.servers.get("github").unwrap() else { panic!("http oauth") };
        assert!(client_id.as_deref().is_some_and(|c| !c.is_empty()) && device_authorization_endpoint.is_some(), "the device flow arrived");
        assert_eq!(store.values("github").get("GITHUB_TOKEN").map(String::as_str), Some("ghp-secret"), "secrets kept");
        // A plugin the user added by hand is left alone.
        drop(store);
        let mine = Manifest::from_mcp_json("Mine", &json!({ "url": "https://example.com/mcp" })).unwrap();
        install(app, mine, "inline").unwrap();
        assert!(refresh_installed(app, &bundled()).is_empty());
    }
}
