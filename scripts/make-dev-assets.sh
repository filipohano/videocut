#!/usr/bin/env bash
# Generates sample media for the browser-only dev mock (src/dev/mockTauri.ts).
# Output goes to public/dev/ which is git-ignored.
set -euo pipefail
cd "$(dirname "$0")/.."
mkdir -p public/dev
ffmpeg -hide_banner -loglevel error -y \
  -f lavfi -i "testsrc2=size=608x496:rate=30:duration=9.7" \
  -f lavfi -i "sine=frequency=440:duration=9.7" \
  -c:v libvpx-vp9 -b:v 1M -pix_fmt yuv420p -c:a libopus -shortest public/dev/sample.webm
ffmpeg -hide_banner -loglevel error -y -f lavfi -i "color=c=0x5b8def:s=420x147,drawtext=text='BRAND':fontcolor=white:fontsize=84:x=(w-text_w)/2:y=(h-text_h)/2" -frames:v 1 public/dev/logo-a.png
ffmpeg -hide_banner -loglevel error -y -f lavfi -i "color=c=0xf0b24a:s=500x100,drawtext=text='@filippohano':fontcolor=black:fontsize=52:x=(w-text_w)/2:y=(h-text_h)/2" -frames:v 1 public/dev/logo-b.png

# A logo with lots of transparent margin (visible part is the middle 40% x 40%)
ffmpeg -hide_banner -loglevel error -y -f lavfi -i "color=c=0xe0457b:s=320x160,format=rgba,drawtext=text='PADDED':fontcolor=white:fontsize=60:x=(w-text_w)/2:y=(h-text_h)/2,pad=800:400:240:120:color=black@0" -frames:v 1 public/dev/logo-padded.png
echo "Dev assets written to public/dev/"
