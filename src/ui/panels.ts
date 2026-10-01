/** Crop, trim and output-quality panels in the right-hand column. */
import { api } from "../api";
import { ASPECT_OPTIONS, clampRect, evenRect, fitRatio, fullRect, ratioValue, type Rect } from "../lib/crop";
import { clamp, formatSeconds, parseDecimal, parseSeconds } from "../lib/format";
import { store } from "../store";
import { reclampAll } from "../watermarkOps";
import { $, debounce, h } from "./dom";
import type { Stage } from "./stage";

const locale = navigator.language;

// ───────────────────────── crop ─────────────────────────
export function initCropPanel(): void {
  const inputs = {
    x: $<HTMLInputElement>("#crop-x"),
    y: $<HTMLInputElement>("#crop-y"),
    w: $<HTMLInputElement>("#crop-w"),
    h: $<HTMLInputElement>("#crop-h"),
  };
  const aspect = $<HTMLSelectElement>("#crop-aspect");
  aspect.append(...ASPECT_OPTIONS.map((o) => h("option", { value: o.key }, o.label)));

  function sync(): void {
    const v = store.video;
    if (!v) return;
    for (const key of ["x", "y", "w", "h"] as const) {
      if (document.activeElement !== inputs[key]) inputs[key].value = String(v.crop[key]);
    }
    aspect.value = v.aspect;
  }
  store.on(["crop", "video"], sync);

  function commit(key: "x" | "y" | "w" | "h"): void {
    const v = store.video;
    if (!v) return;
    const typed = Math.round(parseDecimal(inputs[key].value));
    if (!Number.isFinite(typed)) return sync();
    const frame = store.frame!;
    const ratio = ratioValue(v.aspect, frame);
    const next: Rect = { ...v.crop, [key]: typed };
    // With a locked ratio, changing one dimension drives the other.
    if (ratio !== null && key === "w") next.h = Math.round(typed / ratio);
    if (ratio !== null && key === "h") next.w = Math.round(typed * ratio);
    let out = evenRect(clampRect(next, frame));
    if (ratio !== null && (key === "w" || key === "h") && Math.abs(out.w / out.h - ratio) > 0.02) {
      out = evenRect(fitRatio(out, ratio, frame)); // clamped by the frame: shrink to keep the ratio
    }
    v.crop = out;
    reclampAll();
    store.emit("crop");
    sync();
  }
  for (const key of ["x", "y", "w", "h"] as const) {
    inputs[key].addEventListener("change", () => commit(key));
    inputs[key].addEventListener("keydown", (e) => {
      if (e.key === "Enter") inputs[key].blur();
    });
    inputs[key].addEventListener("blur", sync);
  }

  aspect.addEventListener("change", () => {
    const v = store.video;
    if (!v) return;
    v.aspect = aspect.value;
    const ratio = ratioValue(v.aspect, store.frame!);
    if (ratio !== null) v.crop = evenRect(fitRatio(v.crop, ratio, store.frame!));
    reclampAll();
    store.emit("crop");
  });

  $("#crop-reset").addEventListener("click", () => {
    const v = store.video;
    if (!v) return;
    v.aspect = "free";
    v.crop = fullRect(store.frame!);
    reclampAll();
    store.emit("crop");
  });
}

// ───────────────────────── trim ─────────────────────────
export function initTrimPanel(stage: Stage): void {
  const start = $<HTMLInputElement>("#trim-start");
  const end = $<HTMLInputElement>("#trim-end");
  const MIN_LEN = 0.1;

  function sync(): void {
    const v = store.video;
    if (!v) return;
    if (document.activeElement !== start) start.value = formatSeconds(v.trimStart, locale);
    if (document.activeElement !== end) end.value = formatSeconds(v.trimEnd, locale);
  }
  store.on(["trim", "video"], sync);

  function setStart(t: number): void {
    const v = store.video;
    if (!v) return;
    v.trimStart = clamp(t, 0, Math.max(0, v.trimEnd - MIN_LEN));
    store.emit("trim");
    sync();
  }
  function setEnd(t: number): void {
    const v = store.video;
    if (!v) return;
    v.trimEnd = clamp(t, Math.min(v.info.duration, v.trimStart + MIN_LEN), v.info.duration);
    store.emit("trim");
    sync();
  }

  start.addEventListener("change", () => {
    const t = parseSeconds(start.value);
    if (Number.isFinite(t)) {
      setStart(t);
      stage.seek(store.video!.trimStart);
    } else sync();
  });
  end.addEventListener("change", () => {
    const t = parseSeconds(end.value);
    Number.isFinite(t) ? setEnd(t) : sync();
  });
  for (const el of [start, end]) {
    el.addEventListener("keydown", (e) => e.key === "Enter" && el.blur());
    el.addEventListener("blur", sync);
  }
  $("#trim-set-start").addEventListener("click", () => setStart(stage.currentTime));
  $("#trim-set-end").addEventListener("click", () => setEnd(stage.currentTime));

  // Keyboard shortcuts: I / O set in and out points at the playhead.
  document.addEventListener("keydown", (e) => {
    if (!store.video || e.metaKey || e.ctrlKey || e.altKey) return;
    if ((e.target as HTMLElement)?.matches?.("input, select, textarea")) return;
    if (e.key === "i") setStart(stage.currentTime);
    if (e.key === "o") setEnd(stage.currentTime);
  });
}

// ───────────────────────── output ─────────────────────────
export function initOutputPanel(): void {
  const codec = $<HTMLSelectElement>("#codec");
  const quality = $<HTMLInputElement>("#quality");
  const out = $<HTMLOutputElement>("#quality-out");
  const help = $("#codec-help");
  const summary = $("#output-summary");

  function renderCodec(): void {
    codec.replaceChildren(
      h(
        "option",
        { value: "h264" },
        store.encoder === "videotoolbox"
          ? "H.264 on Apple silicon (VideoToolbox — fastest)"
          : store.encoder === "libx264"
            ? "H.264 on the CPU (libx264)"
            : "No H.264 encoder found",
      ),
    );
    codec.disabled = true;
    help.textContent =
      store.encoder === "videotoolbox"
        ? "Encoded by your Mac's media engine — several times faster than the CPU and very light on battery. 75 is a great default for social media; 90+ is near-lossless but the files get large. Always 4:2:0 (the most compatible)."
        : store.encoder === "libx264"
          ? "VideoToolbox isn't available on this machine, so the CPU encoder is used. 75 is a great default for social media."
          : "ffmpeg wasn't found, so exporting is unavailable. Reinstall FillernCut.";
  }

  const persist = debounce(() => {
    api.saveSettings({ ...store.settings, quality: Number(quality.value) }).then((s) => (store.settings = s));
  }, 500);

  quality.addEventListener("input", () => {
    const q = Number(quality.value);
    out.textContent = String(q);
    if (store.video) store.video.quality = q;
    persist();
  });

  function renderSummary(): void {
    const v = store.video;
    if (!v) return;
    quality.value = String(v.quality);
    out.textContent = String(v.quality);
    const c = evenRect(v.crop);
    const len = Math.max(0, v.trimEnd - v.trimStart);
    const wms = v.watermarks.length;
    summary.textContent =
      `Output: ${c.w} × ${c.h} · ${formatSeconds(len, locale)} s` + (wms ? ` · ${wms} watermark${wms > 1 ? "s" : ""}` : "");
  }
  store.on(["video", "crop", "trim", "watermarks"], renderSummary);
  store.on("video", renderCodec);
  renderCodec();
}

