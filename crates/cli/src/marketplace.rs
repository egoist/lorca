//! The marketplace, after Grok Bot's: plugins to install on a Runner and bots to add from a
//! template, with the ones worth a first look featured. The index in use is the latest of the
//! one this build carries (`marketplace/index.json`), the one cached in `marketplace.json`, and
//! the one served at [`DEFAULT_URL`] (`LORCA_MARKETPLACE_URL`; `LORCA_MARKETPLACE_FETCH=0` turns
//! fetching off), so a plugin added to the index reaches every Device without a release.
//! `lorca serve` checks the server when it starts; after that an app's `bootstrap` and each use
//! of the index ([`index`]) check, and `marketplace.reload` checks even within the hour. A check
//! within the hour of the last one is skipped, and nothing checks on a timer. A bot's search or
//! install that finds nothing checks first, at most every five minutes, since the plugin may have
//! been published since. Installed marketplace plugins follow the index in use.

use std::sync::{Arc, Mutex, OnceLock, RwLock};
use std::time::{Duration, Instant};

use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::app::App;
use crate::config::{self, Config};
use crate::model::{Bot, SetupPlugin, TemplateSetup};
use crate::plugins::Manifest;
use crate::served;

/// The bundled index: first-party plugins and bots.
const BUNDLED_INDEX: &str = include_str!("../marketplace/index.json");

pub const DEFAULT_URL: &str = "https://lorca.app/marketplace/v1.json";

/// The format this build reads. A change older builds cannot read gets a new version, served
/// beside this one.
const VERSION: u64 = 1;

/// How often a search or an install that found nothing may check.
const MISSING_EVERY: Duration = Duration::from_secs(5 * 60);

/// What the marketplace offers, in index order.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Index {
    /// When it last changed, as `YYYY-MM-DDTHH:MM:SSZ`; of two indexes the later one wins.
    pub updated: String,
    pub plugins: Vec<Manifest>,
    pub bots: Vec<BotTemplate>,
}

impl Index {
    pub fn plugin(&self, id: &str) -> Option<&Manifest> {
        self.plugins.iter().find(|p| p.id == id)
    }

    pub fn bot(&self, id: &str) -> Option<&BotTemplate> {
        self.bots.iter().find(|b| b.id == id)
    }
}

/// A bot to add from the marketplace: the profile it starts with, the plugins it works with,
/// the routines it brings, and what it knows from the start. Adding one installs no plugin;
/// its first turn asks which to install.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct BotTemplate {
    /// Lowercase letters, digits, and dashes.
    pub id: String,
    pub name: String,
    /// One line for the marketplace's rows.
    #[serde(default)]
    pub summary: String,
    /// What the bot does and how it should work: the new bot's description.
    pub description: String,
    #[serde(default = "default_symbol")]
    pub symbol_name: String,
    #[serde(default = "default_accent")]
    pub accent: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub category: String,
    /// Listed under Featured Bots, in index order.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub featured: bool,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub author: String,
    /// Marketplace plugin ids.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub plugins: Vec<String>,
    /// Added paused with the bot; it asks whether to turn them on.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub routines: Vec<RoutineTemplate>,
    /// Facts the bot saves to its memory on its first turn.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub memory: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct RoutineTemplate {
    pub name: String,
    /// Anything `crate::schedule::parse` reads: `every 2h`, `0 9 * * 1-5`.
    pub schedule: String,
    pub prompt: String,
}

fn default_symbol() -> String {
    "sparkles".into()
}

fn default_accent() -> String {
    "indigo".into()
}

impl BotTemplate {
    /// Reads a template, refusing one that could not be added.
    pub fn parse(value: &Value) -> Result<BotTemplate, String> {
        let template: BotTemplate = serde_json::from_value(value.clone()).map_err(|e| format!("Not a bot template: {e}"))?;
        if !crate::plugins::is_id(&template.id) {
            return Err(format!("A bot template id is lowercase letters, digits, and dashes; {:?} is not.", template.id));
        }
        if template.name.trim().is_empty() || template.description.trim().is_empty() {
            return Err(format!("The bot template {} needs a name and a description.", template.id));
        }
        for routine in &template.routines {
            if routine.name.trim().is_empty() || routine.prompt.trim().is_empty() {
                return Err(format!("A routine of {} needs a name and a prompt.", template.name));
            }
            crate::schedule::parse(&routine.schedule).map_err(|e| format!("{} · {}: {e}", template.name, routine.name))?;
        }
        Ok(template)
    }
}

// MARK: - The index

/// This Device's marketplace: the index in use, and its checks for a newer one.
pub struct Updates {
    /// The latest of the bundled index, the cached one, and the served one.
    current: RwLock<Arc<Index>>,
    /// Where newer indexes come from. Unset until [`enable`], so an App that never enables it,
    /// a test or a one-shot command, reads only the bundled and cached indexes.
    url: OnceLock<String>,
    /// One check at a time.
    checking: tokio::sync::Mutex<()>,
    /// When a search or an install that found nothing last checked.
    missing_checked: Mutex<Option<Instant>>,
}

impl Updates {
    /// The bundled index, or the cached one when it is later.
    pub fn load(config: &Config) -> Updates {
        let mut index = bundled();
        if let Some(cache) = config::read_json::<Cache>(&config.marketplace_path()).filter(|cache| !cache.index.is_null()) {
            match parse(&cache.index.to_string()) {
                Ok(cached) if cached.updated > index.updated => index = cached,
                Ok(_) => {}
                Err(error) => tracing::warn!(%error, "reading the cached marketplace index"),
            }
        }
        Updates { current: RwLock::new(Arc::new(index)), url: OnceLock::new(), checking: tokio::sync::Mutex::new(()), missing_checked: Mutex::new(None) }
    }
}

/// `marketplace.json` in Lorca's folder: the last index fetched, as served, and what the next
/// check sends back.
#[derive(Debug, Default, Serialize, Deserialize)]
struct Cache {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    etag: Option<String>,
    /// Unix seconds of the last check, answered or not.
    #[serde(default)]
    checked_at: i64,
    #[serde(default)]
    index: Value,
}

/// Lets this Device check for newer indexes, at `LORCA_MARKETPLACE_URL` or lorca.app, unless
/// `LORCA_MARKETPLACE_FETCH=0`.
pub fn enable(app: &App) {
    if std::env::var("LORCA_MARKETPLACE_FETCH").is_ok_and(|value| value.trim() == "0") {
        return;
    }
    let url = std::env::var("LORCA_MARKETPLACE_URL").ok().map(|url| url.trim().to_string()).filter(|url| !url.is_empty());
    let _ = app.marketplace.url.set(url.unwrap_or_else(|| DEFAULT_URL.into()));
}

/// The index in use, as it stands.
pub fn current(app: &App) -> Arc<Index> {
    app.marketplace.current.read().unwrap().clone()
}

/// The index on offer, after a check for a newer one unless the last was within the hour.
pub async fn index(app: &Arc<App>) -> Arc<Index> {
    if app.marketplace.url.get().is_some() {
        if let Err(error) = check(app, false).await {
            tracing::warn!(%error, "checking for a newer marketplace index");
        }
    }
    current(app)
}

/// Checks for a newer index in the background, unless updates are off.
pub fn check_in_background(app: &Arc<App>) {
    if app.marketplace.url.get().is_none() {
        return;
    }
    let app = app.clone();
    tokio::spawn(async move {
        if let Err(error) = check(&app, false).await {
            tracing::warn!(%error, "checking for a newer marketplace index");
        }
    });
}

/// After a bot's search or install found nothing: the plugin may have been published since the
/// last check. Checks now, at most every five minutes. True when a newer index came.
pub async fn check_for_missing(app: &Arc<App>) -> bool {
    if app.marketplace.url.get().is_none() {
        return false;
    }
    {
        let mut last = app.marketplace.missing_checked.lock().unwrap();
        if last.is_some_and(|at| at.elapsed() < MISSING_EVERY) {
            return false;
        }
        *last = Some(Instant::now());
    }
    check(app, true).await.unwrap_or_else(|error| {
        tracing::warn!(%error, "checking for a newer marketplace index");
        false
    })
}

/// Asks the server for its index, unless the last check was within the hour and this one is not
/// forced. A later index than the one in use replaces it, and the installed marketplace plugins
/// follow it. True when one did.
pub async fn check(app: &Arc<App>, force: bool) -> Result<bool, String> {
    let Some(url) = app.marketplace.url.get() else { return Err("Marketplace updates are off".into()) };
    let _one_at_a_time = app.marketplace.checking.lock().await;
    let path = app.config.marketplace_path();
    let mut cache: Cache = config::read_json(&path).unwrap_or_default();
    let now = config::now_unix();
    if !force && (0..served::FRESH_SECS).contains(&(now - cache.checked_at)) {
        return Ok(false);
    }
    // The validator goes only with the index it names, so a 304 always leaves one to keep.
    let etag = cache.etag.as_deref().filter(|_| !cache.index.is_null());
    let fetched = match served::fetch(app, url, etag).await {
        Ok(Some((etag, text))) => parse(&text).map(|index| Some((etag, text, index))),
        Ok(None) => Ok(None),
        Err(error) => Err(error),
    };
    cache.checked_at = now;
    let installed = match fetched {
        Ok(Some((etag, text, index))) => {
            let installed = install_index(app, index);
            // Kept even when it is not newer than the one in use, so the next check can ask
            // whether it changed.
            cache.index = serde_json::from_str(&text).unwrap_or_default();
            cache.etag = etag;
            Ok(installed)
        }
        Ok(None) => Ok(false),
        Err(error) => Err(error),
    };
    if let Err(error) = config::write_json_private(&path, &cache) {
        tracing::warn!(%error, "saving the marketplace index");
    }
    installed
}

/// Makes `index` the one in use when it is later than that one, and has the installed
/// marketplace plugins follow it. True when it was.
fn install_index(app: &Arc<App>, index: Index) -> bool {
    let index = Arc::new(index);
    {
        let mut current = app.marketplace.current.write().unwrap();
        if index.updated <= current.updated {
            return false;
        }
        *current = index.clone();
    }
    crate::plugins::refresh_installed(app, &index.plugins);
    true
}

/// The index this build carries.
pub fn bundled() -> Index {
    parse(BUNDLED_INDEX).unwrap_or_default()
}

#[derive(Deserialize)]
struct RawIndex {
    version: u64,
    updated: String,
    #[serde(default)]
    plugins: Vec<Value>,
    #[serde(default)]
    bots: Vec<Value>,
}

/// Reads an index by rules every later version keeps, so an index written for a newer Lorca
/// never breaks an older one: fields this version does not know are ignored, and an entry it
/// cannot read, or one whose id came before, is left out. An index is refused whole when its
/// `version` is not one this build reads, its `updated` is not a UTC time, or it lists no plugin
/// this build reads.
fn parse(text: &str) -> Result<Index, String> {
    let raw: RawIndex = serde_json::from_str(text).map_err(|e| format!("The marketplace index does not read: {e}"))?;
    if raw.version != VERSION {
        return Err(format!("The marketplace index is version {}; this Lorca reads version {VERSION}", raw.version));
    }
    if !served::is_utc_time(&raw.updated) {
        return Err(format!("The marketplace index's updated time {:?} is not YYYY-MM-DDTHH:MM:SSZ", raw.updated));
    }
    let mut index = Index { updated: raw.updated, ..Index::default() };
    for entry in &raw.plugins {
        match Manifest::parse(entry) {
            Ok(manifest) if index.plugin(&manifest.id).is_some() => tracing::warn!(id = %manifest.id, "skipping a marketplace plugin listed twice"),
            Ok(manifest) => index.plugins.push(manifest),
            Err(error) => tracing::warn!(%error, "skipping a marketplace plugin"),
        }
    }
    for entry in &raw.bots {
        match BotTemplate::parse(entry) {
            Ok(template) if index.bot(&template.id).is_some() => tracing::warn!(id = %template.id, "skipping a marketplace bot listed twice"),
            Ok(template) => index.bots.push(template),
            Err(error) => tracing::warn!(%error, "skipping a marketplace bot"),
        }
    }
    if index.plugins.is_empty() {
        return Err("The marketplace index lists no plugin this Lorca reads".into());
    }
    Ok(index)
}

/// Plugins matching `query`: every word appears in the id, name, description, category, or
/// tags. All of them for an empty query.
pub fn search_plugins<'a>(plugins: &'a [Manifest], query: &str) -> Vec<&'a Manifest> {
    let words = words(query);
    plugins.iter().filter(|p| matches(&[&p.id, &p.name, &p.description, &p.category, &p.author, &p.tags.join(" ")], &words)).collect()
}

/// Bots matching `query` the same way, over the id, name, summary, description, and category.
pub fn search_bots<'a>(bots: &'a [BotTemplate], query: &str) -> Vec<&'a BotTemplate> {
    let words = words(query);
    bots.iter().filter(|b| matches(&[&b.id, &b.name, &b.summary, &b.description, &b.category, &b.author], &words)).collect()
}

fn words(query: &str) -> Vec<String> {
    query.split_whitespace().map(str::to_lowercase).collect()
}

fn matches(fields: &[&str], words: &[String]) -> bool {
    let haystack = fields.join(" ").to_lowercase();
    words.iter().all(|w| haystack.contains(w))
}

// MARK: - Adding a bot

/// The template `id`, and the plugins it names as the index describes them.
pub async fn template(app: &Arc<App>, id: &str) -> Result<(BotTemplate, Vec<SetupPlugin>), String> {
    let index = index(app).await;
    let template = index.bot(id).cloned().ok_or_else(|| format!("No bot {id} in the marketplace"))?;
    let plugins = template
        .plugins
        .iter()
        .filter_map(|id| index.plugin(id))
        .map(|p| SetupPlugin { id: p.id.clone(), name: p.name.clone(), description: p.description.clone() })
        .collect();
    Ok((template, plugins))
}

/// A bot just added from `template`, with its direct chat: its routines, added paused, and a
/// first turn in which it greets the user and sets itself up, as Grok Bot's template import
/// does. `greeting` is the user's first message, worded by the app in the user's language.
pub fn welcome(app: &Arc<App>, bot: &Bot, chat_id: &str, template: &BotTemplate, plugins: Vec<SetupPlugin>, greeting: Option<String>) {
    let mut routines = Vec::new();
    for routine in &template.routines {
        match crate::routines::create(app, &bot.id, &routine.name, &routine.schedule, &routine.prompt, None, false) {
            Ok(created) => routines.push(created.name),
            Err(error) => tracing::warn!(%error, routine = %routine.name, "adding a template's routine"),
        }
    }
    let setup = TemplateSetup { template: template.name.clone(), plugins, routines, memory: template.memory.clone() };
    let greeting = greeting.filter(|g| !g.trim().is_empty()).unwrap_or_else(|| format!("Hi {}, introduce yourself.", bot.name));
    crate::runtime::greet_new_bot(app, chat_id, &bot.id, &greeting, setup);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_bundled_index_parses() {
        let index = bundled();
        let text: Value = serde_json::from_str(BUNDLED_INDEX).unwrap();
        assert_eq!(index.plugins.len(), text["plugins"].as_array().unwrap().len(), "every bundled plugin reads");
        assert_eq!(index.bots.len(), text["bots"].as_array().unwrap().len(), "every bundled bot reads");
        let mut ids = std::collections::HashSet::new();
        for plugin in &index.plugins {
            assert!(ids.insert(plugin.id.as_str()), "{} is listed twice", plugin.id);
            assert!(!plugin.icon.is_empty() && !plugin.description.is_empty() && !plugin.category.is_empty(), "{} is incomplete", plugin.id);
            assert!(!plugin.author.is_empty() && plugin.homepage.as_deref().is_some_and(|h| h.starts_with("https://")), "{} needs its maker and website", plugin.id);
            for (name, server) in &plugin.servers {
                if let crate::plugins::ServerSpec::Http { url, .. } = server {
                    assert!(url.starts_with("https://"), "{}'s server {name} is not HTTPS", plugin.id);
                }
            }
        }
        for bot in &index.bots {
            assert!(!bot.summary.is_empty() && !bot.category.is_empty(), "{} is incomplete", bot.id);
            for id in &bot.plugins {
                assert!(index.plugin(id).is_some(), "{} names the unknown plugin {id}", bot.id);
            }
        }
        assert!(index.plugins.iter().any(|p| p.featured) && index.bots.iter().any(|b| b.featured));
        assert_eq!(search_plugins(&index.plugins, "PULL requests").iter().map(|p| p.id.as_str()).collect::<Vec<_>>(), ["github"]);
        assert_eq!(search_plugins(&index.plugins, "").len(), index.plugins.len());
        assert!(search_plugins(&index.plugins, "nothing-like-this").is_empty());
        assert!(search_bots(&index.bots, "pull requests").iter().any(|b| b.id == "pr-reviewer"));
        assert_eq!(search_bots(&index.bots, "").len(), index.bots.len());
    }

    #[test]
    fn an_index_leaves_out_what_this_build_cannot_read() {
        let index = parse(
            r#"{ "version": 1, "updated": "2026-10-02T00:00:00Z", "later": true,
                 "plugins": [ { "id": "a", "name": "A", "servers": { "s": { "type": "http", "url": "https://a.test/mcp" } }, "later": 1 },
                              { "id": "a", "name": "A again", "servers": { "s": { "type": "http", "url": "https://a.test/mcp" } } },
                              { "id": "w", "name": "W", "servers": { "s": { "type": "websocket", "url": "wss://w.test/mcp" } } } ],
                 "bots": [ { "id": "b", "name": "B", "description": "Does b.", "featured": true },
                           { "id": "Bad Id", "name": "X", "description": "x" },
                           { "id": "c", "name": "C", "description": "Does c.", "routines": [ { "name": "r", "schedule": "every fortnight", "prompt": "p" } ] },
                           { "id": "d", "name": "D", "description": "Does d.", "routines": [ { "name": "Daily", "schedule": "0 9 * * *", "prompt": "Report." } ] } ] }"#,
        )
        .unwrap();
        assert_eq!(index.updated, "2026-10-02T00:00:00Z");
        assert_eq!(index.plugins.iter().map(|p| p.name.as_str()).collect::<Vec<_>>(), ["A"], "a second a and a server this build cannot run are left out");
        assert_eq!(index.bots.iter().map(|b| b.id.as_str()).collect::<Vec<_>>(), ["b", "d"], "a bad id and a bad schedule are left out");
        assert!(index.bot("b").unwrap().featured);
        assert_eq!(index.bot("d").unwrap().symbol_name, "sparkles", "a template without a look gets the default");

        let a = r#"[ { "id": "a", "name": "A", "servers": { "s": { "type": "http", "url": "https://a.test/mcp" } } } ]"#;
        let refused = [
            format!(r#"{{ "version": 2, "updated": "2026-10-02T00:00:00Z", "plugins": {a} }}"#),
            format!(r#"{{ "updated": "2026-10-02T00:00:00Z", "plugins": {a} }}"#),
            format!(r#"{{ "version": 1, "updated": "2026-10-02", "plugins": {a} }}"#),
            r#"{ "version": 1, "updated": "2026-10-02T00:00:00Z", "plugins": [ { "id": "Bad Id", "name": "X" } ] }"#.to_string(),
        ];
        for text in refused {
            assert!(parse(&text).is_err(), "{text}");
        }
    }

    #[tokio::test]
    async fn a_later_index_is_fetched_cached_and_followed_by_installed_plugins() {
        // The bundled index under a later time, with GitHub's entry changed and a plugin it lacks.
        let mut later: Value = serde_json::from_str(BUNDLED_INDEX).unwrap();
        later["updated"] = Value::from("2999-01-01T00:00:00Z");
        let plugins = later["plugins"].as_array_mut().unwrap();
        plugins.iter_mut().find(|p| p["id"] == "github").unwrap()["description"] = Value::from("GitHub, as served.");
        plugins.push(serde_json::json!({ "id": "newcomer", "name": "Newcomer", "servers": { "s": { "type": "http", "url": "https://newcomer.test/mcp" } } }));
        let (url, seen) = served::test_server("/marketplace/v1.json", later.to_string());
        let home = std::env::temp_dir().join(format!("lorca-marketplace-{}", uuid::Uuid::new_v4()));
        let app = &App::load(Config { home: home.clone(), port: 0 }).unwrap();
        crate::plugins::install(app, bundled().plugins.into_iter().find(|m| m.id == "github").unwrap(), "marketplace").unwrap();
        assert_eq!(check(app, true).await, Err("Marketplace updates are off".into()));
        assert!(!check_for_missing(app).await);
        app.marketplace.url.set(url).unwrap();

        assert!(index(app).await.plugin("newcomer").is_some(), "the marketplace checks first");
        assert_eq!(current(app).updated, "2999-01-01T00:00:00Z");
        let description = app.plugins.lock().unwrap().get("github").unwrap().manifest.description.clone();
        assert_eq!(description, "GitHub, as served.", "an installed plugin follows the later index");
        let cache: Cache = config::read_json(&app.config.marketplace_path()).unwrap();
        assert_eq!((cache.etag.as_deref(), cache.index["updated"].as_str()), (Some("\"v1\""), Some("2999-01-01T00:00:00Z")));
        assert_eq!(Updates::load(&app.config).current.read().unwrap().updated, "2999-01-01T00:00:00Z", "the next start reads the cached index");

        // Within the hour nothing is asked; forced, the server is asked whether it changed.
        assert_eq!(check(app, false).await, Ok(false));
        assert_eq!(seen.lock().unwrap().len(), 1);
        assert_eq!(check(app, true).await, Ok(false));
        assert!(seen.lock().unwrap()[1].contains("\r\nif-none-match: \"v1\"\r\n"));

        // A search that found nothing checks again, at most every five minutes.
        assert!(!check_for_missing(app).await);
        assert_eq!(seen.lock().unwrap().len(), 3);
        assert!(!check_for_missing(app).await);
        assert_eq!(seen.lock().unwrap().len(), 3);
        let _ = std::fs::remove_dir_all(&home);
    }

    #[tokio::test]
    async fn a_bot_added_from_a_template_starts_with_paused_routines_and_a_greeting() {
        let home = std::env::temp_dir().join(format!("lorca-marketplace-{}", uuid::Uuid::new_v4()));
        let app = App::load(crate::config::Config { home: home.clone(), port: 0 }).unwrap();
        crate::identity::create(&app, Some("Workbench".into())).unwrap();
        let runner_id = app.this_device_id().unwrap();
        let params = serde_json::json!({ "template_id": "pr-reviewer", "runner_id": runner_id, "greeting": "Hi PR Reviewer, introduce yourself." });
        let out = crate::api::dispatch(&app, "bots.create", params).await.unwrap();
        let bot = app.bot(out["bot"]["id"].as_str().unwrap()).unwrap();
        assert_eq!((bot.name.as_str(), bot.symbol_name.as_str(), bot.accent.as_str()), ("PR Reviewer", "checklist", "blue"));
        assert!(bot.description.starts_with("You review pull requests"));
        let routines = app.routines_of(&bot.id);
        assert_eq!(routines.iter().map(|r| (r.name.as_str(), r.is_enabled)).collect::<Vec<_>>(), [("Review new pull requests", false)]);
        let chat_id = out["chat_id"].as_str().unwrap();
        let (messages, _) = app.store.page(chat_id, None, 10).unwrap();
        let greeted = messages.iter().any(|m| {
            m.author == crate::model::Author::You && matches!(&m.body, crate::model::Body::Text { text, .. } if text == "Hi PR Reviewer, introduce yourself.")
        });
        assert!(greeted, "the user's greeting opens the chat");
        let unknown = serde_json::json!({ "template_id": "nope", "runner_id": runner_id });
        assert!(crate::api::dispatch(&app, "bots.create", unknown).await.unwrap_err().contains("No bot nope"));
        let _ = std::fs::remove_dir_all(&home);
    }
}
