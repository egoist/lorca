//! Model adapters. DeepSeek speaks OpenAI-compatible chat completions; ChatGPT is a
//! subscription OAuth adapter kept isolated in its own module.

pub mod chatgpt;
pub mod openai_compat;

pub use chatgpt::{ChatGptProvider, ChatGptTokens, TokenSource};
pub use openai_compat::OpenAiCompatProvider;
