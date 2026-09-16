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

### OpenAI-compatible chat completions

`OpenAiCompatProvider` speaks the streaming `/chat/completions` API. It works with any server that implements it.

```rust
use agent::providers::OpenAiCompatProvider;

// provider id, base URL, API key, model
let provider = OpenAiCompatProvider::new("ollama", "http://localhost:11434/v1", "ollama", "llama3.2");

// DeepSeek, with its base URL and default model (deepseek-flash)
let provider = OpenAiCompatProvider::deepseek(&api_key, None);
let provider = OpenAiCompatProvider::deepseek(&api_key, Some("deepseek-v4-pro"));
```

It posts to `{base_url}/chat/completions` with bearer auth, `stream: true`, and usage reporting on.

| Transcript | Request |
| --- | --- |
| System prompt | A `system` message, when not blank. |
| User text | `content` as a string. |
| User text and images | `content` parts; images as `data:` URLs. |
| Assistant text and tool calls | `content` and `tool_calls`. Thinking is not sent back. |
| Tool result | A `tool` message with the result's text. |
| Tools | `function` tools. |

From the stream it reads `content` deltas as text, `reasoning_content` deltas as thinking, `tool_calls` deltas as tool calls, `finish_reason` as the stop reason (`length`, `tool_calls`, anything else as stop), and `usage` including `prompt_cache_hit_tokens` as `cache_read`. HTTP errors become an error message with the status and the server's `error.message`.

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

Models: the default is `gpt-5.6-terra`; `gpt-6-astra`, `gpt-5.6-sol`, `gpt-5.6-luna`, and `gpt-5.5` also work with ChatGPT accounts. `*-codex` model ids are rejected for ChatGPT accounts.

Requests carry the backend's own `web_search` tool ahead of your function tools. The model searches and opens pages on the server side; each search or page read arrives as `ServerToolStart` and `ServerToolEnd` events (named `web_search` or `web_fetch`, with the query or URL as `detail` and a one-line `summary`). They show up in `message_update` and never enter the message content.

Reasoning summaries stream as thinking. User images are sent as `input_image`; tool results are sent as text. An incomplete response ends with `stop_reason` `Length`.

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

### The contract

**`stream` never fails.** Network errors, bad status codes, auth problems, and malformed responses are all reported in the stream as `Error { message, aborted: false }`. The loop turns that into an assistant message with `stop_reason` `Error`. Cancellation is `Error { aborted: true }`.

**Event order:**

1. `Start` once, when the response begins. The loop emits `message_start` here. Send it after the request succeeds, so a failed request produces only the error.
2. Content blocks, each opened, filled, and closed:
   - `TextStart { index }`, `TextDelta { index, delta }`, `TextEnd { index }`
   - `ThinkingStart { index }`, `ThinkingDelta { index, delta }`, `ThinkingEnd { index }`
   - `ToolCallStart { index, id, name }`, `ToolCallDelta { index, delta }`, `ToolCallEnd { index }`
3. Exactly one terminal event: `Done { stop_reason, usage }` or `Error { message, aborted }`. The loop stops reading after it.

**Block indices** are positions in the assistant message's `content`. Start at `0` and give each new block the next number, in the order the blocks start. Deltas for different open blocks may interleave.

**Tool call arguments** stream as raw JSON text in `ToolCallDelta`s and are parsed at `ToolCallEnd`. Empty text becomes `{}`.

**Server-side tools** (tools the model's host runs, like web search) are reported with `ServerToolStart { id, name, detail }` and `ServerToolEnd { id, name, detail, summary }`. They take no index and the loop never executes them. `agent::provider::is_server_tool` recognizes the shared names `web_search` and `web_fetch`.

The loop is forgiving at the edges:

- A stream that ends without a terminal event becomes `Error` (`Provider stream ended before completion`), or `Aborted` if the run was cancelled.
- `Done` with `stop_reason` `Stop` on a message that has tool calls is recorded as `ToolUse`.
- Tool calls left open are closed and parsed at the end.

`AssistantAccumulator` (in `agent::provider`) is the builder the loop uses to turn events into an `AssistantMessage`. Feed it your provider's events in tests to check the message they produce.

### Converting messages

`request.messages` is already filtered to `LlmMessage`s. Map each one to your API's format:

- `LlmMessage::User`: text and image parts.
- `LlmMessage::Assistant`: text, thinking, and tool calls. `assistant.text()` joins the text parts and `assistant.tool_calls()` lists the calls.
- `LlmMessage::ToolResult`: `tool_call_id`, `content`, `is_error`. `result.text()` joins the text parts.

The transcript may hold assistant messages from another provider or model. `provider` and `model` on each message tell you where it came from, for APIs that only accept their own reasoning or signatures.
