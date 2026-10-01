/** Applying one video's edit to other videos (batch export). Pure, so it's unit-tested. */
import type { ExportSpec, MediaInfo } from "../api";
import { evenRect, fullRect, sameRect } from "./crop";
import type { VideoState } from "../store";

export interface BatchOptions {
  /** Crop the same *relative* area (e.g. the middle 60%) out of every video. */
  applyCrop: boolean;
  /** Put the same watermarks / text, in the same places, on every video. */
  applyWatermarks: boolean;
}

export const DEFAULT_BATCH_OPTIONS: BatchOptions = { applyCrop: true, applyWatermarks: true };

/**
 * Export spec for `input` (described by `info`) that repeats the template's edit.
 * Trim is never copied: it belongs to one specific clip.
 */
export function buildBatchSpec(
  template: VideoState,
  info: MediaInfo,
  input: string,
  output: string,
  opts: BatchOptions,
): ExportSpec {
  if (template.info.isImage !== info.isImage) {
    throw new Error(info.isImage ? "This is a photo, but you set up a video." : "This is a video, but you set up a photo.");
  }
  const full = evenRect(fullRect({ w: info.width, h: info.height }));
  let crop = full;
  if (opts.applyCrop) {
    const tw = template.info.width;
    const th = template.info.height;
    const c = template.crop;
    crop = evenRect({
      x: Math.round((c.x / tw) * info.width),
      y: Math.round((c.y / th) * info.height),
      w: Math.round((c.w / tw) * info.width),
      h: Math.round((c.h / th) * info.height),
    });
    // Rounding must never push the area outside this video.
    crop.w = Math.max(2, Math.min(crop.w, full.w - crop.x));
    crop.h = Math.max(2, Math.min(crop.h, full.h - crop.y));
    crop = evenRect(crop);
  }
  return {
    input,
    output,
    sourceWidth: info.width,
    sourceHeight: info.height,
    sourceDuration: info.duration,
    hasAudio: info.hasAudio,
    audioCodec: info.audioCodec,
    crop: sameRect(crop, full) ? null : crop,
    trimStart: null,
    trimEnd: null,
    watermarks: opts.applyWatermarks
      ? template.watermarks.map((w) => ({
          path: w.path,
          nx: w.nx,
          ny: w.ny,
          scale: w.scale,
          opacity: w.opacity,
          content: w.content,
        }))
      : [],
    sourceBitrate: info.bitrate,
    fps: info.fps,
    imageFormat: info.isImage ? template.imageFormat : null,
    quality: template.quality,
  };
}
