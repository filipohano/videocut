/** The watermark library drop-down and the list of watermarks on the current video. */
import { convertFileSrc } from "@tauri-apps/api/core";
import { ask, open } from "@tauri-apps/plugin-dialog";
import { IMAGE_EXTENSIONS } from "../lib/links";
import type { Corner } from "../lib/watermarks";
import { store, type ActiveWatermark } from "../store";
import { addToVideo, applyCorner, deleteFromLibrary, importWatermark, removeFromVideo, select, updateActive } from "../watermarkOps";
import { $, h } from "./dom";

const CORNERS: { key: Corner; title: string }[] = [
  { key: "tl", title: "Top left" },
  { key: "tr", title: "Top right" },
  { key: "c", title: "Centre" },
  { key: "bl", title: "Bottom left" },
  { key: "br", title: "Bottom right" },
];

const TRASH =
  '<svg viewBox="0 0 24 24" width="15" height="15" fill="none" stroke="currentColor" stroke-width="1.8" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true"><path d="M4 7h16M10 11v6M14 11v6M6 7l1 12a2 2 0 002 2h6a2 2 0 002-2l1-12M9 7V4h6v3"/></svg>';

function thumb(path: string, alt: string): HTMLElement {
  return h("span", { class: "thumb" }, h("img", { src: convertFileSrc(path), alt }));
}

export function initWatermarkPanel(): void {
  const menuBtn = $("#wm-menu-btn");
  const menu = $("#wm-menu");
  const active = $("#wm-active");

  // ───────── library drop-down ─────────
  function closeMenu(): void {
    menu.classList.add("hidden");
    menuBtn.setAttribute("aria-expanded", "false");
  }

  function renderMenu(): void {
    const onVideo = new Set(store.video?.watermarks.map((w) => w.id));
    const items: HTMLElement[] = [];
    if (store.library.length === 0) {
      items.push(h("div", { class: "menu-empty" }, "No saved watermarks yet. Upload your first one below."));
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
        thumb(entry.path, entry.name),
        h("span", { class: "name" }, entry.name),
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
    const upload = h(
      "div",
      { class: "menu-item", role: "menuitem", tabindex: 0 },
      h("span", { class: "name" }, "Upload new watermark…"),
    );
    const doUpload = async () => {
      closeMenu();
      const picked = await open({ multiple: false, filters: [{ name: "Image", extensions: IMAGE_EXTENSIONS }] });
      if (typeof picked === "string") await importWatermark(picked);
    };
    upload.addEventListener("click", doUpload);
    upload.addEventListener("keydown", (e) => (e.key === "Enter" || e.key === " ") && (e.preventDefault(), doUpload()));
    items.push(upload);
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
    size: HTMLInputElement;
    sizeOut: HTMLOutputElement;
    opacity: HTMLInputElement;
    opacityOut: HTMLOutputElement;
  }
  const rows = new Map<string, Row>();

  function buildRow(wm: ActiveWatermark): Row {
    const size = h("input", { type: "range", min: 3, max: 100, step: 1, "aria-label": `${wm.name} size` });
    const opacity = h("input", { type: "range", min: 0, max: 100, step: 1, "aria-label": `${wm.name} opacity` });
    const sizeOut = h("output");
    const opacityOut = h("output");
    size.addEventListener("input", () => updateActive(wm.id, { scale: Number(size.value) / 100 }));
    opacity.addEventListener("input", () => updateActive(wm.id, { opacity: Number(opacity.value) / 100 }));

    const corners = h(
      "div",
      { class: "corners" },
      "Position",
      ...CORNERS.map((c) =>
        h("button", { class: "corner-btn", "data-c": c.key, title: c.title, "aria-label": c.title, onclick: () => applyCorner(wm.id, c.key) }),
      ),
    );
    const el = h(
      "div",
      { class: "wm-row", "data-id": wm.id },
      h(
        "div",
        { class: "wm-row-head" },
        thumb(wm.path, wm.name),
        h("span", { class: "name", title: wm.name }, wm.name),
        h("button", { class: "btn small", onclick: () => removeFromVideo(wm.id) }, "Remove"),
      ),
      h("div", { class: "wm-sliders" }, "Size", size, sizeOut, "Opacity", opacity, opacityOut),
      corners,
    );
    el.addEventListener("pointerdown", () => select(wm.id));
    return { el, size, sizeOut, opacity, opacityOut };
  }

  function renderActive(): void {
    const list = store.video?.watermarks ?? [];
    const ids = new Set(list.map((w) => w.id));
    for (const [id, row] of rows)
      if (!ids.has(id)) {
        row.el.remove();
        rows.delete(id);
      }
    for (const wm of list) {
      let row = rows.get(wm.id);
      if (!row) {
        row = buildRow(wm);
        rows.set(wm.id, row);
      }
      const sizePct = Math.round(wm.scale * 100);
      const opPct = Math.round(wm.opacity * 100);
      if (document.activeElement !== row.size) row.size.value = String(sizePct);
      if (document.activeElement !== row.opacity) row.opacity.value = String(opPct);
      row.sizeOut.textContent = `${sizePct}%`;
      row.opacityOut.textContent = `${opPct}%`;
      active.append(row.el); // keeps DOM order == array order
    }
    updateSelection();
    renderEmpty();
  }

  function renderEmpty(): void {
    const existing = active.querySelector(".wm-empty");
    const isEmpty = rows.size === 0;
    if (isEmpty && !existing)
      active.append(h("p", { class: "wm-empty" }, "No watermark on this video yet. Pick one from your library ▾ — you can drag it in the preview."));
    if (!isEmpty && existing) existing.remove();
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
