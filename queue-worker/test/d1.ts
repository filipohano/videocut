/** A D1 look-alike on Node's built-in SQLite, so the worker's real SQL runs in the tests. */
import { readFileSync } from "node:fs";
import { DatabaseSync } from "node:sqlite";
import type { D1Like, D1PreparedStatement } from "../src/types";

export function testDb(): D1Like & { raw: DatabaseSync } {
  const raw = new DatabaseSync(":memory:");
  raw.exec(readFileSync(new URL("../schema.sql", import.meta.url), "utf8"));

  const statement = (sql: string, params: unknown[]): D1PreparedStatement => ({
    bind: (...values) => statement(sql, values),
    all: async <T>() => ({ results: raw.prepare(sql).all(...(params as never[])) as T[] }),
    run: async () => {
      const r = raw.prepare(sql).run(...(params as never[]));
      return { meta: { changes: Number(r.changes), last_row_id: Number(r.lastInsertRowid) } };
    },
  });
  return { raw, prepare: (sql) => statement(sql, []) };
}
