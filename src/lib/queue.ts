/** Helpers for the Telegram queue on the start screen. */

/** The queue is fetched at launch and then every 30 minutes while the app is open. */
export const REFRESH_EVERY_MS = 30 * 60 * 1000;

/**
 * `https://www.instagram.com/reel/C1a2B3c4D5e/?igsh=x` → `instagram.com/reel/C1a2B3c4D5e`:
 * no protocol, no www, no query, no trailing slash.
 */
export function shortLink(url: string): string {
  try {
    const u = new URL(url);
    const path = u.pathname.replace(/\/+$/, "");
    return `${u.hostname.replace(/^www\./, "")}${path}`;
  } catch {
    return url;
  }
}

/** "just now", "5 min ago", "3 h ago", "2 days ago". */
export function timeAgo(unixSeconds: number, nowMs = Date.now()): string {
  const seconds = Math.max(0, Math.floor(nowMs / 1000 - unixSeconds));
  if (seconds < 60) return "just now";
  const minutes = Math.floor(seconds / 60);
  if (minutes < 60) return `${minutes} min ago`;
  const hours = Math.floor(minutes / 60);
  if (hours < 24) return `${hours} h ago`;
  const days = Math.floor(hours / 24);
  return days === 1 ? "1 day ago" : `${days} days ago`;
}

/**
 * Runs a task when started and then again `period` ms after the *last* run, so pressing
 * Refresh (`runNow`) pushes the next automatic refresh a full period away. A run never
 * overlaps another, and a failing run doesn't stop the cycle.
 */
export class RefreshTimer {
  private handle: ReturnType<typeof setTimeout> | undefined;
  private running = false;
  private stopped = true;
  private lastRun = 0;

  constructor(
    private readonly task: () => Promise<void>,
    private readonly period = REFRESH_EVERY_MS,
  ) {}

  /** Run now, then keep going every period. */
  start(): Promise<void> {
    this.stopped = false;
    return this.runNow();
  }

  /** Start the countdown without running now (the thing was only just refreshed). */
  arm(): void {
    this.stopped = false;
    clearTimeout(this.handle);
    this.lastRun = Date.now();
    this.handle = setTimeout(() => void this.runNow(), this.period);
  }

  /** Run now (a button press, or the timer) and restart the countdown. `task` replaces the default for this run. */
  async runNow(task: () => Promise<void> = this.task): Promise<void> {
    if (this.running) return;
    this.running = true;
    clearTimeout(this.handle);
    try {
      await task();
    } catch (error) {
      console.warn("queue refresh", error);
    } finally {
      this.running = false;
      this.lastRun = Date.now();
      if (!this.stopped) this.handle = setTimeout(() => void this.runNow(), this.period);
    }
  }

  /**
   * Timers stand still while a laptop sleeps, so when the window comes back refresh at
   * once if a whole period has passed since the last run.
   */
  catchUp(): void {
    if (!this.stopped && !this.running && Date.now() - this.lastRun >= this.period) void this.runNow();
  }

  stop(): void {
    this.stopped = true;
    clearTimeout(this.handle);
  }
}

// ───────── which queue each column shows ─────────

/** How the queue on the start screen is laid out: one or two columns, and which queue each shows. */
export interface QueueLayout {
  columns: 1 | 2;
  /** The queue id each column shows (the second is kept while there is one column, for later). */
  shown: [number | null, number | null];
}

export const DEFAULT_LAYOUT: QueueLayout = { columns: 2, shown: [null, null] };

/** A layout from whatever was stored; anything unreadable is the default. */
export function parseLayout(raw: unknown): QueueLayout {
  if (!raw || typeof raw !== "object") return { ...DEFAULT_LAYOUT, shown: [null, null] };
  const r = raw as Record<string, unknown>;
  const id = (v: unknown) => (typeof v === "number" && Number.isInteger(v) && v > 0 ? v : null);
  const shown = Array.isArray(r.shown) ? r.shown : [];
  return { columns: r.columns === 1 ? 1 : 2, shown: [id(shown[0]), id(shown[1])] };
}

/**
 * Make every column show a queue that exists, and no two columns the same one: a column whose
 * queue is gone (or is already shown to its left) gets the first queue nobody shows yet.
 */
export function resolveShown(layout: QueueLayout, ids: number[]): QueueLayout {
  const out: [number | null, number | null] = [null, null];
  for (const col of [0, 1] as const) {
    const wanted = layout.shown[col];
    const used = out.filter((x): x is number => x !== null);
    out[col] = wanted !== null && ids.includes(wanted) && !used.includes(wanted) ? wanted : (ids.find((i) => !used.includes(i)) ?? null);
  }
  return { columns: layout.columns, shown: out };
}

/** Choose a queue in a column. If the other column showed it, the two swap. */
export function showInColumn(layout: QueueLayout, col: 0 | 1, id: number): QueueLayout {
  const shown: [number | null, number | null] = [...layout.shown];
  const other = col === 0 ? 1 : 0;
  if (shown[other] === id) shown[other] = shown[col];
  shown[col] = id;
  return { columns: layout.columns, shown };
}

/** A queue was just added: show it where it is seen — the left column, or with two the right one (or an empty one). */
export function placeNew(layout: QueueLayout, id: number): QueueLayout {
  if (layout.columns === 1) return showInColumn(layout, 0, id);
  return showInColumn(layout, layout.shown[0] === null ? 0 : 1, id);
}
