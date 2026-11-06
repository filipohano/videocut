import { open } from "@tauri-apps/plugin-dialog";
import { ask } from "@tauri-apps/plugin-dialog";
import { CANCELLED, api, errorMessage, type CookieBrowser, type ModuleView, type Settings, type UpdateMode } from "../api";
import { formatBytes } from "../lib/format";
import { subscribe } from "../progress";
import { checkForUpdates, installUpdate, lastChecked } from "../updates";
import { store } from "../store";
import { $, h } from "./dom";
import { addQueue, deleteQueue } from "./queue";
import { toast } from "./toast";

/** Add-ons being downloaded right now (the download goes on when Settings is closed). */
const installing = new Set<string>();

/** One add-on in Settings: what it is, and Download / Cancel / Remove. */
function addonRow(m: ModuleView): HTMLElement {
  const job = `module-${m.id}` as const;
  const bar = h("div", { class: "bar" }, h("div", { class: "bar-fill" }));
  const label = h("span", { class: "progress-label small" }, "Downloading…");
  const progress = h(
    "div",
    { class: `progress-row${installing.has(m.id) ? "" : " hidden"}` },
    bar,
    label,
    h("button", { class: "btn small", onclick: () => void api.cancelJob(job) }, "Cancel"),
  );
  const off = subscribe(job, (p) => {
    if (!row.isConnected) return off();
    (bar.firstElementChild as HTMLElement).style.width = `${Math.round((p.fraction ?? 0) * 100)}%`;
    label.textContent = `Downloading… ${Math.round((p.fraction ?? 0) * 100)}%`;
  });
  const replace = (next: ModuleView) => {
    store.modules = store.modules.map((x) => (x.id === next.id ? next : x));
    store.emit("modules");
    if (row.isConnected) row.replaceWith(addonRow(next));
  };
  const download = async () => {
    installing.add(m.id);
    progress.classList.remove("hidden");
    button.classList.add("hidden");
    try {
      replace(await api.moduleInstall(m.id));
      toast(`${m.name} is ready. Open a photo and find it in the editor.`, { kind: "success" });
    } catch (e) {
      const msg = errorMessage(e);
      if (msg !== CANCELLED) toast(msg, { kind: "error", timeout: 10000 });
      if (row.isConnected) row.replaceWith(addonRow(m));
    } finally {
      installing.delete(m.id);
    }
  };
  const remove = async () => {
    const ok = await ask(`Remove “${m.name}”? Its features lock until you download it again.`, { title: "Remove add-on", kind: "warning", okLabel: "Remove", cancelLabel: "Keep" });
    if (!ok) return;
    try {
      replace(await api.moduleRemove(m.id));
    } catch (e) {
      toast(errorMessage(e), { kind: "error" });
    }
  };
  const button = m.installed
    ? h("button", { class: "btn small danger", onclick: () => void remove() }, "Remove")
    : h("button", { class: `btn small primary${installing.has(m.id) ? " hidden" : ""}`, onclick: () => void download() }, `Download ${m.name}`);
  const row = h(
    "div",
    { class: "addon" },
    h(
      "div",
      { class: "set-row" },
      h("strong", { class: "grow" }, m.name, h("span", { class: "muted small" }, `  ·  ${formatBytes(m.bytes)}`)),
      m.installed ? h("span", { class: "badge export" }, "Installed") : null,
      button,
    ),
    h("p", { class: "muted small", style: { margin: "0" } }, m.description),
    progress,
  );
  return row;
}

const MODES: { value: UpdateMode; title: string; help: string }[] = [
  { value: "auto", title: "Install automatically at launch", help: "Recommended. Checked before the app opens: if there's a newer version it is installed first, then the app starts." },
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

    const cookies = h("select", { "aria-label": "Browser login for downloads" }, ...BROWSERS.filter((b) => b.value !== "safari" || store.platform === "macos").map((b) => h("option", { value: b.value }, b.label)));
    cookies.value = s.cookiesBrowser ?? "";
    cookies.addEventListener("change", () => void save({ cookiesBrowser: (cookies.value || null) as CookieBrowser | null }));

    // ───────── Telegram queue ─────────
    // Every channel being read, with a way to remove it, and a form to add another.
    const queueList = h("div", { class: "queue-settings-list" });
    const drawQueues = () => {
      queueList.replaceChildren(
        ...(store.queue.length === 0
          ? [h("p", { class: "small muted queue-status" }, "No channels yet.")]
          : store.queue.map((q) =>
              h(
                "div",
                { class: "queue-setting-row" },
                h("span", { class: "grow" }, `@${q.channel}`, h("span", { class: "muted small" }, `  ·  ${q.items.length} waiting`)),
                h("button", { class: "btn small danger", type: "button", onclick: async () => (await deleteQueue(q)) && drawQueues() }, "Remove"),
              ),
            )),
      );
    };
    drawQueues();
    const queueInput = h("input", {
      type: "text",
      spellcheck: false,
      autocapitalize: "off",
      autocorrect: "off",
      placeholder: "@my_links",
      "aria-label": "Telegram channel to add",
    }) as HTMLInputElement;
    const queueAdd = h("button", { class: "btn primary small", type: "submit" }, "Add");
    const queueStatus = h("p", { class: "small muted queue-status", role: "status" });
    const queueForm = h("form", { class: "queue-form", autocomplete: "off" }, queueInput, queueAdd);
    queueForm.addEventListener("submit", async (e) => {
      e.preventDefault();
      if (queueAdd.disabled) return;
      queueAdd.disabled = true;
      queueStatus.className = "small muted queue-status";
      queueStatus.textContent = "Checking the channel…";
      try {
        const result = await addQueue(queueInput.value);
        queueInput.value = "";
        queueStatus.textContent =
          result.added > 0
            ? `Added t.me/${result.view.channel}: found ${result.added === 1 ? "1 link" : `${result.added} links`}.`
            : `Added t.me/${result.view.channel}. Links you post there show up on the start screen.`;
        drawQueues();
      } catch (err) {
        queueStatus.className = "small status-err queue-status";
        queueStatus.textContent = errorMessage(err);
      } finally {
        queueAdd.disabled = false;
      }
    });
    const queueSettings = h(
      "div",
      { class: "queue-setting" },
      queueList,
      h("label", { class: "stack" }, "Add a channel", queueForm),
      queueStatus,
    );

    // ───────── folders ─────────
    const folderRow = (label: string, hint: string, current: string | null, resolve: () => Promise<string>, key: "exportDir") => {
      const path = h("span", { class: "path grow" }, current ?? "…");
      if (!current) resolve().then((d) => (path.textContent = d)).catch(() => {});
      const choose = h("button", { class: "btn small" }, "Choose…");
      choose.addEventListener("click", async () => {
        const dir = await pickFolder();
        if (dir) {
          await save({ [key]: dir });
          path.textContent = dir;
        }
      });
      const reset = h("button", { class: "btn small" }, "Reset");
      reset.addEventListener("click", async () => {
        await save({ [key]: null });
        path.textContent = await resolve();
      });
      return h(
        "div",
        { class: "folder" },
        h("div", { class: "folder-label" }, h("strong", {}, label), h("span", { class: "muted small" }, hint)),
        h("div", { class: "set-row" }, path, choose, reset),
      );
    };
    const finishedRow = folderRow("Finished videos", "Exports land here. Files are named by date and time, e.g. 2026-10-01_15-42-07.mp4.", s.exportDir, api.exportDir, "exportDir");
    const encoderSelect = h(
      "select",
      { "aria-label": "Video encoder" },
      h("option", { value: "auto" }, "Automatic (fastest available)"),
      ...store.encoders.map((e) => h("option", { value: e.id }, e.label)),
    ) as HTMLSelectElement;
    encoderSelect.value = store.encoders.some((e) => e.id === s.encoder) ? s.encoder : "auto";
    encoderSelect.addEventListener("change", async () => {
      await save({ encoder: encoderSelect.value });
      // The encoder exports will actually start with, as the backend resolves it.
      try {
        const info = await api.appInfo();
        store.encoder = info.encoder;
        store.emit("settings");
      } catch (e) {
        toast(errorMessage(e), { kind: "error" });
      }
    });
    const gpuFound = store.encoders.some((e) => e.id !== "libx264");
    const askRow = h(
      "label",
      { class: "check" },
      h("input", {
        type: "checkbox",
        checked: s.askExportLocation,
        onchange: (e: Event) => void save({ askExportLocation: (e.target as HTMLInputElement).checked }),
      }),
      h("span", {}, "Ask where to save every export", h("br"), h("span", { class: "muted small" }, "Off: exports go straight to the Finished folder.")),
    );

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
            `Instagram, some X posts and some YouTube videos (age-restricted ones, or when YouTube asks you to confirm you're not a bot) need you to be logged in. Pick the browser you're logged into; cookies are only read locally by the downloader and never leave your ${store.platform === "macos" ? "Mac" : "PC"}.${store.platform === "macos" ? " Safari needs Full Disk Access for FillernCut (System Settings → Privacy & Security)." : ""}`,
          ),
        ),
        h(
          "section",
          { class: "set-section" },
          h("h3", {}, "Add-ons"),
          h("p", { class: "muted small", style: { margin: "0" } }, "Extra features you download only if you want them, so the app itself stays small."),
          ...store.modules.map(addonRow),
        ),
        h(
          "section",
          { class: "set-section" },
          h("h3", {}, "Telegram queue"),
          queueSettings,
          h(
            "p",
            { class: "muted small", style: { margin: "0" } },
            "Each queue reads one Telegram channel of your own; add as many as you like (also with “+ Add channel” on the start screen). Create a channel, make it Public (Channel settings → Channel type) and type its username here, then share TikTok, Instagram, X and YouTube links to it from your phone. They are read when the app opens and every 30 minutes. Anyone who knows a public channel's name can read it, so pick a name nobody would guess and post only links there.",
          ),
        ),
        h("section", { class: "set-section" }, h("h3", {}, "Folders"), finishedRow, askRow),
        h(
          "section",
          { class: "set-section" },
          h("h3", {}, "Video encoding"),
          h("label", { class: "stack" }, "Encoder for exports", encoderSelect),
          h(
            "p",
            { class: "muted small", style: { margin: "0" } },
            gpuFound
              ? "Automatic uses your graphics chip when it works, and the CPU otherwise. A GPU is much faster; the CPU is slower but compresses a little better. If the chosen encoder fails, the export is retried on the CPU."
              : "No GPU encoder was found on this computer, so exports use the CPU.",
          ),
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
