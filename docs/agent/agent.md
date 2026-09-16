# The `Agent`

`Agent` owns a transcript and runs the loop over it. Use it when one conversation lives across many prompts. For one-shot runs over a transcript you store elsewhere, [the low-level loop](loop.md) is enough.

## Creating an agent

```rust
use std::sync::Arc;

use agent::{Agent, AgentOptions, QueueMode, ToolExecutionMode};

let mut options = AgentOptions::new(provider);            // Arc<dyn Provider>
options.system_prompt = "You are a helpful assistant.".into();
options.tools = tools;                                    // Vec<Arc<dyn Tool>>
options.messages = history;                               // start from a saved transcript
options.hooks = Arc::new(MyHooks);                        // Arc<dyn LoopHooks>
options.steering_mode = QueueMode::OneAtATime;
options.follow_up_mode = QueueMode::OneAtATime;
options.tool_execution = ToolExecutionMode::Parallel;
let mut agent = Agent::new(options);
```

| Option | Default |
| --- | --- |
| `system_prompt` | empty |
| `tools` | none |
| `messages` | empty |
| `hooks` | `NoHooks` |
| `steering_mode`, `follow_up_mode` | `QueueMode::OneAtATime` |
| `tool_execution` | `ToolExecutionMode::Parallel` |

`system_prompt`, `provider`, `tools`, `messages`, and `tool_execution` are public fields on `Agent` too. Change them between runs to swap the model, the tool set, or the transcript; a run takes a snapshot when it starts.

## Prompting

```rust
let (tx, rx) = tokio::sync::mpsc::channel(256);
let added = agent.prompt("Summarize README.md", tx).await?;
```

`prompt` accepts a `&str`, a `String`, a `Vec<ContentPart>`, one `AgentMessage`, or a `Vec<AgentMessage>`. Send text with images through `PromptInput::with_images`:

```rust
use agent::agent::PromptInput;
use agent::ContentPart;

let images = vec![ContentPart::Image { data: base64_png, mime_type: "image/png".into() }];
agent.prompt(PromptInput::with_images("What is in this screenshot?", images), tx).await?;
```

Events arrive on the receiver while the run is in flight; the agent applies each one to its own state before forwarding it. `prompt` resolves once the run has settled, with the messages it added; `agent.messages` has them too.

Drain the receiver on another task, or drop it. A receiver that is held but not read stalls the run once its buffer fills.

## Continuing

```rust
agent.continue_run(tx).await?;
```

`continue_run` runs from the transcript as it is, without a new prompt. Use it after editing `agent.messages`, after loading a transcript that ends with a user message or tool result, or to retry after an error you removed.

When the transcript ends with an assistant message, `continue_run` takes queued steering messages as the prompt, then queued follow-ups. With neither queued it returns `AgentError::LastIsAssistant`.

## Steering, follow-ups, and abort

`prompt` borrows the agent mutably for the whole run. To act on a run in flight, take an `AgentHandle` first. It is cheap to clone and `Send`:

```rust
use std::time::Duration;

use agent::AgentMessage;

let handle = agent.handle();
tokio::spawn(async move {
    tokio::time::sleep(Duration::from_secs(10)).await;
    handle.steer(AgentMessage::user("Skip the integration tests."));
    handle.follow_up(AgentMessage::user("Then write a one-line summary."));
});

agent.prompt("Run the test suite and fix what fails.", tx).await?;
```

| `AgentHandle` method | Effect |
| --- | --- |
| `steer(message)` | Queue a message for after the current turn's tool calls, before the next model call. |
| `follow_up(message)` | Queue a message for when the agent would otherwise stop. |
| `abort()` | Cancel the active run. The model call it interrupts ends with `stop_reason` `Aborted`, and the run ends. |
| `clear_steering_queue()`, `clear_follow_up_queue()`, `clear_all_queues()` | Drop queued messages. |
| `has_queued_messages()` | Whether either queue holds anything. |
| `is_running()` | Whether a run is active. |

Queue modes decide how much each poll takes:

- `QueueMode::OneAtATime` takes the oldest message. The next one waits for the following turn (steering) or the following stop (follow-ups).
- `QueueMode::All` takes everything queued at once.

Change them later with `agent.set_steering_mode(mode)` and `agent.set_follow_up_mode(mode)`.

Queues outlive runs. A message queued while no run is active is picked up by the next `prompt` (steering, right after the prompt) or `continue_run`.

## State

While a run is in flight the agent is borrowed by `prompt`, so live state comes from events: the partial reply from `message_update`, running tools from `tool_execution_start` and `tool_execution_end`. Between runs:

| Method | Returns |
| --- | --- |
| `error_message()` | The error of the last failed turn in the latest run, if any. |
| `is_streaming()` | Whether a run is active (`AgentHandle::is_running` answers the same from other tasks). |
| `streaming_message()`, `pending_tool_calls()` | The in-flight message and tool call ids; empty once a run settles. |
| `reset()` | Clears the transcript, runtime state, and both queues. |

## Persisting a transcript

Messages serialize with serde:

```rust
let json = serde_json::to_string(&agent.messages)?;

// later
let mut options = AgentOptions::new(provider);
options.messages = serde_json::from_str(&json)?;
let mut agent = Agent::new(options);
```

To save as you go, append each `message_end` message from the event stream.
