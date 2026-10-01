/** `83.4` → `"1:23"` (used for the player clock). */
export function formatClock(seconds: number): string {
  if (!Number.isFinite(seconds) || seconds < 0) seconds = 0;
  const s = Math.floor(seconds);
  const h = Math.floor(s / 3600);
  const m = Math.floor((s % 3600) / 60);
  const sec = s % 60;
  const pad = (n: number) => String(n).padStart(2, "0");
  return h > 0 ? `${h}:${pad(m)}:${pad(sec)}` : `${m}:${pad(sec)}`;
}

/** Locale-aware seconds for the trim fields: `9.7` → `"9,7"` on a Norwegian Mac. */
export function formatSeconds(seconds: number, locale?: string): string {
  const v = Math.round(seconds * 100) / 100;
  return v.toLocaleString(locale, { maximumFractionDigits: 2, useGrouping: false });
}

/** Accepts `9,7`, `9.7`, `" 12 "`, `1:23.5`. Returns `NaN` for anything else. */
export function parseSeconds(input: string): number {
  const text = input.trim().replace(/\s+/g, "");
  if (text === "") return NaN;
  if (text.includes(":")) {
    const parts = text.split(":");
    if (parts.length > 3) return NaN;
    let total = 0;
    for (const part of parts) {
      const n = parseDecimal(part);
      if (!Number.isFinite(n) || n < 0) return NaN;
      total = total * 60 + n;
    }
    return total;
  }
  return parseDecimal(text);
}

export function parseDecimal(input: string): number {
  const text = input.trim().replace(/\s+/g, "").replace(",", ".");
  if (!/^-?\d*\.?\d+$|^-?\d+\.$/.test(text)) return NaN;
  return Number(text);
}

export function clamp(v: number, min: number, max: number): number {
  return Math.min(Math.max(v, min), max);
}

export function basename(path: string): string {
  return path.split(/[\\/]/).pop() ?? path;
}

export function stem(path: string): string {
  const name = basename(path);
  const dot = name.lastIndexOf(".");
  return dot > 0 ? name.slice(0, dot) : name;
}
