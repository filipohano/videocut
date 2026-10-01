import { save } from "@tauri-apps/plugin-dialog";
import { CANCELLED, api, errorMessage } from "../api";
import { basename } from "../lib/format";
import { buildExportSpec } from "../lib/spec";
import { withProgress } from "../progress";
import { store } from "../store";
import { flushTextSaves } from "../watermarkOps";
import { $ } from "./dom";
import { toast } from "./toast";

export function initExportBar(): void {
  const btn = $<HTMLButtonElement>("#btn-export");
  const row = $("#export-progress");
  const bar = $(".bar", row);
  const label = $(".progress-label", row);

  $("#export-cancel").addEventListener("click", () => void api.cancelJob("export"));
  store.on("busy", () => {
    btn.disabled = store.busy;
  });

  btn.addEventListener("click", async () => {
    const v = store.video;
    if (!v || store.busy) return;
    if (store.encoder === "none") return toast("ffmpeg wasn't found, so exporting isn't available.", { kind: "error" });

    // Finished videos go straight to the "Finished" folder, named by time —
    // unless the user asked to be asked where to save every time.
    let output: string | null;
    try {
      const suggested = await api.defaultSavePath();
      output = store.settings.askExportLocation
        ? await save({ defaultPath: suggested, filters: [{ name: "MP4 video", extensions: ["mp4"] }] })
        : suggested;
    } catch (e) {
      return toast(errorMessage(e), { kind: "error" });
    }
    if (!output) return;
    if (!/\.mp4$/i.test(output)) output += ".mp4";

    store.setBusy(true);
    row.classList.remove("hidden");
    try {
      // Edited text watermarks must be on disk before ffmpeg reads them.
      await flushTextSaves();
      const saved = await withProgress("export", bar, label, () => api.exportVideo(buildExportSpec(v, output!)), "Exporting…");
      toast(`Saved ${basename(saved)}`, {
        kind: "success",
        timeout: 12000,
        action: { label: "Show in Finder", onClick: () => void api.revealInFinder(saved) },
      });
    } catch (e) {
      const msg = errorMessage(e);
      toast(msg === CANCELLED ? "Export cancelled" : `Export failed: ${msg}`, { kind: msg === CANCELLED ? "info" : "error" });
    } finally {
      row.classList.add("hidden");
      store.setBusy(false);
    }
  });
}
