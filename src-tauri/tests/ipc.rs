//! Drives the real command layer through Tauri's IPC (mock runtime, no window
//! system) with a real ffmpeg. The JSON below has the exact shape the
//! TypeScript frontend sends, so this pins the frontend ⇄ backend contract.
//!
//! Skipped (with a note) when ffmpeg/ffprobe aren't installed.

mod common;

use common::*;
use serde_json::{json, Value};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use tauri::Listener;

#[test]
fn settings_roundtrip_with_frontend_json_shape() {
    let Some(h) = harness() else { return };
    let defaults = call(&h.win, "get_settings", json!({})).unwrap();
    assert_eq!(
        defaults["updateMode"], "auto",
        "auto-install on launch is the default"
    );
    assert_eq!(defaults["autoUpdateYtdlp"], true);

    let mut s = defaults.clone();
    s["updateMode"] = json!("notify");
    s["cookiesBrowser"] = json!("safari");
    s["quality"] = json!(250); // out of range → clamped
    let saved = call(&h.win, "save_settings", json!({ "settings": s })).unwrap();
    assert_eq!(saved["quality"], 100);
    let again = call(&h.win, "get_settings", json!({})).unwrap();
    assert_eq!(again["updateMode"], "notify");
    assert_eq!(again["cookiesBrowser"], "safari");
    assert!(
        h.dir.path().join("data/settings.json").is_file(),
        "settings persist to disk"
    );
}

#[test]
fn watermark_library_persists_and_remembers_placement() {
    let Some(h) = harness() else { return };
    let logo = make_png(h.dir.path(), "My Logo.png", "300x120", "red");

    let entry = call(&h.win, "library_add", json!({ "path": logo })).unwrap();
    assert_eq!(entry["name"], "My Logo");
    assert_eq!(entry["aspect"], 0.4);
    let id = entry["id"].as_str().unwrap().to_string();
    assert!(Path::new(entry["path"].as_str().unwrap()).is_file());

    let updated = call(
        &h.win,
        "library_update",
        json!({ "id": id, "patch": { "nx": 0.1, "ny": 0.2, "scale": 0.35, "opacity": 0.5 } }),
    )
    .unwrap();
    assert_eq!(updated["scale"], 0.35);

    // A brand-new state over the same directory = the app was restarted.
    let reopened = fillerncut_core::LibraryStore::new(h.dir.path().join("data/watermarks"));
    let list = reopened.list();
    assert_eq!(list.len(), 1);
    assert_eq!(
        (list[0].nx, list[0].ny, list[0].scale, list[0].opacity),
        (0.1, 0.2, 0.35, 0.5)
    );

    let listed = call(&h.win, "library_list", json!({})).unwrap();
    assert_eq!(listed.as_array().unwrap().len(), 1);
    call(&h.win, "library_remove", json!({ "id": id })).unwrap();
    assert!(call(&h.win, "library_list", json!({}))
        .unwrap()
        .as_array()
        .unwrap()
        .is_empty());

    assert!(call(
        &h.win,
        "library_add",
        json!({ "path": h.dir.path().join("nope.txt") })
    )
    .is_err());
}

#[test]
fn probe_then_export_with_crop_trim_and_watermark() {
    let Some(h) = harness() else { return };
    let src = make_video(h.dir.path(), "src.mp4", "640x360", 4);
    let logo = make_png(h.dir.path(), "logo.png", "200x80", "white");

    let info = call(&h.win, "probe_media", json!({ "path": src })).unwrap();
    assert_eq!(
        (info["width"].as_u64(), info["height"].as_u64()),
        (Some(640), Some(360))
    );
    assert_eq!(info["hasAudio"], true);
    assert_eq!(info["audioCodec"], "aac");

    // Count progress events like the frontend does.
    let seen = Arc::new(Mutex::new(Vec::<f64>::new()));
    let s2 = seen.clone();
    h.app.listen("job-progress", move |ev| {
        let p: Value = serde_json::from_str(ev.payload()).unwrap();
        if p["job"] == "export" {
            s2.lock().unwrap().push(p["fraction"].as_f64().unwrap_or(-1.0));
        }
    });

    let out = h.dir.path().join("out.mp4").to_string_lossy().into_owned();
    let spec = json!({
        "input": src, "output": out,
        "sourceWidth": 640, "sourceHeight": 360, "sourceDuration": info["duration"],
        "hasAudio": true, "audioCodec": "aac",
        "crop": { "x": 100, "y": 40, "w": 300, "h": 200 },
        "trimStart": 1.0, "trimEnd": 3.0,
        "watermarks": [{ "path": logo, "nx": 0.7, "ny": 0.8, "scale": 0.25, "opacity": 0.6 }],
        "quality": 75
    });
    let result = call(&h.win, "export_video", json!({ "spec": spec })).unwrap();
    assert_eq!(result, json!(out));

    let (w, hgt, dur) = probe_dims(&out);
    assert_eq!((w, hgt), (300, 200));
    assert!((dur - 2.0).abs() < 0.2, "duration {dur}");
    assert!(
        !h.dir.path().join("out.partial.mp4").exists(),
        "partial file is renamed away"
    );

    let fractions = seen.lock().unwrap().clone();
    assert!(!fractions.is_empty(), "progress events were emitted");
    assert!(fractions.iter().all(|f| (0.0..=1.0).contains(f)));
    assert_eq!(*fractions.last().unwrap(), 1.0);
}

#[test]
fn export_rejects_bad_requests() {
    let Some(h) = harness() else { return };
    let src = make_video(h.dir.path(), "src.mp4", "320x240", 1);
    let base = |output: &str, wm: Value| {
        json!({ "spec": {
            "input": src, "output": output, "sourceWidth": 320, "sourceHeight": 240, "sourceDuration": 1.0,
            "hasAudio": true, "audioCodec": "aac", "crop": null, "trimStart": null, "trimEnd": null,
            "watermarks": wm, "quality": 75 } })
    };
    let dir = h.dir.path();
    assert!(
        call(
            &h.win,
            "export_video",
            base(&dir.join("x.mov").to_string_lossy(), json!([]))
        )
        .is_err(),
        "non-mp4 target"
    );
    assert!(
        call(&h.win, "export_video", base(&src, json!([]))).is_err(),
        "overwriting the source"
    );
    let missing = json!([{ "path": dir.join("gone.png"), "nx": 0, "ny": 0, "scale": 0.2, "opacity": 1 }]);
    let err = call(
        &h.win,
        "export_video",
        base(&dir.join("y.mp4").to_string_lossy(), missing),
    )
    .unwrap_err();
    assert!(err.as_str().unwrap().contains("watermark"), "{err}");
    assert!(!dir.join("y.mp4").exists() && !dir.join("y.partial.mp4").exists());
}

#[test]
fn cancelling_an_export_stops_ffmpeg_and_leaves_no_files() {
    let Some(h) = harness() else { return };
    let src = make_video(h.dir.path(), "long.mp4", "1280x720", 40);
    let out = h.dir.path().join("cancelled.mp4").to_string_lossy().into_owned();
    let spec = json!({ "spec": {
        "input": src, "output": out, "sourceWidth": 1280, "sourceHeight": 720, "sourceDuration": 40.0,
        "hasAudio": true, "audioCodec": "aac", "crop": null, "trimStart": null, "trimEnd": null,
        "watermarks": [], "quality": 100 } });

    let win = h.win.clone();
    let stop = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
    let stop2 = stop.clone();
    let canceller = std::thread::spawn(move || {
        // Keep cancelling until the export has registered and stops.
        while !stop2.load(std::sync::atomic::Ordering::Relaxed) {
            std::thread::sleep(std::time::Duration::from_millis(100));
            let _ = call(&win, "cancel_job", json!({ "job": "export" }));
        }
    });
    let started = std::time::Instant::now();
    let err = call(&h.win, "export_video", spec).unwrap_err();
    assert_eq!(err, json!("Cancelled"));
    assert!(started.elapsed().as_secs() < 20, "cancel should be prompt");
    stop.store(true, std::sync::atomic::Ordering::Relaxed);
    canceller.join().unwrap();
    assert!(!Path::new(&out).exists());
    assert!(!h.dir.path().join("cancelled.partial.mp4").exists());

    // The job slot is released, so exporting again works.
    let ok = call(&h.win, "default_save_path", json!({ "source": src })).unwrap();
    assert!(ok.as_str().unwrap().ends_with("long-cut.mp4"));
}

#[test]
fn preview_proxy_is_made_once_and_cached() {
    let Some(h) = harness() else { return };
    let src = make_video(h.dir.path(), "big.mp4", "1280x720", 1);
    let args = json!({ "path": src, "hasAudio": true, "duration": 1.0 });
    let first = call(&h.win, "make_preview", args.clone()).unwrap();
    let path = PathBuf::from(first.as_str().unwrap());
    assert!(path.is_file());
    assert_eq!(probe_dims(path.to_str().unwrap()).1, 720);
    let modified = std::fs::metadata(&path).unwrap().modified().unwrap();
    let second = call(&h.win, "make_preview", args).unwrap();
    assert_eq!(second, first);
    assert_eq!(
        std::fs::metadata(&path).unwrap().modified().unwrap(),
        modified,
        "cache hit: not re-encoded"
    );
}

#[test]
fn default_save_path_never_clobbers() {
    let Some(h) = harness() else { return };
    let src = h.dir.path().join("clip.mov");
    std::fs::write(&src, b"x").unwrap();
    let first = call(&h.win, "default_save_path", json!({ "source": src })).unwrap();
    assert!(first.as_str().unwrap().ends_with("clip-cut.mp4"));
    std::fs::write(h.dir.path().join("clip-cut.mp4"), b"x").unwrap();
    let second = call(&h.win, "default_save_path", json!({ "source": src })).unwrap();
    assert!(second.as_str().unwrap().ends_with("clip-cut-2.mp4"));
}

#[test]
fn app_info_reports_an_h264_encoder() {
    let Some(h) = harness() else { return };
    let info = call(&h.win, "app_info", json!({})).unwrap();
    assert_eq!(info["ffmpegFound"], true);
    assert!(matches!(
        info["encoder"].as_str(),
        Some("videotoolbox" | "libx264")
    ));
}
