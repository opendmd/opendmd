// The tooling side of the opendmd-site Worker (cloudflare.config.ts is the
// Worker itself): where the static assets are and the dev server's port.
// The Makefile sets SITE_DIR (an absolute path to make site's output),
// SITE_DEV_PORT and SITE_VERSION (git describe, shown by /api/health).
//
// It also copies the site's Psi (SITE_DIR/demo/pkg-psi: the wasm the demo
// and /f/ draw with) into ./psi/ (gitignored), which src/psi.ts bundles:
// POST /api/f checks a Formula with the very compiler of the site it
// serves, whichever command (dev, deploy, a dry run) reads this file.
import { createHash } from "node:crypto";
import { cpSync, existsSync, mkdirSync, readFileSync } from "node:fs";
import { join } from "node:path";
import { defineWranglerConfig } from "wrangler/experimental-config";

const site = process.env.SITE_DIR ?? "../../target/site";
const psi = join(site, "demo/pkg-psi");
if (!existsSync(join(psi, "opendmd_psi_web_bg.wasm"))) {
  throw new Error(`${psi} has no opendmd_psi_web_bg.wasm: make site first`);
}
mkdirSync("psi", { recursive: true });
for (const f of ["opendmd_psi_web.js", "opendmd_psi_web_bg.wasm"]) cpSync(join(psi, f), join("psi", f));

// The gallery store (S6): this build's examples are known by the hash of
// formulas/gallery.json's examples (the Worker seeds them from that file
// when the store holds another build's), and the public repo whose
// gallery/ the cron syncs. SITE_TEST_GALLERY_SOURCE points cf dev at
// site-worker-test's fixture repo; the Makefile's deploys clear it.
const built = join(site, "formulas/gallery.json");
if (!existsSync(built)) throw new Error(`${built} is missing: make site first`);
const examples = createHash("sha256").update(JSON.stringify(JSON.parse(readFileSync(built, "utf8")).examples)).digest("hex").slice(0, 16);
const gallerySource = process.env.SITE_TEST_GALLERY_SOURCE || "https://github.com/opendmd/opendmd.github.io";

export default defineWranglerConfig({
  assetsDirectory: site,
  define: {
    __SITE_VERSION__: JSON.stringify(process.env.SITE_VERSION ?? "dev"),
    __GALLERY_EXAMPLES__: JSON.stringify(examples),
    __GALLERY_SOURCE__: JSON.stringify(gallerySource),
  },
  dev: {
    ip: "127.0.0.1",
    port: Number(process.env.SITE_DEV_PORT ?? 8787),
  },
  sendMetrics: false,
});
