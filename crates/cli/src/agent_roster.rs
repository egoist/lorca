//! Every coding agent on the account, whichever bot, chat, and Runner started it, read from the
//! cards the chats carry (`Body::Tool.agent`), which reach every Device with their chat. A bot
//! lists them with `coding_agent list` and `scope: "account"`, `list_teammates` names each bot's
//! live ones, a start hears which agents already work where it starts, and the apps ask
//! `coding.list`.

use serde::Serialize;
use serde_json::{json, Value};

use crate::app::App;
use crate::model::{coding_agent_name, AgentRun, Author, Body};

/// An agent is listed while it is open, and this long after it last changed.
const LISTED_FOR: f64 = 30.0 * 24.0 * 3600.0;
/// An idle agent that heard nothing this long is no longer live, as the apps' Coding sections
/// judge it.
const STALE_IDLE: f64 = 3.0 * 24.0 * 3600.0;
/// A start hears of an agent that failed where it starts this long after it ended: one that
/// failed is often started again on its session.
const RECENT_FAILURE: f64 = 24.0 * 3600.0;

/// A coding agent on the account, with the bot and the chat it belongs to.
#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct Listed {
    pub chat_id: String,
    pub chat_title: String,
    /// The `coding_agent` row that started it, which carries its card.
    pub message_id: String,
    pub bot_id: String,
    pub bot_name: String,
    pub agent: AgentRun,
}

impl Listed {
    /// Live: it has not ended, and has not sat idle for days.
    pub fn is_live(&self, now: f64) -> bool {
        self.agent.is_open() && !(self.agent.state == "idle" && now - changed_at(&self.agent) > STALE_IDLE)
    }

    /// "- agent-3f9a2c1d Claude Code “Login”: working, for Scout (bot-…) in “Shop” (chat-…) on
    /// Workbench, in ~/dev/shop on fix-login, https://github.com/…/pull/7", as the bot reads it;
    /// the bot's own agents in its chat say so.
    pub fn line(&self, bot_id: &str, chat_id: &str) -> String {
        let agent = &self.agent;
        let owner = match (self.bot_id == bot_id, self.chat_id == chat_id) {
            (true, true) => "yours, in this chat".to_string(),
            (true, false) => format!("yours, in “{}” ({})", self.chat_title, self.chat_id),
            (false, true) => format!("for {} ({}) in this chat", self.bot_name, self.bot_id),
            (false, false) => format!("for {} ({}) in “{}” ({})", self.bot_name, self.bot_id, self.chat_title, self.chat_id),
        };
        let runner = agent.device.as_ref().map(|name| format!(" on {name}")).unwrap_or_default();
        let place = match &agent.branch {
            Some(branch) => format!("{} on {branch}", agent.folder),
            None => agent.folder.clone(),
        };
        let pull_request = agent.pull_request.as_ref().map(|link| format!(", {link}")).unwrap_or_default();
        format!("- {} {} “{}”: {}, {owner}{runner}, in {place}{pull_request}", agent.id, coding_agent_name(&agent.kind), agent.title, state_words(agent))
    }
}

/// When the agent last changed state, or started.
fn changed_at(agent: &AgentRun) -> f64 {
    agent.updated_at.or(agent.started_at).unwrap_or(0.0)
}

/// How an agent stands, in a few words for a bot.
pub fn state_words(agent: &AgentRun) -> String {
    match agent.state.as_str() {
        "starting" => "starting".into(),
        "working" => "working".into(),
        "asking" => "waiting on the user".into(),
        "idle" => "done".into(),
        _ => format!("ended ({})", agent.outcome.as_deref().unwrap_or(&agent.state)),
    }
}

/// Every coding agent this Device's chats show, open ones first, then the rest, each newest
/// first: those open, and those that changed in the last 30 days.
pub fn account(app: &App) -> Vec<Listed> {
    let rows = app.store.agent_rows().map_err(|error| tracing::error!(%error, "reading the coding agents' rows")).unwrap_or_default();
    let (chats, bots) = {
        let state = app.state.lock().unwrap();
        (state.chats.iter().map(|chat| chat.meta.clone()).collect::<Vec<_>>(), state.bots.clone())
    };
    let now = crate::config::now_secs();
    let mut listed: Vec<Listed> = rows
        .into_iter()
        .filter_map(|message| {
            let (Body::Tool { agent: Some(agent), .. }, Author::Bot { bot_id }) = (message.body, message.author) else { return None };
            // A start that has not started its agent yet, or never did.
            if agent.id.is_empty() || (!agent.is_open() && now - changed_at(&agent) > LISTED_FOR) {
                return None;
            }
            let chat = chats.iter().find(|chat| chat.id == message.chat_id)?;
            let bot_name = bots.iter().find(|bot| bot.id == bot_id).map(|bot| bot.name.clone()).unwrap_or_else(|| bot_id.clone());
            Some(Listed { chat_id: message.chat_id, chat_title: app.chat_title(chat), message_id: message.id, bot_id, bot_name, agent })
        })
        .collect();
    listed.sort_by(|a, b| b.agent.is_open().cmp(&a.agent.is_open()).then(changed_at(&b.agent).total_cmp(&changed_at(&a.agent))));
    listed
}

/// Where an agent is about to start: a folder on a Runner, and the repository it is a clone of.
pub struct Place<'a> {
    pub device_id: Option<&'a str>,
    /// From the home folder, as the cards say it: `~/dev/shop`.
    pub folder: &'a str,
    pub repository: Option<&'a str>,
}

/// The agents already at work where `place` is: live ones in the same folder on the same
/// Runner, one folder inside the other, or in a clone of the same repository on any Runner;
/// and ones that failed there in the last day, live first. `except` is the agent starting.
pub fn working_at(app: &App, place: &Place, except: Option<&str>) -> Vec<Listed> {
    let now = crate::config::now_secs();
    account(app)
        .into_iter()
        .filter(|listed| Some(listed.agent.id.as_str()) != except)
        .filter(|listed| listed.is_live(now) || (listed.agent.state == "failed" && now - changed_at(&listed.agent) <= RECENT_FAILURE))
        .filter(|listed| {
            let agent = &listed.agent;
            let same_folder = agent.device_id.is_some() && agent.device_id.as_deref() == place.device_id && nested(&agent.folder, place.folder);
            let same_repository = place.repository.is_some() && agent.repository.as_deref() == place.repository;
            same_folder || same_repository
        })
        .collect()
}

/// One folder is the other, or inside it.
fn nested(a: &str, b: &str) -> bool {
    let inside = |inner: &str, outer: &str| inner.strip_prefix(outer).is_some_and(|rest| rest.starts_with('/') || rest.starts_with('\\'));
    a == b || inside(a, b) || inside(b, a)
}

/// What a start tells the bot about agents already at work where it starts, if any: who they
/// belong to and what they work on. A warning only: two agents on one repository is sometimes
/// what the user wants.
pub fn warning(others: &[Listed], bot_id: &str, chat_id: &str) -> Option<String> {
    if others.is_empty() {
        return None;
    }
    let rows: Vec<String> = others.iter().take(8).map(|listed| listed.line(bot_id, chat_id)).collect();
    let more = if others.len() > rows.len() { format!("\n…and {} more.", others.len() - rows.len()) } else { String::new() };
    Some(format!(
        "Other coding agents already work in this folder or repository:\n{}{more}\nTell the user who else is working there and on \
         what, in a line. If it is the same job, say so: they decide whether to stop one. An agent of yours that failed takes a \
         message with coding_agent send, which starts it again on its session.",
        rows.join("\n")
    ))
}

/// `coding.list`: every coding agent this Device's chats show, with its bot and chat.
pub fn serve(app: &App) -> Value {
    json!({ "agents": account(app) })
}

/// A git remote as the repository it names: `git@github.com:egoist/lorca.git`,
/// `https://github.com/egoist/lorca`, and `ssh://git@github.com/egoist/lorca.git` are all
/// "github.com/egoist/lorca".
pub fn repository_of_remote(remote: &str) -> Option<String> {
    let remote = remote.trim();
    let rest = match remote.split_once("://") {
        Some((_, rest)) => rest.to_string(),
        // scp-like: user@host:path
        None => match remote.split_once(':') {
            Some((host, path)) if !host.contains('/') && !path.starts_with('/') => format!("{host}/{path}"),
            _ => return None,
        },
    };
    let rest = rest.rsplit_once('@').map(|(_, rest)| rest.to_string()).unwrap_or(rest);
    let (host, path) = rest.split_once('/')?;
    let host = host.split(':').next().unwrap_or(host).to_lowercase();
    let path = path.trim_matches('/');
    let path = path.strip_suffix(".git").unwrap_or(path);
    if host.is_empty() || path.is_empty() {
        return None;
    }
    Some(format!("{host}/{}", path.to_lowercase()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn remotes_name_their_repository_however_they_are_spelled() {
        for remote in ["git@github.com:egoist/lorca.git", "https://github.com/egoist/lorca", "https://github.com/Egoist/Lorca.git/", "ssh://git@github.com:22/egoist/lorca.git"] {
            assert_eq!(repository_of_remote(remote).as_deref(), Some("github.com/egoist/lorca"), "{remote}");
        }
        assert_eq!(repository_of_remote("/srv/git/shop.git"), None);
        assert_eq!(repository_of_remote(""), None);
    }

    #[test]
    fn folders_meet_when_one_is_inside_the_other() {
        assert!(nested("~/dev/lorca", "~/dev/lorca"));
        assert!(nested("~/dev/lorca/desktop", "~/dev/lorca"));
        assert!(nested("~/dev/lorca", "~/dev/lorca/desktop"));
        assert!(!nested("~/dev/lorca-258", "~/dev/lorca"));
    }
}
