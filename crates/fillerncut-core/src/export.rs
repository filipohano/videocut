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

/// The visible (non-transparent) part of a watermark image, as fractions of
/// the image: `l`/`t` where the content starts, `r`/`b` where it ends.
///
/// Watermarks may be moved so their transparent margins leave the frame, but
/// the visible content must stay inside it.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct ContentBox {
    pub l: f64,
    pub t: f64,
    pub r: f64,
    pub b: f64,
}

impl Default for ContentBox {
    fn default() -> Self {
        ContentBox {
            l: 0.0,
            t: 0.0,
            r: 1.0,
            b: 1.0,
        }
    }
}

impl ContentBox {
    /// Force sane values (finite, inside 0..1, at least 1% wide/tall).
    pub fn sanitized(self) -> ContentBox {
        let f = |v: f64, d: f64| if v.is_finite() { v.clamp(0.0, 1.0) } else { d };
        let (l, t, r, b) = (f(self.l, 0.0), f(self.t, 0.0), f(self.r, 1.0), f(self.b, 1.0));
        if r - l < 0.01 || b - t < 0.01 {
            return ContentBox::default();
        }
        ContentBox { l, t, r, b }
    }

    fn is_full(&self) -> bool {
        self.l <= 0.0 && self.t <= 0.0 && self.r >= 1.0 && self.b >= 1.0
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct WatermarkPlacement {
    pub path: String,
    /// Top-left corner of the *image* as a fraction of the cropped frame. May be
    /// negative or past 1 when only transparent margin leaves the frame.
    pub nx: f64,
    pub ny: f64,
    /// Image width as a fraction of the cropped frame width.
    pub scale: f64,
    /// 0 = invisible, 1 = fully opaque.
    pub opacity: f64,
    /// Where the visible content sits inside the image (default: all of it).
    #[serde(default)]
    pub content: ContentBox,
}

/// Output format when the source is a photo.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ImageFormat {
    Jpg,
    Png,
}

impl ImageFormat {
    pub fn extension(self) -> &'static str {
        match self {
            ImageFormat::Jpg => "jpg",
            ImageFormat::Png => "png",
        }
    }
}

/// ffmpeg `-q:v` for MJPEG from a 1..=100 "JPEG quality" (100 = best): 90 → 5, 75 → 9, 50 → 16.
pub fn jpeg_qscale(quality: u8) -> u32 {
    (31.0 - quality.clamp(1, 100) as f64 * 0.29)
        .round()
        .clamp(2.0, 31.0) as u32
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
    /// Bitrate of the source video stream, bits/s, when known.
    #[serde(default)]
    pub source_bitrate: Option<u64>,
    #[serde(default)]
    pub fps: Option<f64>,
    /// Set when the source is a photo: export a single JPG/PNG instead of a video.
    #[serde(default)]
    pub image_format: Option<ImageFormat>,
    /// Video: 1..=100, 50 aims for roughly the source's quality and file size, each
    /// step of 25 doubles / halves the bitrate. Photo (JPG): JPEG quality 1..=100.
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

    fn video_args(self, bitrate: u64) -> Vec<String> {
        let br = bitrate.clamp(MIN_BITRATE, MAX_BITRATE);
        match self {
            Encoder::VideoToolbox => s(&[
                "-c:v",
                "h264_videotoolbox",
                "-b:v",
                &br.to_string(),
                "-profile:v",
                "high",
                "-tag:v",
                "avc1",
            ]),
            Encoder::Libx264 => s(&[
                "-c:v",
                "libx264",
                "-preset",
                "medium",
                "-b:v",
                &br.to_string(),
                "-maxrate",
                &(br * 2).to_string(),
                "-bufsize",
                &(br * 2).to_string(),
                "-profile:v",
                "high",
                "-tag:v",
                "avc1",
            ]),
        }
    }
}

const MIN_BITRATE: u64 = 300_000;
const MAX_BITRATE: u64 = 60_000_000;

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

/// Largest watermark scale (image width / frame width). Large because a
/// watermark with lots of transparent margin needs a big image to make its
/// visible part fill the frame.
pub const MAX_WATERMARK_SCALE: f64 = 10.0;
const MAX_WATERMARK_PX: u32 = 8192;

/// Pixel width of a watermark inside a cropped frame `crop_w` wide.
pub fn watermark_width(scale: f64, crop_w: u32) -> u32 {
    let scale = if scale.is_finite() { scale } else { 0.2 };
    even_round(scale.clamp(0.01, MAX_WATERMARK_SCALE) * crop_w as f64).min(MAX_WATERMARK_PX)
}

fn clamp01(v: f64) -> f64 {
    if v.is_finite() {
        v.clamp(0.0, 1.0)
    } else {
        0.0
    }
}

/// Lowest and highest allowed overlay offset along one axis, as ffmpeg expressions.
fn bounds(start: f64, end: f64, own: &str, frame: &str, full: bool) -> (String, String) {
    if full || (start <= 0.0 && end >= 1.0) {
        return ("0".into(), format!("{frame}-{own}"));
    }
    let lo = if start <= 0.0 {
        "0".to_string()
    } else {
        format!("-{}*{own}", fnum(start))
    };
    let hi = if end >= 1.0 {
        format!("{frame}-{own}")
    } else {
        format!("{frame}-{}*{own}", fnum(end))
    };
    (lo, hi)
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
        let finite = |v: f64| if v.is_finite() { v.clamp(-20.0, 20.0) } else { 0.0 };
        let x = (finite(wm.nx) * crop.w as f64).round() as i64;
        let y = (finite(wm.ny) * crop.h as f64).round() as i64;
        let c = wm.content.sanitized();

        let mut chain = format!("[{idx}:v]scale={width}:-2,format=rgba");
        if opacity < 0.999 {
            chain.push_str(&format!(",colorchannelmixer=aa={}", fnum(opacity)));
        }
        chain.push_str(&format!("[w{idx}]"));
        parts.push(chain);

        // The clamp keeps the watermark's *visible content* inside the frame; its
        // transparent margins may hang outside.
        let (x_lo, x_hi) = bounds(c.l, c.r, "overlay_w", "main_w", c.is_full());
        let (y_lo, y_hi) = bounds(c.t, c.b, "overlay_h", "main_h", c.is_full());
        parts.push(format!(
            "[c{prev}][w{idx}]overlay=x='min(max({x_lo},{x}),{x_hi})':y='min(max({y_lo},{y}),{y_hi})':format=auto[c{idx}]",
            prev = idx - 1,
        ));
    }

    parts.push(format!("[c{}]format=yuv420p[vout]", spec.watermarks.len()));
    Ok(parts.join(";"))
}

fn copyable_audio(codec: Option<&str>) -> bool {
    matches!(codec, Some("aac" | "mp3" | "ac3" | "eac3"))
}

/// Photo export: crop + watermarks, one frame out as JPG or PNG.
fn build_image_args(spec: &ExportSpec, fmt: ImageFormat) -> Result<Vec<String>, ExportError> {
    // Same graph as for video, but the last step picks the photo pixel format.
    let graph = build_filter_complex(spec)?;
    let last = format!("[c{}]format=yuv420p[vout]", spec.watermarks.len());
    let pix = match fmt {
        ImageFormat::Jpg => "yuvj420p",
        ImageFormat::Png => "rgba",
    };
    let filter = graph.replacen(
        &last,
        &format!("[c{}]format={pix}[vout]", spec.watermarks.len()),
        1,
    );

    let mut a = s(&[
        "-hide_banner",
        "-nostdin",
        "-y",
        "-progress",
        "pipe:1",
        "-nostats",
    ]);
    a.extend(s(&["-i", &spec.input]));
    for wm in &spec.watermarks {
        a.extend(s(&["-i", &wm.path]));
    }
    a.extend(s(&[
        "-filter_complex",
        &filter,
        "-map",
        "[vout]",
        "-frames:v",
        "1",
        "-an",
    ]));
    match fmt {
        ImageFormat::Jpg => a.extend(s(&[
            "-c:v",
            "mjpeg",
            "-q:v",
            &jpeg_qscale(spec.quality).to_string(),
        ])),
        ImageFormat::Png => a.extend(s(&["-c:v", "png"])),
    }
    // A single named file, not an image sequence.
    a.extend(s(&["-update", "1", &spec.output]));
    Ok(a)
}

/// Video bitrate (bits/s) the export aims for: the source's own bitrate, scaled
/// for the cropped area and the quality slider. Keeps files about as big as the
/// original instead of ballooning, which fixed-quality modes do.
pub fn target_bitrate(spec: &ExportSpec) -> u64 {
    let src_px = (spec.source_width.max(1) as f64) * (spec.source_height.max(1) as f64);
    let crop_px = spec
        .normalized_crop()
        .map(|c| (c.w * c.h) as f64)
        .unwrap_or(src_px);
    let fps = spec
        .fps
        .filter(|f| f.is_finite() && *f > 1.0)
        .unwrap_or(30.0)
        .min(120.0);
    // Without a known bitrate assume a typical ~0.09 bits per pixel per frame.
    let base = spec
        .source_bitrate
        .filter(|b| *b > 0)
        .map(|b| b as f64)
        .unwrap_or(0.09 * src_px * fps);
    let mult = 2f64.powf((spec.quality.clamp(1, 100) as f64 - 50.0) / 25.0);
    // Smaller frames need relatively more bits per pixel, so area scales sub-linearly.
    let rate = base * (crop_px / src_px).powf(0.85) * mult;
    (rate.clamp(MIN_BITRATE as f64, MAX_BITRATE as f64)) as u64
}

/// Rough output size in bytes (video target + 128 kbit/s audio).
pub fn estimate_bytes(spec: &ExportSpec) -> u64 {
    if let Some(fmt) = spec.image_format {
        let px = spec
            .normalized_crop()
            .map(|c| (c.w * c.h) as f64)
            .unwrap_or((spec.source_width * spec.source_height) as f64);
        let bytes_per_px = match fmt {
            ImageFormat::Jpg => 0.04 + 0.5 * (spec.quality.clamp(1, 100) as f64 / 100.0).powf(2.2),
            ImageFormat::Png => 1.6,
        };
        return (px * bytes_per_px) as u64;
    }
    let secs = export_duration(spec).max(0.0);
    let audio = if spec.has_audio { 128_000.0 } else { 0.0 };
    ((target_bitrate(spec) as f64 + audio) * secs / 8.0) as u64
}

pub fn build_export_args(spec: &ExportSpec, encoder: Encoder) -> Result<Vec<String>, ExportError> {
    if let Some(fmt) = spec.image_format {
        return build_image_args(spec, fmt);
    }
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
    a.extend(encoder.video_args(target_bitrate(spec)));
    a.extend(s(&["-pix_fmt", "yuv420p"]));

    if !spec.has_audio {
        a.push("-an".into());
    } else if trim.is_none() && copyable_audio(spec.audio_codec.as_deref()) {
        a.extend(s(&["-c:a", "copy"]));
    } else {
        a.extend(s(&["-c:a", "aac", "-b:a", "160k"]));
    }

    if let Some((_, dur)) = trim {
        a.extend(s(&["-t", &format!("{dur:.3}")]));
    }
    a.extend(s(&["-movflags", "+faststart", &spec.output]));
    Ok(a)
}

/// Seconds of output the export will produce (for progress percentages).
pub fn export_duration(spec: &ExportSpec) -> f64 {
    if spec.image_format.is_some() {
        return 0.0;
    }
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
    a.extend(encoder.video_args(2_500_000));
    // A keyframe every ~0.4 s: seeking (scrubbing the timeline) never has to decode far back.
    a.extend(s(&["-g", "12", "-keyint_min", "12"]));
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
    a.extend(encoder.video_args(6_000_000));
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
            source_bitrate: Some(4_000_000),
            fps: Some(30.0),
            image_format: None,
            quality: 50,
        }
    }

    fn wm(path: &str, nx: f64, ny: f64, scale: f64, opacity: f64) -> WatermarkPlacement {
        WatermarkPlacement {
            path: path.into(),
            nx,
            ny,
            scale,
            opacity,
            content: ContentBox::default(),
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
        assert_eq!(
            arg_after(&args, "-b:v"),
            "4000000",
            "quality 50 keeps the source bitrate"
        );
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
        // 5.0 of the frame height, far past the edge, is still passed through; the
        // overlay clamp (not the number) keeps it inside.
        assert!(f.contains("max(0,9600)"), "{f}");
        assert!(f.contains("colorchannelmixer=aa=0"), "{f}");
    }

    #[test]
    fn watermark_width_is_even_and_capped() {
        assert_eq!(watermark_width(0.2, 1080), 216);
        assert_eq!(watermark_width(0.333, 1001), 334);
        assert_eq!(watermark_width(5.0, 1000), 5000);
        assert_eq!(watermark_width(99.0, 1000), 8192, "capped");
        assert_eq!(watermark_width(0.0, 1000), 10);
    }

    #[test]
    fn quality_scales_the_bitrate_around_the_source() {
        let mut sp = spec();
        for (q, want) in [
            (50, 4_000_000),
            (75, 8_000_000),
            (25, 2_000_000),
            (100, 16_000_000),
            (1, 1_000_000),
        ] {
            sp.quality = q;
            let got = target_bitrate(&sp) as f64;
            assert!((got / want as f64 - 1.0).abs() < 0.06, "q={q}: {got} vs {want}");
        }
    }

    #[test]
    fn cropping_lowers_the_bitrate_and_an_unknown_source_gets_a_sane_guess() {
        let mut sp = spec();
        sp.crop = Some(CropRect {
            x: 0,
            y: 0,
            w: 540,
            h: 960,
        }); // a quarter of the area
        let cropped = target_bitrate(&sp);
        assert!(cropped < 2_000_000 && cropped > 1_000_000, "{cropped}");
        sp.crop = None;
        sp.source_bitrate = None;
        let guess = target_bitrate(&sp);
        assert!((4_000_000..7_000_000).contains(&guess), "{guess}"); // 0.09 * 1080*1920 * 30
    }

    #[test]
    fn bitrate_is_clamped_and_estimate_follows_duration() {
        let mut sp = spec();
        sp.source_bitrate = Some(1);
        assert_eq!(target_bitrate(&sp), 300_000);
        sp.source_bitrate = Some(900_000_000);
        assert_eq!(target_bitrate(&sp), 60_000_000);
        sp.source_bitrate = Some(4_000_000);
        // 10 s of 4 Mbit/s video + 128 kbit/s audio
        assert_eq!(
            estimate_bytes(&sp),
            ((4_000_000.0 + 128_000.0) * 10.0 / 8.0) as u64
        );
        sp.trim_start = Some(0.0);
        sp.trim_end = Some(5.0);
        assert_eq!(
            estimate_bytes(&sp),
            ((4_000_000.0 + 128_000.0) * 5.0 / 8.0) as u64
        );
    }

    #[test]
    fn both_encoders_are_bitrate_driven() {
        let sp = spec();
        let vt = build_export_args(&sp, Encoder::VideoToolbox).unwrap();
        assert!(!vt.contains(&"-q:v".to_string()));
        let x264 = build_export_args(&sp, Encoder::Libx264).unwrap();
        assert_eq!(arg_after(&x264, "-b:v"), "4000000");
        assert_eq!(arg_after(&x264, "-maxrate"), "8000000");
    }

    #[test]
    fn watermark_content_box_lets_transparent_margin_leave_the_frame() {
        let mut sp = spec();
        let mut w = wm("/lib/logo.png", -0.1, 0.5, 0.5, 1.0);
        w.content = ContentBox {
            l: 0.2,
            t: 0.0,
            r: 0.9,
            b: 1.0,
        };
        sp.watermarks = vec![w];
        let f = build_filter_complex(&sp).unwrap();
        // x may go as low as -0.2 image widths, and the content's right edge (0.9)
        // must stay inside the frame.
        assert!(
            f.contains("x='min(max(-0.2*overlay_w,-108),main_w-0.9*overlay_w)'"),
            "{f}"
        );
        assert!(f.contains("y='min(max(0,960),main_h-overlay_h)'"), "{f}");
    }

    #[test]
    fn photo_export_is_one_frame_with_the_right_codec() {
        let mut sp = spec();
        sp.image_format = Some(ImageFormat::Jpg);
        sp.output = "/out/a.jpg".into();
        sp.quality = 90;
        sp.watermarks = vec![wm("/lib/logo.png", 0.5, 0.5, 0.2, 1.0)];
        let a = build_export_args(&sp, Encoder::Libx264).unwrap();
        assert_eq!(arg_after(&a, "-c:v"), "mjpeg");
        assert_eq!(arg_after(&a, "-q:v"), "5");
        assert_eq!(arg_after(&a, "-frames:v"), "1");
        assert!(arg_after(&a, "-filter_complex").ends_with("[c1]format=yuvj420p[vout]"));
        assert!(!a.contains(&"-movflags".to_string()) && !a.contains(&"-ss".to_string()));
        assert_eq!(a.last().unwrap(), "/out/a.jpg");

        sp.image_format = Some(ImageFormat::Png);
        sp.output = "/out/a.png".into();
        let a = build_export_args(&sp, Encoder::Libx264).unwrap();
        assert_eq!(arg_after(&a, "-c:v"), "png");
        assert!(arg_after(&a, "-filter_complex").ends_with("[c1]format=rgba[vout]"));
        assert!(!a.contains(&"-q:v".to_string()));
    }

    #[test]
    fn photos_ignore_trim_and_have_no_duration() {
        let mut sp = spec();
        sp.image_format = Some(ImageFormat::Png);
        sp.source_duration = 0.0;
        sp.trim_start = Some(3.0); // nonsense for a photo; must not error
        assert!(build_export_args(&sp, Encoder::Libx264).is_ok());
        assert_eq!(export_duration(&sp), 0.0);
    }

    #[test]
    fn jpeg_quality_maps_to_ffmpeg_qscale() {
        assert_eq!(jpeg_qscale(100), 2);
        assert_eq!(jpeg_qscale(90), 5);
        assert_eq!(jpeg_qscale(75), 9);
        assert_eq!(jpeg_qscale(1), 31);
        assert_eq!(jpeg_qscale(0), 31, "clamped");
    }

    #[test]
    fn photo_size_estimates_scale_with_pixels_and_quality() {
        let mut sp = spec();
        sp.image_format = Some(ImageFormat::Jpg);
        sp.quality = 90;
        let hi = estimate_bytes(&sp);
        sp.quality = 50;
        let lo = estimate_bytes(&sp);
        assert!(hi > lo * 2 && lo > 100_000, "{hi} {lo}");
        sp.crop = Some(CropRect {
            x: 0,
            y: 0,
            w: 540,
            h: 960,
        });
        assert!(estimate_bytes(&sp) < lo / 3);
    }

    #[test]
    fn bad_content_boxes_fall_back_to_the_whole_image() {
        let b = ContentBox {
            l: 0.5,
            t: 0.0,
            r: 0.5,
            b: 1.0,
        }
        .sanitized();
        assert_eq!(b, ContentBox::default());
        let nan = ContentBox {
            l: f64::NAN,
            t: 0.0,
            r: 1.0,
            b: 1.0,
        }
        .sanitized();
        assert_eq!(nan, ContentBox::default());
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
        assert_eq!(arg_after(&a, "-g"), "12", "short GOP for smooth scrubbing");
        let silent = build_preview_args("/in/x.mkv", "/tmp/p.mp4", Encoder::Libx264, false);
        assert!(silent.contains(&"-an".to_string()));
    }
}
