// The main wasm module (pkg/opendmd_web.js), imported at this build's
// address (js/wasm.js) by everything that used to import it by name:
// `import web, { init } from './web.js'`. Its importers wait for it (a
// top-level await), as they waited for the static import.
import { at } from './wasm.js';

const web = await import(at('pkg/opendmd_web.js'));
let ready = null;
// Instantiate its wasm once, from this build's address.
export const init = () => (ready ??= web.default({ module_or_path: at('pkg/opendmd_web_bg.wasm') }));
export default web;
