//! Runtime providers built from the account's credentials. Credential setup lives in
//! `provider_auth`, which is also linked by Devices that never run a bot.

use std::collections::HashMap;
use std::sync::{Arc, LazyLock, Mutex};

use async_trait::async_trait;

use lorca_agent::models::{ModelInfo, Rates, ThinkingMode, Wire};
use lorca_agent::providers::anthropic::ANTHROPIC_BASE_URL;
use lorca_agent::providers::{
    AnthropicProvider, ChatGptProvider, ChatGptTokens, GrokProvider, GrokTokenSource, GrokTokens, OpenAiCompatProvider,
    OpenAiResponsesProvider, TokenSource,
};
use lorca_agent::{models, AssistantEventStream, ModelRequest, Provider, ThinkingLevel};
use tokio_util::sync::CancellationToken;

use crate::app::App;
use crate::credentials::{is_custom, CustomApi, CustomProvider};
use crate::decisions::{Decider, Shape};

pub const OPENCODE_BASE_URL: &str = "https://opencode.ai/zen";
pub const OPENCODE_GO_BASE_URL: &str = "https://opencode.ai/zen/go";

const USER_AGENT: &str = concat!("lorca/", env!("CARGO_PKG_VERSION"));

/// Identifies Lorca to OpenCode and sends the session header it uses for routing and prompt
/// caching. The regular request options also send `x-session-affinity`.
struct OpenCodeHeaders {
    inner: Arc<dyn Provider>,
}

#[async_trait]
impl Provider for OpenCodeHeaders {
    fn provider_id(&self) -> &str {
        self.inner.provider_id()
    }

    fn model_id(&self) -> &str {
        self.inner.model_id()
    }

    fn supports_images(&self) -> bool {
        self.inner.supports_images()
    }

    fn model_info(&self) -> Option<&'static lorca_agent::models::ModelInfo> {
        self.inner.model_info()
    }

    async fn stream(&self, mut request: ModelRequest, cancel: CancellationToken) -> AssistantEventStream {
        request.options.headers.insert("User-Agent".into(), USER_AGENT.into());
        if let Some(session_id) = request.options.session_id.clone() {
            request.options.headers.insert("x-opencode-session".into(), session_id);
        }
        self.inner.stream(request, cancel).await
    }
}

fn opencode_headers(inner: Arc<dyn Provider>) -> Arc<dyn Provider> {
    Arc::new(OpenCodeHeaders { inner })
}

/// Reads ChatGPT tokens from the account's credentials and hands refreshed ones to every Device.
pub struct AppTokenSource(pub Arc<App>);

#[async_trait]
impl TokenSource for AppTokenSource {
    async fn tokens(&self) -> Result<ChatGptTokens, String> {
        self.0.credentials.lock().unwrap().chatgpt.clone().ok_or_else(|| "ChatGPT is not connected".to_string())
    }

    async fn store(&self, tokens: ChatGptTokens) -> Result<(), String> {
        self.0.update_credentials("chatgpt", |c| c.chatgpt = Some(tokens)).map_err(|e| e.to_string())
    }
}

/// Reads Grok tokens from the account's credentials and hands refreshed ones to every Device.
pub struct AppGrokTokenSource(pub Arc<App>);

#[async_trait]
impl GrokTokenSource for AppGrokTokenSource {
    async fn tokens(&self) -> Result<GrokTokens, String> {
        self.0.credentials.lock().unwrap().grok.clone().ok_or_else(|| "Grok is not connected".to_string())
    }

    async fn store(&self, tokens: GrokTokens) -> Result<(), String> {
        self.0.update_credentials("grok", |c| c.grok = Some(tokens)).map_err(|e| e.to_string())
    }
}

/// Whether the model takes image content parts. A text-only model rejects the whole request,
/// so a Runner sends it the attachment's path alone.
pub fn supports_vision(app: &App, kind: &str, model: Option<&str>) -> bool {
    if is_custom(kind) {
        let credentials = app.credentials.lock().unwrap();
        let Some(provider) = credentials.custom.get(kind) else { return false };
        let model = model.map(str::trim).filter(|m| !m.is_empty()).or_else(|| provider.models.first().map(|m| m.id.as_str()));
        return model.is_some_and(|model| custom_model_info(kind, provider, model).images);
    }
    built_in_vision(kind, model)
}

/// Whether a built-in provider's model takes images: the catalog's word, else a guess from the
/// model's name.
fn built_in_vision(kind: &str, model: Option<&str>) -> bool {
    let model = model.map(str::trim).filter(|m| !m.is_empty()).unwrap_or_else(|| default_model(kind));
    if let Some(info) = models::find(kind, model) {
        return info.images;
    }
    let model = model.to_ascii_lowercase();
    match kind {
        "chatgpt" | "anthropic" | "grok" => true,
        "deepseek" => model.contains("vl") || model.contains("vision"),
        "opencode" | "opencode-go" => {
            model.contains("claude")
                || model.contains("gpt")
                || model.contains("gemini")
                || model.contains("grok")
                || model.contains("kimi")
                || model.contains("vision")
                || model.contains("qwen")
                || model.contains("glm")
        }
        _ => false,
    }
}

/// The model a bot of `kind` runs without one of its own: the first the catalog lists for it.
pub fn default_model(kind: &str) -> &'static str {
    models::default_model(kind).unwrap_or_default()
}

/// The small, fast model of `kind` that `models.ask()` runs, and Auto-review unless the user
/// picked another, and how much it thinks: the catalog's `review` model for a built-in
/// provider, whatever the bot itself runs, and a custom provider's first model, the one the user
/// put at the top ([`Credentials::review_model`](crate::credentials::Credentials::review_model)),
/// at [`lowest_level`]. Empty when `kind` has none.
pub fn small_model(app: &App, kind: &str) -> (String, Option<ThinkingLevel>) {
    let model = app.credentials.lock().unwrap().review_model(kind).unwrap_or_default();
    let thinking = if model.is_empty() { None } else { lowest_level(kind, &model) };
    (model, thinking)
}

/// The least a model can think, for a quick answer: thinking off where the model allows it and
/// its lowest effort where it does not. A custom provider's model takes the catalog's lowest
/// level for a model the catalog knows and the server's default for any other.
fn lowest_level(kind: &str, model: &str) -> Option<ThinkingLevel> {
    if is_custom(kind) {
        return models::find_any(model).and_then(|known| known.levels.first().copied());
    }
    Some(models::find(kind, model).and_then(|info| info.levels.first().copied()).unwrap_or(ThinkingLevel::Off))
}

/// How Auto-review asks its model.
pub enum Reviewer {
    /// A chat model, at the level it thinks.
    Chat { provider: Arc<dyn Provider>, thinking: Option<ThinkingLevel> },
    /// A decision model, which picks among typed choices and writes nothing.
    Decides(Decider),
}

/// The model Auto-review runs for a bot on `kind`: the review model of the provider the user
/// picked in Auto-review's settings while it is connected, else of the bot's own provider. A
/// provider's review model is the one picked for it in Providers, else its [`small_model`]. A
/// chat model thinks the least it can.
pub fn reviewer(app: &Arc<App>, kind: &str) -> Result<Reviewer, String> {
    let review = app.auto_review();
    let connected = app.credentials.lock().unwrap().connected_kinds();
    let kind = review.provider.filter(|picked| connected.contains(picked)).unwrap_or_else(|| kind.to_string());
    let model = review.models.get(&kind).cloned().unwrap_or_else(|| small_model(app, &kind).0);
    if let Some(decider) = decider(app, &kind, &model)? {
        return Ok(Reviewer::Decides(decider));
    }
    let thinking = lowest_level(&kind, &model);
    Ok(Reviewer::Chat { provider: provider_for(app, &kind, Some(&model), thinking)?, thinking })
}

/// Where a decision model of `kind` is asked, or `None` for a model that chats: a custom
/// provider's decision API at its endpoint, or a decision model the catalog lists for OpenCode,
/// at the gateway's `/v1/systemone`.
fn decider(app: &App, kind: &str, model: &str) -> Result<Option<Decider>, String> {
    let credentials = app.credentials.lock().unwrap();
    if let Some(provider) = credentials.custom.get(kind) {
        let shape = match provider.api {
            CustomApi::SystemOne => Shape::SystemOne,
            CustomApi::Decisions => Shape::OpenAi,
            _ => return Ok(None),
        };
        let model = Some(model).filter(|m| !m.is_empty()).map(str::to_string).or_else(|| provider.models.first().map(|m| m.id.clone()));
        let model = model.ok_or_else(|| format!("{} has no models", provider.name))?;
        let decider = Decider { shape, url: provider.base_url.clone(), api_key: provider.api_key.clone(), model, headers: Vec::new(), session_header: None };
        return Ok(Some(decider));
    }
    if !matches!(kind, "opencode" | "opencode-go") || !models::find(kind, model).is_some_and(|info| info.decides()) {
        return Ok(None);
    }
    let (key, env, default) = match kind {
        "opencode" => (credentials.opencode.clone(), "LORCA_OPENCODE_BASE_URL", OPENCODE_BASE_URL),
        _ => (credentials.opencode_go.clone(), "LORCA_OPENCODE_GO_BASE_URL", OPENCODE_GO_BASE_URL),
    };
    let key = key.ok_or_else(|| format!("{} is not connected", credentials.label(kind)))?;
    let root = key.base_url.clone().or_else(|| env_url(env)).unwrap_or_else(|| default.into());
    Ok(Some(Decider {
        shape: Shape::SystemOne,
        url: format!("{}/v1/systemone", opencode_root(&root)),
        api_key: key.api_key,
        model: model.to_string(),
        headers: vec![("User-Agent".into(), USER_AGENT.into())],
        session_header: Some("x-opencode-session"),
    }))
}

/// A bot's thinking level as stored, or nothing for the provider's default.
pub fn thinking_level(bot: &crate::model::Bot) -> Option<ThinkingLevel> {
    bot.thinking.as_deref().and_then(|s| s.parse().ok())
}

pub fn provider_for(app: &Arc<App>, kind: &str, model: Option<&str>, thinking: Option<ThinkingLevel>) -> Result<Arc<dyn Provider>, String> {
    let model = model.map(str::trim).filter(|m| !m.is_empty()).map(str::to_string);
    match kind {
        kind if is_custom(kind) => {
            let credentials = app.credentials.lock().unwrap();
            let provider = credentials.custom.get(kind).ok_or_else(|| format!("{} is not connected", credentials.label(kind)))?;
            let model = model.or_else(|| provider.models.first().map(|m| m.id.clone())).ok_or_else(|| format!("{} has no models", provider.name))?;
            custom_provider(kind, provider, &model, thinking)
        }
        "deepseek" => {
            let key = app
                .credentials
                .lock()
                .unwrap()
                .deepseek
                .clone()
                .ok_or_else(|| "DeepSeek is not connected".to_string())?;
            let model = model.or_else(|| std::env::var("LORCA_DEEPSEEK_MODEL").ok()).unwrap_or_else(|| default_model(kind).into());
            // The Anthropic-compatible endpoint: the one with DeepSeek's server-side web search.
            let base_url = deepseek_anthropic_url(&key.base_url.clone().unwrap_or_else(deepseek_base_url));
            Ok(Arc::new(AnthropicProvider::deepseek(&key.api_key, Some(model.as_str())).with_base_url(&base_url).with_thinking(thinking)))
        }
        "anthropic" => {
            let key = app
                .credentials
                .lock()
                .unwrap()
                .anthropic
                .clone()
                .ok_or_else(|| "Anthropic is not connected".to_string())?;
            let model = model.or_else(|| std::env::var("LORCA_ANTHROPIC_MODEL").ok()).unwrap_or_else(|| default_model(kind).into());
            let base_url = key.base_url.clone().unwrap_or_else(anthropic_base_url);
            Ok(Arc::new(AnthropicProvider::anthropic(&key.api_key, Some(model.as_str())).with_base_url(&base_url).with_thinking(thinking)))
        }
        "opencode" => {
            let key = app
                .credentials
                .lock()
                .unwrap()
                .opencode
                .clone()
                .ok_or_else(|| "OpenCode Zen is not connected".to_string())?;
            let model = model.or_else(|| std::env::var("LORCA_OPENCODE_MODEL").ok()).unwrap_or_else(|| default_model(kind).into());
            let root = key.base_url.clone().or_else(|| env_url("LORCA_OPENCODE_BASE_URL")).unwrap_or_else(|| OPENCODE_BASE_URL.into());
            opencode_provider("opencode", &root, &key.api_key, &model, thinking)
        }
        "opencode-go" => {
            let key = app
                .credentials
                .lock()
                .unwrap()
                .opencode_go
                .clone()
                .ok_or_else(|| "OpenCode Go is not connected".to_string())?;
            let model = model.or_else(|| std::env::var("LORCA_OPENCODE_GO_MODEL").ok()).unwrap_or_else(|| default_model(kind).into());
            let root = key
                .base_url
                .clone()
                .or_else(|| env_url("LORCA_OPENCODE_GO_BASE_URL"))
                .unwrap_or_else(|| OPENCODE_GO_BASE_URL.into());
            opencode_provider("opencode-go", &root, &key.api_key, &model, thinking)
        }
        "chatgpt" => {
            if app.credentials.lock().unwrap().chatgpt.is_none() {
                return Err("ChatGPT is not connected".into());
            }
            let model = model.or_else(|| std::env::var("LORCA_CHATGPT_MODEL").ok()).unwrap_or_else(|| default_model(kind).into());
            Ok(Arc::new(ChatGptProvider::new(Arc::new(AppTokenSource(app.clone())), Some(model.as_str())).with_thinking(thinking)))
        }
        "grok" => {
            if app.credentials.lock().unwrap().grok.is_none() {
                return Err("Grok is not connected".into());
            }
            let model = model.or_else(|| std::env::var("LORCA_GROK_MODEL").ok()).unwrap_or_else(|| default_model(kind).into());
            let mut provider = GrokProvider::new(Arc::new(AppGrokTokenSource(app.clone())), Some(model.as_str())).with_thinking(thinking);
            if let Some(base_url) = env_url("LORCA_GROK_BASE_URL") {
                provider = provider.with_base_url(&base_url);
            }
            if let Some(issuer) = env_url("LORCA_GROK_ISSUER") {
                provider = provider.with_issuer(&issuer);
            }
            Ok(Arc::new(provider))
        }
        other => Err(format!("Unknown provider {other}")),
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum OpenCodeWire {
    ChatCompletions,
    Messages,
    Responses,
    /// A decision model, which Auto-review asks and no bot runs.
    Decides,
    Unsupported,
}

/// OpenCode publishes the wire protocol beside every model, and the catalog carries it. A model
/// the catalog lacks goes by its family: Zen and Go differ for MiniMax and for Qwen3.8 Max,
/// which Zen serves on Chat Completions, while their GPT, Grok, and Muse families use Responses
/// and the rest of their Qwen family uses Messages.
fn opencode_wire(kind: &str, model: &str) -> OpenCodeWire {
    match models::find(kind, model).and_then(|info| info.wire) {
        Some(Wire::ChatCompletions) => return OpenCodeWire::ChatCompletions,
        Some(Wire::Messages) => return OpenCodeWire::Messages,
        Some(Wire::Responses) => return OpenCodeWire::Responses,
        Some(Wire::SystemOne) => return OpenCodeWire::Decides,
        None => {}
    }
    let model = model.to_ascii_lowercase();
    if model.starts_with("jev-") {
        return OpenCodeWire::Decides;
    }
    if kind == "opencode" && model.starts_with("gemini-") {
        return OpenCodeWire::Unsupported;
    }
    if model.starts_with("gpt-") || model.starts_with("grok-") || model.starts_with("muse-spark-") {
        return OpenCodeWire::Responses;
    }
    let qwen = model.starts_with("qwen") && !(kind == "opencode" && model == "qwen3.8-max");
    if qwen || (kind == "opencode-go" && model.starts_with("minimax-")) || model.starts_with("claude-") {
        return OpenCodeWire::Messages;
    }
    OpenCodeWire::ChatCompletions
}

/// Removes an optional `/v1` from a custom OpenCode root; each adapter adds the path its wire
/// protocol needs.
fn opencode_root(root: &str) -> &str {
    root.trim_end_matches('/').strip_suffix("/v1").unwrap_or_else(|| root.trim_end_matches('/'))
}

fn opencode_provider(
    kind: &str,
    root: &str,
    api_key: &str,
    model: &str,
    thinking: Option<ThinkingLevel>,
) -> Result<Arc<dyn Provider>, String> {
    let root = opencode_root(root);
    let provider: Arc<dyn Provider> = match opencode_wire(kind, model) {
        OpenCodeWire::ChatCompletions => {
            let mut provider = OpenAiCompatProvider::new(kind, &format!("{root}/v1"), api_key, model).with_thinking(thinking);
            provider.supports_images = built_in_vision(kind, Some(model));
            Arc::new(provider)
        }
        OpenCodeWire::Messages => {
            let mut provider = AnthropicProvider::new(kind, root, api_key, model).with_thinking(thinking);
            provider.supports_images = built_in_vision(kind, Some(model));
            provider.eager_tool_streaming = false;
            provider.max_tokens = 32_000;
            Arc::new(provider)
        }
        OpenCodeWire::Responses => {
            let mut provider = OpenAiResponsesProvider::new(kind, &format!("{root}/v1"), api_key, model).with_thinking(thinking);
            provider.supports_images = built_in_vision(kind, Some(model));
            Arc::new(provider)
        }
        OpenCodeWire::Decides => {
            return Err(format!("{model} is a decision model: it can review actions, not chat"));
        }
        OpenCodeWire::Unsupported => {
            return Err(format!("{model} uses an OpenCode endpoint Lorca does not support"));
        }
    };
    Ok(opencode_headers(provider))
}

/// A bot's adapter for a custom provider: the wire protocol the user picked, at the root they
/// gave, with what is known about the model. A decision API's models do not chat.
fn custom_provider(kind: &str, provider: &CustomProvider, model: &str, thinking: Option<ThinkingLevel>) -> Result<Arc<dyn Provider>, String> {
    if provider.api.decides() {
        return Err(format!("{} serves decision models: they can review actions, not chat", provider.name));
    }
    let info = custom_model_info(kind, provider, model);
    Ok(match provider.api {
        CustomApi::ChatCompletions => {
            let mut adapter = OpenAiCompatProvider::new(kind, &provider.base_url, &provider.api_key, model).with_thinking(thinking);
            adapter.info = Some(info);
            adapter.supports_images = info.images;
            // OpenAI's own field; servers such as Gemini's refuse a request with one they lack.
            adapter.prompt_cache_key = false;
            Arc::new(adapter)
        }
        CustomApi::Responses => {
            let mut adapter = OpenAiResponsesProvider::new(kind, &provider.base_url, &provider.api_key, model).with_thinking(thinking);
            adapter.info = Some(info);
            adapter.supports_images = info.images;
            Arc::new(adapter)
        }
        CustomApi::Messages => {
            let root = crate::provider_auth::messages_root(&provider.base_url);
            let mut adapter = AnthropicProvider::new(kind, root, &provider.api_key, model).with_thinking(thinking);
            adapter.info = Some(info);
            adapter.supports_images = info.images;
            // Arguments streamed as they are generated are Anthropic's own extension.
            adapter.eager_tool_streaming = false;
            // Room for long replies where the model's cap is known; the API requires a cap.
            adapter.max_tokens = if info.max_output > 0 { 32_000 } else { 16_384 };
            Arc::new(adapter)
        }
        CustomApi::SystemOne | CustomApi::Decisions => unreachable!("refused above"),
    })
}

/// What a custom provider's model takes: the window, output cap, and inputs its server's list
/// gave, else the catalog's for that model id, else nothing known, which means no window to
/// compact by, text only, and the common thinking levels. Its cost is zero, since Lorca does
/// not know what the server charges. Kept per kind and model, so a turn reuses the entry
/// until the provider changes.
fn custom_model_info(kind: &str, provider: &CustomProvider, model: &str) -> &'static ModelInfo {
    static INFO: LazyLock<Mutex<HashMap<(String, String), &'static ModelInfo>>> = LazyLock::new(Default::default);
    let listed = provider.models.iter().find(|m| m.id == model);
    let known = models::find_any(model);
    let thinking = match known {
        Some(known) => known.thinking,
        // Messages servers that are not Anthropic take a token budget, not an effort.
        None if provider.api == CustomApi::Messages => ThinkingMode::Budget,
        None => ThinkingMode::Effort,
    };
    let levels = crate::credentials::custom_levels(model);
    let wanted = ModelInfo {
        id: "",
        name: "",
        provider: "",
        context_window: listed.and_then(|m| m.context_window).or(known.map(|k| k.context_window)).unwrap_or(0),
        max_output: listed.and_then(|m| m.max_output).or(known.map(|k| k.max_output)).unwrap_or(0),
        reasoning: known.is_some_and(|k| k.reasoning),
        images: listed.and_then(|m| m.images).or(known.map(|k| k.images)).unwrap_or(false),
        rates: Rates { input: 0.0, output: 0.0, cache_read: 0.0, cache_write: 0.0 },
        tiers: &[],
        thinking,
        levels,
        // The user picked the protocol for the whole provider.
        wire: None,
    };
    let mut cache = INFO.lock().unwrap();
    let key = (kind.to_string(), model.to_string());
    if let Some(info) = cache.get(&key) {
        if (ModelInfo { id: "", name: "", provider: "", ..**info }) == wanted {
            return info;
        }
    }
    let name = listed.and_then(|m| m.name.clone()).or(known.map(|k| k.name.to_string())).unwrap_or_else(|| model.to_string());
    let leak = |text: String| -> &'static str { Box::leak(text.into_boxed_str()) };
    let info: &'static ModelInfo = Box::leak(Box::new(ModelInfo { id: leak(model.to_string()), name: leak(name), provider: leak(kind.to_string()), ..wanted }));
    cache.insert(key, info);
    info
}

/// DeepSeek's API root when the credential has none: `LORCA_DEEPSEEK_BASE_URL` (a proxy or a
/// test server) or DeepSeek itself.
fn deepseek_base_url() -> String {
    env_url("LORCA_DEEPSEEK_BASE_URL").unwrap_or_else(|| lorca_agent::providers::openai_compat::DEEPSEEK_BASE_URL.to_string())
}

/// The Anthropic-compatible endpoint under a DeepSeek API root. A root given with its
/// `/anthropic` path already is used as is.
fn deepseek_anthropic_url(root: &str) -> String {
    let root = root.trim_end_matches('/');
    if root.ends_with("/anthropic") {
        root.to_string()
    } else {
        format!("{root}/anthropic")
    }
}

/// Anthropic's API root when the credential has none: `LORCA_ANTHROPIC_BASE_URL` or
/// Anthropic itself.
fn anthropic_base_url() -> String {
    env_url("LORCA_ANTHROPIC_BASE_URL").unwrap_or_else(|| ANTHROPIC_BASE_URL.to_string())
}

fn env_url(name: &str) -> Option<String> {
    std::env::var(name).ok().filter(|s| !s.trim().is_empty()).map(|s| s.trim_end_matches('/').to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::credentials::CustomModel;

    struct CaptureProvider(Arc<Mutex<Option<ModelRequest>>>);

    #[async_trait]
    impl Provider for CaptureProvider {
        fn provider_id(&self) -> &str {
            "capture"
        }

        fn model_id(&self) -> &str {
            "capture"
        }

        async fn stream(&self, request: ModelRequest, _cancel: CancellationToken) -> AssistantEventStream {
            *self.0.lock().unwrap() = Some(request);
            Box::pin(futures::stream::empty())
        }
    }

    #[test]
    fn opencode_models_use_their_published_wire_protocols() {
        assert_eq!(opencode_wire("opencode", "gpt-6.1-sol"), OpenCodeWire::Responses);
        assert_eq!(opencode_wire("opencode", "muse-spark-1.3"), OpenCodeWire::Responses);
        assert_eq!(opencode_wire("opencode", "claude-sonnet-5-5"), OpenCodeWire::Messages);
        assert_eq!(opencode_wire("opencode", "deepseek-v4.1-flash"), OpenCodeWire::ChatCompletions);
        assert_eq!(opencode_wire("opencode", "minimax-m3"), OpenCodeWire::ChatCompletions);
        assert_eq!(opencode_wire("opencode", "qwen3.8-max"), OpenCodeWire::ChatCompletions);
        assert_eq!(opencode_wire("opencode", "gemini-3.8-flash"), OpenCodeWire::Unsupported);
        assert_eq!(opencode_wire("opencode-go", "minimax-m3"), OpenCodeWire::Messages);
        assert_eq!(opencode_wire("opencode-go", "qwen3.8-max"), OpenCodeWire::Messages);
        assert_eq!(opencode_wire("opencode-go", "grok-4.7"), OpenCodeWire::Responses);
        // Every model the catalog offers on OpenCode has a wire Lorca speaks.
        for kind in ["opencode", "opencode-go"] {
            for model in models::for_provider(kind) {
                assert_ne!(opencode_wire(kind, model.id), OpenCodeWire::Unsupported, "{kind}/{}", model.id);
            }
        }
    }

    #[test]
    fn every_opencode_route_takes_images_when_the_runner_sends_pixels() {
        for model in ["gpt-6.1-sol", "grok-4.7", "muse-spark-1.3", "claude-sonnet-5-5", "kimi-k3"] {
            assert!(opencode_provider("opencode", OPENCODE_BASE_URL, "k", model, None).unwrap().supports_images(), "{model}");
        }
        // A Responses model the catalog lacks, which the Runner sends no pixels, gets notes for
        // its tools' images too.
        assert!(!built_in_vision("opencode", Some("muse-spark-2")));
        assert!(!opencode_provider("opencode", OPENCODE_BASE_URL, "k", "muse-spark-2", None).unwrap().supports_images());
    }

    #[test]
    fn every_provider_has_a_default_and_roots_accept_v1() {
        for kind in ["deepseek", "anthropic", "chatgpt", "grok", "opencode", "opencode-go"] {
            assert!(models::find(kind, default_model(kind)).is_some(), "{kind}");
        }
        assert_eq!(default_model("custom:lab"), "");
        // A model the catalog lacks goes by its family.
        assert_eq!(opencode_wire("opencode-go", "claude-unlisted-9"), OpenCodeWire::Messages);
        assert_eq!(opencode_root("https://opencode.ai/zen/v1/"), OPENCODE_BASE_URL);
        assert_eq!(opencode_root("https://opencode.ai/zen"), OPENCODE_BASE_URL);
    }

    struct ScratchApp(Arc<App>, std::path::PathBuf);

    impl Drop for ScratchApp {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.1);
        }
    }

    fn scratch_app() -> ScratchApp {
        let home = std::env::temp_dir().join(format!("lorca-providers-{}", uuid::Uuid::new_v4()));
        ScratchApp(App::load(crate::config::Config { home: home.clone(), port: 0 }).unwrap(), home)
    }

    fn model(id: &str) -> CustomModel {
        CustomModel { id: id.into(), name: None, context_window: None, max_output: None, images: None }
    }

    fn add_custom(app: &App, kind: &str, api: CustomApi, models: Vec<CustomModel>) {
        let provider = CustomProvider { name: "Lab".into(), api, base_url: "http://127.0.0.1:9/v1".into(), api_key: String::new(), models, created_at: 1 };
        app.credentials.lock().unwrap().custom.insert(kind.into(), provider);
    }

    #[test]
    fn auto_review_runs_a_small_model_that_thinks_least() {
        let scratch = scratch_app();
        let app = &scratch.0;
        for kind in ["deepseek", "anthropic", "chatgpt", "grok", "opencode", "opencode-go"] {
            assert!(models::find(kind, &small_model(app, kind).0).is_some(), "{kind}");
        }
        assert_eq!(small_model(app, "deepseek"), ("deepseek-flash".into(), Some(ThinkingLevel::Off)));
        assert_eq!(small_model(app, "anthropic"), ("claude-haiku-4-5".into(), Some(ThinkingLevel::Off)));
        assert_eq!(small_model(app, "chatgpt"), ("gpt-6-luna".into(), Some(ThinkingLevel::Low)));
        assert_eq!(small_model(app, "grok"), ("grok-4.7".into(), Some(ThinkingLevel::Low)));
        assert_eq!(small_model(app, "opencode-go"), ("deepseek-v4.1-flash".into(), Some(ThinkingLevel::Low)));
        // A custom provider reviews with its first model, at the server's default unless the
        // catalog knows the model.
        add_custom(app, "custom:lab", CustomApi::ChatCompletions, vec![model("qwen3:8b"), model("llama4")]);
        assert_eq!(small_model(app, "custom:lab"), ("qwen3:8b".into(), None));
        add_custom(app, "custom:proxy", CustomApi::Messages, vec![model("anthropic/claude-haiku-4-5")]);
        assert_eq!(small_model(app, "custom:proxy"), ("anthropic/claude-haiku-4-5".into(), Some(ThinkingLevel::Off)));
        assert_eq!(small_model(app, "custom:gone"), (String::new(), None));
    }

    #[test]
    fn auto_review_runs_the_model_the_user_picked_while_its_provider_is_connected() {
        use crate::credentials::ApiKeyCredential;
        use crate::model::AutoReview;

        let scratch = scratch_app();
        let app = &scratch.0;
        let key = || Some(ApiKeyCredential { api_key: "key".into(), base_url: None, connected_at: 0 });
        app.credentials.lock().unwrap().deepseek = key();
        // Reviews with `provider`, at the review model picked for it, if any.
        let pick = |provider: &str, model: Option<&str>| {
            let models = model.map(|model| [(provider.to_string(), model.to_string())].into()).unwrap_or_default();
            app.set_auto_review(AutoReview { provider: Some(provider.into()), models, ..AutoReview::default() })
        };
        let chat = |reviewer: Reviewer| match reviewer {
            Reviewer::Chat { provider, thinking } => (provider.provider_id().to_string(), provider.model_id().to_string(), thinking),
            Reviewer::Decides(decider) => panic!("{} decides", decider.model),
        };
        let decides = |reviewer: Reviewer| match reviewer {
            Reviewer::Decides(decider) => decider,
            Reviewer::Chat { provider, .. } => panic!("{} chats", provider.model_id()),
        };

        // Nothing picked: the bot's own provider's small model, or the review model picked for it.
        assert_eq!(chat(reviewer(app, "deepseek").unwrap()), ("deepseek".into(), "deepseek-flash".into(), Some(ThinkingLevel::Off)));
        app.set_auto_review(AutoReview { models: [("deepseek".to_string(), "deepseek-v4-pro".to_string())].into(), ..AutoReview::default() });
        assert_eq!(chat(reviewer(app, "deepseek").unwrap()).1, "deepseek-v4-pro");
        // Another provider's chat model.
        add_custom(app, "custom:lab", CustomApi::ChatCompletions, vec![model("qwen3:8b"), model("llama4")]);
        pick("custom:lab", Some("llama4"));
        assert_eq!(chat(reviewer(app, "deepseek").unwrap()), ("custom:lab".into(), "llama4".into(), None));
        // A decision API the user added, at its endpoint.
        add_custom(app, "custom:typesafe", CustomApi::SystemOne, vec![model("jev-latest")]);
        app.credentials.lock().unwrap().custom.get_mut("custom:typesafe").unwrap().base_url = "https://api.typesafe.ai/v1/systemone".into();
        pick("custom:typesafe", None);
        let decider = decides(reviewer(app, "deepseek").unwrap());
        assert_eq!((decider.shape, decider.url.as_str(), decider.model.as_str()), (Shape::SystemOne, "https://api.typesafe.ai/v1/systemone", "jev-latest"));
        add_custom(app, "custom:openai", CustomApi::Decisions, vec![model("gpt-6-luna")]);
        pick("custom:openai", Some("gpt-6-luna"));
        assert_eq!(decides(reviewer(app, "deepseek").unwrap()).shape, Shape::OpenAi);
        // Zen's decision models, at its System One endpoint.
        app.credentials.lock().unwrap().opencode = key();
        pick("opencode", Some("jev-1.13-free"));
        let decider = decides(reviewer(app, "deepseek").unwrap());
        assert_eq!((decider.url.as_str(), decider.session_header), ("https://opencode.ai/zen/v1/systemone", Some("x-opencode-session")));
        pick("opencode", Some("deepseek-v4.1-flash"));
        assert_eq!(chat(reviewer(app, "deepseek").unwrap()).1, "deepseek-v4.1-flash");
        // A picked provider the account no longer has hands the review back to the bot's.
        app.credentials.lock().unwrap().opencode = None;
        assert_eq!(chat(reviewer(app, "deepseek").unwrap()).0, "deepseek");
        // No bot chats with a decision model.
        assert!(provider_for(app, "custom:typesafe", None, None).err().unwrap().contains("not chat"));
        assert_eq!(opencode_wire("opencode", "jev-1.13"), OpenCodeWire::Decides);
    }

    #[test]
    fn custom_providers_run_their_first_model_with_what_is_known_of_it() {
        let scratch = scratch_app();
        let app = &scratch.0;
        let listed = CustomModel { context_window: Some(32_768), images: Some(true), ..model("qwen3:8b") };
        add_custom(app, "custom:vision-lab", CustomApi::ChatCompletions, vec![listed, model("anthropic/claude-sonnet-5"), model("mystery")]);

        let provider = provider_for(app, "custom:vision-lab", None, None).unwrap();
        assert_eq!((provider.provider_id(), provider.model_id()), ("custom:vision-lab", "qwen3:8b"));
        assert_eq!(provider.model_info().map(|info| (info.context_window, info.images)), Some((32_768, true)));
        assert!(provider.supports_images());
        assert!(supports_vision(app, "custom:vision-lab", None));

        // The catalog knows a gateway's model by its id; the cost stays zero.
        let known = provider_for(app, "custom:vision-lab", Some("anthropic/claude-sonnet-5"), None).unwrap();
        let info = known.model_info().unwrap();
        assert_eq!((info.context_window, info.images, info.rates.input), (1_000_000, true, 0.0));

        // Nothing known: no window, text only, the common levels.
        let unknown = provider_for(app, "custom:vision-lab", Some("mystery"), Some(ThinkingLevel::Max)).unwrap();
        let info = unknown.model_info().unwrap();
        assert_eq!((info.context_window, info.images, info.clamp_level(ThinkingLevel::Max)), (0, false, Some(ThinkingLevel::High)));
        assert!(!supports_vision(app, "custom:vision-lab", Some("mystery")));

        // The entry is kept until the provider changes.
        let again = provider_for(app, "custom:vision-lab", None, None).unwrap();
        assert!(std::ptr::eq(provider.model_info().unwrap(), again.model_info().unwrap()));
        app.credentials.lock().unwrap().custom.get_mut("custom:vision-lab").unwrap().models[0].context_window = Some(65_536);
        assert_eq!(provider_for(app, "custom:vision-lab", None, None).unwrap().model_info().map(|info| info.context_window), Some(65_536));

        app.credentials.lock().unwrap().custom.clear();
        assert_eq!(provider_for(app, "custom:vision-lab", None, None).err().unwrap(), "vision-lab is not connected");
    }

    /// Answers one model call with `body` as a stream and hands back the request it got.
    fn answer_once(body: &'static str) -> (String, std::thread::JoinHandle<String>) {
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let root = format!("http://{}", listener.local_addr().unwrap());
        let server = std::thread::spawn(move || {
            use std::io::{Read, Write};
            let (mut socket, _) = listener.accept().unwrap();
            let mut request = [0u8; 8192];
            let read = socket.read(&mut request).unwrap();
            let reply = format!("HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}", body.len());
            socket.write_all(reply.as_bytes()).unwrap();
            String::from_utf8_lossy(&request[..read]).to_ascii_lowercase()
        });
        (root, server)
    }

    #[tokio::test]
    async fn a_server_that_takes_no_key_gets_no_auth_header() {
        use futures::StreamExt;
        let scratch = scratch_app();
        let app = &scratch.0;
        let request = || ModelRequest {
            system_prompt: String::new(),
            messages: vec![lorca_agent::LlmMessage::User(lorca_agent::UserMessage::text("hi"))],
            tools: Vec::new(),
            cache_points: Vec::new(),
            max_tokens: None,
            options: lorca_agent::RequestOptions::default().with_session_id("chat-1"),
        };
        // A Messages root with its `/v1` and one without reach the same endpoint.
        let cases = [
            (CustomApi::ChatCompletions, "/v1", "data: [DONE]\n\n", "post /v1/chat/completions "),
            (CustomApi::Messages, "", "event: message_stop\ndata: {\"type\":\"message_stop\"}\n\n", "post /v1/messages "),
            (CustomApi::Messages, "/v1", "event: message_stop\ndata: {\"type\":\"message_stop\"}\n\n", "post /v1/messages "),
        ];
        for (index, (api, path, body, line)) in cases.into_iter().enumerate() {
            let (root, server) = answer_once(body);
            let kind = format!("custom:keyless-{index}");
            let provider = CustomProvider { name: "Keyless".into(), api, base_url: format!("{root}{path}"), api_key: String::new(), models: vec![model("m")], created_at: 1 };
            app.credentials.lock().unwrap().custom.insert(kind.clone(), provider);
            let mut stream = provider_for(app, &kind, None, None).unwrap().stream(request(), CancellationToken::new()).await;
            while stream.next().await.is_some() {}
            let seen = server.join().unwrap();
            assert!(seen.starts_with(line), "{seen}");
            assert!(!seen.contains("\r\nauthorization:") && !seen.contains("\r\nx-api-key:"), "{seen}");
            assert!(!seen.contains("prompt_cache_key"), "{seen}");
        }
    }

    #[test]
    fn a_custom_messages_server_thinks_by_budget() {
        let scratch = scratch_app();
        let app = &scratch.0;
        add_custom(app, "custom:proxy", CustomApi::Messages, vec![model("glm-6"), model("claude-opus-5")]);
        let info = provider_for(app, "custom:proxy", None, None).unwrap().model_info().unwrap();
        assert_eq!(info.thinking, ThinkingMode::Budget);
        let claude = provider_for(app, "custom:proxy", Some("claude-opus-5"), None).unwrap();
        assert_eq!(claude.model_info().unwrap().thinking, ThinkingMode::Adaptive);
    }

    #[tokio::test]
    async fn opencode_requests_identify_lorca_and_carry_the_conversation() {
        let seen = Arc::new(Mutex::new(None));
        let provider = opencode_headers(Arc::new(CaptureProvider(seen.clone())));
        let request = ModelRequest {
            system_prompt: String::new(),
            messages: Vec::new(),
            tools: Vec::new(),
            cache_points: Vec::new(),
            max_tokens: None,
            options: lorca_agent::RequestOptions::default().with_session_id("chat-1"),
        };
        let _ = provider.stream(request, CancellationToken::new()).await;
        let request = seen.lock().unwrap().take().unwrap();
        assert_eq!(request.options.headers.get("User-Agent").map(String::as_str), Some(USER_AGENT));
        assert_eq!(request.options.headers.get("x-opencode-session").map(String::as_str), Some("chat-1"));
    }
}
