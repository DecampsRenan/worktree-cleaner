use std::path::{Path, PathBuf};

use anyhow::Result;
use chrono::{DateTime, Utc};
use git2::Repository;
use ignore::WalkBuilder;

use crate::size::directory_size;
use crate::worktree::{Worktree, WorktreeStatus};

/// Directory names that never contain worktrees worth scanning and are
/// expensive to descend into.
const PRUNED_DIRS: &[&str] = &["node_modules", "target", ".cargo", ".cache"];

/// A linked worktree is considered stale once its last activity is older than this.
const STALE_AFTER_DAYS: i64 = 30;

/// Recursively walk `root` and return every git worktree found, each enriched
/// with git metadata and classified into a [`WorktreeStatus`].
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
        let is_main = git_path.is_dir();
        let repo_path = if is_main {
            Some(git_path.to_path_buf())
        } else {
            linked_repo_path(git_path)
        };

        found.push(classify(worktree_dir.to_path_buf(), repo_path, is_main));
    }

    Ok(found)
}

/// Build a fully-classified [`Worktree`] for a discovered worktree directory.
fn classify(path: PathBuf, repo_path: Option<PathBuf>, is_main: bool) -> Worktree {
    let last_modified = fs_mtime(&path);

    // Open the repository to read branch / HEAD / last commit. For a linked
    // worktree, failure to open means its repo or admin dir is gone.
    let opened = Repository::open(&path).ok();
    let (branch, head, last_commit) = match &opened {
        Some(repo) => read_head(repo),
        None => (None, None, None),
    };

    let status = if is_main {
        WorktreeStatus::MainRepo
    } else if opened.is_none() {
        WorktreeStatus::Orphaned
    } else if is_stale(last_commit.or(last_modified)) {
        WorktreeStatus::Stale
    } else {
        WorktreeStatus::Active
    };

    // Only meaningful for linked worktrees we can open; unknown ⇒ false.
    let merged = match (&opened, is_main) {
        (Some(repo), false) => is_merged(repo).unwrap_or(false),
        _ => false,
    };

    let size_bytes = directory_size(&path);

    Worktree {
        path,
        repo_path,
        branch,
        head,
        last_commit,
        last_modified,
        status,
        merged,
        size_bytes,
    }
}

/// Whether the worktree's HEAD is the default branch tip or an ancestor of it
/// (i.e. already merged). `None` if it can't be determined.
fn is_merged(repo: &Repository) -> Option<bool> {
    let head = repo.head().ok()?.peel_to_commit().ok()?.id();
    let default_tip = default_branch_tip(repo)?;
    if head == default_tip {
        return Some(true);
    }
    repo.graph_descendant_of(default_tip, head).ok()
}

/// Resolve the owning repo's default branch tip: prefer the remote's default
/// (`origin/HEAD`), then a local `main`, then `master`.
fn default_branch_tip(repo: &Repository) -> Option<git2::Oid> {
    for spec in [
        "refs/remotes/origin/HEAD",
        "refs/heads/main",
        "refs/heads/master",
    ] {
        if let Ok(obj) = repo.revparse_single(spec)
            && let Ok(commit) = obj.peel_to_commit()
        {
            return Some(commit.id());
        }
    }
    None
}

/// Read the branch shorthand, short HEAD id, and last commit time from a repo.
fn read_head(repo: &Repository) -> (Option<String>, Option<String>, Option<DateTime<Utc>>) {
    let Ok(head_ref) = repo.head() else {
        return (None, None, None);
    };
    let branch = head_ref.shorthand().ok().map(str::to_owned);
    let commit = head_ref.peel_to_commit().ok();
    let head = commit.as_ref().map(|c| {
        let id = c.id().to_string();
        id[..7.min(id.len())].to_owned()
    });
    let last_commit = commit
        .as_ref()
        .and_then(|c| DateTime::from_timestamp(c.time().seconds(), 0));
    (branch, head, last_commit)
}

/// Filesystem mtime of the worktree directory, as a UTC timestamp.
fn fs_mtime(path: &Path) -> Option<DateTime<Utc>> {
    let modified = std::fs::metadata(path).ok()?.modified().ok()?;
    Some(modified.into())
}

/// Whether the most recent activity is older than [`STALE_AFTER_DAYS`].
fn is_stale(last_activity: Option<DateTime<Utc>>) -> bool {
    last_activity
        .map(|t| Utc::now().signed_duration_since(t).num_days() > STALE_AFTER_DAYS)
        .unwrap_or(false)
}

/// Resolve the owning repository's `.git` directory from a linked worktree's
/// `.git` file, which holds a line like `gitdir: /path/repo/.git/worktrees/<name>`.
fn linked_repo_path(git_file: &Path) -> Option<PathBuf> {
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
    use crate::testutil::{add_worktree, commit, commit_at, init_repo};
    use crate::worktree::WorktreeStatus;
    use std::path::PathBuf;
    use tempfile::tempdir;

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
        add_worktree(&repo, &wt);

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

    #[test]
    fn populates_git_metadata_for_main_repo() {
        let tmp = tempdir().unwrap();
        let repo = tmp.path().join("repo");
        init_repo(&repo);
        commit(&repo);

        let found = scan(tmp.path()).unwrap();
        let w = &found[0];

        assert_eq!(w.status, WorktreeStatus::MainRepo);
        assert_eq!(w.branch.as_deref(), Some("main"));
        assert!(w.head.is_some(), "head should be the short commit id");
        assert!(w.last_commit.is_some(), "last_commit should be set");
        assert!(w.last_modified.is_some(), "last_modified should be set");
    }

    #[test]
    fn detects_a_merged_worktree() {
        let tmp = tempdir().unwrap();
        let repo = tmp.path().join("repo");
        init_repo(&repo);
        commit(&repo); // C1 on main
        let wt = tmp.path().join("wt");
        add_worktree(&repo, &wt); // new branch at C1
        commit(&repo); // C2 on main; the worktree's branch is now an ancestor

        let found = scan(tmp.path()).unwrap();
        let w = found.iter().find(|w| w.path == wt).unwrap();

        assert!(w.merged, "branch is an ancestor of main, so it is merged");
    }

    #[test]
    fn a_worktree_ahead_of_default_is_not_merged() {
        let tmp = tempdir().unwrap();
        let repo = tmp.path().join("repo");
        init_repo(&repo);
        commit(&repo); // C1 on main
        let wt = tmp.path().join("wt");
        add_worktree(&repo, &wt); // branch at C1
        commit(&wt); // advance the worktree's own branch ahead of main

        let found = scan(tmp.path()).unwrap();
        let w = found.iter().find(|w| w.path == wt).unwrap();

        assert!(
            !w.merged,
            "branch has commits not on main, so it is not merged"
        );
    }

    #[test]
    fn populates_size_bytes() {
        let tmp = tempdir().unwrap();
        let repo = tmp.path().join("repo");
        init_repo(&repo);
        commit(&repo);
        let wt = tmp.path().join("wt");
        add_worktree(&repo, &wt);
        std::fs::write(wt.join("big.bin"), vec![0u8; 4096]).unwrap();

        let found = scan(tmp.path()).unwrap();
        let w = found.iter().find(|w| w.path == wt).unwrap();

        assert!(
            w.size_bytes >= 4096,
            "size should include the 4 KB file, got {}",
            w.size_bytes
        );
    }

    #[test]
    fn classifies_worktree_whose_repo_is_gone_as_orphaned() {
        let tmp = tempdir().unwrap();
        let repo = tmp.path().join("repo");
        init_repo(&repo);
        commit(&repo);
        let wt = tmp.path().join("wt");
        add_worktree(&repo, &wt);

        // The owning repository disappears, leaving the worktree dangling.
        std::fs::remove_dir_all(&repo).unwrap();

        let found = scan(tmp.path()).unwrap();

        assert_eq!(paths(&found), vec![wt.clone()]);
        assert_eq!(found[0].status, WorktreeStatus::Orphaned);
    }

    #[test]
    fn classifies_old_worktree_as_stale_and_recent_as_active() {
        let tmp = tempdir().unwrap();

        // Stale: its only commit is well past the staleness threshold.
        let stale_repo = tmp.path().join("stale");
        init_repo(&stale_repo);
        commit_at(&stale_repo, "2020-01-01T00:00:00");
        let stale_wt = tmp.path().join("stale-wt");
        add_worktree(&stale_repo, &stale_wt);

        // Active: a recent commit.
        let active_repo = tmp.path().join("active");
        init_repo(&active_repo);
        commit(&active_repo);
        let active_wt = tmp.path().join("active-wt");
        add_worktree(&active_repo, &active_wt);

        let found = scan(tmp.path()).unwrap();
        let by_path = |p: &Path| found.iter().find(|w| w.path == p).unwrap();

        assert_eq!(by_path(&stale_wt).status, WorktreeStatus::Stale);
        assert_eq!(by_path(&active_wt).status, WorktreeStatus::Active);
    }
}
