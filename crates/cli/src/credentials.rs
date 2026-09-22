//! The account's provider credentials: API keys with an optional base URL, and the ChatGPT
//! and Grok sign-ins. They travel as one `credentials` blob under the account DEK, so every
//! Device holds the same set in its private core folder; a Runner builds its providers from
//! them (`runner` feature).

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

use crate::config::{self, Config};
use crate::model::ProviderStatus;

#[cfg(feature = "provider-auth")]
pub use lorca_provider_auth::{chatgpt::ChatGptTokens, grok::GrokTokens};
/// Builds without provider setup carry subscription tokens as opaque JSON.
#[cfg(not(feature = "provider-auth"))]
pub type ChatGptTokens = serde_json::Value;
#[cfg(not(feature = "provider-auth"))]
pub type GrokTokens = serde_json::Value;

pub const PROVIDER_KINDS: [&str; 6] = ["deepseek", "anthropic", "opencode", "opencode-go", "chatgpt", "grok"];

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
    pub opencode: Option<ApiKeyCredential>,
    #[serde(default)]
    pub opencode_go: Option<ApiKeyCredential>,
    #[serde(default)]
    pub chatgpt: Option<ChatGptTokens>,
    #[serde(default)]
    pub grok: Option<GrokTokens>,
    /// When each kind last changed on any Device (connected, tokens refreshed, disconnected),
    /// in seconds. Two Devices' sets merge kind by kind, the later change winning, so a
    /// disconnect is an entry here with no credential beside it.
    #[serde(default)]
    pub changed_at: BTreeMap<String, f64>,
}

/// What `Credentials::merge` found.
#[derive(Debug, Default, PartialEq)]
pub struct Merge {
    /// Kinds taken from the other set.
    pub taken: Vec<String>,
    /// This set has a change the other lacks, so the other side needs this one.
    pub is_ahead: bool,
}

impl Credentials {
    pub fn load(config: &Config) -> Self {
        let mut credentials: Credentials = config::read_json(&config.credentials_path()).unwrap_or_default();
        // A credential with no change time has never been merged: it counts from now, once.
        let unstamped: Vec<String> = credentials.connected_kinds().into_iter().filter(|kind| !credentials.changed_at.contains_key(kind)).collect();
        if !unstamped.is_empty() {
            for kind in unstamped {
                credentials.touch(&kind);
            }
            if let Err(error) = credentials.save(config) {
                tracing::error!(%error, "saving credentials");
            }
        }
        credentials
    }

    /// Marks `kind` as changed now, after a connect, a token refresh, or a disconnect.
    pub fn touch(&mut self, kind: &str) {
        self.changed_at.insert(kind.to_string(), config::now_secs());
    }

    pub fn is_empty(&self) -> bool {
        self.changed_at.is_empty()
    }

    /// Takes every kind `other` changed later than this set did.
    pub fn merge(&mut self, other: &Credentials) -> Merge {
        let mut merge = Merge::default();
        for kind in PROVIDER_KINDS {
            let (ours, theirs) = (self.changed_at.get(kind).copied(), other.changed_at.get(kind).copied());
            match (ours, theirs) {
                (ours, Some(theirs)) if ours.is_none_or(|ours| theirs > ours) => {
                    match kind {
                        "deepseek" => self.deepseek = other.deepseek.clone(),
                        "anthropic" => self.anthropic = other.anthropic.clone(),
                        "opencode" => self.opencode = other.opencode.clone(),
                        "opencode-go" => self.opencode_go = other.opencode_go.clone(),
                        "chatgpt" => self.chatgpt = other.chatgpt.clone(),
                        "grok" => self.grok = other.grok.clone(),
                        _ => unreachable!(),
                    }
                    self.changed_at.insert(kind.to_string(), theirs);
                    merge.taken.push(kind.to_string());
                }
                (Some(ours), theirs) if theirs.is_none_or(|theirs| ours > theirs) => merge.is_ahead = true,
                _ => {}
            }
        }
        merge
    }

    pub fn save(&self, config: &Config) -> anyhow::Result<()> {
        config::write_json_private(&config.credentials_path(), self)
    }

    pub(crate) fn api_key(&self, kind: &str) -> Option<&ApiKeyCredential> {
        match kind {
            "deepseek" => self.deepseek.as_ref(),
            "anthropic" => self.anthropic.as_ref(),
            "opencode" => self.opencode.as_ref(),
            "opencode-go" => self.opencode_go.as_ref(),
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


#[cfg(feature = "provider-auth")]
fn chatgpt_email(tokens: &ChatGptTokens) -> Option<String> {
    tokens.email.clone()
}

#[cfg(not(feature = "provider-auth"))]
fn chatgpt_email(tokens: &ChatGptTokens) -> Option<String> {
    tokens["email"].as_str().map(str::to_string)
}

#[cfg(feature = "provider-auth")]
fn grok_email(tokens: &GrokTokens) -> Option<String> {
    tokens.email.clone()
}

#[cfg(not(feature = "provider-auth"))]
fn grok_email(tokens: &GrokTokens) -> Option<String> {
    tokens["email"].as_str().map(str::to_string)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn key(api_key: &str) -> Option<ApiKeyCredential> {
        Some(ApiKeyCredential { api_key: api_key.into(), base_url: None, connected_at: 0 })
    }

    #[test]
    fn merge_takes_the_later_change_of_each_kind() {
        let mut ours = Credentials { deepseek: key("old"), anthropic: key("ours"), ..Default::default() };
        ours.changed_at.insert("deepseek".into(), 1.0);
        ours.changed_at.insert("anthropic".into(), 5.0);
        let mut theirs = Credentials { deepseek: key("new"), ..Default::default() };
        theirs.changed_at.insert("deepseek".into(), 2.0);

        let merge = ours.merge(&theirs);
        assert_eq!(merge, Merge { taken: vec!["deepseek".into()], is_ahead: true });
        assert_eq!(ours.deepseek.as_ref().unwrap().api_key, "new");
        assert_eq!(ours.anthropic.as_ref().unwrap().api_key, "ours");

        // The same set again changes nothing and owes nothing.
        let again = ours.clone();
        assert_eq!(ours.merge(&again), Merge::default());
    }

    #[test]
    fn merge_carries_a_disconnect() {
        let mut ours = Credentials { deepseek: key("k"), ..Default::default() };
        ours.changed_at.insert("deepseek".into(), 1.0);
        let mut theirs = Credentials::default();
        theirs.changed_at.insert("deepseek".into(), 2.0);

        assert_eq!(ours.merge(&theirs).taken, vec!["deepseek".to_string()]);
        assert!(ours.deepseek.is_none());
        assert!(ours.connected_kinds().is_empty());
    }

    #[test]
    fn opencode_credentials_merge_and_report_in_provider_order() {
        let mut ours = Credentials { opencode: key("zen-old"), ..Default::default() };
        ours.changed_at.insert("opencode".into(), 1.0);
        let mut theirs = Credentials { opencode: key("zen-new"), opencode_go: key("go-key"), ..Default::default() };
        theirs.changed_at.insert("opencode".into(), 2.0);
        theirs.changed_at.insert("opencode-go".into(), 2.0);

        assert_eq!(ours.merge(&theirs).taken, vec!["opencode".to_string(), "opencode-go".to_string()]);
        assert_eq!(ours.opencode.as_ref().unwrap().api_key, "zen-new");
        assert_eq!(ours.opencode_go.as_ref().unwrap().api_key, "go-key");
        assert_eq!(ours.statuses().into_iter().map(|status| status.kind).collect::<Vec<_>>(), PROVIDER_KINDS);
    }
}
