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
import { showOverlay, type OverlayHandle } from "./ui/overlay";

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

/** Where install progress is shown: the in-app overlay, or the launch splash screen. */
export type ProgressUi = Pick<OverlayHandle, "setMessage" | "setProgress" | "close">;

/** Download, install and restart. Throws if anything goes wrong (the progress UI is closed first). */
export async function installUpdate(ui?: ProgressUi): Promise<void> {
  if (!pending) throw new Error("There's no update to install");
  const update = pending;
  const overlay =
    ui ?? showOverlay(`Updating to FillernCut ${update.version}…`, "Downloading the update. The app restarts by itself when it's done.");
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

const SPLASH_CHECK_TIMEOUT_MS = 10_000;
const SKIP_BUTTON_AFTER_MS = 3_000;

/**
 * Before the app opens (mode "auto" only): show the splash screen, look for a newer
 * release and, if there is one, download and install it and restart straight into it,
 * so you never use an old version. With no update (or offline, or if the check is slow
 * or fails) it simply lets the app start. It never traps you on the splash screen.
 */
export async function launchGate(): Promise<void> {
  if (store.settings.updateMode !== "auto") return;
  const msg = $("#splash-msg");
  const bar = $("#splash-bar");
  const fill = $(".bar-fill", bar);
  const btn = $<HTMLButtonElement>("#splash-skip");
  const ui: ProgressUi = {
    setMessage: (t) => (msg.textContent = t),
    setProgress(f) {
      bar.classList.toggle("indeterminate", f === null);
      fill.style.width = f === null ? "" : `${Math.round(f * 100)}%`;
    },
    close() {},
  };

  msg.textContent = "Checking for updates…";
  btn.textContent = "Skip";
  const showSkip = setTimeout(() => btn.classList.remove("hidden"), SKIP_BUTTON_AFTER_MS);
  const skipped = new Promise<"skip">((resolve) => (btn.onclick = () => resolve("skip")));
  const timedOut = new Promise<"timeout">((resolve) => setTimeout(() => resolve("timeout"), SPLASH_CHECK_TIMEOUT_MS));
  const result = await Promise.race([checkForUpdates(), skipped, timedOut]);
  clearTimeout(showSkip);
  btn.classList.add("hidden");
  if (typeof result === "string" || result.status !== "available") return; // up to date, offline, skipped…

  msg.textContent = `Updating to version ${result.version}…`;
  ui.setProgress(null);
  try {
    await installUpdate(ui);
    // The app is restarting into the new version; don't start the old UI in the meantime.
    msg.textContent = "Restarting…";
    await new Promise<never>(() => {});
  } catch (e) {
    msg.textContent = `Couldn't install the update (${errorMessage(e)}). You can keep using this version.`;
    ui.setProgress(0);
    btn.textContent = "Continue";
    btn.classList.remove("hidden");
    await new Promise<void>((resolve) => (btn.onclick = () => resolve()));
    btn.classList.add("hidden");
    store.update = { version: result.version, notes: result.notes, error: errorMessage(e) };
    store.emit("update");
  }
}

/** After the app has opened: in "notify" mode, look for an update in the background and show the banner. */
export async function launchCheck(): Promise<void> {
  if (store.settings.updateMode !== "notify") return;
  const result = await checkForUpdates();
  if (result.status === "error") console.warn("Update check failed:", result.message);
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
