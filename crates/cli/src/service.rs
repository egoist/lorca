//! `lorca service`: runs `lorca serve` in the background on a computer without the app, from
//! login on and again whenever it stops, the way the apps keep their own CLI running. macOS gets a
//! launchd agent, Linux a systemd user unit (with lingering, so it runs without a login), and
//! Windows a supervisor, `lorca service run`, that the user's Run key starts at sign-in. Each
//! starts `lorca serve` with `LORCA_SERVICE=1`, and with the `LORCA_HOME`, `LORCA_PORT`, and
//! `LORCA_RELAY_URL` of the shell that installed it.

use std::path::PathBuf;
use std::process::Command;

use crate::config::Config;

/// The launchd label.
#[cfg(target_os = "macos")]
const NAME: &str = "app.lorca.serve";
/// The systemd user unit.
#[cfg(target_os = "linux")]
const UNIT: &str = "lorca.service";
/// A log larger than this starts over when `lorca serve` starts under the service.
const LOG_LIMIT: u64 = 20 << 20;

/// Whether `lorca service` started this process.
pub fn supervised() -> bool {
    std::env::var("LORCA_SERVICE").is_ok_and(|value| value == "1")
}

/// How the service stands, in words for `lorca service status`.
pub struct Status {
    pub installed: bool,
    pub running: Option<u32>,
    /// Where `lorca serve` writes its log, or how to read it.
    pub log: String,
}

/// The binary the service runs: this one, through any symlink to it.
fn exe() -> anyhow::Result<PathBuf> {
    let exe = std::env::current_exe()?;
    Ok(std::fs::canonicalize(&exe).unwrap_or(exe))
}

/// What `lorca serve` needs to serve the same account as the command that installed it: a
/// folder or port other than the default, and a relay named in the environment.
fn passed_on(config: &Config) -> Vec<(&'static str, String)> {
    let mut vars = Vec::new();
    if Some(&config.home) != dirs::home_dir().map(|home| home.join(".lorca")).as_ref() {
        vars.push(("LORCA_HOME", config.home.to_string_lossy().to_string()));
    }
    if config.port != crate::config::DEFAULT_PORT {
        vars.push(("LORCA_PORT", config.port.to_string()));
    }
    if let Some(url) = std::env::var("LORCA_RELAY_URL").ok().filter(|url| !url.trim().is_empty()) {
        vars.push(("LORCA_RELAY_URL", url));
    }
    vars
}

/// A `lorca serve` started under the service keeps its log in bounds: launchd appends to the
/// same file forever, so one past [`LOG_LIMIT`] starts over.
pub fn trim_log() {
    #[cfg(unix)]
    if supervised() {
        let mut stat: libc::stat = unsafe { std::mem::zeroed() };
        if unsafe { libc::fstat(2, &mut stat) } == 0 && (stat.st_mode & libc::S_IFMT) == libc::S_IFREG && stat.st_size as u64 > LOG_LIMIT {
            unsafe { libc::ftruncate(2, 0) };
        }
    }
}

// MARK: - macOS

#[cfg(target_os = "macos")]
fn plist_path() -> anyhow::Result<PathBuf> {
    Ok(dirs::home_dir().ok_or_else(|| anyhow::anyhow!("no home folder"))?.join("Library/LaunchAgents").join(format!("{NAME}.plist")))
}

#[cfg(target_os = "macos")]
fn log_path() -> PathBuf {
    dirs::home_dir().unwrap_or_default().join("Library/Logs/Lorca/serve.log")
}

#[cfg(target_os = "macos")]
fn domain() -> String {
    format!("gui/{}", unsafe { libc::getuid() })
}

#[cfg(target_os = "macos")]
pub fn install(config: &Config) -> anyhow::Result<Status> {
    let escape = |text: &str| text.replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;");
    let exe = exe()?;
    let log = log_path();
    std::fs::create_dir_all(log.parent().unwrap())?;
    let mut environment = String::from("\t\t<key>LORCA_SERVICE</key>\n\t\t<string>1</string>\n");
    for (name, value) in passed_on(config) {
        environment.push_str(&format!("\t\t<key>{name}</key>\n\t\t<string>{}</string>\n", escape(&value)));
    }
    let plist = format!(
        r#"<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
	<key>Label</key>
	<string>{NAME}</string>
	<key>ProgramArguments</key>
	<array>
		<string>{exe}</string>
		<string>serve</string>
	</array>
	<key>EnvironmentVariables</key>
	<dict>
{environment}	</dict>
	<key>RunAtLoad</key>
	<true/>
	<key>KeepAlive</key>
	<true/>
	<key>StandardOutPath</key>
	<string>{log}</string>
	<key>StandardErrorPath</key>
	<string>{log}</string>
</dict>
</plist>
"#,
        exe = escape(&exe.to_string_lossy()),
        log = escape(&log.to_string_lossy()),
    );
    let path = plist_path()?;
    std::fs::create_dir_all(path.parent().unwrap())?;
    std::fs::write(&path, plist)?;
    // A service from before goes first, so the new definition is the one that runs.
    let _ = Command::new("launchctl").args(["bootout", &format!("{}/{NAME}", domain())]).output();
    run(Command::new("launchctl").args(["bootstrap", &domain()]).arg(&path))?;
    Ok(status(config))
}

#[cfg(target_os = "macos")]
pub fn uninstall(_config: &Config) -> anyhow::Result<bool> {
    let path = plist_path()?;
    let _ = Command::new("launchctl").args(["bootout", &format!("{}/{NAME}", domain())]).output();
    let installed = path.exists();
    if installed {
        std::fs::remove_file(&path)?;
    }
    Ok(installed)
}

#[cfg(target_os = "macos")]
pub fn status(_config: &Config) -> Status {
    let installed = plist_path().is_ok_and(|path| path.exists());
    let output = Command::new("launchctl").args(["print", &format!("{}/{NAME}", domain())]).output();
    let running = output.ok().filter(|o| o.status.success()).and_then(|o| {
        String::from_utf8_lossy(&o.stdout).lines().find_map(|line| line.trim().strip_prefix("pid = ").and_then(|pid| pid.trim().parse().ok()))
    });
    Status { installed, running, log: log_path().display().to_string() }
}

// MARK: - Linux

#[cfg(target_os = "linux")]
fn unit_path() -> anyhow::Result<PathBuf> {
    Ok(dirs::config_dir().ok_or_else(|| anyhow::anyhow!("no configuration folder"))?.join("systemd/user").join(UNIT))
}

#[cfg(target_os = "linux")]
fn systemctl(args: &[&str]) -> Command {
    let mut command = Command::new("systemctl");
    command.arg("--user").args(args);
    command
}

#[cfg(target_os = "linux")]
pub fn install(config: &Config) -> anyhow::Result<Status> {
    if !systemctl(&["show-environment"]).output().is_ok_and(|o| o.status.success()) {
        anyhow::bail!(
            "This computer has no systemd user session (a container, or WSL without systemd), so lorca service cannot keep lorca serve running. Start `lorca serve` from your own init system, or in tmux."
        );
    }
    // systemd splits ExecStart on spaces and reads quotes and backslashes.
    let quote = |text: &str| format!("\"{}\"", text.replace('\\', "\\\\").replace('"', "\\\""));
    let mut environment = String::from("Environment=LORCA_SERVICE=1\n");
    for (name, value) in passed_on(config) {
        environment.push_str(&format!("Environment={}\n", quote(&format!("{name}={value}"))));
    }
    let unit = format!(
        "[Unit]\nDescription=Lorca: this computer's Runner (lorca serve)\n\n[Service]\nExecStart={} serve\n{environment}Restart=always\nRestartSec=2\n\n[Install]\nWantedBy=default.target\n",
        quote(&exe()?.to_string_lossy())
    );
    let path = unit_path()?;
    std::fs::create_dir_all(path.parent().unwrap())?;
    std::fs::write(&path, unit)?;
    run(&mut systemctl(&["daemon-reload"]))?;
    run(&mut systemctl(&["enable", UNIT]))?;
    run(&mut systemctl(&["restart", UNIT]))?;
    // Without lingering, the user's services stop at logout and start only at the next login.
    let user = std::env::var("USER").unwrap_or_default();
    let lingers = || Command::new("loginctl").args(["show-user", &user, "-p", "Linger", "--value"]).output().is_ok_and(|o| String::from_utf8_lossy(&o.stdout).trim() == "yes");
    if !user.is_empty() && !lingers() && Command::new("loginctl").args(["enable-linger", &user]).output().map_or(true, |o| !o.status.success()) {
        eprintln!("lorca serve runs while you are logged in. To keep it running after you log out, and to start it at boot, run: sudo loginctl enable-linger {user}");
    }
    Ok(status(config))
}

#[cfg(target_os = "linux")]
pub fn uninstall(_config: &Config) -> anyhow::Result<bool> {
    let path = unit_path()?;
    let installed = path.exists();
    let _ = systemctl(&["disable", "--now", UNIT]).output();
    if installed {
        std::fs::remove_file(&path)?;
        let _ = systemctl(&["daemon-reload"]).output();
    }
    Ok(installed)
}

#[cfg(target_os = "linux")]
pub fn status(_config: &Config) -> Status {
    let installed = unit_path().is_ok_and(|path| path.exists());
    let output = systemctl(&["show", "-p", "MainPID", "--value", UNIT]).output();
    let running = output.ok().and_then(|o| String::from_utf8_lossy(&o.stdout).trim().parse::<u32>().ok()).filter(|pid| *pid != 0);
    Status { installed, running, log: format!("journalctl --user -u {UNIT}") }
}

// MARK: - Windows

#[cfg(windows)]
const RUN_KEY: &str = r"HKCU\Software\Microsoft\Windows\CurrentVersion\Run";
#[cfg(windows)]
const RUN_VALUE: &str = "Lorca";
#[cfg(windows)]
const CREATE_NO_WINDOW: u32 = 0x0800_0000;
#[cfg(windows)]
const DETACHED_PROCESS: u32 = 0x0000_0008;

#[cfg(windows)]
fn pid_path(config: &Config) -> PathBuf {
    config.home.join("service.pid")
}

#[cfg(windows)]
fn log_path(config: &Config) -> PathBuf {
    config.home.join("serve.log")
}

#[cfg(windows)]
pub fn install(config: &Config) -> anyhow::Result<Status> {
    let exe = exe()?;
    // The Run key holds a command line. The supervisor names the home and port it serves, since
    // a sign-in starts it without the installing shell's variables.
    let mut line = format!("\"{}\" service run", exe.display());
    for (name, value) in passed_on(config) {
        // A backslash before the closing quote would escape it.
        line.push_str(&format!(" --env \"{name}={}\"", value.replace('"', "").trim_end_matches('\\')));
    }
    run(Command::new("reg").args(["add", RUN_KEY, "/v", RUN_VALUE, "/t", "REG_SZ", "/d", &line, "/f"]))?;
    stop(config);
    use std::os::windows::process::CommandExt;
    let mut command = Command::new(&exe);
    command.args(["service", "run"]).creation_flags(DETACHED_PROCESS | CREATE_NO_WINDOW);
    for (name, value) in passed_on(config) {
        command.args(["--env", &format!("{name}={value}")]);
    }
    command.spawn()?;
    // The supervisor writes its pid once it runs.
    for _ in 0..50 {
        if status(config).running.is_some() {
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(100));
    }
    Ok(status(config))
}

#[cfg(windows)]
pub fn uninstall(config: &Config) -> anyhow::Result<bool> {
    let installed = Command::new("reg").args(["query", RUN_KEY, "/v", RUN_VALUE]).output().is_ok_and(|o| o.status.success());
    if installed {
        run(Command::new("reg").args(["delete", RUN_KEY, "/v", RUN_VALUE, "/f"]))?;
    }
    stop(config);
    Ok(installed)
}

/// Stops the supervisor and the `lorca serve` it started.
#[cfg(windows)]
fn stop(config: &Config) {
    if let Some(pid) = supervisor_pid(config) {
        let _ = Command::new("taskkill").args(["/PID", &pid.to_string(), "/T", "/F"]).output();
    }
    let _ = std::fs::remove_file(pid_path(config));
}

#[cfg(windows)]
fn supervisor_pid(config: &Config) -> Option<u32> {
    let pid: u32 = std::fs::read_to_string(pid_path(config)).ok()?.trim().parse().ok()?;
    process_alive(pid).then_some(pid)
}

#[cfg(windows)]
pub fn status(config: &Config) -> Status {
    let installed = Command::new("reg").args(["query", RUN_KEY, "/v", RUN_VALUE]).output().is_ok_and(|o| o.status.success());
    Status { installed, running: supervisor_pid(config), log: log_path(config).display().to_string() }
}

#[cfg(windows)]
fn process_alive(pid: u32) -> bool {
    use windows_sys::Win32::Foundation::{CloseHandle, WAIT_TIMEOUT};
    use windows_sys::Win32::System::Threading::{OpenProcess, WaitForSingleObject, PROCESS_SYNCHRONIZE};
    unsafe {
        let handle = OpenProcess(PROCESS_SYNCHRONIZE, 0, pid);
        if handle.is_null() {
            return false;
        }
        let running = WaitForSingleObject(handle, 0) == WAIT_TIMEOUT;
        CloseHandle(handle);
        running
    }
}

/// `lorca service run`, the supervisor on Windows: leaves the console a sign-in opened for it,
/// then runs `lorca serve` from the installed binary until it is stopped, at once again after an
/// update ([`crate::update::RESTART_EXIT`]), and after a pause that grows while it keeps stopping
/// right after it starts. The log goes to `serve.log` in Lorca's folder, which starts over past
/// [`LOG_LIMIT`].
#[cfg(windows)]
pub fn supervise(config: &Config, env: &[(String, String)]) -> anyhow::Result<()> {
    use std::os::windows::process::CommandExt;
    unsafe { windows_sys::Win32::System::Console::FreeConsole() };
    if supervisor_pid(config).is_some_and(|pid| pid != std::process::id()) {
        return Ok(());
    }
    config.ensure_home()?;
    std::fs::write(pid_path(config), std::process::id().to_string())?;
    let exe = exe()?;
    let mut pause = std::time::Duration::from_secs(2);
    loop {
        let log = log_path(config);
        if std::fs::metadata(&log).is_ok_and(|m| m.len() > LOG_LIMIT) {
            let _ = std::fs::rename(&log, log.with_extension("log.1"));
        }
        let output = std::fs::OpenOptions::new().create(true).append(true).open(&log)?;
        let started = std::time::Instant::now();
        let mut command = Command::new(&exe);
        command.arg("serve").env("LORCA_SERVICE", "1").stdin(std::process::Stdio::null()).stdout(output.try_clone()?).stderr(output).creation_flags(CREATE_NO_WINDOW);
        for (name, value) in env {
            command.env(name, value);
        }
        let code = match command.spawn().and_then(|mut child| child.wait()) {
            Ok(status) => status.code(),
            Err(_) => None,
        };
        if code == Some(crate::update::RESTART_EXIT) {
            continue;
        }
        pause = if started.elapsed() > std::time::Duration::from_secs(60) { std::time::Duration::from_secs(2) } else { (pause * 2).min(std::time::Duration::from_secs(60)) };
        std::thread::sleep(pause);
    }
}

// MARK: - Elsewhere

#[cfg(not(any(target_os = "macos", target_os = "linux", windows)))]
pub fn install(_config: &Config) -> anyhow::Result<Status> {
    anyhow::bail!("lorca service runs on macOS, Linux, and Windows; start `lorca serve` from this system's own init.")
}

#[cfg(not(any(target_os = "macos", target_os = "linux", windows)))]
pub fn uninstall(_config: &Config) -> anyhow::Result<bool> {
    Ok(false)
}

#[cfg(not(any(target_os = "macos", target_os = "linux", windows)))]
pub fn status(_config: &Config) -> Status {
    Status { installed: false, running: None, log: String::new() }
}

fn run(command: &mut Command) -> anyhow::Result<()> {
    let output = command.output()?;
    if output.status.success() {
        return Ok(());
    }
    let text = String::from_utf8_lossy(&output.stderr).trim().to_string();
    let program = command.get_program().to_string_lossy().to_string();
    anyhow::bail!("{program} failed{}", if text.is_empty() { String::new() } else { format!(": {text}") })
}
