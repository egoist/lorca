//! Where a harness gets a provider for a model it is asked to run: a factory keyed by
//! provider id. [`EnvProviderFactory`] covers the built-in adapters with keys from the
//! environment; a host registers its own for anything else.

use std::collections::HashMap;
use std::sync::Arc;

use crate::providers::chatgpt::TokenSource;
use crate::providers::{AnthropicProvider, ChatGptProvider, OpenAiCompatProvider};
use crate::types::ThinkingLevel;
use crate::Provider;

/// A model by provider and id, as a harness names it.
#[derive(Debug, Clone, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize)]
pub struct ModelIdentity {
    pub provider: String,
    pub model: String,
}

impl ModelIdentity {
    pub fn new(provider: &str, model: &str) -> Self {
        ModelIdentity { provider: provider.into(), model: model.into() }
    }

    /// `provider/model`, or a bare model id when the provider is implied.
    pub fn parse(text: &str) -> Self {
        match text.split_once('/') {
            Some((provider, model)) => ModelIdentity::new(provider.trim(), model.trim()),
            None => ModelIdentity::new("", text.trim()),
        }
    }
}

impl std::fmt::Display for ModelIdentity {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}/{}", self.provider, self.model)
    }
}

/// Builds providers on demand, every time a harness switches model or thinking level.
pub trait ProviderFactory: Send + Sync {
    fn provider(&self, model: &ModelIdentity, thinking: Option<ThinkingLevel>) -> Result<Arc<dyn Provider>, String>;
}

type Builder = Arc<dyn Fn(&str, Option<ThinkingLevel>) -> Result<Arc<dyn Provider>, String> + Send + Sync>;

/// The built-in adapters with credentials from the environment:
///
/// | Provider | Key | Base URL override |
/// | --- | --- | --- |
/// | `deepseek` | `DEEPSEEK_API_KEY` | `DEEPSEEK_BASE_URL` (the API root; its `/anthropic` is used) |
/// | `anthropic` | `ANTHROPIC_API_KEY` | `ANTHROPIC_BASE_URL` |
/// | `openai` | `OPENAI_API_KEY` | `OPENAI_BASE_URL` (default `https://api.openai.com/v1`) |
/// | `chatgpt` | the [`TokenSource`] given to `with_chatgpt` | |
///
/// `register` adds or replaces a provider id with a builder of the host's own.
#[derive(Default)]
pub struct EnvProviderFactory {
    chatgpt: Option<Arc<dyn TokenSource>>,
    custom: HashMap<String, Builder>,
}

impl EnvProviderFactory {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn with_chatgpt(mut self, tokens: Arc<dyn TokenSource>) -> Self {
        self.chatgpt = Some(tokens);
        self
    }

    pub fn register(mut self, provider: &str, builder: impl Fn(&str, Option<ThinkingLevel>) -> Result<Arc<dyn Provider>, String> + Send + Sync + 'static) -> Self {
        self.custom.insert(provider.to_string(), Arc::new(builder));
        self
    }

    fn env(name: &str) -> Option<String> {
        std::env::var(name).ok().map(|v| v.trim().to_string()).filter(|v| !v.is_empty())
    }
}

impl ProviderFactory for EnvProviderFactory {
    fn provider(&self, model: &ModelIdentity, thinking: Option<ThinkingLevel>) -> Result<Arc<dyn Provider>, String> {
        if let Some(builder) = self.custom.get(&model.provider) {
            return builder(&model.model, thinking);
        }
        let model_id = (!model.model.is_empty()).then_some(model.model.as_str());
        match model.provider.as_str() {
            "deepseek" => {
                let key = Self::env("DEEPSEEK_API_KEY").ok_or("DEEPSEEK_API_KEY is not set")?;
                let mut provider = AnthropicProvider::deepseek(&key, model_id);
                if let Some(root) = Self::env("DEEPSEEK_BASE_URL") {
                    let root = root.trim_end_matches('/').to_string();
                    provider = provider.with_base_url(&if root.ends_with("/anthropic") { root } else { format!("{root}/anthropic") });
                }
                Ok(Arc::new(provider.with_thinking(thinking)))
            }
            "anthropic" => {
                let key = Self::env("ANTHROPIC_API_KEY").ok_or("ANTHROPIC_API_KEY is not set")?;
                let mut provider = AnthropicProvider::anthropic(&key, model_id);
                if let Some(base) = Self::env("ANTHROPIC_BASE_URL") {
                    provider = provider.with_base_url(&base);
                }
                Ok(Arc::new(provider.with_thinking(thinking)))
            }
            "openai" => {
                let key = Self::env("OPENAI_API_KEY").ok_or("OPENAI_API_KEY is not set")?;
                let base = Self::env("OPENAI_BASE_URL").unwrap_or_else(|| "https://api.openai.com/v1".into());
                let model_id = model_id.ok_or("an OpenAI model id is required")?;
                Ok(Arc::new(OpenAiCompatProvider::new("openai", &base, &key, model_id).with_thinking(thinking)))
            }
            "chatgpt" => {
                let tokens = self.chatgpt.clone().ok_or("ChatGPT needs a token source; give the factory one with with_chatgpt")?;
                Ok(Arc::new(ChatGptProvider::new(tokens, model_id).with_thinking(thinking)))
            }
            other => Err(format!("Unknown provider {other}")),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn identities_parse_and_print() {
        assert_eq!(ModelIdentity::parse("anthropic/claude-opus-5"), ModelIdentity::new("anthropic", "claude-opus-5"));
        assert_eq!(ModelIdentity::parse("deepseek-flash"), ModelIdentity::new("", "deepseek-flash"));
        assert_eq!(ModelIdentity::new("deepseek", "deepseek-flash").to_string(), "deepseek/deepseek-flash");
    }

    #[test]
    fn the_factory_builds_registered_and_built_in_providers() {
        let factory = EnvProviderFactory::new().register("scripted", |model, _| {
            Ok(Arc::new(AnthropicProvider::new("scripted", "http://localhost:1", "k", model)) as Arc<dyn Provider>)
        });
        let provider = factory.provider(&ModelIdentity::new("scripted", "m1"), None).unwrap();
        assert_eq!((provider.provider_id(), provider.model_id()), ("scripted", "m1"));
        assert!(factory.provider(&ModelIdentity::new("nope", "x"), None).is_err());
        match factory.provider(&ModelIdentity::new("chatgpt", "x"), None) {
            Err(error) => assert!(error.contains("token source")),
            Ok(_) => panic!("chatgpt needs tokens"),
        }
    }
}
