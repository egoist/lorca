//! The one interface every coding agent is driven through, whatever protocol it speaks: what the
//! bot sends it, and the events it reports back to its supervisor (`super::supervise`).

use async_trait::async_trait;
use serde_json::Value;
use tokio::sync::{mpsc, oneshot};

use super::transcript::Item;

/// What a coding agent reports.
pub(crate) enum Event {
    /// Its session, which resumes it after its process is gone: Claude Code's session id, Codex's
    /// thread, OpenCode's session, pi's session file.
    Session(String),
    /// A step of its transcript, new or changed: an item with an id it reported before replaces
    /// that one.
    Item(Item),
    /// It took up a message and works on it.
    Working,
    /// It is done with what it was asked, with its last message.
    Idle(Option<String>),
    /// It wants to do something that needs a decision first; the answer goes back on the
    /// channel: `Ok` lets it, `Err` says why not.
    Approve(Approval, oneshot::Sender<Result<(), String>>),
    /// It is gone.
    Ended(Ended),
}

/// What an agent asks to do.
#[derive(Debug, Clone)]
pub(crate) enum Approval {
    /// Run a shell command.
    Command { command: String },
    /// Change files, which may be outside the folder it works in.
    Files { paths: Vec<String> },
    /// Use one of its other tools: a web fetch, an MCP tool.
    Tool { name: String, input: Value },
}

/// How it ended, in words: "Stopped", "Codex exited with code 1: …".
#[derive(Debug, Clone)]
pub(crate) struct Ended {
    pub outcome: String,
}

pub(crate) type Events = mpsc::UnboundedSender<Event>;

#[async_trait]
pub(crate) trait Driver: Send + Sync {
    /// Gives it a message: taken up when it is done with what it does now, or at once with
    /// `interrupt`, which stops that first.
    async fn send(&self, text: &str, interrupt: bool) -> Result<(), String>;
    /// Ends it and what it started. Its `Ended` event follows.
    async fn stop(&self);
}

/// How a process ended, as the agent's `Ended` event says it.
pub(crate) fn ended(name: &str, exit: Option<super::process::Exit>) -> Ended {
    use super::process::Exit;
    match exit {
        Some(Exit { stopped: true, .. }) => Ended { outcome: "Stopped".into() },
        Some(Exit { code: Some(0), .. }) | None => Ended { outcome: format!("{name} exited") },
        Some(Exit { code, said, .. }) => {
            let code = code.map(|code| format!(" with code {code}")).unwrap_or_default();
            Ended { outcome: format!("{name} exited{code}{}", said.map(|said| format!(": {said}")).unwrap_or_default()) }
        }
    }
}
