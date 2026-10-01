import { open } from "@tauri-apps/plugin-dialog";
import { api, errorMessage, type CookieBrowser, type Settings, type UpdateMode } from "../api";
import { checkForUpdates, installUpdate, lastChecked } from "../updates";
import { store } from "../store";
import { $, h } from "./dom";
import { toast } from "./toast";

const MODES: { value: UpdateMode; title: string; help: string }[] = [
  { value: "auto", title: "Install automatically at launch", help: "Recommended. A newer version is downloaded, installed and the app restarts by itself." },
  { value: "notify", title: "Tell me, and let me decide", help: "Shows an “out of date” banner with an Update button." },
  { value: "manual", title: "Only when I check", help: "No check at launch. Use “Check now”." },
];

const BROWSERS: { value: CookieBrowser | ""; label: string }[] = [
  { value: "", label: "Don't use a browser login" },
  { value: "safari", label: "Safari" },
  { value: "chrome", label: "Chrome" },
  { value: "firefox", label: "Firefox" },
  { value: "brave", label: "Brave" },
  { value: "edge", label: "Edge" },
];

async function save(patch: Partial<Settings>): Promise<void> {
  try {
    store.settings = await api.saveSettings({ ...store.settings, ...patch });
    store.emit("settings");
  } catch (e) {
    toast(errorMessage(e), { kind: "error" });
  }
}

async function pickFolder(): Promise<string | null> {
  const dir = await open({ directory: true, multiple: false });
  return typeof dir === "string" ? dir : null;
}

export function initSettingsDialog(): void {
  const dialog = $<HTMLDialogElement>("#settings-dialog");
  $("#btn-settings").addEventListener("click", () => {
    render();
    dialog.showModal();
  });
  dialog.addEventListener("click", (e) => e.target === dialog && dialog.close());

  function render(): void {
    const s = store.settings;

    // ───────── updates ─────────
    const status = h("span", { class: "small muted" });
    const installBtn = h("button", { class: "btn primary small hidden" }, "Install & restart");
    const checkBtn = h("button", { class: "btn small" }, "Check now");

    const showStatus = () => {
      const last = lastChecked();
      const when = last ? ` Last checked ${last.toLocaleString()}.` : "";
      if (store.update) {
        status.className = "small status-warn";
        status.textContent = `Out of date — version ${store.update.version} is available.${when}`;
        installBtn.classList.remove("hidden");
      } else {
        status.className = "small status-ok";
        status.textContent = `You're up to date.${when}`;
        installBtn.classList.add("hidden");
      }
    };
    if (lastChecked() || store.update) showStatus();
    else status.textContent = "Not checked yet.";

    checkBtn.addEventListener("click", async () => {
      checkBtn.disabled = true;
      status.className = "small muted";
      status.textContent = "Checking…";
      const res = await checkForUpdates();
      checkBtn.disabled = false;
      if (res.status === "error") {
        status.className = "small status-err";
        status.textContent = res.message;
      } else showStatus();
    });
    installBtn.addEventListener("click", async () => {
      dialog.close();
      try {
        await installUpdate();
      } catch (e) {
        toast(`Update failed: ${errorMessage(e)}`, { kind: "error" });
      }
    });

    const modeRows = MODES.map((m) =>
      h(
        "label",
        { class: "check" },
        h("input", {
          type: "radio",
          name: "update-mode",
          value: m.value,
          checked: s.updateMode === m.value,
          onchange: () => void save({ updateMode: m.value }),
        }),
        h("span", {}, h("strong", {}, m.title), h("br"), h("span", { class: "muted small" }, m.help)),
      ),
    );

    // ───────── downloader ─────────
    const ytVersion = h("span", { class: "small muted" }, "…");
    api.ytdlpVersion().then((v) => (ytVersion.textContent = `yt-dlp ${v}`)).catch(() => (ytVersion.textContent = "yt-dlp not found"));
    const ytBtn = h("button", { class: "btn small" }, "Update now");
    ytBtn.addEventListener("click", async () => {
      ytBtn.disabled = true;
      ytVersion.textContent = "Updating…";
      try {
        ytVersion.textContent = `yt-dlp ${await api.updateYtdlp()}`;
        toast("Downloader is up to date", { kind: "success" });
      } catch (e) {
        ytVersion.textContent = "Update failed";
        toast(errorMessage(e), { kind: "error" });
      } finally {
        ytBtn.disabled = false;
      }
    });

    const cookies = h("select", { "aria-label": "Browser login for downloads" }, ...BROWSERS.map((b) => h("option", { value: b.value }, b.label)));
    cookies.value = s.cookiesBrowser ?? "";
    cookies.addEventListener("change", () => void save({ cookiesBrowser: (cookies.value || null) as CookieBrowser | null }));

    const dlPath = h("span", { class: "path grow" }, s.downloadDir ?? "~/Movies/FillernCut");
    api.downloadDir().then((d) => !s.downloadDir && (dlPath.textContent = d)).catch(() => {});
    const dlChoose = h("button", { class: "btn small" }, "Choose…");
    dlChoose.addEventListener("click", async () => {
      const dir = await pickFolder();
      if (dir) {
        await save({ downloadDir: dir });
        dlPath.textContent = dir;
      }
    });
    const dlReset = h("button", { class: "btn small" }, "Reset");
    dlReset.addEventListener("click", async () => {
      await save({ downloadDir: null });
      dlPath.textContent = await api.downloadDir();
    });

    // ───────── export ─────────
    const exPath = h("span", { class: "path grow" }, s.exportDir ?? "Same folder as the source video");
    const exChoose = h("button", { class: "btn small" }, "Choose…");
    exChoose.addEventListener("click", async () => {
      const dir = await pickFolder();
      if (dir) {
        await save({ exportDir: dir });
        exPath.textContent = dir;
      }
    });
    const exReset = h("button", { class: "btn small" }, "Reset");
    exReset.addEventListener("click", async () => {
      await save({ exportDir: null });
      exPath.textContent = "Same folder as the source video";
    });

    dialog.replaceChildren(
      h(
        "div",
        { class: "dialog-head" },
        h("h2", {}, "Settings"),
        h("button", { class: "icon-btn", "aria-label": "Close", onclick: () => dialog.close() }, "×"),
      ),
      h(
        "div",
        { class: "dialog-body" },
        h(
          "section",
          { class: "set-section" },
          h("h3", {}, "Updates"),
          h("div", { class: "set-row" }, h("strong", {}, `FillernCut ${store.appVersion}`), h("span", { class: "grow" }), checkBtn, installBtn),
          status,
          ...modeRows,
        ),
        h(
          "section",
          { class: "set-section" },
          h("h3", {}, "Downloads"),
          h(
            "label",
            { class: "check" },
            h("input", {
              type: "checkbox",
              checked: s.autoUpdateYtdlp,
              onchange: (e: Event) => void save({ autoUpdateYtdlp: (e.target as HTMLInputElement).checked }),
            }),
            h("span", {}, "Keep the downloader up to date automatically", h("br"), h("span", { class: "muted small" }, "Instagram, X and TikTok change often; the downloader is refreshed in the background at launch.")),
          ),
          h("div", { class: "set-row" }, ytVersion, h("span", { class: "grow" }), ytBtn),
          h(
            "label",
            { class: "stack" },
            "Use the login from a browser",
            cookies,
          ),
          h(
            "p",
            { class: "muted small", style: { margin: "0" } },
            "Instagram (and some X posts) need you to be logged in. Pick the browser you're logged into; cookies are only read locally by the downloader and never leave your Mac. Safari needs Full Disk Access for FillernCut (System Settings → Privacy & Security).",
          ),
          h("div", { class: "set-row" }, h("span", { class: "muted small" }, "Save downloads to"), dlPath, dlChoose, dlReset),
        ),
        h(
          "section",
          { class: "set-section" },
          h("h3", {}, "Export"),
          h("div", { class: "set-row" }, h("span", { class: "muted small" }, "Default folder"), exPath, exChoose, exReset),
        ),
        h(
          "section",
          { class: "set-section" },
          h("h3", {}, "About"),
          h(
            "div",
            { class: "set-row" },
            h("span", { class: "small muted grow" }, "Crop, trim and watermark videos locally with ffmpeg. Free and open source."),
            h("button", { class: "btn small", onclick: () => void api.openUrl("https://github.com/filipohano/videocut") }, "GitHub"),
          ),
        ),
      ),
    );
  }
}
