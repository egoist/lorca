//! Cloud sandboxes a Runner's coding agents run in instead of on the Runner itself: Vercel
//! Sandbox and Cloudflare Sandbox, each set up by the user with an account of their own. Lorca
//! has no account with either: the user gives the Runner their Vercel token and project, or the
//! address and token of the Cloudflare Worker they deployed from `sandboxes/cloudflare`, and the
//! environment variables the agents need there (a GitHub token to push and open pull requests, a
//! model provider's key). They stay in `sandboxes.json` in the Runner's data directory (0600),
//! and every value is replaced by its placeholder wherever the Runner's saved secrets are.
//!
//! An agent started in one gets a sandbox of its own, named after it (`lorca-agent-3f9a2c1d`),
//! with the repository of the folder the bot named cloned into it from its remote, on the branch
//! and commit the Runner has checked out when the remote has them. The agent's process runs there
//! and is spoken to over the provider's API as a local one is over its pipes ([`attach`]), so
//! every driver, review, and transcript works as it does on the Runner. When its process ends the
//! sandbox pauses (Vercel keeps its files in a snapshot; a Cloudflare container sleeps once idle),
//! and a later message resumes it there. Forgetting the agent deletes its sandbox.

pub(crate) mod cloudflare;
#[cfg(all(test, unix))]
pub(crate) mod here;
pub(crate) mod vercel;

use std::collections::BTreeMap;
use std::path::Path;
use std::sync::Arc;

use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::sync::{mpsc, oneshot};
use tokio_util::sync::CancellationToken;

use super::process::{Exit, Process, Stdout, Variable};
use super::workspace;
use crate::app::App;
use crate::model::AgentChanges;

/// The sandboxes a Runner can be set up with.
pub const KINDS: [&str; 2] = ["vercel", "cloudflare"];

const FILE: &str = "sandboxes.json";
/// Variables a sandbox's environment may not set: what its shell runs by.
const RESERVED: &[&str] = &["PATH", "HOME", "SHELL", "USER", "LOGNAME", "PWD", "TERM", "LANG"];
/// A value shorter than this is not replaced in what an agent prints: it could be any word.
const MIN_REDACTED_CHARS: usize = 4;

/// A sandbox's product name, which is not translated.
pub fn name(kind: &str) -> &'static str {
    match kind {
        "cloudflare" => "Cloudflare Sandbox",
        _ => "Vercel Sandbox",
    }
}

/// How this Runner's sandboxes are set up, in `sandboxes.json`.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Settings {
    /// Where a start that names no place runs: empty for this Runner, else a sandbox.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub default: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub vercel: Option<vercel::Settings>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cloudflare: Option<cloudflare::Settings>,
}

impl Settings {
    fn is_set_up(&self, kind: &str) -> bool {
        match kind {
            "vercel" => self.vercel.is_some(),
            "cloudflare" => self.cloudflare.is_some(),
            _ => false,
        }
    }

    /// The names of the variables the user gave sandbox `kind`.
    pub fn env_names(&self, kind: &str) -> Vec<String> {
        self.env(kind).into_keys().collect()
    }

    /// The environment the user gave sandbox `kind`.
    fn env(&self, kind: &str) -> BTreeMap<String, String> {
        match kind {
            "vercel" => self.vercel.as_ref().map(|settings| settings.env.clone()),
            "cloudflare" => self.cloudflare.as_ref().map(|settings| settings.env.clone()),
            _ => None,
        }
        .unwrap_or_default()
    }
}

fn file(app: &App) -> std::path::PathBuf {
    app.config.home.join(FILE)
}

/// This Runner's sandboxes, none until the user sets one up.
pub fn of(app: &App) -> Settings {
    read(&app.config.home)
}

fn read(home: &Path) -> Settings {
    std::fs::read(home.join(FILE)).ok().and_then(|bytes| serde_json::from_slice(&bytes).ok()).unwrap_or_default()
}

fn save(app: &App, settings: &Settings) -> Result<(), String> {
    crate::config::write_json_private(&file(app), settings).map_err(|error| format!("Could not save the sandboxes: {error}"))
}

/// Every value the user gave the sandboxes, with the placeholder that replaces it in what an
/// agent prints, shows, or is sent: `{{sandbox:GITHUB_TOKEN}}`.
pub fn redactions(app: &App) -> Vec<(String, String)> {
    let settings = of(app);
    let mut out = Vec::new();
    let mut add = |value: &str, placeholder: String| {
        if value.chars().count() >= MIN_REDACTED_CHARS {
            out.push((value.to_string(), placeholder));
        }
    };
    if let Some(vercel) = &settings.vercel {
        add(&vercel.token, "{{sandbox:vercel-token}}".into());
    }
    if let Some(cloudflare) = &settings.cloudflare {
        add(&cloudflare.token, "{{sandbox:cloudflare-token}}".into());
    }
    for kind in KINDS {
        for (name, value) in settings.env(kind) {
            add(&value, format!("{{{{sandbox:{name}}}}}"));
        }
    }
    out
}

/// Where a start runs: the sandbox the bot names (`local` for this Runner), else the one the user
/// made the default; None for this Runner.
pub(crate) fn pick(app: &App, asked: Option<&str>) -> Result<Option<String>, String> {
    let settings = of(app);
    let kind = match asked.map(str::trim).filter(|kind| !kind.is_empty()) {
        Some("local") => return Ok(None),
        Some(kind) if !KINDS.contains(&kind) => return Err("sandbox is local, vercel, or cloudflare".into()),
        Some(kind) => kind.to_string(),
        None if settings.default.is_empty() => return Ok(None),
        None => settings.default.clone(),
    };
    if !settings.is_set_up(&kind) {
        return Err(format!("{} is not set up on this Runner. Leave sandbox out, or ask the user to set it up in Settings → Coding Agents.", name(&kind)));
    }
    Ok(Some(kind))
}

/// Where an agent in a sandbox works, as its record keeps it.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub(crate) struct Place {
    /// `vercel` or `cloudflare`.
    pub kind: String,
    /// The sandbox's name with its provider: `lorca-agent-3f9a2c1d`.
    pub name: String,
    /// The repository cloned into it, as the user reads it: `github.com/egoist/lorca`.
    pub repository: String,
}

impl Place {
    /// Where it works, in a few words: "Vercel Sandbox · github.com/egoist/lorca".
    pub fn words(&self) -> String {
        format!("{} · {}", name(&self.kind), self.repository)
    }
}

/// The repository a sandbox clones, from the folder the bot named on this Runner.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct Source {
    /// What git clones over https, without credentials: the sandbox's environment holds those.
    pub url: String,
    /// `github.com/egoist/lorca`.
    pub shown: String,
    /// The folder it goes in, its last part: `lorca`.
    pub folder: String,
    pub branch: Option<String>,
    /// The commit the Runner has checked out.
    pub base: Option<String>,
    /// Who the user's commits are by, as git on the Runner has them.
    pub author: (String, String),
}

/// The repository the folder at `local` is a clone of, from the remote its branch tracks or
/// `origin`.
pub(crate) async fn source(local: &workspace::Place) -> Result<Source, String> {
    let folder = super::home_relative(&local.folder);
    if local.base.is_none() {
        return Err(format!("A sandbox works on a clone of a git repository, and {folder} is not one. Run it on this Runner instead (sandbox: \"local\")."));
    }
    let remote = match &local.branch {
        Some(branch) => workspace::git(&local.folder, &["config", &format!("branch.{branch}.remote")]).await.ok().filter(|remote| !remote.is_empty() && remote != "."),
        None => None,
    }
    .unwrap_or_else(|| "origin".into());
    let url = workspace::git(&local.folder, &["remote", "get-url", &remote])
        .await
        .map_err(|_| format!("A sandbox clones the repository from its remote, and {folder} has none. Run it on this Runner instead (sandbox: \"local\")."))?;
    let url = https_url(&url).ok_or_else(|| format!("A sandbox clones the repository over https, and {folder}'s remote is {url}. Run it on this Runner instead (sandbox: \"local\")."))?;
    let shown = url.trim_start_matches("https://").trim_start_matches("http://").trim_end_matches(".git").to_string();
    let name = shown.rsplit('/').next().filter(|name| !name.is_empty()).unwrap_or("repository").to_string();
    let author = (
        workspace::git(&local.folder, &["config", "user.name"]).await.unwrap_or_default(),
        workspace::git(&local.folder, &["config", "user.email"]).await.unwrap_or_default(),
    );
    Ok(Source { url, shown, folder: name, branch: local.branch.clone(), base: local.base.clone(), author })
}

/// A remote's address as git clones it over https, without the credentials it may hold:
/// `git@github.com:egoist/lorca.git` and `ssh://git@github.com/egoist/lorca` are
/// `https://github.com/egoist/lorca.git` and `https://github.com/egoist/lorca`. None for a remote
/// that is a path.
pub(crate) fn https_url(remote: &str) -> Option<String> {
    let remote = remote.trim();
    let (host, path) = if let Some(rest) = remote.strip_prefix("https://").or_else(|| remote.strip_prefix("http://")).or_else(|| remote.strip_prefix("ssh://")).or_else(|| remote.strip_prefix("git://")) {
        let (authority, path) = rest.split_once('/')?;
        let host = authority.rsplit('@').next()?;
        // An ssh port is not the https one.
        let host = if remote.starts_with("ssh://") { host.split(':').next()? } else { host };
        (host.to_string(), path.to_string())
    } else {
        // scp-like: [user@]host:path
        let (authority, path) = remote.split_once(':')?;
        if authority.contains('/') || authority.len() < 2 {
            return None;
        }
        (authority.rsplit('@').next()?.to_string(), path.trim_start_matches('/').to_string())
    };
    (!host.is_empty() && !path.is_empty()).then(|| format!("https://{host}/{path}"))
}

/// One sandbox, open, as an agent runs in it.
pub(crate) struct Sandbox {
    /// The environment the user gave it, for every command.
    env: Vec<Variable>,
    inner: Inner,
}

enum Inner {
    Vercel(vercel::Session),
    Cloudflare(cloudflare::Client),
    /// A folder on this computer that stands in for one, in the tests.
    #[cfg(all(test, unix))]
    Here(here::Shell),
}

/// What a command printed, and how it ended.
#[derive(Debug, Default)]
pub(crate) struct Output {
    pub code: Option<i32>,
    pub stdout: String,
    pub stderr: String,
}

impl Output {
    /// What it printed when it worked, else why not.
    fn ok(self) -> Result<String, String> {
        match self.code {
            Some(0) => Ok(self.stdout),
            _ => Err(self.stderr.lines().map(str::trim).rfind(|line| !line.is_empty()).unwrap_or("it failed").chars().take(300).collect()),
        }
    }
}

/// A path in a sandbox as its shell writes it.
fn unix(path: &Path) -> String {
    path.to_string_lossy().replace('\\', "/")
}

impl Sandbox {
    /// Opens the sandbox `place` names: resumes it, or makes it when it does not exist yet.
    pub async fn open(app: &App, place: &Place) -> Result<Arc<Sandbox>, String> {
        let settings = of(app);
        let env = settings.env(&place.kind).into_iter().map(|(name, value)| (name.into(), value.into())).collect();
        let inner = match place.kind.as_str() {
            "vercel" => Inner::Vercel(vercel::open(settings.vercel.as_ref().ok_or("Vercel Sandbox is no longer set up on this Runner.")?, &place.name).await?),
            "cloudflare" => Inner::Cloudflare(cloudflare::open(settings.cloudflare.as_ref().ok_or("Cloudflare Sandbox is no longer set up on this Runner.")?, &place.name).await?),
            #[cfg(all(test, unix))]
            "here" => Inner::Here(here::Shell::open(app, &place.name)),
            other => return Err(format!("Unknown sandbox {other}")),
        };
        Ok(Arc::new(Sandbox { env, inner }))
    }

    /// The folder its commands start in, which Lorca clones into.
    pub fn root(&self) -> String {
        match &self.inner {
            Inner::Vercel(_) => vercel::ROOT.into(),
            Inner::Cloudflare(_) => cloudflare::ROOT.into(),
            #[cfg(all(test, unix))]
            Inner::Here(shell) => shell.root(),
        }
    }

    /// The sandbox's environment, then `variables`, by name.
    fn environment(&self, variables: &[Variable]) -> BTreeMap<String, String> {
        self.env.iter().chain(variables).map(|(name, value)| (name.to_string_lossy().to_string(), value.to_string_lossy().to_string())).collect()
    }

    /// Starts `program` in `cwd` there, on the sandbox's own `PATH`, in a process group of its own
    /// that a stop ends as one. No login shell: a profile could change `PATH` or print into the
    /// agent's protocol. Once it ended, the sandbox pauses.
    pub async fn spawn(self: &Arc<Self>, program: &Path, args: &[String], cwd: &Path, variables: &[Variable]) -> Result<(Arc<Process>, Stdout, oneshot::Receiver<Exit>), String> {
        let mut argv: Vec<String> = vec!["sh".into(), "-c".into(), GROUP.into(), unix(program)];
        argv.extend(args.iter().cloned());
        let env = self.environment(variables);
        let cwd = unix(cwd);
        let (process, stdout, exit) = match &self.inner {
            Inner::Vercel(session) => session.spawn(&argv, &cwd, &env).await?,
            Inner::Cloudflare(client) => client.spawn(&argv, &cwd, &env).await?,
            #[cfg(all(test, unix))]
            Inner::Here(shell) => shell.spawn(&argv, &cwd, &env)?,
        };
        // It pauses before its end is told, so a message that resumes it right away finds it
        // paused, not about to be.
        let (tx, rx) = oneshot::channel();
        let sandbox = self.clone();
        tokio::spawn(async move {
            let ended = exit.await;
            sandbox.pause().await;
            if let Ok(ended) = ended {
                let _ = tx.send(ended);
            }
        });
        Ok((process, stdout, rx))
    }

    /// Runs `script` with `sh -c`, its arguments `args`, and waits for it.
    pub async fn run(&self, script: &str, args: &[&str], cwd: Option<&str>) -> Result<Output, String> {
        let mut argv: Vec<String> = vec!["sh".into(), "-c".into(), script.into(), "lorca".into()];
        argv.extend(args.iter().map(|arg| arg.to_string()));
        let env = self.environment(&[]);
        let cwd = cwd.map(str::to_string).unwrap_or_else(|| self.root());
        match &self.inner {
            Inner::Vercel(session) => session.run(&argv, &cwd, &env).await,
            Inner::Cloudflare(client) => client.run(&argv, &cwd, &env).await,
            #[cfg(all(test, unix))]
            Inner::Here(shell) => shell.run(&argv, &cwd, &env).await,
        }
    }

    /// Writes `contents` to the file at `path` there.
    pub async fn write(&self, path: &str, contents: &str) -> Result<(), String> {
        use base64::Engine;
        let encoded = base64::engine::general_purpose::STANDARD.encode(contents);
        self.run(r#"mkdir -p "$(dirname "$2")" && printf '%s' "$1" | base64 -d > "$2""#, &[&encoded, path], None).await?.ok().map(|_| ())
    }

    /// Whether `program` is installed there.
    pub async fn has(&self, program: &str) -> bool {
        self.run(r#"command -v "$1" >/dev/null 2>&1"#, &[program], None).await.is_ok_and(|output| output.code == Some(0))
    }

    /// Whether the folder at `folder` is there: a paused sandbox keeps it, one that was gone
    /// and made again does not.
    pub async fn has_folder(&self, folder: &Path) -> bool {
        self.run(r#"test -d "$1""#, &[&unix(folder)], None).await.is_ok_and(|output| output.code == Some(0))
    }

    /// Clones `source` into `folder` there, unless it is there already, on its branch and at the
    /// commit the Runner has checked out when the remote has it, with git signed in to GitHub
    /// from the sandbox's `GH_TOKEN` or `GITHUB_TOKEN` and the user's name on commits. Answers the
    /// branch and commit it is on.
    pub async fn clone_repository(&self, source: &Source, folder: &Path) -> Result<(Option<String>, Option<String>), String> {
        let folder = unix(folder);
        let branch = source.branch.as_deref().unwrap_or("");
        let base = source.base.as_deref().unwrap_or("");
        let printed = self.run(CLONE, &[&source.url, &folder, branch, base, &source.author.0, &source.author.1], None).await?.ok().map_err(|why| format!("Could not clone {} into the sandbox: {why}", source.shown))?;
        let mut lines = printed.lines().map(str::trim);
        let branch = lines.next().filter(|line| !line.is_empty()).map(str::to_string);
        let head = lines.next().filter(|line| !line.is_empty()).map(str::to_string);
        Ok((branch, head))
    }

    /// The branch checked out in `folder` there.
    pub async fn branch(&self, folder: &Path) -> Option<String> {
        let folder = unix(folder);
        self.run("git branch --show-current", &[], Some(&folder)).await.ok()?.ok().ok().map(|text| text.trim().to_string()).filter(|branch| !branch.is_empty())
    }

    /// What the agent changed in `folder` there since `base`, as [`workspace::changes`] counts it
    /// on the Runner.
    pub async fn changes(&self, folder: &Path, base: &str) -> Option<AgentChanges> {
        let folder = unix(folder);
        let printed = self.run(CHANGES, &[base], Some(&folder)).await.ok()?.ok().ok()?;
        let (numstat, untracked) = printed.split_once(UNTRACKED).unwrap_or((&printed, ""));
        let mut changes = workspace::numstat(numstat);
        for line in untracked.lines().filter(|line| !line.trim().is_empty()) {
            changes.files += 1;
            changes.added += line.split('\t').next().and_then(|lines| lines.trim().parse::<u32>().ok()).unwrap_or(0);
        }
        Some(changes)
    }

    /// Its process ended: the sandbox pauses until a later message resumes it.
    async fn pause(&self) {
        match &self.inner {
            Inner::Vercel(session) => session.pause().await,
            Inner::Cloudflare(_) => {}
            #[cfg(all(test, unix))]
            Inner::Here(shell) => shell.pause(),
        }
    }
}

/// Runs `"$0" "$@"` in a process group of its own (`setsid`, where the sandbox has it), so a stop's
/// signal reaches everything it started, and exits as it did. A sandbox has no terminal, so the
/// shell has no job control and gives a job started with `&` `/dev/null` for its stdin before any
/// redirection of its own: stdin waits on descriptor 3 and goes back from there.
const GROUP: &str = r#"exec 3<&0; if command -v setsid >/dev/null 2>&1; then setsid "$0" "$@" <&3 3<&- & else "$0" "$@" <&3 3<&- & fi; exec 3<&-; child=$!; trap 'kill -TERM -"$child" 2>/dev/null || kill -TERM "$child" 2>/dev/null' TERM INT HUP; wait "$child"; status=$?; wait "$child" 2>/dev/null; exit $status"#;

/// Clones a repository into a sandbox: `url folder branch base name email`.
const CLONE: &str = r#"set -e
url=$1 folder=$2 branch=$3 base=$4 name=$5 email=$6
git config --global credential.https://github.com.helper '!f() { test "$1" = get || exit 0; t="${GH_TOKEN:-$GITHUB_TOKEN}"; test -n "$t" || exit 0; echo username=x-access-token; echo "password=$t"; }; f'
if [ -n "$name" ]; then git config --global user.name "$name"; fi
if [ -n "$email" ]; then git config --global user.email "$email"; fi
if [ ! -d "$folder/.git" ]; then
  mkdir -p "$(dirname "$folder")"
  if [ -z "$branch" ] || ! git clone -q --branch "$branch" -- "$url" "$folder" 2>/dev/null; then
    git clone -q -- "$url" "$folder"
  fi
  cd "$folder"
  if [ -n "$base" ] && git cat-file -e "$base^{commit}" 2>/dev/null; then
    if [ -n "$branch" ]; then git checkout -q -B "$branch" "$base"; else git checkout -q --detach "$base"; fi
  fi
fi
cd "$folder"
git branch --show-current
git rev-parse HEAD
"#;

const UNTRACKED: &str = "--lorca-untracked--";

/// What changed since `$1`: git's numstat, then each new file's lines (none for one over 1 MiB),
/// at most 200 of them.
const CHANGES: &str = r#"git -c core.quotepath=off diff --numstat "$1" --
echo "--lorca-untracked--"
git ls-files --others --exclude-standard | head -n 200 | while IFS= read -r file; do
  if [ -f "$file" ] && [ "$(wc -c < "$file")" -le 1048576 ]; then printf '%s\t%s\n' "$(wc -l < "$file")" "$file"; else printf '0\t%s\n' "$file"; fi
done
"#;

/// Deletes the sandbox `place` names, with its files: its agent is forgotten.
pub(crate) async fn discard(config: &crate::config::Config, place: &Place) {
    let settings = read(&config.home);
    let result = match place.kind.as_str() {
        "vercel" => match &settings.vercel {
            Some(vercel) => vercel::delete(vercel, &place.name).await,
            None => Ok(()),
        },
        "cloudflare" => match &settings.cloudflare {
            Some(cloudflare) => cloudflare::delete(cloudflare, &place.name).await,
            None => Ok(()),
        },
        _ => Ok(()),
    };
    if let Err(error) = result {
        tracing::warn!(%error, sandbox = %place.name, "deleting a coding agent's sandbox");
    }
}

/// What a process in a sandbox sends back.
pub(crate) enum Chunk {
    Stdout(Vec<u8>),
    Stderr(Vec<u8>),
    /// It ended, with its exit code when the sandbox said.
    Exit(Option<i32>),
}

/// A process running in a sandbox: what takes its stdin, and what stops it.
#[async_trait]
pub(crate) trait Remote: Send + Sync + 'static {
    async fn write(&self, bytes: Vec<u8>) -> Result<(), String>;
    async fn kill(&self);
}

/// How long a stopped process has to say it ended.
const KILL_GRACE: std::time::Duration = std::time::Duration::from_secs(10);

/// A process in a sandbox as a [`Process`] on the Runner is: its stdin goes to `remote` in order,
/// what it prints comes from `chunks`, and how it ended arrives once it has, `name` (the
/// sandbox's) saying so when the connection to it was lost. Stopping it signals `remote`.
pub(crate) fn attach(remote: Arc<dyn Remote>, mut chunks: mpsc::UnboundedReceiver<Chunk>, name: String) -> (Arc<Process>, Stdout, oneshot::Receiver<Exit>) {
    let (stdin, mut stdin_reader) = tokio::io::duplex(64 * 1024);
    let (mut stdout_writer, stdout) = tokio::io::duplex(256 * 1024);
    let kill = CancellationToken::new();
    let (tx, rx) = oneshot::channel();
    // What Lorca writes goes to the sandbox in the order it was written.
    {
        let (remote, kill) = (remote.clone(), kill.clone());
        tokio::spawn(async move {
            let mut buffer = vec![0u8; 64 * 1024];
            loop {
                let read = tokio::select! {
                    read = stdin_reader.read(&mut buffer) => read,
                    _ = kill.cancelled() => return,
                };
                match read {
                    Ok(0) | Err(_) => return,
                    Ok(read) => {
                        if let Err(error) = remote.write(buffer[..read].to_vec()).await {
                            tracing::debug!(%error, "writing to a sandboxed coding agent");
                            return;
                        }
                    }
                }
            }
        });
    }
    let token = kill.clone();
    tokio::spawn(async move {
        let mut tail = String::new();
        let mut stopped = false;
        let mut code = None;
        let mut lost = true;
        loop {
            let chunk = tokio::select! {
                chunk = chunks.recv() => chunk,
                _ = token.cancelled(), if !stopped => {
                    stopped = true;
                    remote.kill().await;
                    continue;
                }
                _ = tokio::time::sleep(KILL_GRACE), if stopped => None,
            };
            match chunk {
                Some(Chunk::Stdout(bytes)) => {
                    if stdout_writer.write_all(&bytes).await.is_err() {
                        token.cancel();
                    }
                }
                Some(Chunk::Stderr(bytes)) => {
                    tail.push_str(&String::from_utf8_lossy(&bytes));
                    if tail.len() > 16_384 {
                        let mut cut = tail.len() - 8192;
                        while !tail.is_char_boundary(cut) {
                            cut += 1;
                        }
                        tail = tail.split_off(cut);
                    }
                }
                Some(Chunk::Exit(exit)) => {
                    code = exit;
                    lost = false;
                    break;
                }
                None => break,
            }
        }
        drop(stdout_writer);
        let said = tail.lines().map(str::trim).rfind(|line| !line.is_empty()).map(|line| line.chars().take(300).collect::<String>());
        let said = match (lost && !stopped, said) {
            (true, said) => Some(said.map(|said| format!("{name} lost the connection to it: {said}")).unwrap_or_else(|| format!("{name} lost the connection to it"))),
            (false, said) => said,
        };
        let _ = tx.send(Exit { code, said, stopped });
    });
    (Process::new(stdin, kill), Box::new(stdout), rx)
}

/// `coding.sandboxes`: this Runner's sandboxes as the apps show them, with each token as its
/// last characters and each variable by name; `coding.sandboxes.set` changes them and answers the
/// same; `coding.sandboxes.check { kind }` asks the provider whether what the user gave works.
pub async fn serve(app: &App, verb: &str, body: &Value) -> Result<Value, String> {
    match verb {
        "coding.sandboxes.set" => {
            let mut settings = of(app);
            apply(&mut settings, body)?;
            save(app, &settings)?;
        }
        "coding.sandboxes.check" => {
            let settings = of(app);
            let kind = body["kind"].as_str().unwrap_or("");
            let checked = match kind {
                "vercel" => vercel::check(settings.vercel.as_ref().ok_or("Vercel Sandbox is not set up")?).await,
                "cloudflare" => cloudflare::check(settings.cloudflare.as_ref().ok_or("Cloudflare Sandbox is not set up")?).await,
                _ => return Err("kind is vercel or cloudflare".into()),
            };
            return Ok(match checked {
                Ok(()) => json!({ "ok": true }),
                Err(error) => json!({ "ok": false, "error": error }),
            });
        }
        _ => {}
    }
    Ok(shown(&of(app)))
}

/// The sandboxes as every app shows them: no token or value, only a token's last characters.
fn shown(settings: &Settings) -> Value {
    let hint = |token: &str| -> Value {
        let count = token.chars().count();
        if count == 0 {
            return Value::Null;
        }
        json!(format!("…{}", token.chars().skip(count.saturating_sub(4)).collect::<String>()))
    };
    let names = |env: &BTreeMap<String, String>| env.keys().cloned().collect::<Vec<_>>();
    let sandboxes: Vec<Value> = KINDS
        .iter()
        .map(|kind| {
            let (fields, token, env) = match *kind {
                "vercel" => match &settings.vercel {
                    Some(vercel) => (vercel.fields(), hint(&vercel.token), names(&vercel.env)),
                    None => (Value::Null, Value::Null, Vec::new()),
                },
                _ => match &settings.cloudflare {
                    Some(cloudflare) => (cloudflare.fields(), hint(&cloudflare.token), names(&cloudflare.env)),
                    None => (Value::Null, Value::Null, Vec::new()),
                },
            };
            json!({ "kind": kind, "name": name(kind), "set_up": settings.is_set_up(kind), "settings": fields, "token": token, "env": env })
        })
        .collect();
    json!({ "default": settings.default, "sandboxes": sandboxes })
}

/// A change to the sandboxes: `vercel` or `cloudflare` as an object sets that sandbox up or
/// changes it (a token or a variable's value left out keeps the saved one; variables it does not
/// list go), `null` removes it; `default` is where a start that names no place runs.
fn apply(settings: &mut Settings, body: &Value) -> Result<(), String> {
    match body.get("vercel") {
        None => {}
        Some(Value::Null) => settings.vercel = None,
        Some(change) => settings.vercel = Some(vercel::Settings::merged(settings.vercel.take(), change)?),
    }
    match body.get("cloudflare") {
        None => {}
        Some(Value::Null) => settings.cloudflare = None,
        Some(change) => settings.cloudflare = Some(cloudflare::Settings::merged(settings.cloudflare.take(), change)?),
    }
    if let Some(default) = body.get("default") {
        let default = default.as_str().unwrap_or("").trim();
        settings.default = match default {
            "" | "local" => String::new(),
            kind if !KINDS.contains(&kind) => return Err("default is local, vercel, or cloudflare".into()),
            kind if !settings.is_set_up(kind) => return Err(format!("Set up {} before running agents there by default", name(kind))),
            kind => kind.to_string(),
        };
    }
    if !settings.default.is_empty() && !settings.is_set_up(&settings.default.clone()) {
        settings.default.clear();
    }
    Ok(())
}

/// `env: [{ name, value? }]` over the saved variables: a variable without a value keeps the
/// saved one, and a saved one the list leaves out goes.
pub(crate) fn merged_env(saved: &BTreeMap<String, String>, change: &Value) -> Result<BTreeMap<String, String>, String> {
    let Some(entries) = change.get("env") else { return Ok(saved.clone()) };
    let entries = entries.as_array().ok_or("env is a list of { name, value }")?;
    let mut env = BTreeMap::new();
    for entry in entries {
        let name = entry["name"].as_str().map(str::trim).filter(|name| !name.is_empty()).ok_or("Each variable needs a name")?;
        let valid = name.chars().next().is_some_and(|first| first.is_ascii_alphabetic() || first == '_') && name.chars().all(|c| c.is_ascii_alphanumeric() || c == '_');
        if !valid {
            return Err(format!("{name} is not a variable name: letters, digits, and _, not starting with a digit"));
        }
        if RESERVED.contains(&name) {
            return Err(format!("{name} is the sandbox's own"));
        }
        let value = match entry["value"].as_str().filter(|value| !value.is_empty()) {
            Some(value) => value.to_string(),
            None => saved.get(name).cloned().ok_or_else(|| format!("{name} needs a value"))?,
        };
        env.insert(name.to_string(), value);
    }
    Ok(env)
}

/// A token in a change, else the saved one.
pub(crate) fn merged_token(saved: Option<&str>, change: &Value) -> Result<String, String> {
    match change["token"].as_str().map(str::trim).filter(|token| !token.is_empty()) {
        Some(token) => Ok(token.to_string()),
        None => saved.filter(|token| !token.is_empty()).map(str::to_string).ok_or_else(|| "A token is needed".into()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_remote_clones_over_https_without_its_credentials() {
        assert_eq!(https_url("git@github.com:egoist/lorca.git").as_deref(), Some("https://github.com/egoist/lorca.git"));
        assert_eq!(https_url("ssh://git@github.com:22/egoist/lorca").as_deref(), Some("https://github.com/egoist/lorca"));
        assert_eq!(https_url("https://x-access-token:abc@github.com/egoist/lorca.git").as_deref(), Some("https://github.com/egoist/lorca.git"));
        assert_eq!(https_url("https://gitlab.com/acme/shop").as_deref(), Some("https://gitlab.com/acme/shop"));
        assert_eq!(https_url("/srv/git/shop.git"), None);
        assert_eq!(https_url("C:/repos/shop"), None);
        assert_eq!(https_url("../shop"), None);
    }

    #[test]
    fn a_change_keeps_what_it_leaves_out_and_drops_what_it_does_not_list() {
        let mut settings = Settings::default();
        let first = json!({ "vercel": { "token": "vercel_secret_token", "project_id": "prj_1", "env": [{ "name": "GITHUB_TOKEN", "value": "ghp_one" }, { "name": "ANTHROPIC_API_KEY", "value": "sk-ant" }] }, "default": "vercel" });
        apply(&mut settings, &first).unwrap();
        assert_eq!(settings.default, "vercel");
        // The token and a variable's value left out stay; a variable not listed goes.
        apply(&mut settings, &json!({ "vercel": { "project_id": "prj_2", "env": [{ "name": "GITHUB_TOKEN" }] } })).unwrap();
        let vercel = settings.vercel.clone().unwrap();
        assert_eq!((vercel.token.as_str(), vercel.project_id.as_str()), ("vercel_secret_token", "prj_2"));
        assert_eq!(vercel.env.into_iter().collect::<Vec<_>>(), vec![("GITHUB_TOKEN".to_string(), "ghp_one".to_string())]);
        // What the apps see holds no token or value.
        let shown = shown(&settings).to_string();
        assert!(!shown.contains("vercel_secret_token") && !shown.contains("ghp_one"), "{shown}");
        assert!(shown.contains("…oken") && shown.contains("GITHUB_TOKEN"), "{shown}");
        // A sandbox that is not set up is no default, and removing the default one moves it back here.
        assert!(apply(&mut settings, &json!({ "default": "cloudflare" })).is_err());
        assert!(apply(&mut settings, &json!({ "cloudflare": { "url": "https://lorca-sandbox.example.workers.dev", "token": "t0ken", "env": [{ "name": "PATH", "value": "/bin" }] } })).is_err());
        apply(&mut settings, &json!({ "vercel": null })).unwrap();
        assert_eq!((settings.vercel.is_none(), settings.default.as_str()), (true, ""));
    }

    #[test]
    fn a_start_runs_where_the_bot_says_or_where_the_user_chose() {
        let home = std::env::temp_dir().join(format!("lorca-sandboxes-{}", uuid::Uuid::new_v4()));
        let app = crate::app::App::load(crate::config::Config { home: home.clone(), port: 0 }).unwrap();
        assert_eq!(pick(&app, None).unwrap(), None);
        assert!(pick(&app, Some("vercel")).is_err());
        let mut settings = Settings::default();
        apply(&mut settings, &json!({ "cloudflare": { "url": "https://lorca-sandbox.example.workers.dev", "token": "t0ken" }, "default": "cloudflare" })).unwrap();
        save(&app, &settings).unwrap();
        assert_eq!(pick(&app, None).unwrap().as_deref(), Some("cloudflare"));
        assert_eq!(pick(&app, Some("local")).unwrap(), None);
        assert!(pick(&app, Some("vercel")).is_err());
        assert!(redactions(&app).iter().any(|(value, placeholder)| value == "t0ken" && placeholder == "{{sandbox:cloudflare-token}}"));
        let _ = std::fs::remove_dir_all(&home);
    }
}
