use anyhow::Result;
use ratatui::crossterm::event::{self, Event, KeyCode, KeyEventKind};
use ratatui::layout::{Constraint, Layout};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::Line;
use ratatui::widgets::{Block, Borders, List, ListItem, ListState};
use ratatui::{DefaultTerminal, Frame};

use crate::worktree::{Worktree, WorktreeStatus};

/// Interactive selection state over a ranked list of worktrees. Pure logic,
/// independent of rendering, so it can be unit-tested without a terminal.
pub struct Selector {
    items: Vec<Worktree>,
    cursor: usize,
    selected: Vec<bool>,
}

impl Selector {
    pub fn new(items: Vec<Worktree>) -> Self {
        let selected = vec![false; items.len()];
        Self {
            items,
            cursor: 0,
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
        if self.cursor + 1 < self.items.len() {
            self.cursor += 1;
        }
    }

    pub fn move_up(&mut self) {
        self.cursor = self.cursor.saturating_sub(1);
    }

    /// Whether row `i` may be selected for deletion (the main working tree
    /// never can).
    pub fn selectable(&self, i: usize) -> bool {
        self.items[i].status != WorktreeStatus::MainRepo
    }

    pub fn toggle(&mut self) {
        let i = self.cursor;
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

    /// Consume the selector, returning the worktrees the user chose.
    pub fn selected_worktrees(self) -> Vec<Worktree> {
        self.items
            .into_iter()
            .zip(self.selected)
            .filter_map(|(w, sel)| sel.then_some(w))
            .collect()
    }
}

/// Run the interactive selection TUI and return the worktrees the user chose to
/// delete.
///
/// TODO(#4): ratatui + crossterm stateful list with multi-select, a relevance
/// column, status badges, and a final confirmation step before returning.
pub fn select_for_deletion(worktrees: Vec<Worktree>) -> Result<Vec<Worktree>> {
    if worktrees.is_empty() {
        return Ok(Vec::new());
    }

    let mut selector = Selector::new(worktrees);
    let mut terminal = ratatui::init();
    let confirmed = run(&mut terminal, &mut selector);
    ratatui::restore();

    if confirmed? {
        Ok(selector.selected_worktrees())
    } else {
        Ok(Vec::new())
    }
}

/// Event loop: draw, read a key, update state. Returns whether the user
/// confirmed (`enter`) or cancelled (`q`/`esc`).
fn run(terminal: &mut DefaultTerminal, selector: &mut Selector) -> Result<bool> {
    loop {
        terminal.draw(|frame| render(frame, selector))?;

        let Event::Key(key) = event::read()? else {
            continue;
        };
        if key.kind != KeyEventKind::Press {
            continue;
        }
        match key.code {
            KeyCode::Char('q') | KeyCode::Esc => return Ok(false),
            KeyCode::Enter => return Ok(true),
            KeyCode::Down | KeyCode::Char('j') => selector.move_down(),
            KeyCode::Up | KeyCode::Char('k') => selector.move_up(),
            KeyCode::Char(' ') | KeyCode::Char('x') => selector.toggle(),
            KeyCode::Char('a') => selector.toggle_all(),
            _ => {}
        }
    }
}

fn render(frame: &mut Frame, selector: &Selector) {
    let [list_area, footer_area] =
        Layout::vertical([Constraint::Min(1), Constraint::Length(1)]).areas(frame.area());

    let rows: Vec<ListItem> = selector
        .items()
        .iter()
        .enumerate()
        .map(|(i, w)| {
            let mark = if !selector.selectable(i) {
                "   "
            } else if selector.is_selected(i) {
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
            let text = format!(
                "{mark} {:<8} {:>12}  {:<22} {:<8} {}",
                w.status.label(),
                w.age_label(),
                branch,
                w.head.as_deref().unwrap_or("-"),
                w.path.display(),
            );
            ListItem::new(text).style(Style::default().fg(status_color(w.status)))
        })
        .collect();

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

    let mut state = ListState::default();
    state.select(Some(selector.cursor()));
    frame.render_stateful_widget(list, list_area, &mut state);

    let footer = Line::from(format!(
        " {selected} selected · space/x toggle · a all · ↑/↓ move · enter delete · q cancel"
    ))
    .style(Style::default().add_modifier(Modifier::DIM));
    frame.render_widget(footer, footer_area);
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
}
