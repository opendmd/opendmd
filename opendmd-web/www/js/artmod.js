// The second wasm module (pkg-art/: Add GIFs' fitting and the prepared-copy
// encoder, Smooth motion, tags; on the website also its converter's
// Studio parts: kept out of the module every page load fetches).
// Loaded and initialised once, on first use, whoever asks first.
import { load } from './wasm.js';

let mod = null;

export function art() {
  mod ??= load('pkg-art').then(([m]) => m)
    .catch((e) => { mod = null; throw e; });
  return mod;
}
