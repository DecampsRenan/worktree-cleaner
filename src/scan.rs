use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, mpsc};
use std::thread;

#[cfg(test)]
use anyhow::Result;
use chrono::{DateTime, Utc};
use git2::Repository;
use ignore::{Walk, WalkBuilder};

use crate::size::directory_size;
use crate::worktree::{Worktree, WorktreeStatus};

/// Directory names that never contain worktrees worth scanning and are
/// expensive to descend into.
const PRUNED_DIRS: &[&str] = &["node_modules", "target", ".cargo", ".cache"];

/// A linked worktree is considered stale once its last activity is older than this.
const STALE_AFTER_DAYS: i64 = 30;

/// Number of background workers computing worktree sizes during a streaming
/// scan. Small and fixed rather than one thread per worktree: size
/// computation is I/O-bound (recursive `read_dir`), so a handful of workers
/// is enough to keep it off the walk's critical path without oversubscribing.
const SIZE_WORKERS: usize = 4;

/// Recursively walk `root` and return every git worktree found, each enriched
/// with git metadata and classified into a [`WorktreeStatus`].
///
/// The production binary uses [`scan_streaming`] instead, so the whole scan
/// isn't blocking; this synchronous, collect-into-a-`Vec` form is kept as a
/// test utility (simpler to assert against than draining a channel) and as
/// straightforward documentation of what a scan produces.
#[cfg(test)]
pub fn scan(root: &Path) -> Result<Vec<Worktree>> {
    let mut found = Vec::new();

    for entry in build_walker(root) {
        let entry = entry?;
        if entry.file_name() != ".git" {
            continue;
        }
        let Some((worktree_dir, repo_path, is_main)) = discover_at(entry.path()) else {
            continue;
        };
        let size_bytes = Some(directory_size(&worktree_dir));
        found.push(classify(worktree_dir, repo_path, is_main, size_bytes));
    }

    Ok(found)
}

/// An event emitted while a [`scan_streaming`] scan is in progress.
#[derive(Debug)]
pub enum ScanEvent {
    /// A directory currently being visited, for a live "Checking ..." line.
    /// Purely cosmetic — the receiver only needs the most recent one.
    Progress(PathBuf),
    /// A newly discovered, classified worktree. Its `size_bytes` is `None`
    /// until a matching [`ScanEvent::Size`] arrives from a background worker.
    Found(Worktree),
    /// The computed on-disk size for the worktree at this path.
    Size(PathBuf, u64),
    /// The directory walk has finished; no more `Progress` or `Found` events
    /// will follow. `Size` events may still arrive afterwards — size
    /// computation runs independently and isn't awaited here.
    Done,
}

/// Streaming counterpart to [`scan`]: walks `root` on the calling thread,
/// sending a [`ScanEvent`] for every visited directory and discovered
/// worktree instead of collecting a `Vec`. Meant to be run on its own thread
/// (e.g. via `thread::spawn`) since it blocks until the walk completes.
///
/// Size computation — the expensive part of a scan for large worktrees — is
/// farmed out to a small pool of background workers (see [`SIZE_WORKERS`])
/// so it never blocks the walk itself; each worktree is reported with
/// `size_bytes: None` and a `ScanEvent::Size` follows once its worker
/// finishes, possibly after `ScanEvent::Done`.
///
/// Walk errors (e.g. a permission-denied subdirectory) are skipped rather
/// than aborting the whole scan, unlike [`scan`]: a live, long-running scan
/// shouldn't die on one unreadable directory. If the receiving end is
/// dropped (e.g. the TUI exited), the walk stops early instead of continuing
/// to produce events nobody will read.
pub fn scan_streaming(root: PathBuf, tx: mpsc::Sender<ScanEvent>) {
    let (size_tx, size_rx) = mpsc::channel::<PathBuf>();
    let size_rx = Arc::new(Mutex::new(size_rx));
    for _ in 0..SIZE_WORKERS {
        let size_rx = Arc::clone(&size_rx);
        let worker_tx = tx.clone();
        thread::spawn(move || size_worker(&size_rx, &worker_tx));
    }

    for entry in build_walker(&root) {
        let Ok(entry) = entry else { continue };

        if entry.file_name() != ".git" {
            let is_dir = entry.file_type().is_some_and(|ft| ft.is_dir());
            if is_dir
                && tx
                    .send(ScanEvent::Progress(entry.path().to_path_buf()))
                    .is_err()
            {
                return; // receiver gone; stop walking early
            }
            continue;
        }

        let Some((worktree_dir, repo_path, is_main)) = discover_at(entry.path()) else {
            continue;
        };
        let wt = classify(worktree_dir.clone(), repo_path, is_main, None);
        if tx.send(ScanEvent::Found(wt)).is_err() {
            return;
        }
        // Best-effort: if every size worker has already exited (which only
        // happens once this same job channel is closed, i.e. never before
        // this point), the job is simply dropped and that worktree's size
        // stays `None` — acceptable degradation, not a correctness issue.
        let _ = size_tx.send(worktree_dir);
    }

    let _ = tx.send(ScanEvent::Done);
    // `size_tx` (and this function's `tx`) are dropped here. Once the size
    // workers drain the remaining queued jobs, their `recv()` calls return
    // `Err` and they exit on their own — we don't wait for them.
}

/// One size-computation worker: pulls worktree paths off the shared job
/// queue and reports each one's on-disk size, until the queue is closed and
/// empty or the receiving end of `tx` is gone.
fn size_worker(job_rx: &Mutex<mpsc::Receiver<PathBuf>>, tx: &mpsc::Sender<ScanEvent>) {
    loop {
        let path = {
            let rx = job_rx
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner());
            rx.recv()
        };
        let Ok(path) = path else { return };
        let bytes = directory_size(&path);
        if tx.send(ScanEvent::Size(path, bytes)).is_err() {
            return;
        }
    }
}

/// Build the shared directory walker used by both [`scan`] and
/// [`scan_streaming`]: recursive, showing hidden entries (needed to see
/// `.git`), and pruning [`PRUNED_DIRS`].
fn build_walker(root: &Path) -> Walk {
    WalkBuilder::new(root)
        .hidden(false)
        .filter_entry(|entry| {
            entry
                .file_name()
                .to_str()
                .is_none_or(|name| !PRUNED_DIRS.contains(&name))
        })
        .build()
}

/// Given the path to a discovered `.git` entry, resolve the worktree's own
/// directory, its owning repo's `.git` path, and whether it's the main
/// working tree. Shared between the synchronous and streaming scanners.
fn discover_at(git_path: &Path) -> Option<(PathBuf, Option<PathBuf>, bool)> {
    let worktree_dir = git_path.parent()?;
    // Resolve to an absolute, symlink-free path so it matches how git
    // identifies worktrees internally (and stays consistent regardless of
    // whether `root` was given as relative or absolute).
    let worktree_dir = canonicalize_path(worktree_dir);

    // A `.git` directory marks the main working tree of a repository; a
    // `.git` file (containing a `gitdir:` pointer) marks a linked worktree.
    let is_main = git_path.is_dir();
    let repo_path = if is_main {
        Some(canonicalize_path(git_path))
    } else {
        linked_repo_path(git_path).map(|p| canonicalize_path(&p))
    };
    Some((worktree_dir, repo_path, is_main))
}

/// Best-effort canonicalization: resolves symlinks (e.g. macOS's `/var` ->
/// `/private/var`) and makes relative paths absolute. Falls back to the
/// original path if canonicalization fails (e.g. it no longer exists).
///
/// Shared with `delete::git_worktree_remove`, which uses it as
/// defense-in-depth for the same reason `scan` uses it here: `git -C
/// <repo_dir> worktree remove <path>` resolves a non-absolute `path`
/// unreliably (relative to the caller's cwd for some forms, literally for
/// others, e.g. a `./`-prefixed one), so every path git sees should already
/// be absolute.
pub(crate) fn canonicalize_path(path: &Path) -> PathBuf {
    std::fs::canonicalize(path).unwrap_or_else(|_| path.to_path_buf())
}

/// Build a fully-classified [`Worktree`] for a discovered worktree directory.
/// `size_bytes` is threaded through rather than computed here so callers can
/// choose to compute it eagerly ([`scan`]) or defer it to a background
/// worker ([`scan_streaming`]).
fn classify(
    path: PathBuf,
    repo_path: Option<PathBuf>,
    is_main: bool,
    size_bytes: Option<u64>,
) -> Worktree {
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

    /// Canonicalize an expected path the same way `scan` does, for
    /// comparison against discovered worktrees.
    fn canon(p: &Path) -> PathBuf {
        p.canonicalize().unwrap()
    }

    #[test]
    fn finds_a_main_working_tree() {
        let tmp = tempdir().unwrap();
        let repo = tmp.path().join("repo");
        init_repo(&repo);

        let found = scan(tmp.path()).unwrap();

        assert_eq!(paths(&found), vec![canon(&repo)]);
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
        assert_eq!(paths(&found), {
            let mut expected = vec![canon(&repo), canon(&wt)];
            expected.sort();
            expected
        });

        let linked = found.iter().find(|w| w.path == canon(&wt)).unwrap();
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

        assert_eq!(paths(&found), vec![canon(&repo)]);
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
        let w = found.iter().find(|w| w.path == canon(&wt)).unwrap();

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
        let w = found.iter().find(|w| w.path == canon(&wt)).unwrap();

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
        let w = found.iter().find(|w| w.path == canon(&wt)).unwrap();
        let size = w
            .size_bytes
            .expect("scan (non-streaming) computes size eagerly");

        assert!(
            size >= 4096,
            "size should include the 4 KB file, got {size}"
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

        assert_eq!(paths(&found), vec![canon(&wt)]);
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
        let by_path = |p: &Path| found.iter().find(|w| w.path == canon(p)).unwrap();

        assert_eq!(by_path(&stale_wt).status, WorktreeStatus::Stale);
        assert_eq!(by_path(&active_wt).status, WorktreeStatus::Active);
    }

    #[test]
    fn resolves_paths_to_absolute_even_from_a_relative_root() {
        let tmp = tempdir().unwrap();
        let repo = tmp.path().join("repo");
        init_repo(&repo);
        commit(&repo);
        let wt = tmp.path().join("wt");
        add_worktree(&repo, &wt);

        // Give `scan` a relative root, as `wtc` does by default (`.`).
        let _cwd_guard = crate::testutil::CwdGuard::change_to(tmp.path());
        let found = scan(Path::new(".")).unwrap();

        for w in &found {
            assert!(
                w.path.is_absolute(),
                "path should be absolute even when the scan root is relative, got {:?}",
                w.path
            );
        }
        assert_eq!(paths(&found), {
            let mut expected = vec![canon(&repo), canon(&wt)];
            expected.sort();
            expected
        });
    }

    #[test]
    fn scan_streaming_finds_worktrees_reports_progress_and_computes_size() {
        let tmp = tempdir().unwrap();
        let repo = tmp.path().join("repo");
        init_repo(&repo);
        commit(&repo);
        let wt = tmp.path().join("wt");
        add_worktree(&repo, &wt);
        std::fs::write(wt.join("big.bin"), vec![0u8; 4096]).unwrap();

        let (tx, rx) = mpsc::channel();
        // Run on the test thread directly: `scan_streaming` only blocks for
        // the walk itself (size workers are detached), and iterating `rx`
        // below blocks until every sender clone — the walker's and every
        // size worker's — is dropped, so this still waits for all sizes.
        scan_streaming(tmp.path().to_path_buf(), tx);

        let mut progress_seen = false;
        let mut found_paths = Vec::new();
        let mut sizes = std::collections::HashMap::new();
        let mut saw_done = false;

        for event in rx {
            match event {
                ScanEvent::Progress(_) => progress_seen = true,
                ScanEvent::Found(w) => {
                    assert!(
                        w.size_bytes.is_none(),
                        "streaming should report a worktree before its size is known"
                    );
                    found_paths.push(w.path);
                }
                ScanEvent::Size(path, bytes) => {
                    sizes.insert(path, bytes);
                }
                ScanEvent::Done => saw_done = true,
            }
        }

        found_paths.sort();
        let mut expected = vec![canon(&repo), canon(&wt)];
        expected.sort();
        assert_eq!(found_paths, expected);
        assert!(
            progress_seen,
            "should report at least one visited directory"
        );
        assert!(saw_done, "should signal when the walk itself finishes");
        assert!(
            *sizes
                .get(&canon(&wt))
                .expect("a Size event should eventually arrive for the linked worktree")
                >= 4096,
            "size should include the 4 KB file"
        );
    }

    #[test]
    fn scan_streaming_does_not_panic_when_the_receiver_is_dropped() {
        let tmp = tempdir().unwrap();
        let repo = tmp.path().join("repo");
        init_repo(&repo);
        commit(&repo);
        for i in 0..20 {
            std::fs::create_dir_all(tmp.path().join(format!("unrelated-{i}"))).unwrap();
        }

        let (tx, rx) = mpsc::channel();
        drop(rx);

        // `tx.send(..)` fails on the very first directory visited; the walk
        // must bail out via the `.is_err() => return` paths instead of
        // panicking (e.g. by unwrapping a send result).
        scan_streaming(tmp.path().to_path_buf(), tx);
    }
}
