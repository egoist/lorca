//! API-key authentication for an OpenAI-compatible Responses endpoint.
//!
//! ChatGPT and Grok have their own subscription adapters. This adapter is for gateways that
//! expose the same `/responses` wire shape with a bearer API key.

use async_trait::async_trait;
use serde_json::{json, Value};
use tokio::sync::mpsc;
use tokio_util::sync::CancellationToken;

use super::responses::{self, ToolImages};
use crate::models::{self, ModelInfo};
use crate::provider::{
    channel_stream, AssistantEvent, AssistantEventStream, ModelRequest, Provider,
};
use crate::request::bearer_auth;
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
    /// Where tool results' images go: a user message after the outputs by default, since a
    /// gateway may take only a string `output`; `InOutput` for OpenAI's own API.
    pub tool_images: ToolImages,
    /// Retries of a request that fails before it streams (408, 409, 429, 5xx, transport).
    pub max_retries: u32,
    pub max_retry_delay_ms: u64,
    /// Sent as `reasoning.effort` when the catalog says the model takes an effort. `Off` is the
    /// effort `none` on a model the catalog says can stop thinking, and sends nothing on any
    /// other.
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
            tool_images: ToolImages::UserMessage,
            max_retries: 2,
            max_retry_delay_ms: DEFAULT_MAX_RETRY_DELAY_MS,
            thinking_level: None,
            info,
            client: lorca_tls::client(),
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
            "input": responses::input_items(&transformed, self.tool_images),
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
            None => None,
            // Only a model the catalog says can stop thinking keeps `Off` through the clamp.
            Some(ThinkingLevel::Off) => Some("none"),
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
                let request = bearer_auth(client.post(&url), &api_key)
                    .header("Accept", "text/event-stream")
                    .header("User-Agent", USER_AGENT);
                options.apply_to(request).json(&body)
            };
            let response =
                match send_with_retry(build, max_retries, max_retry_delay_ms, &cancel, &options).await {
                    Ok(response) => {
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
    use crate::providers::responses::testing;
    use crate::transform::NON_VISION_TOOL_IMAGE_PLACEHOLDER;
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
            "gpt-6.1-sol",
        )
        .with_thinking(Some(ThinkingLevel::Medium));
        let body = provider.body(&request());
        assert_eq!(body["model"], "gpt-6.1-sol");
        assert_eq!(body["instructions"], "be brief");
        assert_eq!(body["input"][0]["type"], "message");
        assert_eq!(body["tools"][0]["name"], "read");
        assert_eq!(body["max_output_tokens"], 200);
        assert_eq!(body["reasoning"], json!({ "effort": "medium" }));
        assert!(body.get("prompt_cache_key").is_none());
    }

    #[test]
    fn the_chat_keys_the_prompt_cache() {
        let provider = OpenAiResponsesProvider::new("opencode", "https://opencode.ai/zen/v1", "k", "gpt-6.1-sol");
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

    #[test]
    fn off_is_the_effort_none_where_the_model_can_stop_thinking() {
        let body = |kind: &str, model: &str| {
            OpenAiResponsesProvider::new(kind, "https://opencode.ai/zen/go/v1", "k", model).with_thinking(Some(ThinkingLevel::Off)).body(&request())
        };
        assert_eq!(body("opencode-go", "gpt-6-luna")["reasoning"], json!({ "effort": "none" }));
        // GPT-6.1 Sol always reasons, so Off runs at its lowest effort.
        assert_eq!(body("opencode", "gpt-6.1-sol")["reasoning"], json!({ "effort": "low" }));
        assert!(body("opencode", "unknown-model").get("reasoning").is_none());
    }

    #[test]
    fn a_tool_image_follows_the_outputs_and_a_text_only_model_gets_a_note() {
        let mut request = request();
        request.messages.extend(testing::screenshot_turn("opencode", "gpt-6.1-sol", "AAAA"));
        let provider = OpenAiResponsesProvider::new("opencode", "https://opencode.ai/zen/v1", "k", "gpt-6.1-sol");
        let input = provider.body(&request)["input"].clone();
        assert_eq!(input[2]["output"], "Took a screenshot of the page.");
        assert_eq!(input[3]["role"], "user");
        assert_eq!(input[3]["content"][1]["image_url"], "data:image/png;base64,AAAA");

        let mut text_only = OpenAiResponsesProvider::new("opencode", "https://opencode.ai/zen/v1", "k", "gpt-6.1-sol");
        text_only.supports_images = false;
        let input = text_only.body(&request)["input"].clone();
        assert_eq!(input[2]["output"], format!("Took a screenshot of the page.\n{NON_VISION_TOOL_IMAGE_PLACEHOLDER}"));
        assert_eq!(input.as_array().unwrap().len(), 3);

        // OpenAI's own Responses API takes the image inside the output.
        let mut openai = OpenAiResponsesProvider::new("openai", "https://api.openai.com/v1", "k", "gpt-6.1-sol");
        openai.tool_images = ToolImages::InOutput;
        assert_eq!(openai.body(&request)["input"][2]["output"][1]["type"], "input_image");
    }

    /// `LORCA_CREDENTIALS=~/.lorca/credentials.json cargo test -p lorca-agent live_opencode_zen --
    /// --ignored --nocapture`: Zen's GPT, Grok, and Muse Spark routes read a random code off a
    /// tool's screenshot sent in a user message, and the probe reports whether each also reads
    /// one inside the output. `live_opencode_go` does the same on Go's GPT and Grok.
    #[tokio::test]
    #[ignore]
    async fn live_opencode_zen_reads_a_tool_screenshot() {
        opencode_reads_a_tool_screenshot("opencode", "opencode", "https://opencode.ai/zen", &["gpt-6.1-sol", "grok-4.7", "muse-spark-1.3"]).await;
    }

    #[tokio::test]
    #[ignore]
    async fn live_opencode_go_reads_a_tool_screenshot() {
        opencode_reads_a_tool_screenshot("opencode-go", "opencode_go", "https://opencode.ai/zen/go", &["gpt-6-luna", "grok-4.7"]).await;
    }

    /// The probe on each of `models`, with the API key under `key` in the credentials and the
    /// root it names, else `root`.
    async fn opencode_reads_a_tool_screenshot(kind: &str, key: &str, root: &str, models: &[&str]) {
        let Some(credential) = testing::credential::<Value>(key) else { return };
        let root = credential["base_url"].as_str().map(|url| url.trim_end_matches('/').trim_end_matches("/v1")).unwrap_or(root);
        let api_key = credential["api_key"].as_str().expect("an API key");
        // Go routes a request by its session header, which the Runner sends.
        let options = || {
            let session = uuid::Uuid::new_v4().to_string();
            let mut options = crate::RequestOptions::default().with_session_id(&session);
            options.headers.insert("x-opencode-session".into(), session);
            options
        };
        let mut failures = Vec::new();
        for model in models {
            let mut provider = OpenAiResponsesProvider::new(kind, &format!("{root}/v1"), api_key, model);
            if let Err(problem) = testing::reads_a_screenshot(&provider, options()).await {
                failures.push(format!("{kind} {model}: {problem}"));
            }
            provider.tool_images = ToolImages::InOutput;
            match testing::reads_a_screenshot(&provider, options()).await {
                Ok(_) => eprintln!("{kind} {model} read the image inside the function call output too"),
                Err(problem) => eprintln!("{kind} {model} did not read the image inside the function call output: {problem}"),
            }
        }
        assert!(failures.is_empty(), "{}", failures.join("\n"));
    }
}
