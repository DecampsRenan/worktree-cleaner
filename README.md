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

## Install

### Homebrew

```bash
brew install decampsrenan/tap/worktree-cleaner
```

This builds from source, so a Rust toolchain is pulled in automatically as a
build dependency (nothing to install by hand). For the bleeding edge, add
`--HEAD` to build from the latest `main`.

### From source

```bash
cargo install --path .        # installs the `wtc` binary
# or run without installing:
cargo run -- <args>
```

## Usage

```bash
wtc                 # scan the current directory tree
wtc ~/code          # scan a specific root
wtc --dry-run       # show what would be deleted, delete nothing
wtc --force         # also remove worktrees with uncommitted/untracked changes
```

In the TUI, worktrees are listed best-deletion-candidate first, each row showing
its status, age, reclaimable size, branch (tagged `(merged)` when merged), and
short HEAD:

| Key | Action |
| --- | --- |
| `↑`/`↓` or `k`/`j` | move |
| `space` / `x` | toggle the row |
| `a` | toggle all selectable rows |
| `enter` | delete the selected worktrees |
| `q` / `esc` | cancel (delete nothing) |

The footer shows how many worktrees are selected and their total reclaimable
size. The main working tree is shown greyed out and can never be selected.

### How deletion works

- **Healthy linked worktree** → `git worktree remove`. If it's dirty
  (uncommitted or untracked changes) the removal is refused and reported as
  failed — pass `--force` to remove it anyway.
- **Orphaned worktree** (its repo or admin dir is gone) → the directory is
  removed from the filesystem.
- **Main working tree** → never deleted.

A failure on one worktree never aborts the others; each gets a line in the
summary, which also reports the total space freed.

## Relevance ranking

Worktrees are ordered by a relevance score:

- **status** — orphaned (repo/branch gone) > stale (no activity for 30+ days) >
  active; the main working tree is excluded entirely
- **merged** — a branch already merged into the repo's default branch outranks
  an unmerged peer of the same status and age
- **age** — time since the most recent commit / filesystem activity

Within a status tier, merged and age refine the order but never lift a worktree
into a different tier.

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
