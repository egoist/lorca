//! ChatGPT subscription adapter. Isolated from the API-key providers: it authenticates with
//! the OAuth tokens a ChatGPT login yields and calls the Codex responses backend.

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
    /// Sent as `reasoning.effort` (`Off` sends nothing).
    pub thinking_level: Option<ThinkingLevel>,
    /// The catalog entry for the model, when it has one.
    pub info: Option<&'static ModelInfo>,
    client: reqwest::Client,
}

impl ChatGptProvider {
    pub fn new(tokens: Arc<dyn TokenSource>, model: Option<&str>) -> Self {
        let model = model.unwrap_or(CHATGPT_DEFAULT_MODEL);
        ChatGptProvider {
            tokens,
            model: model.to_string(),
            thinking_level: None,
            info: models::find("chatgpt", model),
            client: reqwest::Client::new(),
        }
    }

    pub fn with_thinking(mut self, level: Option<ThinkingLevel>) -> Self {
        self.thinking_level = level;
        self
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
        let transformed = transform_messages(
            &request.messages,
            &TransformOptions { provider: "chatgpt", model: &self.model, supports_images: true, normalize_tool_call_id: None },
        );
        let input = responses::input_items(&transformed);

        // The backend's own web search: it searches and reads pages server-side and streams
        // `web_search_call` items, which become server-tool events here.
        let mut tools = vec![json!({ "type": "web_search" })];
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
            Some(ThinkingLevel::High) => Some("high"),
            Some(ThinkingLevel::XHigh | ThinkingLevel::Max) => Some("xhigh"),
        };
        if let Some(effort) = effort {
            body["reasoning"] = json!({ "effort": effort, "summary": "auto" });
        }
        body
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
            let build = || {
                let request = client
                    .post(CHATGPT_RESPONSES_URL)
                    .bearer_auth(&tokens.access_token)
                    .header("chatgpt-account-id", &tokens.account_id)
                    .header("OpenAI-Beta", "responses=experimental")
                    .header("originator", "codex_cli_rs")
                    .header("Accept", "text/event-stream");
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

            responses::pump(response, tx, cancel, info, "chatgpt").await;
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
    impl TokenSource for StaticTokens {
        async fn tokens(&self) -> Result<ChatGptTokens, String> {
            Err("unused".into())
        }
        async fn store(&self, _tokens: ChatGptTokens) -> Result<(), String> {
            Ok(())
        }
    }

    #[test]
    fn the_request_carries_the_backend_web_search_before_the_functions() {
        let provider = ChatGptProvider::new(Arc::new(StaticTokens), None);
        let request = ModelRequest {
            system_prompt: "be brief".into(),
            messages: vec![LlmMessage::User(crate::types::UserMessage::text("hi"))],
            tools: vec![ToolSpec { name: "read".into(), description: "read a file".into(), parameters: json!({ "type": "object" }) }],
            max_tokens: None,
            options: Default::default(),
        };
        let body = provider.body(&request);
        let tools = body["tools"].as_array().unwrap();
        assert_eq!(tools[0], json!({ "type": "web_search" }));
        assert_eq!(tools[1]["type"], "function");
        assert_eq!(tools[1]["name"], "read");
    }
}
