#!/usr/bin/env bash
# Downloads ffmpeg + ffprobe for the Windows (x64) build — the "sidecars" Tauri bundles in the installer.
#   https://github.com/BtbN/FFmpeg-Builds  (static GPL build: has libx264 and the NVIDIA / Intel / AMD GPU encoders)
# (yt-dlp is NOT bundled: the app downloads it itself on first launch.)
# The zip is verified against the publisher's SHA-256 checksum list.
#
# Runs in Git Bash on Windows (what GitHub Actions uses), or anywhere with
# FILLERNCUT_ANY_HOST=1 (to just fetch the files).
#
#   bash scripts/fetch-binaries-windows.sh           fetch if missing
#   bash scripts/fetch-binaries-windows.sh --force   re-download
set -euo pipefail
cd "$(dirname "$0")/.."

TRIPLE="x86_64-pc-windows-msvc"
DEST="src-tauri/binaries"
BASE="https://github.com/BtbN/FFmpeg-Builds/releases/download/latest"
FORCE=0
[[ "${1:-}" == "--force" ]] && FORCE=1

case "$(uname -s)" in
  MINGW*|MSYS*|CYGWIN*) ;;
  *) [[ "${FILLERNCUT_ANY_HOST:-}" == "1" ]] || { echo "These are Windows binaries; run this on Windows in Git Bash (or set FILLERNCUT_ANY_HOST=1 to just download them)." >&2; exit 1; } ;;
esac

sha256() { if command -v sha256sum >/dev/null; then sha256sum "$1" | cut -d' ' -f1; else shasum -a 256 "$1" | cut -d' ' -f1; fi; }

mkdir -p "$DEST"
if [[ $FORCE -eq 0 && -s "$DEST/ffmpeg-$TRIPLE.exe" && -s "$DEST/ffprobe-$TRIPLE.exe" ]]; then
  echo "Sidecars already in $DEST"; exit 0
fi

tmp="$(mktemp -d)"; trap 'rm -rf "$tmp"' EXIT

curl -fsSL --retry 3 -o "$tmp/checksums.sha256" "$BASE/checksums.sha256"
# The newest stable release branch, e.g. ffmpeg-n8.1-latest-win64-gpl-8.1.zip (not the "master" nightly).
line="$(grep -E ' ffmpeg-n[0-9.]+-latest-win64-gpl-[0-9.]+\.zip$' "$tmp/checksums.sha256" | sort -k2 -V | tail -n 1)"
[[ -n "$line" ]] || { echo "Couldn't find a Windows ffmpeg build in the checksum list" >&2; exit 1; }
expected="$(echo "$line" | cut -d' ' -f1)"
name="$(echo "$line" | awk '{print $2}')"
echo "ffmpeg build: $name"

curl -fSL --retry 3 -o "$tmp/ffmpeg.zip" "$BASE/$name"
actual="$(sha256 "$tmp/ffmpeg.zip")"
[[ "$actual" == "$expected" ]] || { echo "Checksum mismatch for $name: expected $expected, got $actual" >&2; exit 1; }

if command -v unzip >/dev/null; then
  unzip -o -q "$tmp/ffmpeg.zip" '*/bin/ffmpeg.exe' '*/bin/ffprobe.exe' -d "$tmp/x"
elif command -v 7z >/dev/null; then
  7z x -y -o"$tmp/x" "$tmp/ffmpeg.zip" '*/bin/ffmpeg.exe' '*/bin/ffprobe.exe' >/dev/null
else
  powershell -NoProfile -Command "Expand-Archive -Force '$(cygpath -w "$tmp/ffmpeg.zip" 2>/dev/null || echo "$tmp/ffmpeg.zip")' '$(cygpath -w "$tmp/x" 2>/dev/null || echo "$tmp/x")'"
fi
for tool in ffmpeg ffprobe; do
  src="$(find "$tmp/x" -name "$tool.exe" -path '*/bin/*' | head -n 1)"
  [[ -n "$src" ]] || { echo "$tool.exe wasn't in the download" >&2; exit 1; }
  install -m 755 "$src" "$DEST/$tool-$TRIPLE.exe"
done
echo "$name" > "$DEST/ffmpeg-windows.version"

# Sanity check: a Windows (PE) executable.
if command -v file >/dev/null; then
  for tool in ffmpeg ffprobe; do
    file "$DEST/$tool-$TRIPLE.exe" | grep -q "PE32+" || { echo "$tool is not a 64-bit Windows executable" >&2; exit 1; }
  done
fi
echo "Sidecars ready in $DEST:"
ls -lh "$DEST" | grep windows
