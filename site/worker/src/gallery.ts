// The gallery store (S6, docs/research/site-cloudflare.md "S6 as built"):
// the formula gallery in D1 (migrations/0002_gallery.sql), kept by the
// Worker itself.
//
// - OpenDMD's examples are seeded from the deployed site's own
//   formulas/gallery.json (written by tools/gallery_page.py, every entry
//   checked by opendmd-gallery at the build) the first time a request or a
//   sync finds the store holding another build's (`__GALLERY_EXAMPLES__`,
//   that file's examples' hash, set by wrangler.config.ts).
// - The contributions are synced from the public repo's gallery/ by the
//   cron (cloudflare.config.ts) or POST /api/gallery/sync: main's commit is
//   read from git's own ref advertisement (`info/refs`, no API, no token,
//   no rate limit), and only when it moved is that commit's tarball
//   fetched (github.com/<repo>/archive/<sha>.tar.gz). Every changed file is
//   checked as opendmd-gallery checks it (src/check.ts) and drawn as its
//   thumbnail; a file that fails leaves the last version that passed in
//   place (gallery_refused says why), a file that goes is hidden.
// - /formulas/ is the built page with its two lists written from the store
//   (the same tiles as gallery_page.py's), /formulas/src/<stem>.psi and
//   /formulas/thumbs/<slug>.png come from it, /api/gallery lists it. Until
//   the store holds this build's examples and a first sync, or when D1
//   fails, the built files are served as they are.

import { checkFile, checkSlug, type Entry } from "./check";
import { thumbnail } from "./psi";
import { png } from "./png";

declare const __GALLERY_SOURCE__: string;
declare const __GALLERY_EXAMPLES__: string;

export interface GalleryEnv {
  ASSETS: { fetch(request: Request | string): Promise<Response> };
  DB: D1Database;
}

/** The public repo whose gallery/ is synced (a local fixture in the tests). */
export const SOURCE = __GALLERY_SOURCE__;
const BRANCH = "refs/heads/main";
const UA = "opendmd-site/1 (+https://opendmd.dev)";
// More .psi files than this in gallery/ and the rest are left out (gallery_page.py's MAX).
const MAX = 2000;
// Files checked in one run at most; the rest in the next (the commit is
// recorded only once every file of it was seen).
const PER_RUN = 200;
// A sync started less than this long ago keeps a second one out (seconds).
const GAP = 10;
const TARBALL_MAX = 32 << 20;

const now = () => Math.floor(Date.now() / 1000);

// ---------------------------------------------------------------- state

async function metaGet(db: D1Database, keys: string[]): Promise<Record<string, string>> {
  const { results } = await db.prepare(`select key, value from meta where key in (${keys.map(() => "?").join(",")})`).bind(...keys).all<{ key: string; value: string }>();
  return Object.fromEntries(results.map((r) => [r.key, r.value]));
}

const metaSet = (db: D1Database, key: string, value: string) =>
  db.prepare("insert into meta (key, value) values (?1, ?2) on conflict(key) do update set value = excluded.value").bind(key, value);

/** Claim `key` unless it was claimed less than `gap` seconds ago. */
async function claim(db: D1Database, key: string, gap: number): Promise<boolean> {
  const t = now();
  const r = await db
    .prepare("insert into meta (key, value) values (?1, ?2) on conflict(key) do update set value = excluded.value where cast(meta.value as integer) <= ?3")
    .bind(key, String(t), t - gap)
    .run();
  return r.meta.changes === 1;
}

// ---------------------------------------------------------------- the examples

interface Built extends Entry {
  file: string;
  added: string; // ISO time (opendmd-web/formulas/added.json)
}

/** Git's id of a blob: sha1 of "blob <length>\0" and the bytes, hex. */
async function blobSha(bytes: Uint8Array): Promise<string> {
  const head = new TextEncoder().encode(`blob ${bytes.length}\0`);
  const all = new Uint8Array(head.length + bytes.length);
  all.set(head);
  all.set(bytes, head.length);
  return [...new Uint8Array(await crypto.subtle.digest("SHA-1", all))].map((b) => b.toString(16).padStart(2, "0")).join("");
}

/** Put this build's examples in the store (from its formulas/gallery.json),
 * replacing another build's. */
export async function seed(env: GalleryEnv, origin = "https://opendmd.dev"): Promise<{ seeded: boolean; examples: number }> {
  const db = env.DB;
  const m = await metaGet(db, ["examples"]);
  if (m.examples === __GALLERY_EXAMPLES__) return { seeded: false, examples: -1 };
  if (!(await claim(db, "seeding", 30))) return { seeded: false, examples: -1 };
  const res = await env.ASSETS.fetch(new Request(new URL("/formulas/gallery.json", origin)));
  if (!res.ok) throw new Error(`formulas/gallery.json: ${res.status}`);
  const { examples } = (await res.json()) as { examples: Built[] };
  const t = now();
  const rows = await Promise.all(
    examples.map(async (e) =>
      db
        .prepare(
          "insert into gallery (source, slug, file, src, name, about, author, licence, look, controls, esp32_us, loop_ms, inputs, psb_len, abi, vm_us, sha, thumb, status, updated, added) " +
            "values (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, null, 'ok', ?, ?)",
        )
        .bind(e.bundle ? "bundle" : "example", e.slug, e.file, e.src, e.name, e.about, e.author, e.licence, e.look ? JSON.stringify(e.look) : null,
          JSON.stringify(e.controls), e.us, e.loop_ms, e.inputs, e.psb_len, e.abi, e.vm_us, await blobSha(new TextEncoder().encode(e.file)), t, Math.floor(Date.parse(e.added) / 1000)),
    ),
  );
  await db.batch([db.prepare("delete from gallery where source in ('bundle', 'example')"), ...rows, metaSet(db, "examples", __GALLERY_EXAMPLES__)]);
  return { seeded: true, examples: examples.length };
}

// ---------------------------------------------------------------- the public repo

/** main's commit, from git's ref advertisement (smart HTTP). */
async function head(): Promise<string> {
  const res = await fetch(`${SOURCE}.git/info/refs?service=git-upload-pack`, { headers: { "User-Agent": UA } });
  if (!res.ok) throw new Error(`info/refs: ${res.status}`);
  const b = new Uint8Array(await res.arrayBuffer());
  const td = new TextDecoder();
  for (let o = 0; o + 4 <= b.length;) {
    const n = parseInt(td.decode(b.subarray(o, o + 4)), 16);
    if (Number.isNaN(n)) break;
    if (n < 4) {
      o += 4;
      continue;
    }
    const line = td.decode(b.subarray(o + 4, o + n)).split("\0")[0].trimEnd();
    o += n;
    const m = /^([0-9a-f]{40}) (\S+)$/.exec(line);
    if (m && m[2] === BRANCH) return m[1];
  }
  throw new Error(`info/refs: no ${BRANCH}`);
}

interface TarEntry {
  path: string;
  type: string;
  mode: number;
  data: Uint8Array;
}

/** The entries of a tar (ustar, with pax and GNU long names). */
function* untar(b: Uint8Array): Generator<TarEntry> {
  const td = new TextDecoder();
  const str = (h: Uint8Array, s: number, e: number) => {
    const f = h.subarray(s, e);
    const z = f.indexOf(0);
    return td.decode(z < 0 ? f : f.subarray(0, z));
  };
  let long: string | null = null;
  for (let o = 0; o + 512 <= b.length;) {
    const h = b.subarray(o, o + 512);
    if (h.every((x) => x === 0)) break;
    const size = parseInt(str(h, 124, 136).trim() || "0", 8);
    const type = h[156] ? String.fromCharCode(h[156]) : "0";
    const data = b.subarray(o + 512, o + 512 + size);
    o += 512 + Math.ceil(size / 512) * 512;
    if (type === "g") continue;
    if (type === "x") {
      // pax records: "<length> <key>=<value>\n"
      for (const rec of td.decode(data).split("\n")) {
        const m = /^\d+ path=(.*)$/s.exec(rec);
        if (m) long = m[1];
      }
      continue;
    }
    if (type === "L") {
      long = str(data, 0, data.length);
      continue;
    }
    const prefix = str(h, 257, 263).startsWith("ustar") ? str(h, 345, 500) : "";
    const path = long ?? (prefix ? `${prefix}/${str(h, 0, 100)}` : str(h, 0, 100));
    long = null;
    yield { path, type, mode: parseInt(str(h, 100, 108).trim() || "0", 8), data };
  }
}

async function gunzip(buf: ArrayBuffer): Promise<Uint8Array> {
  const s = new Blob([buf]).stream().pipeThrough(new DecompressionStream("gzip"));
  return new Uint8Array(await new Response(s).arrayBuffer());
}

/** gallery/'s .psi files at `commit`: each a file to check, or why not. */
async function files(commit: string): Promise<Map<string, { bytes: Uint8Array } | { why: string }>> {
  const res = await fetch(`${SOURCE}/archive/${commit}.tar.gz`, { headers: { "User-Agent": UA } });
  if (!res.ok) throw new Error(`the tarball: ${res.status}`);
  const gz = await res.arrayBuffer();
  if (gz.byteLength > TARBALL_MAX) throw new Error(`the tarball is ${gz.byteLength} bytes`);
  const out = new Map<string, { bytes: Uint8Array } | { why: string }>();
  for (const e of untar(await gunzip(gz))) {
    // "<repo>-<sha>/gallery/<name>": straight in gallery/ only.
    const m = /^[^/]+\/gallery\/([^/]+)\/?$/.exec(e.path);
    if (!m || !m[1].endsWith(".psi") || out.size >= MAX) continue;
    const name = m[1].slice(0, -4);
    // (opendmd-gallery's `check_path`, then `read_file`, then the pull
    // request check's mode rule)
    const slug = checkSlug(name);
    if (slug) out.set(name, { why: slug });
    else if (e.type !== "0") out.set(name, { why: "not a regular file (a link or a folder)" });
    else if (e.mode & 0o111) out.set(name, { why: "the file is executable: a gallery file is a plain text file (mode 100644)" });
    else out.set(name, { bytes: e.data.slice() });
  }
  return out;
}

export interface Report {
  ok: boolean;
  source: string;
  commit?: string;
  previous?: string | null;
  changed?: boolean;
  examples?: { seeded: boolean; examples: number };
  added: string[];
  updated: string[];
  gone: string[];
  refused: { slug: string; problems: string[] }[];
  kept: number;
  more?: boolean;
  busy?: boolean;
  error?: string;
}

/** One sync of the public repo's gallery/ into the store (the cron's and
 * POST /api/gallery/sync's). `busy` when another started under GAP s ago. */
export async function sync(env: GalleryEnv, origin?: string): Promise<Report> {
  const db = env.DB;
  const r: Report = { ok: false, source: SOURCE, added: [], updated: [], gone: [], refused: [], kept: 0 };
  try {
    if (!(await claim(db, "sync_started", GAP))) return { ...r, busy: true };
    r.examples = await seed(env, origin);
    const commit = await head();
    const m = await metaGet(db, ["commit"]);
    r.commit = commit;
    r.previous = m.commit ?? null;
    r.changed = m.commit !== commit;
    if (!r.changed) {
      await db.batch([metaSet(db, "synced", String(now()))]);
      return { ...r, ok: true };
    }
    const want = await files(commit);
    const { results: had } = await db
      .prepare("select slug, sha, status from gallery where source = 'gallery'")
      .all<{ slug: string; sha: string; status: string }>();
    const { results: refusedHad } = await db.prepare("select slug, sha from gallery_refused").all<{ slug: string; sha: string }>();
    const rows = new Map(had.map((x) => [x.slug, x]));
    const refusedAt = new Map(refusedHad.map((x) => [x.slug, x.sha]));
    const t = now();
    const writes: D1PreparedStatement[] = [];
    let checked = 0;
    for (const [slug, f] of want) {
      const sha = "bytes" in f ? await blobSha(f.bytes) : `not-a-file:${f.why}`;
      const row = rows.get(slug);
      if (row && row.sha === sha && row.status === "ok") {
        r.kept++;
        if (refusedAt.has(slug)) writes.push(db.prepare("delete from gallery_refused where slug = ?").bind(slug));
        continue;
      }
      if (refusedAt.get(slug) === sha) continue; // refused before, unchanged
      if (checked >= PER_RUN) {
        r.more = true;
        continue;
      }
      checked++;
      const v = "bytes" in f ? checkFile(slug, f.bytes) : { ok: false as const, problems: [f.why] };
      if (!v.ok) {
        r.refused.push({ slug, problems: v.problems });
        writes.push(
          db.prepare("insert into gallery_refused (slug, sha, problems, seen) values (?1, ?2, ?3, ?4) on conflict(slug) do update set sha = ?2, problems = ?3, seen = ?4")
            .bind(slug, sha, JSON.stringify(v.problems), t),
        );
        continue;
      }
      const e = v.entry;
      const pic = await png(128, 32, thumbnail(v.program));
      const file = new TextDecoder().decode((f as { bytes: Uint8Array }).bytes);
      (row ? r.updated : r.added).push(slug);
      writes.push(
        db.prepare(
          // `added`: when first accepted, kept through later versions (a file that went and came back: again).
          "insert into gallery (source, slug, file, src, name, about, author, licence, look, controls, esp32_us, loop_ms, inputs, psb_len, abi, vm_us, sha, thumb, status, updated, added) " +
            "values ('gallery', ?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?16, ?17, ?18, ?13, ?14, 'ok', ?15, ?15) on conflict(source, slug) do update set " +
            "file = ?2, src = ?3, name = ?4, about = ?5, author = ?6, licence = ?7, look = ?8, controls = ?9, esp32_us = ?10, loop_ms = ?11, inputs = ?12, psb_len = ?16, abi = ?17, vm_us = ?18, " +
            "sha = ?13, thumb = ?14, updated = ?15, added = case when gallery.status = 'gone' or gallery.added is null then ?15 else gallery.added end, status = 'ok'",
        ).bind(slug, file, e.src, e.name, e.about, e.author, e.licence, e.look ? JSON.stringify(e.look) : null, JSON.stringify(e.controls), e.us,
          e.loop_ms, e.inputs, sha, pic, t, e.psb_len, e.abi, e.vm_us),
        db.prepare("delete from gallery_refused where slug = ?").bind(slug),
      );
    }
    for (const x of had) {
      if (!want.has(x.slug) && x.status === "ok") {
        r.gone.push(x.slug);
        writes.push(db.prepare("update gallery set status = 'gone', updated = ? where source = 'gallery' and slug = ?").bind(t, x.slug));
      }
    }
    for (const slug of refusedAt.keys()) {
      if (!want.has(slug)) writes.push(db.prepare("delete from gallery_refused where slug = ?").bind(slug));
    }
    if (!r.more) writes.push(metaSet(db, "commit", commit));
    writes.push(metaSet(db, "synced", String(t)));
    await db.batch(writes);
    return { ...r, ok: true };
  } catch (e) {
    // Nothing written: the store keeps what it had.
    return { ...r, ok: false, error: (e as Error).message };
  }
}

// ---------------------------------------------------------------- serving

interface Row {
  source: string;
  slug: string;
  name: string;
  about: string;
  author: string;
  licence: string;
  look: string | null;
  controls: string;
  src: string;
  sha: string;
  esp32_us: number;
  loop_ms: number;
  psb_len: number | null;
  abi: number | null;
  vm_us: number | null;
  status: string;
  updated: number;
  added: number | null;
}

/** The store, when it is ready to serve this build: its examples, and a
 * first sync done. Null otherwise (the built files are served instead). */
async function listed(env: GalleryEnv, ctx: ExecutionContext | null, origin: string): Promise<{ rows: Row[]; meta: Record<string, string> } | null> {
  const db = env.DB;
  const [rows, meta] = await db.batch([
    db.prepare("select source, slug, name, about, author, licence, look, controls, src, sha, esp32_us, loop_ms, psb_len, abi, vm_us, status, updated, added from gallery where status = 'ok'"),
    db.prepare("select key, value from meta where key in ('examples', 'synced', 'commit')"),
  ]);
  const m = Object.fromEntries((meta.results as { key: string; value: string }[]).map((r) => [r.key, r.value]));
  if (m.examples !== __GALLERY_EXAMPLES__) {
    // Another build's examples (a deploy since): seed this one's, and serve the built page meanwhile.
    ctx?.waitUntil(seed(env, origin).catch(() => undefined));
    return null;
  }
  if (!m.synced) return null;
  return { rows: rows.results as Row[], meta: m };
}

const LICENCE_LINKS: Record<string, [string, string]> = {
  "CC0-1.0": ["CC0", "https://creativecommons.org/publicdomain/zero/1.0/"],
  "CC-BY-4.0": ["CC BY 4.0", "https://creativecommons.org/licenses/by/4.0/"],
};
const ESC: Record<string, string> = { "&": "&amp;", "<": "&lt;", ">": "&gt;", '"': "&quot;", "'": "&#x27;" };
/** Python's html.escape. */
const esc = (s: string) => s.replace(/[&<>"']/g, (c) => ESC[c]);

/** application/x-www-form-urlencoded as URLSearchParams (and gallery_page.py's `urlencoded`) writes it. */
function urlencoded(pairs: [string, string][]): string {
  const enc = (v: string) =>
    [...new TextEncoder().encode(v)]
      .map((b) => (/[A-Za-z0-9*\-._]/.test(String.fromCharCode(b)) && b < 128 ? String.fromCharCode(b) : b === 32 ? "+" : `%${b.toString(16).toUpperCase().padStart(2, "0")}`))
      .join("");
  return pairs.map(([k, v]) => `${enc(k)}=${enc(v)}`).join("&");
}

interface Tile {
  slug: string;
  name: string;
  about: string;
  author: string;
  licence: string;
  bundle: boolean;
  look: unknown;
  controls: string[];
  src: string;
  added: string | null; // ISO time (`iso`)
  us: number;
  vm_us: number | null;
  abi: number | null;
}

/** gallery_page.py `iso`: UTC to the second. */
const iso = (t: number) => new Date(t * 1000).toISOString().replace(/\.\d{3}Z$/, "Z");

/** gallery_page.py `share_hash`: by name, never the text. */
function shareHash(e: Tile): string {
  return "#" + urlencoded([["gallery", e.slug]]);
}

// gallery_page.py ABI_SERVED, ABI_FIRMWARE: the compiled format's version the
// newest release the site serves plays, and the first release of each version.
const ABI_SERVED = 2;
const ABI_FIRMWARE: Record<number, string> = { 1: "1.0.0-beta.13", 2: "1.0.0-beta.14" };

/** gallery_page.py `cost`: what a Formula costs on the panel, as the web app says it. */
function cost(e: Tile): string {
  if (e.vm_us == null) return "";
  const tier = e.us <= 5000 ? "Light" : e.us <= 8000 ? "Medium" : "Heavy";
  const without = e.vm_us > e.us ? `, ${(e.vm_us / 1000).toFixed(1)} ms without the JIT` : "";
  const tip = `About ${(e.us / 1000).toFixed(1)} ms a frame on the panel${without} (light: under 5 ms; medium: under 8; the gallery takes up to 10)`;
  return `<p class="fcost" title="${esc(tip)}">${tier} · about ${(e.us / 1000).toFixed(1)} ms a frame</p>`;
}

/** gallery_page.py `needs`: the firmware a Formula needs, when its compiled format is newer than the one the latest release plays (ABI_SERVED). */
function needs(e: Tile): string {
  if (e.abi == null || e.abi <= ABI_SERVED) return "";
  const v = ABI_FIRMWARE[e.abi];
  return `<p class="fneeds">${v ? `Needs firmware ${esc(v)} or later` : "Needs newer firmware"}</p>`;
}

/** gallery_page.py `tile`. */
function tile(e: Tile, pic: string): string {
  const [lic, url] = LICENCE_LINKS[e.licence];
  const badges = ([
    ["look", "It can be the panel's look: drawn over every GIF (Settings › Look)", !!e.look],
    ["on the panel", "The web app comes with it: in Formulas, under Bundled", e.bundle],
  ] as const).filter(([, , on]) => on).map(([b, why]) => `<span class="fbadge" title="${esc(why)}">${esc(b)}</span>`).join("");
  const h = esc(shareHash(e));
  const data = (e.added ? ` data-added="${esc(e.added)}"` : "") +
    ([["look", !!e.look], ["panel", e.bundle], ["controls", e.controls.length > 0]] as const).filter(([, on]) => on).map(([k]) => ` data-${k}="1"`).join("");
  return (
    `<li class="ftile"${data}><a class="fpic" href="../f/${h}" tabindex="-1" aria-hidden="true">` +
    `<picture class="led"><img src="${esc(pic)}" alt="" width="128" height="32" loading="lazy" decoding="async"></picture></a>` +
    `<h3>${esc(e.name)}${badges}</h3><p class="fdesc">${esc(e.about)}</p>` +
    (e.controls.length ? `<p class="fctl">Controls: ${esc(e.controls.join(", "))}</p>` : "") +
    cost(e) + needs(e) +
    `<p class="fby">by ${esc(e.author)} · <a href="${url}" rel="license">${lic}</a></p>` +
    `<p class="facts"><a class="cta add" href="../f/${h}&amp;panel=1">Open on my panel</a>` +
    `<a class="cta alt" href="../f/${h}">Open</a></p></li>`
  );
}

// gallery_page.py EMPTY: the community list with nothing in it.
const EMPTY =
  '<p class="tiles-note">Nothing here yet: be the first. Make a formula in the <a href="../demo/#formulas">demo</a>, ' +
  "then Submit to the gallery from its editor.</p>\n";

/** Python's sort key (name.casefold(), slug), near enough: lower case with
 * casefold's few extra foldings, compared by code point. */
function byName(a: Tile, b: Tile): number {
  const fold = (s: string) => s.toLowerCase().replace(/ß/g, "ss").replace(/ς/g, "σ").replace(/ſ/g, "s");
  const cmp = (x: string, y: string) => {
    const p = Array.from(x, (c) => c.codePointAt(0)!), q = Array.from(y, (c) => c.codePointAt(0)!);
    for (let i = 0; i < Math.min(p.length, q.length); i++) if (p[i] !== q[i]) return p[i] - q[i];
    return p.length - q.length;
  };
  return cmp(fold(a.name), fold(b.name)) || cmp(a.slug, b.slug);
}

/** gallery_page.py `order`: newest first, then by name; the undated last. */
function order(tiles: Tile[]): Tile[] {
  const named = [...tiles].sort(byName);
  const dated = named.filter((e) => e.added).sort((a, b) => (a.added! < b.added! ? 1 : a.added! > b.added! ? -1 : 0));
  return [...dated, ...named.filter((e) => !e.added)];
}

const asTile = (r: Row): Tile => ({
  slug: r.slug, name: r.name, about: r.about, author: r.author, licence: r.licence, bundle: r.source === "bundle",
  look: r.look ? JSON.parse(r.look) : null, controls: JSON.parse(r.controls), src: r.src, added: r.added == null ? null : iso(r.added),
  us: r.esp32_us, vm_us: r.vm_us, abi: r.abi,
});

/** The two lists between their markers, from the store's rows. */
function lists(rows: Row[]): { community: string; examples: string } {
  const ex = order(rows.filter((r) => r.source !== "gallery").map(asTile));
  const known = new Set(ex.map((e) => e.src.trim()));
  const sha = new Map(rows.filter((r) => r.source === "gallery").map((r) => [r.slug, r.sha]));
  const co = order(rows.filter((r) => r.source === "gallery" && !known.has(r.src.trim())).map(asTile));
  // A contribution's thumbnail is the store's, named by its version (it can change in place).
  const community = co.map((e) => tile(e, `thumbs/${e.slug}.png?v=${sha.get(e.slug)!.slice(0, 12)}`)).join("");
  return {
    community: community ? `<ul class="ftiles">${community}</ul>\n` : EMPTY,
    examples: `<ul class="ftiles">${ex.map((e) => tile(e, `examples/${e.slug}.png`)).join("")}</ul>\n`,
  };
}

function splice(page: string, part: string, inner: string): string | null {
  const a = `<!-- gallery:${part} -->`, b = `<!-- gallery:${part}-end -->`;
  const i = page.indexOf(a), j = page.indexOf(b);
  if (i < 0 || j < i || page.indexOf(a, i + 1) >= 0) return null;
  return page.slice(0, i + a.length) + inner + page.slice(j);
}

const marked = (r: Response, how: string) => {
  const out = new Response(r.body, r);
  out.headers.set("X-Gallery", how);
  return out;
};

/** GET /formulas/: the built page with its lists from the store. */
export async function page(request: Request, env: GalleryEnv, ctx: ExecutionContext): Promise<Response> {
  const url = new URL(request.url);
  const built = () => env.ASSETS.fetch(request).then((r) => marked(r, "built"));
  let got;
  try {
    got = await listed(env, ctx, url.origin);
  } catch {
    return built(); // D1 out of reach: the page as built
  }
  if (!got) return built();
  // (Not conditional: the asset's ETag is the built page's.)
  const res = await env.ASSETS.fetch(new Request(url.origin + url.pathname));
  if (!res.ok) return marked(res, "built");
  const { community, examples } = lists(got.rows);
  const text = splice(splice(await res.text(), "community", community) ?? "", "examples", examples);
  if (!text) return built();
  const out = new Response(request.method === "HEAD" ? null : text, res);
  out.headers.set("Cache-Control", "public, max-age=60");
  out.headers.delete("ETag");
  out.headers.delete("Content-Length");
  out.headers.set("X-Gallery", "store");
  return out;
}

const TEXT_HEADERS = {
  "Content-Type": "text/plain; charset=utf-8",
  "Cache-Control": "public, max-age=60",
  "X-Content-Type-Options": "nosniff",
  "Content-Security-Policy": "default-src 'none'; frame-ancestors 'none'",
  "Access-Control-Allow-Origin": "*",
  "X-Gallery": "store",
};

/** /formulas/src/<stem>.psi (an example's file first, then a
 * contribution's) and /formulas/thumbs/<slug>.png from the store; the built
 * files when it can't say. */
export async function file(request: Request, env: GalleryEnv, ctx: ExecutionContext, kind: "src" | "thumb", slug: string): Promise<Response> {
  const built = () => env.ASSETS.fetch(request).then((r) => marked(r, "built"));
  try {
    const url = new URL(request.url);
    if (!(await listed(env, ctx, url.origin))) return built();
    if (kind === "src") {
      const f = await env.DB.prepare("select file from gallery where slug = ? and status = 'ok' order by source = 'gallery' limit 1").bind(slug).first<{ file: string }>();
      if (!f) return built();
      return new Response(request.method === "HEAD" ? null : f.file, { headers: TEXT_HEADERS });
    }
    const t = await env.DB.prepare("select thumb from gallery where source = 'gallery' and slug = ? and status = 'ok'").bind(slug).first<{ thumb: ArrayBuffer | number[] | null }>();
    if (!t?.thumb) return built();
    return new Response(request.method === "HEAD" ? null : new Uint8Array(t.thumb as ArrayBuffer), {
      headers: { ...TEXT_HEADERS, "Content-Type": "image/png", "Cache-Control": "public, max-age=600", "Access-Control-Allow-Origin": "*" },
    });
  } catch {
    return built();
  }
}

/** GET /api/gallery: the store as JSON (what the page lists, and what the
 * last sync refused). */
export async function api(env: GalleryEnv): Promise<unknown> {
  const db = env.DB;
  const [rows, meta, refused] = await db.batch([
    db.prepare("select source, slug, name, about, author, licence, look, controls, src, sha, esp32_us, loop_ms, psb_len, abi, vm_us, status, updated, added from gallery where status = 'ok' order by source, slug"),
    db.prepare("select key, value from meta where key in ('examples', 'synced', 'commit')"),
    db.prepare("select slug, sha, problems, seen from gallery_refused order by slug"),
  ]);
  const m = Object.fromEntries((meta.results as { key: string; value: string }[]).map((r) => [r.key, r.value]));
  return {
    source: SOURCE,
    commit: m.commit ?? null,
    synced: m.synced ? Number(m.synced) : null,
    examples_build: m.examples === __GALLERY_EXAMPLES__ ? "this" : m.examples ? "another" : null,
    entries: (rows.results as Row[]).map((r) => ({
      source: r.source, slug: r.slug, name: r.name, about: r.about, author: r.author, licence: r.licence,
      look: r.look ? JSON.parse(r.look) : null, controls: JSON.parse(r.controls), esp32_us: r.esp32_us, loop_ms: r.loop_ms, vm_us: r.vm_us, psb_len: r.psb_len, abi: r.abi,
      sha: r.sha, updated: r.updated, added: r.added, src: r.src,
    })),
    refused: (refused.results as { slug: string; sha: string; problems: string; seen: number }[]).map((x) => ({ ...x, problems: JSON.parse(x.problems) })),
  };
}
