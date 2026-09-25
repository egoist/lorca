//! ChatGPT subscription adapter. Isolated from the API-key providers: it authenticates with
//! the OAuth tokens a ChatGPT login yields and calls the Codex responses backend.

pub use lorca_provider_auth::chatgpt as oauth;
pub use lorca_provider_auth::chatgpt::ChatGptTokens;

use std::sync::Arc;

use async_trait::async_trait;
use serde_json::{json, Value};
use tokio::sync::mpsc;
use tokio_util::sync::CancellationToken;

use super::responses;
use crate::models::{self, ModelInfo};
use crate::provider::{channel_stream, AssistantEvent, AssistantEventStream, ModelRequest, Provider};
use crate::request::RequestOptions;
use crate::retry::{send_with_retry, RequestFailure, DEFAULT_MAX_RETRY_DELAY_MS};
use crate::transform::{transform_messages, TransformOptions};
use crate::types::ThinkingLevel;

pub const CHATGPT_RESPONSES_URL: &str = "https://chatgpt.com/backend-api/codex/responses";
/// The workhorse model Codex offers to ChatGPT sign-ins for coding and everyday work. The
/// catalog also lists `gpt-6-astra` and `gpt-6-luna`. The `*-codex` ids are rejected for
/// ChatGPT accounts.
pub const CHATGPT_DEFAULT_MODEL: &str = "gpt-6-sol";

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
        let refreshed = match oauth::refresh(&self.client, &tokens.refresh_token).await {
            Ok(refreshed) => refreshed,
            // The source may be shared: another holder that refreshed first spent this
            // refresh token, and its tokens are the ones to use.
            Err(error) => match self.tokens.tokens().await {
                Ok(current) if current.refresh_token != tokens.refresh_token && !current.is_expired() => return Ok(current),
                _ => return Err(error),
            },
        };
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
        if let Some(session_id) = &request.options.session_id {
            body["prompt_cache_key"] = Value::String(session_id.clone());
        }
        let effort = match self.thinking_level {
            None | Some(ThinkingLevel::Off) => None,
            Some(ThinkingLevel::Minimal | ThinkingLevel::Low) => Some("low"),
            Some(ThinkingLevel::Medium) => Some("medium"),
            Some(ThinkingLevel::High) => Some("high"),
            Some(ThinkingLevel::XHigh) => Some("xhigh"),
            Some(ThinkingLevel::Max) => Some("max"),
        };
        if let Some(effort) = effort {
            body["reasoning"] = json!({ "effort": effort, "summary": "auto" });
        }
        body
    }
}

/// One call to the backend with the sign-in's headers. The backend keeps a conversation on the
/// server that holds its prompt cache by the `session-id` header, as the Codex CLI sends it.
fn build_request(client: &reqwest::Client, tokens: &ChatGptTokens, options: &RequestOptions, body: &Value) -> reqwest::RequestBuilder {
    let mut request = client
        .post(CHATGPT_RESPONSES_URL)
        .bearer_auth(&tokens.access_token)
        .header("chatgpt-account-id", &tokens.account_id)
        .header("OpenAI-Beta", "responses=experimental")
        .header("originator", "codex_cli_rs")
        .header("Accept", "text/event-stream");
    if let Some(session_id) = &options.session_id {
        request = request.header("session-id", session_id);
    }
    options.apply_to(request).json(body)
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
            let build = || build_request(&client, &tokens, &options, &body);
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

    fn request() -> ModelRequest {
        ModelRequest {
            system_prompt: "be brief".into(),
            messages: vec![LlmMessage::User(crate::types::UserMessage::text("hi"))],
            tools: vec![ToolSpec { name: "read".into(), description: "read a file".into(), parameters: json!({ "type": "object" }) }],
            cache_points: Vec::new(),
            max_tokens: None,
            options: Default::default(),
        }
    }

    #[test]
    fn the_request_carries_the_backend_web_search_before_the_functions() {
        let provider = ChatGptProvider::new(Arc::new(StaticTokens), None);
        let body = provider.body(&request());
        assert_eq!(body["model"], CHATGPT_DEFAULT_MODEL);
        let tools = body["tools"].as_array().unwrap();
        assert_eq!(tools[0], json!({ "type": "web_search" }));
        assert_eq!(tools[1]["type"], "function");
        assert_eq!(tools[1]["name"], "read");
    }

    #[test]
    fn a_thinking_level_is_sent_as_the_effort() {
        let max = ChatGptProvider::new(Arc::new(StaticTokens), None).with_thinking(Some(ThinkingLevel::Max));
        assert_eq!(max.body(&request())["reasoning"], json!({ "effort": "max", "summary": "auto" }));
        let low = ChatGptProvider::new(Arc::new(StaticTokens), Some("gpt-6-luna")).with_thinking(Some(ThinkingLevel::Minimal));
        assert_eq!(low.body(&request())["reasoning"]["effort"], "low");
        let off = ChatGptProvider::new(Arc::new(StaticTokens), None).with_thinking(Some(ThinkingLevel::Off));
        assert!(off.body(&request()).get("reasoning").is_none());
    }

    #[test]
    fn the_chat_keys_the_prompt_cache() {
        let provider = ChatGptProvider::new(Arc::new(StaticTokens), None);
        assert!(provider.body(&request()).get("prompt_cache_key").is_none());
        let mut request = request();
        request.options = RequestOptions::default().with_session_id("chat-1");
        let body = provider.body(&request);
        assert_eq!(body["prompt_cache_key"], "chat-1");

        let tokens = ChatGptTokens {
            access_token: "a".into(),
            refresh_token: "r".into(),
            id_token: None,
            account_id: "acct".into(),
            email: None,
            expires_at: 0,
        };
        let built = build_request(&reqwest::Client::new(), &tokens, &request.options, &body).build().unwrap();
        assert_eq!(built.headers().get("session-id").unwrap(), "chat-1");
        assert_eq!(built.headers().get("chatgpt-account-id").unwrap(), "acct");
    }
}
