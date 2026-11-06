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

# extra clips so the batch dialog has something to chew on
cp public/dev/sample.webm public/dev/sample2.webm
cp public/dev/sample.webm public/dev/sample3.webm
# sample photos
ffmpeg -hide_banner -loglevel error -y -f lavfi -i "testsrc2=size=1200x800:rate=1:duration=1" -frames:v 1 public/dev/photo.jpg
ffmpeg -hide_banner -loglevel error -y -f lavfi -i "testsrc=size=900x1200:rate=1:duration=1" -frames:v 1 public/dev/photo.png
# A photo post as the app builds it: three photos with captions, two seconds each (Remove text)
ffmpeg -hide_banner -loglevel error -y \
  -f lavfi -i "testsrc2=size=540x960:rate=30:duration=2" \
  -f lavfi -i "smptebars=size=540x960:rate=30:duration=2" \
  -f lavfi -i "color=c=0x406080:size=540x960:rate=30:duration=2" \
  -f lavfi -i "sine=frequency=330:duration=6" \
  -filter_complex "[0:v]trim=end_frame=1,loop=59:1,setpts=N/30/TB,drawbox=x=90:y=640:w=360:h=60:color=white:t=fill[a];[1:v]trim=end_frame=1,loop=59:1,setpts=N/30/TB,drawbox=x=60:y=300:w=420:h=60:color=white:t=fill[b];[2:v]drawbox=x=120:y=700:w=300:h=50:color=white:t=fill[c];[a][b][c]concat=n=3:v=1:a=0[v]" \
  -map "[v]" -map 3:a -c:v libx264 -pix_fmt yuv420p -c:a aac public/dev/slides.mp4
# What the mock returns as a "filled" area
ffmpeg -hide_banner -loglevel error -y -f lavfi -i "color=c=0xd96b8a@0.85:s=64x32,format=rgba" -frames:v 1 public/dev/fill-patch.png
echo "Dev assets written to public/dev/"
