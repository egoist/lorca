//! Secrets a bot asks the user for: a password, an API key, or a one-time code, asked on a card in
//! the chat (`request_secret`) rather than in a message. The value is sealed to the bot's Runner
//! from whichever Device answers, kept there in `secrets.enc` (encrypted with the account key),
//! and never enters the transcript, the synced chat, or the model's context. The bot uses it by
//! name, where the card said it would go:
//!
//! - `browser`: `{{secret:NAME}}` in what Browser types (`browser_type`, `browser_fill_form`),
//!   filled in right before the call reaches the browser, and only while its page is on the
//!   secret's site, over https.
//! - `command`: `secrets: ["NAME"]` on a `bash` call, which sets `$NAME` for that command alone.
//! - `plugin`: a setting of an installed plugin, written to the plugin's own secrets.
//!
//! Every tool result, and every command card, has the Runner's saved values replaced by their
//! placeholders before the model, the chat, or another Device sees it. The user lists, replaces,
//! and deletes a Runner's secrets from any Device (`secrets.*`).

use std::collections::BTreeMap;
use std::ffi::OsString;
use std::sync::{Arc, Mutex, MutexGuard};

use async_trait::async_trait;
use lorca_agent::agent_loop::ToolExecutionMode;
use lorca_agent::{AfterToolCallResult, ContentPart, Tool, ToolError, ToolResult, ToolUpdateFn};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use tokio_util::sync::CancellationToken;

use crate::app::App;
use crate::config::now_secs;
use crate::model::{Author, Body, Bot, Message, SecretAsk, SecretField};
use crate::plugins::mcp::{self, Decision};

/// Typed into a sign-in page of the secret's site in the bot's Browser.
pub const BROWSER: &str = "browser";
/// An environment variable of the bot's commands that name it.
pub const COMMAND: &str = "command";
/// A setting of an installed plugin.
pub const PLUGIN: &str = "plugin";

/// The most values one card asks for.
const MAX_FIELDS: usize = 5;
/// A value has at least this many characters, so it can be told apart wherever a tool echoes
/// it, and at most `MAX_VALUE_CHARS`: a password, a key, or a code, not a file.
const MIN_VALUE_CHARS: usize = 4;
const MAX_VALUE_CHARS: usize = 8 * 1024;
const FILE: &str = "secrets.enc";
const AAD: &str = "secrets";
/// Variables a command secret may not replace: what the shell and Lorca run by.
const RESERVED: &[&str] = &["PATH", "HOME", "SHELL", "USER", "LOGNAME", "PWD", "TERM", "PAGER", "GIT_PAGER", "TMPDIR", "LANG"];

/// A secret kept on this Runner.
#[derive(Clone, Serialize, Deserialize)]
pub struct Secret {
    pub id: String,
    /// The bot that asked for it and alone may use it.
    pub bot_id: String,
    pub name: String,
    pub label: String,
    #[serde(rename = "use")]
    pub target: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub site: Option<String>,
    pub value: String,
    pub updated_at: f64,
}

impl std::fmt::Debug for Secret {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Secret").field("id", &self.id).field("bot_id", &self.bot_id).field("name", &self.name).finish_non_exhaustive()
    }
}

/// A secret as the apps list it: everything but its value.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct SecretInfo {
    pub id: String,
    pub bot_id: String,
    pub name: String,
    pub label: String,
    #[serde(rename = "use")]
    pub target: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub site: Option<String>,
    pub updated_at: f64,
}

impl Secret {
    fn info(&self) -> SecretInfo {
        SecretInfo {
            id: self.id.clone(),
            bot_id: self.bot_id.clone(),
            name: self.name.clone(),
            label: self.label.clone(),
            target: self.target.clone(),
            site: self.site.clone(),
            updated_at: self.updated_at,
        }
    }
}

/// This Runner's secrets, read from `secrets.enc` once per account.
#[derive(Default)]
pub struct Store {
    /// The account `items` were read for: a Device paired again starts over.
    account: Mutex<Option<String>>,
    items: Mutex<Vec<Secret>>,
}

impl Store {
    /// Forgetting the identity: the file goes with the account, and so does what was read.
    pub fn reset(&self) {
        let mut items = self.items.lock().unwrap();
        items.clear();
        *self.account.lock().unwrap() = None;
    }
}

/// The secrets, read for the current account. Holds the store's lock: `items` before `account`.
fn loaded(app: &App) -> Result<MutexGuard<'_, Vec<Secret>>, String> {
    let account = app.this_device_id().ok_or("Pair or create an identity first.")?;
    let mut items = app.secrets.items.lock().unwrap();
    let mut read_for = app.secrets.account.lock().unwrap();
    if read_for.as_deref() != Some(account.as_str()) {
        let path = app.config.home.join(FILE);
        *items = if path.is_file() {
            let dek = app.dek().ok_or("The account key is unavailable.")?;
            crate::crypto::decrypt_json(&dek, AAD, &std::fs::read(&path).map_err(|e| e.to_string())?).map_err(|e| e.to_string())?
        } else {
            Vec::new()
        };
        *read_for = Some(account);
    }
    Ok(items)
}

/// Writes the secrets to a private file and renames it into place.
fn persist(app: &App, items: &[Secret]) -> Result<(), String> {
    let dek = app.dek().ok_or("The account key is unavailable.")?;
    let bytes = crate::crypto::encrypt_json(&dek, AAD, &items).map_err(|e| e.to_string())?;
    let path = app.config.home.join(FILE);
    let pending = path.with_extension("enc.pending");
    crate::config::write_private(&pending, &bytes).map_err(|e| e.to_string())?;
    std::fs::rename(pending, path).map_err(|e| e.to_string())
}

/// Every secret on this Runner, by bot and then by label.
pub fn list(app: &App) -> Result<Vec<SecretInfo>, String> {
    let mut infos: Vec<SecretInfo> = loaded(app)?.iter().map(Secret::info).collect();
    let name = |bot_id: &str| app.bot(bot_id).map(|bot| bot.name.to_lowercase()).unwrap_or_default();
    infos.sort_by_key(|info| (name(&info.bot_id), info.label.to_lowercase()));
    Ok(infos)
}

/// A new value for secret `id`, from the user.
pub fn replace(app: &App, id: &str, value: &str) -> Result<SecretInfo, String> {
    let mut items = loaded(app)?;
    let secret = items.iter_mut().find(|secret| secret.id == id).ok_or("This secret was deleted.")?;
    secret.value = checked_value(value, &secret.label)?;
    secret.updated_at = now_secs();
    let info = secret.info();
    persist(app, &items)?;
    Ok(info)
}

pub fn delete(app: &App, id: &str) -> Result<(), String> {
    let mut items = loaded(app)?;
    let before = items.len();
    items.retain(|secret| secret.id != id);
    if items.len() == before {
        return Ok(());
    }
    persist(app, &items)
}

/// Deleted bots' secrets go with them.
pub fn forget_bots(app: &App, bot_ids: &[String]) {
    if bot_ids.is_empty() {
        return;
    }
    let Ok(mut items) = loaded(app) else { return };
    let before = items.len();
    items.retain(|secret| !bot_ids.contains(&secret.bot_id));
    if items.len() != before {
        if let Err(error) = persist(app, &items) {
            tracing::warn!(%error, "forgetting deleted bots' secrets");
        }
    }
}

/// Keeps the values a card was answered with, replacing the bot's secrets of the same names.
fn keep(app: &App, bot_id: &str, ask: &SecretAsk, values: &BTreeMap<String, String>) -> Result<(), String> {
    let mut items = loaded(app)?;
    let now = now_secs();
    for field in &ask.fields {
        let Some(value) = values.get(&field.name) else { continue };
        match items.iter_mut().find(|secret| secret.bot_id == bot_id && secret.name == field.name) {
            Some(secret) => {
                secret.label = field.label.clone();
                secret.target = ask.target.clone();
                secret.site = ask.site.clone();
                secret.value = value.clone();
                secret.updated_at = now;
            }
            None => items.push(Secret {
                id: format!("secret-{}", uuid::Uuid::new_v4()),
                bot_id: bot_id.to_string(),
                name: field.name.clone(),
                label: field.label.clone(),
                target: ask.target.clone(),
                site: ask.site.clone(),
                value: value.clone(),
                updated_at: now,
            }),
        }
    }
    persist(app, &items)
}

/// What the user typed, trimmed, as a value worth keeping.
fn checked_value(value: &str, label: &str) -> Result<String, String> {
    let value = value.trim();
    if value.is_empty() {
        return Err(format!("Enter {label}."));
    }
    if value.chars().count() < MIN_VALUE_CHARS {
        return Err(format!("{label} is too short to keep as a secret."));
    }
    if value.chars().count() > MAX_VALUE_CHARS {
        return Err(format!("{label} is too long."));
    }
    Ok(value.to_string())
}

/// A name the bot refers to a secret by: letters, digits, and `_`, not starting with a digit.
fn valid_name(name: &str) -> bool {
    let mut chars = name.chars();
    chars.next().is_some_and(|c| c.is_ascii_alphabetic() || c == '_') && chars.all(|c| c.is_ascii_alphanumeric() || c == '_') && name.len() <= 64
}

/// How the bot writes a secret into what Browser types.
pub fn placeholder(name: &str) -> String {
    format!("{{{{secret:{name}}}}}")
}

/// The names `text` refers to as `{{secret:NAME}}`.
fn placeholders(text: &str) -> Vec<String> {
    let mut names = Vec::new();
    let mut rest = text;
    while let Some(start) = rest.find("{{secret:") {
        let after = &rest[start + "{{secret:".len()..];
        match after.find("}}") {
            Some(end) => {
                names.push(after[..end].trim().to_string());
                rest = &after[end + 2..];
            }
            None => break,
        }
    }
    names
}

/// The host a site is named by: `https://www.github.com/login` and `github.com` are `github.com`.
pub fn site_of(input: &str) -> Option<String> {
    let input = input.trim().to_ascii_lowercase();
    let url = if input.contains("://") { input } else { format!("https://{input}") };
    let host = reqwest::Url::parse(&url).ok()?.host_str()?.to_string();
    let host = host.strip_prefix("www.").unwrap_or(&host).to_string();
    (!host.is_empty()).then_some(host)
}

/// A page the secret of `site` may be typed into: on the site or one of its subdomains, over
/// https, or plain http on this computer.
fn page_is_on(site: &str, page: &str) -> bool {
    let Ok(url) = reqwest::Url::parse(page) else { return false };
    let Some(host) = url.host_str().map(str::to_ascii_lowercase) else { return false };
    let local = matches!(host.as_str(), "localhost" | "127.0.0.1" | "[::1]");
    let secure = url.scheme() == "https" || (url.scheme() == "http" && local);
    secure && (host == site || host.ends_with(&format!(".{site}")))
}

/// The address of the tab Browser has current, from what `browser_tabs` lists:
/// `- 0: (current) [Sign in · GitHub](https://github.com/login)`.
pub fn current_page(tabs: &str) -> Option<String> {
    let line = tabs.lines().find(|line| line.contains(": (current) ["))?;
    let start = line.rfind("](")? + 2;
    let rest = &line[start..];
    Some(rest[..rest.rfind(')')?].to_string())
}

/// Where Browser's input tools take their text, the only places a secret goes.
fn browser_slots<'a>(tool: &str, args: &'a mut Value) -> Vec<&'a mut Value> {
    match tool {
        "browser_type" => args.get_mut("text").into_iter().collect(),
        "browser_fill_form" => args.get_mut("fields").and_then(Value::as_array_mut).map(|fields| fields.iter_mut().filter_map(|field| field.get_mut("value")).collect()).unwrap_or_default(),
        _ => Vec::new(),
    }
}

/// Every string in `value`, for finding placeholders.
fn strings(value: &Value, out: &mut Vec<String>) {
    match value {
        Value::String(text) => out.push(text.clone()),
        Value::Array(items) => items.iter().for_each(|item| strings(item, out)),
        Value::Object(fields) => fields.values().for_each(|item| strings(item, out)),
        _ => {}
    }
}

/// A plugin call's arguments with the secrets they name filled in, right before the call goes
/// to the server. Only Browser's input tools take one, and only while the page `page` reads out
/// is on each secret's site. A call that names none goes as it is.
pub async fn fill_call<F>(app: &App, bot: Option<&Bot>, plugin_id: &str, tool: &str, mut args: Value, page: F) -> Result<Value, String>
where
    F: std::future::Future<Output = Result<String, String>>,
{
    let mut all = Vec::new();
    strings(&args, &mut all);
    if !all.iter().any(|text| !placeholders(text).is_empty()) {
        return Ok(args);
    }
    let refuse = || "Saved secrets go only into what Browser types on the secret's own site (browser_type's text, browser_fill_form's values). For a command, name it in bash's secrets instead.".to_string();
    if plugin_id != crate::browser::PLUGIN_ID {
        return Err(refuse());
    }
    let mut outside = args.clone();
    for slot in browser_slots(tool, &mut outside) {
        *slot = Value::Null;
    }
    let mut left = Vec::new();
    strings(&outside, &mut left);
    if left.iter().any(|text| !placeholders(text).is_empty()) {
        return Err(refuse());
    }
    let bot = bot.ok_or_else(refuse)?;
    let secrets: Vec<Secret> = {
        let items = loaded(app)?;
        let mut wanted = Vec::new();
        for text in &all {
            for name in placeholders(text) {
                if wanted.iter().any(|secret: &Secret| secret.name == name) {
                    continue;
                }
                let secret = items
                    .iter()
                    .find(|secret| secret.bot_id == bot.id && secret.name == name && secret.target == BROWSER)
                    .ok_or_else(|| format!("No secret {name} is saved for your Browser. Ask the user for it with request_secret (use: browser)."))?;
                wanted.push(secret.clone());
            }
        }
        wanted
    };
    let page = page.await?;
    for secret in &secrets {
        let site = secret.site.as_deref().unwrap_or_default();
        if !page_is_on(site, &page) {
            return Err(format!("{} is saved for {site}, and the page open now is {page}. Open {site}'s sign-in page first.", secret.name));
        }
    }
    for slot in browser_slots(tool, &mut args) {
        if let Value::String(text) = slot {
            for secret in &secrets {
                *text = text.replace(&placeholder(&secret.name), &secret.value);
            }
        }
    }
    Ok(args)
}

/// The saved values on this Runner and their placeholders, in the spellings a tool may echo a
/// value in: as typed, escaped in a JSON or a JavaScript string (Playwright writes the code it
/// ran), or percent-encoded in a URL. Longest first, so one never cuts into another.
pub struct Redactions(Vec<(String, String)>);

impl Redactions {
    pub fn load(app: &App) -> Redactions {
        let Ok(items) = loaded(app) else { return Redactions(Vec::new()) };
        let mut pairs: Vec<(String, String)> = items.iter().flat_map(|secret| spellings(&secret.value).into_iter().map(|spelling| (spelling, placeholder(&secret.name)))).collect();
        pairs.sort_by(|a, b| b.0.len().cmp(&a.0.len()).then_with(|| a.0.cmp(&b.0)));
        pairs.dedup_by(|a, b| a.0 == b.0);
        Redactions(pairs)
    }

    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }

    /// `text` with each value replaced, or None when it held none.
    pub fn text(&self, text: &str) -> Option<String> {
        if !self.0.iter().any(|(value, _)| text.contains(value.as_str())) {
            return None;
        }
        let mut out = text.to_string();
        for (value, placeholder) in &self.0 {
            out = out.replace(value.as_str(), placeholder);
        }
        Some(out)
    }

    /// Replaces each value in every string of `value`; true when one held any.
    pub fn json(&self, value: &mut Value) -> bool {
        match value {
            Value::String(text) => match self.text(text) {
                Some(clean) => {
                    *text = clean;
                    true
                }
                None => false,
            },
            Value::Array(items) => items.iter_mut().fold(false, |changed, item| self.json(item) | changed),
            Value::Object(fields) => fields.values_mut().fold(false, |changed, item| self.json(item) | changed),
            _ => false,
        }
    }

    pub fn option(&self, text: &mut Option<String>) {
        if let Some(clean) = text.as_deref().and_then(|text| self.text(text)) {
            *text = Some(clean);
        }
    }
}

fn spellings(value: &str) -> Vec<String> {
    let mut out = vec![value.to_string()];
    if let Ok(quoted) = serde_json::to_string(value) {
        out.push(quoted[1..quoted.len() - 1].to_string());
    }
    out.push(value.replace('\\', "\\\\").replace('\'', "\\'").replace('\n', "\\n"));
    out.push(value.bytes().map(|byte| if byte.is_ascii_alphanumeric() || b"-_.~".contains(&byte) { (byte as char).to_string() } else { format!("%{byte:02X}") }).collect());
    out.retain(|spelling| spelling.chars().count() >= MIN_VALUE_CHARS);
    out.sort();
    out.dedup();
    out
}

/// A tool's result with this Runner's saved values replaced by their placeholders, before the
/// loop hands it to the model, the chat row, or a script. None when it held none.
pub fn scrub_result(app: &App, result: &ToolResult) -> Option<AfterToolCallResult> {
    let redactions = Redactions::load(app);
    if redactions.is_empty() {
        return None;
    }
    let mut changed = false;
    let content = result
        .content
        .iter()
        .map(|part| match part.as_text().and_then(|text| redactions.text(text)) {
            Some(clean) => {
                changed = true;
                ContentPart::text(clean)
            }
            None => part.clone(),
        })
        .collect();
    let mut details = result.details.clone();
    changed |= redactions.json(&mut details);
    let mut structured = result.structured.clone();
    if let Some(structured) = &mut structured {
        changed |= redactions.json(structured);
    }
    changed.then(|| AfterToolCallResult { content: Some(content), details: Some(details), structured, ..AfterToolCallResult::default() })
}

/// A bot's command secrets, for the `secrets` its `bash` calls name.
pub struct CommandSecrets {
    pub app: Arc<App>,
    pub bot_id: String,
}

impl lorca_agent::tools::SecretVariables for CommandSecrets {
    fn variables(&self, names: &[String]) -> Result<Vec<(OsString, OsString)>, String> {
        let items = loaded(&self.app)?;
        names
            .iter()
            .map(|name| {
                let secret = items
                    .iter()
                    .find(|secret| secret.bot_id == self.bot_id && &secret.name == name && secret.target == COMMAND)
                    .ok_or_else(|| format!("No secret {name} is saved for your commands. Ask the user for it with request_secret (use: command)."))?;
                Ok((OsString::from(name), OsString::from(&secret.value)))
            })
            .collect()
    }
}

/// The names a `bash` call asks for in `secrets`.
pub fn named_in(args: &Value) -> Vec<String> {
    args.get("secrets").and_then(Value::as_array).map(|names| names.iter().filter_map(Value::as_str).map(str::to_string).collect()).unwrap_or_default()
}

/// The part of the system prompt that names the bot's saved secrets, when it has any.
pub fn prompt(app: &App, bot_id: &str) -> String {
    let Ok(items) = loaded(app) else { return String::new() };
    let mine: Vec<String> = items
        .iter()
        .filter(|secret| secret.bot_id == bot_id)
        .map(|secret| match (secret.target.as_str(), &secret.site) {
            (BROWSER, Some(site)) => format!("{} (Browser on {site})", secret.name),
            _ => format!("{} (commands)", secret.name),
        })
        .collect();
    if mine.is_empty() {
        return String::new();
    }
    format!("Secrets the user saved for you, which you use by name and never see: {}.\n", mine.join(", "))
}

/// The user answered secret request `message` with `values`, by field name. They are kept, or
/// written to the plugin's settings, before the waiting call hears it: so a card nothing waits
/// on any more keeps nothing.
pub fn answer(app: &Arc<App>, message: &Message, values: Option<&serde_json::Map<String, Value>>) -> Result<(), String> {
    let (Body::Permission { secret: Some(ask), plugin_id, .. }, Author::Bot { bot_id }) = (&message.body, &message.author) else {
        return Err("That is not a secret request.".into());
    };
    if !app.pending_permissions.lock().unwrap().contains_key(&message.id) {
        return Err("This request is no longer waiting for an answer.".into());
    }
    let values = values.ok_or("Enter a value for each field.")?;
    let mut checked = BTreeMap::new();
    for field in &ask.fields {
        let value = values.get(&field.name).and_then(Value::as_str).unwrap_or_default();
        checked.insert(field.name.clone(), checked_value(value, &field.label)?);
    }
    match ask.target.as_str() {
        PLUGIN => crate::plugins::set_variables(app, plugin_id, &checked).map(|_| ()),
        _ => keep(app, bot_id, ask, &checked),
    }
}

/// `secrets.list`, `secrets.set`, and `secrets.delete` on this Runner, from the local app or a
/// sealed request.
pub fn serve(app: &Arc<App>, verb: &str, body: &Value) -> Result<Value, String> {
    let id = || body["id"].as_str().ok_or("missing id");
    match verb {
        "secrets.list" => Ok(json!({ "secrets": list(app)? })),
        "secrets.set" => Ok(json!({ "secret": replace(app, id()?, body["value"].as_str().ok_or("missing value")?)? })),
        "secrets.delete" => delete(app, id()?).map(|()| json!({})),
        other => Err(format!("Unknown request {other}")),
    }
}

const DESCRIPTION: &str = "Ask the user for a password, an API key, or a one-time code on a card in this chat. Never ask for one in a message. \
     What they enter goes encrypted to your Runner and is kept there; you never see it and it never enters the chat. Say where it goes in `use`: \
     `browser` for a sign-in page of `site` in your Browser, where you type it as {{secret:NAME}} with browser_type or browser_fill_form while \
     that site is open; `command` for the commands that need it, which get it as the environment variable $NAME when bash names it in \
     `secrets`; `plugin` for a setting of an installed plugin, where NAME is the setting. Asking again for a name replaces it. Waits for \
     the answer.";

/// `request_secret`: the bot asks for one or more secrets on a card, and its call waits for the
/// answer as a permission card's does. A turn nobody watches cannot ask.
pub struct RequestSecret {
    pub app: Arc<App>,
    pub chat_id: String,
    pub bot: Bot,
    pub unattended: bool,
}

#[async_trait]
impl Tool for RequestSecret {
    fn name(&self) -> &str {
        "request_secret"
    }
    fn description(&self) -> &str {
        DESCRIPTION
    }
    fn parameters(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "use": { "type": "string", "enum": [BROWSER, COMMAND, PLUGIN], "description": "Where it goes: browser, command, or plugin" },
                "why": { "type": "string", "description": "Why you need it, one sentence in the user's language, shown on the card" },
                "site": { "type": "string", "description": "For browser: the site whose sign-in page gets it, such as github.com" },
                "plugin": { "type": "string", "description": "For plugin: the installed plugin's id or name" },
                "secrets": {
                    "type": "array",
                    "minItems": 1,
                    "maxItems": MAX_FIELDS,
                    "items": {
                        "type": "object",
                        "properties": {
                            "name": { "type": "string", "description": "How you refer to it: letters, digits, and _. For command, the environment variable (GITHUB_TOKEN); for plugin, the setting's name" },
                            "label": { "type": "string", "description": "What the card calls it, in the user's language: \"GitHub password\"" }
                        },
                        "required": ["name", "label"],
                        "additionalProperties": false
                    }
                }
            },
            "required": ["use", "why", "secrets"],
            "additionalProperties": false
        })
    }
    fn execution_mode(&self) -> Option<ToolExecutionMode> {
        Some(ToolExecutionMode::Sequential)
    }
    async fn execute(&self, _id: &str, args: Value, cancel: CancellationToken, _on_update: ToolUpdateFn) -> Result<ToolResult, ToolError> {
        if self.unattended {
            return Err("Nobody is here to answer a secret request now. Ask in a chat with the user.".into());
        }
        if self.app.this_device_id().as_deref() != Some(self.bot.runner_id.as_str()) {
            return Err("Secrets are kept on your Runner, which is not this Device.".into());
        }
        let request = self.request(&args).map_err(ToolError)?;
        let message = Message::new(
            &self.chat_id,
            Author::Bot { bot_id: self.bot.id.clone() },
            Body::Permission {
                plugin_id: request.plugin_id.clone(),
                plugin_name: request.plugin_name.clone(),
                tool: "secret".into(),
                summary: request.ask.fields.iter().map(|field| field.label.as_str()).collect::<Vec<_>>().join(", "),
                arguments: args.clone(),
                decision: "pending".into(),
                reason: Some(request.why.clone()),
                rule: None,
                command: None,
                link: None,
                code: None,
                secret: Some(request.ask.clone()),
            },
        );
        let decision = mcp::await_answer(&self.app, &self.chat_id, &message.id, None, &cancel, || {
            self.app.upsert_message(message.clone(), true);
            crate::push::permission(&self.app, &message);
        })
        .await;
        if let Some(mut card) = self.app.message(&self.chat_id, &message.id) {
            if let Body::Permission { decision: shown, .. } = &mut card.body {
                *shown = match decision {
                    Decision::Always => "allowed",
                    Decision::Denied if cancel.is_cancelled() => "dismissed",
                    decision => decision.as_str(),
                }
                .into();
            }
            self.app.upsert_message(card, true);
        }
        let names = request.ask.fields.iter().map(|field| field.name.as_str()).collect::<Vec<_>>().join(", ");
        let labels = message_labels(&request.ask.fields);
        match decision {
            Decision::Allowed | Decision::Always => {
                let how = match request.ask.target.as_str() {
                    BROWSER => {
                        let site = request.ask.site.as_deref().unwrap_or_default();
                        format!("With {site}'s sign-in page open in your Browser, type each as {} with browser_type or browser_fill_form; Lorca fills it in there and nowhere else.", placeholder("NAME"))
                    }
                    COMMAND => "Name them in bash's secrets for the commands that need them, which read each as $NAME. Never print them.".into(),
                    _ => {
                        let status = self.app.plugins.lock().unwrap().status(&request.plugin_id).map(|status| status.detail).unwrap_or_default();
                        format!("{} has them now ({status}).", request.plugin_name)
                    }
                };
                Ok(ToolResult::text(format!("The user saved {names}. {how}")).with_details(json!({ "summary": format!("Saved {labels}") })))
            }
            // Stop ends the wait too: the user stopped the turn, they did not refuse.
            Decision::Denied if cancel.is_cancelled() => Err(ToolError(format!("Stopped before the user answered, so {labels} was not saved."))),
            Decision::Denied => Err(ToolError(format!("The user did not give {labels} now. Do not ask again unless they bring it up."))),
            Decision::Expired => Err("Nobody answered the secret request in time. Say what you needed and stop.".into()),
            Decision::Dismissed => Ok(mcp::dismissed_call(format!("The user sent a new message instead of answering, so {labels} was not saved. Follow that message."))),
        }
    }
}

fn message_labels(fields: &[SecretField]) -> String {
    fields.iter().map(|field| field.label.as_str()).collect::<Vec<_>>().join(", ")
}

/// A secret request the card can show: what it asks for, where it goes, and why.
struct Request {
    ask: SecretAsk,
    why: String,
    plugin_id: String,
    plugin_name: String,
}

impl RequestSecret {
    fn request(&self, args: &Value) -> Result<Request, String> {
        let why = args["why"].as_str().map(str::trim).filter(|why| !why.is_empty()).ok_or("why is required: say why you need it")?;
        let fields: Vec<SecretField> = args["secrets"]
            .as_array()
            .ok_or("secrets is required")?
            .iter()
            .map(|field| {
                let name = field["name"].as_str().unwrap_or_default().trim().to_string();
                let label = field["label"].as_str().unwrap_or_default().trim().to_string();
                if !valid_name(&name) {
                    return Err(format!("{name:?} is not a name: use letters, digits, and _, starting with a letter."));
                }
                if label.is_empty() {
                    return Err(format!("{name} needs a label."));
                }
                Ok(SecretField { name, label: label.chars().take(80).collect() })
            })
            .collect::<Result<_, _>>()?;
        if fields.is_empty() || fields.len() > MAX_FIELDS {
            return Err(format!("Ask for one to {MAX_FIELDS} secrets at a time."));
        }
        if fields.iter().enumerate().any(|(i, field)| fields[..i].iter().any(|other| other.name == field.name)) {
            return Err("Each secret needs a name of its own.".into());
        }
        let target = args["use"].as_str().unwrap_or_default();
        let runner = self.app.device(&self.bot.runner_id).map(|device| device.name).unwrap_or_else(|| "this Runner".into());
        let (site, plugin_id, plugin_name) = match target {
            BROWSER => {
                let site = args["site"].as_str().and_then(site_of).ok_or("site is required for browser: the site whose sign-in page gets it")?;
                if self.app.plugins.lock().unwrap().get(crate::browser::PLUGIN_ID).is_none() {
                    return Err("Browser is not installed on your Runner.".into());
                }
                crate::permissions::check_plugin(&self.app, &self.bot, crate::browser::PLUGIN_ID, "browser_type").map_err(|denied| denied.to_string())?;
                (Some(site), crate::browser::PLUGIN_ID.to_string(), "Browser".to_string())
            }
            COMMAND => {
                crate::permissions::check_tool(&self.app, &self.bot, "bash").map_err(|denied| denied.to_string())?;
                if let Some(field) = fields.iter().find(|field| RESERVED.contains(&field.name.as_str()) || field.name.starts_with("LORCA_") || field.name.starts_with("DYLD_") || field.name.starts_with("LD_")) {
                    return Err(format!("{} is a variable the shell runs by; name the secret something else.", field.name));
                }
                (None, "computer".to_string(), runner)
            }
            PLUGIN => {
                let wanted = args["plugin"].as_str().unwrap_or_default().trim().to_lowercase();
                let (id, name, variables) = {
                    let store = self.app.plugins.lock().unwrap();
                    let status = store.statuses().into_iter().find(|status| status.id.to_lowercase() == wanted || status.name.to_lowercase() == wanted).ok_or_else(|| format!("No plugin {wanted:?} is installed on {runner}."))?;
                    let variables: Vec<String> = store.get(&status.id).map(|plugin| plugin.manifest.variables.iter().map(|variable| variable.name.clone()).collect()).unwrap_or_default();
                    (status.id, status.name, variables)
                };
                if !self.app.bot(&self.bot.id).and_then(|bot| bot.permissions).is_none_or(|policy| policy.allows_connection(&id)) {
                    return Err(format!("{name} is off for you in your Access settings, which only the user changes."));
                }
                if let Some(field) = fields.iter().find(|field| !variables.contains(&field.name)) {
                    return Err(if variables.is_empty() { format!("{name} has no settings to fill.") } else { format!("{name} has no setting {}; its settings are {}.", field.name, variables.join(", ")) });
                }
                (None, id, name)
            }
            _ => return Err("use is browser, command, or plugin".into()),
        };
        Ok(Request { ask: SecretAsk { target: target.to_string(), site, fields }, why: why.chars().take(300).collect(), plugin_id, plugin_name })
    }
}

#[cfg(test)]
mod tests;
