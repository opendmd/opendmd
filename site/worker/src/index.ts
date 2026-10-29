// The opendmd-site Worker. It runs only for /f/* and /api/*
// (cloudflare.config.ts `runWorkerFirst`); everything else is a static
// asset. Short links (S5, docs/research/site-cloudflare.md "S5 as built"):
//
//   /api/health     200 {"ok": true, "site": "opendmd-site", "version": …}
//   POST /api/f     body: a Formula's source as text/plain (<= 4,096 bytes
//                   once CRLF is LF and the trailing blank space cut); it is
//                   checked (UTF-8, no control/invisible/bidi/private-use
//                   characters, compiles with the site's own Psi) and
//                   stored under its content address: 201 {id, url, name}
//                   (200 when it was already there: the same source is
//                   always the same id). 413 too long, 422 refused (with
//                   the compiler's sentence and line), 429 past the rate
//                   limit (POSTS, per client address), 403 from another
//                   site's page (an Origin that isn't this one).
//   /api/f/<id>     200 {id, src, name, about, controls, esp32_us, created}, CORS *
//   /api/*          404 JSON
//   /f/<id>         the Formula's page: the static /f/ page with the
//                   Formula in it (#f-data, read by f.js instead of the
//                   hash), its name as the title and OG tags; slider values
//                   as a query (?a=0.5). An unknown id: the 404 page.
//   /f/<id>.psi     the source as text/plain, CORS *
//   /f/ …           the static Formula page (opendmd-web/share/f/), whose
//                   Formula is in the hash (#formula=…): the asset as it is
//   www.opendmd.dev/…  301 to https://opendmd.dev/… (for the paths that
//                   reach the Worker; a zone redirect rule does the rest)
//
// Asset responses carry target/site/_headers' headers whether they come
// from the asset layer or through env.ASSETS here.

import { check } from "./psi";
import { D1Store, digest, type Store, type Stored } from "./store";

declare const __SITE_VERSION__: string;

interface Env {
  ASSETS: { fetch(request: Request | string): Promise<Response> };
  DB: D1Database;
  POSTS: { limit(o: { key: string }): Promise<{ success: boolean }> };
}

const APEX = "opendmd.dev";
// A short Formula id: Crockford base32 (no i, l, o, u), 10 characters or
// more (docs/research/site-cloudflare.md §5.2).
const ID = "[0-9a-hjkmnp-tv-z]{10,}";
const PAGE = new RegExp(`^/f/(${ID})$`);
const PSI = new RegExp(`^/f/(${ID})\\.psi$`);
const API_ONE = new RegExp(`^/api/f/(${ID})$`);
// What a stored Formula may be: the gallery's size cap.
const MAX_BYTES = 4096;
// The characters the gallery's checker refuses (f.js BAD, line by line):
// controls but tab, invisible and direction-changing ones, private use.
const BAD =
  /[\0-\x08\x0a-\x1f\x7f-\xa0\xad\u034f\u061c\u115f\u1160\u17b4\u17b5\u180b-\u180e\u2000-\u200f\u2028-\u202f\u205f-\u206f\u3000\u3164\ufe00-\ufe0f\ufeff\uffa0\ufff0-\uffff\ue000-\uf8ff\udb40-\udb43\udb80-\udbff]/;
const API_HEADERS = {
  "Content-Type": "application/json; charset=utf-8",
  "Cache-Control": "no-store",
  "X-Content-Type-Options": "nosniff",
  "Content-Security-Policy": "default-src 'none'; frame-ancestors 'none'",
};
const CORS = { "Access-Control-Allow-Origin": "*" };
// A stored Formula never changes; a takedown should land within minutes.
const CACHE_STORED = "public, max-age=600";

function json(body: unknown, status = 200, extra: Record<string, string> = {}): Response {
  return new Response(JSON.stringify(body) + "\n", { status, headers: { ...API_HEADERS, ...extra } });
}

function options(allow: string, cors: boolean): Response {
  return new Response(null, {
    status: 204,
    headers: { ...API_HEADERS, ...(cors ? CORS : {}), "Access-Control-Allow-Methods": allow, "Access-Control-Max-Age": "86400" },
  });
}

async function api(request: Request, url: URL, env: Env): Promise<Response> {
  const m = request.method;
  if (url.pathname === "/api/health") {
    const allow = "GET, HEAD, OPTIONS";
    if (m === "OPTIONS") return options(allow, true);
    if (m !== "GET" && m !== "HEAD") return json({ error: "method not allowed" }, 405, { ...CORS, Allow: allow });
    return json({ ok: true, site: "opendmd-site", version: __SITE_VERSION__ }, 200, CORS);
  }
  if (url.pathname === "/api/f") {
    // The site's own pages post here (same origin: no CORS); tools may
    // too (no Origin). Another site's page may not: no CORS headers, and
    // a 403 before anything is stored. (Sec-Fetch-Site first: cf dev
    // answers as opendmd.dev whatever address the page was loaded from.)
    const allow = "POST";
    if (m === "OPTIONS") return options(allow, false);
    if (m !== "POST") return json({ error: "method not allowed" }, 405, { Allow: allow });
    const site = request.headers.get("Sec-Fetch-Site"), origin = request.headers.get("Origin");
    if (site ? site !== "same-origin" && site !== "none" : origin && origin !== url.origin) {
      return json({ error: "short links are made on the site's own pages" }, 403);
    }
    return post(request, url, env);
  }
  const one = API_ONE.exec(url.pathname);
  if (one) {
    const allow = "GET, HEAD, OPTIONS";
    if (m === "OPTIONS") return options(allow, true);
    if (m !== "GET" && m !== "HEAD") return json({ error: "method not allowed" }, 405, { ...CORS, Allow: allow });
    const f = await store(env).get(one[1]);
    if (!f) return json({ error: "no such formula" }, 404, CORS);
    return json(
      { id: f.id, src: f.src, name: f.name, about: f.about, controls: f.controls ? JSON.parse(f.controls).controls : [], esp32_us: f.esp32_us, created: f.created },
      200,
      { ...CORS, "Cache-Control": CACHE_STORED },
    );
  }
  return json({ error: "not found" }, 404, CORS);
}

const store = (env: Env): Store => new D1Store(env.DB);

async function post(request: Request, url: URL, env: Env): Promise<Response> {
  // Every POST counts, a repeat too: per client address, per location.
  const who = request.headers.get("CF-Connecting-IP") ?? "local";
  if (!(await env.POSTS.limit({ key: who })).success) {
    return json({ error: "too many short links from here: try again in a minute" }, 429, { "Retry-After": "60" });
  }
  const len = Number(request.headers.get("Content-Length") ?? 0);
  if (len > 4 * MAX_BYTES) return json({ error: `a formula can be ${MAX_BYTES} bytes at most` }, 413);
  const raw = new Uint8Array(await request.arrayBuffer());
  if (raw.length > 4 * MAX_BYTES) return json({ error: `a formula can be ${MAX_BYTES} bytes at most` }, 413);
  let text: string;
  try {
    text = new TextDecoder("utf-8", { fatal: true }).decode(raw);
  } catch {
    return json({ error: "the formula isn't UTF-8 text" }, 400);
  }
  const src = text.replace(/\r\n?/g, "\n").replace(/\s+$/, "");
  if (!src) return json({ error: "no formula: send its source as the body" }, 400);
  const bytes = new TextEncoder().encode(src).length;
  if (bytes > MAX_BYTES) return json({ error: `a formula can be ${MAX_BYTES} bytes at most (this one is ${bytes})` }, 413);
  const badLine = src.split("\n").findIndex((l) => BAD.test(l.replace(/\t/g, "")));
  if (badLine >= 0) return json({ error: "the formula has an invisible or special character in it", line: badLine + 1 }, 422);

  const d = await digest(src);
  const s = store(env);
  const had = await s.byHash(d.hex);
  if (had) return json({ id: had, url: `${url.origin}/f/${had}`, new: false }, 200);
  const c = check(src);
  if (!c.ok) return json({ error: `this formula doesn't work: line ${c.line}: ${c.error}`, line: c.line }, 422);
  const [id, fresh] = await s.put(src, d, { name: c.meta.name, about: c.meta.about, controls: c.metaJson, esp32_us: c.esp32_us });
  return json({ id, url: `${url.origin}/f/${id}`, name: c.meta.name, new: fresh }, fresh ? 201 : 200);
}

/** Text for an attribute value written as HTML. */
const attr = (s: string) => s.replace(/&/g, "&amp;").replace(/"/g, "&quot;").replace(/</g, "&lt;").replace(/>/g, "&gt;");

async function page(request: Request, url: URL, env: Env, f: Stored): Promise<Response> {
  // (Not conditional: the asset's ETag is the page's, not this Formula's.)
  const res = await env.ASSETS.fetch(new Request(new URL("/f/", url), { method: request.method }));
  if (!res.ok) return res;
  const name = f.name || "Shared formula";
  const about = f.about || "A formula for the OpenDMD LED display: try it, then open it on your own display.";
  const canonical = `${url.origin}/f/${f.id}`;
  // The page's data for f.js (a JSON block, not a script: the CSP has no
  // inline scripts, and none is needed). `<` escaped, so the source cannot
  // end the element.
  const data = JSON.stringify({ id: f.id, src: f.src, name: f.name }).replace(/</g, "\\u003c");
  const head =
    `<link rel="canonical" href="${attr(canonical)}">` +
    `<meta property="og:type" content="website"><meta property="og:site_name" content="OpenDMD">` +
    `<meta property="og:title" content="${attr(name)}"><meta property="og:description" content="${attr(about)}">` +
    `<meta property="og:url" content="${attr(canonical)}"><meta name="twitter:card" content="summary">` +
    `<script type="application/json" id="f-data">${data}</script>`;
  const out = new HTMLRewriter()
    .on("title", { element: (e) => void e.setInnerContent(`${name} · OpenDMD`) })
    .on('meta[name="description"]', { element: (e) => void e.setAttribute("content", about) })
    .on("head", { element: (e) => void e.append(head, { html: true }) })
    .on("#f-name", { element: (e) => void e.setInnerContent(name) })
    .on("#f-code", { element: (e) => void e.setInnerContent(f.src + "\n") })
    .transform(res);
  const r = new Response(out.body, out);
  r.headers.set("Cache-Control", CACHE_STORED);
  r.headers.set("X-Robots-Tag", "noindex");
  r.headers.delete("ETag");
  return r;
}

async function formula(request: Request, url: URL, env: Env): Promise<Response> {
  const p = PAGE.exec(url.pathname), t = PSI.exec(url.pathname);
  if (p || t) {
    const f = await store(env).get((p || t)![1]);
    if (f && t) {
      return new Response(f.src + "\n", {
        headers: {
          "Content-Type": "text/plain; charset=utf-8", "Cache-Control": CACHE_STORED, "X-Content-Type-Options": "nosniff",
          "Content-Security-Policy": "default-src 'none'; frame-ancestors 'none'", "X-Robots-Tag": "noindex", ...CORS,
        },
      });
    }
    if (f) return page(request, url, env, f);
  }
  const res = await env.ASSETS.fetch(request);
  if (!(p || t) || res.status !== 404) return res;
  // An id with no Formula (never stored, or taken down): the 404 page, not
  // cached.
  const out = new Response(res.body, res);
  out.headers.set("Cache-Control", "no-store");
  out.headers.set("X-Robots-Tag", "noindex");
  return out;
}

export default {
  async fetch(request: Request, env: Env): Promise<Response> {
    const url = new URL(request.url);
    if (url.hostname === `www.${APEX}`) {
      return Response.redirect(`https://${APEX}${url.pathname}${url.search}`, 301);
    }
    if (url.pathname.startsWith("/api/")) return api(request, url, env);
    if (url.pathname.startsWith("/f/")) return formula(request, url, env);
    return env.ASSETS.fetch(request);
  },
};
