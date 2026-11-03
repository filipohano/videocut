import { describe, expect, it } from "vitest";
import {
  DEFAULT_STYLE,
  LINE_HEIGHT,
  MAX_BLOCK,
  MAX_CHARS,
  MAX_LINES,
  MAX_SIZE,
  MIN_SIZE,
  PAD_X,
  RADIUS,
  backgroundShapes,
  cleanText,
  clampCenter,
  cornerRadii,
  imagePosition,
  isEmptyText,
  layoutText,
  newTextBox,
  normalizeHex,
  overlayFor,
  sanitizeStyle,
  sanitizeTextBox,
  simplify,
  sizeAfterResize,
  wrapText,
  type Measurer,
  type Pt,
} from "./textBox";

/** Every character is half an em wide, so widths can be worked out by hand. */
const measurerFor = (fontPx: number): Measurer => ({
  width: (s) => [...s].length * fontPx * 0.5,
  ascent: fontPx * 0.9,
  descent: fontPx * 0.3,
});
/** 10 px per character. */
const ten = (s: string) => [...s].length * 10;

describe("colours", () => {
  it("normalises hex colours and rejects the rest", () => {
    expect(normalizeHex("#FFF")).toBe("#ffffff");
    expect(normalizeHex("ff00AA")).toBe("#ff00aa");
    expect(normalizeHex(" #0a84ff ")).toBe("#0a84ff");
    for (const bad of ["", "red", "#12", "#12345", "#gggggg", "#1234567", "rgb(1,2,3)", null, undefined, 5]) expect(normalizeHex(bad as never), String(bad)).toBeNull();
  });
});

describe("sanitising", () => {
  it("fills in a style from whatever was stored", () => {
    expect(sanitizeStyle({})).toEqual(DEFAULT_STYLE);
    expect(sanitizeStyle({ color: "#F00", background: null, bold: false, align: "left", size: 9 })).toEqual({ color: "#ff0000", background: null, bold: false, align: "left", size: 9 });
    // Junk falls back, size is kept in range, and a broken background means none (never one that hides the text).
    const junk = sanitizeStyle({ color: "blue", background: "nope", bold: "yes", align: "justify", size: 500 });
    expect(junk).toEqual({ color: "#ffffff", background: null, bold: true, align: "center", size: MAX_SIZE });
    expect(sanitizeStyle({ size: -3 }).size).toBe(MIN_SIZE);
    expect(sanitizeStyle({ size: Number.NaN }).size).toBeGreaterThan(MIN_SIZE);
  });

  it("only accepts real text boxes", () => {
    const ok = sanitizeTextBox({ id: "a", text: "hi", color: "#fff", background: "#000", bold: true, align: "right", size: 6, cx: 0.2, cy: 0.9 });
    expect(ok).toEqual({ id: "a", text: "hi", color: "#ffffff", background: "#000000", bold: true, align: "right", size: 6, cx: 0.2, cy: 0.9 });
    for (const bad of [null, "x", 5, {}, { id: "", text: "a" }, { id: "a" }, { id: "a", text: 5 }]) expect(sanitizeTextBox(bad)).toBeNull();
    // Places outside the frame are pulled back in.
    expect(sanitizeTextBox({ id: "a", text: "", cx: 4, cy: -1 })).toMatchObject({ cx: 1, cy: 0 });
    expect(sanitizeTextBox({ id: "a", text: "", cx: "x" })).toMatchObject({ cx: 0.5 });
  });

  it("makes a new box in the middle with the default look, and each gets its own id", () => {
    const a = newTextBox();
    const b = newTextBox({ color: "#ff0000", background: null });
    expect(a).toMatchObject({ ...DEFAULT_STYLE, cx: 0.5, cy: 0.5, text: "Your text" });
    expect(b).toMatchObject({ color: "#ff0000", background: null });
    expect(a.id).not.toBe(b.id);
  });

  it("cleans typed text", () => {
    expect(cleanText("a\r\nb\rc\td")).toBe("a\nb\nc d");
    expect(cleanText("a\u0000b\u0007c\u007fd")).toBe("abcd");
    expect(cleanText("x".repeat(MAX_CHARS + 50))).toHaveLength(MAX_CHARS);
    expect(cleanText("keep ✓ emoji 😀 æøå")).toBe("keep ✓ emoji 😀 æøå");
    expect(isEmptyText("  \n ")).toBe(true);
    expect(isEmptyText(" a ")).toBe(false);
  });
});

describe("wrapText", () => {
  it("leaves short text alone and keeps the lines the user typed", () => {
    expect(wrapText("hello world", 200, ten)).toEqual(["hello world"]);
    expect(wrapText("one\ntwo\nthree", 200, ten)).toEqual(["one", "two", "three"]);
  });

  it("wraps at spaces", () => {
    // 10 px per character, 100 px wide = 10 characters.
    expect(wrapText("the quick brown fox", 100, ten)).toEqual(["the quick", "brown fox"]);
    expect(wrapText("aaa bbb ccc ddd", 70, ten)).toEqual(["aaa bbb", "ccc ddd"]);
    expect(wrapText("a b c d e f g h", 30, ten)).toEqual(["a b", "c d", "e f", "g h"]);
  });

  it("keeps blank lines, and ignores extra spaces", () => {
    expect(wrapText("a\n\nb", 100, ten)).toEqual(["a", "", "b"]);
    expect(wrapText("a\n   \nb", 100, ten)).toEqual(["a", "", "b"]);
    expect(wrapText("a    b", 100, ten)).toEqual(["a b"]);
    expect(wrapText("  leading and trailing  ", 400, ten)).toEqual(["leading and trailing"]);
    expect(wrapText("", 100, ten)).toEqual([""]);
  });

  it("cuts a word that is wider than the whole line", () => {
    expect(wrapText("abcdefghijklmnop", 50, ten)).toEqual(["abcde", "fghij", "klmno", "p"]);
    expect(wrapText("hi abcdefghij", 50, ten)).toEqual(["hi", "abcde", "fghij"]);
    // Never an endless loop, even when not a single character fits.
    expect(wrapText("abc", 5, ten)).toEqual(["a", "b", "c"]);
  });

  it("counts an emoji as one character, not two halves", () => {
    expect(wrapText("😀😀😀😀😀😀", 30, ten)).toEqual(["😀😀😀", "😀😀😀"]);
  });
});

describe("layoutText", () => {
  const box = { text: "hello", bold: true, align: "center" as const, size: 10, background: "#000000" as string | null };

  it("sizes the font from the frame width", () => {
    const l = layoutText(box, 1000, measurerFor);
    expect(l.fontPx).toBe(100);
    expect(l.padX).toBeCloseTo(100 * PAD_X);
    expect(l.lineH).toBeCloseTo(100 * LINE_HEIGHT);
    expect(l.radius).toBeCloseTo(100 * RADIUS);
    // 5 characters of 50 px plus padding on both sides.
    expect(l.blockW).toBeCloseTo(250 + 2 * 34);
    expect(l.blockH).toBeCloseTo(130);
    expect(layoutText({ ...box, size: 5 }, 1000, measurerFor).fontPx).toBe(50);
    expect(layoutText({ ...box, size: 5 }, 2000, measurerFor).fontPx).toBe(100);
  });

  it("centres the text on its baseline", () => {
    const l = layoutText(box, 1000, measurerFor);
    // Half the line height, plus half of (ascent - descent) = (90 - 30) / 2.
    expect(l.baseline).toBeCloseTo(65 + 30);
  });

  it("gives every line its own block, aligned to the widest", () => {
    const text = "aa\nbbbbbb\ncccc";
    const widths = (align: "left" | "center" | "right") => layoutText({ ...box, text, align }, 1000, measurerFor).lines.map((l) => [Math.round(l.left), Math.round(l.right)]);
    // 100 px per... 50 px per character plus 68 padding: 168, 368, 268; widest is 368.
    expect(widths("left")).toEqual([[0, 168], [0, 368], [0, 268]]);
    expect(widths("right")).toEqual([[200, 368], [0, 368], [100, 368]]);
    expect(widths("center")).toEqual([[100, 268], [0, 368], [50, 318]]);
    const l = layoutText({ ...box, text }, 1000, measurerFor);
    expect(l.lines.map((x) => x.top)).toEqual([0, 130, 260]);
    expect(l.blockW).toBeCloseTo(368);
    expect(l.blockH).toBeCloseTo(390);
  });

  it("wraps long text to the frame instead of running off it", () => {
    const l = layoutText({ ...box, text: "word ".repeat(40).trim() }, 1000, measurerFor);
    expect(l.lines.length).toBeGreaterThan(3);
    expect(l.blockW).toBeLessThanOrEqual(1000 * MAX_BLOCK + 1e-6);
    for (const line of l.lines) expect(line.right - line.left).toBeLessThanOrEqual(1000 * MAX_BLOCK + 1e-6);
  });

  it("caps the number of lines", () => {
    const l = layoutText({ ...box, text: Array.from({ length: 60 }, (_, i) => `line ${i}`).join("\n") }, 1000, measurerFor);
    expect(l.lines).toHaveLength(MAX_LINES);
  });

  it("a blank line has no background and the lines around it stay separate", () => {
    const l = layoutText({ ...box, text: "ab\n\ncd" }, 1000, measurerFor);
    expect(l.lines.map((x) => x.blank)).toEqual([false, true, false]);
    expect(l.lines[1].right - l.lines[1].left).toBe(0);
    expect(l.blockH).toBeCloseTo(390);
  });

  it("an empty text still has a small block, so it can be found and selected", () => {
    const l = layoutText({ ...box, text: "   " }, 1000, measurerFor);
    expect(l.lines.every((x) => x.blank)).toBe(true);
    expect(l.blockW).toBeGreaterThan(0);
    expect(l.blockH).toBeGreaterThan(0);
  });

  it("leaves room for a shadow only when there is no background, and the image has even sides", () => {
    const withBg = layoutText(box, 1001, measurerFor);
    expect(withBg.margin).toBe(0);
    const plain = layoutText({ ...box, background: null }, 1001, measurerFor);
    expect(plain.margin).toBeGreaterThan(0);
    for (const l of [withBg, plain]) {
      expect(l.imgW % 2).toBe(0);
      expect(l.imgH % 2).toBe(0);
      expect(l.imgW).toBeGreaterThanOrEqual(l.blockW + 2 * l.margin - 1e-6);
      // The block is centred in the image.
      expect(l.offX * 2 + l.blockW).toBeCloseTo(l.imgW);
      expect(l.offY * 2 + l.blockH).toBeCloseTo(l.imgH);
    }
  });

  it("survives silly sizes and frames", () => {
    for (const [size, frame] of [[0, 1000], [-5, 1000], [999, 1000], [Number.NaN, 1000], [10, 0], [10, 1]] as const) {
      const l = layoutText({ ...box, size }, frame, measurerFor);
      expect(Number.isFinite(l.blockW) && Number.isFinite(l.imgW) && l.imgW > 0, `${size} ${frame}`).toBe(true);
    }
  });
});

describe("the background block", () => {
  const lines = (spec: [number, number][], lineH = 100) => spec.map(([left, right], i) => ({ text: "x", blank: left === right, textW: 0, left, right, top: i * lineH }));

  it("one line is a plain rectangle", () => {
    expect(backgroundShapes(lines([[0, 200]]), 100)).toEqual([[[200, 0], [200, 100], [0, 100], [0, 0]]]);
  });

  it("lines of the same width are one rectangle, however many", () => {
    const shapes = backgroundShapes(lines([[0, 200], [0, 200], [0, 200]]), 100);
    expect(shapes).toEqual([[[200, 0], [200, 300], [0, 300], [0, 0]]]);
  });

  it("a longer line below a shorter one makes a step, on both sides", () => {
    // Centred: 100..300 over 0..400.
    const [shape] = backgroundShapes(lines([[100, 300], [0, 400]]), 100);
    expect(shape).toEqual([[300, 0], [300, 100], [400, 100], [400, 200], [0, 200], [0, 100], [100, 100], [100, 0]]);
  });

  it("a shorter line below a longer one steps in", () => {
    const [shape] = backgroundShapes(lines([[0, 400], [100, 300]]), 100);
    expect(shape).toEqual([[400, 0], [400, 100], [300, 100], [300, 200], [100, 200], [100, 100], [0, 100], [0, 0]]);
  });

  it("left-aligned lines share a straight left edge", () => {
    const [shape] = backgroundShapes(lines([[0, 150], [0, 300], [0, 200]]), 100);
    expect(shape).toEqual([[150, 0], [150, 100], [300, 100], [300, 200], [200, 200], [200, 300], [0, 300], [0, 0]]);
  });

  it("a blank line splits the background into separate blocks", () => {
    const shapes = backgroundShapes(lines([[0, 200], [0, 0], [0, 100]]), 100);
    expect(shapes).toHaveLength(2);
    expect(shapes[0]).toEqual([[200, 0], [200, 100], [0, 100], [0, 0]]);
    expect(shapes[1]).toEqual([[100, 200], [100, 300], [0, 300], [0, 200]]);
    expect(backgroundShapes(lines([[0, 0]]), 100)).toEqual([]);
    expect(backgroundShapes([], 100)).toEqual([]);
  });

  it("simplify removes repeats and straight runs, but keeps real corners", () => {
    const pts: Pt[] = [[0, 0], [10, 0], [20, 0], [20, 0], [20, 10], [20, 20], [0, 20], [0, 10]];
    expect(simplify(pts)).toEqual([[0, 0], [20, 0], [20, 20], [0, 20]]);
    expect(simplify([[0, 0], [5, 0], [5, 5]])).toHaveLength(3);
  });

  it("corners are rounded by the full radius, or by half of a short side", () => {
    const rect: Pt[] = [[200, 0], [200, 100], [0, 100], [0, 0]];
    expect(cornerRadii(rect, 30)).toEqual([30, 30, 30, 30]);
    // A step of only 10 px can't take a 30 px radius on both of its corners: 5 each.
    const stepped: Pt[] = [[300, 0], [300, 100], [310, 100], [310, 200], [0, 200], [0, 0]];
    const radii = cornerRadii(stepped, 30);
    expect(radii[1]).toBe(5);
    expect(radii[2]).toBe(5);
    expect(radii[0]).toBe(30);
    expect(radii.every((r) => r >= 0 && r <= 30)).toBe(true);
    expect(cornerRadii(rect, 500)).toEqual([50, 50, 50, 50]);
  });
});

describe("placing", () => {
  const layout = { blockW: 200, blockH: 100 };

  it("keeps the whole block inside the frame", () => {
    expect(clampCenter(0.5, 0.5, layout, 1000, 1000)).toEqual({ cx: 0.5, cy: 0.5 });
    expect(clampCenter(0, 0, layout, 1000, 1000)).toEqual({ cx: 0.1, cy: 0.05 });
    expect(clampCenter(1, 1, layout, 1000, 1000)).toEqual({ cx: 0.9, cy: 0.95 });
    expect(clampCenter(-4, 7, layout, 1000, 500)).toEqual({ cx: 0.1, cy: 0.9 });
  });

  it("centres a block that is bigger than the frame, and copes with nonsense", () => {
    expect(clampCenter(0.1, 0.9, { blockW: 2000, blockH: 3000 }, 1000, 1000)).toEqual({ cx: 0.5, cy: 0.5 });
    expect(clampCenter(Number.NaN, Number.NaN, layout, 1000, 1000)).toEqual({ cx: 0.5, cy: 0.5 });
  });

  const lay = layoutText({ text: "hello", bold: true, align: "center", size: 10, background: "#000" }, 1000, measurerFor);

  it("puts the image's top-left on a whole pixel", () => {
    const pos = imagePosition({ cx: 0.5, cy: 0.5 }, lay, 1000, 2000);
    expect(Number.isInteger(pos.left) && Number.isInteger(pos.top)).toBe(true);
    // The block's centre lands on the frame's centre (within the half pixel of rounding).
    expect(pos.left + lay.offX + lay.blockW / 2).toBeCloseTo(500, 0);
    expect(pos.top + lay.offY + lay.blockH / 2).toBeCloseTo(1000, 0);
  });

  it("expresses the image for the exporter's overlay filter", () => {
    const pos = { left: 100, top: 400 };
    const o = overlayFor(lay, pos, 1000, 2000);
    expect(o.nx).toBe(0.1);
    expect(o.ny).toBe(0.2);
    // The overlay is scaled to imgW px, i.e. exactly the pixels that were drawn.
    expect(Math.round(o.scale * 1000)).toBe(lay.imgW);
    expect(o.content.l).toBeCloseTo(lay.offX / lay.imgW);
    expect(o.content.r).toBeCloseTo((lay.offX + lay.blockW) / lay.imgW);
    expect(o.content.t).toBeCloseTo(lay.offY / lay.imgH);
    expect(o.content.b).toBeCloseTo((lay.offY + lay.blockH) / lay.imgH);
    // A block with no shadow margin fills the image: nothing to clamp.
    expect(lay.margin).toBe(0);
  });

  it("resizing by the corner grip scales the font with the width", () => {
    expect(sizeAfterResize(8, 400, 0)).toBe(8);
    expect(sizeAfterResize(8, 400, 400)).toBe(16);
    expect(sizeAfterResize(8, 400, -200)).toBe(4);
    expect(sizeAfterResize(8, 400, 100000)).toBe(MAX_SIZE);
    expect(sizeAfterResize(8, 400, -100000)).toBe(MIN_SIZE);
    expect(sizeAfterResize(8, 0, 10)).toBeGreaterThanOrEqual(MIN_SIZE);
  });
});
