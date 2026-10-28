// The opendmd-site Worker. It runs only for /f/* and /api/*
// (cloudflare.config.ts `runWorkerFirst`); everything else is a static
// asset. S1: the skeleton.
//
//   /api/health   200 {"ok": true, "site": "opendmd-site", "version": …}
//   /api/*        404 JSON
//   /f/ …         the static Formula page (opendmd-web/share/f/), whose
//                 Formula is in the hash (#formula=…): the asset as it is
//   /f/<id>       a short Formula link (S5, not live yet): the 404 page,
//                 never cached
//   www.opendmd.dev/…  301 to https://opendmd.dev/… (for the paths that
//                 reach the Worker; the cutover adds a zone redirect rule
//                 for the rest)
//
// Asset responses carry target/site/_headers' headers whether they come
// from the asset layer or through env.ASSETS here.

declare const __SITE_VERSION__: string;

interface Env {
  ASSETS: { fetch(request: Request | string): Promise<Response> };
}

const APEX = "opendmd.dev";
// A short Formula id: Crockford base32 (no i, l, o, u), 10 characters or
// more (docs/research/site-cloudflare.md §5.2).
const FORMULA_ID = /^\/f\/[0-9a-hjkmnp-tv-z]{10,}$/;
const API_HEADERS = {
  "Content-Type": "application/json; charset=utf-8",
  "Cache-Control": "no-store",
  "Access-Control-Allow-Origin": "*",
  "X-Content-Type-Options": "nosniff",
  "Content-Security-Policy": "default-src 'none'; frame-ancestors 'none'",
};

function json(body: unknown, status = 200, extra: Record<string, string> = {}): Response {
  return new Response(JSON.stringify(body) + "\n", { status, headers: { ...API_HEADERS, ...extra } });
}

function api(request: Request, url: URL): Response {
  const allow = "GET, HEAD, OPTIONS";
  if (url.pathname !== "/api/health") return json({ error: "not found" }, 404);
  if (request.method === "OPTIONS") {
    return new Response(null, {
      status: 204,
      headers: { ...API_HEADERS, "Access-Control-Allow-Methods": allow, "Access-Control-Max-Age": "86400" },
    });
  }
  if (request.method !== "GET" && request.method !== "HEAD") {
    return json({ error: "method not allowed" }, 405, { Allow: allow });
  }
  return json({ ok: true, site: "opendmd-site", version: __SITE_VERSION__ });
}

async function formula(request: Request, url: URL, env: Env): Promise<Response> {
  const res = await env.ASSETS.fetch(request);
  if (!FORMULA_ID.test(url.pathname) || res.status !== 404) return res;
  // A short link before short links exist: the site's 404 page, not cached
  // (so the link works the moment the Formula store does).
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
    if (url.pathname.startsWith("/api/")) return api(request, url);
    if (url.pathname.startsWith("/f/")) return formula(request, url, env);
    return env.ASSETS.fetch(request);
  },
};
