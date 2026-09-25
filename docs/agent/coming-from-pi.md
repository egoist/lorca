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
| `prepareNextTurn` returning `{ context, model }` | `LoopHooks::prepare_next_turn` returning `TurnUpdate { context, provider }` |
| `prompt(text, images)` | `agent.prompt(PromptInput::with_images(text, images), tx)` |
| `AgentTool { name, label, description, parameters, executionMode, prepareArguments, execute }` | `impl Tool` with the same methods |
| `validateToolArguments` (coercion, then the schema) | Built into the loop, before `before_tool_call`; `agent::schema::validate_tool_arguments` on its own |
| `BeforeToolCallResult { block, reason, terminate }` | The same fields |
| A `length` stop failing its tool calls | The same |
| `transformMessages` (`pi-ai`) | `agent::transform::transform_messages` |
| `parseStreamingJson`, `parseJsonWithRepair` (`pi-ai`) | `agent::json::parse_streaming_json`, `parse_json_with_repair` |
| `retryProviderRequest` (`pi-ai`) | `agent::retry::send_with_retry`, inside every built-in adapter |
| `RetryPolicy`, `isRetryableAssistantError`, `isContextOverflow` (`pi-ai`) | `agent::retry::{RetryPolicy, is_retryable_assistant_error, is_context_overflow}` |
| The harness's turn retry (`retry_scheduled`) | `AgentLoopConfig::with_retry`, the `retry` event |
| `ThinkingLevel`, `reasoning` in stream options | `ThinkingLevel` and `with_thinking(level)` on each adapter |
| `Model` (cost, context window, compat), `calculateCost` | `agent::models::{ModelInfo, find}`, `Provider::model_info`, `Usage.cost` |
| `estimateContextTokens`, `shouldCompact`, `compact`, the summarization prompts | `agent::estimate` and `agent::compaction` |
| `AgentHarness` (`prompt`, `skill`, `promptFromTemplate`, `compact`, `steer`, `followUp`, `nextRun`, `abort`, `setModel`, `setThinkingLevel`, `hooks`, `events`) | `agent::harness::AgentHarness` with the same operations; no session, the host keeps the transcript |
| `Session`, `SessionRepo`, branches, `navigateTree`, branch summaries | Not here: persistence and branching are the host's |
| `loadSkills`, `formatSkillsForSystemPrompt`, `formatSkillInvocation` | `agent::harness::{load_skills, format_skills_for_system_prompt, format_skill_invocation}` |
| `loadPromptTemplates`, `parseCommandArgs`, `substituteArgs` | `agent::harness::{load_prompt_templates, parse_command_args, substitute_args}` |
| `Models` + `getApiKey` | `ProviderFactory` (`EnvProviderFactory` reads the environment) and `RequestHooks::api_key` per call |
| `onPayload`, `onResponse`, `sessionId`, `headers`, `timeoutMs` | `RequestOptions` and `RequestHooks` on every model call |
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

**Provider failures are messages.** As in pi, a failed request ends the run with an assistant message whose `stop_reason` is `Error` or `Aborted`, rather than an `Err` from `prompt`. A run that fails outside the loop emits that message's `message_start`, `message_end`, `turn_end`, and `agent_end`, as pi's `Agent` does.

**Commands can take input.** pi's bash runs a command on pipes with nothing on stdin and fails fast when the command wants input, because the person is at the terminal and answers there. `coding_tools` does the same. A host whose commands run where nobody sits, as on Lorca's Runners, builds them with `coding_tools_with_sessions` instead: each command gets a pseudo-terminal, a command that stops at a question or goes quiet returns with a session id rather than holding the run, and `bash_input` and `bash_output` answer and read it. The host keeps those terminal sessions and ends them. See [Commands in a terminal](tools.md#commands-in-a-terminal).

**Not here.** Sessions, branches, branch summaries, and the durable operation state of pi's coding agent: persistence is the host's job. The harness keeps its transcript in memory, hands it back through `messages()`, and tells the host everything through events. Telemetry is `tracing`; the execution-environment abstraction (a pluggable filesystem and shell) is the real filesystem.
