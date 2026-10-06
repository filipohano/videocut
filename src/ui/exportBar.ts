import { fileManager } from "../lib/platform";
import { save } from "@tauri-apps/plugin-dialog";
import { CANCELLED, api, errorMessage } from "../api";
import { basename } from "../lib/format";
import { buildExportSpec } from "../lib/spec";
import { withProgress } from "../progress";
import { store } from "../store";
import { isDirty, markClean, onUndoChange } from "../undo";
import { flushTextSaves } from "../watermarkOps";
import { $ } from "./dom";
import { toast } from "./toast";

export function initExportBar(): void {
  const btn = $<HTMLButtonElement>("#btn-export");
  const row = $("#export-progress");
  const bar = $(".bar", row);
  const label = $(".progress-label", row);

  $("#export-cancel").addEventListener("click", () => void api.cancelJob("export"));
  // True from a successful export until the next edit (or a new file).
  let finished = false;
  const render = () => {
    const v = store.video;
    btn.disabled = store.busy || finished;
    btn.classList.toggle("is-finished", finished);
    if (finished) btn.textContent = "Export finished";
    else if (v) btn.textContent = v.info.isImage ? "Export photo" : "Export video";
  };
  store.on("busy", render);
  store.on("video", () => {
    finished = false;
    render();
  });
  onUndoChange(() => {
    if (finished && isDirty()) {
      finished = false;
      render();
    }
  });

  btn.addEventListener("click", async () => {
    const v = store.video;
    if (!v || store.busy) return;
    if (store.encoder === "none") return toast("ffmpeg wasn't found, so exporting isn't available.", { kind: "error" });

    const ext = v.info.isImage ? v.imageFormat : "mp4";
    // Finished files go straight to the "Finished" folder, named by time —
    // unless the user asked to be asked where to save every time.
    let output: string | null;
    try {
      const suggested = await api.defaultSavePath(ext);
      output = store.settings.askExportLocation
        ? await save({ defaultPath: suggested, filters: [{ name: ext === "mp4" ? "MP4 video" : ext.toUpperCase() + " photo", extensions: ext === "jpg" ? ["jpg", "jpeg"] : [ext] }] })
        : suggested;
    } catch (e) {
      return toast(errorMessage(e), { kind: "error" });
    }
    if (!output) return;
    if (!new RegExp(`\\.${ext === "jpg" ? "(jpg|jpeg)" : ext}$`, "i").test(output)) output += `.${ext}`;

    store.setBusy(true);
    row.classList.remove("hidden");
    try {
      // Edited text watermarks must be on disk before ffmpeg reads them.
      await flushTextSaves();
      const saved = await withProgress("export", bar, label, () => api.exportVideo(buildExportSpec(v, output!)), v.info.isImage ? "Saving…" : "Exporting…");
      markClean();
      finished = true;
      toast(`Saved ${basename(saved)}`, {
        kind: "success",
        timeout: 12000,
        action: { label: `Show in ${fileManager(store.platform)}`, onClick: () => void api.revealInFinder(saved) },
      });
    } catch (e) {
      const msg = errorMessage(e);
      toast(msg === CANCELLED ? "Export cancelled" : `Export failed: ${msg}`, { kind: msg === CANCELLED ? "info" : "error" });
    } finally {
      row.classList.add("hidden");
      store.setBusy(false);
      render();
    }
  });
}
