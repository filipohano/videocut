/** The watermark library drop-down and the list of watermarks (images and text) on the current video. */
import { convertFileSrc } from "@tauri-apps/api/core";
import { ask, open } from "@tauri-apps/plugin-dialog";
import type { TextStyle } from "../api";
import { IMAGE_EXTENSIONS } from "../lib/links";
import { FONT_CHOICES, MAX_CHARS } from "../lib/textImage";
import { MIN_SIZE_PCT, maxSizePct, sizePctFromScale, type Corner } from "../lib/watermarks";
import { store, type ActiveWatermark } from "../store";
import {
  MAX_ACTIVE,
  addToVideo,
  applyCorner,
  createTextWatermark,
  deleteFromLibrary,
  importWatermark,
  removeFromVideo,
  select,
  setOpacity,
  setSizePct,
  takeTextFocus,
  updateText,
} from "../watermarkOps";
import { $, h } from "./dom";
import { bindRange, type BoundRange } from "./range";

const CORNERS: { key: Corner; title: string }[] = [
  { key: "tl", title: "Top left" },
  { key: "tr", title: "Top right" },
  { key: "c", title: "Centre" },
  { key: "bl", title: "Bottom left" },
  { key: "br", title: "Bottom right" },
];

const TRASH =
  '<svg viewBox="0 0 24 24" width="15" height="15" fill="none" stroke="currentColor" stroke-width="1.8" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true"><path d="M4 7h16M10 11v6M14 11v6M6 7l1 12a2 2 0 002 2h6a2 2 0 002-2l1-12M9 7V4h6v3"/></svg>';

function thumb(src: string, alt: string): HTMLElement {
  return h("span", { class: "thumb" }, h("img", { src, alt }));
}

export function initWatermarkPanel(): void {
  const menuBtn = $("#wm-menu-btn");
  const menu = $("#wm-menu");
  const active = $("#wm-active");

  // Fonts offered in the text editor (any other installed font can be typed in).
  const fontList = h("datalist", { id: "font-list" }, ...FONT_CHOICES.map((f) => h("option", { value: f })));
  document.body.append(fontList);

  // ───────── library drop-down ─────────
  function closeMenu(): void {
    menu.classList.add("hidden");
    menuBtn.setAttribute("aria-expanded", "false");
  }

  function renderMenu(): void {
    const onVideo = new Set(store.video?.watermarks.map((w) => w.id));
    const items: HTMLElement[] = [];
    if (store.library.length === 0) {
      items.push(h("div", { class: "menu-empty" }, "No saved watermarks yet. Add text or upload an image below."));
    }
    for (const entry of store.library) {
      const del = h("button", { class: "icon-btn", title: "Delete from library", "aria-label": `Delete ${entry.name}` });
      del.innerHTML = TRASH;
      del.addEventListener("click", async (e) => {
        e.stopPropagation();
        const ok = await ask(`Delete “${entry.name}” from your watermark library?`, { title: "Delete watermark", kind: "warning" });
        if (ok) await deleteFromLibrary(entry.id);
      });
      const item = h(
        "div",
        { class: "menu-item", role: "menuitem", tabindex: 0 },
        thumb(convertFileSrc(entry.path), entry.name),
        h("span", { class: "name" }, entry.name),
        entry.text && h("span", { class: "tag muted" }, "Text"),
        onVideo.has(entry.id) && h("span", { class: "tag" }, "On video"),
        del,
      );
      const choose = () => {
        addToVideo(entry);
        closeMenu();
      };
      item.addEventListener("click", choose);
      item.addEventListener("keydown", (e) => (e.key === "Enter" || e.key === " ") && (e.preventDefault(), choose()));
      items.push(item);
    }
    items.push(h("div", { class: "menu-sep" }));

    const actionItem = (label: string, run: () => Promise<void>) => {
      const el = h("div", { class: "menu-item action", role: "menuitem", tabindex: 0 }, h("span", { class: "name" }, label));
      const go = async () => {
        closeMenu();
        await run();
      };
      el.addEventListener("click", go);
      el.addEventListener("keydown", (e) => (e.key === "Enter" || e.key === " ") && (e.preventDefault(), go()));
      return el;
    };
    items.push(actionItem("Add text…", createTextWatermark));
    items.push(
      actionItem("Upload image…", async () => {
        const picked = await open({ multiple: false, filters: [{ name: "Image", extensions: IMAGE_EXTENSIONS }] });
        if (typeof picked === "string") await importWatermark(picked);
      }),
    );
    menu.replaceChildren(...items);
  }

  menuBtn.addEventListener("click", (e) => {
    e.stopPropagation();
    const opening = menu.classList.contains("hidden");
    if (opening) {
      renderMenu();
      menu.classList.remove("hidden");
    } else menu.classList.add("hidden");
    menuBtn.setAttribute("aria-expanded", String(opening));
  });
  document.addEventListener("click", (e) => {
    if (!menu.contains(e.target as Node)) closeMenu();
  });
  document.addEventListener("keydown", (e) => e.key === "Escape" && closeMenu());
  store.on(["library", "watermarks"], () => !menu.classList.contains("hidden") && renderMenu());

  // ───────── watermarks on this video ─────────
  interface Row {
    el: HTMLElement;
    thumbImg: HTMLImageElement;
    nameEl: HTMLElement;
    size: HTMLInputElement;
    sizeRange: BoundRange;
    sizeOut: HTMLOutputElement;
    opacityRange: BoundRange;
    opacityOut: HTMLOutputElement;
    text?: { area: HTMLTextAreaElement; font: HTMLInputElement };
  }
  const rows = new Map<string, Row>();

  function textControls(wm: ActiveWatermark): { el: HTMLElement; area: HTMLTextAreaElement; font: HTMLInputElement } {
    const style = (): TextStyle => (store.video?.watermarks.find((w) => w.id === wm.id)?.text ?? wm.text!) as TextStyle;
    const patch = (p: Partial<TextStyle>) => updateText(wm.id, p);

    const area = h("textarea", {
      class: "text-input",
      rows: 2,
      maxLength: MAX_CHARS,
      placeholder: "Type your text…",
      "aria-label": "Watermark text",
    });
    area.value = wm.text!.text;
    area.addEventListener("input", () => patch({ text: area.value }));

    const font = h("input", { type: "text", list: "font-list", class: "font-input", "aria-label": "Font", spellcheck: false });
    font.value = wm.text!.fontFamily;
    font.addEventListener("change", () => font.value.trim() && patch({ fontFamily: font.value.trim() }));
    font.addEventListener("keydown", (e) => e.key === "Enter" && font.blur());

    const toggle = (label: string, title: string, key: "bold" | "italic", extra = "") => {
      const b = h("button", { class: `btn small toggle ${extra}`, type: "button", title, "aria-pressed": String(style()[key]) }, label);
      b.addEventListener("click", () => {
        const next = !style()[key];
        b.setAttribute("aria-pressed", String(next));
        patch({ [key]: next });
      });
      return b;
    };
    const aligns = (["left", "center", "right"] as const).map((a) => {
      const b = h("button", { class: "btn small toggle", type: "button", title: `Align ${a}`, "aria-pressed": String(style().align === a) }, a === "left" ? "⬅" : a === "right" ? "➡" : "↔");
      b.addEventListener("click", () => {
        aligns.forEach((o) => o.setAttribute("aria-pressed", String(o === b)));
        patch({ align: a });
      });
      return b;
    });

    const color = h("input", { type: "color", class: "color", "aria-label": "Text colour", value: wm.text!.color });
    color.addEventListener("input", () => patch({ color: color.value }));
    const outline = h("input", { type: "checkbox", checked: wm.text!.outline });
    outline.addEventListener("change", () => patch({ outline: outline.checked }));
    const outlineColor = h("input", { type: "color", class: "color", "aria-label": "Outline colour", value: wm.text!.outlineColor });
    outlineColor.addEventListener("input", () => patch({ outlineColor: outlineColor.value, outline: true }));
    const shadow = h("input", { type: "checkbox", checked: wm.text!.shadow });
    shadow.addEventListener("change", () => patch({ shadow: shadow.checked }));

    const el = h(
      "div",
      { class: "text-controls" },
      area,
      h("div", { class: "text-row" }, font, toggle("B", "Bold", "bold", "b"), toggle("I", "Italic", "italic", "i"), ...aligns),
      h(
        "div",
        { class: "text-row small-gap" },
        h("label", { class: "inline" }, "Colour", color),
        h("label", { class: "inline" }, outline, "Outline", outlineColor),
        h("label", { class: "inline" }, shadow, "Shadow"),
      ),
    );
    return { el, area, font };
  }

  function buildRow(wm: ActiveWatermark): Row {
    const size = h("input", { type: "range", min: MIN_SIZE_PCT, max: 100, step: 1, "aria-label": `${wm.name} size` });
    const opacity = h("input", { type: "range", min: 0, max: 100, step: 1, "aria-label": `${wm.name} opacity` });
    const sizeOut = h("output");
    const opacityOut = h("output");
    const sizeRange = bindRange(size, (v) => setSizePct(wm.id, v), () => sync(wm.id));
    const opacityRange = bindRange(opacity, (v) => setOpacity(wm.id, v / 100), () => sync(wm.id));

    const corners = h(
      "div",
      { class: "corners" },
      "Position",
      ...CORNERS.map((c) =>
        h("button", { class: "corner-btn", "data-c": c.key, title: c.title, "aria-label": c.title, onclick: () => applyCorner(wm.id, c.key) }),
      ),
      h("span", { class: "muted small hint-drag" }, "or drag in the preview"),
    );
    const thumbEl = thumb(wm.url, wm.name);
    const nameEl = h("span", { class: "name", title: wm.name }, wm.name);
    const text = wm.text ? textControls(wm) : undefined;
    const el = h(
      "div",
      { class: "wm-row", "data-id": wm.id },
      h("div", { class: "wm-row-head" }, thumbEl, nameEl, h("button", { class: "btn small", onclick: () => removeFromVideo(wm.id) }, "Remove")),
      text?.el,
      h("div", { class: "wm-sliders" }, "Size", size, sizeOut, "Opacity", opacity, opacityOut),
      corners,
    );
    el.addEventListener("pointerdown", () => select(wm.id));
    const row: Row = {
      el,
      thumbImg: thumbEl.querySelector("img")!,
      nameEl,
      size,
      sizeRange,
      sizeOut,
      opacityRange,
      opacityOut,
      text: text && { area: text.area, font: text.font },
    };
    if (text && takeTextFocus(wm.id))
      queueMicrotask(() => {
        text.area.focus();
        text.area.select();
      });
    return row;
  }

  function sync(id: string): void {
    const wm = store.video?.watermarks.find((w) => w.id === id);
    const row = rows.get(id);
    if (!wm || !row) return;
    const max = maxSizePct(wm.content);
    if (row.size.max !== String(max)) row.size.max = String(max);
    const sizePct = Math.min(max, Math.max(MIN_SIZE_PCT, sizePctFromScale(wm.scale, wm.content)));
    const opPct = Math.round(wm.opacity * 100);
    row.sizeRange.set(sizePct);
    row.opacityRange.set(opPct);
    row.sizeOut.textContent = `${sizePct}%`;
    row.opacityOut.textContent = `${opPct}%`;
    if (row.thumbImg.getAttribute("src") !== wm.url) row.thumbImg.src = wm.url;
    row.nameEl.textContent = wm.name;
    if (row.text && document.activeElement !== row.text.area && wm.text && row.text.area.value !== wm.text.text) row.text.area.value = wm.text.text;
  }

  function renderActive(): void {
    const list = store.video?.watermarks ?? [];
    const ids = new Set(list.map((w) => w.id));
    for (const [id, row] of rows)
      if (!ids.has(id)) {
        row.el.remove();
        rows.delete(id);
      }
    list.forEach((wm, index) => {
      let row = rows.get(wm.id);
      if (!row) {
        row = buildRow(wm);
        rows.set(wm.id, row);
      }
      // Keep DOM order == array order, but never move a node that is already in
      // place: re-inserting a row mid-drag cancels the slider drag.
      if (active.children[index] !== row.el) active.insertBefore(row.el, active.children[index] ?? null);
      sync(wm.id);
    });
    updateSelection();
    renderEmpty();
  }

  function renderEmpty(): void {
    const existing = active.querySelector(".wm-empty");
    const isEmpty = rows.size === 0;
    if (isEmpty && !existing)
      active.append(h("p", { class: "wm-empty" }, "Nothing on this video yet. Pick a watermark or add text from the menu — then drag it in the preview, even to the very edge."));
    if (!isEmpty && existing) existing.remove();
    ($("#wm-menu-btn") as HTMLButtonElement).title = rows.size >= MAX_ACTIVE ? `Max ${MAX_ACTIVE} watermarks per video` : "";
  }

  function updateSelection(): void {
    for (const [id, row] of rows) row.el.classList.toggle("selected", id === store.selectedWatermark);
  }

  store.on("watermarks", renderActive);
  store.on("selection", updateSelection);
  store.on("video", () => {
    for (const row of rows.values()) row.el.remove();
    rows.clear();
    renderActive();
  });
  renderEmpty();
}
