//! A template's skills go through the playbook store: a bot's saved skills as `playbooks.export`
//! gives them, and the new bot's saved through the same guarded `playbooks::save`. Only the
//! content travels, never a skill's id, provenance, or history.

use serde_json::Value;

use super::format::Skill;
use crate::app::App;
use crate::playbooks::{self as store, PlaybookContent, Provenance, Scope};

/// The bot's saved skills, without their bodies.
pub fn list(app: &App, bot_id: &str) -> Vec<Value> {
    let scope = serde_json::to_value(Scope::bot(bot_id)).unwrap_or_default();
    store::summaries(app).into_iter().filter(|skill| skill["scope"] == scope && skill["status"] == "saved").collect()
}

/// A skill's content, scrubbed as the export scrubs it.
pub fn export(app: &App, bot_id: &str, id: &str) -> Result<Skill, String> {
    let exported = store::export(app, &Scope::bot(bot_id), id)?;
    serde_json::from_value(exported["content"].clone()).map_err(|e| format!("Unsupported skill content: {e}"))
}

/// Saves a template's skill as a new skill of the bot, and answers what `remove` takes back.
pub fn save(app: &App, bot_id: &str, skill: &Skill) -> Result<Value, String> {
    let content: PlaybookContent = serde_json::to_value(skill).and_then(serde_json::from_value).map_err(|e| e.to_string())?;
    store::save(app, &Scope::bot(bot_id), None, content, 0, "", Provenance { kind: "template_import".into(), ..Default::default() })
}

pub fn remove(app: &App, bot_id: &str, saved: &Value) -> Result<(), String> {
    let id = saved["id"].as_str().ok_or("Invalid skill id")?;
    let revision = saved["revision"].as_u64().ok_or("Invalid skill revision")?;
    store::remove(app, &Scope::bot(bot_id), id, revision, saved["hash"].as_str().unwrap_or_default()).map(|_| ())
}
