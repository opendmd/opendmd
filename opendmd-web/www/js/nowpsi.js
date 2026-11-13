// Now's preview of a Formula on the panel (imported by js/now.js only when
// one is on screen): its text found from the path the panel plays (a
// bundled one by name, formulas.json; one of yours on the card, its .psi;
// an inline one, the saved or bundled Formula that compiles to it; the
// panel's sent one, `@formula:~sent`, only while it is this page's last
// Show on panel, js/psishow.js `sent`), drawn by pkg-psi (an instance of
// its own, `?now<gen>`) into the Now preview at the panel's position.
// Nothing found or compiled: `play` fails, and Now says so in plain words.
// A Formula that reads its seed or the world (the clock, `since`) draws in
// step with the panel but not always the same picture.

import { app } from './state.js';
import { Q, num, path as fpath, compiler, hasCompiler } from './psikit.js';
import { load } from './wasm.js';
import { store } from './util.js';

// (A trap leaves an instance unusable: the next is a fresh one, `gen`.)
let M = null, gen = 0, B = null, loops = 0;
const own = () => (M ??= load('pkg-psi', 'now' + gen).then(([m, x]) => ({ ...m, mem: x.memory })).catch((e) => { M = null; throw e; }));
// The bundled Formulas, read once (again after a failure).
const bundled = () => (B ??= fetch('formulas.json').then((r) => { if (!r.ok) throw r; return r.json(); })
  .then((x) => (Array.isArray(x) ? x : []).filter((f) => typeof f?.src == 'string'), () => { B = null; return []; }));

// Not now, maybe soon: Now asks again at its next status.
export class Later extends Error {}

// The saved (this browser's) or bundled Formula an inline path
// (`@formula?psb=…`) was compiled from, or undefined.
export async function find(p) {
  if (!hasCompiler()) compiler(await own());
  const want = p.split('&')[0];
  return [...[store.get('psi.mine', [])].flat(), ...await bundled()].find((f) => typeof f?.src == 'string' && fpath({ src: f.src }) == want);
}

// The Formula's text, or null. `start()`: when (page clock) the panel started what it plays.
async function source(p, start) {
  const head = p.split('?')[0];
  if (head == '@formula:~sent') {
    // This page's last send, if what the panel plays started by then and
    // has its name (another's sent since: none; a send of ours on its way
    // to the status: not yet).
    const s = app.device && (await import('./psishow.js')).sent(), t = start(), n = /^\s*name\s+"([^"\n]*)"/m.exec(s?.src || '')?.[1];
    if (!s || t == null) return null;
    if (t <= s.at + 300 && (!n || n.trim() == app.status?.now_playing?.title)) return s.src;
    if (performance.now() - s.at < 2000) throw new Later();
    return null;
  }
  if (head.startsWith('@formula:')) return (await bundled()).find((f) => f.stem === head.slice(9))?.src;
  if (head[0] == '/') return app.src.readText(head);
  return head == '@formula' ? (await find(p))?.src : null;
}

// Plays `p` into `pv` (a PanelPreview) until the returned stop() is
// called or `alive()` turns false. `pos()`: the panel's ms into it, or
// null (the page's clock from now); `fail()` if it stops drawing. Throws
// when it can't be previewed (a Later: not yet; a network error as it came).
export async function play(p, pv, { pos, start, alive, fail }) {
  const P = await own();
  if (!alive()) return () => {};
  const src = await source(p, start);
  if (!alive()) return () => {};
  if (!src || !P.compile(src)) throw new Error('no preview');
  const q = new URLSearchParams(p.split('?')[1] || ''), [a, b, c] = [...'abc'].map((k) => Q(num(q.get(k))));
  const i = P.info(), g = i[10] || 20, lp = i[4] === 0 ? Infinity : i[4] || 8000, t0 = performance.now(), ds = pv.canvas.dataset;
  let raf = 0, n0 = -1, on = true;
  const stop = () => { if (on) { on = false; cancelAnimationFrame(raf); ds.loops = --loops; } };
  const step = (now) => {
    if (!alive()) return stop();
    raf = requestAnimationFrame(step);
    if (pv.canvas.offsetParent === null) return;
    const n = Math.floor(Math.max(0, pos() ?? now - t0) / g);
    if (n === n0) return;
    n0 = n;
    const d = new Date();
    try {
      // (One instance: a Formula compiled since for another path puts this one back.)
      if (P.k !== src && !P.compile(src)) throw 0;
      P.k = src;
      P.frame((n * g) % lp, d.getHours(), d.getMinutes(), d.getSeconds(), a, b, c);
      pv.p.showRgb(new Uint8Array(P.mem.buffer, P.pixels(), 12288));
    } catch { stop(); M = null; gen++; fail(); return; }
    // (For tools/web-smoke.py.)
    Object.assign(ds, { frames: -~ds.frames, ms: (n * g) % lp, src: p, len: src.length, q: `${a},${b},${c}` });
  };
  P.k = src;
  ds.loops = ++loops;
  raf = requestAnimationFrame(step);
  return stop;
}
