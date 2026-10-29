// Shared Formulas, stored in D1 (migrations/): the Store interface the
// router uses, so a Durable Object store could replace this one
// (docs/research/site-cloudflare.md §5.3).

export interface Stored {
  id: string;
  src: string;
  name: string | null;
  about: string | null;
  controls: string | null;
  esp32_us: number;
  created: number;
}

export interface Store {
  /** The Formula with this id, unless there is none or it was taken down. */
  get(id: string): Promise<Stored | null>;
  /** The id of the Formula with this digest (hex), taken down or not. */
  byHash(hex: string): Promise<string | null>;
  /** Store `src` (checked) under its content address: [id, whether it is new]. */
  put(src: string, d: { hex: string; b32: string }, meta: Meta): Promise<[string, boolean]>;
}

export interface Meta {
  name: string | null;
  about: string | null;
  controls: string;
  esp32_us: number;
}

// Crockford's base32, lower case (no i, l, o, u): what FORMULA_ID matches.
const ALPHABET = "0123456789abcdefghjkmnpqrstvwxyz";

/** The digest as Crockford base32, 5 bits a character. */
export function base32(bytes: Uint8Array): string {
  let out = "", acc = 0, bits = 0;
  for (const b of bytes) {
    acc = (acc << 8) | b;
    bits += 8;
    while (bits >= 5) {
      out += ALPHABET[(acc >>> (bits - 5)) & 31];
      bits -= 5;
    }
    acc &= (1 << bits) - 1;
  }
  return bits ? out + ALPHABET[(acc << (5 - bits)) & 31] : out;
}

export async function digest(src: string): Promise<{ hex: string; b32: string }> {
  const d = new Uint8Array(await crypto.subtle.digest("SHA-256", new TextEncoder().encode(src)));
  return { hex: [...d].map((b) => b.toString(16).padStart(2, "0")).join(""), b32: base32(d) };
}

export class D1Store implements Store {
  constructor(private db: D1Database) {}

  async get(id: string): Promise<Stored | null> {
    return await this.db
      .prepare("select id, src, name, about, controls, esp32_us, created from formulas where id = ? and status = 'ok'")
      .bind(id)
      .first<Stored>();
  }

  async byHash(hex: string): Promise<string | null> {
    return (await this.db.prepare("select id from formulas where hash = ?").bind(hex).first<{ id: string }>())?.id ?? null;
  }

  async put(src: string, { hex, b32 }: { hex: string; b32: string }, meta: Meta): Promise<[string, boolean]> {
    const had = await this.byHash(hex);
    if (had) return [had, false];
    // 10 characters, or 12, 14, … when a shorter one is another Formula's
    // (git's abbreviation rule; 50 bits make that astronomically rare).
    for (let n = 10; n <= b32.length; n += 2) {
      const id = b32.slice(0, n);
      const r = await this.db
        .prepare(
          "insert into formulas (id, hash, src, name, about, controls, esp32_us, bytes, created) values (?, ?, ?, ?, ?, ?, ?, ?, ?) " +
            "on conflict do nothing",
        )
        .bind(id, hex, src, meta.name, meta.about, meta.controls, meta.esp32_us, new TextEncoder().encode(src).length, Math.floor(Date.now() / 1000))
        .run();
      if (r.meta.changes === 1) return [id, true];
      // Not inserted: this source already went in (a race), or the id is taken.
      const now = await this.byHash(hex);
      if (now) return [now, false];
    }
    throw new Error("no free id");
  }
}
