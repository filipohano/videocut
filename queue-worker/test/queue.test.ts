import { describe, expect, it, vi } from "vitest";
import { CODE_ALPHABET, newCode, route } from "../src/app";
import * as entry from "../src/index";
import { extractLinks, supportedUrl } from "../src/links";
import { accountIdFor, addItem, cleanup, createPairCode, listItems, MAX_ITEMS, touchAccount } from "../src/store";
import type { Env } from "../src/types";
import { testDb } from "./d1";

const SECRET = "telegram-webhook-secret";
const T0 = 1_800_000_000;

function setup() {
  const db = testDb();
  const env: Env = { DB: db, WEBHOOK_SECRET: SECRET, BOT_USERNAME: "FillernBot" };
  let now = T0;

  const app = async (token: string, method: string, path: string, body?: unknown) => {
    const res = await route(
      new Request(`https://q.example${path}`, {
        method,
        headers: { authorization: `Bearer ${token}`, "content-type": "application/json" },
        body: body === undefined ? undefined : JSON.stringify(body),
      }),
      env,
      now,
    );
    return { status: res.status, body: (await res.json()) as any };
  };

  /** A Telegram update from a private chat; returns what the bot answered. */
  const telegram = async (message: object, secret: string | null = SECRET) => {
    const res = await route(
      new Request("https://q.example/telegram/webhook", {
        method: "POST",
        headers: secret === null ? {} : { "x-telegram-bot-api-secret-token": secret },
        body: JSON.stringify({ update_id: 1, message }),
      }),
      env,
      now,
    );
    return { status: res.status, body: (await res.json()) as any };
  };

  const say = (chat: number, text: string, extra: object = {}, name = "Filip") =>
    telegram({
      chat: { id: chat, type: "private" },
      from: { first_name: name, is_bot: false },
      text,
      entities: urlEntities(text),
      ...extra,
    });

  return { db, env, app, telegram, say, advance: (s: number) => (now += s), now: () => now };
}

/** Telegram's own marking of links in a message (UTF-16 offsets, as in JavaScript). */
function urlEntities(text: string) {
  return [...text.matchAll(/https?:\/\/\S+/g)].map((m) => ({ type: "url", offset: m.index!, length: m[0].length }));
}

const tokenA = "a".repeat(40);
const tokenB = "b".repeat(40);
const REEL = "https://www.instagram.com/reel/C1a2B3c4D5e/";
const TIKTOK = "https://www.tiktok.com/@someone/video/7123456789012345678";
const TWEET = "https://x.com/jack/status/20";

async function connect(s: ReturnType<typeof setup>, token: string, chat: number, name = "Filip") {
  const pair = await s.app(token, "POST", "/v1/pair");
  const reply = await s.say(chat, `/start ${pair.body.code}`, {}, name);
  expect(reply.body.text).toContain("Connected");
  return pair.body.code as string;
}

describe("connecting a chat to an installation", () => {
  it("gives a code and a link that opens the bot", async () => {
    const s = setup();
    const pair = await s.app(tokenA, "POST", "/v1/pair");
    expect(pair.status).toBe(200);
    expect(pair.body.code).toMatch(/^[A-HJ-NP-Z2-9]{8}$/);
    expect(pair.body.bot).toBe("FillernBot");
    expect(pair.body.link).toBe(`https://t.me/FillernBot?start=${pair.body.code}`);
    expect(pair.body.expiresAt).toBe(T0 + 600);
  });

  it("always makes full-length codes from the whole alphabet", () => {
    // The low five bits of a byte pick a symbol, so the alphabet must have exactly 32.
    expect(CODE_ALPHABET).toHaveLength(32);
    expect(new Set(CODE_ALPHABET).size).toBe(32);
    expect(CODE_ALPHABET).not.toMatch(/[01OI]/);
    const used = new Set<string>();
    for (let i = 0; i < 3000; i++) {
      const code = newCode();
      expect(code).toMatch(/^[A-HJ-NP-Z2-9]{8}$/);
      for (const c of code) used.add(c);
    }
    expect(used.size).toBe(32);
  });

  it("links the chat that uses the code, and says who in the queue", async () => {
    const s = setup();
    const before = await s.app(tokenA, "GET", "/v1/queue");
    expect(before.body.linked).toEqual([]);
    await connect(s, tokenA, 111, "Filip");
    const after = await s.app(tokenA, "GET", "/v1/queue");
    expect(after.body.linked).toEqual([{ label: "Filip", linkedAt: T0 }]);
    expect(after.body.bot).toBe("FillernBot");
  });

  it("works once only, and not after ten minutes", async () => {
    const s = setup();
    const once = await s.app(tokenA, "POST", "/v1/pair");
    expect((await s.say(111, `/start ${once.body.code}`)).body.text).toContain("Connected");
    expect((await s.say(222, `/start ${once.body.code}`)).body.text).toContain("expired or was already used");

    const late = await s.app(tokenA, "POST", "/v1/pair");
    s.advance(601);
    expect((await s.say(333, `/start ${late.body.code}`)).body.text).toContain("expired or was already used");
    const queue = await s.app(tokenA, "GET", "/v1/queue");
    expect(queue.body.linked).toHaveLength(1);
  });

  it("accepts the code typed in lower case", async () => {
    const s = setup();
    const pair = await s.app(tokenA, "POST", "/v1/pair");
    expect((await s.say(111, `/start ${pair.body.code.toLowerCase()}`)).body.text).toContain("Connected");
  });

  it("only keeps the newest few codes", async () => {
    const s = setup();
    const codes: string[] = [];
    for (let i = 0; i < 7; i++) {
      codes.push((await s.app(tokenA, "POST", "/v1/pair")).body.code);
      s.advance(1);
    }
    expect((await s.say(111, `/start ${codes[0]}`)).body.text).toContain("expired");
    expect((await s.say(111, `/start ${codes[6]}`)).body.text).toContain("Connected");
  });

  it("moves a chat to a new installation when it uses another code", async () => {
    const s = setup();
    await connect(s, tokenA, 111);
    await connect(s, tokenB, 111);
    await s.say(111, REEL);
    expect((await s.app(tokenA, "GET", "/v1/queue")).body.items).toEqual([]);
    expect((await s.app(tokenB, "GET", "/v1/queue")).body.items).toHaveLength(1);
  });

  it("explains what to do for a bare /start and for unknown commands", async () => {
    const s = setup();
    expect((await s.say(111, "/start")).body.text).toContain("Connect");
    expect((await s.say(111, "/whatever")).body.text).toContain("FillernCut");
  });
});

describe("keeping people apart", () => {
  it("puts each chat's links in its own installation's queue only", async () => {
    const s = setup();
    await connect(s, tokenA, 111, "Filip");
    await connect(s, tokenB, 222, "Mari");

    await s.say(111, `look ${REEL}`);
    await s.say(222, TIKTOK);
    await s.say(111, TWEET);

    const a = (await s.app(tokenA, "GET", "/v1/queue")).body;
    const b = (await s.app(tokenB, "GET", "/v1/queue")).body;
    expect(a.items.map((i: any) => i.url)).toEqual([REEL, TWEET]);
    expect(b.items.map((i: any) => i.url)).toEqual([TIKTOK]);
    expect(a.linked.map((c: any) => c.label)).toEqual(["Filip"]);
    expect(b.linked.map((c: any) => c.label)).toEqual(["Mari"]);
  });

  it("does not queue anything for a chat that never connected", async () => {
    const s = setup();
    await connect(s, tokenA, 111);
    const reply = await s.say(999, REEL);
    expect(reply.body.text).toContain("isn't connected");
    expect((await s.app(tokenA, "GET", "/v1/queue")).body.items).toEqual([]);
  });

  it("lets an installation remove only its own items", async () => {
    const s = setup();
    await connect(s, tokenA, 111);
    await connect(s, tokenB, 222);
    await s.say(222, REEL);
    const theirs = (await s.app(tokenB, "GET", "/v1/queue")).body.items[0].id;

    const attempt = await s.app(tokenA, "POST", "/v1/ack", { ids: [theirs] });
    expect(attempt.body.removed).toBe(0);
    expect((await s.app(tokenB, "GET", "/v1/queue")).body.items).toHaveLength(1);
    expect((await s.app(tokenB, "POST", "/v1/ack", { ids: [theirs] })).body.removed).toBe(1);
  });

  it("ignores group chats and bots", async () => {
    const s = setup();
    await connect(s, tokenA, 111);
    const group = await s.telegram({ chat: { id: 111, type: "group" }, from: { first_name: "X" }, text: REEL, entities: urlEntities(REEL) });
    expect(group.body).toEqual({});
    const bot = await s.telegram({ chat: { id: 111, type: "private" }, from: { is_bot: true }, text: REEL, entities: urlEntities(REEL) });
    expect(bot.body).toEqual({});
    expect((await s.app(tokenA, "GET", "/v1/queue")).body.items).toEqual([]);
  });

  it("stores only a hash of the app's token", async () => {
    const s = setup();
    await s.app(tokenA, "GET", "/v1/queue");
    const rows = s.db.raw.prepare("SELECT id FROM accounts").all() as { id: string }[];
    expect(rows).toEqual([{ id: await accountIdFor(tokenA) }]);
    expect(JSON.stringify(rows)).not.toContain(tokenA);
  });
});

describe("collecting links", () => {
  it("queues a link, answers, and hands it to the app until acknowledged", async () => {
    const s = setup();
    await connect(s, tokenA, 111);
    const reply = await s.say(111, REEL);
    expect(reply.status).toBe(200);
    expect(reply.body).toMatchObject({ method: "sendMessage", chat_id: 111 });
    expect(reply.body.text).toContain("Added the link");

    const first = (await s.app(tokenA, "GET", "/v1/queue")).body;
    expect(first.items).toEqual([{ id: expect.any(Number), url: REEL, at: T0 }]);
    // Not acknowledged yet: it comes again.
    expect((await s.app(tokenA, "GET", "/v1/queue")).body.items).toHaveLength(1);
    expect((await s.app(tokenA, "POST", "/v1/ack", { ids: [first.items[0].id] })).body).toEqual({ ok: true, removed: 1 });
    expect((await s.app(tokenA, "GET", "/v1/queue")).body.items).toEqual([]);
  });

  it("keeps the order links were sent in", async () => {
    const s = setup();
    await connect(s, tokenA, 111);
    await s.say(111, TWEET);
    await s.say(111, REEL);
    await s.say(111, TIKTOK);
    const urls = (await s.app(tokenA, "GET", "/v1/queue")).body.items.map((i: any) => i.url);
    expect(urls).toEqual([TWEET, REEL, TIKTOK]);
  });

  it("does not queue the same link twice", async () => {
    const s = setup();
    await connect(s, tokenA, 111);
    await s.say(111, REEL);
    const again = await s.say(111, REEL);
    expect(again.body.text).toContain("already in your queue");
    expect((await s.app(tokenA, "GET", "/v1/queue")).body.items).toHaveLength(1);
  });

  it("takes several links from one message", async () => {
    const s = setup();
    await connect(s, tokenA, 111);
    const reply = await s.say(111, `${REEL} and ${TIKTOK}\n${TWEET} ${REEL}`);
    expect(reply.body.text).toContain("3 links");
    expect((await s.app(tokenA, "GET", "/v1/queue")).body.items).toHaveLength(3);
  });

  it("reads links from a caption and from text links", async () => {
    const s = setup();
    await connect(s, tokenA, 111);
    await s.telegram({
      chat: { id: 111, type: "private" },
      from: { first_name: "F" },
      caption: `see ${TIKTOK}`,
      caption_entities: [{ type: "url", offset: 4, length: TIKTOK.length }],
    });
    await s.telegram({
      chat: { id: 111, type: "private" },
      from: { first_name: "F" },
      text: "this reel",
      entities: [{ type: "text_link", offset: 0, length: 9, url: REEL }],
    });
    const urls = (await s.app(tokenA, "GET", "/v1/queue")).body.items.map((i: any) => i.url);
    expect(urls).toEqual([TIKTOK, REEL]);
  });

  it("says so when there is nothing it can use", async () => {
    const s = setup();
    await connect(s, tokenA, 111);
    expect((await s.say(111, "hello")).body.text).toContain("only queue TikTok, Instagram and X");
    expect((await s.say(111, "https://youtube.com/watch?v=1")).body.text).toContain("only queue");
    expect((await s.app(tokenA, "GET", "/v1/queue")).body.items).toEqual([]);
  });

  it("stops at a full queue", async () => {
    const s = setup();
    await connect(s, tokenA, 111);
    const id = await accountIdFor(tokenA);
    for (let i = 0; i < MAX_ITEMS; i++) await addItem(s.db, id, `https://x.com/u/status/${i + 1}`, T0);
    const reply = await s.say(111, REEL);
    expect(reply.body.text).toContain("queue is full");
    expect(await listItems(s.db, id, 1000)).toHaveLength(MAX_ITEMS);
  });

  it("/stop disconnects the chat", async () => {
    const s = setup();
    await connect(s, tokenA, 111);
    expect((await s.say(111, "/stop")).body.text).toContain("Disconnected");
    expect((await s.say(111, REEL)).body.text).toContain("isn't connected");
    expect((await s.say(111, "/stop")).body.text).toContain("wasn't connected");
    expect((await s.app(tokenA, "GET", "/v1/queue")).body.linked).toEqual([]);
  });

  it("disconnecting from the app clears the chats and the queue", async () => {
    const s = setup();
    await connect(s, tokenA, 111);
    await s.say(111, REEL);
    expect((await s.app(tokenA, "POST", "/v1/unlink")).body).toEqual({ ok: true });
    const q = (await s.app(tokenA, "GET", "/v1/queue")).body;
    expect(q.linked).toEqual([]);
    expect(q.items).toEqual([]);
    expect((await s.say(111, REEL)).body.text).toContain("isn't connected");
  });
});

describe("link extraction", () => {
  it("only accepts posts from the supported sites", () => {
    for (const ok of [
      "https://www.tiktok.com/@a/video/1",
      "https://vt.tiktok.com/ZSabc/",
      "https://vm.tiktok.com/ZSabc/",
      "https://instagram.com/p/abc/",
      "https://m.twitter.com/a/status/1",
      "http://x.com/a/status/1",
      "instagram.com/p/abc",
    ])
      expect(supportedUrl(ok), ok).not.toBeNull();
    for (const bad of ["https://youtube.com/watch?v=1", "https://instagram.com/", "https://evil.example/https://x.com/a/status/1", "javascript:alert(1)", "ftp://x.com/a", "", "not a link", "https://notx.com/a/status/1"])
      expect(supportedUrl(bad), bad).toBeNull();
  });

  it("counts Telegram's offsets in UTF-16 units", () => {
    const text = `🎬🎬 ${REEL} 🎬`;
    const offset = text.indexOf("https");
    expect(extractLinks(text, [{ type: "url", offset, length: REEL.length }])).toEqual([REEL]);
  });

  it("finds links in plain text when Telegram marked none", () => {
    expect(extractLinks(`check ${TWEET}`, undefined)).toEqual([TWEET]);
  });

  it("caps the number of links per message", () => {
    const text = Array.from({ length: 30 }, (_, i) => `https://x.com/u/status/${i + 1}`).join(" ");
    expect(extractLinks(text, undefined)).toHaveLength(10);
  });
});

describe("security of the endpoints", () => {
  it("refuses the webhook without Telegram's secret", async () => {
    const s = setup();
    const msg = { chat: { id: 1, type: "private" }, from: { first_name: "F" }, text: "/start" };
    expect((await s.telegram(msg, null)).status).toBe(401);
    expect((await s.telegram(msg, "wrong")).status).toBe(401);
    expect((await s.telegram(msg, SECRET)).status).toBe(200);
  });

  it("refuses the webhook when no secret is configured", async () => {
    const s = setup();
    s.env.WEBHOOK_SECRET = "";
    const res = await s.telegram({ chat: { id: 1, type: "private" }, text: "/start" }, "");
    expect(res.status).toBe(401);
  });

  it("needs a proper token for the app's endpoints", async () => {
    const s = setup();
    for (const header of [undefined, "Bearer ", "Bearer short", "Basic abc", `Bearer ${"a".repeat(200)}`, `Bearer ${"a".repeat(39)}!`]) {
      const res = await route(new Request("https://q.example/v1/queue", { headers: header ? { authorization: header } : {} }), s.env, T0);
      expect(res.status, String(header)).toBe(401);
    }
  });

  it("answers unknown routes and methods with errors, and rejects bad acknowledgements", async () => {
    const s = setup();
    expect((await s.app(tokenA, "GET", "/v1/nothing")).status).toBe(404);
    expect((await s.app(tokenA, "GET", "/v1/pair")).status).toBe(404);
    const res = await route(new Request("https://q.example/elsewhere"), s.env, T0);
    expect(res.status).toBe(404);
    const webhookGet = await route(new Request("https://q.example/telegram/webhook"), s.env, T0);
    expect(webhookGet.status).toBe(405);
    for (const ids of [null, "1", [1.5], ["1"], Array.from({ length: 501 }, (_, i) => i)])
      expect((await s.app(tokenA, "POST", "/v1/ack", { ids })).status, JSON.stringify(ids).slice(0, 20)).toBe(400);
  });
});

describe("the Worker's entry module", () => {
  it("exports only the handler: Cloudflare refuses to start a Worker with any other export", () => {
    expect(Object.keys(entry)).toEqual(["default"]);
  });

  it("answers requests, and turns a failing database into a plain 500", async () => {
    const s = setup();
    const ok = await entry.default.fetch(new Request("https://q.example/v1/queue", { headers: { authorization: `Bearer ${tokenA}` } }), s.env);
    expect(ok.status).toBe(200);

    const broken = { ...s.env, DB: { prepare: () => { throw new Error("db down"); } } };
    const error = vi.spyOn(console, "error").mockImplementation(() => {});
    const res = await entry.default.fetch(new Request("https://q.example/v1/queue", { headers: { authorization: `Bearer ${tokenA}` } }), broken);
    expect(res.status).toBe(500);
    expect(await res.json()).toEqual({ ok: false, error: "internal error" });
    error.mockRestore();
  });
});

describe("housekeeping", () => {
  it("forgets old links, expired codes and installations nobody uses", async () => {
    const s = setup();
    const idA = await accountIdFor(tokenA);
    const idB = await accountIdFor(tokenB);
    await touchAccount(s.db, idA, T0);
    await touchAccount(s.db, idB, T0 + 170 * 86400);
    await addItem(s.db, idA, REEL, T0);
    await addItem(s.db, idB, TWEET, T0);
    await createPairCode(s.db, idB, "AAAAAAAA", T0);

    await cleanup(s.db, T0 + 200 * 86400);
    const accounts = s.db.raw.prepare("SELECT id FROM accounts").all() as { id: string }[];
    expect(accounts.map((a) => a.id)).toEqual([idB]);
    expect(await listItems(s.db, idA)).toEqual([]);
    // B's link is older than 30 days, B's code expired.
    expect(await listItems(s.db, idB)).toEqual([]);
    expect(s.db.raw.prepare("SELECT code FROM pair_codes").all()).toEqual([]);
  });

  it("keeps what is still fresh", async () => {
    const s = setup();
    const id = await accountIdFor(tokenA);
    await touchAccount(s.db, id, T0);
    await addItem(s.db, id, REEL, T0);
    await cleanup(s.db, T0 + 86400);
    expect(await listItems(s.db, id)).toHaveLength(1);
  });
});
