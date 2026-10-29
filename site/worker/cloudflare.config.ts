// The opendmd-site Worker (docs/research/site-cloudflare.md, phases S1-S2):
// `make site`'s output (target/site) as static assets, and this Worker only
// for /f/* and /api/* (`runWorkerFirst`); every other request is answered by
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
// www.opendmd.dev (`domains`). Short links (S5): the D1 database
// opendmd-site (DB; its schema in migrations/, applied by make
// site-d1-migrate before a deploy that needs it) and a rate limiter for
// POST /api/f (POSTS). The gallery store (S6): the same D1, a cron that
// syncs the public repo's gallery/ into it, and /formulas/* through the
// Worker (runWorkerFirst). Locally, cf dev keeps its own D1 in a SQLite file
// (--local --persist-to, the Makefile's SITE_DEV_STATE) and simulates the
// limiter: nothing in the account is touched.
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
      runWorkerFirst: ["/f/*", "/api/*", "/formulas/*"],
    },
    env: {
      ASSETS: { type: "assets" },
      // Shared Formulas (cf d1 create --name opendmd-site, 2026-10-29).
      DB: { type: "d1", name: "opendmd-site", id: "72a1c0cf-e5c6-4c75-8ad1-8aaa2c6f3857" },
      // POST /api/f: 10 a minute per client address (per Cloudflare
      // location; the namespace is this account's own number for it).
      POSTS: { type: "rate-limit", namespace: "20261029", simple: { limit: 10, period: 60 } },
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
    // records and the certificate. www → apex is the Worker's for /f/* and
    // /api/*; for the rest, a zone redirect rule (www.opendmd.dev/* →
    // https://opendmd.dev/$1, 301), since _redirects cannot match a host and
    // every other path skips the Worker.
    domains: ["opendmd.dev", "www.opendmd.dev"],
  },
});
