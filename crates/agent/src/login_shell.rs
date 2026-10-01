//! The environment of the user's login shell. A CLI the app starts inherits launchd's bare
//! environment, where `npx`, `bun`, or `cargo` are not on `PATH`, and the shell that sets them up
//! may be fish, whose files bash never reads. So the account's shell runs once as an interactive
//! login shell and prints its environment, and every command a bot runs and every stdio plugin
//! server starts with it, as from the user's terminal. Windows keeps the environment the CLI
//! started with, and finds a bare program name as its terminals do, through `PATHEXT`: npm
//! installs `npx` as `npx.cmd`.

use std::ffi::{OsStr, OsString};
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
/// seconds at most); `lorca serve` starts reading it at launch. On Windows a bare name starts
/// the file a terminal would run ([`find_program`]).
pub async fn command(program: impl AsRef<OsStr>) -> Command {
    command_with(program, environment().await)
}

/// The variables the login shell exports, its `PATH` ahead of the CLI's own and a few install
/// folders after both. Empty on Windows.
pub async fn environment() -> &'static [(OsString, OsString)] {
    ENVIRONMENT.get_or_init(load).await
}

fn command_with(program: impl AsRef<OsStr>, environment: &[(OsString, OsString)]) -> Command {
    #[cfg(windows)]
    let program = windows_program(program.as_ref(), environment);
    let mut command = Command::new(program);
    command.envs(environment.iter().map(|(name, value)| (name, value)));
    command
}

/// What Windows starts for `program`. `Command` gives a bare name `.exe` and no other extension
/// (rust-lang/rust#94743), so `npx` would never find `npx.cmd`. A name the lookup finds on the
/// child's `PATH` starts from that file; anything else goes to `Command` as it is, for its own
/// search and error. `Command` runs a batch file through `cmd.exe`, escaping each argument so it
/// arrives as written (CVE-2024-24576), and refuses one that holds a line break.
#[cfg(windows)]
fn windows_program(program: &OsStr, environment: &[(OsString, OsString)]) -> OsString {
    let variable = |name: &str| environment.iter().rev().find(|(key, _)| key.eq_ignore_ascii_case(name)).map(|(_, value)| value.clone()).or_else(|| std::env::var_os(name));
    find_program(program, variable("PATH").as_deref(), variable("PATHEXT").as_deref(), Path::is_file).map_or_else(|| program.to_owned(), PathBuf::into_os_string)
}

/// The extensions of the files `Command` can start, programs and batch files, in the order of
/// Windows' own `PATHEXT`.
#[cfg(any(windows, test))]
const STARTABLE: [&str; 4] = [".com", ".exe", ".bat", ".cmd"];

/// The file a Windows terminal runs for a bare `name`: `name` with an extension from `pathext`
/// (Windows' own when unset), or `name` itself when it already ends in one, in the first folder
/// of `path` that holds such a file, the extensions tried in `pathext`'s order. Only
/// [`STARTABLE`] extensions count: a terminal opens a `.js` on `PATHEXT` through its file
/// association, and Node installs an extensionless shell script beside `npx.cmd`. A relative
/// folder is skipped, as it would be looked up from the CLI's working directory, not the
/// child's. `exists` says whether a file is there, so tests run the lookup on any platform.
#[cfg(any(windows, test))]
fn find_program(name: &OsStr, path: Option<&OsStr>, pathext: Option<&OsStr>, exists: impl Fn(&Path) -> bool) -> Option<PathBuf> {
    if Path::new(name).file_name() != Some(name) {
        return None;
    }
    let startable = |extension: &str| STARTABLE.iter().any(|known| extension.eq_ignore_ascii_case(known));
    let mut extensions: Vec<&str> = pathext.and_then(OsStr::to_str).unwrap_or_default().split(';').filter(|extension| startable(extension)).collect();
    if extensions.is_empty() {
        extensions = STARTABLE.to_vec();
    }
    let names: Vec<OsString> = match Path::new(name).extension().and_then(OsStr::to_str) {
        Some(extension) if startable(&format!(".{extension}")) => vec![name.to_owned()],
        // Lowercase, as the files are named: `npx.cmd`, not PATHEXT's `npx.CMD`.
        _ => extensions
            .iter()
            .map(|extension| {
                let mut file = name.to_owned();
                file.push(extension.to_ascii_lowercase());
                file
            })
            .collect(),
    };
    std::env::split_paths(path?).filter(|folder| folder.is_absolute()).flat_map(|folder| names.iter().map(move |file| folder.join(file))).find(|file| exists(file))
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

#[cfg(test)]
mod tests {
    use super::*;
    #[cfg(unix)]
    use std::os::unix::fs::PermissionsExt;

    fn fixture_dir(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("lorca-login-shell-{name}-{}", uuid::Uuid::new_v4().simple()));
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[cfg(unix)]
    fn executable(dir: &Path, name: &str, script: &str) -> PathBuf {
        let path = dir.join(name);
        std::fs::write(&path, script).unwrap();
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o700)).unwrap();
        path
    }

    #[cfg(unix)]
    #[test]
    fn the_accounts_shell_outranks_an_inherited_shell_variable() {
        let shells = shell_candidates(Some("/opt/homebrew/bin/fish".into()), Some(OsStr::new("/bin/zsh")));
        assert_eq!(shells[..2], [PathBuf::from("/opt/homebrew/bin/fish"), PathBuf::from("/bin/zsh")]);
        assert_eq!(shells.iter().filter(|shell| *shell == Path::new("/bin/zsh")).count(), 1);

        let shells = shell_candidates(None, Some(OsStr::new("")));
        assert!(!shells.is_empty() && shells.iter().all(|shell| shell.is_absolute()));
    }

    #[cfg(unix)]
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

    #[cfg(unix)]
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

    #[cfg(unix)]
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

    #[cfg(unix)]
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
    #[cfg(unix)]
    #[tokio::test]
    async fn a_bare_program_resolves_through_the_environments_path() {
        let dir = fixture_dir("path");
        executable(&dir, "lorca-fixture-tool", "#!/bin/sh\nprintf found\n");
        let output = command_with("lorca-fixture-tool", &[("PATH".into(), dir.clone().into_os_string())]).output().await.unwrap();
        assert_eq!(output.stdout, b"found");
        let _ = std::fs::remove_dir_all(dir);
    }

    /// npm installs `npx` as `npx.cmd`, beside an extensionless shell script. The batch file runs
    /// through `cmd.exe`, and its arguments reach it as written: quoted where they need it, with
    /// `%PATH%` left as it is and `&` no second command. One with a line break cannot be passed.
    #[cfg(windows)]
    #[tokio::test]
    async fn a_bare_program_starts_a_batch_file_from_the_environments_path() {
        let dir = fixture_dir("Program Files");
        std::fs::write(dir.join("lorca-fixture-tool"), "#!/bin/sh\necho the shell script\n").unwrap();
        std::fs::write(dir.join("lorca-fixture-tool.cmd"), "@echo %*\r\n").unwrap();
        let environment = [("PATH".into(), dir.clone().into_os_string())];
        let command = || command_with("lorca-fixture-tool", &environment);
        assert_eq!(Path::new(command().as_std().get_program()), dir.join("lorca-fixture-tool.cmd"));

        let output = command().args(["-y", "@playwright/mcp@latest", "--headless", "two words", "%PATH%", "a&echo injected"]).output().await.unwrap();
        assert!(output.status.success(), "{output:?}");
        assert_eq!(String::from_utf8_lossy(&output.stdout), "-y @playwright/mcp@latest --headless \"two words\" \"%PATH%\" \"a&echo injected\"\r\n");

        let refused = command().arg("two\nlines").output().await.unwrap_err();
        assert_eq!(refused.kind(), std::io::ErrorKind::InvalidInput);
        let _ = std::fs::remove_dir_all(dir);
    }

    /// The Windows lookup, on any platform: PATH's folders in order, PATHEXT's order within one,
    /// and only the files `Command` can start.
    #[test]
    fn a_bare_name_finds_what_a_windows_terminal_runs() {
        let root = PathBuf::from(if cfg!(windows) { "C:\\" } else { "/" });
        let system32 = root.join("Windows").join("System32");
        let node = root.join("Program Files").join("nodejs");
        let local = root.join("Users").join("me").join(".local").join("bin");
        let relative = PathBuf::from("relative");
        let path = std::env::join_paths([&system32, &relative, &node, &local]).unwrap();
        let files = [
            node.join("npx"),
            node.join("npx.ps1"),
            node.join("npx.cmd"),
            local.join("npx.exe"),
            relative.join("uvx.exe"),
            local.join("uvx.exe"),
            system32.join("tool.js"),
            local.join("tool.bat"),
            local.join("both.cmd"),
            local.join("both.exe"),
            local.join("python3.12.exe"),
        ];
        let find = |name: &str, pathext: Option<&str>| find_program(OsStr::new(name), Some(path.as_os_str()), pathext.map(OsStr::new), |file| files.iter().any(|known| known == file));
        let pathext = Some(".COM;.EXE;.BAT;.CMD;.VBS;.VBE;.JS;.JSE;.WSF;.WSH;.MSC;.PS1");

        // npm's batch file, not the shell or PowerShell script beside it, nor an npx.exe further on.
        assert_eq!(find("npx", pathext), Some(node.join("npx.cmd")));
        assert_eq!(find("uvx", pathext), Some(local.join("uvx.exe")), "a relative folder is skipped");
        assert_eq!(find("tool", pathext), Some(local.join("tool.bat")), "a .js opens through its file association, which Command does not use");
        assert_eq!(find("both", pathext), Some(local.join("both.exe")));
        assert_eq!(find("both", Some(".CMD;.EXE")), Some(local.join("both.cmd")));
        assert_eq!(find("both", None), Some(local.join("both.exe")), "Windows' own PATHEXT when unset");
        assert_eq!(find("npx", Some(".JS;.PS1")), Some(node.join("npx.cmd")));
        assert_eq!(find("npx.exe", pathext), Some(local.join("npx.exe")), "a name with its extension is looked up as it is");
        assert_eq!(find("python3.12", pathext), Some(local.join("python3.12.exe")));
        assert_eq!(find("missing", pathext), None);
        assert_eq!(find("node_modules/.bin/npx", pathext), None, "a path goes to Command as it is");
        assert_eq!(find_program(OsStr::new("npx"), None, None, |_| true), None);
    }
}
