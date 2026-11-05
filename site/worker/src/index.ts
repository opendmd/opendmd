// The opendmd-site Worker. It runs only for /api/* and /formulas/*
// (cloudflare.config.ts `runWorkerFirst`); everything else is a static
// asset, the Formula page /f/ among them (it names a gallery Formula,
// `#gallery=<slug>`: there are no anonymous short links, 2026-10-30;
// sharing is submitting to the gallery, reviewed on GitHub).
//
//   /api/health     200 {"ok": true, "site": "opendmd-site", "version": …}
//   POST /api/fetch Create Art's relay (src/relay.ts): one picture or video
//                   from another site (its address the body), for /create/
//                   only, streamed, never stored
//   POST /api/fetch/token  the relay's token for the page (src/relay.ts)
//   /api/*          404 JSON
//   www.opendmd.dev/…  301 to https://opendmd.dev/… (for the paths that
//                   reach the Worker; a zone redirect rule does the rest)
//   any other host (workers.dev, a preview URL, should either be on again):
//                   the relay, its token and the gallery sync 404, and
//                   /formulas/* only the assets (no store, so no seeding)
//
// The gallery store (S6, src/gallery.ts, "S6 as built"):
//
//   /formulas/      the built gallery page, its two lists written from the
//                   store (X-Gallery: store; the page as built, X-Gallery:
//                   built, until the store is ready or when D1 fails)
//   /formulas/src/<stem>.psi     the file, from the store (an example first),
//                   CORS * (a panel's web app fetches it: Open on my panel)
//   /formulas/thumbs/<slug>.png  a contribution's thumbnail, from the store
//   /formulas/…     anything else: the asset
//   GET /api/gallery        the store: entries, the commit synced, refusals
//   POST /api/gallery/sync  sync the public repo's gallery/ now (what the
//                   cron does): 200 the report, 429 when one started under
//                   10 s ago, 502 when GitHub couldn't be read
//   cron (cloudflare.config.ts)  the same sync
//
// Asset responses carry target/site/_headers' headers whether they come
// from the asset layer or through env.ASSETS here.

import * as gallery from "./gallery";
import * as relay from "./relay";

declare const __SITE_VERSION__: string;

interface Env extends gallery.GalleryEnv, relay.RelayEnv {
  ASSETS: { fetch(request: Request | string): Promise<Response> };
  DB: D1Database;
}

const APEX = "opendmd.dev";
const SLUG = "[a-z0-9]+(?:-[a-z0-9]+)*";
const GALLERY_SRC = new RegExp(`^/formulas/src/(${SLUG})\\.psi$`);
const GALLERY_THUMB = new RegExp(`^/formulas/thumbs/(${SLUG})\\.png$`);
const API_HEADERS = {
  "Content-Type": "application/json; charset=utf-8",
  "Cache-Control": "no-store",
  "X-Content-Type-Options": "nosniff",
  "Content-Security-Policy": "default-src 'none'; frame-ancestors 'none'",
};
const CORS = { "Access-Control-Allow-Origin": "*" };

function json(body: unknown, status = 200, extra: Record<string, string> = {}): Response {
  return new Response(JSON.stringify(body) + "\n", { status, headers: { ...API_HEADERS, ...extra } });
}

function options(allow: string, cors: boolean): Response {
  return new Response(null, {
    status: 204,
    headers: { ...API_HEADERS, ...(cors ? CORS : {}), "Access-Control-Allow-Methods": allow, "Access-Control-Max-Age": "86400" },
  });
}

async function api(request: Request, url: URL, env: Env, ctx: ExecutionContext): Promise<Response> {
  const m = request.method;
  if (url.pathname === "/api/gallery") {
    const allow = "GET, HEAD, OPTIONS";
    if (m === "OPTIONS") return options(allow, true);
    if (m !== "GET" && m !== "HEAD") return json({ error: "method not allowed" }, 405, { ...CORS, Allow: allow });
    try {
      return json(await gallery.api(env), 200, { ...CORS, "Cache-Control": "public, max-age=30" });
    } catch (e) {
      console.log(JSON.stringify({ galleryApi: (e as Error).message }));
      return json({ error: "the gallery store can't be read just now" }, 503, CORS);
    }
  }
  const offHome = url.pathname === "/api/gallery/sync" || url.pathname === "/api/fetch" || url.pathname === "/api/fetch/token";
  if (offHome && !relay.home(request, url)) return json({ error: "not found" }, 404, CORS);
  if (url.pathname === "/api/gallery/sync") {
    // Anyone may ask: it only does sooner what the cron does, from the
    // public repo, and one run at a time (10 s apart at least).
    const allow = "POST";
    if (m === "OPTIONS") return options(allow, false);
    if (m !== "POST") return json({ error: "method not allowed" }, 405, { Allow: allow });
    const r = await gallery.sync(env, url.origin);
    if (r.busy) return json({ error: "a sync started a moment ago: try again in a few seconds" }, 429, { "Retry-After": "10" });
    return json(r, r.ok ? 200 : 502);
  }
  if (url.pathname === "/api/fetch") return relay.relay(request, url, env);
  if (url.pathname === "/api/fetch/token") return relay.token(request, url, env);
  if (url.pathname === "/api/health") {
    const allow = "GET, HEAD, OPTIONS";
    if (m === "OPTIONS") return options(allow, true);
    if (m !== "GET" && m !== "HEAD") return json({ error: "method not allowed" }, 405, { ...CORS, Allow: allow });
    return json({ ok: true, site: "opendmd-site", version: __SITE_VERSION__ }, 200, CORS);
  }
  return json({ error: "not found" }, 404, CORS);
}

async function formulas(request: Request, url: URL, env: Env, ctx: ExecutionContext): Promise<Response> {
  if (request.method !== "GET" && request.method !== "HEAD") return env.ASSETS.fetch(request);
  // Off opendmd.dev the store is not touched: an old version would seed its own examples into it.
  if (!relay.home(request, url)) return env.ASSETS.fetch(request);
  if (url.pathname === "/formulas/") return gallery.page(request, env, ctx);
  const s = GALLERY_SRC.exec(url.pathname);
  if (s) return gallery.file(request, env, ctx, "src", s[1]);
  const t = GALLERY_THUMB.exec(url.pathname);
  if (t) return gallery.file(request, env, ctx, "thumb", t[1]);
  return env.ASSETS.fetch(request);
}

export default {
  async fetch(request: Request, env: Env, ctx: ExecutionContext): Promise<Response> {
    const url = new URL(request.url);
    if (url.hostname === `www.${APEX}`) {
      return Response.redirect(`https://${APEX}${url.pathname}${url.search}`, 301);
    }
    if (url.pathname.startsWith("/api/")) return api(request, url, env, ctx);
    if (url.pathname.startsWith("/formulas/")) return formulas(request, url, env, ctx);
    return env.ASSETS.fetch(request);
  },

  // The cron (cloudflare.config.ts): the public repo's gallery/ into the store.
  async scheduled(_event: ScheduledController, env: Env, ctx: ExecutionContext): Promise<void> {
    ctx.waitUntil(
      gallery.sync(env).then((r) => {
        if (!r.busy && (!r.ok || r.changed)) console.log(JSON.stringify({ gallerySync: r }));
      }),
    );
  },
};
