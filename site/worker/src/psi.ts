// Psi in the Worker: the site's own pkg-psi (the demo's and /f/'s wasm,
// opendmd-psi-web built by make web), copied next to this file by
// wrangler.config.ts from the site being served, so the Worker checks a
// Formula with exactly the compiler the page draws it with.

import { initSync, compile, error, errorAt, info, controls } from "../psi/opendmd_psi_web.js";
import wasm from "../psi/opendmd_psi_web_bg.wasm";

let ready = false;

export interface Checked {
  ok: true;
  /** pkg-psi controls(): {name, about, look, own, controls} */
  meta: { name: string | null; about: string | null; look: unknown; controls: unknown[] };
  metaJson: string;
  /** Estimated µs a frame on the panel (psikit `est`: as a look when it draws over a GIF, else as an effect). */
  esp32_us: number;
}
export interface Refused {
  ok: false;
  error: string;
  line: number;
}

/** Compile `src` with Psi: its declarations and weight, or why not. */
export function check(src: string): Checked | Refused {
  if (!ready) {
    initSync({ module: wasm });
    ready = true;
  }
  let ok: boolean;
  try {
    ok = compile(src);
  } catch (e) {
    return { ok: false, error: `the compiler stopped: ${(e as Error).message}`, line: 0 };
  }
  if (!ok) {
    // (errorAt is in UTF-16 units, as JS strings index)
    const at = errorAt()[0] ?? 0;
    return { ok: false, error: error().replace(/`/g, ""), line: src.slice(0, at).split("\n").length };
  }
  const i = Array.from(info());
  const metaJson = controls(src);
  const est = i[5] & 32 ? (i[13] ?? i[6]) : (i[11] ?? i[6]);
  return { ok: true, meta: JSON.parse(metaJson), metaJson, esp32_us: est };
}
