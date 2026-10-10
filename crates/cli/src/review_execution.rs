//! Resumes one approved call, independently of the model turn that proposed it.

use std::sync::Arc;

use async_trait::async_trait;
use lorca_agent::{Tool, ToolError, ToolResult, ToolUpdateFn};
use serde_json::{json, Value};
use tokio_util::sync::CancellationToken;

use crate::app::App;
use crate::config::now_secs;
use crate::plugins::review::Trigger;
use crate::review_queue::{self as queue, *};

#[cfg(test)]
mod tests;

/// Why an approved or proposed call needs another look.
const STALE: &str = "Something this depends on changed since it was proposed. Review it again.";

struct Prepared {
    tool: Option<Arc<dyn Tool>>,
    preconditions: ReviewPreconditions,
}

/// The authorization boundary for review execution: the current origin and assignment, the
/// approving Device, the bot's Access to the shell or to the exact plugin tool, and the current
/// connection, before a claim can run. The call checks Access again as it starts.
pub fn authorize_execution(app: &Arc<App>, item: &ReviewItem) -> Result<crate::model::Bot, String> {
    let bot = queue::local_bot(app, item)?;
    if let Some(approval) = &item.approval {
        if app.this_device_id().as_deref() != Some(approval.device_id.as_str())
            && app.device(&approval.device_id).is_none()
        {
            return Err("The approving Device is no longer paired.".into());
        }
    }
    let access = match &item.payload {
        ReviewPayload::Shell { .. } => crate::permissions::check_tool(app, &bot, "bash"),
        ReviewPayload::Plugin { plugin_id, tool, .. } => crate::permissions::check_connection(app, &bot, plugin_id, tool, None),
        ReviewPayload::Draft { .. } => Ok(()),
    };
    access.map_err(|denied| denied.to_string())?;
    if let ReviewPayload::Plugin {
        plugin_id,
        server_name,
        tool,
        ..
    } = &item.payload
    {
        let store = app.plugins.lock().unwrap();
        let plugin = store
            .get(plugin_id)
            .ok_or("The reviewed connection was removed.")?;
        if !plugin.manifest.servers.contains_key(server_name) || plugin.manifest.tools.hides(tool) {
            return Err("The reviewed tool is no longer available on this connection.".into());
        }
        if store
            .status(plugin_id)
            .is_some_and(|status| matches!(status.state.as_str(), "needs_setup" | "needs_auth"))
        {
            return Err("The reviewed connection requires setup or sign-in.".into());
        }
    }
    Ok(bot)
}

fn authorization_hash(app: &Arc<App>, item: &ReviewItem) -> Result<String, String> {
    let mut bot = authorize_execution(app, item)?;
    // Turning drafts off decides how the bot's next messages go, not whether this one may.
    if let Some(permissions) = &mut bot.permissions {
        permissions.drafts = true;
    }
    let routine = item.origin.routine_id.as_deref().and_then(|id| app.routine(id)).map(|routine| json!({
        "bot_id": routine.bot_id, "prompt": routine.prompt, "schedule": routine.schedule, "check": routine.check, "enabled": routine.is_enabled
    }));
    Ok(queue::fingerprint(
        &json!({ "format": 1, "bot": bot, "auto_review": app.auto_review(), "routine": routine }),
    ))
}

fn file_state(path: &str) -> Result<ReviewedFile, String> {
    let pathbuf = std::path::Path::new(path);
    if !pathbuf.is_absolute() {
        return Err("A guarded file must have an absolute path.".into());
    }
    let hash = match std::fs::metadata(pathbuf) {
        Ok(metadata) => {
            if !metadata.is_file() || metadata.len() > 16 * 1024 * 1024 {
                return Err("A guarded path must be a file under 16 MiB.".into());
            }
            Some(queue::fingerprint(&std::fs::read(pathbuf).map_err(
                |error| format!("Cannot read guarded file {path}: {error}"),
            )?))
        }
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => None,
        Err(error) => return Err(format!("Cannot check guarded file {path}: {error}")),
    };
    Ok(ReviewedFile {
        path: path.into(),
        hash,
    })
}

async fn prepare(
    app: &Arc<App>,
    item: &ReviewItem,
    cancel: &CancellationToken,
) -> Result<Prepared, String> {
    let bot = authorize_execution(app, item)?;
    let workdir = bot.working_directory(&app.config.home);
    let workdir = std::fs::canonicalize(&workdir)
        .map_err(|error| format!("The reviewed working directory is unavailable: {error}"))?;
    let mut preconditions = ReviewPreconditions {
        authorization_hash: authorization_hash(app, item)?,
        workdir: workdir.display().to_string(),
        connection_hash: None,
        tool_hash: None,
        files: Vec::new(),
    };
    if item.preconditions.files.len() > 16 {
        return Err("A review can guard at most 16 files.".into());
    }
    for guard in &item.preconditions.files {
        preconditions.files.push(file_state(&guard.path)?);
    }
    let tool = match &item.payload {
        ReviewPayload::Draft { text } => {
            if text.trim().is_empty() {
                return Err("A draft needs text.".into());
            }
            None
        }
        ReviewPayload::Shell { arguments } => {
            if arguments["command"]
                .as_str()
                .is_none_or(|command| command.trim().is_empty())
            {
                return Err("A shell proposal needs a command.".into());
            }
            if arguments["background"].as_bool() == Some(true) {
                return Err(
                    "Reviewed commands run to completion; background is unavailable.".into(),
                );
            }
            // The bot's shell Access is checked again when the command starts.
            crate::permissions::guarded::tools(app, &bot, &item.origin.chat_id, vec![crate::shell::script_bash(app, &bot.id, &workdir)]).pop()
        }
        ReviewPayload::Plugin {
            plugin_id,
            server_name,
            tool,
            ..
        } => {
            let executable =
                crate::plugins::mcp::reviewed_tool(app, &bot, &item.origin.chat_id, plugin_id, server_name, tool, cancel)
                    .await?;
            preconditions.connection_hash =
                Some(app.plugins.lock().unwrap().review_fingerprint(plugin_id)?);
            Some(executable)
        }
    };
    if let Some(tool) = &tool {
        preconditions.tool_hash = Some(queue::fingerprint(
            &json!({ "name": tool.name(), "description": tool.description(),
            "parameters": tool.parameters(), "output": tool.output_schema() }),
        ));
        let arguments = match &item.payload {
            ReviewPayload::Shell { arguments } | ReviewPayload::Plugin { arguments, .. } => {
                arguments
            }
            _ => unreachable!(),
        };
        let checked = lorca_agent::schema::validate_tool_arguments(
            tool.name(),
            &tool.parameters(),
            arguments,
        )?;
        if &checked != arguments {
            return Err(
                "Use the tool schema's exact argument types before reviewing this call.".into(),
            );
        }
    }
    // A connection handshake can refresh auth. Capture policy again after that await.
    preconditions.authorization_hash = authorization_hash(app, item)?;
    Ok(Prepared {
        tool,
        preconditions,
    })
}

fn editable(item: &ReviewItem) -> Result<(), String> {
    if !matches!(item.state, ReviewState::Pending | ReviewState::Approved) {
        return Err("This already ran or was decided.".into());
    }
    Ok(())
}

fn apply_fields(app: &App, item: &mut ReviewItem, params: &Value) -> Result<(), String> {
    if let Some(payload) = params.get("payload") {
        item.payload =
            serde_json::from_value(payload.clone()).map_err(|error| error.to_string())?;
    }
    // A draft card's edit, as its fields: the Runner puts them onto the call's arguments.
    if let Some(message) = params.get("message") {
        let message: crate::model::MessageDraft = serde_json::from_value(message.clone()).map_err(|error| error.to_string())?;
        let (tool, _) = crate::drafts::tool_of(app, &item.payload).ok_or("This is not a message draft.")?;
        let ReviewPayload::Plugin { arguments, .. } = &mut item.payload else { unreachable!() };
        crate::drafts::write(&tool, arguments, &message)?;
        item.target.resource = crate::drafts::summary(&crate::drafts::read(&tool, arguments));
    }
    if let Some(target) = params.get("target") {
        item.target = serde_json::from_value(target.clone()).map_err(|error| error.to_string())?;
    }
    if let Some(rationale) = params.get("rationale") {
        item.rationale = rationale.as_str().ok_or("rationale must be text")?.into();
    }
    if let Some(paths) = params.get("guarded_paths") {
        let paths: Vec<String> =
            serde_json::from_value(paths.clone()).map_err(|error| error.to_string())?;
        if paths.len() > 16 {
            return Err("A review can guard at most 16 files.".into());
        }
        item.preconditions.files = paths
            .into_iter()
            .map(|path| ReviewedFile { path, hash: None })
            .collect();
    }
    if item.target.account.trim().is_empty()
        || item.target.resource.trim().is_empty()
        || item.rationale.trim().is_empty()
    {
        return Err("A review needs its target account, resource, and rationale.".into());
    }
    if let Some(id) = &item.origin.task_id {
        crate::tasks::get(app, id).map_err(|_| format!("There is no task {id}."))?;
    }
    if serde_json::to_vec(&item.payload)
        .map_err(|error| error.to_string())?
        .len()
        > 32 * 1024
    {
        return Err(
            "A reviewed payload must fit in 32 KiB; put large content in a guarded file.".into(),
        );
    }
    Ok(())
}

pub async fn mutate(
    app: &Arc<App>,
    method: &str,
    params: &Value,
    actor: &str,
) -> Result<ReviewItem, String> {
    mutate_until(app, method, params, actor, &CancellationToken::new()).await
}

/// `mutate`, for a turn's own staging: Stop ends the server connection it waits on, and an item
/// a stopped turn proposed is never saved.
pub async fn mutate_until(
    app: &Arc<App>,
    method: &str,
    params: &Value,
    actor: &str,
    cancel: &CancellationToken,
) -> Result<ReviewItem, String> {
    let cancel = cancel.clone();
    if method == "reviews.create" {
        let bot_id = params["bot_id"].as_str().ok_or("missing bot_id")?;
        let bot = app.bot(bot_id).ok_or("Unknown bot")?;
        let request_id = params["request_id"]
            .as_str()
            .filter(|id| !id.is_empty())
            .ok_or("missing request_id")?;
        let hash = crate::keys::unb64(&queue::fingerprint(&json!([
            bot.runner_id,
            bot.id,
            request_id
        ])))
        .map_err(|error| error.to_string())?;
        let mut bytes: [u8; 16] = hash[..16].try_into().expect("a SHA-256 prefix");
        bytes[6] = (bytes[6] & 0x0f) | 0x80;
        bytes[8] = (bytes[8] & 0x3f) | 0x80;
        let id = format!("review-{}", uuid::Uuid::from_bytes(bytes));
        let request_hash = queue::fingerprint(
            &json!({ "bot_id": bot_id, "origin": params["origin"], "payload": params["payload"],
            "target": params["target"], "rationale": params["rationale"], "guarded_paths": params["guarded_paths"] }),
        );
        if let Ok(item) = queue::get(app, &id) {
            if item.request_hash != request_hash {
                return Err("This request id was already used for another proposal.".into());
            }
            return Ok(item);
        }
        let origin: ReviewOrigin =
            serde_json::from_value(params["origin"].clone()).map_err(|error| error.to_string())?;
        let at = now_secs();
        let mut item = ReviewItem {
            id,
            runner_id: bot.runner_id,
            bot_id: bot.id,
            request_hash,
            origin,
            target: ReviewTarget {
                account: String::new(),
                resource: String::new(),
            },
            rationale: String::new(),
            payload: ReviewPayload::Draft {
                text: String::new(),
            },
            version: 1,
            revision: 0,
            preconditions: ReviewPreconditions::default(),
            state: ReviewState::Pending,
            approval: None,
            outcome: None,
            history: Vec::new(),
            created_at: at,
            updated_at: at,
            is_message: false,
        };
        apply_fields(app, &mut item, params)?;
        item.is_message = crate::drafts::tool_of(app, &item.payload).is_some();
        let workdir = queue::local_bot(app, &item)?.working_directory(&app.config.home);
        std::fs::create_dir_all(&workdir).map_err(|error| error.to_string())?;
        item.preconditions = prepare(app, &item, &cancel).await?.preconditions;
        {
            let _lock = app.review_lock.lock().unwrap();
            if let Ok(existing) = queue::get(app, &item.id) {
                if existing.request_hash != item.request_hash {
                    return Err("This request id was already used for another proposal.".into());
                }
                return Ok(existing);
            }
            if cancel.is_cancelled() {
                return Err("Stopped".into());
            }
            authorize_execution(app, &item)?;
            item.record(ReviewChange::Created, actor, None);
            queue::save(app, &item, None, ReviewChange::Created)?;
        }
        queue::publish_origin(app, &item.id);
        return Ok(item);
    }
    let id = params["id"].as_str().ok_or("missing id")?;
    let (mut candidate, original) = queue::load(app, id)?;
    queue::require_version(&candidate, params)?;
    editable(&candidate)?;
    let old_payload = candidate.payload.clone();
    if method == "reviews.edit" {
        apply_fields(app, &mut candidate, params)?;
    }
    let prepared = prepare(app, &candidate, &cancel).await?;
    let stale = prepared.preconditions != candidate.preconditions;
    let item = {
        let _lock = app.review_lock.lock().unwrap();
        let (_, current) = queue::load(app, id)?;
        if current != original {
            return Err("This changed on another Device. Review it again.".into());
        }
        authorize_execution(app, &candidate)?;
        if method == "reviews.edit" || stale {
            candidate.preconditions = prepared.preconditions;
            candidate.version += 1;
            candidate.state = ReviewState::Pending;
            candidate.approval = None;
            candidate.outcome = None;
            let change = if method == "reviews.edit" {
                ReviewChange::Edited
            } else {
                ReviewChange::Invalidated
            };
            candidate.record(
                change,
                actor,
                (change == ReviewChange::Edited).then_some(old_payload),
            );
            queue::save(app, &candidate, Some(&current), change)?;
        } else if method == "reviews.approve" {
            // A repeated decision keeps one approval and one feedback history event.
            if candidate.state == ReviewState::Approved {
                return Ok(candidate);
            }
            candidate.state = ReviewState::Approved;
            candidate.approval = Some(ReviewApproval {
                version: candidate.version,
                digest: candidate.reviewed_digest(),
                device_id: actor.into(),
                at: now_secs(),
            });
            candidate.record(ReviewChange::Approved, actor, None);
            queue::save(app, &candidate, Some(&current), ReviewChange::Approved)?;
        } else {
            return Err("Unknown review action".into());
        }
        candidate
    };
    queue::publish_origin(app, id);
    if stale && method == "reviews.approve" {
        return Err(STALE.into());
    }
    if method == "reviews.approve" {
        app.review_wake.notify_one();
    }
    Ok(item)
}

fn invalidate(
    app: &App,
    item: &ReviewItem,
    previous: &[u8],
    preconditions: Option<ReviewPreconditions>,
    why: &str,
) -> Result<(), String> {
    let mut next = item.clone();
    next.version += 1;
    next.state = ReviewState::Pending;
    next.approval = None;
    if let Some(preconditions) = preconditions {
        next.preconditions = preconditions;
    }
    next.outcome = Some(ReviewOutcome {
        summary: why.chars().take(512).collect(),
        result: None,
        message_id: next.message_id(),
        at: now_secs(),
    });
    next.record(ReviewChange::Invalidated, &item.runner_id, None);
    queue::save(app, &next, Some(previous), ReviewChange::Invalidated)
}

/// One durable claim is committed before calling a service or shell. A failed/ambiguous call
/// is recorded once and is never retried, even if a response or the process is lost.
pub async fn execute_approved(app: &Arc<App>, id: &str) -> Result<(), String> {
    let initial = queue::get(app, id)?;
    if initial.state != ReviewState::Approved {
        return Ok(());
    }
    let chat_lock = app.chat_lock(&initial.origin.chat_id);
    let _chat = chat_lock.lock().await;
    let (item, previous) = queue::load(app, id)?;
    if item.state != ReviewState::Approved {
        return Ok(());
    }
    let cancel = CancellationToken::new();
    let prepared = prepare(app, &item, &cancel).await;
    {
        let _lock = app.review_lock.lock().unwrap();
        let (_, current) = queue::load(app, id)?;
        if current != previous {
            return Ok(());
        }
        let approval_valid = item.approval.as_ref().is_some_and(|approval| {
            approval.version == item.version && approval.digest == item.reviewed_digest()
        });
        let why = match &prepared {
            Err(error) => Some(error.as_str()),
            Ok(prepared) if prepared.preconditions != item.preconditions => Some(STALE),
            _ if !approval_valid => Some(STALE),
            _ => None,
        };
        if let Some(why) = why {
            invalidate(
                app,
                &item,
                &current,
                prepared
                    .as_ref()
                    .ok()
                    .map(|prepared| prepared.preconditions.clone()),
                why,
            )?;
            drop(_lock);
            queue::publish_origin(app, id);
            return Ok(());
        }
        // Check the latest policy/assignment again after connection/review waits.
        authorize_execution(app, &item)?;
        let mut claimed = item.clone();
        claimed.state = ReviewState::Executing;
        claimed.revision += 1;
        claimed.updated_at = now_secs();
        queue::save(app, &claimed, Some(&current), ReviewChange::Approved)?;
    }
    // Re-resolve and compare once more after committing the claim. A change while the
    // connection or SQLite was awaited must not inherit the approval for the old state.
    let verified = prepare(app, &item, &cancel).await;
    {
        let _lock = app.review_lock.lock().unwrap();
        let (claimed, previous) = queue::load(app, id)?;
        if claimed.state != ReviewState::Executing || claimed.version != item.version {
            return Ok(());
        }
        let why = match &verified {
            Err(error) => Some(error.as_str()),
            Ok(verified) if verified.preconditions != item.preconditions => Some(STALE),
            _ => None,
        };
        if let Some(why) = why {
            invalidate(
                app,
                &claimed,
                &previous,
                verified
                    .as_ref()
                    .ok()
                    .map(|prepared| prepared.preconditions.clone()),
                why,
            )?;
            drop(_lock);
            queue::publish_origin(app, id);
            return Ok(());
        }
        authorize_execution(app, &claimed)?;
    }
    let prepared = verified?;
    let message = crate::drafts::tool_of(app, &item.payload).map(|(tool, _)| tool);
    let result = match prepared.tool {
        // The accepted text goes back to the chat, where the bot's later turns read it.
        None => Ok(ToolResult::text(match &item.payload {
            ReviewPayload::Draft { text } => text.clone(),
            _ => String::new(),
        })),
        Some(tool) => {
            let mut arguments = match &item.payload {
                ReviewPayload::Shell { arguments } | ReviewPayload::Plugin { arguments, .. } => {
                    arguments.clone()
                }
                _ => unreachable!(),
            };
            match message.as_ref().map(|message| crate::drafts::restore(app, message, &mut arguments)) {
                Some(Err(error)) => Ok(ToolResult { is_error: true, ..ToolResult::text(error) }),
                _ => tokio::time::timeout(
                    std::time::Duration::from_secs(10 * 60),
                    tool.execute(id, arguments, cancel.clone(), Arc::new(|_| {})),
                )
                .await
                .unwrap_or_else(|_| {
                    cancel.cancel();
                    Err(ToolError("timed out after 10 minutes".into()))
                }),
            }
        }
    };
    // A server that only makes drafts sends the one it made now.
    let result = match (result, &message, &item.payload) {
        (Ok(made), Some(message), ReviewPayload::Plugin { plugin_id, server_name, .. }) if !made.is_error && message.send.is_some() => {
            match crate::drafts::deliver(app, plugin_id, server_name, message, made.structured.as_ref().unwrap_or(&Value::Null)).await {
                Ok(()) => Ok(made),
                Err(crate::drafts::Undelivered::Refused(why)) => Ok(ToolResult { is_error: true, ..ToolResult::text(why) }),
                Err(crate::drafts::Undelivered::Unknown(why)) => Err(ToolError(why)),
            }
        }
        (result, ..) => result,
    };
    let (state, summary, output) = match result {
        Ok(result) => {
            let text = result.text_content();
            let shown: String = text.chars().take(2000).collect();
            let failed = result.is_error
                || result
                    .structured
                    .as_ref()
                    .and_then(|value| value["exit_code"].as_i64())
                    .is_some_and(|code| code != 0);
            let mut output = json!({ "text": shown, "details": result.details });
            if serde_json::to_vec(&output).map_err(|error| error.to_string())?.len() > 24 * 1024 {
                output = json!({ "text": shown });
            }
            (
                if failed {
                    ReviewState::Failed
                } else {
                    ReviewState::Succeeded
                },
                if failed {
                    "Failed".into()
                } else if matches!(item.payload, ReviewPayload::Draft { .. }) {
                    "Accepted".into()
                } else if message.is_some() {
                    "Sent".into()
                } else {
                    "Done".into()
                },
                Some(output),
            )
        }
        Err(error) if message.is_some() => (
            ReviewState::Uncertain,
            format!(
                "The send was cut off ({}), so it may have gone out. Check {} before sending it again.",
                error.0.chars().take(200).collect::<String>(),
                crate::drafts::tool_of(app, &item.payload).map(|(_, account)| account).unwrap_or_default()
            ),
            None,
        ),
        Err(error) => (
            ReviewState::Uncertain,
            format!(
                "It may have run, but it ended without a result ({}). Check before trying again.",
                error.0.chars().take(512).collect::<String>()
            ),
            None,
        ),
    };
    {
        let _lock = app.review_lock.lock().unwrap();
        let (mut finished, previous) = queue::load(app, id)?;
        if finished.state != ReviewState::Executing || finished.version != item.version {
            return Err("The execution claim changed.".into());
        }
        finished.state = state;
        finished.outcome = Some(ReviewOutcome {
            summary,
            result: output,
            message_id: finished.message_id(),
            at: now_secs(),
        });
        finished.record(ReviewChange::Executed, &item.runner_id, None);
        queue::save(app, &finished, Some(&previous), ReviewChange::Executed)?;
    }
    queue::publish_origin(app, id);
    if let Ok(finished) = queue::get(app, id) {
        queue::record_on_task(app, &finished).await;
    }
    Ok(())
}

pub fn recover_interrupted(app: &App) -> Result<(), String> {
    for item in queue::list(app)? {
        if item.state != ReviewState::Executing
            || app.this_device_id().as_deref() != Some(item.runner_id.as_str())
        {
            continue;
        }
        {
            let _lock = app.review_lock.lock().unwrap();
            let (mut current, previous) = queue::load(app, &item.id)?;
            if current.state != ReviewState::Executing {
                continue;
            }
            current.state = ReviewState::Uncertain;
            current.outcome = Some(ReviewOutcome { summary: "The Runner restarted while this was running. Check whether it finished before trying again.".into(),
                result: None, message_id: current.message_id(), at: now_secs() });
            current.record(ReviewChange::Interrupted, &item.runner_id, None);
            queue::save(app, &current, Some(&previous), ReviewChange::Interrupted)?;
        }
        queue::publish_origin(app, &item.id);
    }
    // Repairs the originating message after a crash between the item commit and chat write.
    for item in queue::list(app)? {
        if app.this_device_id().as_deref() == Some(item.runner_id.as_str()) {
            queue::publish_origin(app, &item.id);
        }
    }
    Ok(())
}

/// Resumes approved calls at startup and after each approval, one task per item.
pub async fn run(app: Arc<App>) {
    if let Err(error) = recover_interrupted(&app) {
        tracing::error!(%error, "recovering reviews");
        return;
    }
    let running: Arc<std::sync::Mutex<std::collections::HashSet<String>>> = Default::default();
    loop {
        let approved = queue::list(&app).unwrap_or_default().into_iter().filter(|item| {
            item.state == ReviewState::Approved && app.this_device_id().as_deref() == Some(item.runner_id.as_str())
        });
        for item in approved {
            if !running.lock().unwrap().insert(item.id.clone()) {
                continue;
            }
            let (app, running) = (app.clone(), running.clone());
            tokio::spawn(async move {
                if let Err(error) = execute_approved(&app, &item.id).await {
                    tracing::warn!(%error, review_id = %item.id, "executing review");
                }
                running.lock().unwrap().remove(&item.id);
            });
        }
        // An approval saved since the list was read left a permit, so it is not missed.
        app.review_wake.notified().await;
    }
}

/// Stages the exact held call. It returns at once, with no permission waiter or model turn
/// retained. A repeat of the same call id resolves to the same durable item. Stopping the turn
/// (`cancel`) stages nothing.
#[allow(clippy::too_many_arguments)]
pub async fn stage_call(
    app: &Arc<App>,
    bot: &crate::model::Bot,
    chat_id: &str,
    trigger: &Trigger,
    call_id: &str,
    payload: ReviewPayload,
    target: ReviewTarget,
    reason: Option<&str>,
    cancel: &CancellationToken,
) -> Result<ReviewItem, String> {
    mutate_until(app, "reviews.create", &json!({ "bot_id": bot.id, "request_id": format!("{}:{call_id}", trigger.message_id),
        "origin": { "chat_id": chat_id, "message_id": (!trigger.message_id.is_empty()).then_some(&trigger.message_id),
            "routine_id": trigger.routine.as_ref().map(|routine| &routine.id), "task_id": null },
        "payload": payload, "target": target, "rationale": reason.unwrap_or("This action needs your approval.") }), &bot.runner_id, cancel).await
}

pub struct StageReview {
    pub app: Arc<App>,
    pub bot: crate::model::Bot,
    pub chat_id: String,
    pub trigger: Trigger,
}

#[async_trait]
impl Tool for StageReview {
    fn name(&self) -> &str {
        "stage_review"
    }
    fn description(&self) -> &str {
        "Stage an editable draft or exact proposed shell/plugin call for the user to review later. This does not execute it or wait for permission. Include target account/resource, rationale, and optionally a canonical task_id and absolute guarded_paths whose contents must stay unchanged. User edits invalidate approval; only the assigned Runner executes an approved version. Plugin proposals name the installed connection id, server name, original tool name and exact arguments."
    }
    fn parameters(&self) -> Value {
        json!({ "type": "object", "properties": {
            "payload": { "type": "object", "properties": { "kind": { "type": "string", "enum": ["draft", "shell", "plugin"] }, "text": { "type": "string" },
                "plugin_id": { "type": "string" }, "server_name": { "type": "string" }, "tool": { "type": "string" }, "arguments": { "type": "object" } }, "required": ["kind"] },
            "target": { "type": "object", "properties": { "account": { "type": "string" }, "resource": { "type": "string" } }, "required": ["account", "resource"] },
            "rationale": { "type": "string" }, "task_id": { "type": "string" }, "guarded_paths": { "type": "array", "items": { "type": "string" } }
        }, "required": ["payload", "target", "rationale"] })
    }
    async fn execute(
        &self,
        id: &str,
        mut args: Value,
        _cancel: CancellationToken,
        _update: ToolUpdateFn,
    ) -> Result<ToolResult, ToolError> {
        args["bot_id"] = json!(self.bot.id);
        args["request_id"] = json!(format!("{}:{id}", self.trigger.message_id));
        args["origin"] = json!({ "chat_id": self.chat_id, "message_id": self.trigger.message_id, "routine_id": self.trigger.routine.as_ref().map(|routine| &routine.id), "task_id": args["task_id"] });
        let item = mutate(&self.app, "reviews.create", &args, &self.bot.runner_id)
            .await
            .map_err(ToolError)?;
        Ok(ToolResult::text(format!("Staged review {} (version {}). The user can edit, approve, reject, or cancel it later; the action has not run.", item.id, item.version))
            .with_details(json!({ "review_id": item.id, "version": item.version, "summary": "Staged for review" })))
    }
}
