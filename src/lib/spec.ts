/** Turn the editor state into the payload the Rust exporter expects. Pure, so it's unit-tested. */
import type { ExportSpec } from "../api";
import { evenRect, fullRect, sameRect } from "./crop";
import type { VideoState } from "../store";

export function buildExportSpec(v: VideoState, output: string): ExportSpec {
  const full = evenRect(fullRect({ w: v.info.width, h: v.info.height }));
  const crop = evenRect(v.crop);
  const trimmedStart = v.trimStart > 0.001;
  const trimmedEnd = v.trimEnd < v.info.duration - 0.02;
  return {
    input: v.path,
    output,
    sourceWidth: v.info.width,
    sourceHeight: v.info.height,
    sourceDuration: v.info.duration,
    hasAudio: v.info.hasAudio,
    audioCodec: v.info.audioCodec,
    crop: sameRect(crop, full) ? null : crop,
    trimStart: trimmedStart ? v.trimStart : null,
    trimEnd: trimmedEnd ? v.trimEnd : null,
    watermarks: v.watermarks.map((w) => ({
      path: w.path,
      nx: w.nx,
      ny: w.ny,
      scale: w.scale,
      opacity: w.opacity,
      content: w.content,
    })),
    sourceBitrate: v.info.bitrate,
    fps: v.info.fps,
    quality: v.quality,
  };
}
