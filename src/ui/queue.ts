/**
 * The Telegram queue on the start screen: links posted to your own Telegram channels wait here,
 * one tap opens them. Two columns, one per channel; the channels are set in Settings.
 */
import { api, errorMessage, type QueueItem, type QueueView } from "../api";
import { RefreshTimer, shortLink, timeAgo } from "../lib/queue";
import { store } from "../store";
import { $, h } from "./dom";
import { toast } from "./toast";

const PLATFORM_NAMES: Record<string, string> = { tiktok: "TikTok", instagram: "Instagram", twitter: "X", youtube: "YouTube" };

/** The queue has this many columns (the same number the backend keeps). */
export const QUEUE_SLOTS = 2;
export const COLUMN_NAMES = ["Left column", "Right column"];

const startVisible = () => !$("#view-start").classList.contains("hidden");
const plural = (n: number, one: string, many: string) => `${n} ${n === 1 ? one : many}`;

export function initQueue(fetchLink: (url: string) => Promise<boolean>): void {
  const root = $("#queue-card");
  /** Why the last refresh of a column failed, if it did. */
  const failure: (string | null)[] = Array.from({ length: QUEUE_SLOTS }, () => null);
  const refreshing: boolean[] = Array.from({ length: QUEUE_SLOTS }, () => false);
  /** The channel each column was reading when it was last drawn: a change restarts its countdown. */
  const lastChannel: (string | null)[] = Array.from({ length: QUEUE_SLOTS }, () => null);

  /** Every column has its own countdown: pressing one's Refresh doesn't postpone the other's. */
  const timers = Array.from({ length: QUEUE_SLOTS }, (_, slot) => new RefreshTimer(() => refresh(slot, true)));

  const column = (slot: number): QueueView | undefined => store.queue[slot];

  function setColumn(view: QueueView): void {
    store.queue[view.slot] = view;
    store.emit("queue");
  }

  /** Fetch new links for one column. `auto` is the 30-minute / launch refresh: quiet unless something arrives. */
  async function refresh(slot: number, auto: boolean): Promise<void> {
    if (!column(slot)?.channel) return; // nothing to read yet
    refreshing[slot] = true;
    render();
    try {
      const result = await api.queueRefresh(slot);
      failure[slot] = null;
      store.queue[slot] = result.view;
      if (result.added > 0 && !startVisible()) {
        toast(`${plural(result.added, "new link", "new links")} from @${result.view.channel}`, { kind: "success" });
      }
    } catch (e) {
      failure[slot] = errorMessage(e);
      if (!auto) toast(failure[slot]!, { kind: "error" });
    } finally {
      refreshing[slot] = false;
      render();
    }
  }

  async function openItem(slot: number, item: QueueItem): Promise<void> {
    if (store.busy) return;
    // Only an item that really opened leaves the queue; if it failed it stays to try again.
    if (await fetchLink(item.url)) await dismiss(slot, item);
  }

  async function dismiss(slot: number, item: QueueItem): Promise<void> {
    try {
      setColumn(await api.queueRemove(slot, item.id));
    } catch (e) {
      toast(errorMessage(e), { kind: "error" });
    }
  }

  // ───────── drawing ─────────
  function row(slot: number, item: QueueItem): HTMLElement {
    return h(
      "div",
      { class: "queue-row" },
      h(
        "button",
        { class: "queue-item", disabled: store.busy, title: item.url, onclick: () => void openItem(slot, item) },
        h("span", { class: "badge" }, PLATFORM_NAMES[item.platform] ?? item.platform),
        h("span", { class: "queue-link" }, shortLink(item.url)),
        h("span", { class: "muted small queue-age" }, timeAgo(item.addedAt)),
      ),
      h("button", { class: "icon-btn", title: "Remove from the queue", "aria-label": "Remove from the queue", onclick: () => void dismiss(slot, item) }, "×"),
    );
  }

  const openSettings = () => $("#btn-settings").click();

  function emptyColumn(slot: number): HTMLElement {
    return h(
      "section",
      { class: "card queue-col empty", "data-slot": String(slot) },
      h("div", { class: "queue-head" }, h("h2", { class: "queue-title muted" }, COLUMN_NAMES[slot])),
      h("p", { class: "muted small queue-note" }, "No channel yet. Add a second Telegram channel to collect more links here."),
      h("div", {}, h("button", { class: "btn small", onclick: openSettings }, "Choose a channel in Settings")),
    );
  }

  function filledColumn(slot: number, v: QueueView): HTMLElement {
    const status = failure[slot]
      ? h("span", { class: "small status-warn", title: failure[slot]! }, "Couldn't refresh")
      : v.lastRefreshed
        ? h("span", { class: "small muted" }, `Updated ${new Date(v.lastRefreshed * 1000).toLocaleTimeString(undefined, { hour: "2-digit", minute: "2-digit" })}`)
        : null;
    return h(
      "section",
      { class: "card queue-col", "data-slot": String(slot) },
      h(
        "div",
        { class: "queue-head" },
        h("h2", { class: "queue-title", title: `t.me/${v.channel}` }, `@${v.channel}`),
        h("span", { class: "grow" }),
        status,
        h(
          "button",
          { class: "btn small", disabled: refreshing[slot] || store.busy, onclick: () => void timers[slot].runNow(() => refresh(slot, false)) },
          refreshing[slot] ? "Refreshing…" : "Refresh",
        ),
      ),
      v.items.length > 0
        ? h("div", { class: "queue-list" }, ...v.items.map((item) => row(slot, item)))
        : h("p", { class: "muted small queue-note" }, "Nothing waiting. Post a link in the channel and it shows up here."),
      failure[slot] ? h("p", { class: "small status-warn queue-note" }, failure[slot]!) : null,
      h(
        "div",
        { class: "queue-foot" },
        h("button", { class: "btn small", onclick: () => void api.openUrl(`https://t.me/${v.channel}`).catch(() => {}) }, "Open the channel"),
      ),
    );
  }

  function render(): void {
    const views = Array.from({ length: QUEUE_SLOTS }, (_, slot) => column(slot));
    if (store.queue.length === 0) return; // not loaded yet
    if (views.every((v) => !v?.channel)) {
      // Nothing set up yet: one explanation instead of two empty boxes.
      root.className = "queue-intro";
      root.replaceChildren(
        h(
          "section",
          { class: "card queue-col intro" },
          h("div", { class: "queue-head" }, h("h2", { class: "section-title" }, "Telegram queue")),
          h(
            "p",
            { class: "muted small queue-note" },
            "Share TikTok, Instagram, X and YouTube links from your phone to a public Telegram channel of your own, and they wait here: tap one to open it. You can read two channels side by side.",
          ),
          h("div", {}, h("button", { class: "btn primary", onclick: openSettings }, "Set up in Settings")),
        ),
      );
      return;
    }
    root.className = "queue-columns";
    root.replaceChildren(...views.map((v, slot) => (v?.channel ? filledColumn(slot, v) : emptyColumn(slot))));
  }

  /** A channel was set, changed or removed (in Settings): restart or stop that column's countdown. */
  function channelsChanged(): void {
    for (let slot = 0; slot < QUEUE_SLOTS; slot++) {
      const channel = column(slot)?.channel ?? null;
      if (channel === lastChannel[slot]) continue;
      lastChannel[slot] = channel;
      failure[slot] = null;
      // The channel was just read when it was set, so the next refresh is a full period away.
      if (channel) timers[slot].arm();
      else timers[slot].stop();
    }
    render();
  }

  // The queues were loaded at launch (from disk); then read from Telegram: at launch and every
  // 30 minutes while the app is open.
  for (let slot = 0; slot < QUEUE_SLOTS; slot++) {
    const channel = column(slot)?.channel ?? null;
    lastChannel[slot] = channel;
    if (channel) void timers[slot].start();
  }
  render();

  store.on("queue", channelsChanged);
  store.on("busy", render);
  // "5 min ago" stays right, and a refresh that was missed while the computer slept is made up for.
  setInterval(render, 60_000);
  const catchUp = () => timers.forEach((t) => t.catchUp());
  window.addEventListener("focus", catchUp);
  document.addEventListener("visibilitychange", () => document.visibilityState === "visible" && catchUp());
}
