# Release notes template

Use this template for both the next `CHANGELOG.md` entry and its GitHub Release body. Remove empty sections rather than publishing placeholders. Keep the notes to user-visible changes since the previous version.

```markdown
## [{{version}}] - {{yyyy-mm-dd}}

### Added

- {{new user-facing capability}}

### Changed

- {{changed behavior or workflow}}

### Fixed

- {{user-visible defect corrected}}

### Removed

- {{removed behavior, option, or compatibility path}}

## Install

    brew upgrade worktree-cleaner
    # or
    cargo install worktree-cleaner --version {{version}}
```

For GitHub, use `# worktree-cleaner {{version}}` as the release title and render the matching version entry below it. Do not claim that a distribution channel is available until its publication workflow has succeeded.
