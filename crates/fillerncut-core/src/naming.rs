//! Automatic file names: a sortable timestamp, never overwriting an existing file.

use std::path::{Path, PathBuf};

/// `2026-10-01_15-42-07` — sorts chronologically in Finder.
pub fn timestamp_stem(year: i32, month: u32, day: u32, hour: u32, minute: u32, second: u32) -> String {
    format!("{year:04}-{month:02}-{day:02}_{hour:02}-{minute:02}-{second:02}")
}

/// `<dir>/<stem>.<ext>`, or `<stem>-2`, `<stem>-3`… when that name is taken
/// (e.g. two exports within the same second).
pub fn unique_path(dir: &Path, stem: &str, ext: &str) -> PathBuf {
    let first = dir.join(format!("{stem}.{ext}"));
    if !first.exists() {
        return first;
    }
    (2..)
        .map(|n| dir.join(format!("{stem}-{n}.{ext}")))
        .find(|p| !p.exists())
        .expect("an unused name exists")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn stems_are_zero_padded_and_sortable() {
        assert_eq!(timestamp_stem(2026, 3, 4, 5, 6, 7), "2026-03-04_05-06-07");
        assert!(timestamp_stem(2026, 3, 4, 5, 6, 7) < timestamp_stem(2026, 3, 4, 5, 6, 8));
        assert!(timestamp_stem(2026, 3, 4, 9, 59, 59) < timestamp_stem(2026, 3, 4, 10, 0, 0));
    }

    #[test]
    fn never_overwrites() {
        let dir = tempfile::tempdir().unwrap();
        let a = unique_path(dir.path(), "x", "mp4");
        assert_eq!(a, dir.path().join("x.mp4"));
        std::fs::write(&a, b"1").unwrap();
        let b = unique_path(dir.path(), "x", "mp4");
        assert_eq!(b, dir.path().join("x-2.mp4"));
        std::fs::write(&b, b"1").unwrap();
        assert_eq!(unique_path(dir.path(), "x", "mp4"), dir.path().join("x-3.mp4"));
    }
}
