//! Connects and disconnects the account's provider credentials on any Device. API keys are
//! checked here before they enter the encrypted `credentials` blob, and a custom provider's
//! server answers for its key and lists its models. Subscription sign-ins use the providers'
//! PKCE loopback flows; a desktop opens the URL itself and a phone hands it to its native
//! in-app browser.

use std::sync::Arc;

use serde_json::Value;

use lorca_provider_auth::chatgpt::{self as chatgpt_oauth, ChatGptTokens};
use lorca_provider_auth::grok::{self as grok_oauth, GrokTokens};

use crate::app::App;
use crate::config;
use crate::credentials::{is_custom, ApiKeyCredential, Credentials, CustomApi, CustomModel, CustomProvider, CUSTOM_PREFIX, PROVIDER_KINDS};

const ANTHROPIC_BASE_URL: &str = "https://api.anthropic.com";
const ANTHROPIC_VERSION: &str = "2023-06-01";
const OPENCODE_BASE_URL: &str = "https://opencode.ai/zen";
const OPENCODE_GO_BASE_URL: &str = "https://opencode.ai/zen/go";

fn env_url(name: &str) -> Option<String> {
    std::env::var(name).ok().filter(|s| !s.trim().is_empty()).map(|s| s.trim_end_matches('/').to_string())
}

fn deepseek_base_url() -> String {
    env_url("LORCA_DEEPSEEK_BASE_URL").unwrap_or_else(|| "https://api.deepseek.com".into())
}

fn anthropic_base_url() -> String {
    env_url("LORCA_ANTHROPIC_BASE_URL").unwrap_or_else(|| ANTHROPIC_BASE_URL.to_string())
}

/// Removes an optional `/v1` from a custom OpenCode root; the credential check adds the
/// endpoint it needs.
fn opencode_root(root: &str) -> &str {
    root.trim_end_matches('/').strip_suffix("/v1").unwrap_or_else(|| root.trim_end_matches('/'))
}

/// A base URL the user typed: blank means the default; otherwise an http(s) root without a
/// trailing slash.
fn custom_base_url(base_url: Option<&str>) -> Result<Option<String>, String> {
    let Some(url) = base_url.map(str::trim).filter(|u| !u.is_empty()) else { return Ok(None) };
    if !url.starts_with("http://") && !url.starts_with("https://") {
        return Err("The base URL must start with http:// or https://".into());
    }
    Ok(Some(url.trim_end_matches('/').to_string()))
}

/// Checks a DeepSeek key against the API (the given root, or DeepSeek's) before saving both.
pub async fn connect_deepseek(app: &Arc<App>, api_key: &str, base_url: Option<&str>) -> Result<(), String> {
    let key = api_key.trim();
    if key.is_empty() {
        return Err("Paste a DeepSeek API key".into());
    }
    let base_url = custom_base_url(base_url)?;
    let root = base_url.clone().unwrap_or_else(deepseek_base_url);
    let request = app.http.get(format!("{}/models", root.trim_end_matches("/anthropic"))).bearer_auth(key);
    check_key("DeepSeek", request).await?;
    save_api_key(app, "deepseek", key, base_url)
}

/// Checks an Anthropic key against the API (the given root, or Anthropic's) before saving both.
pub async fn connect_anthropic(app: &Arc<App>, api_key: &str, base_url: Option<&str>) -> Result<(), String> {
    let key = api_key.trim();
    if key.is_empty() {
        return Err("Paste an Anthropic API key".into());
    }
    let base_url = custom_base_url(base_url)?;
    let root = base_url.clone().unwrap_or_else(anthropic_base_url);
    let request = app.http.get(format!("{root}/v1/models")).header("x-api-key", key).header("anthropic-version", ANTHROPIC_VERSION);
    check_key("Anthropic", request).await?;
    save_api_key(app, "anthropic", key, base_url)
}

/// Checks an OpenCode Zen key. The model catalog is public, so the account check uses Go's
/// authenticated usage endpoint; a valid Zen key without a Go subscription answers 403 after
/// authentication and is still a valid Zen credential.
pub async fn connect_opencode(app: &Arc<App>, api_key: &str, base_url: Option<&str>) -> Result<(), String> {
    let key = api_key.trim();
    if key.is_empty() {
        return Err("Paste an OpenCode Zen API key".into());
    }
    let base_url = custom_base_url(base_url)?;
    let configured = base_url.clone().or_else(|| env_url("LORCA_OPENCODE_BASE_URL"));
    let root = configured.as_deref().map(opencode_root).unwrap_or(OPENCODE_BASE_URL);
    if root == OPENCODE_BASE_URL {
        let request = app.http.get(format!("{root}/go/v1/usage")).bearer_auth(key);
        check_opencode_key("OpenCode Zen", request, false).await?;
    } else {
        check_key("OpenCode Zen", app.http.get(format!("{root}/v1/models")).bearer_auth(key)).await?;
    }
    save_api_key(app, "opencode", key, base_url)
}

/// Checks both the OpenCode key and its Go entitlement against the authenticated usage route.
pub async fn connect_opencode_go(app: &Arc<App>, api_key: &str, base_url: Option<&str>) -> Result<(), String> {
    let key = api_key.trim();
    if key.is_empty() {
        return Err("Paste an OpenCode Go API key".into());
    }
    let base_url = custom_base_url(base_url)?;
    let root = base_url
        .clone()
        .or_else(|| env_url("LORCA_OPENCODE_GO_BASE_URL"))
        .unwrap_or_else(|| OPENCODE_GO_BASE_URL.into());
    let root = opencode_root(&root);
    if root == OPENCODE_GO_BASE_URL {
        check_opencode_key("OpenCode Go", app.http.get(format!("{root}/v1/usage")).bearer_auth(key), true).await?;
    } else {
        check_key("OpenCode Go", app.http.get(format!("{root}/v1/models")).bearer_auth(key)).await?;
    }
    save_api_key(app, "opencode-go", key, base_url)
}

async fn check_key(name: &str, request: reqwest::RequestBuilder) -> Result<(), String> {
    let response = request.send().await.map_err(|e| format!("{name} unreachable: {}", lorca_tls::describe(&e)))?;
    match response.status() {
        reqwest::StatusCode::UNAUTHORIZED | reqwest::StatusCode::FORBIDDEN => Err(format!("{name} rejected that key")),
        status if status.is_success() => Ok(()),
        status => Err(format!("{name} answered {status}")),
    }
}

async fn check_opencode_key(name: &str, request: reqwest::RequestBuilder, requires_go: bool) -> Result<(), String> {
    let response = request.send().await.map_err(|e| format!("{name} unreachable: {}", lorca_tls::describe(&e)))?;
    match response.status() {
        reqwest::StatusCode::UNAUTHORIZED => Err(format!("{name} rejected that key")),
        reqwest::StatusCode::FORBIDDEN if requires_go => Err("OpenCode Go needs an active subscription".into()),
        reqwest::StatusCode::FORBIDDEN => Ok(()),
        status if status.is_success() => Ok(()),
        status => Err(format!("{name} answered {status}")),
    }
}

fn save_api_key(app: &Arc<App>, kind: &str, key: &str, base_url: Option<String>) -> Result<(), String> {
    let credential = Some(ApiKeyCredential { api_key: key.to_string(), base_url, connected_at: config::now_unix() });
    let update = |credentials: &mut Credentials| match kind {
        "deepseek" => credentials.deepseek = credential,
        "anthropic" => credentials.anthropic = credential,
        "opencode" => credentials.opencode = credential,
        "opencode-go" => credentials.opencode_go = credential,
        _ => unreachable!(),
    };
    app.update_credentials(kind, update).map_err(|e| e.to_string())
}

/// What the user typed for a custom provider.
#[derive(Debug, Default)]
pub struct CustomInput {
    /// The provider being edited; `None` adds one.
    pub kind: Option<String>,
    pub name: String,
    pub api: String,
    pub base_url: String,
    pub api_key: String,
    /// Model ids in the user's order. Empty takes every model the server lists.
    pub models: Vec<String>,
}

/// Checks a custom provider's server and saves the provider for the account, answering with
/// its kind. The model list the server publishes checks the key and tells each model's window,
/// output cap, and whether it sees images; a server without one still works with the model ids
/// the user gave.
pub async fn connect_custom(app: &Arc<App>, input: CustomInput) -> Result<String, String> {
    let name = input.name.trim().to_string();
    if name.is_empty() {
        return Err("Name the provider".into());
    }
    if name.chars().count() > 40 {
        return Err("Keep the name under 40 characters".into());
    }
    let api = CustomApi::parse(input.api.trim()).ok_or_else(|| format!("Unknown API {}", input.api.trim()))?;
    let base_url = custom_root(api, &input.base_url)?;
    let api_key = input.api_key.trim().to_string();
    let mut ids: Vec<String> = Vec::new();
    for id in input.models.iter().map(|id| id.trim()).filter(|id| !id.is_empty()) {
        if !ids.iter().any(|seen| seen == id) {
            ids.push(id.to_string());
        }
    }
    if let Some(kind) = &input.kind {
        if !is_custom(kind) {
            return Err(format!("{kind} is not a custom provider"));
        }
    }
    {
        let credentials = app.credentials.lock().unwrap();
        let taken = PROVIDER_KINDS
            .iter()
            .map(|kind| kind.to_string())
            .chain(credentials.custom_kinds())
            .filter(|kind| Some(kind) != input.kind.as_ref())
            .any(|kind| credentials.label(&kind).eq_ignore_ascii_case(&name));
        if taken {
            return Err(format!("A provider named {name} exists already"));
        }
    }

    let listed = list_models(app, &name, api, &base_url, &api_key).await?;
    let models = if ids.is_empty() {
        let listed = listed.ok_or_else(|| format!("{name} publishes no model list. Add the model ids yourself."))?;
        let usable = usable(api, listed);
        if usable.is_empty() {
            return Err(format!("{name} lists no models. Add the model ids yourself."));
        }
        usable
    } else {
        let listed = listed.unwrap_or_default();
        ids.into_iter()
            .map(|id| match listed.iter().find(|(_, model)| model.id == id) {
                Some((_, model)) => model.clone(),
                None => CustomModel { id, name: None, context_window: None, max_output: None, images: None },
            })
            .collect()
    };

    let (kind, created_at) = {
        let credentials = app.credentials.lock().unwrap();
        let kind = input.kind.clone().unwrap_or_else(|| custom_kind(&credentials, &name));
        let created_at = credentials.custom.get(&kind).map(|provider| provider.created_at).unwrap_or_else(config::now_unix);
        (kind, created_at)
    };
    let provider = CustomProvider { name, api, base_url, api_key, models, created_at };
    app.update_credentials(&kind, |credentials| {
        credentials.custom.insert(kind.clone(), provider);
    })
    .map_err(|e| e.to_string())?;
    Ok(kind)
}

/// The models a custom provider's server lists that its protocol can run, for the apps' model
/// picker: `None` when the server publishes no list. The base URL is read as `connect_custom`
/// reads it, and a key the server refuses or a server that cannot be reached fails the same way.
pub async fn list_custom_models(app: &Arc<App>, name: &str, api: &str, base_url: &str, api_key: &str) -> Result<Option<Vec<CustomModel>>, String> {
    let name = Some(name.trim()).filter(|name| !name.is_empty()).unwrap_or("The server");
    let api = CustomApi::parse(api.trim()).ok_or_else(|| format!("Unknown API {}", api.trim()))?;
    let root = custom_root(api, base_url)?;
    let listed = list_models(app, name, api, &root, api_key.trim()).await?;
    Ok(listed.map(|models| usable(api, models)))
}

/// What a listed model is for.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Use {
    Chat,
    /// A decision model: OpenRouter lists its output as `decisions`, Vercel's AI Gateway its
    /// type as `evaluation`.
    Decides,
    /// Embeddings, speech, images, and the rest chat cannot use.
    Other,
}

/// The listed models a provider of `api` can run: the chat models for a chat protocol. A
/// decision API takes the models the list marks as decision models, or every chat model of a
/// list that marks none, as a vendor's list of its own decision models does.
fn usable(api: CustomApi, listed: Vec<(Use, CustomModel)>) -> Vec<CustomModel> {
    let wanted = match api.decides() {
        true if listed.iter().any(|(using, _)| *using == Use::Decides) => Use::Decides,
        _ => Use::Chat,
    };
    listed.into_iter().filter(|(using, _)| *using == wanted).map(|(_, model)| model).collect()
}

/// A new custom provider's kind: `custom:` and a slug of its name, with a number when another
/// provider has it. A provider deleted under that slug gives it up, so its bots run again.
fn custom_kind(credentials: &Credentials, name: &str) -> String {
    let mut slug = String::new();
    for c in name.chars().flat_map(char::to_lowercase) {
        if c.is_ascii_alphanumeric() {
            slug.push(c);
        } else if !slug.is_empty() && !slug.ends_with('-') {
            slug.push('-');
        }
    }
    let slug: String = slug.trim_end_matches('-').chars().take(32).collect();
    let base = format!("{CUSTOM_PREFIX}{}", if slug.is_empty() { "provider" } else { slug.trim_end_matches('-') });
    let mut kind = base.clone();
    let mut n = 2;
    while credentials.custom.contains_key(&kind) {
        kind = format!("{base}-{n}");
        n += 1;
    }
    kind
}

/// A custom provider's base URL: required, http(s), and cut back to the root when the user
/// pasted a whole endpoint, since each adapter adds the path its protocol needs. A decision
/// API's is its endpoint, since vendors serve one shape at different paths (OpenRouter's System
/// One at `/api/alpha/decisions`): the one the user gave, or a root and the path the API has
/// at TypeSafe and OpenAI.
fn custom_root(api: CustomApi, base_url: &str) -> Result<String, String> {
    let url = custom_base_url(Some(base_url))?.ok_or("Enter the server's base URL")?;
    let endpoint: &[&str] = match api {
        CustomApi::ChatCompletions => &["/chat/completions"],
        CustomApi::Responses => &["/responses"],
        CustomApi::Messages => &["/messages"],
        CustomApi::SystemOne | CustomApi::Decisions if url.ends_with("/systemone") || url.ends_with("/decisions") => return Ok(url),
        CustomApi::SystemOne => return Ok(format!("{url}/systemone")),
        CustomApi::Decisions => return Ok(format!("{url}/decisions")),
    };
    Ok(endpoint.iter().find_map(|path| url.strip_suffix(path)).unwrap_or(&url).to_string())
}

/// A Messages base URL without its `/v1`, which the Messages paths add, so `…/v1` and a root
/// without one (`…/anthropic`) reach the same endpoints.
pub(crate) fn messages_root(base_url: &str) -> &str {
    base_url.strip_suffix("/v1").unwrap_or(base_url)
}

/// Where a decision endpoint's server lists its models: beside the endpoint. OpenRouter keeps
/// decisions in its alpha API and lists their models with the rest, by their output.
fn decision_models_url(endpoint: &str) -> String {
    if let Some(root) = endpoint.strip_suffix("/alpha/decisions") {
        return format!("{root}/v1/models?output_modalities=decisions");
    }
    let parent = endpoint.rsplit_once('/').map_or(endpoint, |(parent, _)| parent);
    format!("{parent}/models")
}

/// The models a custom provider's server lists, each with what it is for; `None` when the
/// server publishes no list. A key the server refuses, or a server that cannot be reached,
/// fails.
async fn list_models(app: &Arc<App>, name: &str, api: CustomApi, root: &str, api_key: &str) -> Result<Option<Vec<(Use, CustomModel)>>, String> {
    let mut request = match api {
        CustomApi::ChatCompletions | CustomApi::Responses => {
            let request = app.http.get(format!("{root}/models"));
            if api_key.is_empty() { request } else { request.bearer_auth(api_key) }
        }
        CustomApi::SystemOne | CustomApi::Decisions => {
            let request = app.http.get(decision_models_url(root));
            if api_key.is_empty() { request } else { request.bearer_auth(api_key) }
        }
        CustomApi::Messages => {
            let request = app.http.get(format!("{}/v1/models?limit=1000", messages_root(root))).header("anthropic-version", ANTHROPIC_VERSION);
            if api_key.is_empty() { request } else { request.header("x-api-key", api_key) }
        }
    };
    request = request.timeout(std::time::Duration::from_secs(20));
    let response = request.send().await.map_err(|e| format!("{name} unreachable: {}", lorca_tls::describe(&e)))?;
    match response.status() {
        reqwest::StatusCode::UNAUTHORIZED | reqwest::StatusCode::FORBIDDEN if api_key.is_empty() => Err(format!("{name} needs an API key")),
        reqwest::StatusCode::UNAUTHORIZED | reqwest::StatusCode::FORBIDDEN => Err(format!("{name} rejected that key")),
        status if status.is_success() => {
            let body: Value = response.json().await.map_err(|_| format!("{name} did not answer like an API at {root}. Check the base URL."))?;
            Ok(listed_models(&body))
        }
        _ => Ok(None),
    }
}

/// A model list in the shape OpenAI, Anthropic, and most gateways and local servers answer
/// with: `data`, `models`, or a bare array of entries.
fn listed_models(body: &Value) -> Option<Vec<(Use, CustomModel)>> {
    let entries = body.get("data").or_else(|| body.get("models")).unwrap_or(body).as_array()?;
    Some(entries.iter().filter_map(listed_model).collect())
}

/// Words in the ids of models a server lists that chat cannot use: embeddings, rerankers,
/// speech, transcription, realtime audio, image and video generation, moderation, and
/// completion-only base models.
const NOT_CHAT_WORDS: [&str; 14] =
    ["embed", "rerank", "whisper", "tts", "transcribe", "realtime", "audio", "dall-e", "image", "sora", "moderation", "babbage", "davinci", "guard"];

/// The `type` a model list (Together's) gives a model chat cannot use.
const NOT_CHAT_TYPES: [&str; 8] = ["embedding", "rerank", "image", "audio", "transcribe", "moderation", "video", "tts"];

/// One entry of a model list, read for the fields servers use for a model's name, window,
/// output cap, and inputs, and what it is for.
fn listed_model(entry: &Value) -> Option<(Use, CustomModel)> {
    let id = entry["id"].as_str().or_else(|| entry["name"].as_str()).map(str::trim).filter(|id| !id.is_empty())?.to_string();
    let number = |paths: &[&str]| paths.iter().find_map(|path| entry.pointer(path).and_then(Value::as_u64)).filter(|n| *n > 0);
    let name = ["display_name", "name"]
        .iter()
        .find_map(|key| entry[*key].as_str())
        .map(str::trim)
        .filter(|name| !name.is_empty() && *name != id)
        .map(str::to_string);
    let context_window = number(&["/context_length", "/context_window", "/max_model_len", "/max_context_length", "/max_input_tokens", "/top_provider/context_length"]);
    let max_output = number(&["/max_output_tokens", "/max_completion_tokens", "/top_provider/max_completion_tokens"]);
    let inputs = entry.pointer("/architecture/input_modalities").or_else(|| entry.pointer("/modalities/input")).and_then(Value::as_array);
    let images = inputs.map(|inputs| inputs.iter().any(|input| input == "image")).or_else(|| entry.pointer("/capabilities/vision").and_then(Value::as_bool));
    let lower = id.to_ascii_lowercase();
    let outputs = entry.pointer("/architecture/output_modalities").and_then(Value::as_array);
    let decides = outputs.is_some_and(|outputs| outputs.iter().any(|output| output == "decisions"))
        || entry["type"].as_str().is_some_and(|kind| kind == "evaluation" || kind.contains("decision"));
    let not_chat = NOT_CHAT_WORDS.iter().any(|word| lower.contains(word))
        || entry["type"].as_str().is_some_and(|kind| NOT_CHAT_TYPES.contains(&kind))
        || entry.pointer("/capabilities/completion_chat").and_then(Value::as_bool) == Some(false)
        || outputs.is_some_and(|outputs| !outputs.iter().any(|output| output == "text"));
    let using = if decides {
        Use::Decides
    } else if not_chat {
        Use::Other
    } else {
        Use::Chat
    };
    Some((using, CustomModel { id, name, context_window, max_output, images }))
}

/// Runs a ChatGPT sign-in, opening its authorization URL through the Device's UI.
pub async fn connect_chatgpt(
    app: &Arc<App>,
    open_url: impl FnOnce(&str) -> Result<(), String>,
) -> Result<ChatGptTokens, String> {
    let tokens = chatgpt_oauth::login(&app.http, open_url, std::time::Duration::from_secs(5 * 60)).await?;
    app.update_credentials("chatgpt", |c| c.chatgpt = Some(tokens.clone())).map_err(|e| e.to_string())?;
    Ok(tokens)
}

/// Runs a Grok sign-in, opening its authorization URL through the Device's UI.
pub async fn connect_grok(
    app: &Arc<App>,
    open_url: impl FnOnce(&str) -> Result<(), String>,
) -> Result<GrokTokens, String> {
    let endpoints = env_url("LORCA_GROK_ISSUER").map(|issuer| grok_oauth::Endpoints::at(&issuer)).unwrap_or_else(grok_oauth::Endpoints::xai);
    let tokens = grok_oauth::login(&app.http, &endpoints, open_url, std::time::Duration::from_secs(5 * 60)).await?;
    app.update_credentials("grok", |c| c.grok = Some(tokens.clone())).map_err(|e| e.to_string())?;
    Ok(tokens)
}

/// Disconnects `kind` for the whole account: every Device drops the credential, and a custom
/// provider is deleted.
pub fn disconnect(app: &Arc<App>, kind: &str) -> Result<(), String> {
    if is_custom(kind) {
        if !app.credentials.lock().unwrap().custom.contains_key(kind) {
            return Err(format!("Unknown provider {kind}"));
        }
        return app
            .update_credentials(kind, |credentials| {
                credentials.custom.remove(kind);
            })
            .map_err(|e| e.to_string());
    }
    if !PROVIDER_KINDS.contains(&kind) {
        return Err(format!("Unknown provider {kind}"));
    }
    app.update_credentials(kind, |credentials| match kind {
        "deepseek" => credentials.deepseek = None,
        "anthropic" => credentials.anthropic = None,
        "opencode" => credentials.opencode = None,
        "opencode-go" => credentials.opencode_go = None,
        "chatgpt" => credentials.chatgpt = None,
        "grok" => {
            // Tell xAI the sign-in is over; the account-wide removal stands either way.
            if let Some(tokens) = credentials.grok.take() {
                let http = app.http.clone();
                let endpoints = env_url("LORCA_GROK_ISSUER").map(|issuer| grok_oauth::Endpoints::at(&issuer)).unwrap_or_else(grok_oauth::Endpoints::xai);
                tokio::spawn(async move {
                    if let Err(error) = grok_oauth::revoke(&http, &endpoints, &tokens.refresh_token).await {
                        tracing::debug!("grok revoke: {error}");
                    }
                });
            }
        }
        _ => unreachable!(),
    })
    .map_err(|e| e.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn normalizes_custom_roots() {
        assert_eq!(custom_base_url(Some(" https://proxy.example/v1/ ")).unwrap(), Some("https://proxy.example/v1".into()));
        assert_eq!(custom_base_url(Some("  ")).unwrap(), None);
        assert_eq!(opencode_root("https://proxy.example/v1"), "https://proxy.example");
        assert!(custom_base_url(Some("proxy.example")).unwrap_err().contains("http://"));
    }

    #[test]
    fn a_pasted_endpoint_becomes_the_root_its_protocol_extends() {
        let root = |api, url| custom_root(api, url).unwrap();
        assert_eq!(root(CustomApi::ChatCompletions, "https://openrouter.ai/api/v1/"), "https://openrouter.ai/api/v1");
        assert_eq!(root(CustomApi::ChatCompletions, "http://localhost:11434/v1/chat/completions"), "http://localhost:11434/v1");
        assert_eq!(root(CustomApi::Responses, "https://gateway.example/v1/responses"), "https://gateway.example/v1");
        assert_eq!(root(CustomApi::Messages, "https://api.anthropic.com/v1/messages"), "https://api.anthropic.com/v1");
        assert_eq!(root(CustomApi::Messages, "https://api.anthropic.com/v1"), "https://api.anthropic.com/v1");
        assert_eq!(root(CustomApi::Messages, "https://api.moonshot.ai/anthropic"), "https://api.moonshot.ai/anthropic");
        // Messages paths add the `/v1` a root without one lacks.
        assert_eq!(messages_root("https://api.anthropic.com/v1"), "https://api.anthropic.com");
        assert_eq!(messages_root("https://api.moonshot.ai/anthropic"), "https://api.moonshot.ai/anthropic");
        assert_eq!(custom_root(CustomApi::Messages, " ").unwrap_err(), "Enter the server's base URL");
    }

    #[test]
    fn a_decision_api_keeps_its_endpoint() {
        let endpoint = |api, url| custom_root(api, url).unwrap();
        assert_eq!(endpoint(CustomApi::SystemOne, "https://api.typesafe.ai/v1"), "https://api.typesafe.ai/v1/systemone");
        assert_eq!(endpoint(CustomApi::SystemOne, "https://api.typesafe.ai/v1/systemone/"), "https://api.typesafe.ai/v1/systemone");
        assert_eq!(endpoint(CustomApi::SystemOne, "https://openrouter.ai/api/alpha/decisions"), "https://openrouter.ai/api/alpha/decisions");
        assert_eq!(endpoint(CustomApi::Decisions, "https://api.openai.com/v1"), "https://api.openai.com/v1/decisions");
        assert_eq!(endpoint(CustomApi::Decisions, "https://ai-gateway.vercel.sh/v1/decisions"), "https://ai-gateway.vercel.sh/v1/decisions");
        // Each lists its models beside the endpoint; OpenRouter with the rest of its models.
        assert_eq!(decision_models_url("https://api.typesafe.ai/v1/systemone"), "https://api.typesafe.ai/v1/models");
        assert_eq!(decision_models_url("https://api.openai.com/v1/decisions"), "https://api.openai.com/v1/models");
        assert_eq!(decision_models_url("https://openrouter.ai/api/alpha/decisions"), "https://openrouter.ai/api/v1/models?output_modalities=decisions");
    }

    #[test]
    fn a_decision_api_takes_the_decision_models_a_list_marks() {
        let model = |id: &str| CustomModel { id: id.into(), name: None, context_window: None, max_output: None, images: None };
        let listed = vec![(Use::Chat, model("openai/gpt-6-luna")), (Use::Decides, model("openai/gpt-6-luna-decisions")), (Use::Other, model("embed"))];
        let ids = |api, listed: &Vec<(Use, CustomModel)>| usable(api, listed.clone()).into_iter().map(|m| m.id).collect::<Vec<_>>();
        assert_eq!(ids(CustomApi::Decisions, &listed), ["openai/gpt-6-luna-decisions"]);
        assert_eq!(ids(CustomApi::ChatCompletions, &listed), ["openai/gpt-6-luna"]);
        // A vendor's own list marks nothing; its models are all there is to pick from.
        let own = vec![(Use::Chat, model("jev-latest")), (Use::Chat, model("jev-1.13")), (Use::Other, model("embed"))];
        assert_eq!(ids(CustomApi::SystemOne, &own), ["jev-latest", "jev-1.13"]);
    }

    #[test]
    fn kinds_are_slugs_of_the_name() {
        let mut credentials = Credentials::default();
        assert_eq!(custom_kind(&credentials, "OpenRouter"), "custom:openrouter");
        assert_eq!(custom_kind(&credentials, "  My Mac Studio (LM Studio) "), "custom:my-mac-studio-lm-studio");
        assert_eq!(custom_kind(&credentials, "本地模型"), "custom:provider");
        let provider = CustomProvider { name: "OpenRouter".into(), api: CustomApi::ChatCompletions, base_url: String::new(), api_key: String::new(), models: Vec::new(), created_at: 0 };
        credentials.custom.insert("custom:openrouter".into(), provider);
        assert_eq!(custom_kind(&credentials, "openrouter!"), "custom:openrouter-2");
    }

    #[test]
    fn model_lists_tell_windows_inputs_and_what_is_not_for_chat() {
        // OpenRouter
        let (using, model) = listed_model(&json!({
            "id": "anthropic/claude-sonnet-5", "name": "Anthropic: Claude Sonnet 5", "context_length": 1000000,
            "architecture": { "input_modalities": ["text", "image"], "output_modalities": ["text"] },
            "top_provider": { "max_completion_tokens": 128000 }
        }))
        .unwrap();
        assert_eq!(using, Use::Chat);
        assert_eq!(model.name.as_deref(), Some("Anthropic: Claude Sonnet 5"));
        assert_eq!((model.context_window, model.max_output, model.images), (Some(1_000_000), Some(128_000), Some(true)));
        // Anthropic
        let (_, model) = listed_model(&json!({ "type": "model", "id": "claude-opus-5", "display_name": "Claude Opus 5" })).unwrap();
        assert_eq!((model.name.as_deref(), model.context_window, model.images), (Some("Claude Opus 5"), None, None));
        // Mistral and vLLM
        let (_, model) = listed_model(&json!({ "id": "pixtral-large", "max_context_length": 131072, "capabilities": { "vision": true, "completion_chat": true } })).unwrap();
        assert_eq!((model.context_window, model.images), (Some(131_072), Some(true)));
        assert_eq!(listed_model(&json!({ "id": "mistral-embed", "capabilities": { "completion_chat": false } })).unwrap().0, Use::Other);
        assert_eq!(listed_model(&json!({ "id": "qwen3-32b", "max_model_len": 40960 })).unwrap().1.context_window, Some(40_960));
        // OpenAI lists models chat cannot use.
        let using = |entry: Value| listed_model(&entry).unwrap().0;
        assert_eq!(using(json!({ "id": "text-embedding-3-large", "object": "model" })), Use::Other);
        assert_eq!(using(json!({ "id": "gpt-4o-mini-tts" })), Use::Other);
        assert_eq!(using(json!({ "id": "gpt-image-1" })), Use::Other);
        assert_eq!(using(json!({ "id": "black-forest-labs/FLUX.1-schnell", "type": "image" })), Use::Other);
        assert_eq!(using(json!({ "id": "meta-llama/Llama-4-Scout", "type": "chat" })), Use::Chat);
        // Decision models, as OpenRouter and Vercel's AI Gateway mark them.
        assert_eq!(using(json!({ "id": "typesafe/jev-1.13", "architecture": { "output_modalities": ["decisions"] } })), Use::Decides);
        assert_eq!(using(json!({ "id": "openai/gpt-6-luna-decisions", "type": "evaluation", "modalities": { "output": ["text"] } })), Use::Decides);
        assert!(listed_model(&json!({ "id": "" })).is_none());
        // A bare array (Together) and Ollama's `models`.
        assert_eq!(listed_models(&json!([{ "id": "a" }, { "id": "b" }])).unwrap().len(), 2);
        assert_eq!(listed_models(&json!({ "models": [{ "name": "qwen3:8b" }] })).unwrap()[0].1.id, "qwen3:8b");
        assert!(listed_models(&json!({ "status": "ok" })).is_none());
    }

    struct ScratchApp(Arc<App>, std::path::PathBuf);

    impl Drop for ScratchApp {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.1);
        }
    }

    fn scratch_app() -> ScratchApp {
        let home = std::env::temp_dir().join(format!("lorca-provider-auth-{}", uuid::Uuid::new_v4()));
        ScratchApp(App::load(crate::config::Config { home: home.clone(), port: 0 }).unwrap(), home)
    }

    /// Answers each request with the next status and body, and hands back the request lines.
    fn serve(answers: Vec<(&'static str, String)>) -> (String, std::thread::JoinHandle<Vec<String>>) {
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let root = format!("http://{}", listener.local_addr().unwrap());
        let server = std::thread::spawn(move || {
            use std::io::{Read, Write};
            let mut seen = Vec::new();
            for (status, body) in answers {
                let (mut socket, _) = listener.accept().unwrap();
                let mut request = [0u8; 4096];
                let read = socket.read(&mut request).unwrap();
                seen.push(String::from_utf8_lossy(&request[..read]).to_string());
                let reply = format!("HTTP/1.1 {status}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}", body.len());
                socket.write_all(reply.as_bytes()).unwrap();
            }
            seen
        });
        (root, server)
    }

    fn input(name: &str, api: &str, base_url: &str, models: &[&str]) -> CustomInput {
        CustomInput { name: name.into(), api: api.into(), base_url: base_url.into(), models: models.iter().map(|m| m.to_string()).collect(), ..Default::default() }
    }

    #[tokio::test]
    async fn a_custom_provider_takes_the_models_its_server_lists() {
        let scratch = scratch_app();
        let app = &scratch.0;
        let list = json!({ "object": "list", "data": [
            { "id": "qwen3:8b", "object": "model" },
            { "id": "nomic-embed-text", "object": "model" },
            { "id": "llava", "object": "model", "context_window": 8192 }
        ]});
        let (root, server) = serve(vec![("200 OK", list.to_string())]);
        let kind = connect_custom(app, input("Ollama", "chat-completions", &format!("{root}/v1/chat/completions"), &[])).await.unwrap();
        assert_eq!(kind, "custom:ollama");
        let request = &server.join().unwrap()[0];
        assert!(request.starts_with("GET /v1/models "), "{request}");
        assert!(!request.to_ascii_lowercase().contains("authorization"), "no key, no header");

        let credentials = app.credentials.lock().unwrap();
        let provider = &credentials.custom["custom:ollama"];
        assert_eq!(provider.base_url, format!("{root}/v1"));
        assert_eq!(provider.models.iter().map(|m| m.id.as_str()).collect::<Vec<_>>(), ["qwen3:8b", "llava"]);
        assert_eq!(provider.models[1].context_window, Some(8192));
        let status = credentials.statuses().into_iter().find(|status| status.kind == "custom:ollama").unwrap();
        assert_eq!((status.name.as_deref(), status.api, status.detail.as_str()), (Some("Ollama"), Some(CustomApi::ChatCompletions), provider.base_url.as_str()));
        assert!(credentials.changed_at.contains_key("custom:ollama"));
    }

    #[tokio::test]
    async fn the_users_models_keep_their_order_and_take_what_the_list_says() {
        let scratch = scratch_app();
        let app = &scratch.0;
        let list = json!({ "data": [{ "id": "claude-opus-5", "display_name": "Claude Opus 5" }], "has_more": false });
        let (root, server) = serve(vec![("200 OK", list.to_string())]);
        let mut proxy = input("Claude proxy", "messages", &format!("{root}/v1"), &["claude-sonnet-5", "claude-opus-5", "claude-sonnet-5"]);
        proxy.api_key = " sk-proxy-1234 ".into();
        let kind = connect_custom(app, proxy).await.unwrap();
        let request = &server.join().unwrap()[0];
        assert!(request.starts_with("GET /v1/models?limit=1000 "), "{request}");
        assert!(request.contains("x-api-key: sk-proxy-1234"), "{request}");
        let provider = app.credentials.lock().unwrap().custom[&kind].clone();
        assert_eq!(provider.base_url, format!("{root}/v1"));
        assert_eq!(provider.models.iter().map(|m| m.id.as_str()).collect::<Vec<_>>(), ["claude-sonnet-5", "claude-opus-5"]);
        assert_eq!(provider.models[1].name.as_deref(), Some("Claude Opus 5"));

        // Saving it again under its kind keeps the kind and when it was added.
        let (root, server) = serve(vec![("404 Not Found", "{}".into())]);
        let mut edited = input("Claude", "messages", &root, &["claude-opus-5"]);
        edited.kind = Some(kind.clone());
        assert_eq!(connect_custom(app, edited).await.unwrap(), kind);
        server.join().unwrap();
        let saved = app.credentials.lock().unwrap().custom[&kind].clone();
        assert_eq!((saved.name.as_str(), saved.created_at, saved.models.len()), ("Claude", provider.created_at, 1));
    }

    #[tokio::test]
    async fn a_custom_provider_needs_a_key_it_takes_and_models_to_offer() {
        let scratch = scratch_app();
        let app = &scratch.0;
        let (root, server) = serve(vec![("401 Unauthorized", "{}".into()), ("401 Unauthorized", "{}".into()), ("404 Not Found", "{}".into())]);
        let mut keyed = input("Gateway", "chat-completions", &root, &["m"]);
        keyed.api_key = "bad".into();
        assert_eq!(connect_custom(app, keyed).await.unwrap_err(), "Gateway rejected that key");
        assert_eq!(connect_custom(app, input("Gateway", "chat-completions", &root, &["m"])).await.unwrap_err(), "Gateway needs an API key");
        assert!(connect_custom(app, input("Gateway", "chat-completions", &root, &[])).await.unwrap_err().contains("Add the model ids yourself"));
        server.join().unwrap();

        assert_eq!(connect_custom(app, input(" ", "chat-completions", &root, &["m"])).await.unwrap_err(), "Name the provider");
        assert_eq!(connect_custom(app, input("Anthropic", "messages", &root, &["m"])).await.unwrap_err(), "A provider named Anthropic exists already");
        assert!(connect_custom(app, input("Lab", "completions", &root, &["m"])).await.unwrap_err().starts_with("Unknown API"));
        assert!(app.credentials.lock().unwrap().custom.is_empty());
    }

    #[tokio::test]
    async fn the_picker_gets_the_chat_models_or_hears_there_is_no_list() {
        let scratch = scratch_app();
        let app = &scratch.0;
        let list = json!({ "data": [{ "id": "gpt-6-sol", "context_window": 1050000 }, { "id": "text-embedding-3-small" }] });
        let (root, server) = serve(vec![("200 OK", list.to_string()), ("404 Not Found", "{}".into()), ("401 Unauthorized", "{}".into())]);
        let models = list_custom_models(app, "OpenAI", "responses", &format!("{root}/v1/responses"), " sk-1 ").await.unwrap().unwrap();
        assert_eq!(models.iter().map(|m| (m.id.as_str(), m.context_window)).collect::<Vec<_>>(), [("gpt-6-sol", Some(1_050_000))]);
        assert_eq!(list_custom_models(app, "", "chat-completions", &root, "").await.unwrap(), None);
        assert_eq!(list_custom_models(app, "", "chat-completions", &root, "bad").await.unwrap_err(), "The server rejected that key");
        let requests = server.join().unwrap();
        assert!(requests[0].starts_with("GET /v1/models ") && requests[0].contains("authorization: Bearer sk-1"), "{}", requests[0]);
        assert!(list_custom_models(app, "", "chat-completions", "ftp://lab", "").await.unwrap_err().contains("http://"));
        assert!(app.credentials.lock().unwrap().custom.is_empty(), "listing saves nothing");
    }

    #[tokio::test]
    async fn deleting_a_custom_provider_reaches_every_device_as_a_change() {
        let scratch = scratch_app();
        let app = &scratch.0;
        let (root, server) = serve(vec![("404 Not Found", "{}".into())]);
        let kind = connect_custom(app, input("Lab", "responses", &root, &["gpt-oss-120b"])).await.unwrap();
        server.join().unwrap();
        disconnect(app, &kind).unwrap();
        let credentials = app.credentials.lock().unwrap();
        assert!(credentials.custom.is_empty());
        assert!(credentials.changed_at.contains_key(&kind), "the deletion is a change the merge carries");
        drop(credentials);
        assert_eq!(disconnect(app, &kind).unwrap_err(), format!("Unknown provider {kind}"));
    }
}
