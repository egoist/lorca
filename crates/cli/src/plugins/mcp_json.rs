//! The Runner's `mcp.json`: MCP servers the user adds by hand, with `lorca mcp`, or from the apps,
//! in the file format Claude Desktop, Cursor, and Claude Code share
//! (`{ "mcpServers": { "name": { … } } }`). Each server that is on becomes a plugin of the Runner
//! (`source` `mcp.json`, one server named [`SERVER`]), so bots call its tools from codemode scripts,
//! Auto-review checks the ones that change things, and a remote server signs in with OAuth once it
//! asks for it. The file is the truth: the Runner reads it again when it changes on disk, and
//! writes it back with the user's order and the fields Lorca does not know kept.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
#[cfg(feature = "runner")]
use std::sync::Arc;

use serde::de::{self, Deserializer, MapAccess, SeqAccess, Visitor};
use serde::ser::{SerializeMap, SerializeSeq, Serializer};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

#[cfg(feature = "runner")]
use crate::app::App;

use super::{AuthSpec, Installed, Manifest, ServerSpec, Store, ToolHints};

/// `Installed::source` of a plugin made from an `mcp.json` entry.
pub const SOURCE: &str = "mcp.json";
/// The one server of such a plugin, whatever its entry is called, so a rename keeps its sign-in.
pub const SERVER: &str = "mcp";
/// How long the apps wait for a server to connect again: its start and handshake, and the relay
/// both ways when it runs on another Runner.
pub const RECONNECT_WAIT: std::time::Duration = std::time::Duration::from_secs(150);

// MARK: - Ordered JSON

/// A JSON value whose objects keep their keys in the order they came in, so a file the user
/// arranged keeps its arrangement when Lorca writes it back.
#[derive(Debug, Clone, PartialEq, Default)]
pub enum Json {
    #[default]
    Null,
    Bool(bool),
    Number(serde_json::Number),
    String(String),
    Array(Vec<Json>),
    Object(Vec<(String, Json)>),
}

impl Json {
    pub fn object() -> Json {
        Json::Object(Vec::new())
    }

    pub fn get(&self, key: &str) -> Option<&Json> {
        match self {
            Json::Object(fields) => fields.iter().find(|(k, _)| k == key).map(|(_, v)| v),
            _ => None,
        }
    }

    pub fn as_str(&self) -> Option<&str> {
        match self {
            Json::String(text) => Some(text),
            _ => None,
        }
    }

    pub fn as_object(&self) -> Option<&Vec<(String, Json)>> {
        match self {
            Json::Object(fields) => Some(fields),
            _ => None,
        }
    }

    pub fn as_object_mut(&mut self) -> Option<&mut Vec<(String, Json)>> {
        match self {
            Json::Object(fields) => Some(fields),
            _ => None,
        }
    }

    /// Sets `key` where it is, or at the end.
    pub fn set(&mut self, key: &str, value: Json) {
        if let Json::Object(fields) = self {
            match fields.iter_mut().find(|(k, _)| k == key) {
                Some(slot) => slot.1 = value,
                None => fields.push((key.to_string(), value)),
            }
        }
    }

    pub fn remove(&mut self, key: &str) -> Option<Json> {
        let fields = self.as_object_mut()?;
        let index = fields.iter().position(|(k, _)| k == key)?;
        Some(fields.remove(index).1)
    }

    /// The text Lorca writes: two-space indentation and a final newline, as editors leave it.
    pub fn to_pretty(&self) -> Vec<u8> {
        let mut bytes = serde_json::to_vec_pretty(self).unwrap_or_default();
        bytes.push(b'\n');
        bytes
    }
}

impl From<&Json> for Value {
    fn from(json: &Json) -> Value {
        match json {
            Json::Null => Value::Null,
            Json::Bool(b) => Value::Bool(*b),
            Json::Number(n) => Value::Number(n.clone()),
            Json::String(s) => Value::String(s.clone()),
            Json::Array(items) => Value::Array(items.iter().map(Value::from).collect()),
            Json::Object(fields) => Value::Object(fields.iter().map(|(k, v)| (k.clone(), Value::from(v))).collect()),
        }
    }
}

impl From<&Value> for Json {
    fn from(value: &Value) -> Json {
        match value {
            Value::Null => Json::Null,
            Value::Bool(b) => Json::Bool(*b),
            Value::Number(n) => Json::Number(n.clone()),
            Value::String(s) => Json::String(s.clone()),
            Value::Array(items) => Json::Array(items.iter().map(Json::from).collect()),
            Value::Object(fields) => Json::Object(fields.iter().map(|(k, v)| (k.clone(), Json::from(v))).collect()),
        }
    }
}

impl Serialize for Json {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        match self {
            Json::Null => serializer.serialize_unit(),
            Json::Bool(b) => serializer.serialize_bool(*b),
            Json::Number(n) => n.serialize(serializer),
            Json::String(s) => serializer.serialize_str(s),
            Json::Array(items) => {
                let mut seq = serializer.serialize_seq(Some(items.len()))?;
                for item in items {
                    seq.serialize_element(item)?;
                }
                seq.end()
            }
            Json::Object(fields) => {
                let mut map = serializer.serialize_map(Some(fields.len()))?;
                for (key, value) in fields {
                    map.serialize_entry(key, value)?;
                }
                map.end()
            }
        }
    }
}

impl<'de> Deserialize<'de> for Json {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Json, D::Error> {
        deserializer.deserialize_any(JsonVisitor)
    }
}

struct JsonVisitor;

impl<'de> Visitor<'de> for JsonVisitor {
    type Value = Json;

    fn expecting(&self, formatter: &mut std::fmt::Formatter) -> std::fmt::Result {
        formatter.write_str("JSON")
    }
    fn visit_unit<E: de::Error>(self) -> Result<Json, E> {
        Ok(Json::Null)
    }
    fn visit_none<E: de::Error>(self) -> Result<Json, E> {
        Ok(Json::Null)
    }
    fn visit_some<D: Deserializer<'de>>(self, deserializer: D) -> Result<Json, D::Error> {
        Json::deserialize(deserializer)
    }
    fn visit_bool<E: de::Error>(self, value: bool) -> Result<Json, E> {
        Ok(Json::Bool(value))
    }
    fn visit_i64<E: de::Error>(self, value: i64) -> Result<Json, E> {
        Ok(Json::Number(value.into()))
    }
    fn visit_u64<E: de::Error>(self, value: u64) -> Result<Json, E> {
        Ok(Json::Number(value.into()))
    }
    fn visit_f64<E: de::Error>(self, value: f64) -> Result<Json, E> {
        Ok(serde_json::Number::from_f64(value).map(Json::Number).unwrap_or(Json::Null))
    }
    fn visit_str<E: de::Error>(self, value: &str) -> Result<Json, E> {
        Ok(Json::String(value.to_string()))
    }
    fn visit_string<E: de::Error>(self, value: String) -> Result<Json, E> {
        Ok(Json::String(value))
    }
    fn visit_seq<A: SeqAccess<'de>>(self, mut seq: A) -> Result<Json, A::Error> {
        let mut items = Vec::new();
        while let Some(item) = seq.next_element::<Json>()? {
            items.push(item);
        }
        Ok(Json::Array(items))
    }
    fn visit_map<A: MapAccess<'de>>(self, mut map: A) -> Result<Json, A::Error> {
        // A key given twice takes the last value, as JSON readers do, in the first one's place.
        let mut object = Json::object();
        while let Some((key, value)) = map.next_entry::<String, Json>()? {
            object.set(&key, value);
        }
        Ok(object)
    }
}

// MARK: - A server's entry

/// One server as its entry says it, in Lorca's terms.
#[derive(Debug, Clone, PartialEq)]
pub struct ServerConfig {
    pub kind: Kind,
    pub description: String,
    pub enabled: bool,
    /// Seconds a call may go without an answer or progress (`timeout`).
    pub timeout: Option<u64>,
    /// The tools shown or hidden, from `toolExposure` and a hidden `exposure`.
    pub tools: Vec<super::ToolRule>,
}

#[derive(Debug, Clone, PartialEq)]
pub enum Kind {
    /// A command on the Runner speaking MCP on stdio.
    Stdio { command: String, args: Vec<String>, env: Vec<(String, String)>, cwd: Option<String> },
    /// A streamable-HTTP server. `oauth` is how it signs in when it asks; none when the entry sends
    /// its own `Authorization` header or says `"oauth": false`.
    Http { url: String, headers: Vec<(String, String)>, sse: bool, oauth: Option<OAuth> },
}

/// How a remote server signs in, when its entry says: a preregistered client for a server that
/// registers none on the fly, with the redirect it was registered with, the name to register
/// under, the scopes to ask for, and the authorization server's metadata when discovery is wrong.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct OAuth {
    pub client_id: Option<String>,
    pub client_secret: Option<String>,
    pub scopes: Vec<String>,
    pub client_name: Option<String>,
    pub callback_port: Option<u16>,
    pub callback_url: Option<String>,
    pub auth_server_metadata_url: Option<String>,
}

/// Whether `url` is plain http to this computer, as a redirect a browser comes back to is.
fn is_loopback_http(url: &reqwest::Url) -> bool {
    url.scheme() == "http" && matches!(url.host_str(), Some("localhost" | "127.0.0.1" | "[::1]"))
}

/// An entry's `timeout`: seconds, as pi writes it, or milliseconds when it is 1000 or more, as
/// Gemini CLI writes it.
fn timeout_of(value: &Json) -> Result<u64, String> {
    let number = match value {
        Json::Number(n) => n.as_f64(),
        Json::String(text) => text.trim().parse::<f64>().ok(),
        _ => None,
    }
    .filter(|n| n.is_finite() && *n > 0.0)
    .ok_or("timeout is a number of seconds.")?;
    Ok(if number >= 1000.0 { (number / 1000.0).ceil() as u64 } else { number.ceil() as u64 })
}

/// An entry's `toolExposure` and `exposure`, as pi writes them: `hidden` keeps a tool, or with
/// `exposure`, every tool not named otherwise, from bots; `codemode`, `deferred`, and `direct` all
/// show it, since Lorca's bots reach every plugin tool from a script.
fn tool_rules(entry: &Json) -> Result<Vec<super::ToolRule>, String> {
    let hidden = |value: &Json, field: &str| match value.as_str() {
        Some("hidden") => Ok(true),
        Some("codemode" | "codemode-deferred" | "deferred" | "direct") => Ok(false),
        _ => Err(format!("{field} is hidden, codemode, deferred, or direct.")),
    };
    let mut rules = Vec::new();
    match entry.get("toolExposure") {
        Some(Json::Object(fields)) => {
            for (pattern, value) in fields {
                rules.push(super::ToolRule { pattern: pattern.clone(), hidden: hidden(value, &format!("toolExposure.{pattern}"))? });
            }
        }
        Some(_) => return Err("toolExposure is an object of tool names and exposures.".into()),
        None => {}
    }
    if let Some(value) = entry.get("exposure") {
        if hidden(value, "exposure")? {
            rules.push(super::ToolRule { pattern: "*".into(), hidden: true });
        }
    }
    Ok(rules)
}

/// The keys of an entry in the order Lorca writes them, after the ones the entry already has.
const CANONICAL_KEYS: [&str; 10] = ["type", "command", "args", "env", "cwd", "url", "headers", "oauth", "description", "disabled"];

/// The canonical key another app spells `key` as, when it is one Lorca reads.
fn canonical_key(key: &str) -> Option<&'static str> {
    Some(match key {
        "type" | "transport" => "type",
        "command" => "command",
        "args" => "args",
        "env" | "environment" => "env",
        "cwd" => "cwd",
        "url" | "serverUrl" | "httpUrl" => "url",
        "headers" => "headers",
        "oauth" => "oauth",
        "description" => "description",
        "enabled" | "disabled" => "disabled",
        _ => return None,
    })
}

fn type_of(text: &str) -> Result<&'static str, String> {
    match text.trim().to_ascii_lowercase().as_str() {
        "stdio" | "local" => Ok("stdio"),
        "http" | "streamable-http" | "streamable_http" | "streamablehttp" | "remote" => Ok("http"),
        "sse" => Ok("sse"),
        other => Err(format!("Unknown type \"{other}\": a server's type is stdio, http, or sse.")),
    }
}

/// A list of strings, with numbers and booleans as their text.
fn strings(value: &Json, field: &str) -> Result<Vec<String>, String> {
    let Json::Array(items) = value else { return Err(format!("{field} is a list of strings.")) };
    items.iter().map(|item| scalar(item).ok_or_else(|| format!("{field} is a list of strings."))).collect()
}

fn scalar(value: &Json) -> Option<String> {
    match value {
        Json::String(s) => Some(s.clone()),
        Json::Number(n) => Some(n.to_string()),
        Json::Bool(b) => Some(b.to_string()),
        _ => None,
    }
}

/// Names to strings, in the entry's order; a null value is left out.
fn string_map(value: &Json, field: &str) -> Result<Vec<(String, String)>, String> {
    let Json::Object(fields) = value else { return Err(format!("{field} is an object of names and strings.")) };
    let mut out = Vec::new();
    for (key, value) in fields {
        if *value == Json::Null {
            continue;
        }
        out.push((key.clone(), scalar(value).ok_or_else(|| format!("{field}.{key} is a string."))?));
    }
    Ok(out)
}

fn pairs(pairs: &[(String, String)]) -> Json {
    Json::Object(pairs.iter().map(|(k, v)| (k.clone(), Json::String(v.clone()))).collect())
}

/// One server's entry as Lorca writes it: the spellings other apps use for the same field
/// (Windsurf's `serverUrl`, OpenCode's `environment` and command list, Zed's command object,
/// `enabled`) in the one Claude's and Cursor's files use, values as strings, a remote server's
/// `type`, and the fields Lorca does not know, as they are. The entry's own order stays.
pub fn canonical(entry: &Json) -> Result<Json, String> {
    let Json::Object(fields) = entry else { return Err("A server is a JSON object.".into()) };
    let mut kind: Option<&'static str> = None;
    let mut command: Option<String> = None;
    let mut args: Option<Vec<String>> = None;
    let mut leading_args: Vec<String> = Vec::new();
    let mut env: Option<Vec<(String, String)>> = None;
    let mut url: Option<String> = None;
    let mut values: BTreeMap<&'static str, Json> = BTreeMap::new();
    let mut disabled = false;
    for (key, value) in fields {
        match key.as_str() {
            "type" | "transport" => kind = Some(type_of(value.as_str().ok_or("type is a string.")?)?),
            "command" => match value {
                Json::String(text) => command = Some(text.clone()),
                // OpenCode: the command and its arguments in one list.
                Json::Array(_) => {
                    let mut parts = strings(value, "command")?.into_iter();
                    command = Some(parts.next().ok_or("command is empty.")?);
                    leading_args = parts.collect();
                }
                // Zed: { "path", "args", "env" }.
                Json::Object(_) => {
                    command = Some(value.get("path").and_then(Json::as_str).ok_or("command.path is a string.")?.to_string());
                    if let Some(list) = value.get("args") {
                        leading_args = strings(list, "command.args")?;
                    }
                    if let Some(map) = value.get("env") {
                        env.get_or_insert_with(Vec::new).extend(string_map(map, "command.env")?);
                    }
                }
                _ => return Err("command is a string.".into()),
            },
            "args" => args = Some(strings(value, "args")?),
            "env" | "environment" => env.get_or_insert_with(Vec::new).extend(string_map(value, key)?),
            "cwd" | "description" => {
                let text = scalar(value).ok_or_else(|| format!("{key} is a string."))?;
                values.insert(if key == "cwd" { "cwd" } else { "description" }, Json::String(text));
            }
            "url" | "serverUrl" | "httpUrl" => {
                url = Some(value.as_str().ok_or_else(|| format!("{key} is a string."))?.to_string());
                if key == "httpUrl" && kind.is_none() {
                    kind = Some("http");
                }
            }
            "headers" => {
                values.insert("headers", pairs(&string_map(value, "headers")?));
            }
            "enabled" => disabled |= *value == Json::Bool(false),
            "disabled" => disabled |= *value == Json::Bool(true),
            "oauth" => {
                values.insert("oauth", value.clone());
            }
            _ => {}
        }
    }
    let has_command = command.is_some();
    if let Some(command) = command {
        values.insert("command", Json::String(command));
    }
    if args.is_some() || !leading_args.is_empty() {
        let mut all = leading_args;
        all.extend(args.unwrap_or_default());
        values.insert("args", Json::Array(all.into_iter().map(Json::String).collect()));
    }
    if let Some(env) = env {
        values.insert("env", pairs(&env));
    }
    if let Some(url) = &url {
        values.insert("url", Json::String(url.clone()));
        // Claude Code reads an entry without a type as a command, so a remote one says what it
        // is. One with a command too stays as it is, for `read` to ask which it is.
        if kind.is_some() || !has_command {
            values.insert("type", Json::String(kind.unwrap_or("http").to_string()));
        }
    } else if let Some(kind) = kind {
        values.insert("type", Json::String(kind.to_string()));
    }
    if disabled {
        values.insert("disabled", Json::Bool(true));
    }

    let mut out = Json::object();
    let mut placed: Vec<&'static str> = Vec::new();
    // A remote entry gains its type first, where Claude Code's own entries have it.
    if url.is_some() && !fields.iter().any(|(key, _)| canonical_key(key) == Some("type")) {
        if let Some(value) = values.get("type") {
            out.set("type", value.clone());
            placed.push("type");
        }
    }
    for (key, value) in fields {
        match canonical_key(key) {
            Some(known) if !placed.contains(&known) => {
                placed.push(known);
                if let Some(value) = values.get(known) {
                    out.set(known, value.clone());
                }
                // A list command's arguments follow it, when the entry has no `args` of its own.
                if known == "command" && !fields.iter().any(|(k, _)| k == "args") {
                    if let Some(value) = values.get("args") {
                        out.set("args", value.clone());
                        placed.push("args");
                    }
                }
            }
            Some(_) => {}
            None => out.set(key, value.clone()),
        }
    }
    for key in CANONICAL_KEYS {
        if !placed.contains(&key) {
            if let Some(value) = values.get(key) {
                out.set(key, value.clone());
            }
        }
    }
    Ok(out)
}

impl ServerConfig {
    /// What a canonical entry says, or why it cannot run.
    pub fn read(entry: &Json) -> Result<ServerConfig, String> {
        let text = |key: &str| entry.get(key).and_then(Json::as_str).map(str::to_string);
        let declared = text("type");
        let command = text("command").filter(|c| !c.trim().is_empty());
        let url = text("url").filter(|u| !u.trim().is_empty());
        let remote = match (declared.as_deref(), &command, &url) {
            (Some("stdio"), _, _) => false,
            (Some(_), _, _) => true,
            (None, Some(_), Some(_)) => return Err("A server runs a command or connects to a URL, and this one has both. Set its type to stdio or http.".into()),
            (None, Some(_), None) => false,
            (None, None, Some(_)) => true,
            (None, None, None) => return Err("Give the server a command to run or a URL to connect to.".into()),
        };
        let kind = if remote {
            let url = url.ok_or("Give the server its URL.")?.trim().to_string();
            if !(url.starts_with("http://") || url.starts_with("https://") || url.starts_with("${")) {
                return Err("The server's URL starts with http:// or https://.".into());
            }
            let headers = match entry.get("headers") {
                Some(map) => string_map(map, "headers")?,
                None => Vec::new(),
            };
            let oauth = match entry.get("oauth") {
                Some(Json::Bool(false)) => None,
                Some(object @ Json::Object(_)) => {
                    let field = |a: &str, b: &str| object.get(a).or_else(|| object.get(b)).and_then(Json::as_str).map(str::to_string).filter(|v| !v.trim().is_empty());
                    // `scopes` as a list or a string, or pi's `scope`, a string.
                    let scopes = match object.get("scopes").or_else(|| object.get("scope")) {
                        Some(Json::String(text)) => text.split_whitespace().map(str::to_string).collect(),
                        Some(list) => strings(list, "oauth.scopes")?,
                        None => Vec::new(),
                    };
                    let callback_port = match object.get("callbackPort").or_else(|| object.get("callback_port")) {
                        Some(value) => Some(scalar(value).and_then(|text| text.parse::<u16>().ok()).filter(|port| *port > 0).ok_or("oauth.callbackPort is a port number.")?),
                        None => None,
                    };
                    let callback_url = field("callbackUrl", "callback_url");
                    if let Some(callback) = &callback_url {
                        if !reqwest::Url::parse(callback).is_ok_and(|url| is_loopback_http(&url)) {
                            return Err("oauth.callbackUrl is http on localhost, 127.0.0.1, or [::1].".into());
                        }
                    }
                    let auth_server_metadata_url = field("authServerMetadataUrl", "auth_server_metadata_url");
                    if let Some(metadata) = &auth_server_metadata_url {
                        if !reqwest::Url::parse(metadata).is_ok_and(|url| url.scheme() == "https" || is_loopback_http(&url)) {
                            return Err("oauth.authServerMetadataUrl is an https URL.".into());
                        }
                    }
                    Some(OAuth {
                        client_id: field("clientId", "client_id"),
                        client_secret: field("clientSecret", "client_secret"),
                        scopes,
                        client_name: field("clientName", "client_name"),
                        callback_port,
                        callback_url,
                        auth_server_metadata_url,
                    })
                }
                Some(Json::Bool(true)) | None => {
                    // An entry that sends its own credentials signs in with them.
                    (!headers.iter().any(|(name, _)| name.eq_ignore_ascii_case("authorization"))).then(OAuth::default)
                }
                Some(_) => return Err("oauth is false or an object.".into()),
            };
            Kind::Http { url, headers, sse: declared.as_deref() == Some("sse"), oauth }
        } else {
            let command = command.ok_or("Give the server a command to run.")?.trim().to_string();
            let args = match entry.get("args") {
                Some(list) => strings(list, "args")?,
                None => Vec::new(),
            };
            let env = match entry.get("env") {
                Some(map) => string_map(map, "env")?,
                None => Vec::new(),
            };
            Kind::Stdio { command, args, env, cwd: text("cwd").filter(|c| !c.trim().is_empty()) }
        };
        Ok(ServerConfig {
            kind,
            description: text("description").unwrap_or_default().trim().to_string(),
            enabled: entry.get("disabled") != Some(&Json::Bool(true)),
            timeout: entry.get("timeout").map(timeout_of).transpose()?,
            tools: tool_rules(entry)?,
        })
    }

    pub fn is_remote(&self) -> bool {
        matches!(self.kind, Kind::Http { .. })
    }

    /// The server as a plugin manifest declares one.
    pub fn spec(&self) -> ServerSpec {
        match &self.kind {
            Kind::Stdio { command, args, env, cwd } => {
                ServerSpec::Stdio { command: command.clone(), args: args.clone(), env: env.iter().cloned().collect(), cwd: cwd.clone(), timeout: self.timeout }
            }
            Kind::Http { url, headers, oauth, .. } => ServerSpec::Http {
                url: url.clone(),
                headers: headers.iter().cloned().collect(),
                auth: oauth.as_ref().map(|oauth| AuthSpec::Oauth {
                    scopes: oauth.scopes.clone(),
                    token_variable: None,
                    client_id_variable: None,
                    client_secret_variable: None,
                    client_id: oauth.client_id.clone(),
                    client_secret: oauth.client_secret.clone(),
                    device_authorization_endpoint: None,
                    token_endpoint: None,
                    authorization_endpoint: None,
                    authorization_params: BTreeMap::new(),
                    optional: true,
                    client_name: oauth.client_name.clone(),
                    callback_port: oauth.callback_port,
                    callback_url: oauth.callback_url.clone(),
                    auth_server_metadata_url: oauth.auth_server_metadata_url.clone(),
                }),
                timeout: self.timeout,
            },
        }
    }

    /// What the apps and the bot read under the server's name when the entry has no description:
    /// the program it runs or the host it reaches, never an argument, a header, or the URL's path,
    /// which can hold a key.
    pub fn summary(&self) -> String {
        match &self.kind {
            Kind::Stdio { command, .. } => {
                let program = Path::new(command).file_name().map(|name| name.to_string_lossy().into_owned()).unwrap_or_else(|| command.clone());
                format!("Local MCP server · {program}")
            }
            Kind::Http { url, .. } => match reqwest::Url::parse(url).ok().and_then(|url| url.host_str().map(str::to_string)) {
                Some(host) => format!("Remote MCP server · {host}"),
                None => "Remote MCP server".into(),
            },
        }
    }
}

// MARK: - The file

/// One entry of the file, usable or not.
#[derive(Debug, Clone, PartialEq)]
pub struct FileServer {
    pub name: String,
    /// The plugin id it runs as: its name as an id, or for a name with no letters or digits,
    /// a stable one made from it.
    pub id: String,
    /// The entry as the file holds it.
    pub entry: Json,
    /// What it says, or why it cannot run.
    pub config: Result<ServerConfig, String>,
    /// Another plugin on the Runner already has its id.
    pub clash: Option<String>,
}

impl FileServer {
    fn new(name: String, entry: Json) -> FileServer {
        let config = canonical(&entry).and_then(|canonical| ServerConfig::read(&canonical));
        FileServer { id: server_id(&name), name, entry, config, clash: None }
    }

    /// Why it does not run, when it cannot.
    pub fn problem(&self) -> Option<String> {
        self.config.as_ref().err().cloned().or_else(|| self.clash.clone())
    }

    pub fn is_enabled(&self) -> bool {
        match &self.config {
            Ok(config) => config.enabled,
            Err(_) => self.entry.get("disabled") != Some(&Json::Bool(true)) && self.entry.get("enabled") != Some(&Json::Bool(false)),
        }
    }

    /// The plugin it is while it can run and is on.
    fn plugin(&self) -> Option<Installed> {
        let config = self.config.as_ref().ok().filter(|config| config.enabled && self.clash.is_none())?;
        let description = if config.description.is_empty() { config.summary() } else { config.description.clone() };
        Some(Installed {
            manifest: Manifest {
                id: self.id.clone(),
                name: self.name.clone(),
                description,
                version: String::new(),
                icon: if config.is_remote() { "globe".into() } else { "terminal".into() },
                homepage: None,
                author: String::new(),
                category: String::new(),
                featured: false,
                tags: Vec::new(),
                named_accounts: false,
                servers: BTreeMap::from([(SERVER.to_string(), config.spec())]),
                variables: Vec::new(),
                skills: Vec::new(),
                tools: ToolHints { exposure: config.tools.clone(), ..ToolHints::default() },
            },
            source: SOURCE.into(),
            installed_at: 0.0,
            variables: BTreeMap::new(),
            service_id: None,
            account_name: None,
        })
    }
}

/// The plugin id of the server `name`: `My GitHub` → `my-github`, at most 48 characters; a name
/// with no letters or digits (`日本語`) gets a stable one made from it.
pub fn server_id(name: &str) -> String {
    let mut slug = super::slug(name);
    if slug.len() > 48 {
        slug.truncate(48);
        slug = slug.trim_end_matches('-').to_string();
    }
    if !slug.is_empty() {
        return slug;
    }
    let digest = <sha2::Sha256 as sha2::Digest>::digest(name.as_bytes());
    format!("mcp-{}", digest.iter().take(4).map(|byte| format!("{byte:02x}")).collect::<String>())
}

/// `mcp.json` as the Runner last read it.
#[derive(Debug, Clone, Default)]
pub struct McpFile {
    /// What was read, to tell a change on disk from Lorca's own write.
    pub bytes: Option<Vec<u8>>,
    pub doc: Json,
    pub servers: Vec<FileServer>,
    /// Why the file cannot be read. The servers stay the ones read before.
    pub error: Option<String>,
}

/// The object of the file that holds the servers: `mcpServers`, or VS Code's `servers` in a file
/// that has only that.
fn container_key(doc: &Json) -> &'static str {
    if doc.get("mcpServers").is_none() && doc.get("servers").is_some_and(|servers| matches!(servers, Json::Object(_))) {
        "servers"
    } else {
        "mcpServers"
    }
}

impl McpFile {
    pub fn read(path: &Path) -> McpFile {
        match std::fs::read(path) {
            Ok(bytes) => McpFile::parse(bytes),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => McpFile::default(),
            Err(error) => McpFile { error: Some(format!("mcp.json cannot be read: {error}.")), ..McpFile::default() },
        }
    }

    pub fn parse(bytes: Vec<u8>) -> McpFile {
        if bytes.iter().all(u8::is_ascii_whitespace) {
            return McpFile { bytes: Some(bytes), ..McpFile::default() };
        }
        let doc = match serde_json::from_slice::<Json>(&bytes) {
            Ok(doc) => doc,
            Err(error) => return McpFile { bytes: Some(bytes), error: Some(format!("mcp.json is not valid JSON: {error}.")), ..McpFile::default() },
        };
        if !matches!(doc, Json::Object(_)) {
            return McpFile { bytes: Some(bytes), error: Some("mcp.json is not a JSON object.".into()), ..McpFile::default() };
        }
        let servers = match doc.get(container_key(&doc)) {
            None => Vec::new(),
            Some(Json::Object(entries)) => entries.iter().map(|(name, entry)| FileServer::new(name.clone(), entry.clone())).collect(),
            Some(_) => return McpFile { bytes: Some(bytes), error: Some("mcpServers in mcp.json is not an object.".into()), ..McpFile::default() },
        };
        McpFile { bytes: Some(bytes), doc, servers, error: None }
    }

    pub fn server(&self, name: &str) -> Option<&FileServer> {
        self.servers.iter().find(|server| server.name == name)
    }

    /// The servers' object in the document, made when the file has none yet.
    fn entries_mut(doc: &mut Json) -> &mut Vec<(String, Json)> {
        if !matches!(doc, Json::Object(_)) {
            *doc = Json::object();
        }
        let key = container_key(doc);
        if !matches!(doc.get(key), Some(Json::Object(_))) {
            doc.set(key, Json::object());
        }
        match doc {
            Json::Object(fields) => match fields.iter_mut().find(|(k, _)| k == key) {
                Some((_, Json::Object(entries))) => entries,
                _ => unreachable!("the servers' object was just made"),
            },
            _ => unreachable!("the document was just made an object"),
        }
    }
}

/// An entry's known keys in the order Lorca writes them (`command` before `args`), then the rest
/// as they came: for an entry that arrived as a JSON object from the apps, whose keys are sorted.
pub fn in_canonical_order(entry: &Json) -> Json {
    let Some(fields) = entry.as_object() else { return entry.clone() };
    let mut out = Json::object();
    for key in CANONICAL_KEYS {
        if let Some(value) = entry.get(key) {
            out.set(key, value.clone());
        }
    }
    for (key, value) in fields {
        if out.get(key).is_none() {
            out.set(key, value.clone());
        }
    }
    out
}

/// `desired` with the keys `old` already has where `old` has them, then the rest in `desired`'s
/// order, so an edit leaves the entry's layout as the user arranged it.
fn in_order_of(old: &Json, desired: &Json) -> Json {
    let (Some(old_fields), Some(new_fields)) = (old.as_object(), desired.as_object()) else { return desired.clone() };
    let mut out = Json::object();
    for (key, _) in old_fields {
        if let Some(value) = desired.get(key) {
            out.set(key, value.clone());
        }
    }
    for (key, value) in new_fields {
        if out.get(key).is_none() {
            out.set(key, value.clone());
        }
    }
    out
}

/// The document with the entry `name` saved as `config` (a canonical entry), in place of
/// `previous`'s when it renames one, else in its own place, else at the end.
pub fn with_entry(doc: &Json, name: &str, previous: Option<&str>, config: &Json) -> Json {
    let mut doc = doc.clone();
    let entries = McpFile::entries_mut(&mut doc);
    let at = previous.and_then(|previous| entries.iter().position(|(key, _)| key == previous)).or_else(|| entries.iter().position(|(key, _)| key == name));
    match at {
        Some(index) => {
            let entry = in_order_of(&entries[index].1, config);
            entries[index] = (name.to_string(), entry);
            // A rename onto another entry's name was refused before this; a stray copy goes.
            let mut seen = false;
            entries.retain(|(key, _)| {
                if key != name {
                    return true;
                }
                let keep = !seen;
                seen = true;
                keep
            });
        }
        None => entries.push((name.to_string(), config.clone())),
    }
    doc
}

pub fn without_entry(doc: &Json, name: &str) -> Json {
    let mut doc = doc.clone();
    McpFile::entries_mut(&mut doc).retain(|(key, _)| key != name);
    doc
}

// MARK: - The Runner's plugins

/// What a new reading of the file changed among the Runner's plugins.
#[derive(Debug, Default, PartialEq)]
pub struct Changes {
    /// Servers that came, or whose plugin changed.
    pub changed: Vec<String>,
    /// Servers that went: removed, turned off, or unusable now.
    pub gone: Vec<String>,
    /// Whether the file's state changed at all, an error included.
    pub any: bool,
}

impl Store {
    /// Takes `file` as the Runner's servers: each one that can run and is on becomes a plugin after
    /// the ones installed otherwise, keeping its place in the file. A file that cannot be read
    /// keeps the servers there were, with its error.
    pub fn take_mcp(&mut self, mut file: McpFile) -> Changes {
        if file.error.is_some() {
            let any = self.mcp.error != file.error;
            self.mcp.error = file.error;
            self.mcp.bytes = file.bytes;
            return Changes { any, ..Changes::default() };
        }
        let before: Vec<Installed> = self.installed.iter().filter(|p| p.source == SOURCE).cloned().collect();
        self.installed.retain(|p| p.source != SOURCE);
        let mut taken: Vec<String> = self.installed.iter().map(|p| p.manifest.id.clone()).collect();
        let mut fresh = Vec::new();
        for server in &mut file.servers {
            server.clash = None;
            if taken.contains(&server.id) {
                let other = self.installed.iter().find(|p| p.manifest.id == server.id).map(|p| p.manifest.name.clone());
                server.clash = Some(match other {
                    Some(plugin) => format!("{plugin} is installed with the id {}. Rename this server.", server.id),
                    None => format!("Another server in mcp.json has the id {}. Rename one of them.", server.id),
                });
                continue;
            }
            taken.push(server.id.clone());
            if let Some(plugin) = server.plugin() {
                fresh.push(plugin);
            }
        }
        // Which tools a server shows is read as its tools are listed, so a change to that alone
        // leaves its connection and its tool list as they are.
        let unhinted = |manifest: &Manifest| Manifest { tools: ToolHints::default(), ..manifest.clone() };
        let changed: Vec<String> = fresh
            .iter()
            .filter(|plugin| !before.iter().any(|old| unhinted(&old.manifest) == unhinted(&plugin.manifest)))
            .map(|plugin| plugin.manifest.id.clone())
            .collect();
        let gone: Vec<String> = before
            .iter()
            .filter(|old| !fresh.iter().any(|plugin| plugin.manifest.id == old.manifest.id))
            .map(|old| old.manifest.id.clone())
            .collect();
        let any = !changed.is_empty() || !gone.is_empty() || self.mcp.servers != file.servers || self.mcp.error.is_some();
        self.installed.extend(fresh);
        self.mcp = file;
        Changes { changed, gone, any }
    }
}

// MARK: - Pasted and imported JSON

/// `text` without `//` and `/* */` comments and with no comma before a closing bracket, as VS
/// Code's and Zed's settings allow; strings are left alone.
fn strip_jsonc(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut chars = text.chars().peekable();
    let mut in_string = false;
    while let Some(c) = chars.next() {
        if in_string {
            out.push(c);
            if c == '\\' {
                if let Some(next) = chars.next() {
                    out.push(next);
                }
            } else if c == '"' {
                in_string = false;
            }
            continue;
        }
        match c {
            '"' => {
                in_string = true;
                out.push(c);
            }
            '/' if chars.peek() == Some(&'/') => {
                for c in chars.by_ref() {
                    if c == '\n' {
                        out.push('\n');
                        break;
                    }
                }
            }
            '/' if chars.peek() == Some(&'*') => {
                chars.next();
                let mut last = ' ';
                for c in chars.by_ref() {
                    if last == '*' && c == '/' {
                        break;
                    }
                    last = c;
                }
            }
            _ => out.push(c),
        }
    }
    // Trailing commas: a comma whose next non-space character closes an object or a list.
    let mut cleaned = String::with_capacity(out.len());
    let chars: Vec<char> = out.chars().collect();
    let mut in_string = false;
    let mut index = 0;
    while index < chars.len() {
        let c = chars[index];
        if in_string {
            cleaned.push(c);
            if c == '\\' && index + 1 < chars.len() {
                cleaned.push(chars[index + 1]);
                index += 1;
            } else if c == '"' {
                in_string = false;
            }
        } else if c == '"' {
            in_string = true;
            cleaned.push(c);
        } else if c == ',' && chars[index + 1..].iter().find(|c| !c.is_whitespace()).is_some_and(|c| *c == '}' || *c == ']') {
            // Dropped.
        } else {
            cleaned.push(c);
        }
        index += 1;
    }
    cleaned
}

/// Servers read from pasted or imported JSON: each one's name when it has one, and its canonical
/// entry or why it cannot run.
pub type Parsed = Vec<(Option<String>, Result<Json, String>)>;

fn looks_like_server(value: &Json) -> bool {
    ["command", "url", "serverUrl", "httpUrl"].iter().any(|key| value.get(key).is_some())
}

/// The servers a piece of JSON describes, the way apps' config files and READMEs spell them: a
/// whole file (`mcpServers`, VS Code's `servers`, OpenCode's `mcp`, Zed's `context_servers`),
/// servers by name, a bare `"name": { … }` pair, or one server with no name. Each comes back as
/// its canonical entry, or with why it cannot run.
pub fn parse_servers(text: &str) -> Result<Parsed, String> {
    let cleaned = strip_jsonc(text.trim());
    if cleaned.trim().is_empty() {
        return Err("Paste a server's JSON.".into());
    }
    let doc: Json = match serde_json::from_str(&cleaned) {
        Ok(doc) => doc,
        // A README's `"name": { … }` fragment, without its braces.
        Err(error) => serde_json::from_str(&format!("{{{}}}", cleaned.trim().trim_end_matches(','))).map_err(|_| format!("Not valid JSON: {error}"))?,
    };
    let Json::Object(fields) = &doc else { return Err("Not a server's JSON: expected an object.".into()) };
    if looks_like_server(&doc) {
        return Ok(vec![(None, canonical(&doc))]);
    }
    let container = ["mcpServers", "servers", "context_servers", "mcp"]
        .iter()
        .filter_map(|key| doc.get(key))
        .find(|value| value.as_object().is_some_and(|entries| entries.iter().any(|(_, entry)| looks_like_server(entry))));
    let entries = match container {
        Some(Json::Object(entries)) => entries,
        _ if fields.iter().all(|(_, value)| looks_like_server(value)) => fields,
        _ => return Err("No MCP servers in that JSON: expected mcpServers, a server's command, or its url.".into()),
    };
    Ok(entries.iter().map(|(name, entry)| (Some(name.clone()), canonical(entry))).collect())
}

/// The servers in another app's settings file: what its `mcpServers` (or VS Code's `servers`,
/// OpenCode's `mcp`, Zed's `context_servers`) holds, none when it has no such object.
pub fn servers_in_app_config(text: &str) -> Result<Parsed, String> {
    let doc: Json = serde_json::from_str(&strip_jsonc(text)).map_err(|error| format!("Not valid JSON: {error}"))?;
    let container = ["mcpServers", "servers", "context_servers", "mcp"]
        .iter()
        .filter_map(|key| doc.get(key))
        .find(|value| value.as_object().is_some_and(|entries| entries.iter().any(|(_, entry)| looks_like_server(entry))));
    Ok(match container {
        Some(Json::Object(entries)) => entries.iter().map(|(name, entry)| (Some(name.clone()), canonical(entry))).collect(),
        _ => Vec::new(),
    })
}

/// Where other apps on this computer keep their MCP servers, for `lorca mcp import`: the ones
/// found, with their names.
pub fn known_sources() -> Vec<(&'static str, PathBuf)> {
    let Some(home) = dirs::home_dir() else { return Vec::new() };
    let config = dirs::config_dir().unwrap_or_else(|| home.join(".config"));
    let mut candidates: Vec<(&'static str, PathBuf)> = vec![
        ("Claude Desktop", config.join("Claude").join("claude_desktop_config.json")),
        ("Claude Code", home.join(".claude.json")),
        ("Cursor", home.join(".cursor").join("mcp.json")),
        ("Windsurf", home.join(".codeium").join("windsurf").join("mcp_config.json")),
        ("VS Code", config.join("Code").join("User").join("mcp.json")),
        ("Gemini CLI", home.join(".gemini").join("settings.json")),
    ];
    candidates.retain(|(_, path)| path.is_file());
    candidates
}

// MARK: - The apps' view

/// A server as the apps and `lorca mcp` show it: its entry, whether it is on, why it cannot run,
/// and, for one that runs, its plugin's state and how many tools it offered last time.
pub fn server_out(store: &Store, server: &FileServer, tool_count: Option<usize>) -> Value {
    let canonical_entry = canonical(&server.entry).unwrap_or_else(|_| server.entry.clone());
    let remote = server.config.as_ref().map(ServerConfig::is_remote).ok();
    let signed_in = store.sign_in_secret(&server.id, "oauth", SERVER).is_some();
    let asked = store.sign_in_secret(&server.id, "challenge", SERVER).is_some();
    let signs_in = matches!(&server.config, Ok(ServerConfig { kind: Kind::Http { oauth: Some(_), .. }, .. })) && (signed_in || asked);
    json!({
        "name": server.name,
        "id": server.id,
        "transport": remote.map(|remote| if remote { "http" } else { "stdio" }),
        "enabled": server.is_enabled(),
        "config": Value::from(&canonical_entry),
        "problem": server.problem(),
        "status": store.status(&server.id).filter(|_| server.problem().is_none() && server.is_enabled()),
        "signs_in": signs_in,
        "signed_in": signed_in,
        "tool_count": tool_count,
    })
}

// MARK: - The Runner's side

#[cfg(feature = "runner")]
pub use runner::*;

#[cfg(feature = "runner")]
mod runner {
    use std::sync::atomic::{AtomicBool, Ordering};

    use super::*;

    /// Whether this process is `lorca serve`, which keeps servers connected. A one-off `lorca mcp`
    /// starts none in the background, only the one it was asked about.
    static SERVING: AtomicBool = AtomicBool::new(false);

    fn path(app: &App) -> PathBuf {
        app.config.mcp_path()
    }

    /// The plugin a server of the file runs as, while it runs.
    fn plugin_of<'a>(store: &'a Store, server: &FileServer) -> Option<&'a Installed> {
        store.get(&server.id).filter(|plugin| plugin.source == SOURCE && server.problem().is_none())
    }

    /// Every server in the file, for the apps' MCP Servers list and `lorca mcp list`.
    pub fn list(app: &Arc<App>) -> Value {
        let store = app.plugins.lock().unwrap();
        let servers: Vec<Value> = store
            .mcp
            .servers
            .iter()
            .map(|server| server_out(&store, server, plugin_of(&store, server).and_then(|plugin| super::super::mcp::saved_tool_count(app, plugin))))
            .collect();
        json!({ "path": path(app), "error": store.mcp.error, "servers": servers })
    }

    /// One server in full: what `list` says, with the tools it offered when it last connected.
    pub fn get(app: &Arc<App>, name: &str) -> Result<Value, String> {
        let store = app.plugins.lock().unwrap();
        let server = store.mcp.server(name).ok_or_else(|| format!("No server named {name} in mcp.json."))?;
        let plugin = plugin_of(&store, server);
        let tools = plugin.map(|plugin| super::super::mcp::saved_tools(app, plugin)).unwrap_or_default();
        let mut out = server_out(&store, server, plugin.and_then(|plugin| super::super::mcp::saved_tool_count(app, plugin)));
        out["tools"] = json!(tools);
        Ok(json!({ "path": path(app), "server": out }))
    }

    /// Writes `doc` as the file and takes it, refusing to write over a file that cannot be read.
    fn write(app: &Arc<App>, doc: &Json) -> Result<Changes, String> {
        let bytes = doc.to_pretty();
        let mut store = app.plugins.lock().unwrap();
        crate::config::write_private(&path(app), &bytes).map_err(|e| format!("Cannot write mcp.json: {e}"))?;
        Ok(store.take_mcp(McpFile::parse(bytes)))
    }

    /// The file as it is on disk now, so an edit by hand since the last read is kept, or why it
    /// cannot be changed.
    fn current(app: &Arc<App>) -> Result<McpFile, String> {
        let file = McpFile::read(&path(app));
        match &file.error {
            Some(error) => Err(format!("{error} Fix the file first.")),
            None => Ok(file),
        }
    }

    /// Adds the server `name`, or saves the one `previous` names under `name`. `config` is its
    /// entry, in any app's spelling; the CLI checks it and writes it back canonical.
    pub fn save(app: &Arc<App>, name: &str, previous: Option<&str>, config: &Value) -> Result<String, String> {
        let name = name.trim();
        if name.is_empty() {
            return Err("Give the server a name.".into());
        }
        if name.chars().any(char::is_control) {
            return Err("A server's name is one line of text.".into());
        }
        // The apps send an object whose keys arrive sorted; what the file has keeps its order.
        let entry = in_canonical_order(&canonical(&Json::from(config))?);
        ServerConfig::read(&entry)?;
        let file = current(app)?;
        let previous = previous.map(str::trim).filter(|previous| !previous.is_empty());
        if let Some(previous) = previous {
            if file.server(previous).is_none() {
                return Err(format!("No server named {previous} in mcp.json."));
            }
        }
        let renamed = previous.is_some_and(|previous| previous != name);
        let adding = previous.is_none();
        if (adding || renamed) && file.server(name).is_some() {
            return Err(format!("mcp.json already has a server named {name}."));
        }
        let id = server_id(name);
        let old_id = previous.and_then(|previous| file.server(previous)).map(|server| server.id.clone());
        // Another server's or plugin's id: the new one could not run.
        let clash_in_file = file.servers.iter().find(|server| server.id == id && Some(server.name.as_str()) != previous);
        if let Some(other) = clash_in_file {
            return Err(format!("{name} and {} would share the id {id}. Pick another name.", other.name));
        }
        let installed = app.plugins.lock().unwrap().get(&id).filter(|plugin| plugin.source != SOURCE).map(|plugin| plugin.manifest.name.clone());
        if let Some(plugin) = installed {
            return Err(format!("{plugin} is installed with the id {id}. Pick another name."));
        }
        let changes = write(app, &with_entry(&file.doc, name, previous, &entry))?;
        // A rename keeps the server's sign-in and its always-allowed tools.
        if let Some(old_id) = old_id.filter(|old_id| *old_id != id) {
            move_plugin(app, &old_id, &id);
        }
        settle(app, changes);
        Ok(name.to_string())
    }

    /// Removes a server from the file, with its sign-in, its folder, and its always-allowed tools.
    pub fn remove(app: &Arc<App>, name: &str) -> Result<(), String> {
        let file = current(app)?;
        let server = file.server(name).ok_or_else(|| format!("No server named {name} in mcp.json."))?.clone();
        let changes = write(app, &without_entry(&file.doc, name))?;
        forget_plugin(app, &server.id);
        settle(app, changes);
        Ok(())
    }

    /// Turns a server on or off. Off, no bot sees it and it never starts; its entry, sign-in, and
    /// tools stay.
    pub fn set_enabled(app: &Arc<App>, name: &str, enabled: bool) -> Result<(), String> {
        let file = current(app)?;
        let server = file.server(name).ok_or_else(|| format!("No server named {name} in mcp.json."))?;
        let mut entry = server.entry.clone();
        entry.remove("enabled");
        if enabled {
            entry.remove("disabled");
        } else {
            entry.set("disabled", Json::Bool(true));
        }
        let mut doc = file.doc.clone();
        if let Some((_, slot)) = McpFile::entries_mut(&mut doc).iter_mut().find(|(key, _)| key == name) {
            *slot = entry;
        }
        let changes = write(app, &doc)?;
        settle(app, changes);
        Ok(())
    }

    /// Shows or hides one of a server's tools from bots, in the entry's `toolExposure` as pi
    /// writes it: hiding names the tool `hidden`; showing drops that, and names it `codemode` when
    /// a pattern would still hide it. The tool list stays as it is, so nothing reconnects.
    pub fn hide_tool(app: &Arc<App>, name: &str, tool: &str, hidden: bool) -> Result<(), String> {
        let file = current(app)?;
        let server = file.server(name).ok_or_else(|| format!("No server named {name} in mcp.json."))?;
        let mut entry = server.entry.clone();
        let mut exposure = match entry.get("toolExposure") {
            Some(Json::Object(fields)) => fields.clone(),
            Some(_) => return Err("toolExposure is an object of tool names and exposures.".into()),
            None => Vec::new(),
        };
        exposure.retain(|(key, _)| key != tool);
        if hidden {
            exposure.push((tool.to_string(), Json::String("hidden".into())));
        } else {
            let mut probe = entry.clone();
            probe.set("toolExposure", Json::Object(exposure.clone()));
            let hints = ToolHints { exposure: tool_rules(&probe)?, ..Default::default() };
            if hints.hides(tool) {
                exposure.push((tool.to_string(), Json::String("codemode".into())));
            }
        }
        if exposure.is_empty() {
            entry.remove("toolExposure");
        } else {
            entry.set("toolExposure", Json::Object(exposure));
        }
        let mut doc = file.doc.clone();
        if let Some((_, slot)) = McpFile::entries_mut(&mut doc).iter_mut().find(|(key, _)| key == name) {
            *slot = entry;
        }
        let changes = write(app, &doc)?;
        settle(app, changes);
        Ok(())
    }

    /// Connects a server and waits for it, so the apps and `lorca mcp get` show whether it starts
    /// and what it offers. `fresh` drops its connection first, as the apps' Reconnect does;
    /// otherwise a connection it has, or one under way, is the answer. Its state says how it went.
    pub async fn reconnect(app: &Arc<App>, name: &str, fresh: bool) -> Result<(), String> {
        let (id, problem, enabled) = {
            let store = app.plugins.lock().unwrap();
            let server = store.mcp.server(name).ok_or_else(|| format!("No server named {name} in mcp.json."))?;
            (server.id.clone(), server.problem(), server.is_enabled())
        };
        if let Some(problem) = problem {
            return Err(problem);
        }
        if !enabled {
            return Err(format!("{name} is turned off."));
        }
        if fresh {
            app.mcp.forget(&id);
            super::super::note(app, &id, None);
        }
        // A failure is the server's state now, which the answer carries.
        if let Err(error) = app.mcp.server(app, &id, SERVER).await {
            tracing::info!(%error, server = %name, "an MCP server did not connect");
        }
        Ok(())
    }

    /// What the change means for the Runner's connections: a server that changed or came connects
    /// once to list its tools, one that went or changed drops its connection, and the apps hear.
    fn settle(app: &Arc<App>, changes: Changes) {
        for id in changes.changed.iter().chain(&changes.gone) {
            app.mcp.forget(id);
            app.plugins.lock().unwrap().notes.remove(id);
        }
        for id in &changes.changed {
            // Its tools are the new server's to list.
            let _ = std::fs::remove_file(app.config.plugins_dir().join(id).join("catalog.json"));
            if SERVING.load(Ordering::Relaxed) {
                super::super::mcp::prefetch_tools(app, id);
            }
        }
        if changes.any {
            super::super::announce(app);
        }
    }

    /// The sign-in, folder, and always-allowed tools of a plugin id, under a new id.
    fn move_plugin(app: &Arc<App>, from: &str, to: &str) {
        app.mcp.forget(from);
        {
            let mut store = app.plugins.lock().unwrap();
            if let Some(secrets) = store.secrets.remove(from) {
                store.secrets.insert(to.to_string(), secrets);
            }
            store.notes.remove(from);
            if let Err(error) = store.save(&app.config) {
                tracing::warn!(%error, "moving a renamed server's sign-in");
            }
        }
        let dir = app.config.plugins_dir();
        if dir.join(from).is_dir() && !dir.join(to).exists() {
            let _ = std::fs::rename(dir.join(from), dir.join(to));
        }
        let (old, new) = (format!("{from}/"), format!("{to}/"));
        let mut auto_review = app.auto_review();
        if auto_review.rules.iter().any(|rule| rule.tool.as_deref().is_some_and(|tool| tool.starts_with(&old))) {
            for rule in &mut auto_review.rules {
                if let Some(tool) = rule.tool.as_mut().filter(|tool| tool.starts_with(&old)) {
                    *tool = format!("{new}{}", &tool[old.len()..]);
                }
            }
            app.set_auto_review(auto_review);
        }
    }

    /// Forgets what the Runner keeps for a removed server: its sign-in, folder, and rules.
    fn forget_plugin(app: &Arc<App>, id: &str) {
        app.mcp.forget(id);
        {
            let mut store = app.plugins.lock().unwrap();
            store.secrets.remove(id);
            store.notes.remove(id);
            if let Err(error) = store.save(&app.config) {
                tracing::warn!(%error, "forgetting a removed server's sign-in");
            }
        }
        let dir = app.config.plugins_dir().join(id);
        if dir.is_dir() {
            let _ = std::fs::remove_dir_all(&dir);
        }
        let prefix = format!("{id}/");
        let mut auto_review = app.auto_review();
        if auto_review.rules.iter().any(|rule| rule.tool.as_deref().is_some_and(|tool| tool.starts_with(&prefix))) {
            auto_review.rules.retain(|rule| !rule.tool.as_deref().is_some_and(|tool| tool.starts_with(&prefix)));
            app.set_auto_review(auto_review);
        }
    }

    /// The `mcp.*` verbs this Runner answers, from the local app, `lorca mcp`, or a sealed request.
    pub async fn serve_request(app: &Arc<App>, verb: &str, body: &Value) -> Result<Value, String> {
        let name = || body["name"].as_str().map(str::trim).filter(|name| !name.is_empty()).map(str::to_string).ok_or_else(|| "missing name".to_string());
        match verb {
            "mcp.list" => Ok(list(app)),
            "mcp.get" => get(app, &name()?),
            "mcp.save" => {
                let saved = save(app, &name()?, body["previous_name"].as_str(), &body["config"])?;
                get(app, &saved)
            }
            "mcp.remove" => {
                remove(app, &name()?)?;
                Ok(Value::Null)
            }
            "mcp.set_enabled" => {
                let name = name()?;
                set_enabled(app, &name, body["enabled"].as_bool().ok_or("missing enabled")?)?;
                get(app, &name)
            }
            "mcp.hide_tool" => {
                let name = name()?;
                let tool = body["tool"].as_str().filter(|tool| !tool.is_empty()).ok_or("missing tool")?;
                hide_tool(app, &name, tool, body["hidden"].as_bool().ok_or("missing hidden")?)?;
                get(app, &name)
            }
            "mcp.reconnect" => {
                let name = name()?;
                Box::pin(reconnect(app, &name, body["fresh"].as_bool().unwrap_or(true))).await?;
                get(app, &name)
            }
            // `lorca mcp sign-in`: the browser opens on this Runner. The apps sign in through
            // `plugins.connect`, which opens it on the Device that asked.
            "mcp.sign_in" => {
                let name = name()?;
                let id = {
                    let store = app.plugins.lock().unwrap();
                    let server = store.mcp.server(&name).ok_or_else(|| format!("No server named {name} in mcp.json."))?;
                    plugin_of(&store, server).map(|plugin| plugin.manifest.id.clone()).ok_or_else(|| server.problem().unwrap_or_else(|| format!("{name} is turned off.")))?
                };
                let started = super::super::mcp::connect_oauth(app, &id, SERVER, None).await?;
                // `wait` answers once the sign-in has ended, as `lorca mcp sign-in` waits.
                if let (true, Some(done)) = (body["wait"] == true, started.done) {
                    let deadline = super::super::sign_in::TIMEOUT + std::time::Duration::from_secs(30);
                    match tokio::time::timeout(deadline, done).await {
                        Ok(Ok(Ok(()))) => {}
                        Ok(Ok(Err(error))) => return Err(error),
                        _ => return Err(format!("The {name} sign-in did not finish.")),
                    }
                    return get(app, &name);
                }
                Ok(json!({ "message": started.message }))
            }
            // Forgets the sign-in, even of a server that is off. Nothing is revoked at the server.
            "mcp.sign_out" => {
                let name = name()?;
                let id = app.plugins.lock().unwrap().mcp.server(&name).map(|server| server.id.clone()).ok_or_else(|| format!("No server named {name} in mcp.json."))?;
                super::super::sign_out(app, &id, Some(SERVER))?;
                get(app, &name)
            }
            // The file as it is now, after an edit made outside Lorca.
            "mcp.reload" => {
                reload(app);
                Ok(list(app))
            }
            other => Err(format!("Unknown request {other}")),
        }
    }

    /// Reads the file again and takes what changed.
    pub fn reload(app: &Arc<App>) {
        let file = McpFile::read(&path(app));
        let changes = app.plugins.lock().unwrap().take_mcp(file);
        if changes.any {
            tracing::info!(changed = changes.changed.len(), gone = changes.gone.len(), "read mcp.json again");
        }
        settle(app, changes);
    }

    /// At `lorca serve`'s start: a server added while Lorca was not running connects once to list
    /// its tools. An edit to the file made outside Lorca waits for `mcp.reload`.
    pub fn start(app: &Arc<App>) {
        SERVING.store(true, Ordering::Relaxed);
        let ids: Vec<String> = app.plugins.lock().unwrap().installed().iter().filter(|p| p.source == SOURCE).map(|p| p.manifest.id.clone()).collect();
        for id in ids {
            super::super::mcp::prefetch_tools(app, &id);
        }
    }
}

/// Runs an `mcp.*` verb on `runner_id`: here when it is this Device or none is named, else as a
/// request sealed to that Runner, so the apps manage another computer's servers.
pub async fn on_runner(app: &std::sync::Arc<crate::app::App>, runner_id: Option<&str>, verb: &str, body: Value) -> Result<Value, String> {
    if let Some(runner_id) = runner_id.filter(|id| app.this_device_id().as_deref() != Some(*id)) {
        let wait = if verb == "mcp.reconnect" { RECONNECT_WAIT } else { crate::requests::REQUEST_TIMEOUT };
        return crate::requests::ask_within(app, runner_id, verb, body, wait).await;
    }
    #[cfg(feature = "runner")]
    {
        serve_request(app, verb, &body).await
    }
    #[cfg(not(feature = "runner"))]
    {
        let _ = (verb, body);
        Err("This Device does not run bots, so it has no MCP servers.".into())
    }
}

/// `mcp.parse`: the servers pasted JSON describes, for the apps' JSON field and `lorca mcp`.
pub fn parse_reply(text: &str) -> Result<Value, String> {
    let servers = parse_servers(text)?;
    Ok(json!({
        "servers": servers.iter().map(|(name, entry)| match entry {
            Ok(entry) => json!({ "name": name, "config": Value::from(entry), "problem": ServerConfig::read(entry).err() }),
            Err(problem) => json!({ "name": name, "config": Value::Null, "problem": problem }),
        }).collect::<Vec<_>>(),
    }))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn json(text: &str) -> Json {
        serde_json::from_str(text).unwrap()
    }

    fn text(doc: &Json) -> String {
        String::from_utf8(doc.to_pretty()).unwrap()
    }

    #[test]
    fn ordered_json_keeps_the_files_order() {
        let doc = json(r#"{"zeta": 1, "alpha": {"b": true, "a": null}, "mid": [1, "x"]}"#);
        let keys: Vec<&str> = doc.as_object().unwrap().iter().map(|(k, _)| k.as_str()).collect();
        assert_eq!(keys, ["zeta", "alpha", "mid"]);
        assert_eq!(text(&doc), "{\n  \"zeta\": 1,\n  \"alpha\": {\n    \"b\": true,\n    \"a\": null\n  },\n  \"mid\": [\n    1,\n    \"x\"\n  ]\n}\n");
        let twice = json(r#"{"a": 1, "b": 2, "a": 3}"#);
        assert_eq!(Value::from(&twice), serde_json::json!({ "a": 3, "b": 2 }));
        assert_eq!(twice.as_object().unwrap()[0].0, "a", "a key given twice keeps its first place");
    }

    #[test]
    fn entries_read_in_every_apps_spelling() {
        // Claude Desktop and Cursor.
        let stdio = canonical(&json(r#"{"command": "npx", "args": ["-y", "@modelcontextprotocol/server-filesystem", "/tmp"], "env": {"DEBUG": 1}}"#)).unwrap();
        assert_eq!(Value::from(&stdio), serde_json::json!({ "command": "npx", "args": ["-y", "@modelcontextprotocol/server-filesystem", "/tmp"], "env": { "DEBUG": "1" } }));
        let config = ServerConfig::read(&stdio).unwrap();
        assert!(config.enabled && !config.is_remote());
        assert_eq!(config.summary(), "Local MCP server · npx");
        // Windsurf's serverUrl gains a type, first.
        let remote = canonical(&json(r#"{"serverUrl": "https://mcp.linear.app/mcp?key=secret", "headers": {"X-Team": "eng"}}"#)).unwrap();
        assert_eq!(text(&remote), "{\n  \"type\": \"http\",\n  \"url\": \"https://mcp.linear.app/mcp?key=secret\",\n  \"headers\": {\n    \"X-Team\": \"eng\"\n  }\n}\n");
        let config = ServerConfig::read(&remote).unwrap();
        assert_eq!(config.summary(), "Remote MCP server · mcp.linear.app", "the summary never shows the URL's query or path");
        assert!(matches!(config.kind, Kind::Http { oauth: Some(_), .. }), "a remote server signs in when it asks");
        // OpenCode: a command list, environment, enabled.
        let opencode = canonical(&json(r#"{"type": "local", "command": ["uvx", "mcp-server-fetch"], "environment": {"A": "b"}, "enabled": false}"#)).unwrap();
        assert_eq!(Value::from(&opencode), serde_json::json!({ "type": "stdio", "command": "uvx", "args": ["mcp-server-fetch"], "env": { "A": "b" }, "disabled": true }));
        assert!(!ServerConfig::read(&opencode).unwrap().enabled);
        // Zed's command object.
        let zed = canonical(&json(r#"{"command": {"path": "node", "args": ["server.js"], "env": {"K": "v"}}}"#)).unwrap();
        assert_eq!(Value::from(&zed), serde_json::json!({ "command": "node", "args": ["server.js"], "env": { "K": "v" } }));
        // Gemini's httpUrl, and unknown fields kept in place.
        let gemini = canonical(&json(r#"{"httpUrl": "https://x.test/mcp", "trust": true, "timeout": 5000}"#)).unwrap();
        assert_eq!(gemini.as_object().unwrap().iter().map(|(k, _)| k.as_str()).collect::<Vec<_>>(), ["type", "url", "trust", "timeout"]);
        // An entry that sends its own credentials does not sign in; "oauth": false says so too.
        let bearer = ServerConfig::read(&canonical(&json(r#"{"url": "https://x.test", "headers": {"authorization": "Bearer ${TOKEN}"}}"#)).unwrap()).unwrap();
        assert!(matches!(bearer.kind, Kind::Http { oauth: None, .. }));
        let off = ServerConfig::read(&canonical(&json(r#"{"type": "http", "url": "https://x.test", "oauth": false}"#)).unwrap()).unwrap();
        assert!(matches!(off.kind, Kind::Http { oauth: None, .. }));
        let client = ServerConfig::read(&canonical(&json(r#"{"type": "http", "url": "https://x.test", "oauth": {"clientId": "cid", "scopes": "read write"}}"#)).unwrap()).unwrap();
        let Kind::Http { oauth: Some(oauth), .. } = client.kind else { panic!("oauth") };
        assert_eq!((oauth.client_id.as_deref(), oauth.scopes), (Some("cid"), vec!["read".to_string(), "write".to_string()]));
    }

    #[test]
    fn entries_that_cannot_run_say_why() {
        let problem = |text: &str| canonical(&json(text)).and_then(|entry| ServerConfig::read(&entry)).unwrap_err();
        assert!(problem("{}").contains("command to run or a URL"));
        assert!(problem(r#"{"command": "npx", "url": "https://x.test"}"#).contains("both"));
        assert!(problem(r#"{"type": "http", "url": "ftp://x.test"}"#).contains("http://"));
        assert!(problem(r#"{"type": "pipe", "command": "x"}"#).contains("Unknown type"));
        assert!(problem(r#"{"command": "x", "args": "-y"}"#).contains("list of strings"));
        assert!(problem(r#"[1]"#).contains("JSON object"));
        // A type settles which of the two it is.
        assert!(!ServerConfig::read(&canonical(&json(r#"{"type": "stdio", "command": "x", "url": "https://x.test"}"#)).unwrap()).unwrap().is_remote());
        assert!(ServerConfig::read(&canonical(&json(r#"{"type": "sse", "url": "https://x.test/sse"}"#)).unwrap()).unwrap().is_remote());
    }

    #[test]
    fn ids_come_from_names() {
        assert_eq!(server_id("GitHub"), "github");
        assert_eq!(server_id("My Docs Server"), "my-docs-server");
        assert_eq!(server_id("日本語"), server_id("日本語"));
        assert!(server_id("日本語").starts_with("mcp-") && server_id("日本語") != server_id("中文"));
        assert_eq!(server_id(&"a".repeat(80)).len(), 48);
    }

    #[test]
    fn edits_keep_the_files_shape() {
        let file = McpFile::parse(
            br#"{
  "globalShortcut": "Ctrl+Space",
  "mcpServers": {
    "fs": { "command": "npx", "args": ["-y", "server-fs"], "alwaysAllow": ["read_file"] },
    "linear": { "url": "https://mcp.linear.app/mcp" }
  }
}"#
            .to_vec(),
        );
        assert!(file.error.is_none());
        assert_eq!(file.servers.iter().map(|s| s.name.as_str()).collect::<Vec<_>>(), ["fs", "linear"]);
        // Editing fs: its other fields and place stay, and the file's other settings too.
        let edited = canonical(&json(r#"{"command": "npx", "args": ["-y", "server-fs", "/home"], "alwaysAllow": ["read_file"], "env": {"X": "1"}}"#)).unwrap();
        let doc = with_entry(&file.doc, "fs", Some("fs"), &edited);
        let written = text(&doc);
        assert!(written.starts_with("{\n  \"globalShortcut\": \"Ctrl+Space\",\n  \"mcpServers\": {\n    \"fs\": {\n      \"command\": \"npx\",\n      \"args\": ["), "{written}");
        let fs = McpFile::parse(doc.to_pretty()).servers.remove(0).entry;
        assert_eq!(fs.as_object().unwrap().iter().map(|(k, _)| k.as_str()).collect::<Vec<_>>(), ["command", "args", "alwaysAllow", "env"]);
        // A rename keeps its place.
        let renamed = with_entry(&doc, "files", Some("fs"), &edited);
        assert_eq!(McpFile::parse(renamed.to_pretty()).servers.iter().map(|s| s.name.as_str()).collect::<Vec<_>>(), ["files", "linear"]);
        // A new one goes last, and removing one leaves the rest.
        let added = with_entry(&renamed, "deepwiki", None, &canonical(&json(r#"{"url": "https://mcp.deepwiki.com/mcp"}"#)).unwrap());
        assert_eq!(McpFile::parse(added.to_pretty()).servers.iter().map(|s| s.name.as_str()).collect::<Vec<_>>(), ["files", "linear", "deepwiki"]);
        let removed = without_entry(&added, "linear");
        assert_eq!(McpFile::parse(removed.to_pretty()).servers.iter().map(|s| s.name.as_str()).collect::<Vec<_>>(), ["files", "deepwiki"]);
        // An empty or missing file starts mcpServers; VS Code's servers stay servers.
        let fresh = with_entry(&Json::Null, "a", None, &canonical(&json(r#"{"command": "a"}"#)).unwrap());
        assert_eq!(text(&fresh), "{\n  \"mcpServers\": {\n    \"a\": {\n      \"command\": \"a\"\n    }\n  }\n}\n");
        let vscode = McpFile::parse(br#"{"servers": {"a": {"type": "stdio", "command": "a"}}, "inputs": []}"#.to_vec());
        assert_eq!(vscode.servers.len(), 1);
        let doc = with_entry(&vscode.doc, "b", None, &canonical(&json(r#"{"command": "b"}"#)).unwrap());
        assert!(doc.get("mcpServers").is_none() && doc.get("servers").unwrap().as_object().unwrap().len() == 2);
    }

    #[test]
    fn unreadable_files_say_so() {
        assert!(McpFile::parse(b"{ \"mcpServers\": { ".to_vec()).error.unwrap().contains("not valid JSON"));
        assert!(McpFile::parse(b"[]".to_vec()).error.unwrap().contains("not a JSON object"));
        assert!(McpFile::parse(br#"{"mcpServers": []}"#.to_vec()).error.unwrap().contains("not an object"));
        let empty = McpFile::parse(b"  \n".to_vec());
        assert!(empty.error.is_none() && empty.servers.is_empty());
    }

    #[test]
    fn pasted_json_in_any_shape() {
        let names = |text: &str| parse_servers(text).unwrap().into_iter().map(|(name, entry)| (name, entry.is_ok())).collect::<Vec<_>>();
        // A whole Claude Desktop file, with a comment and a trailing comma as VS Code allows.
        assert_eq!(
            names("{ // mine\n \"mcpServers\": { \"memory\": { \"command\": \"npx\", \"args\": [\"-y\", \"@modelcontextprotocol/server-memory\",], }, }, }"),
            [(Some("memory".to_string()), true)]
        );
        // One server alone, and a README's bare pair.
        assert_eq!(names(r#"{"command": "uvx", "args": ["mcp-server-time"]}"#), [(None, true)]);
        assert_eq!(names(r#""context7": {"url": "https://mcp.context7.com/mcp"}"#), [(Some("context7".to_string()), true)]);
        // Servers by name, OpenCode's mcp, Zed's context_servers.
        assert_eq!(names(r#"{"a": {"command": "a"}, "b": {"url": "https://b.test"}}"#).len(), 2);
        assert_eq!(names(r#"{"$schema": "x", "mcp": {"fetch": {"type": "local", "command": ["uvx", "f"]}}}"#), [(Some("fetch".to_string()), true)]);
        assert_eq!(names(r#"{"context_servers": {"z": {"command": {"path": "z", "args": []}}}}"#), [(Some("z".to_string()), true)]);
        // Strings that look like comments are strings.
        let (_, entry) = parse_servers(r#"{"url": "https://x.test/a//b", "headers": {"K": "/* v */"}}"#).unwrap().remove(0);
        assert_eq!(Value::from(&entry.unwrap())["headers"]["K"], "/* v */");
        assert!(parse_servers("[]").unwrap_err().contains("expected an object"));
        assert!(parse_servers(r#"{"theme": "dark"}"#).unwrap_err().contains("No MCP servers"));
        assert!(parse_servers("").is_err());
    }

    #[test]
    fn timeouts_tools_and_sign_in_settings_read_as_pi_writes_them() {
        let read = |text: &str| ServerConfig::read(&canonical(&json(text)).unwrap());
        // Seconds as pi writes them, milliseconds as Gemini CLI does.
        assert_eq!(read(r#"{"command": "x", "timeout": 90}"#).unwrap().timeout, Some(90));
        assert_eq!(read(r#"{"command": "x", "timeout": 600000}"#).unwrap().timeout, Some(600));
        assert!(read(r#"{"command": "x", "timeout": 0}"#).unwrap_err().contains("timeout"));
        assert_eq!(read(r#"{"command": "x", "timeout": 30}"#).unwrap().spec().call_timeout(), std::time::Duration::from_secs(30));
        assert_eq!(read(r#"{"command": "x"}"#).unwrap().spec().call_timeout(), super::super::CALL_TIMEOUT);

        // An exact name decides first, then the first pattern; a hidden exposure hides the rest.
        let hints = |text: &str| ToolHints { exposure: read(text).unwrap().tools, ..ToolHints::default() };
        let github = hints(r#"{"command": "x", "toolExposure": {"get_*": "codemode", "delete_*": "hidden", "delete_draft": "direct"}}"#);
        assert!(github.hides("delete_repo") && !github.hides("delete_draft") && !github.hides("get_issue") && !github.hides("search"));
        let only = hints(r#"{"command": "x", "exposure": "hidden", "toolExposure": {"search": "direct"}}"#);
        assert!(!only.hides("search") && only.hides("anything_else"));
        assert!(read(r#"{"command": "x", "toolExposure": {"a": "invisible"}}"#).unwrap_err().contains("hidden, codemode"));

        // How a remote server signs in.
        let config = read(
            r#"{"url": "https://mcp.example.com/mcp", "oauth": {"clientId": "c", "clientSecret": "${SECRET}", "callbackPort": 8765, "clientName": "Claude Code", "scope": "read write", "authServerMetadataUrl": "https://auth.example.com/.well-known/openid-configuration"}}"#,
        )
        .unwrap();
        let ServerSpec::Http { auth: Some(AuthSpec::Oauth { client_id, client_secret, callback_port, client_name, scopes, auth_server_metadata_url, optional, .. }), .. } = config.spec() else {
            panic!("an http server that signs in")
        };
        assert_eq!((client_id.as_deref(), client_secret.as_deref(), callback_port, client_name.as_deref()), (Some("c"), Some("${SECRET}"), Some(8765), Some("Claude Code")));
        assert_eq!(scopes, ["read", "write"]);
        assert!(optional && auth_server_metadata_url.as_deref() == Some("https://auth.example.com/.well-known/openid-configuration"));
        assert!(read(r#"{"url": "https://x.test/mcp", "oauth": {"callbackUrl": "https://example.com/cb"}}"#).unwrap_err().contains("callbackUrl"));
        assert!(read(r#"{"url": "https://x.test/mcp", "oauth": {"callbackUrl": "http://localhost:9000/oauth/done"}}"#).is_ok());
        assert!(read(r#"{"url": "https://x.test/mcp", "oauth": {"authServerMetadataUrl": "http://auth.example.com/meta"}}"#).unwrap_err().contains("https"));
    }

    #[cfg(feature = "runner")]
    #[test]
    fn a_tool_is_hidden_and_shown_in_the_entrys_tool_exposure() {
        let home = std::env::temp_dir().join(format!("lorca-mcp-hide-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&home).unwrap();
        std::fs::write(home.join("mcp.json"), r#"{"mcpServers": {"docs": {"command": "x", "toolExposure": {"delete_*": "hidden"}}}}"#).unwrap();
        let app = crate::app::App::load(crate::config::Config { home: home.clone(), port: 0 }).unwrap();
        let exposure = || McpFile::read(&home.join("mcp.json")).server("docs").unwrap().entry.get("toolExposure").map(Value::from).unwrap_or_default();
        hide_tool(&app, "docs", "search", true).unwrap();
        assert_eq!(exposure(), serde_json::json!({ "delete_*": "hidden", "search": "hidden" }));
        // Shown, a tool a pattern still hides is named so the name decides.
        hide_tool(&app, "docs", "delete_draft", false).unwrap();
        assert_eq!(exposure(), serde_json::json!({ "delete_*": "hidden", "search": "hidden", "delete_draft": "codemode" }));
        hide_tool(&app, "docs", "search", false).unwrap();
        assert_eq!(exposure(), serde_json::json!({ "delete_*": "hidden", "delete_draft": "codemode" }));
        let store = app.plugins.lock().unwrap();
        let hints = &store.get("docs").unwrap().manifest.tools;
        assert!(hints.hides("delete_page") && !hints.hides("delete_draft") && !hints.hides("search"));
        drop(store);
        drop(app);
        let _ = std::fs::remove_dir_all(&home);
    }

    #[test]
    fn showing_or_hiding_a_tool_keeps_the_connection() {
        let mut store = Store::default();
        let file = |extra: &str| McpFile::parse(format!(r#"{{"mcpServers": {{"docs": {{"command": "x"{extra}}}}}}}"#).into_bytes());
        assert_eq!(store.take_mcp(file("")).changed, ["docs"]);
        let hidden = store.take_mcp(file(r#", "toolExposure": {"delete_*": "hidden"}"#));
        assert!(hidden.any && hidden.changed.is_empty(), "the tools a server shows are read as they are listed");
        assert!(store.get("docs").unwrap().manifest.tools.hides("delete_page"));
        assert_eq!(store.take_mcp(file(r#", "timeout": 5"#)).changed, ["docs"], "anything else changes the server");
    }

    #[test]
    fn the_store_takes_the_files_servers() {
        let mut store = Store::default();
        let file = McpFile::parse(
            br#"{"mcpServers": {
                "Docs": {"url": "https://docs.test/mcp"},
                "docs": {"command": "x"},
                "off": {"command": "y", "disabled": true},
                "broken": {"args": []}
            }}"#
            .to_vec(),
        );
        let changes = store.take_mcp(file);
        assert_eq!(changes.changed, ["docs"]);
        assert!(changes.any);
        let ids: Vec<&str> = store.installed().iter().map(|p| p.manifest.id.as_str()).collect();
        assert_eq!(ids, ["docs"], "only a server that runs and is on is a plugin");
        let plugin = store.get("docs").unwrap();
        assert_eq!((plugin.source.as_str(), plugin.manifest.name.as_str(), plugin.manifest.icon.as_str()), (SOURCE, "Docs", "globe"));
        assert!(store.mcp.server("docs").unwrap().clash.as_deref().unwrap().contains("Another server"));
        assert!(store.mcp.server("broken").unwrap().problem().unwrap().contains("command to run"));
        assert!(!store.mcp.server("off").unwrap().is_enabled());
        // The same file again changes nothing; an unreadable one keeps the servers.
        let again = McpFile::parse(store.mcp.bytes.clone().unwrap());
        assert_eq!(store.take_mcp(again), Changes::default());
        let broken = store.take_mcp(McpFile::parse(b"{".to_vec()));
        assert!(broken.any && broken.changed.is_empty() && broken.gone.is_empty());
        assert_eq!(store.installed().len(), 1);
        assert!(store.mcp.error.is_some());
        // Emptied, the server goes.
        let gone = store.take_mcp(McpFile::parse(b"{}".to_vec()));
        assert_eq!(gone.gone, ["docs"]);
        assert!(store.installed().is_empty() && store.mcp.error.is_none());
    }
}
