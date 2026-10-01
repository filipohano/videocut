import { describe, expect, it } from "vitest";
import {
  FULL_CONTENT,
  clampPlacement,
  cornerPlacement,
  heightFraction,
  maxSizePct,
  rescaleAroundCenter,
  rescaleKeepingCorner,
  scaleFromSizePct,
  sizePctFromScale,
} from "./watermarks";

const FRAME = { w: 1000, h: 500 };
// A logo with 20% transparent margin on the left, 10% on the right, 25% top and bottom.
const PADDED = { l: 0.2, t: 0.25, r: 0.9, b: 0.75 };

describe("heightFraction", () => {
  it("accounts for the frame's aspect ratio", () => {
    expect(heightFraction(0.2, 1, FRAME)).toBeCloseTo(0.4);
  });
});

describe("clampPlacement without margins", () => {
  it("keeps the whole image inside", () => {
    const p = clampPlacement({ nx: 0.95, ny: 0.95, scale: 0.2 }, 0.5, FRAME);
    expect(p.nx).toBeCloseTo(0.8);
    expect(p.ny).toBeCloseTo(1 - heightFraction(0.2, 0.5, FRAME));
    expect(clampPlacement({ nx: -3, ny: -3, scale: 0.2 }, 0.5, FRAME)).toMatchObject({ nx: 0, ny: 0 });
  });
});

describe("clampPlacement with transparent margins", () => {
  it("lets the margin hang outside but never the visible content", () => {
    const s = 0.4;
    const left = clampPlacement({ nx: -5, ny: 0.3, scale: s }, 0.5, FRAME, PADDED);
    expect(left.nx).toBeCloseTo(-0.2 * s); // content's left edge exactly at the frame edge
    const right = clampPlacement({ nx: 5, ny: 0.3, scale: s }, 0.5, FRAME, PADDED);
    expect(right.nx + 0.9 * s).toBeCloseTo(1); // content's right edge exactly at the frame edge
    const h = heightFraction(s, 0.5, FRAME);
    const up = clampPlacement({ nx: 0.3, ny: -5, scale: s }, 0.5, FRAME, PADDED);
    expect(up.ny).toBeCloseTo(-0.25 * h);
    const down = clampPlacement({ nx: 0.3, ny: 5, scale: s }, 0.5, FRAME, PADDED);
    expect(down.ny + 0.75 * h).toBeCloseTo(1);
  });

  it("can never be placed entirely outside the frame", () => {
    for (const [nx, ny] of [[-100, -100], [100, 100], [-100, 100], [100, -100]]) {
      const p = clampPlacement({ nx, ny, scale: 0.3 }, 0.5, FRAME, PADDED);
      const h = heightFraction(p.scale, 0.5, FRAME);
      const contentLeft = p.nx + PADDED.l * p.scale;
      const contentRight = p.nx + PADDED.r * p.scale;
      const contentTop = p.ny + PADDED.t * h;
      const contentBottom = p.ny + PADDED.b * h;
      expect(contentLeft).toBeGreaterThanOrEqual(-1e-9);
      expect(contentRight).toBeLessThanOrEqual(1 + 1e-9);
      expect(contentTop).toBeGreaterThanOrEqual(-1e-9);
      expect(contentBottom).toBeLessThanOrEqual(1 + 1e-9);
    }
  });

  it("centres content that is bigger than the frame instead of failing", () => {
    // Square logo at 100% width in a 2:1 frame is twice as tall as the frame.
    const p = clampPlacement({ nx: 0, ny: 0.9, scale: 1 }, 1, FRAME);
    expect(p.ny).toBeCloseTo(-0.5);
  });

  it("survives garbage numbers", () => {
    const p = clampPlacement({ nx: NaN, ny: Infinity, scale: NaN }, 0.5, FRAME, PADDED);
    expect(Number.isFinite(p.nx) && Number.isFinite(p.ny) && Number.isFinite(p.scale)).toBe(true);
  });
});

describe("size slider", () => {
  it("is the visible content's width as a percentage of the frame", () => {
    expect(scaleFromSizePct(50, FULL_CONTENT)).toBeCloseTo(0.5);
    // content is 70% of the image, so the image must be larger to get 35% content width
    expect(scaleFromSizePct(35, PADDED)).toBeCloseTo(0.5);
    expect(sizePctFromScale(0.5, PADDED)).toBe(35);
  });
  it("always reaches 100% for reasonably sized content, whatever the video resolution", () => {
    expect(maxSizePct(FULL_CONTENT)).toBe(100);
    expect(maxSizePct(PADDED)).toBe(100);
    expect(scaleFromSizePct(100, PADDED)).toBeLessThanOrEqual(10);
    // tiny content in a huge image can't be blown up past the exporter's limit
    expect(maxSizePct({ l: 0.45, t: 0, r: 0.55, b: 1 })).toBe(100);
    expect(maxSizePct({ l: 0.495, t: 0, r: 0.505, b: 1 })).toBeLessThan(100);
  });
});

describe("rescaling", () => {
  const start = { nx: 0.4, ny: 0.3, scale: 0.2 };
  it("around the centre keeps the content centre fixed", () => {
    const center = (p: typeof start) => p.nx + ((PADDED.l + PADDED.r) / 2) * p.scale;
    const out = rescaleAroundCenter(start, 0.3, 0.5, FRAME, PADDED);
    expect(out.scale).toBe(0.3);
    expect(center(out)).toBeCloseTo(center(start));
  });
  it("keeping the corner pins the content's top-left", () => {
    const out = rescaleKeepingCorner(start, 0.3, 0.5, FRAME, PADDED);
    expect(out.nx + PADDED.l * out.scale).toBeCloseTo(start.nx + PADDED.l * start.scale);
  });
  it("stays inside the frame when growing past an edge", () => {
    const out = rescaleAroundCenter({ nx: 0.8, ny: 0.3, scale: 0.2 }, 1, 0.5, FRAME, FULL_CONTENT);
    expect(out.nx).toBeGreaterThanOrEqual(-1e-9);
    expect(out.nx + out.scale).toBeLessThanOrEqual(1 + 1e-9);
  });
});

describe("cornerPlacement", () => {
  it("equal pixel margins on both axes", () => {
    const p = cornerPlacement("tl", 0.2, 0.5, FRAME);
    expect(p.nx * FRAME.w).toBeCloseTo(p.ny * FRAME.h);
  });
  it("aligns the visible content, not the transparent margin, to the corner", () => {
    const p = cornerPlacement("br", 0.4, 0.5, FRAME, PADDED);
    expect(p.nx + PADDED.r * p.scale).toBeCloseTo(0.97);
    const h = heightFraction(p.scale, 0.5, FRAME);
    expect(p.ny + PADDED.b * h).toBeCloseTo(1 - (0.03 * FRAME.w) / FRAME.h);
    const tl = cornerPlacement("tl", 0.4, 0.5, FRAME, PADDED);
    expect(tl.nx + PADDED.l * tl.scale).toBeCloseTo(0.03);
  });
  it("centre centres the content", () => {
    const p = cornerPlacement("c", 0.2, 0.5, FRAME, PADDED);
    expect(p.nx + ((PADDED.l + PADDED.r) / 2) * p.scale).toBeCloseTo(0.5);
  });
});
