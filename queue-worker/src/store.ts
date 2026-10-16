/** Everything the worker keeps in D1. Every query that touches queue data is scoped to one account. */
import type { D1Like } from "./types";

export const MAX_ITEMS = 200;
const ITEM_TTL = 30 * 24 * 3600;
export const PAIR_TTL = 10 * 60;
const MAX_OPEN_CODES = 5;
const ACCOUNT_TTL = 180 * 24 * 3600;

export interface Item {
  id: number;
  url: string;
  at: number;
}

export interface LinkedChat {
  label: string;
  linkedAt: number;
}

/** SHA-256 of the app's secret token as hex: the account id. The token itself is never stored. */
export async function accountIdFor(token: string): Promise<string> {
  const digest = await crypto.subtle.digest("SHA-256", new TextEncoder().encode(token));
  return [...new Uint8Array(digest)].map((b) => b.toString(16).padStart(2, "0")).join("");
}

export async function touchAccount(db: D1Like, id: string, now: number): Promise<void> {
  await db
    .prepare(
      "INSERT INTO accounts (id, created_at, last_seen) VALUES (?1, ?2, ?2) " +
        "ON CONFLICT(id) DO UPDATE SET last_seen = excluded.last_seen",
    )
    .bind(id, now)
    .run();
}

export async function createPairCode(db: D1Like, accountId: string, code: string, now: number): Promise<number> {
  const expiresAt = now + PAIR_TTL;
  await db
    .prepare("INSERT INTO pair_codes (code, account_id, created_at, expires_at) VALUES (?1, ?2, ?3, ?4)")
    .bind(code, accountId, now, expiresAt)
    .run();
  // Only the newest few codes stay valid, so asking again and again can't pile them up.
  await db
    .prepare(
      "DELETE FROM pair_codes WHERE account_id = ?1 AND code NOT IN " +
        "(SELECT code FROM pair_codes WHERE account_id = ?1 ORDER BY created_at DESC, rowid DESC LIMIT ?2)",
    )
    .bind(accountId, MAX_OPEN_CODES)
    .run();
  return expiresAt;
}

/** Use up a code. Returns the account it belongs to, or null if it's unknown, used or expired. */
export async function consumePairCode(db: D1Like, code: string, now: number): Promise<string | null> {
  const res = await db
    .prepare("DELETE FROM pair_codes WHERE code = ?1 AND expires_at > ?2 RETURNING account_id")
    .bind(code, now)
    .all<{ account_id: string }>();
  return res.results?.[0]?.account_id ?? null;
}

/** Make a chat feed an account (replacing whichever account it fed before). */
export async function linkChat(db: D1Like, chatId: string, accountId: string, label: string, now: number): Promise<void> {
  await db
    .prepare(
      "INSERT INTO chats (chat_id, account_id, label, linked_at) VALUES (?1, ?2, ?3, ?4) " +
        "ON CONFLICT(chat_id) DO UPDATE SET account_id = excluded.account_id, label = excluded.label, linked_at = excluded.linked_at",
    )
    .bind(chatId, accountId, label, now)
    .run();
}

export async function unlinkChat(db: D1Like, chatId: string): Promise<boolean> {
  const res = await db.prepare("DELETE FROM chats WHERE chat_id = ?1").bind(chatId).run();
  return (res.meta?.changes ?? 0) > 0;
}

export async function unlinkAccount(db: D1Like, accountId: string): Promise<void> {
  await db.prepare("DELETE FROM chats WHERE account_id = ?1").bind(accountId).run();
  await db.prepare("DELETE FROM pair_codes WHERE account_id = ?1").bind(accountId).run();
  await db.prepare("DELETE FROM items WHERE account_id = ?1").bind(accountId).run();
}

export async function accountForChat(db: D1Like, chatId: string): Promise<string | null> {
  const res = await db.prepare("SELECT account_id FROM chats WHERE chat_id = ?1").bind(chatId).all<{ account_id: string }>();
  return res.results?.[0]?.account_id ?? null;
}

export async function chatsOf(db: D1Like, accountId: string): Promise<LinkedChat[]> {
  const res = await db
    .prepare("SELECT label, linked_at FROM chats WHERE account_id = ?1 ORDER BY linked_at")
    .bind(accountId)
    .all<{ label: string; linked_at: number }>();
  return (res.results ?? []).map((r) => ({ label: r.label, linkedAt: r.linked_at }));
}

export type AddResult = "added" | "duplicate" | "full";

export async function addItem(db: D1Like, accountId: string, url: string, now: number): Promise<AddResult> {
  // One statement, so the limit holds even when messages arrive at the same time.
  const inserted = await db
    .prepare(
      "INSERT OR IGNORE INTO items (account_id, url, created_at) " +
        "SELECT ?1, ?2, ?3 WHERE (SELECT COUNT(*) FROM items WHERE account_id = ?1) < ?4",
    )
    .bind(accountId, url, now, MAX_ITEMS)
    .run();
  if ((inserted.meta?.changes ?? 0) > 0) return "added";
  const same = await db
    .prepare("SELECT 1 AS found FROM items WHERE account_id = ?1 AND url = ?2")
    .bind(accountId, url)
    .all<{ found: number }>();
  return (same.results?.length ?? 0) > 0 ? "duplicate" : "full";
}

export async function listItems(db: D1Like, accountId: string, limit = MAX_ITEMS): Promise<Item[]> {
  const res = await db
    .prepare("SELECT id, url, created_at FROM items WHERE account_id = ?1 ORDER BY id LIMIT ?2")
    .bind(accountId, limit)
    .all<{ id: number; url: string; created_at: number }>();
  return (res.results ?? []).map((r) => ({ id: r.id, url: r.url, at: r.created_at }));
}

/** Remove items the app has taken over. Only the account's own items can be removed. */
export async function ackItems(db: D1Like, accountId: string, ids: number[]): Promise<number> {
  if (ids.length === 0) return 0;
  const marks = ids.map((_, i) => `?${i + 2}`).join(", ");
  const res = await db
    .prepare(`DELETE FROM items WHERE account_id = ?1 AND id IN (${marks})`)
    .bind(accountId, ...ids)
    .run();
  return res.meta?.changes ?? 0;
}

/** Forget what has expired: old codes, links nobody picked up, installations nobody uses. */
export async function cleanup(db: D1Like, now: number): Promise<void> {
  await db.prepare("DELETE FROM pair_codes WHERE expires_at <= ?1").bind(now).run();
  await db.prepare("DELETE FROM items WHERE created_at <= ?1").bind(now - ITEM_TTL).run();
  const stale = now - ACCOUNT_TTL;
  for (const table of ["chats", "items", "pair_codes"]) {
    await db
      .prepare(`DELETE FROM ${table} WHERE account_id IN (SELECT id FROM accounts WHERE last_seen <= ?1)`)
      .bind(stale)
      .run();
  }
  await db.prepare("DELETE FROM accounts WHERE last_seen <= ?1").bind(stale).run();
}
