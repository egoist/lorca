use std::collections::BTreeMap;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};

use async_trait::async_trait;
use lorca_agent::{BeforeToolCallContext, BeforeToolCallResult, Tool, ToolError, ToolResult, ToolUpdateFn};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use tokio::sync::{Mutex as AsyncMutex, OwnedMutexGuard};
use tokio_util::sync::CancellationToken;

use crate::app::App;
use crate::model::{Bot, Message};
use crate::plugins::mcp::Server;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Control {
    Stopped,
    Bot,
    TakingOver,
    Human,
}

/// One browser profile of a bot. `revision` goes up with every change of control, so a Return
/// to Bot made from an old view cannot hand back a newer takeover.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Session {
    pub id: String,
    pub bot_id: String,
    pub runner_id: String,
    pub name: String,
    pub state: Control,
    pub selected: bool,
    pub revision: u64,
    pub created_at: f64,
}

struct Runtime {
    meta: Mutex<Session>,
    /// Held by a Browser call until its server answers, and by a takeover while it waits.
    input: Arc<AsyncMutex<()>>,
    server: Mutex<Option<Arc<Server>>>,
    opening: Mutex<CancellationToken>,
    stopping: AtomicUsize,
}

impl Runtime {
    fn new(meta: Session) -> Arc<Self> {
        Arc::new(Runtime { meta: Mutex::new(meta), input: Arc::new(AsyncMutex::new(())), server: Mutex::new(None), opening: Mutex::new(CancellationToken::new()), stopping: AtomicUsize::new(0) })
    }

    fn open_server(&self) -> Option<Arc<Server>> {
        self.server.lock().unwrap().clone().filter(|server| !server.is_closed())
    }

    /// The browser went away under the record: it reads as stopped.
    fn mark_stopped(&self) {
        if let Some(server) = self.server.lock().unwrap().take() {
            server.stop();
        }
        let mut meta = self.meta.lock().unwrap();
        meta.state = Control::Stopped;
        meta.revision += 1;
    }
}

struct Stopping<'a>(&'a AtomicUsize);
impl Drop for Stopping<'_> {
    fn drop(&mut self) {
        self.0.fetch_sub(1, Ordering::Release);
    }
}

impl Drop for Runtime {
    fn drop(&mut self) {
        if let Some(server) = self.server.lock().unwrap().take() {
            server.stop();
        }
    }
}

#[derive(Default)]
pub struct Sessions {
    /// The account whose records are loaded.
    account: Mutex<Option<String>>,
    items: Mutex<BTreeMap<String, Arc<Runtime>>>,
    persistence: Mutex<()>,
    changed: tokio::sync::Notify,
}

/// A Browser call's hold on the bot's open profile: no takeover completes while it lives.
pub struct Input {
    _guard: OwnedMutexGuard<()>,
    runtime: Arc<Runtime>,
    pub server: Arc<Server>,
    pub name: String,
}

impl Input {
    /// A call that ran out of time never said it finished. Closing its browser keeps it from
    /// acting after the gate opens to the user.
    pub fn interrupted(&self, app: &Arc<App>) {
        self.server.stop();
        self.runtime.mark_stopped();
        let _ = app.browser_sessions.save(app);
    }
}

impl Sessions {
    pub fn local_bot(&self, app: &App, id: &str) -> Result<Bot, String> {
        let bot = app.bot(id).ok_or("Unknown bot")?;
        if app.this_device_id().as_deref() != Some(bot.runner_id.as_str()) {
            return Err("Browser profiles live on the bot's Runner.".into());
        }
        if !app.device(&bot.runner_id).is_some_and(|device| device.is_runner()) {
            return Err("Browser profiles need a Runner.".into());
        }
        Ok(bot)
    }

    fn load(&self, app: &Arc<App>) -> Result<(), String> {
        let account_key = app.this_device_id().ok_or("Pair or create an identity first.")?;
        let dek = app.dek().ok_or("The account key is unavailable.")?;
        let mut loaded = self.account.lock().unwrap();
        if loaded.as_deref() == Some(&account_key) {
            return Ok(());
        }
        let dir = app.config.home.join("browser");
        std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
        crate::config::set_private(&dir).map_err(|e| e.to_string())?;
        let path = dir.join("sessions.enc");
        let saved: Vec<Session> = if path.is_file() {
            crate::crypto::decrypt_json(&dek, "browser-sessions", &std::fs::read(path).map_err(|e| e.to_string())?).map_err(|e| e.to_string())?
        } else {
            Vec::new()
        };
        let mut items = self.items.lock().unwrap();
        items.clear();
        for mut meta in saved {
            if !valid_id(&meta.id) {
                continue;
            }
            // A restart never hands the bot a browser or opens a window.
            meta.state = Control::Stopped;
            meta.revision += 1;
            items.insert(meta.id.clone(), Runtime::new(meta));
        }
        *loaded = Some(account_key);
        Ok(())
    }

    fn save(&self, app: &Arc<App>) -> Result<(), String> {
        self.changed.notify_waiters();
        let _guard = self.persistence.lock().unwrap();
        let dek = app.dek().ok_or("The account key is unavailable.")?;
        let items: Vec<Session> = self.items.lock().unwrap().values().map(|runtime| runtime.meta.lock().unwrap().clone()).collect();
        let bytes = crate::crypto::encrypt_json(&dek, "browser-sessions", &items).map_err(|e| e.to_string())?;
        let path = app.config.home.join("browser/sessions.enc");
        let pending = path.with_extension("enc.pending");
        crate::config::write_private(&pending, &bytes).map_err(|e| e.to_string())?;
        std::fs::rename(pending, path).map_err(|e| e.to_string())
    }

    /// The bot's profiles, oldest first.
    pub fn list(&self, app: &Arc<App>, bot_id: &str) -> Result<Vec<Session>, String> {
        self.local_bot(app, bot_id)?;
        self.load(app)?;
        let mut sessions: Vec<Session> = self.items.lock().unwrap().values().map(|item| item.meta.lock().unwrap().clone()).filter(|s| s.bot_id == bot_id).collect();
        sessions.sort_by(|a, b| a.created_at.total_cmp(&b.created_at));
        Ok(sessions)
    }

    fn owned(&self, app: &Arc<App>, bot_id: &str, id: &str) -> Result<Arc<Runtime>, String> {
        let bot = self.local_bot(app, bot_id)?;
        self.load(app)?;
        let runtime = self.items.lock().unwrap().get(id).cloned().ok_or("This browser profile no longer exists.")?;
        let meta = runtime.meta.lock().unwrap();
        if meta.bot_id != bot.id || meta.runner_id != bot.runner_id {
            return Err("This browser profile belongs to another bot or Runner.".into());
        }
        drop(meta);
        Ok(runtime)
    }

    pub fn create(&self, app: &Arc<App>, bot_id: &str, name: &str) -> Result<Session, String> {
        let bot = self.local_bot(app, bot_id)?;
        self.load(app)?;
        if app.plugins.lock().unwrap().get(super::PLUGIN_ID).is_none() {
            return Err("Install the Browser plugin on this Runner first.".into());
        }
        let meta = Session {
            id: format!("browser-{}", uuid::Uuid::new_v4()),
            bot_id: bot.id.clone(),
            runner_id: bot.runner_id,
            name: label(name)?,
            state: Control::Stopped,
            selected: false,
            revision: 1,
            created_at: crate::config::now_secs(),
        };
        let runtime = Runtime::new(meta);
        self.items.lock().unwrap().insert(runtime.meta.lock().unwrap().id.clone(), runtime.clone());
        // The first profile is the one the bot opens; another becomes it once opened.
        if !self.items.lock().unwrap().values().any(|item| {
            let item = item.meta.lock().unwrap();
            item.bot_id == bot.id && item.selected
        }) {
            runtime.meta.lock().unwrap().selected = true;
        }
        self.save(app)?;
        let meta = runtime.meta.lock().unwrap().clone();
        Ok(meta)
    }

    fn select(&self, runtime: &Arc<Runtime>) {
        let selected = runtime.meta.lock().unwrap().clone();
        for item in self.items.lock().unwrap().values() {
            let mut meta = item.meta.lock().unwrap();
            if meta.bot_id == selected.bot_id && meta.selected != (meta.id == selected.id) {
                meta.selected = meta.id == selected.id;
                meta.revision += 1;
            }
        }
    }

    /// Whether the user has, or is taking, one of the bot's browsers. Only what is loaded counts:
    /// a takeover needs the records loaded first.
    fn taken_over(&self, bot_id: &str) -> bool {
        self.items.lock().unwrap().values().any(|item| {
            let meta = item.meta.lock().unwrap();
            meta.bot_id == bot_id && matches!(meta.state, Control::Human | Control::TakingOver)
        })
    }

    /// Parks a Browser call while the user has the bot's browser, with its turn and script state
    /// intact. Return to Bot, Stop, and the chat's Stop wake it.
    pub async fn wait_if_taken_over(&self, bot_id: &str, cancel: &CancellationToken) -> Result<(), String> {
        loop {
            let changed = self.changed.notified();
            if !self.taken_over(bot_id) {
                return Ok(());
            }
            tokio::select! {
                _ = changed => {},
                _ = cancel.cancelled() => return Err("Stopped".into()),
            }
        }
    }

    /// The bot's open profile for a Browser call, holding its input gate, or `None` when the bot
    /// has none open and the call goes to the Runner's shared headless browser.
    pub async fn input(&self, app: &Arc<App>, bot_id: &str, cancel: &CancellationToken) -> Result<Option<Input>, String> {
        loop {
            self.wait_if_taken_over(bot_id, cancel).await?;
            let open = self.items.lock().unwrap().values().find(|item| {
                let meta = item.meta.lock().unwrap();
                meta.bot_id == bot_id && meta.selected && meta.state == Control::Bot
            }).cloned();
            let Some(runtime) = open else { return Ok(None) };
            let guard = tokio::select! {
                guard = runtime.input.clone().lock_owned() => guard,
                _ = cancel.cancelled() => return Err("Stopped".into()),
            };
            // A takeover, Stop, or another profile opening can land while the call waits.
            let meta = runtime.meta.lock().unwrap().clone();
            if meta.state != Control::Bot || !meta.selected {
                continue;
            }
            self.owned(app, bot_id, &meta.id)?;
            let server = match runtime.open_server() {
                Some(server) if server.is_current(app) => server,
                _ => {
                    runtime.mark_stopped();
                    let _ = self.save(app);
                    return Err(format!("The {} browser closed. Open it again with browser_session.", meta.name));
                }
            };
            return Ok(Some(Input { _guard: guard, runtime, server, name: meta.name }));
        }
    }

    /// Opens a profile's browser in a window on this Runner, or brings it forward. The user's
    /// open gives the user control; the bot's gives the bot control.
    pub async fn open(&self, app: &Arc<App>, bot_id: &str, id: &str, human: bool) -> Result<Session, String> {
        let runtime = self.owned(app, bot_id, id)?;
        let requested_revision = runtime.meta.lock().unwrap().revision;
        let _guard = runtime.input.lock().await;
        self.owned(app, bot_id, id)?;
        let opening = CancellationToken::new();
        let opening_revision = {
            let meta = runtime.meta.lock().unwrap();
            if meta.revision != requested_revision || runtime.stopping.load(Ordering::Acquire) != 0 {
                return Err("The browser changed while it waited to open. Try again.".into());
            }
            *runtime.opening.lock().unwrap() = opening.clone();
            meta.revision
        };
        if app.plugins.lock().unwrap().get(super::PLUGIN_ID).is_none() {
            return Err("Install the Browser plugin on this Runner first.".into());
        }
        if !human && self.taken_over(bot_id) {
            return Err("The user has the browser. Wait until they hand it back.".into());
        }
        let existing = runtime.server.lock().unwrap().clone();
        let existing = match existing {
            Some(server) if !server.is_current(app) => {
                server.stop();
                server.wait_stopped().await?;
                runtime.server.lock().unwrap().take();
                None
            }
            other => other.filter(|s| !s.is_closed()),
        };
        let server = match existing {
            Some(server) => server,
            None => {
                let dir = app.config.home.join("browser/profiles");
                std::fs::create_dir_all(dir.join(id)).map_err(|e| e.to_string())?;
                crate::config::set_private(&dir).map_err(|e| e.to_string())?;
                crate::config::set_private(&dir.join(id)).map_err(|e| e.to_string())?;
                let server = tokio::select! {
                    server = crate::plugins::mcp::visible_browser(app, id) => server?,
                    _ = opening.cancelled() => return Err("The browser was closed while it opened.".into()),
                };
                *runtime.server.lock().unwrap() = Some(server.clone());
                server
            }
        };
        let opened = tokio::select! {
            opened = server.open_visible() => opened,
            _ = opening.cancelled() => { server.stop(); return Err("The browser was closed while it opened.".into()); },
        };
        if let Err(error) = opened {
            server.stop();
            runtime.server.lock().unwrap().take();
            return Err(error);
        }
        // Stop can run while a browser opens. It revokes input immediately.
        if server.is_closed() {
            return Err("The browser was closed while it opened.".into());
        }
        self.owned(app, bot_id, id)?;
        {
            let mut meta = runtime.meta.lock().unwrap();
            if meta.revision != opening_revision {
                server.stop();
                return Err("The browser changed while it opened. Try again.".into());
            }
            meta.state = if human { Control::Human } else { Control::Bot };
            meta.revision += 1;
        }
        self.select(&runtime);
        if let Err(error) = self.save(app) {
            runtime.mark_stopped();
            return Err(error);
        }
        let meta = runtime.meta.lock().unwrap().clone();
        Ok(meta)
    }

    /// Takes the browser from the bot: new Browser calls wait at once, and control passes once
    /// the call in flight has answered. On the Runner itself the window comes forward.
    pub async fn takeover(&self, app: &Arc<App>, bot_id: &str, id: &str, local: bool) -> Result<Session, String> {
        let runtime = self.owned(app, bot_id, id)?;
        {
            let mut meta = runtime.meta.lock().unwrap();
            if meta.state == Control::Stopped {
                return Err("This browser isn't open.".into());
            }
            if meta.state != Control::Human {
                meta.state = Control::TakingOver;
                meta.revision += 1;
            }
        }
        self.save(app)?;
        let _guard = runtime.input.lock().await;
        self.owned(app, bot_id, id)?;
        let meta = {
            let mut meta = runtime.meta.lock().unwrap();
            if meta.state == Control::TakingOver {
                meta.state = Control::Human;
                meta.revision += 1;
            } else if meta.state != Control::Human {
                return Err("The browser was closed while you took it over.".into());
            }
            meta.clone()
        };
        self.save(app)?;
        if local {
            if let Some(server) = runtime.open_server() {
                let _ = server.open_visible().await;
            }
        }
        Ok(meta)
    }

    pub async fn resume(&self, app: &Arc<App>, bot_id: &str, id: &str, revision: u64) -> Result<Session, String> {
        let runtime = self.owned(app, bot_id, id)?;
        let _guard = runtime.input.lock().await;
        self.owned(app, bot_id, id)?;
        {
            let mut meta = runtime.meta.lock().unwrap();
            if meta.revision != revision || meta.state != Control::Human {
                return Err("The browser changed on its Runner. Look again before handing it back.".into());
            }
            if runtime.open_server().is_none() {
                return Err("The browser was closed. Open it again.".into());
            }
            meta.state = Control::Bot;
            meta.revision += 1;
        }
        self.select(&runtime);
        self.save(app)?;
        let meta = runtime.meta.lock().unwrap().clone();
        Ok(meta)
    }

    /// Closes the profile's browser. The profile keeps its sign-ins for the next open.
    pub async fn stop(&self, app: &Arc<App>, bot_id: &str, id: &str) -> Result<Session, String> {
        let runtime = self.owned(app, bot_id, id)?;
        runtime.stopping.fetch_add(1, Ordering::AcqRel);
        let _stopping = Stopping(&runtime.stopping);
        let meta = {
            let mut meta = runtime.meta.lock().unwrap();
            meta.state = Control::Stopped;
            meta.revision += 1;
            meta.clone()
        };
        // Stop works while a takeover or an open waits for the gate.
        runtime.opening.lock().unwrap().cancel();
        let server = runtime.server.lock().unwrap().take();
        let immediate = runtime.input.clone().try_lock_owned().ok();
        if let Some(server) = &server {
            if immediate.is_some() {
                // With no call in flight, Chromium closes on its own and writes out its cookies.
                let _ = tokio::time::timeout(std::time::Duration::from_secs(3), server.browser_call("browser_close", json!({}))).await;
            }
            server.stop();
        }
        self.save(app)?;
        let _guard = match immediate {
            Some(guard) => guard,
            None => runtime.input.clone().lock_owned().await,
        };
        if let Some(server) = server {
            server.wait_stopped().await?;
        }
        let output = app.config.home.join("browser/output").join(id);
        if output.is_dir() {
            let _ = std::fs::remove_dir_all(output);
        }
        Ok(meta)
    }

    /// Closes the profile's browser and forgets the profile with its sign-ins.
    pub async fn delete(&self, app: &Arc<App>, bot_id: &str, id: &str) -> Result<(), String> {
        let runtime = self.owned(app, bot_id, id)?;
        if runtime.meta.lock().unwrap().state != Control::Stopped || runtime.open_server().is_some() {
            self.stop(app, bot_id, id).await?;
        }
        let selected = runtime.meta.lock().unwrap().selected;
        let mut items = self.items.lock().unwrap();
        items.remove(id);
        if selected {
            // The oldest profile left is the one the bot opens next.
            let next = items
                .values()
                .filter_map(|item| {
                    let meta = item.meta.lock().unwrap();
                    (meta.bot_id == bot_id).then(|| (meta.created_at, item.clone()))
                })
                .min_by(|a, b| a.0.total_cmp(&b.0));
            if let Some((_, next)) = next {
                let mut meta = next.meta.lock().unwrap();
                meta.selected = true;
                meta.revision += 1;
            }
        }
        drop(items);
        self.save(app)?;
        let profile = app.config.home.join("browser/profiles").join(id);
        if profile.is_dir() {
            std::fs::remove_dir_all(profile).map_err(|e| e.to_string())?;
        }
        Ok(())
    }

    pub async fn screenshot(&self, app: &Arc<App>, bot_id: &str, id: &str, chat_id: &str) -> Result<Message, String> {
        let runtime = self.owned(app, bot_id, id)?;
        require_chat(app, chat_id, bot_id)?;
        let _guard = runtime.input.lock().await;
        self.owned(app, bot_id, id)?;
        let server = runtime.open_server().ok_or("Open the browser to take a screenshot.")?;
        let result = server.browser_call("browser_take_screenshot", json!({ "type": "png" })).await?;
        let name = runtime.meta.lock().unwrap().name.clone();
        publish_image(app, bot_id, chat_id, &name, &result)
    }

    /// Closes every open browser at once, as when the Browser plugin is removed.
    pub fn close_all(&self, app: &Arc<App>) {
        let sessions: Vec<_> = self.items.lock().unwrap().values().cloned().collect();
        for runtime in &sessions {
            runtime.opening.lock().unwrap().cancel();
            if runtime.meta.lock().unwrap().state != Control::Stopped {
                runtime.mark_stopped();
            }
        }
        let _ = self.save(app);
    }

    pub fn reset(&self) {
        let mut items = self.items.lock().unwrap();
        for runtime in items.values() {
            runtime.opening.lock().unwrap().cancel();
            runtime.mark_stopped();
        }
        items.clear();
        drop(items);
        *self.account.lock().unwrap() = None;
        self.changed.notify_waiters();
    }

    /// The CLI is quitting: every browser closes and its waiting calls end.
    pub async fn shutdown(&self, app: &Arc<App>) {
        let sessions: Vec<_> = self.items.lock().unwrap().values().cloned().collect();
        for runtime in &sessions {
            runtime.opening.lock().unwrap().cancel();
            if let Some(server) = runtime.open_server() {
                let _ = tokio::time::timeout(std::time::Duration::from_secs(3), server.browser_call("browser_close", json!({}))).await;
            }
            runtime.mark_stopped();
        }
        self.changed.notify_waiters();
        if !sessions.is_empty() {
            let _ = self.save(app);
        }
    }
}

fn valid_id(id: &str) -> bool {
    id.strip_prefix("browser-").is_some_and(|id| uuid::Uuid::parse_str(id).is_ok())
}

fn label(text: &str) -> Result<String, String> {
    let text = text.trim();
    if text.is_empty() || text.chars().count() > 100 || text.chars().any(char::is_control) {
        return Err("Name the profile in 1 to 100 characters.".into());
    }
    Ok(text.to_string())
}

fn require_chat(app: &App, chat_id: &str, bot_id: &str) -> Result<(), String> {
    let chat = app.chat(chat_id).ok_or("Unknown chat")?;
    if !chat.meta.bot_ids.iter().any(|id| id == bot_id) {
        return Err("The bot isn't in this chat.".into());
    }
    Ok(())
}

/// A screenshot of a profile's browser, published as the bot's output in the chat: one series per
/// profile, each screenshot its next version, an after screenshot no one has verified.
pub fn publish_image(app: &Arc<App>, bot_id: &str, chat_id: &str, name: &str, result: &rmcp::model::CallToolResult) -> Result<Message, String> {
    use base64::Engine;
    use crate::outputs::{EvidenceKind, EvidenceStatus, OutputEvidence, PublishOutput};
    require_chat(app, chat_id, bot_id)?;
    let image = result
        .content
        .iter()
        .find_map(|block| match block {
            rmcp::model::ContentBlock::Image(image) => Some(image),
            _ => None,
        })
        .ok_or("The Browser server returned no screenshot.")?;
    let bytes = base64::engine::general_purpose::STANDARD.decode(&image.data).map_err(|e| e.to_string())?;
    if image.mime_type != "image/png" || !bytes.starts_with(b"\x89PNG\r\n\x1a\n") {
        return Err("The Browser server returned a screenshot that isn't a PNG.".into());
    }
    let title = format!("Browser · {name}.png");
    let latest = crate::outputs::list(app, chat_id, None)?
        .into_iter()
        .filter_map(|message| {
            let output = message.output.as_ref()?;
            (output.bot_id == bot_id && output.name == title).then(|| (output.version, message.id.clone()))
        })
        .max_by_key(|(version, _)| *version)
        .map(|(_, id)| id);
    let dir = app.config.home.join("browser");
    let path = dir.join(format!("{}.png", uuid::Uuid::new_v4()));
    crate::config::write_private(&path, &bytes).map_err(|e| e.to_string())?;
    let published = crate::outputs::publish(
        app,
        chat_id,
        bot_id,
        &dir,
        PublishOutput {
            name: title,
            path: Some(path.display().to_string()),
            mime: Some("image/png".into()),
            replaces: latest,
            evidence: Some(OutputEvidence { kind: EvidenceKind::AfterScreenshot, summary: format!("What the {name} browser showed."), status: EvidenceStatus::Unverified, command: None, exit_code: None }),
            ..Default::default()
        },
    );
    let _ = std::fs::remove_file(path);
    published
}

pub struct SessionTool {
    pub app: Arc<App>,
    pub bot: Bot,
}

#[async_trait]
impl Tool for SessionTool {
    fn name(&self) -> &str {
        "browser_session"
    }
    fn description(&self) -> &str {
        "Your browser profiles on your Runner; each keeps its own sign-ins. `list` shows them, `create` adds one with a `name`, and `open` opens one (`session_id`) in a window on the Runner. While a profile is open, your Browser plugin calls use it; without one they use the Runner's shared headless browser. Open a profile when a site needs the user's sign-in, and ask the user to sign in there. While the user has taken over the browser, your Browser calls wait until they hand it back."
    }
    fn parameters(&self) -> Value {
        json!({ "type": "object", "properties": { "action": { "type": "string", "enum": ["list", "create", "open"] }, "session_id": { "type": "string" }, "name": { "type": "string" } }, "required": ["action"] })
    }
    async fn execute(&self, _id: &str, args: Value, cancel: CancellationToken, _on_update: ToolUpdateFn) -> Result<ToolResult, ToolError> {
        if cancel.is_cancelled() {
            return Err(ToolError("Stopped".into()));
        }
        let sessions = &self.app.browser_sessions;
        let result = match args["action"].as_str() {
            Some("list") => json!(sessions.list(&self.app, &self.bot.id).map_err(ToolError)?),
            Some("create") => json!(sessions.create(&self.app, &self.bot.id, args["name"].as_str().unwrap_or("Default")).map_err(ToolError)?),
            Some("open") => {
                let id = args["session_id"].as_str().ok_or_else(|| ToolError("missing session_id".into()))?;
                let opened = tokio::select! {
                    opened = sessions.open(&self.app, &self.bot.id, id, false) => opened.map_err(ToolError)?,
                    _ = cancel.cancelled() => {
                        let _ = sessions.stop(&self.app, &self.bot.id, id).await;
                        return Err(ToolError("Stopped".into()));
                    }
                };
                json!(opened)
            }
            _ => return Err(ToolError("Unknown browser_session action".into())),
        };
        Ok(ToolResult::text(serde_json::to_string(&result).unwrap()))
    }
}

/// `browser_session` needs the bot's Access to Browser, waits out a takeover, and opening a
/// window on the Runner passes Auto-review as a plugin action does.
pub async fn review_call(app: &Arc<App>, bot: &Bot, chat_id: &str, trigger: &crate::plugins::review::Trigger, unattended: bool, ctx: &BeforeToolCallContext<'_>) -> Option<BeforeToolCallResult> {
    if ctx.tool_call.name != "browser_session" {
        return None;
    }
    // Profiles are part of the Browser plugin: the bot's Access to it covers them.
    if let Err(denied) = crate::permissions::check_plugin(app, bot, super::PLUGIN_ID, "browser_session") {
        return Some(crate::permissions::refuse(app, chat_id, bot, denied));
    }
    if let Err(error) = app.browser_sessions.wait_if_taken_over(&bot.id, ctx.cancel).await {
        return Some(crate::local_review::blocked(error));
    }
    if ctx.args["action"] != "open" {
        return None;
    }
    let description = "Open one of this bot's browser profiles in a window on its Runner.";
    let outcome = crate::plugins::review::decide(app, bot, chat_id, trigger, "browser-session", "Browser", "browser_session", description, ctx.args, None, ctx.cancel).await;
    let crate::plugins::review::Outcome::Ask { reason, .. } = outcome else {
        return None;
    };
    if unattended {
        return Some(crate::local_review::blocked("Opening a browser window needs approval; nobody is here to approve it.".into()));
    }
    match crate::plugins::mcp::ask(app, chat_id, &bot.id, "browser-session", "Browser", "browser_session", description, ctx.args.clone(), reason, ctx.cancel).await {
        crate::plugins::mcp::Decision::Allowed | crate::plugins::mcp::Decision::Always => None,
        crate::plugins::mcp::Decision::Dismissed => Some(crate::local_review::dismissed("The user wrote instead of approving the browser.")),
        _ => Some(crate::local_review::blocked("The user didn't approve opening the browser.".into())),
    }
}

#[cfg(test)]
#[path = "tests.rs"]
mod tests;
