//! A headless agent's transcript as text, the way Claude Code's own screen words it: what it
//! was sent (`> …`), what it said, each tool it used (`● Bash(git status)`), and the start of
//! what came back (`  ⎿ …`).

use serde_json::Value;

/// How many lines of a tool's result the transcript keeps.
const RESULT_LINES: usize = 4;
/// How long a tool's one-line summary may get.
const CALL_CHARS: usize = 200;

/// A message the agent was sent.
pub(crate) fn sent(text: &str) -> Vec<String> {
    let mut lines: Vec<String> = Vec::new();
    for (index, line) in text.trim().lines().enumerate() {
        lines.push(if index == 0 { format!("> {line}") } else { format!("  {line}") });
    }
    lines
}

/// What the agent said.
pub(crate) fn said(text: &str) -> Vec<String> {
    let text = text.trim();
    if text.is_empty() {
        return Vec::new();
    }
    text.lines().map(|line| line.trim_end().to_string()).collect()
}

/// One tool call: `● Bash(git status --short)`.
pub(crate) fn call(name: &str, detail: &str) -> String {
    let detail: String = detail.split_whitespace().collect::<Vec<_>>().join(" ");
    let short: String = detail.chars().take(CALL_CHARS).collect();
    let more = if detail.chars().count() > CALL_CHARS { "…" } else { "" };
    if short.is_empty() {
        format!("● {name}")
    } else {
        format!("● {name}({short}{more})")
    }
}

/// A path as the transcript shows it: from the folder the agent works in when it is inside it,
/// else from the home folder.
pub(crate) fn path(path: &str, folder: &std::path::Path) -> String {
    let full = std::path::Path::new(path);
    match full.strip_prefix(folder) {
        Ok(rest) if !rest.as_os_str().is_empty() => rest.display().to_string(),
        _ => super::home_relative(full),
    }
}

/// The most telling input of one of Claude Code's tools, its paths from `folder`.
pub(crate) fn claude_detail(name: &str, input: &Value, folder: &std::path::Path) -> String {
    let field = |key: &str| input.get(key).and_then(Value::as_str).map(str::to_string);
    let path = |key: &str| field(key).map(|path| self::path(&path, folder));
    match name {
        "Bash" => field("command"),
        "Read" | "Write" | "Edit" | "MultiEdit" => path("file_path"),
        "NotebookEdit" => path("notebook_path"),
        "Glob" | "Grep" => field("pattern"),
        "WebFetch" => field("url"),
        "WebSearch" => field("query"),
        "Task" | "Agent" => field("description"),
        "TodoWrite" => Some(String::new()),
        _ => None,
    }
    .unwrap_or_else(|| {
        // Another tool, an MCP server's: its first short string argument.
        input.as_object().and_then(|fields| fields.values().find_map(|value| value.as_str().filter(|text| text.len() < 120).map(str::to_string))).unwrap_or_default()
    })
}

/// The start of what a tool gave back.
pub(crate) fn result(text: &str, is_error: bool) -> Vec<String> {
    let all: Vec<&str> = text.lines().map(str::trim_end).filter(|line| !line.trim().is_empty()).collect();
    if all.is_empty() {
        return vec![if is_error { "  ⎿ Error".into() } else { "  ⎿ (no output)".into() }];
    }
    let mut lines = Vec::new();
    for (index, line) in all.iter().take(RESULT_LINES).enumerate() {
        let line: String = line.chars().take(300).collect();
        let line = if index == 0 && is_error { format!("Error: {line}") } else { line };
        lines.push(if index == 0 { format!("  ⎿ {line}") } else { format!("    {line}") });
    }
    if all.len() > RESULT_LINES {
        lines.push(format!("    … +{} lines", all.len() - RESULT_LINES));
    }
    lines
}

/// The text of a tool result's content, which is a string or a list of text blocks.
pub(crate) fn content_text(content: &Value) -> String {
    match content {
        Value::String(text) => text.clone(),
        Value::Array(parts) => parts.iter().filter_map(|part| part.get("text").and_then(Value::as_str)).collect::<Vec<_>>().join("\n"),
        _ => String::new(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn a_transcript_reads_like_the_agents_own_screen() {
        assert_eq!(sent("Fix the login bug\nand add a test"), vec!["> Fix the login bug", "  and add a test"]);
        assert_eq!(call("Bash", "git status\n  --short"), "● Bash(git status --short)");
        assert_eq!(call("TodoWrite", ""), "● TodoWrite");
        let folder = std::path::Path::new("/work/shop");
        assert_eq!(claude_detail("Grep", &json!({ "pattern": "fn main" }), folder), "fn main");
        assert_eq!(claude_detail("Write", &json!({ "file_path": "/work/shop/src/login.rs" }), folder), "src/login.rs");
        assert_eq!(claude_detail("Read", &json!({ "file_path": "/etc/hosts" }), folder), "/etc/hosts");
        assert_eq!(claude_detail("mcp__github__create_issue", &json!({ "title": "Crash", "body": "x".repeat(500) }), folder), "Crash");
        let lines = result("a\nb\n\nc\nd\ne\nf", false);
        assert_eq!(lines, vec!["  ⎿ a", "    b", "    c", "    d", "    … +2 lines"]);
        assert_eq!(result("", true), vec!["  ⎿ Error"]);
        assert_eq!(content_text(&json!([{ "type": "text", "text": "one" }, { "type": "text", "text": "two" }])), "one\ntwo");
    }
}
