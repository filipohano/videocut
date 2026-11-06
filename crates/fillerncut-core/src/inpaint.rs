//! "Remove text": filling in a part of a still picture with an AI model (MI-GAN), as
//! "content-aware fill" does. Everything around the model is here, so it can be tested
//! without it: which pixels to fill, the square the model looks at, and how the result is
//! blended back in. The model itself is run by the app and handed in as a function.
//!
//! The result is a *patch*: a picture (with transparency) of just the filled area and where it
//! goes in the full frame. The exporter lays it over the picture before cropping, like a
//! watermark, so the original file is never touched.

use image::imageops::{self, FilterType};
use image::{GrayImage, Luma, Rgb, RgbImage, Rgba, RgbaImage};
use serde::{Deserialize, Serialize};
use thiserror::Error;

/// The model works on squares of this many pixels.
pub const MODEL_SIZE: u32 = 512;

#[derive(Debug, Error, PartialEq, Eq)]
pub enum InpaintError {
    #[error("Draw a box over the text you want to remove")]
    NoBox,
    #[error("That box is outside the picture")]
    Outside,
    #[error("The AI model failed: {0}")]
    Model(String),
}

/// A rectangle in pixels of the full (uncropped) frame.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct PixelRect {
    pub x: u32,
    pub y: u32,
    pub w: u32,
    pub h: u32,
}

impl PixelRect {
    /// The part of the rectangle inside a `width` × `height` picture, if any.
    pub fn clamped(self, width: u32, height: u32) -> Option<PixelRect> {
        let x0 = self.x.min(width);
        let y0 = self.y.min(height);
        let x1 = self.x.saturating_add(self.w).min(width);
        let y1 = self.y.saturating_add(self.h).min(height);
        (x1 > x0 && y1 > y0).then_some(PixelRect {
            x: x0,
            y: y0,
            w: x1 - x0,
            h: y1 - y0,
        })
    }
}

/// How much bigger morphology and feathering are for a bigger picture (they were tuned at 640 px).
fn scale_of(width: u32, height: u32) -> f64 {
    (width.min(height) as f64 / 640.0).max(0.5)
}

fn radius(px: f64, width: u32, height: u32) -> u32 {
    (px * scale_of(width, height)).round().max(1.0) as u32
}

/// Is this pixel part of a caption? Captions on TikTok and Instagram are almost always white
/// letters with a black outline or shadow (or black on white), so: very bright and grey, or
/// very dark.
fn caption_pixel(p: &Rgb<u8>) -> bool {
    let max = *p.0.iter().max().unwrap() as u32;
    let min = *p.0.iter().min().unwrap() as u32;
    let white = max > 225 && (max - min) * 255 < 40 * max;
    let black = max < 40;
    white || black
}

/// The pixels to fill (255) inside the boxes the user drew. With `letters` only the caption's
/// own pixels are taken, grown a little so their soft edges and outline go too; otherwise the
/// whole box. Returns the mask and whether it fell back to the whole box (no letters found).
pub fn build_mask(
    frame: &RgbImage,
    boxes: &[PixelRect],
    letters: bool,
) -> Result<(GrayImage, bool), InpaintError> {
    let (w, h) = frame.dimensions();
    if boxes.is_empty() {
        return Err(InpaintError::NoBox);
    }
    let boxes: Vec<PixelRect> = boxes.iter().filter_map(|b| b.clamped(w, h)).collect();
    if boxes.is_empty() {
        return Err(InpaintError::Outside);
    }
    let mut whole = GrayImage::new(w, h);
    for b in &boxes {
        for y in b.y..b.y + b.h {
            for x in b.x..b.x + b.w {
                whole.put_pixel(x, y, Luma([255]));
            }
        }
    }
    if !letters {
        return Ok((whole, false));
    }
    let mut mask = GrayImage::new(w, h);
    let mut found = 0u64;
    let mut area = 0u64;
    for b in &boxes {
        area += b.w as u64 * b.h as u64;
        for y in b.y..b.y + b.h {
            for x in b.x..b.x + b.w {
                if caption_pixel(frame.get_pixel(x, y)) {
                    mask.put_pixel(x, y, Luma([255]));
                    found += 1;
                }
            }
        }
    }
    // Almost nothing white or black in the box: it isn't a caption we can pick out.
    if found * 200 < area {
        return Ok((whole, true));
    }
    // Close the gaps inside letters, then take a margin for the anti-aliased edges.
    let close = radius(2.0, w, h);
    let mask = erode(&dilate(&mask, close), close);
    let mask = dilate(&mask, radius(3.0, w, h));
    Ok((mask, false))
}

/// Grow white areas by `r` pixels (a square brush).
pub fn dilate(mask: &GrayImage, r: u32) -> GrayImage {
    morph(mask, r, true)
}

/// Shrink white areas by `r` pixels.
pub fn erode(mask: &GrayImage, r: u32) -> GrayImage {
    morph(mask, r, false)
}

fn morph(mask: &GrayImage, r: u32, grow: bool) -> GrayImage {
    let pick = |a: u8, b: u8| if grow { a.max(b) } else { a.min(b) };
    let (w, h) = mask.dimensions();
    let r = r as i64;
    let pass = |src: &GrayImage, horizontal: bool| {
        let mut out = GrayImage::new(w, h);
        for y in 0..h as i64 {
            for x in 0..w as i64 {
                let mut v = if grow { 0 } else { 255 };
                for d in -r..=r {
                    let (sx, sy) = if horizontal { (x + d, y) } else { (x, y + d) };
                    // Outside the picture counts as "not filled" when growing and "filled"
                    // when shrinking, so edges don't eat into the mask.
                    let s = if sx < 0 || sy < 0 || sx >= w as i64 || sy >= h as i64 {
                        if grow {
                            0
                        } else {
                            255
                        }
                    } else {
                        src.get_pixel(sx as u32, sy as u32).0[0]
                    };
                    v = pick(v, s);
                }
                out.put_pixel(x as u32, y as u32, Luma([v]));
            }
        }
        out
    };
    let horizontal = pass(mask, true);
    pass(&horizontal, false)
}

/// Average over a (2r+1)² square: soft edges.
pub fn box_blur(mask: &GrayImage, r: u32) -> GrayImage {
    let (w, h) = mask.dimensions();
    let r = r as i64;
    let pass = |src: &GrayImage, horizontal: bool| {
        let mut out = GrayImage::new(w, h);
        for y in 0..h as i64 {
            for x in 0..w as i64 {
                let mut sum = 0u32;
                for d in -r..=r {
                    let (sx, sy) = if horizontal { (x + d, y) } else { (x, y + d) };
                    let sx = sx.clamp(0, w as i64 - 1) as u32;
                    let sy = sy.clamp(0, h as i64 - 1) as u32;
                    sum += src.get_pixel(sx, sy).0[0] as u32;
                }
                out.put_pixel(x as u32, y as u32, Luma([(sum / (2 * r as u32 + 1)) as u8]));
            }
        }
        out
    };
    let horizontal = pass(mask, true);
    pass(&horizontal, false)
}

/// The bounding box of the white pixels of a mask.
pub fn mask_bounds(mask: &GrayImage) -> Option<PixelRect> {
    let (mut x0, mut y0, mut x1, mut y1) = (u32::MAX, u32::MAX, 0, 0);
    for (x, y, p) in mask.enumerate_pixels() {
        if p.0[0] > 0 {
            x0 = x0.min(x);
            y0 = y0.min(y);
            x1 = x1.max(x);
            y1 = y1.max(y);
        }
    }
    (x0 != u32::MAX).then(|| PixelRect {
        x: x0,
        y: y0,
        w: x1 - x0 + 1,
        h: y1 - y0 + 1,
    })
}

/// The area the model is shown: a square around the hole, half again as big so there is
/// picture all round to continue, kept inside the frame. When the frame is too narrow for the
/// square it becomes a rectangle (it is stretched to a square for the model, and back).
pub fn context_rect(hole: PixelRect, width: u32, height: u32) -> PixelRect {
    let want = ((hole.w.max(hole.h) as f64) * 1.5).round() as u32;
    // Small holes still get a decent amount of surroundings.
    let want = want
        .max(MODEL_SIZE.min(width.max(height)) / 2)
        .max(hole.w)
        .max(hole.h);
    let side_w = want.min(width);
    let side_h = want.min(height);
    let place =
        |center: u32, side: u32, total: u32| -> u32 { center.saturating_sub(side / 2).min(total - side) };
    PixelRect {
        x: place(hole.x + hole.w / 2, side_w, width),
        y: place(hole.y + hole.h / 2, side_h, height),
        w: side_w,
        h: side_h,
    }
}

/// A filled area, ready to lay over the frame: `image` goes at (`x`, `y`) of the full frame.
#[derive(Debug, Clone)]
pub struct Patch {
    pub x: u32,
    pub y: u32,
    pub image: RgbaImage,
}

/// Lay a patch over a frame (to fill a second area on top of a first one).
pub fn apply_patch(frame: &mut RgbImage, patch: &RgbaImage, at_x: i64, at_y: i64) {
    let (w, h) = frame.dimensions();
    for (px, py, p) in patch.enumerate_pixels() {
        let (x, y) = (at_x + px as i64, at_y + py as i64);
        if x < 0 || y < 0 || x >= w as i64 || y >= h as i64 {
            continue;
        }
        let a = p.0[3] as u32;
        if a == 0 {
            continue;
        }
        let dst = frame.get_pixel_mut(x as u32, y as u32);
        for c in 0..3 {
            dst.0[c] = ((p.0[c] as u32 * a + dst.0[c] as u32 * (255 - a) + 127) / 255) as u8;
        }
    }
}

/// Fill the white part of `mask` in `frame`. `model` gets a `MODEL_SIZE` square picture and a
/// mask of the same size (255 = keep, 0 = fill, as MI-GAN wants) and returns the filled square.
pub fn fill(
    frame: &RgbImage,
    mask: &GrayImage,
    model: impl FnOnce(&RgbImage, &GrayImage) -> Result<RgbImage, String>,
) -> Result<Patch, InpaintError> {
    let (w, h) = frame.dimensions();
    let hole = mask_bounds(mask).ok_or(InpaintError::NoBox)?;
    let ctx = context_rect(hole, w, h);

    let crop = imageops::crop_imm(frame, ctx.x, ctx.y, ctx.w, ctx.h).to_image();
    let crop_mask = imageops::crop_imm(mask, ctx.x, ctx.y, ctx.w, ctx.h).to_image();
    let small = imageops::resize(&crop, MODEL_SIZE, MODEL_SIZE, FilterType::CatmullRom);
    // A model pixel is filled if any of the frame pixels it stands for is.
    let small_hole = imageops::resize(&crop_mask, MODEL_SIZE, MODEL_SIZE, FilterType::Triangle);
    let keep = GrayImage::from_fn(MODEL_SIZE, MODEL_SIZE, |x, y| {
        Luma([if small_hole.get_pixel(x, y).0[0] > 0 {
            0
        } else {
            255
        }])
    });
    let out = model(&small, &keep).map_err(InpaintError::Model)?;
    if out.dimensions() != (MODEL_SIZE, MODEL_SIZE) {
        return Err(InpaintError::Model(
            "it returned a picture of the wrong size".into(),
        ));
    }
    let filled = imageops::resize(&out, ctx.w, ctx.h, FilterType::CatmullRom);

    // Blend: fully the model's inside the hole, fading to the original over a few pixels
    // around it, so no edge shows.
    let feather = radius(2.0, w, h);
    let alpha = box_blur(&dilate(&crop_mask, feather), feather);
    let bounds = mask_bounds(&alpha).ok_or(InpaintError::NoBox)?;
    let image = RgbaImage::from_fn(bounds.w, bounds.h, |px, py| {
        let (cx, cy) = (bounds.x + px, bounds.y + py);
        let a = alpha.get_pixel(cx, cy).0[0];
        let c = filled.get_pixel(cx, cy).0;
        Rgba([c[0], c[1], c[2], a])
    });
    Ok(Patch {
        x: ctx.x + bounds.x,
        y: ctx.y + bounds.y,
        image,
    })
}

// ───────── which parts of a video stand still ─────────

/// A stretch of a video where the picture doesn't change (one photo of a slideshow).
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct StillSegment {
    pub start: f64,
    pub end: f64,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", tag = "kind")]
pub enum Stillness {
    /// One picture the whole time (a photo with music): a fill applies to all of it.
    Still,
    /// Several pictures one after another: a fill applies to the one it was made on.
    Slides { segments: Vec<StillSegment> },
    /// The picture moves: filling would have to be done frame by frame, which isn't offered.
    Moving,
}

/// ffmpeg's `freezedetect` filter: the arguments that print where the picture stands still.
/// The picture is made small first, which is much faster and ignores compression noise.
pub fn freezedetect_args(input: &str) -> Vec<String> {
    [
        "-hide_banner",
        "-nostdin",
        "-i",
        input,
        "-map",
        "0:v:0",
        "-vf",
        "scale=160:-2,freezedetect=n=0.003:d=0.4",
        "-an",
        "-f",
        "null",
        "-",
    ]
    .iter()
    .map(|s| s.to_string())
    .collect()
}

/// What `freezedetect` printed, as still stretches (in seconds).
pub fn parse_freezes(stderr: &str, duration: f64) -> Vec<StillSegment> {
    let value = |line: &str, key: &str| -> Option<f64> {
        let at = line.find(key)?;
        line[at + key.len()..]
            .trim_start_matches([':', ' '])
            .split_whitespace()
            .next()?
            .parse()
            .ok()
    };
    let mut out = Vec::new();
    let mut open: Option<f64> = None;
    for line in stderr.lines() {
        if let Some(start) = value(line, "lavfi.freezedetect.freeze_start") {
            open = Some(start.max(0.0));
        } else if let Some(end) = value(line, "lavfi.freezedetect.freeze_end") {
            if let Some(start) = open.take() {
                out.push(StillSegment { start, end });
            }
        }
    }
    // Still until the very end: ffmpeg never prints the end of that one.
    if let Some(start) = open {
        out.push(StillSegment { start, end: duration });
    }
    out.retain(|s| s.end > s.start);
    out
}

/// Whether filling is possible, from the still stretches of a video `duration` seconds long.
pub fn classify(segments: &[StillSegment], duration: f64) -> Stillness {
    if duration <= 0.0 || segments.is_empty() {
        return Stillness::Moving;
    }
    let still: f64 = segments.iter().map(|s| s.end - s.start).sum();
    // Fades between photos move, so not every moment has to be still.
    if still < duration * 0.8 {
        return Stillness::Moving;
    }
    if segments.len() == 1 {
        return Stillness::Still;
    }
    Stillness::Slides {
        segments: segments.to_vec(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn frame(w: u32, h: u32) -> RgbImage {
        // A soft gradient, nothing white or black.
        RgbImage::from_fn(w, h, |x, y| {
            Rgb([(60 + x % 120) as u8, (80 + y % 100) as u8, 120])
        })
    }

    fn draw_caption(img: &mut RgbImage, r: PixelRect) {
        // White bars with a black outline: letters, more or less.
        for y in r.y..r.y + r.h {
            for x in r.x..r.x + r.w {
                let edge = y == r.y || y + 1 == r.y + r.h || (x - r.x).is_multiple_of(12);
                img.put_pixel(
                    x,
                    y,
                    if edge {
                        Rgb([5, 5, 5])
                    } else {
                        Rgb([250, 250, 250])
                    },
                );
            }
        }
    }

    fn rect(x: u32, y: u32, w: u32, h: u32) -> PixelRect {
        PixelRect { x, y, w, h }
    }

    #[test]
    fn a_box_is_clamped_to_the_picture() {
        assert_eq!(rect(90, 10, 50, 20).clamped(100, 100), Some(rect(90, 10, 10, 20)));
        assert_eq!(rect(200, 10, 50, 20).clamped(100, 100), None);
        assert_eq!(rect(0, 0, 0, 20).clamped(100, 100), None);
    }

    #[test]
    fn only_the_letters_in_the_box_are_taken() {
        let mut img = frame(400, 300);
        draw_caption(&mut img, rect(120, 140, 160, 24));
        // A white spot outside the box is left alone.
        draw_caption(&mut img, rect(10, 10, 20, 10));
        let (mask, whole) = build_mask(&img, &[rect(100, 120, 200, 64)], true).unwrap();
        assert!(!whole);
        let b = mask_bounds(&mask).unwrap();
        // The letters plus a small margin, not the whole box.
        assert!(b.x >= 110 && b.x < 120, "{b:?}");
        assert!(b.y >= 130 && b.y < 140, "{b:?}");
        assert!(b.x + b.w <= 292 && b.y + b.h <= 176, "{b:?}");
        assert_eq!(mask.get_pixel(15, 15).0[0], 0, "outside the box");
        assert_eq!(mask.get_pixel(105, 125).0[0], 0, "background inside the box");
        assert_eq!(mask.get_pixel(200, 150).0[0], 255, "a letter");
    }

    #[test]
    fn a_box_without_letters_is_filled_whole_and_says_so() {
        let img = frame(400, 300);
        let (mask, whole) = build_mask(&img, &[rect(100, 100, 50, 40)], true).unwrap();
        assert!(whole);
        assert_eq!(mask_bounds(&mask), Some(rect(100, 100, 50, 40)));
        let (mask, whole) = build_mask(&img, &[rect(100, 100, 50, 40)], false).unwrap();
        assert!(!whole);
        assert_eq!(mask_bounds(&mask), Some(rect(100, 100, 50, 40)));
    }

    #[test]
    fn boxes_outside_or_missing_are_explained() {
        let img = frame(100, 100);
        assert_eq!(build_mask(&img, &[], true).unwrap_err(), InpaintError::NoBox);
        assert_eq!(
            build_mask(&img, &[rect(300, 300, 10, 10)], true).unwrap_err(),
            InpaintError::Outside
        );
    }

    #[test]
    fn grow_and_shrink_undo_each_other_on_a_solid_block() {
        let mut m = GrayImage::new(50, 50);
        for y in 20..30 {
            for x in 20..30 {
                m.put_pixel(x, y, Luma([255]));
            }
        }
        assert_eq!(mask_bounds(&dilate(&m, 3)), Some(rect(17, 17, 16, 16)));
        assert_eq!(mask_bounds(&erode(&dilate(&m, 3), 3)), Some(rect(20, 20, 10, 10)));
        // A block touching the edge isn't eaten from the edge.
        let mut edge = GrayImage::new(20, 20);
        for y in 0..20 {
            for x in 0..5 {
                edge.put_pixel(x, y, Luma([255]));
            }
        }
        assert_eq!(mask_bounds(&erode(&edge, 2)), Some(rect(0, 0, 3, 20)));
    }

    #[test]
    fn the_model_sees_a_square_with_room_around_the_hole_inside_the_frame() {
        let c = context_rect(rect(400, 900, 200, 60), 1080, 1920);
        assert_eq!((c.w, c.h), (300, 300));
        assert!(
            c.x <= 400 && c.x + c.w >= 600 && c.y <= 900 && c.y + c.h >= 960,
            "{c:?}"
        );
        // Near an edge it slides inside rather than leaving the frame.
        let c = context_rect(rect(0, 0, 100, 40), 1080, 1920);
        assert_eq!((c.x, c.y), (0, 0));
        // A caption across the whole width of a portrait picture: as wide as the frame.
        let c = context_rect(rect(20, 1500, 1040, 120), 1080, 1920);
        assert_eq!(c.w, 1080);
        assert!(c.h >= 1040 && c.y + c.h <= 1920, "{c:?}");
    }

    #[test]
    fn a_fill_replaces_the_hole_and_blends_into_its_surroundings() {
        let mut img = frame(400, 300);
        draw_caption(&mut img, rect(120, 140, 160, 24));
        let (mask, _) = build_mask(&img, &[rect(100, 120, 200, 64)], true).unwrap();
        let mut seen = None;
        let patch = fill(&img, &mask, |pic, keep| {
            assert_eq!(pic.dimensions(), (MODEL_SIZE, MODEL_SIZE));
            assert_eq!(keep.dimensions(), (MODEL_SIZE, MODEL_SIZE));
            let holes = keep.pixels().filter(|p| p.0[0] == 0).count();
            seen = Some(holes);
            // A "model" that paints the hole red.
            Ok(RgbImage::from_fn(MODEL_SIZE, MODEL_SIZE, |x, y| {
                if keep.get_pixel(x, y).0[0] == 0 {
                    Rgb([255, 0, 0])
                } else {
                    *pic.get_pixel(x, y)
                }
            }))
        })
        .unwrap();
        assert!(seen.unwrap() > 1000, "the model was told where the hole is");

        let mut out = img.clone();
        apply_patch(&mut out, &patch.image, patch.x as i64, patch.y as i64);
        // The middle of the caption is now the model's red.
        let p = out.get_pixel(200, 152).0;
        assert!(p[0] > 200 && p[1] < 40, "{p:?}");
        // Far from the caption nothing changed.
        assert_eq!(out.get_pixel(20, 280), img.get_pixel(20, 280));
        assert_eq!(out.get_pixel(390, 20), img.get_pixel(390, 20));
        // The patch only covers the caption and a feathered margin.
        assert!(patch.x >= 100 && patch.y >= 120, "{:?}", (patch.x, patch.y));
        assert!(patch.image.width() < 200 && patch.image.height() < 64);
        // Its corners fade out instead of having a hard edge.
        assert!(patch.image.get_pixel(0, 0).0[3] < 255);
    }

    #[test]
    fn a_model_that_fails_or_misbehaves_is_explained() {
        let img = frame(100, 100);
        let (mask, _) = build_mask(&img, &[rect(10, 10, 20, 20)], false).unwrap();
        let err = fill(&img, &mask, |_, _| Err("out of memory".into())).unwrap_err();
        assert_eq!(err, InpaintError::Model("out of memory".into()));
        let err = fill(&img, &mask, |_, _| Ok(RgbImage::new(10, 10))).unwrap_err();
        assert!(matches!(err, InpaintError::Model(_)));
    }

    #[test]
    fn freezedetect_output_becomes_still_stretches() {
        // A three-photo slideshow, three seconds each, as ffmpeg reports it.
        let stderr = "\
[freezedetect @ 0x1] lavfi.freezedetect.freeze_start: 0
[freezedetect @ 0x1] lavfi.freezedetect.freeze_duration: 3
[freezedetect @ 0x1] lavfi.freezedetect.freeze_end: 3
[freezedetect @ 0x1] lavfi.freezedetect.freeze_start: 3
[freezedetect @ 0x1] lavfi.freezedetect.freeze_duration: 3
[freezedetect @ 0x1] lavfi.freezedetect.freeze_end: 6
[freezedetect @ 0x1] lavfi.freezedetect.freeze_start: 6.03333
frame=  270 fps=0.0 q=-0.0 Lsize=N/A time=00:00:09.00";
        let segs = parse_freezes(stderr, 9.0);
        assert_eq!(
            segs,
            vec![
                StillSegment { start: 0.0, end: 3.0 },
                StillSegment { start: 3.0, end: 6.0 },
                StillSegment {
                    start: 6.03333,
                    end: 9.0
                },
            ]
        );
        assert_eq!(
            classify(&segs, 9.0),
            Stillness::Slides {
                segments: segs.clone()
            }
        );
    }

    #[test]
    fn a_picture_with_music_is_one_still_and_a_moving_video_is_refused() {
        let still = parse_freezes("lavfi.freezedetect.freeze_start: 0\n", 12.5);
        assert_eq!(
            still,
            vec![StillSegment {
                start: 0.0,
                end: 12.5
            }]
        );
        assert_eq!(classify(&still, 12.5), Stillness::Still);
        // A normal video: never still, or only for a moment.
        assert_eq!(classify(&parse_freezes("", 10.0), 10.0), Stillness::Moving);
        let blip = parse_freezes(
            "lavfi.freezedetect.freeze_start: 2\nlavfi.freezedetect.freeze_end: 3.1\n",
            10.0,
        );
        assert_eq!(classify(&blip, 10.0), Stillness::Moving);
        assert_eq!(classify(&[], 0.0), Stillness::Moving);
    }

    #[test]
    fn the_freezedetect_command_reads_only_the_picture() {
        let a = freezedetect_args("/in/a.mp4");
        assert!(a.contains(&"/in/a.mp4".to_string()));
        assert!(a.iter().any(|s| s.contains("freezedetect")));
        assert!(a.contains(&"-an".to_string()));
    }
}
