/** Finding the links worth queueing in a Telegram message. */

/** The sites FillernCut can download from (the app checks each link again before using it). */
const HOSTS = new Set([
  "tiktok.com",
  "vt.tiktok.com",
  "vm.tiktok.com",
  "instagram.com",
  "twitter.com",
  "x.com",
  "fxtwitter.com",
  "vxtwitter.com",
  "fixupx.com",
]);

const MAX_URL_LENGTH = 2048;
const MAX_LINKS_PER_MESSAGE = 10;

export interface Entity {
  type: string;
  offset: number;
  length: number;
  url?: string;
}

/** `https://…` for a supported post link, otherwise null. */
export function supportedUrl(raw: string): string | null {
  let text = raw.trim();
  if (!text || text.length > MAX_URL_LENGTH) return null;
  if (!/^https?:\/\//i.test(text)) text = `https://${text}`;
  let url: URL;
  try {
    url = new URL(text);
  } catch {
    return null;
  }
  if (url.protocol !== "https:" && url.protocol !== "http:") return null;
  const host = url.hostname.toLowerCase().replace(/^(www|m|mobile)\./, "");
  if (!HOSTS.has(host)) return null;
  // A site's front page isn't a post.
  if (url.pathname.replace(/\//g, "") === "") return null;
  return url.toString();
}

/**
 * Every supported link in a message, in order and without repeats. Telegram marks
 * links itself (`entities`; offsets count UTF-16 units, like JavaScript strings), so
 * only a message without any marking falls back to looking for `http…` in the text.
 */
export function extractLinks(text: string, entities: Entity[] | undefined): string[] {
  const candidates: string[] = [];
  for (const e of entities ?? []) {
    if (e.type === "url") candidates.push(text.slice(e.offset, e.offset + e.length));
    else if (e.type === "text_link" && e.url) candidates.push(e.url);
  }
  if (candidates.length === 0) candidates.push(...(text.match(/https?:\/\/[^\s<>"']+/g) ?? []));

  const found: string[] = [];
  for (const candidate of candidates) {
    const url = supportedUrl(candidate);
    if (url && !found.includes(url)) found.push(url);
    if (found.length >= MAX_LINKS_PER_MESSAGE) break;
  }
  return found;
}
