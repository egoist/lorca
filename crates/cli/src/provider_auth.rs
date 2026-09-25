//! Connects and disconnects the account's provider credentials on any Device. API keys are
//! checked here before they enter the encrypted `credentials` blob. Subscription sign-ins
//! use the providers' PKCE loopback flows; a desktop opens the URL itself and a phone hands
//! it to its native in-app browser.

use std::sync::Arc;

use lorca_provider_auth::chatgpt::{self as chatgpt_oauth, ChatGptTokens};
use lorca_provider_auth::grok::{self as grok_oauth, GrokTokens};

use crate::app::App;
use crate::config;
use crate::credentials::{ApiKeyCredential, Credentials, PROVIDER_KINDS};

const ANTHROPIC_BASE_URL: &str = "https://api.anthropic.com";
const ANTHROPIC_VERSION: &str = "2023-06-01";
const OPENCODE_BASE_URL: &str = "https://opencode.ai/zen";
const OPENCODE_GO_BASE_URL: &str = "https://opencode.ai/zen/go";
const CEREBRAS_BASE_URL: &str = "https://api.cerebras.ai/v1";

fn env_url(name: &str) -> Option<String> {
    std::env::var(name).ok().filter(|s| !s.trim().is_empty()).map(|s| s.trim_end_matches('/').to_string())
}

fn deepseek_base_url() -> String {
    env_url("LORCA_DEEPSEEK_BASE_URL").unwrap_or_else(|| "https://api.deepseek.com".into())
}

fn cerebras_base_url() -> String {
    env_url("LORCA_CEREBRAS_BASE_URL").unwrap_or_else(|| CEREBRAS_BASE_URL.into())
}

fn anthropic_base_url() -> String {
    env_url("LORCA_ANTHROPIC_BASE_URL").unwrap_or_else(|| ANTHROPIC_BASE_URL.to_string())
}

/// Removes an optional `/v1` from a custom OpenCode root; the credential check adds the
/// endpoint it needs.
fn opencode_root(root: &str) -> &str {
    root.trim_end_matches('/').strip_suffix("/v1").unwrap_or_else(|| root.trim_end_matches('/'))
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

/// Checks a Cerebras key against the API (the given root, or Cerebras's) before saving both.
pub async fn connect_cerebras(app: &Arc<App>, api_key: &str, base_url: Option<&str>) -> Result<(), String> {
    let key = api_key.trim();
    if key.is_empty() {
        return Err("Paste a Cerebras API key".into());
    }
    let base_url = custom_base_url(base_url)?;
    let root = base_url.clone().unwrap_or_else(cerebras_base_url);
    check_key("Cerebras", app.http.get(format!("{root}/models")).bearer_auth(key)).await?;
    save_api_key(app, "cerebras", key, base_url)
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
        "cerebras" => credentials.cerebras = credential,
        _ => unreachable!(),
    };
    app.update_credentials(kind, update).map_err(|e| e.to_string())
}

/// Runs a ChatGPT sign-in, opening its authorization URL through the Device's UI.
pub async fn connect_chatgpt(
    app: &Arc<App>,
    open_url: impl FnOnce(&str) -> Result<(), String>,
) -> Result<ChatGptTokens, String> {
    let tokens = chatgpt_oauth::login(&app.http, open_url, std::time::Duration::from_secs(5 * 60)).await?;
    app.update_credentials("chatgpt", |c| c.chatgpt = Some(tokens.clone())).map_err(|e| e.to_string())?;
    Ok(tokens)
}

/// Runs a Grok sign-in, opening its authorization URL through the Device's UI.
pub async fn connect_grok(
    app: &Arc<App>,
    open_url: impl FnOnce(&str) -> Result<(), String>,
) -> Result<GrokTokens, String> {
    let endpoints = env_url("LORCA_GROK_ISSUER").map(|issuer| grok_oauth::Endpoints::at(&issuer)).unwrap_or_else(grok_oauth::Endpoints::xai);
    let tokens = grok_oauth::login(&app.http, &endpoints, open_url, std::time::Duration::from_secs(5 * 60)).await?;
    app.update_credentials("grok", |c| c.grok = Some(tokens.clone())).map_err(|e| e.to_string())?;
    Ok(tokens)
}

/// Disconnects `kind` for the whole account: every Device drops the credential.
pub fn disconnect(app: &Arc<App>, kind: &str) -> Result<(), String> {
    if !PROVIDER_KINDS.contains(&kind) {
        return Err(format!("Unknown provider {kind}"));
    }
    app.update_credentials(kind, |credentials| match kind {
        "deepseek" => credentials.deepseek = None,
        "anthropic" => credentials.anthropic = None,
        "opencode" => credentials.opencode = None,
        "opencode-go" => credentials.opencode_go = None,
        "cerebras" => credentials.cerebras = None,
        "chatgpt" => credentials.chatgpt = None,
        "grok" => {
            // Tell xAI the sign-in is over; the account-wide removal stands either way.
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
    })
    .map_err(|e| e.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn normalizes_custom_roots() {
        assert_eq!(custom_base_url(Some(" https://proxy.example/v1/ ")).unwrap(), Some("https://proxy.example/v1".into()));
        assert_eq!(custom_base_url(Some("  ")).unwrap(), None);
        assert_eq!(opencode_root("https://proxy.example/v1"), "https://proxy.example");
        assert!(custom_base_url(Some("proxy.example")).unwrap_err().contains("http://"));
    }
}
