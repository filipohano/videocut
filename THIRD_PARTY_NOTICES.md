# Third-party software

FillernCut's own code is MIT-licensed. The released app also ships these programs, which it starts as separate
processes (they are not linked into the app):

| Program | Used for | License | Source |
| --- | --- | --- | --- |
| **ffmpeg** and **ffprobe** | decoding, cropping, watermarking, encoding, probing | GPL v3 build (includes libx264 and others) — static macOS arm64 build by Martin Riedl | Binaries: <https://ffmpeg.martin-riedl.de> · Source: <https://ffmpeg.org/download.html> and the build scripts linked from the binary site |
| **yt-dlp** | downloading from Instagram, X and TikTok (downloaded by the app on first launch, not bundled) | The Unlicense | <https://github.com/yt-dlp/yt-dlp> |

`scripts/fetch-binaries.sh` downloads the ffmpeg builds (verifying SHA-256 checksums) at build time; none are
stored in this repository.

The app is built on [Tauri](https://tauri.app) (MIT / Apache-2.0) and the Rust crates and npm packages listed in
`Cargo.lock` and `package-lock.json`.
