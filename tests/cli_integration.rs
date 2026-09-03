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

fn add_worktree(repo: &Path, path: &Path) {
    git(repo, &["worktree", "add", "-q", path.to_str().unwrap()]);
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
    init_repo(&repo);
    commit(&repo);
    let wt_path = tmp.path().join("wt");
    add_worktree(&repo, &wt_path);
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
        stdout.contains("Nothing to delete") || !stdout.contains("removed"),
        "main repo should not be deleted: {stdout}"
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
