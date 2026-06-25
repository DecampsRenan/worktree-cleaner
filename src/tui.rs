use anyhow::Result;

use crate::worktree::Worktree;

/// Run the interactive selection TUI and return the worktrees the user chose to
/// delete.
///
/// TODO(#4): ratatui + crossterm stateful list with multi-select, a relevance
/// column, status badges, and a final confirmation step before returning.
pub fn select_for_deletion(_worktrees: Vec<Worktree>) -> Result<Vec<Worktree>> {
    Ok(Vec::new())
}
