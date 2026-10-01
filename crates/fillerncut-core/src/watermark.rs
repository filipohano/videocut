//! The persistent watermark library.
//!
//! Images are copied into `<dir>/files/` and described by `<dir>/library.json`,
//! so they survive restarts (and the user deleting/moving the original file).
//! Each entry remembers its own placement, size and opacity.

use crate::settings::write_json_atomic;
use serde::{Deserialize, Serialize};
use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use thiserror::Error;

const ALLOWED_EXTENSIONS: &[&str] = &["png", "jpg", "jpeg", "webp", "gif"];
/// Default footprint of a freshly imported watermark.
const DEFAULT_SCALE: f64 = 0.2;
const DEFAULT_MARGIN: f64 = 0.03;

#[derive(Debug, Error)]
pub enum LibraryError {
    #[error("Unsupported image type. Use PNG, JPG, WebP or GIF")]
    UnsupportedType,
    #[error("Couldn't read that image: {0}")]
    Unreadable(String),
    #[error("Watermark not found")]
    NotFound,
    #[error("{0}")]
    Io(#[from] io::Error),
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct WatermarkEntry {
    pub id: String,
    pub name: String,
    /// File name inside `<dir>/files/`.
    pub file_name: String,
    /// Image height / width.
    pub aspect: f64,
    /// Remembered placement: top-left corner as a fraction of the frame.
    pub nx: f64,
    pub ny: f64,
    /// Remembered width as a fraction of the frame width.
    pub scale: f64,
    pub opacity: f64,
    /// Filled in on the way out so the frontend knows where the file lives.
    #[serde(default, skip_deserializing)]
    pub path: String,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct WatermarkPatch {
    pub name: Option<String>,
    pub nx: Option<f64>,
    pub ny: Option<f64>,
    pub scale: Option<f64>,
    pub opacity: Option<f64>,
}

#[derive(Debug, Default, Serialize, Deserialize)]
struct LibraryFile {
    items: Vec<WatermarkEntry>,
}

pub struct LibraryStore {
    dir: PathBuf,
}

impl LibraryStore {
    pub fn new(dir: impl Into<PathBuf>) -> LibraryStore {
        LibraryStore { dir: dir.into() }
    }

    fn index_path(&self) -> PathBuf {
        self.dir.join("library.json")
    }

    fn files_dir(&self) -> PathBuf {
        self.dir.join("files")
    }

    fn load(&self) -> LibraryFile {
        fs::read_to_string(self.index_path())
            .ok()
            .and_then(|t| serde_json::from_str(&t).ok())
            .unwrap_or_default()
    }

    fn save(&self, file: &LibraryFile) -> io::Result<()> {
        write_json_atomic(&self.index_path(), file)
    }

    fn with_path(&self, mut e: WatermarkEntry) -> WatermarkEntry {
        e.path = self.files_dir().join(&e.file_name).to_string_lossy().into_owned();
        e
    }

    /// All watermarks, oldest first. Entries whose image file has vanished are
    /// silently dropped.
    pub fn list(&self) -> Vec<WatermarkEntry> {
        self.load()
            .items
            .into_iter()
            .filter(|e| self.files_dir().join(&e.file_name).is_file())
            .map(|e| self.with_path(e))
            .collect()
    }

    pub fn get(&self, id: &str) -> Option<WatermarkEntry> {
        self.list().into_iter().find(|e| e.id == id)
    }

    /// Copy an image into the library and return the new entry.
    pub fn add_from_path(&self, src: &Path) -> Result<WatermarkEntry, LibraryError> {
        let ext = src
            .extension()
            .and_then(|e| e.to_str())
            .map(|e| e.to_ascii_lowercase())
            .filter(|e| ALLOWED_EXTENSIONS.contains(&e.as_str()))
            .ok_or(LibraryError::UnsupportedType)?;

        let size = imagesize::size(src).map_err(|e| LibraryError::Unreadable(e.to_string()))?;
        if size.width == 0 || size.height == 0 {
            return Err(LibraryError::Unreadable("image has no size".into()));
        }
        let aspect = size.height as f64 / size.width as f64;

        let id = uuid::Uuid::new_v4().simple().to_string();
        let file_name = format!("{id}.{ext}");
        fs::create_dir_all(self.files_dir())?;
        fs::copy(src, self.files_dir().join(&file_name))?;

        let name = src
            .file_stem()
            .and_then(|s| s.to_str())
            .unwrap_or("Watermark")
            .to_string();
        // Rest near the bottom-right corner. The frame's aspect ratio isn't
        // known here, so the UI clamps this into the actual frame when the
        // watermark is first put on a video.
        let entry = WatermarkEntry {
            id,
            name,
            file_name,
            aspect,
            nx: 1.0 - DEFAULT_SCALE - DEFAULT_MARGIN,
            ny: 0.85,
            scale: DEFAULT_SCALE,
            opacity: 1.0,
            path: String::new(),
        };

        let mut file = self.load();
        file.items.push(entry.clone());
        self.save(&file)?;
        Ok(self.with_path(entry))
    }

    pub fn update(&self, id: &str, patch: &WatermarkPatch) -> Result<WatermarkEntry, LibraryError> {
        let mut file = self.load();
        let entry = file
            .items
            .iter_mut()
            .find(|e| e.id == id)
            .ok_or(LibraryError::NotFound)?;
        if let Some(name) = &patch.name {
            let name = name.trim();
            if !name.is_empty() {
                entry.name = name.chars().take(80).collect();
            }
        }
        let finite = |v: f64| v.is_finite();
        if let Some(v) = patch.nx.filter(|v| finite(*v)) {
            entry.nx = v.clamp(0.0, 1.0);
        }
        if let Some(v) = patch.ny.filter(|v| finite(*v)) {
            entry.ny = v.clamp(0.0, 1.0);
        }
        if let Some(v) = patch.scale.filter(|v| finite(*v)) {
            entry.scale = v.clamp(0.01, 1.0);
        }
        if let Some(v) = patch.opacity.filter(|v| finite(*v)) {
            entry.opacity = v.clamp(0.0, 1.0);
        }
        let updated = entry.clone();
        self.save(&file)?;
        Ok(self.with_path(updated))
    }

    pub fn remove(&self, id: &str) -> Result<(), LibraryError> {
        let mut file = self.load();
        let pos = file
            .items
            .iter()
            .position(|e| e.id == id)
            .ok_or(LibraryError::NotFound)?;
        let removed = file.items.remove(pos);
        self.save(&file)?;
        // Best effort: the index is already updated.
        let _ = fs::remove_file(self.files_dir().join(removed.file_name));
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A valid 4x2 PNG header is enough for `imagesize`.
    fn write_png(path: &Path, w: u32, h: u32) {
        let mut b = vec![0x89, b'P', b'N', b'G', 0x0D, 0x0A, 0x1A, 0x0A, 0, 0, 0, 13];
        b.extend_from_slice(b"IHDR");
        b.extend_from_slice(&w.to_be_bytes());
        b.extend_from_slice(&h.to_be_bytes());
        b.extend_from_slice(&[8, 6, 0, 0, 0, 0, 0, 0, 0]);
        fs::write(path, b).unwrap();
    }

    fn store() -> (tempfile::TempDir, LibraryStore) {
        let dir = tempfile::tempdir().unwrap();
        let store = LibraryStore::new(dir.path().join("watermarks"));
        (dir, store)
    }

    #[test]
    fn imported_watermark_survives_restart_and_original_deletion() {
        let (dir, store) = store();
        let src = dir.path().join("My Logo.png");
        write_png(&src, 400, 100);

        let added = store.add_from_path(&src).unwrap();
        assert_eq!(added.name, "My Logo");
        assert_eq!(added.aspect, 0.25);
        assert!(Path::new(&added.path).is_file());

        fs::remove_file(&src).unwrap();
        // "Restart": a brand-new store over the same directory.
        let reopened = LibraryStore::new(dir.path().join("watermarks"));
        let list = reopened.list();
        assert_eq!(list.len(), 1);
        assert_eq!(list[0].id, added.id);
        assert!(Path::new(&list[0].path).is_file());
    }

    #[test]
    fn placement_is_remembered_per_watermark() {
        let (dir, store) = store();
        let a = dir.path().join("a.png");
        let b = dir.path().join("b.png");
        write_png(&a, 100, 100);
        write_png(&b, 200, 100);
        let ea = store.add_from_path(&a).unwrap();
        let eb = store.add_from_path(&b).unwrap();

        store
            .update(
                &ea.id,
                &WatermarkPatch {
                    nx: Some(0.1),
                    ny: Some(0.2),
                    scale: Some(0.4),
                    opacity: Some(0.5),
                    ..Default::default()
                },
            )
            .unwrap();

        let reopened = LibraryStore::new(dir.path().join("watermarks"));
        let a2 = reopened.get(&ea.id).unwrap();
        assert_eq!((a2.nx, a2.ny, a2.scale, a2.opacity), (0.1, 0.2, 0.4, 0.5));
        let b2 = reopened.get(&eb.id).unwrap();
        assert_eq!(b2.scale, DEFAULT_SCALE, "other entries are untouched");
    }

    #[test]
    fn update_clamps_and_ignores_nonsense() {
        let (dir, store) = store();
        let src = dir.path().join("a.png");
        write_png(&src, 10, 10);
        let e = store.add_from_path(&src).unwrap();
        let u = store
            .update(
                &e.id,
                &WatermarkPatch {
                    nx: Some(7.0),
                    scale: Some(f64::NAN),
                    opacity: Some(-1.0),
                    name: Some("   ".into()),
                    ..Default::default()
                },
            )
            .unwrap();
        assert_eq!(u.nx, 1.0);
        assert_eq!(u.scale, DEFAULT_SCALE);
        assert_eq!(u.opacity, 0.0);
        assert_eq!(u.name, "a");
    }

    #[test]
    fn remove_deletes_entry_and_file() {
        let (dir, store) = store();
        let src = dir.path().join("a.png");
        write_png(&src, 10, 10);
        let e = store.add_from_path(&src).unwrap();
        store.remove(&e.id).unwrap();
        assert!(store.list().is_empty());
        assert!(!Path::new(&e.path).exists());
        assert!(matches!(store.remove(&e.id), Err(LibraryError::NotFound)));
    }

    #[test]
    fn rejects_unsupported_and_unreadable_files() {
        let (dir, store) = store();
        let txt = dir.path().join("notes.txt");
        fs::write(&txt, "hi").unwrap();
        assert!(matches!(
            store.add_from_path(&txt),
            Err(LibraryError::UnsupportedType)
        ));
        let fake = dir.path().join("fake.png");
        fs::write(&fake, "not a png").unwrap();
        assert!(matches!(
            store.add_from_path(&fake),
            Err(LibraryError::Unreadable(_))
        ));
        assert!(store.list().is_empty());
    }

    #[test]
    fn entries_with_missing_files_are_hidden() {
        let (dir, store) = store();
        let src = dir.path().join("a.png");
        write_png(&src, 10, 10);
        let e = store.add_from_path(&src).unwrap();
        fs::remove_file(&e.path).unwrap();
        assert!(store.list().is_empty());
    }

    #[test]
    fn corrupt_index_does_not_crash() {
        let (_dir, store) = store();
        fs::create_dir_all(&store.dir).unwrap();
        fs::write(store.index_path(), "{{{").unwrap();
        assert!(store.list().is_empty());
    }
}
