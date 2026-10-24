//! `config.json` in the data folder: settings that must survive restarts.

use serde::{Deserialize, Serialize};
use windows::Win32::Security::Cryptography::{BCRYPT_USE_SYSTEM_PREFERRED_RNG, BCryptGenRandom};

use crate::DisplayMode;
use crate::paths;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Config {
    /// Bearer token for the local MCP endpoint. The stdio shim reads it from here, so agents'
    /// config files never contain it.
    pub token: String,
    /// Keep the user's mouse out of the virtual display (tray menu).
    #[serde(default = "on")]
    pub fence: bool,
    /// The virtual display's resolution.
    #[serde(default)]
    pub mode: DisplayMode,
}

fn on() -> bool {
    true
}

/// Reads the config without creating anything (the stdio shim uses this; the app owns the file).
pub fn load() -> Option<Config> {
    let path = paths::data_dir().join("config.json");
    std::fs::read(path).ok().and_then(|bytes| serde_json::from_slice(&bytes).ok())
}

/// Reads the config, creating it (with a fresh random token) on first run.
pub fn load_or_create() -> Config {
    load().unwrap_or_else(|| {
        let config = Config { token: random_token(), fence: true, mode: DisplayMode::DEFAULT };
        save(&config);
        config
    })
}

pub fn save(config: &Config) {
    let path = paths::data_dir().join("config.json");
    match serde_json::to_vec_pretty(config) {
        Ok(bytes) => {
            if let Err(err) = std::fs::write(&path, bytes) {
                tracing::warn!(?path, %err, "could not save config");
            }
        }
        Err(err) => tracing::warn!(%err, "could not serialize config"),
    }
}

fn random_token() -> String {
    let mut bytes = [0u8; 32];
    let status = unsafe { BCryptGenRandom(None, &mut bytes, BCRYPT_USE_SYSTEM_PREFERRED_RNG) };
    assert!(status.is_ok(), "the system random number generator failed: {status:?}");
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}
