// The tooling side of the opendmd-site Worker (cloudflare.config.ts is the
// Worker itself): where the static assets are and the dev server's port.
// The Makefile sets SITE_DIR (an absolute path to make site's output),
// SITE_DEV_PORT and SITE_VERSION (git describe, shown by /api/health).
import { defineWranglerConfig } from "wrangler/experimental-config";

export default defineWranglerConfig({
  assetsDirectory: process.env.SITE_DIR ?? "../../target/site",
  define: {
    __SITE_VERSION__: JSON.stringify(process.env.SITE_VERSION ?? "dev"),
  },
  dev: {
    ip: "127.0.0.1",
    port: Number(process.env.SITE_DEV_PORT ?? 8787),
  },
  sendMetrics: false,
});
