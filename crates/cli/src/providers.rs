//! Provider credentials for this Runner. They never sync.

use std::sync::Arc;

use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use tinybot_agent::providers::anthropic::{ANTHROPIC_BASE_URL, ANTHROPIC_VERSION};
use tinybot_agent::providers::{AnthropicProvider, ChatGptProvider, ChatGptTokens, TokenSource};
use tinybot_agent::Provider;

use crate::app::App;
use crate::config::{self, Config};
use crate::model::ProviderStatus;

/// The provider kinds a Runner can hold, in the order the apps list them.
pub const PROVIDER_KINDS: [&str; 3] = ["deepseek", "anthropic", "chatgpt"];

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ApiKeyCredential {
    pub api_key: String,
    pub connected_at: i64,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Credentials {
    #[serde(default)]
    pub deepseek: Option<ApiKeyCredential>,
    #[serde(default)]
    pub anthropic: Option<ApiKeyCredential>,
    #[serde(default)]
    pub chatgpt: Option<ChatGptTokens>,
}

impl Credentials {
    pub fn load(config: &Config) -> Self {
        config::read_json(&config.credentials_path()).unwrap_or_default()
    }

    pub fn save(&self, config: &Config) -> anyhow::Result<()> {
        config::write_json_private(&config.credentials_path(), self)
    }

    fn api_key(&self, kind: &str) -> Option<&ApiKeyCredential> {
        match kind {
            "deepseek" => self.deepseek.as_ref(),
            "anthropic" => self.anthropic.as_ref(),
            _ => None,
        }
    }

    pub fn connected_kinds(&self) -> Vec<String> {
        self.statuses().into_iter().filter(|s| s.is_connected).map(|s| s.kind).collect()
    }

    pub fn statuses(&self) -> Vec<ProviderStatus> {
        PROVIDER_KINDS
            .iter()
            .map(|kind| {
                let detail = if *kind == "chatgpt" {
                    self.chatgpt.as_ref().map(|t| t.email.clone().unwrap_or_else(|| "Signed in".into()))
                } else {
                    self.api_key(kind).map(|c| mask_key(&c.api_key))
                };
                ProviderStatus {
                    kind: kind.to_string(),
                    is_connected: detail.is_some(),
                    detail: detail.unwrap_or_else(|| "Not connected".into()),
                }
            })
            .collect()
    }
}

pub fn mask_key(key: &str) -> String {
    let trimmed = key.trim();
    if trimmed.len() <= 8 {
        return "••••".into();
    }
    format!("{}…{}", &trimmed[..3], &trimmed[trimmed.len() - 4..])
}

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

/// Builds the provider for a bot from this Runner's credentials.
/// Whether the model takes image content parts. A text-only model rejects the whole request,
/// so a Runner sends it the attachment's path alone.
pub fn supports_vision(kind: &str, model: Option<&str>) -> bool {
    let model = model.unwrap_or_default().to_ascii_lowercase();
    match kind {
        "chatgpt" | "anthropic" => true,
        "deepseek" => model.contains("vl") || model.contains("vision"),
        _ => false,
    }
}

pub fn provider_for(app: &Arc<App>, kind: &str, model: Option<&str>) -> Result<Arc<dyn Provider>, String> {
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
            let base_url = format!("{}/anthropic", deepseek_base_url());
            Ok(Arc::new(AnthropicProvider::deepseek(&key.api_key, model.as_deref()).with_base_url(&base_url)))
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
            Ok(Arc::new(AnthropicProvider::anthropic(&key.api_key, model.as_deref()).with_base_url(&anthropic_base_url())))
        }
        "chatgpt" => {
            if app.credentials.lock().unwrap().chatgpt.is_none() {
                return Err("ChatGPT is not connected on this Runner".into());
            }
            let model = model.or_else(|| std::env::var("TINYBOT_CHATGPT_MODEL").ok());
            Ok(Arc::new(ChatGptProvider::new(Arc::new(AppTokenSource(app.clone())), model.as_deref())))
        }
        other => Err(format!("Unknown provider {other}")),
    }
}

/// `TINYBOT_DEEPSEEK_BASE_URL` points at a proxy or a test server: DeepSeek's API root, whose
/// `/anthropic` path is the endpoint bots use.
fn deepseek_base_url() -> String {
    env_url("TINYBOT_DEEPSEEK_BASE_URL").unwrap_or_else(|| tinybot_agent::providers::openai_compat::DEEPSEEK_BASE_URL.to_string())
}

/// `TINYBOT_ANTHROPIC_BASE_URL` points at a proxy or a test server.
fn anthropic_base_url() -> String {
    env_url("TINYBOT_ANTHROPIC_BASE_URL").unwrap_or_else(|| ANTHROPIC_BASE_URL.to_string())
}

fn env_url(name: &str) -> Option<String> {
    std::env::var(name).ok().filter(|s| !s.trim().is_empty()).map(|s| s.trim_end_matches('/').to_string())
}

/// Checks a DeepSeek key against the API before saving it.
pub async fn connect_deepseek(app: &Arc<App>, api_key: &str) -> Result<(), String> {
    let key = api_key.trim();
    if key.is_empty() {
        return Err("Paste a DeepSeek API key".into());
    }
    let request = app.http.get(format!("{}/models", deepseek_base_url())).bearer_auth(key);
    check_key("DeepSeek", request).await?;
    save_api_key(app, "deepseek", key)
}

/// Checks an Anthropic key against the API before saving it.
pub async fn connect_anthropic(app: &Arc<App>, api_key: &str) -> Result<(), String> {
    let key = api_key.trim();
    if key.is_empty() {
        return Err("Paste an Anthropic API key".into());
    }
    let request = app
        .http
        .get(format!("{}/v1/models", anthropic_base_url()))
        .header("x-api-key", key)
        .header("anthropic-version", ANTHROPIC_VERSION);
    check_key("Anthropic", request).await?;
    save_api_key(app, "anthropic", key)
}

async fn check_key(name: &str, request: reqwest::RequestBuilder) -> Result<(), String> {
    let response = request.send().await.map_err(|e| format!("{name} unreachable: {e}"))?;
    match response.status() {
        reqwest::StatusCode::UNAUTHORIZED | reqwest::StatusCode::FORBIDDEN => Err(format!("{name} rejected that key")),
        status if status.is_success() => Ok(()),
        status => Err(format!("{name} answered {status}")),
    }
}

fn save_api_key(app: &Arc<App>, kind: &str, key: &str) -> Result<(), String> {
    let credential = Some(ApiKeyCredential { api_key: key.to_string(), connected_at: config::now_unix() });
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

pub fn disconnect(app: &Arc<App>, kind: &str) -> Result<(), String> {
    {
        let mut credentials = app.credentials.lock().unwrap();
        match kind {
            "deepseek" => credentials.deepseek = None,
            "anthropic" => credentials.anthropic = None,
            "chatgpt" => credentials.chatgpt = None,
            other => return Err(format!("Unknown provider {other}")),
        }
    }
    app.save_credentials().map_err(|e| e.to_string())?;
    app.push_machine_blob_if_changed();
    app.emit(app.roster_summary());
    Ok(())
}
