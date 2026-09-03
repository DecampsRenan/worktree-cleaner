mod delete;
mod noninteractive;
mod scan;
mod score;
mod selection;
mod size;
#[cfg(test)]
mod testutil;
mod tui;
mod worktree;

use std::io::{IsTerminal, stdin, stdout};
use std::path::PathBuf;

use anyhow::{Result, bail};
use clap::Parser;

use crate::delete::Reclaimed;
use crate::selection::SelectionFilter;

/// How the binary should run after parsing CLI flags.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum RunMode {
    /// Open the ratatui session (stdin and stdout are TTYs, no `--yes`).
    Interactive,
    /// Print the ranked table and exit without deleting (non-TTY without `--yes`).
    ListOnly,
    /// Delete matching worktrees without prompts (`--yes`).
    NonInteractive,
}

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

    /// Non-interactive: delete all matching selectable worktrees without
    /// opening the TUI. Requires this flag instead of a confirmation prompt.
    #[arg(long, short = 'y')]
    yes: bool,

    /// In non-interactive mode, also remove worktrees with uncommitted or
    /// untracked changes. Without this flag, dirty worktrees are skipped and
    /// reported on stderr.
    #[arg(long)]
    force: bool,

    /// In non-interactive mode, only delete orphaned worktrees.
    #[arg(long, conflicts_with = "stale")]
    orphaned: bool,

    /// In non-interactive mode, only delete orphaned and stale worktrees.
    #[arg(long, conflicts_with = "orphaned")]
    stale: bool,
}

impl Args {
    fn run_mode(&self) -> RunMode {
        if self.yes {
            RunMode::NonInteractive
        } else if stdin().is_terminal() && stdout().is_terminal() {
            RunMode::Interactive
        } else {
            RunMode::ListOnly
        }
    }
}

fn main() -> Result<()> {
    let args = Args::parse();

    // Checked up front rather than left to the scanner: a typo'd path would
    // otherwise just produce a silent, empty "No git worktrees found" TUI
    // session with no indication anything was wrong.
    if !args.path.exists() {
        bail!("path does not exist: {}", args.path.display());
    }

    match args.run_mode() {
        RunMode::ListOnly => noninteractive::list(args.path, args.dry_run),
        RunMode::NonInteractive => {
            let filter = SelectionFilter::from_flags(args.orphaned, args.stale);
            noninteractive::delete(args.path, args.dry_run, args.force, filter)
        }
        RunMode::Interactive => run_interactive(args.path, args.dry_run),
    }
}

fn run_interactive(path: PathBuf, dry_run: bool) -> Result<()> {
    // `tui::run` owns the whole interactive session: it scans in the
    // background while the list streams in live, lets the user browse and
    // select before the scan even finishes, and — once confirmed — deletes
    // the selection in the background too, with live per-item progress. The
    // TUI itself reports "no worktrees found" if the scan comes up empty, so
    // there's no separate upfront check here.
    let results = tui::run(path, dry_run)?;
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
