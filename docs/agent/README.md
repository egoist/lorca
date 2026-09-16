# Agent

`agent` is an agent runtime for Rust, modeled on [pi](https://github.com/badlogic/pi-mono)'s `pi-agent-core`. It runs the loop that sits between a language model and your code: it streams an assistant message, executes the tool calls in it, feeds the results back, and repeats until the model stops. Every step is reported as an event.

It has four parts:

- **The loop** (`agent_loop`, `run_agent_loop`, `run_agent_loop_continue`): a stateless function over a context. It streams turns, runs tools, drains steering and follow-up queues, and emits `AgentEvent`s.
- **`Agent`**: a stateful wrapper that owns the transcript between runs and exposes steering, follow-ups, and abort through a cloneable `AgentHandle`.
- **`Provider`**: a model adapter that turns a request into a stream of `AssistantEvent`s. Three ship with the crate: Anthropic's Messages API (Anthropic, and DeepSeek through its Anthropic-compatible endpoint, both with server-side web search), OpenAI-compatible chat completions, and ChatGPT subscription sign-in.
- **`Tool`**: something the model can call. Seven coding tools ship with the crate: `read`, `write`, `edit`, `bash`, `grep`, `find`, `ls`.

## Install

```toml
[dependencies]
agent = { path = "crates/agent" }
async-trait = "0.1"
serde = { version = "1", features = ["derive"] }
serde_json = "1"
tokio = { version = "1", features = ["full"] }
tokio-util = "0.7"
```

The runtime is async and built on Tokio. Cancellation uses `tokio_util::sync::CancellationToken`; tool arguments and schemas are `serde_json::Value`.

## Quick start

A coding assistant over the current directory, printing the reply as it streams:

```rust
use std::io::Write;
use std::sync::Arc;

use agent::providers::AnthropicProvider;
use agent::tools::coding_tools;
use agent::{Agent, AgentEvent, AgentOptions, AssistantEvent};
use tokio::sync::mpsc;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let api_key = std::env::var("DEEPSEEK_API_KEY")?;
    let provider = AnthropicProvider::deepseek(&api_key, None);

    let mut options = AgentOptions::new(Arc::new(provider));
    options.system_prompt = "You are a concise coding assistant.".into();
    options.tools = coding_tools(std::env::current_dir()?);
    let mut agent = Agent::new(options);

    let (tx, mut rx) = mpsc::channel(256);
    let printer = tokio::spawn(async move {
        while let Some(event) = rx.recv().await {
            match event {
                AgentEvent::MessageUpdate { assistant_message_event: AssistantEvent::TextDelta { delta, .. }, .. } => {
                    print!("{delta}");
                    std::io::stdout().flush().ok();
                }
                AgentEvent::ToolExecutionStart { tool_name, args, .. } => println!("\n[{tool_name}] {args}"),
                AgentEvent::AgentEnd { .. } => println!(),
                _ => {}
            }
        }
    });

    agent.prompt("What does this project do? Look around before answering.", tx).await?;
    printer.await?;
    Ok(())
}
```

`prompt` resolves when the run has settled. By then `agent.messages` holds the whole transcript: the prompt, each assistant message, and each tool result.

Read events on a separate task, as above. The run waits for the channel to accept each event, so a receiver that is held but never read stalls the run once its buffer fills. A dropped receiver is fine.

## Guides

- [How it works](how-it-works.md): messages, the turn loop, event order, errors, cancellation
- [The `Agent`](agent.md): prompting, continuing, steering, follow-ups, abort, persistence
- [Tools](tools.md): writing a tool, streaming updates, terminating a run, the built-in coding tools
- [Providers](providers.md): the provider contract, the Anthropic Messages, OpenAI-compatible, and ChatGPT adapters, writing your own
- [Hooks](hooks.md): context transforms, custom messages, tool-call gates, stopping early
- [The low-level loop](loop.md): running the loop without `Agent`, event sinks
- [Coming from pi](coming-from-pi.md): how `pi-agent-core` concepts map here
