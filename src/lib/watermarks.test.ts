import { describe, expect, it } from "vitest";
import { clampPlacement, cornerPlacement, heightFraction } from "./watermarks";

const FRAME = { w: 1000, h: 500 };

describe("heightFraction", () => {
  it("accounts for the frame's aspect ratio", () => {
    // 200px-wide square logo in a 1000x500 frame is 200px tall = 40% of the height.
    expect(heightFraction(0.2, 1, FRAME)).toBeCloseTo(0.4);
  });
});

describe("clampPlacement", () => {
  it("keeps the watermark fully inside", () => {
    const p = clampPlacement({ nx: 0.95, ny: 0.95, scale: 0.2 }, 0.5, FRAME);
    expect(p.nx).toBeCloseTo(0.8);
    expect(p.ny).toBeCloseTo(1 - heightFraction(0.2, 0.5, FRAME));
    expect(clampPlacement({ nx: -3, ny: -3, scale: 0.2 }, 0.5, FRAME)).toMatchObject({ nx: 0, ny: 0 });
  });
  it("shrinks a watermark that is taller than the frame", () => {
    const p = clampPlacement({ nx: 0, ny: 0, scale: 0.9 }, 2, FRAME);
    expect(heightFraction(p.scale, 2, FRAME)).toBeLessThanOrEqual(1.0001);
  });
});

describe("cornerPlacement", () => {
  it("equal pixel margins on both axes", () => {
    const p = cornerPlacement("tl", 0.2, 0.5, FRAME);
    expect(p.nx * FRAME.w).toBeCloseTo(p.ny * FRAME.h);
  });
  it("bottom-right sits against the corner minus margin", () => {
    const p = cornerPlacement("br", 0.2, 0.5, FRAME);
    expect(p.nx + p.scale).toBeCloseTo(0.97);
    expect(p.ny + heightFraction(0.2, 0.5, FRAME)).toBeCloseTo(1 - (0.03 * FRAME.w) / FRAME.h);
  });
  it("centre centres", () => {
    const p = cornerPlacement("c", 0.2, 0.5, FRAME);
    expect(p.nx).toBeCloseTo(0.4);
    expect(p.ny + heightFraction(0.2, 0.5, FRAME) / 2).toBeCloseTo(0.5);
  });
});
