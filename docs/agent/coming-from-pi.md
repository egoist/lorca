# Coming from pi

The runtime follows `pi-agent-core`'s design: the same message model, the same event names and order, steering and follow-up queues, tool-call hooks, and the stateful agent over a stateless loop. Code written against pi maps over closely.

## Names

| `pi-agent-core` (TypeScript) | `agent` (Rust) |
| --- | --- |
| `new Agent({ initialState, ... })` | `Agent::new(AgentOptions)` |
| `initialState.systemPrompt`, `tools`, `messages` | `AgentOptions::system_prompt`, `tools`, `messages` |
| `initialState.model` + `streamFn` | `AgentOptions::new(provider)` with an `Arc<dyn Provider>` |
| `agent.prompt(...)` | `agent.prompt(input, events_tx).await` |
| `agent.continue()` | `agent.continue_run(events_tx).await` |
| `agent.steer(message)`, `agent.followUp(message)` | `handle.steer(message)`, `handle.follow_up(message)` |
| `agent.abort()` | `handle.abort()` |
| `agent.subscribe(listener)` | The `mpsc::Sender<AgentEvent>` passed to each run |
| `agent.waitForIdle()` | Awaiting `prompt` / `continue_run` |
| `agent.reset()` | `agent.reset()` |
| `isStreaming`, `streamingMessage`, `pendingToolCalls`, `errorMessage` | `is_streaming()`, `streaming_message()`, `pending_tool_calls()`, `error_message()` |
| `steeringMode`, `followUpMode`: `"one-at-a-time"` / `"all"` | `QueueMode::OneAtATime` / `QueueMode::All` |
| `toolExecution`: `"parallel"` / `"sequential"` | `ToolExecutionMode::Parallel` / `ToolExecutionMode::Sequential` |
| `agentLoop(prompts, context, config)` | `agent_loop(prompts, context, config, cancel)` or `run_agent_loop(...)` |
| `agentLoopContinue(context, config)` | `run_agent_loop_continue(context, &config, &tx, cancel)` |
| `convertToLlm` | `LoopHooks::convert_to_llm` |
| `transformContext(messages, signal)` | `LoopHooks::transform_context(messages, &cancel)` |
| `beforeToolCall`, `afterToolCall` | `LoopHooks::before_tool_call`, `after_tool_call` |
| `shouldStopAfterTurn` | `LoopHooks::should_stop_after_turn` |
| `AgentTool { name, label, description, parameters, executionMode, execute }` | `impl Tool` with the same methods |
| `execute(toolCallId, params, signal, onUpdate)` | `execute(tool_call_id, args, cancel, on_update)` |
| Throwing from `execute` | Returning `Err(ToolError)` |
| `terminate: true` in a tool result | `ToolResult::terminating()` |
| `AbortSignal` | `CancellationToken` |
| `CustomAgentMessages` declaration merging | `AgentMessage::Custom { kind, data, timestamp }` |
| `agent_start` … `agent_end` events | `AgentEvent::AgentStart` … `AgentEvent::AgentEnd`, serialized with the same `type` names |

## How it plays out in Rust

**Models are providers.** A `Provider` is a trait object that streams `AssistantEvent`s for one request. The model id, API key or token source, base URL, and any reasoning settings live inside it. To switch models between runs, assign a new provider to `agent.provider`. See [Providers](providers.md).

**Events go to a channel per run.** Each `prompt` or `continue_run` takes an `mpsc::Sender<AgentEvent>`. The run waits on the channel, so a slow consumer slows the run, and a receiver that is kept but never read stalls it; drop the receiver when you do not need events. On the low-level loop, an `EventSink` is the awaited listener, like an async subscriber.

**The agent is borrowed during a run.** `prompt` takes `&mut self` until the run settles. Steering, follow-ups, and abort go through an `AgentHandle` you take beforehand and move to other tasks.

**Tool arguments are JSON values.** `parameters()` returns a JSON Schema as `serde_json::Value`, and `execute` receives the arguments as a JSON object. Validate by deserializing into a struct; a `serde_json::Error` converts into a `ToolError` the model reads.

**Custom messages are one variant.** Instead of extending a message union through declaration merging, put application entries in `AgentMessage::Custom` with a `kind` string and JSON `data`, and match on `kind` in `convert_to_llm`.

**Context size is managed in `transform_context`.** Window, summarize, or drop old turns there; the stored transcript stays whole. Transcripts serialize with serde for storage.

**Provider failures are messages.** As in pi, a failed request ends the run with an assistant message whose `stop_reason` is `Error` or `Aborted`, rather than an `Err` from `prompt`.
