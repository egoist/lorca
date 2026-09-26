//! Auto-review: the check a Runner runs before an action that may have effects, after Grok
//! Bot's. A rule Always allow saved for an exact plugin tool decides at once; otherwise, with
//! Auto-review on, a small model of the bot's provider judges the one action against the user's
//! plain-language rules and the built-in checks and answers allow or ask. When a shell command asks, the review
//! also proposes the plain-language rule that Always allow adds. With Auto-review off, every
//! such action asks.

use std::sync::Arc;

use futures::StreamExt;
use serde_json::Value;
use lorca_agent::provider::AssistantAccumulator;
use lorca_agent::types::{LlmMessage, StopReason, UserMessage};
use lorca_agent::{ModelRequest, RequestOptions};
use tokio_util::sync::CancellationToken;

use crate::app::App;
use crate::model::{Body, Bot, Routine};

/// What happens to the action: it runs, or the user is asked, with why when Auto-review
/// itself paused it and the allow rule it proposes for Always allow.
#[derive(Debug, Clone, PartialEq)]
pub enum Outcome {
    Allow,
    Ask { reason: Option<String>, rule: Option<String> },
}

impl Outcome {
    fn ask(reason: impl Into<String>) -> Outcome {
        Outcome::Ask { reason: Some(reason.into()), rule: None }
    }
}

/// One action as Auto-review reads it.
pub struct Action<'a> {
    /// Where it runs: the plugin's name, or the Runner's for a shell command.
    pub target_name: &'a str,
    pub tool: &'a str,
    pub description: &'a str,
    pub args: &'a Value,
    /// Asks the review for the plain-language rule Always allow adds when it asks.
    pub propose_rule: bool,
}

/// What started a turn, which the review reads as the request behind its actions: the message,
/// and for a routine's run the routine as it stood when the run began. The roster's copy can be
/// edited or deleted while the run goes on, by the bot itself too, and the run keeps the task
/// it started with.
#[derive(Debug, Clone, Default)]
pub struct Trigger {
    pub message_id: String,
    pub routine: Option<Routine>,
}

const SYSTEM_PROMPT: &str = "You are Auto-review, the safety check that runs before a bot acts on a connected service or its Runner. \
Decide whether this one action may run on its own or must be shown to the user first. Answer with JSON only, \
{\"verdict\": \"allow\" | \"ask\", \"reason\": \"one short sentence\"}, nothing else.\n\n\
Built-in checks that always apply. Ask when the action deletes or overwrites existing data, sends or posts something \
other people will see (a message, an email, a comment, a reply, a review), spends money or touches billing, changes who \
has access, affects many items at once, runs with elevated privileges, reads credentials or private keys, uploads local \
data, changes system configuration, executes downloaded or obfuscated code, writes outside the bot's working directory \
and the temporary directories, or cannot be undone easily. Temporary directories (/tmp, /var/tmp, $TMPDIR) are scratch \
space: cloning, building, overwriting, or deleting there is contained work, not the user's data. Allow contained, \
reversible work in the user's own space that the request behind the turn plainly calls for, whether the user's message, a \
teammate bot's message, or a routine's task. Allow ordinary read-only inspection, including compound pipelines, loops, grouping, and visible command \
substitutions; syntax complexity alone is not a risk when every visible command is read-only. Also allow creating a \
draft, an issue, a page, a branch, or a task; editing or closing something the bot itself just \
made; updating a field the user asked to change; building or testing inside the named working directory. The user's rules come first: \
an allow rule that covers the action means allow, an ask rule that covers it means ask, and ask wins when both apply. \
When unsure, ask. The reason is shown to the user, so write it about the action, not about yourself, in the language of \
the user's latest message (English when there is none).";

const RULE_PROMPT: &str = "When the verdict is ask, also answer \"rule\": one allow rule the user could add so that actions \
like this one run on their own from now on, in the same language as the reason. It completes \"When a bot wants to:\", \
starts with a verb, and names the kind of work rather than repeating the command. It must cover every part of this action, \
deletions included, and name the folder the command acts on: the one it changes into or the paths it names, and only when \
it names none, its working directory, with ~ for the home folder. For example, `npm install` run in ~/code/shop gives \
\"install npm packages in ~/code/shop\"; `rm -rf target` run in ~/code/shop gives \"delete build output such as target/ in \
~/code/shop\", in Chinese \"删除 ~/code/shop 中 target/ 等构建产物\"; `cd /tmp && rm -rf demo && git clone \
https://github.com/acme/demo` run anywhere gives \"delete and re-clone GitHub repositories in /tmp\". Never repeat one of \
the user's rules. Keep it as narrow as this action, so it never also covers something riskier: more deletion, force \
pushes, elevated privileges, other folders, other people. Leave out secrets, tokens, and one-off values such as ids and \
hashes. Answer \"rule\": \"\" only for an action nobody should let run unattended, such as deleting the user's documents, \
wiping data, or changing security settings.";

/// Decides one effectful plugin action for `bot`. A rule Always allow saved for this exact
/// tool decides without a review.
#[allow(clippy::too_many_arguments)]
pub async fn decide(app: &Arc<App>, bot: &Bot, chat_id: &str, trigger: &Trigger, plugin_id: &str, plugin_name: &str, tool: &str, description: &str, args: &Value, cancel: &CancellationToken) -> Outcome {
    let auto_review = app.auto_review();
    if let Some(rule) = auto_review.rule_for(plugin_id, tool).filter(|_| auto_review.is_enabled) {
        return if rule.behavior == "allow" { Outcome::Allow } else { Outcome::ask(format!("Your rule: {}", rule.text)) };
    }
    let action = Action { target_name: plugin_name, tool, description, args, propose_rule: false };
    review(app, bot, chat_id, trigger, action, cancel).await
}

/// Reviews one action of the turn that `trigger` started. With Auto-review off it asks; on,
/// the review model of the bot's provider
/// ([`review_model`](crate::providers::review_model)) judges it against the user's
/// plain-language rules, the built-in checks, and the request behind the turn ([`request`]).
pub async fn review(app: &Arc<App>, bot: &Bot, chat_id: &str, trigger: &Trigger, action: Action<'_>, cancel: &CancellationToken) -> Outcome {
    let auto_review = app.auto_review();
    if !auto_review.is_enabled {
        return Outcome::Ask { reason: None, rule: None };
    }
    if bot.harness == crate::model::Harness::Codex {
        return Outcome::ask("Approve this Lorca plugin action. Codex runs this bot; provider-based Auto-review is unavailable.");
    }
    let (model, thinking) = crate::providers::review_model(&bot.provider);
    let provider = match crate::providers::provider_for(app, &bot.provider, Some(model), Some(thinking)) {
        Ok(provider) => provider,
        Err(error) => return Outcome::ask(format!("Auto-review could not check this action ({error}).")),
    };
    // A rule Always allow saved for one plugin tool applies only to that tool. Feeding it to the
    // model would broaden it through its human-readable label.
    let allow: Vec<&str> = auto_review.rules.iter().filter(|r| r.tool.is_none() && r.behavior == "allow").map(|r| r.text.as_str()).collect();
    let ask: Vec<&str> = auto_review.rules.iter().filter(|r| r.tool.is_none() && r.behavior == "ask").map(|r| r.text.as_str()).collect();
    let mut text = String::new();
    if !allow.is_empty() {
        text.push_str("Rules that allow automatically, when the bot wants to:\n");
        for (index, rule) in allow.iter().enumerate() {
            text.push_str(&format!("{}. {rule}\n", index + 1));
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
    let request = request(app, chat_id, trigger);
    if let Some(request) = &request {
        text.push_str(&request.text);
    }
    let mut arguments = serde_json::to_string_pretty(action.args).unwrap_or_default();
    if arguments.len() > 4000 {
        arguments.truncate(arguments.floor_char_boundary(4000));
        arguments.push_str("\n…");
    }
    text.push_str(&format!(
        "The action: bot {} wants to call {} on {}.\nWhat the tool does: {}\nArguments:\n{arguments}",
        bot.name,
        action.tool,
        action.target_name,
        if action.description.trim().is_empty() { "(no description)" } else { action.description.trim() }
    ));
    if let Some(request) = &request {
        let answer = if action.propose_rule { "the reason and the rule" } else { "the reason" };
        text.push_str(&format!("\n\nWrite {answer} in the language {}.", request.language));
    }
    let request = ModelRequest {
        system_prompt: if action.propose_rule { format!("{SYSTEM_PROMPT}\n\n{RULE_PROMPT}") } else { SYSTEM_PROMPT.into() },
        messages: vec![LlmMessage::User(UserMessage::text(text))],
        tools: Vec::new(),
        cache_points: Vec::new(),
        // The verdict is short; the rest is room for a model that reasons at its lowest effort.
        max_tokens: Some(4096),
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
        return Outcome::ask("Auto-review could not check this action.");
    }
    tracing::debug!(reply = %message.text(), "auto-review verdict");
    match parse_verdict(&message.text()) {
        Some(mut verdict) => {
            // A rule the user already has did not cover this action, so offering it again would
            // leave the next one asking just the same.
            verdict.rule = verdict.rule.filter(|rule| !auto_review.rules.iter().any(|r| r.text.eq_ignore_ascii_case(rule)));
            if verdict.allow {
                Outcome::Allow
            } else {
                Outcome::Ask { reason: Some(verdict.reason), rule: verdict.rule }
            }
        }
        None => {
            tracing::warn!(reply = %message.text(), "auto-review answered off-format");
            Outcome::ask("Auto-review could not read its own check.")
        }
    }
}

/// The model's answer: the verdict, why, and the rule it proposes.
#[derive(Debug, PartialEq)]
struct Verdict {
    allow: bool,
    reason: String,
    rule: Option<String>,
}

/// The model's JSON, tolerating prose or a code fence around it.
fn parse_verdict(reply: &str) -> Option<Verdict> {
    let start = reply.find('{')?;
    let end = reply.rfind('}')?;
    let value: Value = serde_json::from_str(&reply[start..=end]).ok()?;
    let allow = match value["verdict"].as_str()?.trim().to_ascii_lowercase().as_str() {
        "allow" => true,
        "ask" => false,
        _ => return None,
    };
    let reason = value["reason"].as_str().map(str::trim).filter(|reason| !reason.is_empty());
    Some(Verdict {
        allow,
        reason: reason.unwrap_or("This action needs a look first.").to_string(),
        rule: value["rule"].as_str().and_then(rule_text),
    })
}

/// A proposed rule as the rules list shows it: one line with no closing period, or nothing when
/// the model left it empty or wrote a paragraph.
fn rule_text(rule: &str) -> Option<String> {
    let rule = rule.split_whitespace().collect::<Vec<_>>().join(" ");
    let rule = rule.trim_end_matches(['.', '。']);
    (!rule.is_empty() && rule.chars().count() <= 200).then(|| rule.to_string())
}

/// The request behind a turn, as the review reads it.
#[derive(Debug, PartialEq)]
struct Request {
    /// What opens the review's message: who asked for the work, in their words.
    text: String,
    /// The words that set the language of the answer, as they end "Write the reason in the
    /// language …": "of the user's latest message".
    language: String,
}

/// The request behind the turn that `trigger` started: the message that asked for the work
/// (the user's, a teammate's handoff, or a routine's marker) and the user's latest message
/// since. What the user wrote before it was about earlier work and is left out, so a "stop"
/// there does not reach this turn.
fn request(app: &App, chat_id: &str, trigger: &Trigger) -> Option<Request> {
    app.chat(chat_id)?;
    let opening = app.store.request_at(chat_id, &trigger.message_id).ok().flatten()?;
    let mut text = String::new();
    let mut language = None;
    match &opening.body {
        Body::Handoff { from, reason, .. } => {
            let name = app.bot(from).map(|bot| bot.name).unwrap_or_else(|| "a teammate".into());
            text.push_str(&format!("A message from the bot's teammate {name} started this turn:\n{}\n\n", clipped(reason)));
            language = Some(format!("of {name}'s message"));
        }
        Body::Notice { routine_id: Some(id), .. } => {
            // A later turn, such as a command's end, reads the roster, as its transcript does.
            let routine = trigger.routine.clone().filter(|routine| &routine.id == id).or_else(|| app.routine(id));
            if let Some(routine) = routine {
                text.push_str(&format!(
                    "This turn is a scheduled run of the bot's routine \"{}\", with nobody watching. Its task:\n{}\n\n",
                    routine.name,
                    clipped(&routine.prompt)
                ));
                // DeepSeek writes most answers to "the language of the routine's task" in Chinese.
                language = Some("the routine's task is written in".into());
            }
        }
        // The user's own message: the latest below is that one or a later one of theirs.
        _ => {}
    }
    if let Some(latest) = app.store.last_user_text(chat_id, &opening.id).ok().flatten() {
        let since = if text.is_empty() { "" } else { ", sent since" };
        text.push_str(&format!("The user's latest message to the bot{since}:\n{}\n\n", clipped(&latest)));
        language = Some("of the user's latest message".into());
    }
    Some(Request { text, language: language? })
}

/// The most of one message the review reads.
const REQUEST_CHARS: usize = 1500;

fn clipped(text: &str) -> String {
    let text = text.trim();
    match text.char_indices().nth(REQUEST_CHARS) {
        Some((end, _)) => format!("{}…", &text[..end]),
        None => text.to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn verdicts_parse_with_fences_prose_and_a_proposed_rule() {
        let verdict = parse_verdict("```json\n{\"verdict\": \"ask\", \"reason\": \"It runs build scripts.\", \"rule\": \" run the Rust tests\\n in ~/dev/lorca. \"}\n```").unwrap();
        assert!(!verdict.allow);
        assert_eq!(verdict.reason, "It runs build scripts.");
        assert_eq!(verdict.rule.as_deref(), Some("run the Rust tests in ~/dev/lorca"));
        assert_eq!(parse_verdict("Sure: {\"verdict\":\"ALLOW\",\"reason\":\"A draft.\"}").map(|verdict| verdict.allow), Some(true));
        let bare = parse_verdict("{\"verdict\":\"ask\",\"rule\":\"\"}").unwrap();
        assert_eq!((bare.reason.as_str(), bare.rule), ("This action needs a look first.", None));
        assert_eq!(parse_verdict("{\"verdict\":\"maybe\"}"), None);
        assert_eq!(parse_verdict("no json here"), None);
    }

    #[test]
    fn the_review_reads_the_request_that_started_the_turn() {
        use crate::model::{Author, Device, Message};

        let home = std::env::temp_dir().join(format!("lorca-review-request-{}", uuid::Uuid::new_v4()));
        let app = App::load(crate::config::Config { home: home.clone(), port: 0 }).unwrap();
        app.state.lock().unwrap().devices.push(Device {
            id: "runner".into(), name: "MacBook Air".into(), model: String::new(), os: "macos".into(), os_version: String::new(),
            box_pubkey: String::new(), plugins: Vec::new(), updated_at: 0,
        });
        let bot = |id: &str, name: &str| Bot {
            id: id.into(), name: name.into(), description: String::new(), symbol_name: String::new(), accent: String::new(), avatar: None,
            runner_id: "runner".into(), harness: crate::model::Harness::default(), codex_options: crate::model::CodexOptions::default(), provider: "deepseek".into(), model: None, thinking: None, legacy_instructions: String::new(), workdir: None, created_at: 0.0,
        };
        let (devops, dm) = app.create_bot_with_dm(bot("bot-devops", "DevOps"), None).unwrap();
        app.create_bot_with_dm(bot("bot-chef", "Chef"), None).unwrap();
        let chat_id = dm.meta.id.as_str();
        let say = |author: Author, body: Body| {
            let message = Message::new(chat_id, author, body);
            let id = message.id.clone();
            app.upsert_message(message, false);
            id
        };
        let at = |message_id: &str| Trigger { message_id: message_id.into(), routine: None };

        let stop = say(Author::You, Body::text("actually stop that"));
        say(Author::Bot { bot_id: devops.id.clone() }, Body::text("Stopped."));
        let handoff = say(
            Author::Bot { bot_id: "bot-chef".into() },
            Body::Handoff { from: "bot-chef".into(), to: devops.id.clone(), reason: "You own Railway monitoring from now on.".into() },
        );
        // Hours later a teammate starts the turn: the user's stop was about earlier work.
        let heard = request(&app, chat_id, &at(&handoff)).unwrap();
        assert_eq!(heard.text, "A message from the bot's teammate Chef started this turn:\nYou own Railway monitoring from now on.\n\n");
        assert_eq!(heard.language, "of Chef's message");
        assert_eq!(request(&app, chat_id, &at(&stop)).unwrap().text, "The user's latest message to the bot:\nactually stop that\n\n");

        // What the user writes while the turn runs is heard with it.
        say(Author::You, Body::text("leave Postgres alone"));
        let heard = request(&app, chat_id, &at(&handoff)).unwrap();
        assert!(heard.text.ends_with("\n\nThe user's latest message to the bot, sent since:\nleave Postgres alone\n\n"), "{}", heard.text);
        assert_eq!(heard.language, "of the user's latest message");

        let routine = app
            .insert_routine(Routine {
                id: "rt-watch".into(), bot_id: devops.id.clone(), name: "Railway memory watch".into(), prompt: "Check Railway memory.".into(),
                schedule: "every 2h".into(), is_enabled: true, enabled_at: 0.0, last_run_at: None, last_outcome: None, paused_reason: None, created_at: 0.0,
            })
            .unwrap();
        let marker = say(Author::System, Body::Notice { text: "Routine · Railway memory watch".into(), routine_id: Some(routine.id.clone()) });
        let run = Trigger { message_id: marker.clone(), routine: Some(routine.clone()) };
        let task = "This turn is a scheduled run of the bot's routine \"Railway memory watch\", with nobody watching. Its task:\nCheck Railway memory.\n\n";
        let heard = request(&app, chat_id, &run).unwrap();
        assert_eq!(heard.text, task);
        assert_eq!(heard.language, "the routine's task is written in");

        // The run keeps the task it started with, as its own context does, while the roster's copy
        // is edited or deleted. A later turn reads the roster, as its transcript does.
        app.update_routine(&routine.id, |routine| routine.prompt = "Redeploy the relay service.".into()).unwrap();
        assert_eq!(request(&app, chat_id, &run).unwrap().text, task);
        assert!(request(&app, chat_id, &at(&marker)).unwrap().text.ends_with("Its task:\nRedeploy the relay service.\n\n"));
        app.delete_routine(&routine.id).unwrap();
        assert_eq!(request(&app, chat_id, &run).unwrap().text, task);
        assert_eq!(request(&app, chat_id, &at("gone")), None);
        let _ = std::fs::remove_dir_all(home);
    }
}
