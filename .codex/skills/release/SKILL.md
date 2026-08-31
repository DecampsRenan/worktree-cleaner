---
name: release
description: Release the worktree-cleaner Rust CLI. Use in this repository when the user asks to prepare, cut, publish, tag, or automate a worktree-cleaner release; bump its version; publish it to crates.io; or update its Homebrew formula.
---

# Release worktree-cleaner

Release this repository through its existing GitHub Actions workflows. Preserve unrelated working-tree changes and never assume that a tag creates a GitHub Release: here, a tag updates the Homebrew tap while crates.io publication is a separate manual workflow.

## Inspect the release state

Read these files before proposing a release:

- `Cargo.toml` for the package version and MSRV;
- `.github/workflows/ci.yml` for the required checks;
- `.github/workflows/publish.yml` for crates.io publication;
- `.github/workflows/bump-homebrew.yml` for the Homebrew update;
- `README.md` for user-facing installation and version guidance.
- `CHANGELOG.md`, when present, for the published release history.

Also inspect the current branch, worktree status, latest `v*` tag, and configured remote. Report the proposed semantic version, the latest tag, and any uncommitted changes that are outside the release itself. Do not overwrite or include unrelated changes in a release commit.

The expected release contract is:

1. `Cargo.toml` contains the new version, greater than the latest crates.io version.
2. CI succeeds on the release commit: formatting, Clippy, tests, and MSRV check.
3. The manually dispatched `Publish to crates.io` workflow tests and publishes with `CARGO_REGISTRY_TOKEN`.
4. Pushing tag `v<version>` triggers `Bump Homebrew formula`, which updates `DecampsRenan/homebrew-tap` using `HOMEBREW_TAP_TOKEN`.
5. A GitHub Release is created from that pushed tag with release notes derived from the version's changelog entry.

## Prepare and validate

Make the minimal repository changes needed for the requested version. Update `Cargo.toml`; update the lockfile only if Cargo requires it; and add a version entry to `CHANGELOG.md`. Use [the release-notes template](references/release-notes-template.md) when drafting that entry and the matching GitHub Release body. Keep release notes factual, user-facing, and limited to changes since the previous release.

Run the same checks as CI before requesting approval:

```bash
cargo fmt --all --check
cargo clippy --all-targets --locked -- -D warnings
cargo test --all-targets --locked
cargo check --all-targets --locked   # with Rust 1.88.0 when available
```

If the requested version is missing, recommend a SemVer version from the changes but ask the user to choose it. Report the exact release diff and validation outcomes.

## Approval boundary

Pause and get explicit approval before any external or durable mutation:

- creating the release commit or pushing it to `main`;
- dispatching the crates.io publish workflow;
- creating or pushing `v<version>`;
- triggering or manually re-running the Homebrew workflow.
- creating or editing the associated GitHub Release.

The approval request must state the version, commit or branch, and action order. A request to “prepare a release” does not authorize publishing, tagging, or pushing.

## Execute after approval

Follow this order unless the user asks otherwise:

1. Commit the approved release changes and push the target branch; wait for CI to pass.
2. Manually dispatch `Publish to crates.io` from the release commit and wait for it to succeed.
3. Create and push `v<version>` pointing to that same commit.
4. Monitor the Homebrew workflow triggered by the tag. If its token was unavailable, re-run it manually with the existing tag input after the user resolves access.
5. Create the GitHub Release for the already-pushed tag with the approved changelog body, for example with `gh release create v<version> --title "v<version>" --notes-file <notes-file> --verify-tag`.

Do not upload binaries unless the user explicitly asks: this repository has no workflow that does so automatically.

## Verify and report

Verify the crates.io publish workflow, the tag on the intended commit, the Homebrew bump workflow, and the GitHub Release body. State which channels are live, any failed or skipped channel, and the exact recovery action. Include the release version and tag in the final handoff.
