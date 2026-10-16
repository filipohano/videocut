/** The Telegram queue on the start screen: links sent to the bot wait here, one tap opens them. */
import { ask } from "@tauri-apps/plugin-dialog";
import { api, errorMessage, type PairInfo, type QueueItem, type QueueView } from "../api";
import { RefreshTimer, shortLink, timeAgo } from "../lib/queue";
import { store } from "../store";
import { $, h } from "./dom";
import { toast } from "./toast";

const PLATFORM_NAMES: Record<string, string> = { tiktok: "TikTok", instagram: "Instagram", twitter: "X" };

const startVisible = () => !$("#view-start").classList.contains("hidden");
const plural = (n: number, one: string, many: string) => `${n} ${n === 1 ? one : many}`;

export function initQueue(fetchLink: (url: string) => Promise<boolean>): void {
  const card = $("#queue-card");
  const dialog = $<HTMLDialogElement>("#queue-dialog");
  let view: QueueView | null = null;
  /** Why the last refresh failed, if it did. */
  let failure: string | null = null;
  let refreshing = false;

  const timer = new RefreshTimer(() => refresh(true));

  /** Fetch new links. `auto` is the 30-minute / launch refresh: quiet unless something arrives. */
  async function refresh(auto: boolean): Promise<void> {
    refreshing = true;
    render();
    try {
      const result = await api.queueRefresh();
      failure = null;
      view = result.view;
      if (result.added > 0 && !startVisible()) {
        toast(`${plural(result.added, "new link", "new links")} in your Telegram queue`, { kind: "success" });
      }
    } catch (e) {
      failure = errorMessage(e);
      if (!auto) toast(failure, { kind: "error" });
    } finally {
      refreshing = false;
      render();
    }
  }

  async function openItem(item: QueueItem): Promise<void> {
    if (store.busy) return;
    // Only an item that really opened leaves the queue; if it failed it stays to try again.
    if (await fetchLink(item.url)) {
      try {
        view = await api.queueRemove(item.id);
        render();
      } catch (e) {
        toast(errorMessage(e), { kind: "error" });
      }
    }
  }

  async function dismiss(item: QueueItem): Promise<void> {
    try {
      view = await api.queueRemove(item.id);
      render();
    } catch (e) {
      toast(errorMessage(e), { kind: "error" });
    }
  }

  async function disconnect(): Promise<void> {
    const sure = await ask("Disconnect Telegram? Links still waiting in the queue are removed.", {
      title: "Disconnect Telegram",
      kind: "warning",
    });
    if (!sure) return;
    try {
      view = await api.queueDisconnect();
      failure = null;
      render();
    } catch (e) {
      toast(errorMessage(e), { kind: "error" });
    }
  }

  // ───────── connecting a chat ─────────
  async function connect(): Promise<void> {
    try {
      showPairing(await api.queuePair());
    } catch (e) {
      toast(errorMessage(e), { kind: "error" });
    }
  }

  function showPairing(pair: PairInfo): void {
    // A chat counts as new if it was linked after this dialog opened.
    const baseline = Math.max(0, ...(view?.linked ?? []).map((c) => c.linkedAt));
    let poll: ReturnType<typeof setInterval> | undefined;
    const stop = () => clearInterval(poll);

    const status = h("p", { class: "muted small", role: "status" }, "Waiting for you to press Start in Telegram…");
    const again = h("button", { class: "btn small hidden", onclick: () => { dialog.close(); void connect(); } }, "Get a new code");

    const check = async () => {
      if (Date.now() / 1000 > pair.expiresAt) {
        stop();
        status.textContent = "This code has expired.";
        again.classList.remove("hidden");
        return;
      }
      try {
        const result = await api.queueRefresh();
        view = result.view;
        render();
        const fresh = result.view.linked.filter((c) => c.linkedAt > baseline);
        if (fresh.length > 0) {
          stop();
          status.textContent = `Connected as ${fresh.map((c) => c.label).join(", ")} ✓`;
          toast("Telegram connected. Send links to the bot and they show up here.", { kind: "success" });
          setTimeout(() => dialog.close(), 1200);
        }
      } catch {
        // Offline for a moment: try again on the next tick.
      }
    };

    dialog.replaceChildren(
      h("div", { class: "dialog-head" }, h("h2", {}, "Connect Telegram"), h("button", { class: "icon-btn", "aria-label": "Close", onclick: () => dialog.close() }, "×")),
      h(
        "div",
        { class: "dialog-body" },
        h(
          "section",
          { class: "set-section" },
          h("p", { style: { margin: "0" } }, h("strong", {}, "1. "), "Open the bot in Telegram"),
          h(
            "div",
            { class: "set-row" },
            h("button", { class: "btn primary", onclick: () => void api.openUrl(pair.link).catch((e) => toast(errorMessage(e), { kind: "error" })) }, "Open Telegram"),
            h("button", { class: "btn small", onclick: () => void copy(pair.link, "Link copied") }, "Copy link"),
          ),
          h("p", { style: { margin: "0" } }, h("strong", {}, "2. "), "Press ", h("strong", {}, "Start"), " in the chat that opens"),
          status,
          again,
        ),
        h(
          "section",
          { class: "set-section" },
          h("p", { class: "muted small", style: { margin: "0" } }, `Telegram not opening? Send this message to @${pair.bot} yourself:`),
          h("code", { class: "queue-code" }, `/start ${pair.code}`),
          h(
            "p",
            { class: "muted small", style: { margin: "0" } },
            "Only this chat is connected to this copy of FillernCut: links sent from other chats never land here, and yours never land in anyone else's. The code works once and expires after 10 minutes.",
          ),
        ),
      ),
    );
    dialog.addEventListener("close", stop, { once: true });
    dialog.showModal();
    poll = setInterval(() => void check(), 3000);
  }

  async function copy(text: string, done: string): Promise<void> {
    try {
      await navigator.clipboard.writeText(text);
      toast(done, { kind: "success", timeout: 2500 });
    } catch {
      toast(text, { kind: "info" });
    }
  }

  // ───────── drawing the card ─────────
  function row(item: QueueItem): HTMLElement {
    return h(
      "div",
      { class: "queue-row" },
      h(
        "button",
        { class: "queue-item", disabled: store.busy, title: item.url, onclick: () => void openItem(item) },
        h("span", { class: "badge" }, PLATFORM_NAMES[item.platform] ?? item.platform),
        h("span", { class: "queue-link" }, shortLink(item.url)),
        h("span", { class: "muted small" }, timeAgo(item.addedAt)),
      ),
      h("button", { class: "icon-btn", title: "Remove from the queue", "aria-label": "Remove from the queue", onclick: () => void dismiss(item) }, "×"),
    );
  }

  function render(): void {
    if (!view) return;
    const v = view;
    const linked = v.linked.length > 0;
    const parts: (HTMLElement | false)[] = [];

    const status = failure
      ? h("span", { class: "small status-warn", title: failure }, "Couldn't refresh")
      : v.lastRefreshed
        ? h("span", { class: "small muted" }, `Updated ${new Date(v.lastRefreshed * 1000).toLocaleTimeString(undefined, { hour: "2-digit", minute: "2-digit" })}`)
        : null;
    const canRefresh = v.configured && v.connected;
    parts.push(
      h(
        "div",
        { class: "queue-head" },
        h("h2", { class: "section-title" }, "Telegram queue"),
        h("span", { class: "grow" }),
        status,
        canRefresh &&
          h("button", { class: "btn small", disabled: refreshing || store.busy, onclick: () => void timer.runNow(() => refresh(false)) }, refreshing ? "Refreshing…" : "Refresh"),
      ),
    );

    if (!v.configured) {
      parts.push(
        h("p", { class: "muted small queue-note" }, "The Telegram queue isn't set up in this version of FillernCut. If you run the queue server yourself, add its address in Settings."),
        h("div", {}, h("button", { class: "btn small", onclick: () => $("#btn-settings").click() }, "Open Settings")),
      );
    } else if (!linked) {
      parts.push(
        h(
          "p",
          { class: "muted small queue-note" },
          v.connected
            ? "Almost there: press Start in the Telegram chat, or get a new code."
            : "Send TikTok, Instagram and X links to a Telegram bot from your phone, and they wait here. Tap one to open it.",
        ),
        h("div", {}, h("button", { class: "btn primary", onclick: () => void connect() }, "Connect Telegram")),
      );
    } else {
      parts.push(
        v.items.length > 0
          ? h("div", { class: "queue-list" }, ...v.items.map(row))
          : h("p", { class: "muted small queue-note" }, "Nothing waiting. Send a link to the bot in Telegram and it shows up here."),
        h(
          "div",
          { class: "queue-foot" },
          h("span", { class: "muted small grow" }, `Connected as ${v.linked.map((c) => c.label).join(", ")}`),
          v.bot ? h("button", { class: "btn small", onclick: () => void api.openUrl(`https://t.me/${v.bot}`).catch(() => {}) }, "Open the bot") : null,
          h("button", { class: "btn small", onclick: () => void connect() }, "Add a chat"),
          h("button", { class: "btn small danger", onclick: () => void disconnect() }, "Disconnect"),
        ),
      );
    }
    card.replaceChildren(...(parts.filter(Boolean) as HTMLElement[]));
  }

  // From disk straight away, then the network: at launch and every 30 minutes while the app is open.
  void api
    .queueGet()
    .then((v) => {
      view = v;
      render();
    })
    .catch(() => {})
    .finally(() => void timer.start());

  store.on("busy", render);
  store.on("settings", () => {
    void api.queueGet().then((v) => {
      view = v;
      render();
    });
  });
  // "5 min ago" stays right, and a refresh that was missed while the computer slept is made up for.
  setInterval(render, 60_000);
  window.addEventListener("focus", () => timer.catchUp());
  document.addEventListener("visibilitychange", () => document.visibilityState === "visible" && timer.catchUp());
}
