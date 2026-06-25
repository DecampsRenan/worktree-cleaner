use chrono::{DateTime, Utc};

use crate::worktree::{Worktree, WorktreeStatus};

/// Relevance score for deletion: a higher score means a stronger deletion
/// candidate. The main working tree is never a candidate.
///
/// The integer part is the status tier (orphaned > stale > active); the
/// fractional part in `[0, 1)` grows with age, so older worktrees sort first
/// within a tier without ever crossing into another tier.
///
/// TODO(#3): factor in whether the branch is already merged (needs a `merged`
/// signal recorded during the scan).
pub fn relevance(wt: &Worktree) -> f64 {
    let tier = match wt.status {
        WorktreeStatus::MainRepo => return f64::NEG_INFINITY,
        WorktreeStatus::Orphaned => 3.0,
        WorktreeStatus::Stale => 2.0,
        WorktreeStatus::Active => 1.0,
    };
    tier + age_factor(wt)
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

/// Sort worktrees in place so the strongest deletion candidates come first.
pub fn rank(worktrees: &mut [Worktree]) {
    worktrees.sort_by(|a, b| {
        relevance(b)
            .partial_cmp(&relevance(a))
            .unwrap_or(std::cmp::Ordering::Equal)
    });
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
        }
    }

    fn statuses(worktrees: &[Worktree]) -> Vec<WorktreeStatus> {
        worktrees.iter().map(|w| w.status).collect()
    }

    #[test]
    fn ranks_orphaned_above_stale_above_active_and_main_last() {
        let mut v = vec![
            wt(WorktreeStatus::Active, 1),
            wt(WorktreeStatus::MainRepo, 1),
            wt(WorktreeStatus::Orphaned, 1),
            wt(WorktreeStatus::Stale, 1),
        ];

        rank(&mut v);

        assert_eq!(
            statuses(&v),
            vec![
                WorktreeStatus::Orphaned,
                WorktreeStatus::Stale,
                WorktreeStatus::Active,
                WorktreeStatus::MainRepo,
            ]
        );
    }

    #[test]
    fn older_worktrees_rank_higher_within_a_status() {
        let mut v = vec![
            wt(WorktreeStatus::Stale, 10),
            wt(WorktreeStatus::Stale, 200),
            wt(WorktreeStatus::Stale, 50),
        ];

        rank(&mut v);

        // Oldest first ⇒ ascending timestamps down the list.
        let times: Vec<_> = v.iter().map(|w| w.last_commit.unwrap()).collect();
        assert!(
            times[0] < times[1] && times[1] < times[2],
            "expected oldest first"
        );
    }

    #[test]
    fn an_ancient_main_repo_never_outranks_a_fresh_candidate() {
        let mut v = vec![
            wt(WorktreeStatus::MainRepo, 9999),
            wt(WorktreeStatus::Active, 0),
        ];

        rank(&mut v);

        assert_eq!(v[0].status, WorktreeStatus::Active);
        assert_eq!(v[1].status, WorktreeStatus::MainRepo);
    }
}
