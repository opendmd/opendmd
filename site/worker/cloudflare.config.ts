// The opendmd-site Worker (docs/research/site-cloudflare.md, phases S1-S2):
// `make site`'s output (target/site) as static assets, and this Worker only
// for /api/* and /formulas/* (`runWorkerFirst`); every other request is answered by
// the asset layer without running it. target/site carries _headers (CORS,
// caching, CSP per path), _redirects (the old doc addresses as 301s),
// .assetsignore and 404.html, all written by tools/site.py.
//
// This is `cf`'s own config format (it does not read wrangler.jsonc: without
// this file it would detect a "static" project and offer to write one).
// `cf` builds, serves and deploys by delegating to the pinned wrangler in
// package.json; wrangler.config.ts holds the tooling side (the assets
// directory, the dev port).
//
//   make site-dev            cf dev on http://localhost:8787 (SITE_DEV_PORT), serving target/site
//   make site-deploy-check   cf deploy --dry-run: builds and checks, uploads nothing
//   make site-deploy         make site, then cf deploy (the owner's OAuth login, or CLOUDFLARE_API_TOKEN)
//
// What a deploy does with this file as it is: creates or updates the Worker
// script "opendmd-site" with its assets, on opendmd-site.<account
// subdomain>.workers.dev (workersDev) and per-version preview URLs
// (previewUrls). Since the cutover (S3) it also serves opendmd.dev and
// www.opendmd.dev (`domains`). The gallery store (S6): the D1 database
// opendmd-site (DB; its schema in migrations/, applied by make
// site-d1-migrate, which make site-deploy-built runs first; the S5 short
// links that shared it are gone, 0003), a cron that syncs the public repo's
// gallery/ into it, and /formulas/* through the Worker (runWorkerFirst).
// Locally, cf dev keeps its own D1 in a SQLite file (--local --persist-to,
// the Makefile's SITE_DEV_STATE): nothing in the account is touched.
import { defineConfig, triggers } from "cf/config";

export default defineConfig({
  worker: {
    name: "opendmd-site",
    compatibilityDate: "2026-09-21",
    entrypoint: "./src/index.ts",
    assets: {
      // x.html is served at /x and x/index.html at /x/ (a .html address redirects once).
      htmlHandling: "auto-trailing-slash",
      notFoundHandling: "404-page",
      // /formulas/*: the gallery page and its files from the store (S6);
      // the Worker hands everything else under it to the assets.
      runWorkerFirst: ["/api/*", "/formulas/*"],
    },
    env: {
      ASSETS: { type: "assets" },
      // The gallery store (cf d1 create --name opendmd-site, 2026-10-29).
      DB: { type: "d1", name: "opendmd-site", id: "72a1c0cf-e5c6-4c75-8ad1-8aaa2c6f3857" },
      // Create Art's relay (/api/fetch, src/relay.ts): 30 loads a minute per
      // visitor's IP (Workers Rate Limiting, per location and approximate;
      // a link that names a page costs two). The namespace is ours to pick,
      // unique in the account; nothing to create before a deploy.
      RELAY_LIMIT: { type: "rate-limit", namespace: "1101", simple: { limit: 30, period: 60 } },
    },
    // The gallery sync (S6, src/gallery.ts): every 5 minutes it asks GitHub
    // for the public repo's main (one small request) and, only when it
    // moved, fetches that commit's gallery/ into the store, so a merged
    // contribution is on opendmd.dev within minutes, without a deploy.
    triggers: [triggers.scheduled({ schedule: "*/5 * * * *" })],
    workersDev: true,
    previewUrls: true,
    observability: { enabled: true },
    // Custom domains (S3, 2026-10-28): Cloudflare writes the proxied DNS
    // records and the certificate. www → apex is the Worker's for /api/* and
    // /formulas/*; for the rest, a zone redirect rule (www.opendmd.dev/* →
    // https://opendmd.dev/$1, 301), since _redirects cannot match a host and
    // every other path skips the Worker.
    domains: ["opendmd.dev", "www.opendmd.dev"],
  },
});
