/**
 * Update checks against GitHub Releases (via the Tauri updater plugin).
 *
 * Modes (Settings → Updates):
 *  • auto   – on launch, download + install a newer version and restart (default)
 *  • notify – on launch, show an "out of date" banner with an Update button
 *  • manual – never check by itself
 */
import { check, type Update } from "@tauri-apps/plugin-updater";
import { relaunch } from "@tauri-apps/plugin-process";
import { api, errorMessage } from "./api";
import { store } from "./store";
import { $, h, show } from "./ui/dom";
import { showOverlay } from "./ui/overlay";

const LAST_CHECK_KEY = "fillerncut.lastUpdateCheck";

let pending: Update | null = null;

export type CheckResult =
  | { status: "available"; version: string; notes: string | null }
  | { status: "current" }
  | { status: "error"; message: string };

export function lastChecked(): Date | null {
  try {
    const raw = localStorage.getItem(LAST_CHECK_KEY);
    return raw ? new Date(raw) : null;
  } catch {
    return null;
  }
}

function markChecked(): void {
  try {
    localStorage.setItem(LAST_CHECK_KEY, new Date().toISOString());
  } catch {
    /* private mode etc. — not important */
  }
}

export async function checkForUpdates(): Promise<CheckResult> {
  try {
    const update = await check({ timeout: 15_000 });
    markChecked();
    if (update) {
      pending = update;
      store.update = { version: update.version, notes: update.body ?? null };
      store.emit("update");
      return { status: "available", version: update.version, notes: update.body ?? null };
    }
    pending = null;
    store.update = null;
    store.emit("update");
    return { status: "current" };
  } catch (e) {
    return { status: "error", message: friendlyUpdateError(errorMessage(e)) };
  }
}

function friendlyUpdateError(msg: string): string {
  if (/pubkey|public key|base64|minisign/i.test(msg)) return "Updates aren't set up in this build yet (missing updater key).";
  if (/could not fetch|404|not found|release json/i.test(msg)) return "No published release was found on GitHub yet.";
  if (/network|connect|dns|timed out|timeout|offline/i.test(msg)) return "Couldn't reach GitHub. Check your internet connection.";
  return msg;
}

/** Download, install and restart. Throws if anything goes wrong (the overlay is closed first). */
export async function installUpdate(): Promise<void> {
  if (!pending) throw new Error("There's no update to install");
  const update = pending;
  const overlay = showOverlay(`Updating to FillernCut ${update.version}…`, "Downloading the update. The app restarts by itself when it's done.");
  store.setBusy(true);
  let total = 0;
  let done = 0;
  try {
    await update.download((ev) => {
      if (ev.event === "Started") total = ev.data.contentLength ?? 0;
      else if (ev.event === "Progress") {
        done += ev.data.chunkLength;
        overlay.setProgress(total > 0 ? Math.min(done / total, 1) : null);
      } else overlay.setProgress(1);
    });
    overlay.setMessage("Installing…");
    await update.install();
    await relaunch();
  } catch (e) {
    overlay.close();
    store.setBusy(false);
    throw e;
  }
}

/** Runs once at startup, according to the user's update mode. */
export async function launchCheck(): Promise<void> {
  const mode = store.settings.updateMode;
  if (mode === "manual") return;
  const result = await checkForUpdates();
  if (result.status !== "available") {
    if (result.status === "error") console.warn("Update check failed:", result.message);
    return;
  }
  // Never restart under someone who already started a download or export.
  if (mode === "auto" && !store.busy) {
    try {
      await installUpdate();
    } catch (e) {
      store.update = { version: result.version, notes: result.notes, error: errorMessage(e) };
      store.emit("update");
    }
  }
}

// ───────────────────────── banner + header chip ─────────────────────────
export function initUpdateUi(): void {
  const banner = $("#update-banner");
  const chip = $("#btn-update-chip");
  let dismissed = false;

  function releaseUrl(version: string): string {
    return `https://github.com/filipohano/videocut/releases/tag/v${version}`;
  }

  async function install(): Promise<void> {
    try {
      await installUpdate();
    } catch (e) {
      if (store.update) store.update = { ...store.update, error: errorMessage(e) };
      dismissed = false;
      store.emit("update");
    }
  }

  function render(): void {
    const u = store.update;
    show(chip, !!u);
    if (!u) {
      show(banner, false);
      return;
    }
    chip.textContent = `Update ${u.version} available`;
    banner.classList.toggle("error", !!u.error);
    banner.replaceChildren(
      h(
        "div",
        { class: "banner-text" },
        u.error
          ? h("span", {}, h("strong", {}, "Couldn't install the update. "), u.error)
          : h("span", {}, h("strong", {}, "FillernCut is out of date. "), `Version ${u.version} is available — you're on ${store.appVersion}.`),
      ),
      h("button", { class: "btn primary small", onclick: () => void install() }, u.error ? "Try again" : "Update now"),
      h("button", { class: "btn small", onclick: () => void api.openUrl(releaseUrl(u.version)) }, "What's new"),
      h("button", { class: "icon-btn", "aria-label": "Hide", onclick: () => ((dismissed = true), show(banner, false)) }, "×"),
    );
    show(banner, !dismissed);
  }

  chip.addEventListener("click", () => {
    dismissed = false;
    render();
  });
  store.on("update", render);
  render();
}
