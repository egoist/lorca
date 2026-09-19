//! Grok subscription adapter. Isolated from the API-key providers: it authenticates with the
//! OAuth tokens a Grok sign-in yields (a SuperGrok or X Premium+ account) and calls xAI's
//! Responses API with them.

pub mod oauth;

use std::sync::Arc;

use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use tokio::sync::mpsc;
use tokio_util::sync::CancellationToken;

use super::responses;
use crate::models::{self, ModelInfo};
use crate::provider::{channel_stream, AssistantEvent, AssistantEventStream, ModelRequest, Provider};
use crate::retry::{send_with_retry, RequestFailure, DEFAULT_MAX_RETRY_DELAY_MS};
use crate::transform::{transform_messages, TransformOptions};
use crate::types::ThinkingLevel;

pub const GROK_BASE_URL: &str = "https://api.x.ai/v1";
/// xAI's current reasoning model. Others a subscription runs: `grok-4.5`, `grok-4.3`,
/// `grok-4.20-0309-reasoning`, `grok-build-0.1`.
pub const GROK_DEFAULT_MODEL: &str = "grok-4.6";

/// Tokens from a Grok sign-in. The CLI persists these on the Runner.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct GrokTokens {
    pub access_token: String,
    pub refresh_token: String,
    #[serde(default)]
    pub id_token: Option<String>,
    /// The account's `sub`, when the sign-in named it.
    #[serde(default)]
    pub account_id: Option<String>,
    #[serde(default)]
    pub email: Option<String>,
    /// Unix seconds.
    pub expires_at: u64,
}

impl GrokTokens {
    /// Access tokens last about six hours; one within five minutes of its end is refreshed
    /// before it is used.
    pub fn is_expired(&self) -> bool {
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_secs())
            .unwrap_or(0);
        now + 5 * 60 >= self.expires_at
    }
}

/// Where the adapter reads tokens from and writes refreshed ones back to.
#[async_trait]
pub trait GrokTokenSource: Send + Sync {
    async fn tokens(&self) -> Result<GrokTokens, String>;
    async fn store(&self, tokens: GrokTokens) -> Result<(), String>;
}

pub struct GrokProvider {
    tokens: Arc<dyn GrokTokenSource>,
    model: String,
    /// The API root; `/responses` is appended.
    pub base_url: String,
    /// The OAuth issuer the tokens refresh against.
    pub endpoints: oauth::Endpoints,
    /// Sent as `reasoning.effort` on the models that take it (`Off` sends nothing).
    pub thinking_level: Option<ThinkingLevel>,
    /// The catalog entry for the model, when it has one.
    pub info: Option<&'static ModelInfo>,
    client: reqwest::Client,
}

/// Whether the model takes `reasoning.effort` at all. The others reason on their own and
/// answer 400 to the parameter, so they are sent none.
pub fn takes_reasoning_effort(model: &str) -> bool {
    let model = model.trim().to_ascii_lowercase();
    ["grok-3-mini", "grok-4.20-multi-agent", "grok-4.3", "grok-4.5", "grok-4.6"].iter().any(|prefix| model.starts_with(prefix))
}

impl GrokProvider {
    pub fn new(tokens: Arc<dyn GrokTokenSource>, model: Option<&str>) -> Self {
        let model = model.unwrap_or(GROK_DEFAULT_MODEL);
        GrokProvider {
            tokens,
            model: model.to_string(),
            base_url: GROK_BASE_URL.to_string(),
            endpoints: oauth::Endpoints::xai(),
            thinking_level: None,
            info: models::find("grok", model),
            client: reqwest::Client::new(),
        }
    }

    pub fn with_thinking(mut self, level: Option<ThinkingLevel>) -> Self {
        self.thinking_level = level;
        self
    }

    /// Another API root, for a proxy or a test server.
    pub fn with_base_url(mut self, base_url: &str) -> Self {
        self.base_url = base_url.trim_end_matches('/').to_string();
        self
    }

    /// Another OAuth issuer, for a test server.
    pub fn with_issuer(mut self, issuer: &str) -> Self {
        self.endpoints = oauth::Endpoints::at(issuer);
        self
    }

    async fn fresh_tokens(&self) -> Result<GrokTokens, String> {
        let tokens = self.tokens.tokens().await?;
        if !tokens.is_expired() {
            return Ok(tokens);
        }
        let mut refreshed = match oauth::refresh(&self.client, &self.endpoints, &tokens.refresh_token).await {
            Ok(refreshed) => refreshed,
            // The source may be shared: another holder that refreshed first spent this
            // refresh token, and its tokens are the ones to use.
            Err(error) => match self.tokens.tokens().await {
                Ok(current) if current.refresh_token != tokens.refresh_token && !current.is_expired() => return Ok(current),
                _ => return Err(error),
            },
        };
        if refreshed.account_id.is_none() {
            refreshed.account_id = tokens.account_id.clone();
        }
        if refreshed.email.is_none() {
            refreshed.email = tokens.email.clone();
        }
        self.tokens.store(refreshed.clone()).await?;
        Ok(refreshed)
    }

    fn body(&self, request: &ModelRequest) -> Value {
        let transformed = transform_messages(
            &request.messages,
            &TransformOptions { provider: "grok", model: &self.model, supports_images: true, normalize_tool_call_id: None },
        );
        let input = responses::input_items(&transformed);

        // xAI's own search tools: the model searches the web and X on the server and streams
        // `web_search_call` and `x_search_call` items, which become server-tool events here.
        let mut tools = vec![json!({ "type": "web_search" }), json!({ "type": "x_search" })];
        tools.extend(responses::function_tools(&request.tools));

        let mut body = json!({
            "model": self.model,
            "instructions": request.system_prompt,
            "input": input,
            "tools": tools,
            "tool_choice": "auto",
            "parallel_tool_calls": true,
            "store": false,
            "stream": true,
        });
        if let Some(max_tokens) = request.max_tokens {
            body["max_output_tokens"] = Value::from(max_tokens);
        }
        let effort = match self.thinking_level {
            None | Some(ThinkingLevel::Off) => None,
            Some(ThinkingLevel::Minimal | ThinkingLevel::Low) => Some("low"),
            Some(ThinkingLevel::Medium) => Some("medium"),
            Some(ThinkingLevel::High | ThinkingLevel::XHigh | ThinkingLevel::Max) => Some("high"),
        };
        if let (Some(effort), true) = (effort, takes_reasoning_effort(&self.model)) {
            body["reasoning"] = json!({ "effort": effort });
        }
        body
    }
}

#[async_trait]
impl Provider for GrokProvider {
    fn provider_id(&self) -> &str {
        "grok"
    }

    fn model_id(&self) -> &str {
        &self.model
    }

    fn model_info(&self) -> Option<&'static ModelInfo> {
        self.info
    }

    async fn stream(&self, request: ModelRequest, cancel: CancellationToken) -> AssistantEventStream {
        let (tx, rx) = mpsc::channel(64);
        let mut body = self.body(&request);
        let options = request.options.clone();
        options.before_payload(&mut body);
        let client = self.client.clone();
        let info = self.info;
        let url = format!("{}/responses", self.base_url);
        let tokens = match self.fresh_tokens().await {
            Ok(tokens) => tokens,
            Err(message) => {
                tokio::spawn(async move {
                    let _ = tx.send(AssistantEvent::Error { message: format!("Grok sign-in: {message}"), aborted: false }).await;
                });
                return channel_stream(rx);
            }
        };

        tokio::spawn(async move {
            let build = || {
                let request = client.post(&url).bearer_auth(&tokens.access_token).header("Accept", "text/event-stream");
                options.apply_to(request).json(&body)
            };
            let response = match send_with_retry(build, 2, DEFAULT_MAX_RETRY_DELAY_MS, &cancel).await {
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

            responses::pump(response, tx, cancel, info, "grok").await;
        });

        channel_stream(rx)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::provider::ToolSpec;
    use crate::types::LlmMessage;

    struct StaticTokens;

    #[async_trait]
    impl GrokTokenSource for StaticTokens {
        async fn tokens(&self) -> Result<GrokTokens, String> {
            Err("unused".into())
        }
        async fn store(&self, _tokens: GrokTokens) -> Result<(), String> {
            Ok(())
        }
    }

    fn request() -> ModelRequest {
        ModelRequest {
            system_prompt: "be brief".into(),
            messages: vec![LlmMessage::User(crate::types::UserMessage::text("hi"))],
            tools: vec![ToolSpec { name: "read".into(), description: "read a file".into(), parameters: json!({ "type": "object" }) }],
            max_tokens: None,
            options: Default::default(),
        }
    }

    #[test]
    fn the_request_carries_both_searches_before_the_functions() {
        let provider = GrokProvider::new(Arc::new(StaticTokens), None);
        let body = provider.body(&request());
        assert_eq!(body["model"], GROK_DEFAULT_MODEL);
        let tools = body["tools"].as_array().unwrap();
        assert_eq!(tools[0], json!({ "type": "web_search" }));
        assert_eq!(tools[1], json!({ "type": "x_search" }));
        assert_eq!(tools[2]["type"], "function");
        assert_eq!(tools[2]["name"], "read");
        assert!(body.get("reasoning").is_none());
    }

    #[test]
    fn effort_goes_only_to_models_that_take_it() {
        let high = GrokProvider::new(Arc::new(StaticTokens), Some("grok-4.6")).with_thinking(Some(ThinkingLevel::Max));
        assert_eq!(high.body(&request())["reasoning"], json!({ "effort": "high" }));
        let low = GrokProvider::new(Arc::new(StaticTokens), Some("grok-4.3")).with_thinking(Some(ThinkingLevel::Minimal));
        assert_eq!(low.body(&request())["reasoning"], json!({ "effort": "low" }));
        let off = GrokProvider::new(Arc::new(StaticTokens), Some("grok-4.6")).with_thinking(Some(ThinkingLevel::Off));
        assert!(off.body(&request()).get("reasoning").is_none());
        let native = GrokProvider::new(Arc::new(StaticTokens), Some("grok-4.20-0309-reasoning")).with_thinking(Some(ThinkingLevel::High));
        assert!(native.body(&request()).get("reasoning").is_none());
    }

    #[test]
    fn a_token_near_its_end_counts_as_expired() {
        let now = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_secs();
        let soon = GrokTokens { access_token: "a".into(), refresh_token: "r".into(), id_token: None, account_id: None, email: None, expires_at: now + 60 };
        assert!(soon.is_expired());
        let later = GrokTokens { expires_at: now + 3600, ..soon };
        assert!(!later.is_expired());
    }
}
