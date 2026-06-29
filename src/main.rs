// Scaffold: module stubs return placeholder data until implemented per the
// tracking issues. Remove this allow once the modules are filled in.
#![allow(dead_code)]

mod delete;
mod scan;
mod score;
#[cfg(test)]
mod testutil;
mod tui;
mod worktree;

use std::path::PathBuf;

use anyhow::Result;
use chrono::{DateTime, Utc};
use clap::Parser;

use crate::worktree::Worktree;

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

    // Interim non-interactive listing. The interactive multi-select TUI (#4)
    // and deletion (#5) replace this flow once implemented.
    print_listing(&worktrees);

    let selected = tui::select_for_deletion(worktrees)?;
    for outcome in delete::delete(&selected, args.dry_run) {
        println!(
            "  {:?}  {}  {}",
            outcome.action,
            outcome.path.display(),
            outcome.detail
        );
    }

    Ok(())
}

/// Print discovered worktrees, best deletion candidate first.
fn print_listing(worktrees: &[Worktree]) {
    println!(
        "Found {} worktree(s), most worth deleting first:\n",
        worktrees.len()
    );
    for w in worktrees {
        println!(
            "  {:<8}  {:>12}  {:<20}  {}",
            w.status.label(),
            humanize_age(w.last_commit.or(w.last_modified)),
            w.branch.as_deref().unwrap_or("-"),
            w.path.display(),
        );
    }
    println!("\nInteractive selection and deletion are coming in #4 and #5.");
}

/// Render a timestamp as a coarse "Nd/Nmo/Ny ago" age, or "unknown".
fn humanize_age(when: Option<DateTime<Utc>>) -> String {
    let Some(when) = when else {
        return "unknown".to_string();
    };
    let days = Utc::now().signed_duration_since(when).num_days().max(0);
    match days {
        0 => "today".to_string(),
        1 => "1 day ago".to_string(),
        2..=30 => format!("{days} days ago"),
        31..=364 => format!("{} mo ago", days / 30),
        _ => format!("{} yr ago", days / 365),
    }
}
