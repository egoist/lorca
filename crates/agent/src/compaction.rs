//! Keeping a long conversation inside the model's window, after pi's compaction: the older
//! part of the transcript becomes a structured summary the model writes, the recent part stays
//! as it is, and the two go back as the context from then on.

use std::collections::BTreeSet;

use futures::StreamExt;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use tokio_util::sync::CancellationToken;

use crate::estimate::{estimate_context_tokens, estimate_message_tokens, estimate_text_tokens};
use crate::provider::{AssistantAccumulator, ModelRequest, Provider, ToolSpec};
use crate::request::RequestOptions;
use crate::types::{AgentMessage, AssistantPart, LlmMessage, StopReason, Usage, UserMessage};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct CompactionSettings {
    pub enabled: bool,
    /// Tokens kept free for the summary prompt and the reply: compaction starts when the
    /// context passes `window - reserve_tokens`.
    pub reserve_tokens: u64,
    /// About this many tokens of recent messages stay as they are.
    pub keep_recent_tokens: u64,
    /// The most history one summarization request carries; a longer history is summarized in
    /// pieces, each updating the summary of the ones before. 0 means no limit.
    #[serde(default)]
    pub max_input_tokens: u64,
}

impl Default for CompactionSettings {
    fn default() -> Self {
        CompactionSettings { enabled: true, reserve_tokens: 16_384, keep_recent_tokens: 20_000, max_input_tokens: 0 }
    }
}

/// Splits `messages` into runs of at most `max_tokens` (estimated), never between a call and
/// its result. A single message over the limit is a run of its own.
pub fn chunk_by_tokens(messages: &[AgentMessage], max_tokens: u64) -> Vec<&[AgentMessage]> {
    if max_tokens == 0 || messages.is_empty() {
        return vec![messages];
    }
    let mut chunks = Vec::new();
    let mut start = 0;
    let mut accumulated = 0u64;
    for (i, message) in messages.iter().enumerate() {
        let size = estimate_message_tokens(message);
        if i > start && accumulated + size > max_tokens && is_cut_point(message) {
            chunks.push(&messages[start..i]);
            start = i;
            accumulated = 0;
        }
        accumulated += size;
    }
    chunks.push(&messages[start..]);
    chunks
}

/// Whether a context of `context_tokens` in a `context_window` should be compacted.
pub fn should_compact(context_tokens: u64, context_window: u64, settings: &CompactionSettings) -> bool {
    settings.enabled && context_window > 0 && context_tokens > context_window.saturating_sub(settings.reserve_tokens)
}

/// Where the kept part of a transcript starts.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CutPoint {
    /// The first message kept as it is.
    pub first_kept: usize,
    /// When the cut falls inside a turn: the user message that started it. The turn's prefix
    /// gets its own short summary so the kept suffix makes sense.
    pub turn_start: Option<usize>,
}

/// A cut can fall before a user or assistant message, never before a tool result (which
/// belongs to its call).
fn is_cut_point(message: &AgentMessage) -> bool {
    !matches!(message, AgentMessage::ToolResult(_))
}

/// The cut that keeps about `keep_recent_tokens` of recent messages.
pub fn find_cut_point(messages: &[AgentMessage], keep_recent_tokens: u64) -> CutPoint {
    let cut_points: Vec<usize> = (0..messages.len()).filter(|&i| is_cut_point(&messages[i])).collect();
    if cut_points.is_empty() {
        return CutPoint { first_kept: 0, turn_start: None };
    }
    let mut cut_index = cut_points[0];
    let mut accumulated = 0u64;
    for i in (0..messages.len()).rev() {
        accumulated += estimate_message_tokens(&messages[i]);
        if accumulated >= keep_recent_tokens {
            if let Some(&point) = cut_points.iter().find(|&&c| c >= i) {
                cut_index = point;
            }
            break;
        }
    }
    let is_user = matches!(messages[cut_index], AgentMessage::User(_));
    let turn_start = if is_user {
        None
    } else {
        (0..cut_index).rev().find(|&i| matches!(messages[i], AgentMessage::User(_)))
    };
    CutPoint { first_kept: cut_index, turn_start }
}

pub const SUMMARIZATION_SYSTEM_PROMPT: &str = "You are a context summarization assistant. Your task is to read a conversation between a user and an AI assistant, then produce a structured summary following the exact format specified.\n\nDo NOT continue the conversation. Do NOT respond to any questions in the conversation. ONLY output the structured summary.";

const SUMMARIZATION_PROMPT: &str = "The messages above are a conversation to summarize. Create a structured context checkpoint summary that another LLM will use to continue the work.

Use this EXACT format:

## Goal
[What is the user trying to accomplish? Can be multiple items if the session covers different tasks.]

## Constraints & Preferences
- [Any constraints, preferences, or requirements mentioned by user]
- [Or \"(none)\" if none were mentioned]

## Progress
### Done
- [x] [Completed tasks/changes]

### In Progress
- [ ] [Current work]

### Blocked
- [Issues preventing progress, if any]

## Key Decisions
- **[Decision]**: [Brief rationale]

## Next Steps
1. [Ordered list of what should happen next]

## Critical Context
- [Any data, examples, or references needed to continue]
- [Or \"(none)\" if not applicable]

Keep each section concise. Preserve exact file paths, function names, and error messages.";

const UPDATE_SUMMARIZATION_PROMPT: &str = "The messages above are NEW conversation messages to incorporate into the existing summary provided in <previous-summary> tags.

Update the existing structured summary with new information. RULES:
- PRESERVE all existing information from the previous summary
- ADD new progress, decisions, and context from the new messages
- UPDATE the Progress section: move items from \"In Progress\" to \"Done\" when completed
- UPDATE \"Next Steps\" based on what was accomplished
- PRESERVE exact file paths, function names, and error messages
- If something is no longer relevant, you may remove it

Use this EXACT format:

## Goal
[Preserve existing goals, add new ones if the task expanded]

## Constraints & Preferences
- [Preserve existing, add new ones discovered]

## Progress
### Done
- [x] [Include previously done items AND newly completed items]

### In Progress
- [ ] [Current work - update based on progress]

### Blocked
- [Current blockers - remove if resolved]

## Key Decisions
- **[Decision]**: [Brief rationale] (preserve all previous, add new)

## Next Steps
1. [Update based on current state]

## Critical Context
- [Preserve important context, add new if needed]

Keep each section concise. Preserve exact file paths, function names, and error messages.";

const TURN_PREFIX_SUMMARIZATION_PROMPT: &str = "This is the PREFIX of a turn that was too large to keep. The SUFFIX (recent work) is retained.

Summarize the prefix to provide context for the retained suffix:

## Original Request
[What did the user ask for in this turn?]

## Early Progress
- [Key decisions and work done in the prefix]

## Context for Suffix
- [Information needed to understand the retained recent work]

Be concise. Focus on what's needed to understand the kept suffix.";

/// The transcript as text for the summarizer.
pub fn serialize_conversation(messages: &[AgentMessage]) -> String {
    let mut out = String::new();
    for message in messages {
        match message {
            AgentMessage::User(user) => {
                let text: Vec<&str> = user.content.iter().filter_map(|p| p.as_text()).collect();
                out.push_str("[User]\n");
                out.push_str(&text.join("\n"));
                if user.content.iter().any(|p| p.as_text().is_none()) {
                    out.push_str("\n(with an image)");
                }
                out.push_str("\n\n");
            }
            AgentMessage::Assistant(assistant) => {
                out.push_str("[Assistant]\n");
                for part in &assistant.content {
                    match part {
                        AssistantPart::Text { text } => {
                            out.push_str(text);
                            out.push('\n');
                        }
                        AssistantPart::ToolCall(call) => {
                            out.push_str(&format!("[Tool call: {}({})]\n", call.name, call.arguments));
                        }
                        AssistantPart::Thinking { .. } | AssistantPart::ServerBlock { .. } => {}
                    }
                }
                out.push('\n');
            }
            AgentMessage::ToolResult(result) => {
                let text = result.text();
                let shown: String = if text.chars().count() > 4000 { text.chars().take(4000).collect::<String>() + "…" } else { text };
                out.push_str(&format!("[Tool result: {}{}]\n{shown}\n\n", result.tool_name, if result.is_error { ", error" } else { "" }));
            }
            AgentMessage::Custom { kind, data, .. } => {
                out.push_str(&format!("[{kind}]\n{data}\n\n"));
            }
        }
    }
    out
}

/// Files the built-in coding tools touched, from the tool calls in `messages`.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct FileOperations {
    pub read: BTreeSet<String>,
    pub modified: BTreeSet<String>,
}

pub fn extract_file_operations(messages: &[AgentMessage], into: &mut FileOperations) {
    for message in messages {
        let AgentMessage::Assistant(assistant) = message else { continue };
        for call in assistant.tool_calls() {
            let Some(path) = call.arguments["path"].as_str().filter(|p| !p.is_empty()) else { continue };
            match call.name.as_str() {
                "read" => {
                    into.read.insert(path.to_string());
                }
                "write" | "edit" => {
                    into.modified.insert(path.to_string());
                }
                _ => {}
            }
        }
    }
}

fn format_file_operations(ops: &FileOperations) -> String {
    let mut out = String::new();
    let read: Vec<&String> = ops.read.iter().filter(|p| !ops.modified.contains(*p)).collect();
    if !read.is_empty() {
        out.push_str("\n\n## Files Read\n");
        for path in read {
            out.push_str(&format!("- {path}\n"));
        }
    }
    if !ops.modified.is_empty() {
        out.push_str("\n\n## Files Modified\n");
        for path in &ops.modified {
            out.push_str(&format!("- {path}\n"));
        }
    }
    out
}

#[derive(Debug, Clone, PartialEq)]
pub struct Summary {
    pub text: String,
    pub usage: Usage,
}

/// One summarization request: the conversation, the previous summary to update if there is
/// one, and the prompt. `max_tokens` caps the reply.
async fn summarize(
    provider: &dyn Provider,
    conversation: &str,
    previous_summary: Option<&str>,
    prompt: &str,
    max_tokens: u64,
    options: &RequestOptions,
    cancel: &CancellationToken,
) -> Result<Summary, String> {
    let mut text = format!("<conversation>\n{conversation}\n</conversation>\n\n");
    if let Some(previous) = previous_summary {
        text.push_str(&format!("<previous-summary>\n{previous}\n</previous-summary>\n\n"));
    }
    text.push_str(prompt);
    let request = ModelRequest {
        system_prompt: SUMMARIZATION_SYSTEM_PROMPT.into(),
        messages: vec![LlmMessage::User(UserMessage::text(text))],
        tools: Vec::new(),
        cache_points: Vec::new(),
        max_tokens: Some(max_tokens),
        options: options.clone(),
    };
    let mut stream = provider.stream(request, cancel.clone()).await;
    let mut acc = AssistantAccumulator::new(provider.provider_id(), provider.model_id());
    while let Some(event) = stream.next().await {
        acc.apply(&event);
    }
    let message = acc.finish(cancel.is_cancelled());
    match message.stop_reason {
        StopReason::Aborted => Err(message.error_message.unwrap_or_else(|| "Summarization aborted".into())),
        StopReason::Error => Err(format!("Summarization failed: {}", message.error_message.unwrap_or_else(|| "Unknown error".into()))),
        _ => Ok(Summary { text: message.text().trim().to_string(), usage: message.usage }),
    }
}

/// A summary of `messages`, updating `previous_summary` when there is one. `reserve_tokens`
/// bounds the reply.
pub async fn generate_summary(
    provider: &dyn Provider,
    messages: &[AgentMessage],
    previous_summary: Option<&str>,
    reserve_tokens: u64,
    custom_instructions: Option<&str>,
    options: &RequestOptions,
    cancel: &CancellationToken,
) -> Result<Summary, String> {
    let mut prompt = (if previous_summary.is_some() { UPDATE_SUMMARIZATION_PROMPT } else { SUMMARIZATION_PROMPT }).to_string();
    if let Some(custom) = custom_instructions.map(str::trim).filter(|s| !s.is_empty()) {
        prompt.push_str(&format!("\n\nAdditional focus: {custom}"));
    }
    let max_tokens = (reserve_tokens as f64 * 0.8) as u64;
    summarize(provider, &serialize_conversation(messages), previous_summary, &prompt, max_tokens.max(1024), options, cancel).await
}

#[derive(Debug, Clone, PartialEq)]
pub struct CompactResult {
    /// The summary that replaces everything before `first_kept`.
    pub summary: String,
    /// The context size before, as well as it was known.
    pub tokens_before: u64,
    /// What the summarization requests cost.
    pub usage: Usage,
    /// The first message of `messages` kept as it is.
    pub first_kept: usize,
    /// Whether the cut fell inside a turn, whose prefix is summarized in `summary` too.
    pub split_turn: bool,
}

/// Summarizes the older part of `messages`, keeping about `keep_recent_tokens` of the recent
/// part. `previous_summary` is the summary a prior compaction left, updated rather than
/// rewritten. `Ok(None)` when there is nothing to compact.
pub async fn compact(
    provider: &dyn Provider,
    messages: &[AgentMessage],
    previous_summary: Option<&str>,
    settings: &CompactionSettings,
    custom_instructions: Option<&str>,
    options: &RequestOptions,
    cancel: &CancellationToken,
) -> Result<Option<CompactResult>, String> {
    if messages.is_empty() {
        return Ok(None);
    }
    let tokens_before = estimate_context_tokens(messages).tokens;
    let cut = find_cut_point(messages, settings.keep_recent_tokens);
    let history_end = cut.turn_start.unwrap_or(cut.first_kept);
    if history_end == 0 && cut.turn_start.is_none() {
        return Ok(None);
    }
    let history = &messages[..history_end];
    let mut usage = Usage::default();
    let mut file_ops = FileOperations::default();
    extract_file_operations(history, &mut file_ops);

    let mut summary = if history.is_empty() {
        previous_summary.map(str::to_string).unwrap_or_else(|| "No prior history.".into())
    } else {
        // A history longer than one request may carry is summarized piece by piece, each piece
        // updating the summary of the ones before it.
        let mut running: Option<String> = previous_summary.map(str::to_string);
        for chunk in chunk_by_tokens(history, settings.max_input_tokens) {
            let result = generate_summary(provider, chunk, running.as_deref(), settings.reserve_tokens, custom_instructions, options, cancel).await?;
            usage.add(&result.usage);
            running = Some(result.text);
        }
        running.unwrap_or_default()
    };

    let split_turn = cut.turn_start.is_some();
    if let Some(turn_start) = cut.turn_start {
        let prefix = &messages[turn_start..cut.first_kept];
        extract_file_operations(prefix, &mut file_ops);
        if !prefix.is_empty() {
            let max_tokens = ((settings.reserve_tokens as f64 * 0.5) as u64).max(1024);
            let result = summarize(provider, &serialize_conversation(prefix), None, TURN_PREFIX_SUMMARIZATION_PROMPT, max_tokens, options, cancel).await?;
            usage.add(&result.usage);
            summary = format!("{summary}\n\n---\n\n**Turn Context (split turn):**\n\n{}", result.text);
        }
    }
    summary.push_str(&format_file_operations(&file_ops));

    Ok(Some(CompactResult { summary, tokens_before, usage, first_kept: cut.first_kept, split_turn }))
}

/// What a conversation sends beside its messages on every model call. A request that sends the
/// same ahead of the same messages shares the conversation's prompt cache.
#[derive(Debug, Clone)]
pub struct RequestShape {
    pub system_prompt: String,
    pub tools: Vec<ToolSpec>,
    /// Prefix lengths of the messages, counted as `AgentMessage`s, that the conversation's
    /// requests mark for the provider's cache.
    pub cache_points: Vec<usize>,
}

/// The least room for the summary that makes asking inside the conversation worth a try.
const MIN_IN_PLACE_REPLY_TOKENS: u64 = 4096;
/// Kept free on top of the estimate, which counts characters for what the provider has not
/// counted yet.
const IN_PLACE_MARGIN_TOKENS: u64 = 1024;

/// `compact`, asked inside the conversation: the summary request is the conversation's next
/// request, with the same system prompt, tools, and messages, and the summary prompt as one more
/// user message. The provider's prompt cache serves everything the conversation's last request
/// sent, where `compact` sends the whole history again as new text. `messages` is the context as
/// the conversation's loop holds it, an earlier summary first when there is one; `convert` turns
/// it into what the model reads, as that loop's `convert_to_llm` does. The summary covers
/// everything before the cut, the start of a turn the cut falls inside included, and
/// `first_kept` counts from after an earlier summary, as `compact`'s does.
///
/// Falls back to `compact` when the window is unknown, when the context leaves too little room
/// for the reply, and when the model answers with anything but a summary: a tool call, a reply
/// the token limit cut, or an error.
#[allow(clippy::too_many_arguments)]
pub async fn compact_in_place(
    provider: &dyn Provider,
    shape: &RequestShape,
    convert: fn(&[AgentMessage]) -> Vec<LlmMessage>,
    messages: &[AgentMessage],
    settings: &CompactionSettings,
    custom_instructions: Option<&str>,
    options: &RequestOptions,
    cancel: &CancellationToken,
) -> Result<Option<CompactResult>, String> {
    let (previous_summary, skip) = earlier_summary(messages);
    let body = &messages[skip..];
    let cut = find_cut_point(body, settings.keep_recent_tokens);
    if cut.first_kept == 0 {
        return Ok(None);
    }
    let mut usage = Usage::default();
    match summarize_in_place(provider, shape, convert, messages, skip, &cut, previous_summary.is_some(), settings, custom_instructions, options, cancel).await? {
        InPlace::Summary(result) => return Ok(Some(result)),
        InPlace::Declined { why, spent } => {
            tracing::info!(why, "summarizing inside the conversation fell back to a summary of the transcript");
            usage.add(&spent);
        }
    }
    let result = compact(provider, body, previous_summary.as_deref(), settings, custom_instructions, options, cancel).await?;
    Ok(result.map(|mut result| {
        result.usage.add(&usage);
        result
    }))
}

/// The summary an earlier compaction left at the start of `messages`, and how many messages it
/// takes there.
pub fn earlier_summary(messages: &[AgentMessage]) -> (Option<String>, usize) {
    match messages.first() {
        Some(AgentMessage::Custom { kind, data, .. }) if kind == "compaction" => (data["summary"].as_str().map(str::to_string), 1),
        _ => (None, 0),
    }
}

enum InPlace {
    Summary(CompactResult),
    /// No summary came of it, for `why`; `spent` is what an answered request cost.
    Declined { why: &'static str, spent: Usage },
}

#[allow(clippy::too_many_arguments)]
async fn summarize_in_place(
    provider: &dyn Provider,
    shape: &RequestShape,
    convert: fn(&[AgentMessage]) -> Vec<LlmMessage>,
    messages: &[AgentMessage],
    skip: usize,
    cut: &CutPoint,
    has_previous: bool,
    settings: &CompactionSettings,
    custom_instructions: Option<&str>,
    options: &RequestOptions,
    cancel: &CancellationToken,
) -> Result<InPlace, String> {
    let declined = |why| Ok(InPlace::Declined { why, spent: Usage::default() });
    let window = provider.model_info().map(|info| info.context_window).unwrap_or(0);
    if window == 0 {
        return declined("the model's window is unknown");
    }
    let body = &messages[skip..];
    let prompt = in_place_prompt(&describe_cut(&body[cut.first_kept]), has_previous, custom_instructions);
    // A provider count covers the system prompt and the tools; a transcript rebuilt without one
    // is counted by its characters, and they are added.
    let estimate = estimate_context_tokens(messages);
    let mut size = estimate.tokens + estimate_text_tokens(&prompt) + IN_PLACE_MARGIN_TOKENS;
    if estimate.last_usage_index.is_none() {
        size += estimate_text_tokens(&shape.system_prompt) + estimate_text_tokens(&serde_json::to_string(&shape.tools).unwrap_or_default());
    }
    let max_tokens = window.saturating_sub(size).min(((settings.reserve_tokens as f64 * 0.8) as u64).max(1024));
    if max_tokens < MIN_IN_PLACE_REPLY_TOKENS {
        return declined("the context leaves too little room for the summary");
    }

    let mut llm_messages = convert(messages);
    llm_messages.push(LlmMessage::User(UserMessage::text(prompt)));
    let cache_points = shape.cache_points.iter().filter(|point| **point <= messages.len()).map(|point| convert(&messages[..*point]).len()).collect();
    let request = ModelRequest {
        system_prompt: shape.system_prompt.clone(),
        messages: llm_messages,
        tools: shape.tools.clone(),
        cache_points,
        max_tokens: Some(max_tokens),
        options: options.clone(),
    };
    let mut stream = provider.stream(request, cancel.clone()).await;
    let mut acc = AssistantAccumulator::new(provider.provider_id(), provider.model_id());
    while let Some(event) = stream.next().await {
        acc.apply(&event);
    }
    let message = acc.finish(cancel.is_cancelled());
    let spent = message.usage.clone();
    let why = match message.stop_reason {
        StopReason::Aborted => return Err(message.error_message.unwrap_or_else(|| "Summarization aborted".into())),
        StopReason::Error => "the request failed",
        StopReason::Length => "the summary was cut off",
        _ if !message.tool_calls().is_empty() => "the model called a tool",
        _ if !message.text().contains("## Goal") => "the answer was not a summary",
        _ => "",
    };
    if !why.is_empty() {
        return Ok(InPlace::Declined { why, spent });
    }

    let mut summary = message.text().trim().to_string();
    let mut file_ops = FileOperations::default();
    extract_file_operations(&body[..cut.first_kept], &mut file_ops);
    summary.push_str(&format_file_operations(&file_ops));
    Ok(InPlace::Summary(CompactResult {
        summary,
        tokens_before: estimate_context_tokens(body).tokens,
        usage: spent,
        first_kept: cut.first_kept,
        split_turn: cut.turn_start.is_some(),
    }))
}

/// The message a cut falls before, in words the model can find in its own context.
pub fn describe_cut(message: &AgentMessage) -> String {
    let opening = |text: &str| -> Option<String> {
        let line: String = text.split_whitespace().collect::<Vec<_>>().join(" ");
        if line.is_empty() {
            return None;
        }
        Some(if line.chars().count() > 80 { format!("{}…", line.chars().take(80).collect::<String>()) } else { line })
    };
    match message {
        AgentMessage::User(user) => {
            let text: Vec<&str> = user.content.iter().filter_map(|part| part.as_text()).collect();
            match opening(&text.join(" ")) {
                Some(opening) => format!("the message that begins \"{opening}\""),
                None => "the message with an image".into(),
            }
        }
        AgentMessage::Assistant(assistant) => match (opening(&assistant.text()), assistant.tool_calls().first()) {
            (Some(opening), _) => format!("your message that begins \"{opening}\""),
            (None, Some(call)) => {
                let arguments = call.arguments.to_string();
                let arguments = if arguments.chars().count() > 80 { format!("{}…", arguments.chars().take(80).collect::<String>()) } else { arguments };
                format!("your {} call with {arguments}", call.name)
            }
            (None, None) => "your next message".into(),
        },
        AgentMessage::ToolResult(result) => format!("the result of your {} call", result.tool_name),
        AgentMessage::Custom { kind, .. } => format!("the {kind} entry"),
    }
}

/// The summary prompt as the conversation's next user message: the checkpoint `compact` asks
/// for, of everything before the cut.
fn in_place_prompt(cut: &str, has_previous: bool, custom_instructions: Option<&str>) -> String {
    let mut prompt = format!(
        "[Context checkpoint] Stop the work here and call no tools: your context is nearly full. Everything before {cut} is \
         about to leave it, and the summary you write now takes its place. That message and the ones after it stay as they are, \
         after your summary, so leave them out."
    );
    if has_previous {
        prompt.push_str(" The conversation opens with the summary of an earlier part: yours replaces it too, so carry forward what still holds.");
    }
    prompt.push_str(" Answer with the summary alone.\n\n");
    prompt.push_str(SUMMARIZATION_PROMPT);
    if let Some(custom) = custom_instructions.map(str::trim).filter(|s| !s.is_empty()) {
        prompt.push_str(&format!("\n\nAdditional focus: {custom}"));
    }
    prompt
}

/// The message that carries a summary into the context, ahead of the kept messages.
pub fn summary_message(summary: &str, tokens_before: u64) -> AgentMessage {
    AgentMessage::Custom {
        kind: "compaction".into(),
        data: serde_json::json!({ "summary": summary, "tokens_before": tokens_before }),
        timestamp: crate::now_ms(),
    }
}

/// How the model sees a compaction summary: a user message that says what it is.
pub fn summary_as_llm(data: &Value, timestamp: u64) -> LlmMessage {
    let summary = data["summary"].as_str().unwrap_or("");
    LlmMessage::User(UserMessage {
        content: vec![crate::types::ContentPart::text(format!(
            "The conversation before this point was compacted into the summary below. Continue from it as if you had lived through it.\n\n{summary}"
        ))],
        timestamp,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::{AssistantMessage, ContentPart, ToolCall, ToolResultMessage};
    use serde_json::json;

    fn user(text: &str) -> AgentMessage {
        AgentMessage::user(text)
    }
    fn assistant(text: &str) -> AgentMessage {
        let mut m = AssistantMessage::empty("p", "m");
        m.content = vec![AssistantPart::Text { text: text.into() }];
        AgentMessage::Assistant(m)
    }
    fn call(name: &str, path: &str) -> AgentMessage {
        let mut m = AssistantMessage::empty("p", "m");
        m.content = vec![AssistantPart::ToolCall(ToolCall { id: "c".into(), name: name.into(), arguments: json!({ "path": path }) })];
        AgentMessage::Assistant(m)
    }
    fn result(text: &str) -> AgentMessage {
        AgentMessage::ToolResult(ToolResultMessage { tool_call_id: "c".into(), tool_name: "read".into(), content: vec![ContentPart::text(text)], details: Value::Null, is_error: false, timestamp: 0 })
    }

    #[test]
    fn the_cut_keeps_recent_turns_and_never_splits_a_call_from_its_result() {
        let messages = vec![user("a"), assistant(&"x".repeat(400)), user("b"), call("read", "f.rs"), result(&"y".repeat(400)), assistant("done")];
        // Keep ~100 tokens: the last assistant (1 token) and the result (100) reach it at the
        // result, which is not a cut point, so the cut moves to the next one after it; the
        // call and its result are summarized together.
        let cut = find_cut_point(&messages, 100);
        assert_eq!(cut, CutPoint { first_kept: 5, turn_start: Some(2) });
        // Keep a lot: everything stays from the first message.
        assert_eq!(find_cut_point(&messages, 10_000), CutPoint { first_kept: 0, turn_start: None });
        // Keep little: the cut is at the last user message, a clean turn boundary.
        let cut = find_cut_point(&[user("a"), assistant("b"), user("c"), assistant("d")], 1);
        assert_eq!(cut, CutPoint { first_kept: 3, turn_start: Some(2) });
    }

    #[test]
    fn thresholds_and_file_operations() {
        let settings = CompactionSettings::default();
        assert!(!should_compact(100_000, 128_000, &settings));
        assert!(should_compact(120_000, 128_000, &settings));
        assert!(!should_compact(120_000, 0, &settings));
        let mut ops = FileOperations::default();
        extract_file_operations(&[call("read", "a.rs"), call("edit", "a.rs"), call("write", "b.rs")], &mut ops);
        let text = format_file_operations(&ops);
        assert_eq!(text, "\n\n## Files Modified\n- a.rs\n- b.rs\n");
    }

    /// Answers every request with the same text and records the prompts it saw.
    struct Summarizer(std::sync::Mutex<Vec<String>>);

    #[async_trait::async_trait]
    impl Provider for Summarizer {
        fn provider_id(&self) -> &str {
            "p"
        }
        fn model_id(&self) -> &str {
            "m"
        }
        async fn stream(&self, request: ModelRequest, _cancel: CancellationToken) -> crate::provider::AssistantEventStream {
            let LlmMessage::User(prompt) = &request.messages[0] else { panic!() };
            self.0.lock().unwrap().push(prompt.content[0].as_text().unwrap().to_string());
            let (tx, rx) = tokio::sync::mpsc::channel(8);
            tokio::spawn(async move {
                let _ = tx.send(crate::provider::AssistantEvent::Start).await;
                let _ = tx.send(crate::provider::AssistantEvent::TextStart { index: 0 }).await;
                let _ = tx.send(crate::provider::AssistantEvent::TextDelta { index: 0, delta: "## Goal\nShip it".into() }).await;
                let _ = tx.send(crate::provider::AssistantEvent::TextEnd { index: 0 }).await;
                let _ = tx.send(crate::provider::AssistantEvent::Done { stop_reason: StopReason::Stop, usage: Usage { output: 7, ..Usage::default() } }).await;
            });
            crate::provider::channel_stream(rx)
        }
    }

    #[tokio::test]
    async fn compact_summarizes_the_older_part_and_keeps_the_recent_one() {
        let messages = vec![user("first"), assistant(&"a".repeat(400)), user("second"), call("edit", "x.rs"), result("ok"), assistant(&"b".repeat(400)), user("third"), assistant("c")];
        let summarizer = Summarizer(Default::default());
        let settings = CompactionSettings { enabled: true, reserve_tokens: 4096, keep_recent_tokens: 50, max_input_tokens: 0 };
        let result = compact(&summarizer, &messages, None, &settings, None, &RequestOptions::default(), &CancellationToken::new()).await.unwrap().unwrap();
        // The recent budget is reached inside the second turn: its long reply stays, the turn's
        // start and tool work before it become a prefix summary.
        assert_eq!(result.first_kept, 5);
        assert!(result.split_turn);
        assert!(result.summary.starts_with("## Goal\nShip it"), "{}", result.summary);
        assert!(result.summary.contains("**Turn Context (split turn):**"), "{}", result.summary);
        assert!(result.summary.contains("## Files Modified\n- x.rs"), "{}", result.summary);
        assert_eq!(result.usage.output, 14, "two summarization requests");
        let prompts = summarizer.0.lock().unwrap();
        assert_eq!(prompts.len(), 2);
        assert!(prompts[0].contains("[User]\nfirst") && !prompts[0].contains("second"));
        assert!(prompts[0].ends_with(SUMMARIZATION_PROMPT));
        assert!(prompts[1].contains("[User]\nsecond") && prompts[1].contains("[Tool call: edit") && !prompts[1].contains("third"));
        assert!(prompts[1].ends_with(TURN_PREFIX_SUMMARIZATION_PROMPT));

        // An update carries the previous summary and asks for the update.
        let summarizer = Summarizer(Default::default());
        compact(&summarizer, &messages, Some("## Goal\nOld"), &settings, None, &RequestOptions::default(), &CancellationToken::new()).await.unwrap().unwrap();
        let prompts = summarizer.0.lock().unwrap();
        assert!(prompts[0].contains("<previous-summary>\n## Goal\nOld\n</previous-summary>"));
        assert!(prompts[0].ends_with(UPDATE_SUMMARIZATION_PROMPT));

        // Nothing older than the recent part: nothing to do.
        assert_eq!(compact(&summarizer, &messages[6..], None, &settings, None, &RequestOptions::default(), &CancellationToken::new()).await.unwrap(), None);
    }

    /// `DEEPSEEK_API_KEY=… cargo test -p lorca-agent live_deepseek_summary -- --ignored --nocapture`
    #[tokio::test]
    #[ignore]
    async fn live_deepseek_summary() {
        let Ok(key) = std::env::var("DEEPSEEK_API_KEY") else { return };
        let provider = crate::providers::AnthropicProvider::deepseek(&key, None);
        let messages = vec![
            user("Rename the config module to settings and update the imports."),
            call("edit", "src/lib.rs"),
            result("Edited src/lib.rs"),
            assistant("Renamed the module in lib.rs. The imports in main.rs still point at config."),
            user("Fix main.rs too, and keep the old name as an alias for a release."),
        ];
        let summary = generate_summary(&provider, &messages, None, 4096, None, &RequestOptions::default(), &CancellationToken::new()).await.unwrap();
        eprintln!("{}\n\nusage {:?}", summary.text, summary.usage);
        assert!(summary.text.contains("## Goal"));
        assert!(summary.text.contains("main.rs"));
    }

    #[tokio::test]
    async fn a_long_history_is_summarized_in_pieces() {
        // ~100 tokens per assistant message; a 250-token limit cuts the history into pieces
        // that never separate a call from its result.
        let messages = vec![
            user("one"),
            assistant(&"a".repeat(400)),
            user("two"),
            call("read", "x.rs"),
            result(&"r".repeat(400)),
            assistant(&"b".repeat(400)),
            user("three"),
            assistant(&"c".repeat(400)),
            user("four"),
            assistant("done"),
        ];
        let chunks = chunk_by_tokens(&messages[..8], 250);
        let sizes: Vec<usize> = chunks.iter().map(|c| c.len()).collect();
        assert_eq!(sizes, vec![5, 3], "{sizes:?}");
        assert!(matches!(&chunks[1][0], AgentMessage::Assistant(m) if m.text().starts_with('b')), "a piece ends after a result, never between a call and its result");
        assert_eq!(chunk_by_tokens(&messages, 0).len(), 1);

        let summarizer = Summarizer(Default::default());
        let settings = CompactionSettings { enabled: true, reserve_tokens: 4096, keep_recent_tokens: 10, max_input_tokens: 250 };
        let result = compact(&summarizer, &messages, None, &settings, None, &RequestOptions::default(), &CancellationToken::new()).await.unwrap().unwrap();
        // The recent budget is met inside the fourth turn: its reply stays, its start becomes a
        // prefix summary, and the six messages before it are the history.
        assert_eq!(result.first_kept, 7);
        assert!(result.split_turn);
        let prompts = summarizer.0.lock().unwrap();
        // Two pieces of history, then the split turn's prefix.
        assert_eq!(prompts.len(), 3, "{prompts:#?}");
        assert!(prompts[0].ends_with(SUMMARIZATION_PROMPT) && prompts[0].contains("[User]\ntwo") && !prompts[0].contains("bbb"));
        assert!(prompts[1].contains("<previous-summary>\n## Goal\nShip it\n</previous-summary>") && prompts[1].ends_with(UPDATE_SUMMARIZATION_PROMPT));
        assert!(prompts[1].contains("bbb") && !prompts[1].contains("three"));
        assert!(prompts[2].contains("[User]\nthree") && prompts[2].ends_with(TURN_PREFIX_SUMMARIZATION_PROMPT));
    }

    /// Answers each request with the next scripted reply, a summary or a tool call, and keeps
    /// the requests. Its window is Haiku 4.5's.
    struct Conversation {
        replies: std::sync::Mutex<Vec<&'static str>>,
        requests: std::sync::Mutex<Vec<ModelRequest>>,
    }

    impl Conversation {
        fn answering(replies: Vec<&'static str>) -> Self {
            Conversation { replies: std::sync::Mutex::new(replies), requests: Default::default() }
        }
    }

    #[async_trait::async_trait]
    impl Provider for Conversation {
        fn provider_id(&self) -> &str {
            "p"
        }
        fn model_id(&self) -> &str {
            "m"
        }
        fn model_info(&self) -> Option<&'static crate::models::ModelInfo> {
            crate::models::find("anthropic", "claude-haiku-4-5")
        }
        async fn stream(&self, request: ModelRequest, _cancel: CancellationToken) -> crate::provider::AssistantEventStream {
            self.requests.lock().unwrap().push(request);
            let reply = self.replies.lock().unwrap().remove(0);
            let (tx, rx) = tokio::sync::mpsc::channel(8);
            tokio::spawn(async move {
                use crate::provider::AssistantEvent as E;
                let _ = tx.send(E::Start).await;
                let stop_reason = if let Some(tool) = reply.strip_prefix("call:") {
                    let _ = tx.send(E::ToolCallStart { index: 0, id: "t".into(), name: tool.into() }).await;
                    let _ = tx.send(E::ToolCallDelta { index: 0, delta: "{}".into() }).await;
                    let _ = tx.send(E::ToolCallEnd { index: 0 }).await;
                    StopReason::ToolUse
                } else {
                    let _ = tx.send(E::TextStart { index: 0 }).await;
                    let _ = tx.send(E::TextDelta { index: 0, delta: reply.into() }).await;
                    let _ = tx.send(E::TextEnd { index: 0 }).await;
                    StopReason::Stop
                };
                let _ = tx.send(E::Done { stop_reason, usage: Usage { output: 7, ..Usage::default() } }).await;
            });
            crate::provider::channel_stream(rx)
        }
    }

    fn as_llm(messages: &[AgentMessage]) -> Vec<LlmMessage> {
        messages
            .iter()
            .filter_map(|message| match message {
                AgentMessage::Custom { kind, data, timestamp } if kind == "compaction" => Some(summary_as_llm(data, *timestamp)),
                other => other.as_llm(),
            })
            .collect()
    }

    fn shape() -> RequestShape {
        RequestShape {
            system_prompt: "You are Chef.".into(),
            tools: vec![ToolSpec { name: "read".into(), description: "Read a file".into(), parameters: json!({ "type": "object" }) }],
            cache_points: vec![3],
        }
    }

    #[tokio::test]
    async fn in_place_the_summary_request_is_the_conversations_next_request() {
        let messages = vec![
            summary_message("## Goal\nOld", 900),
            user("first"),
            assistant(&"a".repeat(400)),
            user("second"),
            call("edit", "x.rs"),
            result("ok"),
            assistant(&"b".repeat(400)),
            user("third"),
            assistant("c"),
        ];
        let provider = Conversation::answering(vec!["## Goal\nShip it"]);
        let settings = CompactionSettings { enabled: true, reserve_tokens: 16_384, keep_recent_tokens: 50, max_input_tokens: 0 };
        let result = compact_in_place(&provider, &shape(), as_llm, &messages, &settings, None, &RequestOptions::default(), &CancellationToken::new()).await.unwrap().unwrap();
        // The cut falls inside the second turn, as `compact`'s does, counted after the old summary.
        assert_eq!(result.first_kept, 5);
        assert!(result.split_turn);
        assert!(result.summary.starts_with("## Goal\nShip it") && !result.summary.contains("Turn Context"), "{}", result.summary);
        assert!(result.summary.contains("## Files Modified\n- x.rs"), "{}", result.summary);
        assert_eq!(result.usage.output, 7);

        let requests = provider.requests.lock().unwrap();
        assert_eq!(requests.len(), 1, "one request, no summary of the prefix apart");
        let request = &requests[0];
        assert_eq!(request.system_prompt, "You are Chef.");
        assert_eq!(request.tools.len(), 1);
        // Every message as the conversation sends it, then the prompt.
        let sent = as_llm(&messages);
        assert_eq!(request.messages.len(), sent.len() + 1);
        assert_eq!(format!("{:?}", &request.messages[..sent.len()]), format!("{sent:?}"));
        assert_eq!(request.cache_points, vec![3]);
        let LlmMessage::User(prompt) = request.messages.last().unwrap() else { panic!("a user message") };
        let prompt = prompt.content[0].as_text().unwrap();
        assert!(prompt.starts_with("[Context checkpoint]") && prompt.contains("Everything before your message that begins \"bbbb"), "{prompt}");
        assert!(prompt.contains("opens with the summary of an earlier part") && prompt.ends_with(SUMMARIZATION_PROMPT), "{prompt}");
        assert!(request.max_tokens.unwrap() >= MIN_IN_PLACE_REPLY_TOKENS);
    }

    #[tokio::test]
    async fn in_place_falls_back_to_a_summary_of_the_transcript() {
        // The cut falls at "third", between turns, so the transcript's summary is one request.
        let messages = vec![user("first"), assistant(&"a".repeat(400)), user("second"), assistant(&"b".repeat(400)), user("third"), assistant("c")];
        let settings = CompactionSettings { enabled: true, reserve_tokens: 16_384, keep_recent_tokens: 2, max_input_tokens: 0 };

        // The model went on with the work instead: the transcript is summarized as text.
        let provider = Conversation::answering(vec!["call:read", "## Goal\nFrom text"]);
        let result = compact_in_place(&provider, &shape(), as_llm, &messages, &settings, None, &RequestOptions::default(), &CancellationToken::new()).await.unwrap().unwrap();
        assert_eq!(result.summary, "## Goal\nFrom text");
        assert_eq!(result.usage.output, 14, "both requests count");
        let requests = provider.requests.lock().unwrap();
        assert_eq!(requests[1].system_prompt, SUMMARIZATION_SYSTEM_PROMPT);
        assert!(requests[1].tools.is_empty());

        // A context that leaves no room for the reply never asks inside it.
        let mut full = messages.clone();
        let mut huge = AssistantMessage::empty("p", "m");
        huge.content = vec![AssistantPart::Text { text: "z".repeat(4 * 195_000) }];
        full.insert(1, AgentMessage::Assistant(huge));
        let provider = Conversation::answering(vec!["## Goal\nFrom text"]);
        compact_in_place(&provider, &shape(), as_llm, &full, &settings, None, &RequestOptions::default(), &CancellationToken::new()).await.unwrap().unwrap();
        let requests = provider.requests.lock().unwrap();
        assert_eq!(requests.len(), 1);
        assert_eq!(requests[0].system_prompt, SUMMARIZATION_SYSTEM_PROMPT);
    }

    #[test]
    fn a_cut_reads_as_the_message_it_falls_before() {
        assert_eq!(describe_cut(&user("Fix   main.rs\ntoo")), "the message that begins \"Fix main.rs too\"");
        assert_eq!(describe_cut(&assistant("Renamed it.")), "your message that begins \"Renamed it.\"");
        assert_eq!(describe_cut(&call("read", "a.rs")), "your read call with {\"path\":\"a.rs\"}");
    }

    #[test]
    fn the_conversation_serializes_with_calls_and_results() {
        let text = serialize_conversation(&[user("hi"), call("read", "a.rs"), result("contents"), assistant("ok")]);
        assert_eq!(text, "[User]\nhi\n\n[Assistant]\n[Tool call: read({\"path\":\"a.rs\"})]\n\n[Tool result: read]\ncontents\n\n[Assistant]\nok\n\n");
    }
}
