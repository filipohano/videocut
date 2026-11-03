//! Saved looks for text boxes ("presets"): colour, background, weight, alignment and size.
//! Kept in the app data folder, like the watermark library.

use crate::settings::write_json_atomic;
use serde::{Deserialize, Serialize};
use std::io;
use std::path::PathBuf;
use std::sync::Mutex;
use thiserror::Error;
use uuid::Uuid;

pub const MAX_PRESETS: usize = 50;
const MAX_NAME: usize = 40;
const MIN_SIZE: f64 = 2.0;
const MAX_SIZE: f64 = 20.0;

#[derive(Debug, Error, PartialEq, Eq)]
pub enum PresetError {
    #[error("Give the style a name")]
    NoName,
    #[error("That style has a colour FillernCut can't read")]
    BadColor,
    #[error("That style has an alignment FillernCut doesn't know")]
    BadAlign,
    #[error("You can keep at most {MAX_PRESETS} text styles. Delete one first")]
    TooMany,
    #[error("Couldn't save the text styles: {0}")]
    Io(String),
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TextPresetStyle {
    /// `#rrggbb`
    pub color: String,
    /// `#rrggbb`, or none for text without a background block
    pub background: Option<String>,
    pub bold: bool,
    /// `left`, `center` or `right`
    pub align: String,
    /// Font size as a % of the frame width
    pub size: f64,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TextPreset {
    pub id: String,
    pub name: String,
    pub style: TextPresetStyle,
}

/// `#abc` / `#AABBCC` / `aabbcc` → `#aabbcc`
pub fn normalize_hex(input: &str) -> Option<String> {
    let t = input.trim().trim_start_matches('#');
    if !t.bytes().all(|b| b.is_ascii_hexdigit()) {
        return None;
    }
    match t.len() {
        3 => Some(format!(
            "#{}",
            t.chars()
                .flat_map(|c| [c, c])
                .collect::<String>()
                .to_ascii_lowercase()
        )),
        6 => Some(format!("#{}", t.to_ascii_lowercase())),
        _ => None,
    }
}

impl TextPresetStyle {
    /// The style with its colours tidied and its size in range, or why it can't be used.
    pub fn sanitized(self) -> Result<TextPresetStyle, PresetError> {
        let color = normalize_hex(&self.color).ok_or(PresetError::BadColor)?;
        let background = match self.background {
            Some(b) => Some(normalize_hex(&b).ok_or(PresetError::BadColor)?),
            None => None,
        };
        if !matches!(self.align.as_str(), "left" | "center" | "right") {
            return Err(PresetError::BadAlign);
        }
        let size = if self.size.is_finite() {
            self.size.clamp(MIN_SIZE, MAX_SIZE)
        } else {
            7.0
        };
        Ok(TextPresetStyle {
            color,
            background,
            bold: self.bold,
            align: self.align,
            size,
        })
    }
}

pub struct TextPresetStore {
    path: PathBuf,
    presets: Mutex<Vec<TextPreset>>,
}

impl TextPresetStore {
    /// A missing or damaged file starts an empty list rather than stopping the app.
    pub fn new(path: PathBuf) -> TextPresetStore {
        let presets: Vec<TextPreset> = std::fs::read_to_string(&path)
            .ok()
            .and_then(|t| serde_json::from_str(&t).ok())
            .unwrap_or_default();
        // Whatever a hand-edited file holds, only usable presets are kept.
        let presets = presets
            .into_iter()
            .filter_map(|p| {
                let style = p.style.sanitized().ok()?;
                (!p.id.is_empty() && !p.name.trim().is_empty()).then_some(TextPreset { style, ..p })
            })
            .take(MAX_PRESETS)
            .collect();
        TextPresetStore {
            path,
            presets: Mutex::new(presets),
        }
    }

    pub fn list(&self) -> Vec<TextPreset> {
        self.presets.lock().unwrap().clone()
    }

    /// Save a style under a name. A name that is already used (capitals don't matter) is
    /// replaced, so saving again is how a style is changed.
    pub fn save(&self, name: &str, style: TextPresetStyle) -> Result<TextPreset, PresetError> {
        let name: String = name
            .split_whitespace()
            .collect::<Vec<_>>()
            .join(" ")
            .chars()
            .take(MAX_NAME)
            .collect();
        if name.is_empty() {
            return Err(PresetError::NoName);
        }
        let style = style.sanitized()?;
        let mut presets = self.presets.lock().unwrap();
        let saved = match presets
            .iter_mut()
            .find(|p| p.name.to_lowercase() == name.to_lowercase())
        {
            Some(existing) => {
                existing.style = style;
                existing.name = name;
                existing.clone()
            }
            None => {
                if presets.len() >= MAX_PRESETS {
                    return Err(PresetError::TooMany);
                }
                let preset = TextPreset {
                    id: Uuid::new_v4().to_string(),
                    name,
                    style,
                };
                presets.push(preset.clone());
                preset
            }
        };
        write_json_atomic(&self.path, &*presets).map_err(|e: io::Error| PresetError::Io(e.to_string()))?;
        Ok(saved)
    }

    pub fn remove(&self, id: &str) -> Result<(), PresetError> {
        let mut presets = self.presets.lock().unwrap();
        presets.retain(|p| p.id != id);
        write_json_atomic(&self.path, &*presets).map_err(|e: io::Error| PresetError::Io(e.to_string()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn style() -> TextPresetStyle {
        TextPresetStyle {
            color: "#FFF".into(),
            background: Some("#000000".into()),
            bold: true,
            align: "center".into(),
            size: 7.0,
        }
    }

    fn store() -> (tempfile::TempDir, TextPresetStore) {
        let dir = tempfile::tempdir().unwrap();
        let store = TextPresetStore::new(dir.path().join("data").join("text-presets.json"));
        (dir, store)
    }

    #[test]
    fn hex_colours_are_tidied() {
        assert_eq!(normalize_hex("#FFF").as_deref(), Some("#ffffff"));
        assert_eq!(normalize_hex("0A84FF").as_deref(), Some("#0a84ff"));
        for bad in [
            "",
            "#12",
            "#12345",
            "#ggg",
            "red",
            "#1234567",
            "rgb(1,2,3)",
            "#ff 00 ff",
        ] {
            assert_eq!(normalize_hex(bad), None, "{bad}");
        }
    }

    #[test]
    fn a_saved_style_comes_back_tidied_and_survives_a_restart() {
        let (dir, store) = store();
        assert!(store.list().is_empty());
        let saved = store.save("  White on   black ", style()).unwrap();
        assert_eq!(saved.name, "White on black");
        assert_eq!(saved.style.color, "#ffffff");
        assert!(!saved.id.is_empty());

        let reopened = TextPresetStore::new(dir.path().join("data").join("text-presets.json"));
        assert_eq!(reopened.list(), vec![saved]);
    }

    #[test]
    fn saving_under_an_existing_name_replaces_that_style() {
        let (_dir, store) = store();
        let first = store.save("Caption", style()).unwrap();
        let second = store
            .save(
                "CAPTION",
                TextPresetStyle {
                    background: None,
                    size: 12.0,
                    ..style()
                },
            )
            .unwrap();
        assert_eq!(first.id, second.id, "same preset, new look");
        assert_eq!(store.list().len(), 1);
        assert_eq!(store.list()[0].style.background, None);
        assert_eq!(store.list()[0].name, "CAPTION");
    }

    #[test]
    fn bad_styles_are_refused() {
        let (_dir, store) = store();
        assert_eq!(store.save("", style()), Err(PresetError::NoName));
        assert_eq!(store.save("   ", style()), Err(PresetError::NoName));
        assert_eq!(
            store.save(
                "x",
                TextPresetStyle {
                    color: "red".into(),
                    ..style()
                }
            ),
            Err(PresetError::BadColor)
        );
        assert_eq!(
            store.save(
                "x",
                TextPresetStyle {
                    background: Some("nope".into()),
                    ..style()
                }
            ),
            Err(PresetError::BadColor)
        );
        assert_eq!(
            store.save(
                "x",
                TextPresetStyle {
                    align: "justify".into(),
                    ..style()
                }
            ),
            Err(PresetError::BadAlign)
        );
        assert!(store.list().is_empty());
        // Size is kept in range rather than refused.
        let big = store
            .save(
                "big",
                TextPresetStyle {
                    size: 900.0,
                    ..style()
                },
            )
            .unwrap();
        assert_eq!(big.style.size, 20.0);
        let nan = store
            .save(
                "nan",
                TextPresetStyle {
                    size: f64::NAN,
                    ..style()
                },
            )
            .unwrap();
        assert_eq!(nan.style.size, 7.0);
    }

    #[test]
    fn long_names_are_cut_and_the_list_is_capped() {
        let (_dir, store) = store();
        let saved = store.save(&"n".repeat(200), style()).unwrap();
        assert_eq!(saved.name.chars().count(), 40);
        for i in 1..MAX_PRESETS {
            store.save(&format!("style {i}"), style()).unwrap();
        }
        assert_eq!(store.list().len(), MAX_PRESETS);
        assert_eq!(store.save("one too many", style()), Err(PresetError::TooMany));
        // Changing an existing one still works when the list is full.
        assert!(store
            .save(
                "style 3",
                TextPresetStyle {
                    bold: false,
                    ..style()
                }
            )
            .is_ok());
    }

    #[test]
    fn removing_forgets_a_style_for_good() {
        let (dir, store) = store();
        let a = store.save("a", style()).unwrap();
        let b = store.save("b", style()).unwrap();
        store.remove(&a.id).unwrap();
        store.remove("no-such-id").unwrap();
        assert_eq!(store.list(), vec![b.clone()]);
        let reopened = TextPresetStore::new(dir.path().join("data").join("text-presets.json"));
        assert_eq!(reopened.list(), vec![b]);
    }

    #[test]
    fn a_damaged_or_hand_edited_file_keeps_only_what_is_usable() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("text-presets.json");
        std::fs::write(&path, "{ not json").unwrap();
        assert!(TextPresetStore::new(path.clone()).list().is_empty());
        std::fs::write(
            &path,
            r##"[
              {"id":"1","name":"ok","style":{"color":"#FFF","background":null,"bold":false,"align":"left","size":99}},
              {"id":"2","name":"bad colour","style":{"color":"red","background":null,"bold":false,"align":"left","size":5}},
              {"id":"3","name":"  ","style":{"color":"#fff","background":null,"bold":false,"align":"left","size":5}},
              {"id":"","name":"no id","style":{"color":"#fff","background":null,"bold":false,"align":"left","size":5}}
            ]"##,
        )
        .unwrap();
        let list = TextPresetStore::new(path).list();
        assert_eq!(list.len(), 1);
        assert_eq!(list[0].style.color, "#ffffff");
        assert_eq!(list[0].style.size, 20.0);
    }
}
