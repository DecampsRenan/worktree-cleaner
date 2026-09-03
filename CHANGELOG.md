# Changelog

All notable user-facing changes are documented here.

## [0.4.0] - 2026-09-03

### Added

- Non-interactive mode for scripts and agents: `--yes`/`-y` deletes all
  selectable worktrees without opening the TUI.
- `--force` (non-interactive only) also removes worktrees with local changes.
- `--orphaned` and `--stale` deletion filters.
- When stdout is not a TTY and `--yes` is not given, the ranked worktree list is
  printed as a TSV table and the command exits without prompting.

## [0.3.0] - 2026-08-31

### Added

- Sortable worktree list, with sorting by relevance, age, size, branch, or path.
- Explicit confirmation before deleting selected worktrees that contain local changes.

### Changed

- The worktree list now uses labeled columns for status, age, size, branch, and path.

### Removed

- The `--force` CLI option; deletion of dirty worktrees is now confirmed in the TUI.

## Install

```bash
brew upgrade worktree-cleaner
# or
cargo install worktree-cleaner --version 0.4.0
```
