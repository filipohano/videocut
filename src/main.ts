import "./styles.css";
import { listen } from "@tauri-apps/api/event";
import { getCurrentWebview } from "@tauri-apps/api/webview";
import { api, errorMessage } from "./api";
import { IMAGE_EXTENSIONS, VIDEO_EXTENSIONS, extensionOf } from "./lib/links";
import { startProgressRouter } from "./progress";
import { initSession, openVideo } from "./session";
import { store } from "./store";
import { canRedo, canUndo, initUndo, onUndoChange, redo, undo } from "./undo";
import { initUpdateUi, launchCheck, launchGate } from "./updates";
import { importWatermark, refreshLibrary } from "./watermarkOps";
import { $ } from "./ui/dom";
import { addToBatch, initBatchDialog, isBatchOpen, openBatch } from "./ui/batchDialog";
import { initExportBar } from "./ui/exportBar";
import { initHistoryDialog } from "./ui/historyDialog";
import { initCropPanel, initOutputPanel, initTrimPanel } from "./ui/panels";
import { initSettingsDialog } from "./ui/settingsDialog";
import { initStage } from "./ui/stage";
import { initStart } from "./ui/start";
import { toast } from "./ui/toast";
import { initWatermarkPanel } from "./ui/watermarkPanel";

async function boot(): Promise<void> {
  // Outside Tauri (plain `npm run dev` in a browser) fall back to a fake backend.
  if (import.meta.env.DEV && !("__TAURI_INTERNALS__" in window)) {
    await import("./dev/mockTauri");
  }

  const [settings, info] = await Promise.all([api.getSettings(), api.appInfo()]);
  store.settings = settings;
  store.appVersion = info.version;
  store.encoder = info.encoder;

  // Splash screen: with automatic updates on, check (and install) BEFORE the app opens.
  await launchGate();

  await refreshLibrary().catch((e) => console.warn("library", e));
  await startProgressRouter();

  const stage = initStage();
  initSession(stage);
  initStart(openVideo);
  initCropPanel();
  initTrimPanel(stage);
  initWatermarkPanel();
  initOutputPanel();
  initExportBar();
  initSettingsDialog();
  initHistoryDialog(openVideo);
  initBatchDialog();
  initUpdateUi();
  initUndoUi();

  // While a job runs, the editing panels are frozen.
  store.on("busy", () => {
    document.querySelectorAll<HTMLElement>("#view-editor .card, #view-editor .stage-wrap").forEach((el) => el.toggleAttribute("inert", store.busy));
    ($("#btn-new") as HTMLButtonElement).disabled = store.busy;
  });

  // Space toggles playback.
  document.addEventListener("keydown", (e) => {
    if (e.code !== "Space" || !store.video || e.metaKey || e.ctrlKey || e.altKey) return;
    if ((e.target as HTMLElement)?.matches?.("input[type=text], select, textarea, button, [role=menuitem]")) return;
    e.preventDefault();
    stage.toggle();
  });

  await setupDragAndDrop();

  if (!info.ffmpegFound) {
    toast("ffmpeg wasn't found. Reinstall FillernCut; exporting and previews need it.", { kind: "error", timeout: 0 });
  }

  document.body.classList.remove("booting");
  // Notify mode: check in the background, without holding up the app.
  void launchCheck();
}

/** Undo / Redo: header buttons, ⌘Z / ⇧⌘Z, and the Edit menu items. */
function initUndoUi(): void {
  initUndo();
  const undoBtn = $<HTMLButtonElement>("#btn-undo");
  const redoBtn = $<HTMLButtonElement>("#btn-redo");
  const refresh = () => {
    undoBtn.disabled = store.busy || !canUndo();
    redoBtn.disabled = store.busy || !canRedo();
  };
  onUndoChange(refresh);
  store.on("busy", refresh);
  undoBtn.addEventListener("click", undo);
  redoBtn.addEventListener("click", redo);

  document.addEventListener("keydown", (e) => {
    if (!store.video || !(e.metaKey || e.ctrlKey) || e.altKey) return;
    if (document.querySelector("dialog[open]")) return;
    const key = e.key.toLowerCase();
    if (key === "z") {
      e.preventDefault();
      e.shiftKey ? redo() : undo();
    } else if (key === "y" && e.ctrlKey) {
      e.preventDefault();
      redo();
    }
  });
  // On macOS the Edit menu owns ⌘Z and forwards it here.
  void listen("menu-undo", () => store.video && !document.querySelector("dialog[open]") && undo());
  void listen("menu-redo", () => store.video && !document.querySelector("dialog[open]") && redo());
}

async function setupDragAndDrop(): Promise<void> {
  const hint = $("#drop-hint");
  try {
    await getCurrentWebview().onDragDropEvent(async (event) => {
      const p = event.payload;
      if (p.type === "enter" || p.type === "over") {
        document.body.classList.add("dragging");
        hint.classList.remove("hidden");
      } else if (p.type === "leave") {
        document.body.classList.remove("dragging");
        hint.classList.add("hidden");
      } else if (p.type === "drop") {
        document.body.classList.remove("dragging");
        hint.classList.add("hidden");
        const videos = p.paths.filter((f) => VIDEO_EXTENSIONS.includes(extensionOf(f)));
        const images = p.paths.filter((f) => IMAGE_EXTENSIONS.includes(extensionOf(f)));
        const video = videos[0];
        if (isBatchOpen()) addToBatch(videos);
        else if (videos.length > 1 && store.video) openBatch(videos);
        else if (videos.length > 1) {
          // Several videos and nothing open yet: edit the first, then offer to repeat it on the rest.
          if (await openVideo(video)) {
            const rest = videos.slice(1);
            toast(`Dropped ${videos.length} videos. Set up the first, then apply it to the other ${rest.length}.`, {
              kind: "info",
              timeout: 20000,
              action: { label: "Batch the rest", onClick: () => openBatch(rest) },
            });
          }
        } else if (video) await openVideo(video);
        if (!isBatchOpen()) for (const img of images) await importWatermark(img);
        if (!video && images.length === 0) toast("That file type isn't supported. Drop a video or a PNG/JPG watermark.", { kind: "error" });
      }
    });
  } catch (e) {
    console.warn("Drag & drop unavailable:", errorMessage(e));
  }
}

boot().catch((e) => {
  document.body.classList.remove("booting");
  console.error(e);
  toast(`FillernCut failed to start: ${errorMessage(e)}`, { kind: "error", timeout: 0 });
});
