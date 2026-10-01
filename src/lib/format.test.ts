import { describe, expect, it } from "vitest";
import { basename, formatClock, formatSeconds, parseSeconds, stem } from "./format";

describe("formatClock", () => {
  it("formats minutes and hours", () => {
    expect(formatClock(0)).toBe("0:00");
    expect(formatClock(9.7)).toBe("0:09");
    expect(formatClock(83)).toBe("1:23");
    expect(formatClock(3725)).toBe("1:02:05");
    expect(formatClock(NaN)).toBe("0:00");
  });
});

describe("parseSeconds", () => {
  it("accepts dot and comma decimals", () => {
    expect(parseSeconds("9,7")).toBe(9.7);
    expect(parseSeconds("9.7")).toBe(9.7);
    expect(parseSeconds(" 12 ")).toBe(12);
    expect(parseSeconds(".5")).toBe(0.5);
  });
  it("accepts clock notation", () => {
    expect(parseSeconds("1:23")).toBe(83);
    expect(parseSeconds("1:02:05.5")).toBe(3725.5);
  });
  it("rejects garbage", () => {
    for (const bad of ["", "abc", "1.2.3", "1:2:3:4", "-1:00", "--1"]) expect(parseSeconds(bad)).toBeNaN();
  });
});

describe("formatSeconds", () => {
  it("rounds to two decimals", () => {
    expect(formatSeconds(9.7, "en-US")).toBe("9.7");
    expect(formatSeconds(9.7, "nb-NO")).toBe("9,7");
    expect(formatSeconds(1.23456, "en-US")).toBe("1.23");
  });
});

describe("paths", () => {
  it("splits names", () => {
    expect(basename("/a/b/clip.final.mp4")).toBe("clip.final.mp4");
    expect(stem("/a/b/clip.final.mp4")).toBe("clip.final");
    expect(stem("noext")).toBe("noext");
  });
});
