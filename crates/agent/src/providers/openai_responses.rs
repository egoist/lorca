//! API-key authentication for an OpenAI-compatible Responses endpoint.
//!
//! ChatGPT and Grok have their own subscription adapters. This adapter is for gateways that
//! expose the same `/responses` wire shape with a bearer API key.

use async_trait::async_trait;
use serde_json::{json, Value};
use tokio::sync::mpsc;
use tokio_util::sync::CancellationToken;

use super::responses;
use crate::models::{self, ModelInfo};
use crate::provider::{
    channel_stream, AssistantEvent, AssistantEventStream, ModelRequest, Provider,
};
use crate::retry::{send_with_retry, RequestFailure, DEFAULT_MAX_RETRY_DELAY_MS};
use crate::transform::{transform_messages, TransformOptions};
use crate::types::ThinkingLevel;

const USER_AGENT: &str = concat!("lorca-agent/", env!("CARGO_PKG_VERSION"));

pub struct OpenAiResponsesProvider {
    pub provider_id: String,
    /// The API root; `/responses` is appended.
    pub base_url: String,
    pub api_key: String,
    pub model: String,
    /// Whether the model takes images; a text-only model gets a note in their place.
    pub supports_images: bool,
    /// Retries of a request that fails before it streams (408, 409, 429, 5xx, transport).
    pub max_retries: u32,
    pub max_retry_delay_ms: u64,
    /// Sent as `reasoning.effort` when the catalog says the model takes an effort.
    pub thinking_level: Option<ThinkingLevel>,
    /// The catalog entry for the model, when it has one.
    pub info: Option<&'static ModelInfo>,
    client: reqwest::Client,
}

impl OpenAiResponsesProvider {
    pub fn new(provider_id: &str, base_url: &str, api_key: &str, model: &str) -> Self {
        let info = models::find(provider_id, model);
        OpenAiResponsesProvider {
            provider_id: provider_id.to_string(),
            base_url: base_url.trim_end_matches('/').to_string(),
            api_key: api_key.to_string(),
            model: model.to_string(),
            supports_images: info.map(|model| model.images).unwrap_or(true),
            max_retries: 2,
            max_retry_delay_ms: DEFAULT_MAX_RETRY_DELAY_MS,
            thinking_level: None,
            info,
            client: reqwest::Client::new(),
        }
    }

    pub fn with_thinking(mut self, level: Option<ThinkingLevel>) -> Self {
        self.thinking_level = level;
        self
    }

    fn body(&self, request: &ModelRequest) -> Value {
        let transformed = transform_messages(
            &request.messages,
            &TransformOptions {
                provider: &self.provider_id,
                model: &self.model,
                supports_images: self.supports_images,
                normalize_tool_call_id: None,
            },
        );
        let mut body = json!({
            "model": self.model,
            "instructions": request.system_prompt,
            "input": responses::input_items(&transformed),
            "store": false,
            "stream": true,
        });
        if !request.tools.is_empty() {
            body["tools"] = Value::Array(responses::function_tools(&request.tools));
            body["tool_choice"] = Value::String("auto".into());
            body["parallel_tool_calls"] = Value::Bool(true);
        }
        if let Some(max_tokens) = request.max_tokens {
            body["max_output_tokens"] = Value::from(max_tokens);
        }
        if let Some(session_id) = &request.options.session_id {
            body["prompt_cache_key"] = Value::String(session_id.clone());
        }
        let level = self.thinking_level.and_then(|level| match self.info {
            Some(info) => info.clamp_level(level),
            None if level == ThinkingLevel::Off => None,
            None => Some(level),
        });
        let effort = match level {
            None | Some(ThinkingLevel::Off) => None,
            Some(ThinkingLevel::Minimal) => Some("minimal"),
            Some(ThinkingLevel::Low) => Some("low"),
            Some(ThinkingLevel::Medium) => Some("medium"),
            Some(ThinkingLevel::High) => Some("high"),
            Some(ThinkingLevel::XHigh) => Some("xhigh"),
            Some(ThinkingLevel::Max) => Some("max"),
        };
        if let Some(effort) = effort {
            body["reasoning"] = json!({ "effort": effort });
        }
        body
    }
}

#[async_trait]
impl Provider for OpenAiResponsesProvider {
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

    async fn stream(
        &self,
        request: ModelRequest,
        cancel: CancellationToken,
    ) -> AssistantEventStream {
        let (tx, rx) = mpsc::channel(64);
        let mut body = self.body(&request);
        let options = request.options.clone();
        options.before_payload(&mut body);
        let api_key = options.api_key(&self.api_key).await;
        let url = format!("{}/responses", self.base_url);
        let client = self.client.clone();
        let (max_retries, max_retry_delay_ms) = (self.max_retries, self.max_retry_delay_ms);
        let info = self.info;
        let provider = self.provider_id.clone();

        tokio::spawn(async move {
            let build = || {
                let request = client
                    .post(&url)
                    .bearer_auth(&api_key)
                    .header("Accept", "text/event-stream")
                    .header("User-Agent", USER_AGENT);
                options.apply_to(request).json(&body)
            };
            let response =
                match send_with_retry(build, max_retries, max_retry_delay_ms, &cancel).await {
                    Ok(response) => {
                        options.report(&response);
                        response
                    }
                    Err(failure) => {
                        let aborted = matches!(failure, RequestFailure::Aborted);
                        let _ = tx
                            .send(AssistantEvent::Error {
                                message: failure.message(),
                                aborted,
                            })
                            .await;
                        return;
                    }
                };

            responses::pump(response, tx, cancel, info, &provider).await;
        });

        channel_stream(rx)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::provider::ToolSpec;
    use crate::types::{LlmMessage, UserMessage};

    fn request() -> ModelRequest {
        ModelRequest {
            system_prompt: "be brief".into(),
            messages: vec![LlmMessage::User(UserMessage::text("hi"))],
            tools: vec![ToolSpec {
                name: "read".into(),
                description: "read a file".into(),
                parameters: json!({ "type": "object" }),
            }],
            cache_points: Vec::new(),
            max_tokens: Some(200),
            options: Default::default(),
        }
    }

    #[test]
    fn request_uses_the_responses_shape_and_catalog_effort() {
        let provider = OpenAiResponsesProvider::new(
            "opencode",
            "https://opencode.ai/zen/v1",
            "k",
            "gpt-5.6-terra",
        )
        .with_thinking(Some(ThinkingLevel::Medium));
        let body = provider.body(&request());
        assert_eq!(body["model"], "gpt-5.6-terra");
        assert_eq!(body["instructions"], "be brief");
        assert_eq!(body["input"][0]["type"], "message");
        assert_eq!(body["tools"][0]["name"], "read");
        assert_eq!(body["max_output_tokens"], 200);
        assert_eq!(body["reasoning"], json!({ "effort": "medium" }));
        assert!(body.get("prompt_cache_key").is_none());
    }

    #[test]
    fn the_chat_keys_the_prompt_cache() {
        let provider = OpenAiResponsesProvider::new("opencode", "https://opencode.ai/zen/v1", "k", "gpt-5.6-terra");
        let mut request = request();
        request.options = crate::RequestOptions::default().with_session_id("chat-1");
        assert_eq!(provider.body(&request)["prompt_cache_key"], "chat-1");
    }

    #[test]
    fn a_model_without_effort_levels_gets_no_reasoning_parameter() {
        let provider = OpenAiResponsesProvider::new(
            "opencode",
            "https://opencode.ai/zen/v1",
            "k",
            "big-pickle",
        )
        .with_thinking(Some(ThinkingLevel::High));
        assert!(provider.body(&request()).get("reasoning").is_none());
    }
}
