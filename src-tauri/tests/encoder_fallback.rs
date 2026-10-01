//! When the hardware encoder fails, the export must silently retry on the CPU.
//! Forced here by asking for VideoToolbox on a machine that doesn't have it.
//! (Own test binary: it sets a process-wide environment variable.)

mod common;

use common::*;
use serde_json::json;

#[test]
fn export_falls_back_to_the_cpu_encoder_when_videotoolbox_fails() {
    std::env::set_var("FILLERNCUT_ENCODER", "videotoolbox");
    let Some(h) = harness() else { return };
    let help = std::process::Command::new("ffmpeg")
        .args(["-hide_banner", "-encoders"])
        .output()
        .unwrap();
    let encoders = String::from_utf8_lossy(&help.stdout);
    if encoders.contains("h264_videotoolbox") || !encoders.contains("libx264") {
        eprintln!("skipping: needs an ffmpeg with libx264 and without VideoToolbox");
        return;
    }

    let src = make_video(h.dir.path(), "src.mp4", "320x240", 2);
    let out = h.dir.path().join("out.mp4").to_string_lossy().into_owned();
    let spec = json!({ "spec": {
        "input": src, "output": out, "sourceWidth": 320, "sourceHeight": 240, "sourceDuration": 2.0,
        "hasAudio": true, "audioCodec": "aac", "crop": { "x": 0, "y": 0, "w": 200, "h": 100 },
        "trimStart": null, "trimEnd": null, "watermarks": [], "quality": 75 } });

    let res = call(&h.win, "export_video", spec).expect("fallback should succeed");
    assert_eq!(res, json!(out));
    assert_eq!(probe_dims(&out).0, 200);
    assert!(!h.dir.path().join("out.partial.mp4").exists());
}
