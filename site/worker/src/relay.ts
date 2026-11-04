// Create Art's relay (docs/research/tenor.md §5, "Bringing your own GIFs"):
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
//   - the caller: POST from our own page: Sec-Fetch-Site same-origin, or
//     (a browser that sends none) an Origin, or failing that a Referer, of
//     this origin; else 403 (why "caller"); another method 405;
//   - the rate: the RELAY_LIMIT binding (cloudflare.config.ts), keyed on
//     CF-Connecting-IP; 429 with Retry-After (why "rate");
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
}

export const MAX_BYTES = 16 << 20;
const HTML_MAX = 512 << 10;
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

type Why = "caller" | "rate" | "url" | "address" | "dns" | "redirects" | "upstream" | "timeout" | "size" | "type";

class Refused extends Error {
  constructor(readonly status: number, readonly why: Why, message: string, readonly extra: Record<string, unknown> = {}) {
    super(message);
  }
}

function refuse(e: Refused, more: Record<string, string> = {}): Response {
  return new Response(JSON.stringify({ error: e.message, why: e.why, ...e.extra }) + "\n", {
    status: e.status,
    headers: { ...HEADERS, "Content-Type": "application/json; charset=utf-8", ...more },
  });
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

async function checked(u: URL, ownHost: string): Promise<void> {
  const why = badAddress(u, ownHost);
  if (why) throw new Refused(400, "address", `That address can't be used (${why}).`);
  await resolves(u.hostname.toLowerCase().replace(/\.$/, ""));
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
  const metas = html.match(/<meta\b[^>]*>/gi) ?? [];
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
  for (const tag of html.match(/<link\b[^>]*>/gi) ?? []) {
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

async function readPage(reader: ReadableStreamDefaultReader<Uint8Array>, first: Uint8Array, done: boolean): Promise<string> {
  const dec = new TextDecoder();
  let text = dec.decode(first, { stream: true });
  let len = first.length;
  while (!done && len < HTML_MAX && !/<\/head\s*>/i.test(text)) {
    const r = await reader.read();
    if (r.done) break;
    len += r.value.length;
    text += dec.decode(r.value, { stream: true });
  }
  reader.cancel().catch(() => {});
  return text.slice(0, HTML_MAX);
}

// ------------------------------------------------------------ the route

export async function relay(request: Request, url: URL, env: RelayEnv): Promise<Response> {
  try {
    if (request.method !== "POST") return refuse(new Refused(405, "caller", "method not allowed"), { Allow: "POST" });
    if (!sameOrigin(request, url.origin)) throw new Refused(403, "caller", "Only Create Art on this site can use this.");
    if (env.RELAY_LIMIT) {
      const key = request.headers.get("CF-Connecting-IP") ?? "local";
      const { success } = await env.RELAY_LIMIT.limit({ key });
      if (!success) return refuse(new Refused(429, "rate", "Too many tries in a minute: wait a moment and try again."), { "Retry-After": "60" });
    }
    let u: URL;
    try {
      if (Number(request.headers.get("Content-Length") ?? 0) > 2048) throw new Error();
      const raw = (await request.text()).trim();
      if (!raw || raw.length > 2048) throw new Error();
      u = new URL(raw);
    } catch {
      throw new Refused(400, "url", "That is not a web address.");
    }

    let res: Response | null = null;
    for (let hop = 0; ; hop++) {
      await checked(u, url.hostname);
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
