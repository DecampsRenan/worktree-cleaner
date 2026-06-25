use std::path::Path;

use anyhow::Result;
use ignore::WalkBuilder;

use crate::worktree::{Worktree, WorktreeStatus};

/// Recursively walk `root` and return every git worktree found.
///
/// TODO(#2): enrich each with git2 metadata (branch, HEAD, last commit) and
/// refine linked worktrees into Orphaned/Stale/Active.
/// Directory names that never contain worktrees worth scanning and are
/// expensive to descend into.
const PRUNED_DIRS: &[&str] = &["node_modules", "target", ".cargo", ".cache"];

pub fn scan(root: &Path) -> Result<Vec<Worktree>> {
    let mut found = Vec::new();

    let walker = WalkBuilder::new(root)
        .hidden(false)
        .filter_entry(|entry| {
            entry
                .file_name()
                .to_str()
                .is_none_or(|name| !PRUNED_DIRS.contains(&name))
        })
        .build();
    for entry in walker {
        let entry = entry?;
        if entry.file_name() != ".git" {
            continue;
        }
        let git_path = entry.path();
        let Some(worktree_dir) = git_path.parent() else {
            continue;
        };

        // A `.git` directory marks the main working tree of a repository; a
        // `.git` file (containing a `gitdir:` pointer) marks a linked worktree.
        let (repo_path, status) = if git_path.is_dir() {
            (Some(git_path.to_path_buf()), WorktreeStatus::MainRepo)
        } else {
            // Placeholder status; #2 refines linked worktrees into
            // Orphaned/Stale/Active.
            (linked_repo_path(git_path), WorktreeStatus::Active)
        };

        found.push(Worktree {
            path: worktree_dir.to_path_buf(),
            repo_path,
            branch: None,
            head: None,
            last_commit: None,
            last_modified: None,
            status,
        });
    }

    Ok(found)
}

/// Resolve the owning repository's `.git` directory from a linked worktree's
/// `.git` file, which holds a line like `gitdir: /path/repo/.git/worktrees/<name>`.
fn linked_repo_path(git_file: &Path) -> Option<std::path::PathBuf> {
    let contents = std::fs::read_to_string(git_file).ok()?;
    let gitdir = contents
        .lines()
        .find_map(|l| l.strip_prefix("gitdir:"))?
        .trim();
    // gitdir = <repo/.git>/worktrees/<name>; strip the last two components.
    Path::new(gitdir)
        .parent()
        .and_then(Path::parent)
        .map(Path::to_path_buf)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::worktree::WorktreeStatus;
    use std::path::PathBuf;
    use std::process::Command;
    use tempfile::tempdir;

    /// Run a git command in `cwd`, isolated from the user's global/system config.
    fn git(cwd: &Path, args: &[&str]) {
        let ok = Command::new("git")
            .args(args)
            .current_dir(cwd)
            .env("GIT_CONFIG_GLOBAL", "/dev/null")
            .env("GIT_CONFIG_SYSTEM", "/dev/null")
            .env("GIT_AUTHOR_NAME", "test")
            .env("GIT_AUTHOR_EMAIL", "test@example.com")
            .env("GIT_COMMITTER_NAME", "test")
            .env("GIT_COMMITTER_EMAIL", "test@example.com")
            .status()
            .expect("git should be runnable");
        assert!(ok.success(), "git {args:?} failed");
    }

    /// `git init` a repo at `path` (creating parent dirs).
    fn init_repo(path: &Path) {
        std::fs::create_dir_all(path).unwrap();
        git(path, &["init", "-q", "-b", "main"]);
    }

    /// Create an empty commit so `HEAD` exists (needed for `worktree add`).
    fn commit(repo: &Path) {
        git(repo, &["commit", "-q", "--allow-empty", "-m", "init"]);
    }

    fn paths(found: &[Worktree]) -> Vec<PathBuf> {
        let mut p: Vec<PathBuf> = found.iter().map(|w| w.path.clone()).collect();
        p.sort();
        p
    }

    #[test]
    fn finds_a_main_working_tree() {
        let tmp = tempdir().unwrap();
        let repo = tmp.path().join("repo");
        init_repo(&repo);

        let found = scan(tmp.path()).unwrap();

        assert_eq!(paths(&found), vec![repo.clone()]);
        assert_eq!(found[0].status, WorktreeStatus::MainRepo);
    }

    #[test]
    fn finds_a_linked_worktree() {
        let tmp = tempdir().unwrap();
        let repo = tmp.path().join("repo");
        init_repo(&repo);
        commit(&repo);

        let wt = tmp.path().join("wt");
        git(&repo, &["worktree", "add", "-q", wt.to_str().unwrap()]);

        let found = scan(tmp.path()).unwrap();

        // Both the main working tree and the linked worktree are discovered.
        assert_eq!(paths(&found), vec![repo.clone(), wt.clone()]);

        let linked = found.iter().find(|w| w.path == wt).unwrap();
        assert_ne!(linked.status, WorktreeStatus::MainRepo);
        // Its repo_path points back at the owning repo's `.git` directory.
        assert_eq!(
            linked.repo_path.as_ref().unwrap().canonicalize().unwrap(),
            repo.join(".git").canonicalize().unwrap(),
        );
    }

    #[test]
    fn skips_heavy_ignored_directories() {
        let tmp = tempdir().unwrap();
        let repo = tmp.path().join("repo");
        init_repo(&repo);
        // Stray repos buried in dependency/build dirs must not be reported.
        init_repo(&tmp.path().join("node_modules/pkg/vendored"));
        init_repo(&tmp.path().join("target/debug/build/dep"));

        let found = scan(tmp.path()).unwrap();

        assert_eq!(paths(&found), vec![repo.clone()]);
    }
}
