//! Herdr plugin that keeps tab labels in sync with the repo and ticket each
//! pane is working in.

mod config;
mod git;
mod herdr;
mod label;
mod state;
mod sync;
mod watcher;

#[cfg(test)]
mod test_support;
#[cfg(test)]
mod tests;

use std::process::ExitCode;
use std::time::Duration;
use sync::LiveHerdr;

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().collect();
    match args.get(1).map(String::as_str) {
        Some("startup") | Some("event") => match watcher::ensure() {
            Ok(()) => ExitCode::SUCCESS,
            Err(e) => {
                eprintln!("herdr-tab-naming: {e}");
                ExitCode::FAILURE
            }
        },
        Some("sync") => run_sync(false),
        Some("force-sync") => run_sync(true),
        Some("status") => {
            print_status();
            ExitCode::SUCCESS
        }
        Some("watch") => ExitCode::from(watcher::run() as u8),
        Some("restart") => {
            watcher::stop();
            std::thread::sleep(Duration::from_millis(1500));
            match watcher::ensure() {
                Ok(()) => ExitCode::SUCCESS,
                Err(e) => {
                    eprintln!("herdr-tab-naming: {e}");
                    ExitCode::FAILURE
                }
            }
        }
        Some("stop") => {
            watcher::stop();
            ExitCode::SUCCESS
        }
        Some("help") | Some("--help") | Some("-h") | None => {
            print_usage();
            ExitCode::SUCCESS
        }
        Some(other) => {
            eprintln!("herdr-tab-naming: unknown command '{other}'\n");
            print_usage();
            ExitCode::from(2)
        }
    }
}

fn run_sync(force: bool) -> ExitCode {
    let cfg = config::load();
    let (mut owned, first_run) = state::load();
    match sync::reconcile(&LiveHerdr, &cfg, &mut owned, first_run, force) {
        Ok(outcome) => {
            for change in &outcome.renamed {
                println!("{} '{}' -> '{}'", change.tab_id, change.from, change.to);
            }
            for error in &outcome.errors {
                eprintln!("herdr-tab-naming: {error}");
            }
            if outcome.changed {
                if let Err(e) = state::save(&owned) {
                    eprintln!("herdr-tab-naming: save state: {e}");
                }
            }
            println!(
                "{} tabs: {} renamed, {} already correct, {} manual, {} errors",
                outcome.tabs,
                outcome.renamed.len(),
                outcome.claimed,
                outcome.manual,
                outcome.errors.len()
            );
            if outcome.errors.is_empty() {
                ExitCode::SUCCESS
            } else {
                ExitCode::FAILURE
            }
        }
        Err(e) => {
            eprintln!("herdr-tab-naming: {e}");
            ExitCode::FAILURE
        }
    }
}

fn print_status() {
    let cfg = config::load();
    let (owned, first_run) = state::load();
    println!("config : {}", config::config_path().display());
    println!("state  : {}", state::labels_path().display());
    match state::read_pid() {
        Some(pid) => println!(
            "watcher: pid {pid} ({})",
            if state::pid_alive(pid) {
                "alive"
            } else {
                "dead"
            }
        ),
        None => println!("watcher: not running"),
    }

    let (panes, tabs) = match (herdr::pane_list(), herdr::tab_list()) {
        (Ok(panes), Ok(tabs)) => (panes, tabs),
        (Err(e), _) | (_, Err(e)) => {
            eprintln!("herdr-tab-naming: {e}");
            return;
        }
    };

    println!("\n{:<10} {:<30} {:<30} OWNER", "TAB", "CURRENT", "DESIRED");
    for tab in &tabs {
        let pane = panes
            .iter()
            .filter(|p| p.tab_id == tab.tab_id)
            .min_by_key(|p| (!p.focused, p.pane_id.as_str()));
        let desired = pane
            .and_then(|p| p.workdir().map(|d| sync::compute_label(&cfg, d, p)))
            .unwrap_or_default();
        let recorded = owned.labels.get(&tab.tab_id).map(String::as_str);
        let owner = match state::decide(&tab.label, recorded, first_run, false) {
            state::Decision::Manual => "manual",
            state::Decision::Ours if desired == tab.label => "ours",
            state::Decision::Ours => "ours*",
        };
        println!(
            "{:<10} {:<30} {:<30} {}",
            tab.tab_id,
            clip(&tab.label, 29),
            clip(&desired, 29),
            owner
        );
    }
}

fn clip(value: &str, max: usize) -> String {
    if value.chars().count() <= max {
        return value.to_string();
    }
    let mut out: String = value.chars().take(max.saturating_sub(1)).collect();
    out.push('…');
    out
}

fn print_usage() {
    println!(
        "herdr-tab-naming {version}

Keep Herdr tab labels in sync with the repo and ticket each pane is working in.

USAGE:
    herdr-tab-naming <COMMAND>

COMMANDS:
    startup      Ensure the watcher is running (manifest startup hook)
    event        Ensure the watcher is running (manifest event hook)
    sync         Reconcile tab labels once
    force-sync   Re-adopt every tab, overwriting manual names
    status       Show current vs desired labels without changing anything
    watch        Run the resident watcher (spawned by `startup`)
    restart      Stop and start the watcher
    stop         Ask the watcher to exit

Config: {config}
",
        version = env!("CARGO_PKG_VERSION"),
        config = config::config_path().display(),
    );
}
