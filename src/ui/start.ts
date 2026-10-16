/** Start screen: choose a file or paste a link. */
import { open } from "@tauri-apps/plugin-dialog";
import { CANCELLED, api, errorMessage } from "../api";
import { PHOTO_EXTENSIONS, VIDEO_EXTENSIONS, detectPlatform } from "../lib/links";
import { withProgress } from "../progress";
import { store } from "../store";
import { $ } from "./dom";
import { toast } from "./toast";

export function initStart(openVideo: (path: string) => Promise<boolean>): { fetchLink: (url: string) => Promise<boolean> } {
  const dropzone = $("#dropzone");
  const form = $<HTMLFormElement>("#link-form");
  const input = $<HTMLInputElement>("#link-input");
  const badge = $("#link-platform");
  const go = $<HTMLButtonElement>("#link-go");
  const progress = $("#link-progress");
  const bar = $(".bar", progress);
  const label = $(".progress-label", progress);

  async function pick(): Promise<void> {
    const picked = await open({
      multiple: false,
      filters: [{ name: "Videos and photos", extensions: [...VIDEO_EXTENSIONS, ...PHOTO_EXTENSIONS] }],
    });
    if (typeof picked === "string") await openVideo(picked);
  }
  dropzone.addEventListener("click", () => void pick());
  dropzone.addEventListener("keydown", (e) => (e.key === "Enter" || e.key === " ") && (e.preventDefault(), void pick()));

  input.addEventListener("input", () => {
    const text = input.value.trim();
    const platform = detectPlatform(text);
    badge.classList.toggle("hidden", text === "");
    badge.classList.toggle("bad", text !== "" && !platform);
    badge.textContent = platform ?? "Unsupported";
  });

  $("#link-cancel").addEventListener("click", () => void api.cancelJob("download"));
  store.on("busy", () => {
    go.disabled = store.busy;
    input.disabled = store.busy;
  });

  /**
   * Download a link and open it in the editor (shared by the link box and the Telegram
   * queue). Returns whether it was opened.
   */
  async function fetchLink(text: string): Promise<boolean> {
    if (store.busy) {
      toast("Wait for the current job to finish first", { kind: "info" });
      return false;
    }
    store.setBusy(true);
    progress.classList.remove("hidden");
    let downloaded: string | null = null;
    let note: string | null = null;
    try {
      const result = await withProgress("download", bar, label, () => api.downloadLink(text), "Looking up the post…");
      downloaded = result.path;
      note = result.note;
      if (input.value.trim() === text) {
        input.value = "";
        badge.classList.add("hidden");
      }
    } catch (err) {
      const msg = errorMessage(err);
      toast(msg === CANCELLED ? "Download cancelled" : msg, { kind: msg === CANCELLED ? "info" : "error", timeout: 12000 });
    } finally {
      progress.classList.add("hidden");
      store.setBusy(false);
    }
    // The busy flag has to be cleared first: opening a video refuses to run while busy.
    const opened = downloaded ? await openVideo(downloaded) : false;
    if (opened && note) toast(note, { kind: "info", timeout: 15000 });
    return opened;
  }

  form.addEventListener("submit", (e) => {
    e.preventDefault();
    const text = input.value.trim();
    if (!text) return toast("Paste a TikTok, Instagram or X link first", { kind: "info" });
    void fetchLink(text);
  });

  return { fetchLink };
}
