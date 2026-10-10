//! The one interface every coding agent is driven through, whether Lorca runs it on its own
//! (`claude`, `codex`) or a terminal host runs it in a pane (`hosts`): what the bot and the user
//! send it, and the events it reports back to its supervisor (`super::supervise`).

use async_trait::async_trait;
use serde_json::Value;
use tokio::sync::{mpsc, oneshot};

/// What a coding agent reports.
pub(crate) enum Event {
    /// Its session, which resumes it after its process is gone: Claude Code's session id or
    /// Codex's thread id.
    Session(String),
    /// More of its transcript.
    Lines(Vec<String>),
    /// What its pane shows now, for an agent in a terminal host: it stands for the transcript.
    Screen(String),
    /// It took up a message and works on it.
    Working,
    /// It is done with what it was asked, with its last message.
    Idle(Option<String>),
    /// It wants to do something that needs a decision first; the answer goes back on the
    /// channel: `Ok` lets it, `Err` says why not.
    Approve(Approval, oneshot::Sender<Result<(), String>>),
    /// Its pane asks something (an agent in a terminal host): what the pane shows.
    Blocked(String),
    /// Its pane no longer asks: answered there or from its card.
    Unblocked,
    /// It is gone.
    Ended(Ended),
}

/// What a headless agent asks to do.
#[derive(Debug, Clone)]
pub(crate) enum Approval {
    /// Run a shell command.
    Command { command: String },
    /// Change files outside the folder it works in.
    Files { paths: Vec<String> },
    /// Use one of its other tools: a web fetch, an MCP tool.
    Tool { name: String, input: Value },
}

#[derive(Debug, Clone)]
pub(crate) struct Ended {
    pub outcome: String,
    pub failed: bool,
}

/// An answer for an agent whose pane asks: keys to press, or text to type with Return.
#[derive(Debug, Clone, PartialEq)]
pub(crate) enum Answer {
    Keys(Vec<String>),
    Text(String),
}

pub(crate) type Events = mpsc::UnboundedSender<Event>;

#[async_trait]
pub(crate) trait Driver: Send + Sync {
    /// Gives it a message: taken up when it is done with what it does now, or at once with
    /// `interrupt`, which stops that first.
    async fn send(&self, text: &str, interrupt: bool) -> Result<(), String>;
    /// Answers what its pane asks. Only an agent in a terminal host has a pane.
    async fn answer(&self, _answer: &Answer) -> Result<(), String> {
        Err("It asks nothing that takes typed answers".into())
    }
    /// What its pane shows, for an agent in a terminal host: as much as the transcript keeps.
    async fn screen(&self) -> Option<String> {
        None
    }
    /// What its pane shows on screen now, for an agent in a terminal host.
    async fn screen_now(&self) -> Option<String> {
        None
    }
    /// Brings its pane forward on the Runner's screen.
    async fn focus(&self) -> Result<(), String> {
        Err("It runs without a window".into())
    }
    /// Ends it and what it started. Its `Ended` event follows.
    async fn stop(&self);
}
