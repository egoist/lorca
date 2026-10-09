//! What is known about a model ahead of time: its context window and output cap, whether it
//! reasons or sees images, how it is asked to think, and what its tokens cost. The catalog is
//! `catalog.json`, which every build carries and lorca.app serves, so a Device takes a newer one
//! without a new release (see [`install`]). A model that is not listed runs with no window, no
//! levels beyond the provider's default, and zero cost.
//!
//! Every Device has it, the phone included: the agent's adapters read it, and the snapshot each
//! app gets carries its models for the Model and Thinking pickers.

mod catalog;
mod types;

use serde::Deserialize;

pub use catalog::{install, parse, updated, Catalog, BUNDLED};
pub use types::{Cost, ThinkingLevel, Usage};

/// Dollars per million tokens.
#[derive(Debug, Clone, Copy, PartialEq, Deserialize)]
pub struct Rates {
    pub input: f64,
    pub output: f64,
    pub cache_read: f64,
    pub cache_write: f64,
}

/// Rates that apply once a request's input tokens exceed a size.
#[derive(Debug, Clone, Copy, PartialEq, Deserialize)]
pub struct CostTier {
    pub input_tokens_above: u64,
    pub rates: Rates,
}

/// How a model is asked to think.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ThinkingMode {
    /// `thinking: { type: "adaptive" }` with an `output_config.effort`; `Off` is
    /// `{ type: "disabled" }` when the model allows it.
    Adaptive,
    /// Adaptive thinking whose `Off` is `{ type: "between_tools" }` at the default effort, the
    /// lowest setting of a model that refuses `disabled` (Sonnet 5.5).
    AdaptiveBetweenTools,
    /// `thinking: { type: "enabled", budget_tokens }`; `Off` sends no thinking.
    Budget,
    /// A reasoning effort word (`reasoning_effort`, `reasoning.effort`). `Off`, on a model that
    /// lists it, is `thinking: { type: "disabled" }` on Chat Completions and the effort `none`
    /// on Responses.
    Effort,
}

impl ThinkingMode {
    const ALL: [ThinkingMode; 4] = [ThinkingMode::Adaptive, ThinkingMode::AdaptiveBetweenTools, ThinkingMode::Budget, ThinkingMode::Effort];

    /// Its name in the catalog.
    pub fn as_str(&self) -> &'static str {
        match self {
            ThinkingMode::Adaptive => "adaptive",
            ThinkingMode::AdaptiveBetweenTools => "adaptive-between-tools",
            ThinkingMode::Budget => "budget",
            ThinkingMode::Effort => "effort",
        }
    }
}

impl std::str::FromStr for ThinkingMode {
    type Err = String;
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        ThinkingMode::ALL.into_iter().find(|mode| mode.as_str() == s).ok_or_else(|| format!("Unknown thinking mode: {s}"))
    }
}

/// The wire protocol a provider serves a model on, for a gateway that publishes one per model
/// (OpenCode).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Wire {
    /// `/v1/chat/completions`
    ChatCompletions,
    /// `/v1/messages`
    Messages,
    /// `/v1/responses`
    Responses,
    /// `/v1/systemone`: a decision model, which answers typed questions with probabilities and
    /// writes no text, so Auto-review can run it and bots cannot.
    SystemOne,
}

impl Wire {
    const ALL: [Wire; 4] = [Wire::ChatCompletions, Wire::Messages, Wire::Responses, Wire::SystemOne];

    /// Its name in the catalog, the one custom providers use for their protocol.
    pub fn as_str(&self) -> &'static str {
        match self {
            Wire::ChatCompletions => "chat-completions",
            Wire::Messages => "messages",
            Wire::Responses => "responses",
            Wire::SystemOne => "system-one",
        }
    }
}

impl std::str::FromStr for Wire {
    type Err = String;
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        Wire::ALL.into_iter().find(|wire| wire.as_str() == s).ok_or_else(|| format!("Unknown wire protocol: {s}"))
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ModelInfo {
    pub id: &'static str,
    pub name: &'static str,
    pub provider: &'static str,
    pub context_window: u64,
    pub max_output: u64,
    pub reasoning: bool,
    pub images: bool,
    pub rates: Rates,
    pub tiers: &'static [CostTier],
    pub thinking: ThinkingMode,
    /// The levels the model takes, lowest first. A model whose thinking cannot be turned off
    /// lists no `Off`.
    pub levels: &'static [ThinkingLevel],
    /// The wire protocol its provider serves it on, for a provider whose models differ (OpenCode).
    pub wire: Option<Wire>,
}

impl ModelInfo {
    /// Whether it is a decision model, one that answers typed questions instead of chatting:
    /// Auto-review can run it, and no bot can.
    pub fn decides(&self) -> bool {
        self.wire == Some(Wire::SystemOne)
    }

    /// What a response cost, at the tier its input size lands in.
    pub fn cost_of(&self, usage: &Usage) -> Cost {
        let input_tokens = usage.input + usage.cache_read + usage.cache_write;
        let mut rates = self.rates;
        let mut matched = None;
        for tier in self.tiers {
            if input_tokens > tier.input_tokens_above && matched.is_none_or(|t| tier.input_tokens_above > t) {
                rates = tier.rates;
                matched = Some(tier.input_tokens_above);
            }
        }
        let per = |rate: f64, tokens: u64| rate / 1_000_000.0 * tokens as f64;
        let input = per(rates.input, usage.input);
        let output = per(rates.output, usage.output);
        let cache_read = per(rates.cache_read, usage.cache_read);
        let cache_write = per(rates.cache_write, usage.cache_write);
        Cost { input, output, cache_read, cache_write, total: input + output + cache_read + cache_write }
    }

    /// The level this model runs at when asked for `level`: itself when supported, else the
    /// nearest supported level above it, else the highest the model has.
    pub fn clamp_level(&self, level: ThinkingLevel) -> Option<ThinkingLevel> {
        if self.levels.is_empty() {
            return None;
        }
        if self.levels.contains(&level) {
            return Some(level);
        }
        ThinkingLevel::ALL
            .iter()
            .copied()
            .find(|candidate| *candidate > level && self.levels.contains(candidate))
            .or_else(|| self.levels.last().copied())
    }
}

/// Every model the catalog in use offers, in its order.
pub fn models() -> &'static [ModelInfo] {
    &catalog::current().models
}

/// The catalog entry for a model of a provider: an exact id, or a dated variant of one
/// (`claude-haiku-4-5-20251001`).
pub fn find(provider: &str, model: &str) -> Option<&'static ModelInfo> {
    let models = models();
    models
        .iter()
        .find(|m| m.provider == provider && m.id == model)
        .or_else(|| models.iter().find(|m| m.provider == provider && model.starts_with(m.id) && model[m.id.len()..].starts_with('-')))
}

/// The catalog entry for a model id under whichever provider offers it, for a server the
/// catalog does not know: a gateway's `vendor/` prefix is ignored, a dated variant finds its
/// model, and the first provider in catalog order that lists the id answers.
pub fn find_any(model: &str) -> Option<&'static ModelInfo> {
    let model = model.rsplit('/').next().unwrap_or(model);
    let dated = |m: &&ModelInfo| {
        model.strip_prefix(m.id).and_then(|rest| rest.strip_prefix('-')).is_some_and(|date| !date.is_empty() && date.chars().all(|c| c.is_ascii_digit()))
    };
    let models = models();
    models.iter().find(|m| m.id == model).or_else(|| models.iter().find(dated))
}

/// The models a bot of a provider can run, in the catalog's order (the first is the default):
/// every one it lists but its decision models.
pub fn for_provider(provider: &str) -> Vec<&'static ModelInfo> {
    models().iter().filter(|m| m.provider == provider && !m.decides()).collect()
}

/// The model a provider runs when a bot names none: the first it lists.
pub fn default_model(provider: &str) -> Option<&'static str> {
    for_provider(provider).first().map(|m| m.id)
}

/// The small, fast model Auto-review runs on for a provider.
pub fn review_model(provider: &str) -> Option<&'static str> {
    catalog::current().review.get(provider).map(String::as_str)
}

#[cfg(test)]
mod tests {
    use super::*;
    use ThinkingLevel::{High, Low, Max, Minimal, Off, XHigh};

    #[test]
    fn cost_follows_the_rates_and_the_tier() {
        let sol = find("chatgpt", "gpt-6-sol").unwrap();
        let usage = Usage { input: 1_000_000, output: 100_000, cache_read: 0, cache_write: 0, ..Usage::default() };
        let cost = sol.cost_of(&usage);
        // Above 272k input tokens the whole request is at the long-context rate.
        assert!((cost.input - 4.0).abs() < 1e-9, "{cost:?}");
        assert!((cost.output - 1.5).abs() < 1e-9);
        assert!((cost.total - 5.5).abs() < 1e-9);

        let small = Usage { input: 1_000, output: 1_000, cache_read: 10_000, cache_write: 0, ..Usage::default() };
        let cost = sol.cost_of(&small);
        assert!((cost.input - 0.002).abs() < 1e-9);
        assert!((cost.cache_read - 0.002).abs() < 1e-9);
    }

    #[test]
    fn dated_ids_and_unknown_models() {
        assert_eq!(find("anthropic", "claude-haiku-4-5-20251001").map(|m| m.id), Some("claude-haiku-4-5"));
        assert!(find("anthropic", "claude-haiku-4").is_none());
        assert!(find("deepseek", "deepseek-chat").is_none());
        assert_eq!(for_provider("deepseek").first().map(|m| m.id), Some("deepseek-flash"));
        assert_eq!(default_model("chatgpt"), Some("gpt-6.1-sol"));
        assert_eq!(default_model("opencode"), Some("deepseek-v4.1-flash"));
        assert_eq!(default_model("opencode-go"), Some("glm-5.3-flash"));
        assert_eq!(default_model("custom:lab"), None);
        assert_eq!(find("opencode-go", "qwen3.8-flash").map(|m| m.images), Some(true));
    }

    #[test]
    fn any_provider_knows_a_gateway_model() {
        assert_eq!(find_any("anthropic/claude-sonnet-5").map(|m| m.id), Some("claude-sonnet-5"));
        assert_eq!(find_any("claude-haiku-4-5-20251001").map(|m| m.id), Some("claude-haiku-4-5"));
        assert_eq!(find_any("moonshotai/kimi-k3").map(|m| m.provider), Some("opencode"));
        assert!(find_any("gpt-6-sol-mini").is_none(), "only a date extends an id");
        assert!(find_any("qwen3:8b").is_none());
    }

    #[test]
    fn levels_clamp_to_what_the_model_takes() {
        let fable = find("anthropic", "claude-fable-5-1").unwrap();
        assert_eq!(fable.clamp_level(Off), Some(Low));
        let opus = find("anthropic", "claude-opus-5-5").unwrap();
        assert_eq!(opus.clamp_level(Off), Some(Low));
        assert_eq!(opus.clamp_level(Minimal), Some(Low));
        assert_eq!(opus.clamp_level(Max), Some(Max));
        let haiku = find("anthropic", "claude-haiku-4-5").unwrap();
        assert_eq!(haiku.clamp_level(XHigh), Some(High));
        assert_eq!(haiku.clamp_level(Off), Some(Off));
        let sol = find("chatgpt", "gpt-6-sol").unwrap();
        assert_eq!(sol.clamp_level(Max), Some(Max));
        assert_eq!(sol.clamp_level(Off), Some(Low));
        assert_eq!(find("grok", "grok-4.7").unwrap().clamp_level(Max), Some(XHigh));
        // Zen can turn DeepSeek V4 Pro's thinking off; on Go it thinks at least at high.
        assert_eq!(find("opencode", "deepseek-v4-pro").unwrap().clamp_level(Off), Some(Off));
        assert_eq!(find("opencode-go", "deepseek-v4-pro").unwrap().clamp_level(Off), Some(High));
    }

    #[test]
    fn decision_models_are_for_auto_review_alone() {
        let jev = find("opencode", "jev-1.13").unwrap();
        assert!(jev.decides() && jev.levels.is_empty());
        assert!(models().iter().any(|m| m.provider == "opencode" && m.id == "jev-1.13-free"));
        assert!(!for_provider("opencode").iter().any(|m| m.decides()), "a bot cannot run a decision model");
        assert!(!find("opencode", "deepseek-v4.1-flash").unwrap().decides());
    }

    #[test]
    fn every_provider_has_a_review_model_it_lists() {
        for provider in ["deepseek", "anthropic", "chatgpt", "grok", "opencode", "opencode-go"] {
            let review = review_model(provider).unwrap_or_else(|| panic!("{provider} has no review model"));
            assert!(find(provider, review).is_some(), "{provider}/{review}");
        }
        assert_eq!(review_model("anthropic"), Some("claude-haiku-4-5"));
    }
}
