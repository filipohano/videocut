/**
 * Operations on watermarks shared by the preview overlay and the side panel.
 * Placement lives in the store; changes are written back to the library
 * (debounced) so every watermark remembers its own position, size and opacity.
 */
import { convertFileSrc } from "@tauri-apps/api/core";
import { api, errorMessage, type WatermarkEntry } from "./api";
import { debounce } from "./ui/dom";
import { toast } from "./ui/toast";
import { clamp } from "./lib/format";
import { clampPlacement, cornerPlacement, type Corner } from "./lib/watermarks";
import { store, type ActiveWatermark } from "./store";

function cropSize() {
  const v = store.video;
  return v ? { w: v.crop.w, h: v.crop.h } : { w: 1, h: 1 };
}

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

export function addToVideo(entry: WatermarkEntry, opts: { snapToCorner?: boolean } = {}): void {
  const v = store.video;
  if (!v) return;
  if (activeWatermark(entry.id)) {
    select(entry.id);
    return;
  }
  const crop = cropSize();
  const placed = opts.snapToCorner
    ? cornerPlacement("br", entry.scale, entry.aspect, crop)
    : clampPlacement({ nx: entry.nx, ny: entry.ny, scale: entry.scale }, entry.aspect, crop);
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

export function updateActive(
  id: string,
  patch: Partial<{ nx: number; ny: number; scale: number; opacity: number }>,
): void {
  const wm = activeWatermark(id);
  if (!wm) return;
  const next = clampPlacement(
    { nx: patch.nx ?? wm.nx, ny: patch.ny ?? wm.ny, scale: patch.scale ?? wm.scale },
    wm.aspect,
    cropSize(),
  );
  Object.assign(wm, next);
  if (patch.opacity !== undefined) wm.opacity = clamp(patch.opacity, 0, 1);
  persist(wm);
  store.emit("watermarks");
}

export function applyCorner(id: string, corner: Corner): void {
  const wm = activeWatermark(id);
  if (!wm) return;
  const p = cornerPlacement(corner, wm.scale, wm.aspect, cropSize());
  updateActive(id, p);
}

/** Keep every watermark inside the frame after the crop changed. */
export function reclampAll(): void {
  const v = store.video;
  if (!v) return;
  let changed = false;
  for (const wm of v.watermarks) {
    const next = clampPlacement({ nx: wm.nx, ny: wm.ny, scale: wm.scale }, wm.aspect, cropSize());
    if (next.nx !== wm.nx || next.ny !== wm.ny || next.scale !== wm.scale) {
      Object.assign(wm, next);
      persist(wm);
      changed = true;
    }
  }
  if (changed) store.emit("watermarks");
}

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
