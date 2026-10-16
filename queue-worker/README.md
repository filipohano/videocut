# FillernCut Telegram queue

Send a TikTok, Instagram or X link to a Telegram bot from your phone, and it shows up in
FillernCut's **Telegram queue** on your computer. One tap opens it for editing.

```
phone ──▶ Telegram bot ──▶ this worker (Cloudflare, free) ◀── FillernCut asks for new links
                                                               at launch and every 30 min
```

This folder is the small server in the middle. You set it up **once**; everyone who uses your
build of FillernCut then shares it, and each person only ever gets their own links.

## How people are kept apart

- Every copy of FillernCut makes a long random secret on its own computer. The worker only stores
  a hash of it, so it can't be read back out of the database.
- **Connect Telegram** in the app asks the worker for a one-time code (valid 10 minutes). Pressing
  *Start* in the bot with that code links **that Telegram chat to that copy of FillernCut**.
- A chat feeds exactly one copy of the app. A link sent from your chat goes into your queue and
  nobody else's; a chat that never connected gets "not connected" and nothing is stored.
- Group chats are ignored. `/stop` in the bot, or **Disconnect** in the app, ends the connection.

What the worker stores: the links people send, the Telegram first name (or username) and chat id
of connected chats, and hashes of the app secrets. Links are deleted when the app picks them up
(or after 30 days), and copies of the app that haven't been seen for 180 days are forgotten.
It all lives in **your own** Cloudflare account.

## Setting it up (about 10 minutes)

You need a free [Cloudflare](https://dash.cloudflare.com/sign-up) account, a Telegram account and
Node 20+.

### 1. Make the bot

In Telegram, talk to [@BotFather](https://t.me/BotFather): `/newbot`, pick a name and a username
(it must end in `bot`, e.g. `FillernCutBot`). Keep the **token** it gives you (looks like
`123456789:AAE…`) and the **username**.

### 2. Put the worker online

```bash
cd queue-worker
npm install
npx wrangler login                       # opens your browser once
npx wrangler d1 create fillerncut-queue  # makes the database and prints its database_id
```

If wrangler offers to add the database to your configuration automatically, say **no** and do it by
hand: open `wrangler.toml`, paste the `database_id`, and set `BOT_USERNAME` to the bot's username
(without the @). Then:

```bash
npm run db:init                          # creates the tables
npm run deploy                           # prints your address: https://fillerncut-queue.<you>.workers.dev
openssl rand -hex 24                     # prints a secret: keep it for the next command and step 3
npx wrangler secret put WEBHOOK_SECRET   # paste that secret when asked
```

### 3. Tell Telegram where to send messages

Use the bot token from step 1, the address from step 2 and the same secret:

```bash
curl "https://api.telegram.org/bot<BOT_TOKEN>/setWebhook" \
  -d "url=https://fillerncut-queue.<you>.workers.dev/telegram/webhook" \
  -d "secret_token=<THE SECRET>" \
  -d 'allowed_updates=["message"]'
```

It should answer `{"ok":true,"result":true,"description":"Webhook was set"}`. The bot token is only
used here: the worker itself never sees it. (Check later with `…/bot<BOT_TOKEN>/getWebhookInfo`.)

### 4. Point FillernCut at it

- **Releases built on GitHub:** in the repository go to *Settings → Secrets and variables → Actions
  → Variables* and add `FILLERNCUT_QUEUE_URL` = your address. The release workflow builds it into the
  app, so nobody has to type anything.
- **Your own build, or to try it first:** paste the address in the app under *Settings → Telegram
  queue*. (Or build with `FILLERNCUT_QUEUE_URL=https://… npm run tauri build`.)

### 5. Try it

In FillernCut: **Telegram queue → Connect Telegram → Open Telegram → Start**. Then send the bot a
link. It answers "✅ Added …", and the link appears in the app within 30 minutes, or straight away
when you press **Refresh**.

## The app's side

- The queue is read at launch and then every 30 minutes while the app is open. Pressing **Refresh**
  fetches at once and starts the 30 minutes over.
- Links stay in the app's queue (on disk) until you open them or press ×. Opening one downloads it
  exactly as if you had pasted it.

## API (for reference)

| Call | Who | What |
|---|---|---|
| `POST /telegram/webhook` | Telegram | A message. Needs the `X-Telegram-Bot-Api-Secret-Token` header. The answer is Telegram's `sendMessage` method, so no bot token is needed here. |
| `POST /v1/pair` | app | New one-time code → `{ code, bot, link, expiresAt }` |
| `GET /v1/queue` | app | `{ bot, linked: [{ label, linkedAt }], items: [{ id, url, at }] }` |
| `POST /v1/ack` `{ ids }` | app | Forget items the app has taken over |
| `POST /v1/unlink` | app | Disconnect: forget the chats and the waiting links |

The app calls send `Authorization: Bearer <secret>`. Limits: 200 waiting links per copy of the app,
10 links per message.

## Trying it locally

```bash
cd queue-worker
printf 'WEBHOOK_SECRET=local-secret\n' > .dev.vars
npx wrangler d1 execute fillerncut-queue --local --file=schema.sql
npx wrangler dev --local --port 8787 --var BOT_USERNAME:FillernBot
```

and, from the repository root, play two people against it with the real app code:

```bash
FILLERNCUT_REAL_QUEUE_URL=http://127.0.0.1:8787 cargo test -p fillerncut --test queue_real_worker
```

The worker's own tests run with the rest of the project: `npm test`.

## Switching it off

`npx wrangler delete` removes the worker, and `npx wrangler d1 delete fillerncut-queue` the database.
Telegram stops sending when the webhook is cleared:
`curl "https://api.telegram.org/bot<BOT_TOKEN>/deleteWebhook"`.
