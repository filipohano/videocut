import { describe, expect, it } from "vitest";
import { DEFAULT_TEXT_STYLE, FONT_CHOICES, MAX_CHARS, MAX_LINES, fontCss, textLines } from "./textImage";

describe("textLines", () => {
  it("splits lines and caps their number and length", () => {
    expect(textLines("a\nb")).toEqual(["a", "b"]);
    expect(textLines("a\r\nb")).toEqual(["a", "b"]);
    expect(textLines(Array.from({ length: 20 }, (_, i) => `l${i}`).join("\n"))).toHaveLength(MAX_LINES);
    expect(textLines("x".repeat(5000))[0]).toHaveLength(MAX_CHARS);
  });
  it("never returns nothing to draw", () => {
    expect(textLines("")).toEqual(["…"]);
    expect(textLines(" \n  ")).toEqual(["…"]);
  });
});

describe("fontCss", () => {
  it("builds a safe font string with fallbacks", () => {
    expect(fontCss(DEFAULT_TEXT_STYLE, 100)).toBe('700 100px "Helvetica Neue", "Helvetica Neue", Arial, sans-serif');
    expect(fontCss({ ...DEFAULT_TEXT_STYLE, bold: false, italic: true, fontFamily: "Impact" }, 50)).toContain('italic 400 50px "Impact"');
  });
  it("strips quotes so a font name can't break out of the CSS string", () => {
    expect(fontCss({ ...DEFAULT_TEXT_STYLE, fontFamily: 'Evil", sans-serif; x:"' }, 10)).not.toContain('Evil"');
    expect(fontCss({ ...DEFAULT_TEXT_STYLE, fontFamily: "   " }, 10)).toContain('"Helvetica Neue"');
  });
});

describe("font list", () => {
  it("includes the default font", () => {
    expect(FONT_CHOICES).toContain(DEFAULT_TEXT_STYLE.fontFamily);
  });
});
