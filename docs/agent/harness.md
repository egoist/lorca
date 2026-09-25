# The harness

`agent::harness::AgentHarness` is a general agent over the loop: a transcript in memory, a model and thinking level that can change between turns, tools, skills and prompt templates, compaction, retry, queues, hooks, and events. It is what pi's coding-agent harness is, without persistence: the host keeps the transcript wherever it likes (`messages()` in, `HarnessOptions::messages` back), and stores what it wants from the events.

## Building one

```rust
use std::sync::Arc;

use agent::harness::{
    build_system_prompt, load_prompt_templates, load_skills, AgentHarness, EnvProviderFactory, HarnessOptions, ModelIdentity,
    ProviderFactory, Resources, SystemPromptParts,
};
use agent::tools::coding_tools;

let factory: Arc<dyn ProviderFactory> = Arc::new(EnvProviderFactory::new());
let provider = factory.provider(&ModelIdentity::parse("anthropic/claude-opus-5"), None)?;

let skills = load_skills(&[cwd.join("skills")]);
let templates = load_prompt_templates(&[cwd.join("prompts")]);

let mut options = HarnessOptions::new(provider);
options.factory = Some(factory);
options.system_prompt = build_system_prompt(&SystemPromptParts {
    cwd: Some(&cwd),
    coding_tools: true,
    skills: &skills.skills,
    ..Default::default()
});
options.tools = coding_tools(cwd.clone());
options.resources = Resources { skills: skills.skills, prompt_templates: templates.templates };
let mut harness = AgentHarness::new(options);
```

| Option | Default |
| --- | --- |
| `provider` | required: the model to start with |
| `factory` | none; needed by `set_model` and `set_thinking_level` |
| `thinking_level` | none: the provider's own setting |
| `system_prompt` | empty; `build_system_prompt` makes the usual one |
| `tools`, `active_tools` | none; `active_tools: None` means every tool |
| `resources` | no skills, no templates |
| `request` | no headers, timeout, session id, or metadata |
| `retry` | `RetryPolicy::default()`: three retries from one second |
| `compaction` | `CompactionSettings::default()`: on, reserve 16k, keep 20k |
| `steering_mode`, `follow_up_mode` | `QueueMode::All` |
| `tool_execution` | `Parallel` |
| `messages` | empty |

`EnvProviderFactory` builds the built-in adapters from `DEEPSEEK_API_KEY`, `ANTHROPIC_API_KEY`, `CEREBRAS_API_KEY`, `OPENAI_API_KEY` (with `*_BASE_URL` overrides) a `TokenSource` for ChatGPT, and a `GrokTokenSource` for Grok (`GROK_BASE_URL` overrides its API root), and takes `register(provider, builder)` for anything else. Implement `ProviderFactory` yourself when keys live elsewhere.

## Running

```rust
let result = harness.prompt("Summarize README.md").await?;       // text, text with images, or messages
let result = harness.skill("deploy", Some("to staging")).await?;   // a SKILL.md by name
let result = harness.prompt_from_template("review", &["src/a.rs".into()]).await?;
let result = harness.continue_run().await?;                        // from the transcript as it is
```

`RunResult` has the run id, the `Outcome` (`Completed`, `Aborted`, `Failed { error }`), and the messages the run added. One `prompt` can hold several runs: a `before_run_end` hook that answers with a follow-up starts another, and so does a prompt queued with `next_run`.

`continue_run` continues after a user message or tool results. After an assistant message whose tool calls never got results (a run that was interrupted), the calls get error results asking the model to call again if still needed, and the run goes on from there. After a finished assistant message it takes queued steering, follow-up, or next-run messages as the prompt, else fails with `LastIsAssistant`.

Before a run, and between the turns of one, the harness estimates the context and compacts it when it passes `window - reserve_tokens`; when the model answers that the prompt is too long, it compacts and tries the turn once more. `compact(custom_instructions)` does it by hand. See [Compaction](compaction.md).

## From other tasks

`harness.handle()` is cheap to clone:

| Method | Effect |
| --- | --- |
| `steer(message)` | After the current turn's tool calls, before the next model call. |
| `follow_up(message)` | When the run would otherwise stop; keeps it going. |
| `next_run(message)` | A new run once this one ends. |
| `cancel_queued(id)` | Removes a queued message. |
| `queued()` | What is waiting. |
| `abort()` | Cancels the run. |

Each queue call returns the message's id and sends a `queue_update` event.

## Changing things

`set_model(&ModelIdentity)` and `set_thinking_level(level)` build a new provider through the factory; `set_provider(provider)` takes one directly. `set_tools`, `set_active_tools`, `set_resources`, `set_request_options`, `set_retry_policy`, `set_compaction_settings`, `set_steering_mode`, `set_follow_up_mode`, and `set_tool_execution` change the rest; `system_prompt` is a public field. Every change sends a `config_update` event with the property's name and new value. Changes apply from the next run; a run in flight keeps what it started with.

`stats()` is the harness at a glance: the model, thinking level, the context as it would be sent now (estimated), the window, the usage and cost totals since the harness was made, the message count, and whether a run is active. `record_usage` adds usage the host incurred elsewhere to the totals.

## Events

`subscribe()` returns a channel that gets every `HarnessEvent` (tagged `type` when serialized); `events().listen(listener)` registers an awaited listener that runs before the channels are fed. A full channel makes the run wait, so read it on its own task.

| Event | When |
| --- | --- |
| `run_start`, `run_end` | A run begins and ends, with its `Outcome`. |
| `turn_start`, `turn_end` | A turn, with the assistant message and its tool results. |
| `retry_scheduled` | A model call failed before streaming and will be asked again. |
| `message_start`, `message_update`, `message_end` | The loop's message events. |
| `tool_start`, `tool_update`, `tool_end` | The loop's tool events. |
| `queue_update` | A queue changed. |
| `config_update` | A setting changed. |
| `compaction_start`, `compaction_end` | A compaction, with its reason (`manual`, `threshold`, `overflow`) and outcome. |
| `usage` | A model call's usage and the totals. |

## Hooks

`hooks().register(id, Arc<dyn HarnessHooks>)` adds hooks; every method has a default that changes nothing. In registration order:

| Hook | Sees | Can |
| --- | --- | --- |
| `before_run` | the prompt and the resources | add messages after the prompt |
| `transform_context` | the messages and system prompt | replace either (the system prompt part applies at run start, the messages before every call) |
| `before_request` | the step and the request options | patch the options |
| `before_payload`, `after_response` | the request body, the response status and headers | change the body in place, observe the response |
| `after_message` | the assistant message a call ended with | replace it before it is recorded |
| `before_tool` | a checked call | replace its arguments or block it (the first block wins) |
| `after_tool` | a result | replace its content, details, error flag, or stop hint |
| `before_compaction` | the messages about to be summarized | decline, or hand over a summary of its own |
| `before_run_end` | the run's messages | answer with a follow-up prompt that starts another run |

## Skills, templates, and the system prompt

`load_skills(dirs)` finds `SKILL.md` files (agentskills.io) and root `.md` files with a `description`; `format_skills_for_system_prompt` lists the model-visible ones and `build_system_prompt` includes them. `harness.skill(name, extra)` runs one. `load_prompt_templates(paths)` reads `.md` files whose `$1`, `$@`, `$ARGUMENTS`, `${@:N}`, and `${@:N:L}` are filled from `parse_command_args`; `harness.prompt_from_template(name, args)` runs one.

`crates/agent/examples/chat.rs` is a terminal chat over all of this: `cargo run -p lorca-agent --example chat -- deepseek/deepseek-flash`.
