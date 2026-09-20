//! Lorca agent runtime, after pi-agent-core.
//!
//! - [`agent_loop`]: the low-level loop. Streams one assistant turn, executes tool calls,
//!   drains steering and follow-up queues, and emits [`AgentEvent`]s.
//! - [`Agent`]: a stateful wrapper that owns the transcript and the queues.
//! - [`Provider`]: a model adapter that turns a request into a stream of [`AssistantEvent`]s.
//! - [`Tool`]: something the model can call.
//! - `providers`: Anthropic Messages (Anthropic and DeepSeek, with server-side web search),
//!   OpenAI-compatible Chat Completions and Responses, and ChatGPT and Grok (subscription OAuth).

pub mod agent;
pub mod agent_loop;
pub mod compaction;
pub mod estimate;
pub mod harness;
pub mod json;
pub mod models;
pub mod provider;
pub mod providers;
pub mod request;
pub mod retry;
pub mod schema;
pub mod sse;
pub mod tool;
pub mod tools;
pub mod transform;
pub mod types;

pub use agent::{Agent, AgentError, AgentHandle, AgentMessageQueue, AgentOptions, QueueMode};
pub use agent_loop::{
    agent_loop, run_agent_loop, run_agent_loop_continue, AfterToolCallContext, AfterToolCallResult,
    AgentContext, AgentLoopConfig, BeforeToolCallContext, BeforeToolCallResult, EventSink, LoopHooks, NoHooks,
    PrepareNextTurnContext, ShouldStopAfterTurnContext, ToolExecutionMode, TurnUpdate,
};
pub use models::ModelInfo;
pub use retry::{is_context_overflow, is_retryable_error, RetryPolicy};
pub use provider::{AssistantEvent, AssistantEventStream, ModelRequest, Provider, ToolSpec};
pub use request::{RequestHooks, RequestOptions, RequestOptionsPatch, ResponseInfo};
pub use tool::{Tool, ToolError, ToolResult, ToolUpdateFn};
pub use types::{
    AgentEvent, AgentMessage, AssistantMessage, AssistantPart, ContentPart, Cost, LlmMessage, StopReason,
    ThinkingLevel, ToolCall, ToolResultMessage, Usage, UserMessage,
};

/// Milliseconds since the Unix epoch.
pub fn now_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}
