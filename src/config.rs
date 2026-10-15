//! Persistent user settings (`settings.json`) stored in the data directory.

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::path::Path;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct Config {
    /// Device IP (ESP8266 softAP gateway).
    pub host: String,
    /// TCP port of the device transfer server.
    pub port: u16,
    /// AP SSID advertised by the device.
    pub ssid: String,
    /// AP password.
    pub password: String,
    /// Optional command run before a transfer (`sh -c` / `cmd /C`).
    /// When set, it takes precedence over the built-in auto-connect.
    pub pre_connect_cmd: String,
    /// Try to join the AP automatically via OS tools (nmcli / netsh).
    pub auto_connect: bool,
    /// Match the old Android client, which sent a 0-based month.
    pub zero_based_month: bool,
    /// Seconds to wait for the device to become reachable after connecting.
    pub connect_timeout_secs: u64,
    /// Seconds to wait while reading the data stream.
    pub transfer_timeout_secs: u64,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            host: "192.168.4.1".to_string(),
            port: 31415,
            ssid: "NFCoffee".to_string(),
            password: "freecoffee".to_string(),
            pre_connect_cmd: String::new(),
            auto_connect: true,
            zero_based_month: true,
            connect_timeout_secs: 60,
            transfer_timeout_secs: 30,
        }
    }
}

impl Config {
    pub fn path(dir: &Path) -> std::path::PathBuf {
        dir.join("settings.json")
    }

    /// Load from `dir/settings.json`, creating a default file if missing.
    pub fn load(dir: &Path) -> Result<Self> {
        let path = Self::path(dir);
        if path.exists() {
            let text = std::fs::read_to_string(&path)
                .with_context(|| format!("reading {}", path.display()))?;
            let cfg: Config = serde_json::from_str(&text)
                .with_context(|| format!("parsing {}", path.display()))?;
            Ok(cfg)
        } else {
            let cfg = Config::default();
            cfg.save(dir).ok();
            Ok(cfg)
        }
    }

    pub fn save(&self, dir: &Path) -> Result<()> {
        std::fs::create_dir_all(dir).ok();
        let path = Self::path(dir);
        let text = serde_json::to_string_pretty(self)?;
        std::fs::write(&path, text)
            .with_context(|| format!("writing {}", path.display()))?;
        Ok(())
    }
}
