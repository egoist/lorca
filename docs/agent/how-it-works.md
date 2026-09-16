# How it works

## Messages

The transcript is a `Vec<AgentMessage>`:

```rust
pub enum AgentMessage {
    User(UserMessage),              // content: Vec<ContentPart>
    Assistant(AssistantMessage),    // content: Vec<AssistantPart>, stop_reason, usage, provider, model
    ToolResult(ToolResultMessage),  // tool_call_id, tool_name, content, details, is_error
    Custom { kind: String, data: Value, timestamp: u64 },
}
```

- `ContentPart` is `Text { text }` or `Image { data, mime_type }` (base64 data).
- `AssistantPart` is `Text { text }`, `Thinking { thinking }`, or `ToolCall(ToolCall { id, name, arguments })`.
- `Custom` holds anything your application wants in the transcript: notes, UI markers, summaries. The model never sees it unless a hook rewrites it (see [Hooks](hooks.md)).

A model only understands `LlmMessage` (`User`, `Assistant`, `ToolResult`). The loop keeps `AgentMessage`s throughout and converts right before each model call, with two hooks:

```
AgentMessage[] ──transform_context──▶ AgentMessage[] ──convert_to_llm──▶ LlmMessage[] ──▶ Provider
```

`transform_context` works on a copy for that one call, so pruning there never changes the stored transcript. The default `convert_to_llm` drops `Custom` messages.

`AssistantMessage::stop_reason` is one of:

| `StopReason` | Meaning |
| --- | --- |
| `Stop` | The model finished its answer. |
| `ToolUse` | The message contains tool calls. |
| `Length` | The output hit the model's length limit. |
| `Error` | The request or stream failed; `error_message` says why. |
| `Aborted` | The run was cancelled; `error_message` says so. |

Every message type is `Serialize` and `Deserialize`: `AgentMessage` is tagged by `role` (`user`, `assistant`, `tool_result`, `custom`), content parts and events by `type`. A transcript round-trips through JSON as is.

## A run

A run starts from a context (system prompt, messages, tools) and one or more prompt messages, or from the context alone when it continues. It then turns until nothing is left to do:

```
run
├─ poll steering queue
└─ loop
   ├─ while the last turn made tool calls, or steering messages are pending:
   │    turn
   │    ├─ append pending steering messages
   │    ├─ stream one assistant message from the provider
   │    │    stop_reason error or aborted → end the run
   │    ├─ execute its tool calls, append one tool result per call
   │    ├─ should_stop_after_turn? → end the run
   │    └─ poll steering queue
   └─ poll follow-up queue
        messages → they become pending, loop again
        none     → end the run
```

- A **turn** is one assistant message plus the tool calls it made.
- **Steering messages** land between turns: after the current turn's tool calls finish and before the next model call. Tool calls already in flight run to completion. A steering message queued while the model writes its final answer starts another turn.
- **Follow-up messages** wait until the agent would otherwise stop: no tool calls left and no steering pending.
- When every tool call in a turn returns `terminate`, the turn makes no further model call on its own. Steering and follow-ups still apply.

The run returns the messages it added, in order: prompts, assistant messages, tool results, and injected steering and follow-up messages.

## Events

A run emits `AgentEvent`s, each tagged with `type` when serialized:

| Event | Payload | When |
| --- | --- | --- |
| `agent_start` | | The run begins. |
| `turn_start` | | A turn begins. |
| `message_start` | `message` | A message enters the transcript: prompt, steering or follow-up message, assistant message, tool result. |
| `message_update` | `message`, `assistant_message_event` | Assistant messages only: the partial message so far, and the provider event that produced it. |
| `message_end` | `message` | The message is complete. |
| `tool_execution_start` | `tool_call_id`, `tool_name`, `args` | A tool call is about to run. |
| `tool_execution_update` | `tool_call_id`, `tool_name`, `args`, `partial_result` | A tool reported progress. |
| `tool_execution_end` | `tool_call_id`, `tool_name`, `result`, `is_error` | A tool call finished. |
| `turn_end` | `message`, `tool_results` | The assistant message and its tool results are in. |
| `agent_end` | `messages` | The run is over; `messages` are the ones it added. |

One prompt, one tool call, then an answer:

```
agent_start
turn_start
  message_start   user
  message_end     user
  message_start   assistant (empty)
  message_update  assistant ×n      ← text, thinking, and tool-call deltas
  message_end     assistant (tool call)
  tool_execution_start
  tool_execution_update ×n
  tool_execution_end
  message_start   tool_result
  message_end     tool_result
turn_end
turn_start
  message_start   assistant
  message_update  assistant ×n
  message_end     assistant (text)
turn_end
agent_end
```

A consumer that appends every `message_end` message to a list rebuilds the transcript exactly. `Agent` does this.

`message_update` carries the raw `AssistantEvent`: `TextDelta`, `ThinkingDelta`, `ToolCallStart`, `ToolCallDelta`, and so on. It also carries `ServerToolStart` and `ServerToolEnd` for tools the provider runs on its side, such as web search. Those are activity to show and never part of the message content.

### Tool execution order

With `ToolExecutionMode::Sequential`, each call runs start to finish before the next: `tool_execution_start`, updates, `tool_execution_end`, then its tool-result `message_start` and `message_end`.

With `ToolExecutionMode::Parallel` (the default):

1. Every call gets `tool_execution_start` and is prepared in order: lookup, argument check, `before_tool_call`. A call that fails preparation gets its `tool_execution_end` right away.
2. The prepared calls run concurrently. `tool_execution_end` fires in completion order.
3. Tool-result messages are emitted and appended in the order the model made the calls.

If any tool in the batch declares `execution_mode()` `Sequential`, the whole batch runs sequentially.

### Delivery

The loop awaits delivery of each event before it moves on. An `EventSink` sees the event first; then the event goes to the channel. So state you derive from events is always in step with the loop: when a tool starts, its `tool_execution_start` has already been handled. A closed channel is ignored; a full one makes the loop wait.

## Errors

Nothing in a run returns `Err` for a model or tool failure. Failures become messages the transcript keeps:

- **Provider failures** (network, HTTP status, auth, a stream that ends early) become an assistant message with `stop_reason` `Error` and an `error_message`. The run emits `turn_end` and `agent_end` and stops.
- **Tool failures** become a tool result with `is_error: true` and the error text as content, and the model sees it on the next turn. This covers a tool returning `Err`, an unknown tool name (`Tool <name> not found`), arguments that are not a JSON object, and a call blocked by `before_tool_call`.

The functions that start a run fail only on a precondition. `run_agent_loop_continue` needs a non-empty transcript that does not end with an assistant message, and `Agent::prompt` refuses to start while a run is active.

## Cancellation

Every run takes a `CancellationToken` (for `Agent`, `AgentHandle::abort` cancels it). The token reaches the provider's `stream` and every tool's `execute`:

- A provider that sees it ends its stream with `Error { aborted: true }`. A stream that simply stops after cancellation counts as aborted too. The assistant message gets `stop_reason` `Aborted`, and the run ends.
- A tool that sees it returns early. The built-in `bash` kills the command's whole process group.

If cancellation arrives while tools run, those tools finish or bail out, their results are appended, and the run ends no later than its next model call.
