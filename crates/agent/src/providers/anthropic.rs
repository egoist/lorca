//! Anthropic Messages API streaming (`/v1/messages`). Anthropic with an API key, and DeepSeek
//! through its Anthropic-compatible endpoint, which is the one that runs DeepSeek's web search
//! on the server (its OpenAI-compatible endpoint takes only `function` tools).

use std::collections::HashMap;

use async_trait::async_trait;
use futures::StreamExt;
use serde_json::{json, Value};
use tokio::sync::mpsc;
use tokio_util::sync::CancellationToken;

use crate::provider::{
    channel_stream, AssistantEvent, AssistantEventStream, ModelRequest, Provider, WEB_FETCH_TOOL, WEB_SEARCH_TOOL,
};
use crate::sse::SseParser;
use crate::types::{AssistantPart, ContentPart, LlmMessage, StopReason, Usage};

pub const ANTHROPIC_BASE_URL: &str = "https://api.anthropic.com";
pub const ANTHROPIC_DEFAULT_MODEL: &str = "claude-opus-5";
pub const ANTHROPIC_VERSION: &str = "2023-06-01";
/// DeepSeek's Anthropic-compatible endpoint.
pub const DEEPSEEK_ANTHROPIC_BASE_URL: &str = "https://api.deepseek.com/anthropic";
pub const DEEPSEEK_DEFAULT_MODEL: &str = super::openai_compat::DEEPSEEK_DEFAULT_MODEL;

pub struct AnthropicProvider {
    pub provider_id: String,
    pub base_url: String,
    pub api_key: String,
    pub model: String,
    /// Tools the server runs itself (web search, page reads), declared ahead of the request's
    /// function tools. Their calls and results stream back as server-tool events and as
    /// [`AssistantPart::ServerBlock`]s.
    pub server_tools: Vec<Value>,
    /// The request's `thinking` field, when set.
    pub thinking: Option<Value>,
    pub max_tokens: u64,
    client: reqwest::Client,
}

impl AnthropicProvider {
    /// A bare adapter: no server tools, the server's default thinking.
    pub fn new(provider_id: &str, base_url: &str, api_key: &str, model: &str) -> Self {
        AnthropicProvider {
            provider_id: provider_id.to_string(),
            base_url: base_url.trim_end_matches('/').to_string(),
            api_key: api_key.to_string(),
            model: model.to_string(),
            server_tools: Vec::new(),
            thinking: None,
            max_tokens: 16384,
            client: reqwest::Client::new(),
        }
    }

    /// Anthropic with an API key: `claude-opus-5` unless told otherwise, adaptive thinking, and
    /// web search and web fetch on Anthropic's side.
    pub fn anthropic(api_key: &str, model: Option<&str>) -> Self {
        let model = model.unwrap_or(ANTHROPIC_DEFAULT_MODEL);
        let (search, fetch) = if legacy_model(model) {
            ("web_search_20250305", "web_fetch_20250910")
        } else {
            ("web_search_20260209", "web_fetch_20260209")
        };
        let mut provider = Self::new("anthropic", ANTHROPIC_BASE_URL, api_key, model);
        provider.server_tools = vec![json!({ "type": search, "name": "web_search" }), json!({ "type": fetch, "name": "web_fetch" })];
        provider.thinking = (!legacy_model(model)).then(|| json!({ "type": "adaptive" }));
        provider.max_tokens = 32000;
        provider
    }

    /// DeepSeek through its Anthropic-compatible endpoint, with its server-side web search.
    pub fn deepseek(api_key: &str, model: Option<&str>) -> Self {
        let mut provider = Self::new("deepseek", DEEPSEEK_ANTHROPIC_BASE_URL, api_key, model.unwrap_or(DEEPSEEK_DEFAULT_MODEL));
        provider.server_tools = vec![json!({ "type": "web_search_20250305", "name": "web_search" })];
        provider
    }

    pub fn with_base_url(mut self, base_url: &str) -> Self {
        self.base_url = base_url.trim_end_matches('/').to_string();
        self
    }

    fn body(&self, request: &ModelRequest) -> Value {
        let mut messages: Vec<Value> = Vec::new();
        for message in &request.messages {
            let Some((role, blocks)) = self.convert_message(message) else { continue };
            // The API reads consecutive same-role messages as one turn; merged here so a tool
            // result always sits in the message right after its call.
            match messages.last_mut() {
                Some(last) if last["role"] == role => {
                    if let Some(content) = last["content"].as_array_mut() {
                        content.extend(blocks);
                    }
                }
                _ => messages.push(json!({ "role": role, "content": blocks })),
            }
        }

        let mut body = json!({
            "model": self.model,
            "max_tokens": self.max_tokens,
            "stream": true,
            "messages": messages,
        });
        if !request.system_prompt.trim().is_empty() {
            body["system"] = Value::String(request.system_prompt.clone());
        }
        let mut tools = self.server_tools.clone();
        tools.extend(request.tools.iter().map(|tool| {
            json!({ "name": tool.name, "description": tool.description, "input_schema": tool.parameters })
        }));
        if !tools.is_empty() {
            body["tools"] = Value::Array(tools);
        }
        if let Some(thinking) = &self.thinking {
            body["thinking"] = thinking.clone();
        }
        body
    }

    /// One transcript message as a role and its content blocks; `None` when nothing is left
    /// to send (the API rejects empty content).
    fn convert_message(&self, message: &LlmMessage) -> Option<(&'static str, Vec<Value>)> {
        match message {
            LlmMessage::User(user) => {
                let blocks: Vec<Value> = user
                    .content
                    .iter()
                    .filter_map(|part| match part {
                        ContentPart::Text { text } if text.is_empty() => None,
                        ContentPart::Text { text } => Some(json!({ "type": "text", "text": text })),
                        ContentPart::Image { data, mime_type } => Some(json!({
                            "type": "image",
                            "source": { "type": "base64", "media_type": mime_type, "data": data },
                        })),
                    })
                    .collect();
                (!blocks.is_empty()).then_some(("user", blocks))
            }
            LlmMessage::Assistant(assistant) => {
                // Thinking seals and server blocks are this provider's own; another provider's
                // (or a rebuilt transcript's, with no provider) are left out.
                let own = assistant.provider == self.provider_id;
                let mut blocks = Vec::new();
                for part in &assistant.content {
                    match part {
                        AssistantPart::Text { text } if !text.trim().is_empty() => {
                            blocks.push(json!({ "type": "text", "text": text }));
                        }
                        AssistantPart::Text { .. } => {}
                        AssistantPart::Thinking { thinking, signature: Some(signature) } if own => {
                            blocks.push(json!({ "type": "thinking", "thinking": thinking, "signature": signature }));
                        }
                        AssistantPart::Thinking { .. } => {}
                        AssistantPart::ToolCall(call) => {
                            let input = if call.arguments.is_object() { call.arguments.clone() } else { json!({}) };
                            blocks.push(json!({ "type": "tool_use", "id": call.id, "name": call.name, "input": input }));
                        }
                        AssistantPart::ServerBlock { block } if own => blocks.push(block.clone()),
                        AssistantPart::ServerBlock { .. } => {}
                    }
                }
                (!blocks.is_empty()).then_some(("assistant", blocks))
            }
            LlmMessage::ToolResult(result) => {
                let mut block = json!({ "type": "tool_result", "tool_use_id": result.tool_call_id, "is_error": result.is_error });
                let text = result.text();
                if !text.is_empty() {
                    block["content"] = Value::String(text);
                }
                Some(("user", vec![block]))
            }
        }
    }
}

/// Models before adaptive thinking and the filtering web tools: Haiku 4.5 and the 4.5 and
/// earlier generations.
fn legacy_model(model: &str) -> bool {
    let model = model.to_ascii_lowercase();
    model.contains("haiku") || model.contains("-4-5") || model.contains("-4-1") || model.contains("-4-0") || model.contains("claude-3")
}

/// What a content block of the stream is, on this side.
enum Block {
    Text,
    Thinking,
    ToolCall,
    /// A server tool call; its `input` arrives as JSON deltas.
    ServerToolUse { id: String, name: String, input: String },
    /// Any other block the provider owns, kept verbatim.
    Server(Value),
}

/// Tracks the stream's content blocks and the server tool calls awaiting their result.
struct MessagesState {
    next_index: usize,
    blocks: HashMap<u64, (usize, Block)>,
    /// Server tool calls by id: the tool name and detail reported at their start.
    pending: HashMap<String, (String, String)>,
    stop_reason: StopReason,
    usage: Usage,
}

impl MessagesState {
    fn new() -> Self {
        MessagesState { next_index: 0, blocks: HashMap::new(), pending: HashMap::new(), stop_reason: StopReason::Stop, usage: Usage::default() }
    }

    fn read_usage(&mut self, usage: &Value) {
        if let Some(input) = usage["input_tokens"].as_u64() {
            self.usage.input = input;
        }
        if let Some(output) = usage["output_tokens"].as_u64() {
            self.usage.output = output;
        }
        if let Some(read) = usage["cache_read_input_tokens"].as_u64() {
            self.usage.cache_read = read;
        }
        if let Some(write) = usage["cache_creation_input_tokens"].as_u64() {
            self.usage.cache_write = write;
        }
        self.usage.total_tokens = self.usage.input + self.usage.output + self.usage.cache_read + self.usage.cache_write;
    }

    /// Applies one event. `Ok(true)` when the message is complete.
    async fn apply(&mut self, kind: &str, value: &Value, tx: &mpsc::Sender<AssistantEvent>) -> Result<bool, String> {
        match kind {
            "message_start" => self.read_usage(&value["message"]["usage"]),
            "content_block_start" => {
                let provider_index = value["index"].as_u64().unwrap_or(0);
                let block = &value["content_block"];
                let index = self.next_index;
                self.next_index += 1;
                let kind = match block["type"].as_str().unwrap_or("") {
                    "text" => {
                        let _ = tx.send(AssistantEvent::TextStart { index }).await;
                        if let Some(text) = block["text"].as_str().filter(|t| !t.is_empty()) {
                            let _ = tx.send(AssistantEvent::TextDelta { index, delta: text.to_string() }).await;
                        }
                        Block::Text
                    }
                    "thinking" => {
                        let _ = tx.send(AssistantEvent::ThinkingStart { index }).await;
                        if let Some(text) = block["thinking"].as_str().filter(|t| !t.is_empty()) {
                            let _ = tx.send(AssistantEvent::ThinkingDelta { index, delta: text.to_string() }).await;
                        }
                        if let Some(signature) = block["signature"].as_str().filter(|s| !s.is_empty()) {
                            let _ = tx.send(AssistantEvent::ThinkingSignature { index, signature: signature.to_string() }).await;
                        }
                        Block::Thinking
                    }
                    "tool_use" => {
                        let id = block["id"].as_str().unwrap_or("").to_string();
                        let name = block["name"].as_str().unwrap_or("").to_string();
                        let _ = tx.send(AssistantEvent::ToolCallStart { index, id, name }).await;
                        Block::ToolCall
                    }
                    "server_tool_use" => {
                        let id = block["id"].as_str().unwrap_or("").to_string();
                        let name = block["name"].as_str().unwrap_or("").to_string();
                        let placeholder = json!({ "type": "server_tool_use", "id": id, "name": name, "input": {} });
                        let _ = tx.send(AssistantEvent::ServerBlock { index, block: placeholder }).await;
                        Block::ServerToolUse { id, name, input: String::new() }
                    }
                    _ => {
                        let _ = tx.send(AssistantEvent::ServerBlock { index, block: block.clone() }).await;
                        Block::Server(block.clone())
                    }
                };
                self.blocks.insert(provider_index, (index, kind));
            }
            "content_block_delta" => {
                let provider_index = value["index"].as_u64().unwrap_or(0);
                let Some((index, block)) = self.blocks.get_mut(&provider_index) else { return Ok(false) };
                let index = *index;
                let delta = &value["delta"];
                match delta["type"].as_str().unwrap_or("") {
                    "text_delta" => {
                        if let Some(text) = delta["text"].as_str() {
                            let _ = tx.send(AssistantEvent::TextDelta { index, delta: text.to_string() }).await;
                        }
                    }
                    "thinking_delta" => {
                        if let Some(text) = delta["thinking"].as_str() {
                            let _ = tx.send(AssistantEvent::ThinkingDelta { index, delta: text.to_string() }).await;
                        }
                    }
                    "signature_delta" => {
                        if let Some(signature) = delta["signature"].as_str().filter(|s| !s.is_empty()) {
                            let _ = tx.send(AssistantEvent::ThinkingSignature { index, signature: signature.to_string() }).await;
                        }
                    }
                    "input_json_delta" => {
                        let partial = delta["partial_json"].as_str().unwrap_or("");
                        match block {
                            Block::ToolCall if !partial.is_empty() => {
                                let _ = tx.send(AssistantEvent::ToolCallDelta { index, delta: partial.to_string() }).await;
                            }
                            Block::ServerToolUse { input, .. } => input.push_str(partial),
                            _ => {}
                        }
                    }
                    _ => {}
                }
            }
            "content_block_stop" => {
                let provider_index = value["index"].as_u64().unwrap_or(0);
                let Some((index, block)) = self.blocks.remove(&provider_index) else { return Ok(false) };
                match block {
                    Block::Text => {
                        let _ = tx.send(AssistantEvent::TextEnd { index }).await;
                    }
                    Block::Thinking => {
                        let _ = tx.send(AssistantEvent::ThinkingEnd { index }).await;
                    }
                    Block::ToolCall => {
                        let _ = tx.send(AssistantEvent::ToolCallEnd { index }).await;
                    }
                    Block::ServerToolUse { id, name, input } => {
                        let input: Value = serde_json::from_str(input.trim()).unwrap_or_else(|_| json!({}));
                        let (tool, detail) = server_tool_call(&name, &input);
                        let block = json!({ "type": "server_tool_use", "id": id, "name": name, "input": input });
                        let _ = tx.send(AssistantEvent::ServerBlock { index, block }).await;
                        let _ = tx.send(AssistantEvent::ServerToolStart { id: id.clone(), name: tool.clone(), detail: detail.clone() }).await;
                        self.pending.insert(id, (tool, detail));
                    }
                    Block::Server(block) => {
                        let is_result = block["type"].as_str().is_some_and(|t| t.ends_with("_tool_result"));
                        let Some(id) = block["tool_use_id"].as_str().filter(|_| is_result) else { return Ok(false) };
                        if let Some((name, detail)) = self.pending.remove(id) {
                            let summary = server_tool_summary(&name, &detail, &block);
                            let _ = tx.send(AssistantEvent::ServerToolEnd { id: id.to_string(), name, detail, summary }).await;
                        }
                    }
                }
            }
            "message_delta" => {
                self.read_usage(&value["usage"]);
                if let Some(reason) = value["delta"]["stop_reason"].as_str() {
                    self.stop_reason = match reason {
                        "max_tokens" => StopReason::Length,
                        "tool_use" => StopReason::ToolUse,
                        _ => StopReason::Stop,
                    };
                }
            }
            "message_stop" => return Ok(true),
            "error" => {
                let message = value["error"]["message"].as_str().unwrap_or("Provider error").to_string();
                return Err(message);
            }
            _ => {}
        }
        Ok(false)
    }
}

/// The shared tool name and the detail to show for a server tool call: the query of a search,
/// the URL of a page read. Any other server tool keeps its name and shows its input.
fn server_tool_call(name: &str, input: &Value) -> (String, String) {
    match name {
        "web_search" => (WEB_SEARCH_TOOL.into(), input["query"].as_str().unwrap_or("").trim().to_string()),
        "web_fetch" => (WEB_FETCH_TOOL.into(), input["url"].as_str().unwrap_or("").trim().to_string()),
        _ => (name.to_string(), if input.as_object().is_some_and(|o| o.is_empty()) { String::new() } else { input.to_string() }),
    }
}

/// One line for the finished row. A result whose content is an error object (Anthropic) or a
/// list holding an error entry (DeepSeek) names the error code.
fn server_tool_summary(name: &str, detail: &str, result: &Value) -> String {
    let content = &result["content"];
    let error = content["error_code"]
        .as_str()
        .or_else(|| content.as_array().and_then(|items| items.iter().find_map(|item| item["error_code"].as_str())))
        .map(|code| code.replace('_', " "));
    match (name, error) {
        (WEB_SEARCH_TOOL, Some(error)) => format!("Web search failed: {error}"),
        (WEB_SEARCH_TOOL, None) if detail.is_empty() => "Searched the web".into(),
        (WEB_SEARCH_TOOL, None) => format!("Searched the web for “{detail}”"),
        (WEB_FETCH_TOOL, Some(error)) => format!("Could not read {detail}: {error}"),
        (WEB_FETCH_TOOL, None) => format!("Read {detail}"),
        (_, Some(error)) => format!("{name} failed: {error}"),
        (_, None) => format!("Ran {name}"),
    }
}

#[async_trait]
impl Provider for AnthropicProvider {
    fn provider_id(&self) -> &str {
        &self.provider_id
    }

    fn model_id(&self) -> &str {
        &self.model
    }

    async fn stream(&self, request: ModelRequest, cancel: CancellationToken) -> AssistantEventStream {
        let (tx, rx) = mpsc::channel(64);
        let body = self.body(&request);
        let url = format!("{}/v1/messages", self.base_url);
        let client = self.client.clone();
        let api_key = self.api_key.clone();

        tokio::spawn(async move {
            let send = client
                .post(&url)
                .header("x-api-key", &api_key)
                .header("anthropic-version", ANTHROPIC_VERSION)
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
                let _ = tx
                    .send(AssistantEvent::Error { message: format!("{status}: {}", summarize_error(&text)), aborted: false })
                    .await;
                return;
            }

            let _ = tx.send(AssistantEvent::Start).await;
            let mut parser = SseParser::new();
            let mut state = MessagesState::new();
            let mut bytes = response.bytes_stream();

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
                    let Ok(value) = serde_json::from_str::<Value>(&event.data) else {
                        tracing::debug!(data = %event.data, "unparsed sse chunk");
                        continue;
                    };
                    let kind = value["type"].as_str().map(str::to_string).or(event.event.clone()).unwrap_or_default();
                    match state.apply(&kind, &value, &tx).await {
                        Ok(true) => break 'outer,
                        Ok(false) => {}
                        Err(message) => {
                            let _ = tx.send(AssistantEvent::Error { message, aborted: false }).await;
                            return;
                        }
                    }
                }
            }

            // Blocks still open when the stream ends are closed so the message is whole.
            let mut open: Vec<(u64, (usize, Block))> = state.blocks.drain().collect();
            open.sort_by_key(|(provider_index, _)| *provider_index);
            for (_, (index, block)) in open {
                match block {
                    Block::Text => {
                        let _ = tx.send(AssistantEvent::TextEnd { index }).await;
                    }
                    Block::Thinking => {
                        let _ = tx.send(AssistantEvent::ThinkingEnd { index }).await;
                    }
                    Block::ToolCall => {
                        let _ = tx.send(AssistantEvent::ToolCallEnd { index }).await;
                    }
                    Block::ServerToolUse { .. } | Block::Server(_) => {}
                }
            }
            let _ = tx.send(AssistantEvent::Done { stop_reason: state.stop_reason, usage: state.usage.clone() }).await;
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
    if trimmed.chars().count() > 300 {
        format!("{}…", trimmed.chars().take(300).collect::<String>())
    } else {
        trimmed.to_string()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::provider::{AssistantAccumulator, ToolSpec};
    use crate::types::{AssistantMessage, ToolCall, ToolResultMessage, UserMessage};

    fn request(messages: Vec<LlmMessage>) -> ModelRequest {
        ModelRequest {
            system_prompt: "be brief".into(),
            messages,
            tools: vec![ToolSpec { name: "read".into(), description: "read a file".into(), parameters: json!({ "type": "object" }) }],
        }
    }

    #[test]
    fn deepseek_declares_its_web_search_before_the_functions() {
        let provider = AnthropicProvider::deepseek("k", None);
        let body = provider.body(&request(vec![LlmMessage::User(UserMessage::text("hi"))]));
        assert_eq!(body["model"], "deepseek-flash");
        assert_eq!(body["system"], "be brief");
        assert!(body.get("thinking").is_none());
        let tools = body["tools"].as_array().unwrap();
        assert_eq!(tools[0], json!({ "type": "web_search_20250305", "name": "web_search" }));
        assert_eq!(tools[1]["name"], "read");
        assert_eq!(tools[1]["input_schema"], json!({ "type": "object" }));
        assert_eq!(body["messages"], json!([{ "role": "user", "content": [{ "type": "text", "text": "hi" }] }]));
    }

    #[test]
    fn anthropic_declares_search_fetch_and_adaptive_thinking() {
        let provider = AnthropicProvider::anthropic("k", None);
        let body = provider.body(&request(vec![LlmMessage::User(UserMessage::text("hi"))]));
        assert_eq!(body["model"], "claude-opus-5");
        assert_eq!(body["thinking"], json!({ "type": "adaptive" }));
        let tools = body["tools"].as_array().unwrap();
        assert_eq!(tools[0]["type"], "web_search_20260209");
        assert_eq!(tools[1]["type"], "web_fetch_20260209");
        assert_eq!(tools[2]["name"], "read");

        let haiku = AnthropicProvider::anthropic("k", Some("claude-haiku-4-5"));
        assert!(haiku.thinking.is_none());
        assert_eq!(haiku.server_tools[0]["type"], "web_search_20250305");
    }

    #[test]
    fn a_turn_replays_its_own_seals_and_server_blocks_and_merges_roles() {
        let provider = AnthropicProvider::deepseek("k", None);
        let mut own = AssistantMessage::empty("deepseek", "deepseek-flash");
        own.content = vec![
            AssistantPart::Thinking { thinking: "hm".into(), signature: Some("sig".into()) },
            AssistantPart::ServerBlock { block: json!({ "type": "server_tool_use", "id": "s1", "name": "web_search", "input": { "query": "q" } }) },
            AssistantPart::ServerBlock { block: json!({ "type": "web_search_tool_result", "tool_use_id": "s1", "content": [] }) },
            AssistantPart::Text { text: "Let me read it.".into() },
            AssistantPart::ToolCall(ToolCall { id: "t1".into(), name: "read".into(), arguments: json!({ "path": "a" }) }),
        ];
        // A rebuilt transcript message: no provider, thinking without a seal, a foreign block.
        let mut rebuilt = AssistantMessage::empty("", "");
        rebuilt.content = vec![
            AssistantPart::Thinking { thinking: "old".into(), signature: None },
            AssistantPart::ServerBlock { block: json!({ "type": "web_search_tool_result" }) },
            AssistantPart::Text { text: "Earlier.".into() },
        ];
        let result = ToolResultMessage {
            tool_call_id: "t1".into(),
            tool_name: "read".into(),
            content: vec![ContentPart::text("contents")],
            details: Value::Null,
            is_error: false,
            timestamp: 0,
        };
        let body = provider.body(&request(vec![
            LlmMessage::User(UserMessage::text("hi")),
            LlmMessage::Assistant(rebuilt),
            LlmMessage::Assistant(own),
            LlmMessage::ToolResult(result),
            LlmMessage::User(UserMessage::text("and then")),
        ]));
        let messages = body["messages"].as_array().unwrap();
        assert_eq!(messages.len(), 3, "user, merged assistant, merged user");
        let assistant = messages[1]["content"].as_array().unwrap();
        assert_eq!(assistant[0], json!({ "type": "text", "text": "Earlier." }));
        assert_eq!(assistant[1], json!({ "type": "thinking", "thinking": "hm", "signature": "sig" }));
        assert_eq!(assistant[2]["type"], "server_tool_use");
        assert_eq!(assistant[3]["type"], "web_search_tool_result");
        assert_eq!(assistant[4], json!({ "type": "text", "text": "Let me read it." }));
        assert_eq!(assistant[5], json!({ "type": "tool_use", "id": "t1", "name": "read", "input": { "path": "a" } }));
        let user = messages[2]["content"].as_array().unwrap();
        assert_eq!(user[0], json!({ "type": "tool_result", "tool_use_id": "t1", "is_error": false, "content": "contents" }));
        assert_eq!(user[1], json!({ "type": "text", "text": "and then" }));
    }

    async fn drive(events: &[(&str, Value)]) -> (Vec<AssistantEvent>, MessagesState) {
        let (tx, mut rx) = mpsc::channel(256);
        let mut state = MessagesState::new();
        for (kind, value) in events {
            let done = state.apply(kind, value, &tx).await.unwrap();
            assert_eq!(done, *kind == "message_stop");
        }
        drop(tx);
        let mut out = Vec::new();
        while let Some(event) = rx.recv().await {
            out.push(event);
        }
        (out, state)
    }

    /// The shape DeepSeek's endpoint streams for a search followed by a reply.
    fn search_stream() -> Vec<(&'static str, Value)> {
        vec![
            ("message_start", json!({ "message": { "usage": { "input_tokens": 341, "cache_read_input_tokens": 0, "output_tokens": 1 } } })),
            ("content_block_start", json!({ "index": 0, "content_block": { "type": "thinking", "thinking": "", "signature": "" } })),
            ("content_block_delta", json!({ "index": 0, "delta": { "type": "thinking_delta", "thinking": "I should search." } })),
            ("content_block_delta", json!({ "index": 0, "delta": { "type": "signature_delta", "signature": "msg-1" } })),
            ("content_block_stop", json!({ "index": 0 })),
            ("content_block_start", json!({ "index": 1, "content_block": { "type": "server_tool_use", "id": "call_00", "name": "web_search", "input": {} } })),
            ("content_block_delta", json!({ "index": 1, "delta": { "type": "input_json_delta", "partial_json": "{\"query\": \"tiny" } })),
            ("content_block_delta", json!({ "index": 1, "delta": { "type": "input_json_delta", "partial_json": "bot relay\"}" } })),
            ("content_block_stop", json!({ "index": 1 })),
            ("content_block_start", json!({ "index": 2, "content_block": { "type": "web_search_tool_result", "tool_use_id": "call_00", "content": [
                { "type": "web_search_result", "title": "Tinybot", "url": "https://example.com", "encrypted_content": "xx", "page_age": null }
            ] } })),
            ("content_block_stop", json!({ "index": 2 })),
            ("content_block_start", json!({ "index": 3, "content_block": { "type": "text", "text": "" } })),
            ("content_block_delta", json!({ "index": 3, "delta": { "type": "text_delta", "text": "Found it." } })),
            ("content_block_stop", json!({ "index": 3 })),
            ("message_delta", json!({ "delta": { "stop_reason": "end_turn" }, "usage": { "input_tokens": 3200, "output_tokens": 40, "server_tool_use": { "web_search_requests": 1 } } })),
            ("message_stop", json!({})),
        ]
    }

    #[tokio::test]
    async fn a_search_streams_as_server_tool_events_and_stays_in_the_message() {
        let (events, state) = drive(&search_stream()).await;
        let starts: Vec<_> = events.iter().filter(|e| matches!(e, AssistantEvent::ServerToolStart { .. })).collect();
        assert_eq!(starts.len(), 1);
        assert!(matches!(starts[0], AssistantEvent::ServerToolStart { id, name, detail } if id == "call_00" && name == WEB_SEARCH_TOOL && detail == "tinybot relay"));
        assert!(events.iter().any(|e| matches!(e, AssistantEvent::ServerToolEnd { id, name, detail, summary }
            if id == "call_00" && name == WEB_SEARCH_TOOL && detail == "tinybot relay" && summary == "Searched the web for “tinybot relay”")));
        assert_eq!(state.stop_reason, StopReason::Stop);
        assert_eq!((state.usage.input, state.usage.output), (3200, 40));

        let mut acc = AssistantAccumulator::new("deepseek", "deepseek-flash");
        for event in &events {
            acc.apply(event);
        }
        let message = acc.finish(false);
        assert_eq!(message.content.len(), 4);
        assert!(matches!(&message.content[0], AssistantPart::Thinking { thinking, signature: Some(s) } if thinking == "I should search." && s == "msg-1"));
        assert!(matches!(&message.content[1], AssistantPart::ServerBlock { block } if block["input"]["query"] == "tinybot relay"));
        assert!(matches!(&message.content[2], AssistantPart::ServerBlock { block } if block["type"] == "web_search_tool_result"));
        assert!(matches!(&message.content[3], AssistantPart::Text { text } if text == "Found it."));
        assert_eq!(message.text(), "Found it.");
    }

    #[tokio::test]
    async fn a_tool_call_after_a_search_keeps_its_index_and_stop_reason() {
        let mut events = search_stream();
        events.truncate(events.len() - 2);
        events.extend([
            ("content_block_start", json!({ "index": 4, "content_block": { "type": "tool_use", "id": "t1", "name": "read", "input": {} } })),
            ("content_block_delta", json!({ "index": 4, "delta": { "type": "input_json_delta", "partial_json": "{\"path\":\"a\"}" } })),
            ("content_block_stop", json!({ "index": 4 })),
            ("message_delta", json!({ "delta": { "stop_reason": "tool_use" }, "usage": { "output_tokens": 60 } })),
            ("message_stop", json!({})),
        ]);
        let (events, state) = drive(&events).await;
        assert_eq!(state.stop_reason, StopReason::ToolUse);
        let mut acc = AssistantAccumulator::new("deepseek", "deepseek-flash");
        for event in &events {
            acc.apply(event);
        }
        let message = acc.finish(false);
        let calls = message.tool_calls();
        assert_eq!(calls.len(), 1);
        assert_eq!((calls[0].id.as_str(), calls[0].arguments.clone()), ("t1", json!({ "path": "a" })));
        assert!(matches!(&message.content[4], AssistantPart::ToolCall(_)));
    }

    #[tokio::test]
    async fn a_failed_search_and_a_page_read_are_summarized() {
        let events = vec![
            ("content_block_start", json!({ "index": 0, "content_block": { "type": "server_tool_use", "id": "s1", "name": "web_search", "input": {} } })),
            ("content_block_stop", json!({ "index": 0 })),
            ("content_block_start", json!({ "index": 1, "content_block": { "type": "web_search_tool_result", "tool_use_id": "s1", "content": { "type": "web_search_tool_result_error", "error_code": "max_uses_exceeded" } } })),
            ("content_block_stop", json!({ "index": 1 })),
            ("content_block_start", json!({ "index": 2, "content_block": { "type": "server_tool_use", "id": "f1", "name": "web_fetch", "input": {} } })),
            ("content_block_delta", json!({ "index": 2, "delta": { "type": "input_json_delta", "partial_json": "{\"url\":\"https://x.dev\"}" } })),
            ("content_block_stop", json!({ "index": 2 })),
            ("content_block_start", json!({ "index": 3, "content_block": { "type": "web_fetch_tool_result", "tool_use_id": "f1", "content": { "type": "web_fetch_result", "url": "https://x.dev" } } })),
            ("content_block_stop", json!({ "index": 3 })),
        ];
        let (events, _) = drive(&events).await;
        let ends: Vec<_> = events.iter().filter_map(|e| match e {
            AssistantEvent::ServerToolEnd { name, detail, summary, .. } => Some((name.as_str(), detail.as_str(), summary.as_str())),
            _ => None,
        }).collect();
        assert_eq!(ends, vec![(WEB_SEARCH_TOOL, "", "Web search failed: max uses exceeded"), (WEB_FETCH_TOOL, "https://x.dev", "Read https://x.dev")]);
    }

    #[tokio::test]
    async fn an_error_event_fails_the_stream() {
        let (tx, _rx) = mpsc::channel(8);
        let mut state = MessagesState::new();
        let error = json!({ "type": "error", "error": { "type": "overloaded_error", "message": "Overloaded" } });
        assert_eq!(state.apply("error", &error, &tx).await, Err("Overloaded".into()));
    }

    /// Runs against DeepSeek's endpoint: `DEEPSEEK_API_KEY=… cargo test -p tinybot-agent
    /// live_deepseek -- --ignored --nocapture`. A search, then a function call the turn
    /// continues from with the seals and server blocks replayed.
    #[tokio::test]
    #[ignore]
    async fn live_deepseek_search_then_tool_call_replays() {
        let Ok(key) = std::env::var("DEEPSEEK_API_KEY") else { return };
        let provider = AnthropicProvider::deepseek(&key, None);
        let tools = vec![ToolSpec {
            name: "save_note".into(),
            description: "Saves a note for the user. Call it once you have the answer.".into(),
            parameters: json!({ "type": "object", "properties": { "text": { "type": "string" } }, "required": ["text"] }),
        }];
        let first = ModelRequest {
            system_prompt: "Search the web before answering questions about current software versions. When you know the answer, call save_note with one line, then say done.".into(),
            messages: vec![LlmMessage::User(UserMessage::text("What is the latest stable Rust release? Save the version as a note."))],
            tools: tools.clone(),
        };
        let mut stream = provider.stream(first, CancellationToken::new()).await;
        let mut acc = AssistantAccumulator::new("deepseek", "deepseek-flash");
        let mut searched = false;
        while let Some(event) = stream.next().await {
            if let AssistantEvent::ServerToolEnd { summary, .. } = &event {
                eprintln!("server tool: {summary}");
                searched = true;
            }
            acc.apply(&event);
        }
        let message = acc.finish(false);
        eprintln!("first turn: {:?} {:?}", message.stop_reason, message.error_message);
        assert!(searched, "the model searched");
        assert!(message.content.iter().any(|p| matches!(p, AssistantPart::ServerBlock { .. })));
        let calls = message.tool_calls();
        assert_eq!(calls.len(), 1, "one save_note call: {:?}", message.content);

        let result = ToolResultMessage {
            tool_call_id: calls[0].id.clone(),
            tool_name: "save_note".into(),
            content: vec![ContentPart::text("Saved.")],
            details: Value::Null,
            is_error: false,
            timestamp: 0,
        };
        let second = ModelRequest {
            system_prompt: "Search the web before answering questions about current software versions. When you know the answer, call save_note with one line, then say done.".into(),
            messages: vec![
                LlmMessage::User(UserMessage::text("What is the latest stable Rust release? Save the version as a note.")),
                LlmMessage::Assistant(message),
                LlmMessage::ToolResult(result),
            ],
            tools,
        };
        let mut stream = provider.stream(second, CancellationToken::new()).await;
        let mut acc = AssistantAccumulator::new("deepseek", "deepseek-flash");
        while let Some(event) = stream.next().await {
            acc.apply(&event);
        }
        let reply = acc.finish(false);
        eprintln!("second turn: {:?} {:?} {}", reply.stop_reason, reply.error_message, reply.text());
        assert_eq!(reply.stop_reason, StopReason::Stop);
    }
}
