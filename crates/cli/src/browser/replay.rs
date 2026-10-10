//! Running a skill's recorded browser steps (`browser_session { action: "run" }`): each step in
//! the bot's profile through the Browser server's own tools, with the first of its targets the
//! page has, then what it expects afterwards. A step the page no longer matches stops the run
//! with a screenshot in the chat instead of guessing, and ends the turn.

use std::collections::BTreeMap;
use std::time::{Duration, Instant};

use lorca_agent::ContentPart;
use rmcp::model::ContentBlock;

use super::*;
use crate::browser::steps::{self, Action, Expect, Step, Steps};

/// How long a run waits before it looks for a step's element, or what it expects, again.
const RETRY: Duration = Duration::from_millis(700);

/// Errors that say a target isn't the element on this page, so the next target is tried:
/// nothing matches, more than one does, or what matches can't take the action.
const NOT_THIS_ELEMENT: [&str; 12] = [
    "does not match any elements",
    "strict mode violation",
    "Timeout",
    "not visible",
    "not enabled",
    "not editable",
    "Element is not",
    "intercepts pointer events",
    "not attached",
    "Not a checkbox",
    "did not find some options",
    "not found in the current page snapshot",
];

/// The steps of a saved skill this turn sees, by its name, id, or `playbook://` path.
pub fn skill_steps(app: &App, bot_id: &str, chat_id: &str, skill: &str) -> Result<(String, Steps), String> {
    let scopes = crate::playbooks::scopes_for_turn(app, bot_id, chat_id);
    let (name, text) = crate::playbooks::resource_for_turn(app, &scopes, skill, steps::PATH)?;
    Ok((name, steps::parse(&text)?))
}

enum Halt {
    Stopped,
    /// The page doesn't match the step, in words.
    Mismatch(String),
}

fn text_of(result: &rmcp::model::CallToolResult) -> String {
    result.content.iter().filter_map(|block| match block { ContentBlock::Text(text) => Some(text.text.as_str()), _ => None }).collect::<Vec<_>>().join("\n")
}

/// An error's first line that says something, without Playwright's headings.
fn first_line(error: &str) -> String {
    let line = error.lines().map(str::trim).find(|line| !line.is_empty() && !line.starts_with('#')).unwrap_or("it failed");
    line.trim_start_matches("Error: ").chars().take(300).collect()
}

async fn pause(cancel: &CancellationToken) -> Result<(), Halt> {
    tokio::select! {
        _ = tokio::time::sleep(RETRY) => Ok(()),
        _ = cancel.cancelled() => Err(Halt::Stopped),
    }
}

/// Runs `steps` (with their inputs in place) in the bot's profile they name, opening it for the
/// bot first when it is closed.
pub async fn run(app: &Arc<App>, bot: &Bot, chat_id: &str, skill: &str, inputs: &BTreeMap<String, String>, cancel: &CancellationToken) -> Result<ToolResult, ToolError> {
    let (name, steps) = skill_steps(app, &bot.id, chat_id, skill).map_err(ToolError)?;
    let steps = steps.with_inputs(inputs).map_err(ToolError)?;
    if let Some(index) = steps.steps.iter().position(|step| step.secret) {
        return Err(ToolError(format!("Step {} of {name} types a password, which a skill doesn't keep. Ask the user to sign in in the {} browser, and leave that step out of the skill.", index + 1, steps.profile)));
    }
    let sessions = &app.browser_sessions;
    let profile = sessions
        .list(app, &bot.id)
        .map_err(ToolError)?
        .into_iter()
        .find(|session| session.name.trim().eq_ignore_ascii_case(steps.profile.trim()))
        .ok_or_else(|| ToolError(format!("{name} runs in the “{}” browser profile, which you don't have on this Runner. Add it with browser_session create, and ask the user to sign in there.", steps.profile)))?;
    if profile.state == Control::Stopped || (profile.state == Control::Bot && !profile.selected) {
        tokio::select! {
            opened = sessions.open(app, &bot.id, &profile.id, false) => { opened.map_err(ToolError)?; }
            _ = cancel.cancelled() => return Err(ToolError("Stopped".into())),
        }
    }
    let total = steps.steps.len();
    for (index, step) in steps.steps.iter().enumerate() {
        // Each step holds the browser's input gate: a takeover waits for the step, and the run
        // waits for the user to hand the browser back.
        let input = sessions.input(app, &bot.id, cancel).await.map_err(ToolError)?;
        let Some(input) = input.filter(|input| input.name == profile.name) else {
            return Err(ToolError(format!("The {} browser closed at step {} of {total}.", profile.name, index + 1)));
        };
        match perform(&input.server, step, cancel).await {
            Ok(()) => {}
            Err(Halt::Stopped) => return Err(ToolError("Stopped".into())),
            Err(Halt::Mismatch(why)) if input.server.is_closed() => return Err(ToolError(format!("The {} browser closed at step {} of {total}: {why}", profile.name, index + 1))),
            Err(Halt::Mismatch(why)) => return Ok(stop_here(app, bot, chat_id, &input, &name, index, total, step, &why).await),
        }
    }
    Ok(ToolResult::text(format!("Ran the {total} steps of {name} in the {} browser.", profile.name)))
}

async fn perform(server: &Server, step: &Step, cancel: &CancellationToken) -> Result<(), Halt> {
    if cancel.is_cancelled() {
        return Err(Halt::Stopped);
    }
    let deadline = Instant::now() + Duration::from_secs(step.timeout.unwrap_or(steps::DEFAULT_TIMEOUT_SECS));
    match step.action {
        Action::Goto => {
            server.browser_call("browser_navigate", json!({ "url": step.url })).await.map_err(|error| Halt::Mismatch(format!("the page didn't open: {}", first_line(&error))))?;
        }
        Action::Press => {
            server.browser_call("browser_press_key", json!({ "key": step.key })).await.map_err(|error| Halt::Mismatch(first_line(&error)))?;
        }
        _ => act(server, step, deadline, cancel).await?,
    }
    if let Some(expect) = &step.expect {
        expected(server, expect, deadline, cancel).await?;
    }
    Ok(())
}

/// The step's action on the first of its targets that is the element, looking again until the
/// step's time is up: the page may still be loading.
async fn act(server: &Server, step: &Step, deadline: Instant, cancel: &CancellationToken) -> Result<(), Halt> {
    let mut last = String::new();
    loop {
        for target in &step.targets {
            let result = match step.action {
                Action::Click => server.browser_call("browser_click", json!({ "target": target })).await,
                Action::Fill => server.browser_call("browser_type", json!({ "target": target, "text": step.value })).await,
                Action::Select => server.browser_call("browser_select_option", json!({ "target": target, "values": step.values })).await,
                Action::Check | Action::Uncheck => {
                    let field = json!({ "name": step.element.as_deref().unwrap_or("field"), "type": "checkbox", "target": target, "value": (step.action == Action::Check).to_string() });
                    server.browser_call("browser_fill_form", json!({ "fields": [field] })).await
                }
                Action::Upload => {
                    let clicked = server.browser_call("browser_click", json!({ "target": target })).await;
                    match clicked {
                        Ok(clicked) if !text_of(&clicked).contains("File chooser") => return Err(Halt::Mismatch("clicking it didn't ask for a file".into())),
                        Ok(_) => match server.browser_call("browser_file_upload", json!({ "paths": step.files })).await {
                            Ok(uploaded) => Ok(uploaded),
                            Err(error) => {
                                // The file chooser it opened would hold up the browser's next call.
                                let _ = server.browser_call("browser_file_upload", json!({})).await;
                                return Err(Halt::Mismatch(format!("the file wasn't taken: {}", first_line(&error))));
                            }
                        },
                        Err(error) => Err(error),
                    }
                }
                Action::Goto | Action::Press => return Ok(()),
            };
            match result {
                Ok(_) => return Ok(()),
                Err(error) if NOT_THIS_ELEMENT.iter().any(|words| error.contains(words)) && !server.is_closed() => last = first_line(&error),
                Err(error) => return Err(Halt::Mismatch(first_line(&error))),
            }
        }
        if Instant::now() >= deadline {
            let what = match step.targets.len() {
                1 => "its element isn't on the page".to_string(),
                count => format!("none of its {count} ways to find the element matches the page"),
            };
            return Err(Halt::Mismatch(format!("{what} ({last})")));
        }
        pause(cancel).await?;
    }
}

/// The current tab's title and address, from the Browser server's list of tabs.
async fn current_page(server: &Server) -> Option<(String, String)> {
    let tabs = server.browser_call("browser_tabs", json!({ "action": "list" })).await.ok()?;
    let text = text_of(&tabs);
    // `- 0: (current) [Title](https://…)`
    let line = text.lines().find(|line| line.contains("(current)"))?;
    let rest = line.split_once("(current) [")?.1;
    let (title, url) = rest.rsplit_once("](")?;
    Some((title.to_string(), url.trim_end().trim_end_matches(')').to_string()))
}

async fn expected(server: &Server, expect: &Expect, deadline: Instant, cancel: &CancellationToken) -> Result<(), Halt> {
    loop {
        let mut missing = Vec::new();
        if expect.url.is_some() || expect.title.is_some() {
            let (title, url) = current_page(server).await.unwrap_or_default();
            if let Some(part) = expect.url.as_deref().filter(|part| !url.contains(part)) {
                missing.push(format!("the address has no “{part}” ({url})"));
            }
            if let Some(part) = expect.title.as_deref().filter(|part| !title.contains(part)) {
                missing.push(format!("the title has no “{part}” ({title})"));
            }
        }
        if let Some(text) = &expect.text {
            if server.browser_call("browser_wait_for", json!({ "text": text })).await.is_err() {
                missing.push(format!("“{text}” isn't on the page"));
            }
        }
        if missing.is_empty() {
            return Ok(());
        }
        if Instant::now() >= deadline || server.is_closed() {
            return Err(Halt::Mismatch(format!("afterwards {}", missing.join(", and "))));
        }
        pause(cancel).await?;
    }
}

/// The run stops where the page no longer matches: a screenshot of it in the chat, a notice that
/// says where and why, and a result that ends the turn.
#[allow(clippy::too_many_arguments)]
async fn stop_here(app: &Arc<App>, bot: &Bot, chat_id: &str, input: &Input, skill: &str, index: usize, total: usize, step: &Step, why: &str) -> ToolResult {
    let shot = match input.server.browser_call("browser_take_screenshot", json!({ "type": "png" })).await {
        Ok(result) => publish_image(app, &bot.id, chat_id, &input.name, &result).ok(),
        Err(_) => None,
    };
    let at = format!("{skill} stopped at step {} of {total} ({}): {why}.", index + 1, step.describe());
    app.notice(chat_id, at.clone());
    let text = format!(
        "{at} {}The page no longer matches the recorded steps, so the run stopped there instead of guessing.",
        if shot.is_some() { "A screenshot of the page is in the chat. " } else { "" }
    );
    ToolResult { content: vec![ContentPart::text(text)], details: Value::Null, structured: None, is_error: true, terminate: true }
}
