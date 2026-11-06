/**
 * "Remove text": finding out whether the open video stands still, and filling in the text inside
 * a box the user drew (with the AI add-on). The filled areas live in the store (and the undo
 * history) and are laid over the picture by the preview and the exporter.
 */
import { convertFileSrc } from "@tauri-apps/api/core";
import { api, errorMessage, type PixelRect } from "./api";
import { store, type Fill } from "./store";
import { toast } from "./ui/toast";

export const MODULE_ID = "remove-text";
const LETTERS_KEY = "fillerncut.fillLetters";

export const moduleInstalled = () => store.modules.some((m) => m.id === MODULE_ID && m.installed);

// ───────── the drawing mode and its options (not part of the edit) ─────────
export const fillUi = {
  /** Dragging on the preview draws a box to remove text from. */
  drawing: false,
  /** A box is being filled right now. */
  working: null as PixelRect | null,
  /** Only the caption's letters (white or black) are filled, not the whole box. */
  letters: (() => {
    try {
      return localStorage.getItem(LETTERS_KEY) !== "0";
    } catch {
      return true;
    }
  })(),
  /** Hide the fills for a moment, to compare. */
  showOriginal: false,
};

export function setDrawing(on: boolean): void {
  if (fillUi.drawing === on) return;
  fillUi.drawing = on;
  store.emit("fills");
}

export function setLetters(on: boolean): void {
  fillUi.letters = on;
  try {
    localStorage.setItem(LETTERS_KEY, on ? "1" : "0");
  } catch {
    /* not remembered */
  }
  store.emit("fills");
}

export function setShowOriginal(on: boolean): void {
  fillUi.showOriginal = on;
  store.emit("fills");
}

// ───────── does the picture stand still? ─────────
let checking: string | null = null;

/** Find out (once per video) whether text can be removed from it. Photos always can. */
export async function checkStills(): Promise<void> {
  const v = store.video;
  if (!v || v.stills || !moduleInstalled() || checking === v.path) return;
  checking = v.path;
  try {
    const result = await api.analyzeStills(v.path, v.info.duration);
    if (store.video === v) {
      v.stills = result;
      store.emit("stills");
    }
  } catch (e) {
    if (store.video === v) {
      v.stills = { kind: "moving" };
      store.emit("stills");
    }
    console.warn("Couldn't check whether the picture stands still", e);
  } finally {
    checking = null;
  }
}

/** The photo of a slideshow at time `t` (index, start, end), or null between photos. */
export function slideAt(t: number): { index: number; start: number; end: number } | null {
  const s = store.video?.stills;
  if (s?.kind !== "slides") return null;
  const i = s.segments.findIndex((seg) => t >= seg.start - 0.01 && t < seg.end - 0.01);
  return i < 0 ? null : { index: i, ...s.segments[i] };
}

/**
 * What a new fill applies to, for the picture shown now: the time to take the picture from
 * and the window it covers (null = all of it). Null when nothing can be filled here.
 */
export function currentTarget(): { time: number | null; start: number | null; end: number | null } | null {
  const v = store.video;
  const s = v?.stills;
  if (!v || !s || s.kind === "moving") return null;
  if (v.info.isImage) return { time: null, start: null, end: null };
  if (s.kind === "still") return { time: Math.min(v.info.duration / 2, 1), start: null, end: null };
  const slide = slideAt(store.playhead);
  if (!slide) return null;
  return { time: (slide.start + slide.end) / 2, start: slide.start, end: slide.end };
}

/** Fills that show on the picture of `target` (for a slideshow: on that photo). */
export function fillsFor(target: { start: number | null }): Fill[] {
  return store.video?.fills.filter((f) => f.start === target.start) ?? [];
}

/** Is this fill on the picture at time `t`? */
export function fillVisibleAt(f: Fill, t: number): boolean {
  return f.start === null || f.end === null || (t >= f.start - 0.01 && t < f.end - 0.01);
}

const newId = () => (globalThis.crypto?.randomUUID?.() ?? `f${Date.now().toString(36)}${Math.random().toString(36).slice(2, 7)}`);

/** Remove the text inside `box` (pixels of the full frame) on the picture shown now. */
export async function fillBox(box: PixelRect): Promise<void> {
  const v = store.video;
  const target = currentTarget();
  if (!v || !target || store.busy) return;
  fillUi.working = box;
  store.setBusy(true);
  store.emit("fills");
  try {
    const under = fillsFor(target).map((f) => ({ path: f.path, x: f.x, y: f.y, start: f.start, end: f.end }));
    const r = await api.inpaintFill(v.path, target.time, [box], fillUi.letters, under);
    if (store.video !== v) return;
    v.fills.push({ id: newId(), path: r.path, url: convertFileSrc(r.path), x: r.x, y: r.y, w: r.w, h: r.h, start: target.start, end: target.end });
    if (r.wholeBox && fillUi.letters) {
      toast("No white or black letters were found in that box, so all of it was filled in.", { kind: "info" });
    }
  } catch (e) {
    toast(errorMessage(e), { kind: "error", timeout: 10000 });
  } finally {
    fillUi.working = null;
    store.setBusy(false);
    store.emit("fills");
  }
}

export function removeFill(id: string): void {
  const v = store.video;
  if (!v) return;
  v.fills = v.fills.filter((f) => f.id !== id);
  store.emit("fills");
}

// A video is checked as soon as it is open (and again once the add-on arrives).
store.on(["video", "modules"], () => void checkStills());
store.on("video", () => {
  fillUi.drawing = false;
  fillUi.showOriginal = false;
});
