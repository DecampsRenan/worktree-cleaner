use std::io::{self, Write};
use std::path::PathBuf;

use anyhow::Result;

use crate::delete::{self, DeleteOutcome, Reclaimed};
use crate::scan;
use crate::score::relevance;
use crate::selection::{self, SelectionFilter};
use crate::size::format_size;
use crate::tui;
use crate::worktree::Worktree;

/// Print the ranked worktree table to stdout and exit without deleting.
pub fn list(root: PathBuf, dry_run: bool) -> Result<()> {
    let worktrees = scan::scan(&root)?;
    print_table(&worktrees, dry_run)?;
    Ok(())
}

/// Delete worktrees matching `filter` without opening the TUI.
pub fn delete(root: PathBuf, dry_run: bool, force: bool, filter: SelectionFilter) -> Result<()> {
    let worktrees = scan::scan(&root)?;
    let selected: Vec<Worktree> = selection::select_for_deletion(&worktrees, filter)
        .into_iter()
        .cloned()
        .collect();

    if selected.is_empty() {
        println!("Nothing to delete.");
        return Ok(());
    }

    let (to_delete, dirty_skipped): (Vec<_>, Vec<_>) = if force {
        (selected, Vec::new())
    } else {
        selected.into_iter().partition(|wt| !wt.dirty)
    };

    for wt in &dirty_skipped {
        eprintln!(
            "skipping {}: local changes (use --force to delete)",
            tui::display_path(&wt.path)
        );
    }

    if to_delete.is_empty() {
        return Ok(());
    }

    let outcomes = delete::delete_batch(&to_delete, dry_run, force);
    print_outcomes(&to_delete, &outcomes)
}

fn print_table(worktrees: &[Worktree], dry_run: bool) -> Result<()> {
    let mut ranked: Vec<&Worktree> = worktrees.iter().collect();
    ranked.sort_by(|a, b| relevance(b).total_cmp(&relevance(a)));

    let stdout = io::stdout();
    let mut out = stdout.lock();

    if dry_run {
        writeln!(out, "STATUS\tAGE\tSIZE\tBRANCH\tDELETE\tPATH")?;
    } else {
        writeln!(out, "STATUS\tAGE\tSIZE\tBRANCH\tPATH")?;
    }

    let mut would_delete = 0usize;
    for wt in &ranked {
        let branch = branch_label(wt);
        let size = wt
            .size_bytes
            .map(format_size)
            .unwrap_or_else(|| "unknown".to_string());
        if dry_run {
            let mark = if selection::is_selectable(wt) {
                would_delete += 1;
                "yes"
            } else {
                "no"
            };
            writeln!(
                out,
                "{}\t{}\t{}\t{}\t{}\t{}",
                wt.status.label(),
                wt.age_label(),
                size,
                branch,
                mark,
                tui::display_path(&wt.path)
            )?;
        } else {
            writeln!(
                out,
                "{}\t{}\t{}\t{}\t{}",
                wt.status.label(),
                wt.age_label(),
                size,
                branch,
                tui::display_path(&wt.path)
            )?;
        }
    }

    if dry_run && would_delete > 0 {
        writeln!(
            out,
            "\n{would_delete} worktree(s) would be deleted (pass --yes to confirm)."
        )?;
    }

    Ok(())
}

fn print_outcomes(worktrees: &[Worktree], outcomes: &[DeleteOutcome]) -> Result<()> {
    for (wt, outcome) in worktrees.iter().zip(outcomes) {
        println!(
            "{}: {} ({})",
            outcome.action.verb(),
            tui::display_path(&wt.path),
            outcome.detail
        );
    }

    let pairs: Vec<_> = worktrees.iter().zip(outcomes).collect();
    let reclaimed = Reclaimed::of(pairs.iter().copied());
    if let Some(total) = reclaimed.freed.label() {
        println!("Freed {total}.");
    }
    if let Some(total) = reclaimed.would_free.label() {
        println!("Would free {total}.");
    }

    Ok(())
}

fn branch_label(wt: &Worktree) -> String {
    match (wt.branch.as_deref(), wt.merged) {
        (Some(b), true) => format!("{b} (merged)"),
        (Some(b), false) => b.to_string(),
        (None, true) => "(merged)".to_string(),
        (None, false) => "-".to_string(),
    }
}
