//! Which coding agents a Runner's bots may start, and which one a start takes. Each Runner keeps
//! the agents in its user's order of preference, each on or off, in `coding-agents.json` in its
//! data directory (0600); any Device reads and changes it with `coding.agents` and
//! `coding.agents.set`, sealed to the Runner when it is another. A start that names no agent
//! takes the first one on that is installed, found on the login shell's `PATH` as a terminal
//! would find it; one that names an agent the user turned off fails. `coding.agents` answers each
//! agent in order with whether it is on, installed, and its version.

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
pub(crate) fn program(kind: &str) -> &'static str {
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

fn file(app: &App) -> PathBuf {
    app.config.home.join("coding-agents.json")
}

/// This Runner's agents in its order, each on or off: every agent on, in Lorca's order, until
/// the user changes them.
pub fn of(app: &App) -> CodingAgents {
    let saved: Option<CodingAgents> = std::fs::read(file(app)).ok().and_then(|bytes| serde_json::from_slice(&bytes).ok());
    saved.unwrap_or_default().normalized()
}

/// Keeps `settings` as this Runner's agents.
pub(crate) fn save(app: &App, settings: &CodingAgents) -> Result<(), String> {
    crate::config::write_json_private(&file(app), &settings.clone().normalized()).map_err(|error| format!("Could not save the coding agents: {error}"))
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
            return Err(format!("The user turned {} off for this Runner in Settings → Coding Agents. Use another, or leave agent out.", name(kind)));
        }
        return match find(kind).await {
            Some(program) => Ok((kind.to_string(), program)),
            None => Err(format!("{} is not installed on this Runner. Leave agent out to use one that is, or ask the user to install it.", name(kind))),
        };
    }
    if on.is_empty() {
        return Err("The user turned every coding agent off for this Runner in Settings → Coding Agents.".into());
    }
    for kind in &on {
        if let Some(program) = find(kind).await {
            return Ok((kind.to_string(), program));
        }
    }
    let names: Vec<&str> = on.iter().map(|kind| name(kind)).collect();
    Err(format!("None of the coding agents the user turned on ({}) is installed on this Runner. Ask the user to install one and sign in to it there.", names.join(", ")))
}

/// The agents a start in a cloud sandbox may take, in the account's order: the one the bot names
/// when it is on, else every one on. Which is installed is the sandbox's to say.
pub(crate) fn pick_anywhere(app: &App, asked: Option<&str>) -> Result<Vec<String>, String> {
    let settings = of(app);
    let on: Vec<String> = settings.agents.iter().filter(|choice| choice.enabled).map(|choice| choice.kind.clone()).collect();
    match asked.map(str::trim).filter(|kind| !kind.is_empty()) {
        Some(kind) if !KINDS.contains(&kind) => Err(format!("agent is one of {}", KINDS.join(", "))),
        Some(kind) if !on.iter().any(|each| each == kind) => Err(format!("The user turned {} off for this Runner in Settings → Coding Agents. Use another, or leave agent out.", name(kind))),
        Some(kind) => Ok(vec![kind.to_string()]),
        None if on.is_empty() => Err("The user turned every coding agent off for this Runner in Settings → Coding Agents.".into()),
        None => Ok(on),
    }
}

/// What `--version` says, its first line.
async fn version(path: &PathBuf) -> Option<String> {
    let mut command = lorca_agent::login_shell::command(path).await;
    command.arg("--version").stdin(std::process::Stdio::null()).stdout(std::process::Stdio::piped()).stderr(std::process::Stdio::null()).kill_on_drop(true);
    let output = tokio::time::timeout(VERSION_TIMEOUT, command.output()).await.ok()?.ok()?;
    let text = String::from_utf8_lossy(&output.stdout);
    text.lines().map(str::trim).find(|line| !line.is_empty()).map(|line| line.chars().take(80).collect())
}

/// `coding.agents`: each agent in this Runner's order, on or off, and whether it is installed
/// here, with its version.
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

/// `coding.agents` and `coding.agents.set { agents: [{ kind, enabled }] }` on this Runner; a set
/// answers the list as it now stands.
pub async fn serve(app: &App, verb: &str, body: &Value) -> Result<Value, String> {
    if verb == "coding.agents.set" {
        let settings: CodingAgents = serde_json::from_value(json!({ "agents": body["agents"] })).map_err(|_| "agents is a list of { kind, enabled }".to_string())?;
        save(app, &settings)?;
    }
    Ok(list(app).await)
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
