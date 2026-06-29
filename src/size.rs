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
    fn formats_sizes_human_readably() {
        assert_eq!(format_size(0), "0 B");
        assert_eq!(format_size(512), "512 B");
        assert_eq!(format_size(1024), "1.0 KB");
        assert_eq!(format_size(1536), "1.5 KB");
        assert_eq!(format_size(1024 * 1024), "1.0 MB");
        assert_eq!(format_size(5 * 1024 * 1024 * 1024), "5.0 GB");
    }
}
