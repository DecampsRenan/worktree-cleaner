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
    let mut freed_partial = false;
    let mut would_free = 0u64;
    let mut would_free_partial = false;
    for (wt, outcome) in &results {
        // A worktree's size can in rare cases still be unknown here (e.g.
        // confirmed for deletion before its background size computation
        // finished, and the process exited before that arrived) — track
        // that rather than silently treating it as 0, so the total below
        // can honestly say ">= N" instead of undercounting.
        match outcome.action {
            DeleteAction::Removed => match wt.size_bytes {
                Some(bytes) => freed += bytes,
                None => freed_partial = true,
            },
            DeleteAction::DryRun => match wt.size_bytes {
                Some(bytes) => would_free += bytes,
                None => would_free_partial = true,
            },
            DeleteAction::Skipped | DeleteAction::Failed => {}
        }
        println!(
            "{}: {} ({})",
            outcome.action.verb(),
            tui::display_path(&outcome.path),
            outcome.detail
        );
    }

    if freed > 0 || freed_partial {
        let prefix = if freed_partial { ">= " } else { "" };
        println!("Freed {prefix}{}.", size::format_size(freed));
    }
    if would_free > 0 || would_free_partial {
        let prefix = if would_free_partial { ">= " } else { "" };
        println!("Would free {prefix}{}.", size::format_size(would_free));
    }

    Ok(())
}
