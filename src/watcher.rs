//! The resident watcher: one process that keeps labels current.
//!
//! Herdr emits no event for a working-directory change, so the watcher both
//! subscribes to structural events (instant) and reconciles on a timer (the
//! `cd` safety net). Structural event hooks in the manifest only ensure this
//! process is alive.

use crate::config;
use crate::herdr;
use crate::state;
use crate::sync::{self, LiveHerdr};
use std::io::{BufRead, BufReader};
use std::process::{Command, Stdio};
use std::sync::mpsc::{self, RecvTimeoutError};
use std::thread;
use std::time::Duration;

/// Start the watcher if it is not already running. Safe to call repeatedly.
pub fn ensure() -> Result<(), String> {
    if let Some(pid) = state::read_pid() {
        if state::pid_is_watcher(pid) {
            return Ok(());
        }
    }
    state::clear_pid();

    let exe = std::env::current_exe().map_err(|e| format!("current_exe: {e}"))?;
    let log_path = state::log_path();
    if let Some(parent) = log_path.parent() {
        std::fs::create_dir_all(parent).map_err(|e| format!("create {}: {e}", parent.display()))?;
    }
    let stdout = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(&log_path)
        .map_err(|e| format!("open {}: {e}", log_path.display()))?;
    let stderr = stdout.try_clone().map_err(|e| e.to_string())?;

    let child = Command::new(&exe)
        .arg("watch")
        .stdin(Stdio::null())
        .stdout(Stdio::from(stdout))
        .stderr(Stdio::from(stderr))
        .spawn()
        .map_err(|e| format!("spawn watcher: {e}"))?;
    state::log(&format!("ensure: spawned watcher pid {}", child.id()));
    Ok(())
}

/// Ask the watcher to exit.
pub fn stop() {
    state::request_stop();
}

/// Take the pid lock, or `None` when another live watcher holds it.
///
/// A pid file that exists but is not yet readable is treated as owned: another
/// watcher is mid-startup, so we must not delete its lock and race it.
fn acquire_lock() -> Option<std::fs::File> {
    use std::io::Write;
    for _ in 0..2 {
        match std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(state::pid_path())
        {
            Ok(mut file) => {
                let _ = write!(file, "{}", std::process::id());
                let _ = file.flush();
                return Some(file);
            }
            Err(_) => match state::read_pid() {
                Some(pid) if state::pid_alive(pid) => return None,
                // Stale lock from a dead watcher: clear and retry.
                Some(_) => {
                    let _ = std::fs::remove_file(state::pid_path());
                }
                // Unreadable/empty: another watcher is claiming it.
                None => return None,
            },
        }
    }
    None
}

pub fn run() -> i32 {
    let Some(_lock) = acquire_lock() else {
        state::log("watch: another watcher is live; exiting");
        return 0;
    };
    state::clear_stop();
    let me = std::process::id();
    state::log(&format!("watch: started pid {me}"));

    let api = LiveHerdr;
    let (mut owned, first_run) = state::load();
    let mut cfg = config::load();
    reconcile_once(&api, &cfg, &mut owned, first_run);

    'outer: loop {
        if state::stop_requested() {
            state::log("watch: stop requested");
            break;
        }
        if superseded(me) {
            state::log("watch: superseded by a newer watcher; exiting");
            break;
        }

        let stream = match herdr::subscribe() {
            Ok(stream) => stream,
            Err(e) => {
                state::log(&format!("watch: subscribe failed: {e}"));
                sleep_cancellable(Duration::from_secs(1));
                continue;
            }
        };
        state::log("watch: subscribed to herdr events");

        let (tx, rx) = mpsc::channel();
        let reader = thread::spawn(move || {
            let mut reader = BufReader::new(stream);
            loop {
                let mut line = String::new();
                match reader.read_line(&mut line) {
                    Ok(0) | Err(_) => break,
                    Ok(_) => {
                        if tx.send(()).is_err() {
                            break;
                        }
                    }
                }
            }
        });

        loop {
            if state::stop_requested() {
                break 'outer;
            }
            cfg = config::load();
            match rx.recv_timeout(Duration::from_millis(cfg.poll_ms)) {
                Ok(()) => {
                    // Coalesce the burst of events a single action produces.
                    sleep_cancellable(Duration::from_millis(cfg.debounce_ms));
                    while rx.try_recv().is_ok() {}
                    reconcile_once(&api, &cfg, &mut owned, false);
                }
                Err(RecvTimeoutError::Timeout) => reconcile_once(&api, &cfg, &mut owned, false),
                Err(RecvTimeoutError::Disconnected) => {
                    state::log("watch: subscription ended; reconnecting");
                    break;
                }
            }
        }
        drop(reader);
    }

    state::clear_pid();
    state::log("watch: stopped");
    0
}

fn superseded(me: u32) -> bool {
    matches!(state::read_pid(), Some(pid) if pid != me && state::pid_is_watcher(pid))
}

fn reconcile_once(
    api: &dyn sync::Herdr,
    cfg: &config::Config,
    owned: &mut state::State,
    first_run: bool,
) {
    match sync::reconcile(api, cfg, owned, first_run, false) {
        Ok(outcome) => {
            for change in &outcome.renamed {
                state::log(&format!(
                    "watch: {} '{}' -> '{}'",
                    change.tab_id, change.from, change.to
                ));
            }
            for error in &outcome.errors {
                state::log(&format!("watch: error: {error}"));
            }
            if outcome.changed {
                if let Err(e) = state::save(owned) {
                    state::log(&format!("watch: save state: {e}"));
                }
            }
        }
        Err(e) => state::log(&format!("watch: reconcile failed: {e}")),
    }
}

fn sleep_cancellable(total: Duration) {
    let step = Duration::from_millis(50);
    let mut left = total;
    while left > Duration::ZERO {
        if state::stop_requested() {
            return;
        }
        let nap = left.min(step);
        thread::sleep(nap);
        left = left.saturating_sub(nap);
    }
}
