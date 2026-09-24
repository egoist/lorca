//! The marketplace, after Grok Bot's: plugins to install on a Runner and bots to add from a
//! template, with the ones worth a first look featured. The index ships in the CLI
//! (`marketplace/index.json`); `marketplace_url` in settings (or `LORCA_MARKETPLACE_URL`) names
//! another, fetched at most once an hour, whose entries add to the bundled ones or replace them
//! by id.

use std::sync::Arc;

use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::app::App;
use crate::config::now_secs;
use crate::model::{Bot, SetupPlugin, TemplateSetup};
use crate::plugins::Manifest;

/// The bundled index: first-party plugins and bots.
const BUNDLED_INDEX: &str = include_str!("../marketplace/index.json");

/// How long a fetched index is kept before it is asked for again.
const INDEX_TTL_SECS: f64 = 3600.0;

/// What the marketplace offers, in index order.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Index {
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

    /// Entries of `other` replace the ones with the same id and follow the rest.
    fn merge(&mut self, other: Index) {
        for plugin in other.plugins {
            match self.plugins.iter_mut().find(|p| p.id == plugin.id) {
                Some(existing) => *existing = plugin,
                None => self.plugins.push(plugin),
            }
        }
        for bot in other.bots {
            match self.bots.iter_mut().find(|b| b.id == bot.id) {
                Some(existing) => *existing = bot,
                None => self.bots.push(bot),
            }
        }
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

/// The index on offer: the bundled one, plus the one at `marketplace_url` when set. A fetch
/// that fails leaves the bundled index. Installed marketplace plugins follow what it lists.
pub async fn index(app: &Arc<App>) -> Index {
    let mut index = bundled();
    let url = app.settings.lock().unwrap().marketplace_url.clone().or_else(|| std::env::var("LORCA_MARKETPLACE_URL").ok()).filter(|u| !u.trim().is_empty());
    if let Some(url) = url {
        let cached = app.marketplace_cache.lock().unwrap().clone();
        let extra = match cached {
            Some((at, extra)) if now_secs() - at < INDEX_TTL_SECS => extra,
            _ => match fetch(app, &url).await {
                Ok(extra) => {
                    *app.marketplace_cache.lock().unwrap() = Some((now_secs(), extra.clone()));
                    extra
                }
                Err(error) => {
                    tracing::warn!(%error, url, "fetching the marketplace index");
                    Index::default()
                }
            },
        };
        index.merge(extra);
    }
    crate::plugins::refresh_installed(app, &index.plugins);
    index
}

/// The bundled index alone, for startup.
pub fn bundled() -> Index {
    parse(BUNDLED_INDEX).unwrap_or_default()
}

async fn fetch(app: &Arc<App>, url: &str) -> Result<Index, String> {
    let text = app.http.get(url).send().await.map_err(|e| e.to_string())?.error_for_status().map_err(|e| e.to_string())?.text().await.map_err(|e| e.to_string())?;
    parse(&text)
}

/// `{ "plugins": [manifest…], "bots": [template…] }`; either list may be missing. An entry
/// that does not read is skipped.
fn parse(text: &str) -> Result<Index, String> {
    let value: Value = serde_json::from_str(text).map_err(|e| e.to_string())?;
    let plugins = value.get("plugins").and_then(Value::as_array);
    let bots = value.get("bots").and_then(Value::as_array);
    if plugins.is_none() && bots.is_none() {
        return Err("The index has no plugins or bots list".into());
    }
    let mut index = Index::default();
    for entry in plugins.into_iter().flatten() {
        match Manifest::parse(entry) {
            Ok(manifest) => index.plugins.push(manifest),
            Err(error) => tracing::warn!(%error, "skipping a marketplace plugin"),
        }
    }
    for entry in bots.into_iter().flatten() {
        match BotTemplate::parse(entry) {
            Ok(template) => index.bots.push(template),
            Err(error) => tracing::warn!(%error, "skipping a marketplace bot"),
        }
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
        match crate::routines::create(app, &bot.id, &routine.name, &routine.schedule, &routine.prompt, false) {
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
        for plugin in &index.plugins {
            assert!(!plugin.icon.is_empty() && !plugin.description.is_empty() && !plugin.category.is_empty(), "{} is incomplete", plugin.id);
        }
        for bot in &index.bots {
            assert!(!bot.summary.is_empty() && !bot.category.is_empty(), "{} is incomplete", bot.id);
            for id in &bot.plugins {
                assert!(index.plugin(id).is_some(), "{} names the unknown plugin {id}", bot.id);
            }
        }
        assert!(index.plugins.iter().any(|p| p.featured) && index.bots.iter().any(|b| b.featured));
        assert_eq!(search_plugins(&index.plugins, "GIT hub").len(), 1);
        assert_eq!(search_plugins(&index.plugins, "").len(), index.plugins.len());
        assert!(search_plugins(&index.plugins, "nothing-like-this").is_empty());
        assert!(search_bots(&index.bots, "pull requests").iter().any(|b| b.id == "pr-reviewer"));
        assert_eq!(search_bots(&index.bots, "").len(), index.bots.len());
    }

    #[test]
    fn an_index_merges_by_id_and_skips_what_does_not_read() {
        let mut index = parse(r#"{ "plugins": [ { "id": "a", "name": "A", "servers": { "s": { "type": "http", "url": "https://a.test/mcp" } } } ],
                                   "bots": [ { "id": "b", "name": "B", "description": "Does b." } ] }"#)
        .unwrap();
        let other = parse(
            r#"{ "bots": [ { "id": "b", "name": "B2", "description": "Does b better.", "featured": true },
                           { "id": "Bad Id", "name": "X", "description": "x" },
                           { "id": "c", "name": "C", "description": "Does c.", "routines": [ { "name": "r", "schedule": "every fortnight", "prompt": "p" } ] },
                           { "id": "d", "name": "D", "description": "Does d.", "routines": [ { "name": "Daily", "schedule": "0 9 * * *", "prompt": "Report." } ] } ] }"#,
        )
        .unwrap();
        assert_eq!(other.bots.iter().map(|b| b.id.as_str()).collect::<Vec<_>>(), ["b", "d"], "a bad id and a bad schedule are skipped");
        index.merge(other);
        assert_eq!(index.plugins.len(), 1);
        assert_eq!(index.bots.iter().map(|b| (b.id.as_str(), b.name.as_str())).collect::<Vec<_>>(), [("b", "B2"), ("d", "D")]);
        assert!(index.bot("b").unwrap().featured);
        assert_eq!(index.bot("d").unwrap().symbol_name, "sparkles", "a template without a look gets the default");
        assert!(parse(r#"{ "nothing": [] }"#).is_err());
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
