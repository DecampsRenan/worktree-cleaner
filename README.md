# worktree-cleaner (`wtc`)

[![CI](https://github.com/DecampsRenan/worktree-cleaner/actions/workflows/ci.yml/badge.svg)](https://github.com/DecampsRenan/worktree-cleaner/actions/workflows/ci.yml)
[![License: MIT](https://img.shields.io/badge/license-MIT-blue.svg)](LICENSE)

A terminal UI for reclaiming disk and mental space from stray **git worktrees**.

Run it anywhere: it walks the folder tree, finds every git worktree, ranks them by how worth-deleting they are (orphaned first, then stale, never the main checkout), and drops you into an interactive list. Tick the ones to remove, confirm, done.

`git worktree prune` only knows about worktrees still registered to a repo it can find. `wtc` scans the filesystem instead, so it also catches **orphaned** ones — directories whose backing repo or branch is already gone.

## Install

Needs `git` on your `PATH` at runtime (`wtc` shells out to `git worktree remove`).

### Homebrew

```bash
brew install decampsrenan/tap/worktree-cleaner
```

Builds from source; a Rust toolchain is pulled in as a build dependency. Add `--HEAD` to track latest `main`.

### Cargo

```bash
cargo install worktree-cleaner
```

Needs Rust 1.88 or newer. Installs the `wtc` binary into `~/.cargo/bin`.

### From this repo

```bash
git clone https://github.com/DecampsRenan/worktree-cleaner
cd worktree-cleaner
cargo install --path .
```

## Usage

```bash
wtc                 # scan the current directory tree (TUI when stdin/stdout are TTYs)
wtc ~/code          # scan a specific root
wtc --dry-run       # show what would be deleted, delete nothing
wtc --help          # all flags
```

Not sure yet? Start with `wtc --dry-run`: same walk and ranking, nothing deleted.

### Non-interactive mode (scripts, CI, agents)

When **stdin or stdout is not a TTY** (piped, redirected, or running in CI), `wtc` prints a ranked tab-separated table and exits without opening the TUI or deleting anything:

```bash
wtc ~/code | less          # list worktrees ranked by relevance
wtc --dry-run ~/code       # same table, with a DELETE column marking selectable rows
```

To delete without the TUI, pass **`--yes`** / **`-y`** (this replaces any confirmation prompt — there is never a keypress wait):

```bash
wtc --yes ~/code                    # delete all selectable worktrees
wtc --yes --dry-run ~/code          # print what would be deleted, delete nothing
wtc --yes --orphaned ~/code         # only orphaned worktrees
wtc --yes --stale ~/code            # orphaned + stale worktrees
wtc --yes --force ~/code            # also remove worktrees with local changes
```

Without `--force`, worktrees with uncommitted or untracked changes are **skipped** and listed on stderr. The main working tree is never deleted.

`ctrl-c` still aborts at any point.

### Interactive TUI

When stdin and stdout are both TTYs and `--yes` is not passed, `wtc` opens the interactive list. Worktrees are listed best-deletion-candidate first, with columns for status, age, reclaimable size, branch (tagged `(merged)` when merged), and path:

| Key | Action |
| --- | --- |
| `↑`/`↓` or `k`/`j` | move |
| `space` / `x` | toggle the row |
| `a` | toggle all selectable rows |
| `s` | cycle the sort column |
| `S` | reverse the current sort direction |
| `enter` | delete the selected worktrees |
| `q` / `esc` | cancel (delete nothing) |

The footer shows how many are selected and their total reclaimable size. The main working tree is greyed out and can never be selected.

The scan streams: rows appear (and re-rank) as they're found, so you can start picking before it finishes. Sizes show as `…` until they land. `ctrl-c` aborts at any point.

### How deletion works

- **Healthy linked worktree** → `git worktree remove`. If it has uncommitted or untracked changes, the TUI lists those worktrees and asks for an explicit `enter` before removing them.
- **Orphaned worktree** (repo or admin dir gone) → the directory is removed from the filesystem.
- **Main working tree** → never deleted.

A failure on one worktree never aborts the others; each gets a line in the summary, which also reports the space freed.

## What it never touches

- The main working tree of any repository — greyed out, unselectable.
- Anything you didn't tick. Nothing is deleted implicitly.
- Anything under `node_modules`, `target`, `.cargo`, or `.cache` — skipped during the walk.

## Relevance ranking

Worktrees are ordered by a relevance score:

- **status** — orphaned (repo/branch gone) > stale (no activity for 30+ days) > active; the main working tree is excluded entirely
- **merged** — a branch already merged into the repo's default branch outranks an unmerged peer of the same status and age
- **age** — time since the most recent commit / filesystem activity

Within a status tier, merged and age refine the order but never lift a worktree into a different tier.

## Development

```bash
cargo run -- --dry-run              # run against the current directory
cargo build --release               # produce target/release/wtc
cargo fmt --all --check
cargo clippy --all-targets -- -D warnings
cargo test
```

The test suite creates real repositories and worktrees in temp directories via the `git` CLI, so `git` is needed to run it too.

Built with [ratatui](https://ratatui.rs/) + [crossterm](https://github.com/crossterm-rs/crossterm),
[ignore](https://docs.rs/ignore) for traversal, and [git2](https://docs.rs/git2)
for worktree introspection.

## License

MIT — see [LICENSE](LICENSE).
