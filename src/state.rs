//! Persistent state: which labels this plugin owns, plus watcher pid/log
//! bookkeeping. The ownership record is what makes a hand-renamed tab stick.

use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct State {
    #[serde(default)]
    pub labels: HashMap<String, String>,
}

pub fn state_dir() -> PathBuf {
    if let Some(dir) = std::env::var_os("HERDR_PLUGIN_STATE_DIR") {
        return PathBuf::from(dir);
    }
    crate::config::home_dir()
        .map(|h| h.join(".local/state/herdr-tab-naming"))
        .unwrap_or_else(|| PathBuf::from(".herdr-tab-naming"))
}

pub fn labels_path() -> PathBuf {
    state_dir().join("labels.json")
}
pub fn pid_path() -> PathBuf {
    state_dir().join("watcher.pid")
}
pub fn log_path() -> PathBuf {
    state_dir().join("watcher.log")
}
pub fn stop_path() -> PathBuf {
    state_dir().join("stop")
}

/// `first_run` means we have never written labels, so every tab is fair game.
pub fn load() -> (State, bool) {
    let path = labels_path();
    let first_run = !path.exists();
    let state = std::fs::read_to_string(&path)
        .ok()
        .and_then(|text| serde_json::from_str(&text).ok())
        .unwrap_or_default();
    (state, first_run)
}

pub fn save(state: &State) -> Result<(), String> {
    let dir = state_dir();
    std::fs::create_dir_all(&dir).map_err(|e| format!("create {}: {e}", dir.display()))?;
    let path = labels_path();
    let text = serde_json::to_string_pretty(state).map_err(|e| e.to_string())?;
    std::fs::write(&path, text).map_err(|e| format!("write {}: {e}", path.display()))
}

// -- ownership ---------------------------------------------------------------

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Decision {
    /// We may (re)label this tab.
    Ours,
    /// A human set this label; leave it alone.
    Manual,
}

/// Decide whether a tab's current label belongs to us.
pub fn decide(
    current: &str,
    recorded: Option<&str>,
    first_run: bool,
    overwrite_manual: bool,
) -> Decision {
    if first_run || overwrite_manual {
        return Decision::Ours;
    }
    if recorded == Some(current) {
        return Decision::Ours;
    }
    if is_default_label(current) {
        return Decision::Ours;
    }
    Decision::Manual
}

/// Herdr's own un-renamed labels are bare numbers.
pub fn is_default_label(label: &str) -> bool {
    !label.is_empty() && label.chars().all(|c| c.is_ascii_digit())
}

// -- watcher process bookkeeping --------------------------------------------

pub fn read_pid() -> Option<u32> {
    std::fs::read_to_string(pid_path())
        .ok()?
        .trim()
        .parse()
        .ok()
}

pub fn clear_pid() {
    let _ = std::fs::remove_file(pid_path());
}

pub fn stop_requested() -> bool {
    stop_path().exists()
}

pub fn clear_stop() {
    let _ = std::fs::remove_file(stop_path());
}

pub fn request_stop() {
    let _ = std::fs::write(stop_path(), b"stop");
}

/// Is `pid` a live process?
pub fn pid_alive(pid: u32) -> bool {
    if pid == 0 {
        return false;
    }
    #[cfg(target_os = "linux")]
    {
        Path::new(&format!("/proc/{pid}")).exists()
    }
    #[cfg(not(target_os = "linux"))]
    {
        std::process::Command::new("kill")
            .args(["-0", &pid.to_string()])
            .status()
            .map(|s| s.success())
            .unwrap_or(false)
    }
}

/// Is the live pid actually our watcher? Guards against a recycled pid.
pub fn pid_is_watcher(pid: u32) -> bool {
    if !pid_alive(pid) {
        return false;
    }
    #[cfg(target_os = "linux")]
    {
        match std::fs::read(format!("/proc/{pid}/cmdline")) {
            Ok(bytes) => {
                let cmd = String::from_utf8_lossy(&bytes).replace('\0', " ");
                cmd.contains("herdr-tab-naming") && cmd.contains("watch")
            }
            // If procfs is unreadable, trust liveness rather than double-spawn.
            Err(_) => true,
        }
    }
    #[cfg(not(target_os = "linux"))]
    {
        true
    }
}

/// Append a line to the watcher log, ignoring failures.
pub fn log(message: &str) {
    let path = log_path();
    if let Some(parent) = path.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    let stamp = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    if let Ok(mut file) = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(&path)
    {
        use std::io::Write;
        let _ = writeln!(file, "{stamp} {message}");
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn first_run_adopts_everything() {
        assert_eq!(decide("BACK-565", None, true, false), Decision::Ours);
    }

    #[test]
    fn manual_labels_are_respected() {
        assert_eq!(
            decide("my-tab", Some("llama-rs"), false, false),
            Decision::Manual
        );
        assert_eq!(decide("my-tab", None, false, false), Decision::Manual);
    }

    #[test]
    fn owned_labels_stay_ours() {
        assert_eq!(
            decide("llama-rs", Some("llama-rs"), false, false),
            Decision::Ours
        );
    }

    #[test]
    fn numeric_default_labels_are_adopted() {
        assert_eq!(decide("3", None, false, false), Decision::Ours);
    }

    #[test]
    fn overwrite_manual_forces_adoption() {
        assert_eq!(
            decide("my-tab", Some("llama-rs"), false, true),
            Decision::Ours
        );
    }
}
