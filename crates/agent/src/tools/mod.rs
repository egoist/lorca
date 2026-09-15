//! The built-in tools of pi's coding agent, ported: `read`, `write`, `edit`, `bash`, `grep`,
//! `find`, `ls`. Every tool resolves relative paths against one working directory.

pub mod bash;
pub mod edit;
pub mod find;
pub mod grep;
pub mod ls;
pub mod read;
pub mod truncate;
pub mod write;

use std::path::{Path, PathBuf};
use std::sync::Arc;

use crate::tool::Tool;

pub use bash::BashTool;
pub use edit::EditTool;
pub use find::FindTool;
pub use grep::GrepTool;
pub use ls::LsTool;
pub use read::ReadTool;
pub use write::WriteTool;

/// Resolves a tool path against the working directory. `~` expands to the home directory.
pub fn resolve_to_cwd(path: &str, cwd: &Path) -> PathBuf {
    let trimmed = path.trim();
    let expanded: PathBuf = if trimmed == "~" || trimmed.starts_with("~/") {
        let home = std::env::var_os("HOME").map(PathBuf::from).unwrap_or_else(|| cwd.to_path_buf());
        home.join(trimmed.trim_start_matches('~').trim_start_matches('/'))
    } else {
        PathBuf::from(trimmed)
    };
    if expanded.is_absolute() {
        expanded
    } else {
        cwd.join(expanded)
    }
}

/// All seven tools bound to one working directory.
pub fn coding_tools(cwd: impl Into<PathBuf>) -> Vec<Arc<dyn Tool>> {
    let cwd: PathBuf = cwd.into();
    vec![
        Arc::new(ReadTool::new(cwd.clone())),
        Arc::new(WriteTool::new(cwd.clone())),
        Arc::new(EditTool::new(cwd.clone())),
        Arc::new(BashTool::new(cwd.clone())),
        Arc::new(GrepTool::new(cwd.clone())),
        Arc::new(FindTool::new(cwd.clone())),
        Arc::new(LsTool::new(cwd)),
    ]
}

/// One-line summaries for a system prompt, in pi's words.
pub fn coding_tools_snippet() -> &'static str {
    "read: Read file contents. write: Create or overwrite files. edit: Make precise file edits with exact text \
     replacement, including multiple disjoint edits in one call. bash: Execute bash commands (ls, grep, find, etc.). \
     grep: Search file contents for patterns (respects .gitignore). find: Find files by glob pattern (respects \
     .gitignore). ls: List directory contents."
}

pub fn coding_tools_guidelines() -> &'static [&'static str] {
    &[
        "Use read to examine files instead of cat or sed.",
        "Use write only for new files or complete rewrites.",
        "Use edit for precise changes (edits[].oldText must match exactly).",
        "When changing multiple separate locations in one file, use one edit call with multiple entries in edits[] instead of multiple edit calls.",
        "Each edits[].oldText is matched against the original file, not after earlier edits are applied. Do not emit overlapping or nested edits. Merge nearby changes into one edit.",
        "Keep edits[].oldText as small as possible while still being unique in the file. Do not pad with large unchanged regions.",
    ]
}
