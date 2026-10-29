// The tooling side of the opendmd-site Worker (cloudflare.config.ts is the
// Worker itself): where the static assets are and the dev server's port.
// The Makefile sets SITE_DIR (an absolute path to make site's output),
// SITE_DEV_PORT and SITE_VERSION (git describe, shown by /api/health).
//
// It also copies the site's Psi (SITE_DIR/demo/pkg-psi: the wasm the demo
// and /f/ draw with) into ./psi/ (gitignored), which src/psi.ts bundles:
// POST /api/f checks a Formula with the very compiler of the site it
// serves, whichever command (dev, deploy, a dry run) reads this file.
import { cpSync, existsSync, mkdirSync } from "node:fs";
import { join } from "node:path";
import { defineWranglerConfig } from "wrangler/experimental-config";

const site = process.env.SITE_DIR ?? "../../target/site";
const psi = join(site, "demo/pkg-psi");
if (!existsSync(join(psi, "opendmd_psi_web_bg.wasm"))) {
  throw new Error(`${psi} has no opendmd_psi_web_bg.wasm: make site first`);
}
mkdirSync("psi", { recursive: true });
for (const f of ["opendmd_psi_web.js", "opendmd_psi_web_bg.wasm"]) cpSync(join(psi, f), join("psi", f));

export default defineWranglerConfig({
  assetsDirectory: site,
  define: {
    __SITE_VERSION__: JSON.stringify(process.env.SITE_VERSION ?? "dev"),
  },
  dev: {
    ip: "127.0.0.1",
    port: Number(process.env.SITE_DEV_PORT ?? 8787),
  },
  sendMetrics: false,
});
