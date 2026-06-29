//! Shared git fixture helpers for tests.

use std::path::Path;
use std::process::{Command, Output};

/// Run a git command in `cwd`, isolated from the user's global/system config,
/// asserting success and returning its captured output.
pub fn git(cwd: &Path, args: &[&str]) -> Output {
    git_dated(cwd, args, None)
}

/// Like [`git`], but optionally stamps author/committer dates (ISO 8601).
pub fn git_dated(cwd: &Path, args: &[&str], date: Option<&str>) -> Output {
    let mut cmd = Command::new("git");
    cmd.args(args)
        .current_dir(cwd)
        .env("GIT_CONFIG_GLOBAL", "/dev/null")
        .env("GIT_CONFIG_SYSTEM", "/dev/null")
        .env("GIT_AUTHOR_NAME", "test")
        .env("GIT_AUTHOR_EMAIL", "test@example.com")
        .env("GIT_COMMITTER_NAME", "test")
        .env("GIT_COMMITTER_EMAIL", "test@example.com");
    if let Some(date) = date {
        cmd.env("GIT_AUTHOR_DATE", date)
            .env("GIT_COMMITTER_DATE", date);
    }
    let out = cmd.output().expect("git should be runnable");
    assert!(out.status.success(), "git {args:?} failed");
    out
}

/// `git init` a repo at `path` (creating parent dirs).
pub fn init_repo(path: &Path) {
    std::fs::create_dir_all(path).unwrap();
    git(path, &["init", "-q", "-b", "main"]);
}

/// Create an empty commit so `HEAD` exists.
pub fn commit(repo: &Path) {
    git(repo, &["commit", "-q", "--allow-empty", "-m", "init"]);
}

/// Create an empty commit dated `date` (e.g. "2020-01-01T00:00:00").
pub fn commit_at(repo: &Path, date: &str) {
    git_dated(
        repo,
        &["commit", "-q", "--allow-empty", "-m", "old"],
        Some(date),
    );
}

/// Add a linked worktree at `path`.
pub fn add_worktree(repo: &Path, path: &Path) {
    git(repo, &["worktree", "add", "-q", path.to_str().unwrap()]);
}
