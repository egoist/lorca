//! `grep`: regex search over files, respecting .gitignore, with pi's output shape.

use std::path::{Path, PathBuf};

use async_trait::async_trait;
use serde_json::{json, Value};
use tokio_util::sync::CancellationToken;

use super::resolve_to_cwd;
use super::truncate::{format_size, truncate_head, truncate_line, TruncationOptions, DEFAULT_MAX_BYTES, GREP_MAX_LINE_LENGTH};
use crate::tool::{Tool, ToolError, ToolResult, ToolUpdateFn};

const DEFAULT_LIMIT: usize = 100;

pub struct GrepTool {
    cwd: PathBuf,
}

impl GrepTool {
    pub fn new(cwd: PathBuf) -> Self {
        GrepTool { cwd }
    }
}

struct Match {
    path: PathBuf,
    line_number: usize,
}

fn walk(root: &Path, glob: Option<&str>) -> Result<ignore::Walk, ToolError> {
    let mut builder = ignore::WalkBuilder::new(root);
    builder.hidden(false).git_ignore(true).git_global(true).git_exclude(true);
    if let Some(glob) = glob {
        let mut overrides = ignore::overrides::OverrideBuilder::new(root);
        let pattern = if glob.contains('/') { glob.to_string() } else { format!("**/{glob}") };
        overrides.add(&pattern).map_err(|e| ToolError(format!("Invalid glob: {e}")))?;
        builder.overrides(overrides.build().map_err(|e| ToolError(format!("Invalid glob: {e}")))?);
    }
    builder.filter_entry(|entry| entry.file_name() != ".git");
    Ok(builder.build())
}

fn is_probably_binary(bytes: &[u8]) -> bool {
    bytes.iter().take(8192).any(|b| *b == 0)
}

#[async_trait]
impl Tool for GrepTool {
    fn name(&self) -> &str {
        "grep"
    }
    fn description(&self) -> &str {
        "Search file contents for a pattern. Returns matching lines with file paths and line numbers. Respects .gitignore. \
         Output is truncated to 100 matches or 50KB (whichever is hit first). Long lines are truncated to 500 chars."
    }
    fn parameters(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "pattern": { "type": "string", "description": "Search pattern (regex or literal string)" },
                "path": { "type": "string", "description": "Directory or file to search (default: current directory)" },
                "glob": { "type": "string", "description": "Filter files by glob pattern, e.g. '*.ts' or '**/*.spec.ts'" },
                "ignoreCase": { "type": "boolean", "description": "Case-insensitive search (default: false)" },
                "literal": { "type": "boolean", "description": "Treat pattern as literal string instead of regex (default: false)" },
                "context": { "type": "number", "description": "Number of lines to show before and after each match (default: 0)" },
                "limit": { "type": "number", "description": "Maximum number of matches to return (default: 100)" }
            },
            "required": ["pattern"]
        })
    }
    async fn execute(&self, _id: &str, args: Value, cancel: CancellationToken, _on_update: ToolUpdateFn) -> Result<ToolResult, ToolError> {
        let pattern = args["pattern"].as_str().ok_or("pattern is required")?.to_string();
        let search_path = resolve_to_cwd(args["path"].as_str().unwrap_or("."), &self.cwd);
        let glob = args["glob"].as_str().map(str::to_string);
        let ignore_case = args["ignoreCase"].as_bool().or(args["ignore_case"].as_bool()).unwrap_or(false);
        let literal = args["literal"].as_bool().unwrap_or(false);
        let context = args["context"].as_f64().map(|c| c.max(0.0) as usize).unwrap_or(0);
        let limit = args["limit"].as_f64().map(|l| (l as usize).max(1)).unwrap_or(DEFAULT_LIMIT);

        let metadata = tokio::fs::metadata(&search_path).await.map_err(|_| ToolError(format!("Path not found: {}", search_path.display())))?;
        let is_directory = metadata.is_dir();
        let regex = regex::RegexBuilder::new(&if literal { regex::escape(&pattern) } else { pattern.clone() })
            .case_insensitive(ignore_case)
            .build()
            .map_err(|e| ToolError(format!("Invalid pattern: {e}")))?;

        let cwd_root = search_path.clone();
        let result = tokio::task::spawn_blocking(move || -> Result<(Vec<Match>, bool, std::collections::HashMap<PathBuf, Vec<String>>), ToolError> {
            let mut matches = Vec::new();
            let mut limit_reached = false;
            let mut files: std::collections::HashMap<PathBuf, Vec<String>> = std::collections::HashMap::new();
            let entries: Vec<PathBuf> = if is_directory {
                walk(&cwd_root, glob.as_deref())?
                    .filter_map(Result::ok)
                    .filter(|e| e.file_type().map(|t| t.is_file()).unwrap_or(false))
                    .map(|e| e.into_path())
                    .collect()
            } else {
                vec![cwd_root.clone()]
            };
            'files: for file in entries {
                if cancel.is_cancelled() {
                    return Err("Operation aborted".into());
                }
                let Ok(bytes) = std::fs::read(&file) else { continue };
                if is_probably_binary(&bytes) {
                    continue;
                }
                let text = String::from_utf8_lossy(&bytes).replace("\r\n", "\n").replace('\r', "\n");
                let lines: Vec<String> = text.split('\n').map(str::to_string).collect();
                for (index, line) in lines.iter().enumerate() {
                    if regex.is_match(line) {
                        matches.push(Match { path: file.clone(), line_number: index + 1 });
                        if matches.len() >= limit {
                            limit_reached = true;
                            files.insert(file.clone(), lines);
                            break 'files;
                        }
                    }
                }
                if matches.iter().any(|m| m.path == file) {
                    files.insert(file, lines);
                }
            }
            Ok((matches, limit_reached, files))
        })
        .await
        .map_err(|e| ToolError(e.to_string()))??;

        let (matches, limit_reached, files) = result;
        if matches.is_empty() {
            return Ok(ToolResult::text("No matches found").with_details(json!({ "summary": "No matches" })));
        }

        let format_path = |path: &Path| -> String {
            if is_directory {
                if let Ok(relative) = path.strip_prefix(&search_path) {
                    return relative.to_string_lossy().replace('\\', "/");
                }
            }
            path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default()
        };

        let mut out_lines = Vec::new();
        let mut lines_truncated = false;
        for m in &matches {
            let lines = files.get(&m.path).cloned().unwrap_or_default();
            let relative = format_path(&m.path);
            if lines.is_empty() {
                out_lines.push(format!("{relative}:{}: (unable to read file)", m.line_number));
                continue;
            }
            let start = if context > 0 { m.line_number.saturating_sub(context).max(1) } else { m.line_number };
            let end = if context > 0 { (m.line_number + context).min(lines.len()) } else { m.line_number };
            for current in start..=end {
                let text = lines.get(current - 1).map(String::as_str).unwrap_or("");
                let (shown, was_truncated) = truncate_line(text, GREP_MAX_LINE_LENGTH);
                if was_truncated {
                    lines_truncated = true;
                }
                if current == m.line_number {
                    out_lines.push(format!("{relative}:{current}: {shown}"));
                } else {
                    out_lines.push(format!("{relative}-{current}- {shown}"));
                }
            }
        }

        let raw = out_lines.join("\n");
        let truncation = truncate_head(&raw, TruncationOptions { max_lines: Some(usize::MAX), max_bytes: None });
        let mut output = truncation.content.clone();
        let mut notices = Vec::new();
        if limit_reached {
            notices.push(format!("{limit} matches limit reached. Use limit={} for more, or refine pattern", limit * 2));
        }
        if truncation.truncated {
            notices.push(format!("{} limit reached", format_size(DEFAULT_MAX_BYTES)));
        }
        if lines_truncated {
            notices.push(format!("Some lines truncated to {GREP_MAX_LINE_LENGTH} chars. Use read tool to see full lines"));
        }
        if !notices.is_empty() {
            output.push_str(&format!("\n\n[{}]", notices.join(". ")));
        }
        Ok(ToolResult::text(output).with_details(json!({
            "summary": format!("{} match{} for {pattern}", matches.len(), if matches.len() == 1 { "" } else { "es" }),
            "match_limit_reached": limit_reached,
            "lines_truncated": lines_truncated,
        })))
    }
}
