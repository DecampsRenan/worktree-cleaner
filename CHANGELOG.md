# Changelog

All notable user-facing changes are documented here.

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
cargo install worktree-cleaner --version 0.3.0
```
