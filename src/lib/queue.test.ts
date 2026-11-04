import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { DEFAULT_LAYOUT, REFRESH_EVERY_MS, RefreshTimer, parseLayout, placeNew, resolveShown, shortLink, showInColumn, timeAgo, type QueueLayout } from "./queue";

const MIN = 60_000;

describe("shortLink", () => {
  it("drops protocol, www, query and trailing slash", () => {
    expect(shortLink("https://www.instagram.com/reel/C1a2B3c4D5e/?igsh=x")).toBe("instagram.com/reel/C1a2B3c4D5e");
    expect(shortLink("https://www.tiktok.com/@someone/video/7123456789012345678")).toBe("tiktok.com/@someone/video/7123456789012345678");
    expect(shortLink("https://x.com/jack/status/20")).toBe("x.com/jack/status/20");
  });
  it("leaves things that aren't links alone", () => {
    expect(shortLink("not a link")).toBe("not a link");
  });
});

describe("timeAgo", () => {
  const now = 1_800_000_000_000;
  const ago = (seconds: number) => timeAgo(now / 1000 - seconds, now);
  it("counts in the largest sensible unit", () => {
    expect(ago(5)).toBe("just now");
    expect(ago(59)).toBe("just now");
    expect(ago(60)).toBe("1 min ago");
    expect(ago(45 * 60)).toBe("45 min ago");
    expect(ago(3 * 3600)).toBe("3 h ago");
    expect(ago(30 * 3600)).toBe("1 day ago");
    expect(ago(3 * 86400)).toBe("3 days ago");
  });
  it("never goes negative when the clocks disagree", () => {
    expect(ago(-120)).toBe("just now");
  });
});

describe("RefreshTimer", () => {
  beforeEach(() => vi.useFakeTimers());
  afterEach(() => vi.useRealTimers());

  it("refreshes at start and then every 30 minutes", async () => {
    const task = vi.fn(async () => {});
    const timer = new RefreshTimer(task);
    await timer.start();
    expect(task).toHaveBeenCalledTimes(1);
    await vi.advanceTimersByTimeAsync(REFRESH_EVERY_MS - 1);
    expect(task).toHaveBeenCalledTimes(1);
    await vi.advanceTimersByTimeAsync(1);
    expect(task).toHaveBeenCalledTimes(2);
    await vi.advanceTimersByTimeAsync(REFRESH_EVERY_MS);
    expect(task).toHaveBeenCalledTimes(3);
    timer.stop();
  });

  it("pressing refresh restarts the 30 minutes", async () => {
    const task = vi.fn(async () => {});
    const timer = new RefreshTimer(task);
    await timer.start();
    await vi.advanceTimersByTimeAsync(20 * MIN);
    const manual = vi.fn(async () => {});
    await timer.runNow(manual);
    expect(manual).toHaveBeenCalledTimes(1);
    // The automatic refresh that was due at 30 min no longer happens...
    await vi.advanceTimersByTimeAsync(10 * MIN);
    expect(task).toHaveBeenCalledTimes(1);
    // ...it comes 30 min after the button press instead.
    await vi.advanceTimersByTimeAsync(20 * MIN);
    expect(task).toHaveBeenCalledTimes(2);
    timer.stop();
  });

  it("keeps going after a failed refresh", async () => {
    let calls = 0;
    const timer = new RefreshTimer(async () => {
      calls++;
      throw new Error("offline");
    });
    const warn = vi.spyOn(console, "warn").mockImplementation(() => {});
    await timer.start();
    await vi.advanceTimersByTimeAsync(REFRESH_EVERY_MS);
    expect(calls).toBe(2);
    warn.mockRestore();
    timer.stop();
  });

  it("can be armed without running, and then refreshes after a full period", async () => {
    const task = vi.fn(async () => {});
    const timer = new RefreshTimer(task);
    timer.arm();
    expect(task).toHaveBeenCalledTimes(0);
    await vi.advanceTimersByTimeAsync(REFRESH_EVERY_MS - 1);
    expect(task).toHaveBeenCalledTimes(0);
    await vi.advanceTimersByTimeAsync(1);
    expect(task).toHaveBeenCalledTimes(1);
    // Arming again restarts the countdown.
    await vi.advanceTimersByTimeAsync(20 * MIN);
    timer.arm();
    await vi.advanceTimersByTimeAsync(20 * MIN);
    expect(task).toHaveBeenCalledTimes(1);
    await vi.advanceTimersByTimeAsync(10 * MIN);
    expect(task).toHaveBeenCalledTimes(2);
    timer.stop();
    await vi.advanceTimersByTimeAsync(2 * REFRESH_EVERY_MS);
    expect(task).toHaveBeenCalledTimes(2);
  });

  it("catches up when the window returns after the computer slept", async () => {
    const task = vi.fn(async () => {});
    const timer = new RefreshTimer(task);
    await timer.start();
    // Not yet due: nothing happens.
    vi.setSystemTime(Date.now() + 10 * MIN);
    timer.catchUp();
    expect(task).toHaveBeenCalledTimes(1);
    // A whole period passed (the timer itself never fired because the computer slept).
    vi.setSystemTime(Date.now() + 25 * MIN);
    timer.catchUp();
    await vi.advanceTimersByTimeAsync(0);
    expect(task).toHaveBeenCalledTimes(2);
    timer.stop();
    timer.catchUp();
    expect(task).toHaveBeenCalledTimes(2);
  });

  it("never runs two refreshes at once", async () => {
    let release!: () => void;
    const task = vi.fn(() => new Promise<void>((resolve) => (release = resolve)));
    const timer = new RefreshTimer(task);
    const first = timer.start();
    await timer.runNow(); // ignored: one is already running
    expect(task).toHaveBeenCalledTimes(1);
    release();
    await first;
    timer.stop();
  });

  it("stops when asked, even in the middle of a run", async () => {
    const task = vi.fn(async () => {});
    const timer = new RefreshTimer(task);
    await timer.start();
    timer.stop();
    await vi.advanceTimersByTimeAsync(2 * REFRESH_EVERY_MS);
    expect(task).toHaveBeenCalledTimes(1);

    let release!: () => void;
    const slow = new RefreshTimer(() => new Promise<void>((resolve) => (release = resolve)));
    const running = slow.start();
    slow.stop();
    release();
    await running;
    await vi.advanceTimersByTimeAsync(2 * REFRESH_EVERY_MS);
    expect(vi.getTimerCount()).toBe(0);
  });
});

describe("queue layout", () => {
  const layout = (columns: 1 | 2, a: number | null, b: number | null): QueueLayout => ({ columns, shown: [a, b] });

  it("reads back only what makes sense", () => {
    expect(parseLayout({ columns: 1, shown: [3, 4] })).toEqual(layout(1, 3, 4));
    expect(parseLayout({ columns: 7, shown: ["x", -1] })).toEqual(layout(2, null, null));
    expect(parseLayout(null)).toEqual(DEFAULT_LAYOUT);
    expect(parseLayout("junk")).toEqual(DEFAULT_LAYOUT);
  });

  it("fills empty columns with the first queues nobody shows", () => {
    expect(resolveShown(layout(2, null, null), [5, 6, 7]).shown).toEqual([5, 6]);
    expect(resolveShown(layout(2, 7, null), [5, 6, 7]).shown).toEqual([7, 5]);
    expect(resolveShown(layout(2, null, null), [5]).shown).toEqual([5, null]);
    expect(resolveShown(layout(2, null, null), []).shown).toEqual([null, null]);
  });

  it("replaces a queue that was removed and never shows one queue twice", () => {
    expect(resolveShown(layout(2, 9, 6), [5, 6]).shown).toEqual([5, 6]);
    expect(resolveShown(layout(2, 6, 6), [5, 6]).shown).toEqual([6, 5]);
    expect(resolveShown(layout(1, 6, 9), [5, 6]).columns).toBe(1);
  });

  it("swaps when a column picks the queue the other one shows", () => {
    expect(showInColumn(layout(2, 5, 6), 0, 6).shown).toEqual([6, 5]);
    expect(showInColumn(layout(2, 5, 6), 1, 7).shown).toEqual([5, 7]);
  });

  it("shows a new queue straight away", () => {
    expect(placeNew(layout(1, 5, 6), 8).shown).toEqual([8, 6]);
    expect(placeNew(layout(2, 5, 6), 8).shown).toEqual([5, 8]);
    expect(placeNew(layout(2, null, null), 8).shown).toEqual([8, null]);
  });
});
