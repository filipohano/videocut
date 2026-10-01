/** Batch: apply the open video's crop, watermarks and quality to more videos. */
import { open } from "@tauri-apps/plugin-dialog";
import { CANCELLED, api, errorMessage } from "../api";
import { DEFAULT_BATCH_OPTIONS, buildBatchSpec, type BatchOptions } from "../lib/batch";
import { basename } from "../lib/format";
import { PHOTO_EXTENSIONS, VIDEO_EXTENSIONS, isMediaPath } from "../lib/links";
import { subscribe } from "../progress";
import { store } from "../store";
import { flushTextSaves } from "../watermarkOps";
import { $, h } from "./dom";
import { toast } from "./toast";

type Status = "queued" | "running" | "done" | "error" | "skipped";
interface Item {
  path: string;
  status: Status;
  fraction: number;
  message?: string;
  output?: string;
}

const MAX_ITEMS = 200;

let dialog: HTMLDialogElement;
let items: Item[] = [];
let opts: BatchOptions = { ...DEFAULT_BATCH_OPTIONS };
let running = false;
let cancelled = false;
let finished = false;
let render: () => void;

export const isBatchOpen = () => dialog?.open ?? false;

/** Add videos to the queue (ignores non-videos and duplicates). Returns how many were added. */
export function addToBatch(paths: string[]): number {
  if (running) return 0;
  let added = 0;
  for (const p of paths) {
    if (!isMediaPath(p)) continue;
    if (p === store.video?.path || items.some((i) => i.path === p)) continue;
    if (items.length >= MAX_ITEMS) break;
    items.push({ path: p, status: "queued", fraction: 0 });
    added++;
  }
  if (added) {
    finished = false;
    render?.();
  }
  return added;
}

export function openBatch(paths: string[] = []): void {
  if (!store.video) {
    toast("Open a video or photo and set up the crop and watermarks first. Batch repeats that on more files.", { kind: "info" });
    return;
  }
  if (!running) {
    items = [];
    finished = false;
    cancelled = false;
  }
  addToBatch(paths);
  render();
  if (!dialog.open) dialog.showModal();
}

export function initBatchDialog(): void {
  dialog = $<HTMLDialogElement>("#batch-dialog");
  $("#btn-batch").addEventListener("click", () => openBatch());
  // Esc / backdrop must not close it mid-run.
  dialog.addEventListener("cancel", (e) => running && e.preventDefault());
  dialog.addEventListener("click", (e) => e.target === dialog && !running && dialog.close());

  const statusBadge = (i: Item) =>
    ({
      queued: h("span", { class: "badge" }, "Queued"),
      running: h("span", { class: "badge" }, "Exporting…"),
      done: h("span", { class: "badge export" }, "Done"),
      error: h("span", { class: "badge err" }, "Failed"),
      skipped: h("span", { class: "badge warn" }, "Skipped"),
    })[i.status];

  render = () => {
    const v = store.video;
    const crop = v ? `${Math.round((v.crop.w / v.info.width) * 100)}% × ${Math.round((v.crop.h / v.info.height) * 100)}% of the frame` : "";
    const wms = v?.watermarks.length ?? 0;
    const queued = items.filter((i) => i.status === "queued").length;
    const done = items.filter((i) => i.status === "done").length;
    const failed = items.filter((i) => i.status === "error").length;

    const option = (label: string, hint: string, key: keyof BatchOptions) =>
      h(
        "label",
        { class: "check" },
        h("input", { type: "checkbox", checked: opts[key], disabled: running, onchange: (e: Event) => (opts[key] = (e.target as HTMLInputElement).checked) }),
        h("span", {}, label, h("br"), h("span", { class: "muted small" }, hint)),
      );

    const rows = items.map((i) =>
      h(
        "div",
        { class: "item batch" },
        h(
          "div",
          { class: "info" },
          h("strong", { title: i.path }, basename(i.path)),
          h("span", { class: "muted small" }, i.message ?? (i.status === "done" && i.output ? `Saved as ${basename(i.output)}` : "")),
          i.status === "running" && h("div", { class: "mini-bar" }, h("div", { style: { width: `${Math.round(i.fraction * 100)}%` } })),
        ),
        h(
          "div",
          { class: "actions" },
          statusBadge(i),
          !running && i.status === "queued" && h("button", { class: "icon-btn", "aria-label": "Remove", onclick: () => { items = items.filter((x) => x !== i); render(); } }, "×"),
        ),
      ),
    );

    const parts: (HTMLElement | false)[] = [
      h("div", { class: "dialog-head" }, h("h2", {}, "Apply this edit to more files"), !running && h("button", { class: "icon-btn", "aria-label": "Close", onclick: () => dialog.close() }, "×")),
      h(
        "p",
        { class: "batch-summary" },
        `Repeats what you set up on “${v ? basename(v.path) : ""}”: ${v && v.crop.w === v.info.width && v.crop.h === v.info.height ? "no crop" : `crop ${crop}`}, ${wms} watermark${wms === 1 ? "" : "s"}, quality ${v?.quality ?? ""}. Trim isn't copied. Each result is saved to your Finished folder.`,
      ),
      h("div", { class: "batch-opts" }, option("Crop the same area", "Same relative area of every video. The aspect ratio follows each video.", "applyCrop"), option("Add the watermarks and text", "In the same places and sizes.", "applyWatermarks")),
      h(
        "div",
        { class: "list" },
        ...(rows.length ? rows : [h("div", { class: "empty" }, "Add the videos or photos you want to process, or drop them here.")]),
      ),
      h(
        "div",
        { class: "dialog-foot" },
        h("span", { class: "grow" }, running ? "Working… this can take a while for long videos." : finished ? `${done} exported${failed ? `, ${failed} failed` : ""}.` : queued ? `${queued} video${queued === 1 ? "" : "s"} ready` : ""),
        !running && h("button", { class: "btn small", onclick: async () => { const picked = await open({ multiple: true, filters: [{ name: "Videos and photos", extensions: [...VIDEO_EXTENSIONS, ...PHOTO_EXTENSIONS] }] }); if (Array.isArray(picked)) addToBatch(picked); else if (typeof picked === "string") addToBatch([picked]); } }, "Add files…"),
        finished && done > 0 && h("button", { class: "btn small", onclick: () => { const last = [...items].reverse().find((i) => i.output); if (last?.output) void api.revealInFinder(last.output); } }, "Show in Finder"),
        running
          ? h("button", { class: "btn small danger", onclick: () => { cancelled = true; void api.cancelJob("export"); } }, "Stop")
          : h("button", { class: "btn small primary", disabled: queued === 0, onclick: () => void run() }, queued ? `Export ${queued}` : "Export"),
      ),
    ];
    dialog.replaceChildren(...(parts.filter(Boolean) as HTMLElement[]));
  };

  async function run(): Promise<void> {
    const template = store.video;
    if (!template || running) return;
    running = true;
    cancelled = false;
    finished = false;
    store.setBusy(true);
    let current: Item | null = null;
    const off = subscribe("export", (p) => {
      if (current && p.fraction !== null) {
        current.fraction = p.fraction;
        render();
      }
    });
    try {
      await flushTextSaves();
      for (const item of items.filter((i) => i.status === "queued")) {
        if (cancelled) {
          item.status = "skipped";
          continue;
        }
        current = item;
        item.status = "running";
        item.fraction = 0;
        render();
        try {
          const info = await api.probeMedia(item.path);
          const output = await api.defaultSavePath(template.info.isImage ? template.imageFormat : "mp4");
          item.output = await api.exportVideo(buildBatchSpec(template, info, item.path, output, opts));
          item.status = "done";
        } catch (e) {
          const msg = errorMessage(e);
          if (msg === CANCELLED) {
            item.status = "skipped";
            item.message = "Stopped";
            cancelled = true;
          } else {
            item.status = "error";
            item.message = msg;
          }
        }
        render();
      }
    } finally {
      off();
      current = null;
      running = false;
      finished = true;
      store.setBusy(false);
      render();
      const done = items.filter((i) => i.status === "done").length;
      toast(cancelled ? `Stopped after ${done} video${done === 1 ? "" : "s"}` : `Batch finished: ${done} of ${items.length} exported`, { kind: done ? "success" : "error", timeout: 8000 });
    }
  }
}
