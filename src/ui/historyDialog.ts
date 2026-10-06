/** History of downloads and exports. */
import { fileManager } from "../lib/platform";
import { store } from "../store";
import { convertFileSrc } from "@tauri-apps/api/core";
import { ask } from "@tauri-apps/plugin-dialog";
import { api, errorMessage, type HistoryEntry } from "../api";
import { basename, formatBytes } from "../lib/format";
import { $, h } from "./dom";
import { toast } from "./toast";

type Filter = "all" | "download" | "export";

const PLATFORM_NAMES: Record<string, string> = { tiktok: "TikTok", instagram: "Instagram", twitter: "X / Twitter" };

export function formatWhen(unixSeconds: number, now = new Date()): string {
  const d = new Date(unixSeconds * 1000);
  const time = d.toLocaleTimeString(undefined, { hour: "2-digit", minute: "2-digit" });
  const sameDay = d.toDateString() === now.toDateString();
  const yesterday = new Date(now.getTime() - 86_400_000).toDateString() === d.toDateString();
  if (sameDay) return `Today ${time}`;
  if (yesterday) return `Yesterday ${time}`;
  return `${d.toLocaleDateString(undefined, { day: "numeric", month: "short", year: d.getFullYear() === now.getFullYear() ? undefined : "numeric" })} ${time}`;
}

export function initHistoryDialog(openVideo: (path: string) => Promise<boolean>): void {
  const dialog = $<HTMLDialogElement>("#history-dialog");
  let filter: Filter = "all";
  let entries: HistoryEntry[] = [];

  $("#btn-history").addEventListener("click", async () => {
    await reload();
    dialog.showModal();
  });
  dialog.addEventListener("click", (e) => e.target === dialog && dialog.close());

  async function reload(): Promise<void> {
    try {
      entries = await api.historyList();
    } catch (e) {
      toast(errorMessage(e), { kind: "error" });
      entries = [];
    }
    render();
  }

  function row(e: HistoryEntry): HTMLElement {
    const name = e.title ?? basename(e.path);
    const meta = [
      h("span", { class: `badge ${e.kind}` }, e.kind === "download" ? "Download" : "Export"),
      e.platform ? `${PLATFORM_NAMES[e.platform] ?? e.platform} · ` : "",
      e.bytes != null ? `${formatBytes(e.bytes)} · ` : "",
      formatWhen(e.createdAt),
    ];
    const actions: HTMLElement[] = [];
    if (e.exists) {
      actions.push(
        h("button", { class: "btn small primary", onclick: async () => { dialog.close(); await openVideo(e.path); } }, "Edit"),
        h("button", { class: "btn small", onclick: () => void api.revealInFinder(e.path) }, `Show in ${fileManager(store.platform)}`),
      );
    } else actions.push(h("span", { class: "badge warn" }, "File moved or deleted"));
    if (e.sourceUrl)
      actions.push(
        h("button", {
          class: "btn small",
          onclick: async () => {
            try {
              await navigator.clipboard.writeText(e.sourceUrl!);
              toast("Link copied", { kind: "success", timeout: 2500 });
            } catch {
              toast(e.sourceUrl!, { kind: "info" });
            }
          },
        }, "Copy link"),
      );
    actions.push(
      h("button", {
        class: "icon-btn", title: "Remove from history (the video file is kept)", "aria-label": "Remove from history",
        onclick: async () => { await api.historyRemove(e.id); await reload(); },
      }, "×"),
    );
    return h(
      "div",
      { class: `item${e.exists ? "" : " missing"}`, "data-id": e.id },
      h("div", { class: "pic" }, e.thumbPath ? h("img", { src: convertFileSrc(e.thumbPath), alt: "" }) : "no preview"),
      h("div", { class: "info" }, h("strong", { title: e.path }, name), h("span", { class: "muted small" }, ...meta)),
      h("div", { class: "actions" }, ...actions),
    );
  }

  function render(): void {
    const shown = entries.filter((e) => filter === "all" || e.kind === filter);
    const tab = (f: Filter, label: string) =>
      h("button", { class: "tab", "aria-pressed": String(filter === f), onclick: () => { filter = f; render(); } }, label);
    const parts: (HTMLElement | false)[] = [
      h("div", { class: "dialog-head" }, h("h2", {}, "History"), h("button", { class: "icon-btn", "aria-label": "Close", onclick: () => dialog.close() }, "×")),
      h("div", { class: "tabs" }, tab("all", "All"), tab("download", "Downloads"), tab("export", "Exports")),
      h(
        "div",
        { class: "list" },
        ...(shown.length
          ? shown.map(row)
          : [h("div", { class: "empty" }, entries.length ? "Nothing here with that filter." : "Nothing yet. Downloads and exports show up here.")]),
      ),
      entries.length > 0 &&
        h(
          "div",
          { class: "dialog-foot" },
          h("span", { class: "grow" }, `${entries.length} item${entries.length > 1 ? "s" : ""} · removing an entry never deletes the video`),
          h("button", {
            class: "btn small danger",
            onclick: async () => {
              if (await ask("Clear the whole history list? Your videos are not deleted.", { title: "Clear history", kind: "warning" })) {
                await api.historyClear();
                await reload();
              }
            },
          }, "Clear history"),
        ),
    ];
    dialog.replaceChildren(...(parts.filter(Boolean) as HTMLElement[]));
  }
}
