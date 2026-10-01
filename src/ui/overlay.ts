/** Full-window blocking overlay for opening files, preparing previews and installing updates. */
import { $, h } from "./dom";

export interface OverlayHandle {
  bar: HTMLElement;
  label: HTMLElement;
  setTitle(text: string): void;
  setMessage(text: string): void;
  setProgress(fraction: number | null): void;
  close(): void;
}

export function showOverlay(title: string, message = "", onCancel?: () => void): OverlayHandle {
  const root = $("#overlay");
  const titleEl = h("h2", {}, title);
  const msgEl = h("p", { class: "muted" }, message);
  const fill = h("div", { class: "bar-fill" });
  const bar = h("div", { class: "bar indeterminate" }, fill);
  const label = h("span", { class: "progress-label muted small" });
  const card = h(
    "div",
    { class: "overlay-card", role: "alertdialog", "aria-live": "polite" },
    titleEl,
    msgEl,
    h("div", { class: "progress-row" }, bar, label),
    onCancel && h("p", { style: { marginTop: "14px", marginBottom: "0" } }, h("button", { class: "btn small", onclick: onCancel }, "Cancel")),
  );
  root.replaceChildren(card);
  root.classList.remove("hidden");
  return {
    bar,
    label,
    setTitle: (t) => (titleEl.textContent = t),
    setMessage: (t) => (msgEl.textContent = t),
    setProgress(f) {
      bar.classList.toggle("indeterminate", f === null);
      fill.style.width = f === null ? "" : `${Math.round(f * 100)}%`;
      label.textContent = f === null ? "" : `${Math.round(f * 100)}%`;
    },
    close() {
      root.classList.add("hidden");
      root.replaceChildren();
    },
  };
}
