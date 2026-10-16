/** The small part of Cloudflare's D1 API this worker uses (so no extra type packages are needed). */
export interface D1Result<T = unknown> {
  results?: T[];
  meta?: { changes?: number; last_row_id?: number };
}

export interface D1PreparedStatement {
  bind(...values: unknown[]): D1PreparedStatement;
  all<T = Record<string, unknown>>(): Promise<D1Result<T>>;
  run(): Promise<D1Result>;
}

export interface D1Like {
  prepare(sql: string): D1PreparedStatement;
}

export interface Env {
  DB: D1Like;
  /** Telegram sends this with every webhook call (the `secret_token` given to setWebhook). */
  WEBHOOK_SECRET: string;
  /** The bot's username, without the @. */
  BOT_USERNAME: string;
}
