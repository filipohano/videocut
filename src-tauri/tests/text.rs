//! Text on the video: saving the picture of a text box for the exporter, burning it in with a
//! real ffmpeg (the pixels are read back), and the saved text styles.

mod common;

use base64::Engine;
use common::*;
use serde_json::{json, Value};
use std::path::Path;
use std::process::Command;

fn b64(bytes: &[u8]) -> String {
    base64::engine::general_purpose::STANDARD.encode(bytes)
}

/// A PNG of one colour, `size` like "100x40", as base64.
fn png_b64(h: &Harness, name: &str, size: &str, color: &str) -> String {
    let path = make_png(h.dir.path(), name, size, color);
    b64(&std::fs::read(path).unwrap())
}

fn save_image(h: &Harness, png: &str) -> Result<Value, Value> {
    call(&h.win, "save_text_image", json!({ "pngBase64": png }))
}

/// The colour of one pixel of an image or video frame, as (r, g, b).
fn pixel(path: &str, x: u32, y: u32) -> (u8, u8, u8) {
    let out = Command::new("ffmpeg")
        .args([
            "-v",
            "error",
            "-i",
            path,
            "-vf",
            &format!("format=rgb24,crop=1:1:{x}:{y}"),
            "-frames:v",
            "1",
            "-f",
            "rawvideo",
            "-pix_fmt",
            "rgb24",
            "-",
        ])
        .output()
        .unwrap();
    assert!(out.status.success(), "{}", String::from_utf8_lossy(&out.stderr));
    assert_eq!(out.stdout.len(), 3, "one pixel");
    (out.stdout[0], out.stdout[1], out.stdout[2])
}

fn near(a: (u8, u8, u8), b: (u8, u8, u8), tolerance: i32) -> bool {
    [(a.0, b.0), (a.1, b.1), (a.2, b.2)]
        .iter()
        .all(|(x, y)| (*x as i32 - *y as i32).abs() <= tolerance)
}

#[test]
fn a_text_picture_is_saved_for_the_exporter_and_named_by_its_contents() {
    let Some(h) = harness() else { return };
    let red = png_b64(&h, "red.png", "40x20", "red");
    let blue = png_b64(&h, "blue.png", "40x20", "blue");

    let first = save_image(&h, &red).unwrap();
    let path = first.as_str().unwrap();
    assert!(Path::new(path).is_file());
    assert!(
        Path::new(path).starts_with(h.dir.path().join("cache").join("texts")),
        "{path}"
    );
    assert!(path.ends_with(".png"));
    assert!(std::fs::read(path).unwrap().starts_with(b"\x89PNG"));
    // The same picture again is the same file; another picture is another one.
    assert_eq!(save_image(&h, &red).unwrap(), first);
    assert_ne!(save_image(&h, &blue).unwrap(), first);
    assert!(std::fs::read_dir(h.dir.path().join("cache/texts"))
        .unwrap()
        .flatten()
        .all(|e| !e.file_name().to_string_lossy().ends_with(".part")));
}

#[test]
fn something_that_is_not_a_png_is_never_written_to_disk() {
    let Some(h) = harness() else { return };
    for (what, bad) in [
        ("not base64", "###".to_string()),
        ("text", b64(b"hello, this is not a picture")),
        ("empty", String::new()),
        ("a jpeg", b64(b"\xff\xd8\xff\xe0 pretend jpeg")),
        ("a script", b64(b"#!/bin/sh\nrm -rf ~\n")),
    ] {
        let err = save_image(&h, &bad).unwrap_err();
        assert!(err.as_str().unwrap().contains("text picture"), "{what}: {err}");
    }
    assert!(
        !h.dir.path().join("cache/texts").exists()
            || std::fs::read_dir(h.dir.path().join("cache/texts"))
                .unwrap()
                .count()
                == 0
    );
}

#[test]
fn a_text_picture_is_burned_in_exactly_where_it_was_placed() {
    let Some(h) = harness() else { return };
    // A flat grey picture, so the colour under the text is known.
    let src = h.dir.path().join("grey.mp4").to_string_lossy().into_owned();
    ff(&[
        "-f",
        "lavfi",
        "-i",
        "color=c=0x808080:s=320x240:r=30:d=1",
        "-c:v",
        "libx264",
        "-pix_fmt",
        "yuv420p",
        &src,
    ]);
    // A 100x40 magenta block with a transparent 10 px margin all round (so the block is 80x20).
    let block = h.dir.path().join("block.png").to_string_lossy().into_owned();
    ff(&[
        "-f",
        "lavfi",
        "-i",
        "color=c=0xff00ff:s=80x20",
        "-vf",
        "format=rgba,pad=100:40:10:10:color=black@0",
        "-frames:v",
        "1",
        &block,
    ]);
    let path = save_image(&h, &b64(&std::fs::read(&block).unwrap())).unwrap();

    let out = h.dir.path().join("out.mp4").to_string_lossy().into_owned();
    // The image's top-left is at (60, 100), 100 px wide: scale = 100/320. The block inside is
    // the middle 80x20 (content .1 .25 .9 .75).
    let spec = json!({ "spec": {
        "input": src, "output": out, "sourceWidth": 320, "sourceHeight": 240, "sourceDuration": 1.0,
        "hasAudio": false, "audioCodec": null, "crop": null, "trimStart": null, "trimEnd": null,
        "watermarks": [{ "path": path, "nx": 60.0 / 320.0, "ny": 100.0 / 240.0, "scale": 100.0 / 320.0, "opacity": 1.0,
                         "content": { "l": 0.1, "t": 0.25, "r": 0.9, "b": 0.75 } }],
        "quality": 90 } });
    call(&h.win, "export_video", spec).expect("export with text");

    // Inside the block: magenta. In the transparent margin and outside the picture: the grey video.
    assert!(
        near(pixel(&out, 100, 120), (255, 0, 255), 40),
        "block: {:?}",
        pixel(&out, 100, 120)
    );
    assert!(
        near(pixel(&out, 61, 101), (128, 128, 128), 24),
        "margin: {:?}",
        pixel(&out, 61, 101)
    );
    assert!(
        near(pixel(&out, 10, 10), (128, 128, 128), 24),
        "far away: {:?}",
        pixel(&out, 10, 10)
    );
    // The block's corners land on the pixels they were placed on, not a pixel off.
    assert!(
        near(pixel(&out, 70, 110), (255, 0, 255), 60),
        "top-left of the block"
    );
    assert!(
        near(pixel(&out, 149, 129), (255, 0, 255), 60),
        "bottom-right of the block"
    );
    assert!(
        near(pixel(&out, 68, 108), (128, 128, 128), 40),
        "just outside the block"
    );
    assert!(
        near(pixel(&out, 152, 132), (128, 128, 128), 40),
        "just outside the block"
    );
}

#[test]
fn text_is_burned_into_a_photo_after_the_crop() {
    let Some(h) = harness() else { return };
    let photo = make_png(h.dir.path(), "photo.png", "400x300", "0x808080");
    let block = png_b64(&h, "block.png", "40x20", "0x00ff00");
    let path = save_image(&h, &block).unwrap();
    let out = h.dir.path().join("out.png").to_string_lossy().into_owned();
    // Crop to the right half, 200x300 starting at x=200. The text sits at (10, 20) of the *cropped* photo.
    let spec = json!({ "spec": {
        "input": photo, "output": out, "sourceWidth": 400, "sourceHeight": 300, "sourceDuration": 0.0,
        "hasAudio": false, "audioCodec": null, "crop": { "x": 200, "y": 0, "w": 200, "h": 300 },
        "trimStart": null, "trimEnd": null, "imageFormat": "png",
        "watermarks": [{ "path": path, "nx": 10.0 / 200.0, "ny": 20.0 / 300.0, "scale": 40.0 / 200.0, "opacity": 1.0 }],
        "quality": 90 } });
    call(&h.win, "export_video", spec).expect("photo export with text");
    assert!(
        near(pixel(&out, 30, 30), (0, 255, 0), 8),
        "{:?}",
        pixel(&out, 30, 30)
    );
    assert!(near(pixel(&out, 100, 150), (128, 128, 128), 8));
    assert!(near(pixel(&out, 5, 5), (128, 128, 128), 8), "left of the text");
}

#[test]
fn a_text_picture_that_has_gone_missing_is_explained() {
    let Some(h) = harness() else { return };
    let src = make_video(h.dir.path(), "src.mp4", "320x240", 1);
    let out = h.dir.path().join("out.mp4").to_string_lossy().into_owned();
    let spec = json!({ "spec": {
        "input": src, "output": out, "sourceWidth": 320, "sourceHeight": 240, "sourceDuration": 1.0,
        "hasAudio": true, "audioCodec": "aac", "crop": null, "trimStart": null, "trimEnd": null,
        "watermarks": [{ "path": "/no/such/text.png", "nx": 0.1, "ny": 0.1, "scale": 0.2, "opacity": 1.0 }],
        "quality": 60 } });
    let err = call(&h.win, "export_video", spec).unwrap_err();
    assert!(err.as_str().unwrap().contains("image is missing"), "{err}");
}

fn style(bg: Value) -> Value {
    json!({ "color": "#FFFFFF", "background": bg, "bold": true, "align": "center", "size": 7.0 })
}

#[test]
fn text_styles_can_be_saved_listed_replaced_and_removed() {
    let Some(h) = harness() else { return };
    assert_eq!(call(&h.win, "text_presets_list", json!({})).unwrap(), json!([]));

    let saved = call(
        &h.win,
        "text_presets_save",
        json!({ "name": "White on black", "style": style(json!("#000000")) }),
    )
    .unwrap();
    assert_eq!(saved["name"], "White on black");
    assert_eq!(saved["style"]["color"], "#ffffff", "tidied");
    let id = saved["id"].as_str().unwrap().to_string();

    let other = call(
        &h.win,
        "text_presets_save",
        json!({ "name": "Plain", "style": style(Value::Null) }),
    )
    .unwrap();
    assert!(other["style"]["background"].is_null());
    assert_eq!(
        call(&h.win, "text_presets_list", json!({}))
            .unwrap()
            .as_array()
            .unwrap()
            .len(),
        2
    );

    // The same name replaces that style.
    let again = call(
        &h.win,
        "text_presets_save",
        json!({ "name": "white ON black", "style": style(json!("#ff0000")) }),
    )
    .unwrap();
    assert_eq!(again["id"], id.as_str());
    let list = call(&h.win, "text_presets_list", json!({})).unwrap();
    assert_eq!(list.as_array().unwrap().len(), 2);
    assert_eq!(list[0]["style"]["background"], "#ff0000");

    call(&h.win, "text_presets_remove", json!({ "id": id })).unwrap();
    let list = call(&h.win, "text_presets_list", json!({})).unwrap();
    assert_eq!(list.as_array().unwrap().len(), 1);
    assert_eq!(list[0]["name"], "Plain");
    // They are on disk.
    let file = std::fs::read_to_string(h.dir.path().join("data/text-presets.json")).unwrap();
    assert!(
        file.contains("Plain") && !file.contains("White on black"),
        "{file}"
    );
}

#[test]
fn bad_text_styles_are_explained() {
    let Some(h) = harness() else { return };
    for (name, style, wanted) in [
        ("", style(json!("#000")), "name"),
        (
            "x",
            json!({ "color": "red", "background": null, "bold": true, "align": "center", "size": 7 }),
            "colour",
        ),
        (
            "x",
            json!({ "color": "#fff", "background": "blue", "bold": true, "align": "center", "size": 7 }),
            "colour",
        ),
        (
            "x",
            json!({ "color": "#fff", "background": null, "bold": true, "align": "diagonal", "size": 7 }),
            "alignment",
        ),
    ] {
        let err = call(
            &h.win,
            "text_presets_save",
            json!({ "name": name, "style": style }),
        )
        .unwrap_err();
        assert!(err.as_str().unwrap().contains(wanted), "{name:?}: {err}");
    }
    assert_eq!(call(&h.win, "text_presets_list", json!({})).unwrap(), json!([]));
}
