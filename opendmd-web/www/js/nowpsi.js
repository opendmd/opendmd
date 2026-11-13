// Now's preview of a Formula on the panel (imported by js/now.js only when
// one is on screen): its text found from the path the panel plays (a
// bundled one by name, formulas.json; one of yours on the card, its .psi;
// an inline one, the saved or bundled Formula that compiles to it; the
// one sent from this page, Show on panel's), drawn
// by pkg-psi (an instance of its own, `?now`) into the Now preview at the
// panel's position (`pos()`: the panel's ms into it, or null: the page's
// clock from now). Nothing found or compiled: `play` fails, and Now says
// so in plain words.

import { app } from './state.js';
import { Q, num, path as fpath, compiler, hasCompiler } from './psikit.js';
import { load } from './wasm.js';
import { store } from './util.js';

let M = null;
const own = () => (M ??= load('pkg-psi', 'now').then(([m, x]) => ({ ...m, mem: x.memory })).catch((e) => { M = null; throw e; }));
const bundled = () => fetch('formulas.json').then((r) => (r.ok ? r.json() : []), () => []).then((x) => (Array.isArray(x) ? x : []).filter((f) => typeof f?.src == 'string'));

// The Formula's text, or null.
async function source(P, p) {
  const head = p.split('?')[0];
  // (`~sent`: the one sent from this page, if it was.)
  if (head == '@formula:~sent') return app.device && (await import('./psishow.js')).current()?.src;
  if (head.startsWith('@formula:')) return (await bundled()).find((f) => f.stem === head.slice(9))?.src;
  if (head[0] == '/') return app.src.readText(head);
  if (head != '@formula') return null;
  if (!hasCompiler()) compiler(P);
  const want = p.split('&')[0];
  return [...[store.get('psi.mine', [])].flat(), ...await bundled()].find((f) => typeof f?.src == 'string' && fpath({ src: f.src }) == want)?.src;
}

// Plays `p` into `pv` (a PanelPreview) until the returned stop() is called;
// `fail()` if it stops drawing. Throws when it can't be previewed.
export async function play(p, pv, pos, fail) {
  const P = await own(), src = await source(P, p);
  if (!src || !P.compile(src)) throw new Error('no preview');
  const q = new URLSearchParams(p.split('?')[1] || ''), [a, b, c] = [...'abc'].map((k) => Q(num(q.get(k))));
  const i = P.info(), g = i[10] || 20, lp = i[4] === 0 ? Infinity : i[4] || 8000, t0 = performance.now();
  let raf = 0, n0 = -1;
  const step = (now) => {
    raf = requestAnimationFrame(step);
    if (pv.canvas.offsetParent === null) return;
    const n = Math.floor(Math.max(0, pos() ?? now - t0) / g);
    if (n === n0) return;
    n0 = n;
    const d = new Date();
    try {
      P.frame((n * g) % lp, d.getHours(), d.getMinutes(), d.getSeconds(), a, b, c);
      pv.p.showRgb(new Uint8Array(P.mem.buffer, P.pixels(), 12288));
    } catch { cancelAnimationFrame(raf); M = null; fail(); return; }
    // (For tools/web-smoke.py.)
    Object.assign(pv.canvas.dataset, { frames: -~pv.canvas.dataset.frames, ms: (n * g) % lp });
  };
  raf = requestAnimationFrame(step);
  return () => cancelAnimationFrame(raf);
}
