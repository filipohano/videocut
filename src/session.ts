/** Opening and closing videos, and switching between the start and editor views. */
import { convertFileSrc } from "@tauri-apps/api/core";
import { ask, open } from "@tauri-apps/plugin-dialog";
import { api, errorMessage } from "./api";
import { evenRect, fullRect } from "./lib/crop";
import { basename } from "./lib/format";
import type { Platform } from "./lib/platform";
import { PHOTO_EXTENSIONS, VIDEO_EXTENSIONS, extensionOf } from "./lib/links";
import { withProgress } from "./progress";
import { store } from "./store";
import { isDirty } from "./undo";
import { $ } from "./ui/dom";
import { showOverlay } from "./ui/overlay";
import type { Stage } from "./ui/stage";
import { toast } from "./ui/toast";

/** Video codecs the system webview plays reliably (including seeking). */
const NATIVE_CODECS: Record<Platform, Set<string>> = {
  macos: new Set(["h264", "hevc", "prores", "mpeg4"]),
  // WebView2 (Chromium) plays these out of the box; HEVC/AV1 need extra Windows add-ons.
  windows: new Set(["h264", "vp8", "vp9"]),
  linux: new Set(["h264", "vp8", "vp9"]),
};

let stage: Stage;

export function initSession(s: Stage): void {
  stage = s;
  $("#btn-new").addEventListener("click", () => void newMedia());
}

export function setView(view: "start" | "editor"): void {
  $("#view-start").classList.toggle("hidden", view !== "start");
  $("#view-editor").classList.toggle("hidden", view !== "editor");
  $("#btn-new").classList.toggle("hidden", view !== "editor");
  $("#undo-group").classList.toggle("hidden", view !== "editor");
}

/** True if it's fine to throw away the current edit (nothing unsaved, or the user agrees). */
async function confirmDiscard(): Promise<boolean> {
  if (!store.video || !isDirty()) return true;
  return ask("You have edits that haven't been exported. Start over and discard them?", {
    title: "Start over?",
    kind: "warning",
    okLabel: "Discard",
    cancelLabel: "Keep editing",
  });
}

/** ⌘N / the New button: back to the start screen for a fresh video or photo. */
export async function newMedia(): Promise<void> {
  if (!store.video || store.busy) return;
  if (await confirmDiscard()) closeVideo();
}

/** ⌘O: choose a video or photo from disk. */
export async function chooseAndOpen(): Promise<void> {
  if (store.busy || !(await confirmDiscard())) return;
  const picked = await open({
    multiple: false,
    filters: [{ name: "Videos and photos", extensions: [...VIDEO_EXTENSIONS, ...PHOTO_EXTENSIONS] }],
  });
  if (typeof picked === "string") await openVideo(picked);
}

/** Downloads are temporary: delete the file once we move on. Other files are never touched. */
function discardDownload(path: string | undefined): void {
  if (path) void api.discardDownload(path).catch(() => {});
}

export function closeVideo(): void {
  if (store.busy) return;
  stage.pause();
  discardDownload(store.video?.path);
  document.body.classList.remove("is-photo");
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
  const overlay = showOverlay("Opening…");
  const previous = store.video;
  try {
    const info = await api.probeMedia(path);
    // The editor has to be visible so the stage can measure itself.
    setView("editor");

    let playUrl = convertFileSrc(path);
    if (info.isImage) {
      await stage.loadPhoto(playUrl);
    } else {
      try {
        // WebKit is solid with these; anything else (VP9, AV1, VP8…) can stutter or
        // misbehave when scrubbing, so those get a small H.264 preview straight away.
        if (!NATIVE_CODECS[store.platform].has(info.videoCodec ?? "")) throw new Error("preview copy needed");
        await stage.load(playUrl);
      } catch {
        // WKWebView can't play every container/codec: make a small H.264 proxy
        // for the preview. Export always reads the original file.
        overlay.setTitle("Preparing a preview…");
        overlay.setMessage("A lightweight copy is made so previewing and scrubbing stay smooth. Your export still uses the original, full-quality file.");
        const proxy = await withProgress("preview", overlay.bar, overlay.label, () => api.makePreview(path, info.hasAudio, info.duration));
        playUrl = convertFileSrc(proxy);
        await stage.load(playUrl);
      }
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
      quality: info.isImage ? store.settings.exportImageQuality : store.settings.exportQuality,
      imageFormat: extensionOf(path) === "png" ? "png" : "jpg",
    };
    document.body.classList.toggle("is-photo", info.isImage);
    if (previous && previous.path !== path) discardDownload(previous.path);
    store.selectedWatermark = null;
    $("#source-name").textContent = info.isImage
      ? `${basename(path)} · ${info.width}×${info.height} · photo`
      : `${basename(path)} · ${info.width}×${info.height} · ${info.videoCodec ?? "video"}${info.hasAudio ? "" : " · no audio"}`;
    store.emit("video", "crop", "trim", "watermarks", "selection");
    return true;
  } catch (e) {
    toast(errorMessage(e), { kind: "error", timeout: 10000 });
    if (previous) (previous.info.isImage ? stage.loadPhoto(previous.playUrl) : stage.load(previous.playUrl)).catch(() => {});
    setView(previous ? "editor" : "start");
    return false;
  } finally {
    overlay.close();
    store.setBusy(false);
  }
}
