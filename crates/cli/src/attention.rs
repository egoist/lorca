//! The account's encrypted coordinator attention projection. Task and review ids are links,
//! never duplicate work records. Reporter observations merge by a stable underlying key.

use crate::{
    app::{App, Slot},
    config::now_secs,
    events::Event,
    model::{Author, Body, Job, Message},
};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::collections::{BTreeMap, BTreeSet};
use std::sync::{Arc, Mutex};

static WRITES: Mutex<()> = Mutex::new(());
const KIND: &str = "attention";

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Notification {
    Summary,
    Urgent,
    Quiet,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Preferences {
    pub summaries: bool,
    pub urgent_direct: bool,
    #[serde(default)]
    pub default_coordinator_bot_id: Option<String>,
}
impl Default for Preferences {
    fn default() -> Self {
        Self {
            summaries: true,
            urgent_direct: true,
            default_coordinator_bot_id: None,
        }
    }
}

/// Lamport order avoids one Runner's clock undoing another Device's resolution.
#[derive(Debug, Clone, Default, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub struct Revision {
    pub counter: u64,
    pub device_id: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Source {
    pub chat_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub task_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub message_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub review_id: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Category {
    Review,
    Blocker,
    Commitment,
    Change,
}

impl Category {
    fn word(self) -> &'static str {
        match self {
            Category::Review => "Review",
            Category::Blocker => "Blocker",
            Category::Commitment => "Commitment",
            Category::Change => "Change",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Item {
    pub id: String,
    pub key: String,
    pub category: Category,
    pub title: String,
    pub summary: String,
    pub next_action: String,
    pub coordinator_bot_id: String,
    pub sources: Vec<Source>,
    pub reporters: Vec<String>,
    pub urgent: bool,
    pub resolved: bool,
    pub revision: Revision,
    pub updated_at: f64,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Brief {
    pub coordinator_bot_id: String,
    pub chat_id: String,
    pub decisions: Vec<String>,
    pub changes: Vec<String>,
    pub next_action: String,
    pub item_ids: Vec<String>,
    pub message_id: String,
    pub revision: Revision,
    pub updated_at: f64,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct View {
    pub items: Vec<Item>,
    pub briefs: Vec<Brief>,
    pub preferences: Preferences,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
enum Content {
    Observation { item: Item },
    Brief { brief: Brief },
    Preferences { preferences: Preferences },
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
struct Record {
    id: String,
    revision: Revision,
    content: Content,
}

/// Shared hook for tasks/reviews/handoffs: retain canonical references and a stable topic key.
#[derive(Debug, Clone, Deserialize)]
pub struct Report {
    pub key: String,
    pub category: Category,
    pub title: String,
    pub summary: String,
    pub next_action: String,
    pub source: Source,
    #[serde(default)]
    pub coordinator_bot_id: Option<String>,
    #[serde(default)]
    pub urgent: bool,
    #[serde(default)]
    pub quiet: bool,
}

fn hash(value: &str) -> String {
    crate::model::relay_name(&format!("attention:{value}"))
}

/// Stable projection id for a source subject/topic; adapters need no second id mapping.
pub fn item_id(source: &Source, key: &str) -> String {
    let scope = source.task_id.as_deref().unwrap_or(&source.chat_id);
    hash(&serde_json::to_string(&(scope, key.trim())).unwrap())
}
fn records(app: &App) -> Result<Vec<Record>, String> {
    let Some(dek) = app.dek() else {
        return Ok(vec![]);
    };
    app.store
        .attention_rows()
        .map_err(|e| e.to_string())?
        .into_iter()
        .map(|(id, bytes)| {
            let record: Record =
                crate::crypto::decrypt_json(&dek, KIND, &bytes).map_err(|e| e.to_string())?;
            if record.id != id {
                return Err("Attention record id mismatch".into());
            }
            Ok(record)
        })
        .collect()
}

fn next_revision(app: &App, records: &[Record]) -> Result<Revision, String> {
    Ok(Revision {
        counter: records
            .iter()
            .map(|record| record.revision.counter)
            .max()
            .unwrap_or(0)
            .checked_add(1)
            .ok_or("Attention revision exhausted")?,
        device_id: app.this_device_id().ok_or("Pair this Device first")?,
    })
}

fn preferences_of(records: &[Record]) -> Preferences {
    records
        .iter()
        .filter_map(|record| match &record.content {
            Content::Preferences { preferences } => Some((&record.revision, preferences)),
            _ => None,
        })
        .max_by_key(|(revision, _)| *revision)
        .map(|(_, preferences)| preferences.clone())
        .unwrap_or_default()
}

fn current_preferences(app: &App, records: &[Record]) -> Preferences {
    let mut preferences = preferences_of(records);
    preferences.default_coordinator_bot_id = preferences
        .default_coordinator_bot_id
        .filter(|bot| app.bot(bot).is_some());
    preferences
}

fn project(app: &App, records: &[Record], include_resolved: bool) -> View {
    let mut groups: BTreeMap<String, Vec<&Item>> = BTreeMap::new();
    let mut briefs = Vec::new();
    for record in records {
        match &record.content {
            Content::Observation { item } => groups.entry(item.id.clone()).or_default().push(item),
            Content::Brief { brief }
                if app.bot(&brief.coordinator_bot_id).is_some()
                    && app.chat(&brief.chat_id).is_some() =>
            {
                briefs.push(brief.clone())
            }
            _ => {}
        }
    }
    let mut items = Vec::new();
    for observations in groups.values() {
        // A resolved key is terminal. An offline reporter, even with a higher local clock,
        // cannot resurrect it when its queued report finally arrives.
        let latest = observations
            .iter()
            .filter(|item| item.resolved)
            .max_by_key(|item| &item.revision)
            .or_else(|| observations.iter().max_by_key(|item| &item.revision))
            .unwrap();
        if latest.resolved && !include_resolved {
            continue;
        }
        let mut item = (*latest).clone();
        item.sources.clear();
        let mut reporters = BTreeSet::new();
        for observation in observations {
            for source in &observation.sources {
                if app.chat(&source.chat_id).is_some() && !item.sources.contains(source) {
                    item.sources.push(source.clone())
                }
            }
            reporters.extend(observation.reporters.iter().cloned());
        }
        item.reporters = reporters.into_iter().collect();
        if !item.sources.is_empty() && app.bot(&item.coordinator_bot_id).is_some() {
            items.push(item)
        }
    }
    items.sort_by(|a, b| b.urgent.cmp(&a.urgent).then(b.revision.cmp(&a.revision)));
    briefs.sort_by(|a, b| b.revision.cmp(&a.revision));
    View {
        items,
        briefs,
        preferences: current_preferences(app, records),
    }
}

pub fn view(app: &App) -> Result<View, String> {
    Ok(project(app, &records(app)?, false))
}
pub fn preferences(app: &App) -> Preferences {
    records(app)
        .map(|records| current_preferences(app, &records))
        .unwrap_or_default()
}
pub fn allows(app: &App, notification: Notification) -> bool {
    let preferences = preferences(app);
    match notification {
        Notification::Summary => preferences.summaries,
        Notification::Urgent => preferences.urgent_direct,
        Notification::Quiet => false,
    }
}

fn save(app: &App, record: &Record) -> Result<(), String> {
    let dek = app.dek().ok_or("Pair this Device first")?;
    let ciphertext = crate::crypto::encrypt_json(&dek, KIND, record).map_err(|e| e.to_string())?;
    let item = crate::app::OutboxItem {
        id: uuid::Uuid::new_v4().to_string(),
        kind: KIND.into(),
        recipient: None,
        ciphertext: ciphertext.clone(),
        slot: Some(Slot::latest(record.id.clone())),
        group: None,
    };
    app.store
        .queue_attention(&record.id, &ciphertext, &item)
        .map_err(|e| e.to_string())?;
    app.outbox_notify.notify_waiters();
    Ok(())
}
pub fn changed(app: &App) {
    match view(app) {
        Ok(view) => app.emit(Event::AttentionChanged(view)),
        Err(error) => tracing::warn!(%error, "reading attention"),
    }
}

/// Applying sync never wakes bots or emits alerts. Only the originating Runner does so.
pub fn receive(app: &App, ciphertext: &[u8]) -> Result<(), String> {
    let _write = WRITES.lock().unwrap();
    let dek = app.dek().ok_or("Pair this Device first")?;
    let incoming: Record =
        crate::crypto::decrypt_json(&dek, KIND, ciphertext).map_err(|e| e.to_string())?;
    let known = records(app)?;
    if known
        .iter()
        .any(|record| record.id == incoming.id && record.revision >= incoming.revision)
    {
        return Ok(());
    }
    app.store
        .put_attention(&incoming.id, ciphertext)
        .map_err(|e| e.to_string())?;
    drop(_write);
    changed(app);
    Ok(())
}

pub fn push_history(app: &App) -> Result<(), String> {
    for (id, ciphertext) in app.store.attention_rows().map_err(|e| e.to_string())? {
        app.push_slot_blob(KIND, Slot::latest(id), None, ciphertext);
    }
    Ok(())
}

fn text(value: &str, name: &str, limit: usize) -> Result<String, String> {
    let value = value.trim();
    if value.is_empty() || value.chars().count() > limit {
        return Err(format!("{name} must contain 1–{limit} characters"));
    }
    Ok(value.into())
}

pub fn coordinator(app: &App, chat_id: &str) -> Option<String> {
    let chat = app.chat(chat_id)?;
    let preferences = preferences(app);
    [
        chat.meta.owner().map(str::to_string),
        preferences.default_coordinator_bot_id,
        chat.meta.bot_ids.first().cloned(),
    ]
    .into_iter()
    .flatten()
    .find(|id| app.bot(id).is_some())
}

/// actor is the reporting bot's id; a user API call uses None. No work is executed here.
pub fn report(
    app: &Arc<App>,
    report: Report,
    actor: Option<&str>,
    hops: u32,
) -> Result<Item, String> {
    let quiet = report.quiet;
    let (item, significant) = record(app, report, actor)?;
    if significant && !quiet {
        if item.urgent {
            post_item_alert(app, &item, actor)
        }
        if let Some(actor) = actor.filter(|id| *id != item.coordinator_bot_id) {
            wake_coordinator(app, &item, actor, hops)
        }
    }
    Ok(view(app)?
        .items
        .into_iter()
        .find(|current| current.id == item.id)
        .unwrap_or(item))
}

/// Saves a reporter's observation, and says whether it changed what the item says.
fn record(app: &App, mut report: Report, actor: Option<&str>) -> Result<(Item, bool), String> {
    report.key = text(&report.key, "key", 200)?;
    report.title = text(&report.title, "title", 200)?;
    report.summary = text(&report.summary, "summary", 4000)?;
    report.next_action = text(&report.next_action, "next_action", 1000)?;
    if app.chat(&report.source.chat_id).is_none() {
        return Err("Source chat is gone".into());
    }
    if let Some(id) = &report.source.task_id {
        if crate::tasks::get(app, id).is_err() {
            return Err("Use the id of an existing task from tasks.get".into());
        }
    }
    if let Some(actor) = actor {
        if app.bot(actor).is_none() {
            return Err("Reporting bot is gone".into());
        }
    }
    let coordinator_bot_id = report
        .coordinator_bot_id
        .clone()
        .or_else(|| coordinator(app, &report.source.chat_id))
        .ok_or("Choose a coordinator first")?;
    if app.bot(&coordinator_bot_id).is_none() {
        return Err("Coordinator bot is gone".into());
    }
    let item_id = item_id(&report.source, &report.key);
    let reporter = actor.unwrap_or("user").to_string();
    let _write = WRITES.lock().unwrap();
    let known = records(app)?;
    let old = project(app, &known, true)
        .items
        .into_iter()
        .find(|item| item.id == item_id);
    if old.as_ref().is_some_and(|item| item.resolved) {
        return Err("This attention item is resolved; use a new key for new work".into());
    }
    let revision = next_revision(app, &known)?;
    let item = Item {
        id: item_id.clone(),
        key: report.key,
        category: report.category,
        title: report.title,
        summary: report.summary,
        next_action: report.next_action,
        coordinator_bot_id,
        sources: vec![report.source],
        reporters: vec![reporter.clone()],
        urgent: report.urgent,
        resolved: false,
        revision: revision.clone(),
        updated_at: now_secs(),
    };
    let significant = old.as_ref().is_none_or(|old| {
        old.title != item.title
            || old.summary != item.summary
            || old.next_action != item.next_action
            || old.category != item.category
            || old.urgent != item.urgent
            || old.coordinator_bot_id != item.coordinator_bot_id
    });
    let record = Record {
        id: hash(&format!("{item_id}:{reporter}")),
        revision,
        content: Content::Observation { item: item.clone() },
    };
    // Repeated reports from one bot are no-ops, including their revision and relay write.
    if known.iter().any(|known| known.id == record.id && matches!(&known.content, Content::Observation { item: previous } if previous.title == item.title && previous.summary == item.summary && previous.next_action == item.next_action && previous.category == item.category && previous.urgent == item.urgent && previous.sources == item.sources && previous.coordinator_bot_id == item.coordinator_bot_id)) {
        return Ok((old.unwrap_or(item), false));
    }
    save(app, &record)?;
    drop(_write);
    changed(app);
    Ok((item, significant))
}

/// What a producer (a durable task, a review, a handoff) has the user look at: one active item
/// under a topic `prefix` for the source's task or chat, reported again with fresh words while it
/// lasts, and a new one under `prefix` + `fresh` once the last was resolved. It is quiet: the
/// producer already tells the bots and the user in its own way, so the item only joins the list
/// and the coordinator's prompt. Words past the limits are cut, and a refusal (a deleted chat or
/// bot) only logs.
pub fn raise(app: &App, prefix: &str, fresh: &str, mut report: Report, actor: Option<&str>) {
    report.key = active(app, &report.source, prefix)
        .first()
        .map(|item| item.key.clone())
        .unwrap_or_else(|| format!("{prefix}{fresh}"));
    for (text, limit) in [(&mut report.title, 200), (&mut report.summary, 4000), (&mut report.next_action, 1000)] {
        *text = text.trim().chars().take(limit).collect();
    }
    report.quiet = true;
    if let Err(error) = record(app, report, actor) {
        tracing::debug!(%error, prefix, "raising attention");
    }
}

/// Resolves the active items a producer raised under `prefix` for this source's task or chat, as
/// the work moves on: a review decided, a task unblocked or done, a handoff followed up.
pub fn settle(app: &App, source: &Source, prefix: &str) {
    for item in active(app, source, prefix) {
        if let Err(error) = resolve(app, &item.id, None, None) {
            tracing::warn!(%error, prefix, "settling attention");
        }
    }
}

fn active(app: &App, source: &Source, prefix: &str) -> Vec<Item> {
    view(app)
        .map(|view| view.items)
        .unwrap_or_default()
        .into_iter()
        .filter(|item| item.key.starts_with(prefix) && item.id == item_id(source, &item.key))
        .collect()
}

/// Resolution changes this projection only: tasks and review outcomes keep their own CAS.
pub fn resolve(
    app: &App,
    id: &str,
    actor: Option<&str>,
    expected: Option<&Revision>,
) -> Result<Item, String> {
    let _write = WRITES.lock().unwrap();
    let known = records(app)?;
    let mut item = project(app, &known, true)
        .items
        .into_iter()
        .find(|item| item.id == id)
        .ok_or("Unknown attention item")?;
    if actor.is_some_and(|actor| actor != item.coordinator_bot_id) {
        return Err("Only the owning coordinator or the user can resolve this item".into());
    }
    if item.resolved {
        return Ok(item);
    }
    if expected.is_some_and(|expected| *expected != item.revision) {
        return Err("This item changed on another Device. Look at it again before resolving it.".into());
    }
    item.resolved = true;
    item.revision = next_revision(app, &known)?;
    item.updated_at = now_secs();
    save(
        app,
        &Record {
            id: hash(&format!("{}:resolution", item.id)),
            revision: item.revision.clone(),
            content: Content::Observation { item: item.clone() },
        },
    )?;
    drop(_write);
    changed(app);
    Ok(item)
}

fn post_text(
    app: &Arc<App>,
    bot_id: &str,
    chat_id: &str,
    body: String,
    notification: Notification,
) -> Message {
    let mut message = Message::new(
        chat_id,
        Author::Bot {
            bot_id: bot_id.into(),
        },
        Body::text(body),
    );
    message.notification = Some(notification);
    app.upsert_message(message.clone(), true);
    crate::push::attention(app, &message);
    message
}
fn post_item_alert(app: &Arc<App>, item: &Item, actor: Option<&str>) {
    let bot_id = actor.unwrap_or(&item.coordinator_bot_id);
    let source = &item.sources[0];
    post_text(
        app,
        bot_id,
        &source.chat_id,
        format!(
            "Urgent: {}\n{}\nNext: {}",
            item.title, item.summary, item.next_action
        ),
        Notification::Urgent,
    );
}

fn wake_coordinator(app: &Arc<App>, item: &Item, reporter: &str, hops: u32) {
    if hops >= crate::model::MAX_BOT_HOPS {
        return;
    }
    let Ok(dm) = app.dm_with(&item.coordinator_bot_id, None) else {
        return;
    };
    let mut incoming = Message::new(&dm.meta.id, Author::Bot { bot_id: reporter.into() }, Body::Handoff {
        from: reporter.into(), to: item.coordinator_bot_id.clone(),
        // The transcript's "Message from" marker shows these words; the job's system prompt
        // says what the coordinator does with them.
        reason: format!("{}: {}\n{}\nNext: {}", item.category.word(), item.title, item.summary, item.next_action),
    });
    incoming.notification = Some(Notification::Quiet);
    app.upsert_message(incoming.clone(), true);
    crate::runtime::start_turn(
        app,
        Job {
            id: format!("job-{}", uuid::Uuid::new_v4()),
            chat_id: dm.meta.id,
            bot_id: item.coordinator_bot_id.clone(),
            kind: "attention_report".into(),
            trigger_message_id: incoming.id,
            routine_id: None,
            check: None,
            requested_by: app.this_device_id().unwrap_or_default(),
            from_bot_id: Some(reporter.into()),
            hops: hops + 1,
            round: 0,
            is_winding_down: false,
            setup: None,
            task_id: None,
            task_context: None,
            handoff: None,
            created_at: now_secs(),
        },
    );
}

#[derive(Deserialize)]
struct BriefInput {
    coordinator_bot_id: String,
    decisions: Vec<String>,
    changes: Vec<String>,
    next_action: String,
    #[serde(default)]
    item_ids: Vec<String>,
    #[serde(default)]
    quiet: bool,
}
fn publish_brief(
    app: &Arc<App>,
    mut input: BriefInput,
    actor: Option<&str>,
) -> Result<Brief, String> {
    if actor.is_some_and(|actor| actor != input.coordinator_bot_id) {
        return Err("Publish your own coordinator brief".into());
    }
    if app.bot(&input.coordinator_bot_id).is_none() {
        return Err("Coordinator bot is gone".into());
    }
    if input.decisions.len() + input.changes.len() > 20
        || input.decisions.is_empty() && input.changes.is_empty()
    {
        return Err("A brief needs 1–20 decisions or changes; quiet checks publish nothing".into());
    }
    for line in input.decisions.iter_mut().chain(&mut input.changes) {
        *line = text(line, "brief entry", 1000)?
    }
    input.next_action = text(&input.next_action, "next_action", 1000)?;
    input.item_ids.sort();
    input.item_ids.dedup();
    let _write = WRITES.lock().unwrap();
    let known = records(app)?;
    let current = project(app, &known, false);
    for id in &input.item_ids {
        if !current
            .items
            .iter()
            .any(|item| item.id == *id && item.coordinator_bot_id == input.coordinator_bot_id)
        {
            return Err("Brief links must name your active attention items".into());
        }
    }
    if let Some(old) = current.briefs.iter().find(|brief| {
        brief.coordinator_bot_id == input.coordinator_bot_id
            && brief.decisions == input.decisions
            && brief.changes == input.changes
            && brief.next_action == input.next_action
            && brief.item_ids == input.item_ids
    }) {
        return Ok(old.clone());
    }
    let dm = app
        .dm_with(&input.coordinator_bot_id, None)
        .map_err(|e| e.to_string())?;
    let mut lines: Vec<String> = input
        .decisions
        .iter()
        .map(|line| format!("Decision: {line}"))
        .collect();
    lines.extend(input.changes.iter().map(|line| format!("Changed: {line}")));
    lines.push(format!("Next: {}", input.next_action));
    let mut message = Message::new(
        &dm.meta.id,
        Author::Bot {
            bot_id: input.coordinator_bot_id.clone(),
        },
        Body::text(lines.join("\n")),
    );
    message.notification = Some(if input.quiet {
        Notification::Quiet
    } else {
        Notification::Summary
    });
    let revision = next_revision(app, &known)?;
    let brief = Brief {
        coordinator_bot_id: input.coordinator_bot_id,
        chat_id: dm.meta.id,
        decisions: input.decisions,
        changes: input.changes,
        next_action: input.next_action,
        item_ids: input.item_ids,
        message_id: message.id.clone(),
        revision: revision.clone(),
        updated_at: now_secs(),
    };
    save(
        app,
        &Record {
            id: hash(&format!("brief:{}", brief.coordinator_bot_id)),
            revision,
            content: Content::Brief {
                brief: brief.clone(),
            },
        },
    )?;
    drop(_write);
    app.upsert_message(message.clone(), true);
    changed(app);
    crate::push::attention(app, &message);
    Ok(brief)
}

pub fn dispatch(
    app: &Arc<App>,
    method: &str,
    params: Value,
    actor: Option<&str>,
) -> Result<Value, String> {
    if !app.has_identity() {
        return Err("Pair this Device first".into());
    }
    match method {
        "attention.list" => Ok(json!(project(
            app,
            &records(app)?,
            params["include_resolved"].as_bool().unwrap_or(false)
        ))),
        "attention.report" => Ok(json!(report(
            app,
            serde_json::from_value(params).map_err(|e| e.to_string())?,
            actor,
            0
        )?)),
        "attention.resolve" => {
            let id = params["id"].as_str().ok_or("missing id")?;
            let expected: Option<Revision> = params
                .get("expected_revision")
                .map(|value| serde_json::from_value(value.clone()))
                .transpose()
                .map_err(|e| e.to_string())?;
            Ok(json!(resolve(app, id, actor, expected.as_ref())?))
        }
        "attention.brief" => Ok(json!(publish_brief(
            app,
            serde_json::from_value(params).map_err(|e| e.to_string())?,
            actor
        )?)),
        "attention.preferences" => {
            if actor.is_some() {
                return Err("Notification preferences belong to the user".into());
            }
            let _write = WRITES.lock().unwrap();
            let known = records(app)?;
            let mut preferences = current_preferences(app, &known);
            if let Some(value) = params.get("summaries") {
                preferences.summaries = value.as_bool().ok_or("summaries must be a boolean")?
            }
            if let Some(value) = params.get("urgent_direct") {
                preferences.urgent_direct =
                    value.as_bool().ok_or("urgent_direct must be a boolean")?
            }
            if let Some(value) = params.get("default_coordinator_bot_id") {
                preferences.default_coordinator_bot_id = if value.is_null() {
                    None
                } else {
                    Some(value.as_str().ok_or("Invalid coordinator")?.into())
                };
            }
            if let Some(id) = &preferences.default_coordinator_bot_id {
                if app.bot(id).is_none() {
                    return Err("Unknown coordinator bot".into());
                }
            }
            if preferences == preferences_of(&known) {
                return Ok(json!(preferences));
            }
            let revision = next_revision(app, &known)?;
            save(
                app,
                &Record {
                    id: hash("preferences"),
                    revision,
                    content: Content::Preferences {
                        preferences: preferences.clone(),
                    },
                },
            )?;
            drop(_write);
            changed(app);
            Ok(json!(preferences))
        }
        _ => Err("Unknown attention method".into()),
    }
}

#[cfg(feature = "runner")]
pub struct AttentionTool {
    pub app: Arc<App>,
    pub bot_id: String,
    pub chat_id: String,
    pub hops: u32,
    pub handled: Arc<std::sync::atomic::AtomicBool>,
    pub requesting_bot_id: Option<String>,
}
#[cfg(feature = "runner")]
#[async_trait::async_trait]
impl lorca_agent::Tool for AttentionTool {
    fn name(&self) -> &str {
        "attention"
    }
    fn description(&self) -> &str {
        "Maintain the consolidated attention view. Report reviews, blockers, commitments and changes with a stable underlying key and next action; ordinary reports go to the owning coordinator, urgent reports also alert directly. List active items, resolve your coordinator's items, and publish one brief emphasizing decisions and changes. Quiet checks publish no alert. Task/review ids are existing records' references."
    }
    fn parameters(&self) -> Value {
        json!({"type":"object", "properties":{
            "action":{"type":"string","enum":["list","report","resolve","brief"]},
            "key":{"type":"string"}, "category":{"type":"string","enum":["review","blocker","commitment","change"]},
            "title":{"type":"string"}, "summary":{"type":"string"}, "next_action":{"type":"string"},
            "source":{"type":"object","properties":{"chat_id":{"type":"string"},"task_id":{"type":"string"},"message_id":{"type":"string"},"review_id":{"type":"string"}},"required":["chat_id"],"additionalProperties":false},
            "coordinator_bot_id":{"type":"string"}, "urgent":{"type":"boolean"}, "quiet":{"type":"boolean"},
            "id":{"type":"string"}, "decisions":{"type":"array","items":{"type":"string"}}, "changes":{"type":"array","items":{"type":"string"}},
            "item_ids":{"type":"array","items":{"type":"string"}}, "include_resolved":{"type":"boolean"}
        },"required":["action"],"additionalProperties":false})
    }
    fn execution_mode(&self) -> Option<lorca_agent::ToolExecutionMode> {
        Some(lorca_agent::ToolExecutionMode::Sequential)
    }
    async fn execute(
        &self,
        _id: &str,
        mut args: Value,
        _cancel: tokio_util::sync::CancellationToken,
        _on_update: lorca_agent::ToolUpdateFn,
    ) -> Result<lorca_agent::ToolResult, lorca_agent::ToolError> {
        let action = args["action"].as_str().ok_or("missing action")?.to_string();
        if action == "report" {
            if args.get("source").is_none() {
                args["source"] = json!({"chat_id":self.chat_id})
            }
            if args.get("coordinator_bot_id").is_none() {
                if let Some(id) = &self.requesting_bot_id {
                    args["coordinator_bot_id"] = json!(id)
                }
            }
        }
        if action == "brief" {
            args["coordinator_bot_id"] = json!(self.bot_id)
        }
        let quiet = args["quiet"].as_bool().unwrap_or(false);
        // Whether the call speaks for this turn: then the turn's own words alert nobody, so the
        // user hears the news once. A report the bot keeps for itself leaves its reply alone.
        let mut speaks = action == "brief";
        let result = if action == "report" {
            report(
                &self.app,
                serde_json::from_value(args).map_err(|e| lorca_agent::ToolError(e.to_string()))?,
                Some(&self.bot_id),
                self.hops,
            )
            .map(|item| {
                speaks = quiet || item.urgent || item.coordinator_bot_id != self.bot_id;
                json!(item)
            })
        } else {
            dispatch(
                &self.app,
                &format!("attention.{action}"),
                args,
                Some(&self.bot_id),
            )
        }
        .map_err(lorca_agent::ToolError)?;
        if speaks {
            self.handled
                .store(true, std::sync::atomic::Ordering::Relaxed);
        }
        Ok(lorca_agent::ToolResult::text(
            serde_json::to_string(&result).unwrap(),
        ))
    }
}

/// The attention part of a turn's system prompt. `reported` is a coordinator's turn that an
/// attention report started.
pub fn prompt(app: &App, bot_id: &str, chat_id: &str, reported: bool) -> String {
    let mut prompt = format!("\nAttention: when work needs the user (a review, a blocker, a commitment, or an important change), record it with the attention tool's report under a stable topic key, with the next action. This chat's id is {chat_id}. A report goes to the coordinating bot, which tells the user; mark it urgent only when it cannot wait, which also alerts the user directly, and quiet for a check that found nothing new. A coordinating bot keeps one brief of decisions and changes, without repeating transcripts, and resolves items once they are done.\n");
    if reported {
        prompt.push_str("This turn was started by an attention report. Read the attention list, publish a brief when the user needs to know something, resolve what is finished, and answer with exactly PASS when nothing needs the user.\n");
    }
    if let Ok(view) = view(app) {
        for item in view
            .items
            .iter()
            .filter(|item| item.coordinator_bot_id == bot_id)
            .take(20)
        {
            prompt.push_str(&format!(
                "- Attention {} ({}): {} · next: {} · source chat {}\n",
                item.id, item.category.word(), item.title, item.next_action, item.sources[0].chat_id
            ));
        }
    }
    prompt
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        config::Config,
        model::{Bot, MessageState},
    };

    struct Account {
        app: Arc<App>,
        home: std::path::PathBuf,
        chef: Bot,
        scout: Bot,
        chef_chat: String,
        scout_chat: String,
        task_id: String,
    }
    impl Account {
        fn new() -> Self {
            let home =
                std::env::temp_dir().join(format!("lorca-attention-{}", uuid::Uuid::new_v4()));
            let app = App::load(Config {
                home: home.clone(),
                port: 0,
            })
            .unwrap();
            crate::identity::create(&app, Some("Attention test".into())).unwrap();
            let chef = app.state.lock().unwrap().bots[0].clone();
            let chef_chat = app.dm_with(&chef.id, None).unwrap().meta.id;
            let mut scout = chef.clone();
            scout.id = "scout".into();
            scout.name = "Scout".into();
            let (scout, dm) = app.create_bot_with_dm(scout, None).unwrap();
            // A durable task the reports link, made on a runtime of its own so a #[tokio::test]
            // can build the account too.
            let params = json!({"request_id":"attention-task","owner_bot_id":scout.id,"chat_ids":[dm.meta.id],
                "goal":"Close the contract","acceptance_criteria":["Signed"],"next_action":"Review the draft"});
            let task_app = app.clone();
            let task_id = std::thread::spawn(move || {
                let runtime = tokio::runtime::Builder::new_current_thread().enable_all().build().unwrap();
                runtime.block_on(crate::tasks::dispatch(&task_app, "tasks.create", params)).unwrap()["id"].as_str().unwrap().to_string()
            })
            .join()
            .unwrap();
            Self {
                app,
                home,
                chef,
                scout,
                chef_chat,
                scout_chat: dm.meta.id,
                task_id,
            }
        }
        fn report(&self) -> Report {
            Report {
                key: "contract-review".into(),
                category: Category::Review,
                title: "Decide the contract terms".into(),
                summary: "Two teams found the same pending decision".into(),
                next_action: "Review the draft".into(),
                source: Source {
                    chat_id: self.scout_chat.clone(),
                    task_id: Some(self.task_id.clone()),
                    message_id: None,
                    review_id: Some("review-contract".into()),
                },
                coordinator_bot_id: Some(self.chef.id.clone()),
                urgent: false,
                quiet: true,
            }
        }
    }
    impl Drop for Account {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.home);
        }
    }

    #[test]
    fn reports_merge_sources_without_duplicate_work_and_resolutions_are_terminal() {
        let account = Account::new();
        let first = report(&account.app, account.report(), Some(&account.scout.id), 0).unwrap();
        let before = records(&account.app).unwrap();
        assert_eq!(
            report(&account.app, account.report(), Some(&account.scout.id), 0)
                .unwrap()
                .revision,
            first.revision
        );
        assert_eq!(
            records(&account.app).unwrap(),
            before,
            "an identical retry makes no relay write"
        );
        let mut other = account.report();
        other.source.chat_id = account.chef_chat.clone();
        let merged = report(&account.app, other, Some(&account.chef.id), 0).unwrap();
        assert_eq!(view(&account.app).unwrap().items.len(), 1);
        assert_eq!(merged.sources.len(), 2);
        assert_eq!(merged.reporters.len(), 2);
        assert!(resolve(&account.app, &first.id, Some(&account.scout.id), None).is_err());
        assert!(
            resolve(&account.app, &first.id, None, Some(&first.revision)).is_err(),
            "stale UI resolution is refused"
        );
        resolve(
            &account.app,
            &first.id,
            Some(&account.chef.id),
            Some(&merged.revision),
        )
        .unwrap();
        assert!(view(&account.app).unwrap().items.is_empty());
        // A report queued on an offline Device cannot undo the resolution even if its clock
        // is ahead. A new topic key is required to represent new work.
        let mut stale = before[0].clone();
        stale.revision.counter += 500;
        if let Content::Observation { item } = &mut stale.content {
            item.revision = stale.revision.clone();
        }
        let bytes = crate::crypto::encrypt_json(&account.app.dek().unwrap(), KIND, &stale).unwrap();
        receive(&account.app, &bytes).unwrap();
        assert!(view(&account.app).unwrap().items.is_empty());
        assert!(report(&account.app, account.report(), Some(&account.scout.id), 0).is_err());
    }

    #[test]
    fn encrypted_rows_outbox_preferences_and_briefs_survive_device_sync_and_restart() {
        let account = Account::new();
        let item = report(&account.app, account.report(), Some(&account.scout.id), 0).unwrap();
        dispatch(&account.app, "attention.preferences", json!({"summaries":false,"urgent_direct":false,"default_coordinator_bot_id":account.scout.id}), None).unwrap();
        assert_eq!(
            coordinator(&account.app, &account.chef_chat).as_deref(),
            Some(account.scout.id.as_str()),
            "any bot can coordinate"
        );
        assert!(!allows(&account.app, Notification::Summary));
        assert!(!allows(&account.app, Notification::Urgent));
        assert!(!allows(&account.app, Notification::Quiet));
        assert!(dispatch(
            &account.app,
            "attention.preferences",
            json!({"summaries":true}),
            Some(&account.chef.id)
        )
        .is_err());
        let args = json!({"coordinator_bot_id":account.chef.id,"decisions":["Approve the reduced scope"],"changes":["Delivery moves to Friday"],"next_action":"Review the contract","item_ids":[item.id],"quiet":true});
        let brief = dispatch(
            &account.app,
            "attention.brief",
            args.clone(),
            Some(&account.chef.id),
        )
        .unwrap();
        assert_eq!(
            dispatch(
                &account.app,
                "attention.brief",
                args,
                Some(&account.chef.id)
            )
            .unwrap(),
            brief
        );
        let messages = account.app.messages(&account.chef_chat);
        assert_eq!(
            messages
                .iter()
                .filter(|message| message.notification.is_some())
                .count(),
            1,
            "one brief, not repeated transcripts"
        );
        assert_eq!(
            messages.last().unwrap().notification,
            Some(Notification::Quiet)
        );
        let rows = account.app.store.attention_rows().unwrap();
        let queued = account.app.store.outbox().unwrap();
        for (id, bytes) in &rows {
            assert!(!bytes
                .windows(b"contract terms".len())
                .any(|part| part == b"contract terms"));
            assert!(
                crate::crypto::decrypt_json::<Record>(&crate::keys::random_32(), KIND, bytes)
                    .is_err()
            );
            assert!(
                queued.iter().any(|blob| blob.kind == KIND
                    && blob.slot.as_ref().is_some_and(|slot| slot.name == *id)
                    && blob.ciphertext == *bytes),
                "encrypted row and its upload commit together"
            );
        }
        let paired = Account::new();
        *paired.app.machine.lock().unwrap() = account.app.machine_file();
        paired.app.save_machine().unwrap();
        *paired.app.state.lock().unwrap() = account.app.state.lock().unwrap().clone();
        paired.app.save_state_now();
        let mut bus = paired.app.events.subscribe();
        for (_, bytes) in rows.iter().rev() {
            receive(&paired.app, bytes).unwrap();
            receive(&paired.app, bytes).unwrap();
        }
        assert_eq!(view(&paired.app).unwrap(), view(&account.app).unwrap());
        while let Ok(event) = bus.try_recv() {
            assert!(
                matches!(event, Event::AttentionChanged(_)),
                "sync does not start turns or alert"
            );
        }
        let reopened = App::load(Config {
            home: paired.home.clone(),
            port: 0,
        })
        .unwrap();
        assert_eq!(view(&reopened).unwrap(), view(&account.app).unwrap());
        assert_eq!(reopened.snapshot()["attention"]["items"][0]["id"], item.id);
        reopened.forget_identity().unwrap();
        assert!(reopened.store.attention_rows().unwrap().is_empty());
    }

    #[test]
    fn source_validation_and_coordinator_permissions_preserve_the_projection() {
        let account = Account::new();
        let mut invalid = account.report();
        invalid.source.task_id = Some("invented-task".into());
        assert!(report(&account.app, invalid, None, 0).is_err());
        let mut invalid = account.report();
        invalid.source.chat_id = "gone".into();
        assert!(report(&account.app, invalid, None, 0).is_err());
        assert!(records(&account.app).unwrap().is_empty());
        let item = report(&account.app, account.report(), None, 0).unwrap();
        assert!(dispatch(&account.app, "attention.brief", json!({"coordinator_bot_id":account.chef.id,"decisions":["Decide"],"changes":[],"next_action":"Act","item_ids":[item.id]}), Some(&account.scout.id)).is_err());
        account.app.delete_chat(&account.scout_chat);
        assert!(
            view(&account.app).unwrap().items.is_empty(),
            "deleted source chats do not remain active"
        );
        let mut message = Message::new(
            &account.chef_chat,
            Author::Bot {
                bot_id: account.chef.id.clone(),
            },
            Body::text("Quiet result"),
        );
        message.notification = Some(Notification::Quiet);
        message.state = MessageState::Complete;
        assert!(!allows(&account.app, message.notification.unwrap()));
        assert!(
            !message.counts_unread(),
            "quiet checks do not add an unread badge"
        );
    }

    #[tokio::test]
    async fn ordinary_reports_route_to_the_coordinator_and_urgent_reports_keep_the_direct_path() {
        let account = Account::new();
        account
            .app
            .set_watched_chat(Some(account.scout_chat.clone()));
        let mut ordinary = account.report();
        ordinary.quiet = false;
        report(&account.app, ordinary.clone(), Some(&account.scout.id), 0).unwrap();
        let routed = account.app.messages(&account.chef_chat);
        assert_eq!(routed.iter().filter(|message| matches!(&message.body, Body::Handoff { from, to, reason } if from == &account.scout.id && to == &account.chef.id && reason.starts_with("Review: Decide the contract terms\n"))).count(), 1, "the marker reads as words, not ids");
        assert!(
            account.app.messages(&account.scout_chat).is_empty(),
            "ordinary reports do not alert from the specialist chat"
        );
        report(&account.app, ordinary, Some(&account.scout.id), 0).unwrap();
        assert_eq!(
            account.app.messages(&account.chef_chat).len(),
            routed.len(),
            "identical reports do not wake the coordinator twice"
        );
        let mut urgent = account.report();
        urgent.quiet = false;
        urgent.urgent = true;
        urgent.key = "urgent-release".into();
        report(&account.app, urgent, Some(&account.scout.id), 0).unwrap();
        assert!(account
            .app
            .messages(&account.scout_chat)
            .iter()
            .any(|message| message.notification == Some(Notification::Urgent)));
        tokio::task::yield_now().await;
        account.app.cancel_chat(&account.chef_chat);
        tokio::task::yield_now().await;
    }
}
