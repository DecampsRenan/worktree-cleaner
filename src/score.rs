use chrono::{DateTime, Utc};

use crate::worktree::{Worktree, WorktreeStatus};

/// Relevance score for deletion: a higher score means a stronger deletion
/// candidate. The main working tree is never a candidate.
///
/// The integer part is the status tier (orphaned > stale > active); the
/// fractional part in `[0, 1)` ranks within a tier — half weight to whether the
/// branch is already merged, half to age — so a merged worktree outranks an
/// unmerged peer of the same age, and neither ever crosses into another tier.
pub fn relevance(wt: &Worktree) -> f64 {
    let tier = match wt.status {
        WorktreeStatus::MainRepo => return f64::NEG_INFINITY,
        WorktreeStatus::Orphaned => 3.0,
        WorktreeStatus::Stale => 2.0,
        WorktreeStatus::Active => 1.0,
    };
    let merged_factor = if wt.merged { 1.0 } else { 0.0 };
    tier + 0.5 * merged_factor + 0.5 * age_factor(wt)
}

/// Age contribution in `[0, 1)`: 0 for just-touched, approaching 1 as the
/// worktree's last activity recedes into the past.
fn age_factor(wt: &Worktree) -> f64 {
    // Most recent sign of life: latest of the last commit and the fs mtime.
    let last_touched: Option<DateTime<Utc>> = [wt.last_commit, wt.last_modified]
        .into_iter()
        .flatten()
        .max();
    match last_touched {
        Some(t) => {
            let days = Utc::now().signed_duration_since(t).num_seconds() as f64 / 86_400.0;
            let days = days.max(0.0);
            days / (days + 30.0)
        }
        // Unknown age: treat as middling so it doesn't dominate or vanish.
        None => 0.5,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::worktree::WorktreeStatus;
    use chrono::{Duration, Utc};

    fn wt(status: WorktreeStatus, days_old: i64) -> Worktree {
        let when = Utc::now() - Duration::days(days_old);
        Worktree {
            path: format!("/tmp/{status:?}-{days_old}").into(),
            repo_path: None,
            branch: None,
            head: None,
            last_commit: Some(when),
            last_modified: Some(when),
            status,
            merged: false,
            size_bytes: None,
        }
    }

    fn merged_wt(status: WorktreeStatus, days_old: i64) -> Worktree {
        Worktree {
            merged: true,
            ..wt(status, days_old)
        }
    }

    // These used to assert on the result of sorting a `Vec` with the now-
    // removed `rank()` (superseded by `Selector::insert_found`, which keeps
    // the live TUI list ordered incrementally via this same `relevance()` —
    // see tui.rs). Ported to direct comparisons so the underlying ordering
    // guarantees `rank()` used to provide stay covered.

    #[test]
    fn orphaned_outranks_stale_outranks_active() {
        assert!(
            relevance(&wt(WorktreeStatus::Orphaned, 1)) > relevance(&wt(WorktreeStatus::Stale, 1))
        );
        assert!(
            relevance(&wt(WorktreeStatus::Stale, 1)) > relevance(&wt(WorktreeStatus::Active, 1))
        );
    }

    #[test]
    fn main_repo_is_never_a_deletion_candidate() {
        assert_eq!(
            relevance(&wt(WorktreeStatus::MainRepo, 1)),
            f64::NEG_INFINITY
        );
    }

    #[test]
    fn older_worktrees_score_higher_within_a_status() {
        let newer = wt(WorktreeStatus::Stale, 10);
        let middle = wt(WorktreeStatus::Stale, 50);
        let older = wt(WorktreeStatus::Stale, 200);

        assert!(relevance(&older) > relevance(&middle));
        assert!(relevance(&middle) > relevance(&newer));
    }

    #[test]
    fn an_ancient_main_repo_never_outranks_a_fresh_candidate() {
        let ancient_main = wt(WorktreeStatus::MainRepo, 9999);
        let fresh_active = wt(WorktreeStatus::Active, 0);

        assert!(relevance(&fresh_active) > relevance(&ancient_main));
    }

    #[test]
    fn merged_worktrees_score_higher_than_unmerged_peers() {
        let unmerged = wt(WorktreeStatus::Stale, 30);
        let merged = merged_wt(WorktreeStatus::Stale, 30); // same status and age

        assert!(relevance(&merged) > relevance(&unmerged));
    }
}
