/** What the bot does with a message. Replies are sent back in the webhook response itself. */
import { extractLinks, type Entity } from "./links";
import {
  accountForChat,
  addItem,
  consumePairCode,
  linkChat,
  MAX_ITEMS,
  unlinkChat,
} from "./store";
import type { D1Like } from "./types";

export interface Update {
  message?: {
    message_id?: number;
    text?: string;
    caption?: string;
    entities?: Entity[];
    caption_entities?: Entity[];
    chat?: { id?: number; type?: string };
    from?: { is_bot?: boolean; first_name?: string; username?: string };
  };
}

const HELP =
  "Send me TikTok, Instagram or X links and they show up in your FillernCut queue.\n\n" +
  "To connect: open FillernCut → Telegram queue → Connect, then tap the link it shows.\n" +
  "/stop disconnects this chat.";

const NOT_CONNECTED =
  "This chat isn't connected to FillernCut yet. Open FillernCut → Telegram queue → Connect, then tap the link it shows.";

function labelFor(from: { first_name?: string; username?: string } | undefined): string {
  const name = (from?.first_name ?? "").trim() || (from?.username ? `@${from.username}` : "Telegram");
  return name.slice(0, 40);
}

/** Handle one update. Returns the text to answer with, or null to stay quiet. */
export async function handleUpdate(db: D1Like, update: Update, now: number): Promise<{ chatId: number; text: string } | null> {
  const msg = update.message;
  const chatId = msg?.chat?.id;
  // Only private chats: a group is shared, so its messages can't belong to one person's queue.
  if (!msg || chatId === undefined || msg.chat?.type !== "private" || msg.from?.is_bot) return null;
  const text = await answer(db, msg, String(chatId), now);
  return text === null ? null : { chatId, text };
}

async function answer(db: D1Like, msg: NonNullable<Update["message"]>, chat: string, now: number): Promise<string | null> {
  const body = msg.text ?? msg.caption ?? "";

  if (body.startsWith("/")) {
    const [rawCommand, ...rest] = body.trim().split(/\s+/);
    const command = rawCommand.split("@")[0].toLowerCase();
    if (command === "/start") return start(db, chat, rest[0], labelFor(msg.from), now);
    if (command === "/stop") {
      return (await unlinkChat(db, chat))
        ? "Disconnected. Nothing you send here will reach FillernCut until you connect again."
        : "This chat wasn't connected.";
    }
    return HELP;
  }

  const links = extractLinks(body, msg.text !== undefined ? msg.entities : msg.caption_entities);
  const account = await accountForChat(db, chat);
  if (!account) return NOT_CONNECTED;
  if (links.length === 0) return "I can only queue TikTok, Instagram and X links. Send one and it shows up in FillernCut.";

  let added = 0;
  let duplicates = 0;
  let full = false;
  for (const url of links) {
    const result = await addItem(db, account, url, now);
    if (result === "added") added++;
    else if (result === "duplicate") duplicates++;
    else full = true;
  }
  if (full && added === 0) return `Your queue is full (${MAX_ITEMS} links). Open FillernCut and clear some, then send it again.`;
  if (added === 0) return links.length === 1 ? "That link is already in your queue." : "Those links are already in your queue.";
  const what = added === 1 ? "the link" : `${added} links`;
  const extra = [duplicates > 0 ? `${duplicates} already there` : "", full ? "queue full, some left out" : ""].filter(Boolean);
  return `✅ Added ${what} to your FillernCut queue.` + (extra.length ? ` (${extra.join(", ")})` : "");
}

async function start(db: D1Like, chat: string, code: string | undefined, label: string, now: number): Promise<string> {
  if (!code) return (await accountForChat(db, chat)) ? "You're connected. " + HELP : HELP;
  const account = await consumePairCode(db, code.toUpperCase(), now);
  if (!account) {
    return "That code has expired or was already used. Open FillernCut → Telegram queue → Connect to get a new one.";
  }
  await linkChat(db, chat, account, label, now);
  return "✅ Connected to FillernCut! Send me TikTok, Instagram or X links and they show up in the app's queue.\n\n/stop disconnects this chat.";
}
