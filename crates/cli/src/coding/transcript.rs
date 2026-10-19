//! A coding agent's transcript as steps, whatever protocol it speaks: what it was sent, what it
//! thought and said, each tool it used with what came back, its plan, and what went wrong. The
//! apps draw it the way Cursor draws an agent's run; the bot reads it as text. Every step is kept
//! in `agents/<id>.jsonl` (0600), a step that changed appended again, so the last line with an
//! id is the step.

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use serde_json::Value;

/// How many lines of a tool's output a step keeps.
const OUTPUT_LINES: usize = 12;
/// How long a line of it may get.
const LINE_CHARS: usize = 300;
/// How long a tool's one-line title may get.
const TITLE_CHARS: usize = 200;
/// The steps kept in memory; the file keeps them all.
const KEEP_ITEMS: usize = 4000;

/// One step of an agent's transcript.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
pub struct Item {
    /// Its id in the transcript: a later step with the same id is this one, changed.
    pub id: String,
    /// `prompt` (what it was sent), `status` (Lorca's own line: where it works, an interrupt),
    /// `thought`, `text` (what it said), `tool`, `plan`, or `error`.
    pub kind: String,
    /// When it happened, in Unix seconds.
    pub at: f64,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub text: String,
    /// How long it thought, for a `thought`, when the agent says.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub seconds: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tool: Option<ToolStep>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub plan: Vec<PlanEntry>,
    /// Who wrote a `prompt`: `user` when the user wrote it on the agent's page; none when the
    /// bot passed it on.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub from: Option<String>,
}

/// A tool the agent used.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
pub struct ToolStep {
    /// `read`, `search`, `edit`, `execute`, `fetch`, or `other`: the apps group reads and
    /// searches as "Explored 3 files 2 searches".
    pub kind: String,
    /// What it did, in a line: the command, the file from the folder it works in, the pattern.
    pub title: String,
    /// The file it read or changed, from the folder it works in.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub path: Option<String>,
    /// Lines an edit added and removed.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub added: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub removed: Option<u32>,
    /// `running`, `done`, `failed`, or `denied`.
    pub status: String,
    /// The first lines of what came back.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub output: Option<String>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
pub struct PlanEntry {
    pub text: String,
    /// `pending`, `in_progress`, or `completed`.
    pub status: String,
}

impl Item {
    pub fn new(id: impl Into<String>, kind: &str) -> Item {
        Item { id: id.into(), kind: kind.into(), at: crate::config::now_secs(), ..Item::default() }
    }

    pub fn text(id: impl Into<String>, kind: &str, text: &str) -> Item {
        Item { text: text.trim().to_string(), ..Item::new(id, kind) }
    }

    pub fn tool(id: impl Into<String>, step: ToolStep) -> Item {
        Item { tool: Some(step), ..Item::new(id, "tool") }
    }

    pub fn plan(id: impl Into<String>, plan: Vec<PlanEntry>) -> Item {
        Item { plan, ..Item::new(id, "plan") }
    }

    /// The step as a bot reads it, in the words of Claude Code's own screen: `> ` and what it
    /// was sent, what it said, `● Bash(git status)` and the start of what came back (`  ⎿ …`).
    pub fn lines(&self) -> Vec<String> {
        match self.kind.as_str() {
            "prompt" => {
                // The bot reads which messages the user wrote to the agent themselves.
                let by = if self.from.as_deref() == Some("user") { "(from the user) " } else { "" };
                self.text.lines().enumerate().map(|(index, line)| if index == 0 { format!("> {by}{line}") } else { format!("  {line}") }).collect()
            }
            "text" => self.text.lines().map(|line| line.trim_end().to_string()).collect(),
            "status" => vec![format!("· {}", self.text)],
            "error" => vec![format!("Error: {}", self.text)],
            "plan" => {
                let mut lines = vec!["● Plan".to_string()];
                lines.extend(self.plan.iter().map(|entry| {
                    let mark = match entry.status.as_str() {
                        "completed" => "[x]",
                        "in_progress" => "[~]",
                        _ => "[ ]",
                    };
                    format!("  {mark} {}", entry.text)
                }));
                lines
            }
            "tool" => {
                let Some(step) = &self.tool else { return Vec::new() };
                let mut lines = vec![call_line(step)];
                match step.status.as_str() {
                    "running" => {}
                    "denied" => lines.push(format!("  ⎿ Not allowed{}", step.output.as_deref().map(|why| format!(": {}", first_line(why))).unwrap_or_default())),
                    status => {
                        let output = step.output.as_deref().unwrap_or("");
                        let all: Vec<&str> = output.lines().filter(|line| !line.trim().is_empty()).collect();
                        if all.is_empty() {
                            lines.push(if status == "failed" { "  ⎿ Error".into() } else { "  ⎿ (no output)".into() });
                        }
                        for (index, line) in all.iter().take(4).enumerate() {
                            let line = if index == 0 && status == "failed" { format!("Error: {line}") } else { line.to_string() };
                            lines.push(if index == 0 { format!("  ⎿ {line}") } else { format!("    {line}") });
                        }
                        if all.len() > 4 {
                            lines.push(format!("    … +{} lines", all.len() - 4));
                        }
                    }
                }
                lines
            }
            _ => Vec::new(),
        }
    }

    /// The step with the bytes the apps need, for `coding.transcript`, which goes through the
    /// relay to another Device.
    fn for_app(&self) -> Item {
        let mut item = self.clone();
        if item.text.chars().count() > 8000 {
            item.text = format!("{}…", item.text.chars().take(8000).collect::<String>());
        }
        item
    }
}

/// `● Bash(git status --short)`.
fn call_line(step: &ToolStep) -> String {
    let name = match step.kind.as_str() {
        "read" => "Read",
        "search" => "Search",
        "edit" => "Edit",
        "execute" => "Bash",
        "fetch" => "Fetch",
        _ => "Tool",
    };
    let stats = match (step.added, step.removed) {
        (Some(added), Some(removed)) => format!(" +{added} -{removed}"),
        (Some(added), None) => format!(" +{added}"),
        _ => String::new(),
    };
    if step.title.is_empty() { format!("● {name}{stats}") } else { format!("● {name}({}){stats}", step.title) }
}

fn first_line(text: &str) -> &str {
    text.lines().map(str::trim).find(|line| !line.is_empty()).unwrap_or("")
}

/// A one-line title from free text: its whitespace folded, at most `TITLE_CHARS` long.
pub fn title(text: &str) -> String {
    let text: String = text.split_whitespace().collect::<Vec<_>>().join(" ");
    if text.chars().count() > TITLE_CHARS { format!("{}…", text.chars().take(TITLE_CHARS).collect::<String>()) } else { text }
}

/// The first lines of what a tool gave back, each at most `LINE_CHARS` long, with how many more
/// there were.
pub fn output(text: &str) -> Option<String> {
    let all: Vec<&str> = text.lines().map(str::trim_end).filter(|line| !line.trim().is_empty()).collect();
    if all.is_empty() {
        return None;
    }
    let mut lines: Vec<String> = all.iter().take(OUTPUT_LINES).map(|line| line.chars().take(LINE_CHARS).collect()).collect();
    if all.len() > OUTPUT_LINES {
        lines.push(format!("… +{} lines", all.len() - OUTPUT_LINES));
    }
    Some(lines.join("\n"))
}

/// A path as the transcript shows it: from the folder the agent works in when it is inside it,
/// else from the home folder.
pub fn path(path: &str, folder: &Path) -> String {
    let full = Path::new(path);
    let full = if full.is_absolute() { full.to_path_buf() } else { folder.join(full) };
    // An agent may name the folder by its real path, `/private/var/…` for `/var/…` on macOS.
    let real = std::fs::canonicalize(folder).ok();
    let rest = full.strip_prefix(folder).ok().or_else(|| real.as_deref().and_then(|real| full.strip_prefix(real).ok()));
    match rest {
        Some(rest) if !rest.as_os_str().is_empty() => rest.display().to_string(),
        _ => super::home_relative(&full),
    }
}

/// Lines `text` holds, for what an edit added or removed.
pub fn line_count(text: &str) -> u32 {
    if text.is_empty() { 0 } else { text.lines().count() as u32 }
}

/// Lines a unified diff adds and removes.
pub fn diff_stats(diff: &str) -> (u32, u32) {
    let mut stats = (0, 0);
    for line in diff.lines() {
        if line.starts_with("+++") || line.starts_with("---") {
            continue;
        }
        if line.starts_with('+') {
            stats.0 += 1;
        } else if line.starts_with('-') {
            stats.1 += 1;
        }
    }
    stats
}

/// The text of a tool result's content, which is a string or a list of text blocks.
pub fn content_text(content: &Value) -> String {
    match content {
        Value::String(text) => text.clone(),
        Value::Array(parts) => parts.iter().filter_map(|part| part.get("text").and_then(Value::as_str).or_else(|| part["content"]["text"].as_str())).collect::<Vec<_>>().join("\n"),
        _ => String::new(),
    }
}

/// The steps of one agent's transcript: kept in order of first appearance, each with the
/// revision it last changed at, so the bot reads what changed since it last read.
#[derive(Default)]
pub struct Transcript {
    items: Vec<(u64, Item)>,
    index: HashMap<String, usize>,
    /// How many steps were dropped from memory before the ones kept.
    dropped: usize,
    revision: u64,
    /// The revision the bot has read up to.
    pub read: u64,
}

impl Transcript {
    /// Puts `item` in, replacing the step with its id. True when it changed anything.
    pub fn upsert(&mut self, item: Item) -> bool {
        self.revision += 1;
        if let Some(&position) = self.index.get(&item.id) {
            let slot = &mut self.items[position];
            // A step keeps the time it began.
            let item = Item { at: slot.1.at, ..item };
            if slot.1 == item {
                return false;
            }
            *slot = (self.revision, item);
            return true;
        }
        self.index.insert(item.id.clone(), self.items.len());
        self.items.push((self.revision, item));
        if self.items.len() > KEEP_ITEMS {
            let cut = self.items.len() - KEEP_ITEMS;
            self.items.drain(..cut);
            self.dropped += cut;
            self.index = self.items.iter().enumerate().map(|(position, (_, item))| (item.id.clone(), position)).collect();
        }
        true
    }

    pub fn get(&self, id: &str) -> Option<&Item> {
        self.index.get(id).map(|&position| &self.items[position].1)
    }

    pub fn items(&self) -> impl Iterator<Item = &Item> {
        self.items.iter().map(|(_, item)| item)
    }

    /// The last `count` steps, for the apps.
    pub fn tail(&self, count: usize) -> Vec<Item> {
        let start = self.items.len().saturating_sub(count);
        self.items[start..].iter().map(|(_, item)| item.for_app()).collect()
    }

    /// What the bot has not read, up to `limit` lines of its end, or its last `lines`; and how
    /// many lines were left out.
    pub fn read(&mut self, lines: Option<usize>, limit: usize) -> (String, usize) {
        let all: Vec<String> = match lines {
            Some(_) => self.items.iter().flat_map(|(_, item)| item.lines()).collect(),
            None => self.items.iter().filter(|(revision, _)| *revision > self.read).flat_map(|(_, item)| item.lines()).collect(),
        };
        let keep = lines.unwrap_or(limit);
        let skipped = if lines.is_none() { all.len().saturating_sub(keep) } else { 0 };
        let text = all[all.len().saturating_sub(keep)..].join("\n");
        self.read = self.revision;
        (text, skipped)
    }

    /// The last `count` lines, as the bot reads them.
    pub fn last_lines(&self, count: usize) -> String {
        let all: Vec<String> = self.items.iter().flat_map(|(_, item)| item.lines()).collect();
        all[all.len().saturating_sub(count)..].join("\n")
    }

    /// The last thing it said.
    pub fn last_said(&self) -> Option<String> {
        self.items.iter().rev().map(|(_, item)| item).find(|item| item.kind == "text" && !item.text.is_empty()).map(|item| item.text.clone())
    }

    /// The transcript `agents/<id>.jsonl` holds.
    pub fn load(path: &Path) -> Transcript {
        let mut transcript = Transcript::default();
        let Ok(text) = std::fs::read_to_string(path) else { return transcript };
        for line in text.lines() {
            if let Ok(item) = serde_json::from_str::<Item>(line) {
                transcript.upsert(item);
            }
        }
        transcript.read = transcript.revision;
        transcript
    }
}

pub fn file(home: &Path, id: &str) -> PathBuf {
    home.join("agents").join(format!("{id}.jsonl"))
}

/// Appends `item` to the agent's file.
pub fn append(home: &Path, id: &str, item: &Item) {
    use std::io::Write;
    let path = file(home, id);
    let _ = std::fs::create_dir_all(path.parent().unwrap_or(home));
    let mut options = std::fs::OpenOptions::new();
    options.create(true).append(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    let Ok(line) = serde_json::to_string(item) else { return };
    if let Ok(mut file) = options.open(&path) {
        let _ = file.write_all(format!("{line}\n").as_bytes());
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn step(kind: &str, title: &str, status: &str, output: Option<&str>) -> ToolStep {
        ToolStep { kind: kind.into(), title: title.into(), status: status.into(), output: output.map(str::to_string), ..ToolStep::default() }
    }

    #[test]
    fn a_transcript_reads_like_the_agents_own_screen() {
        assert_eq!(Item::text("p", "prompt", "Fix the login bug\nand add a test").lines(), vec!["> Fix the login bug", "  and add a test"]);
        let user = Item { from: Some("user".into()), ..Item::text("p", "prompt", "Use the staging key") };
        assert_eq!(user.lines(), vec!["> (from the user) Use the staging key"]);
        assert_eq!(Item::tool("t", step("execute", "git status --short", "running", None)).lines(), vec!["● Bash(git status --short)"]);
        let done = Item::tool("t", step("execute", "ls", "done", Some("a\nb\n\nc\nd\ne\nf")));
        assert_eq!(done.lines(), vec!["● Bash(ls)", "  ⎿ a", "    b", "    c", "    d", "    … +2 lines"]);
        assert_eq!(Item::tool("t", step("execute", "rm -rf /", "denied", Some("Too risky"))).lines(), vec!["● Bash(rm -rf /)", "  ⎿ Not allowed: Too risky"]);
        let edit = Item::tool("e", ToolStep { added: Some(3), removed: Some(1), ..step("edit", "src/login.rs", "done", Some("ok")) });
        assert_eq!(edit.lines()[0], "● Edit(src/login.rs) +3 -1");
        let plan = Item::plan("plan", vec![PlanEntry { text: "Read it".into(), status: "completed".into() }, PlanEntry { text: "Fix it".into(), status: "pending".into() }]);
        assert_eq!(plan.lines(), vec!["● Plan", "  [x] Read it", "  [ ] Fix it"]);
        let folder = Path::new("/work/shop");
        assert_eq!(path("/work/shop/src/login.rs", folder), "src/login.rs");
        assert_eq!(path("src/login.rs", folder), "src/login.rs");
        assert_eq!(diff_stats("--- a/x\n+++ b/x\n@@\n-old\n+new\n+more\n"), (2, 1));
        assert_eq!(output(&"line\n".repeat(20)).unwrap().lines().last(), Some("… +8 lines"));
    }

    #[test]
    fn the_bot_reads_what_changed_since_it_last_read() {
        let mut transcript = Transcript::default();
        transcript.upsert(Item::text("p", "prompt", "Fix it"));
        transcript.upsert(Item::tool("t", step("execute", "cargo test", "running", None)));
        let (first, skipped) = transcript.read(None, 200);
        assert_eq!((first.as_str(), skipped), ("> Fix it\n● Bash(cargo test)", 0));
        // The command finishing changes its step: the bot reads the step again, with its output.
        transcript.upsert(Item::tool("t", step("execute", "cargo test", "done", Some("ok"))));
        transcript.upsert(Item::text("m", "text", "Fixed."));
        assert_eq!(transcript.read(None, 200).0, "● Bash(cargo test)\n  ⎿ ok\nFixed.");
        assert_eq!(transcript.read(None, 200).0, "");
        assert_eq!(transcript.read(Some(1), 200).0, "Fixed.");
        assert_eq!(transcript.last_said().as_deref(), Some("Fixed."));
        // The same step again changes nothing.
        let same = transcript.get("m").unwrap().clone();
        assert!(!transcript.upsert(same));
    }

    #[test]
    fn the_file_keeps_each_steps_last_version() {
        let home = std::env::temp_dir().join(format!("lorca-transcript-{}", uuid::Uuid::new_v4()));
        append(&home, "agent-1", &Item::tool("t", step("execute", "ls", "running", None)));
        append(&home, "agent-1", &Item::text("m", "text", "Hi"));
        append(&home, "agent-1", &Item::tool("t", step("execute", "ls", "done", Some("a"))));
        let transcript = Transcript::load(&file(&home, "agent-1"));
        let items: Vec<&Item> = transcript.items().collect();
        assert_eq!(items.len(), 2);
        assert_eq!(items[0].tool.as_ref().unwrap().status, "done");
        assert_eq!(transcript.read, transcript.revision, "a restart has read it all");
        let _ = std::fs::remove_dir_all(&home);
    }
}
