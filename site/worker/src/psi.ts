// Psi in the Worker: the site's own pkg-psi (the demo's and /f/'s wasm,
// opendmd-psi-web built by make web), copied next to this file by
// wrangler.config.ts from the site being served, so the Worker checks a
// gallery Formula with exactly the compiler the page draws it with.

import { initSync, compile, program, error, errorAt, info, controls, frame, frameOver, pixels, under } from "../psi/opendmd_psi_web.js";
import wasm from "../psi/opendmd_psi_web_bg.wasm";

let mem: WebAssembly.Memory | null = null;

function ready(): WebAssembly.Memory {
  if (!mem) mem = (initSync({ module: wasm }) as unknown as { memory: WebAssembly.Memory }).memory;
  return mem;
}

/** A control as pkg-psi controls() describes it. */
export interface Control {
  slot: string;
  name: string;
  label: string;
}

export interface Compiled {
  ok: true;
  meta: { name: string | null; about: string | null; look: [string, number] | null; controls: Control[] };
  /** pkg-psi info(): [steps, …, loop ms (4), inputs (5), …, µs with the still cache on the VM (14)] */
  info: number[];
  /** pkg-psi program(): the Formula compiled for the panel, its length and the format's version (byte 3). */
  psb: number;
  abi: number;
}

/** Compile `src` for the gallery's checker (src/check.ts): its
 * declarations and info(), or the compiler's own sentence and where (in
 * UTF-16 units). It stays the module's program until the next compile, so
 * `thumbnail` can draw it. */
export function compiled(src: string): Compiled | { ok: false; error: string; at: number } {
  ready();
  let ok: boolean;
  try {
    ok = compile(src);
  } catch (e) {
    return { ok: false, error: `the compiler stopped: ${(e as Error).message}`, at: 0 };
  }
  if (!ok) return { ok: false, error: error(), at: errorAt()[0] ?? 0 };
  const info_ = Array.from(info());
  let meta: Compiled["meta"];
  try {
    meta = JSON.parse(controls(src));
  } catch {
    return { ok: false, error: "the checker could not read its declarations", at: 0 };
  }
  // (controls() compiles into the module's other program: the one drawn is still this.)
  const psb = program(src);
  return { ok: true, meta, info: info_, psb: psb.length, abi: psb[3] ?? 0 };
}

// The picture a Formula that draws over a GIF is previewed over (inputs bit
// 5): js/psikit.js `sample()`, opendmd-gallery's `sample()`.
let SAMPLE: Uint8Array | null = null;
function sample(): Uint8Array {
  if (!SAMPLE) {
    SAMPLE = new Uint8Array(12288);
    for (let i = 0; i < 4096; i++) {
      const x = i % 128, y = i >> 7;
      SAMPLE.set(
        y > 23 - 4 * Math.sin(x / 13) - 2 * Math.sin(x / 5) ? [30, 110 + ((x * 7 + y * 13) % 50), 40]
          : (x - 98) ** 2 + 1.5 * (y - 10) ** 2 < 50 ? [255, 215, 80] : [40 + y * 6, 60 + y * 4, 170 - y * 3],
        i * 3,
      );
    }
  }
  return SAMPLE;
}

/** The gallery's thumbnail of the program `compiled` last made: one 128 ×
 * 32 RGB frame 30% into its loop (of its first minute if it never repeats),
 * on a 20 ms frame, the clock at 10:09:30 and the controls at their
 * defaults, over the sample picture when it draws over a GIF: what
 * opendmd-gallery's `thumbnail` draws. */
export function thumbnail(c: Compiled): Uint8Array {
  const m = ready();
  const shown = c.info[4] || 60_000;
  const ms = Math.floor(Math.floor((shown * 3) / 10) / 20) * 20;
  const over = (c.info[5] & 32) !== 0;
  const UNSET = -2147483648;
  if (over) new Uint8Array(m.buffer, under(), 12288).set(sample());
  (over ? frameOver : frame)(ms, 10, 9, 30, UNSET, UNSET, UNSET);
  return new Uint8Array(m.buffer, pixels(), 12288).slice();
}
