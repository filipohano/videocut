//! End-to-end checks that the generated command lines really work with a real
//! ffmpeg. Skipped (with a note) when ffmpeg/ffprobe aren't installed.

use fillerncut_core::*;
use std::path::{Path, PathBuf};
use std::process::Command;

fn tool(name: &str) -> Option<PathBuf> {
    let out = Command::new(name).arg("-version").output().ok()?;
    out.status.success().then(|| PathBuf::from(name))
}

struct Env {
    dir: tempfile::TempDir,
    ffmpeg: PathBuf,
    ffprobe: PathBuf,
    encoder: Encoder,
}

fn env() -> Option<Env> {
    let ffmpeg = tool("ffmpeg")?;
    let ffprobe = tool("ffprobe")?;
    let enc = Command::new(&ffmpeg)
        .args(["-hide_banner", "-encoders"])
        .output()
        .ok()?;
    let support = parse_encoders(&String::from_utf8_lossy(&enc.stdout));
    let encoder = Encoder::pick(&support)?;
    Some(Env {
        dir: tempfile::tempdir().unwrap(),
        ffmpeg,
        ffprobe,
        encoder,
    })
}

fn run(bin: &Path, args: &[String]) {
    let out = Command::new(bin).args(args).output().expect("spawn");
    assert!(
        out.status.success(),
        "{} failed:\nargs: {:?}\nstderr: {}",
        bin.display(),
        args,
        String::from_utf8_lossy(&out.stderr)
    );
}

fn lavfi(env: &Env, name: &str, size: &str, dur: &str) -> String {
    let path = env.dir.path().join(name).to_string_lossy().into_owned();
    let args: Vec<String> = [
        "-hide_banner",
        "-y",
        "-f",
        "lavfi",
        "-i",
        &format!("testsrc2=size={size}:rate=30:duration={dur}"),
        "-f",
        "lavfi",
        "-i",
        &format!("sine=frequency=440:duration={dur}"),
        "-c:v",
        "libx264",
        "-pix_fmt",
        "yuv420p",
        "-c:a",
        "aac",
        "-shortest",
        &path,
    ]
    .iter()
    .map(|s| s.to_string())
    .collect();
    run(&env.ffmpeg, &args);
    path
}

fn logo(env: &Env, name: &str, size: &str, color: &str) -> String {
    let path = env.dir.path().join(name).to_string_lossy().into_owned();
    let args: Vec<String> = [
        "-hide_banner",
        "-y",
        "-f",
        "lavfi",
        "-i",
        &format!("color=c={color}:s={size},format=rgba"),
        "-frames:v",
        "1",
        &path,
    ]
    .iter()
    .map(|s| s.to_string())
    .collect();
    run(&env.ffmpeg, &args);
    path
}

fn probe(env: &Env, path: &str) -> MediaInfo {
    let out = Command::new(&env.ffprobe)
        .args([
            "-v",
            "error",
            "-print_format",
            "json",
            "-show_streams",
            "-show_format",
            path,
        ])
        .output()
        .unwrap();
    parse_ffprobe(&String::from_utf8_lossy(&out.stdout)).unwrap()
}

fn spec_for(env: &Env, input: &str, info: &MediaInfo, out_name: &str) -> ExportSpec {
    ExportSpec {
        input: input.into(),
        output: env.dir.path().join(out_name).to_string_lossy().into_owned(),
        source_width: info.width,
        source_height: info.height,
        source_duration: info.duration,
        has_audio: info.has_audio,
        audio_codec: info.audio_codec.clone(),
        crop: None,
        trim_start: None,
        trim_end: None,
        watermarks: vec![],
        quality: 75,
    }
}

#[test]
fn crop_trim_and_two_watermarks_export_with_the_right_geometry() {
    let Some(env) = env() else {
        eprintln!("skipping: ffmpeg/ffprobe not available");
        return;
    };
    let input = lavfi(&env, "src.mp4", "640x360", "4");
    let info = probe(&env, &input);
    assert_eq!((info.width, info.height), (640, 360));

    let mut spec = spec_for(&env, &input, &info, "out.mp4");
    spec.crop = Some(CropRect {
        x: 101,
        y: 51,
        w: 301,
        h: 201,
    }); // odd on purpose
    spec.trim_start = Some(1.0);
    spec.trim_end = Some(3.0);
    spec.watermarks = vec![
        WatermarkPlacement {
            path: logo(&env, "a.png", "200x100", "red"),
            nx: 0.8,
            ny: 0.8,
            scale: 0.3,
            opacity: 0.5,
        },
        // Deliberately placed past the corner: must be clamped inside the frame.
        WatermarkPlacement {
            path: logo(&env, "b.png", "50x200", "blue"),
            nx: 1.0,
            ny: 1.0,
            scale: 0.1,
            opacity: 1.0,
        },
    ];

    let args = build_export_args(&spec, env.encoder).unwrap();
    run(&env.ffmpeg, &args);

    let out = probe(&env, &spec.output);
    assert_eq!((out.width, out.height), (300, 200), "even-aligned crop");
    assert!((out.duration - 2.0).abs() < 0.15, "duration was {}", out.duration);
    assert!(out.has_audio);
    assert_eq!(out.video_codec.as_deref(), Some("h264"));
}

#[test]
fn untouched_export_keeps_size_and_copies_audio() {
    let Some(env) = env() else {
        eprintln!("skipping: ffmpeg/ffprobe not available");
        return;
    };
    let input = lavfi(&env, "src.mp4", "320x240", "2");
    let info = probe(&env, &input);
    let spec = spec_for(&env, &input, &info, "same.mp4");
    run(&env.ffmpeg, &build_export_args(&spec, env.encoder).unwrap());
    let out = probe(&env, &spec.output);
    assert_eq!((out.width, out.height), (320, 240));
    assert_eq!(out.audio_codec.as_deref(), Some("aac"));
}

#[test]
fn watermark_really_lands_inside_the_frame_at_the_requested_spot() {
    let Some(env) = env() else {
        eprintln!("skipping: ffmpeg/ffprobe not available");
        return;
    };
    // Black video + opaque white logo makes the check trivial: sample two pixels.
    let black = env.dir.path().join("black.mp4").to_string_lossy().into_owned();
    run(
        &env.ffmpeg,
        &[
            "-hide_banner",
            "-y",
            "-f",
            "lavfi",
            "-i",
            "color=c=black:s=400x300:r=10:d=1",
            "-c:v",
            "libx264",
            "-pix_fmt",
            "yuv420p",
            &black,
        ]
        .iter()
        .map(|s| s.to_string())
        .collect::<Vec<_>>(),
    );
    let info = probe(&env, &black);
    let mut spec = spec_for(&env, &black, &info, "wm.mp4");
    spec.quality = 100;
    // 100x100 white logo, 25% of 400 = 100px wide, top-left at (200, 100).
    spec.watermarks = vec![WatermarkPlacement {
        path: logo(&env, "w.png", "100x100", "white"),
        nx: 0.5,
        ny: 1.0 / 3.0,
        scale: 0.25,
        opacity: 1.0,
    }];
    run(&env.ffmpeg, &build_export_args(&spec, env.encoder).unwrap());

    let pixel = |x: u32, y: u32| -> u8 {
        let out = Command::new(&env.ffmpeg)
            .args([
                "-v",
                "error",
                "-i",
                &spec.output,
                "-vf",
                &format!("format=gray,crop=1:1:{x}:{y}"),
                "-frames:v",
                "1",
                "-f",
                "rawvideo",
                "-",
            ])
            .output()
            .unwrap();
        out.stdout[0]
    };
    assert!(pixel(250, 150) > 200, "inside the logo should be white");
    assert!(pixel(50, 50) < 40, "outside the logo should be black");
    assert!(pixel(350, 250) < 40, "below/right of the logo should be black");
}

#[test]
fn slideshow_builds_a_portrait_video_of_the_right_length() {
    let Some(env) = env() else {
        eprintln!("skipping: ffmpeg/ffprobe not available");
        return;
    };
    let images = vec![
        logo(&env, "1.png", "1080x1440", "red"),
        logo(&env, "2.png", "1080x1920", "green"),
        logo(&env, "3.png", "800x800", "blue"),
    ];
    let audio = env.dir.path().join("music.m4a").to_string_lossy().into_owned();
    run(
        &env.ffmpeg,
        &[
            "-hide_banner",
            "-y",
            "-f",
            "lavfi",
            "-i",
            "sine=frequency=330:duration=1.5",
            "-c:a",
            "aac",
            &audio,
        ]
        .iter()
        .map(|s| s.to_string())
        .collect::<Vec<_>>(),
    );

    let spec = SlideshowSpec {
        images,
        audio: Some(audio),
        output: env.dir.path().join("slides.mp4").to_string_lossy().into_owned(),
        seconds_per_image: 1.0,
    };
    run(&env.ffmpeg, &build_slideshow_args(&spec, env.encoder).unwrap());
    let out = probe(&env, &spec.output);
    assert_eq!((out.width, out.height), (1080, 1920));
    assert!((out.duration - 3.0).abs() < 0.2, "duration was {}", out.duration);
    assert!(out.has_audio, "short music must be looped under the slideshow");
}

#[test]
fn preview_proxy_plays_back_as_h264_capped_at_720p() {
    let Some(env) = env() else {
        eprintln!("skipping: ffmpeg/ffprobe not available");
        return;
    };
    let input = lavfi(&env, "big.mp4", "1920x1080", "1");
    let out = env.dir.path().join("proxy.mp4").to_string_lossy().into_owned();
    run(&env.ffmpeg, &build_preview_args(&input, &out, env.encoder, true));
    let info = probe(&env, &out);
    assert_eq!(info.height, 720);
    assert_eq!(info.width, 1280);
    assert_eq!(info.video_codec.as_deref(), Some("h264"));
}
