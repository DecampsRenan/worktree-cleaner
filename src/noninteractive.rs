use std::io::{self, Write};
use std::path::PathBuf;

use anyhow::{Result, bail};

use crate::delete::{self, DeleteAction, DeleteOutcome, Reclaimed};
use crate::scan;
use crate::score::relevance;
use crate::selection::{self, SelectionFilter};
use crate::size::format_size;
use crate::tui;
use crate::worktree::Worktree;

/// Print the ranked worktree table to stdout and exit without deleting.
///
/// In `dry_run` mode the table gains a DELETE column marking exactly the rows
/// a matching `delete` run (same `force` and `filter`) would remove, so the
/// preview never overstates the deletion set.
pub fn list(root: PathBuf, dry_run: bool, force: bool, filter: SelectionFilter) -> Result<()> {
    let worktrees = scan::scan(&root)?;
    let would_delete = match write_table(&worktrees, dry_run, force, filter) {
        Ok(n) => n,
        // A closed downstream pipe (`wtc | head`) is a normal end of output,
        // not an error worth reporting.
        Err(e) if e.kind() == io::ErrorKind::BrokenPipe => return Ok(()),
        Err(e) => return Err(e.into()),
    };

    // Advisory prose goes to stderr so stdout stays a clean, parseable TSV.
    if dry_run {
        if would_delete > 0 {
            eprintln!("{would_delete} worktree(s) would be deleted (pass --yes to confirm).");
        } else {
            eprintln!("No worktrees match the current filter; nothing would be deleted.");
        }
    }

    Ok(())
}

/// Delete worktrees matching `filter` without opening the TUI.
///
/// Returns an error (non-zero exit) if any removal failed, so scripts and CI
/// can detect a partial failure — every outcome is still printed first.
pub fn delete(root: PathBuf, dry_run: bool, force: bool, filter: SelectionFilter) -> Result<()> {
    let worktrees = scan::scan(&root)?;
    let selected = selection::select_for_deletion(&worktrees, filter);

    if selected.is_empty() {
        println!("Nothing to delete.");
        return Ok(());
    }

    // Without `--force`, worktrees with local changes are kept and reported;
    // partition the borrows so only the ones we actually delete are cloned.
    let (to_delete, dirty_skipped): (Vec<&Worktree>, Vec<&Worktree>) = if force {
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

    let to_delete: Vec<Worktree> = to_delete.into_iter().cloned().collect();
    let outcomes = delete::delete_batch(&to_delete, dry_run, force);

    match print_outcomes(&to_delete, &outcomes) {
        Ok(()) => {}
        Err(e) if e.kind() == io::ErrorKind::BrokenPipe => {}
        Err(e) => return Err(e.into()),
    }

    let failed = outcomes
        .iter()
        .filter(|o| o.action == DeleteAction::Failed)
        .count();
    if failed > 0 {
        bail!("{failed} worktree(s) failed to delete");
    }

    Ok(())
}

/// Write the ranked table to stdout, returning how many rows were marked for
/// deletion (always 0 unless `dry_run`).
fn write_table(
    worktrees: &[Worktree],
    dry_run: bool,
    force: bool,
    filter: SelectionFilter,
) -> io::Result<usize> {
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
        let branch = wt.branch_label();
        let size = wt
            .size_bytes
            .map(format_size)
            .unwrap_or_else(|| "unknown".to_string());
        if dry_run {
            // Exactly what `delete` would remove: a filter match that is
            // either clean or force-removable.
            let will_delete = selection::matches_filter(wt, filter) && (force || !wt.dirty);
            let mark = if will_delete {
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

    Ok(would_delete)
}

fn print_outcomes(worktrees: &[Worktree], outcomes: &[DeleteOutcome]) -> io::Result<()> {
    let stdout = io::stdout();
    let mut out = stdout.lock();

    for (wt, outcome) in worktrees.iter().zip(outcomes) {
        writeln!(
            out,
            "{}: {} ({})",
            outcome.action.verb(),
            tui::display_path(&wt.path),
            outcome.detail
        )?;
    }

    let pairs: Vec<_> = worktrees.iter().zip(outcomes).collect();
    let reclaimed = Reclaimed::of(pairs.iter().copied());
    if let Some(total) = reclaimed.freed.label() {
        writeln!(out, "Freed {total}.")?;
    }
    if let Some(total) = reclaimed.would_free.label() {
        writeln!(out, "Would free {total}.")?;
    }

    Ok(())
}
