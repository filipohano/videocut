/**
 * Operations on watermarks shared by the preview overlay and the side panel.
 * Placement lives in the store; changes are written back to the library
 * (debounced) so every watermark remembers its own position, size and opacity.
 */
import { convertFileSrc } from "@tauri-apps/api/core";
import { api, errorMessage, type TextStyle, type WatermarkEntry } from "./api";
import { clamp } from "./lib/format";
import { DEFAULT_TEXT_STYLE, renderText } from "./lib/textImage";
import {
  clampPlacement,
  cornerPlacement,
  rescaleAroundCenter,
  rescaleKeepingCorner,
  scaleFromSizePct,
  type Corner,
} from "./lib/watermarks";
import { store, type ActiveWatermark } from "./store";
import { debounce } from "./ui/dom";
import { toast } from "./ui/toast";

/** Most watermarks on one video; more than this is almost certainly a mistake. */
export const MAX_ACTIVE = 8;

function cropSize() {
  const v = store.video;
  return v ? { w: v.crop.w, h: v.crop.h } : { w: 1, h: 1 };
}

// ───────── persistence of placement ─────────
const persisters = new Map<string, (wm: ActiveWatermark) => void>();

function persist(wm: ActiveWatermark): void {
  let fn = persisters.get(wm.id);
  if (!fn) {
    fn = debounce((w: ActiveWatermark) => {
      const patch = { nx: w.nx, ny: w.ny, scale: w.scale, opacity: w.opacity };
      api
        .libraryUpdate(w.id, patch)
        .then((saved) => {
          const i = store.library.findIndex((e) => e.id === saved.id);
          if (i >= 0) store.library[i] = saved;
        })
        .catch((e) => console.warn("Couldn't save watermark placement", e));
    }, 400);
    persisters.set(wm.id, fn);
  }
  fn(wm);
}

export function activeWatermark(id: string): ActiveWatermark | undefined {
  return store.video?.watermarks.find((w) => w.id === id);
}

export function select(id: string | null): void {
  if (store.selectedWatermark === id) return;
  store.selectedWatermark = id;
  store.emit("selection");
}

// ───────── adding / removing ─────────
export function addToVideo(entry: WatermarkEntry, opts: { snapToCorner?: boolean; sizePct?: number } = {}): void {
  const v = store.video;
  if (!v) return;
  if (activeWatermark(entry.id)) {
    select(entry.id);
    return;
  }
  if (v.watermarks.length >= MAX_ACTIVE) {
    toast(`That's a lot of watermarks. Remove one before adding another (max ${MAX_ACTIVE}).`, { kind: "info" });
    return;
  }
  const crop = cropSize();
  const scale = opts.sizePct ? scaleFromSizePct(opts.sizePct, entry.content) : entry.scale;
  const placed = opts.snapToCorner
    ? cornerPlacement("br", scale, entry.aspect, crop, entry.content)
    : clampPlacement({ nx: entry.nx, ny: entry.ny, scale }, entry.aspect, crop, entry.content);
  const wm: ActiveWatermark = { ...entry, ...placed, url: convertFileSrc(entry.path) };
  v.watermarks.push(wm);
  persist(wm);
  store.selectedWatermark = wm.id;
  store.emit("watermarks", "selection");
}

export function removeFromVideo(id: string): void {
  const v = store.video;
  if (!v) return;
  v.watermarks = v.watermarks.filter((w) => w.id !== id);
  if (store.selectedWatermark === id) store.selectedWatermark = null;
  store.emit("watermarks", "selection");
}

// ───────── placement changes ─────────
function apply(wm: ActiveWatermark, p: { nx: number; ny: number; scale: number }): void {
  Object.assign(wm, p);
  persist(wm);
  store.emit("watermarks");
}

/** Move (visible content stays inside the frame; transparent margin may leave it). */
export function moveTo(id: string, nx: number, ny: number): void {
  const wm = activeWatermark(id);
  if (!wm) return;
  apply(wm, clampPlacement({ nx, ny, scale: wm.scale }, wm.aspect, cropSize(), wm.content));
}

/** Size slider: content width as % of the frame, grown around the centre. */
export function setSizePct(id: string, pct: number): void {
  const wm = activeWatermark(id);
  if (!wm) return;
  apply(wm, rescaleAroundCenter(wm, scaleFromSizePct(pct, wm.content), wm.aspect, cropSize(), wm.content));
}

/** Corner grip: set the scale directly, keeping the content's top-left corner. */
export function setScaleKeepingCorner(id: string, scale: number): void {
  const wm = activeWatermark(id);
  if (!wm) return;
  apply(wm, rescaleKeepingCorner(wm, scale, wm.aspect, cropSize(), wm.content));
}

export function setOpacity(id: string, opacity: number): void {
  const wm = activeWatermark(id);
  if (!wm) return;
  wm.opacity = clamp(opacity, 0, 1);
  persist(wm);
  store.emit("watermarks");
}

export function applyCorner(id: string, corner: Corner): void {
  const wm = activeWatermark(id);
  if (!wm) return;
  apply(wm, cornerPlacement(corner, wm.scale, wm.aspect, cropSize(), wm.content));
}

/** Keep every watermark's content inside the frame after the crop changed. */
export function reclampAll(): void {
  const v = store.video;
  if (!v) return;
  let changed = false;
  for (const wm of v.watermarks) {
    const next = clampPlacement({ nx: wm.nx, ny: wm.ny, scale: wm.scale }, wm.aspect, cropSize(), wm.content);
    if (next.nx !== wm.nx || next.ny !== wm.ny || next.scale !== wm.scale) {
      Object.assign(wm, next);
      persist(wm);
      changed = true;
    }
  }
  if (changed) store.emit("watermarks");
}

// ───────── library ─────────
export async function refreshLibrary(): Promise<void> {
  store.library = await api.libraryList();
  store.emit("library");
}

/** Import an image into the persistent library and put it on the current video. */
export async function importWatermark(path: string): Promise<void> {
  try {
    const entry = await api.libraryAdd(path);
    await refreshLibrary();
    if (store.video) addToVideo(entry, { snapToCorner: true });
    toast(`Added “${entry.name}” to your watermarks`, { kind: "success" });
  } catch (e) {
    toast(errorMessage(e), { kind: "error" });
  }
}

export async function deleteFromLibrary(id: string): Promise<void> {
  try {
    await api.libraryRemove(id);
    removeFromVideo(id);
    await refreshLibrary();
  } catch (e) {
    toast(errorMessage(e), { kind: "error" });
  }
}

// ───────── text watermarks ─────────
/** Set when a text watermark was just created so the panel can focus its text box. */
export let focusTextFor: string | null = null;
export function takeTextFocus(id: string): boolean {
  const hit = focusTextFor === id;
  if (hit) focusTextFor = null;
  return hit;
}

export async function createTextWatermark(): Promise<void> {
  if (!store.video) return;
  try {
    const rendered = renderText(DEFAULT_TEXT_STYLE);
    const entry = await api.libraryAddText(rendered.base64, { ...DEFAULT_TEXT_STYLE });
    await refreshLibrary();
    focusTextFor = entry.id;
    addToVideo(entry, { snapToCorner: true, sizePct: 40 });
  } catch (e) {
    toast(errorMessage(e), { kind: "error" });
  }
}

const textSaves = new Map<string, { timer: ReturnType<typeof setTimeout>; run: () => Promise<void> }>();

/** Edit a text watermark: the preview updates at once, the library copy a moment later. */
export function updateText(id: string, patch: Partial<TextStyle>): void {
  const wm = activeWatermark(id);
  if (!wm?.text) return;
  const before = renderText(wm.text);
  const style: TextStyle = { ...wm.text, ...patch };
  const after = renderText(style);

  // Keep the letters the same size when the image gets wider/narrower, and keep
  // the text's centre where it was.
  const scale = (wm.scale * before.width) / after.width;
  const oldBox = wm.content;
  const crop = cropSize();
  const cx = wm.nx + ((oldBox.l + oldBox.r) / 2) * wm.scale;
  const hOld = (wm.scale * crop.w * wm.aspect) / crop.h;
  const cy = wm.ny + ((oldBox.t + oldBox.b) / 2) * hOld;
  const aspect = after.height / after.width;
  const hNew = (scale * crop.w * aspect) / crop.h;
  const placed = clampPlacement(
    {
      scale,
      nx: cx - ((after.content.l + after.content.r) / 2) * scale,
      ny: cy - ((after.content.t + after.content.b) / 2) * hNew,
    },
    aspect,
    crop,
    after.content,
  );

  wm.text = style;
  wm.url = after.dataUrl;
  wm.aspect = aspect;
  wm.content = after.content;
  wm.name = style.text.split("\n").find((l) => l.trim())?.slice(0, 28) ?? "Text";
  Object.assign(wm, placed);
  persist(wm);
  store.emit("watermarks");
  scheduleTextSave(wm, after.base64);
}

function scheduleTextSave(wm: ActiveWatermark, base64: string): void {
  const prior = textSaves.get(wm.id);
  if (prior) clearTimeout(prior.timer);
  const run = async () => {
    textSaves.delete(wm.id);
    const style = wm.text;
    if (!style) return;
    try {
      const saved = await api.libraryReplaceText(wm.id, base64, style);
      // Adopt the backend's exact measurements and the stored file.
      wm.content = saved.content;
      wm.aspect = saved.aspect;
      wm.path = saved.path;
      const i = store.library.findIndex((e) => e.id === saved.id);
      if (i >= 0) store.library[i] = saved;
      store.emit("library");
    } catch (e) {
      toast(errorMessage(e), { kind: "error" });
    }
  };
  textSaves.set(wm.id, { timer: setTimeout(() => void run(), 600), run });
}

/** Make sure every edited text is on disk — the exporter reads the files. */
export async function flushTextSaves(): Promise<void> {
  const pending = [...textSaves.values()];
  pending.forEach((p) => clearTimeout(p.timer));
  await Promise.all(pending.map((p) => p.run()));
}
