//! Content-only adapter for #77's public playbook API. Using the JSON boundary keeps this
//! feature independently buildable and reuses the encrypted account store when available.

use std::sync::Arc;

use serde_json::{json, Value};

use super::format::Skill;
use crate::app::App;

pub const UNAVAILABLE: &str = "This template has skills. Update Lorca to import it.";

pub async fn list(app: &Arc<App>, bot_id: &str) -> Result<Option<Vec<Value>>, String> {
    match call(
        app,
        "playbooks.list",
        json!({ "scope": scope(bot_id), "include_drafts": false }),
    )
    .await
    {
        Ok(value) => {
            let items = value["items"]
                .as_array()
                .ok_or("Invalid playbook list response")?;
            Ok(Some(
                items
                    .iter()
                    .filter(|item| item["status"] == "saved")
                    .cloned()
                    .collect(),
            ))
        }
        Err(error) if error == "unknown method playbooks.list" => Ok(None),
        Err(error) => Err(error),
    }
}

pub async fn export(app: &Arc<App>, bot_id: &str, id: &str) -> Result<Skill, String> {
    let value = call(
        app,
        "playbooks.export",
        json!({ "scope": scope(bot_id), "id": id }),
    )
    .await?;
    serde_json::from_value(value["content"].clone())
        .map_err(|e| format!("Unsupported reusable skill content: {e}"))
}

pub async fn save(app: &Arc<App>, bot_id: &str, content: &Skill) -> Result<Value, String> {
    call(app, "playbooks.save", json!({
        "scope": scope(bot_id), "content": content, "expected_revision": 0, "expected_hash": "",
        "provenance": { "kind": "template_import", "message_ids": [], "note": "Imported from a reviewed private bot template." }
    })).await
}

pub async fn remove(app: &Arc<App>, bot_id: &str, receipt: &Value) -> Result<(), String> {
    call(app, "playbooks.remove", json!({ "scope": scope(bot_id), "id": receipt["id"], "expected_revision": receipt["revision"], "expected_hash": receipt["hash"] })).await?;
    Ok(())
}

fn scope(bot_id: &str) -> Value {
    json!({ "kind": "bot", "id": bot_id })
}

async fn call(app: &Arc<App>, method: &str, params: Value) -> Result<Value, String> {
    // Indirection breaks the dispatch -> templates -> playbooks -> dispatch future cycle.
    Box::pin(crate::api::dispatch(app, method, params)).await
}
