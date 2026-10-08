//! Evidence-backed, user-reviewed workflow revisions on the bot's assigned Runner.
mod store;
use crate::{
    app::App,
    config::now_secs,
    memory,
    model::{Body, Bot, Job},
};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::sync::Arc;
pub use store::{Feedback, Kind, Origin, Proposal, Scope, Snapshot, Target};
use store::{Revision, Store, MAX_TEXT};
#[cfg(feature = "runner")]
mod review;
#[cfg(feature = "runner")]
pub use review::{tick, FeedbackTool};

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Record {
    pub kind: Kind,
    pub origin: Origin,
    #[serde(default)]
    pub note: String,
    #[serde(default)]
    pub before: Option<String>,
    #[serde(default)]
    pub after: Option<String>,
    #[serde(default)]
    pub target: Option<Target>,
    #[serde(default)]
    pub excluded: bool,
    /// Stable event id, for replay-safe review queue/routine hooks.
    #[serde(default)]
    pub event_id: Option<String>,
}
#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Proposed {
    pub target: Target,
    pub after: Value,
    pub evidence: Vec<String>,
    pub explanation: String,
}

pub fn local_bot(app: &App, bot_id: &str) -> Result<Bot, String> {
    let bot = app.bot(bot_id).ok_or("Unknown bot")?;
    if app.this_device_id().as_deref() != Some(&bot.runner_id) {
        return Err("Feedback belongs to the bot's assigned Runner".into());
    }
    if !app.local_device().is_some_and(|d| d.is_runner()) {
        return Err("Feedback processing requires a Runner".into());
    }
    Ok(bot)
}
fn validate_origin(
    app: &App,
    bot_id: &str,
    origin: &Origin,
) -> Result<crate::model::Message, String> {
    let chat = app
        .chat(&origin.chat_id)
        .ok_or("Unknown originating chat")?;
    if !chat.meta.bot_ids.iter().any(|id| id == bot_id) {
        return Err("The originating chat does not contain this bot".into());
    }
    let message = app
        .message(&origin.chat_id, &origin.message_id)
        .ok_or("Load the originating message before recording feedback")?;
    if let Some(id) = &origin.routine_id {
        if !app.routine(id).is_some_and(|r| r.bot_id == bot_id) {
            return Err("The originating routine does not belong to this bot".into());
        }
    }
    Ok(message)
}
/// Called by explicit user decisions and outcome hooks. Silence is never a user decision.
pub async fn record(app: &Arc<App>, bot_id: &str, input: Record) -> Result<Feedback, String> {
    let _guard = app.feedback_lock.lock().await;
    local_bot(app, bot_id)?;
    let message = validate_origin(app, bot_id, &input.origin)?;
    let mut store = store::load(app, bot_id)?;
    let id = input
        .event_id
        .as_ref()
        .map(|id| format!("feedback-{}", memory::hash_text(id)))
        .unwrap_or_else(|| format!("feedback-{}", uuid::Uuid::new_v4()));
    if let Some(existing) = store.feedback.iter().find(|f| f.id == id) {
        return Ok(existing.clone());
    }
    let excluded = input.excluded
        || store
            .settings
            .excluded_chats
            .contains(&input.origin.chat_id)
        || input
            .target
            .as_ref()
            .is_some_and(|t| store.settings.excluded_targets.contains(t));
    let example = match &message.body {
        Body::Text { text, .. } | Body::Notice { text, .. } => text.as_str(),
        _ => "",
    };
    let feedback = Feedback {
        id,
        kind: input.kind,
        link: input.origin.link(),
        origin: input.origin,
        note: if excluded {
            String::new()
        } else {
            store::clean(&input.note, 4000)
        },
        example: if excluded {
            String::new()
        } else {
            store::clean(example, 2000)
        },
        before: if excluded {
            None
        } else {
            input.before.map(|s| store::clean(&s, 4000))
        },
        after: if excluded {
            None
        } else {
            input.after.map(|s| store::clean(&s, 4000))
        },
        target: input.target,
        created_at: now_secs(),
        excluded,
    };
    if feedback.kind == Kind::Edited
        && !excluded
        && (feedback.before.is_none() || feedback.after.is_none())
    {
        return Err("Edited feedback needs before and after text".into());
    }
    if feedback.kind == Kind::Explicit && !excluded && feedback.note.trim().is_empty() {
        return Err("Explicit feedback needs the user's words".into());
    }
    store.feedback.push(feedback.clone());
    store.prune();
    store::save(app, bot_id, &store)?;
    announce(app, bot_id, &store);
    Ok(feedback)
}

fn check_scope(app: &App, bot_id: &str, scope: &Scope) -> Result<(), String> {
    match scope.kind.as_str() {
        "bot" if scope.id == bot_id => Ok(()),
        "project"
            if app.chat(&scope.id).is_some_and(|c| {
                c.meta.kind == "group" && c.meta.bot_ids.iter().any(|b| b == bot_id)
            }) =>
        {
            Ok(())
        }
        _ => Err("A playbook scope must be this bot or a group containing it".into()),
    }
}
fn skill_path(app: &App, plugin_id: &str, name: &str) -> Result<std::path::PathBuf, String> {
    let plugins = app.plugins.lock().unwrap();
    let plugin = plugins.get(plugin_id).ok_or("Unknown installed plugin")?;
    if !plugin.manifest.skills.iter().any(|s| s.name == name) {
        return Err("Unknown installed skill".into());
    }
    // The identity comes from an installed manifest, never an arbitrary file path.
    Ok(app
        .config
        .plugins_dir()
        .join(plugin_id)
        .join("skills")
        .join(format!("{}.md", crate::plugins::slug(name))))
}
async fn read_target(app: &Arc<App>, bot_id: &str, target: &Target) -> Result<Snapshot, String> {
    local_bot(app, bot_id)?;
    let snapshot = match target {
        Target::RoutinePrompt { id } => {
            let r = app
                .routine(id)
                .filter(|r| r.bot_id == bot_id)
                .ok_or("Unknown routine for this bot")?;
            store::snapshot(json!(r.prompt), 0)
        }
        Target::PluginSkill { plugin_id, name } => {
            let text = std::fs::read_to_string(skill_path(app, plugin_id, name)?)
                .map_err(|e| e.to_string())?;
            store::snapshot(json!(text), 0)
        }
        Target::Playbook { scope, id } => {
            check_scope(app, bot_id, scope)?;
            let value = Box::pin(crate::api::dispatch(
                app,
                "playbooks.get",
                json!({"scope":scope,"id":id}),
            ))
            .await
            .map_err(|e| {
                format!("Canonical playbook adapter unavailable or refused the read: {e}")
            })?;
            Snapshot {
                content: value
                    .get("content")
                    .cloned()
                    .ok_or("playbooks.get omitted content")?,
                hash: value["hash"]
                    .as_str()
                    .ok_or("playbooks.get omitted hash")?
                    .into(),
                revision: value["revision"]
                    .as_u64()
                    .ok_or("playbooks.get omitted revision")?,
            }
        }
    };
    let text = store::text(&snapshot.content);
    if text.len() > MAX_TEXT {
        return Err(format!(
            "Workflow revision targets must fit in {MAX_TEXT} bytes"
        ));
    }
    if memory::scrub(&text) != text {
        return Err("This target contains credential material; remove it or exclude it from feedback processing".into());
    }
    Ok(snapshot)
}

pub async fn propose(app: &Arc<App>, bot_id: &str, input: Proposed) -> Result<Proposal, String> {
    let _guard = app.feedback_lock.lock().await;
    local_bot(app, bot_id)?;
    let mut store = store::load(app, bot_id)?;
    propose_locked(app, bot_id, &mut store, input).await
}
async fn propose_locked(
    app: &Arc<App>,
    bot_id: &str,
    store: &mut Store,
    input: Proposed,
) -> Result<Proposal, String> {
    if store.settings.excluded_targets.contains(&input.target) {
        return Err("This workflow is excluded from feedback processing".into());
    }
    if input.explanation.trim().is_empty() || input.explanation.len() > 4000 {
        return Err("Explain the specific improvement in at most 4000 bytes".into());
    }
    if input.evidence.is_empty() || input.evidence.len() > 20 {
        return Err("Cite 1–20 feedback examples".into());
    }
    let mut origins = Vec::new();
    for id in &input.evidence {
        let item = store.feedback.iter().find(|f| &f.id == id && !f.excluded && f.kind.actionable()).ok_or("Evidence must be recorded, included, explicit feedback or an observed failure; an ignored alert is neutral")?;
        validate_origin(app, bot_id, &item.origin)?;
        if !origins.contains(&item.origin) {
            origins.push(item.origin.clone());
        }
    }
    let before = read_target(app, bot_id, &input.target).await?;
    match &input.target {
        Target::RoutinePrompt { .. } | Target::PluginSkill { .. } if !input.after.is_string() => return Err("This target accepts text only; permission, budget and schedule fields cannot be revised".into()),
        Target::Playbook { .. } if !input.after.is_object() => return Err("A playbook revision takes its canonical content object".into()),
        _ => {},
    }
    let after_text = store::text(&input.after);
    if after_text.trim().is_empty()
        || after_text.len() > MAX_TEXT
        || memory::scrub(&after_text) != after_text
    {
        return Err("Proposed content must be nonempty, bounded and free of credentials".into());
    }
    if input.after == before.content {
        return Err("There is no change to review".into());
    }
    if let Some(existing) = store.proposals.iter().find(|p| {
        p.target == input.target
            && p.before.hash == before.hash
            && p.after == input.after
            && (p.state == "pending" || p.state == "rejected" || p.state == "accepted")
    }) {
        return Ok(existing.clone());
    }
    let diff = store::diff(&before.content, &input.after);
    let proposal = Proposal {
        id: format!("proposal-{}", uuid::Uuid::new_v4()),
        target: input.target,
        before,
        after: input.after,
        evidence: input.evidence,
        origins,
        explanation: store::clean(&input.explanation, 4000),
        diff_hash: memory::hash_text(&diff),
        diff,
        state: "pending".into(),
        created_at: now_secs(),
    };
    store.proposals.push(proposal.clone());
    store::save(app, bot_id, store)?;
    announce(app, bot_id, store);
    Ok(proposal)
}

async fn write_target(
    app: &Arc<App>,
    bot_id: &str,
    target: &Target,
    before: &Snapshot,
    after: &Value,
    origins: &[Origin],
) -> Result<(), String> {
    local_bot(app, bot_id)?;
    match target {
        Target::RoutinePrompt { id } => {
            // Compare and write under the roster mutex. Preserve the original authorization task.
            {
                let mut state = app.state.lock().unwrap();
                let r = state
                    .routines
                    .iter_mut()
                    .find(|r| &r.id == id && r.bot_id == bot_id)
                    .ok_or("Unknown routine")?;
                if store::snapshot(json!(r.prompt), 0).hash != before.hash {
                    return Err("The routine changed; review a fresh proposal".into());
                }
                let prior = r.clone();
                if r.feedback_authorization_prompt.is_none() {
                    r.feedback_authorization_prompt = Some(r.prompt.clone());
                }
                r.prompt = after.as_str().ok_or("A routine prompt is text")?.into();
                // Back at the task the user wrote, the run needs no other authority.
                if r.feedback_authorization_prompt.as_deref() == Some(r.prompt.as_str()) {
                    r.feedback_authorization_prompt = None;
                }
                if let Err(error) = app.store.save_state(&state) {
                    *state.routines.iter_mut().find(|r| &r.id == id).unwrap() = prior;
                    return Err(error.to_string());
                }
            }
            app.roster_changed(true);
            Ok(())
        }
        Target::PluginSkill { plugin_id, name } => {
            let path = skill_path(app, plugin_id, name)?;
            let _plugins = app.plugins.lock().unwrap();
            let current = std::fs::read_to_string(&path).map_err(|e| e.to_string())?;
            if memory::hash_text(&current) != before.hash {
                return Err("The skill changed; review a fresh proposal".into());
            }
            crate::config::write_private(&path, after.as_str().ok_or("A skill is text")?.as_bytes())
                .map_err(|e| e.to_string())
        }
        Target::Playbook { scope, id } => {
            check_scope(app, bot_id, scope)?;
            let origin = origins.first();
            let provenance = json!({"kind":"workflow_feedback","chat_id":origin.map(|o| &o.chat_id),"message_ids":origins.iter().filter(|o| Some(&o.chat_id) == origin.map(|o| &o.chat_id)).map(|o| &o.message_id).collect::<Vec<_>>(),"note":"Explicitly reviewed workflow feedback revision"});
            Box::pin(crate::api::dispatch(app, "playbooks.save", json!({"scope":scope,"id":id,"content":after,"expected_revision":before.revision,"expected_hash":before.hash,"provenance":provenance}))).await.map(|_| ())
        }
    }
}

/// Reconcile a write-ahead revision after a process stops between the two persisted writes.
async fn reconcile(app: &Arc<App>, bot_id: &str, store: &mut Store) -> Result<(), String> {
    let mut changed = false;
    for revision in &mut store.revisions {
        if revision.state != "applying" {
            continue;
        }
        let current = read_target(app, bot_id, &revision.target).await?;
        revision.state = if current.content == revision.after {
            "applied"
        } else if current.content == revision.before.content {
            "not_applied"
        } else {
            "conflict"
        }
        .into();
        if revision.state == "applied" && revision.rollback_of.is_none() {
            if let Some(p) = store
                .proposals
                .iter_mut()
                .find(|p| p.id == revision.proposal_id)
            {
                p.state = "accepted".into();
            }
        }
        changed = true;
    }
    if changed {
        store::save(app, bot_id, store)?;
    }
    Ok(())
}

pub async fn decide(
    app: &Arc<App>,
    bot_id: &str,
    id: &str,
    diff_hash: &str,
    accept: bool,
    actor: &str,
) -> Result<Value, String> {
    let _guard = app.feedback_lock.lock().await;
    local_bot(app, bot_id)?;
    let mut store = store::load(app, bot_id)?;
    reconcile(app, bot_id, &mut store).await?;
    let index = store
        .proposals
        .iter()
        .position(|p| p.id == id)
        .ok_or("Unknown proposal")?;
    let proposal = store.proposals[index].clone();
    if proposal.diff_hash != diff_hash {
        return Err("The displayed diff changed; reload it before deciding".into());
    }
    if proposal.state == if accept { "accepted" } else { "rejected" } {
        return Ok(json!({"proposal":proposal}));
    }
    if proposal.state != "pending" {
        return Err("This proposal is no longer pending".into());
    }
    for evidence in &proposal.evidence {
        let f = store
            .feedback
            .iter()
            .find(|f| &f.id == evidence && !f.excluded && f.kind.actionable())
            .ok_or("Proposal evidence is no longer included")?;
        validate_origin(app, bot_id, &f.origin)?;
    }
    if !accept {
        store.proposals[index].state = "rejected".into();
        store::save(app, bot_id, &store)?;
        announce(app, bot_id, &store);
        return Ok(json!({"proposal":store.proposals[index]}));
    }
    let current = read_target(app, bot_id, &proposal.target).await?;
    if current.hash != proposal.before.hash || current.revision != proposal.before.revision {
        return Err("The target changed; review a fresh proposal".into());
    }
    let revision = Revision {
        id: format!("revision-{}", uuid::Uuid::new_v4()),
        version: store
            .revisions
            .iter()
            .filter(|r| r.target == proposal.target)
            .count() as u64
            + 1,
        proposal_id: id.into(),
        target: proposal.target.clone(),
        before: current,
        after: proposal.after.clone(),
        state: "applying".into(),
        created_at: now_secs(),
        actor_device_id: actor.into(),
        rollback_of: None,
    };
    store.revisions.push(revision.clone());
    store::save(app, bot_id, &store)?;
    let result = write_target(
        app,
        bot_id,
        &proposal.target,
        &revision.before,
        &revision.after,
        &proposal.origins,
    )
    .await;
    store.revisions.last_mut().unwrap().state = if result.is_ok() {
        "applied"
    } else {
        "not_applied"
    }
    .into();
    if result.is_ok() {
        store.proposals[index].state = "accepted".into();
    }
    store::save(app, bot_id, &store)?;
    result?;
    announce(app, bot_id, &store);
    Ok(json!({"proposal":store.proposals[index],"revision":store.revisions.last()}))
}

async fn rollback(
    app: &Arc<App>,
    bot_id: &str,
    id: &str,
    expected_hash: &str,
    actor: &str,
) -> Result<Value, String> {
    let _guard = app.feedback_lock.lock().await;
    local_bot(app, bot_id)?;
    let mut store = store::load(app, bot_id)?;
    reconcile(app, bot_id, &mut store).await?;
    let prior = store
        .revisions
        .iter()
        .find(|r| r.id == id && r.state == "applied")
        .cloned()
        .ok_or("Unknown applied revision")?;
    let current = read_target(app, bot_id, &prior.target).await?;
    if current.hash != expected_hash || current.content != prior.after {
        return Err(
            "The target changed since this revision; rollback cannot overwrite later edits".into(),
        );
    }
    let revision = Revision {
        id: format!("revision-{}", uuid::Uuid::new_v4()),
        version: store
            .revisions
            .iter()
            .filter(|r| r.target == prior.target)
            .count() as u64
            + 1,
        proposal_id: prior.proposal_id.clone(),
        target: prior.target.clone(),
        before: current,
        after: prior.before.content,
        state: "applying".into(),
        created_at: now_secs(),
        actor_device_id: actor.into(),
        rollback_of: Some(id.into()),
    };
    store.revisions.push(revision.clone());
    store::save(app, bot_id, &store)?;
    let result = write_target(
        app,
        bot_id,
        &revision.target,
        &revision.before,
        &revision.after,
        &[],
    )
    .await;
    store.revisions.last_mut().unwrap().state = if result.is_ok() {
        "applied"
    } else {
        "not_applied"
    }
    .into();
    store::save(app, bot_id, &store)?;
    result?;
    announce(app, bot_id, &store);
    Ok(json!({"revision":store.revisions.last()}))
}

fn announce(app: &App, bot_id: &str, store: &Store) {
    app.emit(crate::events::Event::FeedbackChanged {
        bot_id: bot_id.into(),
        pending_count: store
            .proposals
            .iter()
            .filter(|p| p.state == "pending")
            .count(),
    });
}

pub async fn on_runner(app: &Arc<App>, method: &str, params: Value) -> Result<Value, String> {
    let bot_id = params["bot_id"].as_str().ok_or("missing bot_id")?;
    let bot = app.bot(bot_id).ok_or("Unknown bot")?;
    if app.this_device_id().as_deref() != Some(&bot.runner_id) {
        return crate::requests::ask_within(
            app,
            &bot.runner_id,
            method,
            params,
            std::time::Duration::from_secs(150),
        )
        .await;
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
    let bot_id = params["bot_id"].as_str().ok_or("missing bot_id")?;
    local_bot(app, bot_id)?;
    let field = |key: &str| params[key].as_str().ok_or_else(|| format!("missing {key}"));
    match method {
        "feedback.record" => Ok(
            json!({"feedback":record(app,bot_id,serde_json::from_value(params["feedback"].clone()).map_err(|e| e.to_string())?).await?}),
        ),
        "feedback.propose" => Ok(
            json!({"proposal":propose(app,bot_id,serde_json::from_value(params["proposal"].clone()).map_err(|e| e.to_string())?).await?}),
        ),
        "feedback.accept" | "feedback.reject" => {
            decide(
                app,
                bot_id,
                field("id")?,
                field("diff_hash")?,
                method == "feedback.accept",
                actor,
            )
            .await
        }
        "feedback.rollback" => {
            rollback(app, bot_id, field("id")?, field("expected_hash")?, actor).await
        }
        // What the apps show, bounded to stay well under the app socket's message size: the
        // newest included feedback and every example a pending proposal cites, pending
        // proposals, and the newest applied revisions with the change each one made.
        "feedback.list" => {
            let _guard = app.feedback_lock.lock().await;
            let mut store = store::load(app, bot_id)?;
            reconcile(app, bot_id, &mut store).await?;
            let pending: Vec<&Proposal> = store.proposals.iter().filter(|p| p.state == "pending").collect();
            let included: Vec<&Feedback> = store.feedback.iter().filter(|f| !f.excluded).collect();
            let feedback: Vec<Value> = included
                .iter()
                .rev()
                .enumerate()
                .filter(|(index, f)| *index < 30 || pending.iter().any(|p| p.evidence.contains(&f.id)))
                .map(|(_, f)| json!({"id":f.id,"kind":f.kind,"origin":f.origin,"note":store::clean(&f.note,300),"example":store::clean(&f.example,300),"target":f.target,"created_at":f.created_at}))
                .collect();
            let mut revisions = Vec::new();
            for r in store.revisions.iter().rev().filter(|r| r.state == "applied").take(20) {
                let current = read_target(app, bot_id, &r.target).await.ok();
                revisions.push(json!({
                    "id": r.id, "target": r.target, "created_at": r.created_at, "rollback_of": r.rollback_of,
                    "diff": store::diff(&r.before.content, &r.after),
                    "can_rollback": current.as_ref().is_some_and(|c| c.content == r.after),
                    "current_hash": current.map(|c| c.hash),
                }));
            }
            let mut targets = Vec::new();
            for r in app.routines_of(bot_id) {
                targets.push(json!({"target":Target::RoutinePrompt{id:r.id},"name":r.name}));
            }
            for p in app.plugins.lock().unwrap().installed() {
                for s in &p.manifest.skills {
                    targets.push(json!({"target":Target::PluginSkill{plugin_id:p.manifest.id.clone(),name:s.name.clone()},"name":format!("{} · {}",p.manifest.name,s.name)}));
                }
            }
            Ok(json!({"feedback":feedback,"feedback_count":included.len(),"proposals":pending,"revisions":revisions,"settings":{"review_every_secs":store.settings.review_every_secs},"targets":targets}))
        }
        "feedback.settings" => {
            #[cfg(feature = "runner")]
            if params.get("review_every_secs").is_some() {
                if let Some(cancel) = app.feedback_reviews.lock().unwrap().get(bot_id) {
                    cancel.cancel();
                }
            }
            let _guard = app.feedback_lock.lock().await;
            let mut store = store::load(app, bot_id)?;
            let interval = match params.get("review_every_secs") {
                Some(Value::Null) => None,
                Some(value) => Some(value.as_i64().filter(|n| (86_400..=30*86_400).contains(n)).ok_or("Choose a review interval from one day to thirty days, or null to disable")?),
                None => return Ok(json!({"settings":store.settings})),
            };
            store.settings.review_every_secs = interval;
            store.settings.last_review_at = Some(now_secs());
            store::save(app, bot_id, &store)?;
            announce(app, bot_id, &store);
            Ok(json!({"settings":store.settings}))
        }
        "feedback.exclude" => {
            #[cfg(feature = "runner")]
            if let Some(cancel) = app.feedback_reviews.lock().unwrap().get(bot_id) {
                cancel.cancel();
            }
            let _guard = app.feedback_lock.lock().await;
            let mut store = store::load(app, bot_id)?;
            let chat_id = params["chat_id"].as_str();
            let id = params["id"].as_str();
            let target: Option<Target> = params
                .get("target")
                .map(|t| serde_json::from_value(t.clone()).map_err(|e| e.to_string()))
                .transpose()?;
            if chat_id.is_none() && id.is_none() && target.is_none() {
                return Err("Choose a chat_id, feedback id or workflow target to exclude".into());
            }
            if let Some(target) = &target {
                if !store.settings.excluded_targets.contains(target) {
                    store.settings.excluded_targets.push(target.clone());
                }
            }
            if let Some(chat) = chat_id {
                if !app
                    .chat(chat)
                    .is_some_and(|c| c.meta.bot_ids.iter().any(|b| b == bot_id))
                {
                    return Err("Unknown chat for this bot".into());
                }
                if !store.settings.excluded_chats.iter().any(|c| c == chat) {
                    store.settings.excluded_chats.push(chat.into());
                }
            }
            if let Some(id) = id {
                if !store.feedback.iter().any(|f| f.id == id) {
                    return Err("Unknown feedback".into());
                }
            }
            let mut removed = Vec::new();
            for f in &mut store.feedback {
                if chat_id == Some(&f.origin.chat_id)
                    || id == Some(&f.id)
                    || (target.is_some() && f.target == target)
                {
                    f.excluded = true;
                    f.note.clear();
                    f.example.clear();
                    f.before = None;
                    f.after = None;
                    removed.push(f.id.clone());
                }
            }
            for p in &mut store.proposals {
                if p.state == "pending"
                    && (p.evidence.iter().any(|e| removed.contains(e))
                        || target.as_ref() == Some(&p.target))
                {
                    p.state = "excluded".into();
                    p.before.content = Value::Null;
                    p.after = Value::Null;
                    p.diff.clear();
                    p.explanation.clear();
                }
            }
            store::save(app, bot_id, &store)?;
            announce(app, bot_id, &store);
            Ok(json!({"excluded":removed,"settings":store.settings}))
        }
        #[cfg(feature = "runner")]
        "feedback.review" => review::run(app, bot_id, false).await,
        other => Err(format!("Unknown feedback method {other}")),
    }
}

/// A deleted bot's feedback, proposals and revision history go with it.
pub fn forget_bot(app: &App, bot_id: &str) {
    #[cfg(feature = "runner")]
    if let Some(review) = app.feedback_reviews.lock().unwrap().get(bot_id) {
        review.cancel();
    }
    if let Ok(path) = store::path(app, bot_id) {
        match std::fs::remove_file(path) {
            Err(error) if error.kind() != std::io::ErrorKind::NotFound => tracing::warn!(%error, "forgetting a deleted bot's feedback"),
            _ => {}
        }
    }
}

/// Record routine failures from the actual job outcome, with a stable source message.
pub fn routine_outcome(app: &Arc<App>, job: &Job, failed: bool) {
    if !failed {
        return;
    }
    let Some(id) = job.routine_id.clone() else {
        return;
    };
    let Some(origin) = app.message(&job.chat_id, &format!("routine-run-{}", job.id)) else {
        return;
    };
    let Ok((messages, _)) = app.store.page(&job.chat_id, None, 20) else {
        return;
    };
    let detail = messages
        .iter()
        .rev()
        .find_map(|m| match &m.body {
            Body::Text { text, .. }
                if matches!(m.state, crate::model::MessageState::Failed { .. })
                    && m.created_at >= origin.created_at =>
            {
                Some(text.clone())
            }
            Body::Notice {
                text,
                routine_id: None,
            } if m.created_at >= origin.created_at => Some(text.clone()),
            _ => None,
        })
        .unwrap_or_else(|| "The routine did not complete successfully".into());
    let input = Record {
        kind: Kind::RoutineFailure,
        origin: Origin {
            chat_id: job.chat_id.clone(),
            message_id: origin.id.clone(),
            routine_id: Some(id.clone()),
            review_id: None,
            task_id: None,
        },
        note: job
            .check
            .as_ref()
            .and_then(|check| check.error.clone())
            .unwrap_or(detail),
        before: None,
        after: None,
        target: Some(Target::RoutinePrompt { id }),
        excluded: false,
        event_id: Some(job.id.clone()),
    };
    let (app, bot_id) = (app.clone(), job.bot_id.clone());
    tokio::spawn(async move {
        if let Err(error) = record(&app, &bot_id, input).await {
            tracing::warn!(%error,"recording routine feedback");
        }
    });
}

#[cfg(test)]
mod tests;

/// Feedback-authored guidance does not authorize unattended control-plane changes.
/// Routine edits can authorize another task as well as change its schedule, and bot edits
/// change what a bot runs with, so a revised run stages a proposal or asks the user in chat.
pub fn changes_controls(tool: &str, args: &Value) -> bool {
    match tool {
        "routines" => !matches!(args["action"].as_str(), Some("list" | "pause")),
        "edit_bot" | "create_bot" => true,
        _ => false,
    }
}
