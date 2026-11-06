// Anonymous usage counts (migrations/0006_counts.sql) and their public
// page, /stats. Panels never ask the site anything themselves; two things
// people do are counted, a total a day:
//
//   install  GET /install/firmware/opendmd-<version>-<variant>-app.bin (the
//            one part of an install named by its release: ESP Web Tools
//            fetches it once per install, beside the bootloader, the table
//            and the rest, which are not counted), and the merged
//            opendmd-<version>-<variant>-full.bin (an install by hand)
//   check    GET /install/release.json (Settings › Check for updates in a
//            panel's web app), or /install/beta.json (the same, with Get
//            beta updates on), ?v=<the panel's firmware version>&layout=<4mb|8mb>
//            (update.js; a web app from before sends neither: unknown). Both
//            feeds count as one kind: a check is a check
//   (an install of the beta, from install/firmware/beta/, is an install too)
//
// Counted only on opendmd.dev (relay.home), for a GET whose answer is the
// file (200, or 304: the browser had it and asked again) and not a later
// piece of it (a Range not from byte 0). The file is answered exactly as
// the asset layer answers it; the count is written after, in waitUntil,
// and a failed write is dropped: a download never waits for D1 or fails
// with it. A retried install counts again (nothing tells two installs apart:
// nothing about who asks is kept).
//
// What is stored is the day, the kind, the version (a version-like string,
// else 'other'), the variant (4mb, 8mb, else 'other') and a count. No
// address, user agent, cookie, referrer or device id; nothing per request.
// A day takes at most MAX_VERSIONS versions of a kind; later ones count as
// 'other', so the table stays bounded whatever is sent.

import { home } from "./relay";

export interface CountsEnv {
  ASSETS: { fetch(request: Request | string): Promise<Response> };
  DB: D1Database;
}

export const RELEASE_JSON = "/install/release.json";
export const BETA_JSON = "/install/beta.json";
const FEEDS = new Set([RELEASE_JSON, BETA_JSON]);
const IMAGE = /^\/install\/firmware\/(?:beta\/)?opendmd-(.+)-(4mb|8mb)-(?:app|full)\.bin$/;
const VERSION = /^\d{1,3}\.\d{1,3}\.\d{1,3}(?:-[0-9a-z]{1,16}(?:\.[0-9a-z]{1,16}){0,3})?$/;
const VARIANTS = new Set(["4mb", "8mb"]);
const MAX_VERSIONS = 40;

/** A version as stored: version-like (a leading v dropped, lower case), or
 * 'other'; 'unknown' when none was sent. */
export function version(v: string | null): string {
  if (v === null || v === "") return "unknown";
  const s = v.trim().replace(/^v/i, "").toLowerCase();
  return s.length <= 40 && VERSION.test(s) ? s : "other";
}

/** A flash layout as stored. */
export function variant(v: string | null): string {
  if (v === null || v === "") return "unknown";
  const s = v.trim().toLowerCase();
  return VARIANTS.has(s) ? s : "other";
}

/** What a request counts as, if anything (before its answer is known). */
function what(request: Request, url: URL): { kind: string; version: string; variant: string } | null {
  if (request.method !== "GET") return null;
  if (FEEDS.has(url.pathname)) {
    return { kind: "check", version: version(url.searchParams.get("v")), variant: variant(url.searchParams.get("layout")) };
  }
  const m = IMAGE.exec(url.pathname);
  return m ? { kind: "install", version: version(m[1]), variant: variant(m[2]) } : null;
}

/** The whole file was asked for (no Range, or one from byte 0) and given. */
function whole(request: Request, res: Response): boolean {
  const range = request.headers.get("Range");
  if (range && !/^bytes=0-/i.test(range.trim())) return false;
  return res.status === 200 || res.status === 206 || res.status === 304;
}

const day = (t: number) => new Date(t).toISOString().slice(0, 10);

const UPSERT =
  "insert into counts (day, kind, version, variant, n) " +
  "select ?1, ?2, case when exists (select 1 from counts where day = ?1 and kind = ?2 and version = ?3) " +
  `or (select count(distinct version) from counts where day = ?1 and kind = ?2) < ${MAX_VERSIONS} then ?3 else 'other' end, ?4, 1 ` +
  "where true on conflict (day, kind, version, variant) do update set n = n + 1";

/** Add one to a day's count (serve calls it in waitUntil). */
async function bump(db: D1Database, kind: string, ver: string, vari: string, now = Date.now()): Promise<void> {
  await db.prepare(UPSERT).bind(day(now), kind, ver, vari).run();
}

/** Is this one of the counted paths (cloudflare.config.ts runWorkerFirst)? */
export function counted(pathname: string): boolean {
  return FEEDS.has(pathname) || IMAGE.test(pathname);
}

/** The asset, as the asset layer answers it; then the count, if it is one. */
export async function serve(request: Request, url: URL, env: CountsEnv, ctx: ExecutionContext): Promise<Response> {
  const res = await env.ASSETS.fetch(request);
  try {
    const w = what(request, url);
    if (w && whole(request, res) && home(request, url)) {
      ctx.waitUntil(bump(env.DB, w.kind, w.version, w.variant).catch((e) => console.log(JSON.stringify({ countFailed: String(e?.message ?? e) }))));
    }
  } catch {
    // never the download's problem
  }
  return res;
}

// ------------------------------------------------------------ /stats

interface Row { kind?: string; day?: string; version?: string; variant?: string; n: number; recent?: number; first?: string }

const esc = (s: string) => s.replace(/[&<>"']/g, (c) => `&#${c.charCodeAt(0)};`);
const num = (n: number) => String(Math.round(n)).replace(/\B(?=(\d{3})+(?!\d))/g, ",");
const MONTHS = ["Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec"];
const DAY_MS = 86400000;
const WEEKS = 26;
const LAYOUTS: [string, string][] = [["4mb", "DevKit V1 (4 MB)"], ["8mb", "8 MB build"], ["unknown", "Not sent"], ["other", "Other"]];

/** Semver-ish order, newest first; unknown and other last. */
function newestFirst(a: string, b: string): number {
  const key = (v: string) => {
    if (!VERSION.test(v)) return [-1];
    const [core, pre] = v.split("-", 2) as [string, string | undefined];
    const k: (number | string)[] = core.split(".").map(Number);
    k.push(pre === undefined ? 1 : 0);
    for (const p of (pre ?? "").split(".").filter(Boolean)) k.push(/^\d+$/.test(p) ? Number(p) : p);
    return k;
  };
  const x = key(a), y = key(b);
  for (let i = 0; i < Math.max(x.length, y.length); i++) {
    const p = x[i] ?? -2, q = y[i] ?? -2;
    if (p === q) continue;
    if (typeof p === typeof q) return p < q ? 1 : -1;
    return typeof p === "number" ? 1 : -1;
  }
  return 0;
}

const mondayOf = (t: number) => {
  const d = new Date(t);
  d.setUTCHours(0, 0, 0, 0);
  return d.getTime() - ((d.getUTCDay() + 6) % 7) * DAY_MS;
};
const shortDate = (t: number) => { const d = new Date(t); return `${d.getUTCDate()} ${MONTHS[d.getUTCMonth()]}`; };

function tile(value: number, label: string, sub: string): string {
  return `<div class="us-tile"><p class="us-num">${num(value)}</p><p class="us-label">${esc(label)}</p><p class="us-sub">${esc(sub)}</p></div>`;
}

/** Installs a week, the last WEEKS weeks (this one so far last): CSS bars. */
function weekly(days: Row[], now: number): string {
  const end = mondayOf(now), start = end - (WEEKS - 1) * 7 * DAY_MS;
  const weeks = Array.from({ length: WEEKS }, (_, i) => ({ t: start + i * 7 * DAY_MS, n: 0 }));
  for (const r of days) {
    const i = Math.floor((Date.parse(r.day + "T00:00:00Z") - start) / (7 * DAY_MS));
    if (i >= 0 && i < WEEKS) weeks[i].n += r.n;
  }
  const max = Math.max(...weeks.map((w) => w.n));
  if (!max) return `<p class="us-empty">No installs counted in the last ${WEEKS} weeks.</p>`;
  const top = niceMax(max);
  const bars = weeks.map((w, i) => {
    const label = `Week of ${shortDate(w.t)}${i === WEEKS - 1 ? " (so far)" : ""}: ${num(w.n)} install${w.n === 1 ? "" : "s"}`;
    const h = w.n ? Math.max(2, (w.n / top) * 100) : 0;
    return `<li title="${esc(label)}"><span class="us-bar${i === WEEKS - 1 ? " us-now" : ""}" style="height:${h.toFixed(1)}%"></span><span class="us-sr">${esc(label)}</span></li>`;
  }).join("");
  // Month names under the first week starting in each month.
  const ticks = weeks.map((w, i) => {
    const d = new Date(w.t), prev = new Date(w.t - 7 * DAY_MS);
    return i > 0 && d.getUTCMonth() !== prev.getUTCMonth() ? `<span style="left:${((i / WEEKS) * 100).toFixed(2)}%">${MONTHS[d.getUTCMonth()]}</span>` : "";
  }).join("");
  return `<div class="us-chart" role="img" aria-label="Installs a week for the last ${WEEKS} weeks; the busiest week had ${num(max)}">` +
    `<div class="us-grid"><span style="bottom:100%">${num(top)}</span><span style="bottom:50%">${num(top / 2)}</span><span style="bottom:0">0</span></div>` +
    `<ol class="us-weeks">${bars}</ol><div class="us-ticks" aria-hidden="true">${ticks}</div></div>` +
    `<details class="us-table"><summary>As a table</summary><table><thead><tr><th>Week of</th><th>Installs</th></tr></thead><tbody>` +
    weeks.slice().reverse().map((w) => `<tr><td>${shortDate(w.t)}</td><td>${num(w.n)}</td></tr>`).join("") + `</tbody></table></details>`;
}

function niceMax(n: number): number {
  if (n <= 4) return 4;
  const p = 10 ** Math.floor(Math.log10(n));
  // (even, so the middle line is a whole number)
  for (const m of [2, 4, 6, 8, 10]) if (m * p >= n) return m * p;
  return 10 * p;
}

/** Counts by version, each a bar split by flash layout (the newest
 * versions first; the rest as one "Older" row). */
function byVersion(rows: Row[], what: string, empty: string): string {
  const per = new Map<string, Map<string, number>>();
  for (const r of rows) {
    const v = r.version!, l = LAYOUTS.some(([k]) => k === r.variant) ? r.variant! : "other";
    const m = per.get(v) ?? new Map();
    m.set(l, (m.get(l) ?? 0) + r.n);
    per.set(v, m);
  }
  if (!per.size) return `<p class="us-empty">${esc(empty)}</p>`;
  let vs = [...per.keys()].sort(newestFirst);
  const SHOWN = 8;
  if (vs.length > SHOWN) {
    const older = new Map<string, number>();
    for (const v of vs.slice(SHOWN - 1)) for (const [l, n] of per.get(v)!) older.set(l, (older.get(l) ?? 0) + n);
    vs = vs.slice(0, SHOWN - 1);
    per.set("Older", older);
    vs.push("Older");
  }
  const total = (v: string) => [...per.get(v)!.values()].reduce((a, b) => a + b, 0);
  const max = Math.max(...vs.map(total));
  const used = LAYOUTS.filter(([k]) => vs.some((v) => per.get(v)!.get(k)));
  const legend = `<ul class="us-legend">${used.map(([k, name]) => `<li><span class="us-key us-l-${k}"></span>${esc(name)}</li>`).join("")}</ul>`;
  const name = (v: string) => (v === "unknown" ? "Not sent" : v === "other" ? "Other" : v === "Older" ? "Older" : v);
  const lines = vs.map((v) => {
    const m = per.get(v)!, t = total(v);
    const segs = used.filter(([k]) => m.get(k)).map(([k, ln]) => {
      const n = m.get(k)!;
      return `<span class="us-seg us-l-${k}" style="width:${((n / max) * 100).toFixed(2)}%" title="${esc(`${name(v)}, ${ln}: ${num(n)} ${what}`)}"></span>`;
    }).join("");
    const parts = used.filter(([k]) => m.get(k)).map(([, ln]) => ln);
    const detail = used.filter(([k]) => m.get(k)).map(([k, ln]) => `${ln} ${num(m.get(k)!)}`).join(", ");
    return `<li><span class="us-v">${esc(name(v))}</span><span class="us-track">${segs}</span><span class="us-n">${num(t)}</span>` +
      `<span class="us-sr">${esc(`${name(v)}: ${num(t)} ${what}${parts.length ? ` (${detail})` : ""}`)}</span></li>`;
  }).join("");
  return legend + `<ol class="us-versions">${lines}</ol>`;
}

/** The numbers, as HTML for the built page's <!-- stats:numbers --> part. */
export async function numbers(db: D1Database, now = Date.now()): Promise<string> {
  const since30 = day(now - 29 * DAY_MS);
  const sinceWeeks = day(mondayOf(now) - (WEEKS - 1) * 7 * DAY_MS);
  const [totals, days, checks, installs] = await db.batch<Row>([
    db.prepare("select kind, sum(n) as n, sum(case when day >= ?1 then n else 0 end) as recent, min(day) as first from counts group by kind").bind(since30),
    db.prepare("select day, sum(n) as n from counts where kind = 'install' and day >= ?1 group by day").bind(sinceWeeks),
    db.prepare("select version, variant, sum(n) as n from counts where kind = 'check' and day >= ?1 group by version, variant").bind(since30),
    db.prepare("select version, variant, sum(n) as n from counts where kind = 'install' group by version, variant"),
  ]);
  const t = Object.fromEntries((totals.results ?? []).map((r) => [r.kind!, r]));
  const inst = t.install, chk = t.check;
  const first = [inst?.first, chk?.first].filter(Boolean).sort()[0];
  const since = first ? `since ${shortDate(Date.parse(first + "T00:00:00Z"))} ${first.slice(0, 4)}` : "nothing yet";
  return `<div class="us-tiles">` +
    tile(inst?.n ?? 0, "Installs", since) +
    tile(inst?.recent ?? 0, "Installs", "in the last 30 days") +
    tile(chk?.recent ?? 0, "Update checks", "in the last 30 days") +
    `</div>` +
    `<section class="us-panel"><h2>Installs a week</h2>${weekly(days.results ?? [], now)}</section>` +
    `<section class="us-panel"><h2>Update checks by version</h2><p class="us-about">The last 30 days, by the panel's firmware version and board.</p>` +
    byVersion(checks.results ?? [], "checks", "No update checks counted in the last 30 days.") + `</section>` +
    `<section class="us-panel"><h2>Installs by release</h2><p class="us-about">Every install counted, by the release installed and board.</p>` +
    byVersion(installs.results ?? [], "installs", "No installs counted yet.") + `</section>` +
    `<p class="us-asof">Updated every few minutes. Days are UTC (today is ${esc(day(now))}); the lighter bar is this week so far.</p>`;
}

function splice(page: string, inner: string): string | null {
  const a = "<!-- stats:numbers -->", b = "<!-- stats:numbers-end -->";
  const i = page.indexOf(a), j = page.indexOf(b);
  if (i < 0 || j < i) return null;
  return page.slice(0, i + a.length) + inner + page.slice(j);
}

/** GET /stats: the built page (stats.html) with the numbers written in;
 * the page as built (it says the numbers can't be read) when D1 fails. */
export async function page(request: Request, url: URL, env: CountsEnv): Promise<Response> {
  if (request.method !== "GET" && request.method !== "HEAD") {
    return new Response("method not allowed\n", { status: 405, headers: { Allow: "GET, HEAD", "Content-Type": "text/plain; charset=utf-8" } });
  }
  if (url.pathname !== "/stats") return Response.redirect(`${url.origin}/stats`, 301);
  const res = await env.ASSETS.fetch(new Request(`${url.origin}/stats`));
  if (!res.ok) return res;
  const built = await res.text();
  let text: string | null = null;
  try {
    if (home(request, url)) text = splice(built, await numbers(env.DB));
  } catch (e) {
    console.log(JSON.stringify({ statsPage: String((e as Error)?.message ?? e) }));
  }
  const out = new Response(request.method === "HEAD" ? null : (text ?? built), res);
  out.headers.set("Cache-Control", text ? "public, max-age=300" : "public, max-age=60");
  out.headers.delete("ETag");
  out.headers.delete("Content-Length");
  out.headers.set("X-Robots-Tag", "noindex");
  return out;
}
