# worktree-cleaner

`wtc` — a Rust + ratatui TUI that traverses a directory tree, finds git
worktrees, ranks them by relevance, and lets the user interactively delete
orphaned or stale ones.

## Stack

- Rust (edition 2024), binary crate, binary name `wtc`
- [ratatui](https://ratatui.rs/) + [crossterm](https://github.com/crossterm-rs/crossterm) for the TUI
- [ignore](https://docs.rs/ignore) for filesystem traversal
- [git2](https://docs.rs/git2) for worktree introspection
- [clap](https://docs.rs/clap) (derive) for the CLI

## Module layout

- `src/main.rs` — CLI args (clap) + the scan → rank → select → delete pipeline
- `src/worktree.rs` — `Worktree` data model + `WorktreeStatus`
- `src/scan.rs` — recursive traversal + worktree detection
- `src/score.rs` — relevance scoring + ranking
- `src/tui.rs` — interactive multi-select TUI
- `src/delete.rs` — safe deletion (with `--dry-run`)

## Commands

```bash
cargo run -- --dry-run     # run against the current directory
cargo build --release      # produce target/release/wtc
cargo clippy --all-targets # lint
cargo test                 # tests
```

## Agent skills

### Issue tracker

Issues live in this repo's GitHub Issues; use the `gh` CLI. See `docs/agents/issue-tracker.md`.

### Triage labels

Canonical triage vocabulary (`needs-triage`, `needs-info`, `ready-for-agent`, `ready-for-human`, `wontfix`). See `docs/agents/triage-labels.md`.

### Domain docs

Single-context repo (`CONTEXT.md` + `docs/adr/` at the root). See `docs/agents/domain.md`.
