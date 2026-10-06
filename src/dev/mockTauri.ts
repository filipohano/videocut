/**
 * A fake backend so the UI can be developed and screenshot-tested in a normal
 * browser (`npm run dev`, then open http://localhost:1420). Only ever loaded in
 * dev builds, and only when not running inside Tauri.
 *
 * Sample media comes from `npm run dev:assets` (needs ffmpeg).
 * Query flags:  ?update=1  pretend a newer release exists
 *               ?mode=notify|manual  start with that update mode
 */
import { emit } from "@tauri-apps/api/event";
import { mockIPC, mockWindows } from "@tauri-apps/api/mocks";
import type { Channel } from "@tauri-apps/api/core";
import type { MediaInfo, Settings, WatermarkEntry } from "../api";

const params = new URLSearchParams(location.search);
const sleep = (ms: number) => new Promise((r) => setTimeout(r, ms));

const mockPlatform = (params.get("platform") as "macos" | "windows") ?? "macos";
function mockAppInfo() {
  const encoders =
    mockPlatform === "windows"
      ? [{ id: "nvenc", label: "NVIDIA NVENC (GPU)" }, { id: "amf", label: "AMD AMF (GPU)" }, { id: "libx264", label: "CPU (libx264)" }]
      : [{ id: "videotoolbox", label: "Apple VideoToolbox (GPU)" }, { id: "libx264", label: "CPU (libx264)" }];
  const choice = settings.encoder;
  const first = encoders.find((e) => e.id === choice) ?? encoders[0];
  return { version: "0.1.0", platform: mockPlatform, encoder: first.id, encoders, ffmpegFound: true };
}

let settings: Settings = {
  updateMode: (params.get("mode") as Settings["updateMode"]) ?? "notify",
  autoUpdateYtdlp: true,
  cookiesBrowser: null,
  exportDir: null,
  askExportLocation: false,
  exportQuality: 60,
  exportImageQuality: 90,
  encoder: "auto",
};

const entry = (id: string, name: string, file: string, aspect: number, nx: number, ny: number, scale: number, opacity: number, content = { l: 0, t: 0, r: 1, b: 1 }): WatermarkEntry => ({
  id, name, fileName: file, aspect, nx, ny, scale, opacity, content, text: null, path: `/dev/${file}`,
});
let library: WatermarkEntry[] = [
  entry("a", "Brand logo", "logo-a.png", 0.35, 0.74, 0.88, 0.2, 0.9),
  // Wide PNG with lots of transparent margin around the visible part
  entry("p", "Padded logo", "logo-padded.png", 0.5, 0.3, 0.3, 0.5, 1, { l: 0.3, t: 0.3, r: 0.7, b: 0.7 }),
  entry("b", "@filippohano handle", "logo-b.png", 0.2, 0.05, 0.05, 0.3, 0.6),
];

let history = [
  { id: "h1", kind: "export", path: "/dev/sample.webm", createdAt: Math.floor(Date.now() / 1000) - 300, title: null, sourceUrl: null, platform: null, bytes: 1_200_000, duration: 9.7, exists: true, thumbPath: "/dev/logo-a.png" },
  { id: "h2", kind: "download", path: "/dev/sample.webm", createdAt: Math.floor(Date.now() / 1000) - 3600, title: "Funny cat", sourceUrl: "https://www.tiktok.com/@a/video/1234567890123456789", platform: "tiktok", bytes: 3_100_000, duration: null, exists: true, thumbPath: "/dev/logo-b.png" },
  { id: "h3", kind: "download", path: "/dev/gone.mp4", createdAt: Math.floor(Date.now() / 1000) - 90000, title: null, sourceUrl: "https://x.com/jack/status/20", platform: "twitter", bytes: 800_000, duration: null, exists: false, thumbPath: null },
];

const cancelled = new Set<string>();

async function simulate(job: string, message: string, ms: number): Promise<void> {
  cancelled.delete(job);
  const steps = 24;
  for (let i = 0; i <= steps; i++) {
    if (cancelled.has(job)) throw "Cancelled";
    await emit("job-progress", { job, fraction: i / steps, message });
    await sleep(ms / steps);
  }
}

function probe(url: string): Promise<MediaInfo> {
  if (/\.(jpe?g|png|webp)$/i.test(url)) {
    return new Promise((resolve, reject) => {
      const img = new Image();
      img.onload = () =>
        resolve({ width: img.naturalWidth, height: img.naturalHeight, duration: 0, fps: null, isImage: true, bitrate: null, videoCodec: "mjpeg", hasAudio: false, audioCodec: null });
      img.onerror = () => reject("Couldn't read that file as a photo. Run `npm run dev:assets` to create sample media.");
      img.src = url;
    });
  }
  return new Promise((resolve, reject) => {
    const v = document.createElement("video");
    v.preload = "metadata";
    v.onloadedmetadata = () =>
      resolve({ width: v.videoWidth, height: v.videoHeight, duration: v.duration, fps: 30, isImage: false, bitrate: 600_000, videoCodec: "h264", hasAudio: true, audioCodec: "aac" });
    v.onerror = () => reject("Couldn't read that file as a video. Run `npm run dev:assets` to create sample media.");
    v.src = url;
  });
}

mockWindows("main");
mockIPC(
  async (cmd, args) => {
    const a = (args ?? {}) as Record<string, any>;
    switch (cmd) {
      case "app_info":
        return mockAppInfo();
      case "get_settings":
        return settings;
      case "save_settings":
        settings = a.settings;
        return settings;
      case "library_list":
        return library;
      case "library_add": {
        const e = entry(crypto.randomUUID(), "New watermark", "logo-a.png", 0.35, 0.77, 0.85, 0.2, 1);
        library = [...library, e];
        return e;
      }
      case "library_update": {
        library = library.map((e) => (e.id === a.id ? { ...e, ...a.patch } : e));
        return library.find((e) => e.id === a.id);
      }
      case "library_remove":
        library = library.filter((e) => e.id !== a.id);
        return null;
      case "probe_media":
        return probe(a.path);
      case "make_preview":
        return a.path;
      case "default_save_path":
        return `/Users/you/Movies/FillernCut/Finished/2026-10-01_15-42-07.${a.ext ?? "mp4"}`;
      case "export_dir":
        return "/Users/you/Movies/FillernCut/Finished";
      case "estimate_export": {
        const s = a.spec;
        const c = s.crop ?? { w: s.sourceWidth, h: s.sourceHeight };
        if (s.imageFormat) return { videoBitrate: 0, bytes: Math.round(c.w * c.h * (s.imageFormat === "png" ? 1.6 : 0.04 + 0.5 * Math.pow(s.quality / 100, 2.2))) };
        const secs = (s.trimEnd ?? s.sourceDuration) - (s.trimStart ?? 0);
        const rate = (s.sourceBitrate ?? 2_000_000) * Math.pow((c.w * c.h) / (s.sourceWidth * s.sourceHeight), 0.85) * Math.pow(2, (s.quality - 50) / 25);
        return { videoBitrate: Math.round(rate), bytes: Math.round(((rate + 128_000) * secs) / 8) };
      }
      case "library_add_text": {
        const e = { ...entry(crypto.randomUUID(), String(a.style.text).split("\n")[0].slice(0, 28), "logo-b.png", 0.2, 0.5, 0.85, 0.4, 1), text: a.style };
        library = [...library, e];
        return e;
      }
      case "library_replace_text": {
        library = library.map((e) => (e.id === a.id ? { ...e, text: a.style, name: String(a.style.text).split("\n")[0].slice(0, 28) } : e));
        return library.find((e) => e.id === a.id);
      }
      case "export_video":
        await simulate("export", "Exporting…", 3000);
        return a.spec.output;
      case "download_link":
        await simulate("download", "Downloading…", 2400);
        return { path: "/dev/sample.webm", platform: "tiktok", title: null };
      case "history_list":
        return history;
      case "history_remove":
        history = history.filter((e) => e.id !== a.id);
        return null;
      case "history_clear":
        history = [];
        return null;
      case "discard_download":
        return true;
      case "cancel_job":
        cancelled.add(a.job);
        return null;
      case "ytdlp_version":
        return "2026.09.28";
      case "update_ytdlp":
        await sleep(800);
        return "2026.09.30";
      case "reveal_in_finder":
      case "open_url":
      case "plugin:process|restart":
      case "plugin:resources|close":
        return null;
      case "plugin:dialog|open":
        if (a.options?.multiple) return ["/dev/sample2.webm", "/dev/sample3.webm"];
        return (a.options?.filters?.[0]?.name === "Image" ? "/dev/logo-a.png" : "/dev/sample.webm") as string;
      case "plugin:dialog|save":
        return "/Users/you/Movies/sample-cut.mp4";
      case "plugin:dialog|ask":
        return true;
      case "plugin:dialog|message":
        // The real dialog answers with the label of the button pressed (custom labels, or "Yes").
        return a.buttons?.OkCancelCustom?.[0] ?? a.buttons?.OkCustom ?? "Yes";
      case "plugin:updater|check":
        if (params.get("slow")) await sleep(Number(params.get("slow")));
        if (params.get("fail")) throw "network unreachable";
        return params.get("update")
          ? { rid: 1, currentVersion: "0.1.0", version: "0.2.0", date: "2026-10-01T10:00:00Z", body: "Faster exports and a bigger preview.", rawJson: {} }
          : null;
      case "plugin:updater|download": {
        const ch = a.onEvent as Channel<any>;
        ch.onmessage({ event: "Started", data: { contentLength: 1000 } });
        for (let i = 0; i < 10; i++) {
          await sleep(120);
          ch.onmessage({ event: "Progress", data: { chunkLength: 100 } });
        }
        ch.onmessage({ event: "Finished" });
        return null;
      }
      case "plugin:updater|install":
        return null;
      default:
        console.warn("[mock] unhandled command", cmd, args);
        return null;
    }
  },
  { shouldMockEvents: true },
);

// Serve /dev/* assets as-is; anything else would be a real file path.
(window as any).__TAURI_INTERNALS__.convertFileSrc = (p: string) => (p.startsWith("/dev/") ? p : `/missing${p}`);
