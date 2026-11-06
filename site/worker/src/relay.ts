// Create Art's relay (opendmd-notes docs/research/tenor.md §5, "Bringing your own GIFs"):
//
//   POST /api/fetch   the address as the body (text/plain, at most 2 KB)
//
// One picture or video a visitor asked for on /create/, fetched and streamed
// back, nothing stored. The address is the body, not a query string, so
// that it is in no request log: Workers Logs' invocation logs
// (cloudflare.config.ts `observability`) record each request's URL. Create Art asks a host directly first (most GIF
// CDNs send CORS headers); only when the browser refuses that does it come
// here. It is a feature of our own page, not a proxy for anyone: every guard
// below is what keeps it that (Cloudflare's Self-Serve Subscription
// Agreement §2.2.1(j); the doc's "Terms and legal").
//
// Guards, in order (each refusal JSON {error, why} with no-store):
//   - the host: only on opendmd.dev (and cf dev's 127.0.0.1 / localhost);
//     anywhere else index.ts answers 404 (`home` below; cloudflare.config.ts
//     also turns workers.dev and preview URLs off);
//   - the caller: POST from our own page: Sec-Fetch-Site same-origin, or
//     (a browser that sends none) an Origin, or failing that a Referer, of
//     this origin; else 403 (why "caller"); another method 405;
//   - the page's token: X-Relay-Token, from POST /api/fetch/token (same
//     caller fence), an HMAC of its time and the caller's network (the
//     IPv4 address, or the IPv6 /64), good for 15 minutes; the key is a
//     random one kept in D1 (meta 'relay_key', made on first use); else 403
//     (why "token": the page asks for a new one and tries once more). This
//     stops a bare curl, not a determined script: the token is one same-
//     origin-looking POST away (opendmd-notes docs/research/tenor.md §5, "Caller");
//   - the rate: the RELAY_LIMIT binding (cloudflare.config.ts), keyed on
//     CF-Connecting-IP's network (an IPv6 /64, not the address: one host has
//     2^64 of them); 429 with Retry-After (why "rate");
//   - the address as written (below), then everyone's budget: GLOBAL_MINUTE
//     a minute and GLOBAL_DAY a day across all callers, two counters in D1
//     (meta 'relay_minute', 'relay_day'); over either, 503 with Retry-After
//     (why "busy"), as is a D1 that can't be asked (fail closed);
//   - the host denylist: D1 meta 'relay_deny', host names separated by
//     spaces, commas or lines, each refusing itself and its subdomains;
//     changed without a deploy (opendmd-notes docs/research/tenor.md §5); 400 (why "address");
//   - the address, at every hop: https only, port 443, no user or password,
//     no IP literal in any form the URL parser knows (decimal, hex, IPv6),
//     no localhost, single-label, .local/.internal/.lan/.home.arpa/... name,
//     no wildcard-DNS name that spells an address (127.0.0.1.nip.io,
//     10-0-0-1.sslip.io, localtest.me ...), none of our own hosts or
//     *.workers.dev / *.pages.dev; then the name resolved over DNS-over-HTTPS
//     (1.1.1.1) and refused if any A or AAAA record is private, loopback,
//     link-local, shared, documentation, multicast or otherwise reserved
//     (a Worker has no resolver of its own, and its fetch() cannot reach a
//     private network in any case: this check is the second fence, not
//     the only one); 400 (why "address"), or 502 (why "dns") when the
//     name does not resolve;
//   - redirects: followed by hand (redirect: "manual"), at most 3, each
//     Location checked as above; 502 (why "redirects");
//   - the request: GET, no cookies, no Referer, an honest User-Agent,
//     cache: "no-store", 15 s for the headers;
//   - the answer: 2xx or 502 (why "upstream", with its status); a
//     Content-Length over 16 MB 413 (why "size");
//   - the type, by the first bytes, whatever Content-Type says: GIF, PNG,
//     WebP, JPEG, MP4 (an ftyp box that is not an AVIF/HEIF still) or WebM
//     (EBML); anything else 415 (why "type"). Our own Content-Type from the
//     sniff, nosniff, a CSP that runs nothing, CORP same-origin;
//   - the size while streaming: a counting TransformStream errors the
//     stream past 16 MB (a body with no length, or one that lied);
//   - a page's address (a Tenor, Reddit, Imgur link): HTML, read up to
//     512 KB (or its </head>), answers JSON {media: [...]} with X-Relayed:
//     page, never the page itself: its og:video / og:image /
//     twitter:player:stream / twitter:image / link rel=image_src, https
//     only, GIFs first, then videos, then stills, at most 6; the page then
//     asks for one of them (the same guards again). None: 415 (why "type");
//   - the address body itself: at most 2 KB, counted as it is read (a
//     body with no Content-Length too);
//   - CPU: every pattern run over a page is linear (tags as [^<>]*, the
//     </head> search over the newest chunk only, at most 512 tags looked at);
//   - nothing logged: no console.log here, and the address is never in
//     the request's URL (above).
//
// Tests: tools/site-worker-test.py `relay` runs cf dev with
// SITE_TEST_RELAY_UPSTREAM (wrangler.config.ts → __RELAY_TEST_UPSTREAM__):
// hosts under .relay-test.invalid are then fetched from that local server
// (after every guard has passed on the address as written), and so are the
// DNS answers. Empty in every deploy (the Makefile unsets it).

declare const __RELAY_TEST_UPSTREAM__: string;

export interface RelayEnv {
  RELAY_LIMIT?: { limit(o: { key: string }): Promise<{ success: boolean }> };
  DB?: D1Database;
}

export const MAX_BYTES = 16 << 20;
const HTML_MAX = 512 << 10;
const BODY_MAX = 2048;
const TAGS_MAX = 512;
/** Everyone's loads together (a page's address costs two). */
export const GLOBAL_MINUTE = 120;
export const GLOBAL_DAY = 3000;
const TOKEN_TTL = 15 * 60;
const HOPS = 3;
const TIMEOUT_MS = 15_000;
const UA = "OpenDMD-CreateArt/1 (+https://opendmd.dev/create/)";
const DOH = "https://cloudflare-dns.com/dns-query";
const OWN = ["opendmd.dev", "opendmd.github.io"];
const TEST = typeof __RELAY_TEST_UPSTREAM__ === "string" ? __RELAY_TEST_UPSTREAM__ : "";
const TEST_SUFFIX = ".relay-test.invalid";

const HEADERS = {
  "Cache-Control": "no-store",
  "X-Content-Type-Options": "nosniff",
  "Content-Security-Policy": "default-src 'none'; sandbox; frame-ancestors 'none'",
  "Cross-Origin-Resource-Policy": "same-origin",
  "Referrer-Policy": "no-referrer",
};

type Why = "caller" | "token" | "busy" | "rate" | "url" | "address" | "dns" | "redirects" | "upstream" | "timeout" | "size" | "type";

class Refused extends Error {
  constructor(readonly status: number, readonly why: Why, message: string, readonly extra: Record<string, unknown> = {},
              readonly headers: Record<string, string> = {}) {
    super(message);
  }
}

function refuse(e: Refused, more: Record<string, string> = {}): Response {
  return new Response(JSON.stringify({ error: e.message, why: e.why, ...e.extra }) + "\n", {
    status: e.status,
    headers: { ...HEADERS, "Content-Type": "application/json; charset=utf-8", ...e.headers, ...more },
  });
}

/** The relay, its token and the gallery sync answer only here: not on
 * workers.dev or a version's preview URL, where an old version (with the
 * production D1) would otherwise keep running (index.ts). */
export function home(request: Request, url: URL): boolean {
  const host = (TEST && request.headers.get("X-Relay-Test-Host")) || url.hostname;
  return host === "opendmd.dev" || host === "127.0.0.1" || host === "localhost";
}

// ------------------------------------------------------------ the client

/** Who a rate is counted for: an IPv4 address, or an IPv6 /64 (one
 * subscriber's network: it holds 2^64 addresses). */
export function clientKey(ip: string | null): string {
  if (!ip) return "local";
  if (!ip.includes(":")) return ip;
  const w = v6words(ip);
  return w ? w.slice(0, 4).map((x) => x.toString(16)).join(":") + "::/64" : "v6:" + ip;
}

const b64url = (b: ArrayBuffer) => btoa(String.fromCharCode(...new Uint8Array(b))).replace(/\+/g, "-").replace(/\//g, "_").replace(/=+$/, "");
const unb64url = (s: string) => Uint8Array.from(atob(s.replace(/-/g, "+").replace(/_/g, "/")), (c) => c.charCodeAt(0));

let relayKey: Promise<CryptoKey> | null = null;
/** The token key: random, made once and kept in D1 (no secret to set up). */
function key(db: D1Database): Promise<CryptoKey> {
  relayKey ??= (async () => {
    const fresh = [...crypto.getRandomValues(new Uint8Array(32))].map((b) => b.toString(16).padStart(2, "0")).join("");
    const [, got] = await db.batch([
      db.prepare("insert into meta (key, value) values ('relay_key', ?1) on conflict(key) do nothing").bind(fresh),
      db.prepare("select value from meta where key = 'relay_key'"),
    ]);
    const hex = (got.results[0] as { value: string } | undefined)?.value ?? "";
    if (!/^[0-9a-f]{64}$/.test(hex)) throw new Error("no relay key");
    const raw = Uint8Array.from(hex.match(/../g)!, (h) => parseInt(h, 16));
    return crypto.subtle.importKey("raw", raw, { name: "HMAC", hash: "SHA-256" }, false, ["sign", "verify"]);
  })().catch((e) => {
    relayKey = null;
    throw e;
  });
  return relayKey;
}

const unavailable = () => new Refused(503, "busy", "Create Art can't fetch from other sites just now: try again in a while.", {}, { "Retry-After": "60" });

async function mint(env: RelayEnv, client: string): Promise<string> {
  if (!env.DB) throw unavailable();
  const ts = String(Math.floor(Date.now() / 1000));
  const sig = await crypto.subtle.sign("HMAC", await key(env.DB).catch(() => { throw unavailable(); }), new TextEncoder().encode(`${ts}|${client}`));
  return `${ts}.${b64url(sig)}`;
}

async function tokenOk(env: RelayEnv, client: string, token: string | null): Promise<boolean> {
  if (!env.DB) throw unavailable();
  const m = /^(\d{1,12})\.([A-Za-z0-9_-]{43})$/.exec(token ?? "");
  if (!m) return false;
  const age = Math.floor(Date.now() / 1000) - Number(m[1]);
  if (age < -60 || age > TOKEN_TTL) return false;
  const raw = unb64url(m[2]);
  // Only the canonical spelling: no second token for the same signature (padding bits in the last character).
  if (b64url(raw.buffer) !== m[2]) return false;
  return crypto.subtle.verify("HMAC", await key(env.DB).catch(() => { throw unavailable(); }), raw, new TextEncoder().encode(`${m[1]}|${client}`));
}

async function limited(env: RelayEnv, client: string): Promise<Response | null> {
  if (!env.RELAY_LIMIT) return null;
  const { success } = await env.RELAY_LIMIT.limit({ key: client });
  return success ? null : refuse(new Refused(429, "rate", "Too many tries in a minute: wait a moment and try again."), { "Retry-After": "60" });
}

/** POST /api/fetch/token: the page's token (`{token, ttl}`), for this caller's network. */
export async function token(request: Request, url: URL, env: RelayEnv): Promise<Response> {
  try {
    if (request.method !== "POST") return refuse(new Refused(405, "caller", "method not allowed"), { Allow: "POST" });
    if (!sameOrigin(request, url.origin)) throw new Refused(403, "caller", "Only Create Art on this site can use this.");
    const client = clientKey(request.headers.get("CF-Connecting-IP"));
    const slow = await limited(env, client);
    if (slow) return slow;
    return new Response(JSON.stringify({ token: await mint(env, client), ttl: TOKEN_TTL }) + "\n", {
      headers: { ...HEADERS, "Content-Type": "application/json; charset=utf-8" },
    });
  } catch (e) {
    return refuse(e instanceof Refused ? e : unavailable());
  }
}

// ------------------------------------------------------------ everyone's budget

// One row a counter: "<minute or day number> <count>", restarted when the number moves on.
const BUMP = `insert into meta (key, value) values (?1, ?2 || ' 1')
  on conflict(key) do update set value = case
    when substr(meta.value, 1, length(?2) + 1) = ?2 || ' ' then ?2 || ' ' || (cast(substr(meta.value, length(?2) + 2) as integer) + 1)
    else ?2 || ' 1' end
  returning value`;

/** Counts this load against everyone's minute and day; the host denylist. */
async function budget(env: RelayEnv, request: Request): Promise<string[]> {
  const db = env.DB;
  if (!db) throw unavailable();
  const cap = (h: string, d: number) => (TEST && Number(request.headers.get(h))) || d;
  const now = Date.now();
  let r: D1Result[];
  try {
    r = await db.batch([
      db.prepare(BUMP).bind("relay_minute", String(Math.floor(now / 60_000))),
      db.prepare(BUMP).bind("relay_day", String(Math.floor(now / 86_400_000))),
      db.prepare("select value from meta where key = 'relay_deny'"),
    ]);
  } catch {
    throw unavailable();
  }
  const count = (x: D1Result) => Number(String((x.results[0] as { value?: string } | undefined)?.value ?? "").split(" ")[1] ?? 0);
  if (count(r[0]) > cap("X-Relay-Test-Minute-Cap", GLOBAL_MINUTE)) {
    throw new Refused(503, "busy", "Lots of people are fetching through Create Art right now: wait a minute and try again.", {}, { "Retry-After": "60" });
  }
  if (count(r[1]) > cap("X-Relay-Test-Day-Cap", GLOBAL_DAY)) {
    const left = 86_400 - Math.floor((now % 86_400_000) / 1000);
    throw new Refused(503, "busy", "Create Art has fetched all it can for today: try again tomorrow, or download the picture and add the file here.", {},
      { "Retry-After": String(left) });
  }
  const deny = String((r[2].results[0] as { value?: string } | undefined)?.value ?? "");
  return deny.toLowerCase().split(/[\s,]+/).map((h) => h.replace(/^\*?\./, "").replace(/\.$/, "")).filter(Boolean);
}

// ------------------------------------------------------------ the caller

function sameOrigin(request: Request, origin: string): boolean {
  const site = request.headers.get("Sec-Fetch-Site");
  if (site) return site === "same-origin";
  const o = request.headers.get("Origin");
  if (o) return o === origin;
  const r = request.headers.get("Referer");
  if (!r) return false;
  try {
    return new URL(r).origin === origin;
  } catch {
    return false;
  }
}

// ------------------------------------------------------------ addresses

const BLOCKED_SUFFIXES = [
  "localhost", "local", "localdomain", "internal", "intranet", "lan", "home", "corp", "private",
  "home.arpa", "arpa", "test", "example", "invalid", "onion", "alt",
  "workers.dev", "pages.dev",
  // Wildcard DNS that answers with the address spelled in the name.
  "nip.io", "sslip.io", "xip.io", "nip.direct", "localtest.me", "lvh.me", "vcap.me", "traefik.me", "local.gd",
  "localhost.direct", "lacolhost.com", "fuf.me", "readme.localtest.me", "1u.ms", "rbndr.us",
];

/** Why the address may not be fetched, or null. Names only; DNS is `resolves`. */
export function badAddress(u: URL, ownHost: string): string | null {
  if (u.protocol !== "https:") return "only https:// addresses";
  if (u.username || u.password) return "no user names in addresses";
  if (u.port && u.port !== "443") return "only the standard port";
  let host = u.hostname.toLowerCase();
  if (host.endsWith(".")) host = host.slice(0, -1);
  if (!host) return "no host";
  if (TEST && host.endsWith(TEST_SUFFIX) && host.length > TEST_SUFFIX.length) return null;
  if (host.startsWith("[") || host.includes(":")) return "no IP addresses";
  if (/^[0-9.]+$/.test(host) || /^0x/i.test(host.split(".").pop() ?? "")) return "no IP addresses";
  if (!host.includes(".")) return "no single-word hosts";
  if (host === ownHost || OWN.some((o) => host === o || host.endsWith("." + o))) return "not our own site";
  if (BLOCKED_SUFFIXES.some((s) => host === s || host.endsWith("." + s))) return "not a public host";
  // A name that spells an IPv4 address (1.2.3.4.example, 10-0-0-1.host, 0a000001.host).
  if (/(^|[.-])(\d{1,3})[.-](\d{1,3})[.-](\d{1,3})[.-](\d{1,3})([.-]|$)/.test(host)) return "not a public host";
  if (/(^|\.)[0-9a-f]{8}\.(rbndr|1u)\b/.test(host)) return "not a public host";
  return null;
}

/** IPv4 dotted quad: private or reserved. */
function privateV4(ip: string): boolean {
  const p = ip.split(".").map(Number);
  if (p.length !== 4 || p.some((x) => !Number.isInteger(x) || x < 0 || x > 255)) return true;
  const [a, b, c] = p;
  return a === 0 || a === 10 || a === 127 || a >= 224
    || (a === 100 && b >= 64 && b <= 127)
    || (a === 169 && b === 254)
    || (a === 172 && b >= 16 && b <= 31)
    || (a === 192 && b === 0 && (c === 0 || c === 2))
    || (a === 192 && b === 88 && c === 99)
    || (a === 192 && b === 168)
    || (a === 198 && (b === 18 || b === 19))
    || (a === 198 && b === 51 && c === 100)
    || (a === 203 && b === 0 && c === 113);
}

/** IPv6 as eight numbers, or null. */
function v6words(ip: string): number[] | null {
  let s = ip.toLowerCase().replace(/^\[|\]$/g, "").split("%")[0];
  const v4 = /(\d+\.\d+\.\d+\.\d+)$/.exec(s);
  if (v4) {
    const q = v4[1].split(".").map(Number);
    s = s.slice(0, -v4[1].length) + ((q[0] << 8) | q[1]).toString(16) + ":" + ((q[2] << 8) | q[3]).toString(16);
  }
  const halves = s.split("::");
  if (halves.length > 2) return null;
  const part = (x: string) => (x ? x.split(":").map((w) => parseInt(w, 16)) : []);
  const head = part(halves[0]), tail = halves.length === 2 ? part(halves[1]) : [];
  const fill = halves.length === 2 ? 8 - head.length - tail.length : 0;
  const w = [...head, ...Array(Math.max(0, fill)).fill(0), ...tail];
  return w.length === 8 && w.every((x) => Number.isInteger(x) && x >= 0 && x <= 0xffff) ? w : null;
}

/** IPv6: anything outside global unicast, and the special blocks inside it. */
function privateV6(ip: string): boolean {
  const w = v6words(ip);
  if (!w) return true;
  const v4of = (hi: number, lo: number) => `${hi >> 8}.${hi & 255}.${lo >> 8}.${lo & 255}`;
  if (w.slice(0, 5).every((x) => x === 0) && w[5] === 0xffff) return privateV4(v4of(w[6], w[7])); // ::ffff:a.b.c.d
  if (w[0] === 0x64 && w[1] === 0xff9b) return privateV4(v4of(w[6], w[7])); // NAT64
  if (w[0] === 0x2002) return privateV4(v4of(w[1], w[2])); // 6to4
  if ((w[0] & 0xe000) !== 0x2000) return true; // not 2000::/3
  if (w[0] === 0x2001 && w[1] < 0x200) return true; // 2001::/23: Teredo, ORCHID, benchmarking ...
  if (w[0] === 0x2001 && w[1] === 0xdb8) return true; // documentation
  if (w[0] === 0x3fff && w[1] < 0x1000) return true; // documentation (3fff::/20)
  return false;
}

export function privateIp(ip: string): boolean {
  return ip.includes(":") ? privateV6(ip) : privateV4(ip);
}

/** The name's A and AAAA records over DNS-over-HTTPS: refused if any is private. */
async function resolves(host: string): Promise<void> {
  const ask = async (type: "A" | "AAAA"): Promise<string[]> => {
    const q = `?name=${encodeURIComponent(host)}&type=${type}`;
    const r = await fetch((TEST ? TEST + "/dns-query" : DOH) + q, {
      headers: { Accept: "application/dns-json" },
      signal: AbortSignal.timeout(5000),
    });
    if (!r.ok) throw new Refused(502, "dns", "That site's name could not be looked up.");
    const d = (await r.json()) as { Status?: number; Answer?: { type: number; data: string }[] };
    if (d.Status !== 0 && d.Status !== 3) throw new Refused(502, "dns", "That site's name could not be looked up.");
    return (d.Answer ?? []).filter((a) => a.type === 1 || a.type === 28).map((a) => a.data);
  };
  let ips: string[];
  try {
    ips = (await Promise.all([ask("A"), ask("AAAA")])).flat();
  } catch (e) {
    if (e instanceof Refused) throw e;
    throw new Refused(502, "dns", "That site's name could not be looked up.");
  }
  if (!ips.length) throw new Refused(502, "dns", "That site could not be found.");
  if (ips.some(privateIp)) throw new Refused(400, "address", "That address is not on the public internet.");
}

async function checked(u: URL, ownHost: string, deny: string[]): Promise<void> {
  const why = badAddress(u, ownHost);
  if (why) throw new Refused(400, "address", `That address can't be used (${why}).`);
  const host = u.hostname.toLowerCase().replace(/\.$/, "");
  if (deny.some((d) => host === d || host.endsWith("." + d))) throw new Refused(400, "address", "That address can't be used (that site has asked not to be fetched from here).");
  await resolves(host);
}

/** The test hook: a .relay-test.invalid address, from the local server. */
function target(u: URL): string {
  const host = u.hostname.toLowerCase();
  if (TEST && host.endsWith(TEST_SUFFIX)) return `${TEST}/${host}${u.pathname}${u.search}`;
  return u.href;
}

// ------------------------------------------------------------ the type

export type Kind = { type: string; video: boolean } | null;

/** What the first bytes are, of the six kinds let through. */
export function sniff(b: Uint8Array): Kind {
  const s = (i: number, n: number) => String.fromCharCode(...b.subarray(i, i + n));
  if (b.length >= 6 && (s(0, 6) === "GIF87a" || s(0, 6) === "GIF89a")) return { type: "image/gif", video: false };
  if (b.length >= 8 && b[0] === 0x89 && s(1, 7) === "PNG\r\n\x1a\n") return { type: "image/png", video: false };
  if (b.length >= 12 && s(0, 4) === "RIFF" && s(8, 4) === "WEBP") return { type: "image/webp", video: false };
  if (b.length >= 3 && b[0] === 0xff && b[1] === 0xd8 && b[2] === 0xff) return { type: "image/jpeg", video: false };
  if (b.length >= 12 && s(4, 4) === "ftyp") {
    const brand = s(8, 4);
    if (/^(avif|avis|heic|heix|hevc|heif|mif1|msf1)$/.test(brand)) return null; // a still in an ftyp box
    return { type: "video/mp4", video: true };
  }
  if (b.length >= 4 && b[0] === 0x1a && b[1] === 0x45 && b[2] === 0xdf && b[3] === 0xa3) return { type: "video/webm", video: true };
  return null;
}

function looksHtml(head: Uint8Array, ctype: string): boolean {
  if (/^\s*(text\/html|application\/xhtml\+xml)/i.test(ctype)) return true;
  const t = new TextDecoder().decode(head.subarray(0, 64)).replace(/^﻿/, "").trimStart().toLowerCase();
  return t.startsWith("<!doctype html") || t.startsWith("<html") || t.startsWith("<head") || t.startsWith("<meta");
}

/** At least `n` bytes from the stream (fewer at its end), and what is left. */
async function head(reader: ReadableStreamDefaultReader<Uint8Array>, n: number): Promise<{ bytes: Uint8Array; done: boolean }> {
  const parts: Uint8Array[] = [];
  let len = 0;
  while (len < n) {
    const { value, done } = await reader.read();
    if (done) return { bytes: join(parts, len), done: true };
    parts.push(value);
    len += value.length;
  }
  return { bytes: join(parts, len), done: false };
}

function join(parts: Uint8Array[], len: number): Uint8Array {
  if (parts.length === 1) return parts[0];
  const out = new Uint8Array(len);
  let o = 0;
  for (const p of parts) {
    out.set(p, o);
    o += p.length;
  }
  return out;
}

// ------------------------------------------------------------ pages

/** og:video, og:image and their kin from a page's HTML, best first. */
export function mediaOf(html: string, base: URL): string[] {
  const found: { url: string; rank: number; order: number }[] = [];
  const attr = (tag: string, name: string) => {
    const m = new RegExp(`\\b${name}\\s*=\\s*("([^"]*)"|'([^']*)'|([^\\s>]+))`, "i").exec(tag);
    return m ? (m[2] ?? m[3] ?? m[4] ?? "") : null;
  };
  const types: Record<string, string> = {};
  // [^<>]*, not [^>]*: `<meta<meta<meta…` with no `>` would rescan the
  // rest of the page from every start (quadratic); this stops at the next `<`.
  const metas = (html.match(/<meta\b[^<>]*>/gi) ?? []).slice(0, TAGS_MAX);
  for (const tag of metas) {
    const key = (attr(tag, "property") ?? attr(tag, "name") ?? "").toLowerCase();
    const v = attr(tag, "content");
    if (v && (key === "og:video:type" || key === "og:image:type")) types[key] = v.toLowerCase();
  }
  const add = (raw: string | null, rank: number) => {
    if (!raw) return;
    let u: URL;
    try {
      u = new URL(decode(raw.trim()), base);
    } catch {
      return;
    }
    if (u.protocol === "http:") u.protocol = "https:";
    if (u.protocol !== "https:") return;
    if (/\.gif(\?|$)/i.test(u.pathname)) rank = Math.min(rank, 0);
    found.push({ url: u.href, rank, order: found.length });
  };
  const videoOk = !types["og:video:type"] || /^video\/(mp4|webm)/.test(types["og:video:type"]);
  const imageGif = types["og:image:type"] === "image/gif";
  for (const tag of metas) {
    const key = (attr(tag, "property") ?? attr(tag, "name") ?? "").toLowerCase();
    const v = attr(tag, "content");
    if (["og:video:secure_url", "og:video:url", "og:video"].includes(key) && videoOk) add(v, 1);
    else if (key === "twitter:player:stream") add(v, 1);
    else if (["og:image:secure_url", "og:image:url", "og:image"].includes(key)) add(v, imageGif ? 0 : 2);
    else if (key === "twitter:image" || key === "twitter:image:src") add(v, 3);
  }
  for (const tag of (html.match(/<link\b[^<>]*>/gi) ?? []).slice(0, TAGS_MAX)) {
    if ((attr(tag, "rel") ?? "").toLowerCase() === "image_src") add(attr(tag, "href"), 3);
  }
  const seen = new Set<string>();
  return found
    .sort((a, b) => a.rank - b.rank || a.order - b.order)
    .map((f) => f.url)
    .filter((u) => !seen.has(u) && !!seen.add(u))
    .slice(0, 6);
}

function decode(s: string): string {
  return s.replace(/&(amp|quot|#39|#x27|lt|gt);/g, (_, e) => ({ amp: "&", quot: '"', "#39": "'", "#x27": "'", lt: "<", gt: ">" })[e as string] ?? "");
}

/** A page's HTML up to its </head> or 512 KB, whatever its chunks' sizes.
 * Each chunk is searched for </head> with only a short overlap of the text
 * before it (searching the whole text again per chunk was quadratic). */
async function readPage(reader: ReadableStreamDefaultReader<Uint8Array>, first: Uint8Array, done: boolean): Promise<string> {
  const END = /<\/head\s{0,16}>/i;
  const dec = new TextDecoder();
  let len = Math.min(first.length, HTML_MAX);
  let text = dec.decode(first.subarray(0, len), { stream: true });
  let found = END.test(text);
  while (!done && !found && len < HTML_MAX) {
    const r = await reader.read();
    if (r.done) break;
    const chunk = r.value.subarray(0, HTML_MAX - len);
    len += chunk.length;
    const more = dec.decode(chunk, { stream: true });
    found = END.test(text.slice(-24) + more);
    text += more;
  }
  reader.cancel().catch(() => {});
  return text;
}

/** The request's body, the address: at most 2 KB, counted as it comes
 * (Content-Length is only a hint: a chunked body has none). */
async function address(request: Request): Promise<string> {
  if (Number(request.headers.get("Content-Length") ?? 0) > BODY_MAX) throw new Error("too long");
  if (!request.body) return "";
  const reader = request.body.getReader();
  const parts: Uint8Array[] = [];
  let n = 0;
  for (;;) {
    const { value, done } = await reader.read();
    if (done) break;
    n += value.length;
    if (n > BODY_MAX) {
      reader.cancel().catch(() => {});
      throw new Error("too long");
    }
    parts.push(value);
  }
  return new TextDecoder().decode(join(parts, n));
}

// ------------------------------------------------------------ the route

export async function relay(request: Request, url: URL, env: RelayEnv): Promise<Response> {
  try {
    if (request.method !== "POST") return refuse(new Refused(405, "caller", "method not allowed"), { Allow: "POST" });
    if (!sameOrigin(request, url.origin)) throw new Refused(403, "caller", "Only Create Art on this site can use this.");
    const client = clientKey(request.headers.get("CF-Connecting-IP"));
    if (!(await tokenOk(env, client, request.headers.get("X-Relay-Token")))) {
      throw new Refused(403, "token", "Reload Create Art and try again.");
    }
    const slow = await limited(env, client);
    if (slow) return slow;
    let u: URL;
    try {
      const raw = (await address(request)).trim();
      if (!raw || raw.length > BODY_MAX) throw new Error();
      u = new URL(raw);
    } catch {
      throw new Refused(400, "url", "That is not a web address.");
    }
    // The address as written first: what is refused anyway costs no one's budget.
    const why = badAddress(u, url.hostname);
    if (why) throw new Refused(400, "address", `That address can't be used (${why}).`);
    const deny = await budget(env, request);

    let res: Response | null = null;
    for (let hop = 0; ; hop++) {
      await checked(u, url.hostname, deny);
      try {
        res = await fetch(target(u), {
          method: "GET",
          redirect: "manual",
          headers: { "User-Agent": UA, Accept: "image/gif,image/webp,image/png,image/jpeg,video/mp4,video/webm;q=0.9,text/html;q=0.5,*/*;q=0.1" },
          cache: "no-store",
          signal: AbortSignal.timeout(TIMEOUT_MS),
        } as RequestInit);
      } catch (e) {
        const t = (e as Error).name === "TimeoutError";
        throw new Refused(t ? 504 : 502, t ? "timeout" : "upstream", t ? "That site took too long to answer." : "That site could not be reached.");
      }
      if (![301, 302, 303, 307, 308].includes(res.status)) break;
      const loc = res.headers.get("Location");
      res.body?.cancel().catch(() => {});
      if (!loc) throw new Refused(502, "upstream", "That site refused to send it.", { status: res.status });
      if (hop >= HOPS) throw new Refused(502, "redirects", "That site sent us around in circles (too many redirects).");
      try {
        u = new URL(loc, u);
      } catch {
        throw new Refused(502, "upstream", "That site refused to send it.", { status: res.status });
      }
    }
    if (!res.ok || !res.body) {
      res.body?.cancel().catch(() => {});
      throw new Refused(502, "upstream", "That site refused to send it.", { status: res.status });
    }
    const len = Number(res.headers.get("Content-Length") ?? "");
    const ctype = res.headers.get("Content-Type") ?? "";
    if (len > MAX_BYTES && !/^\s*text\/html/i.test(ctype)) {
      res.body.cancel().catch(() => {});
      throw new Refused(413, "size", "That file is too big (over 16 MB).");
    }
    const reader = res.body.getReader();
    const { bytes, done } = await head(reader, 16);
    const kind = sniff(bytes);
    if (!kind) {
      if (looksHtml(bytes, ctype)) {
        const media = mediaOf(await readPage(reader, bytes, done), u);
        if (!media.length) throw new Refused(415, "type", "That page has no picture or video we can find.");
        return new Response(JSON.stringify({ media }) + "\n", {
          headers: { ...HEADERS, "Content-Type": "application/json; charset=utf-8", "X-Relayed": "page" },
        });
      }
      reader.cancel().catch(() => {});
      throw new Refused(415, "type", "That is not a picture or a video.");
    }
    // The bytes read so far, then the rest, counted: past the cap the
    // stream errors and the page gets less than it asked for.
    let count = 0;
    const cap = new TransformStream<Uint8Array, Uint8Array>({
      transform(chunk, c) {
        count += chunk.length;
        if (count > MAX_BYTES) {
          c.error(new Error("over 16 MB"));
          reader.cancel().catch(() => {});
          return;
        }
        c.enqueue(chunk);
      },
    });
    const body = new ReadableStream<Uint8Array>({
      start(c) {
        c.enqueue(bytes);
        if (done) c.close();
      },
      async pull(c) {
        const r = await reader.read();
        if (r.done) c.close();
        else c.enqueue(r.value);
      },
      cancel() {
        reader.cancel().catch(() => {});
      },
    });
    return new Response(body.pipeThrough(cap), { headers: { ...HEADERS, "Content-Type": kind.type, "X-Relayed": "1" } });
  } catch (e) {
    if (e instanceof Refused) return refuse(e);
    return refuse(new Refused(502, "upstream", "That site could not be reached."));
  }
}
