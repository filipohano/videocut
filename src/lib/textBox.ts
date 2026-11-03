/**
 * Text boxes: the "Text" section. Captions in the style of the TikTok / Instagram stories text
 * tool: a font, a colour, and an optional solid background block with rounded corners that
 * hugs every line (so a short line under a long one makes a stepped shape).
 *
 * Everything here is pure maths, so it's unit-tested; drawing is in textRender.ts. A text box
 * is stored relative to the *cropped* frame, like the watermarks:
 *   size   — the font size as a % of the frame's width
 *   cx, cy — the centre of the block as a fraction of the frame's width / height
 * and is laid out in pixels of the cropped frame, which is what the preview and the export draw.
 */
import { clamp } from "./format";
import type { ContentBox } from "./watermarks";

export type TextAlign = "left" | "center" | "right";

export interface TextBox {
  id: string;
  text: string;
  /** `#rrggbb` */
  color: string;
  /** `#rrggbb`, or null for no background */
  background: string | null;
  bold: boolean;
  align: TextAlign;
  size: number;
  cx: number;
  cy: number;
}

/** What a preset keeps: the look, not the words or the place. */
export type TextBoxStyle = Pick<TextBox, "color" | "background" | "bold" | "align" | "size">;

export const FONT_FAMILY = "TikTok Sans";
export const MIN_SIZE = 2;
export const MAX_SIZE = 20;
export const DEFAULT_SIZE = 7;
export const MAX_CHARS = 280;
/** Lines after wrapping; more than this is almost certainly a mistake. */
export const MAX_LINES = 16;
/** Most text boxes on one video. */
export const MAX_TEXTS = 12;

export const DEFAULT_STYLE: TextBoxStyle = {
  color: "#ffffff",
  background: "#000000",
  bold: true,
  align: "center",
  size: DEFAULT_SIZE,
};

/** Quick colours for the text and the background. */
export const SWATCHES: { name: string; color: string }[] = [
  { name: "White", color: "#ffffff" },
  { name: "Black", color: "#000000" },
  { name: "Red", color: "#fe2c55" },
  { name: "Yellow", color: "#ffd60a" },
  { name: "Orange", color: "#ff9f0a" },
  { name: "Green", color: "#30d158" },
  { name: "Blue", color: "#0a84ff" },
  { name: "Purple", color: "#bf5af2" },
];

// Proportions, in em (1 em = the font size).
export const PAD_X = 0.34;
export const LINE_HEIGHT = 1.3;
export const RADIUS = 0.36;
/** Room around the block for the soft shadow of text without a background. */
export const SHADOW_MARGIN = 0.3;
/** The widest the block may get, as a fraction of the frame width; longer lines wrap. */
export const MAX_BLOCK = 0.94;
/** A canvas taller than this can fail to draw in some webviews, so the lines are cut off here. */
export const MAX_IMAGE_HEIGHT = 12000;

/** `#abc` / `#AABBCC` / `aabbcc` → `#aabbcc`; anything else → null. */
export function normalizeHex(input: unknown): string | null {
  if (typeof input !== "string") return null;
  const m = /^#?([0-9a-f]{3}|[0-9a-f]{6})$/i.exec(input.trim());
  if (!m) return null;
  const hex = m[1].length === 3 ? [...m[1]].map((c) => c + c).join("") : m[1];
  return `#${hex.toLowerCase()}`;
}

export function newId(): string {
  const c = (globalThis as { crypto?: Crypto }).crypto;
  return c?.randomUUID ? c.randomUUID() : `t${Date.now().toString(36)}${Math.random().toString(36).slice(2, 8)}`;
}

export function newTextBox(style: Partial<TextBoxStyle> = {}, text = "Your text"): TextBox {
  const merged = sanitizeStyle({ ...DEFAULT_STYLE, ...style });
  return { id: newId(), text, ...merged, cx: 0.5, cy: 0.5 };
}

/** A usable style from whatever was stored (a preset file, an undo snapshot). */
export function sanitizeStyle(raw: Partial<Record<keyof TextBoxStyle, unknown>>): TextBoxStyle {
  const background = raw.background === null ? null : normalizeHex(raw.background);
  return {
    color: normalizeHex(raw.color) ?? DEFAULT_STYLE.color,
    // An unreadable background is "none", not black: that would hide the text.
    background: raw.background === undefined ? DEFAULT_STYLE.background : background,
    bold: typeof raw.bold === "boolean" ? raw.bold : DEFAULT_STYLE.bold,
    align: raw.align === "left" || raw.align === "right" || raw.align === "center" ? raw.align : DEFAULT_STYLE.align,
    size: clamp(Number.isFinite(raw.size) ? (raw.size as number) : DEFAULT_SIZE, MIN_SIZE, MAX_SIZE),
  };
}

/** A whole text box from untrusted data, or null if it isn't one. */
export function sanitizeTextBox(raw: unknown): TextBox | null {
  if (!raw || typeof raw !== "object") return null;
  const r = raw as Record<string, unknown>;
  if (typeof r.id !== "string" || r.id === "" || typeof r.text !== "string") return null;
  return {
    id: r.id,
    text: cleanText(r.text),
    ...sanitizeStyle(r),
    cx: clamp(Number.isFinite(r.cx) ? (r.cx as number) : 0.5, 0, 1),
    cy: clamp(Number.isFinite(r.cy) ? (r.cy as number) : 0.5, 0, 1),
  };
}

/** What the user typed, made safe to lay out: no control characters, tabs as spaces, a length cap. */
export function cleanText(text: string): string {
  return text
    .replace(/\r\n?/g, "\n")
    .replace(/\t/g, " ")
    // eslint-disable-next-line no-control-regex
    .replace(/[\u0000-\u0009\u000b-\u001f\u007f]/g, "")
    .slice(0, MAX_CHARS);
}

/** Nothing to see: no characters other than spaces and line breaks. */
export function isEmptyText(text: string): boolean {
  return text.trim() === "";
}

// ───────── wrapping ─────────

/**
 * Break text into lines no wider than `maxWidth`: at line breaks the user typed, then at
 * spaces, and inside a word only if the word alone is too wide. `width` measures a string.
 * Blank lines are kept (they separate the background blocks).
 */
export function wrapText(text: string, maxWidth: number, width: (s: string) => number): string[] {
  const out: string[] = [];
  for (const paragraph of cleanText(text).split("\n")) {
    if (paragraph.trim() === "") {
      out.push("");
      continue;
    }
    let line = "";
    const push = () => {
      out.push(line.trimEnd());
      line = "";
    };
    for (const word of paragraph.split(/ +/).filter((w) => w !== "")) {
      const candidate = line === "" ? word : `${line} ${word}`;
      if (width(candidate) <= maxWidth) {
        line = candidate;
        continue;
      }
      if (line !== "") push();
      line = word;
      // A word wider than a whole line: cut it where it no longer fits.
      while (width(line) > maxWidth && [...line].length > 1) {
        const chars = [...line];
        let fit = 1;
        let lo = 1;
        let hi = chars.length - 1;
        while (lo <= hi) {
          const mid = (lo + hi) >> 1;
          if (width(chars.slice(0, mid).join("")) <= maxWidth) {
            fit = mid;
            lo = mid + 1;
          } else hi = mid - 1;
        }
        out.push(chars.slice(0, fit).join(""));
        line = chars.slice(fit).join("");
      }
    }
    if (line !== "") push();
  }
  return out;
}

// ───────── layout ─────────

/** Measures text in one font at one size. */
export interface Measurer {
  width(text: string): number;
  /** Distance from the baseline to the top / bottom of the font's box, in px (both positive). */
  ascent: number;
  descent: number;
}

export interface LineLayout {
  text: string;
  blank: boolean;
  textW: number;
  /** The line's background block, in px from the left edge of the whole block. */
  left: number;
  right: number;
  top: number;
}

export interface TextLayout {
  fontPx: number;
  padX: number;
  lineH: number;
  radius: number;
  lines: LineLayout[];
  /** The block (the lines with their backgrounds). */
  blockW: number;
  blockH: number;
  /** Transparent room around the block inside the image. */
  margin: number;
  /** The image that is drawn and exported: the block plus its margin, with even sides. */
  imgW: number;
  imgH: number;
  /** Where the block sits inside the image. */
  offX: number;
  offY: number;
  /** Distance from the top of a line to its baseline. */
  baseline: number;
}

const even = (n: number) => {
  const c = Math.ceil(n);
  return c + (c % 2);
};

type Looks = Pick<TextBox, "text" | "bold" | "align" | "size" | "background">;

export function layoutText(box: Looks, frameW: number, measurerFor: (fontPx: number, bold: boolean) => Measurer): TextLayout {
  const size = clamp(Number.isFinite(box.size) ? box.size : DEFAULT_SIZE, MIN_SIZE, MAX_SIZE);
  const fontPx = Math.max(4, (size / 100) * Math.max(2, frameW));
  const m = measurerFor(fontPx, box.bold);
  const padX = fontPx * PAD_X;
  const lineH = fontPx * LINE_HEIGHT;
  const radius = fontPx * RADIUS;
  const maxText = Math.max(fontPx, Math.max(2, frameW) * MAX_BLOCK - 2 * padX);

  const wrapped = wrapText(box.text, maxText, m.width).slice(0, Math.min(MAX_LINES, Math.max(1, Math.floor(MAX_IMAGE_HEIGHT / lineH) - 1)));
  const texts = wrapped.length > 0 ? wrapped : [""];
  const widths = texts.map((t) => (t.trim() === "" ? 0 : m.width(t)));
  const blockWs = widths.map((w) => (w > 0 ? w + 2 * padX : 0));
  // With nothing to show the block keeps a small size, so the box can still be found and selected.
  const blockW = Math.max(...blockWs, fontPx);
  const blockH = texts.length * lineH;

  const lines: LineLayout[] = texts.map((text, i) => {
    const w = blockWs[i];
    const left = box.align === "left" ? 0 : box.align === "right" ? blockW - w : (blockW - w) / 2;
    return { text, blank: w === 0, textW: widths[i], left, right: left + w, top: i * lineH };
  });

  const margin = box.background === null ? Math.ceil(fontPx * SHADOW_MARGIN) : 0;
  const imgW = even(blockW + 2 * margin);
  const imgH = even(blockH + 2 * margin);
  return {
    fontPx,
    padX,
    lineH,
    radius,
    lines,
    blockW,
    blockH,
    margin,
    imgW,
    imgH,
    offX: (imgW - blockW) / 2,
    offY: (imgH - blockH) / 2,
    baseline: lineH / 2 + (m.ascent - m.descent) / 2,
  };
}

// ───────── the background block ─────────

export type Pt = [number, number];

/**
 * The outline of the background behind each run of non-blank lines, as polygons with square
 * corners (clockwise, starting at the top right). Blank lines split the background in two.
 */
export function backgroundShapes(lines: LineLayout[], lineH: number): Pt[][] {
  const shapes: Pt[][] = [];
  let run: LineLayout[] = [];
  const flush = () => {
    if (run.length === 0) return;
    const pts: Pt[] = [];
    for (const l of run) pts.push([l.right, l.top], [l.right, l.top + lineH]);
    for (const l of [...run].reverse()) pts.push([l.left, l.top + lineH], [l.left, l.top]);
    shapes.push(simplify(pts));
    run = [];
  };
  for (const line of lines) {
    if (line.blank) flush();
    else run.push(line);
  }
  flush();
  return shapes;
}

/** Drop repeated points and points in the middle of a straight run. */
export function simplify(points: Pt[]): Pt[] {
  const same = (a: Pt, b: Pt) => Math.abs(a[0] - b[0]) < 1e-6 && Math.abs(a[1] - b[1]) < 1e-6;
  let pts = points.filter((p, i) => !same(p, points[(i + points.length - 1) % points.length]));
  let changed = true;
  while (changed && pts.length > 3) {
    changed = false;
    for (let i = 0; i < pts.length; i++) {
      const a = pts[(i + pts.length - 1) % pts.length];
      const b = pts[i];
      const c = pts[(i + 1) % pts.length];
      const straightX = Math.abs(a[0] - b[0]) < 1e-6 && Math.abs(b[0] - c[0]) < 1e-6;
      const straightY = Math.abs(a[1] - b[1]) < 1e-6 && Math.abs(b[1] - c[1]) < 1e-6;
      if (straightX || straightY) {
        pts = pts.filter((_, j) => j !== i);
        changed = true;
        break;
      }
    }
  }
  return pts;
}

/**
 * The radius to round each corner with: `radius`, but never more than half of either side, so
 * that two steps close together still join smoothly (the "stepped" look of the stories tool).
 */
export function cornerRadii(points: Pt[], radius: number): number[] {
  const len = (a: Pt, b: Pt) => Math.hypot(a[0] - b[0], a[1] - b[1]);
  return points.map((p, i) => {
    const prev = points[(i + points.length - 1) % points.length];
    const next = points[(i + 1) % points.length];
    return Math.max(0, Math.min(radius, len(prev, p) / 2, len(p, next) / 2));
  });
}

// ───────── placing ─────────

/** Keep the block inside the frame (a block bigger than the frame is centred). */
export function clampCenter(cx: number, cy: number, layout: Pick<TextLayout, "blockW" | "blockH">, frameW: number, frameH: number): { cx: number; cy: number } {
  const bw = layout.blockW / Math.max(1, frameW);
  const bh = layout.blockH / Math.max(1, frameH);
  const fix = (v: number, half: number) => (half >= 0.5 ? 0.5 : clamp(Number.isFinite(v) ? v : 0.5, half, 1 - half));
  return { cx: fix(cx, bw / 2), cy: fix(cy, bh / 2) };
}

/** Top-left of the image in whole frame pixels, which is where both the preview and the export put it. */
export function imagePosition(box: Pick<TextBox, "cx" | "cy">, layout: TextLayout, frameW: number, frameH: number): { left: number; top: number } {
  return {
    left: Math.round(box.cx * frameW - layout.blockW / 2 - layout.offX),
    top: Math.round(box.cy * frameH - layout.blockH / 2 - layout.offY),
  };
}

/** The same thing in the terms the exporter's overlay filter uses. */
export function overlayFor(layout: TextLayout, pos: { left: number; top: number }, frameW: number, frameH: number): { nx: number; ny: number; scale: number; content: ContentBox } {
  return {
    nx: pos.left / frameW,
    ny: pos.top / frameH,
    scale: layout.imgW / frameW,
    content: {
      l: layout.offX / layout.imgW,
      t: layout.offY / layout.imgH,
      r: (layout.offX + layout.blockW) / layout.imgW,
      b: (layout.offY + layout.blockH) / layout.imgH,
    },
  };
}

/** Dragging the corner grip: the new font size when the block's width changes by `dx` px. */
export function sizeAfterResize(startSize: number, startBlockW: number, dx: number): number {
  const base = Math.max(1, startBlockW);
  return clamp(startSize * Math.max(0.05, (base + dx) / base), MIN_SIZE, MAX_SIZE);
}
