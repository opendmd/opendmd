// The opendmd-site Worker (opendmd-notes docs/research/site-cloudflare.md, phases S1-S2):
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
// script "opendmd-site" with its assets, on opendmd.dev and www.opendmd.dev
// only (`domains`, since the cutover S3). workers.dev and the per-version
// preview URLs are off (2026-11-05, security review M4): they ran the same
// Worker with the production D1, an old version's bugs included, and
// nothing needs them now that opendmd.dev serves. (The code refuses the
// relay, the sync and seeding off opendmd.dev as well: src/relay.ts `home`.) The gallery store (S6): the D1 database
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
      // /stats and the counted paths (src/counts.ts): the update check's
      // release.json and beta.json and the release's (and a beta's) install
      // images, counted and then answered by the assets unchanged. The rest
      // of /install/ skips the Worker.
      runWorkerFirst: [
        "/api/*",
        "/formulas/*",
        "/stats",
        "/stats/",
        "/install/release.json",
        "/install/beta.json",
        "/install/firmware/opendmd-*-app.bin",
        "/install/firmware/opendmd-*-full.bin",
        "/install/firmware/beta/opendmd-*-app.bin",
        "/install/firmware/beta/opendmd-*-full.bin",
      ],
    },
    env: {
      ASSETS: { type: "assets" },
      // The gallery store (cf d1 create --name opendmd-site, 2026-10-29).
      DB: { type: "d1", name: "opendmd-site", id: "72a1c0cf-e5c6-4c75-8ad1-8aaa2c6f3857" },
      // Create Art's relay (/api/fetch, src/relay.ts): 30 loads a minute per
      // visitor's network, an IPv4 address or an IPv6 /64 (Workers Rate
      // Limiting, per location and approximate; a link that names a page
      // costs two, a token one). The namespace is ours to pick, unique in
      // the account; nothing to create before a deploy. Everyone's minute
      // and day caps are counters in DB (relay.ts GLOBAL_MINUTE, GLOBAL_DAY:
      // the binding's period is at most a minute, and per location).
      RELAY_LIMIT: { type: "rate-limit", namespace: "1101", simple: { limit: 30, period: 60 } },
    },
    // The gallery sync (S6, src/gallery.ts): every 5 minutes it asks GitHub
    // for the public repo's main (one small request) and, only when it
    // moved, fetches that commit's gallery/ into the store, so a merged
    // contribution is on opendmd.dev within minutes, without a deploy.
    triggers: [triggers.scheduled({ schedule: "*/5 * * * *" })],
    workersDev: false,
    previewUrls: false,
    // Workers Logs: what the code logs (console.log: a failed sync, a
    // refused relay, a count that couldn't be written), but no invocation
    // logs, the per-request records with the visitor's address and
    // location (2026-11-05, the usage counts: installs and update checks
    // now run the Worker, and nothing per request is to be kept for them).
    observability: { enabled: true, logs: { invocationLogs: false } },
    // Custom domains (S3, 2026-10-28): Cloudflare writes the proxied DNS
    // records and the certificate. www → apex is the Worker's for /api/* and
    // /formulas/*; for the rest, a zone redirect rule (www.opendmd.dev/* →
    // https://opendmd.dev/$1, 301), since _redirects cannot match a host and
    // every other path skips the Worker.
    domains: ["opendmd.dev", "www.opendmd.dev"],
  },
});
