//! Providers built from the account's credentials, and the connect and disconnect flows. A
//! change made here reaches every Device through the `credentials` blob.

use std::sync::Arc;

use async_trait::async_trait;

use lorca_agent::providers::anthropic::{ANTHROPIC_BASE_URL, ANTHROPIC_VERSION};
use lorca_agent::providers::grok::oauth as grok_oauth;
use lorca_agent::providers::{
    AnthropicProvider, ChatGptProvider, ChatGptTokens, GrokProvider, GrokTokenSource, GrokTokens, OpenAiCompatProvider,
    OpenAiResponsesProvider, TokenSource,
};
use lorca_agent::{models, AssistantEventStream, ModelRequest, Provider, ThinkingLevel};
use tokio_util::sync::CancellationToken;

use crate::app::App;
use crate::config;

/// The provider kinds an account can hold, in the order the apps list them.
pub use crate::credentials::{ApiKeyCredential, Credentials, PROVIDER_KINDS};

pub const OPENCODE_BASE_URL: &str = "https://opencode.ai/zen";
pub const OPENCODE_DEFAULT_MODEL: &str = "deepseek-v4.1-flash";
pub const OPENCODE_GO_BASE_URL: &str = "https://opencode.ai/zen/go";
pub const OPENCODE_GO_DEFAULT_MODEL: &str = "glm-5.3-flash";

const USER_AGENT: &str = concat!("lorca/", env!("CARGO_PKG_VERSION"));

/// Identifies Lorca to OpenCode and sends the session header it uses for routing and prompt
/// caching. The regular request options also send `x-session-affinity`.
struct OpenCodeHeaders {
    inner: Arc<dyn Provider>,
}

#[async_trait]
impl Provider for OpenCodeHeaders {
    fn provider_id(&self) -> &str {
        self.inner.provider_id()
    }

    fn model_id(&self) -> &str {
        self.inner.model_id()
    }

    fn supports_images(&self) -> bool {
        self.inner.supports_images()
    }

    fn model_info(&self) -> Option<&'static lorca_agent::models::ModelInfo> {
        self.inner.model_info()
    }

    async fn stream(&self, mut request: ModelRequest, cancel: CancellationToken) -> AssistantEventStream {
        request.options.headers.insert("User-Agent".into(), USER_AGENT.into());
        if let Some(session_id) = request.options.session_id.clone() {
            request.options.headers.insert("x-opencode-session".into(), session_id);
        }
        self.inner.stream(request, cancel).await
    }
}

fn opencode_headers(inner: Arc<dyn Provider>) -> Arc<dyn Provider> {
    Arc::new(OpenCodeHeaders { inner })
}

/// Reads ChatGPT tokens from the account's credentials and hands refreshed ones to every Device.
pub struct AppTokenSource(pub Arc<App>);

#[async_trait]
impl TokenSource for AppTokenSource {
    async fn tokens(&self) -> Result<ChatGptTokens, String> {
        self.0.credentials.lock().unwrap().chatgpt.clone().ok_or_else(|| "ChatGPT is not connected".to_string())
    }

    async fn store(&self, tokens: ChatGptTokens) -> Result<(), String> {
        self.0.update_credentials("chatgpt", |c| c.chatgpt = Some(tokens)).map_err(|e| e.to_string())
    }
}

/// Reads Grok tokens from the account's credentials and hands refreshed ones to every Device.
pub struct AppGrokTokenSource(pub Arc<App>);

#[async_trait]
impl GrokTokenSource for AppGrokTokenSource {
    async fn tokens(&self) -> Result<GrokTokens, String> {
        self.0.credentials.lock().unwrap().grok.clone().ok_or_else(|| "Grok is not connected".to_string())
    }

    async fn store(&self, tokens: GrokTokens) -> Result<(), String> {
        self.0.update_credentials("grok", |c| c.grok = Some(tokens)).map_err(|e| e.to_string())
    }
}

/// Whether the model takes image content parts. A text-only model rejects the whole request,
/// so a Runner sends it the attachment's path alone.
pub fn supports_vision(kind: &str, model: Option<&str>) -> bool {
    let model = model.map(str::trim).filter(|m| !m.is_empty()).unwrap_or_else(|| default_model(kind));
    if let Some(info) = models::find(kind, model) {
        return info.images;
    }
    let model = model.to_ascii_lowercase();
    match kind {
        "chatgpt" | "anthropic" => true,
        "grok" => !model.contains("build"),
        "deepseek" => model.contains("vl") || model.contains("vision"),
        "opencode" | "opencode-go" => {
            model.contains("claude")
                || model.contains("gpt")
                || model.contains("gemini")
                || model.contains("grok")
                || model.contains("kimi")
                || model.contains("vision")
                || model.contains("qwen")
                || model.contains("glm")
        }
        _ => false,
    }
}

/// The model a bot of `kind` runs without one of its own.
pub fn default_model(kind: &str) -> &'static str {
    match kind {
        "deepseek" => lorca_agent::providers::openai_compat::DEEPSEEK_DEFAULT_MODEL,
        "anthropic" => lorca_agent::providers::anthropic::ANTHROPIC_DEFAULT_MODEL,
        "chatgpt" => lorca_agent::providers::chatgpt::CHATGPT_DEFAULT_MODEL,
        "grok" => lorca_agent::providers::grok::GROK_DEFAULT_MODEL,
        "opencode" => OPENCODE_DEFAULT_MODEL,
        "opencode-go" => OPENCODE_GO_DEFAULT_MODEL,
        _ => "",
    }
}

/// A bot's thinking level as stored, or nothing for the provider's default.
pub fn thinking_level(bot: &crate::model::Bot) -> Option<ThinkingLevel> {
    bot.thinking.as_deref().and_then(|s| s.parse().ok())
}

pub fn provider_for(app: &Arc<App>, kind: &str, model: Option<&str>, thinking: Option<ThinkingLevel>) -> Result<Arc<dyn Provider>, String> {
    let model = model.map(str::trim).filter(|m| !m.is_empty()).map(str::to_string);
    match kind {
        "deepseek" => {
            let key = app
                .credentials
                .lock()
                .unwrap()
                .deepseek
                .clone()
                .ok_or_else(|| "DeepSeek is not connected".to_string())?;
            let model = model.or_else(|| std::env::var("LORCA_DEEPSEEK_MODEL").ok());
            // The Anthropic-compatible endpoint: the one with DeepSeek's server-side web search.
            let base_url = deepseek_anthropic_url(&key.base_url.clone().unwrap_or_else(deepseek_base_url));
            Ok(Arc::new(AnthropicProvider::deepseek(&key.api_key, model.as_deref()).with_base_url(&base_url).with_thinking(thinking)))
        }
        "anthropic" => {
            let key = app
                .credentials
                .lock()
                .unwrap()
                .anthropic
                .clone()
                .ok_or_else(|| "Anthropic is not connected".to_string())?;
            let model = model.or_else(|| std::env::var("LORCA_ANTHROPIC_MODEL").ok());
            let base_url = key.base_url.clone().unwrap_or_else(anthropic_base_url);
            Ok(Arc::new(AnthropicProvider::anthropic(&key.api_key, model.as_deref()).with_base_url(&base_url).with_thinking(thinking)))
        }
        "opencode" => {
            let key = app
                .credentials
                .lock()
                .unwrap()
                .opencode
                .clone()
                .ok_or_else(|| "OpenCode Zen is not connected".to_string())?;
            let model = model.or_else(|| std::env::var("LORCA_OPENCODE_MODEL").ok()).unwrap_or_else(|| OPENCODE_DEFAULT_MODEL.into());
            let root = key.base_url.clone().or_else(|| env_url("LORCA_OPENCODE_BASE_URL")).unwrap_or_else(|| OPENCODE_BASE_URL.into());
            opencode_provider("opencode", &root, &key.api_key, &model, thinking)
        }
        "opencode-go" => {
            let key = app
                .credentials
                .lock()
                .unwrap()
                .opencode_go
                .clone()
                .ok_or_else(|| "OpenCode Go is not connected".to_string())?;
            let model = model.or_else(|| std::env::var("LORCA_OPENCODE_GO_MODEL").ok()).unwrap_or_else(|| OPENCODE_GO_DEFAULT_MODEL.into());
            let root = key
                .base_url
                .clone()
                .or_else(|| env_url("LORCA_OPENCODE_GO_BASE_URL"))
                .unwrap_or_else(|| OPENCODE_GO_BASE_URL.into());
            opencode_provider("opencode-go", &root, &key.api_key, &model, thinking)
        }
        "chatgpt" => {
            if app.credentials.lock().unwrap().chatgpt.is_none() {
                return Err("ChatGPT is not connected".into());
            }
            let model = model.or_else(|| std::env::var("LORCA_CHATGPT_MODEL").ok());
            Ok(Arc::new(ChatGptProvider::new(Arc::new(AppTokenSource(app.clone())), model.as_deref()).with_thinking(thinking)))
        }
        "grok" => {
            if app.credentials.lock().unwrap().grok.is_none() {
                return Err("Grok is not connected".into());
            }
            let model = model.or_else(|| std::env::var("LORCA_GROK_MODEL").ok());
            let mut provider = GrokProvider::new(Arc::new(AppGrokTokenSource(app.clone())), model.as_deref()).with_thinking(thinking);
            if let Some(base_url) = env_url("LORCA_GROK_BASE_URL") {
                provider = provider.with_base_url(&base_url);
            }
            if let Some(issuer) = env_url("LORCA_GROK_ISSUER") {
                provider = provider.with_issuer(&issuer);
            }
            Ok(Arc::new(provider))
        }
        other => Err(format!("Unknown provider {other}")),
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum OpenCodeWire {
    ChatCompletions,
    Messages,
    Responses,
    Unsupported,
}

/// OpenCode publishes the wire protocol beside every model. Zen and Go differ for MiniMax,
/// while their GPT, Grok, and Muse families use Responses and their Qwen family uses Messages.
fn opencode_wire(kind: &str, model: &str) -> OpenCodeWire {
    let model = model.to_ascii_lowercase();
    if (kind == "opencode" && model.starts_with("gemini-")) || model.starts_with("jev-") {
        return OpenCodeWire::Unsupported;
    }
    if model.starts_with("gpt-") || model.starts_with("grok-") || model.starts_with("muse-spark-") {
        return OpenCodeWire::Responses;
    }
    if model.starts_with("qwen") || (kind == "opencode-go" && model.starts_with("minimax-")) || model.starts_with("claude-") {
        return OpenCodeWire::Messages;
    }
    OpenCodeWire::ChatCompletions
}

/// Removes an optional `/v1` from a custom OpenCode root; each adapter adds the path its wire
/// protocol needs.
fn opencode_root(root: &str) -> &str {
    root.trim_end_matches('/').strip_suffix("/v1").unwrap_or_else(|| root.trim_end_matches('/'))
}

fn opencode_provider(
    kind: &str,
    root: &str,
    api_key: &str,
    model: &str,
    thinking: Option<ThinkingLevel>,
) -> Result<Arc<dyn Provider>, String> {
    let root = opencode_root(root);
    let provider: Arc<dyn Provider> = match opencode_wire(kind, model) {
        OpenCodeWire::ChatCompletions => {
            let mut provider = OpenAiCompatProvider::new(kind, &format!("{root}/v1"), api_key, model).with_thinking(thinking);
            provider.supports_images = supports_vision(kind, Some(model));
            Arc::new(provider)
        }
        OpenCodeWire::Messages => {
            let mut provider = AnthropicProvider::new(kind, root, api_key, model).with_thinking(thinking);
            provider.supports_images = supports_vision(kind, Some(model));
            provider.eager_tool_streaming = false;
            provider.max_tokens = 32_000;
            Arc::new(provider)
        }
        OpenCodeWire::Responses => Arc::new(OpenAiResponsesProvider::new(kind, &format!("{root}/v1"), api_key, model).with_thinking(thinking)),
        OpenCodeWire::Unsupported => {
            return Err(format!("{model} uses an OpenCode endpoint Lorca does not support"));
        }
    };
    Ok(opencode_headers(provider))
}

/// DeepSeek's API root when the credential has none: `LORCA_DEEPSEEK_BASE_URL` (a proxy or a
/// test server) or DeepSeek itself.
fn deepseek_base_url() -> String {
    env_url("LORCA_DEEPSEEK_BASE_URL").unwrap_or_else(|| lorca_agent::providers::openai_compat::DEEPSEEK_BASE_URL.to_string())
}

/// The Anthropic-compatible endpoint under a DeepSeek API root. A root given with its
/// `/anthropic` path already is used as is.
fn deepseek_anthropic_url(root: &str) -> String {
    let root = root.trim_end_matches('/');
    if root.ends_with("/anthropic") {
        root.to_string()
    } else {
        format!("{root}/anthropic")
    }
}

/// Anthropic's API root when the credential has none: `LORCA_ANTHROPIC_BASE_URL` or
/// Anthropic itself.
fn anthropic_base_url() -> String {
    env_url("LORCA_ANTHROPIC_BASE_URL").unwrap_or_else(|| ANTHROPIC_BASE_URL.to_string())
}

fn env_url(name: &str) -> Option<String> {
    std::env::var(name).ok().filter(|s| !s.trim().is_empty()).map(|s| s.trim_end_matches('/').to_string())
}

/// A base URL the user typed: blank means the default; otherwise an http(s) root without a
/// trailing slash.
fn custom_base_url(base_url: Option<&str>) -> Result<Option<String>, String> {
    let Some(url) = base_url.map(str::trim).filter(|u| !u.is_empty()) else { return Ok(None) };
    if !url.starts_with("http://") && !url.starts_with("https://") {
        return Err("The base URL must start with http:// or https://".into());
    }
    Ok(Some(url.trim_end_matches('/').to_string()))
}

/// Checks a DeepSeek key against the API (the given root, or DeepSeek's) before saving both.
pub async fn connect_deepseek(app: &Arc<App>, api_key: &str, base_url: Option<&str>) -> Result<(), String> {
    let key = api_key.trim();
    if key.is_empty() {
        return Err("Paste a DeepSeek API key".into());
    }
    let base_url = custom_base_url(base_url)?;
    let root = base_url.clone().unwrap_or_else(deepseek_base_url);
    let request = app.http.get(format!("{}/models", root.trim_end_matches("/anthropic"))).bearer_auth(key);
    check_key("DeepSeek", request).await?;
    save_api_key(app, "deepseek", key, base_url)
}

/// Checks an Anthropic key against the API (the given root, or Anthropic's) before saving both.
pub async fn connect_anthropic(app: &Arc<App>, api_key: &str, base_url: Option<&str>) -> Result<(), String> {
    let key = api_key.trim();
    if key.is_empty() {
        return Err("Paste an Anthropic API key".into());
    }
    let base_url = custom_base_url(base_url)?;
    let root = base_url.clone().unwrap_or_else(anthropic_base_url);
    let request = app.http.get(format!("{root}/v1/models")).header("x-api-key", key).header("anthropic-version", ANTHROPIC_VERSION);
    check_key("Anthropic", request).await?;
    save_api_key(app, "anthropic", key, base_url)
}

/// Checks an OpenCode Zen key. The model catalog is public, so the account check uses Go's
/// authenticated usage endpoint; a valid Zen key without a Go subscription answers 403 after
/// authentication and is still a valid Zen credential.
pub async fn connect_opencode(app: &Arc<App>, api_key: &str, base_url: Option<&str>) -> Result<(), String> {
    let key = api_key.trim();
    if key.is_empty() {
        return Err("Paste an OpenCode Zen API key".into());
    }
    let base_url = custom_base_url(base_url)?;
    let configured = base_url.clone().or_else(|| env_url("LORCA_OPENCODE_BASE_URL"));
    let root = configured.as_deref().map(opencode_root).unwrap_or(OPENCODE_BASE_URL);
    if root == OPENCODE_BASE_URL {
        let request = app.http.get(format!("{root}/go/v1/usage")).bearer_auth(key);
        check_opencode_key("OpenCode Zen", request, false).await?;
    } else {
        check_key("OpenCode Zen", app.http.get(format!("{root}/v1/models")).bearer_auth(key)).await?;
    }
    save_api_key(app, "opencode", key, base_url)
}

/// Checks both the OpenCode key and its Go entitlement against the authenticated usage route.
pub async fn connect_opencode_go(app: &Arc<App>, api_key: &str, base_url: Option<&str>) -> Result<(), String> {
    let key = api_key.trim();
    if key.is_empty() {
        return Err("Paste an OpenCode Go API key".into());
    }
    let base_url = custom_base_url(base_url)?;
    let root = base_url
        .clone()
        .or_else(|| env_url("LORCA_OPENCODE_GO_BASE_URL"))
        .unwrap_or_else(|| OPENCODE_GO_BASE_URL.into());
    let root = opencode_root(&root);
    if root == OPENCODE_GO_BASE_URL {
        check_opencode_key("OpenCode Go", app.http.get(format!("{root}/v1/usage")).bearer_auth(key), true).await?;
    } else {
        check_key("OpenCode Go", app.http.get(format!("{root}/v1/models")).bearer_auth(key)).await?;
    }
    save_api_key(app, "opencode-go", key, base_url)
}

async fn check_key(name: &str, request: reqwest::RequestBuilder) -> Result<(), String> {
    let response = request.send().await.map_err(|e| format!("{name} unreachable: {e}"))?;
    match response.status() {
        reqwest::StatusCode::UNAUTHORIZED | reqwest::StatusCode::FORBIDDEN => Err(format!("{name} rejected that key")),
        status if status.is_success() => Ok(()),
        status => Err(format!("{name} answered {status}")),
    }
}

async fn check_opencode_key(name: &str, request: reqwest::RequestBuilder, requires_go: bool) -> Result<(), String> {
    let response = request.send().await.map_err(|e| format!("{name} unreachable: {e}"))?;
    match response.status() {
        reqwest::StatusCode::UNAUTHORIZED => Err(format!("{name} rejected that key")),
        reqwest::StatusCode::FORBIDDEN if requires_go => Err("OpenCode Go needs an active subscription".into()),
        reqwest::StatusCode::FORBIDDEN => Ok(()),
        status if status.is_success() => Ok(()),
        status => Err(format!("{name} answered {status}")),
    }
}

fn save_api_key(app: &Arc<App>, kind: &str, key: &str, base_url: Option<String>) -> Result<(), String> {
    let credential = Some(ApiKeyCredential { api_key: key.to_string(), base_url, connected_at: config::now_unix() });
    let update = |credentials: &mut Credentials| match kind {
        "deepseek" => credentials.deepseek = credential,
        "anthropic" => credentials.anthropic = credential,
        "opencode" => credentials.opencode = credential,
        "opencode-go" => credentials.opencode_go = credential,
        _ => unreachable!(),
    };
    app.update_credentials(kind, update).map_err(|e| e.to_string())
}

/// Opens the browser for the ChatGPT sign-in and waits for the callback.
pub async fn connect_chatgpt(app: &Arc<App>) -> Result<ChatGptTokens, String> {
    let tokens = lorca_agent::providers::chatgpt::oauth::login(
        &app.http,
        |url| open::that(url).map_err(|e| format!("Cannot open the browser: {e}")),
        std::time::Duration::from_secs(5 * 60),
    )
    .await?;
    app.update_credentials("chatgpt", |c| c.chatgpt = Some(tokens.clone())).map_err(|e| e.to_string())?;
    Ok(tokens)
}

/// Opens the browser for the Grok sign-in (xAI's OAuth at `auth.x.ai`, or `LORCA_GROK_ISSUER`
/// for a test server) and waits for the loopback callback.
pub async fn connect_grok(app: &Arc<App>) -> Result<GrokTokens, String> {
    let endpoints = env_url("LORCA_GROK_ISSUER").map(|issuer| grok_oauth::Endpoints::at(&issuer)).unwrap_or_else(grok_oauth::Endpoints::xai);
    let tokens = grok_oauth::login(
        &app.http,
        &endpoints,
        |url| open::that(url).map_err(|e| format!("Cannot open the browser: {e}")),
        std::time::Duration::from_secs(5 * 60),
    )
    .await?;
    app.update_credentials("grok", |c| c.grok = Some(tokens.clone())).map_err(|e| e.to_string())?;
    Ok(tokens)
}

/// Disconnects `kind` for the whole account: every Device drops the credential.
pub fn disconnect(app: &Arc<App>, kind: &str) -> Result<(), String> {
    if !PROVIDER_KINDS.contains(&kind) {
        return Err(format!("Unknown provider {kind}"));
    }
    app.update_credentials(kind, |credentials| {
        match kind {
            "deepseek" => credentials.deepseek = None,
            "anthropic" => credentials.anthropic = None,
            "opencode" => credentials.opencode = None,
            "opencode-go" => credentials.opencode_go = None,
            "chatgpt" => credentials.chatgpt = None,
            "grok" => {
                // Tell xAI the sign-in is over; the removal stands either way.
                if let Some(tokens) = credentials.grok.take() {
                    let http = app.http.clone();
                    let endpoints = env_url("LORCA_GROK_ISSUER").map(|issuer| grok_oauth::Endpoints::at(&issuer)).unwrap_or_else(grok_oauth::Endpoints::xai);
                    tokio::spawn(async move {
                        if let Err(error) = grok_oauth::revoke(&http, &endpoints, &tokens.refresh_token).await {
                            tracing::debug!("grok revoke: {error}");
                        }
                    });
                }
            }
            _ => unreachable!(),
        }
    })
    .map_err(|e| e.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Mutex;

    struct CaptureProvider(Arc<Mutex<Option<ModelRequest>>>);

    #[async_trait]
    impl Provider for CaptureProvider {
        fn provider_id(&self) -> &str {
            "capture"
        }

        fn model_id(&self) -> &str {
            "capture"
        }

        async fn stream(&self, request: ModelRequest, _cancel: CancellationToken) -> AssistantEventStream {
            *self.0.lock().unwrap() = Some(request);
            Box::pin(futures::stream::empty())
        }
    }

    #[test]
    fn opencode_models_use_their_published_wire_protocols() {
        assert_eq!(opencode_wire("opencode", "gpt-5.6-terra"), OpenCodeWire::Responses);
        assert_eq!(opencode_wire("opencode", "claude-sonnet-5"), OpenCodeWire::Messages);
        assert_eq!(opencode_wire("opencode", "deepseek-v4.1-flash"), OpenCodeWire::ChatCompletions);
        assert_eq!(opencode_wire("opencode", "gemini-3.8-flash"), OpenCodeWire::Unsupported);
        assert_eq!(opencode_wire("opencode-go", "minimax-m3"), OpenCodeWire::Messages);
        assert_eq!(opencode_wire("opencode-go", "grok-4.6"), OpenCodeWire::Responses);
    }

    #[test]
    fn defaults_are_the_first_catalog_models_and_roots_accept_v1() {
        assert_eq!(models::for_provider("opencode")[0].id, OPENCODE_DEFAULT_MODEL);
        assert_eq!(models::for_provider("opencode-go")[0].id, OPENCODE_GO_DEFAULT_MODEL);
        assert_eq!(opencode_root("https://opencode.ai/zen/v1/"), OPENCODE_BASE_URL);
        assert_eq!(opencode_root("https://opencode.ai/zen"), OPENCODE_BASE_URL);
    }

    #[tokio::test]
    async fn opencode_requests_identify_lorca_and_carry_the_conversation() {
        let seen = Arc::new(Mutex::new(None));
        let provider = opencode_headers(Arc::new(CaptureProvider(seen.clone())));
        let request = ModelRequest {
            system_prompt: String::new(),
            messages: Vec::new(),
            tools: Vec::new(),
            max_tokens: None,
            options: lorca_agent::RequestOptions::default().with_session_id("chat-1"),
        };
        let _ = provider.stream(request, CancellationToken::new()).await;
        let request = seen.lock().unwrap().take().unwrap();
        assert_eq!(request.options.headers.get("User-Agent").map(String::as_str), Some(USER_AGENT));
        assert_eq!(request.options.headers.get("x-opencode-session").map(String::as_str), Some("chat-1"));
    }
}
