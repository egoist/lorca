//! `find`: files by glob, respecting .gitignore.

use std::path::PathBuf;

use async_trait::async_trait;
use serde_json::{json, Value};
use tokio_util::sync::CancellationToken;

use super::resolve_to_cwd;
use super::truncate::{format_size, truncate_head, TruncationOptions, DEFAULT_MAX_BYTES};
use crate::tool::{Tool, ToolError, ToolResult, ToolUpdateFn};

const DEFAULT_LIMIT: usize = 1000;

pub struct FindTool {
    cwd: PathBuf,
}

impl FindTool {
    pub fn new(cwd: PathBuf) -> Self {
        FindTool { cwd }
    }
}

#[async_trait]
impl Tool for FindTool {
    fn name(&self) -> &str {
        "find"
    }
    fn description(&self) -> &str {
        "Search for files by glob pattern. Returns matching file paths relative to the search directory. Respects .gitignore. \
         Output is truncated to 1000 results or 50KB (whichever is hit first)."
    }
    fn parameters(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "pattern": { "type": "string", "description": "Glob pattern to match files, e.g. '*.ts', '**/*.json', or 'src/**/*.spec.ts'" },
                "path": { "type": "string", "description": "Directory to search in (default: current directory)" },
                "limit": { "type": "number", "description": "Maximum number of results (default: 1000)" }
            },
            "required": ["pattern"]
        })
    }
    async fn execute(&self, _id: &str, args: Value, cancel: CancellationToken, _on_update: ToolUpdateFn) -> Result<ToolResult, ToolError> {
        let pattern = args["pattern"].as_str().ok_or("pattern is required")?.to_string();
        let search_path = resolve_to_cwd(args["path"].as_str().unwrap_or("."), &self.cwd);
        let limit = args["limit"].as_f64().map(|l| (l as usize).max(1)).unwrap_or(DEFAULT_LIMIT);
        if !search_path.exists() {
            return Err(ToolError(format!("Path not found: {}", search_path.display())));
        }

        // A bare name matches the basename anywhere; a path-containing pattern matches the
        // path relative to the search directory.
        let effective = if pattern.contains('/') || pattern.starts_with("**") {
            pattern.trim_start_matches("./").to_string()
        } else {
            format!("**/{pattern}")
        };
        let matcher = globset::GlobBuilder::new(&effective)
            .literal_separator(true)
            .build()
            .map_err(|e| ToolError(format!("Invalid glob: {e}")))?
            .compile_matcher();

        let root = search_path.clone();
        let results = tokio::task::spawn_blocking(move || -> Result<Vec<String>, ToolError> {
            let mut results = Vec::new();
            let walker = ignore::WalkBuilder::new(&root)
                .hidden(false)
                .git_ignore(true)
                .git_global(true)
                .git_exclude(true)
                .filter_entry(|entry| entry.file_name() != ".git")
                .build();
            for entry in walker.filter_map(Result::ok) {
                if cancel.is_cancelled() {
                    return Err("Operation aborted".into());
                }
                let path = entry.path();
                if path == root {
                    continue;
                }
                let Ok(relative) = path.strip_prefix(&root) else { continue };
                let mut text = relative.to_string_lossy().replace('\\', "/");
                let is_dir = entry.file_type().map(|t| t.is_dir()).unwrap_or(false);
                if !matcher.is_match(&text) {
                    continue;
                }
                if is_dir {
                    text.push('/');
                }
                results.push(text);
                if results.len() >= limit {
                    break;
                }
            }
            results.sort();
            Ok(results)
        })
        .await
        .map_err(|e| ToolError(e.to_string()))??;

        if results.is_empty() {
            return Ok(ToolResult::text("No files found matching pattern").with_details(json!({ "summary": "No files" })));
        }
        let limit_reached = results.len() >= limit;
        let raw = results.join("\n");
        let truncation = truncate_head(&raw, TruncationOptions { max_lines: Some(usize::MAX), max_bytes: None });
        let mut output = truncation.content.clone();
        let mut notices = Vec::new();
        if limit_reached {
            notices.push(format!("{limit} results limit reached. Use limit={} for more, or refine pattern", limit * 2));
        }
        if truncation.truncated {
            notices.push(format!("{} limit reached", format_size(DEFAULT_MAX_BYTES)));
        }
        if !notices.is_empty() {
            output.push_str(&format!("\n\n[{}]", notices.join(". ")));
        }
        Ok(ToolResult::text(output).with_details(json!({ "summary": format!("Found {} file{}", results.len(), if results.len() == 1 { "" } else { "s" }), "result_limit_reached": limit_reached })))
    }
}
