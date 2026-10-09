//! Plugins: MCP servers a bot can use, after Grok Bot's. A plugin is a manifest (`plugin.json`:
//! servers, variables, skills, tool hints) that a Runner installs; the Runner keeps the
//! package, the variables, the secrets, and the OAuth tokens, and advertises what it has in
//! its machine blob. Every bot on the Runner may use every plugin installed there. The
//! manifests on offer are the marketplace's (`crate::marketplace`); the servers the user adds
//! themselves live in the Runner's `mcp.json` (`mcp_json`), each a plugin of its own. The MCP
//! side, with the permission gate, is `mcp` under the `runner` feature.

#[cfg(feature = "runner")]
pub mod mcp;
pub mod mcp_json;
#[cfg(feature = "runner")]
pub mod review;
pub mod sign_in;

use std::collections::BTreeMap;
use std::sync::Arc;

use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

use crate::app::App;
use crate::config::{self, now_secs};
use crate::model::PluginStatus;

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
    /// Who makes the server, for the marketplace's Developer line.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub author: String,
    /// The marketplace section it is listed under: `code`, `productivity`, `research`, ….
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub category: String,
    /// Listed under Featured Plugins, in index order.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub featured: bool,
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

// A manifest's own settings, a few per plugin: their size never matters.
#[allow(clippy::large_enum_variant)]
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
        /// Where it starts; the plugin's folder when unset. A leading `~` is the home folder.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        cwd: Option<String>,
        /// Seconds a call may go without an answer or progress (`ServerSpec::call_timeout`).
        #[serde(default, skip_serializing_if = "Option::is_none")]
        timeout: Option<u64>,
    },
    /// A streamable-HTTP server. `${VAR}` in `headers` is filled the same way.
    Http {
        url: String,
        #[serde(default)]
        headers: BTreeMap<String, String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        auth: Option<AuthSpec>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        timeout: Option<u64>,
    },
}

/// How long a plugin tool's call may go without an answer or progress, unless its server says.
pub const CALL_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(10 * 60);

impl ServerSpec {
    /// How long a call to the server may go without an answer or progress: its own `timeout`, else
    /// ten minutes.
    pub fn call_timeout(&self) -> std::time::Duration {
        let own = match self {
            ServerSpec::Stdio { timeout, .. } | ServerSpec::Http { timeout, .. } => *timeout,
        };
        own.filter(|seconds| *seconds > 0).map(std::time::Duration::from_secs).unwrap_or(CALL_TIMEOUT)
    }
}

#[allow(clippy::large_enum_variant)]
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
        /// Sign in only once the server asks for it, with a 401 and its challenge: an `mcp.json`
        /// server says nothing about how it signs in, and many need no sign-in at all.
        #[serde(default, skip_serializing_if = "std::ops::Not::not")]
        optional: bool,
        /// The name registered with a server that registers clients on the fly; Lorca when unset.
        /// Some servers take only names they know.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        client_name: Option<String>,
        /// A redirect a preregistered client must use: a fixed port on 127.0.0.1 (`/callback`),
        /// or a whole loopback URL. Such a sign-in opens in the Runner's own browser.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        callback_port: Option<u16>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        callback_url: Option<String>,
        /// The authorization server's metadata document, taken as it is in place of discovery, for
        /// a server that names the wrong authorization server or none.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        auth_server_metadata_url: Option<String>,
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
    /// Tools that stage drafts without sending or publishing them. Still cross Auto-review.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub draft: Vec<String>,
    /// Never offered to the bot.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub hide: Vec<String>,
    /// Tools shown or hidden by name or pattern, in order, as pi reads an `mcp.json` entry's
    /// `toolExposure`: an exact name decides first, then the first pattern that matches.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub exposure: Vec<ToolRule>,
}

/// One `toolExposure` entry: a tool's name or a pattern ending in `*`, and whether it is hidden.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ToolRule {
    pub pattern: String,
    pub hidden: bool,
}

impl ToolHints {
    fn is_empty(&self) -> bool {
        self.readonly.is_empty() && self.draft.is_empty() && self.hide.is_empty() && self.exposure.is_empty()
    }

    /// Whether `tool` is kept from bots: by its `exposure` rule (its exact name first, then the
    /// first pattern that matches), else by a `hide` pattern.
    pub fn hides(&self, tool: &str) -> bool {
        let rule = self.exposure.iter().find(|rule| rule.pattern == tool).or_else(|| self.exposure.iter().find(|rule| pattern_matches(&rule.pattern, tool)));
        match rule {
            Some(rule) => rule.hidden,
            None => self.hide.iter().any(|pattern| pattern_matches(pattern, tool)),
        }
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
        if !is_id(&self.id) {
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
}

/// Lowercase letters, digits, and dashes: a plugin or bot template id.
pub fn is_id(id: &str) -> bool {
    !id.is_empty() && id.chars().all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-')
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

/// `https://mcp.example.com` for any URL there, or the URL as written when it is not one yet
/// (`${DOCS_URL}`).
fn origin_of(url: &str) -> String {
    match reqwest::Url::parse(url) {
        Ok(parsed) if parsed.host_str().is_some() => parsed.origin().ascii_serialization(),
        _ => url.to_string(),
    }
}

/// A placeholder's variable and its default: `${NAME}`, or `${NAME:-default}` as a shell writes
/// it, which `mcp.json` files use.
fn placeholder(inner: &str) -> (&str, Option<&str>) {
    match inner.split_once(":-") {
        Some((name, default)) => (name, Some(default)),
        None => (inner, None),
    }
}

/// Fills a header or an environment value, or `None` when it names a variable with no value and
/// no default, so an optional key left unset is not sent at all. Sent as its `${VAR}` placeholder
/// it reads as a key: Context7 answers every tool call with "Invalid API key".
pub fn fill_if_set(template: &str, values: &BTreeMap<String, String>) -> Option<String> {
    let mut rest = template;
    while let Some(start) = rest.find("${") {
        let Some(end) = rest[start + 2..].find('}') else { break };
        let (name, default) = placeholder(&rest[start + 2..start + 2 + end]);
        if default.is_none() && values.get(name).is_none_or(|value| value.trim().is_empty()) {
            return None;
        }
        rest = &rest[start + 3 + end..];
    }
    Some(fill(template, values))
}

/// Fills `${VAR}` from the plugin's variables and secrets, and `${VAR:-default}` with the default
/// when the variable has no value; an unknown name with no default stays as it is.
pub fn fill(template: &str, values: &BTreeMap<String, String>) -> String {
    let mut out = String::new();
    let mut rest = template;
    while let Some(start) = rest.find("${") {
        out.push_str(&rest[..start]);
        match rest[start + 2..].find('}') {
            Some(end) => {
                let (name, default) = placeholder(&rest[start + 2..start + 2 + end]);
                match (values.get(name).filter(|value| default.is_none() || !value.is_empty()), default) {
                    (Some(value), _) => out.push_str(value),
                    (None, Some(default)) => out.push_str(default),
                    (None, None) => out.push_str(&rest[start..start + 3 + end]),
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
    /// The code a device-flow sign-in waits for, by plugin id, from the code's arrival until
    /// the flow ends. The plugin's detail carries it, so the app that started the sign-in
    /// without a card can show it.
    pub codes: BTreeMap<String, SignInCode>,
    /// The Runner's `mcp.json` as last read, every server in it: the ones that run are plugins
    /// in `installed` too.
    pub mcp: mcp_json::McpFile,
}

/// A device-flow code waiting to be entered: which server it signs in, and where.
#[derive(Debug, Clone, PartialEq)]
pub struct SignInCode {
    pub server: String,
    pub code: String,
    pub link: String,
}

impl Store {
    pub fn load(config: &config::Config) -> Store {
        let dir = config.plugins_dir();
        let installed: InstalledFile = config::read_json(&dir.join("installed.json")).unwrap_or_default();
        let secrets: SecretsFile = config::read_json(&dir.join("secrets.json")).unwrap_or_default();
        let mut store = Store { installed: installed.plugins, secrets, notes: BTreeMap::new(), codes: BTreeMap::new(), mcp: mcp_json::McpFile::default() };
        store.installed.retain(|plugin| plugin.source != mcp_json::SOURCE);
        store.take_mcp(mcp_json::McpFile::read(&config.mcp_path()));
        store
    }

    fn save(&self, config: &config::Config) -> anyhow::Result<()> {
        let dir = config.plugins_dir();
        std::fs::create_dir_all(&dir)?;
        config::set_private(&dir)?;
        // The servers from mcp.json are that file's.
        let plugins = self.installed.iter().filter(|plugin| plugin.source != mcp_json::SOURCE).cloned().collect();
        config::write_json_private(&dir.join("installed.json"), &InstalledFile { plugins })?;
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

    /// A remote server's saved sign-in (`oauth`) or the challenge it answered with (`challenge`),
    /// while the server is where it was then: tokens a server issued never go to another host,
    /// and a server moved to another host starts out signed out. One saved before sign-ins
    /// named their host counts as here.
    pub fn sign_in_secret(&self, id: &str, kind: &str, server: &str) -> Option<Value> {
        let saved = self.secret(id, &format!("{kind}:{server}"))?;
        match saved.get("origin").and_then(Value::as_str) {
            Some(origin) if Some(origin) != self.server_origin(id, server).as_deref() => None,
            _ => Some(saved),
        }
    }

    /// Where a remote server lives, as its sign-ins name it.
    fn server_origin(&self, id: &str, server: &str) -> Option<String> {
        match self.get(id)?.manifest.servers.get(server)? {
            ServerSpec::Http { url, .. } => Some(origin_of(url)),
            ServerSpec::Stdio { .. } => None,
        }
    }

    /// `value` with the origin of the server it belongs to.
    fn stamped(&self, id: &str, server: &str, mut value: Value) -> Value {
        if let (Some(origin), Some(fields)) = (self.server_origin(id, server), value.as_object_mut()) {
            fields.insert("origin".into(), json!(origin));
        }
        value
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
            source: (plugin.source == mcp_json::SOURCE).then(|| plugin.source.clone()),
        }
    }

    /// An OAuth server with no tokens and no pasted token. One that signs in only when asked
    /// needs it once the server has answered with its challenge.
    pub fn needs_sign_in(&self, id: &str, server: &str, spec: &ServerSpec, values: &BTreeMap<String, String>) -> bool {
        match spec {
            ServerSpec::Http { auth: Some(AuthSpec::Oauth { token_variable, optional, .. }), .. } => {
                let pasted = token_variable.as_ref().map(|v| values.contains_key(v)).unwrap_or(false);
                let asked = !*optional || self.sign_in_secret(id, "challenge", server).is_some();
                !pasted && asked && self.sign_in_secret(id, "oauth", server).is_none()
            }
            _ => false,
        }
    }

    /// Keeps the challenge a server that signs in only when asked answered with, so the plugin
    /// reads Sign in from then on, across restarts, until it is signed in.
    pub fn note_challenge(&mut self, config: &config::Config, id: &str, server: &str, challenge: &str) {
        if self.sign_in_secret(id, "challenge", server).is_some() {
            return;
        }
        // An object, so `values` never takes it for a variable.
        let challenge = self.stamped(id, server, json!({ "header": challenge }));
        self.set_secret(id, &format!("challenge:{server}"), Some(challenge));
        if let Err(error) = self.save(config) {
            tracing::warn!(%error, "saving a server's sign-in challenge");
        }
    }

    /// Forgets the challenge of a server that has since let Lorca in without a sign-in, so it no
    /// longer reads Sign in.
    pub fn forget_challenge(&mut self, config: &config::Config, id: &str, server: &str) {
        if self.secret(id, &format!("challenge:{server}")).is_none() {
            return;
        }
        self.set_secret(id, &format!("challenge:{server}"), None);
        if let Err(error) = self.save(config) {
            tracing::warn!(%error, "forgetting a server's sign-in challenge");
        }
    }
}

// MARK: - Installing

/// Installs or updates a plugin on this Runner and writes its skills to its folder.
pub fn install(app: &Arc<App>, manifest: Manifest, source: &str) -> Result<PluginStatus, String> {
    manifest.check()?;
    if let Some(server) = app.plugins.lock().unwrap().get(&manifest.id).filter(|plugin| plugin.source == mcp_json::SOURCE) {
        return Err(format!("The MCP server {} in mcp.json has the id {}. Rename it there first.", server.manifest.name, manifest.id));
    }
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
    {
        app.mcp.forget(&manifest.id);
        mcp::prefetch_tools(app, &manifest.id);
    }
    announce(app);
    Ok(status)
}

/// Removes a plugin, its secrets, and its folder; every bot on this Runner loses it. A server
/// from `mcp.json` leaves that file.
pub fn uninstall(app: &Arc<App>, id: &str) -> Result<(), String> {
    #[cfg(feature = "runner")]
    {
        let from_file = app.plugins.lock().unwrap().get(id).filter(|plugin| plugin.source == mcp_json::SOURCE).map(|plugin| plugin.manifest.name.clone());
        if let Some(name) = from_file {
            return mcp_json::remove(app, &name);
        }
    }
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
    {
        app.mcp.forget(id);
        mcp::prefetch_tools(app, id);
    }
    announce(app);
    Ok(status)
}

/// Forgets the sign-in of a plugin's OAuth servers, or of the one named: the saved tokens and a
/// device code waiting for one. A server that is off is no plugin now, but its sign-in is
/// forgotten all the same. Nothing is revoked at the server, which honors a token until it
/// expires; the next use asks for a sign-in again.
pub fn sign_out(app: &Arc<App>, id: &str, server: Option<&str>) -> Result<Option<PluginStatus>, String> {
    let status = {
        let mut store = app.plugins.lock().unwrap();
        let servers: Vec<String> = match store.get(id) {
            Some(plugin) => plugin
                .manifest
                .servers
                .iter()
                .filter(|(name, spec)| server.is_none_or(|server| server == name.as_str()) && matches!(spec, ServerSpec::Http { auth: Some(AuthSpec::Oauth { .. }), .. }))
                .map(|(name, _)| name.clone())
                .collect(),
            None => server.map(|server| vec![server.to_string()]).unwrap_or_default(),
        };
        for name in &servers {
            store.set_secret(id, &format!("oauth:{name}"), None);
        }
        store.codes.remove(id);
        store.notes.remove(id);
        store.save(&app.config).map_err(|e| e.to_string())?;
        store.status(id)
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
    let tokens = tokens.map(|tokens| store.stamped(id, server, tokens));
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
pub(crate) fn announce(app: &Arc<App>) {
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
                ServerSpec::Http { url, auth, .. } => {
                    let waiting = store.codes.get(id).filter(|code| &code.server == name);
                    // A server that signs in only when asked shows its sign-in once it has asked.
                    let tokens = store.sign_in_secret(id, "oauth", name).is_some();
                    let oauth = match auth {
                        Some(AuthSpec::Oauth { optional: true, .. }) => tokens || store.sign_in_secret(id, "challenge", name).is_some(),
                        Some(AuthSpec::Oauth { .. }) => true,
                        _ => false,
                    };
                    (
                        "http",
                        json!({
                            "url": url,
                            "oauth": oauth,
                            "signed_in": oauth && !store.needs_sign_in(id, name, spec, &values),
                            "code": waiting.map(|code| &code.code),
                            "link": waiting.map(|code| &code.link),
                        }),
                    )
                }
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

// MARK: - Where a verb runs

/// Runs a plugin verb on `runner_id`: here when that is this Device, else as a request sealed
/// to that Runner (`crate::requests`), so a phone installs a plugin on another computer without the relay
/// seeing the manifest or a secret.
pub async fn on_runner(app: &Arc<App>, runner_id: &str, verb: &str, body: Value) -> Result<Value, String> {
    if app.this_device_id().as_deref() == Some(runner_id) {
        #[cfg(feature = "runner")]
        {
            return serve_request(app, verb, &body, None).await;
        }
        #[cfg(not(feature = "runner"))]
        {
            let _ = (verb, body);
            return Err("This Device does not run bots, so it has no plugins.".into());
        }
    }
    crate::requests::ask(app, runner_id, verb, body).await
}

/// The plugin verbs this Runner answers, from the local app or a sealed request from the
/// Device `requested_by`.
#[cfg(feature = "runner")]
pub async fn serve_request(app: &Arc<App>, verb: &str, body: &Value, requested_by: Option<&str>) -> Result<Value, String> {
    let plugin_id = || body["plugin_id"].as_str().map(str::to_string).ok_or_else(|| "missing plugin_id".to_string());
    // A Device that listens on its own loopback for the browser's redirect opens the sign-in
    // page itself (`sign_in::from_here`).
    let elsewhere = || {
        let redirect_uri = body["redirect_uri"].as_str().filter(|uri| sign_in::is_loopback_redirect(uri))?.to_string();
        let device = requested_by.and_then(|id| app.device(id)).map(|d| d.name).unwrap_or_else(|| "the Device that asked".into());
        Some(mcp::Elsewhere { device, redirect_uri })
    };
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
            let status = set_variables(app, &plugin_id()?, &variables)?;
            // A pasted token stands in for the sign-in the cards ask for.
            if status.state == "ready" {
                mcp::settle_sign_in_cards(app, &status.id, &status.name);
            }
            Ok(json!(status))
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
            let started = mcp::connect_oauth(app, &id, &server, elsewhere()).await?;
            Ok(json!({ "message": started.message, "url": started.url, "sign_in": started.id }))
        }
        "plugins.sign_in.finish" => {
            let id = body["sign_in"].as_str().ok_or("missing sign_in")?;
            mcp::finish_sign_in(app, &plugin_id()?, id, body["url"].as_str().ok_or("missing url")?).await
        }
        "plugins.sign_in.cancel" => mcp::cancel_sign_in(app, &plugin_id()?, body["sign_in"].as_str().ok_or("missing sign_in")?),
        "plugins.detail" => detail(app, &plugin_id()?),
        // What the Access sheet offers: each installed plugin by name, with the tools it offered
        // when it last connected and what each does, read, draft, or write.
        "permissions.catalog" => {
            let installed = app.plugins.lock().unwrap().installed().to_vec();
            let statuses = app.plugins.lock().unwrap().statuses();
            Ok(json!(installed.iter().map(|plugin| json!({
                "id": plugin.manifest.id,
                "name": statuses.iter().find(|status| status.id == plugin.manifest.id).map(|status| &status.name).unwrap_or(&plugin.manifest.name),
                "tools": mcp::saved_tools(app, plugin).into_iter().filter(|tool| tool["hidden"] != true).collect::<Vec<_>>(),
            })).collect::<Vec<_>>()))
        }
        "plugins.sign_out" => Ok(json!(sign_out(app, &plugin_id()?, body["server"].as_str())?)),
        "permission.answer" => {
            let message_id = body["message_id"].as_str().ok_or("missing message_id")?;
            let decision = body["decision"].as_str().and_then(mcp::Decision::parse).ok_or("decision is allow, always, or deny")?;
            let chat_id = body["chat_id"].as_str().ok_or("missing chat_id")?;
            // An access request is only ever dismissed: access changes in the bot's Access sheet.
            if let Some(mut message) = app.message(chat_id, message_id) {
                if let crate::model::Body::Permission { tool, decision: current, .. } = &mut message.body {
                    if tool == "access" {
                        if decision != mcp::Decision::Denied {
                            return Err("Change this bot's Access settings in its profile. An access request cannot grant permissions or add an Always allow rule.".into());
                        }
                        *current = "dismissed".into();
                        app.upsert_message(message, true);
                        return Ok(json!({ "answered": true }));
                    }
                }
            }
            if mcp::answer(app, message_id, decision) {
                return Ok(json!({ "answered": true }));
            }
            // Not a waiting tool: a sign-in card, answered by starting the flow.
            mcp::answer_sign_in(app, chat_id, message_id, decision, elsewhere()).await
        }
        other => Err(format!("Unknown request {other}")),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn manifests_are_checked_and_templates_filled() {
        assert!(Manifest::parse(&json!({ "id": "Bad Id", "name": "x", "servers": {} })).unwrap_err().contains("lowercase"));
        assert!(Manifest::parse(&json!({ "id": "x", "name": "x", "servers": {} })).unwrap_err().contains("no MCP server"));

        let mut values = BTreeMap::new();
        values.insert("TOKEN".to_string(), "abc".to_string());
        assert_eq!(fill("Bearer ${TOKEN}", &values), "Bearer abc");
        assert_eq!(fill("${MISSING}/x${TOKEN}", &values), "${MISSING}/xabc");
        assert_eq!(fill("no vars", &values), "no vars");
        assert_eq!(fill_if_set("Bearer ${TOKEN}", &values).as_deref(), Some("Bearer abc"));
        assert_eq!(fill_if_set("${MISSING}", &values), None, "an unset key is left out, not sent as its placeholder");
        assert_eq!(fill_if_set("${TOKEN}-${MISSING}", &values), None);
        assert_eq!(fill_if_set("static", &values).as_deref(), Some("static"));
        values.insert("BLANK".to_string(), " ".to_string());
        assert_eq!(fill_if_set("${BLANK}", &values), None, "a blank value is no value");
        // A default, as mcp.json files write it.
        assert_eq!(fill("${MISSING:-8080}/${TOKEN:-x}", &values), "8080/abc");
        values.insert("EMPTY".to_string(), String::new());
        assert_eq!(fill("${EMPTY:-fallback}", &values), "fallback", "an empty value takes the default, as a shell's :- does");
        assert_eq!(fill("${EMPTY}", &values), "");
        assert_eq!(fill_if_set("Bearer ${MISSING:-anonymous}", &values).as_deref(), Some("Bearer anonymous"));
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
        let mut old = crate::marketplace::bundled().plugins.into_iter().find(|m| m.id == "github").unwrap();
        // As installed before the device flow existed: a bare OAuth entry.
        old.servers.insert("github".into(), ServerSpec::Http { url: "https://api.githubcopilot.com/mcp/".into(), headers: BTreeMap::new(), auth: Some(AuthSpec::Oauth { scopes: vec![], token_variable: Some("GITHUB_TOKEN".into()), client_id_variable: None, client_secret_variable: None, client_id: None, client_secret: None, device_authorization_endpoint: None, token_endpoint: None, optional: false, client_name: None, callback_port: None, callback_url: None, auth_server_metadata_url: None }), timeout: None });
        install(app, old, "marketplace").unwrap();
        let mut vars = BTreeMap::new();
        vars.insert("GITHUB_TOKEN".to_string(), "ghp-secret".to_string());
        set_variables(app, "github", &vars).unwrap();
        assert_eq!(refresh_installed(app, &crate::marketplace::bundled().plugins), vec!["github".to_string()]);
        assert!(refresh_installed(app, &crate::marketplace::bundled().plugins).is_empty(), "already current");
        let store = app.plugins.lock().unwrap();
        let ServerSpec::Http { auth: Some(AuthSpec::Oauth { client_id, device_authorization_endpoint, .. }), .. } = store.get("github").unwrap().manifest.servers.get("github").unwrap() else { panic!("http oauth") };
        assert!(client_id.as_deref().is_some_and(|c| !c.is_empty()) && device_authorization_endpoint.is_some(), "the device flow arrived");
        assert_eq!(store.values("github").get("GITHUB_TOKEN").map(String::as_str), Some("ghp-secret"), "secrets kept");
        // A plugin installed from its own manifest is left alone.
        drop(store);
        let mine = Manifest::parse(&json!({ "id": "mine", "name": "Mine", "servers": { "api": { "type": "http", "url": "https://example.com/mcp" } } })).unwrap();
        install(app, mine, "inline").unwrap();
        assert!(refresh_installed(app, &crate::marketplace::bundled().plugins).is_empty());
    }

    #[test]
    fn a_sign_in_stays_with_the_host_it_was_made_for() {
        let scratch = scratch_app();
        let app = &scratch.0;
        let manifest = Manifest::parse(&json!({ "id": "docs", "name": "Docs", "servers": { "api": { "type": "http", "url": "https://docs.test/mcp", "auth": { "type": "oauth" } } } })).unwrap();
        install(app, manifest, "inline").unwrap();
        let move_to = |url: &str| {
            let mut store = app.plugins.lock().unwrap();
            let plugin = store.installed.iter_mut().find(|p| p.manifest.id == "docs").unwrap();
            if let Some(ServerSpec::Http { url: current, .. }) = plugin.manifest.servers.get_mut("api") {
                *current = url.to_string();
            }
        };
        let state = || app.plugins.lock().unwrap().status("docs").unwrap().state;
        assert_eq!(state(), "needs_auth");
        set_oauth(app, "docs", "api", Some(json!({ "client_id": "c", "tokens": { "access_token": "a" } }))).unwrap();
        assert_eq!(app.plugins.lock().unwrap().sign_in_secret("docs", "oauth", "api").unwrap()["origin"], json!("https://docs.test"));
        assert_eq!(state(), "ready");
        move_to("https://DOCS.test:443/v2/mcp");
        assert_eq!(state(), "ready", "another path on the same host keeps the sign-in");
        move_to("https://elsewhere.test/mcp");
        assert!(app.plugins.lock().unwrap().sign_in_secret("docs", "oauth", "api").is_none(), "tokens never reach another host");
        assert_eq!(state(), "needs_auth");
        move_to("https://docs.test/mcp");
        assert_eq!(state(), "ready", "back where it was made, the sign-in counts again");
        // A sign-in saved before sign-ins named their host still counts.
        app.plugins.lock().unwrap().set_secret("docs", "oauth:api", Some(json!({ "client_id": "c", "tokens": { "access_token": "a" } })));
        move_to("https://elsewhere.test/mcp");
        assert_eq!(state(), "ready");
        // So does a challenge: one a server answered with is that server's.
        let mut store = app.plugins.lock().unwrap();
        store.note_challenge(&app.config, "docs", "chat", "Bearer");
        assert!(store.secret("docs", "challenge:chat").is_some());
        assert_eq!(origin_of("${DOCS_URL}"), "${DOCS_URL}", "a URL named by a variable is compared as written");
    }

    #[test]
    fn a_waiting_device_code_shows_on_its_server() {
        let scratch = scratch_app();
        let app = &scratch.0;
        let oauth = json!({ "type": "oauth", "client_id": "cid", "device_authorization_endpoint": "https://hub.test/device", "token_endpoint": "https://hub.test/token" });
        let manifest = Manifest::parse(&json!({
            "id": "hub", "name": "Hub",
            "servers": { "api": { "type": "http", "url": "https://hub.test/mcp", "auth": oauth }, "files": { "type": "http", "url": "https://hub.test/files", "auth": oauth } }
        }))
        .unwrap();
        install(app, manifest, "inline").unwrap();
        let server = |detail: &Value, name: &str| detail["servers"].as_array().unwrap().iter().find(|s| s["name"] == name).unwrap()["auth"].clone();
        let before = detail(app, "hub").unwrap();
        assert_eq!((server(&before, "files")["code"].clone(), server(&before, "files")["link"].clone()), (Value::Null, Value::Null));
        app.plugins.lock().unwrap().codes.insert("hub".into(), SignInCode { server: "files".into(), code: "WDJB-MJHT".into(), link: "https://hub.test/login/device".into() });
        let waiting = detail(app, "hub").unwrap();
        assert_eq!(server(&waiting, "files")["code"], json!("WDJB-MJHT"));
        assert_eq!(server(&waiting, "files")["link"], json!("https://hub.test/login/device"));
        assert_eq!(server(&waiting, "api")["code"], Value::Null, "only the server signing in shows it");
        let status = serde_json::to_string(&app.plugins.lock().unwrap().statuses()).unwrap();
        assert!(!status.contains("WDJB-MJHT"), "the machine blob's statuses never carry the code");
    }
}
