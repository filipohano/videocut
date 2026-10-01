import { describe, expect, it } from "vitest";
import { clampRect, evenRect, fitRatio, fullRect, moveRect, ratioValue, resizeRect } from "./crop";

const B = { w: 1000, h: 500 };

describe("ratioValue", () => {
  it("maps keys to numbers", () => {
    expect(ratioValue("free", B)).toBeNull();
    expect(ratioValue("original", B)).toBe(2);
    expect(ratioValue("9:16", B)).toBeCloseTo(0.5625);
    expect(ratioValue("bogus", B)).toBeNull();
  });
});

describe("moveRect / clampRect", () => {
  it("never leaves the frame", () => {
    const r = { x: 100, y: 100, w: 300, h: 200 };
    expect(moveRect(r, -500, -500, B)).toEqual({ ...r, x: 0, y: 0 });
    expect(moveRect(r, 5000, 5000, B)).toEqual({ ...r, x: 700, y: 300 });
  });
  it("clamps oversized and tiny rects", () => {
    expect(clampRect({ x: -5, y: -5, w: 5000, h: 5000 }, B)).toEqual(fullRect(B));
    expect(clampRect({ x: 0, y: 0, w: 1, h: 1 }, B)).toMatchObject({ w: 16, h: 16 });
  });
});

describe("resizeRect (free)", () => {
  const r = { x: 100, y: 100, w: 400, h: 200 };
  it("moves only the dragged edges", () => {
    expect(resizeRect(r, "e", 50, 999, B, null)).toEqual({ x: 100, y: 100, w: 450, h: 200 });
    expect(resizeRect(r, "nw", -50, -30, B, null)).toEqual({ x: 50, y: 70, w: 450, h: 230 });
  });
  it("stops at the frame and at the minimum size", () => {
    expect(resizeRect(r, "se", 9999, 9999, B, null)).toEqual({ x: 100, y: 100, w: 900, h: 400 });
    expect(resizeRect(r, "e", -9999, 0, B, null).w).toBe(16);
    expect(resizeRect(r, "w", 9999, 0, B, null)).toMatchObject({ x: 484, w: 16 });
  });
});

describe("resizeRect (locked ratio)", () => {
  const r = { x: 100, y: 100, w: 400, h: 225 }; // 16:9
  const ratio = 16 / 9;
  const near = (a: { w: number; h: number }) => Math.abs(a.w / a.h - ratio) < 0.02;

  it("corner drags keep the ratio and pin the opposite corner", () => {
    const out = resizeRect(r, "se", 80, 10, B, ratio);
    expect(near(out)).toBe(true);
    expect(out.x).toBe(100);
    expect(out.y).toBe(100);
    expect(out.w).toBeGreaterThan(400);
    const nw = resizeRect(r, "nw", -40, -40, B, ratio);
    expect(near(nw)).toBe(true);
    expect(nw.x + nw.w).toBe(500);
    expect(nw.y + nw.h).toBe(325);
  });

  it("corner drags can't escape the frame", () => {
    const out = resizeRect(r, "se", 9999, 9999, B, ratio);
    expect(out.x + out.w).toBeLessThanOrEqual(B.w);
    expect(out.y + out.h).toBeLessThanOrEqual(B.h);
    expect(near(out)).toBe(true);
  });

  it("edge drags resize around the centre line", () => {
    const out = resizeRect(r, "e", 100, 0, B, ratio);
    expect(near(out)).toBe(true);
    expect(out.x).toBe(100);
    expect(out.w).toBe(500);
    expect(out.y + out.h / 2).toBeCloseTo(100 + 225 / 2, 0);
    const top = resizeRect(r, "n", -20, -20, B, ratio);
    expect(near(top)).toBe(true);
    expect(top.y + top.h).toBe(325);
  });

  it("works for portrait ratios in a portrait frame", () => {
    const p = { w: 1080, h: 1920 };
    const out = resizeRect({ x: 100, y: 100, w: 540, h: 960 }, "se", 200, 0, p, 9 / 16);
    expect(out.w / out.h).toBeCloseTo(9 / 16, 1);
    expect(out.x + out.w).toBeLessThanOrEqual(p.w);
  });
});

describe("fitRatio", () => {
  it("centres the largest matching rectangle in the current one", () => {
    expect(fitRatio({ x: 0, y: 0, w: 1000, h: 500 }, 1, B)).toEqual({ x: 250, y: 0, w: 500, h: 500 });
    expect(fitRatio({ x: 0, y: 0, w: 400, h: 500 }, 2, B)).toEqual({ x: 0, y: 150, w: 400, h: 200 });
  });
});

describe("evenRect", () => {
  it("rounds down to even numbers like the exporter", () => {
    expect(evenRect({ x: 11, y: 5, w: 301, h: 201 })).toEqual({ x: 10, y: 4, w: 300, h: 200 });
    expect(evenRect({ x: 0, y: 0, w: 1, h: 1 })).toEqual({ x: 0, y: 0, w: 2, h: 2 });
  });
});
