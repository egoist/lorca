//! Anthropic Messages API streaming (`/v1/messages`). Anthropic with an API key, and DeepSeek
//! through its Anthropic-compatible endpoint, which is the one that runs DeepSeek's web search
//! on the server (its OpenAI-compatible endpoint takes only `function` tools).

use std::collections::HashMap;

use async_trait::async_trait;
use futures::StreamExt;
use serde_json::{json, Map, Value};
use tokio::sync::mpsc;
use tokio_util::sync::CancellationToken;

use crate::models::{self, ModelInfo, ThinkingMode};
use crate::provider::{
    channel_stream, AssistantEvent, AssistantEventStream, ModelRequest, Provider, WEB_FETCH_TOOL, WEB_SEARCH_TOOL,
};
use crate::retry::{send_with_retry, RequestFailure, DEFAULT_MAX_RETRY_DELAY_MS};
use crate::sse::SseParser;
use crate::transform::{transform_messages, TransformOptions};
use crate::types::{AssistantPart, ContentPart, LlmMessage, StopReason, ThinkingLevel, Usage};

pub const ANTHROPIC_BASE_URL: &str = "https://api.anthropic.com";
pub const ANTHROPIC_DEFAULT_MODEL: &str = "claude-opus-5";
pub const ANTHROPIC_VERSION: &str = "2023-06-01";
/// DeepSeek's Anthropic-compatible endpoint.
pub const DEEPSEEK_ANTHROPIC_BASE_URL: &str = "https://api.deepseek.com/anthropic";
pub const DEEPSEEK_DEFAULT_MODEL: &str = super::openai_compat::DEEPSEEK_DEFAULT_MODEL;

const USER_AGENT: &str = concat!("lorca-agent/", env!("CARGO_PKG_VERSION"));

pub struct AnthropicProvider {
    pub provider_id: String,
    pub base_url: String,
    pub api_key: String,
    pub model: String,
    /// Tools the server runs itself (web search, page reads), declared ahead of the request's
    /// function tools. Their calls and results stream back as server-tool events and as
    /// [`AssistantPart::ServerBlock`]s.
    pub server_tools: Vec<Value>,
    /// The request's `thinking` field when no level is set.
    pub thinking: Option<Value>,
    /// How much the model should think. `None` leaves it to `thinking`; a level is mapped the
    /// way the catalog says the model takes it (an effort, a token budget, or off).
    pub thinking_level: Option<ThinkingLevel>,
    /// The catalog entry for the model, when it has one: rates for cost, the window, levels.
    pub info: Option<&'static ModelInfo>,
    pub max_tokens: u64,
    /// Whether the model takes images; a text-only model gets a note in their place.
    pub supports_images: bool,
    /// `cache_control: ephemeral` on the system prompt, the last tool, and the last user block,
    /// so the conversation prefix is cached between turns.
    pub cache: bool,
    /// `eager_input_streaming` on function tools: arguments stream as they are generated
    /// instead of arriving once the server has buffered them.
    pub eager_tool_streaming: bool,
    /// Retries of a request that fails before it streams (408, 409, 429, 5xx, transport).
    pub max_retries: u32,
    /// A server-requested wait above this fails the request instead.
    pub max_retry_delay_ms: u64,
    client: reqwest::Client,
}

impl AnthropicProvider {
    /// A bare adapter: no server tools, the server's default thinking, images accepted.
    pub fn new(provider_id: &str, base_url: &str, api_key: &str, model: &str) -> Self {
        AnthropicProvider {
            provider_id: provider_id.to_string(),
            base_url: base_url.trim_end_matches('/').to_string(),
            api_key: api_key.to_string(),
            model: model.to_string(),
            server_tools: Vec::new(),
            thinking: None,
            thinking_level: None,
            info: models::find(provider_id, model),
            max_tokens: 16384,
            supports_images: true,
            cache: true,
            eager_tool_streaming: true,
            max_retries: 2,
            max_retry_delay_ms: DEFAULT_MAX_RETRY_DELAY_MS,
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
    /// Only the vision models take images.
    pub fn deepseek(api_key: &str, model: Option<&str>) -> Self {
        let model = model.unwrap_or(DEEPSEEK_DEFAULT_MODEL);
        let mut provider = Self::new("deepseek", DEEPSEEK_ANTHROPIC_BASE_URL, api_key, model);
        provider.server_tools = vec![json!({ "type": "web_search_20250305", "name": "web_search" })];
        let lower = model.to_ascii_lowercase();
        provider.supports_images = provider.info.map(|i| i.images).unwrap_or(lower.contains("vl") || lower.contains("vision"));
        provider
    }

    pub fn with_base_url(mut self, base_url: &str) -> Self {
        self.base_url = base_url.trim_end_matches('/').to_string();
        self
    }

    pub fn with_thinking(mut self, level: Option<ThinkingLevel>) -> Self {
        self.thinking_level = level;
        self
    }

    /// The `thinking` and `output_config` fields for the level, and the output cap that leaves
    /// room for a token budget.
    fn thinking_fields(&self, max_tokens: u64) -> (Option<Value>, Option<Value>, u64) {
        let Some(level) = self.thinking_level else { return (self.thinking.clone(), None, max_tokens) };
        let mode = self.info.map(|i| i.thinking).unwrap_or(if legacy_model(&self.model) { ThinkingMode::Budget } else { ThinkingMode::Adaptive });
        let level = match self.info {
            Some(info) => match info.clamp_level(level) {
                Some(level) => level,
                None => return (self.thinking.clone(), None, max_tokens),
            },
            None => level,
        };
        match (mode, level) {
            (ThinkingMode::Budget, ThinkingLevel::Off) => (None, None, max_tokens),
            (ThinkingMode::Budget, level) => {
                let budget = thinking_budget(level);
                (Some(json!({ "type": "enabled", "budget_tokens": budget })), None, max_tokens.max(budget + 1024))
            }
            (_, ThinkingLevel::Off) => (Some(json!({ "type": "disabled" })), None, max_tokens),
            (_, level) => (Some(json!({ "type": "adaptive" })), Some(json!({ "effort": effort_word(level) })), max_tokens),
        }
    }

    fn cache_control(&self) -> Option<Value> {
        self.cache.then(|| json!({ "type": "ephemeral" }))
    }

    fn body(&self, request: &ModelRequest) -> Value {
        let transformed = transform_messages(
            &request.messages,
            &TransformOptions {
                provider: &self.provider_id,
                model: &self.model,
                supports_images: self.supports_images,
                normalize_tool_call_id: Some(normalize_tool_call_id),
            },
        );
        let cache_control = self.cache_control();

        let mut messages: Vec<Value> = Vec::new();
        for message in &transformed {
            let Some((role, blocks)) = convert_message(message) else { continue };
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
        // The last user block carries the cache marker, so the whole conversation so far is
        // the cached prefix of the next turn.
        if let (Some(cache_control), Some(last)) = (&cache_control, messages.last_mut()) {
            if last["role"] == "user" {
                if let Some(block) = last["content"].as_array_mut().and_then(|blocks| blocks.last_mut()) {
                    if matches!(block["type"].as_str(), Some("text" | "image" | "tool_result")) {
                        block["cache_control"] = cache_control.clone();
                    }
                }
            }
        }

        let requested = request.max_tokens.unwrap_or(self.max_tokens);
        let (thinking, output_config, max_tokens) = self.thinking_fields(requested);
        let max_tokens = match self.info.map(|i| i.max_output).filter(|cap| *cap > 0) {
            Some(cap) => max_tokens.min(cap),
            None => max_tokens,
        };
        let mut body = json!({
            "model": self.model,
            "max_tokens": max_tokens,
            "stream": true,
            "messages": messages,
        });
        if let Some(output_config) = output_config {
            body["output_config"] = output_config;
        }
        if !request.system_prompt.trim().is_empty() {
            body["system"] = match &cache_control {
                Some(cache_control) => json!([{ "type": "text", "text": request.system_prompt, "cache_control": cache_control }]),
                None => Value::String(request.system_prompt.clone()),
            };
        }
        let mut tools = self.server_tools.clone();
        let function_count = request.tools.len();
        tools.extend(request.tools.iter().enumerate().map(|(index, tool)| {
            let mut spec = json!({ "name": tool.name, "description": tool.description, "input_schema": tool.parameters });
            if self.eager_tool_streaming {
                spec["eager_input_streaming"] = Value::Bool(true);
            }
            if let (Some(cache_control), true) = (&cache_control, index + 1 == function_count) {
                spec["cache_control"] = cache_control.clone();
            }
            spec
        }));
        if !tools.is_empty() {
            body["tools"] = Value::Array(tools);
        }
        if let Some(thinking) = thinking {
            body["thinking"] = thinking;
        }
        if let Some(user_id) = request.options.metadata.get("user_id").and_then(Value::as_str) {
            body["metadata"] = json!({ "user_id": user_id });
        }
        body
    }
}

/// The effort word for a level on an adaptive model.
fn effort_word(level: ThinkingLevel) -> &'static str {
    match level {
        ThinkingLevel::Off | ThinkingLevel::Minimal | ThinkingLevel::Low => "low",
        ThinkingLevel::Medium => "medium",
        ThinkingLevel::High => "high",
        ThinkingLevel::XHigh => "xhigh",
        ThinkingLevel::Max => "max",
    }
}

/// The token budget for a level on a model that thinks by budget.
fn thinking_budget(level: ThinkingLevel) -> u64 {
    match level {
        ThinkingLevel::Off | ThinkingLevel::Minimal => 1024,
        ThinkingLevel::Low => 2048,
        ThinkingLevel::Medium => 8192,
        ThinkingLevel::High | ThinkingLevel::XHigh | ThinkingLevel::Max => 16384,
    }
}

/// One transcript message as a role and its content blocks; `None` when nothing is left to
/// send (the API rejects empty content). The transcript has already been through
/// [`transform_messages`], so seals and server blocks here are this provider's own.
fn convert_message(message: &LlmMessage) -> Option<(&'static str, Vec<Value>)> {
    match message {
        LlmMessage::User(user) => {
            let blocks: Vec<Value> = user
                .content
                .iter()
                .filter_map(|part| match part {
                    ContentPart::Text { text } if text.trim().is_empty() => None,
                    ContentPart::Text { text } => Some(json!({ "type": "text", "text": text })),
                    ContentPart::Image { data, mime_type } => Some(image_block(data, mime_type)),
                })
                .collect();
            (!blocks.is_empty()).then_some(("user", blocks))
        }
        LlmMessage::Assistant(assistant) => {
            let mut blocks = Vec::new();
            for part in &assistant.content {
                match part {
                    AssistantPart::Text { text } if !text.trim().is_empty() => {
                        blocks.push(json!({ "type": "text", "text": text }));
                    }
                    AssistantPart::Text { .. } => {}
                    AssistantPart::Thinking { thinking, signature } => match signature.as_deref().filter(|s| !s.trim().is_empty()) {
                        Some(signature) => blocks.push(json!({ "type": "thinking", "thinking": thinking, "signature": signature })),
                        // Thinking without a seal (a stream that was cut off) goes back as text.
                        None if !thinking.trim().is_empty() => blocks.push(json!({ "type": "text", "text": thinking })),
                        None => {}
                    },
                    AssistantPart::ToolCall(call) => {
                        let input = if call.arguments.is_object() { call.arguments.clone() } else { json!({}) };
                        blocks.push(json!({ "type": "tool_use", "id": call.id, "name": call.name, "input": input }));
                    }
                    AssistantPart::ServerBlock { block } => blocks.push(block.clone()),
                }
            }
            (!blocks.is_empty()).then_some(("assistant", blocks))
        }
        LlmMessage::ToolResult(result) => {
            let has_images = result.content.iter().any(|part| matches!(part, ContentPart::Image { .. }));
            let content = if has_images {
                let mut blocks: Vec<Value> = result
                    .content
                    .iter()
                    .map(|part| match part {
                        ContentPart::Text { text } => json!({ "type": "text", "text": text }),
                        ContentPart::Image { data, mime_type } => image_block(data, mime_type),
                    })
                    .collect();
                if !blocks.iter().any(|b| b["type"] == "text") {
                    blocks.insert(0, json!({ "type": "text", "text": "(see attached image)" }));
                }
                Value::Array(blocks)
            } else {
                Value::String(result.text())
            };
            let block = json!({ "type": "tool_result", "tool_use_id": result.tool_call_id, "content": content, "is_error": result.is_error });
            Some(("user", vec![block]))
        }
    }
}

fn image_block(data: &str, mime_type: &str) -> Value {
    json!({ "type": "image", "source": { "type": "base64", "media_type": mime_type, "data": data } })
}

/// Tool call ids must match `^[a-zA-Z0-9_-]+$` and be at most 64 characters.
fn normalize_tool_call_id(id: &str) -> String {
    id.chars().map(|c| if c.is_ascii_alphanumeric() || c == '_' || c == '-' { c } else { '_' }).take(64).collect()
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
    /// From `message_delta`; a stream that ends without one is an error.
    stop: Option<Result<StopReason, String>>,
    usage: Usage,
}

impl MessagesState {
    fn new() -> Self {
        MessagesState { next_index: 0, blocks: HashMap::new(), pending: HashMap::new(), stop: None, usage: Usage::default() }
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
        if let Some(thinking) = usage["output_tokens_details"]["thinking_tokens"].as_u64() {
            self.usage.reasoning = Some(thinking);
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
                if block["type"] == "fallback" {
                    // The server moved to a fallback model. Fine before any output; after some,
                    // the message would be two models' work.
                    if self.next_index > 0 {
                        return Err("Anthropic performed an unsupported mid-output model fallback".into());
                    }
                    return Ok(false);
                }
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
                        let input = crate::json::parse_streaming_json(&input);
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
                    self.stop = Some(map_stop_reason(reason, &value["delta"]["stop_details"]));
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

/// The loop's stop reason for the API's. A refusal is an error carrying the server's
/// explanation; a reason this adapter does not know is one too, rather than a silent stop.
fn map_stop_reason(reason: &str, details: &Value) -> Result<StopReason, String> {
    match reason {
        "end_turn" | "stop_sequence" | "pause_turn" => Ok(StopReason::Stop),
        "max_tokens" => Ok(StopReason::Length),
        "tool_use" => Ok(StopReason::ToolUse),
        "refusal" => Err(details["explanation"].as_str().filter(|s| !s.is_empty()).map(str::to_string).unwrap_or_else(|| "The model refused to complete the request".into())),
        "sensitive" => Err("Provider stopped with: sensitive".into()),
        other => Err(format!("Unhandled stop reason: {other}")),
    }
}

/// The shared tool name and the detail to show for a server tool call: the query of a search,
/// the URL of a page read. Any other server tool keeps its name and shows its input.
fn server_tool_call(name: &str, input: &Value) -> (String, String) {
    match name {
        "web_search" => (WEB_SEARCH_TOOL.into(), input["query"].as_str().unwrap_or("").trim().to_string()),
        "web_fetch" => (WEB_FETCH_TOOL.into(), input["url"].as_str().unwrap_or("").trim().to_string()),
        _ => (name.to_string(), if input.as_object().is_some_and(Map::is_empty) { String::new() } else { input.to_string() }),
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

    fn supports_images(&self) -> bool {
        self.supports_images
    }

    fn model_info(&self) -> Option<&'static ModelInfo> {
        self.info
    }

    async fn stream(&self, request: ModelRequest, cancel: CancellationToken) -> AssistantEventStream {
        let (tx, rx) = mpsc::channel(64);
        let mut body = self.body(&request);
        let options = request.options.clone();
        options.before_payload(&mut body);
        let api_key = options.api_key(&self.api_key).await;
        let url = format!("{}/v1/messages", self.base_url);
        let client = self.client.clone();
        let (max_retries, max_retry_delay_ms) = (self.max_retries, self.max_retry_delay_ms);
        let info = self.info;

        tokio::spawn(async move {
            let build = || {
                let request = client
                    .post(&url)
                    .header("x-api-key", &api_key)
                    .header("anthropic-version", ANTHROPIC_VERSION)
                    .header("Accept", "text/event-stream")
                    .header("User-Agent", USER_AGENT);
                options.apply_to(request).json(&body)
            };
            let response = match send_with_retry(build, max_retries, max_retry_delay_ms, &cancel).await {
                Ok(response) => {
                    options.report(&response);
                    response
                }
                Err(failure) => {
                    let aborted = matches!(failure, RequestFailure::Aborted);
                    let _ = tx.send(AssistantEvent::Error { message: failure.message(), aborted }).await;
                    return;
                }
            };

            let _ = tx.send(AssistantEvent::Start).await;
            let mut parser = SseParser::new();
            let mut state = MessagesState::new();
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
                    let Ok(value) = crate::json::parse_json_with_repair(&event.data) else {
                        tracing::debug!(data = %event.data, "unparsed sse chunk");
                        continue;
                    };
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
            let mut usage = state.usage.clone();
            if let Some(info) = info {
                usage.cost = info.cost_of(&usage);
            }
            let terminal = match (completed, state.stop.take()) {
                (true, Some(Ok(stop_reason))) => AssistantEvent::Done { stop_reason, usage },
                (true, Some(Err(message))) => AssistantEvent::Error { message, aborted: false },
                (true, None) => AssistantEvent::Error { message: "Anthropic stream ended without a stop reason".into(), aborted: false },
                (false, _) => AssistantEvent::Error { message: "Anthropic stream ended before message_stop".into(), aborted: false },
            };
            let _ = tx.send(terminal).await;
        });

        channel_stream(rx)
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
            max_tokens: None,
            options: Default::default(),
        }
    }

    #[test]
    fn deepseek_declares_its_web_search_before_the_functions_and_caches_the_prefix() {
        let provider = AnthropicProvider::deepseek("k", None);
        let body = provider.body(&request(vec![LlmMessage::User(UserMessage::text("hi"))]));
        assert_eq!(body["model"], "deepseek-flash");
        assert_eq!(body["system"], json!([{ "type": "text", "text": "be brief", "cache_control": { "type": "ephemeral" } }]));
        assert!(body.get("thinking").is_none());
        let tools = body["tools"].as_array().unwrap();
        assert_eq!(tools[0], json!({ "type": "web_search_20250305", "name": "web_search" }));
        assert_eq!(tools[1]["name"], "read");
        assert_eq!(tools[1]["input_schema"], json!({ "type": "object" }));
        assert_eq!(tools[1]["eager_input_streaming"], true);
        assert_eq!(tools[1]["cache_control"], json!({ "type": "ephemeral" }));
        assert_eq!(
            body["messages"],
            json!([{ "role": "user", "content": [{ "type": "text", "text": "hi", "cache_control": { "type": "ephemeral" } }] }])
        );
        assert!(provider.supports_images(), "the catalog says Flash takes images");
        assert!(!AnthropicProvider::deepseek("k", Some("deepseek-v4-pro")).supports_images());
        assert_eq!(provider.model_info().map(|i| i.context_window), Some(1_000_000));
    }

    #[test]
    fn a_thinking_level_is_mapped_to_what_the_model_takes() {
        let body = |provider: AnthropicProvider| provider.body(&request(vec![LlmMessage::User(UserMessage::text("hi"))]));
        let flash = body(AnthropicProvider::deepseek("k", None).with_thinking(Some(ThinkingLevel::High)));
        assert_eq!(flash["thinking"], json!({ "type": "adaptive" }));
        assert_eq!(flash["output_config"], json!({ "effort": "high" }));
        let off = body(AnthropicProvider::deepseek("k", None).with_thinking(Some(ThinkingLevel::Off)));
        assert_eq!(off["thinking"], json!({ "type": "disabled" }));
        assert!(off.get("output_config").is_none());
        // Fable 5.1 and Opus 5.5 cannot stop thinking: Off becomes the lowest level they have.
        for model in ["claude-fable-5-1", "claude-opus-5-5"] {
            let always = body(AnthropicProvider::anthropic("k", Some(model)).with_thinking(Some(ThinkingLevel::Off)));
            assert_eq!(always["thinking"], json!({ "type": "adaptive" }), "{model}");
            assert_eq!(always["output_config"], json!({ "effort": "low" }), "{model}");
        }
        // Haiku thinks by budget, with room left for the answer.
        let haiku = body(AnthropicProvider::anthropic("k", Some("claude-haiku-4-5")).with_thinking(Some(ThinkingLevel::Max)));
        assert_eq!(haiku["thinking"], json!({ "type": "enabled", "budget_tokens": 16384 }));
        assert_eq!(haiku["max_tokens"], 32000);
        let haiku_off = body(AnthropicProvider::anthropic("k", Some("claude-haiku-4-5")).with_thinking(Some(ThinkingLevel::Off)));
        assert!(haiku_off.get("thinking").is_none());
        // A per-request cap wins over the adapter's.
        let mut request_capped = request(vec![LlmMessage::User(UserMessage::text("hi"))]);
        request_capped.max_tokens = Some(500);
        assert_eq!(AnthropicProvider::deepseek("k", None).body(&request_capped)["max_tokens"], 500);
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
        assert!(provider.supports_images());

        let haiku = AnthropicProvider::anthropic("k", Some("claude-haiku-4-5"));
        assert!(haiku.thinking.is_none());
        assert_eq!(haiku.server_tools[0]["type"], "web_search_20250305");

        let mut plain = AnthropicProvider::new("proxy", "http://x", "k", "m");
        plain.cache = false;
        plain.eager_tool_streaming = false;
        let body = plain.body(&request(vec![LlmMessage::User(UserMessage::text("hi"))]));
        assert_eq!(body["system"], "be brief");
        assert!(body["tools"][0].get("eager_input_streaming").is_none());
        assert!(body["tools"][0].get("cache_control").is_none());
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
        assert_eq!(assistant[0], json!({ "type": "text", "text": "old" }), "foreign thinking is plain text");
        assert_eq!(assistant[1], json!({ "type": "text", "text": "Earlier." }));
        assert_eq!(assistant[2], json!({ "type": "thinking", "thinking": "hm", "signature": "sig" }));
        assert_eq!(assistant[3]["type"], "server_tool_use");
        assert_eq!(assistant[4]["type"], "web_search_tool_result");
        assert_eq!(assistant[5], json!({ "type": "text", "text": "Let me read it." }));
        assert_eq!(assistant[6], json!({ "type": "tool_use", "id": "t1", "name": "read", "input": { "path": "a" } }));
        let user = messages[2]["content"].as_array().unwrap();
        assert_eq!(user[0], json!({ "type": "tool_result", "tool_use_id": "t1", "is_error": false, "content": "contents" }));
        assert_eq!(user[1], json!({ "type": "text", "text": "and then", "cache_control": { "type": "ephemeral" } }));
    }

    #[test]
    fn a_failed_turn_is_left_out_and_an_orphaned_call_gets_a_result() {
        let provider = AnthropicProvider::deepseek("k", None);
        let mut failed = AssistantMessage::empty("deepseek", "deepseek-flash");
        failed.stop_reason = StopReason::Error;
        failed.content = vec![AssistantPart::Text { text: "half".into() }];
        let mut calls = AssistantMessage::empty("deepseek", "deepseek-flash");
        calls.content = vec![AssistantPart::ToolCall(ToolCall { id: "t1".into(), name: "read".into(), arguments: json!({}) })];
        let body = provider.body(&request(vec![
            LlmMessage::User(UserMessage::text("hi")),
            LlmMessage::Assistant(failed),
            LlmMessage::Assistant(calls),
            LlmMessage::User(UserMessage::text("next")),
        ]));
        let messages = body["messages"].as_array().unwrap();
        assert_eq!(messages.len(), 3);
        assert_eq!(messages[1]["content"][0]["type"], "tool_use");
        assert_eq!(messages[2]["content"][0]["type"], "tool_result");
        assert_eq!(messages[2]["content"][0]["content"], "No result provided");
        assert_eq!(messages[2]["content"][0]["is_error"], true);
        assert_eq!(messages[2]["content"][1]["text"], "next");
    }

    #[test]
    fn tool_results_carry_images_as_blocks_and_foreign_ids_are_normalized() {
        let provider = AnthropicProvider::anthropic("k", None);
        let mut calls = AssistantMessage::empty("chatgpt", "gpt");
        calls.content = vec![AssistantPart::ToolCall(ToolCall { id: "call|x".repeat(20), name: "read".into(), arguments: json!({}) })];
        let result = ToolResultMessage {
            tool_call_id: "call|x".repeat(20),
            tool_name: "read".into(),
            content: vec![ContentPart::Image { data: "AAAA".into(), mime_type: "image/png".into() }],
            details: Value::Null,
            is_error: false,
            timestamp: 0,
        };
        let body = provider.body(&request(vec![LlmMessage::Assistant(calls), LlmMessage::ToolResult(result)]));
        let id = body["messages"][0]["content"][0]["id"].as_str().unwrap();
        assert_eq!(id.len(), 64);
        assert!(id.starts_with("call_x"));
        let result = &body["messages"][1]["content"][0];
        assert_eq!(result["tool_use_id"], id);
        assert_eq!(result["content"][0], json!({ "type": "text", "text": "(see attached image)" }));
        assert_eq!(result["content"][1]["type"], "image");
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
            ("content_block_delta", json!({ "index": 1, "delta": { "type": "input_json_delta", "partial_json": "{\"query\": \"lor" } })),
            ("content_block_delta", json!({ "index": 1, "delta": { "type": "input_json_delta", "partial_json": "ca relay\"}" } })),
            ("content_block_stop", json!({ "index": 1 })),
            ("content_block_start", json!({ "index": 2, "content_block": { "type": "web_search_tool_result", "tool_use_id": "call_00", "content": [
                { "type": "web_search_result", "title": "Lorca", "url": "https://example.com", "encrypted_content": "xx", "page_age": null }
            ] } })),
            ("content_block_stop", json!({ "index": 2 })),
            ("content_block_start", json!({ "index": 3, "content_block": { "type": "text", "text": "" } })),
            ("content_block_delta", json!({ "index": 3, "delta": { "type": "text_delta", "text": "Found it." } })),
            ("content_block_stop", json!({ "index": 3 })),
            ("message_delta", json!({ "delta": { "stop_reason": "end_turn" }, "usage": { "input_tokens": 3200, "output_tokens": 40, "output_tokens_details": { "thinking_tokens": 12 }, "server_tool_use": { "web_search_requests": 1 } } })),
            ("message_stop", json!({})),
        ]
    }

    #[tokio::test]
    async fn a_search_streams_as_server_tool_events_and_stays_in_the_message() {
        let (events, state) = drive(&search_stream()).await;
        let starts: Vec<_> = events.iter().filter(|e| matches!(e, AssistantEvent::ServerToolStart { .. })).collect();
        assert_eq!(starts.len(), 1);
        assert!(matches!(starts[0], AssistantEvent::ServerToolStart { id, name, detail } if id == "call_00" && name == WEB_SEARCH_TOOL && detail == "lorca relay"));
        assert!(events.iter().any(|e| matches!(e, AssistantEvent::ServerToolEnd { id, name, detail, summary }
            if id == "call_00" && name == WEB_SEARCH_TOOL && detail == "lorca relay" && summary == "Searched the web for “lorca relay”")));
        assert_eq!(state.stop, Some(Ok(StopReason::Stop)));
        assert_eq!((state.usage.input, state.usage.output, state.usage.reasoning), (3200, 40, Some(12)));

        let mut acc = AssistantAccumulator::new("deepseek", "deepseek-flash");
        for event in &events {
            acc.apply(event);
        }
        let message = acc.finish(false);
        assert_eq!(message.content.len(), 4);
        assert!(matches!(&message.content[0], AssistantPart::Thinking { thinking, signature: Some(s) } if thinking == "I should search." && s == "msg-1"));
        assert!(matches!(&message.content[1], AssistantPart::ServerBlock { block } if block["input"]["query"] == "lorca relay"));
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
        assert_eq!(state.stop, Some(Ok(StopReason::ToolUse)));
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
    async fn refusals_unknown_reasons_and_error_events_fail_the_message() {
        let (tx, _rx) = mpsc::channel(8);
        let mut state = MessagesState::new();
        let error = json!({ "type": "error", "error": { "type": "overloaded_error", "message": "Overloaded" } });
        assert_eq!(state.apply("error", &error, &tx).await, Err("Overloaded".into()));

        let refusal = json!({ "delta": { "stop_reason": "refusal", "stop_details": { "type": "refusal", "explanation": "Not this." } }, "usage": {} });
        state.apply("message_delta", &refusal, &tx).await.unwrap();
        assert_eq!(state.stop, Some(Err("Not this.".into())));
        let unknown = json!({ "delta": { "stop_reason": "something_new" }, "usage": {} });
        state.apply("message_delta", &unknown, &tx).await.unwrap();
        assert_eq!(state.stop, Some(Err("Unhandled stop reason: something_new".into())));
        let paused = json!({ "delta": { "stop_reason": "pause_turn" }, "usage": {} });
        state.apply("message_delta", &paused, &tx).await.unwrap();
        assert_eq!(state.stop, Some(Ok(StopReason::Stop)));
    }

    /// Runs against DeepSeek's endpoint: `DEEPSEEK_API_KEY=… cargo test -p lorca-agent
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
            max_tokens: None,
            options: Default::default(),
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
        eprintln!("first turn: {:?} {:?} usage {:?}", message.stop_reason, message.error_message, message.usage);
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
            max_tokens: None,
            options: Default::default(),
        };
        let mut stream = provider.stream(second, CancellationToken::new()).await;
        let mut acc = AssistantAccumulator::new("deepseek", "deepseek-flash");
        while let Some(event) = stream.next().await {
            acc.apply(&event);
        }
        let reply = acc.finish(false);
        eprintln!("second turn: {:?} {:?} {} usage {:?}", reply.stop_reason, reply.error_message, reply.text(), reply.usage);
        assert_eq!(reply.stop_reason, StopReason::Stop);
    }
}
