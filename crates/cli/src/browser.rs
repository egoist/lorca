//! A bot's browser profiles. They live on the bot's Runner, which answers every `browser.*`
//! method; another Device sends the same methods to it as sealed requests.

use crate::app::App;
#[cfg(feature = "runner")]
use serde_json::json;
use serde_json::Value;
use std::sync::Arc;

#[cfg(feature = "runner")]
mod runner;
#[cfg(feature = "runner")]
pub use runner::{publish_image, review_call, SessionTool, Sessions};

pub const PLUGIN_ID: &str = "playwright";

pub async fn dispatch(app: &Arc<App>, method: &str, params: Value) -> Result<Value, String> {
    let bot_id = params["bot_id"].as_str().ok_or("missing bot_id")?;
    let bot = app.bot(bot_id).ok_or("Unknown bot")?;
    if app.this_device_id().as_deref() != Some(bot.runner_id.as_str()) {
        // A window opens only on the screen of the Device that asks for it.
        if method == "browser.open" {
            let runner = app.device(&bot.runner_id).map(|device| device.name).unwrap_or_else(|| "its Runner".into());
            return Err(format!("Open it on {runner}."));
        }
        return crate::requests::ask_within(app, &bot.runner_id, method, params.clone(), std::time::Duration::from_secs(150)).await;
    }
    #[cfg(feature = "runner")]
    return serve(app, method, &params, false).await;
    #[cfg(not(feature = "runner"))]
    Err("Browser profiles need a Runner.".into())
}

/// A sealed request is already addressed to this Runner; it never opens a window here for a
/// Device somewhere else.
#[cfg(feature = "runner")]
pub async fn serve(app: &Arc<App>, method: &str, params: &Value, remote: bool) -> Result<Value, String> {
    let bot_id = params["bot_id"].as_str().ok_or("missing bot_id")?;
    let sessions = &app.browser_sessions;
    sessions.local_bot(app, bot_id)?;
    if method == "browser.sessions" {
        return Ok(json!({ "sessions": sessions.list(app, bot_id)? }));
    }
    if method == "browser.create" {
        return Ok(json!({ "session": sessions.create(app, bot_id, params["name"].as_str().unwrap_or_default())? }));
    }
    let id = params["session_id"].as_str().ok_or("missing session_id")?;
    let session = match method {
        "browser.open" if remote => return Err("Open it on its Runner.".into()),
        "browser.open" => sessions.open(app, bot_id, id, true).await?,
        "browser.takeover" => sessions.takeover(app, bot_id, id, !remote).await?,
        "browser.resume" => sessions.resume(app, bot_id, id, params["revision"].as_u64().ok_or("missing revision")?).await?,
        "browser.stop" => sessions.stop(app, bot_id, id).await?,
        "browser.delete" => {
            sessions.delete(app, bot_id, id).await?;
            return Ok(json!({}));
        }
        "browser.screenshot" => {
            let chat_id = params["chat_id"].as_str().ok_or("missing chat_id")?;
            let message = sessions.screenshot(app, bot_id, id, chat_id).await?;
            return Ok(json!({ "message_id": message.id }));
        }
        _ => return Err(format!("Unknown method {method}")),
    };
    Ok(json!({ "session": session }))
}
