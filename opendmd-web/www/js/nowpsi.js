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

// (A trap leaves an instance unusable: the next is a fresh one, `gen`. `k`:
// the text its program was last compiled from, whoever compiled it, null
// after a failure; `reg`: the instance this module made psikit's compiler.)
let M = null, gen = 0, B = null, loops = 0, reg = null;
const own = () => (M ??= load('pkg-psi', 'now' + gen).then(([m, x]) => {
  const P = { ...m, mem: x.memory, k: null };
  P.compile = (src) => { const ok = m.compile(src); P.k = ok ? src : null; return ok; };
  return P;
}).catch((e) => { M = null; throw e; }));
// The bundled Formulas, read once (again after a failure: no answer
// throws, as js/util.js unreachable reads it; anything else is none).
const bundled = () => (B ??= fetch('formulas.json').then((r) => (r.ok ? r.json() : []), (e) => { e.link = true; throw e; })
  .then((x) => (Array.isArray(x) ? x : []).filter((f) => typeof f?.src == 'string'), (e) => { if (e.link) throw e; return []; })
  .catch((e) => { B = null; throw e; }));

// Not now, maybe soon: Now asks again at its next status (`no`: meanwhile
// it says there's no preview).
export class Later extends Error { constructor(no) { super('later'); this.no = no; } }

// The saved (this browser's) or bundled Formula an inline path
// (`@formula?psb=…`) was compiled from, or undefined.
export async function find(p) {
  const P = await own();
  if (!hasCompiler() || (reg && reg !== P)) compiler((reg = P));
  const want = p.split('&')[0];
  return [...[store.get('psi.mine', [])].flat(), ...await bundled()].find((f) => typeof f?.src == 'string' && fpath({ src: f.src }) == want);
}

// The Formula's text, or null. `start()`: when (page clock) the panel started what it plays.
async function source(p, start) {
  const head = p.split('?')[0];
  if (head == '@formula:~sent') {
    // This page's last send, if what the panel plays started by then (its
    // start, js/now.js, errs early) and has its name; else asked again at
    // each status (a send of ours on its way to it, a clock not yet
    // measured), saying there's none once its send is 2 s old.
    // (Proper: now_playing carrying the send's identity.)
    const s = app.device && (await import('./psishow.js')).sent(), t = start(), n = /^\s*name\s+"([^"\n]*)"/m.exec(s?.src || '')?.[1];
    if (!s) return null;
    if (t != null && t <= s.at + 300 && (!n || n.trim() == (app.status?.now_playing?.title || '').trim())) return s.src;
    throw new Later(performance.now() - s.at >= 2000);
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
      // (One instance: anything compiled since puts this one back.)
      if (P.k !== src && !P.compile(src)) throw 0;
      P.frame((n * g) % lp, d.getHours(), d.getMinutes(), d.getSeconds(), a, b, c);
      pv.p.showRgb(new Uint8Array(P.mem.buffer, P.pixels(), 12288));
    } catch { stop(); M = null; gen++; fail(); return; }
    // (For tools/web-smoke.py.)
    Object.assign(ds, { frames: -~ds.frames, ms: (n * g) % lp, src: p, len: src.length, q: `${a},${b},${c}` });
  };
  ds.loops = ++loops;
  raf = requestAnimationFrame(step);
  return stop;
}
