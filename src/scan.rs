use std::path::Path;

use anyhow::Result;

use crate::worktree::Worktree;

/// Recursively walk `root` and return every git worktree found.
///
/// TODO(#2): use the `ignore` crate to traverse efficiently (pruning
/// `node_modules`, `target`, etc.), detect both main working trees and linked
/// worktrees by inspecting `.git` files/dirs, and enrich each with git2
/// metadata (branch, HEAD, last commit) and a [`WorktreeStatus`].
pub fn scan(_root: &Path) -> Result<Vec<Worktree>> {
    Ok(Vec::new())
}
