//! The record of what intake decided about each message, encrypted with the account key and kept
//! on the channel's Runner. Every Device reads it through `events.decisions`, so the channel's
//! sheet can show what the bot would have taken on without the machine blob changing per message.

use rusqlite::params;
use serde::{Deserialize, Serialize};

use super::{Rule, Strength};
use crate::app::App;
use crate::config::now_unix;

const KIND: &str = "intake_decision";
/// The newest kept per channel, and for how long.
const KEEP_PER_CHANNEL: usize = 500;
const KEEP_SECS: i64 = 30 * 86_400;

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum DecisionKind {
    /// A new Task under a Mandate.
    Accepted,
    /// Would have been accepted, in `observe`.
    WouldAccept,
    /// Went to a Task it belongs to, whose next run reads it.
    Attached,
    /// A listed person stopped a Task.
    Stopped,
    /// The channel's own task answers it in an ordinary turn.
    Replied,
    /// Recorded and left alone.
    Ignored,
    /// The assessment could not finish: its model failed, or answered nothing usable.
    Unavailable,
}

/// A rule's judgement in an assessment: each enabled rule gets exactly one, matched or not.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct RuleJudgement {
    pub rule_id: String,
    pub matched: bool,
    pub reason: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Decision {
    pub id: String,
    pub channel_id: String,
    pub at: i64,
    pub kind: DecisionKind,
    pub conversation_id: String,
    /// The contact's messages it decided, oldest first.
    pub message_ids: Vec<String>,
    pub sender: String,
    pub sender_id: String,
    pub strength: Option<Strength>,
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub addressed: bool,
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub forwarded: bool,
    /// The thread it is in on the service, for a conversation that holds a whole Slack channel.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub thread_id: String,
    /// What the work is; empty unless it became or would have become work.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub title: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub scope: String,
    pub reason: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub rules: Vec<RuleJudgement>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub evidence: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub task_id: Option<String>,
    /// Someone entitled wrote it, and no turn of its Task has read it: a running turn takes it at its
    /// next step, else it starts the next run.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub waiting: bool,
    /// Its messages as they are in the Task's work chat, while unread.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub copies: Vec<String>,
    /// Which version of the channel's intake decided it.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub policy_version: String,
}

impl Decision {
    /// A decision's id follows from what it decided, so a message assessed twice is one record.
    pub(super) fn id_for(channel_id: &str, message_ids: &[String]) -> String {
        let digest = <sha2::Sha256 as sha2::Digest>::digest(serde_json::to_vec(&(channel_id, message_ids)).unwrap_or_default());
        format!("decision-{}", &crate::keys::b64(&digest)[..22])
    }

    /// The rules this decision matched, in the owner's words at the time.
    pub(super) fn matched_rules(judged: &[RuleJudgement], rules: &[Rule]) -> Vec<Rule> {
        rules.iter().filter(|rule| judged.iter().any(|judgement| judgement.matched && judgement.rule_id == rule.id)).cloned().collect()
    }
}

/// Keeps a decision, replacing one with the same id, and drops what has aged out of the channel's
/// record.
pub(super) fn save(app: &App, decision: &Decision) -> anyhow::Result<()> {
    let key = anyhow::Context::context(app.dek(), "No identity")?;
    let bytes = crate::crypto::encrypt_json(&key, KIND, decision)?;
    let db = app.store.connection.lock().unwrap();
    db.execute(
        "INSERT OR REPLACE INTO intake_decisions (id, channel_id, at, waiting, ciphertext) VALUES (?1, ?2, ?3, ?4, ?5)",
        params![decision.id, decision.channel_id, decision.at, decision.waiting, bytes],
    )?;
    db.execute(
        "DELETE FROM intake_decisions WHERE channel_id = ?1 AND (at < ?2 OR id NOT IN (SELECT id FROM intake_decisions WHERE channel_id = ?1 ORDER BY at DESC, id DESC LIMIT ?3))",
        params![decision.channel_id, now_unix() - KEEP_SECS, KEEP_PER_CHANNEL as i64],
    )?;
    Ok(())
}

/// A channel's decisions, newest first.
pub fn list(app: &App, channel_id: &str, limit: usize) -> anyhow::Result<Vec<Decision>> {
    let key = anyhow::Context::context(app.dek(), "No identity")?;
    let db = app.store.connection.lock().unwrap();
    let mut statement = db.prepare("SELECT ciphertext FROM intake_decisions WHERE channel_id = ?1 ORDER BY at DESC, id DESC LIMIT ?2")?;
    let rows = statement.query_map(params![channel_id, limit as i64], |row| row.get::<_, Vec<u8>>(0))?;
    let mut decisions = Vec::new();
    for bytes in rows {
        decisions.push(crate::crypto::decrypt_json(&key, KIND, &bytes?)?);
    }
    Ok(decisions)
}

/// The decisions waiting for their Task's next run, oldest first, across every channel.
pub(super) fn waiting(app: &App) -> anyhow::Result<Vec<Decision>> {
    let key = anyhow::Context::context(app.dek(), "No identity")?;
    let db = app.store.connection.lock().unwrap();
    let mut statement = db.prepare("SELECT ciphertext FROM intake_decisions WHERE waiting = 1 ORDER BY at ASC, id ASC")?;
    let rows = statement.query_map([], |row| row.get::<_, Vec<u8>>(0))?;
    let mut decisions = Vec::new();
    for bytes in rows {
        decisions.push(crate::crypto::decrypt_json(&key, KIND, &bytes?)?);
    }
    Ok(decisions)
}

/// Removes a channel's record with the channel.
pub fn forget_channel(app: &App, channel_id: &str) -> anyhow::Result<()> {
    let db = app.store.connection.lock().unwrap();
    db.execute("DELETE FROM intake_decisions WHERE channel_id = ?1", params![channel_id])?;
    Ok(())
}
