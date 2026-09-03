//! Integration tests for the non-interactive CLI.

use std::path::Path;
use std::process::{Command, Output, Stdio};

use tempfile::tempdir;

fn wtc() -> Command {
    Command::new(env!("CARGO_BIN_EXE_wtc"))
}

fn git(cwd: &Path, args: &[&str]) -> Output {
    let out = Command::new("git")
        .args(args)
        .current_dir(cwd)
        .env("GIT_CONFIG_GLOBAL", "/dev/null")
        .env("GIT_CONFIG_SYSTEM", "/dev/null")
        .env("GIT_AUTHOR_NAME", "test")
        .env("GIT_AUTHOR_EMAIL", "test@example.com")
        .env("GIT_COMMITTER_NAME", "test")
        .env("GIT_COMMITTER_EMAIL", "test@example.com")
        .output()
        .expect("git should be runnable");
    assert!(out.status.success(), "git {args:?} failed");
    out
}

fn init_repo(path: &Path) {
    std::fs::create_dir_all(path).unwrap();
    git(path, &["init", "-q", "-b", "main"]);
}

fn commit(repo: &Path) {
    git(repo, &["commit", "-q", "--allow-empty", "-m", "init"]);
}

/// Commit at a fixed date, so a worktree checked out from it lands past the
/// staleness threshold and is classified `Stale`.
fn commit_at(repo: &Path, date: &str) {
    let out = Command::new("git")
        .args(["commit", "-q", "--allow-empty", "-m", "old"])
        .current_dir(repo)
        .env("GIT_CONFIG_GLOBAL", "/dev/null")
        .env("GIT_CONFIG_SYSTEM", "/dev/null")
        .env("GIT_AUTHOR_NAME", "test")
        .env("GIT_AUTHOR_EMAIL", "test@example.com")
        .env("GIT_COMMITTER_NAME", "test")
        .env("GIT_COMMITTER_EMAIL", "test@example.com")
        .env("GIT_AUTHOR_DATE", date)
        .env("GIT_COMMITTER_DATE", date)
        .output()
        .expect("git should be runnable");
    assert!(
        out.status.success(),
        "git commit failed: {}",
        String::from_utf8_lossy(&out.stderr)
    );
}

fn add_worktree(repo: &Path, path: &Path) {
    git(repo, &["worktree", "add", "-q", path.to_str().unwrap()]);
}

/// A stale linked worktree in its own repo: the only commit is well past the
/// staleness threshold, so `scan` classifies the worktree as `Stale`.
fn add_stale_worktree(repo: &Path, wt: &Path) {
    init_repo(repo);
    commit_at(repo, "2020-01-01T00:00:00");
    add_worktree(repo, wt);
}

#[test]
fn yes_dry_run_lists_without_deleting() {
    let tmp = tempdir().unwrap();
    let repo = tmp.path().join("repo");
    init_repo(&repo);
    commit(&repo);
    let wt_path = tmp.path().join("wt");
    add_worktree(&repo, &wt_path);
    std::fs::remove_dir_all(&repo).unwrap();

    let output = wtc()
        .args(["--yes", "--dry-run", tmp.path().to_str().unwrap()])
        .output()
        .unwrap();

    assert!(
        output.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        stdout.contains("would remove"),
        "expected dry-run output: {stdout}"
    );
    assert!(wt_path.exists(), "dry run must not delete anything");
}

#[test]
fn yes_removes_orphaned_worktree_without_tui() {
    let tmp = tempdir().unwrap();
    let repo = tmp.path().join("repo");
    init_repo(&repo);
    commit(&repo);
    let wt_path = tmp.path().join("wt");
    add_worktree(&repo, &wt_path);
    std::fs::remove_dir_all(&repo).unwrap();

    let output = wtc()
        .args(["--yes", tmp.path().to_str().unwrap()])
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .output()
        .unwrap();

    assert!(
        output.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(!wt_path.exists(), "orphaned worktree should be removed");
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        stdout.contains("removed"),
        "expected removal output: {stdout}"
    );
}

#[test]
fn dirty_worktree_skipped_without_force() {
    let tmp = tempdir().unwrap();
    let repo = tmp.path().join("repo");
    // Stale (so it matches the default filter) *and* dirty: the skip must be
    // driven by the local changes, not by the filter excluding it.
    let wt_path = tmp.path().join("wt");
    add_stale_worktree(&repo, &wt_path);
    std::fs::write(wt_path.join("scratch.txt"), "work in progress").unwrap();

    let output = wtc()
        .args(["--yes", tmp.path().to_str().unwrap()])
        .stdin(Stdio::null())
        .output()
        .unwrap();

    assert!(output.status.success());
    assert!(
        wt_path.exists(),
        "dirty worktree must be kept without --force"
    );
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("skipping") && stderr.contains("--force"),
        "expected skip message on stderr: {stderr}"
    );
}

#[test]
fn force_deletes_dirty_worktree() {
    let tmp = tempdir().unwrap();
    let repo = tmp.path().join("repo");
    let wt_path = tmp.path().join("wt");
    add_stale_worktree(&repo, &wt_path);
    std::fs::write(wt_path.join("scratch.txt"), "work in progress").unwrap();

    let output = wtc()
        .args(["--yes", "--force", tmp.path().to_str().unwrap()])
        .stdin(Stdio::null())
        .output()
        .unwrap();

    assert!(
        output.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(
        !wt_path.exists(),
        "dirty worktree should be removed with --force"
    );
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        stdout.contains("removed") && stdout.contains("confirmed local changes"),
        "expected forced-removal output: {stdout}"
    );
}

#[test]
fn main_working_tree_never_deleted() {
    let tmp = tempdir().unwrap();
    let repo = tmp.path().join("repo");
    init_repo(&repo);
    commit(&repo);

    let output = wtc()
        .args(["--yes", tmp.path().to_str().unwrap()])
        .stdin(Stdio::null())
        .output()
        .unwrap();

    assert!(output.status.success());
    assert!(repo.exists(), "main working tree must never be deleted");
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        stdout.contains("Nothing to delete"),
        "a lone main repo yields nothing to delete: {stdout}"
    );
}

#[test]
fn non_tty_without_yes_lists_and_exits() {
    let tmp = tempdir().unwrap();
    let repo = tmp.path().join("repo");
    init_repo(&repo);
    commit(&repo);

    let output = wtc()
        .arg(tmp.path())
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .output()
        .unwrap();

    assert!(output.status.success());
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("STATUS"), "expected table header: {stdout}");
    assert!(
        stdout.contains("main"),
        "expected main repo in listing: {stdout}"
    );
    assert!(repo.exists(), "list-only mode must not delete");
}

#[test]
fn orphaned_filter_skips_active_worktrees() {
    let tmp = tempdir().unwrap();

    // Active linked worktree in a healthy repo.
    let repo = tmp.path().join("repo");
    init_repo(&repo);
    commit(&repo);
    let active_wt = tmp.path().join("active-wt");
    add_worktree(&repo, &active_wt);

    // Orphaned worktree: repo removed after creation.
    let orphan_repo = tmp.path().join("orphan-repo");
    init_repo(&orphan_repo);
    commit(&orphan_repo);
    let orphan_wt = tmp.path().join("orphan-wt");
    add_worktree(&orphan_repo, &orphan_wt);
    std::fs::remove_dir_all(&orphan_repo).unwrap();

    let output = wtc()
        .args(["--yes", "--orphaned", tmp.path().to_str().unwrap()])
        .stdin(Stdio::null())
        .output()
        .unwrap();

    assert!(output.status.success());
    assert!(!orphan_wt.exists(), "orphaned worktree should be removed");
    assert!(
        active_wt.exists(),
        "active worktree should be kept with --orphaned"
    );
}

#[test]
fn default_yes_keeps_active_and_removes_orphaned() {
    let tmp = tempdir().unwrap();

    // Active linked worktree in a healthy repo.
    let repo = tmp.path().join("repo");
    init_repo(&repo);
    commit(&repo);
    let active_wt = tmp.path().join("active-wt");
    add_worktree(&repo, &active_wt);

    // Orphaned worktree: its repo is removed after creation.
    let orphan_repo = tmp.path().join("orphan-repo");
    init_repo(&orphan_repo);
    commit(&orphan_repo);
    let orphan_wt = tmp.path().join("orphan-wt");
    add_worktree(&orphan_repo, &orphan_wt);
    std::fs::remove_dir_all(&orphan_repo).unwrap();

    // No filter flag: the safe default (orphaned + stale) must leave the
    // active worktree untouched.
    let output = wtc()
        .args(["--yes", tmp.path().to_str().unwrap()])
        .stdin(Stdio::null())
        .output()
        .unwrap();

    assert!(output.status.success());
    assert!(!orphan_wt.exists(), "orphaned worktree should be removed");
    assert!(
        active_wt.exists(),
        "active worktree must be kept by the default --yes filter"
    );
}

#[test]
fn all_flag_removes_active_worktree() {
    let tmp = tempdir().unwrap();
    let repo = tmp.path().join("repo");
    init_repo(&repo);
    commit(&repo);
    let active_wt = tmp.path().join("active-wt");
    add_worktree(&repo, &active_wt);

    let output = wtc()
        .args(["--yes", "--all", tmp.path().to_str().unwrap()])
        .stdin(Stdio::null())
        .output()
        .unwrap();

    assert!(
        output.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(
        !active_wt.exists(),
        "active worktree should be removed with --all"
    );
}

#[test]
fn stale_filter_removes_stale_keeps_active() {
    let tmp = tempdir().unwrap();

    let stale_repo = tmp.path().join("stale-repo");
    let stale_wt = tmp.path().join("stale-wt");
    add_stale_worktree(&stale_repo, &stale_wt);

    let active_repo = tmp.path().join("active-repo");
    init_repo(&active_repo);
    commit(&active_repo);
    let active_wt = tmp.path().join("active-wt");
    add_worktree(&active_repo, &active_wt);

    let output = wtc()
        .args(["--yes", "--stale", tmp.path().to_str().unwrap()])
        .stdin(Stdio::null())
        .output()
        .unwrap();

    assert!(
        output.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(!stale_wt.exists(), "stale worktree should be removed");
    assert!(
        active_wt.exists(),
        "active worktree should be kept with --stale"
    );
}

#[test]
fn list_dry_run_marks_delete_column() {
    let tmp = tempdir().unwrap();

    // Orphaned (matches the default filter) + active (does not).
    let repo = tmp.path().join("repo");
    init_repo(&repo);
    commit(&repo);
    let active_wt = tmp.path().join("active-wt");
    add_worktree(&repo, &active_wt);

    let orphan_repo = tmp.path().join("orphan-repo");
    init_repo(&orphan_repo);
    commit(&orphan_repo);
    let orphan_wt = tmp.path().join("orphan-wt");
    add_worktree(&orphan_repo, &orphan_wt);
    std::fs::remove_dir_all(&orphan_repo).unwrap();

    // Non-TTY, no --yes: ListOnly mode. --dry-run adds the DELETE column.
    let output = wtc()
        .args(["--dry-run", tmp.path().to_str().unwrap()])
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .output()
        .unwrap();

    assert!(output.status.success());
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        stdout.contains("DELETE"),
        "dry-run listing should have a DELETE column: {stdout}"
    );
    // The orphaned row is marked for deletion, the active row is not.
    assert!(
        stdout.contains("\tyes\t"),
        "orphaned row should be marked yes: {stdout}"
    );
    assert!(
        stdout.contains("\tno\t"),
        "active row should be marked no: {stdout}"
    );
    // Advisory summary is on stderr, keeping stdout a clean TSV.
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("would be deleted"),
        "summary should be on stderr: {stderr}"
    );
    assert!(orphan_wt.exists() && active_wt.exists(), "list mode deletes nothing");
}

#[test]
fn orphaned_and_stale_conflict_is_rejected() {
    let tmp = tempdir().unwrap();

    let output = wtc()
        .args(["--yes", "--orphaned", "--stale", tmp.path().to_str().unwrap()])
        .stdin(Stdio::null())
        .output()
        .unwrap();

    assert!(
        !output.status.success(),
        "conflicting filter flags must be rejected"
    );
    assert_eq!(output.status.code(), Some(2), "clap usage error exits 2");
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("cannot be used with"),
        "expected a clap conflict message: {stderr}"
    );
}
