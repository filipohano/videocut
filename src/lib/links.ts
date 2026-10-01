/** Light client-side link detection, for the badge next to the input. The Rust side does the real parsing. */
export type DetectedPlatform = "TikTok" | "Instagram" | "X / Twitter";

export function detectPlatform(input: string): DetectedPlatform | null {
  const text = input.trim();
  if (/^\d{19}$/.test(text)) return "TikTok";
  const match = text.match(/(?:https?:\/\/)?([a-z0-9.-]+\.[a-z]{2,})(?:[/?#]|$)/i);
  if (!match) return null;
  const host = match[1].toLowerCase().replace(/^(www|m|mobile)\./, "");
  if (host === "tiktok.com" || host.endsWith(".tiktok.com")) return "TikTok";
  if (host === "instagram.com") return "Instagram";
  if (["twitter.com", "x.com", "fxtwitter.com", "vxtwitter.com", "fixupx.com"].includes(host)) return "X / Twitter";
  return null;
}

export const VIDEO_EXTENSIONS = ["mp4", "mov", "m4v", "mkv", "webm", "avi", "flv", "wmv", "mpg", "mpeg", "ts", "mts", "m2ts", "3gp"];
export const IMAGE_EXTENSIONS = ["png", "jpg", "jpeg", "webp", "gif"];

export function extensionOf(path: string): string {
  const dot = path.lastIndexOf(".");
  return dot < 0 ? "" : path.slice(dot + 1).toLowerCase();
}
