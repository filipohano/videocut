import { describe, expect, it } from "vitest";
import { buildExportSpec, exportedFrame } from "./spec";
import type { VideoState } from "../store";

function video(over: Partial<VideoState> = {}): VideoState {
  return {
    path: "/in/a.mov",
    info: { width: 1080, height: 1920, duration: 10, fps: 30, isImage: false, bitrate: 3_000_000, videoCodec: "h264", hasAudio: true, audioCodec: "aac" },
    playUrl: "asset://x",
    crop: { x: 0, y: 0, w: 1080, h: 1920 },
    aspect: "free",
    trimStart: 0,
    trimEnd: 10,
    watermarks: [],
    texts: [],
    fills: [],
    stills: null,
    quality: 60,
    imageFormat: "jpg",
    ...over,
  };
}

describe("buildExportSpec for photos", () => {
  const photo = () =>
    video({
      info: { width: 4000, height: 3000, duration: 0, fps: null, isImage: true, bitrate: null, videoCodec: "mjpeg", hasAudio: false, audioCodec: null },
      crop: { x: 0, y: 0, w: 2000, h: 3000 },
      trimEnd: 0,
      imageFormat: "png",
      quality: 90,
    });
  it("exports a single image in the chosen format, never trimmed", () => {
    const s = buildExportSpec(photo(), "/out/a.png");
    expect(s.imageFormat).toBe("png");
    expect(s.trimStart).toBeNull();
    expect(s.trimEnd).toBeNull();
    expect(s.crop).toEqual({ x: 0, y: 0, w: 2000, h: 3000 });
    expect(s.quality).toBe(90);
  });
  it("videos carry no image format", () => {
    expect(buildExportSpec(video(), "/o.mp4").imageFormat).toBeNull();
  });
});

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
      info: { width: 607, height: 495, duration: 9.7, fps: 30, isImage: false, bitrate: null, videoCodec: "h264", hasAudio: false, audioCodec: null },
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

describe("exportedFrame", () => {
  it("is the cropped area, which the text boxes are placed on", () => {
    expect(exportedFrame(buildExportSpec(video({ crop: { x: 100, y: 200, w: 500, h: 600 } }), "/out/a.mp4"))).toEqual({ w: 500, h: 600 });
  });
  it("is the whole picture when nothing is cropped", () => {
    expect(exportedFrame(buildExportSpec(video(), "/out/a.mp4"))).toEqual({ w: 1080, h: 1920 });
  });
  it("is the picture rounded down to even sizes when the source has odd ones", () => {
    const odd = video({ info: { ...video().info, width: 1081, height: 1921 }, crop: { x: 0, y: 0, w: 1081, h: 1921 } });
    expect(exportedFrame(buildExportSpec(odd, "/out/a.mp4"))).toEqual({ w: 1080, h: 1920 });
  });
});
