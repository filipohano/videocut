import "./styles.css";
import { getCurrentWebview } from "@tauri-apps/api/webview";
import { api, errorMessage } from "./api";
import { IMAGE_EXTENSIONS, VIDEO_EXTENSIONS, extensionOf } from "./lib/links";
import { startProgressRouter } from "./progress";
import { initSession, openVideo } from "./session";
import { store } from "./store";
import { initUpdateUi, launchCheck } from "./updates";
import { importWatermark, refreshLibrary } from "./watermarkOps";
import { $ } from "./ui/dom";
import { initExportBar } from "./ui/exportBar";
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
  initUpdateUi();

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

  // Don't block startup on the network.
  void launchCheck();
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
        const video = p.paths.find((f) => VIDEO_EXTENSIONS.includes(extensionOf(f)));
        const images = p.paths.filter((f) => IMAGE_EXTENSIONS.includes(extensionOf(f)));
        if (video) await openVideo(video);
        for (const img of images) await importWatermark(img);
        if (!video && images.length === 0) toast("That file type isn't supported. Drop a video or a PNG/JPG watermark.", { kind: "error" });
      }
    });
  } catch (e) {
    console.warn("Drag & drop unavailable:", errorMessage(e));
  }
}

boot().catch((e) => {
  console.error(e);
  toast(`FillernCut failed to start: ${errorMessage(e)}`, { kind: "error", timeout: 0 });
});
