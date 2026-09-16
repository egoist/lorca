//! How big the context is, after pi's `estimate`: the provider's own count from the last
//! assistant message that has one, plus a character heuristic for everything after it.

use crate::types::{AgentMessage, AssistantMessage, AssistantPart, ContentPart, StopReason, Usage};

pub const CHARS_PER_TOKEN: usize = 4;
/// What an image costs in tokens, roughly, when nothing better is known.
pub const ESTIMATED_IMAGE_CHARS: usize = 4800;

/// The context size a response's usage describes: everything the request carried plus what
/// the model wrote.
pub fn context_tokens(usage: &Usage) -> u64 {
    if usage.total_tokens > 0 {
        usage.total_tokens
    } else {
        usage.input + usage.output + usage.cache_read + usage.cache_write
    }
}

pub fn estimate_text_tokens(text: &str) -> u64 {
    text.len().div_ceil(CHARS_PER_TOKEN) as u64
}

fn content_chars(content: &[ContentPart]) -> usize {
    content
        .iter()
        .map(|part| match part {
            ContentPart::Text { text } => text.len(),
            ContentPart::Image { .. } => ESTIMATED_IMAGE_CHARS,
        })
        .sum()
}

/// A rough token count for one message.
pub fn estimate_message_tokens(message: &AgentMessage) -> u64 {
    let chars = match message {
        AgentMessage::User(user) => content_chars(&user.content),
        AgentMessage::ToolResult(result) => content_chars(&result.content),
        AgentMessage::Assistant(assistant) => assistant
            .content
            .iter()
            .map(|part| match part {
                AssistantPart::Text { text } => text.len(),
                AssistantPart::Thinking { thinking, .. } => thinking.len(),
                AssistantPart::ToolCall(call) => call.name.len() + call.arguments.to_string().len(),
                AssistantPart::ServerBlock { block } => block.to_string().len(),
            })
            .sum(),
        AgentMessage::Custom { data, .. } => data.to_string().len(),
    };
    chars.div_ceil(CHARS_PER_TOKEN) as u64
}

/// The usage a completed assistant message reports, when it has one.
fn assistant_usage(message: &AssistantMessage) -> Option<&Usage> {
    let settled = !matches!(message.stop_reason, StopReason::Error | StopReason::Aborted);
    (settled && context_tokens(&message.usage) > 0).then_some(&message.usage)
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ContextEstimate {
    /// The whole context, as well as it is known.
    pub tokens: u64,
    /// What the provider counted for the last response that has a count.
    pub usage_tokens: u64,
    /// The estimate for what came after that response.
    pub trailing_tokens: u64,
    /// The message that provided the count, when one did.
    pub last_usage_index: Option<usize>,
}

/// The context size of a transcript: the last provider count that still describes the prefix
/// (a message inserted after it, such as a summary, retires it) plus an estimate of the rest.
pub fn estimate_context_tokens(messages: &[AgentMessage]) -> ContextEstimate {
    let mut latest_prefix_timestamp = 0u64;
    let mut usage_info: Option<(u64, usize)> = None;
    for (index, message) in messages.iter().enumerate() {
        if let AgentMessage::Assistant(assistant) = message {
            if assistant.timestamp >= latest_prefix_timestamp {
                if let Some(usage) = assistant_usage(assistant) {
                    usage_info = Some((context_tokens(usage), index));
                }
            }
        }
        latest_prefix_timestamp = latest_prefix_timestamp.max(message.timestamp());
    }
    match usage_info {
        Some((usage_tokens, index)) => {
            let trailing_tokens: u64 = messages[index + 1..].iter().map(estimate_message_tokens).sum();
            ContextEstimate { tokens: usage_tokens + trailing_tokens, usage_tokens, trailing_tokens, last_usage_index: Some(index) }
        }
        None => {
            let tokens: u64 = messages.iter().map(estimate_message_tokens).sum();
            ContextEstimate { tokens, usage_tokens: 0, trailing_tokens: tokens, last_usage_index: None }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::UserMessage;

    #[test]
    fn the_last_count_wins_and_later_messages_are_estimated() {
        let mut assistant = AssistantMessage::empty("p", "m");
        assistant.usage = Usage { input: 900, output: 100, ..Usage::default() };
        assistant.timestamp = 10;
        let mut later = UserMessage::text("x".repeat(400));
        later.timestamp = 11;
        let mut first = UserMessage::text("hi");
        first.timestamp = 5;
        let messages = vec![AgentMessage::User(first), AgentMessage::Assistant(assistant), AgentMessage::User(later)];
        let estimate = estimate_context_tokens(&messages);
        assert_eq!(estimate, ContextEstimate { tokens: 1100, usage_tokens: 1000, trailing_tokens: 100, last_usage_index: Some(1) });
    }

    #[test]
    fn a_summary_inserted_before_the_count_retires_it() {
        let mut assistant = AssistantMessage::empty("p", "m");
        assistant.usage = Usage { input: 900, output: 100, ..Usage::default() };
        assistant.timestamp = 10;
        let mut summary = UserMessage::text("s".repeat(40));
        summary.timestamp = 20;
        let messages = vec![AgentMessage::User(summary), AgentMessage::Assistant(assistant)];
        let estimate = estimate_context_tokens(&messages);
        assert_eq!(estimate.last_usage_index, None);
        assert_eq!(estimate.tokens, 10);
    }
}
