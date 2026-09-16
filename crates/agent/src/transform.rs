//! What every adapter does to the transcript before converting it, after pi's
//! `transformMessages`: images a model cannot see become a note, another model's thinking
//! becomes plain text, tool call ids get the shape the API wants, failed turns are left out,
//! and a call that never got a result gets one.

use crate::types::{AssistantMessage, AssistantPart, ContentPart, LlmMessage, StopReason, ToolResultMessage};

pub const NON_VISION_USER_IMAGE_PLACEHOLDER: &str = "(image omitted: model does not support images)";
pub const NON_VISION_TOOL_IMAGE_PLACEHOLDER: &str = "(tool image omitted: model does not support images)";
pub const NO_RESULT_PROVIDED: &str = "No result provided";

pub struct TransformOptions<'a> {
    /// The provider and model the request goes to. A message from the same pair is "own":
    /// its thinking seals and server blocks are kept, its tool call ids left alone.
    pub provider: &'a str,
    pub model: &'a str,
    pub supports_images: bool,
    /// Rewrites a tool call id from another model into the shape this API accepts.
    pub normalize_tool_call_id: Option<fn(&str) -> String>,
}

pub fn transform_messages(messages: &[LlmMessage], options: &TransformOptions<'_>) -> Vec<LlmMessage> {
    let mut id_map: Vec<(String, String)> = Vec::new();
    let mut transformed: Vec<LlmMessage> = Vec::with_capacity(messages.len());

    for message in messages {
        match message {
            LlmMessage::User(user) => {
                let mut user = user.clone();
                if !options.supports_images {
                    user.content = replace_images(&user.content, NON_VISION_USER_IMAGE_PLACEHOLDER);
                }
                transformed.push(LlmMessage::User(user));
            }
            LlmMessage::ToolResult(result) => {
                let mut result = result.clone();
                if !options.supports_images {
                    result.content = replace_images(&result.content, NON_VISION_TOOL_IMAGE_PLACEHOLDER);
                }
                if let Some((_, normalized)) = id_map.iter().find(|(original, _)| *original == result.tool_call_id) {
                    result.tool_call_id = normalized.clone();
                }
                transformed.push(LlmMessage::ToolResult(result));
            }
            LlmMessage::Assistant(assistant) => {
                let own = assistant.provider == options.provider && assistant.model == options.model;
                let mut out = assistant.clone();
                out.content = Vec::with_capacity(assistant.content.len());
                for part in &assistant.content {
                    match part {
                        AssistantPart::Thinking { thinking, signature } => {
                            if own && signature.as_deref().is_some_and(|s| !s.is_empty()) {
                                out.content.push(part.clone());
                            } else if thinking.trim().is_empty() {
                                // Nothing to say and no seal to keep.
                            } else if own {
                                out.content.push(part.clone());
                            } else {
                                out.content.push(AssistantPart::Text { text: thinking.clone() });
                            }
                        }
                        AssistantPart::Text { .. } => out.content.push(part.clone()),
                        AssistantPart::ToolCall(call) => {
                            let mut call = call.clone();
                            if !own {
                                if let Some(normalize) = options.normalize_tool_call_id {
                                    let normalized = normalize(&call.id);
                                    if normalized != call.id {
                                        id_map.push((call.id.clone(), normalized.clone()));
                                        call.id = normalized;
                                    }
                                }
                            }
                            out.content.push(AssistantPart::ToolCall(call));
                        }
                        AssistantPart::ServerBlock { .. } => {
                            if own {
                                out.content.push(part.clone());
                            }
                        }
                    }
                }
                transformed.push(LlmMessage::Assistant(out));
            }
        }
    }

    // Second pass: a failed or aborted turn is not replayed, and every tool call gets a
    // result before the next assistant turn, the next user message, or the end.
    let mut result: Vec<LlmMessage> = Vec::with_capacity(transformed.len());
    let mut pending: Vec<(String, String)> = Vec::new();
    let mut answered: Vec<String> = Vec::new();
    fn settle(result: &mut Vec<LlmMessage>, pending: &mut Vec<(String, String)>, answered: &mut Vec<String>) {
        for (id, name) in pending.drain(..) {
            if !answered.contains(&id) {
                result.push(LlmMessage::ToolResult(ToolResultMessage {
                    tool_call_id: id,
                    tool_name: name,
                    content: vec![ContentPart::text(NO_RESULT_PROVIDED)],
                    details: serde_json::Value::Null,
                    is_error: true,
                    timestamp: crate::now_ms(),
                }));
            }
        }
        answered.clear();
    }
    for message in transformed {
        match message {
            LlmMessage::Assistant(assistant) => {
                settle(&mut result, &mut pending, &mut answered);
                if matches!(assistant.stop_reason, StopReason::Error | StopReason::Aborted) {
                    continue;
                }
                let calls: Vec<(String, String)> = assistant.tool_calls().iter().map(|c| (c.id.clone(), c.name.clone())).collect();
                if !calls.is_empty() {
                    pending = calls;
                }
                result.push(LlmMessage::Assistant(assistant));
            }
            LlmMessage::ToolResult(tool_result) => {
                answered.push(tool_result.tool_call_id.clone());
                result.push(LlmMessage::ToolResult(tool_result));
            }
            LlmMessage::User(user) => {
                settle(&mut result, &mut pending, &mut answered);
                result.push(LlmMessage::User(user));
            }
        }
    }
    settle(&mut result, &mut pending, &mut answered);
    result
}

fn replace_images(content: &[ContentPart], placeholder: &str) -> Vec<ContentPart> {
    let mut out = Vec::with_capacity(content.len());
    let mut previous_was_placeholder = false;
    for part in content {
        match part {
            ContentPart::Image { .. } => {
                if !previous_was_placeholder {
                    out.push(ContentPart::text(placeholder));
                }
                previous_was_placeholder = true;
            }
            ContentPart::Text { text } => {
                previous_was_placeholder = text == placeholder;
                out.push(part.clone());
            }
        }
    }
    out
}

/// Whether the transcript's assistant message came from this provider and model.
pub fn is_own(message: &AssistantMessage, provider: &str, model: &str) -> bool {
    message.provider == provider && message.model == model
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::{ToolCall, UserMessage};
    use serde_json::json;

    fn options() -> TransformOptions<'static> {
        TransformOptions { provider: "p", model: "m", supports_images: true, normalize_tool_call_id: None }
    }

    fn assistant(provider: &str, parts: Vec<AssistantPart>) -> AssistantMessage {
        let mut message = AssistantMessage::empty(provider, "m");
        message.content = parts;
        message
    }

    #[test]
    fn foreign_thinking_becomes_text_and_own_seals_stay() {
        let own = assistant("p", vec![
            AssistantPart::Thinking { thinking: String::new(), signature: Some("sig".into()) },
            AssistantPart::Thinking { thinking: "unsigned".into(), signature: None },
            AssistantPart::ServerBlock { block: json!({ "type": "server_tool_use" }) },
        ]);
        let foreign = assistant("", vec![
            AssistantPart::Thinking { thinking: "old reasoning".into(), signature: Some("x".into()) },
            AssistantPart::Thinking { thinking: "  ".into(), signature: None },
            AssistantPart::ServerBlock { block: json!({ "type": "server_tool_use" }) },
            AssistantPart::Text { text: "hi".into() },
        ]);
        let out = transform_messages(&[LlmMessage::Assistant(foreign), LlmMessage::Assistant(own)], &options());
        let LlmMessage::Assistant(foreign) = &out[0] else { panic!() };
        assert_eq!(foreign.content, vec![AssistantPart::Text { text: "old reasoning".into() }, AssistantPart::Text { text: "hi".into() }]);
        let LlmMessage::Assistant(own) = &out[1] else { panic!() };
        assert_eq!(own.content.len(), 3);
    }

    #[test]
    fn failed_turns_are_dropped_and_orphaned_calls_get_a_result() {
        let mut failed = assistant("p", vec![AssistantPart::Text { text: "partial".into() }]);
        failed.stop_reason = StopReason::Error;
        let calls = assistant("p", vec![
            AssistantPart::ToolCall(ToolCall { id: "a".into(), name: "read".into(), arguments: json!({}) }),
            AssistantPart::ToolCall(ToolCall { id: "b".into(), name: "read".into(), arguments: json!({}) }),
        ]);
        let result_a = ToolResultMessage { tool_call_id: "a".into(), tool_name: "read".into(), content: vec![ContentPart::text("ok")], details: json!(null), is_error: false, timestamp: 0 };
        let out = transform_messages(
            &[LlmMessage::User(UserMessage::text("go")), LlmMessage::Assistant(failed), LlmMessage::Assistant(calls), LlmMessage::ToolResult(result_a), LlmMessage::User(UserMessage::text("next"))],
            &options(),
        );
        let roles: Vec<&str> = out.iter().map(|m| match m { LlmMessage::User(_) => "user", LlmMessage::Assistant(_) => "assistant", LlmMessage::ToolResult(_) => "tool" }).collect();
        assert_eq!(roles, vec!["user", "assistant", "tool", "tool", "user"]);
        let LlmMessage::ToolResult(synthetic) = &out[3] else { panic!() };
        assert_eq!((synthetic.tool_call_id.as_str(), synthetic.is_error, synthetic.text().as_str()), ("b", true, NO_RESULT_PROVIDED));
    }

    #[test]
    fn foreign_tool_call_ids_are_normalized_with_their_results() {
        let calls = assistant("other", vec![AssistantPart::ToolCall(ToolCall { id: "call|weird".into(), name: "x".into(), arguments: json!({}) })]);
        let result = ToolResultMessage { tool_call_id: "call|weird".into(), tool_name: "x".into(), content: vec![], details: json!(null), is_error: false, timestamp: 0 };
        let opts = TransformOptions { normalize_tool_call_id: Some(|id| id.replace('|', "_")), ..options() };
        let out = transform_messages(&[LlmMessage::Assistant(calls), LlmMessage::ToolResult(result)], &opts);
        let LlmMessage::Assistant(a) = &out[0] else { panic!() };
        assert_eq!(a.tool_calls()[0].id, "call_weird");
        let LlmMessage::ToolResult(r) = &out[1] else { panic!() };
        assert_eq!(r.tool_call_id, "call_weird");
    }

    #[test]
    fn images_become_one_note_for_a_text_only_model() {
        let user = UserMessage { content: vec![ContentPart::text("see"), ContentPart::Image { data: "a".into(), mime_type: "image/png".into() }, ContentPart::Image { data: "b".into(), mime_type: "image/png".into() }], timestamp: 0 };
        let opts = TransformOptions { supports_images: false, ..options() };
        let out = transform_messages(&[LlmMessage::User(user)], &opts);
        let LlmMessage::User(u) = &out[0] else { panic!() };
        assert_eq!(u.content, vec![ContentPart::text("see"), ContentPart::text(NON_VISION_USER_IMAGE_PLACEHOLDER)]);
    }
}
