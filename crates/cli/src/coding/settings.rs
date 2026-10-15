//! Which coding agents bots may start, and which one a start takes. The account keeps the
//! agents in its order of preference, each on or off (`CodingAgents`, in the roster, set from
//! any Device with `coding_agents.set`). A start that names no agent takes the first one on that
//! is installed on the bot's Runner, found on the login shell's `PATH` as a terminal would find
//! it; one that names an agent the user turned off fails. `coding.agents` answers, for this
//! Device, each agent in order with whether it is on, installed, and its version.

use std::path::PathBuf;
use std::time::Duration;

use serde_json::{json, Value};

use super::{name, process, KINDS};
use crate::app::App;
use crate::model::CodingAgents;

/// OpenCode's own permissions for an agent Lorca runs (`OPENCODE_CONFIG_CONTENT`): commands,
/// fetches, and anything outside its folder ask, so they come to Lorca.
pub(crate) const OPENCODE_PERMISSIONS: &str = r#"{"permission":{"bash":"ask","webfetch":"ask","external_directory":"ask","edit":"allow"}}"#;
/// fx's permission mode for an agent Lorca runs (`FX_PERMISSION_MODE`): every command and change
/// asks, so it comes to Lorca. Its default, `auto`, has fx's own reviewer decide.
pub(crate) const FX_PERMISSION_MODE: &str = "ask";

/// How long `--version` has to answer.
const VERSION_TIMEOUT: Duration = Duration::from_secs(5);

/// The program each agent is.
fn program(kind: &str) -> &'static str {
    match kind {
        "codex" => "codex",
        "opencode" => "opencode",
        "pi" => "pi",
        "fx" => "fx",
        _ => "claude",
    }
}

/// Where agent `kind` is installed on this Device.
pub(crate) async fn find(kind: &str) -> Option<PathBuf> {
    process::find(program(kind)).await
}

/// The account's agents in its order, each on or off.
pub fn of(app: &App) -> CodingAgents {
    app.coding_agents_settings().normalized()
}

/// The agent a start takes: the one the bot names, when it is on and installed, else the first
/// one on in the account's order that is installed.
pub(crate) async fn pick(app: &App, asked: Option<&str>) -> Result<(String, PathBuf), String> {
    let settings = of(app);
    let on: Vec<&str> = settings.agents.iter().filter(|choice| choice.enabled).map(|choice| choice.kind.as_str()).collect();
    if let Some(kind) = asked.map(str::trim).filter(|kind| !kind.is_empty()) {
        if !KINDS.contains(&kind) {
            return Err(format!("agent is one of {}", KINDS.join(", ")));
        }
        if !on.contains(&kind) {
            return Err(format!("The user turned {} off in Settings → Coding Agents. Use another, or leave agent out.", name(kind)));
        }
        return match find(kind).await {
            Some(program) => Ok((kind.to_string(), program)),
            None => Err(format!("{} is not installed on this Runner. Leave agent out to use one that is, or ask the user to install it.", name(kind))),
        };
    }
    if on.is_empty() {
        return Err("The user turned every coding agent off in Settings → Coding Agents.".into());
    }
    for kind in &on {
        if let Some(program) = find(kind).await {
            return Ok((kind.to_string(), program));
        }
    }
    let names: Vec<&str> = on.iter().map(|kind| name(kind)).collect();
    Err(format!("None of the coding agents the user turned on ({}) is installed on this Runner. Ask the user to install one and sign in to it there.", names.join(", ")))
}

/// What `--version` says, its first line.
async fn version(path: &PathBuf) -> Option<String> {
    let mut command = lorca_agent::login_shell::command(path).await;
    command.arg("--version").stdin(std::process::Stdio::null()).stdout(std::process::Stdio::piped()).stderr(std::process::Stdio::null()).kill_on_drop(true);
    let output = tokio::time::timeout(VERSION_TIMEOUT, command.output()).await.ok()?.ok()?;
    let text = String::from_utf8_lossy(&output.stdout);
    text.lines().map(str::trim).find(|line| !line.is_empty()).map(|line| line.chars().take(80).collect())
}

/// `coding.agents`: each agent in the account's order, on or off, and whether this Device has
/// it installed, with its version.
pub async fn list(app: &App) -> Value {
    let settings = of(app);
    let mut agents = Vec::new();
    for choice in &settings.agents {
        let path = find(&choice.kind).await;
        let version = match &path {
            Some(path) => version(path).await,
            None => None,
        };
        agents.push(json!({
            "kind": choice.kind,
            "name": name(&choice.kind),
            "enabled": choice.enabled,
            "installed": path.is_some(),
            "version": version,
        }));
    }
    json!({ "agents": agents })
}

#[cfg(test)]
mod tests {
    use crate::model::{CodingAgentChoice, CodingAgents};

    #[test]
    fn the_account_lists_every_agent_once_in_its_order() {
        let settings = CodingAgents {
            agents: vec![
                CodingAgentChoice { kind: "pi".into(), enabled: true },
                CodingAgentChoice { kind: "claude".into(), enabled: false },
                CodingAgentChoice { kind: "pi".into(), enabled: false },
                CodingAgentChoice { kind: "cursor".into(), enabled: true },
            ],
        }
        .normalized();
        let kinds: Vec<(&str, bool)> = settings.agents.iter().map(|choice| (choice.kind.as_str(), choice.enabled)).collect();
        assert_eq!(kinds, vec![("pi", true), ("claude", false), ("codex", true), ("opencode", true), ("fx", true)]);
    }
}
