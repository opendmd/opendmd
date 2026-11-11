// The wasm modules (pkg/, pkg-psi/, pkg-art/; the site's pkg-psi-lang/, pkg-studio/) at addresses that change with
// every build: `?v=<build id>` (js/build.js, written by `make web`: a hash
// of the three folders). wasm-pack's file names carry no hash, so without
// it a browser that kept a module from an earlier build would run it with
// this build's scripts: an old compiler making Formulas the player refuses
// (opendmd-notes docs/notes/web-app.md "Module addresses"). Every import of a module and
// every fetch of its wasm goes through here; js/web.js is the main one's.
import { V } from './build.js';

const NAMES = { pkg: 'opendmd_web', 'pkg-psi': 'opendmd_psi_web', 'pkg-art': 'opendmd_art_web', 'pkg-psi-lang': 'opendmd_psi_web', 'pkg-studio': 'opendmd_studio_web' };

// `path` (from the app's root) at this build's address; `tag` first, for
// another instance of the same module (`look`, `demo`, a slot number).
export const at = (path, tag) => new URL(`../${path}?${tag ? tag + '&' : ''}v=${V}`, import.meta.url).href;

// Import `dir`'s module and instantiate its wasm, both at this build's
// addresses (the glue's own default() would fetch the wasm beside it with
// no query: the address's query is not carried over). The wasm's address
// has no tag, so every instance shares one download.
// → [the module, what its init returned (the exports: memory, …)].
export async function load(dir, tag) {
  const m = await import(at(`${dir}/${NAMES[dir]}.js`, tag));
  return [m, await m.default({ module_or_path: at(`${dir}/${NAMES[dir]}_bg.wasm`) })];
}
