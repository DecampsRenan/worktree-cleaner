use std::path::{Path, PathBuf};
use std::process::{Command, Output};

use crate::scan::canonicalize_path;
use crate::worktree::{Worktree, WorktreeStatus};

/// What happened (or would happen) to a worktree during deletion.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DeleteAction {
    /// The worktree was removed.
    Removed,
    /// Nothing was done because this was a dry run.
    DryRun,
    /// Deliberately not touched (e.g. the main working tree).
    Skipped,
    /// Removal was attempted but failed.
    Failed,
}

/// The outcome of attempting to delete one worktree.
#[derive(Debug, Clone)]
pub struct DeleteOutcome {
    pub path: PathBuf,
    pub action: DeleteAction,
    pub detail: String,
}

/// Delete the given worktrees, returning a per-worktree outcome report.
///
/// A failure on one worktree never aborts the others. With `dry_run`, nothing
/// is removed and every outcome is [`DeleteAction::DryRun`]. With `force`, a
/// worktree that `git worktree remove` refuses (dirty/untracked changes) is
/// retried with `--force`; without it, such a worktree is left as `Failed`.
pub fn delete(worktrees: &[Worktree], dry_run: bool, force: bool) -> Vec<DeleteOutcome> {
    worktrees
        .iter()
        .map(|wt| remove_one(wt, dry_run, force))
        .collect()
}

fn remove_one(wt: &Worktree, dry_run: bool, force: bool) -> DeleteOutcome {
    let outcome = |action, detail: &str| DeleteOutcome {
        path: wt.path.clone(),
        action,
        detail: detail.to_string(),
    };

    // The main working tree is never a deletion candidate.
    if wt.status == WorktreeStatus::MainRepo {
        return outcome(DeleteAction::Skipped, "main working tree");
    }

    // An orphaned worktree has no reachable repo to drive `git worktree
    // remove`, so remove its directory directly.
    if wt.status == WorktreeStatus::Orphaned {
        if dry_run {
            return outcome(DeleteAction::DryRun, "would remove orphaned directory");
        }
        return match std::fs::remove_dir_all(&wt.path) {
            Ok(()) => outcome(DeleteAction::Removed, "removed orphaned directory"),
            Err(e) => outcome(DeleteAction::Failed, &e.to_string()),
        };
    }

    // The owning repo's working directory is the parent of its `.git` dir.
    let Some(repo_dir) = wt.repo_path.as_deref().and_then(Path::parent) else {
        return outcome(DeleteAction::Failed, "no owning repository");
    };

    if dry_run {
        return outcome(DeleteAction::DryRun, "would run git worktree remove");
    }

    // Plain remove first; git refuses a dirty worktree unless forced.
    match git_worktree_remove(repo_dir, &wt.path, false) {
        Ok(o) if o.status.success() => outcome(DeleteAction::Removed, "git worktree remove"),
        Ok(_) if force => match git_worktree_remove(repo_dir, &wt.path, true) {
            Ok(o2) if o2.status.success() => {
                outcome(DeleteAction::Removed, "force-removed (had local changes)")
            }
            Ok(o2) => outcome(
                DeleteAction::Failed,
                String::from_utf8_lossy(&o2.stderr).trim(),
            ),
            Err(e) => outcome(DeleteAction::Failed, &e.to_string()),
        },
        Ok(o) => outcome(
            DeleteAction::Failed,
            String::from_utf8_lossy(&o.stderr).trim(),
        ),
        Err(e) => outcome(DeleteAction::Failed, &e.to_string()),
    }
}

/// Run `git -C <repo_dir> worktree remove [--force] <path>`.
///
/// `git -C` changes directory before resolving `path`, so a non-absolute
/// `path` is resolved unreliably (e.g. a `./`-prefixed one resolves literally
/// and fails to match the registered worktree). `scan` already resolves
/// `Worktree::path` and `Worktree::repo_path` to absolute paths for this
/// reason; canonicalizing both again here is defense-in-depth for any other
/// caller.
fn git_worktree_remove(repo_dir: &Path, path: &Path, force: bool) -> std::io::Result<Output> {
    let repo_dir = canonicalize_path(repo_dir);
    let target = canonicalize_path(path);
    let mut cmd = Command::new("git");
    cmd.arg("-C").arg(&repo_dir).args(["worktree", "remove"]);
    if force {
        cmd.arg("--force");
    }
    cmd.arg(target).output()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::scan::scan;
    use crate::testutil::{CwdGuard, add_worktree, commit, git, init_repo};
    use crate::worktree::WorktreeStatus;
    use std::path::Path;
    use tempfile::tempdir;

    /// Run a scan and return the discovered worktree at `path`. `scan`
    /// resolves paths to their canonical absolute form, so `path` is
    /// canonicalized here too before comparing.
    fn discover(root: &Path, path: &Path) -> Worktree {
        let canon = path.canonicalize().expect("path should exist");
        scan(root)
            .unwrap()
            .into_iter()
            .find(|w| w.path == canon)
            .expect("worktree should be discovered")
    }

    /// Worktree paths git still tracks for `repo`.
    fn tracked_worktrees(repo: &Path) -> String {
        let out = git(repo, &["worktree", "list", "--porcelain"]);
        String::from_utf8_lossy(&out.stdout).into_owned()
    }

    #[test]
    fn removes_a_healthy_linked_worktree() {
        let tmp = tempdir().unwrap();
        let repo = tmp.path().join("repo");
        init_repo(&repo);
        commit(&repo);
        let wt_path = tmp.path().join("wt");
        add_worktree(&repo, &wt_path);

        let wt = discover(tmp.path(), &wt_path);
        let outcomes = delete(&[wt], false, false);

        assert!(!wt_path.exists(), "worktree directory should be gone");
        assert_eq!(outcomes.len(), 1);
        assert_eq!(outcomes[0].action, DeleteAction::Removed);
        assert!(
            !tracked_worktrees(&repo).contains(wt_path.to_str().unwrap()),
            "git should no longer track the removed worktree"
        );
    }

    #[test]
    fn dry_run_removes_nothing() {
        let tmp = tempdir().unwrap();
        let repo = tmp.path().join("repo");
        init_repo(&repo);
        commit(&repo);
        let wt_path = tmp.path().join("wt");
        add_worktree(&repo, &wt_path);

        let wt = discover(tmp.path(), &wt_path);
        let outcomes = delete(&[wt], true, false);

        assert!(wt_path.exists(), "dry run must not remove anything");
        assert_eq!(outcomes[0].action, DeleteAction::DryRun);
    }

    #[test]
    fn removes_an_orphaned_worktree_from_the_filesystem() {
        let tmp = tempdir().unwrap();
        let repo = tmp.path().join("repo");
        init_repo(&repo);
        commit(&repo);
        let wt_path = tmp.path().join("wt");
        add_worktree(&repo, &wt_path);

        // The owning repo disappears, so `git worktree remove` can't help.
        std::fs::remove_dir_all(&repo).unwrap();

        let wt = discover(tmp.path(), &wt_path);
        assert_eq!(wt.status, WorktreeStatus::Orphaned, "precondition");

        let outcomes = delete(&[wt], false, false);

        assert!(!wt_path.exists(), "orphaned directory should be removed");
        assert_eq!(outcomes[0].action, DeleteAction::Removed);
    }

    #[test]
    fn never_deletes_the_main_working_tree() {
        let tmp = tempdir().unwrap();
        let repo = tmp.path().join("repo");
        init_repo(&repo);
        commit(&repo);

        let main = discover(tmp.path(), &repo);
        assert_eq!(main.status, WorktreeStatus::MainRepo, "precondition");

        let outcomes = delete(&[main], false, false);

        assert!(repo.exists(), "main working tree must never be deleted");
        assert_eq!(outcomes[0].action, DeleteAction::Skipped);
    }

    #[test]
    fn one_failure_does_not_abort_the_rest() {
        let tmp = tempdir().unwrap();
        let repo = tmp.path().join("repo");
        init_repo(&repo);
        commit(&repo);
        let wt_path = tmp.path().join("wt");
        add_worktree(&repo, &wt_path);
        let healthy = discover(tmp.path(), &wt_path);

        // A malformed worktree (no owning repo) that cannot be removed.
        let bogus = Worktree {
            path: tmp.path().join("nope"),
            repo_path: None,
            branch: None,
            head: None,
            last_commit: None,
            last_modified: None,
            status: WorktreeStatus::Stale,
            merged: false,
            size_bytes: 0,
        };

        let outcomes = delete(&[bogus, healthy], false, false);

        assert_eq!(outcomes.len(), 2);
        assert_eq!(outcomes[0].action, DeleteAction::Failed);
        assert_eq!(outcomes[1].action, DeleteAction::Removed);
        assert!(!wt_path.exists(), "the healthy worktree was still removed");
    }

    #[test]
    fn refuses_to_remove_a_dirty_worktree() {
        let tmp = tempdir().unwrap();
        let repo = tmp.path().join("repo");
        init_repo(&repo);
        commit(&repo);
        let wt_path = tmp.path().join("wt");
        add_worktree(&repo, &wt_path);
        // Leave an untracked file: git refuses to remove without --force.
        std::fs::write(wt_path.join("scratch.txt"), "work in progress").unwrap();

        let wt = discover(tmp.path(), &wt_path);
        let outcomes = delete(&[wt], false, false);

        assert!(wt_path.exists(), "a dirty worktree must be kept");
        assert_eq!(outcomes[0].action, DeleteAction::Failed);
    }

    #[test]
    fn force_removes_a_dirty_worktree() {
        let tmp = tempdir().unwrap();
        let repo = tmp.path().join("repo");
        init_repo(&repo);
        commit(&repo);
        let wt_path = tmp.path().join("wt");
        add_worktree(&repo, &wt_path);
        std::fs::write(wt_path.join("scratch.txt"), "work in progress").unwrap();

        let wt = discover(tmp.path(), &wt_path);
        let outcomes = delete(&[wt], false, true); // force

        assert!(!wt_path.exists(), "force should remove the dirty worktree");
        assert_eq!(outcomes[0].action, DeleteAction::Removed);
        assert!(
            outcomes[0].detail.contains("force"),
            "detail should note force was used, got {:?}",
            outcomes[0].detail
        );
    }

    #[test]
    fn force_with_dry_run_removes_nothing() {
        let tmp = tempdir().unwrap();
        let repo = tmp.path().join("repo");
        init_repo(&repo);
        commit(&repo);
        let wt_path = tmp.path().join("wt");
        add_worktree(&repo, &wt_path);
        std::fs::write(wt_path.join("scratch.txt"), "work in progress").unwrap();

        let wt = discover(tmp.path(), &wt_path);
        let outcomes = delete(&[wt], true, true); // dry-run wins over force

        assert!(
            wt_path.exists(),
            "dry run must not remove even with --force"
        );
        assert_eq!(outcomes[0].action, DeleteAction::DryRun);
    }

    #[test]
    fn removes_a_linked_worktree_when_scanned_from_a_relative_root() {
        let tmp = tempdir().unwrap();
        let repo = tmp.path().join("repo");
        init_repo(&repo);
        commit(&repo);
        let wt_path = tmp.path().join("wt");
        add_worktree(&repo, &wt_path);

        // Regression test for a bug where `scan` kept worktree paths relative
        // to the (relative) scan root, but the owning repo's dir came back
        // absolute from the `.git` file's `gitdir:` pointer. `git -C
        // <absolute repo dir> worktree remove <relative path>` then resolved
        // the relative path against the repo dir instead of the real cwd,
        // and git refused to remove it ("is not a working tree").
        let _cwd_guard = CwdGuard::change_to(tmp.path());
        let found = scan(Path::new(".")).unwrap();
        let wt = found
            .into_iter()
            .find(|w| w.path == wt_path.canonicalize().unwrap())
            .expect("linked worktree should be discovered from a relative root");
        // `scan`'s own canonicalization is the layer under test here; assert
        // on it directly so this test doesn't pass merely because
        // `git_worktree_remove`'s defense-in-depth canonicalize (below) masks
        // a regression in `scan`.
        assert!(
            wt.path.is_absolute(),
            "scan should resolve paths to absolute even from a relative root, got {:?}",
            wt.path
        );

        let outcomes = delete(&[wt], false, false);

        assert_eq!(
            outcomes[0].action,
            DeleteAction::Removed,
            "detail: {}",
            outcomes[0].detail
        );
        assert!(
            !wt_path.exists(),
            "worktree directory should be gone after removal from a relative root"
        );
    }

    #[test]
    fn git_worktree_remove_succeeds_with_a_relative_path() {
        let tmp = tempdir().unwrap();
        let repo = tmp.path().join("repo");
        init_repo(&repo);
        commit(&repo);
        let wt_path = tmp.path().join("wt");
        add_worktree(&repo, &wt_path);

        // Exercises `git_worktree_remove`'s own canonicalize step in
        // isolation, independent of `scan`, so a regression there can't be
        // masked by `scan` already having done the job (see
        // `removes_a_linked_worktree_when_scanned_from_a_relative_root`,
        // which covers the two layers together).
        //
        // The `./`-prefixed form matters: `ignore::WalkBuilder::new(".")` (as
        // used by `scan` before it canonicalizes) yields entries exactly like
        // `./wt`, and `git -C <repo dir> worktree remove ./wt` fails with
        // "is not a working tree" — a bare `wt` (no `./`) happens to still
        // resolve correctly, so it wouldn't have caught this bug.
        let _cwd_guard = CwdGuard::change_to(tmp.path());
        let relative_wt_path = Path::new("./wt");
        assert!(
            relative_wt_path.is_relative(),
            "precondition: the path handed to git_worktree_remove must be relative"
        );

        let output = git_worktree_remove(&repo, relative_wt_path, false).unwrap();

        assert!(
            output.status.success(),
            "git worktree remove should succeed given a relative path, stderr: {}",
            String::from_utf8_lossy(&output.stderr)
        );
        assert!(
            !wt_path.exists(),
            "worktree directory should be gone after removal via a relative path"
        );
    }

    #[test]
    fn git_worktree_remove_succeeds_with_a_relative_repo_dir() {
        let tmp = tempdir().unwrap();
        let repo = tmp.path().join("repo");
        init_repo(&repo);
        commit(&repo);
        let wt_path = tmp.path().join("wt");
        add_worktree(&repo, &wt_path);

        // Hardening, not a bug fix: `repo_path` is always absolute in
        // practice (git writes an absolute `gitdir:` pointer), so this path
        // is only reachable via a hand-corrupted `.git` file. Canonicalize
        // `repo_dir` too, for the same defense-in-depth reason as `path`.
        let _cwd_guard = CwdGuard::change_to(tmp.path());
        let relative_repo_dir = Path::new("./repo");
        assert!(relative_repo_dir.is_relative());

        let output = git_worktree_remove(relative_repo_dir, &wt_path, false).unwrap();

        assert!(
            output.status.success(),
            "git worktree remove should succeed given a relative repo_dir, stderr: {}",
            String::from_utf8_lossy(&output.stderr)
        );
        assert!(
            !wt_path.exists(),
            "worktree directory should be gone after removal via a relative repo_dir"
        );
    }
}
