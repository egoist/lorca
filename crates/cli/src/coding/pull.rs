//! How an agent's pull request stands on GitHub: the Runner asks `gh` every few minutes for each
//! pull request an agent the apps still list opened, until it is merged or closed, and the agent's
//! card says so.

use std::future::Future;
use std::sync::Arc;
use std::time::Duration;

use crate::app::App;

/// How often the Runner reads the pull requests again.
const EVERY: Duration = Duration::from_secs(5 * 60);
/// An agent the apps no longer list: ended or idle for this long. Its pull request is not read.
const KEEP_SECS: f64 = 3.0 * 24.0 * 3600.0;

/// Reads the pull requests every few minutes, while the CLI runs.
pub fn watch(app: Arc<App>) {
    tokio::spawn(async move {
        loop {
            tokio::time::sleep(EVERY).await;
            refresh(&app, state).await;
        }
    });
}

/// Reads each pull request still open that a listed agent names with `read`, and writes what
/// changed to the agents' cards.
pub(crate) async fn refresh<F, Fut>(app: &App, read: F)
where
    F: Fn(String) -> Fut,
    Fut: Future<Output = Option<String>>,
{
    let now = crate::config::now_secs();
    let mut changed = false;
    for agent in app.coding_agents.all() {
        let (link, had) = {
            let record = agent.record.lock().unwrap();
            let listed = matches!(record.state.as_str(), "starting" | "working") || now - record.updated_at <= KEEP_SECS;
            match &record.pull_request {
                Some(link) if listed && !matches!(record.pull_state.as_deref(), Some("merged" | "closed")) => (link.clone(), record.pull_state.clone()),
                _ => continue,
            }
        };
        let Some(state) = read(link.clone()).await else { continue };
        {
            let mut record = agent.record.lock().unwrap();
            // A new pull request named meanwhile is read next time.
            if record.pull_request.as_ref() != Some(&link) || had.as_ref() == Some(&state) {
                continue;
            }
            record.pull_state = Some(state);
        }
        app.coding_agents.sync(app, &agent);
        changed = true;
    }
    if changed {
        super::save(app);
    }
}

/// How the GitHub pull request at `link` stands, as `gh pr view` says: `open`, `draft`, `merged`,
/// or `closed`. None for another host's link, without `gh`, or when it cannot tell.
pub(crate) async fn state(link: String) -> Option<String> {
    if !link.starts_with("https://github.com/") || !link.contains("/pull/") {
        return None;
    }
    let mut command = lorca_agent::login_shell::command("gh").await;
    command.args(["pr", "view", &link, "--json", "state,isDraft"]).stdin(std::process::Stdio::null());
    let output = tokio::time::timeout(Duration::from_secs(30), command.output()).await.ok()?.ok()?;
    if !output.status.success() {
        tracing::debug!(link, said = %String::from_utf8_lossy(&output.stderr).trim(), "reading a pull request");
        return None;
    }
    parse(&output.stdout)
}

/// `{"isDraft":false,"state":"MERGED"}` as `merged`.
fn parse(json: &[u8]) -> Option<String> {
    let value: serde_json::Value = serde_json::from_slice(json).ok()?;
    let state = match value["state"].as_str()? {
        "OPEN" if value["isDraft"].as_bool() == Some(true) => "draft",
        "OPEN" => "open",
        "MERGED" => "merged",
        "CLOSED" => "closed",
        _ => return None,
    };
    Some(state.into())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn gh_says_how_it_stands() {
        assert_eq!(parse(br#"{"isDraft":false,"state":"MERGED"}"#).as_deref(), Some("merged"));
        assert_eq!(parse(br#"{"isDraft":true,"state":"OPEN"}"#).as_deref(), Some("draft"));
        assert_eq!(parse(br#"{"isDraft":false,"state":"OPEN"}"#).as_deref(), Some("open"));
        assert_eq!(parse(br#"{"isDraft":false,"state":"CLOSED"}"#).as_deref(), Some("closed"));
        assert_eq!(parse(b"not json"), None);
    }
}
