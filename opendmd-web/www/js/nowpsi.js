// Now's preview of a Formula on the panel (imported by js/now.js only when
// one is on screen): its text found from the path the panel plays (a
// bundled one by name, formulas.json; one of yours on the card, its .psi;
// an inline one, the saved or bundled Formula that compiles to it; the
// panel's sent one, `@formula:~sent`, only while it is this page's last
// Show on panel, js/psishow.js `sent`: its bytes' CRC-32 the panel's
// `now_playing.sent`), drawn by pkg-psi (an instance of its own,
// `?now<gen>`) into the Now preview at the panel's position, with the
// panel's `now_playing.seed`. Nothing found or compiled: `play` fails, and
// Now says so in plain words. A Formula that reads the world (the clock,
// `since`) draws in step with the panel but not always the same picture.

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

// A Formula's frames on an even cadence at the display's refresh: the panel
// draws one every refresh, 2nd or 4th (10, 20 or 40 ms, the rate it opens at,
// info()[17]), which a monitor's 60 or 120 Hz can't show on that grid (a 10 ms
// grid at 60 Hz steps its time 10, 20, 20 ms: motion that speeds up and slows
// down; at 120 Hz, a frame held every 6th refresh). So a new frame every k
// display frames (k: the refreshes in the frame time, rounded down from a
// little past the half, at least 1), its time on by the refreshes since the last (k, or the frames a missed one spans),
// eased towards where the panel is (`at`, ms), never back but for a jump of
// more than 100 ms and k refreshes (a seek, a restart). Held (`at` the same
// again: paused), it draws once more exactly at `at`, then nothing.
// pacer(g)(now, at): the time to draw at, or null (nothing new: the last frame
// stays).
export function pacer(g) {
  let pT = 0, p0 = 0, f = 0, t = null, pa = null, k = 0;
  return (now, at) => {
    const vs = refresh(now);
    if (pT && now <= pT) return null;
    pT = now;
    // (Held: the frames held aren't refreshes missed, so the count starts again from here.)
    if (at == null || at === pa) {
      p0 = now;
      f = 0;
      return at == null || t == null || t === at ? null : (t = at);
    }
    pa = at;
    // k: the refreshes in the frame time, rounded down from a little past the half (20 ms at 120 Hz is
    // 2.4 refreshes: 2, unless the display is clearly slower); once the refresh is known (16 intervals),
    // it moves only past its boundary with a margin (up past k + 0.75, down below k - 0.55), so a
    // reading a little off can't make it 3 now and then, and a wrong one is left as soon as it's clear.
    const r = g / vs, want = Math.max(1, Math.floor(r + 0.4));
    if (!k || R.d.length < 16 || (want != k && (r > k + 0.75 || r < k - 0.55))) k = want;
    // (The refreshes since the last new frame: the frames since, or more when the time since says some
    // were missed — one timestamp's error, not each one's.)
    f++;
    const n = t == null ? k : Math.max(f, Math.round((now - p0) / vs));
    if (n < k) return null;
    const x = t == null ? at : t + n * vs, e = at - x;
    p0 = now;
    f = 0;
    return (t = t == null || Math.abs(e) > 100 + k * vs ? at : Math.max(t, x + e * 0.05));
  };
}
// The display's frame time, ms, shared by every pacer and fed every frame by
// the player (`refresh(now)`, the frame's time; the same frame again:
// nothing), so a Formula starts with it known. A first reading: the median of
// the single refreshes (missed frames left out: the intervals under 1.5 times
// their 20th percentile, then under 1.5 times the median of those, three times
// over, so fuzzed timestamps don't read it short). Then the last 60 intervals' span over the
// refreshes they hold (each counted as the whole number of those it spans):
// the jitter of coarse or fuzzed timestamps (Firefox rounds them to 1 ms)
// cancels but at its ends. Within 3% of a common rate, that rate. 60 Hz's
// until 4 are in; a display's new rate takes over within a second.
const R = { pT: 0, vs: 1000 / 60, d: [] }, RATES = [50, 60, 75, 90, 100, 120, 144, 165, 240];
export function refresh(now) {
  const dt = now - R.pT, first = !R.pT;
  if (dt > 0) R.pT = now;
  if (first || !(dt > 2 && dt < 100)) return R.vs;
  if (R.d.push(dt) > 60) R.d.shift();
  if (R.d.length >= 4) {
    const s = [...R.d].sort((a, b) => a - b);
    let m = s[Math.floor(s.length / 5)], n = 0, span = 0;
    for (let i = 0; i < 3; i++) { const one = s.filter((x) => x < m * 1.5); m = one[one.length >> 1]; }
    for (const x of R.d) { n += Math.max(1, Math.round(x / m - 0.1)); span += x; }
    const v = span / n, c = RATES.find((hz) => Math.abs(1000 / v - hz) < hz * 0.03);
    R.vs = c ? 1000 / c : v;
  }
  return R.vs;
}

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

// The Formula's text, or null. `start()`: when (page clock) the panel
// started what it plays; `sent`: its `now_playing.sent` (null: firmware
// without it).
async function source(p, start, sent) {
  const head = p.split('?')[0];
  if (head == '@formula:~sent') {
    // This page's last send, if its bytes are what the panel plays (`sent`);
    // else asked again at each status (a send of ours on its way to it),
    // saying there's none once its send is 2 s old. Firmware that doesn't
    // say: if what the panel plays started by then (its start, js/now.js,
    // errs early) and has its name.
    const s = app.device && (await import('./psishow.js')).sent();
    if (!s) return null;
    if (sent != null) {
      if (s.crc === sent) return s.src;
      if (performance.now() - s.at >= 2000) return null;
      throw new Later(false);
    }
    const t = start(), n = /^\s*name\s+"([^"\n]*)"/m.exec(s.src)?.[1];
    if (t != null && t <= s.at + 300 && (!n || n.trim() == (app.status?.now_playing?.title || '').trim())) return s.src;
    throw new Later(performance.now() - s.at >= 2000);
  }
  if (head.startsWith('@formula:')) return (await bundled()).find((f) => f.stem === head.slice(9))?.src;
  if (head[0] == '/') return app.src.readText(head);
  return head == '@formula' ? (await find(p))?.src : null;
}

// Plays `p` into `pv` (a PanelPreview) until the returned stop() is
// called or `alive()` turns false. `pos()`: the panel's ms into it, or
// null (the page's clock from now); `fail()` if it stops drawing; `seed`,
// `sent`: the panel's `now_playing` ones (null: not said). Throws when it
// can't be previewed (a Later: not yet; a network error as it came).
export async function play(p, pv, { pos, start, alive, fail, seed = null, sent = null }) {
  // (The display's refresh learnt while the module and the Formula load, so its first frames don't run at
  // 60 Hz's.)
  let warm = 0;
  const learn = (now) => { refresh(now); warm = requestAnimationFrame(learn); };
  warm = requestAnimationFrame(learn);
  try { return await play_(p, pv, { pos, start, alive, fail, seed, sent }); } finally { cancelAnimationFrame(warm); }
}
async function play_(p, pv, { pos, start, alive, fail, seed, sent }) {
  const P = await own();
  if (!alive()) return () => {};
  const src = await source(p, start, sent);
  if (!alive()) return () => {};
  if (!src || !P.compile(src)) throw new Error('no preview');
  // The panel's seed (a `loop forever` one draws its picture), else 0.
  P.setSeed?.(Number.isInteger(seed) ? seed : 0);
  const q = new URLSearchParams(p.split('?')[1] || ''), [a, b, c] = [...'abc'].map((k) => Q(num(q.get(k))));
  const i = P.info(), pace = pacer(i[17] || i[10] || 20), lp = i[4] === 0 ? Infinity : i[4] || 8000, t0 = performance.now(), ds = pv.canvas.dataset;
  let raf = 0, on = true;
  const stop = () => { if (on) { on = false; cancelAnimationFrame(raf); ds.loops = --loops; } };
  const step = (now) => {
    if (!alive()) return stop();
    raf = requestAnimationFrame(step);
    if (pv.canvas.offsetParent === null) return;
    const at = pace(now, Math.max(0, pos() ?? now - t0));
    if (at == null) return;
    const d = new Date(), ms = Math.round(at) % lp;
    try {
      // (One instance: anything compiled since puts this one back.)
      if (P.k !== src && !P.compile(src)) throw 0;
      P.frame(ms, d.getHours(), d.getMinutes(), d.getSeconds(), a, b, c);
      pv.p.showRgb(new Uint8Array(P.mem.buffer, P.pixels(), 12288));
    } catch { stop(); M = null; gen++; fail(); return; }
    // (For tools/web-smoke.py.)
    Object.assign(ds, { frames: -~ds.frames, ms, src: p, len: src.length, q: `${a},${b},${c}`, seed: Number.isInteger(seed) ? seed : '' });
  };
  ds.loops = ++loops;
  raf = requestAnimationFrame(step);
  return stop;
}
