import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { REFRESH_EVERY_MS, RefreshTimer, shortLink, timeAgo } from "./queue";

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
