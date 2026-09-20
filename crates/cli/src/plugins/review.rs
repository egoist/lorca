//! Auto-review: the check a Runner runs before an action that may have effects, after Grok
//! Bot's. An exact scoped rule (from a card's Always allow) decides at once; otherwise, with
//! Auto-review on, the bot's own model judges the one action against the user's rules and the
//! built-in checks and answers allow or ask; with it off, every such action asks.

use std::sync::Arc;

use futures::StreamExt;
use serde_json::Value;
use lorca_agent::provider::AssistantAccumulator;
use lorca_agent::types::{LlmMessage, StopReason, UserMessage};
use lorca_agent::{ModelRequest, RequestOptions};
use tokio_util::sync::CancellationToken;

use crate::app::App;
use crate::model::{Author, Body, Bot};

/// What happens to the action: it runs, or the user is asked, with why when Auto-review
/// itself paused it.
#[derive(Debug, Clone, PartialEq)]
pub enum Outcome {
    Allow,
    Ask { reason: Option<String> },
}

const SYSTEM_PROMPT: &str = "You are Auto-review, the safety check that runs before a bot acts on a connected service or its Runner. \
Decide whether this one action may run on its own or must be shown to the user first. Answer with JSON only, \
{\"verdict\": \"allow\" | \"ask\", \"reason\": \"one short sentence\"}, nothing else.\n\n\
Built-in checks that always apply. Ask when the action deletes or overwrites existing data, sends or posts something \
other people will see (a message, an email, a comment, a reply, a review), spends money or touches billing, changes who \
has access, affects many items at once, runs with elevated privileges, reads credentials or private keys, uploads local \
data, changes system configuration, executes downloaded or obfuscated code, writes outside the bot's working directory, \
or cannot be undone easily. Allow contained, reversible work in the user's own space that the user's request plainly \
calls for: creating a draft, an issue, a page, a branch, or a task; editing or closing something the bot itself just \
made; updating a field the user asked to change; building or testing inside the named working directory. The user's rules come first: \
an allow rule that covers the action means allow, an ask rule that covers it means ask, and ask wins when both apply. \
When unsure, ask. The reason is shown to the user, so write it about the action, not about yourself.";

/// Decides one effectful action for `bot`.
#[allow(clippy::too_many_arguments)]
pub async fn decide(app: &Arc<App>, bot: &Bot, chat_id: &str, plugin_id: &str, plugin_name: &str, tool: &str, description: &str, args: &Value, cancel: &CancellationToken) -> Outcome {
    let key = format!("{plugin_id}/{tool}");
    decide_with_rule_key(app, bot, chat_id, Some(&key), plugin_name, tool, description, args, cancel).await
}

/// The same review with a host-supplied exact rule key. Local actions use a key that includes
/// the Runner, workspace, and normalized action, while the model still sees the plain tool
/// name. `None` means that only natural-language rules and the reviewer decide.
#[allow(clippy::too_many_arguments)]
pub async fn decide_with_rule_key(
    app: &Arc<App>,
    bot: &Bot,
    chat_id: &str,
    rule_key: Option<&str>,
    target_name: &str,
    tool: &str,
    description: &str,
    args: &Value,
    cancel: &CancellationToken,
) -> Outcome {
    let auto_review = app.auto_review();
    if !auto_review.is_enabled {
        return Outcome::Ask { reason: None };
    }
    if let Some(rule) = rule_key.and_then(|key| auto_review.rules.iter().find(|rule| rule.tool.as_deref() == Some(key))) {
        return if rule.behavior == "allow" { Outcome::Allow } else { Outcome::Ask { reason: Some(format!("Your rule: {}", rule.text)) } };
    }
    let provider = match crate::providers::provider_for(app, &bot.provider, bot.model.as_deref(), Some(lorca_agent::types::ThinkingLevel::Off)) {
        Ok(provider) => provider,
        Err(error) => return Outcome::Ask { reason: Some(format!("Auto-review could not check this action ({error}).")) },
    };
    // A card-created exact rule applies only to its structured key. Feeding unrelated exact
    // rules to the model would accidentally broaden them through their human-readable label.
    let allow: Vec<&str> = auto_review.rules.iter().filter(|r| r.tool.is_none() && r.behavior == "allow").map(|r| r.text.as_str()).collect();
    let ask: Vec<&str> = auto_review.rules.iter().filter(|r| r.tool.is_none() && r.behavior == "ask").map(|r| r.text.as_str()).collect();
    let mut text = String::new();
    if !allow.is_empty() {
        text.push_str("Rules that allow automatically, when the bot wants to:\n");
        for rule in &allow {
            text.push_str(&format!("- {rule}\n"));
        }
        text.push('\n');
    }
    if !ask.is_empty() {
        text.push_str("Rules that ask first, when the bot wants to:\n");
        for rule in &ask {
            text.push_str(&format!("- {rule}\n"));
        }
        text.push('\n');
    }
    if let Some(request) = last_user_text(app, chat_id) {
        text.push_str(&format!("The user's latest message to the bot:\n{request}\n\n"));
    }
    let mut arguments = serde_json::to_string_pretty(args).unwrap_or_default();
    if arguments.len() > 4000 {
        arguments.truncate(4000);
        arguments.push_str("\n…");
    }
    text.push_str(&format!(
        "The action: bot {} wants to call {tool} on {target_name}.\nWhat the tool does: {}\nArguments:\n{arguments}",
        bot.name,
        if description.trim().is_empty() { "(no description)" } else { description.trim() }
    ));
    let request = ModelRequest {
        system_prompt: SYSTEM_PROMPT.into(),
        messages: vec![LlmMessage::User(UserMessage::text(text))],
        tools: Vec::new(),
        max_tokens: Some(200),
        options: RequestOptions::default().with_session_id(chat_id),
    };
    let mut stream = provider.stream(request, cancel.clone()).await;
    let mut acc = AssistantAccumulator::new(provider.provider_id(), provider.model_id());
    while let Some(event) = stream.next().await {
        acc.apply(&event);
    }
    let message = acc.finish(cancel.is_cancelled());
    if matches!(message.stop_reason, StopReason::Aborted | StopReason::Error) {
        let error = message.error_message.unwrap_or_else(|| "no answer".into());
        tracing::warn!(%error, "auto-review call failed");
        return Outcome::Ask { reason: Some("Auto-review could not check this action.".into()) };
    }
    match parse_verdict(&message.text()) {
        Some((true, _)) => Outcome::Allow,
        Some((false, reason)) => Outcome::Ask { reason: Some(reason) },
        None => {
            tracing::warn!(reply = %message.text(), "auto-review answered off-format");
            Outcome::Ask { reason: Some("Auto-review could not read its own check.".into()) }
        }
    }
}

/// `(allowed, reason)` from the model's JSON, tolerating prose or a code fence around it.
fn parse_verdict(reply: &str) -> Option<(bool, String)> {
    let start = reply.find('{')?;
    let end = reply.rfind('}')?;
    let value: Value = serde_json::from_str(&reply[start..=end]).ok()?;
    let verdict = value["verdict"].as_str()?.trim().to_ascii_lowercase();
    let reason = value["reason"].as_str().unwrap_or("").trim().to_string();
    match verdict.as_str() {
        "allow" => Some((true, reason)),
        "ask" => Some((false, if reason.is_empty() { "This action needs a look first.".into() } else { reason })),
        _ => None,
    }
}

fn last_user_text(app: &App, chat_id: &str) -> Option<String> {
    let chat = app.chat(chat_id)?;
    let text = chat.messages.iter().rev().find_map(|m| match (&m.author, &m.body) {
        (Author::You, Body::Text { text, .. }) if !text.trim().is_empty() => Some(text.trim().to_string()),
        _ => None,
    })?;
    Some(text.chars().take(600).collect())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn verdicts_parse_with_fences_and_prose() {
        assert_eq!(parse_verdict("```json\n{\"verdict\": \"allow\", \"reason\": \"A draft.\"}\n```"), Some((true, "A draft.".into())));
        assert_eq!(parse_verdict("Sure: {\"verdict\":\"ASK\",\"reason\":\"It deletes a repo.\"}"), Some((false, "It deletes a repo.".into())));
        assert_eq!(parse_verdict("{\"verdict\":\"ask\"}"), Some((false, "This action needs a look first.".into())));
        assert_eq!(parse_verdict("{\"verdict\":\"maybe\"}"), None);
        assert_eq!(parse_verdict("no json here"), None);
    }
}
