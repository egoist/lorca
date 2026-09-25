//! The built-in tools of pi's coding agent, ported: `read`, `write`, `edit`, `bash`, `grep`,
//! `find`, `ls`. Every tool resolves relative paths against one working directory. A host that
//! keeps terminal sessions also gets `bash_input` and `bash_output` ([`bash_session`]).

pub mod bash;
pub mod bash_session;
pub mod edit;
pub mod find;
pub mod grep;
pub mod ls;
pub mod read;
pub mod sanitize;
pub mod truncate;
pub mod write;

use std::path::{Path, PathBuf};
use std::sync::Arc;

use crate::tool::Tool;

pub use bash::BashTool;
pub use bash_session::{BashInputTool, BashOutputTool, BashSession, BashSessions, SessionEnd};
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
    with_bash(BashTool::new(cwd.clone()), cwd)
}

/// The seven tools, with `bash` running each command in a terminal session `sessions` keeps,
/// plus `bash_input` and `bash_output` to reach a command that is still running. On Windows
/// `bash` runs on pipes and the two are left out.
pub fn coding_tools_with_sessions(cwd: impl Into<PathBuf>, sessions: Arc<dyn BashSessions>) -> Vec<Arc<dyn Tool>> {
    let cwd: PathBuf = cwd.into();
    let mut tools = with_bash(BashTool::with_sessions(cwd.clone(), sessions.clone()), cwd);
    if cfg!(unix) {
        tools.push(Arc::new(BashInputTool::new(sessions.clone())));
        tools.push(Arc::new(BashOutputTool::new(sessions)));
    }
    tools
}

fn with_bash(bash: BashTool, cwd: PathBuf) -> Vec<Arc<dyn Tool>> {
    vec![
        Arc::new(ReadTool::new(cwd.clone())),
        Arc::new(WriteTool::new(cwd.clone())),
        Arc::new(EditTool::new(cwd.clone())),
        Arc::new(bash),
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

/// The one-line summaries of `bash_input` and `bash_output`, beside [`coding_tools_snippet`].
pub fn session_tools_snippet() -> &'static str {
    "bash_input: Type into a command bash left running (a password prompt, a [Y/n]). bash_output: Read more from a command \
     bash left running."
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
