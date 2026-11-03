/**
 * Operations on the text boxes of the open video, shared by the preview and the Text panel.
 * Text boxes live in the store (and in the undo history); only their *looks* are saved, as
 * presets, so there is nothing to persist for the boxes themselves.
 */
import { api, errorMessage } from "./api";
import {
  DEFAULT_STYLE,
  MAX_TEXTS,
  cleanText,
  clampCenter,
  isEmptyText,
  newTextBox,
  sanitizeStyle,
  type TextBox,
  type TextBoxStyle,
  type TextLayout,
} from "./lib/textBox";
import { layoutFor, onTextFontLoaded } from "./lib/textRender";
import { store } from "./store";
import { toast } from "./ui/toast";

export { MAX_TEXTS };

const pickStyle = (b: TextBoxStyle): TextBoxStyle => ({ color: b.color, background: b.background, bold: b.bold, align: b.align, size: b.size });

/** The look the next new text starts with: whatever was used last. */
let lastStyle: TextBoxStyle = { ...DEFAULT_STYLE };

function frameWidth(): number {
  return store.video?.crop.w ?? 1;
}
function frameHeight(): number {
  return store.video?.crop.h ?? 1;
}

// ───────── layout (measured once, until something that changes it) ─────────
const layouts = new Map<string, TextLayout>();
const layoutKey = (b: TextBox, w: number) => `${w}|${b.size}|${b.bold ? 1 : 0}|${b.align}|${b.background === null ? 0 : 1}|${b.text}`;

/** Where the lines break and how big the block is. The same object is returned until the box changes. */
export function layoutOf(box: TextBox): TextLayout {
  const w = frameWidth();
  const key = `${box.id}|${layoutKey(box, w)}`;
  let layout = layouts.get(key);
  if (!layout) {
    // Keep one entry per box, so typing doesn't leave a pile of old layouts behind.
    for (const k of layouts.keys()) if (k.startsWith(`${box.id}|`)) layouts.delete(k);
    layout = layoutFor(box, w);
    layouts.set(key, layout);
  }
  return layout;
}

// The font arrives a moment after start-up: text measured before that is measured again.
onTextFontLoaded(() => {
  layouts.clear();
  if (store.video) {
    reclampTexts();
    store.emit("texts");
  }
});

// A different video: nothing measured for the old one is of any use.
store.on("video", () => layouts.clear());

export function textById(id: string | null): TextBox | undefined {
  return id ? store.video?.texts.find((t) => t.id === id) : undefined;
}

// ───────── selection ─────────
export function selectText(id: string | null): void {
  if (store.selectedText === id && (id === null || store.selectedWatermark === null)) return;
  store.selectedText = id;
  if (id !== null) store.selectedWatermark = null;
  store.emit("selection");
}

/** Set when a text was just added, so the panel can put the cursor in it. */
let focusFor: string | null = null;
export function takeTextBoxFocus(id: string): boolean {
  const hit = focusFor === id;
  if (hit) focusFor = null;
  return hit;
}

// ───────── adding / removing ─────────
export function addText(style?: Partial<TextBoxStyle>): void {
  const v = store.video;
  if (!v) return;
  if (v.texts.length >= MAX_TEXTS) {
    toast(`That's a lot of text. Remove one before adding another (max ${MAX_TEXTS}).`, { kind: "info" });
    return;
  }
  const base = style ?? textById(store.selectedText) ?? lastStyle;
  const box = newTextBox(pickStyle(sanitizeStyle(base)));
  // A new box doesn't land exactly on top of another one.
  for (let step = 0; step < 6 && v.texts.some((t) => Math.abs(t.cx - box.cx) < 0.02 && Math.abs(t.cy - box.cy) < 0.04); step++) {
    box.cy = Math.min(0.9, box.cy + 0.1);
  }
  Object.assign(box, clampCenter(box.cx, box.cy, layoutOf(box), frameWidth(), frameHeight()));
  v.texts.push(box);
  lastStyle = pickStyle(box);
  focusFor = box.id;
  selectText(box.id);
  store.emit("texts");
}

export function removeText(id: string): void {
  const v = store.video;
  if (!v) return;
  v.texts = v.texts.filter((t) => t.id !== id);
  if (store.selectedText === id) {
    store.selectedText = null;
    store.emit("selection");
  }
  store.emit("texts");
}

// ───────── changing ─────────
/** Change a text box. The block grows or shrinks around its centre, and stays inside the frame. */
export function updateText(id: string, patch: Partial<TextBoxStyle> & { text?: string }): void {
  const box = textById(id);
  if (!box) return;
  const style = sanitizeStyle({ ...box, ...patch });
  const text = patch.text === undefined ? box.text : cleanText(patch.text);
  Object.assign(box, style, { text });
  Object.assign(box, clampCenter(box.cx, box.cy, layoutOf(box), frameWidth(), frameHeight()));
  if (patch.text === undefined || !isEmptyText(text)) lastStyle = pickStyle(box);
  store.emit("texts");
}

/** Apply a saved look to a box, keeping its words and its place. */
export function applyStyle(id: string, style: TextBoxStyle): void {
  updateText(id, pickStyle(sanitizeStyle(style)));
}

export function moveText(id: string, cx: number, cy: number): void {
  const box = textById(id);
  if (!box) return;
  Object.assign(box, clampCenter(cx, cy, layoutOf(box), frameWidth(), frameHeight()));
  store.emit("texts");
}

/** Dragging the corner grip: a new size, with the block's top-left corner (in frame px) staying where it was. */
export function resizeKeepingCorner(id: string, size: number, topLeft: { x: number; y: number }): void {
  const box = textById(id);
  if (!box) return;
  box.size = sanitizeStyle({ ...box, size }).size;
  const layout = layoutOf(box);
  Object.assign(
    box,
    clampCenter((topLeft.x + layout.blockW / 2) / frameWidth(), (topLeft.y + layout.blockH / 2) / frameHeight(), layout, frameWidth(), frameHeight()),
  );
  store.emit("texts");
}

/** The crop changed: the frame width (so the letter sizes) and the room to move both changed. */
export function reclampTexts(): void {
  const v = store.video;
  if (!v || v.texts.length === 0) return;
  let changed = false;
  for (const box of v.texts) {
    const next = clampCenter(box.cx, box.cy, layoutOf(box), frameWidth(), frameHeight());
    if (next.cx !== box.cx || next.cy !== box.cy) {
      Object.assign(box, next);
      changed = true;
    }
  }
  if (changed) store.emit("texts");
}

// ───────── saved looks ─────────
export async function refreshTextStyles(): Promise<void> {
  store.textPresets = await api.textPresetsList();
  store.emit("textStyles");
}

export async function saveStyle(name: string, from: TextBoxStyle): Promise<boolean> {
  try {
    await api.textPresetsSave(name, pickStyle(from));
    await refreshTextStyles();
    toast(`Saved the style “${name.trim()}”`, { kind: "success" });
    return true;
  } catch (e) {
    toast(errorMessage(e), { kind: "error" });
    return false;
  }
}

export async function deleteStyle(id: string): Promise<void> {
  try {
    await api.textPresetsRemove(id);
    await refreshTextStyles();
  } catch (e) {
    toast(errorMessage(e), { kind: "error" });
  }
}
