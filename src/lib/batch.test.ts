import { describe, expect, it } from "vitest";
import type { MediaInfo } from "../api";
import type { VideoState } from "../store";
import { DEFAULT_BATCH_OPTIONS, buildBatchSpec } from "./batch";

const info = (w: number, h: number): MediaInfo => ({
  width: w, height: h, duration: 12, fps: 30, bitrate: 2_000_000, videoCodec: "h264", hasAudio: true, audioCodec: "aac",
});

function template(over: Partial<VideoState> = {}): VideoState {
  return {
    path: "/in/a.mp4",
    info: info(1000, 500),
    playUrl: "x",
    crop: { x: 100, y: 50, w: 500, h: 250 }, // 10%,10%, 50% x 50%
    aspect: "free",
    trimStart: 1,
    trimEnd: 4,
    watermarks: [
      { id: "1", name: "logo", fileName: "1.png", aspect: 0.5, content: { l: 0.1, t: 0, r: 0.9, b: 1 }, text: null, nx: 0.7, ny: 0.8, scale: 0.2, opacity: 0.6, path: "/lib/1.png", url: "u" },
    ],
    quality: 55,
    ...over,
  };
}

describe("buildBatchSpec", () => {
  it("maps the crop to the same relative area on a different resolution", () => {
    const s = buildBatchSpec(template(), info(2000, 1000), "/in/b.mp4", "/out/b.mp4", DEFAULT_BATCH_OPTIONS);
    expect(s.crop).toEqual({ x: 200, y: 100, w: 1000, h: 500 });
    expect(s.sourceWidth).toBe(2000);
    expect(s.input).toBe("/in/b.mp4");
    expect(s.output).toBe("/out/b.mp4");
  });

  it("copies the watermarks as-is (they are relative to the cropped frame) and the quality", () => {
    const s = buildBatchSpec(template(), info(2000, 1000), "/i", "/o", DEFAULT_BATCH_OPTIONS);
    expect(s.watermarks).toEqual([{ path: "/lib/1.png", nx: 0.7, ny: 0.8, scale: 0.2, opacity: 0.6, content: { l: 0.1, t: 0, r: 0.9, b: 1 } }]);
    expect(s.quality).toBe(55);
  });

  it("never copies the trim, and uses each video's own bitrate and audio", () => {
    const own = { ...info(640, 360), bitrate: 900_000, hasAudio: false, audioCodec: null };
    const s = buildBatchSpec(template(), own, "/i", "/o", DEFAULT_BATCH_OPTIONS);
    expect([s.trimStart, s.trimEnd]).toEqual([null, null]);
    expect(s.sourceBitrate).toBe(900_000);
    expect(s.hasAudio).toBe(false);
  });

  it("can skip the crop or the watermarks", () => {
    const s = buildBatchSpec(template(), info(1000, 500), "/i", "/o", { applyCrop: false, applyWatermarks: false });
    expect(s.crop).toBeNull();
    expect(s.watermarks).toEqual([]);
  });

  it("an untouched crop stays untouched", () => {
    const t = template({ crop: { x: 0, y: 0, w: 1000, h: 500 } });
    expect(buildBatchSpec(t, info(800, 800), "/i", "/o", DEFAULT_BATCH_OPTIONS).crop).toBeNull();
  });

  it("rounding can't push the crop outside a small or odd-sized video", () => {
    const t = template({ crop: { x: 900, y: 450, w: 100, h: 50 } }); // bottom-right corner
    const s = buildBatchSpec(t, info(333, 111), "/i", "/o", DEFAULT_BATCH_OPTIONS);
    expect(s.crop!.x + s.crop!.w).toBeLessThanOrEqual(332);
    expect(s.crop!.y + s.crop!.h).toBeLessThanOrEqual(110);
    expect(s.crop!.w).toBeGreaterThanOrEqual(2);
  });
});
