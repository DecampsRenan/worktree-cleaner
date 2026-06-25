use crate::worktree::Worktree;

/// Relevance score for deletion: a higher score means a stronger deletion
/// candidate.
///
/// TODO(#3): weight by status (orphaned > stale > active, main repo excluded),
/// age of the last commit, filesystem mtime, and whether the branch is merged.
pub fn relevance(_wt: &Worktree) -> f64 {
    0.0
}

/// Sort worktrees in place so the strongest deletion candidates come first.
pub fn rank(worktrees: &mut [Worktree]) {
    worktrees.sort_by(|a, b| {
        relevance(b)
            .partial_cmp(&relevance(a))
            .unwrap_or(std::cmp::Ordering::Equal)
    });
}
