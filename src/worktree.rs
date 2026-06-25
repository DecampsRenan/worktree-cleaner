use std::path::PathBuf;

use chrono::{DateTime, Utc};

/// A git worktree discovered on disk.
#[derive(Debug, Clone)]
pub struct Worktree {
    /// Absolute path to the worktree directory.
    pub path: PathBuf,
    /// Path to the main repository this worktree belongs to (its common git dir).
    pub repo_path: Option<PathBuf>,
    /// Checked-out branch or ref name, if any.
    pub branch: Option<String>,
    /// Short HEAD commit id.
    pub head: Option<String>,
    /// Timestamp of the most recent commit reachable from HEAD.
    pub last_commit: Option<DateTime<Utc>>,
    /// Filesystem mtime of the worktree, used as an activity hint.
    pub last_modified: Option<DateTime<Utc>>,
    /// Health / deletion-eligibility of the worktree.
    pub status: WorktreeStatus,
}

/// Why a worktree might (or might not) be a deletion candidate.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WorktreeStatus {
    /// A linked worktree whose backing directory or branch is gone / prunable.
    Orphaned,
    /// A valid worktree that simply looks stale (old, merged, no recent activity).
    Stale,
    /// A healthy, recently-used worktree.
    Active,
    /// The primary working tree of a repository (never auto-deleted).
    MainRepo,
}

impl WorktreeStatus {
    /// Short lowercase label for display.
    pub fn label(self) -> &'static str {
        match self {
            WorktreeStatus::Orphaned => "orphaned",
            WorktreeStatus::Stale => "stale",
            WorktreeStatus::Active => "active",
            WorktreeStatus::MainRepo => "main",
        }
    }
}
