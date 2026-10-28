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
// (previewUrls). No custom domain, route, DNS record, D1, R2 or Durable
// Object: those come with the cutover (S3: `domains`) and short links (S5).
import { defineConfig } from "cf/config";

export default defineConfig({
  worker: {
    name: "opendmd-site",
    compatibilityDate: "2026-09-21",
    entrypoint: "./src/index.ts",
    assets: {
      // x.html is served at /x and x/index.html at /x/ (a .html address redirects once).
      htmlHandling: "auto-trailing-slash",
      notFoundHandling: "404-page",
      runWorkerFirst: ["/f/*", "/api/*"],
    },
    env: {
      ASSETS: { type: "assets" },
    },
    workersDev: true,
    previewUrls: true,
    observability: { enabled: true },
    // S3 (the cutover, not before): domains: ["opendmd.dev", "www.opendmd.dev"].
    // Cloudflare then writes the proxied DNS records and the certificate.
    // www → apex is the Worker's for /f/* and /api/*; for the rest, a zone
    // redirect rule (www.opendmd.dev/* → https://opendmd.dev/$1, 301), since
    // _redirects cannot match a host and every other path skips the Worker.
  },
});
