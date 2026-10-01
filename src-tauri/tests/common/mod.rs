//! Shared helpers for the integration tests: a Tauri mock-runtime app wired to
//! the real command layer, plus ffmpeg-based media generators.
#![allow(dead_code)]

use fillerncut_lib::{register_commands, AppState};
use serde_json::Value;
use std::path::Path;
use std::process::Command;
use tauri::ipc::{CallbackFn, InvokeBody};
use tauri::test::{get_ipc_response, mock_builder, mock_context, noop_assets, MockRuntime, INVOKE_KEY};
use tauri::webview::InvokeRequest;
use tauri::{Manager, WebviewWindow, WebviewWindowBuilder};

pub fn have(tool: &str) -> bool {
    Command::new(tool)
        .arg("-version")
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false)
}

pub struct Harness {
    pub dir: tempfile::TempDir,
    pub app: tauri::App<MockRuntime>,
    pub win: WebviewWindow<MockRuntime>,
}

pub fn harness() -> Option<Harness> {
    if !have("ffmpeg") || !have("ffprobe") {
        eprintln!("skipping: ffmpeg/ffprobe not available");
        return None;
    }
    let dir = tempfile::tempdir().unwrap();
    let app = register_commands(mock_builder())
        .build(mock_context(noop_assets()))
        .unwrap();
    app.manage(AppState::new(dir.path().join("data"), dir.path().join("cache")));
    std::fs::create_dir_all(dir.path().join("data")).unwrap();
    let win = WebviewWindowBuilder::new(&app, "main", Default::default())
        .build()
        .unwrap();
    Some(Harness { dir, app, win })
}

pub fn call(win: &WebviewWindow<MockRuntime>, cmd: &str, body: Value) -> Result<Value, Value> {
    get_ipc_response(
        win,
        InvokeRequest {
            cmd: cmd.into(),
            callback: CallbackFn(0),
            error: CallbackFn(1),
            url: "tauri://localhost".parse().unwrap(),
            body: InvokeBody::Json(body),
            headers: Default::default(),
            invoke_key: INVOKE_KEY.to_string(),
        },
    )
    .map(|b| b.deserialize::<Value>().unwrap())
}

pub fn ff(args: &[&str]) {
    let out = Command::new("ffmpeg")
        .args(["-hide_banner", "-loglevel", "error", "-y"])
        .args(args)
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "ffmpeg {args:?}: {}",
        String::from_utf8_lossy(&out.stderr)
    );
}

pub fn make_video(dir: &Path, name: &str, size: &str, secs: u32) -> String {
    let path = dir.join(name).to_string_lossy().into_owned();
    ff(&[
        "-f",
        "lavfi",
        "-i",
        &format!("testsrc2=size={size}:rate=30:duration={secs}"),
        "-f",
        "lavfi",
        "-i",
        &format!("sine=frequency=440:duration={secs}"),
        "-c:v",
        "libx264",
        "-preset",
        "ultrafast",
        "-pix_fmt",
        "yuv420p",
        "-c:a",
        "aac",
        "-shortest",
        &path,
    ]);
    path
}

pub fn make_png(dir: &Path, name: &str, size: &str, color: &str) -> String {
    let path = dir.join(name).to_string_lossy().into_owned();
    ff(&[
        "-f",
        "lavfi",
        "-i",
        &format!("color=c={color}:s={size},format=rgba"),
        "-frames:v",
        "1",
        &path,
    ]);
    path
}

pub fn probe_dims(path: &str) -> (u64, u64, f64) {
    let out = Command::new("ffprobe")
        .args([
            "-v",
            "error",
            "-select_streams",
            "v:0",
            "-show_entries",
            "stream=width,height:format=duration",
            "-of",
            "json",
            path,
        ])
        .output()
        .unwrap();
    let v: Value = serde_json::from_slice(&out.stdout).unwrap();
    (
        v["streams"][0]["width"].as_u64().unwrap(),
        v["streams"][0]["height"].as_u64().unwrap(),
        v["format"]["duration"].as_str().unwrap().parse().unwrap(),
    )
}
