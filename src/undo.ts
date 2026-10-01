/**
 * Undo / redo for everything you can change on the current video: crop, aspect,
 * trim, quality and the watermark list (placement, size, opacity, text, which are on).
 *
 * Instead of hooking every control, changes are noticed through the store: after a
 * short pause with no further change the state is snapshotted. A drag or a typing
 * burst therefore becomes ONE undo step.
 */
import { convertFileSrc } from "@tauri-apps/api/core";
import { renderText } from "./lib/textImage";
import { store, type ActiveWatermark } from "./store";
import { scheduleTextSave } from "./watermarkOps";
import { debounce } from "./ui/dom";

const MAX_STEPS = 100;
const SETTLE_MS = 350;

interface Snapshot {
  crop: { x: number; y: number; w: number; h: number };
  aspect: string;
  trimStart: number;
  trimEnd: number;
  quality: number;
  watermarks: ActiveWatermark[];
  selected: string | null;
}

let states: string[] = [];
let index = -1;
let restoring = false;
const listeners = new Set<() => void>();

/** Text images are large; they're regenerated from the style on restore instead of stored. */
function serialize(): string | null {
  const v = store.video;
  if (!v) return null;
  const snap: Snapshot = {
    crop: v.crop,
    aspect: v.aspect,
    trimStart: v.trimStart,
    trimEnd: v.trimEnd,
    quality: v.quality,
    watermarks: v.watermarks.map((w) => ({ ...w, url: w.text ? "" : w.url })),
    selected: store.selectedWatermark,
  };
  return JSON.stringify(snap);
}

function notify(): void {
  store.emit("undo");
  listeners.forEach((f) => f());
}

export const canUndo = () => index > 0;
export const canRedo = () => index >= 0 && index < states.length - 1;
export function onUndoChange(fn: () => void): void {
  listeners.add(fn);
}

/** Start a fresh history (a new video was opened or closed). */
function reset(): void {
  states = [];
  index = -1;
  const s = serialize();
  if (s) {
    states = [s];
    index = 0;
  }
  notify();
}

const settle = debounce(() => {
  if (restoring) return;
  const s = serialize();
  if (!s || s === states[index]) return;
  states = states.slice(0, index + 1);
  states.push(s);
  if (states.length > MAX_STEPS) states.shift();
  index = states.length - 1;
  notify();
}, SETTLE_MS);

function restore(json: string): void {
  const v = store.video;
  if (!v) return;
  const snap = JSON.parse(json) as Snapshot;
  restoring = true;
  try {
    v.crop = snap.crop;
    v.aspect = snap.aspect;
    v.trimStart = snap.trimStart;
    v.trimEnd = snap.trimEnd;
    v.quality = snap.quality;
    // Watermarks deleted from the library since can't come back.
    const available = new Set(store.library.map((e) => e.id));
    v.watermarks = snap.watermarks
      .filter((w) => available.has(w.id))
      .map((w) => {
        if (!w.text) return { ...w, url: convertFileSrc(w.path) };
        const r = renderText(w.text);
        scheduleTextSave(w, r.base64); // the exporter reads the saved PNG
        return { ...w, url: r.dataUrl, aspect: r.height / r.width, content: r.content };
      });
    store.selectedWatermark = v.watermarks.some((w) => w.id === snap.selected) ? snap.selected : null;
    store.emit("crop", "trim", "watermarks", "selection", "quality");
  } finally {
    restoring = false;
  }
}

export function undo(): void {
  if (store.busy || !canUndo()) return;
  settle.cancel();
  index--;
  restore(states[index]);
  notify();
}

export function redo(): void {
  if (store.busy || !canRedo()) return;
  settle.cancel();
  index++;
  restore(states[index]);
  notify();
}

export function initUndo(): void {
  store.on("video", reset);
  store.on(["crop", "trim", "watermarks", "quality"], () => !restoring && settle());
}
