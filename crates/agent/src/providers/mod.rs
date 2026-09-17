//! Model adapters. Anthropic's Messages API serves Anthropic itself and DeepSeek, whose
//! Anthropic-compatible endpoint is the one with server-side web search; OpenAI-compatible chat
//! completions serves any server that implements it; ChatGPT and Grok are subscription OAuth
//! adapters, each kept isolated in its own module, over the Responses wire shape they share.

pub mod anthropic;
pub mod chatgpt;
pub mod grok;
pub mod openai_compat;
pub(crate) mod responses;

pub use anthropic::AnthropicProvider;
pub use chatgpt::{ChatGptProvider, ChatGptTokens, TokenSource};
pub use grok::{GrokProvider, GrokTokenSource, GrokTokens};
pub use openai_compat::OpenAiCompatProvider;
