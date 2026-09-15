//! Shared truncation for tool outputs, after pi's `truncate.ts`.
//!
//! Two independent limits, whichever is hit first wins: a line limit (2000) and a byte limit
//! (50KB). Never returns partial lines, except the bash tail edge case.

use serde::{Deserialize, Serialize};

pub const DEFAULT_MAX_LINES: usize = 2000;
pub const DEFAULT_MAX_BYTES: usize = 50 * 1024;
pub const GREP_MAX_LINE_LENGTH: usize = 500;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum TruncatedBy {
    Lines,
    Bytes,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TruncationResult {
    pub content: String,
    pub truncated: bool,
    pub truncated_by: Option<TruncatedBy>,
    pub total_lines: usize,
    pub total_bytes: usize,
    pub output_lines: usize,
    pub output_bytes: usize,
    pub last_line_partial: bool,
    pub first_line_exceeds_limit: bool,
    pub max_lines: usize,
    pub max_bytes: usize,
}

#[derive(Debug, Clone, Copy, Default)]
pub struct TruncationOptions {
    pub max_lines: Option<usize>,
    pub max_bytes: Option<usize>,
}

fn split_lines_for_counting(content: &str) -> Vec<&str> {
    if content.is_empty() {
        return Vec::new();
    }
    let mut lines: Vec<&str> = content.split('\n').collect();
    if content.ends_with('\n') {
        lines.pop();
    }
    lines
}

pub fn format_size(bytes: usize) -> String {
    if bytes < 1024 {
        format!("{bytes}B")
    } else if bytes < 1024 * 1024 {
        format!("{:.1}KB", bytes as f64 / 1024.0)
    } else {
        format!("{:.1}MB", bytes as f64 / (1024.0 * 1024.0))
    }
}

fn untruncated(content: &str, total_lines: usize, total_bytes: usize, max_lines: usize, max_bytes: usize) -> TruncationResult {
    TruncationResult {
        content: content.to_string(),
        truncated: false,
        truncated_by: None,
        total_lines,
        total_bytes,
        output_lines: total_lines,
        output_bytes: total_bytes,
        last_line_partial: false,
        first_line_exceeds_limit: false,
        max_lines,
        max_bytes,
    }
}

/// Keep the first lines that fit. If the first line alone exceeds the byte limit the content
/// is empty and `first_line_exceeds_limit` is set.
pub fn truncate_head(content: &str, options: TruncationOptions) -> TruncationResult {
    let max_lines = options.max_lines.unwrap_or(DEFAULT_MAX_LINES);
    let max_bytes = options.max_bytes.unwrap_or(DEFAULT_MAX_BYTES);
    let total_bytes = content.len();
    let lines = split_lines_for_counting(content);
    let total_lines = lines.len();

    if total_lines <= max_lines && total_bytes <= max_bytes {
        return untruncated(content, total_lines, total_bytes, max_lines, max_bytes);
    }

    if lines.first().map(|l| l.len()).unwrap_or(0) > max_bytes {
        return TruncationResult {
            content: String::new(),
            truncated: true,
            truncated_by: Some(TruncatedBy::Bytes),
            total_lines,
            total_bytes,
            output_lines: 0,
            output_bytes: 0,
            last_line_partial: false,
            first_line_exceeds_limit: true,
            max_lines,
            max_bytes,
        };
    }

    let mut out: Vec<&str> = Vec::new();
    let mut out_bytes = 0usize;
    let mut truncated_by = TruncatedBy::Lines;
    for (i, line) in lines.iter().enumerate() {
        if i >= max_lines {
            break;
        }
        let line_bytes = line.len() + if i > 0 { 1 } else { 0 };
        if out_bytes + line_bytes > max_bytes {
            truncated_by = TruncatedBy::Bytes;
            break;
        }
        out.push(line);
        out_bytes += line_bytes;
    }
    if out.len() >= max_lines && out_bytes <= max_bytes {
        truncated_by = TruncatedBy::Lines;
    }
    let output = out.join("\n");
    TruncationResult {
        output_bytes: output.len(),
        output_lines: out.len(),
        content: output,
        truncated: true,
        truncated_by: Some(truncated_by),
        total_lines,
        total_bytes,
        last_line_partial: false,
        first_line_exceeds_limit: false,
        max_lines,
        max_bytes,
    }
}

/// Keep the last lines that fit. May return a partial line when the last line alone exceeds
/// the byte limit.
pub fn truncate_tail(content: &str, options: TruncationOptions) -> TruncationResult {
    let max_lines = options.max_lines.unwrap_or(DEFAULT_MAX_LINES);
    let max_bytes = options.max_bytes.unwrap_or(DEFAULT_MAX_BYTES);
    let total_bytes = content.len();
    let lines = split_lines_for_counting(content);
    let total_lines = lines.len();

    if total_lines <= max_lines && total_bytes <= max_bytes {
        return untruncated(content, total_lines, total_bytes, max_lines, max_bytes);
    }

    let mut out: std::collections::VecDeque<String> = std::collections::VecDeque::new();
    let mut out_bytes = 0usize;
    let mut truncated_by = TruncatedBy::Lines;
    let mut last_line_partial = false;
    for line in lines.iter().rev() {
        if out.len() >= max_lines {
            break;
        }
        let line_bytes = line.len() + if !out.is_empty() { 1 } else { 0 };
        if out_bytes + line_bytes > max_bytes {
            truncated_by = TruncatedBy::Bytes;
            if out.is_empty() {
                let partial = truncate_string_to_bytes_from_end(line, max_bytes);
                out_bytes = partial.len();
                out.push_front(partial);
                last_line_partial = true;
            }
            break;
        }
        out.push_front((*line).to_string());
        out_bytes += line_bytes;
    }
    if out.len() >= max_lines && out_bytes <= max_bytes {
        truncated_by = TruncatedBy::Lines;
    }
    let output = out.iter().map(String::as_str).collect::<Vec<_>>().join("\n");
    TruncationResult {
        output_bytes: output.len(),
        output_lines: out.len(),
        content: output,
        truncated: true,
        truncated_by: Some(truncated_by),
        total_lines,
        total_bytes,
        last_line_partial,
        first_line_exceeds_limit: false,
        max_lines,
        max_bytes,
    }
}

fn truncate_string_to_bytes_from_end(text: &str, max_bytes: usize) -> String {
    if text.len() <= max_bytes {
        return text.to_string();
    }
    let mut start = text.len() - max_bytes;
    while start < text.len() && !text.is_char_boundary(start) {
        start += 1;
    }
    text[start..].to_string()
}

/// Truncate one line to `max_chars`, marking it. Used for grep match lines.
pub fn truncate_line(line: &str, max_chars: usize) -> (String, bool) {
    if line.chars().count() <= max_chars {
        return (line.to_string(), false);
    }
    (format!("{}... [truncated]", line.chars().take(max_chars).collect::<String>()), true)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn head_keeps_whole_lines() {
        let content = "a\nbb\nccc\n";
        let result = truncate_head(content, TruncationOptions { max_lines: None, max_bytes: Some(4) });
        assert!(result.truncated);
        assert_eq!(result.content, "a\nbb");
        assert_eq!(result.total_lines, 3);
        assert_eq!(result.truncated_by, Some(TruncatedBy::Bytes));
    }

    #[test]
    fn tail_keeps_last_lines() {
        let content = (1..=10).map(|i| i.to_string()).collect::<Vec<_>>().join("\n");
        let result = truncate_tail(&content, TruncationOptions { max_lines: Some(3), max_bytes: None });
        assert_eq!(result.content, "8\n9\n10");
        assert_eq!(result.truncated_by, Some(TruncatedBy::Lines));
    }

    #[test]
    fn tail_partial_line() {
        let content = "x".repeat(100);
        let result = truncate_tail(&content, TruncationOptions { max_lines: None, max_bytes: Some(10) });
        assert!(result.last_line_partial);
        assert_eq!(result.content.len(), 10);
    }
}
