/** The Telegram queue on the start screen: links posted to your own Telegram channel wait here, one tap opens them. */
import { ask } from "@tauri-apps/plugin-dialog";
import { api, errorMessage, type QueueItem, type QueueView } from "../api";
import { RefreshTimer, shortLink, timeAgo } from "../lib/queue";
import { store } from "../store";
import { $, h } from "./dom";
import { toast } from "./toast";

const PLATFORM_NAMES: Record<string, string> = { tiktok: "TikTok", instagram: "Instagram", twitter: "X" };

const startVisible = () => !$("#view-start").classList.contains("hidden");
const plural = (n: number, one: string, many: string) => `${n} ${n === 1 ? one : many}`;

export function initQueue(fetchLink: (url: string) => Promise<boolean>): void {
  const card = $("#queue-card");
  let view: QueueView | null = null;
  /** Why the last refresh failed, if it did. */
  let failure: string | null = null;
  let refreshing = false;
  /** The channel form is showing although a channel is already set ("Change"). */
  let changing = false;
  let saving = false;
  let formError: string | null = null;
  let typed = "";

  const timer = new RefreshTimer(() => refresh(true));

  /** Fetch new links. `auto` is the 30-minute / launch refresh: quiet unless something arrives. */
  async function refresh(auto: boolean): Promise<void> {
    if (!view?.channel) return; // nothing to read yet
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

  async function saveChannel(): Promise<void> {
    if (saving) return;
    saving = true;
    formError = null;
    render();
    try {
      const result = await api.queueSetChannel(typed);
      view = result.view;
      failure = null;
      changing = false;
      typed = "";
      toast(
        result.added > 0
          ? `Found ${plural(result.added, "link", "links")} in t.me/${result.view.channel}`
          : `Reading t.me/${result.view.channel}. Links you post there show up here.`,
        { kind: "success" },
      );
      // The channel was just read, so the next automatic refresh is a full period away.
      void timer.runNow(async () => {});
    } catch (e) {
      formError = errorMessage(e);
    } finally {
      saving = false;
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
    const sure = await ask("Stop reading the channel? Links still waiting in the queue are removed. The channel itself isn't touched.", {
      title: "Disconnect Telegram",
      kind: "warning",
    });
    if (!sure) return;
    try {
      view = await api.queueDisconnect();
      failure = null;
      changing = false;
      render();
    } catch (e) {
      toast(errorMessage(e), { kind: "error" });
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

  function channelForm(v: QueueView): HTMLElement[] {
    const input = h("input", {
      type: "text",
      spellcheck: false,
      autocapitalize: "off",
      autocorrect: "off",
      placeholder: "@my_links",
      "aria-label": "Telegram channel name",
      value: typed,
      disabled: saving,
      oninput: (e: Event) => (typed = (e.target as HTMLInputElement).value),
    }) as HTMLInputElement;
    const form = h(
      "form",
      { class: "queue-form", autocomplete: "off" },
      input,
      h("button", { class: "btn primary", type: "submit", disabled: saving }, saving ? "Checking…" : "Save"),
      v.channel &&
        h("button", { class: "btn", type: "button", disabled: saving, onclick: () => { changing = false; formError = null; typed = ""; render(); } }, "Cancel"),
    );
    form.addEventListener("submit", (e) => {
      e.preventDefault();
      void saveChannel();
    });
    return [
      h(
        "ol",
        { class: "queue-steps muted small" },
        h("li", {}, "In Telegram, create a channel and make it ", h("strong", {}, "Public"), " with a username (Channel settings → Channel type)."),
        h("li", {}, "Type that username here."),
        h("li", {}, "From your phone, share TikTok, Instagram and X links to the channel. They wait here: tap one to open it."),
      ),
      form,
      formError && h("p", { class: "small status-err queue-error", role: "alert" }, formError),
      h(
        "p",
        { class: "muted small queue-error" },
        "A public channel can be read by anyone who knows its name. Pick a name nobody would guess, and only post links there. FillernCut reads it straight from your computer: no other server is involved.",
      ),
    ].filter(Boolean) as HTMLElement[];
  }

  function render(): void {
    if (!view) return;
    const v = view;
    const parts: (HTMLElement | false)[] = [];
    const showForm = !v.channel || changing;

    const status = failure
      ? h("span", { class: "small status-warn", title: failure }, "Couldn't refresh")
      : v.lastRefreshed
        ? h("span", { class: "small muted" }, `Updated ${new Date(v.lastRefreshed * 1000).toLocaleTimeString(undefined, { hour: "2-digit", minute: "2-digit" })}`)
        : null;
    parts.push(
      h(
        "div",
        { class: "queue-head" },
        h("h2", { class: "section-title" }, "Telegram queue"),
        h("span", { class: "grow" }),
        v.channel && !showForm && status,
        v.channel &&
          !showForm &&
          h("button", { class: "btn small", disabled: refreshing || store.busy, onclick: () => void timer.runNow(() => refresh(false)) }, refreshing ? "Refreshing…" : "Refresh"),
      ),
    );

    if (showForm) {
      parts.push(...(channelForm(v) as HTMLElement[]));
    } else {
      parts.push(
        v.items.length > 0
          ? h("div", { class: "queue-list" }, ...v.items.map(row))
          : h("p", { class: "muted small queue-note" }, "Nothing waiting. Post a link in your channel and it shows up here."),
        failure ? h("p", { class: "small status-warn queue-note" }, failure) : false,
        h(
          "div",
          { class: "queue-foot" },
          h("span", { class: "muted small grow" }, `Reading t.me/${v.channel}`),
          h("button", { class: "btn small", onclick: () => void api.openUrl(`https://t.me/${v.channel}`).catch(() => {}) }, "Open the channel"),
          h("button", { class: "btn small", onclick: () => { changing = true; typed = v.channel ?? ""; formError = null; render(); } }, "Change"),
          h("button", { class: "btn small danger", onclick: () => void disconnect() }, "Disconnect"),
        ),
      );
    }
    // A redraw (a refresh finishing, the busy flag) must not take the cursor from the channel field.
    const before = card.querySelector<HTMLInputElement>(".queue-form input");
    const typing = before !== null && document.activeElement === before;
    const selection = typing ? [before.selectionStart, before.selectionEnd] : null;
    card.replaceChildren(...(parts.filter(Boolean) as HTMLElement[]));
    const after = card.querySelector<HTMLInputElement>(".queue-form input");
    if (after && selection) {
      after.focus();
      after.setSelectionRange(selection[0], selection[1]);
    }
  }

  // From disk straight away, then Telegram: at launch and every 30 minutes while the app is open.
  void api
    .queueGet()
    .then((v) => {
      view = v;
      render();
    })
    .catch(() => {})
    .finally(() => void timer.start());

  store.on("busy", render);
  // "5 min ago" stays right, and a refresh that was missed while the computer slept is made up for.
  setInterval(render, 60_000);
  window.addEventListener("focus", () => timer.catchUp());
  document.addEventListener("visibilitychange", () => document.visibilityState === "visible" && timer.catchUp());
}
