//! The environment of the user's login shell. A CLI the app starts inherits launchd's bare
//! environment, where `npx`, `bun`, or `cargo` are not on `PATH`, and the shell that sets them up
//! may be fish, whose files bash never reads. So the account's shell runs once as an interactive
//! login shell and prints its environment, and every command a bot runs and every stdio plugin
//! server starts with it, as from the user's terminal. Windows keeps the environment the CLI
//! started with.

use std::ffi::{OsStr, OsString};
#[cfg(unix)]
use std::path::{Path, PathBuf};
#[cfg(unix)]
use std::time::Duration;

use tokio::process::Command;
use tokio::sync::OnceCell;

type Environment = Vec<(OsString, OsString)>;

static ENVIRONMENT: OnceCell<Environment> = OnceCell::const_new();

/// How long the login shell has to print its environment, all attempts together.
#[cfg(unix)]
const TIMEOUT: Duration = Duration::from_secs(5);
/// An interactive attempt gets part of that, so a plain login shell can still answer when an rc
/// file blocks or exits.
#[cfg(unix)]
const INTERACTIVE_TIMEOUT: Duration = Duration::from_secs(3);
#[cfg(unix)]
const CAPTURE_VARIABLE: &str = "LORCA_SHELL_ENV_FILE";
#[cfg(unix)]
const CAPTURE_COMMAND: &str = "/usr/bin/env -0 > \"$LORCA_SHELL_ENV_FILE\"";
/// What describes the shell that printed the environment rather than the user's setup, and what
/// the capture itself set.
#[cfg(unix)]
const DROPPED: [&str; 8] = ["PWD", "OLDPWD", "SHLVL", "_", CAPTURE_VARIABLE, "DISABLE_AUTO_UPDATE", "ZSH_TMUX_AUTOSTARTED", "ZSH_TMUX_AUTOSTART"];

/// `program` with the login shell's environment. The first call waits for the shell (five
/// seconds at most); `lorca serve` starts reading it at launch.
pub async fn command(program: impl AsRef<OsStr>) -> Command {
    command_with(program, environment().await)
}

/// The variables the login shell exports, its `PATH` ahead of the CLI's own and a few install
/// folders after both. Empty on Windows.
pub async fn environment() -> &'static [(OsString, OsString)] {
    ENVIRONMENT.get_or_init(load).await
}

fn command_with(program: impl AsRef<OsStr>, environment: &[(OsString, OsString)]) -> Command {
    let mut command = Command::new(program);
    command.envs(environment.iter().map(|(name, value)| (name, value)));
    command
}

#[cfg(unix)]
async fn load() -> Environment {
    let started = std::time::Instant::now();
    let home = std::env::var_os("HOME").map(PathBuf::from);
    let inherited_path = std::env::var_os("PATH");
    match capture(&shells(), TIMEOUT, INTERACTIVE_TIMEOUT).await {
        Some((shell, captured)) => {
            tracing::info!(shell = %shell.display(), elapsed_ms = started.elapsed().as_millis() as u64, "read the login shell's environment");
            child_environment(captured, inherited_path.as_deref(), home.as_deref())
        }
        None => {
            tracing::warn!("no login shell printed its environment; commands run with the CLI's own");
            child_environment(Vec::new(), inherited_path.as_deref(), home.as_deref())
        }
    }
}

#[cfg(not(unix))]
async fn load() -> Environment {
    Vec::new()
}

/// The account's shell first. `SHELL` describes whatever started the CLI, and launchd keeps the
/// value the login session began with, so it goes stale after `chsh`; the passwd entry is the
/// shell the user chose. Then `SHELL`, then the platform's.
#[cfg(unix)]
fn shells() -> Vec<PathBuf> {
    shell_candidates(account_shell(), std::env::var_os("SHELL").as_deref())
}

#[cfg(unix)]
fn shell_candidates(account: Option<PathBuf>, variable: Option<&OsStr>) -> Vec<PathBuf> {
    let mut shells: Vec<PathBuf> = account.into_iter().collect();
    shells.extend(variable.filter(|shell| !shell.is_empty()).map(PathBuf::from));
    #[cfg(target_os = "macos")]
    shells.push(PathBuf::from("/bin/zsh"));
    #[cfg(not(target_os = "macos"))]
    shells.extend([PathBuf::from("/bin/bash"), PathBuf::from("/bin/sh")]);
    let mut seen = std::collections::HashSet::new();
    shells.retain(|shell| shell.is_absolute() && seen.insert(shell.clone()));
    shells
}

#[cfg(unix)]
fn account_shell() -> Option<PathBuf> {
    use std::os::unix::ffi::OsStrExt;
    let mut size = 1024;
    loop {
        let mut passwd = std::mem::MaybeUninit::<libc::passwd>::uninit();
        let mut result = std::ptr::null_mut();
        let mut buffer = vec![0u8; size];
        let status = unsafe { libc::getpwuid_r(libc::geteuid(), passwd.as_mut_ptr(), buffer.as_mut_ptr().cast(), buffer.len(), &mut result) };
        if status == libc::ERANGE && size < 1 << 20 {
            size *= 2;
            continue;
        }
        if status != 0 || result.is_null() {
            return None;
        }
        let shell = unsafe { (*result).pw_shell };
        if shell.is_null() {
            return None;
        }
        let bytes = unsafe { std::ffi::CStr::from_ptr(shell) }.to_bytes();
        return (!bytes.is_empty()).then(|| PathBuf::from(OsStr::from_bytes(bytes)));
    }
}

/// Each shell interactive first, since that is where rc files put most of `PATH`, then as a plain
/// login shell.
#[cfg(unix)]
async fn capture(shells: &[PathBuf], timeout: Duration, interactive_timeout: Duration) -> Option<(PathBuf, Environment)> {
    let deadline = tokio::time::Instant::now() + timeout;
    for shell in shells {
        for interactive in [true, false] {
            let remaining = deadline.saturating_duration_since(tokio::time::Instant::now());
            if remaining.is_zero() {
                return None;
            }
            let budget = if interactive { remaining.min(interactive_timeout) } else { remaining };
            if let Some(environment) = capture_from(shell, interactive, budget).await {
                return Some((shell.clone(), environment));
            }
        }
    }
    None
}

/// The environment goes to a file rather than stdout, where rc files print banners and prompts.
#[cfg(unix)]
async fn capture_from(shell: &Path, interactive: bool, timeout: Duration) -> Option<Environment> {
    let file = CaptureFile::create()?;
    let mut command = Command::new(shell);
    if interactive {
        command.arg("-i");
    }
    command
        .args(["-l", "-c", CAPTURE_COMMAND])
        .env(CAPTURE_VARIABLE, &file.0)
        // oh-my-zsh's update prompt and tmux autostart would take the whole budget.
        .env("DISABLE_AUTO_UPDATE", "true")
        .env("ZSH_TMUX_AUTOSTARTED", "true")
        .env("ZSH_TMUX_AUTOSTART", "false")
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .kill_on_drop(true);
    // A session of its own, with no controlling terminal: an interactive shell started under a
    // terminal would otherwise try to take it over and stop.
    unsafe {
        command.pre_exec(|| if libc::setsid() == -1 { Err(std::io::Error::last_os_error()) } else { Ok(()) });
    }
    let mut child = command.spawn().ok()?;
    let status = match tokio::time::timeout(timeout, child.wait()).await {
        Ok(status) => status.ok()?,
        Err(_) => {
            // The session's id is the shell's pid; this also ends what its rc files started.
            if let Some(pid) = child.id() {
                unsafe {
                    libc::kill(-(pid as i32), libc::SIGKILL);
                }
            }
            let _ = child.kill().await;
            return None;
        }
    };
    if !status.success() {
        return None;
    }
    parse(&tokio::fs::read(&file.0).await.ok()?)
}

/// `env -0` output: `NAME=value` entries separated by NUL, so a value may hold newlines.
#[cfg(unix)]
fn parse(bytes: &[u8]) -> Option<Environment> {
    use std::os::unix::ffi::OsStrExt;
    let environment: Environment = bytes
        .split(|byte| *byte == 0)
        .filter_map(|entry| {
            let separator = entry.iter().position(|byte| *byte == b'=').filter(|at| *at > 0)?;
            let name = OsStr::from_bytes(&entry[..separator]);
            (!DROPPED.iter().any(|dropped| name == *dropped)).then(|| (name.to_owned(), OsStr::from_bytes(&entry[separator + 1..]).to_owned()))
        })
        .collect();
    (!environment.is_empty()).then_some(environment)
}

/// The captured variables with one `PATH`: the login shell's, then the CLI's own, then where
/// package managers install, in case neither has caught up with an install.
#[cfg(unix)]
fn child_environment(mut environment: Environment, inherited_path: Option<&OsStr>, home: Option<&Path>) -> Environment {
    let login_path = environment.iter().position(|(name, _)| name == "PATH").map(|at| environment.remove(at).1);
    let mut directories: Vec<PathBuf> = [login_path.as_deref(), inherited_path].into_iter().flatten().flat_map(std::env::split_paths).collect();
    if let Some(home) = home {
        directories.extend([".local/bin", ".bun/bin", ".cargo/bin", ".local/share/mise/shims", ".volta/bin"].map(|directory| home.join(directory)));
    }
    directories.extend(["/opt/homebrew/bin", "/usr/local/bin", "/usr/bin", "/bin", "/usr/sbin", "/sbin"].map(PathBuf::from));
    let mut seen = std::collections::HashSet::new();
    directories.retain(|directory| !directory.as_os_str().is_empty() && seen.insert(directory.clone()));
    if let Ok(path) = std::env::join_paths(directories) {
        environment.push(("PATH".into(), path));
    }
    environment
}

/// A file only this user can open, removed when dropped: the environment can hold tokens.
#[cfg(unix)]
struct CaptureFile(PathBuf);

#[cfg(unix)]
impl CaptureFile {
    fn create() -> Option<Self> {
        use std::os::unix::fs::OpenOptionsExt;
        let path = std::env::temp_dir().join(format!(".lorca-shell-env-{}", uuid::Uuid::new_v4().simple()));
        std::fs::OpenOptions::new().write(true).create_new(true).mode(0o600).open(&path).ok()?;
        Some(CaptureFile(path))
    }
}

#[cfg(unix)]
impl Drop for CaptureFile {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.0);
    }
}

#[cfg(all(test, unix))]
mod tests {
    use super::*;
    use std::os::unix::fs::PermissionsExt;

    fn fixture_dir(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("lorca-login-shell-{name}-{}", uuid::Uuid::new_v4().simple()));
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn executable(dir: &Path, name: &str, script: &str) -> PathBuf {
        let path = dir.join(name);
        std::fs::write(&path, script).unwrap();
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o700)).unwrap();
        path
    }

    #[test]
    fn the_accounts_shell_outranks_an_inherited_shell_variable() {
        let shells = shell_candidates(Some("/opt/homebrew/bin/fish".into()), Some(OsStr::new("/bin/zsh")));
        assert_eq!(shells[..2], [PathBuf::from("/opt/homebrew/bin/fish"), PathBuf::from("/bin/zsh")]);
        assert_eq!(shells.iter().filter(|shell| *shell == Path::new("/bin/zsh")).count(), 1);

        let shells = shell_candidates(None, Some(OsStr::new("")));
        assert!(!shells.is_empty() && shells.iter().all(|shell| shell.is_absolute()));
    }

    #[test]
    fn parses_nul_separated_entries_and_drops_the_capture_shells_own() {
        let environment = parse(b"PATH=/Users/me/.bun/bin:/usr/bin\0TOKEN=line one\nline two=rest\0EMPTY=\0PWD=/tmp\0SHLVL=2\0_=/usr/bin/env\0LORCA_SHELL_ENV_FILE=/tmp/x\0").unwrap();
        assert_eq!(
            environment,
            vec![
                ("PATH".into(), "/Users/me/.bun/bin:/usr/bin".into()),
                ("TOKEN".into(), "line one\nline two=rest".into()),
                ("EMPTY".into(), OsString::new()),
            ]
        );
        assert_eq!(parse(b""), None);
    }

    #[test]
    fn the_login_path_comes_before_the_inherited_one_and_install_folders_last() {
        let environment = child_environment(
            vec![("PATH".into(), "/Users/me/.local/share/mise/shims:/usr/bin".into()), ("GOPATH".into(), "/Users/me/go".into())],
            Some(OsStr::new("/usr/bin:/bin:/usr/sbin:/sbin")),
            Some(Path::new("/Users/me")),
        );
        assert_eq!(environment[0], ("GOPATH".into(), "/Users/me/go".into()));
        let (name, path) = &environment[1];
        assert_eq!(name, "PATH");
        let directories: Vec<PathBuf> = std::env::split_paths(path).collect();
        assert_eq!(directories[..4], ["/Users/me/.local/share/mise/shims", "/usr/bin", "/bin", "/usr/sbin"].map(PathBuf::from));
        assert!(directories.contains(&PathBuf::from("/Users/me/.bun/bin")) && directories.contains(&PathBuf::from("/opt/homebrew/bin")));
        assert_eq!(directories.iter().filter(|directory| *directory == Path::new("/usr/bin")).count(), 1);
    }

    #[tokio::test]
    async fn reads_what_an_interactive_login_shell_exports() {
        let dir = fixture_dir("capture");
        let shell = executable(
            &dir,
            "shell",
            "#!/bin/sh\n[ \"$1 $2 $3\" = '-i -l -c' ] || exit 9\nprintf 'PATH=/fixture/bin\\000LORCA_FIXTURE=from-shell\\000PWD=/elsewhere\\000' > \"$LORCA_SHELL_ENV_FILE\"\n",
        );
        let (used, environment) = capture(&[dir.join("missing"), shell.clone()], TIMEOUT, INTERACTIVE_TIMEOUT).await.unwrap();
        assert_eq!(used, shell);
        assert_eq!(environment, vec![("PATH".into(), "/fixture/bin".into()), ("LORCA_FIXTURE".into(), "from-shell".into())]);
        let _ = std::fs::remove_dir_all(dir);
    }

    #[tokio::test]
    async fn a_hanging_interactive_shell_falls_back_to_a_login_shell_and_is_killed() {
        let dir = fixture_dir("hang");
        let pid_file = dir.join("sleep.pid");
        let script = format!(
            "#!/bin/sh\nif [ \"$1\" = -i ]; then sleep 30 & echo $! > '{}'; wait; exit 0; fi\nprintf 'LORCA_FIXTURE=login\\000' > \"$LORCA_SHELL_ENV_FILE\"\n",
            pid_file.display()
        );
        let shell = executable(&dir, "shell", &script);
        // macOS checks a new executable on its first launch, which can outlast the budget below.
        let _ = std::process::Command::new(&shell).stderr(std::process::Stdio::null()).status();
        let (_, environment) = capture(&[shell], TIMEOUT, Duration::from_secs(1)).await.unwrap();
        assert_eq!(environment, vec![("LORCA_FIXTURE".into(), "login".into())]);

        let sleep = std::fs::read_to_string(&pid_file).expect("the interactive attempt never started");
        let sleep: i32 = sleep.trim().parse().unwrap();
        let mut alive = true;
        for _ in 0..100 {
            alive = unsafe { libc::kill(sleep, 0) } == 0;
            if !alive {
                break;
            }
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
        assert!(!alive, "the interactive shell's child outlived the timeout");
        let _ = std::fs::remove_dir_all(dir);
    }

    /// A stdio plugin server is named bare (`npx`), so the lookup has to use the child's `PATH`.
    #[tokio::test]
    async fn a_bare_program_resolves_through_the_environments_path() {
        let dir = fixture_dir("path");
        executable(&dir, "lorca-fixture-tool", "#!/bin/sh\nprintf found\n");
        let output = command_with("lorca-fixture-tool", &[("PATH".into(), dir.clone().into_os_string())]).output().await.unwrap();
        assert_eq!(output.stdout, b"found");
        let _ = std::fs::remove_dir_all(dir);
    }
}
