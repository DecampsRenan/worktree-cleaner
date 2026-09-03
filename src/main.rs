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

/// Extra `--help` text: the run modes are picked automatically from whether a
/// terminal is attached, which is worth spelling out, plus the dry-run-first
/// recipe scripts and agents should follow.
const AFTER_HELP: &str = "\
MODES (chosen automatically):
  wtc [PATH]            Interactive TUI, when stdin and stdout are both terminals.
  wtc [PATH] | cat      Non-TTY (piped, redirected, or CI): print a ranked TSV
                        table and exit — nothing is deleted.
  wtc --yes [PATH]      Delete the matching worktrees with no prompt (any TTY).

SAFE USAGE (recommended for scripts and agents — preview before deleting):
  wtc --dry-run [PATH]        Preview: a DELETE column marks what --yes would remove.
  wtc --yes --dry-run [PATH]  Same preview, through the delete code path.
  wtc --yes [PATH]            Delete orphaned + stale worktrees (the safe default).
  wtc --yes --all [PATH]      Also delete ACTIVE worktrees (widest, most destructive).

Dirty worktrees (uncommitted or untracked changes) are skipped unless --force.
The main working tree is never deleted. The exit code is non-zero if any deletion fails.";

/// Traverse a directory tree, find git worktrees, rank them by relevance, and
/// delete orphaned or stale ones — interactively (TUI) or non-interactively
/// (`--yes`, for scripts, CI, and agents).
#[derive(Debug, Parser)]
#[command(name = "wtc", version, about, after_help = AFTER_HELP)]
struct Args {
    /// Root directory to scan (defaults to the current directory).
    #[arg(default_value = ".")]
    path: PathBuf,

    /// Show what would be deleted without removing anything.
    #[arg(long)]
    dry_run: bool,

    /// Non-interactive: delete the matching worktrees without opening the TUI
    /// and without any confirmation prompt. By default this removes only
    /// orphaned and stale worktrees; pass --all to also remove active ones.
    /// Preview with --dry-run first.
    #[arg(long, short = 'y')]
    yes: bool,

    /// Also remove worktrees with uncommitted or untracked changes. Without
    /// this flag, dirty worktrees are skipped and reported on stderr.
    #[arg(long)]
    force: bool,

    /// Restrict deletion to orphaned worktrees only.
    #[arg(long, conflicts_with_all = ["stale", "all"])]
    orphaned: bool,

    /// Restrict deletion to orphaned and stale worktrees (this is also the
    /// default when no --orphaned/--stale/--all flag is given).
    #[arg(long, conflicts_with_all = ["orphaned", "all"])]
    stale: bool,

    /// Widen deletion to every worktree except the main working tree —
    /// including active ones. The most destructive filter; preview it with
    /// --dry-run first.
    #[arg(long, conflicts_with_all = ["orphaned", "stale"])]
    all: bool,
}

impl Args {
    /// The filter/force flags refine a non-interactive run; passing any of
    /// them signals non-interactive intent, so we never fall through to the
    /// TUI (which would silently ignore them).
    fn has_noninteractive_flags(&self) -> bool {
        self.force || self.orphaned || self.stale || self.all
    }

    fn run_mode(&self) -> RunMode {
        if self.yes {
            RunMode::NonInteractive
        } else if stdin().is_terminal()
            && stdout().is_terminal()
            && !self.has_noninteractive_flags()
        {
            RunMode::Interactive
        } else {
            RunMode::ListOnly
        }
    }

    fn filter(&self) -> SelectionFilter {
        SelectionFilter::from_flags(self.orphaned, self.stale, self.all)
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

    let filter = args.filter();
    match args.run_mode() {
        RunMode::ListOnly => noninteractive::list(args.path, args.dry_run, args.force, filter),
        RunMode::NonInteractive => {
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
