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
            .or_else(|| std::env::var_os("LORCA_HOME").map(PathBuf::from))
            .unwrap_or_else(|| dirs::home_dir().unwrap_or_else(|| PathBuf::from(".")).join(".lorca"));
        let port = port_override
            .or_else(|| std::env::var("LORCA_PORT").ok().and_then(|p| p.parse().ok()))
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
    pub fn database_path(&self) -> PathBuf {
        self.home.join("lorca.sqlite3")
    }
    pub fn settings_path(&self) -> PathBuf {
        self.home.join("settings.json")
    }

    /// Attachment bytes by id, sent from here or fetched from the relay.
    pub fn files_dir(&self) -> PathBuf {
        self.home.join("files")
    }

    /// Installed plugins: `installed.json`, `secrets.json`, and a folder per plugin.
    pub fn plugins_dir(&self) -> PathBuf {
        self.home.join("plugins")
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
    /// A marketplace index to list beside the bundled one.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub marketplace_url: Option<String>,
}

impl Settings {
    pub fn load(config: &Config) -> Self {
        read_json(&config.settings_path()).unwrap_or_default()
    }

    pub fn save(&self, config: &Config) -> anyhow::Result<()> {
        write_json_private(&config.settings_path(), self)
    }

    /// `LORCA_RELAY_URL` wins over the saved setting.
    pub fn effective_relay_url(&self) -> Option<String> {
        std::env::var("LORCA_RELAY_URL")
            .ok()
            .filter(|s| !s.trim().is_empty())
            .or_else(|| self.relay_url.clone())
            .map(|url| url.trim().trim_end_matches('/').to_string())
            .filter(|url| !url.is_empty())
    }
}

/// The relay the launching app ships with (`LORCA_DEFAULT_RELAY_URL`), used when nothing else
/// names one.
pub fn default_relay_url() -> Option<String> {
    std::env::var("LORCA_DEFAULT_RELAY_URL")
        .ok()
        .map(|url| url.trim().trim_end_matches('/').to_string())
        .filter(|url| !url.is_empty())
}

/// The relay port `bun run dev` and `bun run relay` listen on.
pub const DEV_RELAY_PORT: u16 = 8787;

/// In dev (`LORCA_DEV=1`, set by the dev loop), a Device with no relay configured uses the
/// relay the dev loop runs on this machine, addressed by this computer's LAN IP so a phone on the
/// same network can reach it through the pairing code.
pub fn dev_relay_url() -> Option<String> {
    if std::env::var("LORCA_DEV").ok().filter(|v| !v.is_empty() && v != "0").is_none() {
        return None;
    }
    let host = lan_ip().map(|ip| ip.to_string()).unwrap_or_else(|| "127.0.0.1".into());
    Some(format!("http://{host}:{DEV_RELAY_PORT}"))
}

/// This machine's address on the local network: a private IPv4 on a real interface (Wi-Fi or
/// Ethernet), never a VPN tunnel, which a default route would pick. Falls back to the source
/// address of a route to a public host.
pub fn lan_ip() -> Option<std::net::IpAddr> {
    let mut candidates: Vec<(u8, std::net::Ipv4Addr)> = Vec::new();
    for iface in if_addrs::get_if_addrs().unwrap_or_default() {
        let std::net::IpAddr::V4(ip) = iface.ip() else { continue };
        if ip.is_loopback() || ip.is_link_local() || ip.is_unspecified() {
            continue;
        }
        let name = iface.name.to_lowercase();
        if ["utun", "tun", "tap", "bridge", "docker", "vmnet", "awdl", "llw", "ipsec", "ppp", "wg", "zt"].iter().any(|p| name.starts_with(p)) {
            continue;
        }
        let rank = if ip.is_private() { 0 } else { 1 };
        candidates.push((rank, ip));
    }
    candidates.sort();
    if let Some((_, ip)) = candidates.first() {
        return Some(std::net::IpAddr::V4(*ip));
    }
    let socket = std::net::UdpSocket::bind("0.0.0.0:0").ok()?;
    socket.connect("1.1.1.1:80").ok()?;
    let ip = socket.local_addr().ok()?.ip();
    if ip.is_loopback() || ip.is_unspecified() {
        None
    } else {
        Some(ip)
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
