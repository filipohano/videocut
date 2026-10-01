/**
 * Watermark placement maths.
 *
 * Placement is stored relative to the *cropped* frame — exactly what the
 * exporter uses — so what you see in the preview is what ffmpeg renders:
 *   nx, ny — top-left corner of the *image* as a fraction of cropped width / height
 *            (may be negative / past 1 when only transparent margin leaves the frame)
 *   scale  — image width as a fraction of cropped width
 *
 * `content` is the visible (non-transparent) part of the image as fractions of the
 * image. The rule that keeps the app sane: the visible content always stays
 * inside the frame, but transparent margins may hang outside it.
 */
import { clamp } from "./format";
import type { Size } from "./crop";

export interface ContentBox {
  l: number;
  t: number;
  r: number;
  b: number;
}
export const FULL_CONTENT: ContentBox = { l: 0, t: 0, r: 1, b: 1 };

export interface Placement {
  nx: number;
  ny: number;
  scale: number;
}

export type Corner = "tl" | "tr" | "bl" | "br" | "c";

export const MIN_SCALE = 0.02;
/** Matches the exporter's limit. */
export const MAX_SCALE = 10;
export const MIN_SIZE_PCT = 3;
const MARGIN = 0.03; // of the frame width, equal in pixels on both axes

const contentWidth = (c: ContentBox) => Math.max(0.01, c.r - c.l);

/** Size slider ↔ scale. "Size" is the *visible content's* width as a % of the frame width. */
export function scaleFromSizePct(pct: number, c: ContentBox): number {
  return clamp(pct / 100 / contentWidth(c), MIN_SCALE, MAX_SCALE);
}
export function sizePctFromScale(scale: number, c: ContentBox): number {
  return Math.round(scale * contentWidth(c) * 100);
}
/** Highest size the slider offers: 100% of the frame, unless the image would exceed the exporter's limit. */
export function maxSizePct(c: ContentBox): number {
  return Math.max(MIN_SIZE_PCT, Math.min(100, Math.floor(MAX_SCALE * contentWidth(c) * 100)));
}

/** Image height as a fraction of the cropped height. */
export function heightFraction(scale: number, aspect: number, crop: Size): number {
  return (scale * crop.w * aspect) / crop.h;
}

function within(v: number, lo: number, hi: number): number {
  // If the content is bigger than the frame the range is empty: centre it.
  return (lo <= hi ? clamp(v, lo, hi) : (lo + hi) / 2) + 0; // `+ 0` turns -0 into 0
}

/** Keep the visible content inside the frame (margins may leave it). */
export function clampPlacement(p: Placement, aspect: number, crop: Size, content: ContentBox = FULL_CONTENT): Placement {
  const scale = clamp(Number.isFinite(p.scale) ? p.scale : 0.2, MIN_SCALE, MAX_SCALE);
  const h = heightFraction(scale, aspect, crop);
  return {
    scale,
    nx: within(Number.isFinite(p.nx) ? p.nx : 0, -content.l * scale, 1 - content.r * scale),
    ny: within(Number.isFinite(p.ny) ? p.ny : 0, -content.t * h, 1 - content.b * h),
  };
}

/** Change the size around the content's centre (what a size slider should do). */
export function rescaleAroundCenter(p: Placement, newScale: number, aspect: number, crop: Size, c: ContentBox): Placement {
  const s2 = clamp(newScale, MIN_SCALE, MAX_SCALE);
  const h1 = heightFraction(p.scale, aspect, crop);
  const h2 = heightFraction(s2, aspect, crop);
  const cx = p.nx + ((c.l + c.r) / 2) * p.scale;
  const cy = p.ny + ((c.t + c.b) / 2) * h1;
  return clampPlacement({ scale: s2, nx: cx - ((c.l + c.r) / 2) * s2, ny: cy - ((c.t + c.b) / 2) * h2 }, aspect, crop, c);
}

/** Change the size keeping the content's top-left corner fixed (what dragging the corner grip does). */
export function rescaleKeepingCorner(p: Placement, newScale: number, aspect: number, crop: Size, c: ContentBox): Placement {
  const s2 = clamp(newScale, MIN_SCALE, MAX_SCALE);
  const h1 = heightFraction(p.scale, aspect, crop);
  const h2 = heightFraction(s2, aspect, crop);
  const left = p.nx + c.l * p.scale;
  const top = p.ny + c.t * h1;
  return clampPlacement({ scale: s2, nx: left - c.l * s2, ny: top - c.t * h2 }, aspect, crop, c);
}

/** Put the visible content in a corner (or the centre), with a small margin. */
export function cornerPlacement(corner: Corner, scale: number, aspect: number, crop: Size, content: ContentBox = FULL_CONTENT): Placement {
  const s = clamp(scale, MIN_SCALE, MAX_SCALE);
  const h = heightFraction(s, aspect, crop);
  const mx = MARGIN;
  const my = (MARGIN * crop.w) / crop.h;
  const pos: Record<Corner, [number, number]> = {
    tl: [mx - content.l * s, my - content.t * h],
    tr: [1 - mx - content.r * s, my - content.t * h],
    bl: [mx - content.l * s, 1 - my - content.b * h],
    br: [1 - mx - content.r * s, 1 - my - content.b * h],
    c: [0.5 - ((content.l + content.r) / 2) * s, 0.5 - ((content.t + content.b) / 2) * h],
  };
  const [nx, ny] = pos[corner];
  return clampPlacement({ nx, ny, scale: s }, aspect, crop, content);
}
