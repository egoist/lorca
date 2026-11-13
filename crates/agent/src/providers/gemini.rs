//! Google's Gemini API, `streamGenerateContent` over SSE with an API key (`x-goog-api-key`):
//! Google's own, and OpenCode's, which serves Gemini on the same wire.
//!
//! Gemini seals its reasoning in `thoughtSignature`s on the parts it returns, and Gemini 3 and
//! later refuse a turn whose function calls come back without theirs. A signature travels in the
//! transcript as a [`AssistantPart::ServerBlock`] `{ "gemini_signature": … }` right after the
//! part it belongs to, and goes back on that part. On Gemini 3 and later every request declares
//! Google Search and URL context with tool context circulation on: the server's `toolCall` and
//! `toolResponse` parts stream back as server tool rows and stay in the message, verbatim, as
//! server blocks for the rest of the turn.

use std::collections::HashMap;

use async_trait::async_trait;
use futures::StreamExt;
use serde_json::{json, Map, Value};
use tokio::sync::mpsc;
use tokio_util::sync::CancellationToken;

use crate::models::{self, ModelInfo};
use crate::provider::{channel_stream, AssistantEvent, AssistantEventStream, ModelRequest, Provider, WEB_FETCH_TOOL, WEB_SEARCH_TOOL};
use crate::retry::{send_with_retry, RequestFailure, DEFAULT_MAX_RETRY_DELAY_MS};
use crate::sse::SseParser;
use crate::transform::{transform_messages, TransformOptions};
use crate::types::{AssistantPart, ContentPart, LlmMessage, StopReason, ThinkingLevel, Usage};

pub const GEMINI_BASE_URL: &str = "https://generativelanguage.googleapis.com";
pub const GEMINI_DEFAULT_MODEL: &str = "gemini-4-hydrogen";
/// What one streaming request returns at most; past it Gemini 4 asks for a continuation hop.
pub const GEMINI_STREAM_OUTPUT_CAP: u64 = 65_536;
/// The value Gemini takes in place of a signature on a function call it did not make (one from
/// another model, or from before a compaction), so validation lets the turn through.
const SKIP_SIGNATURE: &str = "skip_thought_signature_validator";
/// The key of the server block that carries a part's signature.
const SIGNATURE_KEY: &str = "gemini_signature";

const USER_AGENT: &str = concat!("lorca-agent/", env!("CARGO_PKG_VERSION"));

pub struct GeminiProvider {
    pub provider_id: String,
    /// Where `models/` hangs, its API version included:
    /// `https://generativelanguage.googleapis.com/v1beta`.
    pub api_url: String,
    pub api_key: String,
    pub model: String,
    pub supports_images: bool,
    pub max_retries: u32,
    pub max_retry_delay_ms: u64,
    /// Sent as `thinkingConfig.thinkingLevel`, held to the model's catalog levels. A model the
    /// catalog lacks thinks at its default.
    pub thinking_level: Option<ThinkingLevel>,
    pub info: Option<&'static ModelInfo>,
    /// Google Search and URL context, run on Google's side, with tool context circulation:
    /// Gemini 3 and later.
    pub server_tools: bool,
    client: reqwest::Client,
}

/// Whether the model is Gemini 3 or later, which combines built-in tools with function calls.
fn is_gemini_3_or_later(model: &str) -> bool {
    let rest = model.strip_prefix("models/").unwrap_or(model);
    let Some(version) = rest.strip_prefix("gemini-") else { return false };
    let major: String = version.chars().take_while(char::is_ascii_digit).collect();
    major.parse::<u32>().is_ok_and(|major| major >= 3)
}

impl GeminiProvider {
    /// Google's own API under `base_url`, the root without a version.
    pub fn new(base_url: &str, api_key: &str, model: Option<&str>) -> Self {
        let api_url = format!("{}/v1beta", base_url.trim_end_matches('/'));
        Self::at("gemini", &api_url, api_key, model.unwrap_or(GEMINI_DEFAULT_MODEL))
    }

    /// Another provider that serves Gemini's wire (OpenCode), with `models/` under `api_url`; the
    /// catalog entry is that provider's.
    pub fn at(provider_id: &str, api_url: &str, api_key: &str, model: &str) -> Self {
        let info = models::find(provider_id, model);
        GeminiProvider {
            provider_id: provider_id.to_string(),
            api_url: api_url.trim_end_matches('/').to_string(),
            api_key: api_key.to_string(),
            supports_images: info.map(|info| info.images).unwrap_or(true),
            max_retries: 2,
            max_retry_delay_ms: DEFAULT_MAX_RETRY_DELAY_MS,
            thinking_level: None,
            info,
            server_tools: is_gemini_3_or_later(model),
            model: model.to_string(),
            client: lorca_tls::client(),
        }
    }

    pub fn with_thinking(mut self, level: Option<ThinkingLevel>) -> Self {
        self.thinking_level = level;
        self
    }

    /// The reply cap of a request that names none: the model's, within one streaming hop.
    fn default_cap(&self) -> u64 {
        self.info.map(|info| info.max_output).filter(|cap| *cap > 0).unwrap_or(GEMINI_STREAM_OUTPUT_CAP).min(GEMINI_STREAM_OUTPUT_CAP)
    }

    fn url(&self) -> String {
        format!("{}/models/{}:streamGenerateContent?alt=sse", self.api_url, self.model.strip_prefix("models/").unwrap_or(&self.model))
    }

    fn body(&self, request: &ModelRequest) -> Value {
        let transformed = transform_messages(
            &request.messages,
            &TransformOptions { provider: &self.provider_id, model: &self.model, supports_images: self.supports_images, normalize_tool_call_id: None },
        );
        let mut body = json!({ "contents": convert_messages(&transformed) });
        if !request.system_prompt.trim().is_empty() {
            body["systemInstruction"] = json!({ "parts": [{ "text": request.system_prompt }] });
        }

        let cap = request.max_tokens.unwrap_or_else(|| self.default_cap()).min(GEMINI_STREAM_OUTPUT_CAP);
        let mut thinking = json!({ "includeThoughts": true });
        // Gemini 3 and later cannot stop thinking; a catalog model's levels never include `off`.
        if let Some(level) = self.thinking_level.and_then(|level| self.info.and_then(|info| info.clamp_level(level))) {
            if let Some(word) = level_word(level) {
                thinking["thinkingLevel"] = Value::String(word.into());
            }
        }
        body["generationConfig"] = json!({ "maxOutputTokens": cap, "thinkingConfig": thinking });

        let mut tools = Vec::new();
        if !request.tools.is_empty() {
            let declarations: Vec<Value> = request
                .tools
                .iter()
                .map(|tool| json!({ "name": tool.name, "description": tool.description, "parametersJsonSchema": tool.parameters }))
                .collect();
            tools.push(json!({ "functionDeclarations": declarations }));
        }
        if self.server_tools {
            tools.push(json!({ "googleSearch": {} }));
            tools.push(json!({ "urlContext": {} }));
            body["toolConfig"] = json!({ "includeServerSideToolInvocations": true });
        }
        if !tools.is_empty() {
            body["tools"] = Value::Array(tools);
        }
        body
    }
}

fn level_word(level: ThinkingLevel) -> Option<&'static str> {
    Some(match level {
        ThinkingLevel::Off => return None,
        ThinkingLevel::Minimal => "minimal",
        ThinkingLevel::Low => "low",
        ThinkingLevel::Medium => "medium",
        ThinkingLevel::High => "high",
        ThinkingLevel::XHigh => "xhigh",
        ThinkingLevel::Max => "max",
    })
}

/// The transcript as Gemini `contents`. Tool results answer their calls in one `user` content,
/// in the calls' order, with any images inside each response; a user message right after them
/// joins that content, since Gemini wants the roles to alternate.
fn convert_messages(messages: &[LlmMessage]) -> Vec<Value> {
    let mut contents: Vec<Value> = Vec::new();
    let mut push = |role: &str, parts: Vec<Value>| {
        if parts.is_empty() {
            return;
        }
        if let Some(last) = contents.last_mut().filter(|last| last["role"] == role && role == "user") {
            last["parts"].as_array_mut().expect("parts").extend(parts);
        } else {
            contents.push(json!({ "role": role, "parts": parts }));
        }
    };
    for message in messages {
        match message {
            LlmMessage::User(user) => push("user", user.content.iter().map(content_part).collect()),
            LlmMessage::Assistant(assistant) => push("model", model_parts(&assistant.content)),
            LlmMessage::ToolResult(result) => {
                let text = result.text();
                let images: Vec<Value> = result.content.iter().filter(|part| matches!(part, ContentPart::Image { .. })).map(content_part).collect();
                let response = if result.is_error {
                    json!({ "error": if text.is_empty() { "(no tool output)".to_string() } else { text } })
                } else if text.is_empty() && images.is_empty() {
                    json!({ "output": "(no tool output)" })
                } else {
                    json!({ "output": text })
                };
                let mut function_response = json!({ "id": result.tool_call_id, "name": result.tool_name, "response": response });
                if !images.is_empty() {
                    function_response["parts"] = Value::Array(images);
                }
                push("user", vec![json!({ "functionResponse": function_response })]);
            }
        }
    }
    contents
}

fn content_part(part: &ContentPart) -> Value {
    match part {
        ContentPart::Text { text } => json!({ "text": text }),
        ContentPart::Image { data, mime_type } => json!({ "inlineData": { "mimeType": mime_type, "data": data } }),
    }
}

/// An assistant message as the parts Gemini returned: each signature goes back on the part it
/// came after, the server's own parts go back as they came, and the first function call of the
/// message carries a signature, a stand-in when it has none of its own.
fn model_parts(content: &[AssistantPart]) -> Vec<Value> {
    let mut parts: Vec<Value> = Vec::new();
    let mut first_call: Option<usize> = None;
    for part in content {
        match part {
            AssistantPart::Text { text } => parts.push(json!({ "text": text })),
            AssistantPart::Thinking { thinking, .. } => parts.push(json!({ "text": thinking, "thought": true })),
            AssistantPart::ToolCall(call) => {
                first_call.get_or_insert(parts.len());
                parts.push(json!({ "functionCall": { "id": call.id, "name": call.name, "args": call.arguments } }));
            }
            AssistantPart::ServerBlock { block } => {
                if let Some(signature) = block[SIGNATURE_KEY].as_str() {
                    match parts.last_mut() {
                        Some(last) if last.get("thoughtSignature").is_none() => last["thoughtSignature"] = Value::String(signature.into()),
                        // A signature that came alone (an empty closing part) goes back alone.
                        _ => parts.push(json!({ "text": "", "thoughtSignature": signature })),
                    }
                } else if block.is_object() {
                    parts.push(block.clone());
                }
            }
        }
    }
    if let Some(index) = first_call {
        if parts[index].get("thoughtSignature").is_none() {
            parts[index]["thoughtSignature"] = Value::String(SKIP_SIGNATURE.into());
        }
    }
    // Empty text that seals nothing is not a part Gemini takes.
    parts.retain(|part| part["text"].as_str() != Some("") || part.get("thoughtSignature").is_some());
    parts
}

/// Which open block a streamed part continues.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Open {
    Text(usize),
    Thinking(usize),
}

#[derive(Default)]
struct StreamState {
    next_index: usize,
    open: Option<Open>,
    /// Server tool calls by id, with the row's name and detail, until their response arrives.
    server_tools: HashMap<String, (String, String)>,
    called_function: bool,
    stop_reason: Option<StopReason>,
    /// A stop that is the model refusing or failing, reported as the turn's error.
    failure: Option<String>,
    usage: Usage,
}

impl StreamState {
    fn take_index(&mut self) -> usize {
        let index = self.next_index;
        self.next_index += 1;
        index
    }

    async fn close(&mut self, tx: &mpsc::Sender<AssistantEvent>) {
        match self.open.take() {
            Some(Open::Text(index)) => {
                let _ = tx.send(AssistantEvent::TextEnd { index }).await;
            }
            Some(Open::Thinking(index)) => {
                let _ = tx.send(AssistantEvent::ThinkingEnd { index }).await;
            }
            None => {}
        }
    }

    async fn block(&mut self, block: Value, tx: &mpsc::Sender<AssistantEvent>) {
        self.close(tx).await;
        let index = self.take_index();
        let _ = tx.send(AssistantEvent::ServerBlock { index, block }).await;
    }

    async fn apply_chunk(&mut self, chunk: &Value, tx: &mpsc::Sender<AssistantEvent>) {
        if let Some(usage) = chunk.get("usageMetadata").filter(|u| u.is_object()) {
            let count = |key: &str| usage[key].as_u64().unwrap_or(0);
            // `promptTokenCount` counts the cached part too; `input` is the part read fresh.
            let cache_read = count("cachedContentTokenCount");
            let thoughts = count("thoughtsTokenCount");
            self.usage = Usage {
                input: (count("promptTokenCount") + count("toolUsePromptTokenCount")).saturating_sub(cache_read),
                output: count("candidatesTokenCount") + thoughts,
                cache_read,
                cache_write: 0,
                reasoning: usage.get("thoughtsTokenCount").map(|_| thoughts),
                total_tokens: count("totalTokenCount"),
                cost: Default::default(),
            };
        }
        if let Some(reason) = chunk["promptFeedback"]["blockReason"].as_str() {
            self.failure = Some(format!("Gemini blocked the prompt ({})", reason.to_ascii_lowercase().replace('_', " ")));
        }
        let Some(candidate) = chunk["candidates"].as_array().and_then(|c| c.first()) else { return };
        for part in candidate["content"]["parts"].as_array().into_iter().flatten() {
            self.apply_part(part, tx).await;
        }
        if let Some(reason) = candidate["finishReason"].as_str() {
            self.stop_reason = Some(match reason {
                "STOP" | "FINISH_REASON_UNSPECIFIED" => StopReason::Stop,
                "MAX_TOKENS" | "CONTINUATION" => StopReason::Length,
                other => {
                    let message = candidate["finishMessage"].as_str().map(str::to_string).unwrap_or_else(|| format!("Gemini stopped: {}", other.to_ascii_lowercase().replace('_', " ")));
                    self.failure = Some(message);
                    StopReason::Error
                }
            });
        }
    }

    async fn apply_part(&mut self, part: &Value, tx: &mpsc::Sender<AssistantEvent>) {
        let signature = part["thoughtSignature"].as_str().filter(|s| !s.is_empty());
        if let Some(call) = part.get("functionCall") {
            self.close(tx).await;
            self.called_function = true;
            let index = self.take_index();
            let id = call["id"].as_str().filter(|id| !id.is_empty()).map(str::to_string).unwrap_or_else(|| format!("call_{index}"));
            let name = call["name"].as_str().unwrap_or("").to_string();
            let arguments = call.get("args").filter(|args| args.is_object()).cloned().unwrap_or_else(|| Value::Object(Map::new()));
            let _ = tx.send(AssistantEvent::ToolCallStart { index, id, name }).await;
            let _ = tx.send(AssistantEvent::ToolCallDelta { index, delta: arguments.to_string() }).await;
            let _ = tx.send(AssistantEvent::ToolCallEnd { index }).await;
        } else if let Some(call) = part.get("toolCall") {
            let id = call["id"].as_str().unwrap_or("").to_string();
            let (name, detail) = server_tool(call);
            self.server_tools.insert(id.clone(), (name.clone(), detail.clone()));
            let _ = tx.send(AssistantEvent::ServerToolStart { id, name, detail }).await;
            self.block(part.clone(), tx).await;
            return;
        } else if let Some(response) = part.get("toolResponse") {
            let id = response["id"].as_str().unwrap_or("").to_string();
            let (name, detail) = self.server_tools.remove(&id).unwrap_or_else(|| server_tool(response));
            let summary = server_tool_summary(&name, &detail);
            let _ = tx.send(AssistantEvent::ServerToolEnd { id, name, detail, summary }).await;
            self.block(part.clone(), tx).await;
            return;
        } else if let Some(text) = part["text"].as_str() {
            let thought = part["thought"].as_bool().unwrap_or(false);
            if !text.is_empty() {
                let index = match (self.open, thought) {
                    (Some(Open::Thinking(index)), true) | (Some(Open::Text(index)), false) => index,
                    _ => {
                        self.close(tx).await;
                        let index = self.take_index();
                        if thought {
                            self.open = Some(Open::Thinking(index));
                            let _ = tx.send(AssistantEvent::ThinkingStart { index }).await;
                        } else {
                            self.open = Some(Open::Text(index));
                            let _ = tx.send(AssistantEvent::TextStart { index }).await;
                        }
                        index
                    }
                };
                let delta = text.to_string();
                let event = if thought { AssistantEvent::ThinkingDelta { index, delta } } else { AssistantEvent::TextDelta { index, delta } };
                let _ = tx.send(event).await;
            }
        } else if part.is_object() && !part.as_object().is_some_and(|p| p.keys().all(|k| k == "thoughtSignature")) {
            // Any other part of the model's own (code it ran, a file): kept for the turn as is.
            self.block(part.clone(), tx).await;
            return;
        }
        if let Some(signature) = signature {
            self.block(json!({ SIGNATURE_KEY: signature }), tx).await;
        }
    }
}

/// The row a built-in tool's call or response shows: Google Search's queries, or the pages URL
/// context reads.
fn server_tool(part: &Value) -> (String, String) {
    let tool_type = part["toolType"].as_str().unwrap_or("").to_ascii_uppercase();
    let list = |value: &Value| -> String {
        match value {
            Value::Array(items) => items.iter().filter_map(Value::as_str).map(str::trim).filter(|s| !s.is_empty()).collect::<Vec<_>>().join(", "),
            Value::String(text) => text.trim().to_string(),
            _ => String::new(),
        }
    };
    if tool_type.contains("URL") {
        (WEB_FETCH_TOOL.into(), list(&part["args"]["urls"]))
    } else if tool_type.contains("SEARCH") {
        (WEB_SEARCH_TOOL.into(), list(&part["args"]["queries"]))
    } else {
        let name = if tool_type.is_empty() { "google_tool".to_string() } else { tool_type.to_ascii_lowercase() };
        (name, String::new())
    }
}

fn server_tool_summary(name: &str, detail: &str) -> String {
    match name {
        WEB_SEARCH_TOOL if detail.is_empty() => "Searched the web".into(),
        WEB_SEARCH_TOOL => format!("Searched the web for “{detail}”"),
        WEB_FETCH_TOOL if detail.is_empty() => "Read a page".into(),
        WEB_FETCH_TOOL => format!("Read {detail}"),
        other => format!("Ran {other}"),
    }
}

#[async_trait]
impl Provider for GeminiProvider {
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

    fn default_max_output_tokens(&self) -> Option<u64> {
        Some(self.default_cap())
    }

    async fn stream(&self, request: ModelRequest, cancel: CancellationToken) -> AssistantEventStream {
        let (tx, rx) = mpsc::channel(64);
        let mut body = self.body(&request);
        let options = request.options.clone();
        options.before_payload(&mut body);
        let api_key = options.api_key(&self.api_key).await;
        let url = self.url();
        let client = self.client.clone();
        let (max_retries, max_retry_delay_ms) = (self.max_retries, self.max_retry_delay_ms);
        let info = self.info;

        tokio::spawn(async move {
            let build = || options.apply_to(client.post(&url).header("x-goog-api-key", &api_key).header("User-Agent", USER_AGENT)).json(&body);
            let response = match send_with_retry(build, max_retries, max_retry_delay_ms, &cancel, &options).await {
                Ok(response) => response,
                Err(failure) => {
                    let aborted = matches!(failure, RequestFailure::Aborted);
                    let _ = tx.send(AssistantEvent::Error { message: failure.message(), aborted }).await;
                    return;
                }
            };

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
                    match crate::json::parse_json_with_repair(&event.data) {
                        Ok(value) => {
                            if let Some(error) = value.get("error") {
                                let message = error["message"].as_str().unwrap_or("Gemini error").to_string();
                                let _ = tx.send(AssistantEvent::Error { message, aborted: false }).await;
                                return;
                            }
                            state.apply_chunk(&value, &tx).await;
                        }
                        Err(_) => tracing::debug!(data = %event.data, "unparsed sse chunk"),
                    }
                }
            }

            state.close(&tx).await;
            if let Some(message) = state.failure.take() {
                let _ = tx.send(AssistantEvent::Error { message, aborted: false }).await;
                return;
            }
            let stop_reason = match state.stop_reason {
                None => {
                    let _ = tx.send(AssistantEvent::Error { message: "Gemini's stream ended without a finish reason".into(), aborted: false }).await;
                    return;
                }
                Some(StopReason::Stop) if state.called_function => StopReason::ToolUse,
                Some(reason) => reason,
            };
            let mut usage = state.usage.clone();
            if let Some(info) = info {
                usage.cost = info.cost_of(&usage);
            }
            let _ = tx.send(AssistantEvent::Done { stop_reason, usage }).await;
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
        ModelRequest { system_prompt: "Be brief.".into(), messages, tools: Vec::new(), cache_points: Vec::new(), max_tokens: None, options: Default::default() }
    }

    async fn stream(chunks: &[Value]) -> (Vec<AssistantEvent>, StreamState) {
        let (tx, mut rx) = mpsc::channel(256);
        let mut state = StreamState::default();
        for chunk in chunks {
            state.apply_chunk(chunk, &tx).await;
        }
        state.close(&tx).await;
        drop(tx);
        let mut events = Vec::new();
        while let Some(event) = rx.recv().await {
            events.push(event);
        }
        (events, state)
    }

    fn message(events: &[AssistantEvent], model: &str) -> AssistantMessage {
        let mut accumulator = AssistantAccumulator::new("gemini", model);
        for event in events {
            accumulator.apply(event);
        }
        accumulator.message()
    }

    #[test]
    fn the_body_declares_tools_thinking_and_the_system_prompt() {
        let mut request = request(vec![LlmMessage::User(UserMessage::text("hi"))]);
        request.tools = vec![ToolSpec { name: "bash".into(), description: "Run a command".into(), parameters: json!({ "type": "object", "additionalProperties": false }) }];
        let provider = GeminiProvider::new(GEMINI_BASE_URL, "k", Some("gemini-3.8-flash")).with_thinking(Some(ThinkingLevel::XHigh));
        let body = provider.body(&request);
        assert_eq!(body["systemInstruction"], json!({ "parts": [{ "text": "Be brief." }] }));
        assert_eq!(body["contents"], json!([{ "role": "user", "parts": [{ "text": "hi" }] }]));
        assert_eq!(body["tools"][0]["functionDeclarations"][0]["parametersJsonSchema"]["additionalProperties"], false);
        assert_eq!(body["tools"][1], json!({ "googleSearch": {} }));
        assert_eq!(body["tools"][2], json!({ "urlContext": {} }));
        assert_eq!(body["toolConfig"], json!({ "includeServerSideToolInvocations": true }));
        assert_eq!(body["generationConfig"]["maxOutputTokens"], GEMINI_STREAM_OUTPUT_CAP);
        // 3.8 Flash stops at high; Hydrogen takes xhigh.
        assert_eq!(body["generationConfig"]["thinkingConfig"], json!({ "includeThoughts": true, "thinkingLevel": "high" }));
        let hydrogen = GeminiProvider::new(GEMINI_BASE_URL, "k", Some("gemini-4-hydrogen")).with_thinking(Some(ThinkingLevel::Max));
        assert_eq!(hydrogen.body(&request)["generationConfig"]["thinkingConfig"]["thinkingLevel"], "xhigh");
        // Off is the lowest level a model that cannot stop thinking has.
        let off = GeminiProvider::new(GEMINI_BASE_URL, "k", Some("gemini-3.5-flash-lite")).with_thinking(Some(ThinkingLevel::Off));
        assert_eq!(off.body(&request)["generationConfig"]["thinkingConfig"]["thinkingLevel"], "minimal");
        // An older model gets no built-in tools, and an unknown one no level.
        let old = GeminiProvider::new(GEMINI_BASE_URL, "k", Some("gemini-2.5-flash-lite")).with_thinking(Some(ThinkingLevel::High));
        let body = old.body(&request);
        assert_eq!(body["tools"].as_array().map(Vec::len), Some(1));
        assert!(body.get("toolConfig").is_none());
        assert_eq!(body["generationConfig"]["thinkingConfig"], json!({ "includeThoughts": true }));
        assert_eq!(old.url(), "https://generativelanguage.googleapis.com/v1beta/models/gemini-2.5-flash-lite:streamGenerateContent?alt=sse");

        // OpenCode serves the same wire under its own route, with its own catalog entry.
        let zen = GeminiProvider::at("opencode", "https://opencode.ai/zen/v1/", "k", "gemini-4-hydrogen");
        assert_eq!(zen.url(), "https://opencode.ai/zen/v1/models/gemini-4-hydrogen:streamGenerateContent?alt=sse");
        assert_eq!((zen.provider_id(), zen.info.map(|info| info.provider)), ("opencode", Some("opencode")));
        assert!(zen.server_tools);
    }

    #[tokio::test]
    async fn a_turn_with_search_and_a_function_call_goes_back_as_it_came() {
        let model = "gemini-3.8-flash";
        let (events, state) = stream(&[
            json!({ "candidates": [{ "content": { "role": "model", "parts": [{ "text": "Looking it up", "thought": true }] } }] }),
            json!({ "candidates": [{ "content": { "role": "model", "parts": [
                { "toolCall": { "toolType": "GOOGLE_SEARCH_WEB", "args": { "queries": ["northernmost US city"] }, "id": "s1" }, "thoughtSignature": "SIG_S" },
                { "toolResponse": { "toolType": "GOOGLE_SEARCH_WEB", "response": { "search_suggestions": "…" }, "id": "s1" }, "thoughtSignature": "SIG_R" },
                { "text": "Utqiaġvik. " }
            ] } }] }),
            json!({ "candidates": [{ "content": { "role": "model", "parts": [
                { "functionCall": { "id": "f1", "name": "weather", "args": { "city": "Utqiaġvik" } }, "thoughtSignature": "SIG_A" },
                { "functionCall": { "id": "f2", "name": "time", "args": {} } }
            ] }, "finishReason": "STOP" }], "usageMetadata": { "promptTokenCount": 100, "cachedContentTokenCount": 60, "candidatesTokenCount": 20, "thoughtsTokenCount": 30, "totalTokenCount": 150 } }),
        ])
        .await;
        assert!(events.iter().any(|e| matches!(e, AssistantEvent::ServerToolStart { name, detail, .. } if name == WEB_SEARCH_TOOL && detail == "northernmost US city")));
        assert!(events.iter().any(|e| matches!(e, AssistantEvent::ServerToolEnd { summary, .. } if summary == "Searched the web for “northernmost US city”")));
        assert!(state.called_function);
        assert_eq!((state.usage.input, state.usage.cache_read, state.usage.output, state.usage.reasoning), (40, 60, 50, Some(30)));

        let reply = message(&events, model);
        assert_eq!(reply.tool_calls().iter().map(|c| c.id.as_str()).collect::<Vec<_>>(), ["f1", "f2"]);
        assert_eq!(reply.text(), "Utqiaġvik. ");

        let results = ["f1", "f2"].map(|id| {
            LlmMessage::ToolResult(ToolResultMessage { tool_call_id: id.into(), tool_name: if id == "f1" { "weather" } else { "time" }.into(), content: vec![ContentPart::text("ok")], details: Value::Null, is_error: false, timestamp: 0 })
        });
        let mut transcript = vec![LlmMessage::User(UserMessage::text("weather in the northernmost city?")), LlmMessage::Assistant(reply)];
        transcript.extend(results);
        let body = GeminiProvider::new(GEMINI_BASE_URL, "k", Some(model)).body(&request(transcript));
        let contents = body["contents"].as_array().unwrap();
        assert_eq!(contents.len(), 3);
        assert_eq!(
            contents[1]["parts"],
            json!([
                { "text": "Looking it up", "thought": true },
                { "toolCall": { "toolType": "GOOGLE_SEARCH_WEB", "args": { "queries": ["northernmost US city"] }, "id": "s1" }, "thoughtSignature": "SIG_S" },
                { "toolResponse": { "toolType": "GOOGLE_SEARCH_WEB", "response": { "search_suggestions": "…" }, "id": "s1" }, "thoughtSignature": "SIG_R" },
                { "text": "Utqiaġvik. " },
                { "functionCall": { "id": "f1", "name": "weather", "args": { "city": "Utqiaġvik" } }, "thoughtSignature": "SIG_A" },
                { "functionCall": { "id": "f2", "name": "time", "args": {} } }
            ])
        );
        // Both answers in one user content, in the calls' order.
        assert_eq!(contents[2]["role"], "user");
        assert_eq!(contents[2]["parts"][0], json!({ "functionResponse": { "id": "f1", "name": "weather", "response": { "output": "ok" } } }));
        assert_eq!(contents[2]["parts"][1]["functionResponse"]["id"], "f2");
    }

    #[tokio::test]
    async fn a_closing_signature_stays_with_the_text_and_foreign_calls_get_a_stand_in() {
        let (events, _) = stream(&[
            json!({ "candidates": [{ "content": { "parts": [{ "text": "Done." }] } }] }),
            json!({ "candidates": [{ "content": { "parts": [{ "text": "", "thoughtSignature": "SIG_T" }] }, "finishReason": "STOP" }] }),
        ])
        .await;
        let reply = message(&events, "gemini-4-hydrogen");
        assert_eq!(model_parts(&reply.content), vec![json!({ "text": "Done.", "thoughtSignature": "SIG_T" })]);

        // A call another model made, now continued on Gemini.
        let mut foreign = AssistantMessage::empty("anthropic", "claude-opus-5-5");
        foreign.content = vec![AssistantPart::ToolCall(ToolCall { id: "toolu_1".into(), name: "read".into(), arguments: json!({ "path": "a.png" }) })];
        foreign.stop_reason = StopReason::ToolUse;
        let image = ContentPart::Image { data: "AAAA".into(), mime_type: "image/png".into() };
        let body = GeminiProvider::new(GEMINI_BASE_URL, "k", None).body(&request(vec![
            LlmMessage::User(UserMessage::text("look")),
            LlmMessage::Assistant(foreign),
            LlmMessage::ToolResult(ToolResultMessage { tool_call_id: "toolu_1".into(), tool_name: "read".into(), content: vec![image], details: Value::Null, is_error: false, timestamp: 0 }),
        ]));
        assert_eq!(body["contents"][1]["parts"][0]["thoughtSignature"], SKIP_SIGNATURE);
        assert_eq!(
            body["contents"][2]["parts"][0]["functionResponse"],
            json!({ "id": "toolu_1", "name": "read", "response": { "output": "" }, "parts": [{ "inlineData": { "mimeType": "image/png", "data": "AAAA" } }] })
        );
    }

    #[tokio::test]
    async fn a_refusal_is_an_error_and_a_cut_is_a_length_stop() {
        let (_, state) = stream(&[json!({ "candidates": [{ "finishReason": "SAFETY" }] })]).await;
        assert_eq!(state.failure.as_deref(), Some("Gemini stopped: safety"));
        let (_, state) = stream(&[json!({ "candidates": [{ "content": { "parts": [{ "text": "…" }] }, "finishReason": "MAX_TOKENS" }] })]).await;
        assert_eq!(state.stop_reason, Some(StopReason::Length));
        assert!(is_gemini_3_or_later("gemini-4-carbon") && is_gemini_3_or_later("gemini-3.5-flash-lite") && !is_gemini_3_or_later("gemini-2.5-pro"));
    }

    /// `GEMINI_API_KEY=… cargo test -p lorca-agent live_gemini -- --ignored --nocapture`
    #[tokio::test]
    #[ignore]
    async fn live_gemini() {
        let Ok(key) = std::env::var("GEMINI_API_KEY") else { return };
        let provider = GeminiProvider::new(GEMINI_BASE_URL, &key, std::env::var("GEMINI_MODEL").ok().as_deref());
        let mut request = request(vec![LlmMessage::User(UserMessage::text("What is the latest stable Rust release? Search, then call done with the version."))]);
        request.tools = vec![ToolSpec { name: "done".into(), description: "Report the version".into(), parameters: json!({ "type": "object", "properties": { "version": { "type": "string" } }, "required": ["version"] }) }];
        let mut events = provider.stream(request.clone(), CancellationToken::new()).await;
        let mut accumulator = AssistantAccumulator::new("gemini", &provider.model);
        while let Some(event) = events.next().await {
            if let AssistantEvent::ServerToolEnd { summary, .. } = &event {
                eprintln!("server tool: {summary}");
            }
            accumulator.apply(&event);
        }
        let reply = accumulator.message();
        eprintln!("{reply:#?}");
        assert_ne!(reply.stop_reason, StopReason::Error, "{:?}", reply.error_message);
        // Continue the turn with the call's result: the signatures must be accepted.
        if let Some(call) = reply.tool_calls().first().map(|c| (*c).clone()) {
            request.messages.push(LlmMessage::Assistant(reply));
            request.messages.push(LlmMessage::ToolResult(ToolResultMessage { tool_call_id: call.id, tool_name: call.name, content: vec![ContentPart::text("recorded")], details: Value::Null, is_error: false, timestamp: 0 }));
            let mut events = provider.stream(request, CancellationToken::new()).await;
            let mut accumulator = AssistantAccumulator::new("gemini", &provider.model);
            while let Some(event) = events.next().await {
                accumulator.apply(&event);
            }
            let reply = accumulator.message();
            eprintln!("{reply:#?}");
            assert_ne!(reply.stop_reason, StopReason::Error, "{:?}", reply.error_message);
        }
    }
}
