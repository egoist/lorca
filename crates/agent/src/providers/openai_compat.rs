//! OpenAI-compatible `/chat/completions` streaming. DeepSeek uses this as is.

use std::collections::HashMap;

use async_trait::async_trait;
use futures::StreamExt;
use serde_json::{json, Value};
use tokio::sync::mpsc;
use tokio_util::sync::CancellationToken;

use crate::provider::{channel_stream, AssistantEvent, AssistantEventStream, ModelRequest, Provider};
use crate::sse::SseParser;
use crate::types::{AssistantPart, ContentPart, LlmMessage, StopReason, Usage};

pub const DEEPSEEK_BASE_URL: &str = "https://api.deepseek.com";
/// DeepSeek V4.1 Flash. `deepseek-v4-pro` is the reasoning-heavy option; the legacy
/// `deepseek-chat` / `deepseek-reasoner` names were retired in 2026.
pub const DEEPSEEK_DEFAULT_MODEL: &str = "deepseek-flash";

pub struct OpenAiCompatProvider {
    pub provider_id: String,
    pub base_url: String,
    pub api_key: String,
    pub model: String,
    client: reqwest::Client,
}

impl OpenAiCompatProvider {
    pub fn new(provider_id: &str, base_url: &str, api_key: &str, model: &str) -> Self {
        OpenAiCompatProvider {
            provider_id: provider_id.to_string(),
            base_url: base_url.trim_end_matches('/').to_string(),
            api_key: api_key.to_string(),
            model: model.to_string(),
            client: reqwest::Client::new(),
        }
    }

    pub fn deepseek(api_key: &str, model: Option<&str>) -> Self {
        Self::new("deepseek", DEEPSEEK_BASE_URL, api_key, model.unwrap_or(DEEPSEEK_DEFAULT_MODEL))
    }

    fn body(&self, request: &ModelRequest) -> Value {
        let mut messages = Vec::new();
        if !request.system_prompt.trim().is_empty() {
            messages.push(json!({ "role": "system", "content": request.system_prompt }));
        }
        for message in &request.messages {
            messages.push(convert_message(message));
        }

        let mut body = json!({
            "model": self.model,
            "messages": messages,
            "stream": true,
            "stream_options": { "include_usage": true },
        });
        if !request.tools.is_empty() {
            body["tools"] = Value::Array(
                request
                    .tools
                    .iter()
                    .map(|tool| {
                        json!({
                            "type": "function",
                            "function": {
                                "name": tool.name,
                                "description": tool.description,
                                "parameters": tool.parameters,
                            }
                        })
                    })
                    .collect(),
            );
        }
        body
    }
}

fn convert_message(message: &LlmMessage) -> Value {
    match message {
        LlmMessage::User(user) => {
            let only_text = user.content.iter().all(|part| matches!(part, ContentPart::Text { .. }));
            if only_text {
                let text = user.content.iter().filter_map(ContentPart::as_text).collect::<Vec<_>>().join("\n");
                json!({ "role": "user", "content": text })
            } else {
                let parts: Vec<Value> = user
                    .content
                    .iter()
                    .map(|part| match part {
                        ContentPart::Text { text } => json!({ "type": "text", "text": text }),
                        ContentPart::Image { data, mime_type } => json!({
                            "type": "image_url",
                            "image_url": { "url": format!("data:{mime_type};base64,{data}") }
                        }),
                    })
                    .collect();
                json!({ "role": "user", "content": parts })
            }
        }
        LlmMessage::Assistant(assistant) => {
            let text = assistant.text();
            let tool_calls: Vec<Value> = assistant
                .content
                .iter()
                .filter_map(|part| match part {
                    AssistantPart::ToolCall(call) => Some(json!({
                        "id": call.id,
                        "type": "function",
                        "function": {
                            "name": call.name,
                            "arguments": call.arguments.to_string(),
                        }
                    })),
                    _ => None,
                })
                .collect();
            let mut value = json!({ "role": "assistant" });
            value["content"] = if text.is_empty() { Value::Null } else { Value::String(text) };
            if !tool_calls.is_empty() {
                value["tool_calls"] = Value::Array(tool_calls);
            }
            value
        }
        LlmMessage::ToolResult(result) => json!({
            "role": "tool",
            "tool_call_id": result.tool_call_id,
            "content": result.text(),
        }),
    }
}

/// Tracks which content block each delta belongs to.
#[derive(Default)]
struct StreamState {
    next_index: usize,
    text: Option<usize>,
    thinking: Option<usize>,
    tool_calls: HashMap<u64, usize>,
    open_tool_calls: Vec<usize>,
    stop_reason: Option<StopReason>,
    usage: Usage,
}

impl StreamState {
    async fn close_text(&mut self, tx: &mpsc::Sender<AssistantEvent>) {
        if let Some(index) = self.text.take() {
            let _ = tx.send(AssistantEvent::TextEnd { index }).await;
        }
    }

    async fn close_thinking(&mut self, tx: &mpsc::Sender<AssistantEvent>) {
        if let Some(index) = self.thinking.take() {
            let _ = tx.send(AssistantEvent::ThinkingEnd { index }).await;
        }
    }

    async fn close_tool_calls(&mut self, tx: &mpsc::Sender<AssistantEvent>) {
        for index in self.open_tool_calls.drain(..) {
            let _ = tx.send(AssistantEvent::ToolCallEnd { index }).await;
        }
    }

    async fn apply_chunk(&mut self, chunk: &Value, tx: &mpsc::Sender<AssistantEvent>) {
        if let Some(usage) = chunk.get("usage").filter(|u| !u.is_null()) {
            self.usage = Usage {
                input: usage["prompt_tokens"].as_u64().unwrap_or(0),
                output: usage["completion_tokens"].as_u64().unwrap_or(0),
                cache_read: usage["prompt_cache_hit_tokens"].as_u64().unwrap_or(0),
                cache_write: 0,
                total_tokens: usage["total_tokens"].as_u64().unwrap_or(0),
            };
        }

        let Some(choice) = chunk["choices"].as_array().and_then(|c| c.first()) else { return };
        let delta = &choice["delta"];

        if let Some(reasoning) = delta["reasoning_content"].as_str().filter(|s| !s.is_empty()) {
            self.close_text(tx).await;
            let index = match self.thinking {
                Some(index) => index,
                None => {
                    let index = self.next_index;
                    self.next_index += 1;
                    self.thinking = Some(index);
                    let _ = tx.send(AssistantEvent::ThinkingStart { index }).await;
                    index
                }
            };
            let _ = tx.send(AssistantEvent::ThinkingDelta { index, delta: reasoning.to_string() }).await;
        }

        if let Some(content) = delta["content"].as_str().filter(|s| !s.is_empty()) {
            self.close_thinking(tx).await;
            let index = match self.text {
                Some(index) => index,
                None => {
                    let index = self.next_index;
                    self.next_index += 1;
                    self.text = Some(index);
                    let _ = tx.send(AssistantEvent::TextStart { index }).await;
                    index
                }
            };
            let _ = tx.send(AssistantEvent::TextDelta { index, delta: content.to_string() }).await;
        }

        if let Some(calls) = delta["tool_calls"].as_array() {
            self.close_text(tx).await;
            self.close_thinking(tx).await;
            for call in calls {
                let provider_index = call["index"].as_u64().unwrap_or(0);
                let index = match self.tool_calls.get(&provider_index) {
                    Some(index) => *index,
                    None => {
                        let index = self.next_index;
                        self.next_index += 1;
                        self.tool_calls.insert(provider_index, index);
                        self.open_tool_calls.push(index);
                        let id = call["id"].as_str().map(str::to_string).unwrap_or_else(|| format!("call_{provider_index}"));
                        let name = call["function"]["name"].as_str().unwrap_or("").to_string();
                        let _ = tx.send(AssistantEvent::ToolCallStart { index, id, name }).await;
                        index
                    }
                };
                if let Some(arguments) = call["function"]["arguments"].as_str().filter(|s| !s.is_empty()) {
                    let _ = tx.send(AssistantEvent::ToolCallDelta { index, delta: arguments.to_string() }).await;
                }
            }
        }

        if let Some(reason) = choice["finish_reason"].as_str() {
            self.stop_reason = Some(match reason {
                "length" => StopReason::Length,
                "tool_calls" | "function_call" => StopReason::ToolUse,
                _ => StopReason::Stop,
            });
        }
    }
}

#[async_trait]
impl Provider for OpenAiCompatProvider {
    fn provider_id(&self) -> &str {
        &self.provider_id
    }

    fn model_id(&self) -> &str {
        &self.model
    }

    async fn stream(&self, request: ModelRequest, cancel: CancellationToken) -> AssistantEventStream {
        let (tx, rx) = mpsc::channel(64);
        let body = self.body(&request);
        let url = format!("{}/chat/completions", self.base_url);
        let client = self.client.clone();
        let api_key = self.api_key.clone();

        tokio::spawn(async move {
            let response = tokio::select! {
                _ = cancel.cancelled() => {
                    let _ = tx.send(AssistantEvent::Error { message: "Request aborted".into(), aborted: true }).await;
                    return;
                }
                response = client.post(&url).bearer_auth(&api_key).json(&body).send() => response,
            };

            let response = match response {
                Ok(response) => response,
                Err(error) => {
                    let _ = tx.send(AssistantEvent::Error { message: format!("Request failed: {error}"), aborted: false }).await;
                    return;
                }
            };

            if !response.status().is_success() {
                let status = response.status();
                let text = response.text().await.unwrap_or_default();
                let _ = tx
                    .send(AssistantEvent::Error { message: format!("{status}: {}", summarize_error(&text)), aborted: false })
                    .await;
                return;
            }

            let _ = tx.send(AssistantEvent::Start).await;
            let mut parser = SseParser::new();
            let mut state = StreamState::default();
            let mut bytes = response.bytes_stream();

            loop {
                let chunk = tokio::select! {
                    _ = cancel.cancelled() => {
                        let _ = tx.send(AssistantEvent::Error { message: "Request aborted".into(), aborted: true }).await;
                        return;
                    }
                    chunk = bytes.next() => chunk,
                };
                let Some(chunk) = chunk else { break };
                let chunk = match chunk {
                    Ok(chunk) => chunk,
                    Err(error) => {
                        let _ = tx.send(AssistantEvent::Error { message: format!("Stream failed: {error}"), aborted: false }).await;
                        return;
                    }
                };
                for event in parser.push(&chunk) {
                    if event.data.trim() == "[DONE]" {
                        continue;
                    }
                    match serde_json::from_str::<Value>(&event.data) {
                        Ok(value) => {
                            if let Some(error) = value.get("error") {
                                let message = error["message"].as_str().unwrap_or("Provider error").to_string();
                                let _ = tx.send(AssistantEvent::Error { message, aborted: false }).await;
                                return;
                            }
                            state.apply_chunk(&value, &tx).await;
                        }
                        Err(_) => tracing::debug!(data = %event.data, "unparsed sse chunk"),
                    }
                }
            }

            state.close_text(&tx).await;
            state.close_thinking(&tx).await;
            state.close_tool_calls(&tx).await;
            let stop_reason = state.stop_reason.unwrap_or(StopReason::Stop);
            let _ = tx.send(AssistantEvent::Done { stop_reason, usage: state.usage.clone() }).await;
        });

        channel_stream(rx)
    }
}

fn summarize_error(text: &str) -> String {
    if let Ok(value) = serde_json::from_str::<Value>(text) {
        if let Some(message) = value["error"]["message"].as_str() {
            return message.to_string();
        }
    }
    let trimmed = text.trim();
    if trimmed.len() > 300 {
        format!("{}…", &trimmed[..300])
    } else {
        trimmed.to_string()
    }
}
