//! History of downloads and exports, so finished work is easy to find again.
//!
//! Stored as `<dir>/history.json` with small preview images in `<dir>/thumbs/`.
//! The list only *describes* files: removing an entry never deletes the video.

use crate::settings::write_json_atomic;
use serde::{Deserialize, Serialize};
use std::fs;
use std::io;
use std::path::{Path, PathBuf};

/// Oldest entries are dropped beyond this many.
pub const MAX_ENTRIES: usize = 500;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum HistoryKind {
    Download,
    Export,
}

/// What the caller knows when something has just been made.
#[derive(Debug, Clone, Default)]
pub struct NewEntry {
    pub path: String,
    pub title: Option<String>,
    pub source_url: Option<String>,
    pub platform: Option<String>,
    pub bytes: Option<u64>,
    pub duration: Option<f64>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct HistoryEntry {
    pub id: String,
    pub kind: HistoryKind,
    pub path: String,
    /// Unix seconds.
    pub created_at: i64,
    pub title: Option<String>,
    pub source_url: Option<String>,
    pub platform: Option<String>,
    pub bytes: Option<u64>,
    pub duration: Option<f64>,
    #[serde(default)]
    pub has_thumb: bool,
    /// Filled in when listing: does the video still exist?
    #[serde(default, skip_deserializing)]
    pub exists: bool,
    /// Filled in when listing: where the preview image lives.
    #[serde(default, skip_deserializing)]
    pub thumb_path: Option<String>,
}

#[derive(Debug, Default, Serialize, Deserialize)]
struct HistoryFile {
    items: Vec<HistoryEntry>,
}

pub struct HistoryStore {
    dir: PathBuf,
}

impl HistoryStore {
    pub fn new(dir: impl Into<PathBuf>) -> HistoryStore {
        HistoryStore { dir: dir.into() }
    }

    fn index_path(&self) -> PathBuf {
        self.dir.join("history.json")
    }

    pub fn thumbs_dir(&self) -> PathBuf {
        self.dir.join("thumbs")
    }

    pub fn thumb_path(&self, id: &str) -> PathBuf {
        self.thumbs_dir().join(format!("{id}.jpg"))
    }

    fn load(&self) -> HistoryFile {
        fs::read_to_string(self.index_path())
            .ok()
            .and_then(|t| serde_json::from_str(&t).ok())
            .unwrap_or_default()
    }

    fn save(&self, file: &HistoryFile) -> io::Result<()> {
        write_json_atomic(&self.index_path(), file)
    }

    /// Record something that was just made. `now` is unix seconds.
    pub fn add(&self, kind: HistoryKind, new: NewEntry, now: i64) -> io::Result<HistoryEntry> {
        let entry = HistoryEntry {
            id: uuid::Uuid::new_v4().simple().to_string(),
            kind,
            path: new.path,
            created_at: now,
            title: new.title,
            source_url: new.source_url,
            platform: new.platform,
            bytes: new.bytes,
            duration: new.duration,
            has_thumb: false,
            exists: true,
            thumb_path: None,
        };
        let mut file = self.load();
        file.items.push(entry.clone());
        while file.items.len() > MAX_ENTRIES {
            let old = file.items.remove(0);
            let _ = fs::remove_file(self.thumb_path(&old.id));
        }
        self.save(&file)?;
        Ok(entry)
    }

    /// Note that a preview image was written for `id`.
    pub fn mark_thumb(&self, id: &str) -> io::Result<()> {
        let mut file = self.load();
        if let Some(e) = file.items.iter_mut().find(|e| e.id == id) {
            e.has_thumb = true;
            self.save(&file)?;
        }
        Ok(())
    }

    /// Newest first, with `exists` / `thumb_path` filled in.
    pub fn list(&self) -> Vec<HistoryEntry> {
        let mut items = self.load().items;
        items.reverse();
        for e in &mut items {
            e.exists = Path::new(&e.path).is_file();
            let thumb = self.thumb_path(&e.id);
            e.thumb_path = (e.has_thumb && thumb.is_file()).then(|| thumb.to_string_lossy().into_owned());
        }
        items
    }

    pub fn remove(&self, id: &str) -> io::Result<()> {
        let mut file = self.load();
        file.items.retain(|e| e.id != id);
        self.save(&file)?;
        let _ = fs::remove_file(self.thumb_path(id));
        Ok(())
    }

    pub fn clear(&self) -> io::Result<()> {
        self.save(&HistoryFile::default())?;
        let _ = fs::remove_dir_all(self.thumbs_dir());
        Ok(())
    }
}

/// ffmpeg arguments for a small preview image of `input` (a frame shortly after the start).
pub fn build_thumbnail_args(input: &str, output: &str, at_seconds: f64) -> Vec<String> {
    [
        "-hide_banner",
        "-nostdin",
        "-y",
        "-ss",
        &format!("{at_seconds:.2}"),
        "-i",
        input,
        "-frames:v",
        "1",
        "-vf",
        "scale=240:-2",
        "-q:v",
        "5",
        output,
    ]
    .iter()
    .map(|s| s.to_string())
    .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn store() -> (tempfile::TempDir, HistoryStore) {
        let dir = tempfile::tempdir().unwrap();
        let s = HistoryStore::new(dir.path().join("history"));
        (dir, s)
    }

    fn new(path: &Path) -> NewEntry {
        NewEntry {
            path: path.to_string_lossy().into_owned(),
            ..Default::default()
        }
    }

    #[test]
    fn newest_first_and_survives_restart() {
        let (dir, s) = store();
        let a = dir.path().join("a.mp4");
        let b = dir.path().join("b.mp4");
        fs::write(&a, b"x").unwrap();
        fs::write(&b, b"x").unwrap();
        s.add(
            HistoryKind::Download,
            NewEntry {
                title: Some("A".into()),
                platform: Some("tiktok".into()),
                ..new(&a)
            },
            100,
        )
        .unwrap();
        s.add(
            HistoryKind::Export,
            NewEntry {
                bytes: Some(42),
                ..new(&b)
            },
            200,
        )
        .unwrap();

        let reopened = HistoryStore::new(dir.path().join("history"));
        let list = reopened.list();
        assert_eq!(list.len(), 2);
        assert_eq!(
            (list[0].kind, list[0].created_at, list[0].bytes),
            (HistoryKind::Export, 200, Some(42))
        );
        assert_eq!(
            (list[1].kind, list[1].title.as_deref()),
            (HistoryKind::Download, Some("A"))
        );
        assert!(list.iter().all(|e| e.exists));
    }

    #[test]
    fn missing_files_are_flagged_not_dropped() {
        let (dir, s) = store();
        let a = dir.path().join("gone.mp4");
        fs::write(&a, b"x").unwrap();
        s.add(HistoryKind::Export, new(&a), 1).unwrap();
        fs::remove_file(&a).unwrap();
        let list = s.list();
        assert_eq!(list.len(), 1);
        assert!(!list[0].exists);
    }

    #[test]
    fn removing_an_entry_never_touches_the_video_but_removes_its_thumbnail() {
        let (dir, s) = store();
        let a = dir.path().join("a.mp4");
        fs::write(&a, b"x").unwrap();
        let e = s.add(HistoryKind::Export, new(&a), 1).unwrap();
        fs::create_dir_all(s.thumbs_dir()).unwrap();
        fs::write(s.thumb_path(&e.id), b"jpg").unwrap();
        s.mark_thumb(&e.id).unwrap();
        assert!(s.list()[0].thumb_path.is_some());

        s.remove(&e.id).unwrap();
        assert!(s.list().is_empty());
        assert!(a.is_file(), "the video itself stays");
        assert!(!s.thumb_path(&e.id).exists());
    }

    #[test]
    fn a_missing_thumbnail_file_is_not_reported() {
        let (dir, s) = store();
        let a = dir.path().join("a.mp4");
        fs::write(&a, b"x").unwrap();
        let e = s.add(HistoryKind::Download, new(&a), 1).unwrap();
        s.mark_thumb(&e.id).unwrap(); // flagged, but the file was never written
        assert!(s.list()[0].thumb_path.is_none());
    }

    #[test]
    fn clear_empties_the_list() {
        let (dir, s) = store();
        let a = dir.path().join("a.mp4");
        fs::write(&a, b"x").unwrap();
        s.add(HistoryKind::Download, new(&a), 1).unwrap();
        s.clear().unwrap();
        assert!(s.list().is_empty());
        assert!(a.is_file());
    }

    #[test]
    fn the_list_is_capped() {
        let (_dir, s) = store();
        for i in 0..(MAX_ENTRIES + 5) {
            s.add(
                HistoryKind::Export,
                NewEntry {
                    path: format!("/x/{i}.mp4"),
                    ..Default::default()
                },
                i as i64,
            )
            .unwrap();
        }
        let list = s.list();
        assert_eq!(list.len(), MAX_ENTRIES);
        assert_eq!(list[0].created_at, (MAX_ENTRIES + 4) as i64, "newest kept");
        assert_eq!(list.last().unwrap().created_at, 5, "oldest dropped");
    }

    #[test]
    fn corrupt_file_gives_an_empty_history() {
        let (_dir, s) = store();
        fs::create_dir_all(&s.dir).unwrap();
        fs::write(s.index_path(), "{{").unwrap();
        assert!(s.list().is_empty());
    }

    #[test]
    fn thumbnail_args_seek_before_the_input() {
        let a = build_thumbnail_args("/in/a.mp4", "/t/x.jpg", 0.3);
        let ss = a.iter().position(|x| x == "-ss").unwrap();
        let i = a.iter().position(|x| x == "-i").unwrap();
        assert!(ss < i);
        assert_eq!(a.last().unwrap(), "/t/x.jpg");
    }
}
