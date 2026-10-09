//! Durable, version-bound proposals owned by one Runner. Only ciphertext is persisted or
//! synced; paired Devices read it and send decisions back to that Runner.

use std::sync::Arc;

use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};

use crate::app::{App, OutboxItem, Slot};
use crate::config::now_secs;
use crate::events::Event;
use crate::model::{Author, Body, Message};

const MAX_ITEM_BYTES: usize = 512 * 1024;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ReviewOrigin {
    pub chat_id: String,
    pub message_id: Option<String>,
    pub routine_id: Option<String>,
    /// The canonical task id from the task subject; the queue owns no task records.
    pub task_id: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ReviewTarget {
    pub account: String,
    pub resource: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum ReviewPayload {
    /// Accepting a draft records the accepted text without publishing it to a service.
    Draft {
        text: String,
    },
    Shell {
        arguments: Value,
    },
    Plugin {
        plugin_id: String,
        server_name: String,
        tool: String,
        arguments: Value,
    },
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ReviewedFile {
    pub path: String,
    /// None means the file does not exist. Creating it also invalidates the review.
    pub hash: Option<String>,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct ReviewPreconditions {
    pub authorization_hash: String,
    pub workdir: String,
    pub connection_hash: Option<String>,
    pub tool_hash: Option<String>,
    pub files: Vec<ReviewedFile>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ReviewState {
    Pending,
    Approved,
    Executing,
    Succeeded,
    Failed,
    Rejected,
    Cancelled,
    Uncertain,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ReviewChange {
    Created,
    Edited,
    Approved,
    Rejected,
    Cancelled,
    Invalidated,
    Executed,
    Interrupted,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ReviewActivity {
    pub id: String,
    pub change: ReviewChange,
    pub version: u64,
    pub actor_device_id: String,
    pub at: f64,
    pub previous_payload: Option<ReviewPayload>,
    pub payload: ReviewPayload,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ReviewApproval {
    pub version: u64,
    pub digest: String,
    pub device_id: String,
    pub at: f64,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ReviewOutcome {
    pub summary: String,
    pub result: Option<Value>,
    pub message_id: String,
    pub at: f64,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ReviewItem {
    pub id: String,
    pub runner_id: String,
    pub bot_id: String,
    /// Immutable receipt binding a create request id to its original proposal.
    pub request_hash: String,
    pub origin: ReviewOrigin,
    pub target: ReviewTarget,
    pub rationale: String,
    pub payload: ReviewPayload,
    /// Changes whenever the reviewed payload, context, or preconditions change.
    pub version: u64,
    /// Changes on every persisted state transition, including execution claims.
    pub revision: u64,
    pub preconditions: ReviewPreconditions,
    pub state: ReviewState,
    pub approval: Option<ReviewApproval>,
    pub outcome: Option<ReviewOutcome>,
    pub history: Vec<ReviewActivity>,
    pub created_at: f64,
    pub updated_at: f64,
}

impl ReviewItem {
    pub fn message_id(&self) -> String {
        format!("review-status-{}", self.id)
    }

    /// A task's `review` evidence: the outcome notice by its stable id. Linking it never
    /// completes a task.
    pub fn outcome_evidence(&self) -> Value {
        json!({ "kind": "review", "label": self.outcome.as_ref().map(|outcome| outcome.summary.as_str()).unwrap_or("Review item"),
            "review_id": self.id, "chat_id": self.origin.chat_id, "message_id": self.message_id() })
    }

    /// What it does, in a line: the command, the call, or the draft's subject.
    pub fn summary(&self) -> String {
        match &self.payload {
            ReviewPayload::Shell { arguments } => {
                let command = arguments["command"].as_str().unwrap_or("");
                format!("$ {}", command.lines().map(str::trim).find(|line| !line.is_empty()).unwrap_or(command))
            }
            ReviewPayload::Plugin { .. } => format!("{}: {}", self.target.account, self.target.resource),
            ReviewPayload::Draft { .. } => format!("Draft: {}", self.target.resource),
        }
    }

    /// "Waiting for your review · $ git push", then what came of it: the output, the accepted
    /// draft, or why it needs another look.
    pub fn notice(&self) -> String {
        let outcome = self.outcome.as_ref();
        let result = outcome.and_then(|outcome| outcome.result.as_ref()).and_then(|result| result["text"].as_str()).unwrap_or("");
        let (state, detail) = match self.state {
            ReviewState::Pending if outcome.is_some() => ("Needs your review again", outcome.map(|outcome| outcome.summary.as_str()).unwrap_or("")),
            ReviewState::Pending => ("Waiting for your review", ""),
            ReviewState::Approved => ("Approved", ""),
            ReviewState::Executing => ("Running", ""),
            ReviewState::Succeeded if matches!(self.payload, ReviewPayload::Draft { .. }) => ("Accepted", result),
            ReviewState::Succeeded => ("Done", result),
            ReviewState::Failed => ("Failed", result),
            ReviewState::Rejected => ("Rejected", ""),
            ReviewState::Cancelled => ("Cancelled", ""),
            ReviewState::Uncertain => ("Didn't finish", outcome.map(|outcome| outcome.summary.as_str()).unwrap_or("")),
        };
        let detail: String = detail.trim().chars().take(2000).collect();
        if detail.is_empty() { format!("{state} · {}", self.summary()) } else { format!("{state} · {}\n{detail}", self.summary()) }
    }

    pub fn reviewed_digest(&self) -> String {
        fingerprint(
            &json!({ "id": self.id, "runner_id": self.runner_id, "bot_id": self.bot_id, "origin": self.origin,
            "version": self.version, "payload": self.payload, "target": self.target, "rationale": self.rationale, "preconditions": self.preconditions }),
        )
    }

    pub(crate) fn record(
        &mut self,
        change: ReviewChange,
        actor: &str,
        previous_payload: Option<ReviewPayload>,
    ) {
        self.revision += 1;
        self.updated_at = now_secs();
        self.history.push(ReviewActivity {
            id: uuid::Uuid::new_v4().to_string(),
            change,
            version: self.version,
            actor_device_id: actor.into(),
            at: self.updated_at,
            previous_payload,
            payload: self.payload.clone(),
        });
    }
}

pub fn fingerprint(value: &impl Serialize) -> String {
    crate::keys::b64(&Sha256::digest(
        serde_json::to_vec(value).expect("serializable review value"),
    ))
}

fn key(app: &App) -> Result<[u8; 32], String> {
    app.dek()
        .ok_or_else(|| "Pair or create an identity first.".into())
}

pub(crate) fn load(app: &App, id: &str) -> Result<(ReviewItem, Vec<u8>), String> {
    let raw = app
        .store
        .review(id)
        .map_err(|error| error.to_string())?
        .ok_or("Unknown review item")?;
    let item: ReviewItem = crate::crypto::decrypt_json(&key(app)?, "review", &raw)
        .map_err(|error| error.to_string())?;
    if item.id != id {
        return Err("Review id does not match its encrypted record".into());
    }
    Ok((item, raw))
}

pub fn get(app: &App, id: &str) -> Result<ReviewItem, String> {
    load(app, id).map(|(item, _)| item)
}

pub fn list(app: &App) -> Result<Vec<ReviewItem>, String> {
    let Some(dek) = app.dek() else {
        return Ok(Vec::new());
    };
    let mut items = app
        .store
        .reviews()
        .map_err(|error| error.to_string())?
        .iter()
        .map(|raw| {
            crate::crypto::decrypt_json::<ReviewItem>(&dek, "review", raw)
                .map_err(|error| error.to_string())
        })
        .collect::<Result<Vec<_>, _>>()?;
    items.sort_by(|a, b| {
        b.updated_at
            .total_cmp(&a.updated_at)
            .then_with(|| a.id.cmp(&b.id))
    });
    Ok(items)
}

/// Called with the review lock held. The item and its encrypted relay outbox entry commit in
/// one SQLite transaction, before a decision is acknowledged or an external action starts.
pub(crate) fn save(
    app: &App,
    item: &ReviewItem,
    previous: Option<&[u8]>,
    change: ReviewChange,
) -> Result<(), String> {
    let plain = serde_json::to_vec(item).map_err(|error| error.to_string())?;
    // Reserve the remaining payload snapshots and bounded outcome before admission.
    let limit = match item.state {
        ReviewState::Pending => MAX_ITEM_BYTES - 144 * 1024,
        ReviewState::Approved | ReviewState::Executing => MAX_ITEM_BYTES - 96 * 1024,
        _ => MAX_ITEM_BYTES,
    };
    if plain.len() > limit {
        return Err("Review item is too large; create a separate proposal.".into());
    }
    let ciphertext =
        crate::crypto::encrypt(&key(app)?, "review", &plain).map_err(|error| error.to_string())?;
    let upload = OutboxItem {
        id: uuid::Uuid::new_v4().to_string(),
        kind: "review".into(),
        recipient: None,
        ciphertext: ciphertext.clone(),
        slot: Some(Slot::latest(crate::model::relay_name(&format!(
            "review/{}",
            item.id
        )))),
        group: None,
    };
    app.store
        .save_review(&item.id, previous, &ciphertext, Some(&upload))
        .map_err(|error| error.to_string())?;
    app.wake_sync();
    app.emit(Event::ReviewChanged {
        item: item.clone(),
        change,
    });
    Ok(())
}

/// Projects the newest status into the originating chat under a stable message id, where the
/// bot's later turns read it. The encrypted review stays authoritative if writing it is cut short.
pub(crate) fn publish_origin(app: &App, id: &str) {
    let Ok(item) = get(app, id) else { return };
    if app.chat(&item.origin.chat_id).is_none() {
        return;
    }
    let mut message = Message::new(
        &item.origin.chat_id,
        Author::System,
        Body::Notice {
            text: item.notice(),
            routine_id: None,
        },
    );
    message.id = item.message_id();
    message.created_at = item.created_at;
    app.upsert_message(message, true);
}

/// A newer encrypted projection arrives. A Runner's own durable claims are authoritative;
/// other Devices never execute a synced approval.
pub fn apply(app: &App, ciphertext: &[u8]) -> Result<(), String> {
    let item: ReviewItem = crate::crypto::decrypt_json(&key(app)?, "review", ciphertext)
        .map_err(|error| error.to_string())?;
    if item.id.is_empty() || item.version == 0 || item.revision == 0 {
        return Err("Invalid review record".into());
    }
    let _lock = app.review_lock.lock().unwrap();
    let previous = app
        .store
        .review(&item.id)
        .map_err(|error| error.to_string())?;
    if let Some(raw) = &previous {
        let old: ReviewItem = crate::crypto::decrypt_json(&key(app)?, "review", raw)
            .map_err(|error| error.to_string())?;
        if old.runner_id != item.runner_id
            || old.revision >= item.revision
            || app.this_device_id().as_deref() == Some(item.runner_id.as_str())
        {
            return Ok(());
        }
    } else if app.this_device_id().as_deref() == Some(item.runner_id.as_str()) {
        // A restored Runner may display its old items, but cannot replay an old approval.
        let mut restored = item.clone();
        if matches!(
            restored.state,
            ReviewState::Approved | ReviewState::Executing
        ) {
            restored.state = ReviewState::Uncertain;
            restored.approval = None;
            restored.outcome = Some(ReviewOutcome { summary: "This Runner was restored while this was approved. Check whether it ran before trying again.".into(),
                result: None, message_id: restored.message_id(), at: now_secs() });
            restored.record(ReviewChange::Interrupted, &item.runner_id, None);
            return save(app, &restored, None, ReviewChange::Interrupted);
        }
    }
    app.store
        .save_review(&item.id, previous.as_deref(), ciphertext, None)
        .map_err(|error| error.to_string())?;
    let change = item
        .history
        .last()
        .map(|entry| entry.change)
        .unwrap_or(ReviewChange::Created);
    app.emit(Event::ReviewChanged { item, change });
    Ok(())
}

/// Re-publish owned records after a relay/account resync without letting stale projections
/// overwrite local state.
pub fn enqueue_owned(app: &App) {
    let Ok(items) = list(app) else { return };
    for item in items
        .into_iter()
        .filter(|item| app.this_device_id().as_deref() == Some(item.runner_id.as_str()))
    {
        if let Ok(raw) = crate::crypto::encrypt_json(&app.dek().unwrap(), "review", &item) {
            app.push_slot_blob(
                "review",
                Slot::latest(crate::model::relay_name(&format!("review/{}", item.id))),
                None,
                raw,
            );
        }
    }
}

/// Once an item staged for a task has ended, its outcome notice joins the task's evidence through
/// the task's own revision check. Retried once when the task moved meanwhile; a refusal (the task
/// ended, the chat left it) leaves the outcome on the item and in the chat.
pub(crate) async fn record_on_task(app: &Arc<App>, item: &ReviewItem) {
    let Some(task_id) = &item.origin.task_id else { return };
    if item.outcome.is_none() || matches!(item.state, ReviewState::Pending | ReviewState::Approved | ReviewState::Executing) {
        return;
    }
    for _ in 0..2 {
        let Ok(task) = crate::tasks::get(app, task_id) else { return };
        if task.evidence.iter().any(|evidence| evidence.review_id.as_deref() == Some(item.id.as_str())) {
            return;
        }
        let mut evidence = serde_json::to_value(&task.evidence).unwrap_or_else(|_| json!([]));
        if let Some(list) = evidence.as_array_mut() {
            list.push(item.outcome_evidence());
        }
        let update = json!({ "id": task_id, "expected_revision": task.revision,
            "request_id": format!("{}-outcome-{}", item.id, task.revision), "evidence": evidence });
        match Box::pin(crate::tasks::dispatch(app, "tasks.update", update)).await {
            Ok(_) => return,
            Err(error) if error.contains("revision conflict") => continue,
            Err(error) => {
                tracing::warn!(%error, review_id = %item.id, "recording a review on its task");
                return;
            }
        }
    }
}

#[cfg(feature = "runner")]
pub(crate) fn local_bot(app: &App, item: &ReviewItem) -> Result<crate::model::Bot, String> {
    if app.this_device_id().as_deref() != Some(item.runner_id.as_str()) {
        return Err("This review belongs to another Runner.".into());
    }
    let bot = app
        .bot(&item.bot_id)
        .ok_or("The originating bot was deleted.")?;
    if bot.runner_id != item.runner_id {
        return Err(
            "The bot's Runner assignment changed; create a new review on its current Runner."
                .into(),
        );
    }
    let chat = app
        .chat(&item.origin.chat_id)
        .ok_or("The originating chat was deleted.")?;
    if !chat.meta.bot_ids.contains(&bot.id) {
        return Err("The bot is no longer in the originating chat.".into());
    }
    if let Some(id) = &item.origin.routine_id {
        let routine = app
            .routine(id)
            .ok_or("The originating routine was deleted.")?;
        if routine.bot_id != bot.id {
            return Err("The originating routine changed owner.".into());
        }
    }
    Ok(bot)
}

pub(crate) fn require_version(item: &ReviewItem, params: &Value) -> Result<(), String> {
    if params["expected_version"].as_u64() != Some(item.version) {
        return Err("This changed on another Device. Review it again.".into());
    }
    Ok(())
}

fn require_actor(app: &App, actor: &str) -> Result<(), String> {
    if actor.is_empty()
        || (app.this_device_id().as_deref() != Some(actor) && app.device(actor).is_none())
    {
        return Err("This decision requires a paired Device.".into());
    }
    Ok(())
}

/// App/local-core entry point. Reads work from the encrypted local projection even offline;
/// mutations are sealed to the one Runner that owns the review.
pub async fn dispatch(app: &Arc<App>, method: &str, params: Value) -> Result<Value, String> {
    match method {
        "reviews.list" => {
            let items = list(app)?
                .into_iter()
                .filter(|item| {
                    params["chat_id"]
                        .as_str()
                        .is_none_or(|chat| chat == item.origin.chat_id)
                        && params["task_id"]
                            .as_str()
                            .is_none_or(|task| item.origin.task_id.as_deref() == Some(task))
                })
                .collect::<Vec<_>>();
            return Ok(json!(items));
        }
        "reviews.get" => return Ok(json!(get(app, params["id"].as_str().ok_or("missing id")?)?)),
        _ => {}
    }
    let runner = if method == "reviews.create" {
        app.bot(params["bot_id"].as_str().ok_or("missing bot_id")?)
            .ok_or("Unknown bot")?
            .runner_id
    } else {
        get(app, params["id"].as_str().ok_or("missing id")?)?.runner_id
    };
    if app.this_device_id().as_deref() != Some(&runner) {
        return crate::requests::ask(app, &runner, method, params).await;
    }
    serve(
        app,
        method,
        &params,
        &app.this_device_id().unwrap_or_default(),
    )
    .await
}

pub async fn serve(
    app: &Arc<App>,
    method: &str,
    params: &Value,
    actor: &str,
) -> Result<Value, String> {
    require_actor(app, actor)?;
    #[cfg(feature = "runner")]
    if method == "reviews.create" || method == "reviews.edit" || method == "reviews.approve" {
        return crate::review_execution::mutate(app, method, params, actor)
            .await
            .map(|item| json!(item));
    }
    if method != "reviews.reject" && method != "reviews.cancel" {
        return Err("This Runner does not support that review action.".into());
    }
    let id = params["id"].as_str().ok_or("missing id")?;
    let item = {
        let _lock = app.review_lock.lock().unwrap();
        let (mut item, previous) = load(app, id)?;
        require_version(&item, params)?;
        if app.this_device_id().as_deref() != Some(item.runner_id.as_str()) {
            return Err("This review belongs to another Runner.".into());
        }
        let (state, change) = if method == "reviews.reject" {
            (ReviewState::Rejected, ReviewChange::Rejected)
        } else {
            (ReviewState::Cancelled, ReviewChange::Cancelled)
        };
        if item.state == state {
            return Ok(json!(item));
        }
        if !matches!(item.state, ReviewState::Pending | ReviewState::Approved) {
            return Err("This already ran or was decided.".into());
        }
        item.state = state;
        item.approval = None;
        item.outcome = Some(ReviewOutcome {
            summary: params["reason"]
                .as_str()
                .filter(|reason| !reason.trim().is_empty())
                .map(str::to_string)
                .unwrap_or_else(|| {
                    if state == ReviewState::Rejected {
                        "Rejected".into()
                    } else {
                        "Cancelled".into()
                    }
                }),
            result: None,
            message_id: item.message_id(),
            at: now_secs(),
        });
        item.record(change, actor, None);
        save(app, &item, Some(&previous), change)?;
        item
    };
    publish_origin(app, id);
    record_on_task(app, &item).await;
    Ok(json!(item))
}
