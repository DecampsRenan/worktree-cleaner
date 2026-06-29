mod delete;
mod scan;
mod score;
#[cfg(test)]
mod testutil;
mod tui;
mod worktree;

use std::path::PathBuf;

use anyhow::Result;
use clap::Parser;

use crate::delete::DeleteAction;

/// Traverse a directory tree, find git worktrees, rank them by relevance, and
/// interactively delete orphaned or stale ones.
#[derive(Debug, Parser)]
#[command(name = "wtc", version, about)]
struct Args {
    /// Root directory to scan (defaults to the current directory).
    #[arg(default_value = ".")]
    path: PathBuf,

    /// Show what would be deleted without removing anything.
    #[arg(long)]
    dry_run: bool,
}

fn main() -> Result<()> {
    let args = Args::parse();

    let mut worktrees = scan::scan(&args.path)?;
    score::rank(&mut worktrees);

    if worktrees.is_empty() {
        println!("No git worktrees found under {}", args.path.display());
        return Ok(());
    }

    let selected = tui::select_for_deletion(worktrees)?;
    if selected.is_empty() {
        println!("Nothing selected.");
        return Ok(());
    }

    let outcomes = delete::delete(&selected, args.dry_run);
    for outcome in &outcomes {
        let verb = match outcome.action {
            DeleteAction::Removed => "removed",
            DeleteAction::DryRun => "would remove",
            DeleteAction::Skipped => "skipped",
            DeleteAction::Failed => "FAILED",
        };
        println!("{verb}: {} ({})", outcome.path.display(), outcome.detail);
    }

    Ok(())
}
