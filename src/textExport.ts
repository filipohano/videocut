/**
 * Getting text boxes ready for the exporter: each one is drawn at the pixel size of the frame
 * it is burned into, saved as a PNG, and handed over as one more overlay.
 */
import { api, type OverlaySpec } from "./api";
import type { TextBox } from "./lib/textBox";
import { ensureTextFont, textOverlays } from "./lib/textRender";

/** The overlays for `texts` on a frame of this size, with their pictures saved where ffmpeg can read them. */
export async function textOverlaySpecs(texts: TextBox[], frame: { w: number; h: number }): Promise<OverlaySpec[]> {
  if (texts.every((t) => t.text.trim() === "")) return [];
  await ensureTextFont(); // a fallback font would be burned in for good
  const specs: OverlaySpec[] = [];
  for (const o of textOverlays(texts, frame)) {
    const path = await api.saveTextImage(o.png);
    specs.push({ path, nx: o.nx, ny: o.ny, scale: o.scale, opacity: 1, content: o.content });
  }
  return specs;
}
