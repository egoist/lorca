//! Questions one Device asks a Runner through the relay: a `request` blob sealed to the Runner's
//! box key, answered with a `response` blob sealed back to the Device that asked. The verbs read
//! and write a bot's memory, which lives on the bot's Runner, so the Mac and the phone show and
//! edit it for a bot that runs on another machine. A turn's `job` / `job_result` pair works the
//! same way; this is the same idea for questions with an answer.

use std::sync::Arc;
use std::time::Duration;

use serde_json::{json, Value};

use crate::app::App;
use crate::config::now_secs;
use crate::memory::MemoryStore;
use crate::model::{Request, Response};

/// How long a question may wait for its answer. A Runner that is online answers in a few
/// seconds; past this the relay is slow or the Runner just went away.
pub const REQUEST_TIMEOUT: Duration = Duration::from_secs(20);

/// Asks `runner_id` to run `verb` and waits for the answer. Refuses up front when the Runner is
/// unknown, unreachable, or offline, so an editor never spins on a machine that is asleep.
pub async fn ask(app: &Arc<App>, runner_id: &str, verb: &str, body: Value) -> Result<Value, String> {
    let runner = app.device(runner_id).ok_or("That bot is assigned to a Runner this Device does not know yet.")?;
    if runner.box_pubkey.is_empty() {
        return Err(format!("{} has not shared its key yet.", runner.name));
    }
    if app.relay_url().is_none() {
        return Err(format!("No relay is configured, so this Device cannot reach {}.", runner.name));
    }
    if !app.device_is_online(runner_id) {
        return Err(format!("{} is offline.", runner.name));
    }
    let request = Request {
        id: format!("req-{}", uuid::Uuid::new_v4()),
        verb: verb.to_string(),
        requested_by: app.this_device_id().unwrap_or_default(),
        body,
        created_at: now_secs(),
    };
    let ciphertext = crate::crypto::seal_json(&runner.box_pubkey, &request).map_err(|e| e.to_string())?;
    let (tx, rx) = tokio::sync::oneshot::channel();
    app.pending_responses.lock().unwrap().insert(request.id.clone(), tx);
    app.push_blob("request", Some(runner.id.clone()), ciphertext);
    let answer = tokio::time::timeout(REQUEST_TIMEOUT, rx).await;
    app.pending_responses.lock().unwrap().remove(&request.id);
    match answer {
        Ok(Ok(Response { error: Some(error), .. })) => Err(error),
        Ok(Ok(Response { body, .. })) => Ok(body),
        Ok(Err(_)) | Err(_) => Err(format!("{} did not answer in time.", runner.name)),
    }
}

/// A `response` reached this Device: hands it to the waiting `ask`, then drops the blob from
/// the relay.
pub fn deliver(app: Arc<App>, response: Response, blob_id: String) {
    if let Some(tx) = app.pending_responses.lock().unwrap().remove(&response.request_id) {
        let _ = tx.send(response);
    }
    tokio::spawn(async move { crate::sync::delete_remote_blob(&app, &blob_id).await });
}

/// A `request` reached this Runner: answers it, seals the answer to the Device that asked, and
/// drops the request from the relay.
pub fn serve(app: Arc<App>, request: Request, blob_id: String) {
    tokio::spawn(async move {
        let (body, error) = match answer(&app, &request) {
            Ok(body) => (body, None),
            Err(error) => (Value::Null, Some(error)),
        };
        let response = Response { request_id: request.id.clone(), body, error };
        match app.device(&request.requested_by).filter(|d| !d.box_pubkey.is_empty()) {
            Some(requester) => match crate::crypto::seal_json(&requester.box_pubkey, &response) {
                Ok(ciphertext) => {
                    app.push_blob("response", Some(requester.id.clone()), ciphertext);
                }
                Err(error) => tracing::warn!(%error, "sealing a response"),
            },
            None => tracing::warn!(verb = %request.verb, "a request from a Device this Runner does not know"),
        }
        crate::sync::delete_remote_blob(&app, &blob_id).await;
    });
}

/// What this Runner can be asked. Every verb checks that the bot runs here: a request that
/// reached the wrong machine is refused, not forwarded.
fn answer(app: &Arc<App>, request: &Request) -> Result<Value, String> {
    let bot_id = request.body["bot_id"].as_str().ok_or("missing bot_id")?;
    match request.verb.as_str() {
        "memory.read" => memory_read(app, bot_id),
        "memory.write" => {
            let text = request.body["text"].as_str().ok_or("missing text")?;
            memory_write(app, bot_id, text, request.body["expected_hash"].as_str())
        }
        other => Err(format!("Unknown request {other}")),
    }
}

/// A bot's memory as this Runner has it: the index with its budget and the other files by name.
pub fn memory_read(app: &Arc<App>, bot_id: &str) -> Result<Value, String> {
    let bot = local_bot(app, bot_id)?;
    Ok(MemoryStore::for_bot(&app.config.home, &bot).overview())
}

/// Replaces a bot's MEMORY.md on this Runner, refusing when it changed since `expected_hash`.
pub fn memory_write(app: &Arc<App>, bot_id: &str, text: &str, expected_hash: Option<&str>) -> Result<Value, String> {
    let bot = local_bot(app, bot_id)?;
    let hash = MemoryStore::for_bot(&app.config.home, &bot).write_index(text, expected_hash).map_err(|e| e.to_string())?;
    Ok(json!({ "hash": hash }))
}

fn local_bot(app: &Arc<App>, bot_id: &str) -> Result<crate::model::Bot, String> {
    let bot = app.bot(bot_id).ok_or("Unknown bot")?;
    if app.this_device_id().as_deref() != Some(bot.runner_id.as_str()) {
        let runner = app.device(&bot.runner_id).map(|d| d.name).unwrap_or_else(|| "another Runner".into());
        return Err(format!("{} runs on {runner}, not here.", bot.name));
    }
    Ok(bot)
}
