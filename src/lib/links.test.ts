import { describe, expect, it } from "vitest";
import { detectPlatform, extensionOf, IMAGE_EXTENSIONS, VIDEO_EXTENSIONS, isMediaPath, isPhotoPath, isVideoPath } from "./links";

describe("detectPlatform", () => {
  it("recognises the three platforms", () => {
    expect(detectPlatform("https://www.tiktok.com/@u/video/1234567890123456789")).toBe("TikTok");
    expect(detectPlatform("https://vt.tiktok.com/ZSabc/")).toBe("TikTok");
    expect(detectPlatform("7345678901234567890")).toBe("TikTok");
    expect(detectPlatform("instagram.com/reel/abc")).toBe("Instagram");
    expect(detectPlatform("https://x.com/jack/status/20")).toBe("X / Twitter");
    expect(detectPlatform("https://mobile.twitter.com/jack/status/20")).toBe("X / Twitter");
  });
  it("rejects everything else", () => {
    expect(detectPlatform("https://youtube.com/watch?v=1")).toBeNull();
    expect(detectPlatform("hello")).toBeNull();
    expect(detectPlatform("")).toBeNull();
    expect(detectPlatform("https://nottiktok.com/x")).toBeNull();
  });
});

describe("extensions", () => {
  it("splits and lowercases", () => {
    expect(extensionOf("/a/B.MOV")).toBe("mov");
    expect(extensionOf("noext")).toBe("");
    expect(VIDEO_EXTENSIONS).toContain("mp4");
    expect(IMAGE_EXTENSIONS).toContain("png");
  });
});

describe("media kinds", () => {
  it("tells photos from videos", () => {
    expect(isPhotoPath("/a/B.JPG")).toBe(true);
    expect(isPhotoPath("/a/b.jpeg")).toBe(true);
    expect(isPhotoPath("/a/b.png")).toBe(true);
    expect(isPhotoPath("/a/b.gif")).toBe(false);
    expect(isVideoPath("/a/b.mov")).toBe(true);
    expect(isMediaPath("/a/b.txt")).toBe(false);
    expect(isMediaPath("/a/b.webp")).toBe(true);
  });
});
