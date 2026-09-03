use crate::worktree::{Worktree, WorktreeStatus};

/// Which worktrees are eligible for non-interactive deletion.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SelectionFilter {
    /// Every selectable row (same as the TUI's "toggle all selectable").
    AllSelectable,
    /// Only orphaned worktrees.
    OrphanedOnly,
    /// Orphaned and stale worktrees.
    OrphanedAndStale,
}

impl SelectionFilter {
    pub fn from_flags(orphaned: bool, stale: bool) -> Self {
        if orphaned {
            Self::OrphanedOnly
        } else if stale {
            Self::OrphanedAndStale
        } else {
            Self::AllSelectable
        }
    }
}

/// Whether `wt` may be selected for deletion (the main working tree never can).
pub fn is_selectable(wt: &Worktree) -> bool {
    wt.status != WorktreeStatus::MainRepo
}

/// Whether `wt` matches the active non-interactive filter.
pub fn matches_filter(wt: &Worktree, filter: SelectionFilter) -> bool {
    if !is_selectable(wt) {
        return false;
    }
    match filter {
        SelectionFilter::AllSelectable => true,
        SelectionFilter::OrphanedOnly => wt.status == WorktreeStatus::Orphaned,
        SelectionFilter::OrphanedAndStale => {
            matches!(wt.status, WorktreeStatus::Orphaned | WorktreeStatus::Stale)
        }
    }
}

/// Return the worktrees that would be deleted for `filter`, in scan order.
pub fn select_for_deletion(worktrees: &[Worktree], filter: SelectionFilter) -> Vec<&Worktree> {
    worktrees
        .iter()
        .filter(|wt| matches_filter(wt, filter))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testutil::fake_worktree;
    use crate::worktree::WorktreeStatus::*;

    #[test]
    fn main_repo_is_never_selectable() {
        let main = fake_worktree("/main", MainRepo);
        assert!(!is_selectable(&main));
        assert!(!matches_filter(&main, SelectionFilter::AllSelectable));
    }

    #[test]
    fn all_selectable_includes_active_and_stale() {
        let worktrees = vec![
            fake_worktree("/orphaned", Orphaned),
            fake_worktree("/stale", Stale),
            fake_worktree("/active", Active),
        ];
        let selected = select_for_deletion(&worktrees, SelectionFilter::AllSelectable);
        assert_eq!(selected.len(), 3);
    }

    #[test]
    fn orphaned_filter_is_exclusive() {
        let worktrees = vec![
            fake_worktree("/orphaned", Orphaned),
            fake_worktree("/stale", Stale),
        ];
        let selected = select_for_deletion(&worktrees, SelectionFilter::OrphanedOnly);
        assert_eq!(selected.len(), 1);
        assert_eq!(selected[0].path.to_str().unwrap(), "/orphaned");
    }

    #[test]
    fn stale_filter_includes_orphaned_and_stale() {
        let worktrees = vec![
            fake_worktree("/orphaned", Orphaned),
            fake_worktree("/stale", Stale),
            fake_worktree("/active", Active),
        ];
        let selected = select_for_deletion(&worktrees, SelectionFilter::OrphanedAndStale);
        assert_eq!(selected.len(), 2);
    }
}
