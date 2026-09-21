//! Reconciliation: compute the desired label for every tab and apply it when
//! the tab is ours to name.

use crate::git;
use crate::herdr::{Pane, Tab};
use crate::label::{self, Ctx};
use crate::state::{self, Decision};
use crate::{config::Config, herdr};
use std::path::Path;

/// The Herdr operations reconciliation needs, so tests can drive a fake.
pub trait Herdr {
    fn pane_list(&self) -> Result<Vec<Pane>, String>;
    fn tab_list(&self) -> Result<Vec<Tab>, String>;
    fn tab_rename(&self, tab_id: &str, label: &str) -> Result<(), String>;
}

pub struct LiveHerdr;

impl Herdr for LiveHerdr {
    fn pane_list(&self) -> Result<Vec<Pane>, String> {
        herdr::pane_list()
    }
    fn tab_list(&self) -> Result<Vec<Tab>, String> {
        herdr::tab_list()
    }
    fn tab_rename(&self, tab_id: &str, label: &str) -> Result<(), String> {
        herdr::tab_rename(tab_id, label)
    }
}

#[derive(Debug, Default, PartialEq, Eq)]
pub struct Outcome {
    pub tabs: usize,
    pub renamed: Vec<Change>,
    pub claimed: usize,
    pub manual: usize,
    pub errors: Vec<String>,
    /// True when the ownership map changed and should be persisted.
    pub changed: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Change {
    pub tab_id: String,
    pub from: String,
    pub to: String,
}

/// Compute the desired label for a pane's working directory.
pub fn compute_label(cfg: &Config, dir: &str, pane: &Pane) -> String {
    let path = Path::new(dir);
    let dir_name = directory_name(path, dir);
    let repo = git::resolve(path);
    let ctx = Ctx {
        repo: repo.as_ref().map(|r| r.name.clone()),
        dir: dir_name.clone(),
        branch: repo.as_ref().and_then(|r| r.branch.clone()),
        ticket: repo
            .as_ref()
            .and_then(|r| r.branch.as_deref())
            .and_then(|b| label::extract_ticket(b, &cfg.ticket_prefixes)),
        worktree: repo.as_ref().and_then(|r| r.worktree.clone()),
        agent: pane.agent.clone(),
        status: pane.agent_status.clone(),
    };
    let rendered = label::render(&cfg.template, &ctx);
    let chosen = if rendered.is_empty() {
        dir_name
    } else {
        rendered
    };
    label::truncate(&chosen, cfg.max_length)
}

fn directory_name(path: &Path, raw: &str) -> String {
    if let Some(home) = crate::config::home_dir() {
        let same =
            path == home.as_path() || path.canonicalize().ok().as_deref() == Some(home.as_path());
        if same {
            return "~".to_string();
        }
    }
    path.file_name()
        .map(|s| s.to_string_lossy().into_owned())
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| raw.to_string())
}

/// Pick the pane that represents a tab: prefer the focused one, then the
/// lowest pane id for determinism. Panes with no resolvable directory lose.
fn pane_for_tab<'a>(panes: &'a [Pane], tab: &Tab) -> Option<&'a Pane> {
    panes
        .iter()
        .filter(|p| p.tab_id == tab.tab_id)
        .filter(|p| p.workdir().is_some())
        .min_by_key(|p| (!p.focused, p.pane_id.as_str()))
}

pub fn reconcile(
    api: &dyn Herdr,
    cfg: &Config,
    state: &mut state::State,
    first_run: bool,
    overwrite_manual: bool,
) -> Result<Outcome, String> {
    let panes = api.pane_list()?;
    let tabs = api.tab_list()?;
    let mut outcome = Outcome {
        tabs: tabs.len(),
        ..Outcome::default()
    };

    for tab in &tabs {
        let Some(pane) = pane_for_tab(&panes, tab) else {
            continue;
        };
        let Some(dir) = pane.workdir() else {
            continue;
        };
        let desired = compute_label(cfg, dir, pane);
        if desired.is_empty() {
            continue;
        }

        // A label that already equals what we would set is ours to claim, even
        // if we have no record of writing it (e.g. after a state reset).
        if desired == tab.label {
            if state.labels.get(&tab.tab_id) != Some(&desired) {
                state.labels.insert(tab.tab_id.clone(), desired);
                outcome.changed = true;
            }
            outcome.claimed += 1;
            continue;
        }

        let recorded = state.labels.get(&tab.tab_id).map(String::as_str);
        if state::decide(&tab.label, recorded, first_run, overwrite_manual) == Decision::Manual {
            outcome.manual += 1;
            continue;
        }

        match api.tab_rename(&tab.tab_id, &desired) {
            Ok(()) => {
                state.labels.insert(tab.tab_id.clone(), desired.clone());
                outcome.changed = true;
                outcome.renamed.push(Change {
                    tab_id: tab.tab_id.clone(),
                    from: tab.label.clone(),
                    to: desired,
                });
            }
            Err(e) => outcome.errors.push(format!("{}: {e}", tab.tab_id)),
        }
    }

    Ok(outcome)
}
