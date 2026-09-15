//! Tinybot agent runtime, after pi-agent-core.
//!
//! - [`agent_loop`]: the low-level loop. Streams one assistant turn, executes tool calls,
//!   drains steering and follow-up queues, and emits [`AgentEvent`]s.
//! - [`Agent`]: a stateful wrapper that owns the transcript and the queues.
//! - [`Provider`]: a model adapter that turns a request into a stream of [`AssistantEvent`]s.
//! - [`Tool`]: something the model can call.
//! - `providers`: DeepSeek (OpenAI-compatible completions) and ChatGPT (subscription OAuth).

pub mod agent;
pub mod agent_loop;
pub mod provider;
pub mod providers;
pub mod sse;
pub mod tool;
pub mod tools;
pub mod types;

pub use agent::{Agent, AgentError, AgentHandle, AgentOptions, QueueMode};
pub use agent_loop::{
    agent_loop, run_agent_loop, run_agent_loop_continue, AfterToolCallContext, AfterToolCallResult,
    AgentContext, AgentLoopConfig, BeforeToolCallContext, BeforeToolCallResult, EventSink, LoopHooks, NoHooks,
    ShouldStopAfterTurnContext, ToolExecutionMode,
};
pub use provider::{AssistantEvent, AssistantEventStream, ModelRequest, Provider, ToolSpec};
pub use tool::{Tool, ToolError, ToolResult, ToolUpdateFn};
pub use types::{
    AgentEvent, AgentMessage, AssistantMessage, AssistantPart, ContentPart, LlmMessage, StopReason,
    ToolCall, ToolResultMessage, Usage, UserMessage,
};

/// Milliseconds since the Unix epoch.
pub fn now_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}
