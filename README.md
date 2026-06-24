# worktree-cleaner (`wtc`)

A terminal UI for reclaiming disk and mental space from stray **git worktrees**.

Run it in any directory and it walks the subfolder tree, finds every git
worktree, ranks them by how worth-deleting they are (orphaned first, then stale,
never the main checkout), and drops you into an interactive list where you pick
the ones to remove.

## Why

If you create a worktree per branch/PR, they pile up: branches get merged or
deleted upstream, directories get abandoned, and `git worktree prune` only knows
about worktrees registered to a repo it can find. `wtc` scans the filesystem
instead, so it catches **orphaned** worktrees too — ones whose backing repo or
branch is already gone.

## Status

🚧 Early scaffold. The CLI surface, module layout, and data model are in place;
the traversal, scoring, TUI, and deletion logic are tracked as GitHub issues.

## Usage (planned)

```bash
wtc                 # scan the current directory tree
wtc ~/code          # scan a specific root
wtc --dry-run       # show what would be deleted, delete nothing
```

In the TUI:

- worktrees are listed best-deletion-candidate first
- `space` / `x` toggles selection, arrows move
- `enter` confirms and deletes the selected worktrees

## Relevance ranking

Each worktree gets a relevance score from:

- **status** — orphaned (repo/branch gone) > stale > active; the main working
  tree is never offered for deletion
- **age** — time since the last commit on its HEAD
- **activity** — filesystem mtime of the worktree
- **merged** — whether its branch is already merged

## Development

```bash
cargo run -- --dry-run    # run against the current directory
cargo build --release     # produce target/release/wtc
cargo clippy --all-targets
cargo test
```

Built with [ratatui](https://ratatui.rs/) + [crossterm](https://github.com/crossterm-rs/crossterm),
[ignore](https://docs.rs/ignore) for traversal, and [git2](https://docs.rs/git2)
for worktree introspection.

## License

MIT
