/**
 * The preview: video + crop overlay + watermark overlays + transport controls.
 * All coordinates in the store are in source-video pixels (crop) or fractions
 * of the cropped frame (watermarks); this module converts to/from screen px.
 */
import { MIN_CROP, evenRect, moveRect, ratioValue, resizeRect, type Handle, type Rect } from "../lib/crop";
import { clamp, formatClock } from "../lib/format";
import { moveTo, reclampAll, select, setScaleKeepingCorner } from "../watermarkOps";
import { MIN_SCALE, heightFraction } from "../lib/watermarks";
import { store, type ActiveWatermark } from "../store";
import { $, h, raf } from "./dom";

const HANDLES: Handle[] = ["nw", "n", "ne", "e", "se", "s", "sw", "w"];
const PLAY_PATH = "M8 5v14l11-7z";
const PAUSE_PATH = "M7 5h4v14H7zM13 5h4v14h-4z";

export interface Stage {
  load(url: string): Promise<void>;
  toggle(): void;
  pause(): void;
  seek(seconds: number): void;
  readonly currentTime: number;
}

export function initStage(): Stage {
  const wrap = $("#stage-wrap");
  const stage = $("#stage");
  const video = $<HTMLVideoElement>("#video");
  const cropFrame = $("#crop-frame");
  const cropDim = $("#crop-dim");
  const handleLayer = $("#handle-layer");
  const wmLayer = $("#wm-layer");
  const playBtn = $("#btn-play");
  const playIcon = $("#play-icon");
  const seek = $<HTMLInputElement>("#seek");
  const clock = $("#clock");
  const trimRange = $("#trim-range");
  const volume = $<HTMLInputElement>("#volume");
  const muteBtn = $("#btn-mute");

  /** Stage pixels per source pixel. */
  let k = 1;

  // ───────── sizing ─────────
  function fit(): void {
    const frame = store.frame;
    if (!frame) return;
    const cs = getComputedStyle(wrap);
    const ww = wrap.clientWidth - parseFloat(cs.paddingLeft) - parseFloat(cs.paddingRight);
    const wh = wrap.clientHeight - parseFloat(cs.paddingTop) - parseFloat(cs.paddingBottom);
    const aspect = frame.w / frame.h;
    let w = ww;
    let hgt = w / aspect;
    if (hgt > wh) {
      hgt = wh;
      w = hgt * aspect;
    }
    stage.style.width = `${Math.floor(w)}px`;
    stage.style.height = `${Math.floor(hgt)}px`;
    k = Math.floor(w) / frame.w;
    layout();
  }
  new ResizeObserver(raf(fit)).observe(wrap);

  // ───────── crop overlay ─────────
  const handleEls = new Map<Handle, HTMLElement>();
  for (const name of HANDLES) {
    const el = h("div", { class: "handle", "data-h": name });
    el.addEventListener("pointerdown", (e) => startCropDrag(e, name));
    handleLayer.append(el);
    handleEls.set(name, el);
  }
  cropFrame.addEventListener("pointerdown", (e) => startCropDrag(e, "move"));

  const dimParts = [0, 1, 2, 3].map(() => h("div"));
  cropDim.append(...dimParts);

  function currentRatio(): number | null {
    const v = store.video;
    return v ? ratioValue(v.aspect, store.frame!) : null;
  }

  function startCropDrag(e: PointerEvent, mode: Handle | "move"): void {
    const v = store.video;
    if (!v || e.button !== 0) return;
    e.preventDefault();
    e.stopPropagation();
    const target = e.currentTarget as HTMLElement;
    target.setPointerCapture(e.pointerId);
    const startX = e.clientX;
    const startY = e.clientY;
    const start: Rect = { ...v.crop };
    const frame = store.frame!;
    const ratio = currentRatio();
    select(null);

    const onMove = (ev: PointerEvent) => {
      const dx = (ev.clientX - startX) / k;
      const dy = (ev.clientY - startY) / k;
      v.crop =
        mode === "move"
          ? moveRect(start, dx, dy, frame)
          : resizeRect(start, mode, dx, dy, frame, ratio, MIN_CROP);
      reclampAll();
      store.emit("crop");
    };
    const onUp = () => {
      target.removeEventListener("pointermove", onMove);
      target.removeEventListener("pointerup", onUp);
      target.removeEventListener("pointercancel", onUp);
      // Snap to even numbers once the drag ends, matching what ffmpeg will use.
      v.crop = evenRect(v.crop);
      store.emit("crop");
    };
    target.addEventListener("pointermove", onMove);
    target.addEventListener("pointerup", onUp);
    target.addEventListener("pointercancel", onUp);
  }

  // ───────── watermark overlay ─────────
  const wmEls = new Map<string, { el: HTMLElement; img: HTMLImageElement }>();
  // One resize grip for the selected watermark, in the (unclipped) handle layer.
  const grip = h("div", { class: "wm-grip hidden", title: "Drag to resize" });
  handleLayer.append(grip);
  grip.addEventListener("pointerdown", (e) => startGripDrag(e));

  function renderWatermarks(): void {
    const list = store.video?.watermarks ?? [];
    const ids = new Set(list.map((w) => w.id));
    for (const [id, { el }] of wmEls)
      if (!ids.has(id)) {
        el.remove();
        wmEls.delete(id);
      }
    for (const wm of list) {
      const existing = wmEls.get(wm.id);
      if (existing) {
        if (existing.img.getAttribute("src") !== wm.url) existing.img.src = wm.url;
        continue;
      }
      const img = h("img", { src: wm.url, alt: wm.name, draggable: false });
      // Outline of the visible content (the image may have transparent margins).
      const box = h("span", { class: "wm-box" });
      const el = h("div", { class: "wm", "data-id": wm.id }, img, box);
      el.addEventListener("pointerdown", (e) => startWmDrag(e, wm.id, el));
      wmLayer.append(el);
      wmEls.set(wm.id, { el, img });
    }
    layout();
  }

  function startWmDrag(e: PointerEvent, id: string, el: HTMLElement): void {
    const v = store.video;
    const wm = v?.watermarks.find((w) => w.id === id);
    if (!v || !wm || e.button !== 0) return;
    e.preventDefault();
    select(id);
    el.setPointerCapture(e.pointerId);
    el.classList.add("dragging");
    const sx = e.clientX;
    const sy = e.clientY;
    const { nx, ny } = wm;
    const onMove = (ev: PointerEvent) => {
      moveTo(id, nx + (ev.clientX - sx) / k / v.crop.w, ny + (ev.clientY - sy) / k / v.crop.h);
    };
    const onUp = () => {
      el.classList.remove("dragging");
      el.removeEventListener("pointermove", onMove);
      el.removeEventListener("pointerup", onUp);
      el.removeEventListener("pointercancel", onUp);
    };
    el.addEventListener("pointermove", onMove);
    el.addEventListener("pointerup", onUp);
    el.addEventListener("pointercancel", onUp);
  }

  function startGripDrag(e: PointerEvent): void {
    const v = store.video;
    const id = store.selectedWatermark;
    const wm = v?.watermarks.find((w) => w.id === id);
    if (!v || !id || !wm || e.button !== 0) return;
    e.preventDefault();
    e.stopPropagation();
    grip.setPointerCapture(e.pointerId);
    const sx = e.clientX;
    const startContentW = wm.scale * (wm.content.r - wm.content.l);
    const contentFrac = Math.max(0.01, wm.content.r - wm.content.l);
    const onMove = (ev: PointerEvent) => {
      const contentW = Math.max(MIN_SCALE * contentFrac, startContentW + (ev.clientX - sx) / k / v.crop.w);
      setScaleKeepingCorner(id, contentW / contentFrac);
    };
    const onUp = () => {
      grip.removeEventListener("pointermove", onMove);
      grip.removeEventListener("pointerup", onUp);
      grip.removeEventListener("pointercancel", onUp);
    };
    grip.addEventListener("pointermove", onMove);
    grip.addEventListener("pointerup", onUp);
    grip.addEventListener("pointercancel", onUp);
  }

  // Clicking empty video deselects the watermark.
  stage.addEventListener("pointerdown", (e) => {
    if (e.target === video) select(null);
  });

  // ───────── layout ─────────
  function layoutWatermark(wm: ActiveWatermark, crop: Rect): void {
    const parts = wmEls.get(wm.id);
    if (!parts) return;
    const { el } = parts;
    el.style.left = `${(crop.x + wm.nx * crop.w) * k}px`;
    el.style.top = `${(crop.y + wm.ny * crop.h) * k}px`;
    el.style.width = `${wm.scale * crop.w * k}px`;
    el.style.opacity = String(wm.opacity);
    el.classList.toggle("selected", store.selectedWatermark === wm.id);
    const box = el.querySelector<HTMLElement>(".wm-box")!;
    box.style.left = `${wm.content.l * 100}%`;
    box.style.top = `${wm.content.t * 100}%`;
    box.style.width = `${(wm.content.r - wm.content.l) * 100}%`;
    box.style.height = `${(wm.content.b - wm.content.t) * 100}%`;
  }

  function layoutGrip(crop: Rect): void {
    const wm = store.video?.watermarks.find((w) => w.id === store.selectedWatermark);
    grip.classList.toggle("hidden", !wm);
    if (!wm) return;
    const hFrac = heightFraction(wm.scale, wm.aspect, crop);
    grip.style.left = `${(crop.x + (wm.nx + wm.content.r * wm.scale) * crop.w) * k}px`;
    grip.style.top = `${(crop.y + (wm.ny + wm.content.b * hFrac) * crop.h) * k}px`;
  }

  function layout(): void {
    const v = store.video;
    if (!v) return;
    const c = v.crop;
    const f = store.frame!;
    cropFrame.style.cssText = `left:${c.x * k}px;top:${c.y * k}px;width:${c.w * k}px;height:${c.h * k}px`;

    const rects: [number, number, number, number][] = [
      [0, 0, f.w, c.y], // top
      [0, c.y + c.h, f.w, f.h - c.y - c.h], // bottom
      [0, c.y, c.x, c.h], // left
      [c.x + c.w, c.y, f.w - c.x - c.w, c.h], // right
    ];
    rects.forEach(([x, y, w, hh], i) => {
      const s = dimParts[i].style;
      s.left = `${x * k}px`;
      s.top = `${y * k}px`;
      s.width = `${Math.max(0, w) * k}px`;
      s.height = `${Math.max(0, hh) * k}px`;
    });

    const pos: Record<Handle, [number, number]> = {
      nw: [c.x, c.y],
      n: [c.x + c.w / 2, c.y],
      ne: [c.x + c.w, c.y],
      e: [c.x + c.w, c.y + c.h / 2],
      se: [c.x + c.w, c.y + c.h],
      s: [c.x + c.w / 2, c.y + c.h],
      sw: [c.x, c.y + c.h],
      w: [c.x, c.y + c.h / 2],
    };
    for (const [name, el] of handleEls) {
      el.style.left = `${pos[name][0] * k}px`;
      el.style.top = `${pos[name][1] * k}px`;
    }
    // Watermarks are only visible inside the crop, exactly as in the export.
    wmLayer.style.clipPath = `inset(${c.y * k}px ${(f.w - c.x - c.w) * k}px ${(f.h - c.y - c.h) * k}px ${c.x * k}px)`;
    v.watermarks.forEach((wm) => layoutWatermark(wm, c));
    layoutGrip(c);
  }

  store.on(["crop", "watermarks", "selection"], raf(layout));
  store.on("watermarks", renderWatermarks);

  // ───────── transport ─────────
  let duration = 0;

  function updateClock(): void {
    const t = video.currentTime;
    store.playhead = t;
    clock.textContent = `${formatClock(t)} / ${formatClock(duration)}`;
    if (duration > 0) seek.value = String(Math.round((t / duration) * 1000));
  }

  function updateTrimRange(): void {
    const v = store.video;
    if (!v || duration <= 0) return;
    trimRange.style.setProperty("--trim-start", `${(v.trimStart / duration) * 100}%`);
    trimRange.style.setProperty("--trim-end", `${(v.trimEnd / duration) * 100}%`);
  }
  store.on("trim", updateTrimRange);

  function setPlayingIcon(playing: boolean): void {
    playIcon.setAttribute("d", playing ? PAUSE_PATH : PLAY_PATH);
  }
  video.addEventListener("play", () => setPlayingIcon(true));
  video.addEventListener("pause", () => setPlayingIcon(false));
  video.addEventListener("timeupdate", () => {
    const v = store.video;
    // Loop inside the trimmed range while playing.
    if (v && !video.paused && video.currentTime >= v.trimEnd - 0.03) video.currentTime = v.trimStart;
    updateClock();
  });
  video.addEventListener("seeked", updateClock);

  function toggle(): void {
    const v = store.video;
    if (!v) return;
    if (video.paused) {
      if (video.currentTime < v.trimStart || video.currentTime >= v.trimEnd - 0.05) video.currentTime = v.trimStart;
      void video.play();
    } else video.pause();
  }
  playBtn.addEventListener("click", toggle);
  video.addEventListener("click", toggle);

  seek.addEventListener("input", () => {
    if (duration > 0) video.currentTime = (Number(seek.value) / 1000) * duration;
  });

  // ───────── app volume (preview only — never changes the exported video) ─────────
  const VOLUME_KEY = "fillerncut.volume";
  const stored = (() => {
    try {
      return JSON.parse(localStorage.getItem(VOLUME_KEY) ?? "null") as { level: number; muted: boolean } | null;
    } catch {
      return null;
    }
  })();
  let level = clamp(stored?.level ?? 0.8, 0, 1);
  let muted = stored?.muted ?? false;
  function applyVolume(save = true): void {
    video.volume = level;
    video.muted = muted || level === 0;
    volume.value = String(Math.round(level * 100));
    muteBtn.classList.toggle("muted", video.muted);
    muteBtn.setAttribute("aria-label", video.muted ? "Unmute preview" : "Mute preview");
    if (save)
      try {
        localStorage.setItem(VOLUME_KEY, JSON.stringify({ level, muted }));
      } catch {
        /* storage unavailable — the setting just won't persist */
      }
  }
  volume.addEventListener("input", () => {
    level = Number(volume.value) / 100;
    if (level > 0) muted = false;
    applyVolume();
  });
  muteBtn.addEventListener("click", () => {
    if (level === 0) level = 0.8;
    muted = !video.muted;
    applyVolume();
  });
  applyVolume(false);

  store.on("video", () => {
    const v = store.video;
    if (!v) {
      video.pause();
      video.removeAttribute("src");
      video.load();
      for (const { el } of wmEls.values()) el.remove();
      wmEls.clear();
      return;
    }
    duration = v.info.duration;
    renderWatermarks();
    updateClock();
    updateTrimRange();
    fit();
  });

  return {
    load(url: string): Promise<void> {
      return new Promise((resolve, reject) => {
        const timer = setTimeout(() => cleanup(new Error("The preview took too long to load")), 7000);
        const onOk = () => {
          if (video.videoWidth === 0) cleanup(new Error("No video track could be played"));
          else cleanup();
        };
        const onErr = () => {
          const code = video.error?.code;
          const detail = [code ? `code ${code}` : "", video.error?.message ?? ""].filter(Boolean).join(": ");
          cleanup(new Error(`The preview can't play this file${detail ? ` (${detail})` : ""}`));
        };
        function cleanup(err?: Error) {
          clearTimeout(timer);
          video.removeEventListener("loadeddata", onOk);
          video.removeEventListener("error", onErr);
          err ? reject(err) : resolve();
        }
        video.addEventListener("loadeddata", onOk);
        video.addEventListener("error", onErr);
        video.src = url;
        video.load();
      });
    },
    toggle,
    pause: () => video.pause(),
    seek: (t: number) => {
      video.currentTime = clamp(t, 0, duration || t);
    },
    get currentTime() {
      return video.currentTime;
    },
  };
}
