//! Keeps a CLI installed with the site's script current, as Sparkle and MyGo's updater keep the
//! apps and the CLI they carry. Only the builds `release-cli.yml` makes replace themselves
//! ([`SELF_UPDATING`]); the apps' bundles, the dev loop's builds, and `cargo build`'s never do.
//!
//! Each `cli-v` release carries `lorca-cli.json`, the version and each build's archive with its
//! SHA-256 and size, and `lorca-cli.json.sig`, an Ed25519 signature of that file by a key in
//! [`KEYS`]. `lorca serve` reads both from the repo's latest release (`LORCA_DOWNLOAD_URL` stands
//! for the releases page, as it does for the install scripts) a few minutes after it starts and
//! then once a day, at once when the relay refuses the protocol it speaks, and when a Device asks
//! (`update.install`). With automatic updates on, the default, a newer release goes in beside the
//! running binary: the archive has to match the signed manifest, the binary in it has to start
//! here and name the manifest's version, and a rename puts it in the old one's place, which stays
//! beside it for a rollback. The CLI then restarts into it once no bot is at work on this Runner
//! ([`restart_when_idle`]): in place on macOS and Linux, through the `lorca service` supervisor on
//! Windows. A release that keeps stopping right after it starts goes back to the one before.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use base64::Engine;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

use crate::app::App;
use crate::config::{self, VERSION};
use crate::model::UpdateStatus;

/// Set by `release-cli.yml` when it builds a release: such a CLI replaces itself.
pub const SELF_UPDATING: bool = option_env!("LORCA_SELF_UPDATE").is_some();

/// The public halves of the keys that sign `lorca-cli.json`, base64. A new key is added here a
/// release before it signs, so the CLIs in the field already trust it.
const KEYS: &[&str] = &["mQrlaAaYt9APwAWYN5smWgxI5QJ/W/9He3gsplur1gM="];

const RELEASES: &str = "https://github.com/egoist/lorca/releases";
const MANIFEST: &str = "lorca-cli.json";
/// What `kind` says in a manifest, so a signature over some other file never passes for one.
const KIND: &str = "lorca-cli";

/// This build's name in the manifest and in the release's archives.
const TARGET: &str = if cfg!(all(target_os = "macos", target_arch = "aarch64")) {
    "macos-aarch64"
} else if cfg!(all(target_os = "linux", target_arch = "aarch64")) {
    "linux-aarch64"
} else if cfg!(all(target_os = "linux", target_arch = "x86_64")) {
    "linux-x86_64"
} else if cfg!(all(target_os = "windows", target_arch = "x86_64")) {
    "windows-x86_64"
} else {
    "unsupported"
};

const DAY: u64 = 24 * 60 * 60;
/// How many times a new release may start without staying up before it goes back.
const MAX_STARTS: u32 = 3;
/// How long a new release has to stay up, or how soon it has to reach the relay, to stay.
const SETTLE: Duration = Duration::from_secs(60);
/// The exit code that tells the `lorca service` supervisor on Windows to start `lorca serve`
/// again at once, from the binary now in place.
pub const RESTART_EXIT: i32 = 75;

/// This Device's updates while `lorca serve` runs.
#[derive(Default)]
pub struct Updater {
    /// What the `machine` blob says; `None` until [`start`], and always for a CLI that does not
    /// replace itself.
    status: Mutex<Option<UpdateStatus>>,
    /// One check or install at a time.
    busy: tokio::sync::Mutex<()>,
    /// The version now in place of the running binary, waiting for the restart.
    installed: Mutex<Option<String>>,
    /// Wakes the daily loop: a Device asked, automatic updates came on, or the relay refused
    /// this build.
    wake: tokio::sync::Notify,
}

impl Updater {
    pub fn status(&self) -> Option<UpdateStatus> {
        self.status.lock().unwrap().clone()
    }
}

/// `update.json` in Lorca's folder.
#[derive(Debug, Default, Serialize, Deserialize)]
struct Record {
    /// Unix seconds of the last check that was answered.
    #[serde(default)]
    checked_at: i64,
    /// The newest version that check found.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    latest: Option<String>,
    /// A restart into a new release, from the moment it goes in until it has stayed up.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pending: Option<Pending>,
    /// A release that went back: automatic updates pass it over, `lorca update` does not.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    skip: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct Pending {
    from: String,
    to: String,
    /// The binary `from` ran, kept beside the new one.
    previous: PathBuf,
    /// How many times `to` started without staying up.
    #[serde(default)]
    starts: u32,
}

#[derive(Debug, Deserialize)]
struct Manifest {
    kind: String,
    version: String,
    builds: BTreeMap<String, Build>,
}

#[derive(Debug, Deserialize)]
struct Build {
    file: String,
    sha256: String,
    size: u64,
}

// MARK: - Starting

/// Starts this CLI's updates with `lorca serve`: settles a restart into a new release, then
/// checks a few minutes from now and daily after. Nothing for a CLI that does not replace itself.
pub fn start(app: &Arc<App>) {
    if !SELF_UPDATING {
        return;
    }
    remove_leftovers();
    let mut record = read(app);
    let mut error = None;
    if let Some(pending) = record.pending.clone() {
        if pending.to != VERSION {
            // Another version runs: a restart by hand, or the rollback itself.
            record.pending = None;
        } else if pending.starts >= MAX_STARTS {
            // Returns only when the release from before could not start.
            error = Some(roll_back(app, &mut record, pending));
        } else {
            record.pending = Some(Pending { starts: pending.starts + 1, ..pending });
            tokio::spawn(settle(app.clone()));
        }
        write(app, &record);
    }
    let latest = record.latest.clone().filter(|latest| is_newer(latest, VERSION));
    // The release from before, back in place: say why.
    if error.is_none() && latest.is_some() && record.skip == latest {
        error = latest.as_ref().map(|skipped| format!("{skipped} kept stopping right after it started, so this Runner went back to {VERSION}."));
    }
    *app.updates.status.lock().unwrap() = Some(UpdateStatus { auto: auto(app), latest, error, ..Default::default() });
    tokio::spawn(run(app.clone()));
}

/// Forgets the restart once the new release reached the relay or stayed up a minute.
async fn settle(app: Arc<App>) {
    let started = tokio::time::Instant::now();
    while started.elapsed() < SETTLE && !app.relay_connected.load(std::sync::atomic::Ordering::Relaxed) {
        tokio::time::sleep(Duration::from_secs(1)).await;
    }
    let mut record = read(&app);
    if record.pending.take().is_some() {
        write(&app, &record);
        tracing::info!(version = VERSION, "running the new release");
    }
}

/// The new release kept stopping: the binary it replaced goes back in its place and starts, and
/// automatic updates pass this release over. Answers why this release still runs when the one
/// from before could not start.
fn roll_back(app: &App, record: &mut Record, pending: Pending) -> String {
    tracing::error!(version = %pending.to, back_to = %pending.from, "the new release did not stay up; going back");
    record.pending = None;
    record.skip = Some(pending.to.clone());
    write(app, record);
    let failed = match current_exe().and_then(|exe| put_back(&pending.previous, &exe)) {
        Ok(()) => restart(),
        Err(error) => error,
    };
    tracing::error!(error = %failed, "going back to the release from before");
    format!("{} stopped right after it started, and {} could not take its place: {failed}", pending.to, pending.from)
}

/// Checks now, and again a day later.
async fn run(app: Arc<App>) {
    // Some minutes after the start, so Runners that start together do not all ask at once.
    let mut wait = Duration::from_secs(60 + rand::random::<u64>() % 540);
    loop {
        tokio::select! {
            _ = tokio::time::sleep(wait) => {}
            _ = app.updates.wake.notified() => {}
        }
        wait = Duration::from_secs(DAY + rand::random::<u64>() % 3600);
        let install = auto(&app);
        if let Err(error) = check(&app, install, false).await {
            tracing::warn!(%error, "checking for a newer lorca");
        }
    }
}

/// The relay answered `426`: a newer release probably speaks its protocol, so check now.
pub fn relay_refused(app: &App) {
    if SELF_UPDATING {
        app.updates.wake.notify_one();
    }
}

// MARK: - Checking and installing

/// Checks the latest release, and installs it when `install` and it is newer: a release that
/// went back only when `forced`. Answers what the apps show.
pub async fn check(app: &Arc<App>, install: bool, forced: bool) -> Result<Value, String> {
    let _busy = app.updates.busy.lock().await;
    if let Some(version) = app.updates.installed.lock().unwrap().clone() {
        return Ok(reply(app, Some(&version)));
    }
    let manifest = match latest(app).await {
        Ok(manifest) => manifest,
        Err(error) => {
            set_status(app, |status| status.error = Some(error.clone()));
            return Err(error);
        }
    };
    let mut record = read(app);
    record.checked_at = config::now_unix();
    record.latest = Some(manifest.version.clone());
    write(app, &record);
    let newer = is_newer(&manifest.version, VERSION);
    set_status(app, |status| {
        status.latest = newer.then(|| manifest.version.clone());
        status.error = None;
    });
    let skipped = record.skip.as_deref() == Some(manifest.version.as_str());
    if newer && install && (forced || !skipped) {
        set_status(app, |status| status.state = "installing".into());
        if let Err(error) = put_in_place(app, &manifest).await {
            tracing::warn!(%error, version = %manifest.version, "installing a newer lorca");
            set_status(app, |status| {
                status.state = String::new();
                status.error = Some(error.clone());
            });
            return Err(error);
        }
        tracing::info!(version = %manifest.version, "installed; restarting into it once no bot is at work");
        *app.updates.installed.lock().unwrap() = Some(manifest.version.clone());
        set_status(app, |status| status.state = if can_restart() { "restarting".into() } else { "installed".into() });
        tokio::spawn(restart_when_idle(app.clone(), manifest.version.clone()));
        return Ok(reply(app, Some(&manifest.version)));
    }
    Ok(reply(app, None))
}

/// `update.install` from a Device, and `lorca update` through a running `lorca serve`: checks
/// now and, when there is a newer release, answers at once while it installs.
pub async fn install_now(app: &Arc<App>) -> Result<Value, String> {
    if !SELF_UPDATING {
        return Err(not_self_updating(app));
    }
    if let Some(version) = app.updates.installed.lock().unwrap().clone() {
        return Ok(reply(app, Some(&version)));
    }
    let manifest = latest(app).await?;
    if !is_newer(&manifest.version, VERSION) {
        set_status(app, |status| {
            status.latest = None;
            status.error = None;
        });
        return Ok(reply(app, None));
    }
    let worker = app.clone();
    tokio::spawn(async move {
        let _ = check(&worker, true, true).await;
    });
    Ok(json!({ "version": VERSION, "latest": manifest.version, "installing": true }))
}

/// `update.auto`: turns automatic updates on or off; on checks now.
pub fn set_auto(app: &Arc<App>, on: bool) -> Result<Value, String> {
    if !SELF_UPDATING {
        return Err(not_self_updating(app));
    }
    {
        let mut settings = app.settings.lock().unwrap();
        settings.auto_update = Some(on);
        settings.save(&app.config).map_err(|e| e.to_string())?;
    }
    set_status(app, |status| status.auto = on);
    if on {
        app.updates.wake.notify_one();
    }
    Ok(reply(app, None))
}

/// Why a Device's CLI does not update itself.
fn not_self_updating(app: &App) -> String {
    let name = app.local_device().map(|d| d.name).unwrap_or_else(|| "This Device".into());
    format!("Lorca on {name} updates with the app that installed it.")
}

/// What `lorca update` and the apps read back: this version, the newest found, and whether one
/// is in place waiting for the restart.
fn reply(app: &App, installed: Option<&str>) -> Value {
    let status = app.updates.status().unwrap_or_default();
    json!({ "version": VERSION, "latest": status.latest, "installed": installed, "auto": status.auto })
}

fn set_status(app: &Arc<App>, change: impl FnOnce(&mut UpdateStatus)) {
    {
        let mut status = app.updates.status.lock().unwrap();
        let Some(status) = status.as_mut() else { return };
        change(status);
    }
    app.push_machine_blob_if_changed();
    app.emit(app.roster_summary());
}

fn auto(app: &App) -> bool {
    app.settings.lock().unwrap().auto_update.unwrap_or(true)
}

/// `lorca update --check` without a running `lorca serve`: the latest release's version.
pub async fn latest_version(app: &App) -> Result<String, String> {
    Ok(latest(app).await?.version)
}

/// `lorca update` without a running `lorca serve`: installs the latest release over this binary
/// when it is newer, and names it.
pub async fn install_here(app: &Arc<App>) -> Result<Option<String>, String> {
    let manifest = latest(app).await?;
    if !is_newer(&manifest.version, VERSION) {
        return Ok(None);
    }
    put_in_place(app, &manifest).await?;
    let mut record = read(app);
    record.latest = Some(manifest.version.clone());
    record.checked_at = config::now_unix();
    record.skip = None;
    write(app, &record);
    Ok(Some(manifest.version))
}

/// The latest release's manifest, its signature checked.
async fn latest(app: &App) -> Result<Manifest, String> {
    if TARGET == "unsupported" {
        return Err("There are no Lorca CLI releases for this computer.".into());
    }
    let base = format!("{}/latest/download", releases());
    let bytes = get(app, &format!("{base}/{MANIFEST}"), 1 << 20, Duration::from_secs(30)).await?;
    let signature = get(app, &format!("{base}/{MANIFEST}.sig"), 4096, Duration::from_secs(30)).await?;
    verify(&bytes, &signature, &trusted_keys())?;
    let manifest: Manifest = serde_json::from_slice(&bytes).map_err(|e| format!("{MANIFEST} does not read: {e}"))?;
    if manifest.kind != KIND {
        return Err(format!("{MANIFEST} is not a Lorca CLI manifest"));
    }
    Ok(manifest)
}

/// The releases page, or the server `LORCA_DOWNLOAD_URL` names in its place.
fn releases() -> String {
    std::env::var("LORCA_DOWNLOAD_URL").ok().map(|url| url.trim().trim_end_matches('/').to_string()).filter(|url| !url.is_empty()).unwrap_or_else(|| RELEASES.into())
}

fn trusted_keys() -> Vec<String> {
    // A scratch build for the end-to-end test names its own key.
    KEYS.iter().map(|key| key.to_string()).chain(option_env!("LORCA_UPDATE_TEST_KEY").map(str::to_string)).collect()
}

async fn get(app: &App, url: &str, limit: u64, timeout: Duration) -> Result<Vec<u8>, String> {
    let response = app
        .http
        .get(url)
        .timeout(timeout)
        .header(reqwest::header::USER_AGENT, concat!("lorca/", env!("CARGO_PKG_VERSION")))
        .send()
        .await
        .map_err(|e| format!("Could not reach {url}: {e}"))?;
    if !response.status().is_success() {
        return Err(format!("{url} answered {}", response.status()));
    }
    if response.content_length().is_some_and(|length| length > limit) {
        return Err(format!("{url} is larger than expected"));
    }
    let bytes = response.bytes().await.map_err(|e| format!("Downloading {url}: {e}"))?;
    if bytes.len() as u64 > limit {
        return Err(format!("{url} is larger than expected"));
    }
    Ok(bytes.to_vec())
}

/// Checks `signature`, a base64 Ed25519 signature, over `bytes` against each of `keys`.
fn verify(bytes: &[u8], signature: &[u8], keys: &[String]) -> Result<(), String> {
    let text = std::str::from_utf8(signature).map_err(|_| "The manifest's signature does not read")?;
    let signature = base64::engine::general_purpose::STANDARD.decode(text.trim()).map_err(|_| "The manifest's signature does not read")?;
    let signature = ed25519_dalek::Signature::from_slice(&signature).map_err(|_| "The manifest's signature does not read")?;
    for key in keys {
        let Ok(key) = base64::engine::general_purpose::STANDARD.decode(key) else { continue };
        let Ok(key) = <[u8; 32]>::try_from(key.as_slice()) else { continue };
        let Ok(key) = ed25519_dalek::VerifyingKey::from_bytes(&key) else { continue };
        if key.verify_strict(bytes, &signature).is_ok() {
            return Ok(());
        }
    }
    Err("The manifest is not signed by Lorca's update key".into())
}

/// Whether `a` is a later `major.minor.patch` than `b`.
pub fn is_newer(a: &str, b: &str) -> bool {
    let parts = |v: &str| v.trim().trim_start_matches('v').split('.').map(|part| part.parse::<u64>().unwrap_or(0)).collect::<Vec<_>>();
    let (a, b) = (parts(a), parts(b));
    for i in 0..a.len().max(b.len()) {
        let (x, y) = (a.get(i).copied().unwrap_or(0), b.get(i).copied().unwrap_or(0));
        if x != y {
            return x > y;
        }
    }
    false
}

/// Downloads this computer's build of `manifest`, checks it, and renames it over the running
/// binary, keeping the old one beside it.
async fn put_in_place(app: &App, manifest: &Manifest) -> Result<(), String> {
    let build = manifest.builds.get(TARGET).ok_or_else(|| format!("Lorca {} has no build for {TARGET}.", manifest.version))?;
    let exe = current_exe()?;
    let dir = exe.parent().ok_or("The lorca binary has no folder")?.to_path_buf();
    let url = format!("{}/download/cli-v{}/{}", releases(), manifest.version, build.file);
    let archive = get(app, &url, build.size, Duration::from_secs(15 * 60)).await?;
    let digest = <sha2::Sha256 as sha2::Digest>::digest(&archive);
    let digest: String = digest.iter().map(|b| format!("{b:02x}")).collect();
    if archive.len() as u64 != build.size || !digest.eq_ignore_ascii_case(build.sha256.trim()) {
        return Err(format!("{} does not match the release's manifest", build.file));
    }
    let binary = unpack(&build.file, &archive)?;
    let staged = dir.join(format!(".lorca-{}-{}{}", manifest.version, std::process::id(), std::env::consts::EXE_SUFFIX));
    write_executable(&staged, &binary).map_err(|e| format!("Cannot write to {}: {e}", dir.display()))?;
    let checked = starts_as(&staged, &manifest.version).await;
    if let Err(error) = checked {
        let _ = std::fs::remove_file(&staged);
        return Err(error);
    }
    let previous = swap(&exe, &staged).map_err(|e| {
        let _ = std::fs::remove_file(&staged);
        format!("Cannot replace {}: {e}", exe.display())
    })?;
    let mut record = read(app);
    record.pending = Some(Pending { from: VERSION.into(), to: manifest.version.clone(), previous, starts: 0 });
    write(app, &record);
    Ok(())
}

/// The `lorca` binary inside a release archive: a `.tar.gz` for macOS and Linux, a `.zip` for
/// Windows, each holding the binary alone.
fn unpack(file: &str, archive: &[u8]) -> Result<Vec<u8>, String> {
    use std::io::Read;
    let name = format!("lorca{}", std::env::consts::EXE_SUFFIX);
    let mut binary = Vec::new();
    if file.ends_with(".tar.gz") {
        let mut tar = tar::Archive::new(flate2::read::GzDecoder::new(archive));
        for entry in tar.entries().map_err(|e| format!("{file}: {e}"))? {
            let mut entry = entry.map_err(|e| format!("{file}: {e}"))?;
            let path = entry.path().map_err(|e| format!("{file}: {e}"))?.into_owned();
            if path.file_name().is_some_and(|n| n == name.as_str()) {
                entry.read_to_end(&mut binary).map_err(|e| format!("{file}: {e}"))?;
                return Ok(binary);
            }
        }
    } else if file.ends_with(".zip") {
        #[cfg(windows)]
        {
            let mut zip = zip::ZipArchive::new(std::io::Cursor::new(archive)).map_err(|e| format!("{file}: {e}"))?;
            let mut entry = zip.by_name(&name).map_err(|e| format!("{file}: {e}"))?;
            entry.read_to_end(&mut binary).map_err(|e| format!("{file}: {e}"))?;
            return Ok(binary);
        }
    }
    Err(format!("{file} holds no {name}"))
}

fn write_executable(path: &Path, bytes: &[u8]) -> std::io::Result<()> {
    std::fs::write(path, bytes)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o755))?;
    }
    Ok(())
}

/// The staged binary has to start on this computer and say it is `version`.
async fn starts_as(path: &Path, version: &str) -> Result<(), String> {
    let run = tokio::process::Command::new(path).arg("--version").kill_on_drop(true).output();
    let output = tokio::time::timeout(Duration::from_secs(30), run)
        .await
        .map_err(|_| format!("The downloaded lorca {version} did not start"))?
        .map_err(|e| format!("The downloaded lorca {version} does not start on this computer: {e}"))?;
    let said = String::from_utf8_lossy(&output.stdout).trim().to_string();
    if !output.status.success() || said != format!("lorca {version}") {
        return Err(format!("The downloaded lorca says \"{said}\", not lorca {version}"));
    }
    Ok(())
}

/// The running binary's file, through any symlink to it.
fn current_exe() -> Result<PathBuf, String> {
    let exe = std::env::current_exe().map_err(|e| e.to_string())?;
    Ok(std::fs::canonicalize(&exe).unwrap_or(exe))
}

/// Renames `staged` over `exe` and answers where the old binary went. On macOS and Linux the
/// running process keeps the file it started from, and the old one stays as a hard link; Windows
/// lets a running program's file be renamed but not replaced, so it moves aside first.
fn swap(exe: &Path, staged: &Path) -> std::io::Result<PathBuf> {
    let dir = exe.parent().unwrap_or(Path::new("."));
    #[cfg(unix)]
    {
        let previous = dir.join(".lorca-previous");
        let _ = std::fs::remove_file(&previous);
        if std::fs::hard_link(exe, &previous).is_err() {
            std::fs::copy(exe, &previous)?;
        }
        std::fs::rename(staged, exe)?;
        Ok(previous)
    }
    #[cfg(windows)]
    {
        let previous = dir.join(format!("lorca-{VERSION}.old.exe"));
        let _ = std::fs::remove_file(&previous);
        std::fs::rename(exe, &previous)?;
        if let Err(error) = std::fs::rename(staged, exe) {
            let _ = std::fs::rename(&previous, exe);
            return Err(error);
        }
        Ok(previous)
    }
}

/// Puts the binary from before back in `exe`'s place.
fn put_back(previous: &Path, exe: &Path) -> Result<(), String> {
    #[cfg(windows)]
    {
        let aside = exe.parent().unwrap_or(Path::new(".")).join(format!("lorca-{VERSION}.bad.exe"));
        let _ = std::fs::remove_file(&aside);
        std::fs::rename(exe, &aside).map_err(|e| e.to_string())?;
    }
    std::fs::rename(previous, exe).map_err(|e| e.to_string())
}

/// Old binaries Windows could not delete while they ran.
fn remove_leftovers() {
    #[cfg(windows)]
    if let Some(dir) = current_exe().ok().and_then(|exe| exe.parent().map(Path::to_path_buf)) {
        for entry in std::fs::read_dir(dir).into_iter().flatten().flatten() {
            let name = entry.file_name().to_string_lossy().to_string();
            if name.starts_with("lorca-") && (name.ends_with(".old.exe") || name.ends_with(".bad.exe")) {
                let _ = std::fs::remove_file(entry.path());
            }
        }
    }
}

fn read(app: &App) -> Record {
    config::read_json(&app.config.update_path()).unwrap_or_default()
}

fn write(app: &App, record: &Record) {
    if let Err(error) = config::write_json_private(&app.config.update_path(), record) {
        tracing::warn!(%error, "saving update.json");
    }
}

// MARK: - Restarting

/// Whether this process can start the new binary itself: in place on macOS and Linux, or
/// through the supervisor `lorca service` runs on Windows.
fn can_restart() -> bool {
    cfg!(unix) || crate::service::supervised()
}

/// Restarts into `version` once this Runner is idle: no turn of its own in flight, no command a
/// bot left running, and what it queued for the relay sent (or ten seconds passed). Jobs that
/// arrive meanwhile run first; a job that lands while the restart starts stays unread on the
/// relay, and the new process takes it. A command still running a day later is stopped, as
/// quitting stops it, rather than hold the update back for good.
async fn restart_when_idle(app: Arc<App>, version: String) {
    if !can_restart() {
        tracing::info!(%version, "installed; restart lorca serve to run it");
        return;
    }
    let waiting = tokio::time::Instant::now();
    loop {
        let patient = || waiting.elapsed() < Duration::from_secs(DAY);
        while !is_idle(&app, patient()) {
            tokio::time::sleep(Duration::from_secs(5)).await;
        }
        let flushed = tokio::time::Instant::now();
        while flushed.elapsed() < Duration::from_secs(10) && app.store.first_outbox().ok().flatten().is_some() {
            tokio::time::sleep(Duration::from_millis(250)).await;
        }
        if !patient() {
            app.shell_sessions.shutdown(&app);
        }
        app.save_state_now();
        let Some(error) = restart_if_idle(&app, &version, patient()) else { continue };
        tracing::error!(%error, "restarting into the new release");
        set_status(&app, |status| {
            status.state = "installed".into();
            status.error = Some(format!("Lorca {version} is installed, but lorca serve could not restart into it: {error}"));
        });
        return;
    }
}

/// Restarts unless a job began since the wait ended. No job begins while this holds the list,
/// and the restart replaces the process; answers why it could not.
fn restart_if_idle(app: &App, version: &str, patient: bool) -> Option<String> {
    let jobs = app.running_jobs.lock().unwrap();
    if !jobs.values().all(|job| job.runner_id.is_some()) || (patient && !app.shell_sessions.is_empty()) {
        return None;
    }
    tracing::info!(%version, "restarting into the new release");
    Some(restart())
}

/// No turn of this Runner's own in flight (a job sent to another Runner waits there, and the
/// new process picks up its wait), and, while `patient`, no command a bot left running.
fn is_idle(app: &App, patient: bool) -> bool {
    app.running_jobs.lock().unwrap().values().all(|job| job.runner_id.is_some()) && (!patient || app.shell_sessions.is_empty())
}

/// Starts the binary now in place with this process's arguments. Returns only when it could not.
fn restart() -> String {
    #[cfg(unix)]
    {
        use std::os::unix::process::CommandExt;
        let exe = match current_exe() {
            Ok(exe) => exe,
            Err(error) => return error,
        };
        // The listening socket and every file the process holds close on exec.
        let error = std::process::Command::new(exe).args(std::env::args_os().skip(1)).exec();
        error.to_string()
    }
    #[cfg(windows)]
    {
        if crate::service::supervised() {
            std::process::exit(RESTART_EXIT);
        }
        "only lorca service restarts lorca serve on Windows".into()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ed25519_dalek::Signer;

    fn key() -> (ed25519_dalek::SigningKey, String) {
        let key = ed25519_dalek::SigningKey::from_bytes(&[7; 32]);
        let public = base64::engine::general_purpose::STANDARD.encode(key.verifying_key().as_bytes());
        (key, public)
    }

    #[test]
    fn a_manifest_needs_a_signature_by_a_trusted_key() {
        let (key, public) = key();
        let manifest = br#"{"kind":"lorca-cli","version":"9.0.0","builds":{}}"#;
        let signature = base64::engine::general_purpose::STANDARD.encode(key.sign(manifest).to_bytes());
        assert!(verify(manifest, signature.as_bytes(), &[public.clone()]).is_ok());
        assert!(verify(manifest, format!("{signature}\n").as_bytes(), &[public.clone()]).is_ok(), "a trailing newline is fine");
        assert!(verify(br#"{"kind":"lorca-cli","version":"9.0.1","builds":{}}"#, signature.as_bytes(), &[public.clone()]).is_err(), "another file");
        let other = ed25519_dalek::SigningKey::from_bytes(&[8; 32]);
        let forged = base64::engine::general_purpose::STANDARD.encode(other.sign(manifest).to_bytes());
        assert!(verify(manifest, forged.as_bytes(), &[public.clone()]).is_err(), "another key");
        assert!(verify(manifest, b"not base64", &[public]).is_err());
    }

    #[test]
    fn versions_compare_by_number() {
        assert!(is_newer("0.1.11", "0.1.10"));
        assert!(is_newer("0.2.0", "0.1.99"));
        assert!(is_newer("1.0", "0.9.9"));
        assert!(!is_newer("0.1.10", "0.1.10"));
        assert!(!is_newer("0.1.9", "0.1.10"));
    }

    #[test]
    fn the_binary_comes_out_of_a_release_archive() {
        let mut tar = tar::Builder::new(flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::default()));
        let mut header = tar::Header::new_ustar();
        header.set_size(5);
        header.set_mode(0o755);
        header.set_cksum();
        tar.append_data(&mut header, format!("lorca{}", std::env::consts::EXE_SUFFIX), &b"hello"[..]).unwrap();
        let archive = tar.into_inner().unwrap().finish().unwrap();
        assert_eq!(unpack("lorca-cli-linux-x86_64.tar.gz", &archive).unwrap(), b"hello");
        assert!(unpack("lorca-cli-linux-x86_64.tar.gz", b"junk").is_err());
    }

    #[cfg(unix)]
    #[test]
    fn a_swap_keeps_the_binary_from_before() {
        let dir = std::env::temp_dir().join(format!("lorca-swap-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&dir).unwrap();
        let exe = dir.join("lorca");
        let staged = dir.join(".lorca-new");
        std::fs::write(&exe, "old").unwrap();
        std::fs::write(&staged, "new").unwrap();
        let previous = swap(&exe, &staged).unwrap();
        assert_eq!(std::fs::read_to_string(&exe).unwrap(), "new");
        assert_eq!(std::fs::read_to_string(&previous).unwrap(), "old");
        assert!(!staged.exists());
        put_back(&previous, &exe).unwrap();
        assert_eq!(std::fs::read_to_string(&exe).unwrap(), "old");
        let _ = std::fs::remove_dir_all(dir);
    }
}
