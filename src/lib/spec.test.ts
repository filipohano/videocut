import { describe, expect, it } from "vitest";
import { buildExportSpec } from "./spec";
import type { VideoState } from "../store";

function video(over: Partial<VideoState> = {}): VideoState {
  return {
    path: "/in/a.mov",
    info: { width: 1080, height: 1920, duration: 10, fps: 30, bitrate: 3_000_000, videoCodec: "h264", hasAudio: true, audioCodec: "aac" },
    playUrl: "asset://x",
    crop: { x: 0, y: 0, w: 1080, h: 1920 },
    aspect: "free",
    trimStart: 0,
    trimEnd: 10,
    watermarks: [],
    quality: 60,
    ...over,
  };
}

describe("buildExportSpec", () => {
  it("omits crop and trim when untouched", () => {
    const s = buildExportSpec(video(), "/out/a.mp4");
    expect(s.crop).toBeNull();
    expect(s.trimStart).toBeNull();
    expect(s.trimEnd).toBeNull();
    expect(s.output).toBe("/out/a.mp4");
    expect(s.sourceWidth).toBe(1080);
  });

  it("includes crop (even-aligned) and trim when changed", () => {
    const s = buildExportSpec(video({ crop: { x: 11, y: 5, w: 501, h: 301 }, trimStart: 1.5, trimEnd: 6 }), "/o.mp4");
    expect(s.crop).toEqual({ x: 10, y: 4, w: 500, h: 300 });
    expect(s.trimStart).toBe(1.5);
    expect(s.trimEnd).toBe(6);
  });

  it("treats an odd-sized full frame as untouched", () => {
    const v = video({
      info: { width: 607, height: 495, duration: 9.7, fps: 30, bitrate: null, videoCodec: "h264", hasAudio: false, audioCodec: null },
      crop: { x: 0, y: 0, w: 606, h: 494 },
      trimEnd: 9.7,
    });
    expect(buildExportSpec(v, "/o.mp4").crop).toBeNull();
  });

  it("maps watermarks to plain placements", () => {
    const s = buildExportSpec(
      video({
        watermarks: [
          { id: "1", name: "logo", fileName: "1.png", aspect: 0.5, content: { l: 0.1, t: 0, r: 0.9, b: 1 }, text: null, nx: 0.7, ny: 0.8, scale: 0.2, opacity: 0.6, path: "/lib/1.png", url: "asset://1" },
        ],
      }),
      "/o.mp4",
    );
    expect(s.watermarks).toEqual([{ path: "/lib/1.png", nx: 0.7, ny: 0.8, scale: 0.2, opacity: 0.6, content: { l: 0.1, t: 0, r: 0.9, b: 1 } }]);
    expect(s.sourceBitrate).toBe(3_000_000);
    expect(s.fps).toBe(30);
  });
});
