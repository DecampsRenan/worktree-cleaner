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

use crate::delete::Reclaimed;

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
    let results = tui::run(args.path, args.dry_run)?;
    if results.is_empty() {
        println!("Nothing selected.");
        return Ok(());
    }

    for (_, outcome) in &results {
        println!(
            "{}: {} ({})",
            outcome.action.verb(),
            tui::display_path(&outcome.path),
            outcome.detail
        );
    }

    // A worktree's size can in rare cases still be unknown here (e.g.
    // confirmed for deletion before its background size computation
    // finished, and the process exited before that arrived); `Reclaimed`
    // tracks that so the totals say ">= N" rather than undercounting.
    let reclaimed = Reclaimed::of(results.iter().map(|(wt, outcome)| (wt, outcome)));
    if let Some(total) = reclaimed.freed.label() {
        println!("Freed {total}.");
    }
    if let Some(total) = reclaimed.would_free.label() {
        println!("Would free {total}.");
    }

    Ok(())
}
