/** Opening and closing videos, and switching between the start and editor views. */
import { convertFileSrc } from "@tauri-apps/api/core";
import { api, errorMessage } from "./api";
import { evenRect, fullRect } from "./lib/crop";
import { basename } from "./lib/format";
import { withProgress } from "./progress";
import { store } from "./store";
import { $ } from "./ui/dom";
import { showOverlay } from "./ui/overlay";
import type { Stage } from "./ui/stage";
import { toast } from "./ui/toast";

let stage: Stage;

export function initSession(s: Stage): void {
  stage = s;
  $("#btn-new").addEventListener("click", closeVideo);
}

export function setView(view: "start" | "editor"): void {
  $("#view-start").classList.toggle("hidden", view !== "start");
  $("#view-editor").classList.toggle("hidden", view !== "editor");
  $("#btn-new").classList.toggle("hidden", view !== "editor");
}

export function closeVideo(): void {
  if (store.busy) return;
  stage.pause();
  store.video = null;
  store.selectedWatermark = null;
  store.emit("video", "watermarks", "selection");
  setView("start");
}

export async function openVideo(path: string): Promise<boolean> {
  if (store.busy) {
    toast("Wait for the current job to finish first", { kind: "info" });
    return false;
  }
  store.setBusy(true);
  const overlay = showOverlay("Opening video…");
  const previous = store.video;
  try {
    const info = await api.probeMedia(path);
    // The editor has to be visible so the stage can measure itself.
    setView("editor");

    let playUrl = convertFileSrc(path);
    try {
      await stage.load(playUrl);
    } catch {
      // WKWebView can't play every container/codec: make a small H.264 proxy
      // for the preview. Export always reads the original file.
      overlay.setTitle("Preparing a preview…");
      overlay.setMessage("This format can't be played directly, so a lightweight copy is made for previewing. Your export still uses the original.");
      const proxy = await withProgress("preview", overlay.bar, overlay.label, () => api.makePreview(path, info.hasAudio, info.duration));
      playUrl = convertFileSrc(proxy);
      await stage.load(playUrl);
    }

    const frame = { w: info.width, h: info.height };
    store.video = {
      path,
      info,
      playUrl,
      crop: evenRect(fullRect(frame)),
      aspect: "free",
      trimStart: 0,
      trimEnd: info.duration,
      watermarks: [],
      quality: store.settings.quality,
    };
    store.selectedWatermark = null;
    $("#source-name").textContent = `${basename(path)} · ${info.width}×${info.height} · ${info.videoCodec ?? "video"}${info.hasAudio ? "" : " · no audio"}`;
    store.emit("video", "crop", "trim", "watermarks", "selection");
    return true;
  } catch (e) {
    toast(errorMessage(e), { kind: "error", timeout: 10000 });
    if (previous) stage.load(previous.playUrl).catch(() => {});
    setView(previous ? "editor" : "start");
    return false;
  } finally {
    overlay.close();
    store.setBusy(false);
  }
}
