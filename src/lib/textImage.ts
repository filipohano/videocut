/**
 * Text watermarks are drawn to a transparent PNG with a canvas, then treated like
 * any other watermark image. That way every font installed on the Mac works, and
 * the preview and the exported video use the very same pixels.
 */
import type { ContentBox, TextStyle } from "../api";

export const DEFAULT_TEXT_STYLE: TextStyle = {
  text: "Your text",
  fontFamily: "Helvetica Neue",
  bold: true,
  italic: false,
  color: "#ffffff",
  outline: true,
  outlineColor: "#000000",
  shadow: false,
  align: "center",
};

/** Common macOS and Windows fonts (a font the PC lacks falls back to Arial). The font field also accepts any other installed font name. */
export const FONT_CHOICES = [
  "Helvetica Neue",
  "Helvetica",
  "Arial",
  "Arial Black",
  "Avenir Next",
  "Futura",
  "Gill Sans",
  "Impact",
  "Verdana",
  "Trebuchet MS",
  "Georgia",
  "Times New Roman",
  "Baskerville",
  "Didot",
  "Palatino",
  "Courier New",
  "Menlo",
  "American Typewriter",
  "Marker Felt",
  "Chalkduster",
  "Brush Script MT",
  "Snell Roundhand",
  "Papyrus",
  "Copperplate",
  "SF Pro Display",
  "Segoe UI",
  "Bahnschrift",
  "Calibri",
  "Cambria",
  "Tahoma",
  "Consolas",
  "Comic Sans MS",
  "Segoe Script",
];

export const MAX_LINES = 6;
export const MAX_CHARS = 300;
const FONT_PX = 200;
const MAX_CANVAS_WIDTH = 6000;

/** Lines to draw: capped in count and length, never empty. */
export function textLines(text: string): string[] {
  const lines = text.slice(0, MAX_CHARS).split(/\r?\n/).slice(0, MAX_LINES);
  return lines.some((l) => l.trim() !== "") ? lines : ["…"];
}

export function fontCss(style: TextStyle, px: number): string {
  const family = style.fontFamily.replace(/["\\]/g, "").trim() || "Helvetica Neue";
  return `${style.italic ? "italic " : ""}${style.bold ? "700" : "400"} ${px}px "${family}", "Helvetica Neue", Arial, sans-serif`;
}

export interface RenderedText {
  canvas: HTMLCanvasElement;
  /** PNG as a data: URL, ready for <img>. */
  dataUrl: string;
  /** PNG bytes, base64 (what the backend stores). */
  base64: string;
  width: number;
  height: number;
  /** visible part of the image */
  content: ContentBox;
}

export function renderText(style: TextStyle): RenderedText {
  const lines = textLines(style.text);
  const probe = document.createElement("canvas").getContext("2d")!;
  let px = FONT_PX;
  const measure = () => {
    probe.font = fontCss(style, px);
    return Math.max(...lines.map((l) => probe.measureText(l || " ").width));
  };
  let textW = measure();
  const decor = (p: number) => (style.outline ? p * 0.09 : 0) + (style.shadow ? p * 0.12 : 0) + p * 0.04;
  // Very long lines are drawn smaller instead of making a gigantic canvas.
  if (textW + 2 * decor(px) > MAX_CANVAS_WIDTH) {
    px = Math.floor((px * (MAX_CANVAS_WIDTH - 2 * decor(px))) / textW);
    textW = measure();
  }
  const pad = Math.ceil(decor(px));
  const lineH = px * 1.2;
  const width = Math.ceil(textW) + 2 * pad;
  const height = Math.ceil(lines.length * lineH) + 2 * pad;

  const canvas = document.createElement("canvas");
  canvas.width = width;
  canvas.height = height;
  const ctx = canvas.getContext("2d")!;
  ctx.font = fontCss(style, px);
  ctx.textBaseline = "middle";
  ctx.textAlign = style.align;
  ctx.lineJoin = "round";
  ctx.miterLimit = 2;
  const x = style.align === "left" ? pad : style.align === "right" ? width - pad : width / 2;

  lines.forEach((line, i) => {
    const y = pad + (i + 0.5) * lineH;
    if (style.outline) {
      ctx.save();
      if (style.shadow) {
        ctx.shadowColor = "rgba(0,0,0,0.65)";
        ctx.shadowBlur = px * 0.08;
        ctx.shadowOffsetY = px * 0.04;
      }
      ctx.strokeStyle = style.outlineColor;
      ctx.lineWidth = px * 0.09;
      ctx.strokeText(line, x, y);
      ctx.restore();
      ctx.fillStyle = style.color;
      ctx.fillText(line, x, y);
    } else {
      ctx.save();
      if (style.shadow) {
        ctx.shadowColor = "rgba(0,0,0,0.65)";
        ctx.shadowBlur = px * 0.08;
        ctx.shadowOffsetY = px * 0.04;
      }
      ctx.fillStyle = style.color;
      ctx.fillText(line, x, y);
      ctx.restore();
    }
  });

  const dataUrl = canvas.toDataURL("image/png");
  return { canvas, dataUrl, base64: dataUrl.slice(dataUrl.indexOf(",") + 1), width, height, content: contentBox(canvas) };
}

/** Bounding box of the non-transparent pixels, as fractions of the canvas. */
export function contentBox(canvas: HTMLCanvasElement): ContentBox {
  const scale = Math.min(1, 800 / canvas.width);
  const w = Math.max(1, Math.round(canvas.width * scale));
  const h = Math.max(1, Math.round(canvas.height * scale));
  const small = document.createElement("canvas");
  small.width = w;
  small.height = h;
  const sctx = small.getContext("2d", { willReadFrequently: true })!;
  sctx.drawImage(canvas, 0, 0, w, h);
  const data = sctx.getImageData(0, 0, w, h).data;
  let minX = w, minY = h, maxX = -1, maxY = -1;
  for (let y = 0; y < h; y++)
    for (let x = 0; x < w; x++)
      if (data[(y * w + x) * 4 + 3] > 8) {
        if (x < minX) minX = x;
        if (x > maxX) maxX = x;
        if (y < minY) minY = y;
        if (y > maxY) maxY = y;
      }
  if (maxX < 0) return { l: 0, t: 0, r: 1, b: 1 };
  return { l: minX / w, t: minY / h, r: (maxX + 1) / w, b: (maxY + 1) / h };
}
