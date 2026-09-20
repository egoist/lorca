//! Model adapters. Anthropic's Messages API serves Anthropic itself and DeepSeek, whose
//! Anthropic-compatible endpoint is the one with server-side web search; OpenAI-compatible chat
//! completions and Responses serve API-key gateways; ChatGPT and Grok are subscription OAuth
//! adapters, each kept isolated in its own module, over the Responses wire shape they share.

pub mod anthropic;
pub mod chatgpt;
pub mod grok;
pub mod openai_compat;
pub mod openai_responses;
pub(crate) mod responses;

pub use anthropic::AnthropicProvider;
pub use chatgpt::{ChatGptProvider, ChatGptTokens, TokenSource};
pub use grok::{GrokProvider, GrokTokenSource, GrokTokens};
pub use openai_compat::OpenAiCompatProvider;
pub use openai_responses::OpenAiResponsesProvider;
