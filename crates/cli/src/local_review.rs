//! Auto-review for commands that act on a Runner itself. A shell command keeps the Runner
//! user's full authority; the boundary is the review and permission decision before it starts.

use std::path::Path;
use std::sync::Arc;

use lorca_agent::{BeforeToolCallContext, BeforeToolCallResult};
use serde_json::Value;
use sha2::{Digest, Sha256};

use crate::app::App;
use crate::model::{AutoReviewRule, Bot};
use crate::plugins::mcp::{self, Decision};
use crate::plugins::review::{self, Outcome};

const LOCAL_TARGET_ID: &str = "computer";

/// Reviews every shell call before `bash` receives it. A returned result blocks the call;
/// `None` lets it execute unchanged with the Runner user's normal authority.
pub async fn before_tool_call(
    app: &Arc<App>,
    chat_id: &str,
    bot: &Bot,
    workdir: &Path,
    unattended: bool,
    ctx: BeforeToolCallContext<'_>,
) -> Option<BeforeToolCallResult> {
    if ctx.tool_call.name != "bash" {
        return None;
    }
    let command = ctx.args.get("command").and_then(Value::as_str).unwrap_or("");
    let inspection = inspect_shell(command);
    let reusable_patterns = inspection.force_ask.is_none();
    let workdir = std::fs::canonicalize(workdir).unwrap_or_else(|_| workdir.to_path_buf());
    let runner_id = app.this_device_id().unwrap_or_else(|| bot.runner_id.clone());
    let runner_name = app.device(&runner_id).map(|device| device.name).unwrap_or_else(|| "this Runner".into());
    let rule_key = scoped_rule_key(&runner_id, &workdir, ctx.args);
    let auto_review = app.auto_review();
    let has_exact_rule = auto_review.rules.iter().any(|rule| rule.tool.as_deref() == Some(&rule_key));
    let pattern_outcome = auto_review
        .is_enabled
        .then(|| shell_rule_outcome(&auto_review.rules, &runner_id, &workdir, command))
        .flatten();
    let description = format!(
        "Run this shell command as the user on {runner_name}, with full filesystem, process, credential, and network access. Working directory: {}. Parsed review: {}",
        workdir.display(), inspection.summary
    );
    let outcome = if let Some(Outcome::Ask { reason }) = &pattern_outcome {
        Outcome::Ask { reason: reason.clone() }
    } else if has_exact_rule {
        review::decide_with_rule_key(
            app,
            bot,
            chat_id,
            Some(&rule_key),
            &runner_name,
            "bash",
            &description,
            ctx.args,
            ctx.cancel,
        )
        .await
    } else if let Some(reason) = inspection.force_ask {
        Outcome::Ask { reason: Some(reason) }
    } else if let Some(outcome) = pattern_outcome {
        outcome
    } else {
        review::decide_with_rule_key(
            app,
            bot,
            chat_id,
            Some(&rule_key),
            &runner_name,
            "bash",
            &description,
            ctx.args,
            ctx.cancel,
        )
        .await
    };
    let Outcome::Ask { reason } = outcome else { return None };
    if unattended {
        return Some(blocked(format!(
            "bash needs the user's permission ({}), and nobody is here to give it. Report what you would do; the user can add a narrow Auto-review rule allowing it.",
            reason.as_deref().unwrap_or("Auto-review requires a check")
        )));
    }

    let patterns = if reusable_patterns { shell_patterns(command) } else { Vec::new() };
    let always_rule = AutoReviewRule {
        id: uuid::Uuid::new_v4().to_string(),
        text: command.to_string(),
        behavior: "allow".into(),
        tool: Some(rule_key),
        runner_id: Some(runner_id),
        workdir: Some(workdir.display().to_string()),
        command: Some(command.to_string()),
        patterns,
    };
    match mcp::ask_with_rule(
        app,
        chat_id,
        &bot.id,
        LOCAL_TARGET_ID,
        &runner_name,
        "bash",
        &command_summary(command),
        ctx.args.clone(),
        reason,
        Some(always_rule),
        ctx.cancel,
    )
    .await
    {
        Decision::Allowed | Decision::Always => None,
        Decision::Denied => Some(blocked("The user did not allow bash. Do not retry it; ask what they want instead.".into())),
        Decision::Expired => Some(blocked("Nobody answered the permission request for bash in time. Say what you needed and stop.".into())),
    }
}

fn blocked(reason: String) -> BeforeToolCallResult {
    BeforeToolCallResult { block: true, reason: Some(reason), args: None, terminate: false }
}

pub(crate) fn scoped_rule_key(runner_id: &str, workdir: &Path, args: &Value) -> String {
    let mut digest = Sha256::new();
    digest.update(runner_id.as_bytes());
    digest.update([0]);
    digest.update(workdir.to_string_lossy().as_bytes());
    digest.update([0]);
    digest.update(serde_json::to_vec(args).unwrap_or_default());
    let hash = digest.finalize();
    let short: String = hash[..16].iter().map(|byte| format!("{byte:02x}")).collect();
    format!("computer/bash/{short}")
}

/// OpenCode-style reusable prefixes for the visible commands in one shell call. The exact
/// command remains on the rule for audit and as a narrow fallback; these patterns let ordinary
/// variations such as `git status --short` reuse an approval for `git status *`.
pub(crate) fn shell_patterns(command: &str) -> Vec<String> {
    shell_actions(command).into_iter().map(|(_, pattern)| pattern).fold(Vec::new(), |mut out, pattern| {
        if !out.contains(&pattern) {
            out.push(pattern);
        }
        out
    })
}

pub(crate) fn reusable_shell_patterns(command: &str) -> Vec<String> {
    inspect_shell(command).force_ask.is_none().then(|| shell_patterns(command)).unwrap_or_default()
}

fn shell_rule_outcome(rules: &[AutoReviewRule], runner_id: &str, workdir: &Path, command: &str) -> Option<Outcome> {
    let resources: Vec<String> = shell_actions(command).into_iter().map(|(resource, _)| resource).collect();
    if resources.is_empty() {
        return None;
    }
    let workdir = workdir.to_string_lossy();
    let scoped: Vec<&AutoReviewRule> = rules
        .iter()
        .filter(|rule| {
            !rule.patterns.is_empty()
                && rule.runner_id.as_deref() == Some(runner_id)
                && rule.workdir.as_deref() == Some(workdir.as_ref())
        })
        .collect();
    if scoped.is_empty() {
        return None;
    }
    if let Some((rule, pattern)) = scoped
        .iter()
        .filter(|rule| rule.behavior == "ask")
        .find_map(|rule| matching_pattern(rule, &resources).map(|pattern| (*rule, pattern)))
    {
        return Some(Outcome::Ask { reason: Some(format!("Your shell rule asks first for {pattern}: {}", rule.text)) });
    }
    let covered = resources.iter().all(|resource| {
        scoped
            .iter()
            .filter(|rule| rule.behavior == "allow")
            .any(|rule| rule.patterns.iter().any(|pattern| shell_pattern_matches(resource, pattern)))
    });
    covered.then_some(Outcome::Allow)
}

fn matching_pattern<'a>(rule: &'a AutoReviewRule, resources: &[String]) -> Option<&'a str> {
    rule.patterns
        .iter()
        .find(|pattern| resources.iter().any(|resource| shell_pattern_matches(resource, pattern)))
        .map(String::as_str)
}

fn shell_pattern_matches(resource: &str, pattern: &str) -> bool {
    if pattern == "*" {
        return true;
    }
    if let Some(prefix) = pattern.strip_suffix(" *") {
        return resource == prefix || resource.strip_prefix(prefix).is_some_and(|tail| tail.starts_with(' '));
    }
    resource == pattern
}

fn shell_actions(command: &str) -> Vec<(String, String)> {
    let (stages, _) = parsed_shell(command);
    let mut out = Vec::new();
    for stage in stages {
        let Some((command, args)) = command_and_args(&stage) else { continue };
        if matches!(
            command.as_str(),
            "cd" | "chdir" | "pushd" | "popd" | "for" | "while" | "until" | "case" | "esac" | "if" | "then" | "else" | "elif" | "fi"
                | "do" | "done" | "select" | "function" | "{" | "}" | "[" | "[["
        ) {
            continue;
        }
        let resource = std::iter::once(command.as_str()).chain(args.iter().map(String::as_str)).collect::<Vec<_>>().join(" ");
        let prefix = shell_pattern_prefix(&command, &args);
        if !prefix.is_empty() {
            out.push((resource, format!("{prefix} *")));
        }
    }
    out
}

fn shell_pattern_prefix(command: &str, args: &[String]) -> String {
    let positionals = positional_arguments(command, args);
    let first = positionals.first().map(String::as_str);
    let arity: usize = match command {
        "aws" | "az" | "doctl" | "gcloud" | "gh" | "sfdx" => 3,
        "git" if matches!(first, Some("config" | "remote" | "stash")) => 3,
        "cargo" if matches!(first, Some("add" | "run")) => 3,
        "npm" | "pnpm" | "yarn" | "bun" if matches!(first, Some("run" | "exec" | "dlx" | "x" | "init" | "view")) => 3,
        "docker" | "podman" if matches!(first, Some("builder" | "compose" | "container" | "image" | "network" | "volume")) => 3,
        "kubectl" if matches!(first, Some("kustomize" | "rollout")) => 3,
        "cargo" | "npm" | "pnpm" | "yarn" | "bun" | "deno" | "docker" | "podman" | "kubectl" | "helm" | "terraform" | "git" | "go"
        | "make" | "cmake" | "gradle" | "mvn" | "swift" | "brew" | "pip" | "pip3" | "poetry" | "rustup" | "systemctl" => 2,
        _ => 1,
    };
    std::iter::once(command)
        .chain(positionals.iter().take(arity.saturating_sub(1)).map(String::as_str))
        .collect::<Vec<_>>()
        .join(" ")
}

fn positional_arguments(command: &str, args: &[String]) -> Vec<String> {
    let mut out = Vec::new();
    let mut skip_next = false;
    for arg in args {
        if skip_next {
            skip_next = false;
            continue;
        }
        if command == "git" && matches!(arg.as_str(), "-C" | "-c" | "--git-dir" | "--work-tree") {
            skip_next = true;
            continue;
        }
        if arg.starts_with('-') {
            continue;
        }
        out.push(arg.clone());
    }
    out
}

fn command_summary(command: &str) -> String {
    let one_line = command.split_whitespace().collect::<Vec<_>>().join(" ");
    let shown: String = one_line.chars().take(180).collect();
    format!("$ {shown}{}", if one_line.chars().count() > 180 { "…" } else { "" })
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct ShellInspection {
    force_ask: Option<String>,
    summary: String,
    read_only: bool,
}

fn inspect_shell(command: &str) -> ShellInspection {
    let (stages, dynamic) = parsed_shell(command);
    let redirection = has_effectful_redirection(command);
    let read_only = !command.trim().is_empty() && !dynamic && !redirection && stages.iter().all(|stage| safe_stage(stage));
    let may_network = dynamic || stages.iter().any(|stage| stage_may_network(stage));
    let force_ask = shell_danger_reason(&stages, dynamic);
    let summary = if let Some(reason) = &force_ask {
        reason.clone()
    } else if redirection {
        "writes through shell redirection".into()
    } else if read_only {
        format!("{} parsed read-only stage(s)", stages.len())
    } else if may_network {
        "runs code or a command that may access the network".into()
    } else {
        "runs code or may change files or processes".into()
    };
    ShellInspection { force_ask, summary, read_only }
}

fn shell_danger_reason(stages: &[String], dynamic: bool) -> Option<String> {
    if dynamic {
        return Some("The command contains dynamic evaluation, so its effects cannot be bounded from the text alone.".into());
    }
    for stage in stages {
        let Some((command, args)) = command_and_args(stage) else { continue };
        if matches!(
            command.as_str(),
            "rm" | "rmdir" | "sudo" | "doas" | "su" | "chmod" | "chown" | "chgrp" | "dd" | "mkfs" | "diskutil" | "shutdown" | "reboot" | "launchctl"
                | "systemctl" | "kill" | "pkill" | "killall" | "eval" | "source" | "bash" | "sh" | "zsh" | "fish" | "python" | "python3" | "node"
                | "deno" | "bun" | "ruby" | "perl" | "php" | "osascript" | "security" | "docker" | "podman" | "ssh" | "open" | "xdg-open"
                | "shortcuts" | "mount" | "umount" | "npx" | "bunx" | "pipx" | "uvx" | "env" | "command" | "timeout" | "nice" | "nohup" | "setsid"
                | "xcrun"
        ) || command == "."
        {
            return Some(format!("The command uses {command}, which can execute opaque code, delete data, change access, elevate privileges, or alter running services."));
        }
        if command == "xargs" && !safe_xargs(&args) {
            return Some("The xargs command can execute another command for every input item.".into());
        }
        if command == "find" && args.iter().any(|arg| matches!(arg.as_str(), "-delete" | "-exec" | "-execdir" | "-ok" | "-okdir")) {
            return Some("The find command can delete files or execute another command for every match.".into());
        }
        if command == "git" {
            let text = args.join(" ");
            if (args.iter().any(|arg| arg == "reset") && args.iter().any(|arg| arg == "--hard"))
                || args.iter().any(|arg| arg == "clean")
                || (args.iter().any(|arg| arg == "push") && (text.contains("--force") || text.contains("-f")))
            {
                return Some("The Git command can discard work or rewrite shared history.".into());
            }
        }
        if command == "kubectl" && args.iter().any(|arg| matches!(arg.as_str(), "exec" | "port-forward" | "proxy" | "cp")) {
            return Some("The kubectl command opens an execution, forwarding, or file-transfer channel on another computer.".into());
        }
        if matches!(command.as_str(), "scp" | "sftp" | "rsync") {
            return Some(format!("The command uses {command} to transfer local data to another machine."));
        }
        if command == "curl"
            && args.iter().any(|arg| {
                matches!(
                    arg.as_str(),
                    "-d" | "--data" | "--data-raw" | "--data-binary" | "-F" | "--form" | "-T" | "--upload-file" | "-X" | "--request"
                )
            })
        {
            return Some("The command sends data or performs a non-read-only HTTP request.".into());
        }
    }
    None
}

/// Parses visible command substitutions recursively, replacing each result in its outer command
/// with a value placeholder. A substitution used as an argument or assignment can then stay
/// read-only, while every command inside it is reviewed as its own stage.
fn parsed_shell(command: &str) -> (Vec<String>, bool) {
    let (outer, substitutions, mut dynamic) = extract_command_substitutions(command);
    let (mut stages, outer_dynamic) = shell_stages(&outer);
    dynamic |= outer_dynamic;
    for substitution in substitutions {
        let (nested, nested_dynamic) = parsed_shell(&substitution);
        stages.extend(nested);
        dynamic |= nested_dynamic;
    }
    (stages, dynamic)
}

fn extract_command_substitutions(command: &str) -> (String, Vec<String>, bool) {
    let chars: Vec<char> = command.chars().collect();
    let mut outer = String::with_capacity(command.len());
    let mut substitutions = Vec::new();
    let mut quote = None;
    let mut escaped = false;
    let mut dynamic = false;
    let mut i = 0;
    while i < chars.len() {
        let c = chars[i];
        if escaped {
            outer.push(c);
            escaped = false;
            i += 1;
            continue;
        }
        if c == '\\' && quote != Some('\'') {
            outer.push(c);
            escaped = true;
            i += 1;
            continue;
        }
        if c == '`' && quote != Some('\'') {
            match backtick_substitution(&chars, i) {
                Some((body, end)) => {
                    substitutions.push(body);
                    outer.push_str("__substitution_value__");
                    i = end + 1;
                }
                None => {
                    dynamic = true;
                    outer.push(c);
                    i += 1;
                }
            }
            continue;
        }
        if c == '$' && chars.get(i + 1) == Some(&'(') && quote != Some('\'') {
            if chars.get(i + 2) == Some(&'(') {
                match command_substitution(&chars, i + 1) {
                    Some((_, end)) => {
                        outer.push_str("__arithmetic_value__");
                        i = end + 1;
                    }
                    None => {
                        dynamic = true;
                        outer.push(c);
                        i += 1;
                    }
                }
                continue;
            }
            match command_substitution(&chars, i + 1) {
                Some((body, end)) => {
                    substitutions.push(body);
                    outer.push_str("__substitution_value__");
                    i = end + 1;
                    continue;
                }
                None => {
                    dynamic = true;
                    outer.push(c);
                    i += 1;
                    continue;
                }
            }
        }
        if let Some(q) = quote {
            outer.push(c);
            if c == q {
                quote = None;
            }
            i += 1;
            continue;
        }
        if matches!(c, '\'' | '"') {
            quote = Some(c);
        }
        outer.push(c);
        i += 1;
    }
    (outer, substitutions, dynamic)
}

fn command_substitution(chars: &[char], open: usize) -> Option<(String, usize)> {
    let mut depth = 1;
    let mut quote = None;
    let mut escaped = false;
    let mut i = open + 1;
    while i < chars.len() {
        let c = chars[i];
        if escaped {
            escaped = false;
            i += 1;
            continue;
        }
        if c == '\\' && quote != Some('\'') {
            escaped = true;
            i += 1;
            continue;
        }
        if let Some(q) = quote {
            if c == q {
                quote = None;
            }
            i += 1;
            continue;
        }
        match c {
            '\'' | '"' | '`' => quote = Some(c),
            '(' => depth += 1,
            ')' => {
                depth -= 1;
                if depth == 0 {
                    return Some((chars[open + 1..i].iter().collect(), i));
                }
            }
            _ => {}
        }
        i += 1;
    }
    None
}

fn backtick_substitution(chars: &[char], open: usize) -> Option<(String, usize)> {
    let mut escaped = false;
    let mut i = open + 1;
    while i < chars.len() {
        let c = chars[i];
        if escaped {
            escaped = false;
            i += 1;
            continue;
        }
        if c == '\\' {
            escaped = true;
            i += 1;
            continue;
        }
        if c == '`' {
            return Some((chars[open + 1..i].iter().collect(), i));
        }
        i += 1;
    }
    None
}

/// Splits `&&`, `||`, pipes, semicolons, and newlines outside quotes. The command is still run
/// as one unit, but every stage is classified before any process starts.
fn shell_stages(command: &str) -> (Vec<String>, bool) {
    let mut stages = Vec::new();
    let mut current = String::new();
    let mut quote = None;
    let mut escaped = false;
    let mut dynamic = false;
    let chars: Vec<char> = command.chars().collect();
    let mut i = 0;
    while i < chars.len() {
        let c = chars[i];
        if escaped {
            current.push(c);
            escaped = false;
            i += 1;
            continue;
        }
        if c == '\\' && quote != Some('\'') {
            current.push(c);
            escaped = true;
            i += 1;
            continue;
        }
        if let Some(q) = quote {
            current.push(c);
            if c == q {
                quote = None;
            } else if q == '"' && c == '`' {
                dynamic = true;
            }
            i += 1;
            continue;
        }
        match c {
            '\'' | '"' => {
                quote = Some(c);
                current.push(c);
            }
            '`' => {
                dynamic = true;
                current.push(c);
            }
            ';' | '\n' | '|' | '&' => {
                if !current.trim().is_empty() {
                    stages.push(current.trim().to_string());
                    current.clear();
                }
                if chars.get(i + 1) == Some(&c) {
                    i += 1;
                }
            }
            '(' | ')' => {
                // Plain subshell grouping is visible syntax, not opaque evaluation. Treat its
                // boundary like the other stage separators. `$(` was marked dynamic above.
                if !current.trim().is_empty() {
                    stages.push(current.trim().to_string());
                    current.clear();
                }
            }
            _ => current.push(c),
        }
        i += 1;
    }
    if !current.trim().is_empty() {
        stages.push(current.trim().to_string());
    }
    if quote.is_some() || escaped {
        dynamic = true;
    }
    (stages, dynamic)
}

fn shell_words(stage: &str) -> Vec<String> {
    let mut words = Vec::new();
    let mut current = String::new();
    let mut quote = None;
    let mut escaped = false;
    for c in stage.chars() {
        if escaped {
            current.push(c);
            escaped = false;
            continue;
        }
        if c == '\\' && quote != Some('\'') {
            escaped = true;
            continue;
        }
        if let Some(q) = quote {
            if c == q {
                quote = None;
            } else {
                current.push(c);
            }
            continue;
        }
        match c {
            '\'' | '"' => quote = Some(c),
            c if c.is_whitespace() => {
                if !current.is_empty() {
                    words.push(std::mem::take(&mut current));
                }
            }
            _ => current.push(c),
        }
    }
    if !current.is_empty() {
        words.push(current);
    }
    words
}

fn command_and_args(stage: &str) -> Option<(String, Vec<String>)> {
    let mut words = shell_words(stage);
    loop {
        while words.first().is_some_and(|word| is_assignment(word)) {
            words.remove(0);
        }
        match words.first().map(String::as_str) {
            Some("then" | "do" | "else" | "elif" | "if" | "while" | "until" | "!") => {
                words.remove(0);
            }
            Some("for" | "select" | "case" | "esac" | "fi" | "done" | "{" | "}") | None => return None,
            _ => break,
        }
    }
    let command = words.first()?.rsplit('/').next().unwrap_or(&words[0]).to_ascii_lowercase();
    Some((command, words.into_iter().skip(1).collect()))
}

fn is_assignment(word: &str) -> bool {
    let Some((name, _)) = word.split_once('=') else { return false };
    !name.is_empty() && name.chars().enumerate().all(|(i, c)| c == '_' || c.is_ascii_alphanumeric() && (i > 0 || !c.is_ascii_digit()))
}

fn safe_stage(stage: &str) -> bool {
    let Some((command, args)) = command_and_args(stage) else {
        // `command_and_args` removes assignments and recognizes inert control syntax such as
        // `for …`, `do`, and `done`; with no executable left, this stage has no effect itself.
        return true;
    };
    match command.as_str() {
        "cd" => true,
        "xargs" => safe_xargs(&args),
        _ => safe_command(&command, &args),
    }
}

fn safe_xargs(args: &[String]) -> bool {
    let mut i = 0;
    while let Some(arg) = args.get(i) {
        if matches!(arg.as_str(), "-0" | "--null" | "-r" | "--no-run-if-empty" | "-t" | "--verbose" | "-x") {
            i += 1;
            continue;
        }
        if matches!(
            arg.as_str(),
            "-n" | "--max-args" | "-P" | "--max-procs" | "-L" | "--max-lines" | "-s" | "--max-chars" | "-d" | "--delimiter" | "-E" | "--eof"
                | "-I" | "--replace"
        ) {
            i += 2;
            continue;
        }
        if arg.starts_with('-') {
            return false;
        }
        let command = arg.rsplit('/').next().unwrap_or(arg).to_ascii_lowercase();
        return safe_command(&command, &args[i + 1..]);
    }
    false
}

fn safe_command(command: &str, args: &[String]) -> bool {
    match command {
        "pwd" | "true" | "false" | "whoami" | "id" | "uname" | "date" | "printf" | "echo" | "cat" | "head" | "tail" | "wc" | "stat"
        | "file" | "du" | "df" | "which" | "type" | "basename" | "dirname" | "realpath" | "sort" | "uniq" | "cut" | "tr" | "jq" | "ls"
        | "grep" | "test" | "[" | "[[" => true,
        "rg" => !args.iter().any(|arg| matches!(arg.as_str(), "-z" | "--search-zip" | "--pre" | "--pre-glob" | "--hostname-bin")),
        "sed" => !args.iter().any(|arg| arg == "-i" || arg.starts_with("-i") || arg == "--in-place" || arg.starts_with("--in-place=")),
        "awk" => !args.iter().any(|arg| {
            let lower = arg.to_ascii_lowercase();
            lower.contains("system(") || lower.contains("| getline") || lower.contains("@load") || lower.contains('>') || arg == "-f"
        }),
        "find" => !args.iter().any(|arg| matches!(arg.as_str(), "-delete" | "-exec" | "-execdir" | "-ok" | "-okdir")),
        "git" => safe_git(args),
        _ => false,
    }
}

fn safe_git(args: &[String]) -> bool {
    if args.iter().any(|arg| arg == "-c" || arg.starts_with("--config-env")) {
        return false;
    }
    let Some(subcommand) = args.iter().find(|arg| !arg.starts_with('-')) else { return false };
    if matches!(subcommand.as_str(), "status" | "diff" | "log" | "show" | "rev-parse" | "describe" | "blame" | "ls-files" | "ls-tree" | "cat-file") {
        return true;
    }
    subcommand == "branch"
        && args
            .iter()
            .filter(|arg| *arg != "branch")
            .all(|arg| matches!(arg.as_str(), "-a" | "--all" | "-r" | "--remotes" | "-v" | "-vv" | "--verbose" | "--list" | "--show-current"))
}

fn stage_may_network(stage: &str) -> bool {
    if safe_stage(stage) {
        return false;
    }
    let Some((command, args)) = command_and_args(stage) else { return false };
    if matches!(
        command.as_str(),
        "curl" | "wget" | "ssh" | "scp" | "sftp" | "rsync" | "nc" | "ncat" | "netcat" | "telnet" | "ftp" | "gh" | "glab" | "aws" | "gcloud"
            | "az" | "kubectl" | "helm" | "docker" | "podman" | "python" | "python3" | "node" | "deno" | "bun" | "ruby" | "perl" | "php" | "java"
            | "bash" | "sh" | "zsh" | "fish" | "osascript" | "make" | "just" | "task" | "npx" | "pnpm" | "yarn" | "pip" | "pip3"
    ) {
        return true;
    }
    if command == "git" {
        return args.iter().any(|arg| matches!(arg.as_str(), "clone" | "fetch" | "pull" | "push" | "ls-remote" | "submodule"));
    }
    if command == "cargo" {
        return args.iter().any(|arg| matches!(arg.as_str(), "install" | "fetch" | "update" | "search" | "publish" | "login"));
    }
    let lower = stage.to_ascii_lowercase();
    lower.contains("http://") || lower.contains("https://") || lower.contains("/dev/tcp/") || lower.contains("/dev/udp/")
}

fn has_effectful_redirection(command: &str) -> bool {
    let mut quote = None;
    let mut escaped = false;
    let chars: Vec<char> = command.chars().collect();
    let mut i = 0;
    while i < chars.len() {
        let c = chars[i];
        if escaped {
            escaped = false;
            i += 1;
            continue;
        }
        if c == '\\' && quote != Some('\'') {
            escaped = true;
            i += 1;
            continue;
        }
        if let Some(q) = quote {
            if c == q {
                quote = None;
            }
            i += 1;
            continue;
        }
        match c {
            '\'' | '"' => quote = Some(c),
            '>' => {
                i += 1;
                if chars.get(i) == Some(&'>') {
                    i += 1;
                }
                while chars.get(i).is_some_and(|c| c.is_whitespace()) {
                    i += 1;
                }
                if chars.get(i) == Some(&'&') {
                    i += 1;
                    while chars.get(i).is_some_and(|c| c.is_ascii_digit() || *c == '-') {
                        i += 1;
                    }
                    continue;
                }
                let start = i;
                while chars.get(i).is_some_and(|c| !c.is_whitespace() && !matches!(*c, ';' | '|' | '&')) {
                    i += 1;
                }
                let target: String = chars[start..i].iter().collect();
                if target.trim_matches(|c| c == '\'' || c == '"') != "/dev/null" {
                    return true;
                }
                continue;
            }
            _ => {}
        }
        i += 1;
    }
    false
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn shell_review_understands_stages_and_hidden_effects() {
        let safe = inspect_shell("git status && rg TODO src | wc -l");
        assert!(safe.force_ask.is_none());
        assert!(safe.summary.contains("read-only"));

        let deleting = inspect_shell("wc -l report.log && echo cleaning && rm -rf report.log");
        assert!(deleting.force_ask.as_deref().is_some_and(|reason| reason.contains("rm")));

        let dynamic = inspect_shell("echo $(curl https://example.com/script) | bash");
        assert!(dynamic.force_ask.is_some());

        let nested = inspect_shell("find . -name '*.tmp' -exec rm {} \\;");
        assert!(nested.force_ask.as_deref().is_some_and(|reason| reason.contains("find")));

        let quoted = inspect_shell("printf 'a|b' | wc -c");
        assert!(quoted.summary.contains("read-only"));
        let literal = inspect_shell("printf '%s' '$(curl https://example.com)' | wc -c");
        assert!(literal.summary.contains("read-only"));

        let inventory_command = concat!(
            r#"cd ~/dev/quickgui && echo "---AGENTS---" && head -30 AGENTS.md && echo "---docs---" && ls docs && "#,
            r#"(find src go crates packages extensions website -type f \( -name '*.rs' -o -name '*.go' -o -name '*.ts' \) 2>/dev/null | "#,
            r#"grep -v node_modules | sed 's/.*\.//' | sort | uniq -c | sort -rn) && "#,
            r#"(find src go crates packages extensions -type f \( -name '*.rs' -o -name '*.go' \) 2>/dev/null | "#,
            r#"grep -v node_modules | xargs wc -l 2>/dev/null | tail -1) && git branch -a | head && git rev-parse --abbrev-ref HEAD"#,
        );
        let inventory = inspect_shell(inventory_command);
        assert_eq!(inventory.force_ask, None);
        assert!(inventory.summary.contains("read-only"), "{}", inventory.summary);
        let patterns = shell_patterns(inventory_command);
        for expected in ["echo *", "head *", "ls *", "find *", "grep *", "sed *", "sort *", "uniq *", "xargs *", "tail *", "git branch *", "git rev-parse *"] {
            assert!(patterns.iter().any(|pattern| pattern == expected), "missing {expected}: {patterns:?}");
        }

        let xargs_delete = inspect_shell("find . -print0 | xargs -0 rm -rf");
        assert!(xargs_delete.force_ask.as_deref().is_some_and(|reason| reason.contains("xargs")));
        assert!(inspect_shell("find . -print0 | xargs -0 wc -l").summary.contains("read-only"));
        assert!(inspect_shell("echo ok > report.txt").summary.contains("redirection"));

        let loop_inventory = concat!(
            r#"cd ~/dev/quickgui && echo "=== LOC by area ===" && for d in src crates go packages website extensions tests; "#,
            r#"do n=$(find $d -type f \( -name '*.rs' -o -name '*.go' -o -name '*.ts' -o -name '*.tsx' \) "#,
            r#"-not -path '*/node_modules/*' -not -path '*/vendor/*' -not -path '*/target/*' 2>/dev/null | "#,
            r#"xargs wc -l 2>/dev/null | tail -1 | awk '{print $1}'); echo "$d: $n"; done && "#,
            r#"echo "=== crates ===" && ls crates/quickgui-host/src crates/quickgui-extension-sdk/src 2>/dev/null | head -40"#,
        );
        let loop_review = inspect_shell(loop_inventory);
        assert_eq!(loop_review.force_ask, None);
        let (loop_stages, loop_dynamic) = parsed_shell(loop_inventory);
        assert!(
            loop_review.summary.contains("read-only"),
            "{}; dynamic={loop_dynamic}; stages={:?}",
            loop_review.summary,
            loop_stages.iter().map(|stage| (stage, safe_stage(stage))).collect::<Vec<_>>()
        );
        let loop_patterns = shell_patterns(loop_inventory);
        for expected in ["echo *", "find *", "xargs *", "tail *", "awk *", "ls *", "head *"] {
            assert!(loop_patterns.iter().any(|pattern| pattern == expected), "missing {expected}: {loop_patterns:?}");
        }
        assert!(inspect_shell(r#"echo "$(rm -rf report)""#).force_ask.as_deref().is_some_and(|reason| reason.contains("rm")));
        assert!(inspect_shell("echo `pwd` $((1 + 2))").summary.contains("read-only"));
        assert!(inspect_shell("echo `rm -rf report`").force_ask.as_deref().is_some_and(|reason| reason.contains("rm")));
    }

    #[test]
    fn exact_rules_are_bound_to_runner_workspace_and_arguments() {
        let args = serde_json::json!({ "command": "git status" });
        let one = scoped_rule_key("runner-a", Path::new("/work/a"), &args);
        assert_eq!(one, scoped_rule_key("runner-a", Path::new("/work/a"), &args));
        assert_ne!(one, scoped_rule_key("runner-b", Path::new("/work/a"), &args));
        assert_ne!(one, scoped_rule_key("runner-a", Path::new("/work/b"), &args));
        assert_ne!(one, scoped_rule_key("runner-a", Path::new("/work/a"), &serde_json::json!({ "command": "git reset --hard" })));
    }

    #[test]
    fn always_allow_uses_reusable_command_prefixes() {
        assert_eq!(
            shell_patterns("git status --short && npm run test -- --watch && ls"),
            vec!["git status *", "npm run test *", "ls *"]
        );
        assert!(shell_pattern_matches("git status", "git status *"));
        assert!(shell_pattern_matches("git status --short", "git status *"));
        assert!(!shell_pattern_matches("git push", "git status *"));
        assert!(reusable_shell_patterns("rm -rf report").is_empty());

        let rule = AutoReviewRule {
            id: "patterns".into(),
            text: "approved from a shell prompt".into(),
            behavior: "allow".into(),
            tool: None,
            runner_id: Some("runner".into()),
            workdir: Some("/work".into()),
            command: None,
            patterns: vec!["git status *".into(), "rg *".into()],
        };
        assert_eq!(
            shell_rule_outcome(&[rule.clone()], "runner", Path::new("/work"), "git status --short && rg TODO src"),
            Some(Outcome::Allow)
        );
        assert_eq!(shell_rule_outcome(&[rule], "runner", Path::new("/work"), "git status && rm report"), None);
    }

    #[tokio::test]
    async fn unattended_shell_fails_closed_without_an_automatic_decision_and_exact_allow_runs() {
        use lorca_agent::{AgentContext, AssistantMessage, ToolCall};
        use tokio_util::sync::CancellationToken;

        let home = std::env::temp_dir().join(format!("lorca-review-hook-{}", uuid::Uuid::new_v4()));
        let work = home.join("workspaces/bot");
        std::fs::create_dir_all(&work).unwrap();
        let app = App::load(crate::config::Config { home: home.clone(), port: 0 }).unwrap();
        let bot = Bot {
            id: "bot".into(), name: "Bot".into(), description: String::new(), symbol_name: String::new(), accent: String::new(), avatar: None,
            runner_id: "runner".into(), provider: "deepseek".into(), model: None, thinking: None, legacy_instructions: String::new(), workdir: Some(work.display().to_string()), created_at: 0.0,
        };
        let assistant = AssistantMessage::empty("test", "test");
        let context = AgentContext { system_prompt: String::new(), messages: Vec::new(), tools: Vec::new() };
        let cancel = CancellationToken::new();
        let args = serde_json::json!({ "command": "cargo test" });
        let call = ToolCall { id: "1".into(), name: "bash".into(), arguments: args.clone() };
        let ctx = BeforeToolCallContext { assistant_message: &assistant, tool_call: &call, args: &args, context: &context, cancel: &cancel };
        let blocked = before_tool_call(&app, "chat", &bot, &work, true, ctx).await.unwrap();
        assert!(blocked.block);

        let canonical = std::fs::canonicalize(&work).unwrap();
        app.add_auto_review_rule(AutoReviewRule {
            id: "allow".into(), text: "cargo test in this workspace".into(), behavior: "allow".into(),
            tool: Some(scoped_rule_key("runner", &canonical, &args)),
            runner_id: Some("runner".into()),
            workdir: Some(canonical.display().to_string()),
            command: Some("cargo test".into()),
            patterns: Vec::new(),
        });
        let ctx = BeforeToolCallContext { assistant_message: &assistant, tool_call: &call, args: &args, context: &context, cancel: &cancel };
        assert!(before_tool_call(&app, "chat", &bot, &work, true, ctx).await.is_none());

        app.add_auto_review_rule(AutoReviewRule {
            id: "pattern".into(),
            text: "cargo test *".into(),
            behavior: "allow".into(),
            tool: Some("computer/bash/pattern".into()),
            runner_id: Some("runner".into()),
            workdir: Some(canonical.display().to_string()),
            command: Some("cargo test".into()),
            patterns: vec!["cargo test *".into()],
        });
        let varied_args = serde_json::json!({ "command": "cargo test --workspace" });
        let varied_call = ToolCall { id: "2".into(), name: "bash".into(), arguments: varied_args.clone() };
        let ctx = BeforeToolCallContext {
            assistant_message: &assistant,
            tool_call: &varied_call,
            args: &varied_args,
            context: &context,
            cancel: &cancel,
        };
        assert!(before_tool_call(&app, "chat", &bot, &work, true, ctx).await.is_none());

        let mut review = app.auto_review();
        review.is_enabled = false;
        app.set_auto_review(review);
        let ctx = BeforeToolCallContext { assistant_message: &assistant, tool_call: &call, args: &args, context: &context, cancel: &cancel };
        assert!(before_tool_call(&app, "chat", &bot, &work, true, ctx).await.unwrap().block);
        let _ = std::fs::remove_dir_all(home);
    }
}
