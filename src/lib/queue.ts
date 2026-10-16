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
