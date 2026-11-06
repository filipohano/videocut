//! "Remove text": downloading the add-on, finding still pictures in videos, filling in text and
//! burning the fill into exports. The model is a tiny stand-in (`fixtures/fake-inpaint.onnx`)
//! with MI-GAN's inputs and outputs that paints every pixel it is asked to fill pure red, so the
//! tests can see exactly where the app filled.

mod common;

use common::*;
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::io::{Read, Write};
use std::net::TcpListener;
use std::path::Path;
use std::sync::{Mutex, OnceLock};

const FAKE_MODEL: &[u8] = include_bytes!("fixtures/fake-inpaint.onnx");
const RED: (u8, u8, u8) = (255, 0, 0);
const GREY: (u8, u8, u8) = (128, 128, 128);

/// What the stand-in download server answers with.
static SERVED: Mutex<Vec<u8>> = Mutex::new(Vec::new());

/// A local server for the add-on download; the app is pointed at it (with the stand-in model's
/// checksum) once for the whole test binary.
fn download_server() {
    static STARTED: OnceLock<()> = OnceLock::new();
    STARTED.get_or_init(|| {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        std::thread::spawn(move || {
            for stream in listener.incoming().flatten() {
                let mut stream = stream;
                let mut buf = [0u8; 4096];
                let _ = stream.read(&mut buf);
                let body = SERVED.lock().unwrap().clone();
                let head = format!(
                    "HTTP/1.1 200 OK\r\nContent-Type: application/octet-stream\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                    body.len()
                );
                let _ = stream.write_all(head.as_bytes());
                let _ = stream.write_all(&body);
            }
        });
        std::env::set_var("FILLERNCUT_MODULE_URL", format!("http://127.0.0.1:{port}/migan.onnx"));
        std::env::set_var("FILLERNCUT_MODULE_SHA256", format!("{:x}", Sha256::digest(FAKE_MODEL)));
    });
}

/// A harness that already has the add-on installed.
fn with_model() -> Option<Harness> {
    harness_with_data(|data| {
        let dir = data.join("modules/remove-text");
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("migan.onnx"), FAKE_MODEL).unwrap();
    })
}

/// A grey picture with a "caption": a white bar with a black outline at (100, 300), 200×40.
fn captioned_photo(dir: &Path) -> String {
    let path = dir.join("captioned.png").to_string_lossy().into_owned();
    ff(&[
        "-f",
        "lavfi",
        "-i",
        "color=c=0x808080:s=400x600",
        "-vf",
        "drawbox=x=100:y=300:w=200:h=40:color=black:t=fill,drawbox=x=104:y=304:w=192:h=32:color=white:t=fill",
        "-frames:v",
        "1",
        &path,
    ]);
    path
}

fn fill(
    h: &Harness,
    path: &str,
    time: Option<f64>,
    boxes: Value,
    letters: bool,
    under: Value,
) -> Result<Value, Value> {
    call(
        &h.win,
        "inpaint_fill",
        json!({ "path": path, "time": time, "boxes": boxes, "letters": letters, "under": under }),
    )
}

fn photo_spec(input: &str, output: &str, fills: Value) -> Value {
    json!({ "spec": {
        "input": input, "output": output, "sourceWidth": 400, "sourceHeight": 600, "sourceDuration": 0.0,
        "hasAudio": false, "audioCodec": null, "crop": null, "trimStart": null, "trimEnd": null,
        "watermarks": [], "fills": fills, "imageFormat": "png", "quality": 90 } })
}

fn modules(h: &Harness) -> Vec<Value> {
    call(&h.win, "modules_list", json!({}))
        .unwrap()
        .as_array()
        .unwrap()
        .clone()
}

#[test]
fn the_add_on_is_downloaded_checked_and_removed() {
    let Some(h) = harness() else { return };
    download_server();
    let list = modules(&h);
    assert_eq!(list.len(), 1);
    assert_eq!(list[0]["id"], "remove-text");
    assert_eq!(list[0]["installed"], false);
    assert!(
        list[0]["bytes"].as_u64().unwrap() > 20_000_000,
        "the size shown is the real model's"
    );

    // Without the add-on, removing text is explained instead of failing oddly.
    let photo = captioned_photo(h.dir.path());
    let err = fill(
        &h,
        &photo,
        None,
        json!([{ "x": 100, "y": 300, "w": 200, "h": 40 }]),
        true,
        json!([]),
    )
    .unwrap_err();
    assert!(err.as_str().unwrap().contains("Settings → Add-ons"), "{err}");

    // A file that isn't the expected one is refused and nothing is installed.
    *SERVED.lock().unwrap() = b"this is not the model".to_vec();
    let err = call(&h.win, "module_install", json!({ "id": "remove-text" })).unwrap_err();
    assert!(err.as_str().unwrap().contains("checksum"), "{err}");
    assert_eq!(modules(&h)[0]["installed"], false);
    let dir = h.dir.path().join("data/modules/remove-text");
    assert!(!dir.join("migan.onnx").exists());
    assert!(
        !dir.join("migan.onnx.part").exists(),
        "no half file is left behind"
    );

    // The real thing installs, and the feature works.
    *SERVED.lock().unwrap() = FAKE_MODEL.to_vec();
    let view = call(&h.win, "module_install", json!({ "id": "remove-text" })).unwrap();
    assert_eq!(view["installed"], true);
    assert_eq!(std::fs::read(dir.join("migan.onnx")).unwrap(), FAKE_MODEL);
    assert_eq!(modules(&h)[0]["installed"], true);
    fill(
        &h,
        &photo,
        None,
        json!([{ "x": 100, "y": 300, "w": 200, "h": 40 }]),
        true,
        json!([]),
    )
    .unwrap();

    // Removing it deletes the file and locks the feature again.
    let view = call(&h.win, "module_remove", json!({ "id": "remove-text" })).unwrap();
    assert_eq!(view["installed"], false);
    assert!(!dir.exists());
    assert!(fill(
        &h,
        &photo,
        None,
        json!([{ "x": 100, "y": 300, "w": 200, "h": 40 }]),
        true,
        json!([])
    )
    .is_err());

    let err = call(&h.win, "module_install", json!({ "id": "no-such-thing" })).unwrap_err();
    assert!(err.as_str().unwrap().contains("no add-on"), "{err}");
}

#[test]
fn text_in_a_photo_is_filled_and_the_export_shows_the_fill() {
    let Some(h) = with_model() else { return };
    let photo = captioned_photo(h.dir.path());
    // The box drawn is bigger than the caption; only the caption's own pixels are filled.
    let result = fill(
        &h,
        &photo,
        None,
        json!([{ "x": 60, "y": 270, "w": 280, "h": 100 }]),
        true,
        json!([]),
    )
    .unwrap();
    assert_eq!(result["wholeBox"], false);
    let (x, y) = (result["x"].as_u64().unwrap(), result["y"].as_u64().unwrap());
    let (w, hh) = (result["w"].as_u64().unwrap(), result["h"].as_u64().unwrap());
    assert!(
        (85..=100).contains(&x) && (285..=300).contains(&y),
        "patch at {x},{y}"
    );
    assert!(
        x + w >= 300 && x + w <= 315 && y + hh >= 340 && y + hh <= 355,
        "patch {w}×{hh}"
    );
    let patch = result["path"].as_str().unwrap();
    assert!(Path::new(patch).is_file());

    let out = h.dir.path().join("filled.png").to_string_lossy().into_owned();
    let fills = json!([{ "path": patch, "x": x, "y": y }]);
    call(&h.win, "export_video", photo_spec(&photo, &out, fills)).expect("export");
    assert!(
        near(pixel(&out, 200, 320), RED, 30),
        "the caption was filled: {:?}",
        pixel(&out, 200, 320)
    );
    assert!(
        near(pixel(&out, 101, 301), RED, 60),
        "its outline too: {:?}",
        pixel(&out, 101, 301)
    );
    assert!(
        near(pixel(&out, 70, 280), GREY, 6),
        "the rest of the box is untouched"
    );
    assert!(
        near(pixel(&out, 20, 20), GREY, 6),
        "and so is the rest of the picture"
    );
}

#[test]
fn a_box_without_letters_is_filled_whole_and_fills_stack() {
    let Some(h) = with_model() else { return };
    let photo = captioned_photo(h.dir.path());
    let first = fill(
        &h,
        &photo,
        None,
        json!([{ "x": 20, "y": 40, "w": 60, "h": 30 }]),
        true,
        json!([]),
    )
    .unwrap();
    assert_eq!(
        first["wholeBox"], true,
        "plain grey: nothing to pick out, so the box is filled"
    );
    let whole = fill(
        &h,
        &photo,
        None,
        json!([{ "x": 20, "y": 40, "w": 60, "h": 30 }]),
        false,
        json!([]),
    )
    .unwrap();
    assert_eq!(whole["wholeBox"], false);

    // A second fill is made on the picture with the first one already in it.
    let under = json!([{ "path": first["path"], "x": first["x"], "y": first["y"] }]);
    let second = fill(
        &h,
        &photo,
        None,
        json!([{ "x": 100, "y": 300, "w": 200, "h": 40 }]),
        false,
        under,
    )
    .unwrap();
    assert_ne!(second["path"], first["path"]);

    let missing = json!([{ "path": h.dir.path().join("gone.png"), "x": 0, "y": 0 }]);
    let err = fill(
        &h,
        &photo,
        None,
        json!([{ "x": 100, "y": 300, "w": 200, "h": 40 }]),
        true,
        missing,
    )
    .unwrap_err();
    assert!(err.as_str().unwrap().contains("earlier fill"), "{err}");
    let err = fill(
        &h,
        &photo,
        None,
        json!([{ "x": 900, "y": 900, "w": 20, "h": 20 }]),
        true,
        json!([]),
    )
    .unwrap_err();
    assert!(err.as_str().unwrap().contains("outside"), "{err}");
}

/// A slideshow like the app builds from photo posts: two photos, two seconds each, with sound.
fn slideshow(dir: &Path) -> String {
    let path = dir.join("slides.mp4").to_string_lossy().into_owned();
    ff(&[
        "-f",
        "lavfi",
        "-i",
        "color=c=0x808080:s=400x600:r=30:d=2",
        "-f",
        "lavfi",
        "-i",
        "color=c=0x406080:s=400x600:r=30:d=2",
        "-f",
        "lavfi",
        "-i",
        "sine=frequency=330:duration=4",
        "-filter_complex",
        "[1:v]drawbox=x=100:y=300:w=200:h=40:color=white:t=fill[b];[0:v][b]concat=n=2:v=1:a=0[v]",
        "-map",
        "[v]",
        "-map",
        "2:a",
        "-c:v",
        "libx264",
        "-preset",
        "ultrafast",
        "-pix_fmt",
        "yuv420p",
        "-c:a",
        "aac",
        &path,
    ]);
    path
}

#[test]
fn still_pictures_slideshows_and_moving_videos_are_told_apart() {
    let Some(h) = harness() else { return };
    let dir = h.dir.path();
    let photo = captioned_photo(dir);
    let still = dir.join("still.mp4").to_string_lossy().into_owned();
    ff(&[
        "-loop",
        "1",
        "-i",
        &photo,
        "-f",
        "lavfi",
        "-i",
        "sine=frequency=330:duration=3",
        "-t",
        "3",
        "-r",
        "30",
        "-c:v",
        "libx264",
        "-preset",
        "ultrafast",
        "-pix_fmt",
        "yuv420p",
        "-c:a",
        "aac",
        "-shortest",
        &still,
    ]);
    let analyze = |path: &str, duration: f64| {
        call(
            &h.win,
            "analyze_stills",
            json!({ "path": path, "duration": duration }),
        )
        .unwrap()
    };

    assert_eq!(analyze(&still, 3.0), json!({ "kind": "still" }));
    let slides = analyze(&slideshow(dir), 4.0);
    assert_eq!(slides["kind"], "slides");
    let segments = slides["segments"].as_array().unwrap();
    assert_eq!(segments.len(), 2, "{slides}");
    assert!(segments[0]["start"].as_f64().unwrap() < 0.05);
    assert!(
        (segments[0]["end"].as_f64().unwrap() - 2.0).abs() < 0.05,
        "{slides}"
    );
    assert!(
        (segments[1]["start"].as_f64().unwrap() - 2.0).abs() < 0.05,
        "{slides}"
    );
    assert!(
        (segments[1]["end"].as_f64().unwrap() - 4.0).abs() < 0.1,
        "{slides}"
    );
    assert_eq!(
        analyze(&make_video(dir, "moving.mp4", "320x240", 2), 2.0),
        json!({ "kind": "moving" })
    );
}

#[test]
fn a_fill_on_the_second_photo_of_a_slideshow_only_covers_that_photo() {
    let Some(h) = with_model() else { return };
    let video = slideshow(h.dir.path());
    // Made on the second photo (the one with the caption), at its middle.
    let result = fill(
        &h,
        &video,
        Some(3.0),
        json!([{ "x": 90, "y": 290, "w": 220, "h": 60 }]),
        false,
        json!([]),
    )
    .unwrap();
    let out = h.dir.path().join("slides-out.mp4").to_string_lossy().into_owned();
    let spec = json!({ "spec": {
        "input": video, "output": out, "sourceWidth": 400, "sourceHeight": 600, "sourceDuration": 4.0,
        "hasAudio": true, "audioCodec": "aac", "crop": { "x": 50, "y": 100, "w": 300, "h": 400 },
        "trimStart": null, "trimEnd": null, "watermarks": [],
        "fills": [{ "path": result["path"], "x": result["x"], "y": result["y"], "start": 2.0, "end": 4.0 }],
        "imageFormat": null, "quality": 50 } });
    call(&h.win, "export_video", spec).expect("export");
    // The crop moved everything 50 left and 100 up: the caption's middle is now at (150, 220).
    assert!(
        near(pixel_at(&out, Some(1.0), 150, 220), GREY, 12),
        "the first photo is untouched"
    );
    assert!(
        near(pixel_at(&out, Some(3.0), 150, 220), RED, 40),
        "the second photo was filled: {:?}",
        pixel_at(&out, Some(3.0), 150, 220)
    );
    let (_, _, duration) = probe_dims(&out);
    assert!(
        (duration - 4.0).abs() < 0.2,
        "the whole slideshow, with its sound"
    );
}

/// With the real MI-GAN (set `FILLERNCUT_MIGAN` to the downloaded `migan.onnx`): a white
/// caption on a smooth background disappears into the background.
#[test]
fn the_real_model_removes_a_caption() {
    let Ok(model) = std::env::var("FILLERNCUT_MIGAN") else {
        eprintln!("skipping: FILLERNCUT_MIGAN isn't set");
        return;
    };
    let Some(h) = harness_with_data(|data| {
        let dir = data.join("modules/remove-text");
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::copy(&model, dir.join("migan.onnx")).unwrap();
    }) else {
        return;
    };
    let photo = h.dir.path().join("gradient.png").to_string_lossy().into_owned();
    ff(&[
        "-f", "lavfi", "-i", "gradients=s=720x1280:c0=0x3060a0:c1=0xc08040:x0=0:y0=0:x1=720:y1=1280:seed=1",
        "-vf", "drawtext=fontfile=/usr/share/fonts/truetype/dejavu/DejaVuSans-Bold.ttf:text='remove me please':fontsize=64:fontcolor=white:borderw=5:bordercolor=black:x=(w-tw)/2:y=820",
        "-frames:v", "1", &photo,
    ]);
    let before = pixel(&photo, 360, 850);
    let started = std::time::Instant::now();
    let r = fill(
        &h,
        &photo,
        None,
        json!([{ "x": 40, "y": 790, "w": 640, "h": 120 }]),
        true,
        json!([]),
    )
    .unwrap();
    eprintln!("fill took {:?}", started.elapsed());
    let out = h.dir.path().join("clean.png").to_string_lossy().into_owned();
    let spec = json!({ "spec": {
        "input": photo, "output": out, "sourceWidth": 720, "sourceHeight": 1280, "sourceDuration": 0.0,
        "hasAudio": false, "audioCodec": null, "crop": null, "trimStart": null, "trimEnd": null, "watermarks": [],
        "fills": [{ "path": r["path"], "x": r["x"], "y": r["y"] }], "imageFormat": "png", "quality": 90 } });
    call(&h.win, "export_video", spec).unwrap();
    // Every pixel of the caption's row is now close to the background just above and below it.
    let mut worst = 0;
    for x in (60..660).step_by(10) {
        let p = pixel(&out, x, 845);
        let (a, b) = (pixel(&out, x, 770), pixel(&out, x, 930));
        let mid = |i: fn((u8, u8, u8)) -> u8| (i(a) as i32 + i(b) as i32) / 2;
        let d = (p.0 as i32 - mid(|c| c.0))
            .abs()
            .max((p.1 as i32 - mid(|c| c.1)).abs())
            .max((p.2 as i32 - mid(|c| c.2)).abs());
        worst = worst.max(d);
    }
    eprintln!("caption pixel before {before:?}, worst difference from the background after: {worst}");
    assert!(worst < 40, "the caption is still visible ({worst})");
}
