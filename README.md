# FillernCut

Download, crop, trim and watermark videos on your Mac — everything runs locally, powered by ffmpeg.

- **Downloader** for **TikTok**, **Instagram** and **X/Twitter** links. TikTok comes in source quality
  *without* the watermark (full links, `vt.tiktok.com` share links, or just the 19-digit post id), and TikTok
  **photo posts** are built into a 1080×1920 video with their sound.
- **Editor** with a large live preview: drag-to-crop with aspect-ratio presets, trim in/out, and an exact
  "what you see is what you get" export.
- **Watermark library.** Upload a logo once, or **add text** (any installed font, bold/italic, colour, outline,
  shadow), and it's in the **Add watermark ▾** drop-down every time you open the app. Use several at once; each
  remembers its own position, size and opacity. Drag them anywhere in the preview, even so a PNG's transparent
  margin hangs outside the picture. The visible part always stays inside the frame.
- **Tidy folders.** Downloads land in `~/Movies/FillernCut/Footage`, finished exports in
  `~/Movies/FillernCut/Finished`, all named by date and time (`2026-10-01_15-42-07.mp4`) so they sort
  chronologically. Both folders, and "ask where to save", are in Settings.
- **Sensible file sizes.** The export aims for the original's bitrate (adjusted for crop and the quality slider)
  instead of a fixed high quality, and shows the expected size before you export.
- **Preview volume** slider in the player bar (only affects what you hear in the app, never the export).
- **Fast export**: H.264 on the Apple-silicon media engine (VideoToolbox) with a quality slider, and an
  automatic CPU fallback if the hardware encoder ever refuses a file.
- **Updates from GitHub**: checks on every launch, shows an *out of date* banner, and can install new versions
  by itself (Settings → Updates).

Requires an Apple-silicon Mac (M1 or newer), macOS 12+.

## Install

1. Download `FillernCut_<version>_aarch64.dmg` from the [latest release](../../releases/latest) and drag the
   app to *Applications*.
2. The app isn't notarized by Apple, so the first launch needs **right-click → Open** (or
   `xattr -dr com.apple.quarantine /Applications/FillernCut.app`). Updates installed by the app itself don't
   show this prompt again.

## Updates

On every launch FillernCut asks GitHub whether a newer release exists. Settings → Updates picks what happens:

| Mode | Behaviour |
| --- | --- |
| **Install automatically at launch** (default) | Downloads, installs and restarts into the new version. |
| **Tell me, and let me decide** | Shows an "out of date" banner and a header badge with an **Update now** button. |
| **Only when I check** | No check at launch; use **Check now**. |

Updates are signature-checked against the key built into the app, so only builds signed by this project install.

The downloader (**yt-dlp**) isn't part of the app: it is downloaded (checksum-verified) on first launch and
refreshed in the background at launch (toggle in Settings → Downloads), because Instagram/X/TikTok break it often.

## Downloads: logins and limits

- **Instagram** (and some X posts) need you to be logged in. In *Settings → Downloads* choose the browser you're
  logged into; yt-dlp reads its cookies locally and nothing is sent anywhere else. Safari additionally needs
  *Full Disk Access* for FillernCut (System Settings → Privacy & Security).
- **TikTok** uses a third-party watermark-free API first (original quality, photo posts) and falls back to yt-dlp
  for plain videos. If that service is down, ordinary videos still work through yt-dlp.
- Only download content you have the right to use.

---

## Development

Prerequisites: an Apple-silicon Mac with Xcode command-line tools, [Node 22](https://nodejs.org) and
[Rust](https://rustup.rs).

```bash
npm install
npm run setup          # downloads ffmpeg and ffprobe (checksum-verified) into src-tauri/binaries/
npm run tauri dev      # runs the real app with hot reload
```

UI work without Rust or ffmpeg — a fake backend serves sample data in a normal browser:

```bash
npm run dev:assets     # needs ffmpeg once; writes sample media to public/dev/ (git-ignored)
npm run dev            # open http://localhost:1420   (?update=1 simulates a newer release, ?mode=notify|auto|manual)
```

### Tests

```bash
npm test               # crop / watermark / spec maths (Vitest)
npm run typecheck
cargo test --workspace # Rust; needs ffmpeg + ffprobe on PATH (those tests skip themselves otherwise)
```

The Rust tests include real-ffmpeg checks of the filter graph (down to sampling pixels to verify a watermark lands
where it should), the full command layer driven through Tauri's IPC with the exact JSON the frontend sends, and
the download flows against a local stand-in for the TikTok API and a fake `yt-dlp`.

### Layout

```
crates/fillerncut-core/   pure logic: link parsing, ffmpeg argument building, watermark library, settings
src-tauri/                Tauri app: commands (download, export, preview, library, settings), updater wiring
src/                      TypeScript UI (no framework): stage + crop overlay, panels, settings, updater UI
scripts/                  fetch-binaries, setup-updater, version, dev assets
.github/workflows/        ci.yml (checks) and release.yml (build + publish)
```

Edits are described in source-pixel / fraction-of-crop terms (`src/lib/`), converted to an ffmpeg filter graph by
`crates/fillerncut-core/src/export.rs`, so the preview and the export can't drift apart.

## Releasing

One-time setup (creates the update-signing key, puts the public key into `tauri.conf.json`, stores the private
key as GitHub secrets):

```bash
bash scripts/setup-updater.sh
git commit -am "Add updater public key" && git push
```

Keep the private key (`~/.tauri/fillerncut.key`) safe — without it installed apps can't be updated.

For every release:

```bash
node scripts/version.mjs 0.2.0     # bumps package.json, tauri.conf.json and both Cargo.toml files
cargo update -w                    # refresh Cargo.lock
git commit -am "Release v0.2.0" && git tag v0.2.0 && git push && git push --tags
```

The **Release** workflow builds the signed app on a macOS runner, publishes the `.dmg`, and uploads
`latest.json` — the file installed apps check at launch.

## Licensing

FillernCut is MIT-licensed (see [LICENSE](LICENSE)). It bundles ffmpeg/ffprobe (a GPL build) and yt-dlp as
separate programs it runs as subprocesses — see [THIRD_PARTY_NOTICES.md](THIRD_PARTY_NOTICES.md).
