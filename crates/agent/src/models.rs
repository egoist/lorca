//! What is known about a model ahead of time: its context window and output cap, whether it
//! reasons or sees images, how it is asked to think, and what its tokens cost. A snapshot of
//! models.dev (fetched 2026-09-20) for the models Lorca offers. A model that is not listed
//! runs with no window, no levels beyond the provider's default, and zero cost.

use crate::types::{Cost, ThinkingLevel, Usage};

/// Dollars per million tokens.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Rates {
    pub input: f64,
    pub output: f64,
    pub cache_read: f64,
    pub cache_write: f64,
}

/// Rates that apply once a request's input tokens exceed a size.
#[derive(Debug, Clone, Copy, PartialEq)]
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
    /// `thinking: { type: "enabled", budget_tokens }`; `Off` sends no thinking.
    Budget,
    /// A reasoning effort word (`reasoning_effort`, `reasoning.effort`).
    Effort,
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
}

impl ModelInfo {
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

use ThinkingLevel::{High, Low, Max, Medium, Minimal, Off, XHigh};

const NO_TIERS: &[CostTier] = &[];
const ADAPTIVE_LEVELS: &[ThinkingLevel] = &[Off, Minimal, Low, Medium, High, XHigh, Max];
/// Fable's thinking is always on.
const ALWAYS_ON_LEVELS: &[ThinkingLevel] = &[Minimal, Low, Medium, High, XHigh, Max];
const BUDGET_LEVELS: &[ThinkingLevel] = &[Off, Minimal, Low, Medium, High];
const DEEPSEEK_LEVELS: &[ThinkingLevel] = &[Off, Low, Medium, High, XHigh, Max];
const CODEX_LEVELS: &[ThinkingLevel] = &[Low, Medium, High, XHigh];
/// The Grok models that take `reasoning.effort`; the others reason on their own.
const GROK_LEVELS: &[ThinkingLevel] = &[Low, Medium, High];
const GROK_XHIGH_LEVELS: &[ThinkingLevel] = &[Low, Medium, High, XHigh];
const FULL_EFFORT_LEVELS: &[ThinkingLevel] = &[Off, Low, Medium, High, XHigh, Max];
const ALWAYS_EFFORT_LEVELS: &[ThinkingLevel] = &[Low, Medium, High, XHigh, Max];
const LOW_HIGH_MAX_LEVELS: &[ThinkingLevel] = &[Low, High, Max];
const MAX_ONLY_LEVELS: &[ThinkingLevel] = &[Max];
const QWEN_LEVELS: &[ThinkingLevel] = &[Off, Low, Medium, XHigh];
const NO_LEVELS: &[ThinkingLevel] = &[];

const fn rates(input: f64, output: f64, cache_read: f64, cache_write: f64) -> Rates {
    Rates { input, output, cache_read, cache_write }
}

macro_rules! tier_272k {
    ($input:expr, $output:expr, $cache_read:expr, $cache_write:expr) => {{
        const TIERS: &[CostTier] = &[CostTier { input_tokens_above: 272_000, rates: rates($input, $output, $cache_read, $cache_write) }];
        TIERS
    }};
}

macro_rules! tier_200k {
    ($input:expr, $output:expr, $cache_read:expr, $cache_write:expr) => {{
        const TIERS: &[CostTier] = &[CostTier { input_tokens_above: 200_000, rates: rates($input, $output, $cache_read, $cache_write) }];
        TIERS
    }};
}

macro_rules! tier_512k {
    ($input:expr, $output:expr, $cache_read:expr, $cache_write:expr) => {{
        const TIERS: &[CostTier] = &[CostTier { input_tokens_above: 512_000, rates: rates($input, $output, $cache_read, $cache_write) }];
        TIERS
    }};
}

pub const MODELS: &[ModelInfo] = &[
    // DeepSeek
    ModelInfo {
        id: "deepseek-flash",
        name: "DeepSeek V4.1 Flash",
        provider: "deepseek",
        context_window: 1_000_000,
        max_output: 384_000,
        reasoning: true,
        images: true,
        rates: rates(0.15, 0.6, 0.003, 0.0),
        tiers: NO_TIERS,
        thinking: ThinkingMode::Adaptive,
        levels: DEEPSEEK_LEVELS,
    },
    ModelInfo {
        id: "deepseek-v4-pro",
        name: "DeepSeek V4 Pro",
        provider: "deepseek",
        context_window: 1_000_000,
        max_output: 384_000,
        reasoning: true,
        images: false,
        rates: rates(0.435, 0.87, 0.003625, 0.0),
        tiers: NO_TIERS,
        thinking: ThinkingMode::Adaptive,
        levels: DEEPSEEK_LEVELS,
    },
    // Anthropic
    ModelInfo {
        id: "claude-opus-5",
        name: "Claude Opus 5",
        provider: "anthropic",
        context_window: 1_000_000,
        max_output: 128_000,
        reasoning: true,
        images: true,
        rates: rates(5.0, 25.0, 0.5, 6.25),
        tiers: NO_TIERS,
        thinking: ThinkingMode::Adaptive,
        levels: ADAPTIVE_LEVELS,
    },
    ModelInfo {
        id: "claude-sonnet-5",
        name: "Claude Sonnet 5",
        provider: "anthropic",
        context_window: 1_000_000,
        max_output: 128_000,
        reasoning: true,
        images: true,
        rates: rates(2.0, 10.0, 0.2, 2.5),
        tiers: NO_TIERS,
        thinking: ThinkingMode::Adaptive,
        levels: ADAPTIVE_LEVELS,
    },
    ModelInfo {
        id: "claude-fable-5-1",
        name: "Claude Fable 5.1",
        provider: "anthropic",
        context_window: 1_000_000,
        max_output: 128_000,
        reasoning: true,
        images: true,
        rates: rates(10.0, 50.0, 0.25, 12.5),
        tiers: NO_TIERS,
        thinking: ThinkingMode::Adaptive,
        levels: ALWAYS_ON_LEVELS,
    },
    ModelInfo {
        id: "claude-opus-4-8",
        name: "Claude Opus 4.8",
        provider: "anthropic",
        context_window: 1_000_000,
        max_output: 128_000,
        reasoning: true,
        images: true,
        rates: rates(5.0, 25.0, 0.5, 6.25),
        tiers: NO_TIERS,
        thinking: ThinkingMode::Adaptive,
        levels: ADAPTIVE_LEVELS,
    },
    ModelInfo {
        id: "claude-haiku-4-5",
        name: "Claude Haiku 4.5",
        provider: "anthropic",
        context_window: 200_000,
        max_output: 64_000,
        reasoning: true,
        images: true,
        rates: rates(1.0, 5.0, 0.1, 1.25),
        tiers: NO_TIERS,
        thinking: ThinkingMode::Budget,
        levels: BUDGET_LEVELS,
    },
    // ChatGPT sign-ins: the subscription is not billed per token; these are the API rates, so
    // a turn's cost still says what the work was worth.
    ModelInfo {
        id: "gpt-5.6-terra",
        name: "GPT-5.6 Terra",
        provider: "chatgpt",
        context_window: 1_050_000,
        max_output: 128_000,
        reasoning: true,
        images: true,
        rates: rates(2.0, 12.0, 0.2, 2.5),
        tiers: tier_272k!(4.0, 18.0, 0.4, 5.0),
        thinking: ThinkingMode::Effort,
        levels: CODEX_LEVELS,
    },
    ModelInfo {
        id: "gpt-6-astra",
        name: "GPT-6 Astra",
        provider: "chatgpt",
        context_window: 1_050_000,
        max_output: 128_000,
        reasoning: true,
        images: true,
        rates: rates(10.0, 50.0, 1.0, 12.5),
        tiers: tier_272k!(20.0, 75.0, 2.0, 25.0),
        thinking: ThinkingMode::Effort,
        levels: CODEX_LEVELS,
    },
    ModelInfo {
        id: "gpt-5.6-sol",
        name: "GPT-5.6 Sol",
        provider: "chatgpt",
        context_window: 1_050_000,
        max_output: 128_000,
        reasoning: true,
        images: true,
        rates: rates(4.0, 20.0, 0.4, 5.0),
        tiers: tier_272k!(8.0, 30.0, 0.8, 10.0),
        thinking: ThinkingMode::Effort,
        levels: CODEX_LEVELS,
    },
    ModelInfo {
        id: "gpt-5.6-luna",
        name: "GPT-5.6 Luna",
        provider: "chatgpt",
        context_window: 1_050_000,
        max_output: 128_000,
        reasoning: true,
        images: true,
        rates: rates(0.2, 1.2, 0.02, 0.25),
        tiers: tier_272k!(0.4, 1.8, 0.04, 0.5),
        thinking: ThinkingMode::Effort,
        levels: CODEX_LEVELS,
    },
    ModelInfo {
        id: "gpt-5.5",
        name: "GPT-5.5",
        provider: "chatgpt",
        context_window: 1_050_000,
        max_output: 128_000,
        reasoning: true,
        images: true,
        rates: rates(5.0, 30.0, 0.5, 0.0),
        tiers: tier_272k!(10.0, 45.0, 1.0, 0.0),
        thinking: ThinkingMode::Effort,
        levels: CODEX_LEVELS,
    },
    ModelInfo {
        id: "gpt-5.3-codex-spark",
        name: "GPT-5.3 Codex Spark",
        provider: "chatgpt",
        context_window: 128_000,
        max_output: 32_000,
        reasoning: true,
        images: true,
        rates: rates(1.75, 14.0, 0.175, 0.0),
        tiers: NO_TIERS,
        thinking: ThinkingMode::Effort,
        levels: CODEX_LEVELS,
    },
    // Grok sign-ins: a SuperGrok or X Premium+ subscription, not billed per token; these are
    // xAI's API rates (docs.x.ai, 2026-09), so a turn's cost still says what the work was worth.
    ModelInfo {
        id: "grok-4.6",
        name: "Grok 4.6",
        provider: "grok",
        context_window: 500_000,
        max_output: 64_000,
        reasoning: true,
        images: true,
        rates: rates(2.0, 6.0, 0.5, 0.0),
        tiers: tier_200k!(4.0, 12.0, 1.0, 0.0),
        thinking: ThinkingMode::Effort,
        levels: GROK_LEVELS,
    },
    ModelInfo {
        id: "grok-4.5",
        name: "Grok 4.5",
        provider: "grok",
        context_window: 500_000,
        max_output: 64_000,
        reasoning: true,
        images: true,
        rates: rates(2.0, 6.0, 0.3, 0.0),
        tiers: tier_200k!(4.0, 12.0, 0.6, 0.0),
        thinking: ThinkingMode::Effort,
        levels: GROK_LEVELS,
    },
    ModelInfo {
        id: "grok-4.3",
        name: "Grok 4.3",
        provider: "grok",
        context_window: 1_000_000,
        max_output: 64_000,
        reasoning: true,
        images: true,
        rates: rates(1.25, 2.5, 0.2, 0.0),
        tiers: tier_200k!(2.5, 5.0, 0.4, 0.0),
        thinking: ThinkingMode::Effort,
        levels: GROK_LEVELS,
    },
    ModelInfo {
        id: "grok-4.20-0309-reasoning",
        name: "Grok 4.20 Reasoning",
        provider: "grok",
        context_window: 1_000_000,
        max_output: 64_000,
        reasoning: true,
        images: true,
        rates: rates(1.25, 2.5, 0.2, 0.0),
        tiers: tier_200k!(2.5, 5.0, 0.4, 0.0),
        thinking: ThinkingMode::Effort,
        levels: NO_LEVELS,
    },
    ModelInfo {
        id: "grok-build-0.1",
        name: "Grok Build 0.1",
        provider: "grok",
        context_window: 256_000,
        max_output: 64_000,
        reasoning: true,
        images: false,
        rates: rates(1.0, 2.0, 0.2, 0.0),
        tiers: tier_200k!(2.0, 4.0, 0.4, 0.0),
        thinking: ThinkingMode::Effort,
        levels: NO_LEVELS,
    },
    // OpenCode Zen. Its gateway routes each model to Responses, Messages, or Chat
    // Completions; the catalog still presents them as one provider.
    ModelInfo {
        id: "deepseek-v4.1-flash",
        name: "DeepSeek V4.1 Flash",
        provider: "opencode",
        context_window: 1_000_000,
        max_output: 384_000,
        reasoning: true,
        images: true,
        rates: rates(0.3, 1.2, 0.006, 0.0),
        tiers: NO_TIERS,
        thinking: ThinkingMode::Effort,
        levels: LOW_HIGH_MAX_LEVELS,
    },
    ModelInfo {
        id: "claude-sonnet-5",
        name: "Claude Sonnet 5",
        provider: "opencode",
        context_window: 1_000_000,
        max_output: 128_000,
        reasoning: true,
        images: true,
        rates: rates(2.0, 10.0, 0.2, 2.5),
        tiers: NO_TIERS,
        thinking: ThinkingMode::Adaptive,
        levels: ALWAYS_EFFORT_LEVELS,
    },
    ModelInfo {
        id: "gpt-5.6-terra",
        name: "GPT-5.6 Terra",
        provider: "opencode",
        context_window: 1_050_000,
        max_output: 128_000,
        reasoning: true,
        images: true,
        rates: rates(2.5, 15.0, 0.25, 3.125),
        tiers: tier_272k!(5.0, 22.5, 0.5, 6.25),
        thinking: ThinkingMode::Effort,
        levels: FULL_EFFORT_LEVELS,
    },
    ModelInfo {
        id: "grok-4.6",
        name: "Grok 4.6",
        provider: "opencode",
        context_window: 500_000,
        max_output: 500_000,
        reasoning: true,
        images: true,
        rates: rates(2.0, 6.0, 0.5, 0.0),
        tiers: tier_200k!(4.0, 12.0, 1.0, 0.0),
        thinking: ThinkingMode::Effort,
        levels: GROK_XHIGH_LEVELS,
    },
    ModelInfo {
        id: "kimi-k3",
        name: "Kimi K3",
        provider: "opencode",
        context_window: 1_048_576,
        max_output: 131_072,
        reasoning: true,
        images: true,
        rates: rates(3.0, 15.0, 0.3, 0.0),
        tiers: NO_TIERS,
        thinking: ThinkingMode::Effort,
        levels: MAX_ONLY_LEVELS,
    },
    ModelInfo {
        id: "big-pickle",
        name: "Big Pickle",
        provider: "opencode",
        context_window: 200_000,
        max_output: 32_000,
        reasoning: true,
        images: false,
        rates: rates(0.0, 0.0, 0.0, 0.0),
        tiers: NO_TIERS,
        thinking: ThinkingMode::Effort,
        levels: NO_LEVELS,
    },
    // OpenCode Go. The first entry is Lorca's default for the subscription.
    ModelInfo {
        id: "glm-5.3-flash",
        name: "GLM-5.3 Flash",
        provider: "opencode-go",
        context_window: 1_000_000,
        max_output: 131_072,
        reasoning: true,
        images: true,
        rates: rates(0.15, 0.5, 0.03, 0.0),
        tiers: NO_TIERS,
        thinking: ThinkingMode::Effort,
        levels: LOW_HIGH_MAX_LEVELS,
    },
    ModelInfo {
        id: "deepseek-v4.1-flash",
        name: "DeepSeek V4.1 Flash",
        provider: "opencode-go",
        context_window: 1_000_000,
        max_output: 384_000,
        reasoning: true,
        images: true,
        rates: rates(0.15, 0.6, 0.003, 0.0),
        tiers: NO_TIERS,
        thinking: ThinkingMode::Effort,
        levels: LOW_HIGH_MAX_LEVELS,
    },
    ModelInfo {
        id: "gpt-5.6-luna",
        name: "GPT-5.6 Luna",
        provider: "opencode-go",
        context_window: 1_050_000,
        max_output: 128_000,
        reasoning: true,
        images: true,
        rates: rates(0.2, 1.2, 0.02, 0.25),
        tiers: tier_272k!(0.4, 1.8, 0.04, 0.5),
        thinking: ThinkingMode::Effort,
        levels: FULL_EFFORT_LEVELS,
    },
    ModelInfo {
        id: "grok-4.6",
        name: "Grok 4.6",
        provider: "opencode-go",
        context_window: 500_000,
        max_output: 500_000,
        reasoning: true,
        images: true,
        rates: rates(2.0, 6.0, 0.5, 0.0),
        tiers: tier_200k!(4.0, 12.0, 1.0, 0.0),
        thinking: ThinkingMode::Effort,
        levels: GROK_XHIGH_LEVELS,
    },
    ModelInfo {
        id: "kimi-k3",
        name: "Kimi K3",
        provider: "opencode-go",
        context_window: 1_048_576,
        max_output: 131_072,
        reasoning: true,
        images: true,
        rates: rates(3.0, 15.0, 0.3, 0.0),
        tiers: NO_TIERS,
        thinking: ThinkingMode::Effort,
        levels: MAX_ONLY_LEVELS,
    },
    ModelInfo {
        id: "qwen3.8-flash",
        name: "Qwen3.8 Flash",
        provider: "opencode-go",
        context_window: 1_000_000,
        max_output: 131_072,
        reasoning: true,
        images: true,
        rates: rates(0.15, 0.47, 0.016, 0.2),
        tiers: NO_TIERS,
        thinking: ThinkingMode::Effort,
        levels: QWEN_LEVELS,
    },
    ModelInfo {
        id: "minimax-m3",
        name: "MiniMax M3",
        provider: "opencode-go",
        context_window: 1_000_000,
        max_output: 131_072,
        reasoning: true,
        images: false,
        rates: rates(0.3, 1.2, 0.06, 0.0),
        tiers: tier_512k!(0.6, 2.4, 0.12, 0.0),
        thinking: ThinkingMode::Effort,
        levels: NO_LEVELS,
    },
];

/// The catalog entry for a model of a provider: an exact id, or a dated variant of one
/// (`claude-haiku-4-5-20251001`).
pub fn find(provider: &str, model: &str) -> Option<&'static ModelInfo> {
    MODELS
        .iter()
        .find(|m| m.provider == provider && m.id == model)
        .or_else(|| MODELS.iter().find(|m| m.provider == provider && model.starts_with(m.id) && model[m.id.len()..].starts_with('-')))
}

/// The models a provider offers, in the catalog's order (the first is the default).
pub fn for_provider(provider: &str) -> Vec<&'static ModelInfo> {
    MODELS.iter().filter(|m| m.provider == provider).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cost_follows_the_rates_and_the_tier() {
        let terra = find("chatgpt", "gpt-5.6-terra").unwrap();
        let usage = Usage { input: 1_000_000, output: 100_000, cache_read: 0, cache_write: 0, ..Usage::default() };
        let cost = terra.cost_of(&usage);
        // Above 272k input tokens the whole request is at the long-context rate.
        assert!((cost.input - 4.0).abs() < 1e-9, "{cost:?}");
        assert!((cost.output - 1.8).abs() < 1e-9);
        assert!((cost.total - 5.8).abs() < 1e-9);

        let small = Usage { input: 1_000, output: 1_000, cache_read: 10_000, cache_write: 0, ..Usage::default() };
        let cost = terra.cost_of(&small);
        assert!((cost.input - 0.002).abs() < 1e-9);
        assert!((cost.cache_read - 0.002).abs() < 1e-9);
    }

    #[test]
    fn dated_ids_and_unknown_models() {
        assert_eq!(find("anthropic", "claude-haiku-4-5-20251001").map(|m| m.id), Some("claude-haiku-4-5"));
        assert!(find("anthropic", "claude-haiku-4").is_none());
        assert!(find("deepseek", "deepseek-chat").is_none());
        assert_eq!(for_provider("deepseek").first().map(|m| m.id), Some("deepseek-flash"));
        assert_eq!(for_provider("opencode").first().map(|m| m.id), Some("deepseek-v4.1-flash"));
        assert_eq!(for_provider("opencode-go").first().map(|m| m.id), Some("glm-5.3-flash"));
        assert_eq!(find("opencode-go", "qwen3.8-flash").map(|m| m.images), Some(true));
    }

    #[test]
    fn levels_clamp_to_what_the_model_takes() {
        let fable = find("anthropic", "claude-fable-5-1").unwrap();
        assert_eq!(fable.clamp_level(Off), Some(Minimal));
        let haiku = find("anthropic", "claude-haiku-4-5").unwrap();
        assert_eq!(haiku.clamp_level(XHigh), Some(High));
        assert_eq!(haiku.clamp_level(Off), Some(Off));
        let terra = find("chatgpt", "gpt-5.6-terra").unwrap();
        assert_eq!(terra.clamp_level(Max), Some(XHigh));
        assert_eq!(terra.clamp_level(Off), Some(Low));
    }
}
