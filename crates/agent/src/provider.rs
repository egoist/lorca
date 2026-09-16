use std::collections::HashMap;
use std::pin::Pin;

use async_trait::async_trait;
use futures::Stream;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use tokio_util::sync::CancellationToken;

use crate::types::{AssistantMessage, AssistantPart, LlmMessage, StopReason, ToolCall, Usage};

/// A tool as the model sees it.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolSpec {
    pub name: String,
    pub description: String,
    pub parameters: Value,
}

/// One model call.
#[derive(Debug, Clone)]
pub struct ModelRequest {
    pub system_prompt: String,
    pub messages: Vec<LlmMessage>,
    pub tools: Vec<ToolSpec>,
}

/// Streaming events a provider emits for one assistant message. `index` is the position of the
/// block inside the assistant message's `content`.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum AssistantEvent {
    Start,
    TextStart { index: usize },
    TextDelta { index: usize, delta: String },
    TextEnd { index: usize },
    ThinkingStart { index: usize },
    ThinkingDelta { index: usize, delta: String },
    ThinkingEnd { index: usize },
    /// The provider's seal on a thinking block, kept on the part for replay.
    ThinkingSignature { index: usize, signature: String },
    ToolCallStart { index: usize, id: String, name: String },
    ToolCallDelta { index: usize, delta: String },
    ToolCallEnd { index: usize },
    /// A tool the provider ran on its side (web search, page reads): not a block of the message
    /// and never executed by the loop; shown as the bot's activity while it runs.
    ServerToolStart { id: String, name: String, detail: String },
    ServerToolEnd { id: String, name: String, detail: String, summary: String },
    /// A block of the message the provider owns (a server tool call, its result): recorded as
    /// [`AssistantPart::ServerBlock`] at `index` so the turn can continue with it, and replaced
    /// when it arrives again with more of its content.
    ServerBlock { index: usize, block: Value },
    Done { stop_reason: StopReason, usage: Usage },
    Error { message: String, aborted: bool },
}

pub type AssistantEventStream = Pin<Box<dyn Stream<Item = AssistantEvent> + Send>>;

/// Names of the tools a provider runs on its side, reported through
/// [`AssistantEvent::ServerToolStart`] / [`AssistantEvent::ServerToolEnd`].
pub const WEB_SEARCH_TOOL: &str = "web_search";
pub const WEB_FETCH_TOOL: &str = "web_fetch";

/// Whether a tool name is one the provider runs itself, so a transcript row for it is a record
/// of activity and never a call for the loop to execute or replay.
pub fn is_server_tool(name: &str) -> bool {
    matches!(name, WEB_SEARCH_TOOL | WEB_FETCH_TOOL)
}

/// A model adapter.
///
/// Contract, after pi: `stream` never fails. Request, auth, and runtime failures are encoded in
/// the stream as an `Error` event, and the loop turns that into an assistant message whose
/// `stop_reason` is `error` or `aborted`.
#[async_trait]
pub trait Provider: Send + Sync {
    fn provider_id(&self) -> &str;
    fn model_id(&self) -> &str;
    async fn stream(&self, request: ModelRequest, cancel: CancellationToken) -> AssistantEventStream;
}

/// Builds the partial assistant message from a provider's event stream.
pub struct AssistantAccumulator {
    message: AssistantMessage,
    buffers: HashMap<usize, String>,
    done: bool,
}

impl AssistantAccumulator {
    pub fn new(provider: &str, model: &str) -> Self {
        AssistantAccumulator {
            message: AssistantMessage::empty(provider, model),
            buffers: HashMap::new(),
            done: false,
        }
    }

    pub fn message(&self) -> AssistantMessage {
        self.message.clone()
    }

    pub fn is_done(&self) -> bool {
        self.done
    }

    fn ensure(&mut self, index: usize, part: AssistantPart) {
        if index >= self.message.content.len() {
            self.message.content.push(part);
        }
    }

    pub fn apply(&mut self, event: &AssistantEvent) {
        match event {
            AssistantEvent::Start => {}
            AssistantEvent::TextStart { index } => {
                self.ensure(*index, AssistantPart::Text { text: String::new() });
            }
            AssistantEvent::TextDelta { index, delta } => {
                self.ensure(*index, AssistantPart::Text { text: String::new() });
                if let Some(AssistantPart::Text { text }) = self.message.content.get_mut(*index) {
                    text.push_str(delta);
                }
            }
            AssistantEvent::TextEnd { .. } => {}
            AssistantEvent::ThinkingStart { index } => {
                self.ensure(*index, AssistantPart::Thinking { thinking: String::new(), signature: None });
            }
            AssistantEvent::ThinkingDelta { index, delta } => {
                self.ensure(*index, AssistantPart::Thinking { thinking: String::new(), signature: None });
                if let Some(AssistantPart::Thinking { thinking, .. }) = self.message.content.get_mut(*index) {
                    thinking.push_str(delta);
                }
            }
            AssistantEvent::ThinkingEnd { .. } => {}
            AssistantEvent::ThinkingSignature { index, signature } => {
                self.ensure(*index, AssistantPart::Thinking { thinking: String::new(), signature: None });
                if let Some(AssistantPart::Thinking { signature: seal, .. }) = self.message.content.get_mut(*index) {
                    *seal = Some(signature.clone());
                }
            }
            AssistantEvent::ToolCallStart { index, id, name } => {
                self.ensure(
                    *index,
                    AssistantPart::ToolCall(ToolCall {
                        id: id.clone(),
                        name: name.clone(),
                        arguments: Value::Object(Default::default()),
                    }),
                );
                self.buffers.insert(*index, String::new());
            }
            AssistantEvent::ToolCallDelta { index, delta } => {
                self.buffers.entry(*index).or_default().push_str(delta);
            }
            AssistantEvent::ToolCallEnd { index } => {
                self.finish_tool_call(*index);
            }
            AssistantEvent::ServerToolStart { .. } | AssistantEvent::ServerToolEnd { .. } => {}
            AssistantEvent::ServerBlock { index, block } => {
                self.ensure(*index, AssistantPart::ServerBlock { block: block.clone() });
                if let Some(part) = self.message.content.get_mut(*index) {
                    *part = AssistantPart::ServerBlock { block: block.clone() };
                }
            }
            AssistantEvent::Done { stop_reason, usage } => {
                self.message.stop_reason = *stop_reason;
                self.message.usage = usage.clone();
                self.done = true;
            }
            AssistantEvent::Error { message, aborted } => {
                self.message.stop_reason = if *aborted { StopReason::Aborted } else { StopReason::Error };
                self.message.error_message = Some(message.clone());
                self.done = true;
            }
        }
    }

    fn finish_tool_call(&mut self, index: usize) {
        let Some(raw) = self.buffers.remove(&index) else { return };
        if let Some(AssistantPart::ToolCall(call)) = self.message.content.get_mut(index) {
            let trimmed = raw.trim();
            call.arguments = if trimmed.is_empty() {
                Value::Object(Default::default())
            } else {
                serde_json::from_str(trimmed).unwrap_or_else(|_| serde_json::json!({ "_raw": raw }))
            };
        }
    }

    /// Final message. Called when the stream ends, with or without a `Done`.
    pub fn finish(mut self, cancelled: bool) -> AssistantMessage {
        let open: Vec<usize> = self.buffers.keys().copied().collect();
        for index in open {
            self.finish_tool_call(index);
        }
        if !self.done {
            if cancelled {
                self.message.stop_reason = StopReason::Aborted;
                self.message.error_message = Some("Request aborted".into());
            } else {
                self.message.stop_reason = StopReason::Error;
                self.message.error_message = Some("Provider stream ended before completion".into());
            }
        }
        if self.message.stop_reason == StopReason::Stop && !self.message.tool_calls().is_empty() {
            self.message.stop_reason = StopReason::ToolUse;
        }
        self.message
    }
}

/// Turns a bounded channel into an [`AssistantEventStream`]. Providers spawn their HTTP work and
/// hand the receiver here.
pub fn channel_stream(rx: tokio::sync::mpsc::Receiver<AssistantEvent>) -> AssistantEventStream {
    Box::pin(futures::stream::unfold(rx, |mut rx| async move {
        rx.recv().await.map(|event| (event, rx))
    }))
}
