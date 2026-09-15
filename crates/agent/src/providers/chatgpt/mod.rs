//! ChatGPT subscription adapter. Isolated from the API-key providers: it authenticates with
//! the OAuth tokens a ChatGPT login yields and calls the Codex responses backend.

pub mod oauth;

use std::sync::Arc;

use async_trait::async_trait;
use futures::StreamExt;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::collections::HashMap;
use tokio::sync::mpsc;
use tokio_util::sync::CancellationToken;

use crate::provider::{channel_stream, AssistantEvent, AssistantEventStream, ModelRequest, Provider};
use crate::sse::SseParser;
use crate::types::{AssistantPart, ContentPart, LlmMessage, StopReason, Usage};

pub const CHATGPT_RESPONSES_URL: &str = "https://chatgpt.com/backend-api/codex/responses";
/// The balanced everyday model Codex offers to ChatGPT sign-ins. Others accepted with a
/// ChatGPT account: `gpt-6-astra`, `gpt-5.6-sol`, `gpt-5.6-luna`, `gpt-5.5`. The `*-codex`
/// ids and `gpt-5.4` are rejected for ChatGPT accounts.
pub const CHATGPT_DEFAULT_MODEL: &str = "gpt-5.6-terra";

/// Tokens from a ChatGPT login. The CLI persists these on the Runner.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ChatGptTokens {
    pub access_token: String,
    pub refresh_token: String,
    #[serde(default)]
    pub id_token: Option<String>,
    pub account_id: String,
    #[serde(default)]
    pub email: Option<String>,
    /// Unix seconds.
    pub expires_at: u64,
}

impl ChatGptTokens {
    pub fn is_expired(&self) -> bool {
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_secs())
            .unwrap_or(0);
        now + 60 >= self.expires_at
    }
}

/// Where the adapter reads tokens from and writes refreshed ones back to.
#[async_trait]
pub trait TokenSource: Send + Sync {
    async fn tokens(&self) -> Result<ChatGptTokens, String>;
    async fn store(&self, tokens: ChatGptTokens) -> Result<(), String>;
}

pub struct ChatGptProvider {
    tokens: Arc<dyn TokenSource>,
    model: String,
    client: reqwest::Client,
}

impl ChatGptProvider {
    pub fn new(tokens: Arc<dyn TokenSource>, model: Option<&str>) -> Self {
        ChatGptProvider {
            tokens,
            model: model.unwrap_or(CHATGPT_DEFAULT_MODEL).to_string(),
            client: reqwest::Client::new(),
        }
    }

    async fn fresh_tokens(&self) -> Result<ChatGptTokens, String> {
        let tokens = self.tokens.tokens().await?;
        if !tokens.is_expired() {
            return Ok(tokens);
        }
        let refreshed = oauth::refresh(&self.client, &tokens.refresh_token).await?;
        self.tokens.store(refreshed.clone()).await?;
        Ok(refreshed)
    }

    fn body(&self, request: &ModelRequest) -> Value {
        let mut input = Vec::new();
        for message in &request.messages {
            match message {
                LlmMessage::User(user) => {
                    let content: Vec<Value> = user
                        .content
                        .iter()
                        .map(|part| match part {
                            ContentPart::Text { text } => json!({ "type": "input_text", "text": text }),
                            ContentPart::Image { data, mime_type } => json!({
                                "type": "input_image",
                                "image_url": format!("data:{mime_type};base64,{data}"),
                            }),
                        })
                        .collect();
                    input.push(json!({ "type": "message", "role": "user", "content": content }));
                }
                LlmMessage::Assistant(assistant) => {
                    let text = assistant.text();
                    if !text.is_empty() {
                        input.push(json!({
                            "type": "message",
                            "role": "assistant",
                            "content": [{ "type": "output_text", "text": text }],
                        }));
                    }
                    for part in &assistant.content {
                        if let AssistantPart::ToolCall(call) = part {
                            input.push(json!({
                                "type": "function_call",
                                "call_id": call.id,
                                "name": call.name,
                                "arguments": call.arguments.to_string(),
                            }));
                        }
                    }
                }
                LlmMessage::ToolResult(result) => {
                    input.push(json!({
                        "type": "function_call_output",
                        "call_id": result.tool_call_id,
                        "output": result.text(),
                    }));
                }
            }
        }

        let tools: Vec<Value> = request
            .tools
            .iter()
            .map(|tool| {
                json!({
                    "type": "function",
                    "name": tool.name,
                    "description": tool.description,
                    "parameters": tool.parameters,
                    "strict": false,
                })
            })
            .collect();

        json!({
            "model": self.model,
            "instructions": request.system_prompt,
            "input": input,
            "tools": tools,
            "tool_choice": "auto",
            "parallel_tool_calls": true,
            "store": false,
            "stream": true,
        })
    }
}

#[derive(Default)]
struct ResponsesState {
    next_index: usize,
    items: HashMap<String, usize>,
    text_item: Option<usize>,
    thinking_item: Option<usize>,
    tool_deltas_seen: HashMap<usize, bool>,
    stop_reason: StopReason,
    usage: Usage,
}

impl ResponsesState {
    fn new() -> Self {
        Self::default()
    }

    async fn apply(&mut self, kind: &str, value: &Value, tx: &mpsc::Sender<AssistantEvent>) -> Result<bool, String> {
        match kind {
            "response.output_item.added" => {
                let item = &value["item"];
                let item_id = item["id"].as_str().unwrap_or("").to_string();
                match item["type"].as_str().unwrap_or("") {
                    "function_call" => {
                        let index = self.next_index;
                        self.next_index += 1;
                        self.items.insert(item_id, index);
                        let id = item["call_id"].as_str().unwrap_or("").to_string();
                        let name = item["name"].as_str().unwrap_or("").to_string();
                        let _ = tx.send(AssistantEvent::ToolCallStart { index, id, name }).await;
                        self.stop_reason = StopReason::ToolUse;
                    }
                    "message" => {
                        let index = self.next_index;
                        self.next_index += 1;
                        self.items.insert(item_id, index);
                        self.text_item = Some(index);
                        let _ = tx.send(AssistantEvent::TextStart { index }).await;
                    }
                    "reasoning" => {
                        let index = self.next_index;
                        self.next_index += 1;
                        self.items.insert(item_id, index);
                        self.thinking_item = Some(index);
                        let _ = tx.send(AssistantEvent::ThinkingStart { index }).await;
                    }
                    _ => {}
                }
            }
            "response.output_text.delta" => {
                let index = self.index_for(&value["item_id"]).or(self.text_item);
                if let (Some(index), Some(delta)) = (index, value["delta"].as_str()) {
                    let _ = tx.send(AssistantEvent::TextDelta { index, delta: delta.to_string() }).await;
                }
            }
            "response.reasoning_summary_text.delta" | "response.reasoning_text.delta" => {
                let index = self.index_for(&value["item_id"]).or(self.thinking_item);
                if let (Some(index), Some(delta)) = (index, value["delta"].as_str()) {
                    let _ = tx.send(AssistantEvent::ThinkingDelta { index, delta: delta.to_string() }).await;
                }
            }
            "response.function_call_arguments.delta" => {
                if let (Some(index), Some(delta)) = (self.index_for(&value["item_id"]), value["delta"].as_str()) {
                    self.tool_deltas_seen.insert(index, true);
                    let _ = tx.send(AssistantEvent::ToolCallDelta { index, delta: delta.to_string() }).await;
                }
            }
            "response.output_item.done" => {
                let item = &value["item"];
                let Some(index) = self.index_for(&item["id"]) else { return Ok(false) };
                match item["type"].as_str().unwrap_or("") {
                    "function_call" => {
                        if !self.tool_deltas_seen.contains_key(&index) {
                            if let Some(arguments) = item["arguments"].as_str() {
                                let _ = tx.send(AssistantEvent::ToolCallDelta { index, delta: arguments.to_string() }).await;
                            }
                        }
                        let _ = tx.send(AssistantEvent::ToolCallEnd { index }).await;
                    }
                    "message" => {
                        let _ = tx.send(AssistantEvent::TextEnd { index }).await;
                        self.text_item = None;
                    }
                    "reasoning" => {
                        let _ = tx.send(AssistantEvent::ThinkingEnd { index }).await;
                        self.thinking_item = None;
                    }
                    _ => {}
                }
            }
            "response.completed" | "response.done" => {
                let usage = &value["response"]["usage"];
                self.usage = Usage {
                    input: usage["input_tokens"].as_u64().unwrap_or(0),
                    output: usage["output_tokens"].as_u64().unwrap_or(0),
                    cache_read: usage["input_tokens_details"]["cached_tokens"].as_u64().unwrap_or(0),
                    cache_write: 0,
                    total_tokens: usage["total_tokens"].as_u64().unwrap_or(0),
                };
                if value["response"]["status"].as_str() == Some("incomplete") {
                    self.stop_reason = StopReason::Length;
                }
                return Ok(true);
            }
            "response.incomplete" => {
                self.stop_reason = StopReason::Length;
                return Ok(true);
            }
            "response.failed" => {
                let message = value["response"]["error"]["message"].as_str().unwrap_or("Response failed").to_string();
                return Err(message);
            }
            "error" => {
                let message = value["message"].as_str().or(value["error"]["message"].as_str()).unwrap_or("Provider error");
                return Err(message.to_string());
            }
            _ => {}
        }
        Ok(false)
    }

    fn index_for(&self, item_id: &Value) -> Option<usize> {
        item_id.as_str().and_then(|id| self.items.get(id).copied())
    }
}

#[async_trait]
impl Provider for ChatGptProvider {
    fn provider_id(&self) -> &str {
        "chatgpt"
    }

    fn model_id(&self) -> &str {
        &self.model
    }

    async fn stream(&self, request: ModelRequest, cancel: CancellationToken) -> AssistantEventStream {
        let (tx, rx) = mpsc::channel(64);
        let body = self.body(&request);
        let client = self.client.clone();
        let tokens = match self.fresh_tokens().await {
            Ok(tokens) => tokens,
            Err(message) => {
                tokio::spawn(async move {
                    let _ = tx.send(AssistantEvent::Error { message: format!("ChatGPT sign-in: {message}"), aborted: false }).await;
                });
                return channel_stream(rx);
            }
        };

        tokio::spawn(async move {
            let send = client
                .post(CHATGPT_RESPONSES_URL)
                .bearer_auth(&tokens.access_token)
                .header("chatgpt-account-id", &tokens.account_id)
                .header("OpenAI-Beta", "responses=experimental")
                .header("originator", "codex_cli_rs")
                .header("Accept", "text/event-stream")
                .json(&body)
                .send();

            let response = tokio::select! {
                _ = cancel.cancelled() => {
                    let _ = tx.send(AssistantEvent::Error { message: "Request aborted".into(), aborted: true }).await;
                    return;
                }
                response = send => response,
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
                let summary = serde_json::from_str::<Value>(&text)
                    .ok()
                    .and_then(|v| v["error"]["message"].as_str().map(str::to_string))
                    .unwrap_or_else(|| text.chars().take(300).collect());
                let _ = tx.send(AssistantEvent::Error { message: format!("{status}: {summary}"), aborted: false }).await;
                return;
            }

            let _ = tx.send(AssistantEvent::Start).await;
            let mut parser = SseParser::new();
            let mut state = ResponsesState::new();
            let mut bytes = response.bytes_stream();
            let mut completed = false;

            'outer: loop {
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
                    let Ok(value) = serde_json::from_str::<Value>(&event.data) else { continue };
                    let kind = value["type"].as_str().map(str::to_string).or(event.event.clone()).unwrap_or_default();
                    match state.apply(&kind, &value, &tx).await {
                        Ok(true) => {
                            completed = true;
                            break 'outer;
                        }
                        Ok(false) => {}
                        Err(message) => {
                            let _ = tx.send(AssistantEvent::Error { message, aborted: false }).await;
                            return;
                        }
                    }
                }
            }

            if !completed {
                tracing::debug!("chatgpt stream ended without response.completed");
            }
            let _ = tx.send(AssistantEvent::Done { stop_reason: state.stop_reason, usage: state.usage.clone() }).await;
        });

        channel_stream(rx)
    }
}
