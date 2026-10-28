//! The Claude Code provider on this Runner: a Claude subscription that Claude Code signs in to,
//! used through `claude -p` (`lorca_agent::providers::claude_code`). Its sign-in is the Runner's
//! own Claude Code login, so it is never part of the account's credentials: this Runner checks
//! it (`claude auth status`), keeps the answer, and lists the provider among the ones it keeps in
//! its machine blob, so the other Devices offer it for its bots.

use std::ffi::OsString;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use serde_json::Value;

use crate::app::App;
use crate::model::ProviderStatus;

pub const KIND: &str = "claude-code";
pub const NAME: &str = "Claude Code";
/// `LORCA_CLAUDE_CODE_COMMAND` names the `claude` to run instead of the one on the login shell's
/// `PATH`.
const COMMAND_ENV: &str = "LORCA_CLAUDE_CODE_COMMAND";
/// Set, a reply Anthropic charged to usage credits does not stop the turn.
const ALLOW_USAGE_CREDITS_ENV: &str = "LORCA_CLAUDE_CODE_ALLOW_USAGE_CREDITS";
const CHECK_TIMEOUT: Duration = Duration::from_secs(20);

/// What the last check found.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct Status {
    /// The `claude` found, or none when it is not installed.
    pub program: Option<PathBuf>,
    pub signed_in: bool,
    /// The plan of a subscription sign-in (`max`, `pro`, …).
    pub plan: Option<String>,
    pub email: Option<String>,
}

static STATUS: Mutex<Option<Status>> = Mutex::new(None);
/// The `lorca` whose `claude-code-tools` lists a bot's tools: this one, or the one a test names.
static TOOLS_PROGRAM: Mutex<Option<PathBuf>> = Mutex::new(None);
/// Whether this process asks Claude Code at all: `lorca serve` turns it on, so tests never run
/// the `claude` installed where they run.
static ENABLED: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);
/// The environment the last check ran Claude Code in, which turns run it in too.
static ENVIRONMENT: Mutex<Vec<(OsString, OsString)>> = Mutex::new(Vec::new());

/// The last check's answer; `None` before the first.
pub fn status() -> Option<Status> {
    STATUS.lock().unwrap().clone()
}

/// Lets this process ask Claude Code.
pub fn enable() {
    ENABLED.store(true, std::sync::atomic::Ordering::Relaxed);
}

/// Names the `lorca` that serves the tool list, for a test binary that is not one.
pub fn set_tools_program(program: Option<PathBuf>) {
    *TOOLS_PROGRAM.lock().unwrap() = program;
}

/// Puts `status` in place of a check, for tests.
pub fn set_status(status: Option<Status>) {
    *STATUS.lock().unwrap() = status;
}

/// Whether Claude Code is signed in here, as the last check found.
pub fn is_signed_in() -> bool {
    status().is_some_and(|status| status.signed_in)
}

/// The `claude` to run: `LORCA_CLAUDE_CODE_COMMAND`, else the one on the login shell's `PATH`.
async fn find() -> Option<PathBuf> {
    if let Some(command) = std::env::var_os(COMMAND_ENV).filter(|command| !command.is_empty()) {
        let path = PathBuf::from(command);
        return if path.components().count() > 1 { path.is_file().then_some(path) } else { crate::coding::process::find(&path.to_string_lossy()).await };
    }
    crate::coding::process::find("claude").await
}

/// The login shell's environment for Claude Code, without what would move it off its sign-in.
async fn environment() -> Vec<(OsString, OsString)> {
    let base: Vec<_> = lorca_agent::login_shell::environment().await.to_vec();
    let base = if base.is_empty() { std::env::vars_os().collect() } else { base };
    let environment: Vec<_> = base.into_iter().filter(|(name, _)| !lorca_agent::providers::claude_code::OVERRIDES.iter().any(|o| name.to_str() == Some(o))).collect();
    *ENVIRONMENT.lock().unwrap() = environment.clone();
    environment
}

/// Asks Claude Code whether it is signed in, and keeps the answer. A change reaches the apps and,
/// in the machine blob, the other Devices.
pub async fn check(app: &Arc<App>) -> Status {
    if !ENABLED.load(std::sync::atomic::Ordering::Relaxed) {
        return status().unwrap_or_default();
    }
    let found = match find().await {
        Some(program) => ask(&program).await,
        None => Status::default(),
    };
    let changed = {
        let mut status = STATUS.lock().unwrap();
        let changed = status.as_ref() != Some(&found);
        *status = Some(found.clone());
        changed
    };
    if changed {
        app.push_machine_blob_if_changed();
        app.emit(app.roster_summary());
    }
    found
}

/// Checks in the background, so the apps and the machine blob catch up with a sign-in made in a
/// terminal.
pub fn check_in_background(app: &Arc<App>) {
    let app = app.clone();
    tokio::spawn(async move {
        check(&app).await;
    });
}

async fn ask(program: &std::path::Path) -> Status {
    let mut status = Status { program: Some(program.to_path_buf()), ..Default::default() };
    let mut command = tokio::process::Command::new(program);
    command.args(["auth", "status"]).env_clear().envs(environment().await).stdin(std::process::Stdio::null()).kill_on_drop(true);
    let Ok(Ok(output)) = tokio::time::timeout(CHECK_TIMEOUT, command.output()).await else { return status };
    let Ok(answer) = serde_json::from_slice::<Value>(&output.stdout) else { return status };
    status.signed_in = answer["loggedIn"] == true;
    status.plan = answer["subscriptionType"].as_str().filter(|plan| !plan.is_empty()).map(str::to_string);
    status.email = answer["email"].as_str().filter(|email| !email.is_empty()).map(str::to_string);
    status
}

/// The provider as this Device lists it: signed in, kept here, or why not.
pub fn provider_status(review_model: Option<String>) -> ProviderStatus {
    let status = status();
    let detail = match &status {
        Some(Status { signed_in: true, plan, email, .. }) => {
            let plan = plan.as_deref().map(|plan| format!("Claude {}", capitalized(plan)));
            [plan, email.clone()].into_iter().flatten().collect::<Vec<_>>().join(" · ")
        }
        Some(Status { program: Some(_), .. }) => "Not signed in. Run claude auth login on this computer.".into(),
        Some(Status { program: None, .. }) => "Claude Code is not installed on this computer.".into(),
        None => "Checking Claude Code…".into(),
    };
    let signed_in = status.is_some_and(|status| status.signed_in);
    ProviderStatus { kind: KIND.into(), is_connected: signed_in, detail: if signed_in && detail.is_empty() { "Signed in".into() } else { detail }, review_model, kept: signed_in, ..Default::default() }
}

fn capitalized(word: &str) -> String {
    let mut chars = word.chars();
    chars.next().map(|first| first.to_uppercase().chain(chars).collect()).unwrap_or_default()
}

/// The adapter for a bot's turn on this Runner, from the last check. One that found no sign-in
/// checks again, for the next turn; a turn checks first when it starts (`turns`).
pub fn provider(app: &Arc<App>, model: &str, thinking: Option<lorca_agent::ThinkingLevel>) -> Result<lorca_agent::providers::ClaudeCodeProvider, String> {
    let status = status().unwrap_or_default();
    let Some(program) = status.program.clone().filter(|_| status.signed_in) else {
        check_in_background(app);
        return Err(not_ready(status.program.is_some()));
    };
    let lorca = match TOOLS_PROGRAM.lock().unwrap().clone() {
        Some(program) => program,
        None => std::env::current_exe().map_err(|error| format!("Could not find the lorca binary: {error}"))?,
    };
    let mut env = ENVIRONMENT.lock().unwrap().clone();
    // Before any check has read the login shell's: this process's, which has HOME and PATH.
    if env.is_empty() {
        env = std::env::vars_os().filter(|(name, _)| !lorca_agent::providers::claude_code::OVERRIDES.iter().any(|o| name.to_str() == Some(o))).collect();
    }
    let mut provider = lorca_agent::providers::ClaudeCodeProvider::new(program, (lorca, vec!["claude-code-tools".into()]), env, model).with_thinking(thinking);
    provider.allow_usage_credits = std::env::var_os(ALLOW_USAGE_CREDITS_ENV).is_some_and(|value| !matches!(value.to_str(), Some("" | "0" | "false" | "no" | "off")));
    Ok(provider)
}

/// Why a turn cannot run with Claude Code on this Runner.
pub fn not_ready(installed: bool) -> String {
    if installed {
        "Claude Code is not signed in on this Runner. Run claude auth login in a terminal on it.".into()
    } else {
        "Claude Code is not installed on this Runner. Install it (npm install -g @anthropic-ai/claude-code), then run claude auth login.".into()
    }
}
