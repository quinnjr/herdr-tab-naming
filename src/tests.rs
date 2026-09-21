//! Reconcile-level tests driven through the fake Herdr.

use crate::config::Config;
use crate::state::{self, State};
use crate::sync;
use crate::test_support::{pane, tab, FakeHerdr};
use std::path::{Path, PathBuf};
use tempfile::TempDir;

/// Build `root/<name>` as a repository on `branch`.
fn repo(tmp: &TempDir, name: &str, branch: &str) -> PathBuf {
    let root = tmp.path().join(name);
    std::fs::create_dir_all(root.join(".git")).unwrap();
    std::fs::write(
        root.join(".git/HEAD"),
        format!("ref: refs/heads/{branch}\n"),
    )
    .unwrap();
    root
}

fn reconcile(
    api: &FakeHerdr,
    state: &mut State,
    first_run: bool,
    overwrite: bool,
) -> sync::Outcome {
    sync::reconcile(api, &Config::default(), state, first_run, overwrite).unwrap()
}

#[test]
fn numeric_default_is_replaced_with_repo_name() {
    let tmp = TempDir::new().unwrap();
    let root = repo(&tmp, "llama-rs", "develop");
    let api = FakeHerdr::new(
        vec![pane("w1:p1", "w1:t1", root.to_str().unwrap())],
        vec![tab("w1:t1", "1")],
    );
    let mut state = State::default();

    let outcome = reconcile(&api, &mut state, false, false);

    assert_eq!(
        api.renames.borrow().as_slice(),
        [("w1:t1".into(), "llama-rs".into())]
    );
    assert_eq!(outcome.renamed.len(), 1);
    assert!(outcome.changed);
    assert_eq!(
        state.labels.get("w1:t1").map(String::as_str),
        Some("llama-rs")
    );
}

#[test]
fn ticket_is_appended_on_feature_branch() {
    let tmp = TempDir::new().unwrap();
    let root = repo(&tmp, "llama-rs", "feature/BACK-565-thing");
    let api = FakeHerdr::new(
        vec![pane("w1:p1", "w1:t1", root.to_str().unwrap())],
        vec![tab("w1:t1", "1")],
    );
    let mut state = State::default();

    reconcile(&api, &mut state, false, false);

    assert_eq!(api.renames.borrow()[0].1, "llama-rs BACK-565");
}

#[test]
fn manual_label_is_left_alone() {
    let tmp = TempDir::new().unwrap();
    let root = repo(&tmp, "llama-rs", "develop");
    let api = FakeHerdr::new(
        vec![pane("w1:p1", "w1:t1", root.to_str().unwrap())],
        vec![tab("w1:t1", "my-hand-name")],
    );
    let mut state = State::default();

    let outcome = reconcile(&api, &mut state, false, false);

    assert_eq!(api.rename_count(), 0);
    assert_eq!(outcome.manual, 1);
    assert!(!outcome.changed);
}

#[test]
fn first_run_adopts_manual_labels() {
    let tmp = TempDir::new().unwrap();
    let root = repo(&tmp, "llama-rs", "develop");
    let api = FakeHerdr::new(
        vec![pane("w1:p1", "w1:t1", root.to_str().unwrap())],
        vec![tab("w1:t1", "my-hand-name")],
    );
    let mut state = State::default();

    let outcome = reconcile(&api, &mut state, true, false);

    assert_eq!(api.rename_count(), 1);
    assert_eq!(outcome.renamed.len(), 1);
}

#[test]
fn force_overwrites_manual_labels() {
    let tmp = TempDir::new().unwrap();
    let root = repo(&tmp, "llama-rs", "develop");
    let api = FakeHerdr::new(
        vec![pane("w1:p1", "w1:t1", root.to_str().unwrap())],
        vec![tab("w1:t1", "my-hand-name")],
    );
    let mut state = State::default();

    reconcile(&api, &mut state, false, true);

    assert_eq!(api.rename_count(), 1);
}

#[test]
fn stable_label_is_claimed_without_renaming() {
    let tmp = TempDir::new().unwrap();
    let root = repo(&tmp, "llama-rs", "develop");
    let api = FakeHerdr::new(
        vec![pane("w1:p1", "w1:t1", root.to_str().unwrap())],
        vec![tab("w1:t1", "llama-rs")],
    );
    let mut state = State::default();

    let outcome = reconcile(&api, &mut state, false, false);

    assert_eq!(api.rename_count(), 0);
    assert_eq!(outcome.claimed, 1);
    assert!(outcome.changed, "ownership should be recorded");
    assert_eq!(
        state.labels.get("w1:t1").map(String::as_str),
        Some("llama-rs")
    );
}

#[test]
fn focused_pane_wins_within_a_tab() {
    let tmp = TempDir::new().unwrap();
    let repo_a = repo(&tmp, "alpha", "develop");
    let repo_b = repo(&tmp, "beta", "develop");

    let mut pane_a = pane("w1:p1", "w1:t1", repo_a.to_str().unwrap());
    pane_a.focused = false;
    let mut pane_b = pane("w1:p2", "w1:t1", repo_b.to_str().unwrap());
    pane_b.focused = true;

    let api = FakeHerdr::new(vec![pane_a, pane_b], vec![tab("w1:t1", "1")]);
    let mut state = State::default();

    reconcile(&api, &mut state, false, false);

    assert_eq!(api.renames.borrow()[0].1, "beta");
}

#[test]
fn non_repo_falls_back_to_directory_name() {
    let tmp = TempDir::new().unwrap();
    let scratch = tmp.path().join("scratch");
    std::fs::create_dir_all(&scratch).unwrap();
    let api = FakeHerdr::new(
        vec![pane("w1:p1", "w1:t1", scratch.to_str().unwrap())],
        vec![tab("w1:t1", "1")],
    );
    let mut state = State::default();

    reconcile(&api, &mut state, false, false);

    assert_eq!(api.renames.borrow()[0].1, "scratch");
}

#[test]
fn worktree_labels_from_main_repo_plus_ticket() {
    let tmp = TempDir::new().unwrap();
    let root = repo(&tmp, "llama-rs", "develop");
    let wt_admin = root.join(".git/worktrees/BACK-565");
    std::fs::create_dir_all(&wt_admin).unwrap();
    std::fs::write(
        wt_admin.join("HEAD"),
        "ref: refs/heads/feature/BACK-565-thing\n",
    )
    .unwrap();
    let wt = root.join(".worktrees/BACK-565");
    std::fs::create_dir_all(&wt).unwrap();
    std::fs::write(wt.join(".git"), format!("gitdir: {}\n", wt_admin.display())).unwrap();

    let api = FakeHerdr::new(
        vec![pane("w1:p1", "w1:t1", wt.to_str().unwrap())],
        vec![tab("w1:t1", "1")],
    );
    let mut state = State::default();

    reconcile(&api, &mut state, false, false);

    assert_eq!(api.renames.borrow()[0].1, "llama-rs BACK-565");
}

#[test]
fn rename_failure_is_reported_and_not_recorded() {
    let tmp = TempDir::new().unwrap();
    let root = repo(&tmp, "llama-rs", "develop");
    let mut api = FakeHerdr::new(
        vec![pane("w1:p1", "w1:t1", root.to_str().unwrap())],
        vec![tab("w1:t1", "1")],
    );
    api.fail = true;
    let mut state = State::default();

    let outcome = reconcile(&api, &mut state, false, false);

    assert_eq!(outcome.errors.len(), 1);
    assert!(state.labels.is_empty());
}

#[test]
fn detect_manual_then_owned_transition() {
    let dir = Path::new("/tmp/whatever");
    assert!(state::is_default_label("12"));
    assert!(!state::is_default_label("12a"));
    assert_eq!(
        state::decide("BACK-565", Some("BACK-565"), false, false),
        state::Decision::Ours
    );
    let _ = dir;
}
