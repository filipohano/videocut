//! Pure, UI-independent logic for FillernCut.
//!
//! Everything in here is deterministic and unit-testable without Tauri,
//! a network connection or an ffmpeg binary.

pub mod export;
pub mod history;
pub mod links;
pub mod media;
pub mod naming;
pub mod progress;
pub mod settings;
pub mod tiktok;
pub mod watermark;
pub mod ytdlp;

pub use export::{
    build_export_args, build_preview_args, build_slideshow_args, ContentBox, CropRect, Encoder, ExportError,
    ExportSpec, SlideshowSpec, WatermarkPlacement,
};
pub use history::{HistoryEntry, HistoryKind, HistoryStore, NewEntry};
pub use links::{parse_link, LinkError, ParsedLink, Platform};
pub use media::{parse_encoders, parse_ffprobe, EncoderSupport, MediaInfo};
pub use settings::{CookieBrowser, Settings, UpdateMode};
pub use watermark::{LibraryStore, TextStyle, WatermarkEntry, WatermarkPatch};
