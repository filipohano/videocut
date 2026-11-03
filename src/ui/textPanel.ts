/** The "Text" section: a list of the text boxes on the video, each with its words and its look. */
import { ask } from "@tauri-apps/plugin-dialog";
import type { TextPreset } from "../api";
import {
  MAX_CHARS,
  MAX_SIZE,
  MIN_SIZE,
  SWATCHES,
  isEmptyText,
  type TextAlign,
  type TextBox,
  type TextBoxStyle,
} from "../lib/textBox";
import { store } from "../store";
import { MAX_TEXTS, addText, applyStyle, deleteStyle, removeText, saveStyle, selectText, takeTextBoxFocus, textById, updateText } from "../textOps";
import { $, h } from "./dom";
import { bindRange, type BoundRange } from "./range";

const TRASH =
  '<svg viewBox="0 0 24 24" width="15" height="15" fill="none" stroke="currentColor" stroke-width="1.8" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true"><path d="M4 7h16M10 11v6M14 11v6M6 7l1 12a2 2 0 002 2h6a2 2 0 002-2l1-12M9 7V4h6v3"/></svg>';

const ALIGN_ICONS: Record<TextAlign, string> = {
  left: "M4 6h16M4 10h10M4 14h16M4 18h10",
  center: "M4 6h16M7 10h10M4 14h16M7 18h10",
  right: "M4 6h16M10 10h10M4 14h16M10 18h10",
};

const icon = (path: string) =>
  `<svg viewBox="0 0 24 24" width="16" height="16" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" aria-hidden="true"><path d="${path}"/></svg>`;

/** A row of colour chips plus a full colour picker. */
function colorField(label: string, allowNone: boolean, pick: (color: string | null) => void) {
  const input = h("input", { type: "color", class: "color", "aria-label": `${label} colour`, title: "Pick any colour" });
  input.addEventListener("input", () => pick(input.value));
  const chips: { color: string | null; el: HTMLButtonElement }[] = [];
  if (allowNone) chips.push({ color: null, el: h("button", { class: "swatch none", type: "button", title: "No background", "aria-label": "No background" }) });
  for (const s of SWATCHES) {
    chips.push({ color: s.color, el: h("button", { class: "swatch", type: "button", title: s.name, "aria-label": `${label}: ${s.name}`, style: { background: s.color } }) });
  }
  for (const c of chips) c.el.addEventListener("click", () => pick(c.color));
  const el = h("div", { class: "color-field" }, h("span", { class: "field-label" }, label), h("div", { class: "swatches" }, ...chips.map((c) => c.el), input));
  return {
    el,
    sync(value: string | null) {
      for (const c of chips) {
        const on = c.color === value;
        c.el.classList.toggle("selected", on);
        c.el.setAttribute("aria-pressed", String(on));
      }
      if (value && input.value !== value) input.value = value;
    },
  };
}

function nameOf(box: TextBox): string {
  const first = box.text.split("\n").find((l) => l.trim() !== "");
  return first ? (first.length > 30 ? `${first.slice(0, 30)}…` : first) : "Empty text";
}

export function initTextPanel(): void {
  const addBtn = $<HTMLButtonElement>("#text-add");
  const menuBtn = $("#text-styles-btn");
  const menu = $("#text-styles-menu");
  const active = $("#text-active");

  addBtn.addEventListener("click", () => addText());

  // ───────── saved styles ─────────
  function closeMenu(): void {
    menu.classList.add("hidden");
    menuBtn.setAttribute("aria-expanded", "false");
  }

  function usePreset(p: TextPreset): void {
    const selected = textById(store.selectedText);
    if (selected) applyStyle(selected.id, p.style);
    else addText(p.style);
  }

  function renderMenu(): void {
    const items: HTMLElement[] = [];
    if (store.textPresets.length === 0) {
      items.push(h("div", { class: "menu-empty" }, "No saved styles yet. Set up a text the way you like it, then press “Save style” on it."));
    } else {
      items.push(h("div", { class: "menu-empty" }, store.video && store.selectedText ? "Click a style to use it on the selected text." : "Click a style to add a new text with it."));
    }
    for (const p of store.textPresets) {
      const del = h("button", { class: "icon-btn", title: "Delete this style", "aria-label": `Delete the style ${p.name}` });
      del.innerHTML = TRASH;
      del.addEventListener("click", async (e) => {
        e.stopPropagation();
        const ok = await ask(`Delete the style “${p.name}”?`, { title: "Delete style", kind: "warning" });
        if (ok) await deleteStyle(p.id);
      });
      const sample = h("span", { class: "style-sample", style: { color: p.style.color, background: p.style.background ?? "#2a2d36", fontWeight: p.style.bold ? "700" : "400" } }, "Aa");
      const item = h("div", { class: "menu-item", role: "menuitem", tabindex: 0 }, sample, h("span", { class: "name" }, p.name), del);
      const choose = () => {
        usePreset(p);
        closeMenu();
      };
      item.addEventListener("click", choose);
      item.addEventListener("keydown", (e) => (e.key === "Enter" || e.key === " ") && (e.preventDefault(), choose()));
      items.push(item);
    }
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
  store.on(["textStyles", "selection"], () => !menu.classList.contains("hidden") && renderMenu());

  // ───────── the text boxes on this video ─────────
  interface Row {
    el: HTMLElement;
    nameEl: HTMLElement;
    area: HTMLTextAreaElement;
    color: ReturnType<typeof colorField>;
    background: ReturnType<typeof colorField>;
    size: HTMLInputElement;
    sizeRange: BoundRange;
    sizeOut: HTMLOutputElement;
    bold: HTMLButtonElement;
    aligns: Record<TextAlign, HTMLButtonElement>;
  }
  const rows = new Map<string, Row>();

  function buildRow(box: TextBox): Row {
    const id = box.id;
    const patch = (p: Partial<TextBoxStyle> & { text?: string }) => updateText(id, p);

    const area = h("textarea", { class: "text-input", rows: 2, maxLength: MAX_CHARS, placeholder: "Type your text…", "aria-label": "Text" });
    area.value = box.text;
    area.addEventListener("input", () => patch({ text: area.value }));

    const color = colorField("Text", false, (c) => c && patch({ color: c }));
    const background = colorField("Background", true, (c) => patch({ background: c }));

    const size = h("input", { type: "range", min: MIN_SIZE, max: MAX_SIZE, step: 0.5, "aria-label": "Text size" });
    const sizeOut = h("output");
    const sizeRange = bindRange(size, (v) => patch({ size: v }), () => sync(id));

    const bold = h("button", { class: "btn small toggle b", type: "button", title: "Bold" }, "B");
    bold.addEventListener("click", () => patch({ bold: !textById(id)?.bold }));
    const aligns = {} as Record<TextAlign, HTMLButtonElement>;
    for (const a of ["left", "center", "right"] as const) {
      const b = h("button", { class: "btn small toggle", type: "button", title: `Align ${a}`, "aria-label": `Align ${a}` });
      b.innerHTML = icon(ALIGN_ICONS[a]);
      b.addEventListener("click", () => patch({ align: a }));
      aligns[a] = b;
    }

    // "Save style" asks for a name right in the row.
    const nameInput = h("input", { type: "text", class: "style-name", maxLength: 40, placeholder: "Name of the style", "aria-label": "Name of the style" });
    const form = h(
      "form",
      { class: "save-style hidden" },
      nameInput,
      h("button", { class: "btn small primary", type: "submit" }, "Save"),
      h("button", { class: "btn small", type: "button", onclick: () => form.classList.add("hidden") }, "Cancel"),
    );
    form.addEventListener("submit", async (e) => {
      e.preventDefault();
      const current = textById(id);
      if (!current || nameInput.value.trim() === "") return nameInput.focus();
      if (await saveStyle(nameInput.value, current)) {
        nameInput.value = "";
        form.classList.add("hidden");
      }
    });
    const saveBtn = h("button", { class: "btn small", type: "button", title: "Keep this look to use again" }, "Save style");
    saveBtn.addEventListener("click", () => {
      form.classList.remove("hidden");
      nameInput.focus();
    });

    const nameEl = h("span", { class: "name" });
    const el = h(
      "div",
      { class: "wm-row text-row-card", "data-id": id },
      h("div", { class: "wm-row-head" }, nameEl, h("button", { class: "btn small", onclick: () => removeText(id) }, "Remove")),
      area,
      color.el,
      background.el,
      h("div", { class: "text-size" }, h("span", { class: "field-label" }, "Size"), size, sizeOut),
      h("div", { class: "text-row" }, bold, ...Object.values(aligns), h("span", { class: "grow" }), saveBtn),
      form,
      h("p", { class: "muted small hint-text" }, "Drag the text in the preview to move it. Drag the dot at its corner to resize."),
    );
    el.addEventListener("pointerdown", () => selectText(id));

    if (takeTextBoxFocus(id))
      queueMicrotask(() => {
        area.focus();
        area.select();
      });
    return { el, nameEl, area, color, background, size, sizeRange, sizeOut, bold, aligns };
  }

  function sync(id: string): void {
    const box = textById(id);
    const row = rows.get(id);
    if (!box || !row) return;
    row.nameEl.textContent = nameOf(box);
    row.nameEl.classList.toggle("muted", isEmptyText(box.text));
    row.color.sync(box.color);
    row.background.sync(box.background);
    row.sizeRange.set(box.size);
    row.sizeOut.textContent = String(Math.round(box.size * 10) / 10);
    row.bold.setAttribute("aria-pressed", String(box.bold));
    for (const [a, b] of Object.entries(row.aligns)) b.setAttribute("aria-pressed", String(a === box.align));
    if (document.activeElement !== row.area && row.area.value !== box.text) row.area.value = box.text;
  }

  function renderActive(): void {
    const list = store.video?.texts ?? [];
    const ids = new Set(list.map((t) => t.id));
    for (const [id, row] of rows)
      if (!ids.has(id)) {
        row.el.remove();
        rows.delete(id);
      }
    list.forEach((box, index) => {
      let row = rows.get(box.id);
      if (!row) {
        row = buildRow(box);
        rows.set(box.id, row);
      }
      // Keep DOM order == array order, but never move a node that is already in place
      // (re-inserting a row mid-drag cancels the slider drag).
      if (active.children[index] !== row.el) active.insertBefore(row.el, active.children[index] ?? null);
      sync(box.id);
    });
    updateSelection();
    renderEmpty();
  }

  function renderEmpty(): void {
    const existing = active.querySelector(".wm-empty");
    const isEmpty = rows.size === 0;
    if (isEmpty && !existing) {
      active.append(h("p", { class: "wm-empty" }, "No text on this video yet. Press Add text, then drag it where you want it in the preview."));
    }
    if (!isEmpty && existing) existing.remove();
    addBtn.title = rows.size >= MAX_TEXTS ? `Max ${MAX_TEXTS} texts per video` : "";
  }

  function updateSelection(): void {
    for (const [id, row] of rows) row.el.classList.toggle("selected", id === store.selectedText);
  }

  store.on("texts", renderActive);
  store.on("selection", updateSelection);
  store.on("video", () => {
    for (const row of rows.values()) row.el.remove();
    rows.clear();
    renderActive();
  });
  renderEmpty();
}
