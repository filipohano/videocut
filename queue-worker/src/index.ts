/**
 * FillernCut link queue: a Telegram bot collects links, the app picks them up.
 * See `app.ts` for the interface and README.md for how to set it up.
 *
 * Keep this file to the default export: Cloudflare only lets an entry module export
 * handlers, and a stray constant here stops the whole Worker from starting.
 */
import { route } from "./app";
import type { Env } from "./types";

export default {
  async fetch(req: Request, env: Env): Promise<Response> {
    try {
      return await route(req, env, Math.floor(Date.now() / 1000));
    } catch (error) {
      console.error(error);
      return new Response(JSON.stringify({ ok: false, error: "internal error" }), {
        status: 500,
        headers: { "content-type": "application/json; charset=utf-8" },
      });
    }
  },
};
