/**
 * Drawing text boxes. A text box is drawn to a canvas the size of its pixels in the cropped
 * frame; the preview shows that canvas and the export overlays the same pixels as a PNG, so
 * what you see is what is burned into the video.
 */
import {
  FONT_FAMILY,
  backgroundShapes,
  cornerRadii,
  imagePosition,
  isEmptyText,
  layoutText,
  overlayFor,
  type Measurer,
  type Pt,
  type TextBox,
  type TextLayout,
} from "./textBox";
import type { ContentBox } from "./watermarks";

const FALLBACK = '"Helvetica Neue", Arial, sans-serif';

export function fontString(px: number, bold: boolean): string {
  return `${bold ? 700 : 400} ${px}px "${FONT_FAMILY}", ${FALLBACK}`;
}

let probe: CanvasRenderingContext2D | null = null;

/** Measures with the real font on a scratch canvas. */
export function canvasMeasurer(fontPx: number, bold: boolean): Measurer {
  probe ??= document.createElement("canvas").getContext("2d")!;
  const ctx = probe;
  const font = fontString(fontPx, bold);
  ctx.font = font;
  const m = ctx.measureText("Hg");
  return {
    width: (s) => {
      ctx.font = font;
      return ctx.measureText(s).width;
    },
    // Webviews that can't tell the font's box get the font's usual proportions.
    ascent: m.fontBoundingBoxAscent ?? fontPx * 0.95,
    descent: m.fontBoundingBoxDescent ?? fontPx * 0.25,
  };
}

// ───────── the font ─────────

let fontsLoaded = false;
const fontListeners = new Set<() => void>();

/** Load both weights before anything is drawn (and measured), or the first text would use a fallback. */
export async function ensureTextFont(): Promise<void> {
  if (fontsLoaded || !document.fonts) return;
  try {
    await Promise.all([document.fonts.load(fontString(32, false)), document.fonts.load(fontString(32, true))]);
  } catch {
    // The text falls back to a system font rather than not appearing.
  }
  fontsLoaded = true;
  fontListeners.forEach((f) => f());
}

/** Called once the font has loaded, so text drawn too early can be drawn again. */
export function onTextFontLoaded(fn: () => void): void {
  fontListeners.add(fn);
}

// ───────── drawing ─────────

/** Round the corners of a polygon with axis-aligned sides, with arcs that are tangent to both sides. */
export function roundedPolygonPath(ctx: CanvasRenderingContext2D, pts: Pt[], radius: number): void {
  const n = pts.length;
  if (n < 3) return;
  const radii = cornerRadii(pts, radius);
  const mid = (a: Pt, b: Pt): Pt => [(a[0] + b[0]) / 2, (a[1] + b[1]) / 2];
  const start = mid(pts[n - 1], pts[0]);
  ctx.beginPath();
  ctx.moveTo(start[0], start[1]);
  for (let i = 0; i < n; i++) {
    const p = pts[i];
    const next = pts[(i + 1) % n];
    ctx.arcTo(p[0], p[1], next[0], next[1], radii[i]);
  }
  ctx.closePath();
}

export function drawTextBox(canvas: HTMLCanvasElement, box: TextBox, layout: TextLayout): void {
  canvas.width = layout.imgW; // also clears it
  canvas.height = layout.imgH;
  const ctx = canvas.getContext("2d")!;
  ctx.translate(layout.offX, layout.offY);

  if (box.background) {
    ctx.fillStyle = box.background;
    for (const shape of backgroundShapes(layout.lines, layout.lineH)) {
      roundedPolygonPath(ctx, shape, layout.radius);
      ctx.fill();
    }
  }

  ctx.font = fontString(layout.fontPx, box.bold);
  ctx.textBaseline = "alphabetic";
  ctx.textAlign = "left";
  ctx.fillStyle = box.color;
  if (!box.background) {
    // Text straight on the video gets a soft shadow so it stays readable on any picture.
    ctx.shadowColor = "rgba(0, 0, 0, 0.5)";
    ctx.shadowBlur = layout.fontPx * 0.12;
    ctx.shadowOffsetY = layout.fontPx * 0.04;
  }
  for (const line of layout.lines) {
    if (!line.blank) ctx.fillText(line.text, line.left + layout.padX, line.top + layout.baseline);
  }
}

export function layoutFor(box: TextBox, frameW: number): TextLayout {
  return layoutText(box, frameW, canvasMeasurer);
}

export interface RenderedTextBox {
  canvas: HTMLCanvasElement;
  layout: TextLayout;
}

export function renderTextBox(box: TextBox, frameW: number, canvas: HTMLCanvasElement = document.createElement("canvas")): RenderedTextBox {
  const layout = layoutFor(box, frameW);
  drawTextBox(canvas, box, layout);
  return { canvas, layout };
}

// ───────── export ─────────

export interface TextOverlay {
  /** PNG, base64 */
  png: string;
  nx: number;
  ny: number;
  scale: number;
  content: ContentBox;
}

/** The pictures (and places) to burn into a frame `frame.w` × `frame.h` px. Boxes with no text are left out. */
export function textOverlays(texts: TextBox[], frame: { w: number; h: number }): TextOverlay[] {
  const out: TextOverlay[] = [];
  for (const box of texts) {
    if (isEmptyText(box.text)) continue;
    const { canvas, layout } = renderTextBox(box, frame.w);
    const pos = imagePosition(box, layout, frame.w, frame.h);
    const url = canvas.toDataURL("image/png");
    out.push({ png: url.slice(url.indexOf(",") + 1), ...overlayFor(layout, pos, frame.w, frame.h) });
  }
  return out;
}
