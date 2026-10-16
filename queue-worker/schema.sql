-- FillernCut link queue. Apply with:
--   npx wrangler d1 execute fillerncut-queue --remote --file=schema.sql
-- Safe to run again.

-- One row per app installation. The id is the SHA-256 of the installation's secret
-- token, so the token itself is never stored.
CREATE TABLE IF NOT EXISTS accounts (
  id TEXT PRIMARY KEY,
  created_at INTEGER NOT NULL,
  last_seen INTEGER NOT NULL
);

-- Short-lived, single-use codes that connect a Telegram chat to an installation.
CREATE TABLE IF NOT EXISTS pair_codes (
  code TEXT PRIMARY KEY,
  account_id TEXT NOT NULL,
  created_at INTEGER NOT NULL,
  expires_at INTEGER NOT NULL
);
CREATE INDEX IF NOT EXISTS pair_codes_by_account ON pair_codes (account_id, created_at);

-- Which installation a Telegram chat feeds. A chat feeds exactly one installation,
-- which is what keeps people's queues apart.
CREATE TABLE IF NOT EXISTS chats (
  chat_id TEXT PRIMARY KEY,
  account_id TEXT NOT NULL,
  label TEXT NOT NULL,
  linked_at INTEGER NOT NULL
);
CREATE INDEX IF NOT EXISTS chats_by_account ON chats (account_id);

-- Links waiting to be picked up by the app.
CREATE TABLE IF NOT EXISTS items (
  id INTEGER PRIMARY KEY AUTOINCREMENT,
  account_id TEXT NOT NULL,
  url TEXT NOT NULL,
  created_at INTEGER NOT NULL,
  UNIQUE (account_id, url)
);
CREATE INDEX IF NOT EXISTS items_by_account ON items (account_id, id);
