/** Application state with a minimal topic-based change notification. */
import type { MediaInfo, Settings, WatermarkEntry } from "./api";
import type { Rect } from "./lib/crop";

export interface ActiveWatermark extends WatermarkEntry {
  /** `asset:` URL for <img>. */
  url: string;
}

export interface VideoState {
  /** Original file on disk — the one ffmpeg reads. */
  path: string;
  info: MediaInfo;
  /** What the <video> element plays (the original, or a transcoded proxy). */
  playUrl: string;
  crop: Rect;
  aspect: string;
  trimStart: number;
  trimEnd: number;
  watermarks: ActiveWatermark[];
  quality: number;
  /** photos only: what to export */
  imageFormat: "jpg" | "png";
}

export type Topic =
  | "video" // a video was opened or closed
  | "crop"
  | "trim"
  | "watermarks" // active watermarks changed
  | "library" // saved watermark list changed
  | "selection" // selected watermark changed
  | "quality"
  | "undo" // undo/redo availability changed
  | "settings"
  | "update"
  | "busy"
  | "playhead";

export interface UpdateInfo {
  version: string;
  notes: string | null;
  error?: string;
}

class Store {
  video: VideoState | null = null;
  settings!: Settings;
  library: WatermarkEntry[] = [];
  selectedWatermark: string | null = null;
  appVersion = "";
  encoder: "videotoolbox" | "libx264" | "none" = "videotoolbox";
  update: UpdateInfo | null = null;
  /** True while a download / export / preview is running. */
  busy = false;
  playhead = 0;

  private listeners = new Map<Topic, Set<() => void>>();

  on(topic: Topic | Topic[], fn: () => void): void {
    for (const t of Array.isArray(topic) ? topic : [topic]) {
      if (!this.listeners.has(t)) this.listeners.set(t, new Set());
      this.listeners.get(t)!.add(fn);
    }
  }

  emit(...topics: Topic[]): void {
    const fns = new Set<() => void>();
    for (const t of topics) this.listeners.get(t)?.forEach((f) => fns.add(f));
    fns.forEach((f) => f());
  }

  setBusy(busy: boolean): void {
    this.busy = busy;
    this.emit("busy");
  }

  get frame(): { w: number; h: number } | null {
    return this.video ? { w: this.video.info.width, h: this.video.info.height } : null;
  }
}

export const store = new Store();
