# The low-level loop

The loop functions run one agent run over a context you provide and hand back the messages it added. They keep no state between runs. Use them when your application already stores the transcript (a database, a chat history) and rebuilds the context for each run.

## Context and config

```rust
pub struct AgentContext {
    pub system_prompt: String,
    pub messages: Vec<AgentMessage>,
    pub tools: Vec<Arc<dyn Tool>>,
}

pub struct AgentLoopConfig {
    pub provider: Arc<dyn Provider>,
    pub hooks: Arc<dyn LoopHooks>,             // default NoHooks
    pub tool_execution: ToolExecutionMode,     // default Parallel
    pub sink: Option<Arc<dyn EventSink>>,      // default None
}
```

```rust
let config = AgentLoopConfig::new(provider)
    .with_hooks(Arc::new(MyHooks))
    .with_sink(Arc::new(MySink));

let config = AgentLoopConfig {
    tool_execution: ToolExecutionMode::Sequential,
    ..AgentLoopConfig::new(provider)
};
```

The loop works on its own copy of the context. Your copy is untouched; add the returned messages to it yourself.

## Three entry points

### `agent_loop`: spawned, with a receiver

```rust
use agent::{agent_loop, AgentContext, AgentEvent, AgentLoopConfig, AgentMessage};
use tokio_util::sync::CancellationToken;

let context = AgentContext {
    system_prompt: "You are a helpful assistant.".into(),
    messages: history.clone(),
    tools,
};
let cancel = CancellationToken::new();

let (mut events, run) = agent_loop(vec![AgentMessage::user("Hello!")], context, config, cancel.clone());
while let Some(event) = events.recv().await {
    if let AgentEvent::TurnEnd { tool_results, .. } = &event {
        println!("turn finished with {} tool result(s)", tool_results.len());
    }
}
history.extend(run.await?);
```

`agent_loop` spawns the run on the Tokio runtime and returns an event receiver (buffer of 256) and a `JoinHandle` that resolves to the new messages. Cancel with `cancel.cancel()`.

### `run_agent_loop`: in the current task

```rust
let (tx, mut rx) = tokio::sync::mpsc::channel(256);
let new_messages = run_agent_loop(prompts, context, &config, &tx, cancel).await;
```

Same run, awaited in place, with events sent on your channel. Read `rx` from another task, or drop it.

### `run_agent_loop_continue`: no new prompt

```rust
let new_messages = run_agent_loop_continue(context, &config, &tx, cancel).await?;
```

Runs from the context as it is. The last message must not be an assistant message: typically a user message you already stored, or tool results from an interrupted run. It returns `LoopError::Empty` (from `agent::agent_loop`) for an empty transcript and `LoopError::LastIsAssistant` when the model already had the last word. `run_agent_loop_continue` emits no prompt `message_start`/`message_end`, since there is no prompt.

## Event sinks

An `EventSink` receives every event before it goes to the channel, and the loop awaits it:

```rust
#[async_trait]
pub trait EventSink: Send + Sync {
    async fn on_event(&self, event: &AgentEvent);
}
```

Because the loop waits for the sink, whatever the sink does is finished before the loop's next step. A sink that writes each completed message to storage has written the assistant's tool call before the tool runs, and the tool result before the next model call.

```rust
use agent::{run_agent_loop_continue, AgentEvent, AgentLoopConfig, EventSink};
use async_trait::async_trait;
use tokio::sync::mpsc;
use tokio_util::sync::CancellationToken;

struct Store;

#[async_trait]
impl EventSink for Store {
    async fn on_event(&self, event: &AgentEvent) {
        if let AgentEvent::MessageEnd { message } = event {
            append_to_storage(message).await;
        }
    }
}

let config = AgentLoopConfig::new(provider).with_sink(Arc::new(Store));
let (tx, rx) = mpsc::channel(1);
drop(rx); // events go to the sink only
let new_messages = run_agent_loop_continue(context, &config, &tx, CancellationToken::new()).await?;
```

A slow sink slows the run: the model stream is not read while the sink handles a `message_update`. Keep per-delta work light, or hand it to another task.

## Steering and follow-ups without `Agent`

Implement `steering_messages` and `follow_up_messages` on your hooks and read from wherever new input arrives:

```rust
use std::sync::Mutex;

use agent::{AgentMessage, LoopHooks};
use async_trait::async_trait;

struct Inbox {
    steering: Mutex<Vec<AgentMessage>>,
}

#[async_trait]
impl LoopHooks for Inbox {
    async fn steering_messages(&self) -> Vec<AgentMessage> {
        std::mem::take(&mut *self.steering.lock().unwrap())
    }
}
```

See [Hooks](hooks.md#queues) for when each is polled.
