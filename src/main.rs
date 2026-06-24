// Scaffold: module stubs return placeholder data until implemented per the
// tracking issues. Remove this allow once the modules are filled in.
#![allow(dead_code)]

mod delete;
mod scan;
mod score;
mod tui;
mod worktree;

use std::path::PathBuf;

use anyhow::Result;
use clap::Parser;

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
    delete::delete(&selected, args.dry_run)?;

    Ok(())
}
