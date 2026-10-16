/**
 * The queue's HTTP interface, kept apart from `index.ts`: a Worker's entry module may only
 * export handlers (Cloudflare refuses to start otherwise), so anything the tests need to
 * reach lives here.
 *
 *   Telegram   ──webhook──▶  POST /telegram/webhook
 *   FillernCut ──bearer───▶  POST /v1/pair · GET /v1/queue · POST /v1/ack · POST /v1/unlink
 *
 * People are kept apart by Telegram chat: a chat feeds exactly the installation whose
 * pairing code it used, so a link sent from one chat never reaches anyone else's queue.
 */
import { createPairCode, accountIdFor, ackItems, chatsOf, cleanup, listItems, touchAccount, unlinkAccount } from "./store";
import { handleUpdate, type Update } from "./telegram";
import type { Env } from "./types";

const MAX_BODY_BYTES = 64 * 1024;
const TOKEN_PATTERN = /^[A-Za-z0-9_-]{32,128}$/;

function json(body: unknown, status = 200): Response {
  return new Response(JSON.stringify(body), {
    status,
    headers: { "content-type": "application/json; charset=utf-8", "cache-control": "no-store" },
  });
}

function fail(status: number, error: string): Response {
  return json({ ok: false, error }, status);
}

function botName(env: Env): string {
  return env.BOT_USERNAME.replace(/^@/, "");
}

/** Exactly 32 symbols, without 0/O and 1/I, so a code that is read out or retyped doesn't go wrong. */
export const CODE_ALPHABET = "ABCDEFGHJKLMNPQRSTUVWXYZ23456789";

export function newCode(): string {
  const bytes = crypto.getRandomValues(new Uint8Array(8));
  // 256 is a multiple of 32, so taking the low five bits is unbiased.
  return [...bytes].map((b) => CODE_ALPHABET[b & 31]).join("");
}

let lastCleanup = 0;

/** Housekeeping at most once an hour per worker instance; correctness never depends on it. */
async function maybeCleanup(env: Env, now: number): Promise<void> {
  if (now - lastCleanup < 3600) return;
  lastCleanup = now;
  await cleanup(env.DB, now);
}

/** Compare secrets without stopping at the first difference. */
function sameSecret(a: string, b: string): boolean {
  const x = new TextEncoder().encode(a);
  const y = new TextEncoder().encode(b);
  let diff = x.length ^ y.length;
  for (let i = 0; i < Math.max(x.length, y.length); i++) diff |= (x[i] ?? 0) ^ (y[i] ?? 0);
  return diff === 0;
}

async function readJson(req: Request): Promise<unknown | null> {
  const text = await req.text();
  if (text.length > MAX_BODY_BYTES) return null;
  try {
    return JSON.parse(text);
  } catch {
    return null;
  }
}

/** The account behind a request's bearer token, creating it the first time it's seen. */
async function authenticate(req: Request, env: Env, now: number): Promise<string | null> {
  const match = /^Bearer (\S+)$/.exec(req.headers.get("authorization") ?? "");
  if (!match || !TOKEN_PATTERN.test(match[1])) return null;
  const id = await accountIdFor(match[1]);
  await touchAccount(env.DB, id, now);
  return id;
}

async function telegramWebhook(req: Request, env: Env, now: number): Promise<Response> {
  const secret = req.headers.get("x-telegram-bot-api-secret-token") ?? "";
  if (!env.WEBHOOK_SECRET || !sameSecret(secret, env.WEBHOOK_SECRET)) return fail(401, "bad secret");
  const update = (await readJson(req)) as Update | null;
  if (!update || typeof update !== "object") return fail(400, "bad update");
  await maybeCleanup(env, now);
  const reply = await handleUpdate(env.DB, update, now);
  // Telegram runs a method given as the webhook's answer, so no bot token is needed here.
  if (!reply) return json({});
  return json({ method: "sendMessage", chat_id: reply.chatId, text: reply.text, disable_web_page_preview: true });
}

export async function route(req: Request, env: Env, now: number): Promise<Response> {
  const { pathname } = new URL(req.url);

  if (pathname === "/telegram/webhook") {
    return req.method === "POST" ? telegramWebhook(req, env, now) : fail(405, "POST only");
  }
  if (!pathname.startsWith("/v1/")) return fail(404, "not found");

  const account = await authenticate(req, env, now);
  if (!account) return fail(401, "missing or invalid token");

  if (pathname === "/v1/queue" && req.method === "GET") {
    await maybeCleanup(env, now);
    return json({ ok: true, bot: botName(env), linked: await chatsOf(env.DB, account), items: await listItems(env.DB, account) });
  }
  if (pathname === "/v1/pair" && req.method === "POST") {
    const code = newCode();
    const expiresAt = await createPairCode(env.DB, account, code, now);
    const bot = botName(env);
    return json({ ok: true, bot, code, link: `https://t.me/${bot}?start=${code}`, expiresAt });
  }
  if (pathname === "/v1/ack" && req.method === "POST") {
    const body = (await readJson(req)) as { ids?: unknown } | null;
    const ids = Array.isArray(body?.ids) ? body.ids : null;
    if (!ids || ids.length > 500 || !ids.every((n) => Number.isSafeInteger(n))) return fail(400, "ids must be a list of numbers");
    return json({ ok: true, removed: await ackItems(env.DB, account, ids as number[]) });
  }
  if (pathname === "/v1/unlink" && req.method === "POST") {
    await unlinkAccount(env.DB, account);
    return json({ ok: true });
  }
  return fail(404, "not found");
}
