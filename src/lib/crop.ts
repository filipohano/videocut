/** Crop-rectangle maths, in source-video pixels. Pure and unit-tested. */
import { clamp } from "./format";

export interface Rect {
  x: number;
  y: number;
  w: number;
  h: number;
}
export interface Size {
  w: number;
  h: number;
}
export type Handle = "n" | "s" | "e" | "w" | "ne" | "nw" | "se" | "sw";

export const MIN_CROP = 16;

export const ASPECT_OPTIONS: { key: string; label: string }[] = [
  { key: "free", label: "Free" },
  { key: "original", label: "Original" },
  { key: "1:1", label: "1:1 (square)" },
  { key: "4:5", label: "4:5 (feed)" },
  { key: "9:16", label: "9:16 (vertical)" },
  { key: "16:9", label: "16:9 (wide)" },
  { key: "3:4", label: "3:4" },
  { key: "4:3", label: "4:3" },
];

/** `null` means unconstrained. */
export function ratioValue(key: string, source: Size): number | null {
  if (key === "free") return null;
  if (key === "original") return source.w / source.h;
  const [a, b] = key.split(":").map(Number);
  return a > 0 && b > 0 ? a / b : null;
}

export const fullRect = (s: Size): Rect => ({ x: 0, y: 0, w: s.w, h: s.h });

export function clampRect(r: Rect, bounds: Size, min = MIN_CROP): Rect {
  const w = clamp(Math.round(r.w), Math.min(min, bounds.w), bounds.w);
  const h = clamp(Math.round(r.h), Math.min(min, bounds.h), bounds.h);
  return {
    w,
    h,
    x: clamp(Math.round(r.x), 0, bounds.w - w),
    y: clamp(Math.round(r.y), 0, bounds.h - h),
  };
}

export function moveRect(r: Rect, dx: number, dy: number, bounds: Size): Rect {
  return {
    ...r,
    x: clamp(Math.round(r.x + dx), 0, bounds.w - r.w),
    y: clamp(Math.round(r.y + dy), 0, bounds.h - r.h),
  };
}

/**
 * Resize `start` by dragging `handle` by (dx, dy). With `ratio` set (w/h) the
 * aspect ratio is kept: corners pin the opposite corner, edges grow around the
 * rectangle's centre line.
 */
export function resizeRect(
  start: Rect,
  handle: Handle,
  dx: number,
  dy: number,
  bounds: Size,
  ratio: number | null,
  min = MIN_CROP,
): Rect {
  return ratio === null
    ? resizeFree(start, handle, dx, dy, bounds, min)
    : resizeLocked(start, handle, dx, dy, bounds, ratio, min);
}

function resizeFree(s: Rect, handle: Handle, dx: number, dy: number, b: Size, min: number): Rect {
  let left = s.x;
  let right = s.x + s.w;
  let top = s.y;
  let bottom = s.y + s.h;
  if (handle.includes("w")) left = clamp(left + dx, 0, right - min);
  if (handle.includes("e")) right = clamp(right + dx, left + min, b.w);
  if (handle.includes("n")) top = clamp(top + dy, 0, bottom - min);
  if (handle.includes("s")) bottom = clamp(bottom + dy, top + min, b.h);
  return clampRect({ x: left, y: top, w: right - left, h: bottom - top }, b, min);
}

function resizeLocked(
  s: Rect,
  handle: Handle,
  dx: number,
  dy: number,
  b: Size,
  ratio: number,
  min: number,
): Rect {
  const minW = Math.max(min, min * ratio);
  const isCorner = handle.length === 2;

  if (isCorner) {
    const east = handle.includes("e");
    const south = handle.includes("s");
    const anchorX = east ? s.x : s.x + s.w;
    const anchorY = south ? s.y : s.y + s.h;
    const pointerX = (east ? s.x + s.w : s.x) + dx;
    const pointerY = (south ? s.y + s.h : s.y) + dy;
    let w = Math.abs(pointerX - anchorX);
    let h = Math.abs(pointerY - anchorY);
    // Grow to cover whichever axis the pointer moved further along.
    if (w / ratio >= h) h = w / ratio;
    else w = h * ratio;
    // Don't leave the frame in the dragged direction.
    const maxW = east ? b.w - anchorX : anchorX;
    const maxH = south ? b.h - anchorY : anchorY;
    if (w > maxW) {
      w = maxW;
      h = w / ratio;
    }
    if (h > maxH) {
      h = maxH;
      w = h * ratio;
    }
    if (w < minW) {
      w = minW;
      h = w / ratio;
    }
    const x = east ? anchorX : anchorX - w;
    const y = south ? anchorY : anchorY - h;
    return clampRect({ x, y, w, h }, b, min);
  }

  const cx = s.x + s.w / 2;
  const cy = s.y + s.h / 2;
  if (handle === "e" || handle === "w") {
    const anchorX = handle === "e" ? s.x : s.x + s.w;
    let w = handle === "e" ? s.w + dx : s.w - dx;
    // Height grows symmetrically, so it's limited by the nearer top/bottom edge.
    const maxH = 2 * Math.min(cy, b.h - cy);
    const maxW = Math.min(handle === "e" ? b.w - anchorX : anchorX, maxH * ratio);
    w = clamp(w, minW, Math.max(minW, maxW));
    const h = w / ratio;
    return clampRect({ x: handle === "e" ? anchorX : anchorX - w, y: cy - h / 2, w, h }, b, min);
  }
  const anchorY = handle === "s" ? s.y : s.y + s.h;
  let h = handle === "s" ? s.h + dy : s.h - dy;
  const maxW = 2 * Math.min(cx, b.w - cx);
  const maxH = Math.min(handle === "s" ? b.h - anchorY : anchorY, maxW / ratio);
  h = clamp(h, minW / ratio, Math.max(minW / ratio, maxH));
  const w = h * ratio;
  return clampRect({ x: cx - w / 2, y: handle === "s" ? anchorY : anchorY - h, w, h }, b, min);
}

/** Largest rectangle with `ratio` that fits inside `r`, centred in it. */
export function fitRatio(r: Rect, ratio: number, bounds: Size): Rect {
  let w = r.w;
  let h = w / ratio;
  if (h > r.h) {
    h = r.h;
    w = h * ratio;
  }
  return clampRect({ x: r.x + (r.w - w) / 2, y: r.y + (r.h - h) / 2, w, h }, bounds);
}

/** H.264 4:2:0 needs even sizes/offsets; this is what the exporter will use. */
export function evenRect(r: Rect): Rect {
  const e = (v: number) => Math.max(0, Math.floor(v / 2) * 2);
  return { x: e(r.x), y: e(r.y), w: Math.max(2, e(r.w)), h: Math.max(2, e(r.h)) };
}

export const sameRect = (a: Rect, b: Rect) =>
  a.x === b.x && a.y === b.y && a.w === b.w && a.h === b.h;
