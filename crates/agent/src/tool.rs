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
    /// Hint that the agent should stop after this batch. Only honored when every tool in the
    /// batch sets it.
    #[serde(default)]
    pub terminate: bool,
}

impl ToolResult {
    pub fn text(text: impl Into<String>) -> Self {
        ToolResult { content: vec![ContentPart::text(text)], details: Value::Null, terminate: false }
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

/// Something the model can call. Throw (`Err`) on failure instead of encoding errors in content.
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

    fn spec(&self) -> ToolSpec {
        ToolSpec {
            name: self.name().to_string(),
            description: self.description().to_string(),
            parameters: self.parameters(),
        }
    }
}
