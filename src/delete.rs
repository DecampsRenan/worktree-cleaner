use anyhow::Result;

use crate::worktree::Worktree;

/// Remove the selected worktrees.
///
/// TODO(#6): use `git worktree remove` / `git worktree prune` for linked
/// worktrees, fall back to filesystem removal for orphans whose repo is gone,
/// and honour `dry_run` by reporting actions without performing them.
pub fn delete(_worktrees: &[Worktree], dry_run: bool) -> Result<()> {
    let _ = dry_run;
    Ok(())
}
