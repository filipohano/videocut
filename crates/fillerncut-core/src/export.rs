//! Building ffmpeg command lines for export, preview proxies and TikTok
//! photo-post slideshows.
//!
//! All functions are pure: they take a spec and return the argument vector,
//! so the (fiddly) filter-graph logic is unit-testable without ffmpeg.

use serde::{Deserialize, Serialize};
use thiserror::Error;

#[derive(Debug, Error, PartialEq)]
pub enum ExportError {
    #[error("The crop area is outside the video")]
    CropOutside,
    #[error("The crop area is too small")]
    CropTooSmall,
    #[error("Trim end must be after trim start")]
    BadTrim,
    #[error("Add at least one image to the slideshow")]
    NoImages,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct CropRect {
    pub x: u32,
    pub y: u32,
    pub w: u32,
    pub h: u32,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct WatermarkPlacement {
    pub path: String,
    /// Top-left corner as a fraction of the *cropped* frame (0..1).
    pub nx: f64,
    pub ny: f64,
    /// Watermark width as a fraction of the cropped frame width (0..1].
    pub scale: f64,
    /// 0 = invisible, 1 = fully opaque.
    pub opacity: f64,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ExportSpec {
    pub input: String,
    pub output: String,
    /// Displayed (rotation-applied) source size and duration, from ffprobe.
    pub source_width: u32,
    pub source_height: u32,
    pub source_duration: f64,
    pub has_audio: bool,
    pub audio_codec: Option<String>,
    pub crop: Option<CropRect>,
    pub trim_start: Option<f64>,
    pub trim_end: Option<f64>,
    pub watermarks: Vec<WatermarkPlacement>,
    /// 1..=100, higher is better.
    pub quality: u8,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Encoder {
    /// Apple's hardware H.264 encoder (the default on every Apple-silicon Mac).
    VideoToolbox,
    /// Software fallback, used when VideoToolbox isn't available.
    Libx264,
}

impl Encoder {
    pub fn pick(support: &crate::media::EncoderSupport) -> Option<Encoder> {
        if support.videotoolbox {
            Some(Encoder::VideoToolbox)
        } else if support.libx264 {
            Some(Encoder::Libx264)
        } else {
            None
        }
    }

    fn video_args(self, quality: u8) -> Vec<String> {
        let q = quality.clamp(1, 100);
        match self {
            Encoder::VideoToolbox => s(&[
                "-c:v",
                "h264_videotoolbox",
                "-q:v",
                &q.to_string(),
                "-profile:v",
                "high",
                "-tag:v",
                "avc1",
            ]),
            Encoder::Libx264 => {
                // 100 → crf 4 (near lossless), 50 → ~18, 1 → ~33
                let crf = (33.0 - (q as f64) * 0.29).round().clamp(4.0, 33.0) as u32;
                s(&[
                    "-c:v",
                    "libx264",
                    "-preset",
                    "medium",
                    "-crf",
                    &crf.to_string(),
                    "-profile:v",
                    "high",
                    "-tag:v",
                    "avc1",
                ])
            }
        }
    }
}

fn s(v: &[&str]) -> Vec<String> {
    v.iter().map(|x| x.to_string()).collect()
}

fn even_down(v: u32) -> u32 {
    v & !1
}

/// Round to nearest even number, at least 2.
fn even_round(v: f64) -> u32 {
    let r = ((v / 2.0).round() as i64) * 2;
    r.max(2) as u32
}

/// Format a float for a filter option: fixed precision, no trailing zeros.
fn fnum(v: f64) -> String {
    let t = format!("{v:.4}");
    t.trim_end_matches('0').trim_end_matches('.').to_string()
}

impl ExportSpec {
    /// Crop rectangle clamped to the frame and aligned to even numbers
    /// (required by yuv420p chroma subsampling).
    pub fn normalized_crop(&self) -> Result<CropRect, ExportError> {
        let (sw, sh) = (even_down(self.source_width), even_down(self.source_height));
        let c = self.crop.unwrap_or(CropRect {
            x: 0,
            y: 0,
            w: sw,
            h: sh,
        });
        if c.x >= self.source_width || c.y >= self.source_height {
            return Err(ExportError::CropOutside);
        }
        let x = even_down(c.x);
        let y = even_down(c.y);
        let w = even_down(c.w.min(self.source_width - x));
        let h = even_down(c.h.min(self.source_height - y));
        if w < 2 || h < 2 {
            return Err(ExportError::CropTooSmall);
        }
        Ok(CropRect { x, y, w, h })
    }

    /// `(start, duration)` when the user trimmed the clip.
    pub fn normalized_trim(&self) -> Result<Option<(f64, f64)>, ExportError> {
        let start = self.trim_start.unwrap_or(0.0).max(0.0);
        let end = self
            .trim_end
            .unwrap_or(self.source_duration)
            .min(self.source_duration.max(0.0));
        if end - start < 0.05 {
            return Err(ExportError::BadTrim);
        }
        let untrimmed = start < 0.001 && (self.source_duration - end).abs() < 0.02;
        Ok((!untrimmed).then_some((start, end - start)))
    }
}

/// Pixel width of a watermark inside a cropped frame `crop_w` wide.
pub fn watermark_width(scale: f64, crop_w: u32) -> u32 {
    let scale = if scale.is_finite() { scale } else { 0.2 };
    let w = even_round(scale.clamp(0.01, 1.0) * crop_w as f64);
    w.min(even_down(crop_w).max(2))
}

fn clamp01(v: f64) -> f64 {
    if v.is_finite() {
        v.clamp(0.0, 1.0)
    } else {
        0.0
    }
}

pub fn build_filter_complex(spec: &ExportSpec) -> Result<String, ExportError> {
    let crop = spec.normalized_crop()?;
    let mut parts: Vec<String> = Vec::new();

    parts.push(format!(
        "[0:v]crop={}:{}:{}:{}[c0]",
        crop.w, crop.h, crop.x, crop.y
    ));

    for (i, wm) in spec.watermarks.iter().enumerate() {
        let idx = i + 1;
        let width = watermark_width(wm.scale, crop.w);
        let opacity = clamp01(wm.opacity);
        let x = (clamp01(wm.nx) * crop.w as f64).round() as i64;
        let y = (clamp01(wm.ny) * crop.h as f64).round() as i64;

        let mut chain = format!("[{idx}:v]scale={width}:-2,format=rgba");
        if opacity < 0.999 {
            chain.push_str(&format!(",colorchannelmixer=aa={}", fnum(opacity)));
        }
        chain.push_str(&format!("[w{idx}]"));
        parts.push(chain);

        // The clamp keeps the watermark fully inside the frame whatever its
        // aspect ratio is.
        parts.push(format!(
            "[c{prev}][w{idx}]overlay=x='min(max(0,{x}),main_w-overlay_w)':y='min(max(0,{y}),main_h-overlay_h)':format=auto[c{idx}]",
            prev = idx - 1,
        ));
    }

    parts.push(format!("[c{}]format=yuv420p[vout]", spec.watermarks.len()));
    Ok(parts.join(";"))
}

fn copyable_audio(codec: Option<&str>) -> bool {
    matches!(codec, Some("aac" | "mp3" | "ac3" | "eac3"))
}

pub fn build_export_args(spec: &ExportSpec, encoder: Encoder) -> Result<Vec<String>, ExportError> {
    let filter = build_filter_complex(spec)?;
    let trim = spec.normalized_trim()?;

    let mut a = s(&[
        "-hide_banner",
        "-nostdin",
        "-y",
        "-progress",
        "pipe:1",
        "-nostats",
    ]);
    if let Some((start, _)) = trim {
        a.extend(s(&["-ss", &format!("{start:.3}")]));
    }
    a.extend(s(&["-i", &spec.input]));
    for wm in &spec.watermarks {
        a.extend(s(&["-i", &wm.path]));
    }
    a.extend(s(&["-filter_complex", &filter, "-map", "[vout]"]));
    if spec.has_audio {
        a.extend(s(&["-map", "0:a:0"]));
    }
    a.extend(s(&["-map_metadata", "0"]));
    a.extend(encoder.video_args(spec.quality));
    a.extend(s(&["-pix_fmt", "yuv420p"]));

    if !spec.has_audio {
        a.push("-an".into());
    } else if trim.is_none() && copyable_audio(spec.audio_codec.as_deref()) {
        a.extend(s(&["-c:a", "copy"]));
    } else {
        a.extend(s(&["-c:a", "aac", "-b:a", "256k"]));
    }

    if let Some((_, dur)) = trim {
        a.extend(s(&["-t", &format!("{dur:.3}")]));
    }
    a.extend(s(&["-movflags", "+faststart", &spec.output]));
    Ok(a)
}

/// Seconds of output the export will produce (for progress percentages).
pub fn export_duration(spec: &ExportSpec) -> f64 {
    match spec.normalized_trim() {
        Ok(Some((_, d))) => d,
        _ => spec.source_duration,
    }
}

/// Re-encode to a small H.264/AAC file WKWebView is guaranteed to play.
/// Used when the user opens a container/codec the webview can't decode.
pub fn build_preview_args(input: &str, output: &str, encoder: Encoder, has_audio: bool) -> Vec<String> {
    let mut a = s(&[
        "-hide_banner",
        "-nostdin",
        "-y",
        "-progress",
        "pipe:1",
        "-nostats",
        "-i",
        input,
    ]);
    a.extend(s(&[
        "-vf",
        "scale=-2:'min(720,ih)',format=yuv420p",
        "-map",
        "0:v:0",
    ]));
    if has_audio {
        a.extend(s(&["-map", "0:a:0", "-c:a", "aac", "-b:a", "128k"]));
    } else {
        a.push("-an".into());
    }
    a.extend(encoder.video_args(55));
    a.extend(s(&["-pix_fmt", "yuv420p", "-movflags", "+faststart", output]));
    a
}

#[derive(Debug, Clone, PartialEq)]
pub struct SlideshowSpec {
    pub images: Vec<String>,
    pub audio: Option<String>,
    pub output: String,
    pub seconds_per_image: f64,
}

pub const SLIDESHOW_WIDTH: u32 = 1080;
pub const SLIDESHOW_HEIGHT: u32 = 1920;

pub fn slideshow_duration(spec: &SlideshowSpec) -> f64 {
    spec.images.len() as f64 * spec.seconds_per_image
}

/// Build a 1080x1920, 30 fps video out of still images (a TikTok photo post),
/// with the post's sound looped underneath.
pub fn build_slideshow_args(spec: &SlideshowSpec, encoder: Encoder) -> Result<Vec<String>, ExportError> {
    if spec.images.is_empty() {
        return Err(ExportError::NoImages);
    }
    let per = spec.seconds_per_image.max(0.5);
    let total = spec.images.len() as f64 * per;
    let mut a = s(&[
        "-hide_banner",
        "-nostdin",
        "-y",
        "-progress",
        "pipe:1",
        "-nostats",
    ]);
    for img in &spec.images {
        a.extend(s(&["-loop", "1", "-t", &format!("{per:.3}"), "-i", img]));
    }
    if let Some(audio) = &spec.audio {
        a.extend(s(&["-stream_loop", "-1", "-i", audio]));
    }

    let mut filter = String::new();
    for i in 0..spec.images.len() {
        filter.push_str(&format!(
            "[{i}:v]scale={SLIDESHOW_WIDTH}:{SLIDESHOW_HEIGHT}:force_original_aspect_ratio=decrease,\
             pad={SLIDESHOW_WIDTH}:{SLIDESHOW_HEIGHT}:(ow-iw)/2:(oh-ih)/2:black,setsar=1,fps=30,format=yuv420p[v{i}];"
        ));
    }
    for i in 0..spec.images.len() {
        filter.push_str(&format!("[v{i}]"));
    }
    filter.push_str(&format!("concat=n={}:v=1:a=0[vout]", spec.images.len()));

    a.extend(s(&["-filter_complex", &filter, "-map", "[vout]"]));
    if spec.audio.is_some() {
        a.extend(s(&[
            "-map",
            &format!("{}:a:0", spec.images.len()),
            "-c:a",
            "aac",
            "-b:a",
            "192k",
        ]));
    } else {
        a.push("-an".into());
    }
    a.extend(encoder.video_args(85));
    a.extend(s(&[
        "-pix_fmt",
        "yuv420p",
        "-t",
        &format!("{total:.3}"),
        "-movflags",
        "+faststart",
        &spec.output,
    ]));
    Ok(a)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn spec() -> ExportSpec {
        ExportSpec {
            input: "/in/a.mp4".into(),
            output: "/out/a-cut.mp4".into(),
            source_width: 1080,
            source_height: 1920,
            source_duration: 10.0,
            has_audio: true,
            audio_codec: Some("aac".into()),
            crop: None,
            trim_start: None,
            trim_end: None,
            watermarks: vec![],
            quality: 75,
        }
    }

    fn wm(path: &str, nx: f64, ny: f64, scale: f64, opacity: f64) -> WatermarkPlacement {
        WatermarkPlacement {
            path: path.into(),
            nx,
            ny,
            scale,
            opacity,
        }
    }

    fn arg_after<'a>(args: &'a [String], flag: &str) -> &'a str {
        let i = args
            .iter()
            .position(|a| a == flag)
            .unwrap_or_else(|| panic!("{flag} missing in {args:?}"));
        &args[i + 1]
    }

    #[test]
    fn plain_export_has_no_crop_and_copies_audio() {
        let args = build_export_args(&spec(), Encoder::VideoToolbox).unwrap();
        assert_eq!(arg_after(&args, "-i"), "/in/a.mp4");
        assert_eq!(
            arg_after(&args, "-filter_complex"),
            "[0:v]crop=1080:1920:0:0[c0];[c0]format=yuv420p[vout]"
        );
        assert_eq!(arg_after(&args, "-c:v"), "h264_videotoolbox");
        assert_eq!(arg_after(&args, "-q:v"), "75");
        assert_eq!(arg_after(&args, "-c:a"), "copy");
        assert!(!args.contains(&"-ss".to_string()));
        assert!(!args.contains(&"-t".to_string()));
        assert_eq!(args.last().unwrap(), "/out/a-cut.mp4");
    }

    #[test]
    fn crop_is_aligned_to_even_pixels_and_clamped() {
        let mut sp = spec();
        sp.crop = Some(CropRect {
            x: 11,
            y: 5,
            w: 501,
            h: 3000,
        });
        let c = sp.normalized_crop().unwrap();
        assert_eq!(
            c,
            CropRect {
                x: 10,
                y: 4,
                w: 500,
                h: 1916
            }
        );
    }

    #[test]
    fn crop_errors() {
        let mut sp = spec();
        sp.crop = Some(CropRect {
            x: 2000,
            y: 0,
            w: 10,
            h: 10,
        });
        assert_eq!(sp.normalized_crop(), Err(ExportError::CropOutside));
        sp.crop = Some(CropRect {
            x: 0,
            y: 0,
            w: 1,
            h: 100,
        });
        assert_eq!(sp.normalized_crop(), Err(ExportError::CropTooSmall));
    }

    #[test]
    fn trim_uses_input_seek_and_reencodes_audio() {
        let mut sp = spec();
        sp.trim_start = Some(1.5);
        sp.trim_end = Some(6.0);
        let args = build_export_args(&sp, Encoder::VideoToolbox).unwrap();
        let ss = args.iter().position(|a| a == "-ss").unwrap();
        let i = args.iter().position(|a| a == "-i").unwrap();
        assert!(ss < i, "-ss must come before -i for fast seeking");
        assert_eq!(args[ss + 1], "1.500");
        assert_eq!(arg_after(&args, "-t"), "4.500");
        assert_eq!(arg_after(&args, "-c:a"), "aac");
        assert_eq!(export_duration(&sp), 4.5);
    }

    #[test]
    fn full_length_trim_counts_as_untrimmed() {
        let mut sp = spec();
        sp.trim_start = Some(0.0);
        sp.trim_end = Some(10.0);
        assert_eq!(sp.normalized_trim(), Ok(None));
    }

    #[test]
    fn bad_trim_is_rejected() {
        let mut sp = spec();
        sp.trim_start = Some(5.0);
        sp.trim_end = Some(5.0);
        assert_eq!(sp.normalized_trim(), Err(ExportError::BadTrim));
    }

    #[test]
    fn non_copyable_audio_is_reencoded() {
        let mut sp = spec();
        sp.audio_codec = Some("opus".into());
        let args = build_export_args(&sp, Encoder::VideoToolbox).unwrap();
        assert_eq!(arg_after(&args, "-c:a"), "aac");
    }

    #[test]
    fn silent_video_has_no_audio_mapping() {
        let mut sp = spec();
        sp.has_audio = false;
        sp.audio_codec = None;
        let args = build_export_args(&sp, Encoder::VideoToolbox).unwrap();
        assert!(args.contains(&"-an".to_string()));
        assert!(!args.contains(&"0:a:0".to_string()));
    }

    #[test]
    fn watermark_overlay_is_scaled_to_the_cropped_frame() {
        let mut sp = spec();
        sp.crop = Some(CropRect {
            x: 100,
            y: 200,
            w: 800,
            h: 600,
        });
        sp.watermarks = vec![wm("/lib/logo.png", 0.5, 0.25, 0.25, 0.6)];
        let f = build_filter_complex(&sp).unwrap();
        assert_eq!(
            f,
            "[0:v]crop=800:600:100:200[c0];\
             [1:v]scale=200:-2,format=rgba,colorchannelmixer=aa=0.6[w1];\
             [c0][w1]overlay=x='min(max(0,400),main_w-overlay_w)':y='min(max(0,150),main_h-overlay_h)':format=auto[c1];\
             [c1]format=yuv420p[vout]"
        );
        let args = build_export_args(&sp, Encoder::VideoToolbox).unwrap();
        let inputs: Vec<&String> = args
            .iter()
            .enumerate()
            .filter(|(_, a)| *a == "-i")
            .map(|(i, _)| &args[i + 1])
            .collect();
        assert_eq!(inputs, vec!["/in/a.mp4", "/lib/logo.png"]);
    }

    #[test]
    fn opaque_watermark_skips_alpha_mixing() {
        let mut sp = spec();
        sp.watermarks = vec![wm("/lib/logo.png", 0.0, 0.0, 0.2, 1.0)];
        let f = build_filter_complex(&sp).unwrap();
        assert!(!f.contains("colorchannelmixer"));
        assert!(f.contains("[1:v]scale=216:-2,format=rgba[w1]"));
    }

    #[test]
    fn multiple_watermarks_chain_in_order() {
        let mut sp = spec();
        sp.watermarks = vec![
            wm("/lib/a.png", 0.0, 0.0, 0.2, 1.0),
            wm("/lib/b.png", 0.8, 0.9, 0.1, 0.5),
        ];
        let f = build_filter_complex(&sp).unwrap();
        assert!(f.contains("[c0][w1]overlay="));
        assert!(f.contains("[c1][w2]overlay="));
        assert!(f.ends_with("[c2]format=yuv420p[vout]"));
        let args = build_export_args(&sp, Encoder::Libx264).unwrap();
        assert_eq!(args.iter().filter(|a| *a == "-i").count(), 3);
    }

    #[test]
    fn nonsense_placement_values_are_sanitised() {
        let mut sp = spec();
        sp.watermarks = vec![wm("/lib/a.png", f64::NAN, 5.0, f64::INFINITY, -3.0)];
        let f = build_filter_complex(&sp).unwrap();
        assert!(f.contains("max(0,0)"), "{f}");
        assert!(f.contains("max(0,1920)"), "{f}");
        assert!(f.contains("colorchannelmixer=aa=0"), "{f}");
    }

    #[test]
    fn watermark_width_is_even_and_never_wider_than_the_frame() {
        assert_eq!(watermark_width(0.2, 1080), 216);
        assert_eq!(watermark_width(0.333, 1001), 334);
        assert_eq!(watermark_width(5.0, 1000), 1000);
        assert_eq!(watermark_width(0.0, 1000), 10);
    }

    #[test]
    fn libx264_fallback_maps_quality_to_crf() {
        let mut sp = spec();
        sp.quality = 100;
        let a = build_export_args(&sp, Encoder::Libx264).unwrap();
        assert_eq!(arg_after(&a, "-crf"), "4");
        sp.quality = 1;
        let a = build_export_args(&sp, Encoder::Libx264).unwrap();
        assert_eq!(arg_after(&a, "-crf"), "33");
    }

    #[test]
    fn encoder_selection_prefers_videotoolbox() {
        use crate::media::EncoderSupport;
        assert_eq!(
            Encoder::pick(&EncoderSupport {
                videotoolbox: true,
                libx264: true
            }),
            Some(Encoder::VideoToolbox)
        );
        assert_eq!(
            Encoder::pick(&EncoderSupport {
                videotoolbox: false,
                libx264: true
            }),
            Some(Encoder::Libx264)
        );
        assert_eq!(Encoder::pick(&EncoderSupport::default()), None);
    }

    #[test]
    fn slideshow_loops_audio_and_concats_every_image() {
        let sp = SlideshowSpec {
            images: vec!["/t/1.jpg".into(), "/t/2.jpg".into(), "/t/3.jpg".into()],
            audio: Some("/t/music.mp3".into()),
            output: "/t/out.mp4".into(),
            seconds_per_image: 3.0,
        };
        let a = build_slideshow_args(&sp, Encoder::VideoToolbox).unwrap();
        assert_eq!(a.iter().filter(|x| *x == "-loop").count(), 3);
        assert!(a.contains(&"-stream_loop".to_string()));
        let f = arg_after(&a, "-filter_complex");
        assert!(f.contains("[v0][v1][v2]concat=n=3:v=1:a=0[vout]"), "{f}");
        assert!(a.windows(2).any(|w| w[0] == "-map" && w[1] == "3:a:0"));
        // Per-image `-t 3.000` inputs come first; the last `-t` caps the output.
        let last_t = a.iter().rposition(|x| x == "-t").unwrap();
        assert_eq!(a[last_t + 1], "9.000");
        assert_eq!(slideshow_duration(&sp), 9.0);
    }

    #[test]
    fn slideshow_without_images_is_an_error() {
        let sp = SlideshowSpec {
            images: vec![],
            audio: None,
            output: "o".into(),
            seconds_per_image: 3.0,
        };
        assert_eq!(
            build_slideshow_args(&sp, Encoder::Libx264),
            Err(ExportError::NoImages)
        );
    }

    #[test]
    fn preview_proxy_is_capped_at_720p() {
        let a = build_preview_args("/in/x.mkv", "/tmp/p.mp4", Encoder::Libx264, true);
        assert!(arg_after(&a, "-vf").contains("min(720,ih)"));
        assert_eq!(arg_after(&a, "-c:a"), "aac");
        let silent = build_preview_args("/in/x.mkv", "/tmp/p.mp4", Encoder::Libx264, false);
        assert!(silent.contains(&"-an".to_string()));
    }
}
