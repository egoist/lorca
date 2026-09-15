//! Where the CLI keeps things and how it is configured.

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

pub const DEFAULT_PORT: u16 = 4862;
pub const VERSION: &str = env!("CARGO_PKG_VERSION");

#[derive(Debug, Clone)]
pub struct Config {
    pub home: PathBuf,
    pub port: u16,
}

impl Config {
    pub fn load(home_override: Option<PathBuf>, port_override: Option<u16>) -> Self {
        let home = home_override
            .or_else(|| std::env::var_os("TINYBOT_HOME").map(PathBuf::from))
            .unwrap_or_else(|| dirs::home_dir().unwrap_or_else(|| PathBuf::from(".")).join(".tinybot"));
        let port = port_override
            .or_else(|| std::env::var("TINYBOT_PORT").ok().and_then(|p| p.parse().ok()))
            .unwrap_or(DEFAULT_PORT);
        Config { home, port }
    }

    pub fn identity_path(&self) -> PathBuf {
        self.home.join("identity.json")
    }
    pub fn machine_path(&self) -> PathBuf {
        self.home.join("machine.json")
    }
    pub fn credentials_path(&self) -> PathBuf {
        self.home.join("credentials.json")
    }
    pub fn state_path(&self) -> PathBuf {
        self.home.join("state.json")
    }
    pub fn settings_path(&self) -> PathBuf {
        self.home.join("settings.json")
    }

    pub fn ensure_home(&self) -> anyhow::Result<()> {
        std::fs::create_dir_all(&self.home)?;
        set_private(&self.home)?;
        Ok(())
    }
}

/// Persistent settings the app can change at runtime.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Settings {
    #[serde(default)]
    pub relay_url: Option<String>,
}

impl Settings {
    pub fn load(config: &Config) -> Self {
        read_json(&config.settings_path()).unwrap_or_default()
    }

    pub fn save(&self, config: &Config) -> anyhow::Result<()> {
        write_json_private(&config.settings_path(), self)
    }

    /// `TINYBOT_RELAY_URL` wins over the saved setting.
    pub fn effective_relay_url(&self) -> Option<String> {
        std::env::var("TINYBOT_RELAY_URL")
            .ok()
            .filter(|s| !s.trim().is_empty())
            .or_else(|| self.relay_url.clone())
            .map(|url| url.trim().trim_end_matches('/').to_string())
            .filter(|url| !url.is_empty())
    }
}

pub fn read_json<T: serde::de::DeserializeOwned>(path: &Path) -> Option<T> {
    let bytes = std::fs::read(path).ok()?;
    serde_json::from_slice(&bytes).ok()
}

/// Writes atomically with mode 0600.
pub fn write_json_private<T: Serialize>(path: &Path, value: &T) -> anyhow::Result<()> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let bytes = serde_json::to_vec_pretty(value)?;
    static COUNTER: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
    let n = COUNTER.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    let tmp = path.with_extension(format!("tmp-{}-{n}", std::process::id()));
    std::fs::write(&tmp, bytes)?;
    set_private(&tmp)?;
    std::fs::rename(&tmp, path)?;
    Ok(())
}

#[cfg(unix)]
pub fn set_private(path: &Path) -> anyhow::Result<()> {
    use std::os::unix::fs::PermissionsExt;
    let metadata = std::fs::metadata(path)?;
    let mode = if metadata.is_dir() { 0o700 } else { 0o600 };
    std::fs::set_permissions(path, std::fs::Permissions::from_mode(mode))?;
    Ok(())
}

#[cfg(not(unix))]
pub fn set_private(_path: &Path) -> anyhow::Result<()> {
    Ok(())
}

pub fn now_secs() -> f64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs_f64())
        .unwrap_or(0.0)
}

pub fn now_unix() -> i64 {
    now_secs() as i64
}
