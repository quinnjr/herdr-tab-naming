//! Plugin configuration (`config.json` inside the plugin config dir).

use serde::{Deserialize, Serialize};
use std::path::PathBuf;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Config {
    /// Label template; see `label::Ctx` for tokens.
    #[serde(default = "default_template")]
    pub template: String,
    /// Truncate labels longer than this with an ellipsis. 0 disables.
    #[serde(default)]
    pub max_length: usize,
    /// Overwrite hand-renamed labels on every reconcile.
    #[serde(default)]
    pub overwrite_manual: bool,
    /// Watcher reconcile interval in milliseconds (the `cd` safety net).
    #[serde(default = "default_poll_ms")]
    pub poll_ms: u64,
    /// Quiet period that coalesces a burst of events, in milliseconds.
    #[serde(default = "default_debounce_ms")]
    pub debounce_ms: u64,
    /// Optional allowlist of ticket prefixes, e.g. `["BACK", "LIT"]`.
    #[serde(default)]
    pub ticket_prefixes: Vec<String>,
}

fn default_template() -> String {
    "{repo} {ticket}".to_string()
}
fn default_poll_ms() -> u64 {
    1000
}
fn default_debounce_ms() -> u64 {
    150
}

impl Default for Config {
    fn default() -> Self {
        Self {
            template: default_template(),
            max_length: 0,
            overwrite_manual: false,
            poll_ms: default_poll_ms(),
            debounce_ms: default_debounce_ms(),
            ticket_prefixes: Vec::new(),
        }
    }
}

pub fn config_dir() -> PathBuf {
    if let Some(dir) = std::env::var_os("HERDR_PLUGIN_CONFIG_DIR") {
        return PathBuf::from(dir);
    }
    home_dir()
        .map(|h| h.join(".config/herdr-tab-naming"))
        .unwrap_or_else(|| PathBuf::from(".herdr-tab-naming"))
}

pub fn config_path() -> PathBuf {
    config_dir().join("config.json")
}

pub fn home_dir() -> Option<PathBuf> {
    std::env::var_os("HOME").map(PathBuf::from)
}

/// Load config, falling back to defaults on a missing or malformed file.
pub fn load() -> Config {
    let path = config_path();
    let mut cfg = match std::fs::read_to_string(&path) {
        Ok(text) => serde_json::from_str(&text).unwrap_or_else(|e| {
            eprintln!("herdr-tab-naming: {}: {e}; using defaults", path.display());
            Config::default()
        }),
        Err(_) => Config::default(),
    };
    cfg.poll_ms = cfg.poll_ms.max(200);
    cfg
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn defaults_match_documented_smart_default() {
        let cfg = Config::default();
        assert_eq!(cfg.template, "{repo} {ticket}");
        assert_eq!(cfg.poll_ms, 1000);
        assert!(!cfg.overwrite_manual);
    }

    #[test]
    fn partial_json_keeps_defaults() {
        let cfg: Config = serde_json::from_str(r#"{"max_length": 20}"#).unwrap();
        assert_eq!(cfg.max_length, 20);
        assert_eq!(cfg.template, "{repo} {ticket}");
        assert_eq!(cfg.poll_ms, 1000);
    }
}
