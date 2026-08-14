//! Disk-size helpers for reporting how much space a worktree would reclaim.

use std::path::Path;

/// Total size in bytes of all regular files under `path`, recursively.
///
/// Counts everything (including `node_modules`, `target`, etc.) because
/// deleting the worktree frees all of it. Symlinks are not followed.
pub fn directory_size(path: &Path) -> u64 {
    let Ok(entries) = std::fs::read_dir(path) else {
        return 0;
    };
    let mut total = 0;
    for entry in entries.flatten() {
        let Ok(file_type) = entry.file_type() else {
            continue;
        };
        if file_type.is_symlink() {
            continue;
        } else if file_type.is_dir() {
            total += directory_size(&entry.path());
        } else if let Ok(meta) = entry.metadata() {
            total += meta.len();
        }
    }
    total
}

/// A byte total that may be an underestimate.
///
/// Worktree sizes are computed by background workers, so a worktree can be
/// acted on before its size is known. Rather than silently counting such a
/// worktree as 0, it's recorded in `partial`, letting callers render
/// `">= 1.2 GB"` instead of a confidently wrong `"1.2 GB"`.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct Amount {
    known: u64,
    partial: bool,
}

impl Amount {
    /// Add `bytes` to the total, or — given `None` — mark the total as an
    /// underestimate because that contributor's size wasn't known.
    pub fn add(&mut self, bytes: Option<u64>) {
        match bytes {
            // Saturating rather than `+=`: overflowing a `u64` of bytes
            // would take exabytes of worktrees, but a debug-build panic in
            // the middle of reporting a deletion is a worse failure than a
            // capped number.
            Some(bytes) => self.known = self.known.saturating_add(bytes),
            None => self.partial = true,
        }
    }

    /// Human-readable total, or `None` when there's nothing to report at all
    /// (no bytes counted and nothing pending), so callers can omit the line
    /// entirely rather than printing a bare `"0 B"`.
    pub fn label(self) -> Option<String> {
        if self.known == 0 && !self.partial {
            return None;
        }
        let prefix = if self.partial { ">= " } else { "" };
        Some(format!("{prefix}{}", format_size(self.known)))
    }
}

/// Render a byte count as a short human-readable string (e.g. `1.5 KB`,
/// `124 MB`). Whole bytes below 1 KB; one decimal place above.
pub fn format_size(bytes: u64) -> String {
    const UNITS: [&str; 5] = ["B", "KB", "MB", "GB", "TB"];
    if bytes < 1024 {
        return format!("{bytes} B");
    }
    let mut size = bytes as f64;
    let mut unit = 0;
    while size >= 1024.0 && unit < UNITS.len() - 1 {
        size /= 1024.0;
        unit += 1;
    }
    format!("{size:.1} {}", UNITS[unit])
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    #[test]
    fn sums_file_sizes_recursively() {
        let tmp = tempdir().unwrap();
        std::fs::write(tmp.path().join("a.txt"), vec![0u8; 100]).unwrap();
        std::fs::create_dir(tmp.path().join("sub")).unwrap();
        std::fs::write(tmp.path().join("sub/b.txt"), vec![0u8; 50]).unwrap();

        assert_eq!(directory_size(tmp.path()), 150);
    }

    #[test]
    fn an_amount_with_nothing_to_report_has_no_label() {
        assert_eq!(Amount::default().label(), None);
    }

    #[test]
    fn an_amount_sums_known_sizes() {
        let mut amount = Amount::default();
        amount.add(Some(1024));
        amount.add(Some(512));

        assert_eq!(amount.label().as_deref(), Some("1.5 KB"));
    }

    #[test]
    fn an_amount_with_a_pending_size_is_reported_as_a_lower_bound() {
        let mut amount = Amount::default();
        amount.add(Some(1024));
        amount.add(None); // still being computed

        assert_eq!(amount.label().as_deref(), Some(">= 1.0 KB"));
    }

    #[test]
    fn an_amount_of_only_pending_sizes_still_reports_a_lower_bound() {
        // Nothing countable yet, but "nothing to report" would be a lie —
        // something *was* removed, its size just isn't known.
        let mut amount = Amount::default();
        amount.add(None);

        assert_eq!(amount.label().as_deref(), Some(">= 0 B"));
    }

    #[test]
    fn formats_sizes_human_readably() {
        assert_eq!(format_size(0), "0 B");
        assert_eq!(format_size(512), "512 B");
        assert_eq!(format_size(1024), "1.0 KB");
        assert_eq!(format_size(1536), "1.5 KB");
        assert_eq!(format_size(1024 * 1024), "1.0 MB");
        assert_eq!(format_size(5 * 1024 * 1024 * 1024), "5.0 GB");
    }
}
