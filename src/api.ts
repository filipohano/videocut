/** Typed wrappers around the Rust commands (src-tauri/src). */
import { invoke } from "@tauri-apps/api/core";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";

export type Platform = "tiktok" | "instagram" | "twitter";
export type UpdateMode = "auto" | "notify" | "manual";
export type CookieBrowser = "safari" | "chrome" | "firefox" | "brave" | "edge";

export interface MediaInfo {
  width: number;
  height: number;
  duration: number;
  fps: number | null;
  /** a still photo (JPG/PNG/WebP), not a video */
  isImage: boolean;
  /** video bitrate, bits/s */
  bitrate: number | null;
  videoCodec: string | null;
  hasAudio: boolean;
  audioCodec: string | null;
}

export interface ContentBox {
  l: number;
  t: number;
  r: number;
  b: number;
}

export interface TextStyle {
  text: string;
  fontFamily: string;
  bold: boolean;
  italic: boolean;
  color: string;
  outline: boolean;
  outlineColor: string;
  shadow: boolean;
  align: "left" | "center" | "right";
}

export interface WatermarkEntry {
  id: string;
  name: string;
  fileName: string;
  /** image height / width */
  aspect: number;
  /** where the visible (non-transparent) part sits inside the image */
  content: ContentBox;
  /** set for text watermarks */
  text: TextStyle | null;
  nx: number;
  ny: number;
  scale: number;
  opacity: number;
  path: string;
}

export interface WatermarkPatch {
  name?: string;
  nx?: number;
  ny?: number;
  scale?: number;
  opacity?: number;
}

export interface Settings {
  updateMode: UpdateMode;
  autoUpdateYtdlp: boolean;
  cookiesBrowser: CookieBrowser | null;
  downloadDir: string | null;
  exportDir: string | null;
  askExportLocation: boolean;
  exportQuality: number;
  exportImageQuality: number;
}

export interface AppInfo {
  version: string;
  encoder: "videotoolbox" | "libx264" | "none";
  ffmpegFound: boolean;
}

export interface DownloadResult {
  path: string;
  platform: Platform;
  title: string | null;
}

export interface ExportSpec {
  input: string;
  output: string;
  sourceWidth: number;
  sourceHeight: number;
  sourceDuration: number;
  hasAudio: boolean;
  audioCodec: string | null;
  crop: { x: number; y: number; w: number; h: number } | null;
  trimStart: number | null;
  trimEnd: number | null;
  watermarks: { path: string; nx: number; ny: number; scale: number; opacity: number; content: ContentBox }[];
  sourceBitrate: number | null;
  fps: number | null;
  /** set for photos: export a single JPG/PNG */
  imageFormat: "jpg" | "png" | null;
  /** video: 1..100, 50 ≈ the source's own bitrate. photo (JPG): JPEG quality */
  quality: number;
}

export interface HistoryEntry {
  id: string;
  kind: "download" | "export";
  path: string;
  /** unix seconds */
  createdAt: number;
  title: string | null;
  sourceUrl: string | null;
  platform: string | null;
  bytes: number | null;
  duration: number | null;
  /** does the video file still exist? */
  exists: boolean;
  thumbPath: string | null;
}

export interface Estimate {
  videoBitrate: number;
  bytes: number;
}

export interface JobProgress {
  job: "download" | "export" | "preview" | "slideshow";
  fraction: number | null;
  message: string | null;
}

/** The error text the backend uses when the user pressed Cancel. */
export const CANCELLED = "Cancelled";

export function errorMessage(e: unknown): string {
  if (typeof e === "string") return e;
  if (e instanceof Error) return e.message;
  return String(e);
}

export const api = {
  appInfo: () => invoke<AppInfo>("app_info"),
  probeMedia: (path: string) => invoke<MediaInfo>("probe_media", { path }),
  makePreview: (path: string, hasAudio: boolean, duration: number) =>
    invoke<string>("make_preview", { path, hasAudio, duration }),
  defaultSavePath: (ext?: "mp4" | "jpg" | "png") => invoke<string>("default_save_path", { ext: ext ?? null }),
  exportDir: () => invoke<string>("export_dir"),
  estimateExport: (spec: ExportSpec) => invoke<Estimate>("estimate_export", { spec }),
  exportVideo: (spec: ExportSpec) => invoke<string>("export_video", { spec }),
  cancelJob: (job: string) => invoke<void>("cancel_job", { job }),
  revealInFinder: (path: string) => invoke<void>("reveal_in_finder", { path }),
  openUrl: (url: string) => invoke<void>("open_url", { url }),

  downloadLink: (url: string) => invoke<DownloadResult>("download_link", { url }),
  downloadDir: () => invoke<string>("download_dir"),

  libraryList: () => invoke<WatermarkEntry[]>("library_list"),
  libraryAdd: (path: string) => invoke<WatermarkEntry>("library_add", { path }),
  libraryUpdate: (id: string, patch: WatermarkPatch) =>
    invoke<WatermarkEntry>("library_update", { id, patch }),
  libraryRemove: (id: string) => invoke<void>("library_remove", { id }),
  libraryAddText: (pngBase64: string, style: TextStyle) => invoke<WatermarkEntry>("library_add_text", { pngBase64, style }),
  libraryReplaceText: (id: string, pngBase64: string, style: TextStyle) =>
    invoke<WatermarkEntry>("library_replace_text", { id, pngBase64, style }),

  historyList: () => invoke<HistoryEntry[]>("history_list"),
  historyRemove: (id: string) => invoke<void>("history_remove", { id }),
  historyClear: () => invoke<void>("history_clear"),

  getSettings: () => invoke<Settings>("get_settings"),
  saveSettings: (settings: Settings) => invoke<Settings>("save_settings", { settings }),
  ytdlpVersion: () => invoke<string>("ytdlp_version"),
  updateYtdlp: () => invoke<string>("update_ytdlp"),
};

export function onProgress(cb: (p: JobProgress) => void): Promise<UnlistenFn> {
  return listen<JobProgress>("job-progress", (e) => cb(e.payload));
}
