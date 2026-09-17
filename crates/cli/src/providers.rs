//! Provider credentials for this Runner. They never sync.

use std::sync::Arc;

use async_trait::async_trait;

use tinybot_agent::providers::anthropic::{ANTHROPIC_BASE_URL, ANTHROPIC_VERSION};
use tinybot_agent::providers::grok::oauth as grok_oauth;
use tinybot_agent::providers::{AnthropicProvider, ChatGptProvider, ChatGptTokens, GrokProvider, GrokTokenSource, GrokTokens, TokenSource};
use tinybot_agent::{models, Provider, ThinkingLevel};

use crate::app::App;
use crate::config;

/// The provider kinds a Runner can hold, in the order the apps list them.
pub use crate::credentials::{ApiKeyCredential, Credentials, PROVIDER_KINDS};

/// Reads and refreshes ChatGPT tokens through the App's credential file.
pub struct AppTokenSource(pub Arc<App>);

#[async_trait]
impl TokenSource for AppTokenSource {
    async fn tokens(&self) -> Result<ChatGptTokens, String> {
        self.0.credentials.lock().unwrap().chatgpt.clone().ok_or_else(|| "ChatGPT is not connected on this Runner".to_string())
    }

    async fn store(&self, tokens: ChatGptTokens) -> Result<(), String> {
        self.0.credentials.lock().unwrap().chatgpt = Some(tokens);
        self.0.save_credentials().map_err(|e| e.to_string())
    }
}

/// Reads and refreshes Grok tokens through the App's credential file.
pub struct AppGrokTokenSource(pub Arc<App>);

#[async_trait]
impl GrokTokenSource for AppGrokTokenSource {
    async fn tokens(&self) -> Result<GrokTokens, String> {
        self.0.credentials.lock().unwrap().grok.clone().ok_or_else(|| "Grok is not connected on this Runner".to_string())
    }

    async fn store(&self, tokens: GrokTokens) -> Result<(), String> {
        self.0.credentials.lock().unwrap().grok = Some(tokens);
        self.0.save_credentials().map_err(|e| e.to_string())
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
        _ => false,
    }
}

/// The model a bot of `kind` runs without one of its own.
pub fn default_model(kind: &str) -> &'static str {
    match kind {
        "deepseek" => tinybot_agent::providers::openai_compat::DEEPSEEK_DEFAULT_MODEL,
        "anthropic" => tinybot_agent::providers::anthropic::ANTHROPIC_DEFAULT_MODEL,
        "chatgpt" => tinybot_agent::providers::chatgpt::CHATGPT_DEFAULT_MODEL,
        "grok" => tinybot_agent::providers::grok::GROK_DEFAULT_MODEL,
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
                .ok_or_else(|| "DeepSeek is not connected on this Runner".to_string())?;
            let model = model.or_else(|| std::env::var("TINYBOT_DEEPSEEK_MODEL").ok());
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
                .ok_or_else(|| "Anthropic is not connected on this Runner".to_string())?;
            let model = model.or_else(|| std::env::var("TINYBOT_ANTHROPIC_MODEL").ok());
            let base_url = key.base_url.clone().unwrap_or_else(anthropic_base_url);
            Ok(Arc::new(AnthropicProvider::anthropic(&key.api_key, model.as_deref()).with_base_url(&base_url).with_thinking(thinking)))
        }
        "chatgpt" => {
            if app.credentials.lock().unwrap().chatgpt.is_none() {
                return Err("ChatGPT is not connected on this Runner".into());
            }
            let model = model.or_else(|| std::env::var("TINYBOT_CHATGPT_MODEL").ok());
            Ok(Arc::new(ChatGptProvider::new(Arc::new(AppTokenSource(app.clone())), model.as_deref()).with_thinking(thinking)))
        }
        "grok" => {
            if app.credentials.lock().unwrap().grok.is_none() {
                return Err("Grok is not connected on this Runner".into());
            }
            let model = model.or_else(|| std::env::var("TINYBOT_GROK_MODEL").ok());
            let mut provider = GrokProvider::new(Arc::new(AppGrokTokenSource(app.clone())), model.as_deref()).with_thinking(thinking);
            if let Some(base_url) = env_url("TINYBOT_GROK_BASE_URL") {
                provider = provider.with_base_url(&base_url);
            }
            if let Some(issuer) = env_url("TINYBOT_GROK_ISSUER") {
                provider = provider.with_issuer(&issuer);
            }
            Ok(Arc::new(provider))
        }
        other => Err(format!("Unknown provider {other}")),
    }
}

/// DeepSeek's API root when the credential has none: `TINYBOT_DEEPSEEK_BASE_URL` (a proxy or a
/// test server) or DeepSeek itself.
fn deepseek_base_url() -> String {
    env_url("TINYBOT_DEEPSEEK_BASE_URL").unwrap_or_else(|| tinybot_agent::providers::openai_compat::DEEPSEEK_BASE_URL.to_string())
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

/// Anthropic's API root when the credential has none: `TINYBOT_ANTHROPIC_BASE_URL` or
/// Anthropic itself.
fn anthropic_base_url() -> String {
    env_url("TINYBOT_ANTHROPIC_BASE_URL").unwrap_or_else(|| ANTHROPIC_BASE_URL.to_string())
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

async fn check_key(name: &str, request: reqwest::RequestBuilder) -> Result<(), String> {
    let response = request.send().await.map_err(|e| format!("{name} unreachable: {e}"))?;
    match response.status() {
        reqwest::StatusCode::UNAUTHORIZED | reqwest::StatusCode::FORBIDDEN => Err(format!("{name} rejected that key")),
        status if status.is_success() => Ok(()),
        status => Err(format!("{name} answered {status}")),
    }
}

fn save_api_key(app: &Arc<App>, kind: &str, key: &str, base_url: Option<String>) -> Result<(), String> {
    let credential = Some(ApiKeyCredential { api_key: key.to_string(), base_url, connected_at: config::now_unix() });
    {
        let mut credentials = app.credentials.lock().unwrap();
        match kind {
            "deepseek" => credentials.deepseek = credential,
            "anthropic" => credentials.anthropic = credential,
            other => return Err(format!("Unknown provider {other}")),
        }
    }
    app.save_credentials().map_err(|e| e.to_string())?;
    app.push_machine_blob_if_changed();
    app.emit(app.roster_summary());
    Ok(())
}

/// Opens the browser for the ChatGPT sign-in and waits for the callback.
pub async fn connect_chatgpt(app: &Arc<App>) -> Result<ChatGptTokens, String> {
    let tokens = tinybot_agent::providers::chatgpt::oauth::login(
        &app.http,
        |url| open::that(url).map_err(|e| format!("Cannot open the browser: {e}")),
        std::time::Duration::from_secs(5 * 60),
    )
    .await?;
    app.credentials.lock().unwrap().chatgpt = Some(tokens.clone());
    app.save_credentials().map_err(|e| e.to_string())?;
    app.push_machine_blob_if_changed();
    app.emit(app.roster_summary());
    Ok(tokens)
}

/// Opens the browser for the Grok sign-in (xAI's OAuth at `auth.x.ai`, or `TINYBOT_GROK_ISSUER`
/// for a test server) and waits for the loopback callback.
pub async fn connect_grok(app: &Arc<App>) -> Result<GrokTokens, String> {
    let endpoints = env_url("TINYBOT_GROK_ISSUER").map(|issuer| grok_oauth::Endpoints::at(&issuer)).unwrap_or_else(grok_oauth::Endpoints::xai);
    let tokens = grok_oauth::login(
        &app.http,
        &endpoints,
        |url| open::that(url).map_err(|e| format!("Cannot open the browser: {e}")),
        std::time::Duration::from_secs(5 * 60),
    )
    .await?;
    app.credentials.lock().unwrap().grok = Some(tokens.clone());
    app.save_credentials().map_err(|e| e.to_string())?;
    app.push_machine_blob_if_changed();
    app.emit(app.roster_summary());
    Ok(tokens)
}

pub fn disconnect(app: &Arc<App>, kind: &str) -> Result<(), String> {
    {
        let mut credentials = app.credentials.lock().unwrap();
        match kind {
            "deepseek" => credentials.deepseek = None,
            "anthropic" => credentials.anthropic = None,
            "chatgpt" => credentials.chatgpt = None,
            "grok" => {
                // Tell xAI the sign-in is over; the local removal stands either way.
                if let Some(tokens) = credentials.grok.take() {
                    let http = app.http.clone();
                    let endpoints = env_url("TINYBOT_GROK_ISSUER").map(|issuer| grok_oauth::Endpoints::at(&issuer)).unwrap_or_else(grok_oauth::Endpoints::xai);
                    tokio::spawn(async move {
                        if let Err(error) = grok_oauth::revoke(&http, &endpoints, &tokens.refresh_token).await {
                            tracing::debug!("grok revoke: {error}");
                        }
                    });
                }
            }
            other => return Err(format!("Unknown provider {other}")),
        }
    }
    app.save_credentials().map_err(|e| e.to_string())?;
    app.push_machine_blob_if_changed();
    app.emit(app.roster_summary());
    Ok(())
}
