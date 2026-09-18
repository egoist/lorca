//! The provider credentials a Runner keeps: API keys with an optional base URL, and the
//! ChatGPT and Grok sign-ins. Read on every Device for the roster's "connected" flags; used by the
//! `runner` feature to build providers.

use serde::{Deserialize, Serialize};

use crate::config::{self, Config};
use crate::model::ProviderStatus;

#[cfg(feature = "runner")]
pub use lorca_agent::providers::{ChatGptTokens, GrokTokens};
/// Without the runner the tokens are carried as they are and never used.
#[cfg(not(feature = "runner"))]
pub type ChatGptTokens = serde_json::Value;
#[cfg(not(feature = "runner"))]
pub type GrokTokens = serde_json::Value;

pub const PROVIDER_KINDS: [&str; 4] = ["deepseek", "anthropic", "chatgpt", "grok"];

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ApiKeyCredential {
    pub api_key: String,
    /// The API root to call instead of the provider's own: a proxy or a compatible server.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub base_url: Option<String>,
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
    #[serde(default)]
    pub grok: Option<GrokTokens>,
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
                    self.chatgpt.as_ref().map(|t| chatgpt_email(t).unwrap_or_else(|| "Signed in".into()))
                } else if *kind == "grok" {
                    self.grok.as_ref().map(|t| grok_email(t).unwrap_or_else(|| "Signed in".into()))
                } else {
                    self.api_key(kind).map(|c| match &c.base_url {
                        Some(base_url) => format!("{} · {base_url}", mask_key(&c.api_key)),
                        None => mask_key(&c.api_key),
                    })
                };
                ProviderStatus {
                    kind: kind.to_string(),
                    is_connected: detail.is_some(),
                    detail: detail.unwrap_or_else(|| "Not connected".into()),
                    base_url: self.api_key(kind).and_then(|c| c.base_url.clone()),
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


#[cfg(feature = "runner")]
fn chatgpt_email(tokens: &ChatGptTokens) -> Option<String> {
    tokens.email.clone()
}

#[cfg(not(feature = "runner"))]
fn chatgpt_email(tokens: &ChatGptTokens) -> Option<String> {
    tokens["email"].as_str().map(str::to_string)
}

#[cfg(feature = "runner")]
fn grok_email(tokens: &GrokTokens) -> Option<String> {
    tokens.email.clone()
}

#[cfg(not(feature = "runner"))]
fn grok_email(tokens: &GrokTokens) -> Option<String> {
    tokens["email"].as_str().map(str::to_string)
}
