//! OpenAI-compatible `/chat/completions` streaming. DeepSeek uses this as is.

use std::collections::HashMap;

use async_trait::async_trait;
use futures::StreamExt;
use serde_json::{json, Value};
use tokio::sync::mpsc;
use tokio_util::sync::CancellationToken;

use crate::models::{self, ModelInfo};
use crate::provider::{channel_stream, AssistantEvent, AssistantEventStream, ModelRequest, Provider};
use crate::request::bearer_auth;
use crate::retry::{send_with_retry, RequestFailure, DEFAULT_MAX_RETRY_DELAY_MS};
use crate::sse::SseParser;
use crate::transform::{transform_messages, TransformOptions};
use crate::types::{AssistantPart, ContentPart, LlmMessage, StopReason, ThinkingLevel, Usage};

pub const DEEPSEEK_BASE_URL: &str = "https://api.deepseek.com";
/// DeepSeek V4.1 Flash. `deepseek-v4-pro` is the reasoning-heavy option; the legacy
/// `deepseek-chat` / `deepseek-reasoner` names were retired in 2026.
pub const DEEPSEEK_DEFAULT_MODEL: &str = "deepseek-flash";

const USER_AGENT: &str = concat!("lorca-agent/", env!("CARGO_PKG_VERSION"));

pub struct OpenAiCompatProvider {
    pub provider_id: String,
    pub base_url: String,
    pub api_key: String,
    pub model: String,
    /// Whether the model takes images; a text-only model gets a note in their place.
    pub supports_images: bool,
    /// Retries of a request that fails before it streams (408, 409, 429, 5xx, transport).
    pub max_retries: u32,
    pub max_retry_delay_ms: u64,
    /// Sent as `reasoning_effort`. `Off` is `thinking: { type: "disabled" }` on a model the
    /// catalog says can stop thinking, and sends nothing on any other.
    pub thinking_level: Option<ThinkingLevel>,
    /// The catalog entry for the model, when it has one.
    pub info: Option<&'static ModelInfo>,
    /// Sends the session id as `prompt_cache_key`, OpenAI's routing hint for its prompt cache.
    /// Off for a server that refuses fields it does not know.
    pub prompt_cache_key: bool,
    client: reqwest::Client,
}

impl OpenAiCompatProvider {
    pub fn new(provider_id: &str, base_url: &str, api_key: &str, model: &str) -> Self {
        OpenAiCompatProvider {
            provider_id: provider_id.to_string(),
            base_url: base_url.trim_end_matches('/').to_string(),
            api_key: api_key.to_string(),
            model: model.to_string(),
            supports_images: true,
            max_retries: 2,
            max_retry_delay_ms: DEFAULT_MAX_RETRY_DELAY_MS,
            thinking_level: None,
            info: models::find(provider_id, model),
            prompt_cache_key: true,
            client: lorca_tls::client(),
        }
    }

    pub fn with_thinking(mut self, level: Option<ThinkingLevel>) -> Self {
        self.thinking_level = level;
        self
    }

    pub fn deepseek(api_key: &str, model: Option<&str>) -> Self {
        Self::new("deepseek", DEEPSEEK_BASE_URL, api_key, model.unwrap_or(DEEPSEEK_DEFAULT_MODEL))
    }

    fn body(&self, request: &ModelRequest) -> Value {
        let transformed = transform_messages(
            &request.messages,
            &TransformOptions { provider: &self.provider_id, model: &self.model, supports_images: self.supports_images, normalize_tool_call_id: None },
        );
        let mut messages = Vec::new();
        if !request.system_prompt.trim().is_empty() {
            messages.push(json!({ "role": "system", "content": request.system_prompt }));
        }
        messages.extend(convert_messages(&transformed));

        let mut body = json!({
            "model": self.model,
            "messages": messages,
            "stream": true,
            "stream_options": { "include_usage": true },
        });
        if let Some(max_tokens) = request.max_tokens {
            body["max_tokens"] = Value::from(max_tokens);
        }
        if let Some(session_id) = request.options.session_id.as_ref().filter(|_| self.prompt_cache_key) {
            body["prompt_cache_key"] = Value::String(session_id.clone());
        }
        let level = self.thinking_level.and_then(|level| match self.info {
            Some(info) => info.clamp_level(level),
            None if level == ThinkingLevel::Off => None,
            None => Some(level),
        });
        // A catalogued model gets the word for its level, since its levels are the words it
        // takes; any other model gets at most `high`, which every gateway takes.
        let catalogued = self.info.is_some();
        let effort = match level {
            None => None,
            // Only a model the catalog says can stop thinking keeps `Off` through the clamp.
            Some(ThinkingLevel::Off) => {
                body["thinking"] = json!({ "type": "disabled" });
                None
            }
            Some(ThinkingLevel::Minimal) => Some("minimal"),
            Some(ThinkingLevel::Low) => Some("low"),
            Some(ThinkingLevel::Medium) => Some("medium"),
            Some(ThinkingLevel::XHigh) if catalogued => Some("xhigh"),
            Some(ThinkingLevel::Max) if catalogued => Some("max"),
            Some(ThinkingLevel::High | ThinkingLevel::XHigh | ThinkingLevel::Max) => Some("high"),
        };
        if let Some(effort) = effort {
            body["reasoning_effort"] = Value::String(effort.into());
        }
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

/// The transcript as request messages. A tool message cannot hold images, and nothing may come
/// between an assistant's tool calls and the tool messages that answer them, so a run of tool
/// results is all of its tool messages and then one user message with the images of the run.
fn convert_messages(messages: &[LlmMessage]) -> Vec<Value> {
    let mut out = Vec::new();
    for run in messages.chunk_by(|a, b| matches!((a, b), (LlmMessage::ToolResult(_), LlmMessage::ToolResult(_)))) {
        let mut images = Vec::new();
        for message in run {
            out.push(convert_message(message, &mut images));
        }
        if !images.is_empty() {
            out.push(json!({ "role": "user", "content": images }));
        }
    }
    out
}

/// One transcript message as the request message it becomes. A tool result's images go to
/// `images` instead, after a line naming the tool, for the user message that follows the run.
fn convert_message(message: &LlmMessage, images: &mut Vec<Value>) -> Value {
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
                        ContentPart::Image { data, mime_type } => image_part(data, mime_type),
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
        LlmMessage::ToolResult(result) => {
            let text = result.text();
            let mut parts: Vec<Value> = result
                .content
                .iter()
                .filter_map(|part| match part {
                    ContentPart::Image { data, mime_type } => Some(image_part(data, mime_type)),
                    ContentPart::Text { .. } => None,
                })
                .collect();
            let content = if !text.is_empty() {
                text
            } else if !parts.is_empty() {
                "(see attached image)".into()
            } else {
                "(no tool output)".into()
            };
            if !parts.is_empty() {
                images.push(json!({ "type": "text", "text": format!("Images from the {} tool result:", result.tool_name) }));
                images.append(&mut parts);
            }
            json!({ "role": "tool", "tool_call_id": result.tool_call_id, "content": content })
        }
    }
}

fn image_part(data: &str, mime_type: &str) -> Value {
    json!({ "type": "image_url", "image_url": { "url": format!("data:{mime_type};base64,{data}") } })
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
            // `prompt_tokens` counts the cached tokens too; `input` is the part read fresh.
            let cache_read = usage["prompt_cache_hit_tokens"]
                .as_u64()
                .or_else(|| usage["prompt_tokens_details"]["cached_tokens"].as_u64())
                .unwrap_or(0);
            self.usage = Usage {
                input: usage["prompt_tokens"].as_u64().unwrap_or(0).saturating_sub(cache_read),
                output: usage["completion_tokens"].as_u64().unwrap_or(0),
                cache_read,
                cache_write: 0,
                reasoning: usage["completion_tokens_details"]["reasoning_tokens"].as_u64(),
                total_tokens: usage["total_tokens"].as_u64().unwrap_or(0),
                cost: Default::default(),
            };
        }

        let Some(choice) = chunk["choices"].as_array().and_then(|c| c.first()) else { return };
        let delta = &choice["delta"];

        if let Some(reasoning) = delta["reasoning_content"]
            .as_str()
            .or_else(|| delta["reasoning"].as_str())
            .filter(|s| !s.is_empty())
        {
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
        let url = format!("{}/chat/completions", self.base_url);
        let client = self.client.clone();
        let (max_retries, max_retry_delay_ms) = (self.max_retries, self.max_retry_delay_ms);
        let info = self.info;

        tokio::spawn(async move {
            let build = || options.apply_to(bearer_auth(client.post(&url), &api_key).header("User-Agent", USER_AGENT)).json(&body);
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
                    match crate::json::parse_json_with_repair(&event.data) {
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
    use crate::transform::NON_VISION_TOOL_IMAGE_PLACEHOLDER;
    use crate::types::{AssistantMessage, ToolCall, ToolResultMessage, UserMessage};

    fn request(messages: Vec<LlmMessage>) -> ModelRequest {
        ModelRequest { system_prompt: String::new(), messages, tools: Vec::new(), cache_points: Vec::new(), max_tokens: None, options: Default::default() }
    }

    /// An assistant turn that called these tools, `(id, name)`, at once.
    fn calls(calls: &[(&str, &str)]) -> LlmMessage {
        let mut message = AssistantMessage::empty("opencode", "kimi-k3");
        message.content = calls
            .iter()
            .map(|(id, name)| AssistantPart::ToolCall(ToolCall { id: (*id).into(), name: (*name).into(), arguments: json!({}) }))
            .collect();
        message.stop_reason = StopReason::ToolUse;
        LlmMessage::Assistant(message)
    }

    fn result(id: &str, tool: &str, content: Vec<ContentPart>) -> LlmMessage {
        LlmMessage::ToolResult(ToolResultMessage { tool_call_id: id.into(), tool_name: tool.into(), content, details: Value::Null, is_error: false, timestamp: 0 })
    }

    fn png(data: &str) -> ContentPart {
        ContentPart::Image { data: data.into(), mime_type: "image/png".into() }
    }

    #[test]
    fn a_parallel_turn_answers_every_call_before_the_images() {
        // A codemode script returned a screenshot, and a bash call in the same batch its output.
        let mut answer = AssistantMessage::empty("opencode", "kimi-k3");
        answer.content = vec![AssistantPart::Text { text: "It loads.".into() }];
        let body = OpenAiCompatProvider::new("opencode", "https://opencode.ai/zen/v1", "k", "kimi-k3").body(&request(vec![
            LlmMessage::User(UserMessage::text("check the page")),
            calls(&[("a", "codemode"), ("b", "bash")]),
            result("a", "codemode", vec![png("AAAA")]),
            result("b", "bash", vec![ContentPart::text("ok")]),
            LlmMessage::Assistant(answer),
            LlmMessage::User(UserMessage::text("and the footer?")),
        ]));
        let messages = body["messages"].as_array().unwrap();
        let roles: Vec<_> = messages.iter().map(|message| message["role"].as_str().unwrap()).collect();
        assert_eq!(roles, ["user", "assistant", "tool", "tool", "user", "assistant", "user"]);
        assert_eq!(messages[1]["tool_calls"].as_array().map(Vec::len), Some(2));
        assert_eq!(messages[2], json!({ "role": "tool", "tool_call_id": "a", "content": "(see attached image)" }));
        assert_eq!(messages[3], json!({ "role": "tool", "tool_call_id": "b", "content": "ok" }));
        assert_eq!(
            messages[4]["content"],
            json!([
                { "type": "text", "text": "Images from the codemode tool result:" },
                { "type": "image_url", "image_url": { "url": "data:image/png;base64,AAAA" } }
            ])
        );
    }

    #[test]
    fn a_turn_that_ends_on_its_results_still_sends_their_images() {
        // Two image files read at once; the loop asks the model again right after.
        let transcript = vec![
            LlmMessage::User(UserMessage::text("compare a.png and b.png")),
            calls(&[("a", "read"), ("b", "read")]),
            result("a", "read", vec![png("AAAA")]),
            result("b", "read", vec![ContentPart::text("Read b.png"), png("BBBB")]),
        ];
        let body = OpenAiCompatProvider::new("opencode", "https://opencode.ai/zen/v1", "k", "kimi-k3").body(&request(transcript.clone()));
        let messages = body["messages"].as_array().unwrap();
        assert_eq!(messages.len(), 5);
        assert_eq!(messages[2], json!({ "role": "tool", "tool_call_id": "a", "content": "(see attached image)" }));
        assert_eq!(messages[3], json!({ "role": "tool", "tool_call_id": "b", "content": "Read b.png" }));
        assert_eq!(
            messages[4],
            json!({ "role": "user", "content": [
                { "type": "text", "text": "Images from the read tool result:" },
                { "type": "image_url", "image_url": { "url": "data:image/png;base64,AAAA" } },
                { "type": "text", "text": "Images from the read tool result:" },
                { "type": "image_url", "image_url": { "url": "data:image/png;base64,BBBB" } }
            ] })
        );

        // GLM-5.3 takes no images: the notes the transform leaves are the tool messages' text.
        let mut text_only = OpenAiCompatProvider::new("opencode", "https://opencode.ai/zen/v1", "k", "glm-5.3");
        text_only.supports_images = false;
        let body = text_only.body(&request(transcript));
        let messages = body["messages"].as_array().unwrap();
        assert_eq!(messages.len(), 4, "no user message after the results");
        assert_eq!(messages[2]["content"], NON_VISION_TOOL_IMAGE_PLACEHOLDER);
        assert_eq!(messages[3]["content"], format!("Read b.png\n{NON_VISION_TOOL_IMAGE_PLACEHOLDER}"));
    }

    #[tokio::test]
    async fn gateway_reasoning_field_streams_as_thinking() {
        let (tx, mut rx) = mpsc::channel(8);
        let mut state = StreamState::default();
        state
            .apply_chunk(&json!({ "choices": [{ "delta": { "reasoning": "considering" } }] }), &tx)
            .await;
        drop(tx);
        assert!(matches!(rx.recv().await, Some(AssistantEvent::ThinkingStart { index: 0 })));
        assert!(matches!(rx.recv().await, Some(AssistantEvent::ThinkingDelta { index: 0, delta }) if delta == "considering"));
    }

    #[tokio::test]
    async fn cached_prompt_tokens_count_once() {
        let (tx, _rx) = mpsc::channel(8);
        let mut deepseek = StreamState::default();
        let usage = json!({ "usage": { "prompt_tokens": 100, "completion_tokens": 7, "total_tokens": 107, "prompt_cache_hit_tokens": 80, "prompt_cache_miss_tokens": 20 } });
        deepseek.apply_chunk(&usage, &tx).await;
        assert_eq!((deepseek.usage.input, deepseek.usage.cache_read, deepseek.usage.output), (20, 80, 7));

        let mut openai = StreamState::default();
        let usage = json!({ "usage": { "prompt_tokens": 100, "completion_tokens": 7, "total_tokens": 107, "prompt_tokens_details": { "cached_tokens": 64 } } });
        openai.apply_chunk(&usage, &tx).await;
        assert_eq!((openai.usage.input, openai.usage.cache_read), (36, 64));
        assert_eq!(crate::estimate::context_tokens(&openai.usage), 107);
    }

    #[test]
    fn a_thinking_level_is_the_word_the_model_takes() {
        let request = request(vec![LlmMessage::User(UserMessage::text("hi"))]);
        let body = |kind: &str, model: &str, level: ThinkingLevel| {
            OpenAiCompatProvider::new(kind, "https://opencode.ai/zen/v1", "k", model).with_thinking(Some(level)).body(&request)
        };
        let max = body("opencode", "deepseek-v4-pro", ThinkingLevel::Max);
        assert_eq!(max["reasoning_effort"], "max");
        assert!(max.get("thinking").is_none());
        // Zen turns DeepSeek V4 Pro's thinking off; Go cannot, so Off runs at its lowest effort.
        let off = body("opencode", "deepseek-v4-pro", ThinkingLevel::Off);
        assert_eq!(off["thinking"], json!({ "type": "disabled" }));
        assert!(off.get("reasoning_effort").is_none());
        let go = body("opencode-go", "deepseek-v4-pro", ThinkingLevel::Off);
        assert_eq!(go["reasoning_effort"], "high");
        assert!(go.get("thinking").is_none());
        assert_eq!(body("opencode", "kimi-k3", ThinkingLevel::Low)["reasoning_effort"], "max");
        // A model the catalog does not know gets at most `high`, and nothing for Off.
        assert_eq!(body("openai", "some-model", ThinkingLevel::Max)["reasoning_effort"], "high");
        let unknown_off = body("openai", "some-model", ThinkingLevel::Off);
        assert!(unknown_off.get("thinking").is_none() && unknown_off.get("reasoning_effort").is_none());
    }
}
