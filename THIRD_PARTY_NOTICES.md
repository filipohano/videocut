# Third-party software

FillernCut's own code is MIT-licensed. The released app also ships these programs, which it starts as separate
processes (they are not linked into the app):

| Program | Used for | License | Source |
| --- | --- | --- | --- |
| **ffmpeg** and **ffprobe** | decoding, cropping, watermarking, encoding, probing | GPL v3 build (includes libx264 and others) — static macOS arm64 build by Martin Riedl; static Windows x64 build by BtbN (adds NVENC / Quick Sync / AMF support) | Binaries: <https://ffmpeg.martin-riedl.de> and <https://github.com/BtbN/FFmpeg-Builds> · Source: <https://ffmpeg.org/download.html> and the build scripts linked from the binary sites |
| **yt-dlp** | downloading from Instagram, X and TikTok (downloaded by the app on first launch, not bundled) | The Unlicense | <https://github.com/yt-dlp/yt-dlp> |

`scripts/fetch-binaries.sh` (macOS) and `scripts/fetch-binaries-windows.sh` (Windows) download the ffmpeg builds (verifying SHA-256 checksums) at build time; none are
stored in this repository.

## Font

The app bundles one font, used by the **Text** section of the editor:

| Font | License | Source |
| --- | --- | --- |
| **TikTok Sans** (Regular and Bold) — Copyright 2024 TikTok Inc. | SIL Open Font License 1.1 (full text in [`src/assets/fonts/OFL.txt`](src/assets/fonts/OFL.txt)) | <https://fonts.google.com/specimen/TikTok+Sans> |

The font files are converted to WOFF2 for the app; the OFL allows that and allows bundling the font with software.

## Downloadable add-on

**Remove text (AI)** is not part of the app: it is downloaded by the user in Settings → Add-ons, from
<https://huggingface.co/andraniksargsyan/migan> (a fixed revision, checked against a SHA-256 written into the app).

| Model | License | Source |
| --- | --- | --- |
| **MI-GAN** (`migan.onnx`) — Copyright (c) 2024 Picsart AI Research (PAIR) | MIT | <https://github.com/Picsart-AI-Research/MI-GAN> |

The model was trained on the Places2 dataset; check that dataset's terms before using the add-on commercially.
The app runs it with [tract](https://github.com/sonos/tract) (MIT / Apache-2.0), which is built into the app.

The app is built on [Tauri](https://tauri.app) (MIT / Apache-2.0) and the Rust crates and npm packages listed in
`Cargo.lock` and `package-lock.json`.
