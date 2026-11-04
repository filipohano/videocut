/**
 * The Telegram queue on the start screen: links posted to your own Telegram channels wait here,
 * one tap opens them. You can have many queues (one per channel, added with "+"); one or two
 * columns show them, each with a drop-down to pick which queue it shows.
 */
import { ask } from "@tauri-apps/plugin-dialog";
import { api, errorMessage, type QueueItem, type QueueRefresh, type QueueView } from "../api";
import {
  DEFAULT_LAYOUT,
  RefreshTimer,
  parseLayout,
  placeNew,
  resolveShown,
  shortLink,
  showInColumn,
  timeAgo,
  type QueueLayout,
} from "../lib/queue";
import { store } from "../store";
import { $, h } from "./dom";
import { toast } from "./toast";

const PLATFORM_NAMES: Record<string, string> = { tiktok: "TikTok", instagram: "Instagram", twitter: "X", youtube: "YouTube" };
const LAYOUT_KEY = "fillerncut.queueLayout";
/** Background refreshes at launch are spread out, so Telegram doesn't get every channel at once. */
const STAGGER_MS = 1500;

const startVisible = () => !$("#view-start").classList.contains("hidden");
const plural = (n: number, one: string, many: string) => `${n} ${n === 1 ? one : many}`;

// ───────── adding and removing queues (also used by Settings) ─────────

/** Read a channel and add a queue for it. Throws the reason it couldn't. */
export async function addQueue(input: string): Promise<QueueRefresh> {
  const result = await api.queueAdd(input);
  store.queue = [...store.queue.filter((q) => q.id !== result.view.id), result.view];
  store.emit("queue");
  return result;
}

/** Stop reading a channel, after asking. Returns true if it was removed. */
export async function deleteQueue(q: QueueView): Promise<boolean> {
  const waiting = q.items.length;
  const ok = await ask(
    `Remove the queue for @${q.channel}?${waiting ? ` The ${plural(waiting, "link", "links")} waiting in it will be forgotten.` : ""} The Telegram channel itself isn't touched.`,
    { title: "Remove queue", kind: "warning", okLabel: "Remove", cancelLabel: "Keep" },
  );
  if (!ok) return false;
  try {
    await api.queueDelete(q.id);
    store.queue = store.queue.filter((x) => x.id !== q.id);
    store.emit("queue");
    return true;
  } catch (e) {
    toast(errorMessage(e), { kind: "error" });
    return false;
  }
}

function loadLayout(): QueueLayout {
  try {
    return parseLayout(JSON.parse(localStorage.getItem(LAYOUT_KEY) ?? "null"));
  } catch {
    return { ...DEFAULT_LAYOUT, shown: [null, null] };
  }
}

function saveLayout(layout: QueueLayout): void {
  try {
    localStorage.setItem(LAYOUT_KEY, JSON.stringify(layout));
  } catch {
    /* storage unavailable: the layout just isn't remembered */
  }
}

export function initQueue(fetchLink: (url: string) => Promise<boolean>): void {
  const root = $("#queue-card");
  /** Why the last refresh of a queue failed, if it did. */
  const failure = new Map<number, string>();
  const refreshing = new Set<number>();
  /** Every queue has its own countdown: pressing one's Refresh doesn't postpone the others. */
  const timers = new Map<number, RefreshTimer>();
  let layout = resolveShown(loadLayout(), store.queue.map((q) => q.id));

  const byId = (id: number | null) => (id === null ? undefined : store.queue.find((q) => q.id === id));
  const visibleIds = () => layout.shown.slice(0, layout.columns).filter((x): x is number => x !== null);

  function setLayout(next: QueueLayout): void {
    layout = resolveShown(next, store.queue.map((q) => q.id));
    saveLayout(layout);
    render();
  }

  function replace(view: QueueView): void {
    const i = store.queue.findIndex((q) => q.id === view.id);
    if (i >= 0) store.queue[i] = view;
  }

  /** Fetch new links for one queue. `auto` is the 30-minute / launch refresh: quiet unless something arrives. */
  async function refresh(id: number, auto: boolean): Promise<void> {
    if (!byId(id)) return;
    refreshing.add(id);
    render();
    try {
      const result = await api.queueRefresh(id);
      failure.delete(id);
      replace(result.view);
      // Say so when the new links can't be seen right now.
      if (result.added > 0 && (!startVisible() || !visibleIds().includes(id))) {
        toast(`${plural(result.added, "new link", "new links")} from @${result.view.channel}`, { kind: "success" });
      }
    } catch (e) {
      if (!byId(id)) return; // removed while it was being read
      failure.set(id, errorMessage(e));
      if (!auto) toast(failure.get(id)!, { kind: "error" });
    } finally {
      refreshing.delete(id);
      render();
    }
  }

  async function openItem(id: number, item: QueueItem): Promise<void> {
    if (store.busy) return;
    // Only an item that really opened leaves the queue; if it failed it stays to try again.
    if (await fetchLink(item.url)) await dismiss(id, item);
  }

  async function dismiss(id: number, item: QueueItem): Promise<void> {
    try {
      replace(await api.queueRemoveItem(id, item.id));
      render();
    } catch (e) {
      toast(errorMessage(e), { kind: "error" });
    }
  }

  // ───────── the "+" form: built once, so typing isn't lost when the queue redraws ─────────
  const addInput = h("input", {
    type: "text",
    spellcheck: false,
    autocapitalize: "off",
    autocorrect: "off",
    placeholder: "@my_links or https://t.me/my_links",
    "aria-label": "Telegram channel to add",
  });
  const addSubmit = h("button", { class: "btn primary small", type: "submit" }, "Add");
  const addCancel = h("button", { class: "btn small", type: "button" }, "Cancel");
  const addStatus = h("p", { class: "small muted queue-status", role: "status" });
  const addForm = h("form", { class: "queue-add", autocomplete: "off" }, h("div", { class: "queue-form" }, addInput, addSubmit, addCancel), addStatus);
  let adding = false;

  function openAdd(): void {
    adding = true;
    addStatus.textContent = "";
    render();
    addInput.focus();
  }
  function closeAdd(): void {
    adding = false;
    addInput.value = "";
    addStatus.textContent = "";
    render();
  }
  addCancel.addEventListener("click", closeAdd);
  addInput.addEventListener("keydown", (e) => e.key === "Escape" && store.queue.length > 0 && closeAdd());
  addForm.addEventListener("submit", async (e) => {
    e.preventDefault();
    if (addSubmit.disabled) return;
    addSubmit.disabled = true;
    addStatus.className = "small muted queue-status";
    addStatus.textContent = "Checking the channel…";
    try {
      const result = await addQueue(addInput.value);
      toast(
        result.added > 0
          ? `Added @${result.view.channel}: ${plural(result.added, "link", "links")} waiting`
          : `Added @${result.view.channel}. Links you post there show up here.`,
        { kind: "success" },
      );
      closeAdd();
    } catch (err) {
      addStatus.className = "small status-err queue-status";
      addStatus.textContent = errorMessage(err);
    } finally {
      addSubmit.disabled = false;
    }
  });

  // ───────── drawing ─────────
  function row(id: number, item: QueueItem): HTMLElement {
    return h(
      "div",
      { class: "queue-row" },
      h(
        "button",
        { class: "queue-item", disabled: store.busy, title: item.url, onclick: () => void openItem(id, item) },
        h("span", { class: "badge" }, PLATFORM_NAMES[item.platform] ?? item.platform),
        h("span", { class: "queue-link" }, shortLink(item.url)),
        h("span", { class: "muted small queue-age" }, timeAgo(item.addedAt)),
      ),
      h("button", { class: "icon-btn", title: "Remove from the queue", "aria-label": "Remove from the queue", onclick: () => void dismiss(id, item) }, "×"),
    );
  }

  /** The drop-down that picks which queue a column shows, with how many links wait in each. */
  function picker(col: 0 | 1, current: QueueView): HTMLElement {
    const select = h(
      "select",
      { class: "queue-picker", "aria-label": layout.columns === 2 ? `Queue in the ${col === 0 ? "left" : "right"} column` : "Queue" },
      ...store.queue.map((q) => h("option", { value: String(q.id), selected: q.id === current.id }, `@${q.channel} (${q.items.length})`)),
    );
    select.title = `t.me/${current.channel}`;
    select.addEventListener("change", () => setLayout(showInColumn(layout, col, Number(select.value))));
    return select;
  }

  function column(col: 0 | 1, q: QueueView): HTMLElement {
    const id = q.id;
    const status = failure.get(id)
      ? h("span", { class: "small status-warn", title: failure.get(id)! }, "Couldn't refresh")
      : q.lastRefreshed
        ? h("span", { class: "small muted" }, `Updated ${new Date(q.lastRefreshed * 1000).toLocaleTimeString(undefined, { hour: "2-digit", minute: "2-digit" })}`)
        : null;
    return h(
      "section",
      { class: "card queue-col", "data-queue": String(id) },
      h(
        "div",
        { class: "queue-head" },
        picker(col, q),
        h("span", { class: "grow" }),
        status,
        h(
          "button",
          { class: "btn small", disabled: refreshing.has(id) || store.busy, onclick: () => void timers.get(id)?.runNow(() => refresh(id, false)) },
          refreshing.has(id) ? "Refreshing…" : "Refresh",
        ),
      ),
      q.items.length > 0
        ? h("div", { class: "queue-list" }, ...q.items.map((item) => row(id, item)))
        : h("p", { class: "muted small queue-note" }, "Nothing waiting. Post a link in the channel and it shows up here."),
      failure.get(id) ? h("p", { class: "small status-warn queue-note" }, failure.get(id)!) : null,
      h(
        "div",
        { class: "queue-foot" },
        h("button", { class: "btn small", onclick: () => void api.openUrl(`https://t.me/${q.channel}`).catch(() => {}) }, "Open the channel"),
        h("button", { class: "btn small subtle", onclick: () => void deleteQueue(q) }, "Remove queue"),
      ),
    );
  }

  function emptyColumn(): HTMLElement {
    return h(
      "section",
      { class: "card queue-col empty" },
      h("p", { class: "muted small queue-note" }, "Add another Telegram channel to see two queues side by side."),
      h("div", {}, h("button", { class: "btn small", onclick: openAdd }, "+ Add a channel")),
    );
  }

  function layoutToggle(): HTMLElement {
    const option = (n: 1 | 2, label: string) =>
      h(
        "button",
        { class: "seg-btn", type: "button", "aria-pressed": String(layout.columns === n), title: `Show ${label.toLowerCase()}`, onclick: () => setLayout({ ...layout, columns: n }) },
        label,
      );
    return h("div", { class: "seg", role: "group", "aria-label": "Columns" }, option(1, "1 column"), option(2, "2 columns"));
  }

  // The parts of the queue: the head and the columns are redrawn, the "+" form between them is
  // never rebuilt or moved (that would take the cursor out of it).
  const head = h("div", { class: "queue-toolbar" });
  const body = h("div", { class: "queue-columns" });
  root.replaceChildren(head, addForm, body);

  let pending = false;
  function render(): void {
    // Redrawing would close a drop-down someone has open: wait until they're done with it.
    const focused = document.activeElement;
    if (focused instanceof HTMLSelectElement && root.contains(focused)) {
      if (!pending) {
        pending = true;
        focused.addEventListener("blur", () => {
          pending = false;
          render();
        }, { once: true });
      }
      return;
    }
    if (store.queue.length === 0) {
      // Nothing set up yet: an explanation and the form to add the first channel.
      root.className = "card queue-col queue-intro";
      head.className = "queue-intro-head";
      head.replaceChildren(
        h("h2", { class: "section-title" }, "Telegram queue"),
        h(
          "p",
          { class: "muted small queue-note" },
          "Share TikTok, Instagram, X and YouTube links from your phone to a public Telegram channel of your own, and they wait here: tap one to open it. Add as many channels as you like, and show one or two queues at a time.",
        ),
      );
      addForm.classList.remove("hidden");
      addCancel.classList.add("hidden");
      body.replaceChildren();
      return;
    }
    root.className = "queue-wrap";
    head.className = "queue-toolbar";
    head.replaceChildren(
      h("h2", { class: "section-title" }, "Telegram queue"),
      h("span", { class: "grow" }),
      layoutToggle(),
      h("button", { class: "btn small primary", disabled: adding, title: "Read another Telegram channel", onclick: openAdd }, "+ Add channel"),
    );
    addForm.classList.toggle("hidden", !adding);
    addCancel.classList.remove("hidden");
    const cols: HTMLElement[] = [];
    for (const col of layout.columns === 2 ? ([0, 1] as const) : ([0] as const)) {
      const q = byId(layout.shown[col]);
      cols.push(q ? column(col, q) : emptyColumn());
    }
    body.className = `queue-columns${layout.columns === 1 ? " single" : ""}`;
    body.replaceChildren(...cols);
  }

  /** Queues were added or removed (here or in Settings): start or stop their countdowns, and show new ones. */
  function queuesChanged(): void {
    const ids = store.queue.map((q) => q.id);
    for (const [id, timer] of timers)
      if (!ids.includes(id)) {
        timer.stop();
        timers.delete(id);
        failure.delete(id);
      }
    let next = layout;
    for (const id of ids) {
      if (timers.has(id)) continue;
      const timer = new RefreshTimer(() => refresh(id, true));
      timers.set(id, timer);
      // It was just read when it was added, so the next refresh is a full period away.
      timer.arm();
      next = placeNew(next, id);
    }
    setLayout(next);
  }

  // The queues were loaded at launch (from disk); then each is read from Telegram: at launch
  // (one after another) and every 30 minutes while the app is open.
  store.queue.forEach((q, i) => {
    const timer = new RefreshTimer(() => refresh(q.id, true));
    timers.set(q.id, timer);
    setTimeout(() => void timer.start(), i * STAGGER_MS);
  });
  saveLayout(layout);
  render();

  store.on("queue", queuesChanged);
  store.on("busy", render);
  // "5 min ago" stays right, and a refresh that was missed while the computer slept is made up for.
  setInterval(render, 60_000);
  const catchUp = () => [...timers.values()].forEach((t, i) => setTimeout(() => t.catchUp(), i * STAGGER_MS));
  window.addEventListener("focus", catchUp);
  document.addEventListener("visibilitychange", () => document.visibilityState === "visible" && catchUp());
}
