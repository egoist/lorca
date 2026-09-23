# Providers

A provider connects the loop to a model. It receives a `ModelRequest` and returns a stream of `AssistantEvent`s for one assistant message.

```rust
#[async_trait]
pub trait Provider: Send + Sync {
    fn provider_id(&self) -> &str;
    fn model_id(&self) -> &str;
    async fn stream(&self, request: ModelRequest, cancel: CancellationToken) -> AssistantEventStream;
}

pub struct ModelRequest {
    pub system_prompt: String,
    pub messages: Vec<LlmMessage>,
    pub tools: Vec<ToolSpec>,     // name, description, parameters (JSON Schema)
}
```

`provider_id()` and `model_id()` are stamped on every assistant message the provider produces.

## Built-in providers

### Anthropic Messages

`AnthropicProvider` speaks the streaming Messages API (`POST {base_url}/v1/messages`, `x-api-key` auth, `anthropic-version: 2023-06-01`). Two servers implement it:

```rust
use agent::providers::AnthropicProvider;

// Anthropic: claude-opus-5 unless told otherwise, adaptive thinking, web search and web fetch
let provider = AnthropicProvider::anthropic(&api_key, None);
let provider = AnthropicProvider::anthropic(&api_key, Some("claude-sonnet-5"));

// DeepSeek through its Anthropic-compatible endpoint, with its web search (deepseek-flash by default)
let provider = AnthropicProvider::deepseek(&api_key, None);
let provider = AnthropicProvider::deepseek(&api_key, Some("deepseek-v4-pro"));

// Any other Messages API server: provider id, base URL, API key, model; no server tools
let provider = AnthropicProvider::new("proxy", "https://llm.example.com", &api_key, "claude-opus-5")
    .with_base_url("http://localhost:8080");
```

`server_tools`, `thinking`, `max_tokens`, `supports_images`, `cache`, `eager_tool_streaming`, `max_retries`, and `max_retry_delay_ms` are public fields. `anthropic()` declares `web_search_20260209` and `web_fetch_20260209` and sends `thinking: { type: "adaptive" }` (the basic `web_search_20250305` / `web_fetch_20250910` and no `thinking` for Haiku 4.5 and the 4.5 generation and earlier); `deepseek()` declares `web_search_20250305`, the tool DeepSeek's endpoint runs, and takes images only on a vision model. DeepSeek's OpenAI-compatible endpoint has no web search: it takes only `function` tools.

By default every request marks the system prompt, the last function tool, and the last user block with `cache_control: ephemeral`, so the conversation so far is the cached prefix of the next turn (DeepSeek's endpoint honors it too: the second turn of a search reads its whole prefix from cache); sets `eager_input_streaming` on function tools, so arguments stream as they are generated; and is retried twice before it streams when the server answers 408, 409, 429, or 5xx or the connection fails, waiting what `retry-after` asks (a wait above `max_retry_delay_ms`, 60 s by default, fails instead) or a backoff from half a second to eight.

| Transcript | Request |
| --- | --- |
| System prompt | `system`, when not blank. |
| User text and images | `text` and base64 `image` blocks. |
| Assistant text and tool calls | `text` and `tool_use` blocks. |
| Assistant thinking | A `thinking` block with its `signature`; thinking without one (a stream that was cut off) goes as text. |
| Assistant server blocks | Verbatim. |
| Tool result | A `tool_result` block: the result's text as a string, or `text` and `image` blocks when it holds images (`(see attached image)` when images only), and `is_error`. |
| Tools | The server tools first, then `input_schema` tools. |

The transcript goes through the [shared transform](#before-conversion) first, so seals and server blocks here are this provider's own and tool call ids from another model have the `^[a-zA-Z0-9_-]{1,64}$` shape this API wants. Consecutive same-role messages are merged into one, so a tool result always follows its call in the next message. Assistant messages that end up empty are left out.

From the stream it reads `text_delta`, `thinking_delta`, `signature_delta` (as `ThinkingSignature`), and `input_json_delta` for `tool_use` blocks. A `server_tool_use` block becomes a `ServerToolStart` once its input is complete (`web_search` with the query, `web_fetch` with the URL) and a `ServerBlock`; its `*_tool_result` block becomes the matching `ServerToolEnd` (a result whose content is an error object, or a list holding one, names the error code in the summary) and another `ServerBlock`. Any other block type is kept as a `ServerBlock` too, except a `fallback` block, which is fine before any output and an error after some. `message_delta` gives the stop reason and usage (`cache_read_input_tokens` as `cache_read`, `cache_creation_input_tokens` as `cache_write`, `output_tokens_details.thinking_tokens` as `reasoning`).

| `stop_reason` | Becomes |
| --- | --- |
| `end_turn`, `stop_sequence`, `pause_turn` | `Stop` |
| `max_tokens` | `Length` |
| `tool_use` | `ToolUse` |
| `refusal` | `Error`, with `stop_details.explanation` |
| `sensitive`, anything unknown | `Error`, naming the reason |
| none before `message_stop` | `Error` |

An `error` event and HTTP errors become an error message with the server's `error.message`.

`DEEPSEEK_API_KEY=… cargo test -p lorca-agent live_deepseek -- --ignored --nocapture` runs a search followed by a function call and continues the turn with the seals and server blocks replayed.

### Thinking levels

`with_thinking(Some(level))` on any built-in adapter asks for a `ThinkingLevel`: `Off`, `Minimal`, `Low`, `Medium`, `High`, `XHigh`, or `Max` (they parse from and print as those words). The adapter maps it to what the model takes, from the catalog entry:

| Model | Sent |
| --- | --- |
| Adaptive (Opus 5, Opus 5.5, Sonnet 5, Opus 4.8, Fable 5.1, DeepSeek) | `thinking: { type: "adaptive" }` and `output_config: { effort }` (`minimal` counts as `low`); `Off` is `{ type: "disabled" }`. A model that cannot stop thinking (Fable 5.1, Opus 5.5) runs `Off` at its lowest level. |
| Budget (Haiku 4.5) | `thinking: { type: "enabled", budget_tokens }` with 1024, 2048, 8192, or 16384 tokens and an output cap that leaves 1024 for the answer; `Off` sends no thinking. |
| OpenAI-compatible | `reasoning_effort`; `Off` sends nothing. |
| API-key Responses | `reasoning: { effort }`; `Off` sends nothing. |
| ChatGPT | `reasoning: { effort, summary: "auto" }` with `low`, `medium`, `high`, `xhigh`, or `max`; `Off` sends nothing. |
| Grok | `reasoning: { effort }` with `low`, `medium`, `high`, or `xhigh`; `Off` sends nothing. |

A level the model does not have becomes the nearest higher one it has. With no level set, the Anthropic adapter sends its `thinking` field as before and the others send nothing.

### The model catalog and cost

`agent::models` is a snapshot of [models.dev](https://models.dev) for the models the adapters offer: `ModelInfo { id, name, provider, context_window, max_output, reasoning, images, rates, tiers, thinking, levels }`. `models::find(provider, id)` looks one up (dated Anthropic ids match their base entry); `models::for_provider(provider)` lists a provider's, default first. Every built-in adapter resolves its entry at construction and reports it through `Provider::model_info`, so a harness can read the window and the levels; an unlisted model runs with none.

When the entry is known, the `Usage` of every message carries `cost` in dollars: input, output, cache reads, and cache writes at the model's rates, at the long-context tier when the request's input is above it. `Usage::add` sums usages and costs. ChatGPT and Grok sign-ins are not billed per token; their cost is what the work would cost at API rates.

### OpenAI-compatible chat completions

`OpenAiCompatProvider` speaks the streaming `/chat/completions` API. It works with any server that implements it.

```rust
use agent::providers::OpenAiCompatProvider;

// provider id, base URL, API key, model
let provider = OpenAiCompatProvider::new("ollama", "http://localhost:11434/v1", "ollama", "llama3.2");

// DeepSeek's OpenAI-compatible endpoint (deepseek-flash by default): no web search here
let provider = OpenAiCompatProvider::deepseek(&api_key, None);
```

It posts to `{base_url}/chat/completions` with bearer auth, `stream: true`, and usage reporting on.

| Transcript | Request |
| --- | --- |
| System prompt | A `system` message, when not blank. |
| User text | `content` as a string. |
| User text and images | `content` parts; images as `data:` URLs. |
| Assistant text and tool calls | `content` and `tool_calls`. Thinking and server blocks are not sent back. |
| Tool result | A `tool` message with the result's text (`(see attached image)` or `(no tool output)` when there is none), then a `user` message with the images, if any. |
| Tools | `function` tools. |

The transcript goes through the [shared transform](#before-conversion) first. `supports_images` (true by default), `max_retries` (2), and `max_retry_delay_ms` are public fields. From the stream it reads `content` deltas as text, `reasoning_content` or `reasoning` deltas as thinking, `tool_calls` deltas as tool calls, `finish_reason` as the stop reason (`length`, `tool_calls`, anything else as stop), and `usage`: `prompt_cache_hit_tokens` or `prompt_tokens_details.cached_tokens` as `cache_read`, `completion_tokens_details.reasoning_tokens` as `reasoning`. HTTP errors become an error message with the status and the server's `error.message`, after the same retries as the Anthropic adapter.

### OpenAI-compatible Responses

`OpenAiResponsesProvider` speaks an API-key Responses endpoint. Gateways such as OpenCode use it for GPT and Grok model families while ChatGPT and Grok subscription tokens stay in their isolated adapters.

```rust
use agent::providers::OpenAiResponsesProvider;

let provider = OpenAiResponsesProvider::new(
    "opencode",
    "https://opencode.ai/zen/v1",
    &api_key,
    "gpt-5.6-terra",
);
```

It posts the shared Responses input and function-tool shapes to `{base_url}/responses` with bearer auth, streams text, reasoning, tool calls, and usage through the same parser as the subscription adapters, and maps a selected thinking level to `reasoning.effort` when the model catalog declares effort levels.

### ChatGPT subscription

`ChatGptProvider` signs in with a ChatGPT account instead of an API key and calls the Codex responses backend (`https://chatgpt.com/backend-api/codex/responses`).

Tokens come from a `TokenSource` you implement. The provider refreshes them when they are within a minute of expiring and hands the new ones to `store`:

```rust
use std::path::PathBuf;
use std::sync::Arc;

use agent::providers::{ChatGptProvider, ChatGptTokens, TokenSource};
use async_trait::async_trait;

struct FileTokens(PathBuf);

#[async_trait]
impl TokenSource for FileTokens {
    async fn tokens(&self) -> Result<ChatGptTokens, String> {
        let bytes = tokio::fs::read(&self.0).await.map_err(|e| e.to_string())?;
        serde_json::from_slice(&bytes).map_err(|e| e.to_string())
    }

    async fn store(&self, tokens: ChatGptTokens) -> Result<(), String> {
        let bytes = serde_json::to_vec(&tokens).map_err(|e| e.to_string())?;
        tokio::fs::write(&self.0, bytes).await.map_err(|e| e.to_string())
    }
}

let provider = ChatGptProvider::new(Arc::new(FileTokens(path)), None); // default model
```

The tokens grant access to the account: keep them in the system keychain or a file only the user can read.

Get the first tokens with the OAuth sign-in in `agent::providers::chatgpt::oauth`. It is the authorization-code flow with PKCE that the Codex CLI uses, with a callback on `http://localhost:1455/auth/callback`:

```rust
use std::time::Duration;

use agent::providers::chatgpt::oauth;

let client = reqwest::Client::new();
let tokens = oauth::login(
    &client,
    |url| {
        println!("Sign in: {url}"); // or open the browser
        Ok(())
    },
    Duration::from_secs(300),
)
.await?;
FileTokens(path.clone()).store(tokens).await?;
```

`login` binds the callback listener, calls your `open_url` with the authorize URL, waits for the browser redirect, and exchanges the code. The pieces are public for other flows: `PkceFlow`, `wait_for_callback`, `exchange_code`, `refresh`, and `jwt_claims`.

Models: the default is `gpt-6-sol`; `gpt-6-astra` and `gpt-6-luna` also work with ChatGPT accounts. `*-codex` model ids are rejected for ChatGPT accounts.

Requests carry the backend's own `web_search` tool ahead of your function tools. The model searches and opens pages on the server side; each search or page read arrives as `ServerToolStart` and `ServerToolEnd` events (named `web_search` or `web_fetch`, with the query or URL as `detail` and a one-line `summary`). They show up in `message_update` and never enter the message content.

Reasoning summaries stream as thinking. User images are sent as `input_image`; tool results are sent as text. An incomplete response ends with `stop_reason` `Length`. The transcript goes through the [shared transform](#before-conversion) first, and a request that fails before it streams is retried like the others.

### Grok subscription

`GrokProvider` signs in with a Grok account (SuperGrok or X Premium+) instead of an API key and calls xAI's Responses API (`https://api.x.ai/v1/responses`) with the bearer. It shares the Responses wire shape with the ChatGPT adapter: the same `input` items, the same stream events, the same server-tool handling.

Tokens come from a `GrokTokenSource` you implement, the same two calls as `TokenSource`. Access tokens last about six hours; the provider refreshes one within five minutes of its end and hands the new tokens to `store`. xAI rotates the refresh token, so store what comes back at once.

```rust
use agent::providers::{GrokProvider, GrokTokens, GrokTokenSource};

let provider = GrokProvider::new(Arc::new(FileTokens(path)), None); // grok-4.7
```

`with_base_url` points it at another API root and `with_issuer` at another OAuth issuer, for a proxy or a test server.

Get the first tokens with the sign-in in `agent::providers::grok::oauth`: the authorization-code flow with PKCE that xAI's Grok CLI runs at `auth.x.ai`, with the public desktop client id, the scopes `openid profile email offline_access grok-cli:access api:access`, and a loopback callback bound to a free port (`http://127.0.0.1:<port>/callback`):

```rust
use agent::providers::grok::oauth::{self, Endpoints};

let tokens = oauth::login(&client, &Endpoints::xai(), |url| open_browser(url), Duration::from_secs(300)).await?;
```

The pieces are public for other flows: `PkceFlow`, `Callback`, `exchange_code`, `refresh`, `revoke`, and `jwt_claims`. The account id and email come from the id token, or from `/oauth2/userinfo` when it carries none.

Models: the default is `grok-4.7`; `grok-4.6` is also in the catalog.

Requests carry xAI's `web_search` and `x_search` tools ahead of your function tools. Each search arrives as `ServerToolStart` and `ServerToolEnd` events named `web_search` (with "Searched X for …" as the summary of an X search) or `web_fetch` for a page read.

## Before conversion

`agent::transform::transform_messages` is what every built-in adapter does to the transcript before converting it, after pi's `transformMessages`. Use it in your own:

```rust
use agent::transform::{transform_messages, TransformOptions};

let messages = transform_messages(&request.messages, &TransformOptions {
    provider: self.provider_id(),
    model: self.model_id(),
    supports_images: false,
    normalize_tool_call_id: Some(|id| id.replace('|', "_")),
});
```

- **Images** in user messages and tool results become one `(image omitted: model does not support images)` note when `supports_images` is false.
- **Another model's thinking** becomes plain text; its seals and server blocks are dropped. A message is the adapter's own when its `provider` and `model` match. Own thinking keeps its signature; empty thinking without one goes.
- **Tool call ids** from another model pass through `normalize_tool_call_id`, and their results are renamed to match.
- **Failed and aborted turns** (`stop_reason` `Error` or `Aborted`) are left out: they are incomplete, and replaying them is what makes APIs reject a request.
- **A call without a result** gets a `No result provided` error result before the next assistant turn, the next user message, or the end, so every `tool_use` has its `tool_result`.

## Retries and error classes

`agent::retry::send_with_retry(build, max_retries, max_delay_ms, &cancel)` sends the request `build` makes and repeats it on 408, 409, 429, 5xx, or a transport failure, honoring `x-should-retry`, `retry-after-ms`, and `retry-after`, with an exponential backoff (half a second doubling to eight, with jitter) otherwise. The sleep ends with cancellation. It answers `RequestFailure::Aborted`, `Transport`, or `Status { status, body }`; `failure.message()` is the text for the error event, with the server's `error.message` pulled out of a JSON body.

For a harness that restarts whole turns: `RetryPolicy` (enabled, `max_retries` 3, `base_delay_ms` 1000, capped at 60 s by default) with `delay_ms(attempt)`, `is_retryable_error(&str)` for messages that read like a transient failure (overloaded, rate limited, 5xx, a dropped connection, a stream that ended early) and never quota or billing exhaustion, `is_context_overflow(&message, context_window)` for the many ways providers say the prompt no longer fits (plus a reply whose input exceeds the window, or a `Length` stop with nothing generated, when the window is given), and `is_recoverable_length`.

## Writing a provider

`stream` returns `AssistantEventStream`, a pinned boxed `Stream<Item = AssistantEvent>`. The usual shape is to spawn the HTTP work and hand back a channel with `channel_stream`:

```rust
use agent::provider::channel_stream;
use agent::{AssistantEvent, AssistantEventStream, ModelRequest, Provider, StopReason, Usage};
use async_trait::async_trait;
use tokio::sync::mpsc;
use tokio_util::sync::CancellationToken;

struct Canned;

#[async_trait]
impl Provider for Canned {
    fn provider_id(&self) -> &str {
        "canned"
    }

    fn model_id(&self) -> &str {
        "canned-1"
    }

    async fn stream(&self, request: ModelRequest, cancel: CancellationToken) -> AssistantEventStream {
        let (tx, rx) = mpsc::channel(32);
        tokio::spawn(async move {
            if cancel.is_cancelled() {
                let _ = tx.send(AssistantEvent::Error { message: "Request aborted".into(), aborted: true }).await;
                return;
            }
            let reply = format!("I can see {} message(s) and {} tool(s).", request.messages.len(), request.tools.len());
            let _ = tx.send(AssistantEvent::Start).await;
            let _ = tx.send(AssistantEvent::TextStart { index: 0 }).await;
            let _ = tx.send(AssistantEvent::TextDelta { index: 0, delta: reply }).await;
            let _ = tx.send(AssistantEvent::TextEnd { index: 0 }).await;
            let _ = tx.send(AssistantEvent::Done { stop_reason: StopReason::Stop, usage: Usage::default() }).await;
        });
        channel_stream(rx)
    }
}
```

### Request options

Every `ModelRequest` carries `options: RequestOptions`, from `AgentLoopConfig::request` (or `AgentOptions::request`): extra `headers`, a `timeout` for the whole request, a `session_id` (sent as `x-session-affinity`, and as `prompt_cache_key` on the OpenAI-compatible adapter) so providers that route by session reuse their prompt cache, `metadata` (the Anthropic adapter forwards `user_id`), and `hooks`: a `RequestHooks` whose `api_key` can hand over a fresh key for this call (for tokens that expire), whose `before_payload` sees the body right before it is sent, and whose `after_response` sees the status and headers. `RequestOptionsPatch` changes options field by field. An adapter of your own applies them with `options.apply_to(request_builder)`, `options.before_payload(&mut body)`, `options.api_key(&own).await`, and `options.report(&response)`.

### The contract

**`stream` never fails.** Network errors, bad status codes, auth problems, and malformed responses are all reported in the stream as `Error { message, aborted: false }`. The loop turns that into an assistant message with `stop_reason` `Error`. Cancellation is `Error { aborted: true }`.

**Event order:**

1. `Start` once, when the response begins. The loop emits `message_start` here. Send it after the request succeeds, so a failed request produces only the error.
2. Content blocks, each opened, filled, and closed:
   - `TextStart { index }`, `TextDelta { index, delta }`, `TextEnd { index }`
   - `ThinkingStart { index }`, `ThinkingDelta { index, delta }`, `ThinkingEnd { index }`, and `ThinkingSignature { index, signature }` for a provider that seals its thinking
   - `ToolCallStart { index, id, name }`, `ToolCallDelta { index, delta }`, `ToolCallEnd { index }`
   - `ServerBlock { index, block }` for a block the provider owns (a server tool call, its result); sending it again for the same index replaces the block
3. Exactly one terminal event: `Done { stop_reason, usage }` or `Error { message, aborted }`. The loop stops reading after it.

**Block indices** are positions in the assistant message's `content`. Start at `0` and give each new block the next number, in the order the blocks start. Deltas for different open blocks may interleave.

**Tool call arguments** stream as raw JSON text in `ToolCallDelta`s and are parsed at `ToolCallEnd` with `agent::json::parse_streaming_json`: repaired when a string carries raw control characters or a bad escape, salvaged when the text was cut off, `{}` when nothing parses. The loop checks them against the tool's schema before the tool runs.

**Server-side tools** (tools the model's host runs, like web search) are reported with `ServerToolStart { id, name, detail }` and `ServerToolEnd { id, name, detail, summary }`. They take no index and the loop never executes them. `agent::provider::is_server_tool` recognizes the shared names `web_search` and `web_fetch`. A provider that needs the call and result back when the turn continues (Anthropic's Messages API) also records them as `ServerBlock`s, which do take an index and become `AssistantPart::ServerBlock` parts of the message; providers that do not (ChatGPT) send only the events.

The loop is forgiving at the edges:

- A stream that ends without a terminal event becomes `Error` (`Provider stream ended before completion`), or `Aborted` if the run was cancelled.
- `Done` with `stop_reason` `Stop` on a message that has tool calls is recorded as `ToolUse`.
- Tool calls left open are closed and parsed at the end.

`AssistantAccumulator` (in `agent::provider`) is the builder the loop uses to turn events into an `AssistantMessage`. Feed it your provider's events in tests to check the message they produce.

### Converting messages

`request.messages` is already filtered to `LlmMessage`s. Map each one to your API's format:

- `LlmMessage::User`: text and image parts.
- `LlmMessage::Assistant`: text, thinking (with an optional `signature`), tool calls, and server blocks. `assistant.text()` joins the text parts and `assistant.tool_calls()` lists the calls. Send seals and server blocks back only when the message is your own (`assistant.provider`); skip them otherwise.
- `LlmMessage::ToolResult`: `tool_call_id`, `content`, `is_error`. `result.text()` joins the text parts.

The transcript may hold assistant messages from another provider or model. `provider` and `model` on each message tell you where it came from, for APIs that only accept their own reasoning or signatures.
