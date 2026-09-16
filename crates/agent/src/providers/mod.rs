//! Model adapters. Anthropic's Messages API serves Anthropic itself and DeepSeek, whose
//! Anthropic-compatible endpoint is the one with server-side web search; OpenAI-compatible chat
//! completions serves any server that implements it; ChatGPT is a subscription OAuth adapter
//! kept isolated in its own module.

pub mod anthropic;
pub mod chatgpt;
pub mod openai_compat;

pub use anthropic::AnthropicProvider;
pub use chatgpt::{ChatGptProvider, ChatGptTokens, TokenSource};
pub use openai_compat::OpenAiCompatProvider;
