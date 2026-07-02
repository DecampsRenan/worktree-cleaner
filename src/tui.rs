use std::path::{Path, PathBuf};
use std::sync::mpsc;
use std::thread;
use std::time::Duration;

use anyhow::Result;
use ratatui::crossterm::event::{self, Event, KeyCode, KeyEvent, KeyEventKind, KeyModifiers};
use ratatui::layout::{Constraint, Layout};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::Line;
use ratatui::widgets::{Block, Borders, List, ListItem, ListState};
use ratatui::{DefaultTerminal, Frame};

use crate::delete::{self, DeleteAction, DeleteEvent, DeleteOutcome};
use crate::scan::{self, ScanEvent};
use crate::score::relevance;
use crate::size::format_size;
use crate::worktree::{Worktree, WorktreeStatus};

/// Interactive selection state over a list of worktrees. Pure logic,
/// independent of rendering, so it can be unit-tested without a terminal.
pub struct Selector {
    items: Vec<Worktree>,
    cursor: usize,
    /// Whether the user has ever moved or acted on the cursor. Before that,
    /// `insert_found` keeps the cursor pinned to the top of the list (the
    /// current best candidate) rather than trying to preserve "attachment"
    /// to whatever item happened to start at index 0 — there's nothing
    /// meaningful to stay attached to yet. See `insert_found`.
    cursor_touched: bool,
    selected: Vec<bool>,
}

impl Selector {
    pub fn new(items: Vec<Worktree>) -> Self {
        let selected = vec![false; items.len()];
        Self {
            items,
            cursor: 0,
            cursor_touched: false,
            selected,
        }
    }

    pub fn items(&self) -> &[Worktree] {
        &self.items
    }

    pub fn cursor(&self) -> usize {
        self.cursor
    }

    pub fn is_selected(&self, i: usize) -> bool {
        self.selected[i]
    }

    pub fn move_down(&mut self) {
        self.cursor_touched = true;
        if self.cursor + 1 < self.items.len() {
            self.cursor += 1;
        }
    }

    pub fn move_up(&mut self) {
        self.cursor_touched = true;
        self.cursor = self.cursor.saturating_sub(1);
    }

    /// Whether row `i` may be selected for deletion (the main working tree
    /// never can).
    pub fn selectable(&self, i: usize) -> bool {
        self.items[i].status != WorktreeStatus::MainRepo
    }

    pub fn toggle(&mut self) {
        self.cursor_touched = true;
        let i = self.cursor;
        // Guard against an empty list (nothing found yet, or a tree with no
        // worktrees at all) or any other state where the cursor doesn't
        // currently refer to a real row: `selectable`/`self.selected[i]`
        // index unconditionally and would panic otherwise. `move_up` /
        // `move_down` never push the cursor out of bounds and `toggle_all`
        // only ever indexes within `0..items.len()`, so this is the one
        // spot that needed the check.
        if i >= self.items.len() {
            return;
        }
        if self.selectable(i) {
            self.selected[i] = !self.selected[i];
        }
    }

    /// Select every selectable row, or clear the selection if all selectable
    /// rows are already selected.
    pub fn toggle_all(&mut self) {
        let all_selected = (0..self.items.len())
            .filter(|&i| self.selectable(i))
            .all(|i| self.selected[i]);
        for i in 0..self.items.len() {
            self.selected[i] = self.selectable(i) && !all_selected;
        }
    }

    /// Total on-disk size of the currently-selected worktrees, in bytes.
    /// Worktrees whose size hasn't been computed yet contribute `0` — the
    /// total simply grows as their `Size` events arrive.
    pub fn selected_size(&self) -> u64 {
        self.items
            .iter()
            .zip(&self.selected)
            .filter(|&(_, &sel)| sel)
            .map(|(w, _)| w.size_bytes.unwrap_or(0))
            .sum()
    }

    /// Consume the selector, returning the worktrees the user chose.
    ///
    /// Production code uses [`selected_worktrees_snapshot`](Self::selected_worktrees_snapshot)
    /// instead, since the selector is still needed for rendering/input after
    /// a selection is taken; this consuming form is kept as a test utility.
    #[cfg(test)]
    pub fn selected_worktrees(self) -> Vec<Worktree> {
        self.items
            .into_iter()
            .zip(self.selected)
            .filter_map(|(w, sel)| sel.then_some(w))
            .collect()
    }

    /// Like [`selected_worktrees`](Self::selected_worktrees), but clones
    /// instead of consuming — used to snapshot the selection while the
    /// selector is still needed for further rendering and input.
    pub fn selected_worktrees_snapshot(&self) -> Vec<Worktree> {
        self.items
            .iter()
            .zip(&self.selected)
            .filter(|&(_, &sel)| sel)
            .map(|(w, _)| w.clone())
            .collect()
    }

    /// Insert a newly discovered worktree, keeping `items` ordered by
    /// descending [`relevance`] score — the same ranking a finished batch
    /// used to get sorted into in one pass, now maintained incrementally as
    /// worktrees stream in. Ties keep the earlier-found item on top.
    ///
    /// The cursor and every existing selection flag stay attached to their
    /// worktree: both shift in lockstep with the insertion, so a row
    /// appearing above the cursor (or an already-selected row) never
    /// changes which worktree it refers to. This is the mechanism that
    /// keeps a relevance-ranked list without ever yanking the cursor or a
    /// selection onto the wrong item while the scan is still running.
    pub fn insert_found(&mut self, wt: Worktree) {
        let had_items = !self.items.is_empty();
        let score = relevance(&wt);
        let idx = self
            .items
            .partition_point(|existing| relevance(existing) >= score);
        self.items.insert(idx, wt);
        self.selected.insert(idx, false);

        if self.cursor_touched {
            // The user has looked at (or acted on) a specific row: keep the
            // cursor attached to that same worktree as the list reshuffles
            // around it, by shifting it exactly like `Vec::insert` shifts
            // every item from `idx` onward. `had_items` guards the very
            // first-ever insertion into an empty list, where the cursor
            // doesn't yet refer to anything — without it, `idx(0) <=
            // cursor(0)` would look like "shift needed" and push the
            // cursor out of bounds.
            if had_items && idx <= self.cursor {
                self.cursor += 1;
            }
        } else {
            // Before that, there's no specific row to stay attached to —
            // pin the cursor to the top of the list so it always
            // highlights the current best candidate, rather than letting
            // it silently drift to whatever item happened to start at
            // index 0 before higher-relevance discoveries got inserted
            // above it.
            self.cursor = 0;
        }
    }

    /// Record the computed size for the worktree at `path`, if it's still
    /// present. A no-op otherwise (e.g. the event arrived after the
    /// worktree was already removed).
    pub fn update_size(&mut self, path: &Path, bytes: u64) {
        if let Some(w) = self.items.iter_mut().find(|w| w.path == path) {
            w.size_bytes = Some(bytes);
        }
    }
}

/// Which screen the app is currently showing.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Phase {
    /// Scanning may still be in progress; the user can navigate, select,
    /// confirm, or quit at any time — they don't have to wait for the full
    /// scan to finish.
    Browsing,
    /// The user confirmed a selection; a background thread is deleting the
    /// chosen worktrees one at a time.
    Deleting,
    /// Deletion finished (or was a dry run); showing the final summary.
    Done,
}

/// The TUI's full state: the live worktree list plus scan/deletion
/// progress. Event handling (`apply_scan_event`, `apply_delete_event`,
/// `begin_deletion`) is pure state mutation with no rendering or terminal
/// I/O, so it's unit-testable on its own — see the tests below.
struct AppState {
    selector: Selector,
    scan_done: bool,
    /// The most recently visited directory, shown as a live "Checking …"
    /// line. Only the latest one matters, so repeated `Progress` events
    /// simply overwrite it rather than accumulating.
    current_scan_path: Option<PathBuf>,
    phase: Phase,
    dry_run: bool,
    force: bool,
    /// The worktree currently being removed, for the live "Deleting …" line.
    deleting_path: Option<PathBuf>,
    /// Snapshot of the worktrees selected for deletion, taken once at
    /// confirmation time. Paired positionally with `deletion_outcomes`.
    deletion_items: Vec<Worktree>,
    /// `None` until that item's `Deleted` event arrives.
    deletion_outcomes: Vec<Option<DeleteOutcome>>,
}

impl AppState {
    fn new(dry_run: bool, force: bool) -> Self {
        Self {
            selector: Selector::new(Vec::new()),
            scan_done: false,
            current_scan_path: None,
            phase: Phase::Browsing,
            dry_run,
            force,
            deleting_path: None,
            deletion_items: Vec::new(),
            deletion_outcomes: Vec::new(),
        }
    }

    fn apply_scan_event(&mut self, event: ScanEvent) {
        match event {
            ScanEvent::Progress(path) => self.current_scan_path = Some(path),
            ScanEvent::Found(wt) => self.selector.insert_found(wt),
            ScanEvent::Size(path, bytes) => {
                self.selector.update_size(&path, bytes);
                // A worktree confirmed for deletion before its size arrived
                // was snapshotted into `deletion_items` with `size_bytes:
                // None`; patch that snapshot too so `totals()` stops
                // undercounting once the size does arrive, even if that's
                // after the phase has already moved past `Browsing`.
                if let Some(w) = self.deletion_items.iter_mut().find(|w| w.path == path) {
                    w.size_bytes = Some(bytes);
                }
            }
            ScanEvent::Done => {
                self.scan_done = true;
                self.current_scan_path = None;
            }
        }
    }

    /// Snapshot the current selection and transition into the deletion
    /// phase. Returns `None` (leaving the phase unchanged) if nothing is
    /// selected, so the caller can treat that the same as cancelling —
    /// matching the pre-streaming behavior where confirming with an empty
    /// selection produced no outcomes.
    fn begin_deletion(&mut self) -> Option<Vec<Worktree>> {
        let chosen = self.selector.selected_worktrees_snapshot();
        if chosen.is_empty() {
            return None;
        }
        self.deletion_outcomes = vec![None; chosen.len()];
        self.deletion_items = chosen.clone();
        self.phase = Phase::Deleting;
        Some(chosen)
    }

    fn apply_delete_event(&mut self, event: DeleteEvent) {
        match event {
            DeleteEvent::Deleting(path) => self.deleting_path = Some(path),
            DeleteEvent::Deleted(outcome) => {
                if let Some(slot) = self
                    .deletion_items
                    .iter()
                    .position(|w| w.path == outcome.path)
                {
                    self.deletion_outcomes[slot] = Some(outcome);
                }
            }
            DeleteEvent::Done => {
                self.phase = Phase::Done;
                self.deleting_path = None;
            }
        }
    }

    /// Consume the app state, pairing each worktree selected for deletion
    /// with its outcome. A worktree with no recorded outcome (deletion
    /// events stop arriving if the user force-quits mid-run) is dropped
    /// rather than fabricating one.
    fn into_results(self) -> Vec<(Worktree, DeleteOutcome)> {
        self.deletion_items
            .into_iter()
            .zip(self.deletion_outcomes)
            .filter_map(|(w, o)| o.map(|o| (w, o)))
            .collect()
    }
}

/// Run the interactive TUI end to end: scan `root` in the background while
/// the list streams in, let the user browse and select while that happens,
/// and — once confirmed — delete the selection in the background too,
/// showing live progress for both phases.
///
/// Returns the worktrees that were attempted paired with their outcomes.
/// An empty result means the user cancelled, or confirmed with nothing
/// selected (treated the same way).
pub fn run(root: PathBuf, dry_run: bool, force: bool) -> Result<Vec<(Worktree, DeleteOutcome)>> {
    let (scan_tx, scan_rx) = mpsc::channel();
    thread::spawn(move || scan::scan_streaming(root, scan_tx));

    let mut state = AppState::new(dry_run, force);
    let mut terminal = ratatui::init();
    let should_return_results = run_loop(&mut terminal, &mut state, scan_rx);
    ratatui::restore();

    if should_return_results? {
        Ok(state.into_results())
    } else {
        Ok(Vec::new())
    }
}

/// Redraw at most this often. The walk can visit thousands of directories a
/// second; redrawing on every single event would be wasted work; capping
/// the loop's pace via `event::poll`'s timeout keeps input responsive while
/// only actually drawing a handful of times per second.
const TICK: Duration = Duration::from_millis(100);

/// Event loop: drain whichever channel is relevant to the current phase,
/// draw, then poll for a key for up to [`TICK`] (which also paces the whole
/// loop). Returns `Ok(true)` if the caller should return the accumulated
/// results, `Ok(false)` if the user cancelled.
fn run_loop(
    terminal: &mut DefaultTerminal,
    state: &mut AppState,
    scan_rx: mpsc::Receiver<ScanEvent>,
) -> Result<bool> {
    let mut delete_rx: Option<mpsc::Receiver<DeleteEvent>> = None;

    loop {
        // Drained every tick regardless of phase (not just while
        // `Phase::Browsing`): confirming deletion mid-scan must not leave
        // the rest of the walk buffering unread in an unbounded channel.
        // `Found` events still get inserted into `selector` even once it's
        // no longer rendered (past `Phase::Browsing`) — simplest choice,
        // and harmless since `selector` is entirely separate from the
        // `deletion_items` snapshot `begin_deletion` already took. `Size`
        // events additionally patch `deletion_items` directly, so a
        // worktree whose size was still pending at confirmation time gets
        // corrected in the live/final totals once it arrives, even after
        // the phase has moved on.
        while let Ok(event) = scan_rx.try_recv() {
            state.apply_scan_event(event);
        }
        if let Some(rx) = &delete_rx {
            while let Ok(event) = rx.try_recv() {
                state.apply_delete_event(event);
            }
        }

        terminal.draw(|frame| render(frame, state))?;

        if event::poll(TICK)?
            && let Event::Key(key) = event::read()?
            && key.kind == KeyEventKind::Press
        {
            if is_ctrl_c(&key) {
                // Hard abort, available in every phase but specifically the
                // only way out of `Phase::Deleting`: normal quit is
                // deliberately disabled there (see below), but a hung `git
                // worktree remove` — or a delete thread that died without
                // ever sending `DeleteEvent::Done` — would otherwise trap
                // the user with no escape. Restore the terminal ourselves
                // since `process::exit` skips the caller's `ratatui::
                // restore()` entirely.
                ratatui::restore();
                std::process::exit(130); // 128 + SIGINT, conventional for ctrl-c
            }
            match (state.phase, key.code) {
                (Phase::Browsing, KeyCode::Char('q') | KeyCode::Esc) => return Ok(false),
                (Phase::Browsing, KeyCode::Enter) => match state.begin_deletion() {
                    Some(chosen) => {
                        let (tx, rx) = mpsc::channel();
                        let dry_run = state.dry_run;
                        let force = state.force;
                        thread::spawn(move || delete::delete_streaming(chosen, dry_run, force, tx));
                        delete_rx = Some(rx);
                    }
                    // Nothing selected: confirming is equivalent to cancelling.
                    None => return Ok(false),
                },
                (Phase::Browsing, KeyCode::Down | KeyCode::Char('j')) => state.selector.move_down(),
                (Phase::Browsing, KeyCode::Up | KeyCode::Char('k')) => state.selector.move_up(),
                (Phase::Browsing, KeyCode::Char(' ') | KeyCode::Char('x')) => {
                    state.selector.toggle()
                }
                (Phase::Browsing, KeyCode::Char('a')) => state.selector.toggle_all(),
                // Deliberately no quit handling while `Phase::Deleting`:
                // destructive operations are already in flight, so we let
                // the run finish rather than risk the user thinking a
                // quit here undoes anything. Ctrl-C above is the escape
                // hatch for this phase.
                (Phase::Done, KeyCode::Enter | KeyCode::Char('q') | KeyCode::Esc) => {
                    return Ok(true);
                }
                _ => {}
            }
        }
    }
}

/// Whether `key` is the ctrl-c hard-abort combination. Split out from
/// `run_loop` so the detection logic is unit-testable without triggering
/// the `process::exit` it guards.
fn is_ctrl_c(key: &KeyEvent) -> bool {
    key.code == KeyCode::Char('c') && key.modifiers.contains(KeyModifiers::CONTROL)
}

fn render(frame: &mut Frame, state: &AppState) {
    match state.phase {
        Phase::Browsing => render_browsing(frame, state),
        Phase::Deleting | Phase::Done => render_deleting(frame, state),
    }
}

fn render_browsing(frame: &mut Frame, state: &AppState) {
    let [status_area, list_area, footer_area] = Layout::vertical([
        Constraint::Length(1),
        Constraint::Min(1),
        Constraint::Length(1),
    ])
    .areas(frame.area());

    let status = match &state.current_scan_path {
        Some(path) => format!("Checking {}", display_path(path)),
        None if state.scan_done => "Scan complete.".to_string(),
        None => "Scanning…".to_string(),
    };
    frame.render_widget(
        Line::from(status).style(Style::default().add_modifier(Modifier::DIM)),
        status_area,
    );

    let selector = &state.selector;
    let rows: Vec<ListItem> = if selector.items().is_empty() {
        let msg = if state.scan_done {
            "No git worktrees found."
        } else {
            "Scanning for worktrees…"
        };
        vec![ListItem::new(msg).style(Style::default().add_modifier(Modifier::DIM))]
    } else {
        selector
            .items()
            .iter()
            .enumerate()
            .map(|(i, w)| worktree_row(w, selector.selectable(i), selector.is_selected(i)))
            .collect()
    };

    let selected = (0..selector.items().len())
        .filter(|&i| selector.is_selected(i))
        .count();

    let list = List::new(rows)
        .block(
            Block::default()
                .borders(Borders::ALL)
                .title(" worktree-cleaner — pick worktrees to delete "),
        )
        .highlight_symbol("▶ ")
        .highlight_style(Style::default().add_modifier(Modifier::REVERSED));

    let mut list_state = ListState::default();
    if !selector.items().is_empty() {
        list_state.select(Some(selector.cursor()));
    }
    frame.render_stateful_widget(list, list_area, &mut list_state);

    let scanning_hint = if state.scan_done {
        ""
    } else {
        " · scanning…"
    };
    let footer = Line::from(format!(
        " {selected} selected ({}) · space/x toggle · a all · ↑/↓ move · enter delete · q cancel{scanning_hint}",
        format_size(selector.selected_size()),
    ))
    .style(Style::default().add_modifier(Modifier::DIM));
    frame.render_widget(footer, footer_area);
}

fn worktree_row(w: &Worktree, selectable: bool, selected: bool) -> ListItem<'static> {
    let mark = if !selectable {
        "   "
    } else if selected {
        "[x]"
    } else {
        "[ ]"
    };
    let branch = match (w.branch.as_deref(), w.merged) {
        (Some(b), true) => format!("{b} (merged)"),
        (Some(b), false) => b.to_string(),
        (None, true) => "(merged)".to_string(),
        (None, false) => "-".to_string(),
    };
    // A pending size (still being computed on a background worker) shows as
    // "…" rather than blocking the row from appearing at all.
    let size = match w.size_bytes {
        Some(bytes) => format_size(bytes),
        None => "…".to_string(),
    };
    let text = format!(
        "{mark} {:<8} {:>12} {:>9}  {:<22} {:<8} {}",
        w.status.label(),
        w.age_label(),
        size,
        branch,
        w.head.as_deref().unwrap_or("-"),
        display_path(&w.path),
    );
    ListItem::new(text).style(Style::default().fg(status_color(w.status)))
}

fn render_deleting(frame: &mut Frame, state: &AppState) {
    let [status_area, list_area, footer_area] = Layout::vertical([
        Constraint::Length(1),
        Constraint::Min(1),
        Constraint::Length(1),
    ])
    .areas(frame.area());

    let status = if state.phase == Phase::Deleting {
        match &state.deleting_path {
            Some(path) if state.dry_run => format!("(dry run) Checking {}", display_path(path)),
            Some(path) => format!("Deleting {}", display_path(path)),
            None => "Deleting…".to_string(),
        }
    } else {
        "Done.".to_string()
    };
    frame.render_widget(
        Line::from(status).style(Style::default().add_modifier(Modifier::DIM)),
        status_area,
    );

    let rows: Vec<ListItem> = state
        .deletion_items
        .iter()
        .zip(&state.deletion_outcomes)
        .map(|(w, outcome)| deletion_row(w, outcome))
        .collect();
    let list = List::new(rows).block(Block::default().borders(Borders::ALL).title(
        if state.dry_run {
            " worktree-cleaner — dry run "
        } else {
            " worktree-cleaner — deleting "
        },
    ));
    frame.render_widget(list, list_area);

    let done_count = state
        .deletion_outcomes
        .iter()
        .filter(|o| o.is_some())
        .count();
    let total = state.deletion_items.len();
    let footer_text = if state.phase == Phase::Deleting {
        format!(" {done_count}/{total} processed… · ctrl-c abort")
    } else {
        let ((freed, freed_partial), (would_free, would_free_partial)) =
            totals(&state.deletion_items, &state.deletion_outcomes);
        let mut parts = vec![format!("{done_count}/{total} done")];
        if freed > 0 || freed_partial {
            let prefix = if freed_partial { ">= " } else { "" };
            parts.push(format!("freed {prefix}{}", format_size(freed)));
        }
        if would_free > 0 || would_free_partial {
            let prefix = if would_free_partial { ">= " } else { "" };
            parts.push(format!("would free {prefix}{}", format_size(would_free)));
        }
        parts.push("press enter/q to exit".to_string());
        format!(" {}", parts.join(" · "))
    };
    frame.render_widget(
        Line::from(footer_text).style(Style::default().add_modifier(Modifier::DIM)),
        footer_area,
    );
}

/// Total bytes freed (`Removed`) and that would be freed (`DryRun`) across a
/// deletion run, used for the final summary line. Each total is paired with
/// a `partial` flag: `true` if at least one contributing worktree's size
/// was still unknown (`size_bytes: None`) when this was computed, so the
/// caller can show ">= N" instead of silently undercounting. In practice a
/// late `ScanEvent::Size` patches `AppState::deletion_items` as soon as it
/// arrives (see `apply_scan_event`), so `partial` is only ever true for the
/// brief window before that happens.
fn totals(items: &[Worktree], outcomes: &[Option<DeleteOutcome>]) -> ((u64, bool), (u64, bool)) {
    let mut freed = 0u64;
    let mut freed_partial = false;
    let mut would_free = 0u64;
    let mut would_free_partial = false;
    for (w, outcome) in items.iter().zip(outcomes) {
        let Some(outcome) = outcome else { continue };
        match outcome.action {
            DeleteAction::Removed => match w.size_bytes {
                Some(bytes) => freed += bytes,
                None => freed_partial = true,
            },
            DeleteAction::DryRun => match w.size_bytes {
                Some(bytes) => would_free += bytes,
                None => would_free_partial = true,
            },
            _ => {}
        }
    }
    ((freed, freed_partial), (would_free, would_free_partial))
}

fn deletion_row(w: &Worktree, outcome: &Option<DeleteOutcome>) -> ListItem<'static> {
    let (verb, color) = match outcome {
        Some(o) => (o.action.verb(), action_color(&o.action)),
        None => ("pending", Color::DarkGray),
    };
    let detail = outcome
        .as_ref()
        .map(|o| format!(" ({})", o.detail))
        .unwrap_or_default();
    let text = format!("{verb:<13} {}{detail}", display_path(&w.path));
    ListItem::new(text).style(Style::default().fg(color))
}

fn action_color(action: &DeleteAction) -> Color {
    match action {
        DeleteAction::Removed => Color::Green,
        DeleteAction::DryRun => Color::Yellow,
        DeleteAction::Skipped => Color::DarkGray,
        DeleteAction::Failed => Color::Red,
    }
}

/// Render `path` relative to the current directory when possible, for
/// readability — worktree paths are always absolute internally so that
/// deletion works regardless of the scan root, but a full absolute path is
/// noisy to read when the user is sitting right above it. Shared with
/// `main`'s post-exit summary so both surfaces stay consistent.
pub(crate) fn display_path(path: &Path) -> String {
    match std::env::current_dir() {
        Ok(cwd) => match path.strip_prefix(&cwd) {
            Ok(rel) if !rel.as_os_str().is_empty() => rel.display().to_string(),
            _ => path.display().to_string(),
        },
        Err(_) => path.display().to_string(),
    }
}

fn status_color(status: WorktreeStatus) -> Color {
    match status {
        WorktreeStatus::Orphaned => Color::Red,
        WorktreeStatus::Stale => Color::Yellow,
        WorktreeStatus::Active => Color::Reset,
        WorktreeStatus::MainRepo => Color::DarkGray,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testutil::fake_worktree;
    use crate::worktree::WorktreeStatus::*;

    fn selector() -> Selector {
        Selector::new(vec![
            fake_worktree("/a", Orphaned),
            fake_worktree("/b", Stale),
            fake_worktree("/c", Active),
        ])
    }

    #[test]
    fn starts_at_top_with_nothing_selected() {
        let s = selector();
        assert_eq!(s.cursor(), 0);
        assert!((0..3).all(|i| !s.is_selected(i)));
    }

    #[test]
    fn navigation_moves_and_clamps_at_bounds() {
        let mut s = selector();
        s.move_up(); // already at top
        assert_eq!(s.cursor(), 0);
        s.move_down();
        s.move_down();
        s.move_down(); // past the end
        assert_eq!(s.cursor(), 2);
        s.move_up();
        assert_eq!(s.cursor(), 1);
    }

    #[test]
    fn toggle_selects_and_deselects_item_at_cursor() {
        let mut s = selector();
        s.move_down();
        s.toggle();
        assert!(s.is_selected(1));
        assert!(!s.is_selected(0));
        s.toggle();
        assert!(!s.is_selected(1));
    }

    #[test]
    fn main_repo_rows_cannot_be_selected() {
        let mut s = Selector::new(vec![
            fake_worktree("/main", MainRepo),
            fake_worktree("/b", Stale),
        ]);
        s.toggle(); // cursor on the main repo: no-op
        assert!(!s.is_selected(0));
        assert!(!s.selectable(0));

        s.move_down();
        s.toggle();
        assert!(s.is_selected(1));
    }

    #[test]
    fn toggle_all_selects_every_selectable_row_then_clears() {
        let mut s = Selector::new(vec![
            fake_worktree("/main", MainRepo),
            fake_worktree("/a", Orphaned),
            fake_worktree("/b", Stale),
        ]);

        s.toggle_all();
        assert!(!s.is_selected(0), "main repo stays unselected");
        assert!(s.is_selected(1) && s.is_selected(2));

        s.toggle_all();
        assert!((0..3).all(|i| !s.is_selected(i)));
    }

    #[test]
    fn selected_worktrees_returns_only_chosen_rows() {
        let mut s = selector(); // /a Orphaned, /b Stale, /c Active
        s.toggle(); // /a
        s.move_down();
        s.move_down();
        s.toggle(); // /c

        let chosen = s.selected_worktrees();
        let paths: Vec<_> = chosen.iter().map(|w| w.path.to_str().unwrap()).collect();
        assert_eq!(paths, vec!["/a", "/c"]);
    }

    #[test]
    fn selected_worktrees_snapshot_does_not_consume_the_selector() {
        let mut s = selector();
        s.toggle(); // /a

        let snapshot = s.selected_worktrees_snapshot();
        assert_eq!(snapshot.len(), 1);
        assert_eq!(snapshot[0].path.to_str().unwrap(), "/a");
        // `s` is still usable afterwards.
        assert!(s.is_selected(0));
    }

    #[test]
    fn selected_size_totals_only_selected_rows() {
        let mut s = Selector::new(vec![
            Worktree {
                size_bytes: Some(100),
                ..fake_worktree("/a", Orphaned)
            },
            Worktree {
                size_bytes: Some(200),
                ..fake_worktree("/b", Stale)
            },
            Worktree {
                size_bytes: Some(400),
                ..fake_worktree("/c", Active)
            },
        ]);
        s.toggle(); // /a (100)
        s.move_down();
        s.move_down();
        s.toggle(); // /c (400)

        assert_eq!(s.selected_size(), 500);
    }

    #[test]
    fn selected_size_treats_a_pending_size_as_zero() {
        let mut s = Selector::new(vec![fake_worktree("/a", Orphaned)]); // size_bytes: None
        s.toggle();

        assert_eq!(s.selected_size(), 0);
    }

    #[test]
    fn insert_found_keeps_items_sorted_by_relevance_descending() {
        let mut s = Selector::new(Vec::new());
        // Insert out of relevance order: Active, Orphaned, Stale.
        s.insert_found(fake_worktree("/active", Active));
        s.insert_found(fake_worktree("/orphaned", Orphaned));
        s.insert_found(fake_worktree("/stale", Stale));

        let paths: Vec<_> = s.items().iter().map(|w| w.path.to_str().unwrap()).collect();
        assert_eq!(paths, vec!["/orphaned", "/stale", "/active"]);
    }

    #[test]
    fn insert_found_breaks_ties_by_keeping_the_earlier_item_on_top() {
        let mut s = Selector::new(Vec::new());
        s.insert_found(fake_worktree("/first", Stale));
        s.insert_found(fake_worktree("/second", Stale)); // same relevance

        let paths: Vec<_> = s.items().iter().map(|w| w.path.to_str().unwrap()).collect();
        assert_eq!(paths, vec!["/first", "/second"]);
    }

    #[test]
    fn insert_found_keeps_the_cursor_attached_to_the_same_worktree() {
        let mut s = Selector::new(Vec::new());
        s.insert_found(fake_worktree("/orphaned", Orphaned)); // items: [orphaned]
        s.insert_found(fake_worktree("/active", Active)); // items: [orphaned, active]
        s.move_down(); // cursor -> /active (index 1)
        assert_eq!(s.cursor(), 1);

        // Inserting something that sorts between them shifts /active to
        // index 2; the cursor must move with it, not stay frozen at index 1
        // (which would silently start pointing at the new item instead).
        s.insert_found(fake_worktree("/stale", Stale)); // items: [orphaned, stale, active]

        assert_eq!(s.cursor(), 2);
        assert_eq!(s.items()[s.cursor()].path.to_str().unwrap(), "/active");
    }

    #[test]
    fn insert_found_pins_the_cursor_to_the_top_until_the_user_moves_it() {
        // Regression test: a manual smoke run showed the cursor landing on
        // an arbitrary middle row after several `insert_found` calls with
        // no user input at all. The bug was applying the "stay attached to
        // the same item" shift logic even before the user had ever looked
        // at (moved onto) a specific row, so the cursor drifted away from
        // index 0 purely as a side effect of insertion order/timing.
        let mut s = Selector::new(Vec::new());
        s.insert_found(fake_worktree("/main", MainRepo)); // lowest relevance, index 0
        assert_eq!(s.cursor(), 0);

        // Each of these sorts *above* /main; if the cursor were (wrongly)
        // shifting to "stay attached" to /main, it would keep climbing
        // instead of staying pinned to the top of the list.
        s.insert_found(fake_worktree("/active", Active));
        assert_eq!(s.cursor(), 0);
        s.insert_found(fake_worktree("/stale", Stale));
        assert_eq!(s.cursor(), 0);
        s.insert_found(fake_worktree("/orphaned", Orphaned));
        assert_eq!(s.cursor(), 0);

        assert_eq!(
            s.items()[s.cursor()].path.to_str().unwrap(),
            "/orphaned",
            "the cursor should highlight the current top (highest-relevance) candidate"
        );
    }

    #[test]
    fn toggle_on_an_empty_selector_does_not_panic() {
        // Regression test for a confirmed crash (B1): pressing space/x with
        // an empty list — either at startup before the first `Found` event,
        // or a scanned tree with zero worktrees — indexed `self.items[0]`
        // unconditionally inside `selectable`, panicking with "index out of
        // bounds". `toggle` must be a no-op here instead.
        let mut s = Selector::new(Vec::new());
        s.toggle();
        assert!(s.items().is_empty());
    }

    #[test]
    fn toggle_when_the_cursor_is_out_of_bounds_does_not_panic() {
        // Not reachable via the public API today — `move_up`/`move_down`
        // never push the cursor past `items.len()`, and `insert_found` only
        // ever grows the list — but `toggle` guards on the cursor's bounds
        // directly rather than leaning on that invariant holding forever.
        // Pokes the private field directly since this test is in the same
        // module.
        let mut s = Selector::new(vec![fake_worktree("/a", Stale)]);
        s.cursor = 5;
        s.toggle();
        assert!(!s.selected[0], "an out-of-bounds toggle should be a no-op");
    }

    #[test]
    fn insert_found_keeps_selection_attached_to_the_same_worktree() {
        let mut s = Selector::new(Vec::new());
        s.insert_found(fake_worktree("/orphaned", Orphaned)); // index 0
        s.toggle(); // select /orphaned
        s.insert_found(fake_worktree("/active", Active)); // index 1, unselected
        s.move_down();
        assert!(!s.is_selected(1));

        // Insert something that lands between them (Stale sorts above
        // Active): /orphaned stays selected, /active (now at index 2) stays
        // unselected, and the new /stale row is not accidentally selected.
        s.insert_found(fake_worktree("/stale", Stale));

        assert_eq!(
            s.items()
                .iter()
                .map(|w| w.path.to_str().unwrap())
                .collect::<Vec<_>>(),
            vec!["/orphaned", "/stale", "/active"]
        );
        assert!(s.is_selected(0), "/orphaned should still be selected");
        assert!(!s.is_selected(1), "/stale should not be selected");
        assert!(!s.is_selected(2), "/active should still be unselected");
    }

    #[test]
    fn update_size_sets_the_matching_items_size() {
        let mut s = Selector::new(vec![fake_worktree("/a", Stale)]);
        s.update_size(Path::new("/a"), 1234);

        assert_eq!(s.items()[0].size_bytes, Some(1234));
    }

    #[test]
    fn update_size_is_a_noop_for_an_unknown_path() {
        let mut s = Selector::new(vec![fake_worktree("/a", Stale)]);
        s.update_size(Path::new("/does-not-exist"), 1234);

        assert_eq!(s.items()[0].size_bytes, None);
    }

    #[test]
    fn display_path_shortens_paths_under_the_current_directory() {
        use crate::testutil::CwdGuard;

        let tmp = tempfile::tempdir().unwrap();
        let nested = tmp.path().join("nested");
        std::fs::create_dir(&nested).unwrap();
        // `Worktree::path` is always canonicalized by `scan` in real usage
        // (e.g. resolving macOS's `/var` -> `/private/var`), so mirror that
        // here rather than passing the tempdir's raw, possibly-symlinked path.
        let nested = nested.canonicalize().unwrap();
        let _cwd_guard = CwdGuard::change_to(tmp.path());

        assert_eq!(display_path(&nested), "nested");
        // A path outside the cwd falls back to the full (absolute) path.
        assert_eq!(
            display_path(Path::new("/definitely/outside")),
            "/definitely/outside"
        );
    }

    // --- AppState reducer tests -------------------------------------------
    // These exercise event -> state transitions directly, with no terminal
    // or rendering involved.

    #[test]
    fn apply_scan_event_progress_tracks_the_latest_path_only() {
        let mut state = AppState::new(false, false);
        state.apply_scan_event(ScanEvent::Progress(PathBuf::from("/a")));
        state.apply_scan_event(ScanEvent::Progress(PathBuf::from("/b")));

        assert_eq!(state.current_scan_path, Some(PathBuf::from("/b")));
    }

    #[test]
    fn apply_scan_event_found_inserts_into_the_selector() {
        let mut state = AppState::new(false, false);
        state.apply_scan_event(ScanEvent::Found(fake_worktree("/a", Orphaned)));

        assert_eq!(state.selector.items().len(), 1);
        assert_eq!(state.selector.items()[0].path.to_str().unwrap(), "/a");
    }

    #[test]
    fn apply_scan_event_size_updates_the_matching_worktree() {
        let mut state = AppState::new(false, false);
        state.apply_scan_event(ScanEvent::Found(fake_worktree("/a", Orphaned)));
        state.apply_scan_event(ScanEvent::Size(PathBuf::from("/a"), 4096));

        assert_eq!(state.selector.items()[0].size_bytes, Some(4096));
    }

    #[test]
    fn apply_scan_event_size_also_patches_a_pending_deletion_snapshot() {
        // Regression test for B3/B4: a worktree can be confirmed for
        // deletion before its size ever arrives (`begin_deletion` snapshots
        // whatever `size_bytes` is at that moment, which may be `None`).
        // Since scan events are now drained in every phase, not just
        // `Browsing`, a `Size` event that arrives afterwards must still
        // reach the deletion snapshot — otherwise the totals shown to the
        // user stay wrong forever instead of just briefly.
        let mut state = AppState::new(false, false);
        state.apply_scan_event(ScanEvent::Found(fake_worktree("/a", Orphaned)));
        state.selector.toggle();
        state.begin_deletion();
        assert_eq!(
            state.deletion_items[0].size_bytes, None,
            "precondition: size hadn't arrived yet at confirmation time"
        );

        // Arrives late, after the phase has already moved to `Deleting`.
        state.apply_scan_event(ScanEvent::Size(PathBuf::from("/a"), 4096));

        assert_eq!(state.deletion_items[0].size_bytes, Some(4096));
    }

    #[test]
    fn apply_scan_event_done_marks_scan_done_and_clears_the_current_path() {
        let mut state = AppState::new(false, false);
        state.apply_scan_event(ScanEvent::Progress(PathBuf::from("/a")));
        state.apply_scan_event(ScanEvent::Done);

        assert!(state.scan_done);
        assert_eq!(state.current_scan_path, None);
    }

    #[test]
    fn begin_deletion_returns_none_and_stays_in_browsing_when_nothing_selected() {
        let mut state = AppState::new(false, false);
        state.apply_scan_event(ScanEvent::Found(fake_worktree("/a", Orphaned)));

        assert!(state.begin_deletion().is_none());
        assert_eq!(state.phase, Phase::Browsing);
    }

    #[test]
    fn begin_deletion_snapshots_the_selection_and_switches_to_deleting() {
        let mut state = AppState::new(false, false);
        state.apply_scan_event(ScanEvent::Found(fake_worktree("/a", Orphaned)));
        state.selector.toggle();

        let chosen = state.begin_deletion().expect("one worktree was selected");

        assert_eq!(chosen.len(), 1);
        assert_eq!(state.phase, Phase::Deleting);
        assert_eq!(state.deletion_outcomes, vec![None]);
    }

    #[test]
    fn apply_delete_event_tracks_the_currently_deleting_path() {
        let mut state = AppState::new(false, false);
        state.apply_scan_event(ScanEvent::Found(fake_worktree("/a", Orphaned)));
        state.selector.toggle();
        state.begin_deletion();

        state.apply_delete_event(DeleteEvent::Deleting(PathBuf::from("/a")));

        assert_eq!(state.deleting_path, Some(PathBuf::from("/a")));
    }

    #[test]
    fn apply_delete_event_accumulates_outcomes_by_matching_path() {
        let mut state = AppState::new(false, false);
        state.apply_scan_event(ScanEvent::Found(fake_worktree("/a", Orphaned)));
        state.apply_scan_event(ScanEvent::Found(fake_worktree("/b", Stale)));
        state.selector.toggle_all();
        state.begin_deletion();

        // Outcomes can arrive in either order; they must land on the right
        // slot regardless.
        state.apply_delete_event(DeleteEvent::Deleted(DeleteOutcome {
            path: PathBuf::from("/b"),
            action: DeleteAction::Removed,
            detail: "git worktree remove".to_string(),
        }));
        state.apply_delete_event(DeleteEvent::Deleted(DeleteOutcome {
            path: PathBuf::from("/a"),
            action: DeleteAction::Failed,
            detail: "boom".to_string(),
        }));

        let a_index = state
            .deletion_items
            .iter()
            .position(|w| w.path == Path::new("/a"))
            .unwrap();
        let b_index = state
            .deletion_items
            .iter()
            .position(|w| w.path == Path::new("/b"))
            .unwrap();
        assert_eq!(
            state.deletion_outcomes[a_index].as_ref().unwrap().action,
            DeleteAction::Failed
        );
        assert_eq!(
            state.deletion_outcomes[b_index].as_ref().unwrap().action,
            DeleteAction::Removed
        );
    }

    #[test]
    fn apply_delete_event_done_switches_phase_and_clears_deleting_path() {
        let mut state = AppState::new(false, false);
        state.apply_scan_event(ScanEvent::Found(fake_worktree("/a", Orphaned)));
        state.selector.toggle();
        state.begin_deletion();
        state.apply_delete_event(DeleteEvent::Deleting(PathBuf::from("/a")));

        state.apply_delete_event(DeleteEvent::Done);

        assert_eq!(state.phase, Phase::Done);
        assert_eq!(state.deleting_path, None);
    }

    #[test]
    fn into_results_pairs_worktrees_with_their_outcomes() {
        let mut state = AppState::new(false, false);
        state.apply_scan_event(ScanEvent::Found(fake_worktree("/a", Orphaned)));
        state.selector.toggle();
        state.begin_deletion();
        state.apply_delete_event(DeleteEvent::Deleted(DeleteOutcome {
            path: PathBuf::from("/a"),
            action: DeleteAction::Removed,
            detail: "git worktree remove".to_string(),
        }));

        let results = state.into_results();

        assert_eq!(results.len(), 1);
        assert_eq!(results[0].0.path.to_str().unwrap(), "/a");
        assert_eq!(results[0].1.action, DeleteAction::Removed);
    }

    #[test]
    fn into_results_drops_worktrees_with_no_recorded_outcome() {
        let mut state = AppState::new(false, false);
        state.apply_scan_event(ScanEvent::Found(fake_worktree("/a", Orphaned)));
        state.apply_scan_event(ScanEvent::Found(fake_worktree("/b", Stale)));
        state.selector.toggle_all();
        state.begin_deletion();
        // Only /a's outcome ever arrives (e.g. the process was interrupted).
        state.apply_delete_event(DeleteEvent::Deleted(DeleteOutcome {
            path: PathBuf::from("/a"),
            action: DeleteAction::Removed,
            detail: "git worktree remove".to_string(),
        }));

        let results = state.into_results();

        assert_eq!(results.len(), 1);
        assert_eq!(results[0].0.path.to_str().unwrap(), "/a");
    }

    #[test]
    fn is_ctrl_c_detects_the_hard_abort_combination() {
        let ctrl_c = KeyEvent::new(KeyCode::Char('c'), KeyModifiers::CONTROL);
        assert!(is_ctrl_c(&ctrl_c));

        let plain_c = KeyEvent::new(KeyCode::Char('c'), KeyModifiers::NONE);
        assert!(!is_ctrl_c(&plain_c));

        let ctrl_x = KeyEvent::new(KeyCode::Char('x'), KeyModifiers::CONTROL);
        assert!(!is_ctrl_c(&ctrl_x));
    }

    #[test]
    fn totals_reports_known_bytes_and_flags_partial_when_a_size_is_still_pending() {
        let items = vec![
            Worktree {
                size_bytes: Some(100),
                ..fake_worktree("/a", Orphaned)
            },
            Worktree {
                size_bytes: None, // still pending
                ..fake_worktree("/b", Stale)
            },
        ];
        let outcomes = vec![
            Some(DeleteOutcome {
                path: PathBuf::from("/a"),
                action: DeleteAction::Removed,
                detail: "git worktree remove".to_string(),
            }),
            Some(DeleteOutcome {
                path: PathBuf::from("/b"),
                action: DeleteAction::Removed,
                detail: "git worktree remove".to_string(),
            }),
        ];

        let ((freed, freed_partial), (would_free, would_free_partial)) = totals(&items, &outcomes);

        assert_eq!(freed, 100, "should sum the known size");
        assert!(freed_partial, "should flag that /b's size wasn't known");
        assert_eq!(would_free, 0);
        assert!(!would_free_partial);
    }

    #[test]
    fn totals_is_not_partial_once_every_relevant_size_is_known() {
        let items = vec![Worktree {
            size_bytes: Some(100),
            ..fake_worktree("/a", Orphaned)
        }];
        let outcomes = vec![Some(DeleteOutcome {
            path: PathBuf::from("/a"),
            action: DeleteAction::Removed,
            detail: "git worktree remove".to_string(),
        })];

        let ((freed, freed_partial), _) = totals(&items, &outcomes);

        assert_eq!(freed, 100);
        assert!(!freed_partial);
    }

    #[test]
    fn totals_ignores_worktrees_with_no_outcome_yet() {
        // Not yet processed (`None` outcome): shouldn't be counted or
        // flagged as partial — it isn't a `Removed`/`DryRun` outcome (yet).
        let items = vec![Worktree {
            size_bytes: None,
            ..fake_worktree("/a", Stale)
        }];
        let outcomes: Vec<Option<DeleteOutcome>> = vec![None];

        let ((freed, freed_partial), (would_free, would_free_partial)) = totals(&items, &outcomes);

        assert_eq!((freed, would_free), (0, 0));
        assert!(!freed_partial);
        assert!(!would_free_partial);
    }
}
