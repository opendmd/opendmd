// The home page's hero panel as a little panel playing a playlist
// (site/pages/home.html: the bezel's `data-show` names a hidden list, one
// <li> an item: a bundled Formula, `data-formula`, or a GIF, `data-gif`,
// with a look over it, `data-look`; how long it plays, `data-ms`; the
// transition that brings it in, `data-in`, as the panel's settings name
// it, `kind:ms`; its words, the caption while it shows). home.js imports
// this once the page has loaded and the browser is idle; tools/site.py
// copies it to the site's root.
// The transitions are the panel's own: compose() is opendmd-core's
// compositor.rs (`compose`, `At`, `dissolve_rank`) line for line, the old
// picture a snapshot of what was shown when the item came in, the new one
// playing underneath. The GIFs are the site's own art, which the page has
// already loaded, decoded here (decode()) and timed by their own delays.
// Formulas draw through psiled.js, one Psi for the show (the item that
// comes in is compiled when it does). An item that cannot load is left
// out; with nothing to show the still stays. It stops while off-screen,
// with the tab hidden or when the visitor asks for less motion, and a
// click (or Enter) pauses it. Nothing goes to the console.

import { psi, use, frameAt, panel, rgbAt, blit } from './psiled.js';

const W = 128, H = 32, N = W * H;

// --- The panel's transitions (opendmd-core/src/compositor.rs) ------------

// dissolve_rank: a bijection on 0..4096, each step invertible on 12 bits.
const RANK = new Uint16Array(N).map((_, i) => {
  let x = (Math.imul(i, 0x9e5) + 0x3a7) & 4095;
  x ^= x >> 7;
  x = Math.imul(x, 0x6cb) & 4095;
  x ^= x >> 5;
  x = (Math.imul(x, 0xb3d) + 0x1f1) & 4095;
  return x ^ (x >> 6);
});

// A sweeping kind's length and soft band (px); [1, 1] for the others.
const sweep = (k) => (k === 'wipe_left' || k === 'wipe_right' ? [W, 2] : k === 'wipe_down' || k === 'wipe_up' ? [H, 2] : k === 'blinds' ? [H / 8, 1] : [1, 1]);
const pos = { wipe_left: (x) => W - 1 - x, wipe_right: (x) => x, wipe_down: (x, y) => y, wipe_up: (x, y) => H - 1 - y, blinds: (x, y) => y % (H / 8) };

// `old` to `nw` (128 x 32 RGB) by `kind` over `d` ms at `t`, into `out`.
// At t = 0 it is `old`, at t >= d `nw`. compositor.rs works in whole
// microseconds and integer steps; so does this.
export function compose(kind, d, t, old, nw, out) {
  d = Math.max(1, Math.round(d * 1000));
  t = Math.min(Math.round(t * 1000), d);
  const [len, band] = sweep(kind);
  const front = Math.floor((t * (len - 1 + band) * 256) / d);
  const n = Math.floor((N * t) / d), off = Math.floor((W * t + Math.floor(d / 2)) / d);
  const fin = 2 * t >= d, level = fin ? Math.floor(((2 * t - d) * 256) / d) : 256 - Math.floor((2 * t * 256) / d);
  const at = pos[kind];
  for (let y = 0, i = 0; y < H; y++) {
    for (let x = 0; x < W; x++, i++) {
      const o = i * 3;
      if (at) {
        // mix(old, new, weight): a + (b - a) * w / 256, rounded toward zero.
        const w = Math.floor(Math.min(Math.max(front - at(x, y) * 256, 0), band * 256) / band);
        for (let c = 0; c < 3; c++) out[o + c] = old[o + c] + Math.trunc(((nw[o + c] - old[o + c]) * w) / 256);
      } else if (kind === 'push') {
        const s = x + off < W ? old : nw, j = (x + off < W ? i + off : i + off - W) * 3;
        out[o] = s[j]; out[o + 1] = s[j + 1]; out[o + 2] = s[j + 2];
      } else if (kind === 'fade') {
        const s = fin ? nw : old;
        for (let c = 0; c < 3; c++) out[o + c] = (s[o + c] * level) >> 8;
      } else {
        const s = kind === 'dissolve' && RANK[i] >= n ? old : nw;
        out[o] = s[o]; out[o + 1] = s[o + 1]; out[o + 2] = s[o + 2];
      }
    }
  }
  return out;
}

// --- A GIF, decoded: its frames as 128 x 32 RGB and their delays ----------

export function decode(b) {
  if (String.fromCharCode(...b.subarray(0, 3)) !== 'GIF' || (b[6] | (b[7] << 8)) !== W || (b[8] | (b[9] << 8)) !== H) throw 0;
  let p = 13, gct = null, delay = 0, tr = -1, disp = 0;
  const table = (f) => { const t = b.subarray(p, p + (3 << ((f & 7) + 1))); p += t.length; return t; };
  if (b[10] & 128) gct = table(b[10]);
  const cv = new Uint8Array(N * 3), frames = [];
  const skip = () => { while (b[p]) p += b[p] + 1; p++; };
  while (p < b.length) {
    const c = b[p++];
    if (c === 0x3b) break;
    if (c === 0x21) {
      if (b[p++] === 0xf9) {
        disp = (b[p + 1] >> 2) & 7;
        delay = (b[p + 2] | (b[p + 3] << 8)) * 10;
        tr = b[p + 1] & 1 ? b[p + 4] : -1;
      }
      skip();
      continue;
    }
    if (c !== 0x2c) throw 0;
    const fx = b[p] | (b[p + 1] << 8), fy = b[p + 2] | (b[p + 3] << 8), fw = b[p + 4] | (b[p + 5] << 8), fh = b[p + 6] | (b[p + 7] << 8), f = b[p + 8];
    p += 9;
    const ct = f & 128 ? table(f) : gct, min = b[p++], data = [];
    while (b[p]) { data.push(b.subarray(p + 1, p + 1 + b[p])); p += b[p] + 1; }
    p++;
    const px = lzw(min, data, fw * fh), keep = disp === 3 ? cv.slice() : null;
    // Interlaced rows come in four passes.
    const rows = [];
    if (f & 64) for (const [s, k] of [[0, 8], [4, 8], [2, 4], [1, 2]]) for (let y = s; y < fh; y += k) rows.push(y);
    else for (let y = 0; y < fh; y++) rows.push(y);
    rows.forEach((y, r) => {
      for (let x = 0; x < fw; x++) {
        const v = px[r * fw + x], X = fx + x, Y = fy + y;
        if (v === tr || X >= W || Y >= H) continue;
        const o = (Y * W + X) * 3;
        cv[o] = ct[v * 3]; cv[o + 1] = ct[v * 3 + 1]; cv[o + 2] = ct[v * 3 + 2];
      }
    });
    frames.push({ rgb: cv.slice(), ms: delay || 100 });
    if (disp === 2) for (let y = fy; y < Math.min(fy + fh, H); y++) cv.fill(0, (y * W + fx) * 3, (y * W + Math.min(fx + fw, W)) * 3);
    if (keep) cv.set(keep);
    delay = 0; tr = -1; disp = 0;
  }
  if (!frames.length) throw 0;
  return frames;
}

function lzw(min, data, n) {
  const out = new Uint8Array(n + 4096), pre = new Int16Array(4096), suf = new Uint8Array(4096), len = new Uint16Array(4096);
  const clear = 1 << min;
  for (let i = 0; i < clear; i++) { suf[i] = i; len[i] = 1; }
  let size = min + 1, next = clear + 2, prev = -1, acc = 0, bits = 0, o = 0;
  for (const d of data) for (const byte of d) {
    acc |= byte << bits;
    bits += 8;
    while (bits >= size) {
      const code = acc & ((1 << size) - 1);
      acc >>>= size;
      bits -= size;
      if (code === clear) { size = min + 1; next = clear + 2; prev = -1; continue; }
      if (code === clear + 1 || o >= n) return out;
      const k = code < next ? code : prev;
      if (k < 0) return out;
      const start = o;
      o += len[k];
      for (let c = k, j = o - 1; j >= start; c = pre[c]) out[j--] = suf[c];
      if (code >= next) out[o++] = out[start];
      if (prev >= 0 && next < 4096) {
        pre[next] = prev; suf[next] = out[start]; len[next] = len[prev] + 1;
        if (++next === 1 << size && size < 12) size++;
      }
      prev = code;
    }
  }
  return out;
}

// --- The show ---------------------------------------------------------------

const get = (u, f) => fetch(u).then((r) => (r.ok ? r[f]() : Promise.reject()));

export async function show(box, slot, less) {
  const pic = box.querySelector('.led'), list = document.getElementById(box.dataset.show);
  const cap = box.parentElement.querySelector('.cap');
  let items = [...list.children].map((li) => {
    const [kind, ms] = (li.dataset.in || 'dissolve').split(':');
    return { formula: li.dataset.formula || li.dataset.look, gif: li.dataset.gif, ms: +li.dataset.ms || 6000, kind, tms: +ms || 400, cap: li.textContent.trim() };
  });
  // Each item's Formula source and GIF frames, fetched once (the first at
  // once, the others after it shows).
  const load = (it) => (it.ready ||= Promise.all([it.formula && get(it.formula, 'text'), it.gif && get(it.gif, 'arrayBuffer')])
    .then(([src, g]) => { it.src = src; it.frames = g && decode(new Uint8Array(g)); it.loop = it.frames?.reduce((a, f) => a + f.ms, 0); it.ok = true; return it; }));
  const L = await psi(slot);
  if (!(await load(items[0]).catch(() => null)) || less.matches) return;
  items.slice(1).forEach((it) => load(it).catch(() => { items = items.filter((i) => i !== it); }));

  const x = panel(pic.querySelector('img')?.alt || '');
  const shown = new Uint8Array(N * 3), old = new Uint8Array(N * 3);
  let it, info, tms = 0, ran = 0, prev = 0, raf = 0, seen = true, paused = false, cur = null, last = '';

  // Bring in `next`, from what is shown: compile its Formula (false when
  // it does not).
  const start = (next) => {
    if (next.src && !use(L.P, next.src)) return false;
    old.set(shown);
    it = next;
    tms = next.tms;
    info = it.src ? L.P.info() : null;
    // A Formula that never repeats starts somewhere else each time, as on the panel.
    if (info) L.P.setSeed(Math.floor(Math.random() * 65536));
    ran = 0;
    last = '';
    if (x.led.isConnected) {
      if (cap) cap.textContent = it.cap;
      x.cv.setAttribute('aria-label', it.cap);
      box.dataset.item = items.indexOf(it);
    }
    return true;
  };
  if (!start(items[0])) return;
  tms = 0; // The page opens on the first item: nothing to come in from.

  // The item's picture `ran` ms in: a GIF's frame by its delays, a
  // Formula's frame as the panel steps it, a look's over the GIF's.
  const frame = () => {
    let under = null, key = '';
    if (it.frames) {
      let t = ran % it.loop, i = 0;
      while (t >= it.frames[i].ms) t -= it.frames[i++].ms;
      under = it.frames[i].rgb;
      key = i;
    }
    if (!info) return [key, under];
    const ms = frameAt(info, ran);
    return [key + ':' + ms, () => rgbAt(L, info, ms, under)];
  };

  const tick = (now) => {
    raf = 0;
    const step = prev ? Math.min(now - prev, 100) : 0;
    prev = now;
    ran += step;
    // Next once this one has had its time and the next has loaded.
    if (ran >= it.ms && items.length > 1) {
      const n = items[(items.indexOf(it) + 1) % items.length];
      if (n.ok && !start(n)) items = items.filter((i) => i !== n);
    }
    const [key, get] = frame();
    if (key !== last) {
      last = key;
      try { cur = (typeof get === 'function' ? get() : get).slice(); } catch { return; }
    }
    if (ran < tms) compose(it.kind, tms, ran, old, cur, shown);
    else shown.set(cur);
    blit(x, shown);
    if (!x.led.isConnected) {
      pic.replaceWith(x.led);
      box.dataset.live = 'on';
      box.dataset.item = 0;
      box.tabIndex = 0;
      box.setAttribute('role', 'button');
      box.setAttribute('aria-pressed', false);
      box.setAttribute('aria-label', 'Pause the demo');
      box.title = 'Click to pause';
    }
    run();
  };
  const run = () => {
    const on = seen && !paused && !document.hidden && !less.matches;
    if (on && !raf) raf = requestAnimationFrame(tick);
    if (!on && raf) { cancelAnimationFrame(raf); raf = 0; }
    if (!on) prev = 0;
  };
  const toggle = () => {
    if (!x.led.isConnected) return;
    paused = !paused;
    box.toggleAttribute('data-paused', paused);
    box.title = paused ? 'Click to play' : 'Click to pause';
    box.setAttribute('aria-pressed', paused);
    run();
  };
  box.addEventListener('click', toggle);
  box.addEventListener('keydown', (e) => { if (e.key === 'Enter' || e.key === ' ') { e.preventDefault(); toggle(); } });
  new IntersectionObserver(([e]) => { seen = e.isIntersecting; run(); }).observe(box);
  document.addEventListener('visibilitychange', run);
  less.addEventListener('change', run);
  run();
}
