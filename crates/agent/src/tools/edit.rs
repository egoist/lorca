//! `edit`: exact text replacement. Every `oldText` must match one unique, non-overlapping
//! region of the original file; edits are matched against the original, not incrementally.

use std::path::PathBuf;

use async_trait::async_trait;
use serde_json::{json, Value};
use tokio_util::sync::CancellationToken;

use super::resolve_to_cwd;
use crate::tool::{Tool, ToolError, ToolResult, ToolUpdateFn};

pub struct EditTool {
    cwd: PathBuf,
}

impl EditTool {
    pub fn new(cwd: PathBuf) -> Self {
        EditTool { cwd }
    }
}

#[derive(Debug, Clone)]
pub struct Edit {
    pub old_text: String,
    pub new_text: String,
}

/// Accepts `edits: [...]`, `edits` as a JSON string, a single edit object, or legacy top-level
/// `oldText`/`newText`, the way pi's `prepareArguments` does.
fn parse_edits(args: &Value) -> Result<Vec<Edit>, ToolError> {
    fn one(value: &Value) -> Option<Edit> {
        let old = value.get("oldText").or(value.get("old_text"))?.as_str()?;
        let new = value.get("newText").or(value.get("new_text"))?.as_str()?;
        Some(Edit { old_text: old.to_string(), new_text: new.to_string() })
    }
    let mut edits = Vec::new();
    match args.get("edits") {
        Some(Value::Array(items)) => edits.extend(items.iter().filter_map(one)),
        Some(Value::String(raw)) => {
            if let Ok(parsed) = serde_json::from_str::<Value>(raw) {
                match &parsed {
                    Value::Array(items) => edits.extend(items.iter().filter_map(one)),
                    other => edits.extend(one(other)),
                }
            }
        }
        Some(other) => edits.extend(one(other)),
        None => {}
    }
    edits.extend(one(args));
    if edits.is_empty() {
        return Err("Edit tool input is invalid. edits must contain at least one replacement.".into());
    }
    Ok(edits)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum LineEnding {
    Lf,
    CrLf,
}

fn detect_line_ending(text: &str) -> LineEnding {
    let crlf = text.matches("\r\n").count();
    let lf = text.matches('\n').count() - crlf;
    if crlf > lf { LineEnding::CrLf } else { LineEnding::Lf }
}

fn normalize_to_lf(text: &str) -> String {
    text.replace("\r\n", "\n").replace('\r', "\n")
}

/// Applies edits to LF-normalized content. Returns the new content and the first changed line.
pub fn apply_edits(content: &str, edits: &[Edit], path: &str) -> Result<(String, usize), ToolError> {
    let mut spans: Vec<(usize, usize, &Edit)> = Vec::new();
    for edit in edits {
        let old = normalize_to_lf(&edit.old_text);
        if old.is_empty() {
            return Err(ToolError(format!("Could not edit file: {path}. oldText must not be empty.")));
        }
        let occurrences: Vec<usize> = content.match_indices(old.as_str()).map(|(i, _)| i).collect();
        match occurrences.len() {
            0 => {
                return Err(ToolError(format!(
                    "Could not find oldText in {path}. The text must match exactly, including whitespace and indentation.\nSearched for:\n{}",
                    edit.old_text
                )))
            }
            1 => spans.push((occurrences[0], occurrences[0] + old.len(), edit)),
            n => {
                return Err(ToolError(format!(
                    "Found {n} occurrences of oldText in {path}. It must be unique; include more surrounding context.\nSearched for:\n{}",
                    edit.old_text
                )))
            }
        }
    }
    spans.sort_by_key(|(start, _, _)| *start);
    for window in spans.windows(2) {
        if window[1].0 < window[0].1 {
            return Err(ToolError(format!("Overlapping edits in {path}. Merge edits that touch the same region into one.")));
        }
    }

    let mut out = String::with_capacity(content.len());
    let mut cursor = 0;
    let mut first_changed_line = None;
    for (start, end, edit) in &spans {
        out.push_str(&content[cursor..*start]);
        if first_changed_line.is_none() {
            first_changed_line = Some(content[..*start].matches('\n').count() + 1);
        }
        out.push_str(&normalize_to_lf(&edit.new_text));
        cursor = *end;
    }
    out.push_str(&content[cursor..]);
    Ok((out, first_changed_line.unwrap_or(1)))
}

#[async_trait]
impl Tool for EditTool {
    fn name(&self) -> &str {
        "edit"
    }
    fn description(&self) -> &str {
        "Edit a single file using exact text replacement. Every edits[].oldText must match a unique, non-overlapping region of the \
         original file. If two changes affect the same block or nearby lines, merge them into one edit instead of emitting \
         overlapping edits. Do not include large unchanged regions just to connect distant changes."
    }
    fn parameters(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "path": { "type": "string", "description": "Path to the file to edit (relative or absolute)" },
                "edits": {
                    "type": "array",
                    "description": "One or more targeted replacements. Each edit is matched against the original file, not incrementally. Do not include overlapping or nested edits. If two changes touch the same block or nearby lines, merge them into one edit instead.",
                    "items": {
                        "type": "object",
                        "properties": {
                            "oldText": { "type": "string", "description": "Exact text for one targeted replacement. It must be unique in the original file and must not overlap with any other edits[].oldText in the same call." },
                            "newText": { "type": "string", "description": "Replacement text for this targeted edit." }
                        },
                        "required": ["oldText", "newText"]
                    }
                }
            },
            "required": ["path", "edits"]
        })
    }
    async fn execute(&self, _id: &str, args: Value, cancel: CancellationToken, _on_update: ToolUpdateFn) -> Result<ToolResult, ToolError> {
        let path = args["path"].as_str().ok_or("path is required")?.to_string();
        let edits = parse_edits(&args)?;
        let absolute = resolve_to_cwd(&path, &self.cwd);

        let raw = tokio::fs::read(&absolute).await.map_err(|e| ToolError(format!("Could not edit file: {path}. {e}.")))?;
        if cancel.is_cancelled() {
            return Err("Operation aborted".into());
        }
        let raw_text = String::from_utf8_lossy(&raw).into_owned();
        // The model never includes an invisible BOM in oldText.
        let (bom, text) = match raw_text.strip_prefix('\u{feff}') {
            Some(rest) => ("\u{feff}", rest.to_string()),
            None => ("", raw_text.clone()),
        };
        let ending = detect_line_ending(&text);
        let normalized = normalize_to_lf(&text);
        let (new_content, first_changed_line) = apply_edits(&normalized, &edits, &path)?;
        let restored = if ending == LineEnding::CrLf { new_content.replace('\n', "\r\n") } else { new_content };
        tokio::fs::write(&absolute, format!("{bom}{restored}")).await.map_err(|e| ToolError(format!("Could not write {path}: {e}")))?;

        Ok(ToolResult::text(format!("Successfully replaced {} block(s) in {path}.", edits.len())).with_details(json!({
            "summary": format!("Edited {path}"),
            "first_changed_line": first_changed_line,
            "blocks": edits.len(),
        })))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn applies_disjoint_edits_against_original() {
        let content = "alpha\nbeta\ngamma\n";
        let edits = vec![
            Edit { old_text: "gamma".into(), new_text: "GAMMA".into() },
            Edit { old_text: "alpha".into(), new_text: "a".into() },
        ];
        let (out, line) = apply_edits(content, &edits, "f").unwrap();
        assert_eq!(out, "a\nbeta\nGAMMA\n");
        assert_eq!(line, 1);
    }

    #[test]
    fn rejects_ambiguous_and_overlapping() {
        assert!(apply_edits("x x", &[Edit { old_text: "x".into(), new_text: "y".into() }], "f").is_err());
        let overlapping = vec![
            Edit { old_text: "abc".into(), new_text: "1".into() },
            Edit { old_text: "bcd".into(), new_text: "2".into() },
        ];
        assert!(apply_edits("abcd", &overlapping, "f").is_err());
    }
}
