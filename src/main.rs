mod delete;
mod scan;
mod score;
mod size;
#[cfg(test)]
mod testutil;
mod tui;
mod worktree;

use std::path::PathBuf;

use anyhow::{Result, bail};
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

    /// Force-remove worktrees with uncommitted or untracked changes.
    #[arg(long)]
    force: bool,
}

fn main() -> Result<()> {
    let args = Args::parse();

    // Checked up front rather than left to the scanner: a typo'd path would
    // otherwise just produce a silent, empty "No git worktrees found" TUI
    // session with no indication anything was wrong.
    if !args.path.exists() {
        bail!("path does not exist: {}", args.path.display());
    }

    // `tui::run` owns the whole interactive session: it scans in the
    // background while the list streams in live, lets the user browse and
    // select before the scan even finishes, and — once confirmed — deletes
    // the selection in the background too, with live per-item progress. The
    // TUI itself reports "no worktrees found" if the scan comes up empty, so
    // there's no separate upfront check here.
    let results = tui::run(args.path, args.dry_run, args.force)?;
    if results.is_empty() {
        println!("Nothing selected.");
        return Ok(());
    }

    let mut freed = 0u64;
    let mut would_free = 0u64;
    for (wt, outcome) in &results {
        match outcome.action {
            DeleteAction::Removed => freed += wt.size_bytes.unwrap_or(0),
            DeleteAction::DryRun => would_free += wt.size_bytes.unwrap_or(0),
            DeleteAction::Skipped | DeleteAction::Failed => {}
        }
        println!(
            "{}: {} ({})",
            outcome.action.verb(),
            tui::display_path(&outcome.path),
            outcome.detail
        );
    }

    if freed > 0 {
        println!("Freed {}.", size::format_size(freed));
    }
    if would_free > 0 {
        println!("Would free {}.", size::format_size(would_free));
    }

    Ok(())
}
