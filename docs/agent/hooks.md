# Hooks

`LoopHooks` is how you shape a run from the outside. Every method has a default, so implement only what you need. Hooks never fail: when something goes wrong inside one, return a fallback.

```rust
#[async_trait]
pub trait LoopHooks: Send + Sync {
    async fn transform_context(&self, messages: Vec<AgentMessage>, cancel: &CancellationToken) -> Vec<AgentMessage>;
    fn convert_to_llm(&self, messages: &[AgentMessage]) -> Vec<LlmMessage>;
    async fn steering_messages(&self) -> Vec<AgentMessage>;
    async fn follow_up_messages(&self) -> Vec<AgentMessage>;
    async fn before_tool_call(&self, ctx: BeforeToolCallContext<'_>) -> Option<BeforeToolCallResult>;
    async fn after_tool_call(&self, ctx: AfterToolCallContext<'_>) -> Option<AfterToolCallResult>;
    async fn should_stop_after_turn(&self, ctx: ShouldStopAfterTurnContext<'_>) -> bool;
    async fn prepare_next_turn(&self, ctx: PrepareNextTurnContext<'_>) -> Option<TurnUpdate>;
}
```

Pass hooks as `AgentOptions::hooks` or `AgentLoopConfig::with_hooks`. `NoHooks` is the empty implementation.

With `Agent`, the agent's own queues answer `steering_messages` and `follow_up_messages`; use `AgentHandle::steer` and `follow_up` there. Your implementations of those two apply when you run [the loop directly](loop.md).

## Shaping the context

### `transform_context`

Called before every model call with the full transcript. What it returns is what the model gets for that call. The stored transcript is unchanged.

Use it to keep the context within the model's window, to summarize old turns, or to add per-call information. It receives the run's cancellation token for async work such as a summarization call.

Keep a window of recent messages, starting on a user message so every tool result keeps the call it answers:

```rust
use agent::{AgentMessage, LoopHooks};
use async_trait::async_trait;
use tokio_util::sync::CancellationToken;

struct Window;

#[async_trait]
impl LoopHooks for Window {
    async fn transform_context(&self, messages: Vec<AgentMessage>, _cancel: &CancellationToken) -> Vec<AgentMessage> {
        const KEEP: usize = 40;
        if messages.len() <= KEEP {
            return messages;
        }
        let mut start = messages.len() - KEEP;
        while start > 0 && !matches!(messages[start], AgentMessage::User(_)) {
            start -= 1;
        }
        messages[start..].to_vec()
    }
}
```

### `convert_to_llm` and custom messages

Called right after `transform_context`. It turns `AgentMessage`s into the `LlmMessage`s the provider receives. The default (`agent::agent_loop::default_convert_to_llm`) keeps user, assistant, and tool-result messages and drops `Custom` ones.

`AgentMessage::Custom { kind, data, timestamp }` lets your application keep its own entries in the transcript, where they survive persistence and show up in events. Rewrite the ones the model should see:

```rust
use agent::{AgentMessage, ContentPart, LlmMessage, LoopHooks, UserMessage};
use async_trait::async_trait;

struct Notes;

#[async_trait]
impl LoopHooks for Notes {
    fn convert_to_llm(&self, messages: &[AgentMessage]) -> Vec<LlmMessage> {
        messages
            .iter()
            .filter_map(|message| match message {
                AgentMessage::Custom { kind, data, timestamp } if kind == "note" => Some(LlmMessage::User(UserMessage {
                    content: vec![ContentPart::text(format!("[Note] {}", data["text"].as_str().unwrap_or_default()))],
                    timestamp: *timestamp,
                })),
                other => other.as_llm(),
            })
            .collect()
    }
}

let note = AgentMessage::Custom {
    kind: "note".into(),
    data: serde_json::json!({ "text": "The user prefers tabs." }),
    timestamp: agent::now_ms(),
};
```

Custom messages can be prompts, steering messages, or follow-ups like any other message.

## Queues

### `steering_messages`

Polled when a run starts and after every turn. Messages it returns are appended before the next model call, and a non-empty result keeps the run going even when the last turn made no tool calls.

### `follow_up_messages`

Polled when the run would otherwise end. A non-empty result starts another turn.

Both are polled once per point in the run, so a hook that returns one message per call delivers one per turn (or one per stop).

## Gating tool calls

### `before_tool_call`

Called for each tool call after the tool is found and its arguments pass the schema check, before `execute`. Return `BeforeToolCallResult { block: true, reason, terminate, .. }` to skip the call, or `args: Some(...)` to run it with other arguments; the reason (or `Tool execution was blocked`) becomes an error result the model sees, and `terminate: true` makes that result count toward ending the run after the batch, like a tool's own `terminate`.

```rust
use agent::{BeforeToolCallContext, BeforeToolCallResult, LoopHooks};
use async_trait::async_trait;

struct Guard;

#[async_trait]
impl LoopHooks for Guard {
    async fn before_tool_call(&self, ctx: BeforeToolCallContext<'_>) -> Option<BeforeToolCallResult> {
        let command = ctx.args["command"].as_str().unwrap_or_default();
        if ctx.tool_call.name == "bash" && command.contains("rm -rf") {
            return Some(BeforeToolCallResult { block: true, reason: Some("rm -rf is not allowed here".into()), args: None, terminate: false });
        }
        None
    }
}
```

The hook is async, so it can wait for a person to approve the call.

`BeforeToolCallContext` has `assistant_message`, `tool_call`, `args` (the arguments after coercion and validation, which is what the tool will run with), and `context` (the loop's `AgentContext`). A run cancelled while the hook waits gets an `Operation aborted` result for the call.

### `after_tool_call`

Called after `execute` returns, before `tool_execution_end`. Return an `AfterToolCallResult` to override any part of the result; fields left `None` keep their value.

| Field | Overrides |
| --- | --- |
| `content` | What the model sees. |
| `details` | The structured details. |
| `is_error` | Whether the result counts as an error. |
| `terminate` | The stop hint. |

```rust
use agent::{AfterToolCallContext, AfterToolCallResult, ContentPart, LoopHooks};
use async_trait::async_trait;

struct Redact;

#[async_trait]
impl LoopHooks for Redact {
    async fn after_tool_call(&self, ctx: AfterToolCallContext<'_>) -> Option<AfterToolCallResult> {
        if ctx.tool_call.name == "read" && ctx.result.text_content().contains("API_KEY=") {
            return Some(AfterToolCallResult {
                content: Some(vec![ContentPart::text("[redacted: file contains secrets]")]),
                ..Default::default()
            });
        }
        None
    }
}
```

`AfterToolCallContext` has `assistant_message`, `tool_call`, `args`, `result`, `is_error`, and `context`.

Calls that fail before execution (unknown tool, bad arguments, blocked) skip `after_tool_call`.

## Stopping early

### `should_stop_after_turn`

Called after each `turn_end`. Return `true` to end the run right there, before steering and follow-up queues are polled; queued messages stay queued.

```rust
use std::sync::atomic::{AtomicUsize, Ordering};

use agent::{LoopHooks, ShouldStopAfterTurnContext};
use async_trait::async_trait;

struct Budget {
    max_turns: usize,
    turns: AtomicUsize,
}

#[async_trait]
impl LoopHooks for Budget {
    async fn should_stop_after_turn(&self, _ctx: ShouldStopAfterTurnContext<'_>) -> bool {
        self.turns.fetch_add(1, Ordering::SeqCst) + 1 >= self.max_turns
    }
}
```

`ShouldStopAfterTurnContext` has `message` (the turn's assistant message), `tool_results`, `context`, and `new_messages` (everything the run has added so far).

A turn that stops this way can leave tool results without a following assistant message. `continue_run` picks up from there.

## Between turns

### `prepare_next_turn`

Called after `turn_end` when the run goes on, right before the next turn starts. It sees the same `PrepareNextTurnContext` as `should_stop_after_turn` (`message`, `tool_results`, `context`, `new_messages`). Return a `TurnUpdate` to replace the context or the provider for the turns that follow; `None` keeps both.

```rust
use agent::{LoopHooks, PrepareNextTurnContext, TurnUpdate};
use async_trait::async_trait;

struct Compact;

#[async_trait]
impl LoopHooks for Compact {
    async fn prepare_next_turn(&self, ctx: PrepareNextTurnContext<'_>) -> Option<TurnUpdate> {
        if ctx.context.messages.len() < 200 {
            return None;
        }
        let mut context = ctx.context.clone();
        context.messages = summarize(&context.messages).await;
        Some(TurnUpdate { context: Some(context), provider: None })
    }
}
```

Unlike `transform_context`, a replacement context here is the loop's context from then on: the messages the run returns are still only the ones it added, but the next turns build on what you handed back. Steering queued while the hook ran is picked up right after it, unless a message was already pending.
