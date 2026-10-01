/**
 * Watermark placement maths.
 *
 * Placement is stored relative to the *cropped* frame — exactly what the
 * exporter uses — so what you see in the preview is what ffmpeg renders:
 *   nx, ny — top-left corner as a fraction of cropped width / height
 *   scale  — watermark width as a fraction of cropped width
 */
import { clamp } from "./format";
import type { Size } from "./crop";

export interface Placement {
  nx: number;
  ny: number;
  scale: number;
}

export type Corner = "tl" | "tr" | "bl" | "br" | "c";

export const MIN_SCALE = 0.03;
export const MAX_SCALE = 1;
const MARGIN = 0.03; // of the frame width, equal in pixels on both axes

/** Watermark height as a fraction of the cropped height. */
export function heightFraction(scale: number, aspect: number, crop: Size): number {
  return (scale * crop.w * aspect) / crop.h;
}

/** Keep the whole watermark inside the frame (shrinking it if it can't fit). */
export function clampPlacement(p: Placement, aspect: number, crop: Size): Placement {
  let scale = clamp(p.scale, MIN_SCALE, MAX_SCALE);
  const hFrac = heightFraction(scale, aspect, crop);
  if (hFrac > 1) scale = clamp(scale / hFrac, MIN_SCALE, MAX_SCALE);
  const h = heightFraction(scale, aspect, crop);
  return {
    scale,
    nx: clamp(p.nx, 0, Math.max(0, 1 - scale)),
    ny: clamp(p.ny, 0, Math.max(0, 1 - h)),
  };
}

export function cornerPlacement(corner: Corner, scale: number, aspect: number, crop: Size): Placement {
  const base = clampPlacement({ nx: 0, ny: 0, scale }, aspect, crop);
  const w = base.scale;
  const h = heightFraction(w, aspect, crop);
  const mx = MARGIN;
  const my = (MARGIN * crop.w) / crop.h;
  const left = mx;
  const right = 1 - w - mx;
  const top = my;
  const bottom = 1 - h - my;
  const pos: Record<Corner, [number, number]> = {
    tl: [left, top],
    tr: [right, top],
    bl: [left, bottom],
    br: [right, bottom],
    c: [(1 - w) / 2, (1 - h) / 2],
  };
  const [nx, ny] = pos[corner];
  return clampPlacement({ nx, ny, scale: w }, aspect, crop);
}
