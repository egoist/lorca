//! Runner-owned encrypted feedback and revision history. No examples live in the roster.
use crate::{app::App, config, crypto, memory};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::path::PathBuf;

pub const MAX_TEXT: usize = 24_000;
const MAX_FILE: usize = 8 * 1024 * 1024;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum Kind {
    Accepted,
    Rejected,
    Edited,
    RoutineFailure,
    Explicit,
    IgnoredAlert,
}
impl Kind {
    pub fn actionable(&self) -> bool {
        *self != Self::IgnoredAlert
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct Origin {
    pub chat_id: String,
    pub message_id: String,
    #[serde(default)]
    pub routine_id: Option<String>,
    #[serde(default)]
    pub review_id: Option<String>,
    #[serde(default)]
    pub task_id: Option<String>,
}
impl Origin {
    pub fn link(&self) -> String {
        format!("lorca://chat/{}/message/{}", self.chat_id, self.message_id)
    }
}

/// What a revision changes: the task of one of the bot's routines, or the instructions of a
/// saved skill in the bot's scope or a group project it belongs to.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum Target {
    RoutinePrompt { id: String },
    Playbook { scope: Scope, id: String },
}
pub use crate::playbooks::Scope;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Feedback {
    pub id: String,
    pub kind: Kind,
    pub origin: Origin,
    pub link: String,
    pub note: String,
    pub example: String,
    pub before: Option<String>,
    pub after: Option<String>,
    pub target: Option<Target>,
    pub created_at: f64,
    pub excluded: bool,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Snapshot {
    pub content: Value,
    pub hash: String,
    pub revision: u64,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Proposal {
    pub id: String,
    pub target: Target,
    pub before: Snapshot,
    pub after: Value,
    pub evidence: Vec<String>,
    pub origins: Vec<Origin>,
    pub explanation: String,
    pub diff: String,
    /// Binds a decision to the exact diff the user saw.
    pub diff_hash: String,
    pub state: String,
    pub created_at: f64,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Revision {
    pub id: String,
    pub version: u64,
    pub proposal_id: String,
    pub target: Target,
    pub before: Snapshot,
    pub after: Value,
    pub state: String,
    pub created_at: f64,
    pub actor_device_id: String,
    pub rollback_of: Option<String>,
}
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Settings {
    /// None means off. Only the user's settings API changes this.
    pub review_every_secs: Option<i64>,
    pub last_review_at: Option<f64>,
    pub excluded_chats: Vec<String>,
    #[serde(default)]
    pub excluded_targets: Vec<Target>,
}
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Store {
    pub feedback: Vec<Feedback>,
    #[serde(default)]
    pub reviewed_ids: Vec<String>,
    pub proposals: Vec<Proposal>,
    pub revisions: Vec<Revision>,
    pub settings: Settings,
}

/// The feedback a bot keeps, oldest first out; a pending proposal keeps the examples it cites.
const MAX_FEEDBACK: usize = 300;
/// Decided proposals kept so a review does not suggest a rejected change again.
const MAX_DECIDED: usize = 100;

impl Store {
    pub fn prune(&mut self) {
        let cited: Vec<String> = self.proposals.iter().filter(|p| p.state == "pending").flat_map(|p| p.evidence.clone()).collect();
        let mut extra = self.feedback.len().saturating_sub(MAX_FEEDBACK);
        self.feedback.retain(|f| {
            let drop = extra > 0 && !cited.contains(&f.id);
            if drop {
                extra -= 1;
            }
            !drop
        });
        let mut extra = self.proposals.iter().filter(|p| p.state != "pending").count().saturating_sub(MAX_DECIDED);
        self.proposals.retain(|p| {
            let drop = extra > 0 && p.state != "pending";
            if drop {
                extra -= 1;
            }
            !drop
        });
        let kept: Vec<&String> = self.feedback.iter().map(|f| &f.id).collect();
        self.reviewed_ids.retain(|id| kept.contains(&id));
    }
}

pub fn path(app: &App, bot_id: &str) -> Result<PathBuf, String> {
    let machine = app
        .machine_file()
        .ok_or("Create or pair an identity first")?;
    Ok(app
        .config
        .home
        .join("feedback")
        .join(memory::hash_text(&machine.identity_pubkey))
        .join(format!("{}.enc", memory::hash_text(bot_id))))
}
pub fn load(app: &App, bot_id: &str) -> Result<Store, String> {
    let path = path(app, bot_id)?;
    match std::fs::read(path) {
        Ok(bytes) => {
            if bytes.len() > MAX_FILE {
                return Err("Feedback store exceeds its size limit".into());
            }
            crypto::decrypt_json(
                &app.dek().ok_or("Missing account key")?,
                "workflow_feedback",
                &bytes,
            )
            .map_err(|e| e.to_string())
        }
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(Store::default()),
        Err(e) => Err(e.to_string()),
    }
}
pub fn save(app: &App, bot_id: &str, store: &Store) -> Result<(), String> {
    let bytes = crypto::encrypt_json(
        &app.dek().ok_or("Missing account key")?,
        "workflow_feedback",
        store,
    )
    .map_err(|e| e.to_string())?;
    if bytes.len() > MAX_FILE {
        return Err(
            "Feedback store is full; exclude sensitive examples before recording more".into(),
        );
    }
    config::write_private(&path(app, bot_id)?, &bytes).map_err(|e| e.to_string())
}
pub fn text(value: &Value) -> String {
    value
        .as_str()
        .map(str::to_string)
        .unwrap_or_else(|| serde_json::to_string_pretty(value).unwrap_or_default())
}
pub fn snapshot(content: Value, revision: u64) -> Snapshot {
    let hash = memory::hash_text(&text(&content));
    Snapshot {
        content,
        hash,
        revision,
    }
}
pub fn clean(text: &str, max: usize) -> String {
    let sanitized = match serde_json::from_str::<Value>(text) {
        Ok(value) => self::text(&scrub_value(&value)),
        Err(_) => memory::scrub(text),
    };
    sanitized.chars().take(max).collect()
}
pub fn diff(before: &Value, after: &Value) -> String {
    let before = text(before);
    let after = text(after);
    let a: Vec<&str> = before.lines().collect();
    let b: Vec<&str> = after.lines().collect();
    let prefix = a.iter().zip(&b).take_while(|(x, y)| x == y).count();
    let suffix = a[prefix..]
        .iter()
        .rev()
        .zip(b[prefix..].iter().rev())
        .take_while(|(x, y)| x == y)
        .count();
    let mut out = format!(
        "--- current\n+++ proposed\n@@ -{},{} +{},{} @@\n",
        prefix + 1,
        a.len() - prefix - suffix,
        prefix + 1,
        b.len() - prefix - suffix
    );
    for line in &a[prefix..a.len() - suffix] {
        out.push_str(&format!("-{line}\n"));
    }
    for line in &b[prefix..b.len() - suffix] {
        out.push_str(&format!("+{line}\n"));
    }
    out
}

/// JSON payloads may include structured credential fields; redact before serializing examples.
fn scrub_value(value: &Value) -> Value {
    match value {
        Value::String(text) => Value::String(memory::scrub(text)),
        Value::Array(items) => Value::Array(items.iter().map(scrub_value).collect()),
        Value::Object(fields) => Value::Object(
            fields
                .iter()
                .map(|(key, value)| {
                    let k = key.to_lowercase().replace(['-', '_'], "");
                    let secret = [
                        "password",
                        "passwd",
                        "secret",
                        "token",
                        "apikey",
                        "authorization",
                        "credential",
                    ]
                    .iter()
                    .any(|s| k.contains(s));
                    (
                        key.clone(),
                        if secret {
                            Value::String("«redacted»".into())
                        } else {
                            scrub_value(value)
                        },
                    )
                })
                .collect(),
        ),
        other => other.clone(),
    }
}
