#!/usr/bin/env bash
# Downloads the programs FillernCut bundles inside the app (Tauri "sidecars"):
#   ffmpeg + ffprobe  – https://ffmpeg.martin-riedl.de (static macOS arm64 build)
#   yt-dlp            – https://github.com/yt-dlp/yt-dlp (official standalone macOS build)
# Every download is verified against the publisher's SHA-256 checksum.
#
#   npm run setup            fetch if missing
#   npm run setup -- --force re-download everything
#
# Files land in src-tauri/binaries/ named <tool>-<target-triple>, which is what Tauri expects.
set -euo pipefail
cd "$(dirname "$0")/.."

TRIPLE="aarch64-apple-darwin"
DEST="src-tauri/binaries"
FORCE=0
[[ "${1:-}" == "--force" ]] && FORCE=1

if [[ "$(uname -s)" != "Darwin" || "$(uname -m)" != "arm64" ]] && [[ "${FILLERNCUT_ANY_HOST:-}" != "1" ]]; then
  echo "These are macOS Apple-silicon binaries; run this on an M-series Mac (or set FILLERNCUT_ANY_HOST=1 to just download them)." >&2
  exit 1
fi

sha256() { if command -v shasum >/dev/null; then shasum -a 256 "$1" | cut -d' ' -f1; else sha256sum "$1" | cut -d' ' -f1; fi; }

verify() { # file expected-hash label
  local actual; actual="$(sha256 "$1")"
  if [[ "$actual" != "$2" ]]; then
    echo "Checksum mismatch for $3: expected $2, got $actual" >&2
    exit 1
  fi
}

mkdir -p "$DEST"
tmp="$(mktemp -d)"; trap 'rm -rf "$tmp"' EXIT

need() { [[ $FORCE -eq 1 || ! -x "$DEST/$1-$TRIPLE" ]]; }

# ───────── ffmpeg + ffprobe (same build, so versions always match) ─────────
if need ffmpeg || need ffprobe; then
  BASE="https://ffmpeg.martin-riedl.de"
  # The "latest release" URL redirects to a versioned directory; pin to it so both zips come from one build.
  versioned="$(curl -fsS -o /dev/null -w '%{redirect_url}' "$BASE/redirect/latest/macos/arm64/release/ffmpeg.zip")"
  dir="${versioned%/ffmpeg.zip}"
  [[ -n "$dir" && "$dir" != "$versioned" ]] || { echo "Couldn't resolve the latest ffmpeg build" >&2; exit 1; }
  echo "ffmpeg build: $dir"
  for tool in ffmpeg ffprobe; do
    curl -fSL --retry 3 -o "$tmp/$tool.zip" "$dir/$tool.zip"
    expected="$(curl -fsSL "$dir/$tool.zip.sha256" | cut -d' ' -f1)"
    verify "$tmp/$tool.zip" "$expected" "$tool.zip"
    unzip -o -q "$tmp/$tool.zip" -d "$tmp/$tool"
    install -m 755 "$tmp/$tool/$tool" "$DEST/$tool-$TRIPLE"
  done
  echo "${dir##*/macos/arm64/}" > "$DEST/ffmpeg.version"
fi

# ───────── yt-dlp ─────────
if need yt-dlp; then
  REL="https://github.com/yt-dlp/yt-dlp/releases/latest/download"
  curl -fSL --retry 3 -o "$tmp/yt-dlp_macos" "$REL/yt-dlp_macos"
  curl -fsSL --retry 3 -o "$tmp/SHA2-256SUMS" "$REL/SHA2-256SUMS"
  expected="$(grep -E ' yt-dlp_macos$' "$tmp/SHA2-256SUMS" | cut -d' ' -f1)"
  verify "$tmp/yt-dlp_macos" "$expected" "yt-dlp_macos"
  install -m 755 "$tmp/yt-dlp_macos" "$DEST/yt-dlp-$TRIPLE"
fi

# Sanity check: the right architecture.
if command -v file >/dev/null; then
  for tool in ffmpeg ffprobe; do
    file "$DEST/$tool-$TRIPLE" | grep -q arm64 || { echo "$tool is not an arm64 binary" >&2; exit 1; }
  done
fi

echo "Sidecars ready in $DEST:"
ls -lh "$DEST" | tail -n +2
