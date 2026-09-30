use std::sync::Arc;

use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use tokio_util::sync::CancellationToken;

use crate::agent_loop::ToolExecutionMode;
use crate::provider::ToolSpec;
use crate::types::ContentPart;

/// Final or partial output of a tool.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct ToolResult {
    /// What the model sees.
    pub content: Vec<ContentPart>,
    /// Structured details for logs or UI rendering.
    #[serde(default)]
    pub details: Value,
    /// Machine-readable output matching the tool's `output_schema`. A codemode script receives
    /// it instead of the text; the model never sees it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub structured: Option<Value>,
    /// The call failed. A tool returns an error result instead of an `Err` when the failure has
    /// content or structured output to go with it.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub is_error: bool,
    /// Hint that the agent should stop after this batch. Only honored when every tool in the
    /// batch sets it.
    #[serde(default)]
    pub terminate: bool,
}

impl ToolResult {
    pub fn text(text: impl Into<String>) -> Self {
        ToolResult { content: vec![ContentPart::text(text)], ..ToolResult::default() }
    }

    pub fn with_details(mut self, details: Value) -> Self {
        self.details = details;
        self
    }

    pub fn terminating(mut self) -> Self {
        self.terminate = true;
        self
    }

    pub fn text_content(&self) -> String {
        self.content.iter().filter_map(ContentPart::as_text).collect::<Vec<_>>().join("\n")
    }
}

/// How a call that a tool made through a [`ToolRunner`] came out.
#[derive(Debug, Clone, Default)]
pub struct ToolOutcome {
    pub result: ToolResult,
    pub is_error: bool,
    /// A `before_tool_call` hook refused the call, so it never ran.
    pub blocked: bool,
}

/// Runs calls for a tool that calls other tools while it runs, as a codemode script does. The
/// loop's runner puts each call through the same pipeline as the model's own: the tool's
/// argument shim and schema check, `before_tool_call`, and `after_tool_call`.
#[async_trait]
pub trait ToolRunner: Send + Sync {
    /// `tool_call_id` is the nested call's own id. Never fails: an error is an outcome.
    async fn run(&self, tool: Arc<dyn Tool>, tool_call_id: String, args: Value, cancel: CancellationToken) -> ToolOutcome;
}

/// Runs calls with argument checks and nothing else: what a tool gets when something other
/// than the loop executes it, so no hook applies.
pub struct DirectRunner;

#[async_trait]
impl ToolRunner for DirectRunner {
    async fn run(&self, tool: Arc<dyn Tool>, tool_call_id: String, args: Value, cancel: CancellationToken) -> ToolOutcome {
        let args = match crate::agent_loop::checked_arguments(tool.as_ref(), &args) {
            Ok(args) => args,
            Err(message) => return ToolOutcome { result: ToolResult { is_error: true, ..ToolResult::text(message) }, is_error: true, blocked: false },
        };
        match tool.execute_with(&tool_call_id, args, cancel, Arc::new(|_| {}), self).await {
            Ok(result) => ToolOutcome { is_error: result.is_error, result, blocked: false },
            Err(error) => ToolOutcome { result: ToolResult { is_error: true, ..ToolResult::text(error.0) }, is_error: true, blocked: false },
        }
    }
}

#[derive(Debug, thiserror::Error)]
#[error("{0}")]
pub struct ToolError(pub String);

impl From<String> for ToolError {
    fn from(value: String) -> Self {
        ToolError(value)
    }
}

impl From<&str> for ToolError {
    fn from(value: &str) -> Self {
        ToolError(value.to_string())
    }
}

impl From<serde_json::Error> for ToolError {
    fn from(value: serde_json::Error) -> Self {
        ToolError(format!("Invalid arguments: {value}"))
    }
}

/// Callback for streaming partial results while a tool runs.
pub type ToolUpdateFn = Arc<dyn Fn(ToolResult) + Send + Sync>;

/// Something the model can call. Throw (`Err`) on failure instead of encoding errors in
/// content, or return a result with `is_error` when the failure comes with output.
#[async_trait]
pub trait Tool: Send + Sync {
    fn name(&self) -> &str;

    /// Human-readable label for UI display.
    fn label(&self) -> &str {
        self.name()
    }

    fn description(&self) -> &str;

    /// JSON Schema for the arguments object.
    fn parameters(&self) -> Value;

    /// JSON Schema of `structured` in this tool's results, for a tool that always sets it. A
    /// codemode script then receives the structured output instead of the text.
    fn output_schema(&self) -> Option<Value> {
        None
    }

    /// Per-tool execution override. `Sequential` forces the whole batch to run one at a time.
    fn execution_mode(&self) -> Option<ToolExecutionMode> {
        None
    }

    /// A shim over the raw arguments before they are checked against `parameters`, for a tool
    /// that accepts an older or looser shape. Returns the arguments to validate.
    fn prepare_arguments(&self, args: Value) -> Value {
        args
    }

    async fn execute(
        &self,
        tool_call_id: &str,
        args: Value,
        cancel: CancellationToken,
        on_update: ToolUpdateFn,
    ) -> Result<ToolResult, ToolError>;

    /// `execute`, with `tools` for calling other tools through the loop's tool pipeline. The
    /// loop calls this one; the default ignores `tools`. A tool that calls other tools, such
    /// as codemode, overrides it.
    async fn execute_with(
        &self,
        tool_call_id: &str,
        args: Value,
        cancel: CancellationToken,
        on_update: ToolUpdateFn,
        _tools: &dyn ToolRunner,
    ) -> Result<ToolResult, ToolError> {
        self.execute(tool_call_id, args, cancel, on_update).await
    }

    fn spec(&self) -> ToolSpec {
        ToolSpec {
            name: self.name().to_string(),
            description: self.description().to_string(),
            parameters: self.parameters(),
        }
    }
}
