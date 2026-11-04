// Formulas drawn live in the site's LED panels: a 128x32 canvas inside a
// `.led` (site.css's dot mask), each frame drawn as the panel draws it by
// the demo's Psi (pkg-psi). Shared by the Psi reference's examples
// (dev/psi/psiref.js), the home page's panels (home.js, and its hero's
// show, hero.js) and the gallery's
// tiles on hover (formulas/ftiles.js); tools/site.py copies it to the
// site's root. No imports: the wasm loads only when `psi()` is first called.

export const UNSET = -2147483648; // a slider left at the formula's default

const loading = new Map();
// The demo's Psi, loaded once: {P, mem} (the module and its memory). A Psi
// holds one compiled formula; panels playing at the same time each want
// their own (`slot` 1, 2…: the module again under another address, so
// another instance; the wasm file is the same, fetched once).
export function psi(slot = 0) {
  if (!loading.has(slot)) {
    loading.set(slot, import('./demo/pkg-psi/opendmd_psi_web.js' + (slot ? '?' + slot : ''))
      .then(async (P) => ({ P, mem: (await P.default()).memory })));
  }
  return loading.get(slot);
}

// Compile `src` unless it is the one P compiled last. False if it does not
// compile (P.error() says why).
const compiled = new WeakMap();
export function use(P, src) {
  if (compiled.get(P) === src) return true;
  const ok = P.compile(src);
  compiled.set(P, ok ? src : null);
  return ok;
}

// The frame `ms` into a formula's run as the panel steps it: a whole
// number of its frames, wrapped at its loop (info(): [4] the loop, 0 for
// one that never repeats; [10] the frame).
export function frameAt(info, ms) {
  const D = info[10] || 20, n = info[4] ? Math.max(1, Math.floor(info[4] / D)) : Infinity;
  return (Math.floor(ms / D) % n) * D;
}

// A canvas for a panel, in its `.led` (no `label`: decoration, hidden).
export function panel(label) {
  const cv = Object.assign(document.createElement('canvas'), { width: 128, height: 32 });
  if (label) {
    cv.setAttribute('role', 'img');
    cv.setAttribute('aria-label', label);
  } else cv.setAttribute('aria-hidden', 'true');
  const led = Object.assign(document.createElement('span'), { className: 'led' });
  led.append(cv);
  return { cv, led, ctx: cv.getContext('2d'), img: null };
}

// Draw the compiled formula's frame at `ms` (frameAt) into `x` (panel()):
// over `under` (12,288 B of RGB) for one that draws over a GIF, with the
// wall clock and the sliders. Counts the frames in the canvas's
// data-frames.
export function paint(L, info, x, ms, under = null, d = new Date(), a = UNSET, b = UNSET, c = UNSET) {
  blit(x, rgbAt(L, info, ms, under, d, a, b, c));
}

// The compiled formula's frame at `ms`, as paint() draws it: 128 x 32 RGB,
// a view of the module's memory (good until its next call).
export function rgbAt({ P, mem }, info, ms, under = null, d = new Date(), a = UNSET, b = UNSET, c = UNSET) {
  const over = !!(info[5] & 32);
  if (over && under) new Uint8Array(mem.buffer, P.under(), 12288).set(under);
  P[over ? 'frameOver' : 'frame'](ms, d.getHours(), d.getMinutes(), d.getSeconds(), a, b, c);
  return new Uint8Array(mem.buffer, P.pixels(), 12288);
}

// Put 128 x 32 RGB on `x`'s canvas (panel()).
export function blit(x, rgb) {
  x.img ||= x.ctx.createImageData(128, 32);
  const o = x.img.data;
  for (let k = 0, j = 0; k < 16384; k += 4, j += 3) { o[k] = rgb[j]; o[k + 1] = rgb[j + 1]; o[k + 2] = rgb[j + 2]; o[k + 3] = 255; }
  x.ctx.putImageData(x.img, 0, 0);
  x.cv.dataset.frames = -~x.cv.dataset.frames;
}
