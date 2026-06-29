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
    /// Whether this worktree's HEAD is already merged into the owning repo's
    /// default branch (a strong hint it's safe to delete). `false` when unknown.
    pub merged: bool,
    /// On-disk size of the worktree directory in bytes (what removing it frees).
    pub size_bytes: u64,
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

impl Worktree {
    /// A coarse "Nd/Nmo/Ny ago" label for the worktree's last activity.
    pub fn age_label(&self) -> String {
        let Some(when) = self.last_commit.or(self.last_modified) else {
            return "unknown".to_string();
        };
        let days = Utc::now().signed_duration_since(when).num_days().max(0);
        match days {
            0 => "today".to_string(),
            1 => "1 day ago".to_string(),
            2..=30 => format!("{days} days ago"),
            31..=364 => format!("{} mo ago", days / 30),
            _ => format!("{} yr ago", days / 365),
        }
    }
}
