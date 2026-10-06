/** Things that differ between macOS and Windows: shortcut names, the file manager, encoders. */
export type Platform = "macos" | "windows" | "linux";

/** `⌘Z` → `Ctrl+Z`, `⇧⌘Z` → `Ctrl+Shift+Z` outside macOS. */
export function shortcut(text: string, platform: Platform): string {
  if (platform === "macos") return text;
  return text.replace(/⇧⌘(\w)/g, "Ctrl+Shift+$1").replace(/⌘(\w)/g, "Ctrl+$1");
}

export const fileManager = (platform: Platform): string => (platform === "macos" ? "Finder" : platform === "windows" ? "Explorer" : "file manager");

/** Short label for the encoder in the quality panel. */
export function encoderSummary(id: string, label: string | undefined): string {
  if (id === "none") return "No H.264 encoder found";
  if (id === "libx264") return "H.264 on the CPU (libx264)";
  return `H.264 on the GPU (${label?.replace(/ \(GPU\)$/, "") ?? id})`;
}
