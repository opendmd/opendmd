// opendmd-connect-s0: cf's own config for the scratch echo Worker (the cf
// CLI does not read wrangler.toml; the site's site/worker/cloudflare.config.ts
// is the model). Same settings as wrangler.toml: one SQLite-backed Durable
// Object class with hibernating WebSockets, only on connect-s0.opendmd.dev,
// no workers.dev, no preview URLs, no logs. Throwaway: delete after S0.
//   cf deploy --dry-run    builds and checks, uploads nothing
//   cf deploy              the maintainer's OAuth login
import { defineConfig } from "cf/config";

export default defineConfig({
  worker: {
    name: "opendmd-connect-s0",
    compatibilityDate: "2026-10-20",
    entrypoint: "./src/index.js",
    exports: {
      EchoS0: { type: "durable-object", storage: "sqlite" },
    },
    env: {
      ECHO: { type: "durable-object", worker: "opendmd-connect-s0", exportName: "EchoS0" },
    },
    workersDev: false,
    previewUrls: false,
    observability: { enabled: false, logs: { enabled: false, invocationLogs: false } },
    domains: ["connect-s0.opendmd.dev"],
  },
});
