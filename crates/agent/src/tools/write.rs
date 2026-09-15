//! `write`: create or overwrite a file, creating parent directories.

use std::path::PathBuf;

use async_trait::async_trait;
use serde_json::{json, Value};
use tokio_util::sync::CancellationToken;

use super::resolve_to_cwd;
use crate::tool::{Tool, ToolError, ToolResult, ToolUpdateFn};

pub struct WriteTool {
    cwd: PathBuf,
}

impl WriteTool {
    pub fn new(cwd: PathBuf) -> Self {
        WriteTool { cwd }
    }
}

#[async_trait]
impl Tool for WriteTool {
    fn name(&self) -> &str {
        "write"
    }
    fn description(&self) -> &str {
        "Write content to a file. Creates the file if it doesn't exist, overwrites if it does. Automatically creates parent directories."
    }
    fn parameters(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "path": { "type": "string", "description": "Path to the file to write (relative or absolute)" },
                "content": { "type": "string", "description": "Content to write to the file" }
            },
            "required": ["path", "content"]
        })
    }
    async fn execute(&self, _id: &str, args: Value, cancel: CancellationToken, _on_update: ToolUpdateFn) -> Result<ToolResult, ToolError> {
        let path = args["path"].as_str().ok_or("path is required")?.to_string();
        let content = args["content"].as_str().ok_or("content is required")?.to_string();
        if cancel.is_cancelled() {
            return Err("Operation aborted".into());
        }
        let absolute = resolve_to_cwd(&path, &self.cwd);
        if let Some(parent) = absolute.parent() {
            tokio::fs::create_dir_all(parent).await.map_err(|e| ToolError(format!("Cannot create {}: {e}", parent.display())))?;
        }
        tokio::fs::write(&absolute, content.as_bytes()).await.map_err(|e| ToolError(format!("Cannot write {path}: {e}")))?;
        Ok(ToolResult::text(format!("Successfully wrote to {path}")).with_details(json!({ "summary": format!("Wrote {path}"), "bytes": content.len() })))
    }
}
