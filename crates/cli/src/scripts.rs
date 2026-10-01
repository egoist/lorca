//! What a turn's codemode scripts get from the Runner besides tools: the values they keep with
//! `store()`, and `models.ask()`, the small model of the bot's provider.

use std::collections::BTreeMap;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;

use async_trait::async_trait;
use futures::StreamExt;
use lorca_agent::codemode::{CodemodeStore, HostFunction, StoreWrites};
use lorca_agent::provider::AssistantAccumulator;
use lorca_agent::types::{LlmMessage, StopReason, UserMessage};
use lorca_agent::{ModelRequest, RequestOptions};
use serde_json::Value;
use tokio_util::sync::CancellationToken;

use crate::app::App;

/// Where a bot's scripts keep `store()` values: this Runner's database, per chat and bot, never
/// synced.
pub struct ScriptStore {
    pub app: Arc<App>,
    pub chat_id: String,
    pub bot_id: String,
}

impl CodemodeStore for ScriptStore {
    fn load(&self) -> BTreeMap<String, Value> {
        self.app.store.codemode_values(&self.chat_id, &self.bot_id).unwrap_or_else(|error| {
            tracing::warn!(%error, "reading a script's stored values");
            Default::default()
        })
    }

    fn save(&self, writes: &StoreWrites) {
        // A bot deleted while its script ran leaves nothing behind.
        if self.app.bot(&self.bot_id).is_none() {
            return;
        }
        if let Err(error) = self.app.store.save_codemode_writes(&self.chat_id, &self.bot_id, &writes.set, &writes.delete) {
            tracing::warn!(%error, "saving a script's stored values");
        }
    }
}

/// How many `models.ask()` calls answer at once, and in one turn at most.
const IN_FLIGHT: usize = 4;
const MAX_CALLS_PER_TURN: usize = 200;
const MAX_PROMPT_CHARS: usize = 32_000;
const DEFAULT_MAX_TOKENS: u64 = 1024;
const MAX_TOKENS: u64 = 4096;

/// `models.ask()`: a script asks the small model of the bot's provider, the one Auto-review
/// uses, one question at a time, to rate, sort, or summarize many items without the items
/// reaching the bot's own context. Each call's cost counts in the chat's spending.
pub struct ModelsAsk {
    app: Arc<App>,
    chat_id: String,
    provider: String,
    in_flight: tokio::sync::Semaphore,
    calls: AtomicUsize,
}

impl ModelsAsk {
    /// `None` for a provider with no small model to ask.
    pub fn new(app: &Arc<App>, chat_id: &str, provider: &str) -> Option<Self> {
        let (model, _) = crate::providers::review_model(app, provider);
        (!model.is_empty()).then(|| ModelsAsk { app: app.clone(), chat_id: chat_id.to_string(), provider: provider.to_string(), in_flight: tokio::sync::Semaphore::new(IN_FLIGHT), calls: AtomicUsize::new(0) })
    }
}

#[async_trait]
impl HostFunction for ModelsAsk {
    fn name(&self) -> &str {
        "models.ask"
    }

    fn description(&self) -> &str {
        "Ask a small, fast model one question and get its answer as text, to rate, sort, or summarize many items one by one without \
         reading them yourself. It sees only the prompt, not this chat. At most 4 answer at once and 200 per turn; each costs a little, \
         which counts in the chat's spending."
    }

    fn signature(&self) -> &str {
        "(prompt: string, options?: { system?: string; maxTokens?: number }): Promise<string>"
    }

    async fn call(&self, args: Vec<Value>, cancel: &CancellationToken) -> Result<Value, String> {
        let prompt = args.first().and_then(Value::as_str).filter(|prompt| !prompt.trim().is_empty()).ok_or("models.ask() expects a prompt string")?;
        let options = args.get(1).cloned().unwrap_or(Value::Null);
        let system = match options.get("system") {
            None | Some(Value::Null) => String::new(),
            Some(Value::String(system)) => system.clone(),
            Some(_) => return Err("models.ask() system must be a string".into()),
        };
        if prompt.chars().count() + system.chars().count() > MAX_PROMPT_CHARS {
            return Err(format!("models.ask() takes a prompt and system of at most {MAX_PROMPT_CHARS} characters together"));
        }
        let max_tokens = match options.get("maxTokens") {
            None | Some(Value::Null) => DEFAULT_MAX_TOKENS,
            Some(value) => value.as_u64().filter(|tokens| (1..=MAX_TOKENS).contains(tokens)).ok_or(format!("models.ask() maxTokens must be between 1 and {MAX_TOKENS}"))?,
        };
        if self.calls.fetch_add(1, Ordering::Relaxed) >= MAX_CALLS_PER_TURN {
            return Err(format!("models.ask() has answered {MAX_CALLS_PER_TURN} times this turn, the most it may"));
        }
        let _permit = tokio::select! {
            permit = self.in_flight.acquire() => permit.map_err(|error| error.to_string())?,
            _ = cancel.cancelled() => return Err("Stopped".into()),
        };

        let (model, thinking) = crate::providers::review_model(&self.app, &self.provider);
        let provider = crate::providers::provider_for(&self.app, &self.provider, Some(&model), thinking)?;
        let request = ModelRequest {
            system_prompt: system,
            messages: vec![LlmMessage::User(UserMessage::text(prompt))],
            tools: Vec::new(),
            cache_points: Vec::new(),
            max_tokens: Some(max_tokens),
            options: RequestOptions::default().with_session_id(&self.chat_id),
        };
        let mut stream = provider.stream(request, cancel.clone()).await;
        let mut acc = AssistantAccumulator::new(provider.provider_id(), provider.model_id());
        while let Some(event) = stream.next().await {
            acc.apply(&event);
        }
        let message = acc.finish(cancel.is_cancelled());
        self.app.add_side_usage(&self.chat_id, &message.usage);
        if matches!(message.stop_reason, StopReason::Aborted | StopReason::Error) {
            return Err(message.error_message.unwrap_or_else(|| "The model gave no answer".into()));
        }
        Ok(Value::String(message.text()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[tokio::test]
    async fn models_ask_checks_its_arguments_and_its_provider() {
        let home = std::env::temp_dir().join(format!("lorca-scripts-{}", uuid::Uuid::new_v4()));
        let app = App::load(crate::config::Config { home: home.clone(), port: 0 }).unwrap();
        assert!(ModelsAsk::new(&app, "chat", "unknown").is_none(), "a provider with no small model has no models.ask()");
        let ask = ModelsAsk::new(&app, "chat", "deepseek").unwrap();
        let cancel = CancellationToken::new();
        assert_eq!(ask.call(vec![], &cancel).await.unwrap_err(), "models.ask() expects a prompt string");
        assert!(ask.call(vec![json!("x".repeat(MAX_PROMPT_CHARS + 1))], &cancel).await.unwrap_err().contains("at most"));
        assert!(ask.call(vec![json!("hi"), json!({ "maxTokens": 0 })], &cancel).await.unwrap_err().contains("maxTokens"));
        assert!(ask.call(vec![json!("hi"), json!({ "system": 1 })], &cancel).await.unwrap_err().contains("system"));
        assert!(ask.call(vec![json!("hi"), json!({ "system": "x".repeat(MAX_PROMPT_CHARS) })], &cancel).await.unwrap_err().contains("together"));
        assert_eq!(ask.call(vec![json!("hi")], &cancel).await.unwrap_err(), "DeepSeek is not connected");
        ask.calls.store(MAX_CALLS_PER_TURN, Ordering::Relaxed);
        assert!(ask.call(vec![json!("hi")], &cancel).await.unwrap_err().contains("200 times"));
        drop(app);
        let _ = std::fs::remove_dir_all(home);
    }
}
