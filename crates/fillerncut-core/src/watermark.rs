//! The persistent watermark library.
//!
//! Images are copied into `<dir>/files/` and described by `<dir>/library.json`,
//! so they survive restarts (and the user deleting/moving the original file).
//! Each entry remembers its own placement, size and opacity, and where its
//! visible (non-transparent) content sits, so transparent margins can be moved
//! out of the frame. Text watermarks are stored the same way: the UI renders the
//! text to a PNG and keeps the style so it can be edited later.

use crate::export::ContentBox;
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
/// Pixels with alpha above this count as visible content.
const ALPHA_THRESHOLD: u8 = 8;
const MAX_IMAGE_BYTES: usize = 40 * 1024 * 1024;
const MAX_TEXT_CHARS: usize = 300;

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

/// How a text watermark looks. The text itself is rendered to PNG by the UI.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TextStyle {
    pub text: String,
    pub font_family: String,
    pub bold: bool,
    pub italic: bool,
    /// `#rrggbb`
    pub color: String,
    pub outline: bool,
    pub outline_color: String,
    pub shadow: bool,
    /// `left`, `center` or `right`
    pub align: String,
}

impl TextStyle {
    /// Trim, cap lengths and replace unusable values so a bad style can't break rendering.
    pub fn sanitized(mut self) -> TextStyle {
        self.text = self.text.chars().take(MAX_TEXT_CHARS).collect();
        self.font_family = self
            .font_family
            .chars()
            .filter(|c| !c.is_control() && *c != '"' && *c != '\\')
            .take(80)
            .collect();
        if self.font_family.trim().is_empty() {
            self.font_family = "Helvetica Neue".into();
        }
        let hex = |s: &str, fallback: &str| {
            let ok = s.len() == 7 && s.starts_with('#') && s[1..].chars().all(|c| c.is_ascii_hexdigit());
            if ok {
                s.to_string()
            } else {
                fallback.to_string()
            }
        };
        self.color = hex(&self.color, "#ffffff");
        self.outline_color = hex(&self.outline_color, "#000000");
        if !matches!(self.align.as_str(), "left" | "center" | "right") {
            self.align = "center".into();
        }
        self
    }
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
    /// Where the visible content sits inside the image.
    #[serde(default)]
    pub content: ContentBox,
    /// Set for text watermarks.
    #[serde(default)]
    pub text: Option<TextStyle>,
    /// False for entries saved by older versions, whose `content` box was never
    /// measured. They're measured once, the next time the library is read.
    #[serde(default)]
    pub analyzed: bool,
    /// Remembered placement: top-left corner of the image as a fraction of the frame.
    pub nx: f64,
    pub ny: f64,
    /// Remembered image width as a fraction of the frame width.
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

/// Decoded size and visible-content box of an image.
struct Analysis {
    aspect: f64,
    content: ContentBox,
}

fn analyse(img: &image::DynamicImage) -> Result<Analysis, LibraryError> {
    use image::GenericImageView;
    let (w, h) = img.dimensions();
    if w == 0 || h == 0 {
        return Err(LibraryError::Unreadable("image has no size".into()));
    }
    let aspect = h as f64 / w as f64;
    if !img.color().has_alpha() {
        return Ok(Analysis {
            aspect,
            content: ContentBox::default(),
        });
    }
    let rgba = img.to_rgba8();
    let (mut min_x, mut min_y, mut max_x, mut max_y) = (w, h, 0u32, 0u32);
    let mut any = false;
    for (x, y, p) in rgba.enumerate_pixels() {
        if p.0[3] > ALPHA_THRESHOLD {
            any = true;
            min_x = min_x.min(x);
            min_y = min_y.min(y);
            max_x = max_x.max(x);
            max_y = max_y.max(y);
        }
    }
    let content = if any {
        ContentBox {
            l: min_x as f64 / w as f64,
            t: min_y as f64 / h as f64,
            r: (max_x + 1) as f64 / w as f64,
            b: (max_y + 1) as f64 / h as f64,
        }
        .sanitized()
    } else {
        ContentBox::default() // fully transparent: treat the whole image as content
    };
    Ok(Analysis { aspect, content })
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

    /// `load`, plus a one-time measurement of the visible area of entries that
    /// were saved before that was tracked.
    fn load_migrated(&self) -> LibraryFile {
        let mut file = self.load();
        let mut changed = false;
        for e in file.items.iter_mut().filter(|e| !e.analyzed) {
            let path = self.files_dir().join(&e.file_name);
            if let Some(a) = fs::read(&path)
                .ok()
                .and_then(|b| image::load_from_memory(&b).ok())
                .and_then(|img| analyse(&img).ok())
            {
                e.content = a.content;
                e.aspect = a.aspect;
            }
            e.analyzed = true;
            changed = true;
        }
        if changed {
            let _ = self.save(&file);
        }
        file
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
        self.load_migrated()
            .items
            .into_iter()
            .filter(|e| self.files_dir().join(&e.file_name).is_file())
            .map(|e| self.with_path(e))
            .collect()
    }

    pub fn get(&self, id: &str) -> Option<WatermarkEntry> {
        self.list().into_iter().find(|e| e.id == id)
    }

    fn insert(
        &self,
        name: String,
        ext: &str,
        bytes: &[u8],
        analysis: Analysis,
        text: Option<TextStyle>,
        scale: f64,
    ) -> Result<WatermarkEntry, LibraryError> {
        let id = uuid::Uuid::new_v4().simple().to_string();
        let file_name = format!("{id}.{ext}");
        fs::create_dir_all(self.files_dir())?;
        fs::write(self.files_dir().join(&file_name), bytes)?;

        // Rest near the bottom-right corner. The frame's aspect ratio isn't
        // known here, so the UI snaps this to a real corner when first used.
        let entry = WatermarkEntry {
            id,
            name,
            file_name,
            aspect: analysis.aspect,
            content: analysis.content,
            text,
            analyzed: true,
            nx: 1.0 - scale - DEFAULT_MARGIN,
            ny: 0.85,
            scale,
            opacity: 1.0,
            path: String::new(),
        };
        let mut file = self.load();
        file.items.push(entry.clone());
        self.save(&file)?;
        Ok(self.with_path(entry))
    }

    /// Copy an image into the library and return the new entry.
    pub fn add_from_path(&self, src: &Path) -> Result<WatermarkEntry, LibraryError> {
        let ext = src
            .extension()
            .and_then(|e| e.to_str())
            .map(|e| e.to_ascii_lowercase())
            .filter(|e| ALLOWED_EXTENSIONS.contains(&e.as_str()))
            .ok_or(LibraryError::UnsupportedType)?;
        let bytes = fs::read(src).map_err(|e| LibraryError::Unreadable(e.to_string()))?;
        if bytes.len() > MAX_IMAGE_BYTES {
            return Err(LibraryError::Unreadable("the image is larger than 40 MB".into()));
        }
        let img = image::load_from_memory(&bytes).map_err(|e| LibraryError::Unreadable(e.to_string()))?;
        let analysis = analyse(&img)?;
        let name = src
            .file_stem()
            .and_then(|s| s.to_str())
            .unwrap_or("Watermark")
            .to_string();
        self.insert(name, &ext, &bytes, analysis, None, DEFAULT_SCALE)
    }

    /// Add a text watermark whose PNG the UI has already rendered.
    pub fn add_text(&self, png: &[u8], style: TextStyle) -> Result<WatermarkEntry, LibraryError> {
        let style = style.sanitized();
        let img = decode_png(png)?;
        let analysis = analyse(&img)?;
        let name = text_name(&style.text);
        self.insert(name, "png", png, analysis, Some(style), 0.4)
    }

    /// Replace a text watermark's image and style (after the user edited the text),
    /// keeping its placement.
    pub fn replace_text(
        &self,
        id: &str,
        png: &[u8],
        style: TextStyle,
    ) -> Result<WatermarkEntry, LibraryError> {
        let style = style.sanitized();
        let img = decode_png(png)?;
        let analysis = analyse(&img)?;
        let mut file = self.load();
        let entry = file
            .items
            .iter_mut()
            .find(|e| e.id == id)
            .ok_or(LibraryError::NotFound)?;
        fs::write(self.files_dir().join(&entry.file_name), png)?;
        entry.aspect = analysis.aspect;
        entry.content = analysis.content;
        entry.name = text_name(&style.text);
        entry.text = Some(style);
        let updated = entry.clone();
        self.save(&file)?;
        Ok(self.with_path(updated))
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
        // Placement can legitimately be negative / past 1 (transparent margin outside the frame).
        if let Some(v) = patch.nx.filter(|v| finite(*v)) {
            entry.nx = v.clamp(-20.0, 20.0);
        }
        if let Some(v) = patch.ny.filter(|v| finite(*v)) {
            entry.ny = v.clamp(-20.0, 20.0);
        }
        if let Some(v) = patch.scale.filter(|v| finite(*v)) {
            entry.scale = v.clamp(0.01, crate::export::MAX_WATERMARK_SCALE);
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

fn decode_png(bytes: &[u8]) -> Result<image::DynamicImage, LibraryError> {
    if bytes.len() > MAX_IMAGE_BYTES {
        return Err(LibraryError::Unreadable("the image is larger than 40 MB".into()));
    }
    if !bytes.starts_with(&[0x89, b'P', b'N', b'G']) {
        return Err(LibraryError::Unreadable("expected a PNG".into()));
    }
    image::load_from_memory(bytes).map_err(|e| LibraryError::Unreadable(e.to_string()))
}

fn text_name(text: &str) -> String {
    let first = text
        .lines()
        .map(str::trim)
        .find(|l| !l.is_empty())
        .unwrap_or("Text");
    let name: String = first.chars().take(28).collect();
    if first.chars().count() > 28 {
        format!("{name}…")
    } else {
        name
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use image::{Rgba, RgbaImage};

    fn write_png(path: &Path, w: u32, h: u32) {
        let img = RgbaImage::from_pixel(w, h, Rgba([255, 0, 0, 255]));
        img.save(path).unwrap();
    }

    /// `w`×`h` transparent image with an opaque rectangle at (x0,y0)-(x1,y1) exclusive.
    fn padded_png(w: u32, h: u32, x0: u32, y0: u32, x1: u32, y1: u32) -> Vec<u8> {
        let mut img = RgbaImage::from_pixel(w, h, Rgba([0, 0, 0, 0]));
        for y in y0..y1 {
            for x in x0..x1 {
                img.put_pixel(x, y, Rgba([255, 255, 255, 255]));
            }
        }
        let mut out = Vec::new();
        image::DynamicImage::ImageRgba8(img)
            .write_to(&mut io::Cursor::new(&mut out), image::ImageFormat::Png)
            .unwrap();
        out
    }

    fn style(text: &str) -> TextStyle {
        TextStyle {
            text: text.into(),
            font_family: "Helvetica Neue".into(),
            bold: true,
            italic: false,
            color: "#ffffff".into(),
            outline: true,
            outline_color: "#000000".into(),
            shadow: false,
            align: "center".into(),
        }
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
        assert_eq!(
            added.content,
            ContentBox::default(),
            "an opaque image is all content"
        );
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
    fn transparent_margins_are_detected() {
        let (dir, store) = store();
        let src = dir.path().join("padded.png");
        fs::write(&src, padded_png(200, 100, 40, 25, 180, 75)).unwrap();
        let e = store.add_from_path(&src).unwrap();
        assert_eq!(
            e.content,
            ContentBox {
                l: 0.2,
                t: 0.25,
                r: 0.9,
                b: 0.75
            }
        );
        // Persisted with the entry.
        assert_eq!(store.get(&e.id).unwrap().content, e.content);
    }

    #[test]
    fn fully_transparent_images_count_as_all_content() {
        let (dir, store) = store();
        let src = dir.path().join("empty.png");
        fs::write(&src, padded_png(20, 20, 0, 0, 0, 0)).unwrap();
        assert_eq!(store.add_from_path(&src).unwrap().content, ContentBox::default());
    }

    #[test]
    fn placement_is_remembered_per_watermark_and_may_hang_outside() {
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
                    nx: Some(-0.15),
                    ny: Some(0.2),
                    scale: Some(0.4),
                    opacity: Some(0.5),
                    ..Default::default()
                },
            )
            .unwrap();

        let reopened = LibraryStore::new(dir.path().join("watermarks"));
        let a2 = reopened.get(&ea.id).unwrap();
        assert_eq!((a2.nx, a2.ny, a2.scale, a2.opacity), (-0.15, 0.2, 0.4, 0.5));
        assert_eq!(
            reopened.get(&eb.id).unwrap().scale,
            DEFAULT_SCALE,
            "other entries are untouched"
        );
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
                    nx: Some(700.0),
                    scale: Some(f64::NAN),
                    opacity: Some(-1.0),
                    name: Some("   ".into()),
                    ..Default::default()
                },
            )
            .unwrap();
        assert_eq!(u.nx, 20.0);
        assert_eq!(u.scale, DEFAULT_SCALE);
        assert_eq!(u.opacity, 0.0);
        assert_eq!(u.name, "a");
    }

    #[test]
    fn text_watermarks_store_their_style_and_can_be_edited() {
        let (_dir, store) = store();
        let png = padded_png(300, 100, 10, 10, 290, 90);
        let e = store.add_text(&png, style("Hello\nWorld")).unwrap();
        assert_eq!(e.name, "Hello");
        assert_eq!(e.text.as_ref().unwrap().text, "Hello\nWorld");
        assert_eq!(e.scale, 0.4);
        assert!(e.content.l > 0.0 && e.content.r < 1.0);

        store
            .update(
                &e.id,
                &WatermarkPatch {
                    nx: Some(0.1),
                    ny: Some(0.2),
                    scale: Some(0.3),
                    ..Default::default()
                },
            )
            .unwrap();
        let png2 = padded_png(400, 200, 0, 0, 400, 200);
        let edited = store.replace_text(&e.id, &png2, style("Other")).unwrap();
        assert_eq!(edited.name, "Other");
        assert_eq!(edited.aspect, 0.5);
        assert_eq!(
            (edited.nx, edited.ny, edited.scale),
            (0.1, 0.2, 0.3),
            "placement kept"
        );
        assert_eq!(store.list().len(), 1);
        assert!(matches!(
            store.replace_text("nope", &png2, style("x")),
            Err(LibraryError::NotFound)
        ));
    }

    #[test]
    fn text_style_is_sanitised() {
        let s = TextStyle {
            text: "x".repeat(1000),
            font_family: "Evil\"Font\\\n".into(),
            color: "red".into(),
            outline_color: "#12345".into(),
            align: "sideways".into(),
            ..style("x")
        }
        .sanitized();
        assert_eq!(s.text.chars().count(), MAX_TEXT_CHARS);
        assert_eq!(s.font_family, "EvilFont");
        assert_eq!(
            (s.color.as_str(), s.outline_color.as_str(), s.align.as_str()),
            ("#ffffff", "#000000", "center")
        );
        let blank = TextStyle {
            font_family: "  ".into(),
            ..style("x")
        }
        .sanitized();
        assert_eq!(blank.font_family, "Helvetica Neue");
    }

    #[test]
    fn text_images_must_be_real_pngs() {
        let (_dir, store) = store();
        assert!(matches!(
            store.add_text(b"not a png", style("x")),
            Err(LibraryError::Unreadable(_))
        ));
        assert!(store.list().is_empty());
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

    #[test]
    fn old_library_files_without_content_or_text_still_load() {
        let (_dir, store) = store();
        fs::create_dir_all(store.files_dir()).unwrap();
        fs::write(store.files_dir().join("old.png"), padded_png(4, 4, 0, 0, 4, 4)).unwrap();
        fs::write(
            store.index_path(),
            r#"{"items":[{"id":"old","name":"Old","fileName":"old.png","aspect":1.0,"nx":0.5,"ny":0.5,"scale":0.2,"opacity":1.0}]}"#,
        )
        .unwrap();
        let list = store.list();
        assert_eq!(list.len(), 1);
        assert_eq!(list[0].content, ContentBox::default());
        assert!(list[0].text.is_none());
    }

    #[test]
    fn old_entries_get_their_visible_area_measured_once() {
        let (_dir, store) = store();
        fs::create_dir_all(store.files_dir()).unwrap();
        // Saved by an older version: transparent margin, but no content box recorded.
        fs::write(
            store.files_dir().join("old.png"),
            padded_png(200, 100, 40, 25, 180, 75),
        )
        .unwrap();
        fs::write(
            store.index_path(),
            r#"{"items":[{"id":"old","name":"Old","fileName":"old.png","aspect":0.5,"nx":0.5,"ny":0.5,"scale":0.2,"opacity":1.0}]}"#,
        )
        .unwrap();
        assert_eq!(
            store.list()[0].content,
            ContentBox {
                l: 0.2,
                t: 0.25,
                r: 0.9,
                b: 0.75
            }
        );
        // Persisted, so the image isn't decoded again next time.
        assert!(fs::read_to_string(store.index_path())
            .unwrap()
            .contains("\"analyzed\": true"));
    }
}
