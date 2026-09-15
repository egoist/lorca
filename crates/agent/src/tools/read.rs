//! `read`: file contents with offset/limit and head truncation. Images come back as attachments.

use std::path::PathBuf;

use async_trait::async_trait;
use base64::Engine;
use serde_json::{json, Value};
use tokio_util::sync::CancellationToken;

use super::resolve_to_cwd;
use super::truncate::{format_size, truncate_head, TruncatedBy, TruncationOptions, DEFAULT_MAX_BYTES, DEFAULT_MAX_LINES};
use crate::tool::{Tool, ToolError, ToolResult, ToolUpdateFn};
use crate::types::ContentPart;

pub struct ReadTool {
    cwd: PathBuf,
}

impl ReadTool {
    pub fn new(cwd: PathBuf) -> Self {
        ReadTool { cwd }
    }
}

fn image_mime(bytes: &[u8]) -> Option<&'static str> {
    if bytes.starts_with(&[0xFF, 0xD8, 0xFF]) {
        Some("image/jpeg")
    } else if bytes.starts_with(&[0x89, b'P', b'N', b'G']) {
        Some("image/png")
    } else if bytes.starts_with(b"GIF8") {
        Some("image/gif")
    } else if bytes.len() > 12 && &bytes[..4] == b"RIFF" && &bytes[8..12] == b"WEBP" {
        Some("image/webp")
    } else if bytes.starts_with(b"BM") {
        Some("image/bmp")
    } else {
        None
    }
}

#[async_trait]
impl Tool for ReadTool {
    fn name(&self) -> &str {
        "read"
    }
    fn description(&self) -> &str {
        "Read the contents of a file. Supports text files and images (jpg, png, gif, webp, bmp). Images are sent as attachments. \
         For text files, output is truncated to 2000 lines or 50KB (whichever is hit first). Use offset/limit for large files. \
         When you need the full file, continue with offset until complete."
    }
    fn parameters(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "path": { "type": "string", "description": "Path to the file to read (relative or absolute)" },
                "offset": { "type": "number", "description": "Line number to start reading from (1-indexed)" },
                "limit": { "type": "number", "description": "Maximum number of lines to read" }
            },
            "required": ["path"]
        })
    }
    async fn execute(&self, _id: &str, args: Value, cancel: CancellationToken, _on_update: ToolUpdateFn) -> Result<ToolResult, ToolError> {
        let path = args["path"].as_str().ok_or("path is required")?.to_string();
        let offset = args["offset"].as_f64().map(|n| n as usize);
        let limit = args["limit"].as_f64().map(|n| n as usize);
        let absolute = resolve_to_cwd(&path, &self.cwd);

        let bytes = tokio::fs::read(&absolute).await.map_err(|e| ToolError(format!("Cannot read {path}: {e}")))?;
        if cancel.is_cancelled() {
            return Err("Operation aborted".into());
        }

        if let Some(mime) = image_mime(&bytes) {
            let data = base64::engine::general_purpose::STANDARD.encode(&bytes);
            return Ok(ToolResult {
                content: vec![ContentPart::text(format!("Read image file [{mime}]")), ContentPart::Image { data, mime_type: mime.to_string() }],
                details: Value::Null,
                terminate: false,
            });
        }

        let text = String::from_utf8_lossy(&bytes).into_owned();
        let all_lines: Vec<&str> = text.split('\n').collect();
        let total_file_lines = all_lines.len();
        let start_line = offset.map(|o| o.saturating_sub(1)).unwrap_or(0);
        let start_display = start_line + 1;
        if start_line >= all_lines.len() {
            return Err(ToolError(format!("Offset {} is beyond end of file ({} lines total)", offset.unwrap_or(1), all_lines.len())));
        }

        let (selected, user_limited) = match limit {
            Some(limit) => {
                let end = (start_line + limit).min(all_lines.len());
                (all_lines[start_line..end].join("\n"), Some(end - start_line))
            }
            None => (all_lines[start_line..].join("\n"), None),
        };

        let truncation = truncate_head(&selected, TruncationOptions::default());
        let output = if truncation.first_line_exceeds_limit {
            let first_line_size = format_size(all_lines[start_line].len());
            format!(
                "[Line {start_display} is {first_line_size}, exceeds {} limit. Use bash: sed -n '{start_display}p' {path} | head -c {DEFAULT_MAX_BYTES}]",
                format_size(DEFAULT_MAX_BYTES)
            )
        } else if truncation.truncated {
            let end_display = start_display + truncation.output_lines - 1;
            let next_offset = end_display + 1;
            let mut out = truncation.content.clone();
            if truncation.truncated_by == Some(TruncatedBy::Lines) {
                out.push_str(&format!("\n\n[Showing lines {start_display}-{end_display} of {total_file_lines}. Use offset={next_offset} to continue.]"));
            } else {
                out.push_str(&format!(
                    "\n\n[Showing lines {start_display}-{end_display} of {total_file_lines} ({} limit). Use offset={next_offset} to continue.]",
                    format_size(DEFAULT_MAX_BYTES)
                ));
            }
            out
        } else if let Some(shown) = user_limited.filter(|shown| start_line + shown < all_lines.len()) {
            let remaining = all_lines.len() - (start_line + shown);
            let next_offset = start_line + shown + 1;
            format!("{}\n\n[{remaining} more lines in file. Use offset={next_offset} to continue.]", truncation.content)
        } else {
            truncation.content.clone()
        };

        let details = if truncation.truncated { json!({ "truncation": truncation, "max_lines": DEFAULT_MAX_LINES }) } else { Value::Null };
        Ok(ToolResult::text(output).with_details(details))
    }
}
