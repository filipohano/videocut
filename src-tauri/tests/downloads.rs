//! Download flows end to end, offline:
//!  * TikTok goes through a local stand-in for the watermark-free API
//!    (`FILLERNCUT_TIKTOK_API_HOST`), serving a video, a photo post and an error.
//!  * Instagram / X / TikTok-fallback go through a fake `yt-dlp` shell script
//!    placed where the app keeps its managed copy.

mod common;

use common::*;
use serde_json::{json, Value};
use std::io::{Read, Write};
use std::net::TcpListener;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, OnceLock};
use tauri::Listener;

// ───────────────────────── local TikTok API stand-in ─────────────────────────

const VIDEO_ID: &str = "1111111111111111111";
const PHOTO_ID: &str = "2222222222222222222";
const ERROR_ID: &str = "3333333333333333333";

struct Fixture {
    _dir: tempfile::TempDir,
    host: String,
}

fn fixture() -> &'static Fixture {
    static F: OnceLock<Fixture> = OnceLock::new();
    F.get_or_init(|| {
        let dir = tempfile::tempdir().unwrap();
        let video = std::fs::read(make_video(dir.path(), "post.mp4", "720x1280", 2)).unwrap();
        let imgs: Vec<Vec<u8>> = ["red", "green", "blue"]
            .iter()
            .enumerate()
            .map(|(i, c)| std::fs::read(make_png(dir.path(), &format!("p{i}.png"), "540x720", c)).unwrap())
            .collect();
        let music_path = dir.path().join("music.aac");
        ff(&[
            "-f",
            "lavfi",
            "-i",
            "sine=frequency=330:duration=2",
            "-c:a",
            "aac",
            "-f",
            "adts",
            music_path.to_str().unwrap(),
        ]);
        let music = std::fs::read(music_path).unwrap();

        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let host = format!("http://{}", listener.local_addr().unwrap());
        let base = host.clone();
        std::thread::spawn(move || {
            for stream in listener.incoming().flatten() {
                let (video, imgs, music, base) = (video.clone(), imgs.clone(), music.clone(), base.clone());
                std::thread::spawn(move || serve(stream, &video, &imgs, &music, &base));
            }
        });
        std::env::set_var("FILLERNCUT_TIKTOK_API_HOST", &host);
        Fixture { _dir: dir, host }
    })
}

fn serve(mut s: std::net::TcpStream, video: &[u8], imgs: &[Vec<u8>], music: &[u8], base: &str) {
    let mut buf = [0u8; 4096];
    let n = s.read(&mut buf).unwrap_or(0);
    let req = String::from_utf8_lossy(&buf[..n]);
    let target = req.split_whitespace().nth(1).unwrap_or("/").to_string();
    let (status, ctype, body): (&str, &str, Vec<u8>) = if target.starts_with("/api/") {
        let json = if target.contains(VIDEO_ID) {
            json!({"code":0,"data":{"id":VIDEO_ID,"title":"A video","hdplay":format!("{base}/video.mp4"),"play":format!("{base}/video.mp4")}})
        } else if target.contains(PHOTO_ID) {
            json!({"code":0,"data":{"id":PHOTO_ID,"title":"Photos",
                "images":[format!("{base}/img0.png"),format!("{base}/img1.png"),format!("{base}/img2.png")],
                "music":format!("{base}/music.aac")}})
        } else {
            json!({"code":-1,"msg":"Url parsing is failed! Please check url."})
        };
        ("200 OK", "application/json", json.to_string().into_bytes())
    } else if target == "/video.mp4" {
        ("200 OK", "video/mp4", video.to_vec())
    } else if let Some(i) = target
        .strip_prefix("/img")
        .and_then(|r| r.strip_suffix(".png"))
        .and_then(|r| r.parse::<usize>().ok())
    {
        ("200 OK", "image/png", imgs[i].clone())
    } else if target == "/music.aac" {
        ("200 OK", "audio/aac", music.to_vec())
    } else {
        ("404 Not Found", "text/plain", b"nope".to_vec())
    };
    let _ = write!(
        s,
        "HTTP/1.1 {status}\r\nContent-Type: {ctype}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
        body.len()
    );
    let _ = s.write_all(&body);
}

// ───────────────────────── fake yt-dlp ─────────────────────────

const FAKE_YTDLP: &str = r#"#!/usr/bin/env bash
here="$(cd "$(dirname "$0")" && pwd)"
printf '%s\n' "$@" > "$here/args.txt"
[ -f "$here/slow.txt" ] && exec sleep 30
if [ -f "$here/fail.txt" ]; then cat "$here/fail.txt" >&2; exit 1; fi
while [ $# -gt 0 ]; do
  if [ "$1" = "-o" ]; then template="$2"; fi
  shift
done
out="$(dirname "$template")/fake-download.mp4"
cp "$(cat "$here/sample_path.txt")" "$out"
echo "FCPROG|50|100|NA|1000"
echo "FCPROG|100|100|NA|1000"
echo "FCFILE|$out"
"#;

fn install_fake_ytdlp(h: &Harness) -> PathBuf {
    let sample = make_video(h.dir.path(), "from-ytdlp.mp4", "320x240", 1);
    let bin_dir = h.dir.path().join("data/bin");
    std::fs::create_dir_all(&bin_dir).unwrap();
    let script = bin_dir.join("yt-dlp");
    std::fs::write(&script, FAKE_YTDLP).unwrap();
    std::fs::write(bin_dir.join("sample_path.txt"), sample).unwrap();
    use std::os::unix::fs::PermissionsExt;
    std::fs::set_permissions(&script, std::fs::Permissions::from_mode(0o755)).unwrap();
    bin_dir
}

fn set_download_dir(h: &Harness) -> PathBuf {
    let dir = h.dir.path().join("downloads");
    let mut s = call(&h.win, "get_settings", json!({})).unwrap();
    s["downloadDir"] = json!(dir);
    call(&h.win, "save_settings", json!({ "settings": s })).unwrap();
    dir
}

fn collect_progress(h: &Harness) -> Arc<Mutex<Vec<f64>>> {
    let seen = Arc::new(Mutex::new(Vec::new()));
    let s2 = seen.clone();
    h.app.listen("job-progress", move |ev| {
        let p: Value = serde_json::from_str(ev.payload()).unwrap();
        if p["job"] == "download" {
            if let Some(f) = p["fraction"].as_f64() {
                s2.lock().unwrap().push(f);
            }
        }
    });
    seen
}

fn dims(path: &str) -> (u64, u64, f64) {
    probe_dims(path)
}

// ───────────────────────── tests ─────────────────────────

#[test]
fn tiktok_video_downloads_without_watermark_via_the_api() {
    let Some(h) = harness() else { return };
    fixture();
    let dir = set_download_dir(&h);
    let progress = collect_progress(&h);

    // The bare 19-digit post id is accepted, as the UI promises.
    let res = call(&h.win, "download_link", json!({ "url": VIDEO_ID })).unwrap();
    let path = res["path"].as_str().unwrap();
    assert_eq!(res["platform"], "tiktok");
    assert_eq!(res["title"], "A video");
    assert_eq!(Path::new(path), dir.join(format!("tiktok-{VIDEO_ID}.mp4")));
    assert_eq!(dims(path).0, 720);

    let p = progress.lock().unwrap().clone();
    assert!(
        !p.is_empty() && p.iter().all(|f| (0.0..=1.0).contains(f)),
        "{p:?}"
    );
    assert!(p.contains(&1.0), "the download reports completion: {p:?}");
}

#[test]
fn tiktok_photo_post_becomes_a_portrait_slideshow_with_sound() {
    let Some(h) = harness() else { return };
    fixture();
    let dir = set_download_dir(&h);
    let progress = collect_progress(&h);

    let url = format!("https://www.tiktok.com/@someone/photo/{PHOTO_ID}");
    let res = call(&h.win, "download_link", json!({ "url": url })).unwrap();
    let path = res["path"].as_str().unwrap();
    assert_eq!(Path::new(path), dir.join(format!("tiktok-{PHOTO_ID}.mp4")));

    let (w, hgt, dur) = dims(path);
    assert_eq!((w, hgt), (1080, 1920));
    assert!((dur - 9.0).abs() < 0.3, "3 photos x 3 s, got {dur}");
    let probe = call(&h.win, "probe_media", json!({ "path": path })).unwrap();
    assert_eq!(probe["hasAudio"], true, "the post's sound is kept");

    // Progress only ever moves forward and the scratch folder is cleaned up.
    let p = progress.lock().unwrap().clone();
    assert!(
        p.windows(2).all(|w| w[1] >= w[0] - 1e-9),
        "progress went backwards: {p:?}"
    );
    assert!(!h.dir.path().join(format!("cache/slideshow-{PHOTO_ID}")).exists());
    assert!(!dir.join(format!("tiktok-{PHOTO_ID}.partial.mp4")).exists());
}

#[test]
fn tiktok_api_failure_falls_back_to_ytdlp() {
    let Some(h) = harness() else { return };
    fixture();
    set_download_dir(&h);
    let bin = install_fake_ytdlp(&h);

    let res = call(&h.win, "download_link", json!({ "url": ERROR_ID })).unwrap();
    assert!(res["path"].as_str().unwrap().ends_with("fake-download.mp4"));
    let args = std::fs::read_to_string(bin.join("args.txt")).unwrap();
    assert!(
        args.contains(&format!("https://www.tiktok.com/@_/video/{ERROR_ID}")),
        "{args}"
    );
}

#[test]
fn instagram_goes_through_ytdlp_with_the_selected_browser_cookies() {
    let Some(h) = harness() else { return };
    let dir = set_download_dir(&h);
    let bin = install_fake_ytdlp(&h);
    let mut s = call(&h.win, "get_settings", json!({})).unwrap();
    s["cookiesBrowser"] = json!("chrome");
    call(&h.win, "save_settings", json!({ "settings": s })).unwrap();
    let progress = collect_progress(&h);

    let res = call(
        &h.win,
        "download_link",
        json!({ "url": "https://www.instagram.com/reel/C1a2B3c4D5e/?igsh=zzz" }),
    )
    .unwrap();
    assert_eq!(res["platform"], "instagram");
    assert!(Path::new(res["path"].as_str().unwrap()).starts_with(&dir));

    let args: Vec<String> = std::fs::read_to_string(bin.join("args.txt"))
        .unwrap()
        .lines()
        .map(String::from)
        .collect();
    let i = args
        .iter()
        .position(|a| a == "--cookies-from-browser")
        .expect("cookies flag");
    assert_eq!(args[i + 1], "chrome");
    assert_eq!(
        args.last().unwrap(),
        "https://www.instagram.com/reel/C1a2B3c4D5e/",
        "tracking params stripped, URL last"
    );
    assert_eq!(args[args.len() - 2], "--");

    let p = progress.lock().unwrap().clone();
    assert!(
        p.contains(&0.5) && p.contains(&1.0),
        "yt-dlp progress is forwarded: {p:?}"
    );
}

#[test]
fn twitter_links_are_normalised_for_ytdlp() {
    let Some(h) = harness() else { return };
    set_download_dir(&h);
    let bin = install_fake_ytdlp(&h);
    let res = call(
        &h.win,
        "download_link",
        json!({ "url": "https://twitter.com/jack/status/20?s=20" }),
    )
    .unwrap();
    assert_eq!(res["platform"], "twitter");
    let args = std::fs::read_to_string(bin.join("args.txt")).unwrap();
    assert!(
        args.trim_end().ends_with("https://x.com/jack/status/20"),
        "{args}"
    );
}

#[test]
fn login_errors_are_explained_with_a_pointer_to_settings() {
    let Some(h) = harness() else { return };
    set_download_dir(&h);
    let bin = install_fake_ytdlp(&h);
    std::fs::write(
        bin.join("fail.txt"),
        "ERROR: [Instagram] C1a2: Login required. Use --cookies-from-browser\n",
    )
    .unwrap();

    let err = call(
        &h.win,
        "download_link",
        json!({ "url": "https://www.instagram.com/reel/C1a2B3c4D5e/" }),
    )
    .unwrap_err();
    let msg = err.as_str().unwrap();
    assert!(msg.contains("Settings") && msg.contains("Instagram"), "{msg}");
}

#[test]
fn unsupported_and_empty_links_are_rejected_before_any_work() {
    let Some(h) = harness() else { return };
    for bad in ["", "hello", "https://youtube.com/watch?v=1", "https://x.com/jack"] {
        let err = call(&h.win, "download_link", json!({ "url": bad })).unwrap_err();
        assert!(!err.as_str().unwrap().is_empty(), "{bad:?}");
    }
}

#[test]
fn cancelling_a_download_kills_the_downloader_and_frees_the_slot() {
    let Some(h) = harness() else { return };
    set_download_dir(&h);
    let bin = install_fake_ytdlp(&h);
    std::fs::write(bin.join("slow.txt"), "").unwrap();

    let win = h.win.clone();
    let t = std::thread::spawn(move || {
        for _ in 0..60 {
            std::thread::sleep(std::time::Duration::from_millis(100));
            let _ = call(&win, "cancel_job", json!({ "job": "download" }));
        }
    });
    let err = call(
        &h.win,
        "download_link",
        json!({ "url": "https://x.com/jack/status/20" }),
    )
    .unwrap_err();
    assert_eq!(err, json!("Cancelled"));
    drop(t);

    // Slot is free again: a normal download now succeeds.
    std::fs::remove_file(bin.join("slow.txt")).unwrap();
    let ok = call(
        &h.win,
        "download_link",
        json!({ "url": "https://x.com/jack/status/20" }),
    )
    .unwrap();
    assert!(ok["path"].as_str().unwrap().ends_with("fake-download.mp4"));
}

#[test]
fn fixture_host_is_local() {
    assert!(fixture().host.starts_with("http://127.0.0.1:"));
}
