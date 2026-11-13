// A sprite from a picture, in the Formula editor (js/formulas.js imports it
// when a picture is chosen with Picture…, dropped on the code or pasted
// into it; js/fsprite.js's Replace from a picture… too). Everything stays
// in the browser: the file is read here, never sent or kept (only the
// dialog's last settings are, `psi.import`). One dialog: the picture with a
// crop, the size, how it fits, the sampling, the colours (three tiers, by
// what the store pays), a palette, the background, the name; its result as
// a grid, on the panel's scale, and what it costs the Formula (the
// candidate compiled by the editor's own compiler, `kit.still`). The dialog
// only peeks (the editor's preview plays the Formula with it); Add is one
// put(), one step of the browser's undo. Its own size line
// (tools/web-size.py).
//
// The pure half is exported for the smoke: sniff, sanitize, resample,
// fitIn, opaque, quantise, keyed, block, cost and make. Deterministic: the
// pixels read 1:1 off a canvas (never the browser's smoothing), resampled
// here, colours reduced by a median cut and a few Lloyd passes in a fixed
// order, no randomness and no dithering.

import { h, clear, store, toast } from './util.js';
import { popup, closeMenu } from './overrides.js';
import { sprites, palettes } from './fsprite.js';

const MB = 1 << 20, SIDE = 4096, WORK = 512, RW = 512, RH = 256, TIERS = [2, 4, 16];
// A new key character after its colour's initial (js/fsprite.js addKey's).
const ABC = 'ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz*+=%&@$';
const HEX = /^#([\da-f]{3}|[\da-f]{6})$/i;
// Den 16, OpenDMD's own palette (the Studio's, opendmd-web/share/studio/palettes.js).
const DEN = [0x0a0e2a, 0x161d52, 0x2e3a8c, 0x6447c8, 0x33d1ff, 0x8a4f1d, 0x14532d, 0x2f9e4f, 0xf7a531, 0xe8601c, 0xc92a2a, 0xff4f8b, 0xf1ebdd, 0x7c7f8a, 0xffffff];
const say = (t) => Object.assign(new Error(t), { said: 1 });
const six = (v) => (v.length == 4 ? '#' + [...v.slice(1)].map((c) => c + c).join('') : v).toLowerCase();
const rgbOf = (v) => parseInt(six(v).slice(1), 16);
// A colour written: #rgb when it is one, else #rrggbb.
const hx = (c) => { const s = c.toString(16).padStart(6, '0'); return '#' + (/^(.)\1(.)\2(.)\3$/.test(s) ? s[0] + s[2] + s[4] : s); };
// As the one it replaces was written: #rgb when it fits, upper case kept (js/fsprite.js `as`).
const as = (v, w) => { const x = six(v), y = w.length == 4 && /^#(.)\1(.)\2(.)\3$/.test(x) ? '#' + x[1] + x[3] + x[5] : x; return /[A-F]/.test(w) ? y.toUpperCase() : y; };

// --- What a file is, by its first bytes (never its name or type).
export function sniff(b) {
  const at = (s, k = 0) => [...s].every((c, i) => b[k + i] === c.charCodeAt(0));
  if (b[0] == 0x89 && at('PNG\r\n\x1a\n', 1)) return 'png';
  if (b[0] == 0xff && b[1] == 0xd8 && b[2] == 0xff) return 'jpeg';
  if (at('GIF87a') || at('GIF89a')) return 'gif';
  if (at('RIFF') && at('WEBP', 8)) return 'webp';
  if (at('BM')) return 'bmp';
  // An SVG: its root first (an XML declaration and comments before it), never an HTML page around one.
  const t = new TextDecoder().decode(b.subarray(0, 4096)).replace(/^﻿/, '').replace(/^(\s+|<\?xml[^>]*\?>|<!--[\s\S]*?-->)*/, '');
  return /^<([\w.-]+:)?svg[\s>/]/.test(t) ? 'svg' : null;
}

// An SVG made safe to draw: no DOCTYPE or entities; parsed as XML (never
// put in the page), its root an `svg` in the SVG namespace; no scripts,
// foreign content, frames, objects, media or animation; no `on…`
// attributes; links only to its own `#parts` (a PNG or JPEG `data:` image
// in an <image>); no styles that load (`@import`, `url(` not `url(#`).
// Sized at RW × RH at most, its own shape kept. {text, w, h}.
const NS = 'http://www.w3.org/2000/svg';
const BAD = new Set(['script', 'foreignObject', 'iframe', 'object', 'embed', 'audio', 'video', 'set', 'animate', 'animateMotion', 'animateTransform', 'handler', 'listener']);
const LOADS = /url\(\s*['"]?\s*(?!#)|@import/i;
export function sanitize(text) {
  const no = () => say('This SVG can’t be read here');
  if (text.length > MB) throw say('This SVG is too big to read here: 1 MB at most');
  if (/<!DOCTYPE|<!ENTITY/i.test(text)) throw no();
  const d = new DOMParser().parseFromString(text, 'image/svg+xml'), r = d.documentElement;
  if (r.namespaceURI != NS || r.localName != 'svg' || d.getElementsByTagNameNS('*', 'parsererror').length) throw no();
  for (const n of [...d.childNodes]) if (n !== r) n.remove();
  const scrub = (e) => {
    for (const a of [...e.attributes]) {
      const n = a.localName.toLowerCase(), v = a.value.trim();
      if (n.startsWith('on') || (n == 'href' && !(v[0] == '#' || (e.localName == 'image' && /^data:image\/(png|jpeg);base64,/i.test(v)))) || LOADS.test(v)) e.removeAttributeNode(a);
    }
  };
  for (const e of [...r.querySelectorAll('*')]) {
    if (e.namespaceURI != NS || BAD.has(e.localName) || (e.localName == 'style' && LOADS.test(e.textContent))) e.remove();
    else scrub(e);
  }
  scrub(r);
  // What it would draw, every <use> copying what it points at counted
  // through (a loop counts as endless): past 20,000 parts it is refused, as
  // drawing it would hold the page up (a picture is drawn in one go).
  const ids = new Map([...r.querySelectorAll('[id]')].map((e) => [e.id, e])), memo = new Map(), on = new Set();
  const parts = (e) => {
    if (memo.has(e)) return memo.get(e);
    if (on.has(e)) return Infinity;
    on.add(e);
    let n = 1;
    for (const c of e.children) if ((n += parts(c)) > 2e4) break;
    const t = e.localName == 'use' && ids.get((e.getAttribute('href') || e.getAttribute('xlink:href') || '').slice(1));
    if (t) n += parts(t);
    on.delete(e);
    memo.set(e, n);
    return n;
  };
  let big;
  try { big = parts(r) > 2e4; } catch { big = true; }
  if (big) throw say('This SVG is too complex to draw here');
  const vb = (r.getAttribute('viewBox') || '').trim().split(/[\s,]+/).map(Number), dim = (k) => { const v = parseFloat(r.getAttribute(k)); return v > 0 && v < 1e5 ? v : 0; };
  let [aw, ah] = vb.length == 4 && vb[2] > 0 && vb[3] > 0 ? vb.slice(2) : [dim('width'), dim('height')];
  if (!(aw && ah)) [aw, ah] = [300, 150];
  if (!(vb.length == 4 && vb[2] > 0 && vb[3] > 0)) r.setAttribute('viewBox', `0 0 ${aw} ${ah}`);
  const z = Math.min(RW / aw, RH / ah), w = Math.max(1, Math.round(aw * z)), hh = Math.max(1, Math.round(ah * z));
  r.setAttribute('width', w);
  r.setAttribute('height', hh);
  return { text: new XMLSerializer().serializeToString(d), w, h: hh };
}

// A file's pixels, {w, h, d: RGBA}: sniffed; an SVG made safe first; one
// way in for every kind (a blob URL, an image never in the page, its size
// checked before it is drawn, drawn 1:1, read, the URL let go).
export async function read(file) {
  if (file.size > 8 * MB) throw say('This picture is too big to read here: 8 MB at most');
  const b = new Uint8Array(await file.arrayBuffer()), k = sniff(b);
  if (!k) throw say('This file isn’t a picture OpenDMD can read: PNG, JPEG, GIF, WebP, BMP or SVG');
  let blob = new Blob([b], { type: 'image/' + k }), w = 0, hh = 0;
  if (k == 'svg') { const s = sanitize(new TextDecoder().decode(b)); blob = new Blob([s.text], { type: 'image/svg+xml' }); w = s.w; hh = s.h; }
  const u = URL.createObjectURL(blob), img = new Image();
  try {
    await new Promise((ok, bad) => {
      const t = setTimeout(() => bad(say('This picture took too long to read')), 5000);
      img.onload = () => { clearTimeout(t); ok(); };
      img.onerror = () => { clearTimeout(t); bad(say('This picture can’t be read: the file may be damaged')); };
      img.src = u;
    });
    w ||= img.naturalWidth;
    hh ||= img.naturalHeight;
    if (!w || !hh || w > SIDE || hh > SIDE) throw say(`This picture is too big: ${SIDE.toLocaleString('en')} pixels a side at most`);
    const g = h('canvas', { width: w, height: hh }).getContext('2d', { willReadFrequently: true });
    g.drawImage(img, 0, 0, w, hh);
    try { return { w, h: hh, d: g.getImageData(0, 0, w, hh).data }; } catch { throw say('This browser can’t read this SVG'); }
  } finally { URL.revokeObjectURL(u); }
}

// Pixels `s` ({w, h, d}) at [x, y, w, h] (source pixels, fractions too)
// into W × H, RGBA: each cell's area averaged, alpha premultiplied
// (`smooth`), else the source pixel at its centre.
export function resample(s, x, y, w, hh, W, H, smooth) {
  const o = new Uint8ClampedArray(W * H * 4), fx = w / W, fy = hh / H, d = s.d;
  for (let v = 0; v < H; v++) for (let u = 0; u < W; u++) {
    const q = (v * W + u) * 4;
    if (!smooth) { const p = (Math.min(s.h - 1, Math.floor(y + (v + 0.5) * fy)) * s.w + Math.min(s.w - 1, Math.floor(x + (u + 0.5) * fx))) * 4; o.set(d.subarray(p, p + 4), q); continue; }
    const x0 = x + u * fx, x1 = x0 + fx, y0 = y + v * fy, y1 = y0 + fy;
    let r = 0, g = 0, b = 0, a = 0, n = 0;
    for (let j = Math.floor(y0); j < y1 && j < s.h; j++) {
      const wy = Math.min(y1, j + 1) - Math.max(y0, j);
      for (let i = Math.floor(x0); i < x1 && i < s.w; i++) {
        const wt = wy * (Math.min(x1, i + 1) - Math.max(x0, i)), p = (j * s.w + i) * 4, al = d[p + 3] * wt;
        r += d[p] * al; g += d[p + 1] * al; b += d[p + 2] * al; a += al; n += wt;
      }
    }
    if (a) { o[q] = r / a; o[q + 1] = g / a; o[q + 2] = b / a; }
    o[q + 3] = n ? a / n : 0;
  }
  return o;
}

// The crop `c` ({x, y, w, h}) into W × H: `fill` (the crop's middle cut to
// the shape), `fit` (all of it, see-through round it) or stretched.
export function fitIn(s, c, W, H, fit, smooth) {
  let { x, y, w, h: hh } = c;
  if (fit == 'fill') { const k = W / H; if (w / hh > k) { x += (w - hh * k) / 2; w = hh * k; } else { y += (hh - w / k) / 2; hh = w / k; } }
  if (fit != 'fit') return resample(s, x, y, w, hh, W, H, smooth);
  const z = Math.min(W / w, H / hh), dw = Math.max(1, Math.min(W, Math.round(w * z))), dh = Math.max(1, Math.min(H, Math.round(hh * z))), ox = (W - dw) >> 1, oy = (H - dh) >> 1;
  const p = resample(s, x, y, w, hh, dw, dh, smooth), o = new Uint8ClampedArray(W * H * 4);
  for (let v = 0; v < dh; v++) o.set(p.subarray(v * dw * 4, (v + 1) * dw * 4), ((v + oy) * W + ox) * 4);
  return o;
}

// Each pixel's colour (0xRRGGBB), or -1 for see-through, by `o.bg`: keep
// (every pixel, over black), alpha (see-through under 50%), pick (and those
// within `tol`% of `bgc`), one (colour `one` where the brightness reaches
// `thr`%, the rest see-through).
export function opaque(p, o) {
  const out = [], bc = o.bgc ?? 0;
  for (let q = 0; q < p.length; q += 4) {
    const a = p[q + 3], k = o.bg == 'keep' ? a / 255 : 1, r = Math.round(p[q] * k), g = Math.round(p[q + 1] * k), b = Math.round(p[q + 2] * k);
    let c = r << 16 | g << 8 | b;
    if (o.bg != 'keep' && a < 128) c = -1;
    else if (o.bg == 'pick' && Math.max(Math.abs(r - (bc >> 16)), Math.abs(g - (bc >> 8 & 255)), Math.abs(b - (bc & 255))) <= o.tol * 2.55) c = -1;
    else if (o.bg == 'one') c = 0.2126 * r + 0.7152 * g + 0.0722 * b >= o.thr * 2.55 ? o.one : -1;
    out.push(c);
  }
  return out;
}

// Colour distance, weighted R 2, G 4, B 3; the nearest of `L` (a tie to the first).
const D = (a, b) => { const r = (a >> 16) - (b >> 16), g = (a >> 8 & 255) - (b >> 8 & 255), u = (a & 255) - (b & 255); return 2 * r * r + 4 * g * g + 3 * u * u; };
const nearI = (c, L) => { let k = 0; for (let i = 1; i < L.length; i++) if (D(c, L[i]) < D(c, L[k])) k = i; return k; };
// The colours of `px` and their counts, [[c, n]]: the most first, a tie by RGB.
const census = (px) => { const m = new Map(); for (const c of px) if (c >= 0) m.set(c, (m.get(c) || 0) + 1); return [...m].sort((a, b) => b[1] - a[1] || a[0] - b[0]); };
const mean = (b) => { let n = 0, r = 0, g = 0, u = 0; for (const [c, m] of b) { n += m; r += (c >> 16) * m; g += (c >> 8 & 255) * m; u += (c & 255) * m; } return Math.round(r / n) << 16 | Math.round(g / n) << 8 | Math.round(u / n); };

// Each colour of `px` to the one it becomes, `N` at most: as they are when
// they fit; else a median cut (the box with the largest spread × count
// split at its weighted median along its widest weighted channel, a tie by
// RGB) and at most 8 Lloyd passes. With a palette `pal`: each to the nearest
// of it, the `N` used most kept when more are (a tie by RGB).
export function quantise(px, N, pal) {
  const cs = census(px);
  if (pal) {
    const to = (L) => new Map(cs.map(([c]) => [c, L[nearI(c, L)]]));
    let m = to(pal);
    const used = census(px.map((c) => (c < 0 ? c : m.get(c))));
    if (used.length > N) m = to(used.slice(0, N).map((x) => x[0]));
    return m;
  }
  if (cs.length <= N) return new Map(cs.map(([c]) => [c, c]));
  const ch = (c, k) => c >> (16 - 8 * k) & 255, wk = [2, 4, 3];
  const boxes = [cs];
  while (boxes.length < N) {
    let bi = -1, bs = -1, ax = 0;
    boxes.forEach((b, i) => {
      if (b.length < 2) return;
      let n = 0, s = -1, a = 0;
      for (const x of b) n += x[1];
      for (let k = 0; k < 3; k++) {
        let lo = 255, hi = 0;
        for (const [c] of b) { const v = ch(c, k); if (v < lo) lo = v; if (v > hi) hi = v; }
        if ((hi - lo) ** 2 * wk[k] > s) { s = (hi - lo) ** 2 * wk[k]; a = k; }
      }
      if (s * n > bs) { bs = s * n; bi = i; ax = a; }
    });
    if (bi < 0) break;
    const b = boxes[bi].slice().sort((p, q) => ch(p[0], ax) - ch(q[0], ax) || p[0] - q[0]);
    let t = 0, acc = 0, k = 0;
    for (const x of b) t += x[1];
    while (k < b.length - 1 && acc * 2 < t) acc += b[k++][1];
    boxes.splice(bi, 1, b.slice(0, Math.max(1, k)), b.slice(Math.max(1, k)));
  }
  let L = boxes.map(mean);
  for (let it = 0; it < 8; it++) {
    const g = L.map(() => []);
    for (const x of cs) g[nearI(x[0], L)].push(x);
    const M = g.map((b, i) => (b.length ? mean(b) : L[i]));
    if (M.every((c, i) => c === L[i])) break;
    L = M;
  }
  return new Map(cs.map(([c]) => [c, L[nearI(c, L)]]));
}

// A colour's initial for its key character: K black, W white, R O Y G C B P by hue ('' for a grey).
function initial(c) {
  const r = c >> 16, g = c >> 8 & 255, b = c & 255, mx = Math.max(r, g, b), d = mx - Math.min(r, g, b);
  if (mx < 48) return 'K';
  if (d < 40) return mx > 200 && d < 24 ? 'W' : '';
  const u = 60 * (mx == r ? ((g - b) / d + 6) % 6 : mx == g ? (b - r) / d + 2 : (r - g) / d + 4);
  return u < 15 || u >= 330 ? 'R' : u < 45 ? 'O' : u < 70 ? 'Y' : u < 165 ? 'G' : u < 200 ? 'C' : u < 260 ? 'B' : 'P';
}
// How near an old key colour may be to be kept for a new one (its character kept).
const NEAR = 9 * 24 * 24;

// The key ([{ch, v}]) and rows for pixels `px` (`W` across) at tier `K`:
// see-through `.` 0 first, there when used or when it doesn't change the
// tier; then the colours, the most used first (a tie by RGB), each at its
// initial when free, else the next free of ABC. Over a sprite's key `old`:
// a colour the key has keeps its character and how it is written; another
// takes the nearest unused one near it (its character kept, the colour
// new); the rest are new; the old ones keep their order, the new follow,
// the unused go.
export function keyed(px, W, K, old = []) {
  const cs = census(px).map((x) => x[0]), see = px.includes(-1) || cs.length < K;
  const ot = old.find((e) => !HEX.test(e.v) && +e.v === 0), oc = old.filter((e) => HEX.test(e.v)).map((e) => ({ e, c: rgbOf(e.v) }));
  const chOf = new Map(), vOf = new Map(), kept = new Set(), taken = new Set();
  const keep = (c, x, v) => { kept.add(x.e); chOf.set(c, x.e.ch); vOf.set(x.e, v); taken.add(x.e.ch); };
  for (const c of cs) { const x = oc.find((x) => !kept.has(x.e) && x.c === c); if (x) keep(c, x, x.e.v); }
  for (const c of cs) {
    if (chOf.has(c)) continue;
    let b = null;
    for (const x of oc) if (!kept.has(x.e) && D(c, x.c) <= NEAR && (!b || D(c, x.c) < D(c, b.c))) b = x;
    if (b) keep(c, b, as(hx(c), b.e.v));
  }
  if (see && ot) taken.add(ot.ch);
  const fresh = [], free = (x) => (x && !taken.has(x) ? x : [...ABC].find((q) => !taken.has(q)));
  let t = see && ot ? ot.ch : null;
  if (see && !ot) { t = free('.'); taken.add(t); fresh.push({ ch: t, v: '0' }); }
  for (const c of cs) if (!chOf.has(c)) { const q = free(initial(c)); taken.add(q); chOf.set(c, q); fresh.push({ ch: q, v: hx(c) }); }
  const ents = [...old.filter((e) => (e === ot ? see : kept.has(e))).map((e) => ({ ch: e.ch, v: e === ot ? e.v : vOf.get(e) })), ...fresh], rows = [];
  for (let k = 0; k < px.length; k += W) rows.push(px.slice(k, k + W).map((c) => (c < 0 ? t : chOf.get(c))).join(''));
  return { ents, rows };
}

// A sprite's text; what one costs the store: its head, its key, each row in
// numbers of 32, 16 or 8 characters (by the key's size, see-through counted).
const inner = (ents, rows, i2) => `${i2}key ${ents.map((e) => e.ch + ' ' + e.v).join(', ')}\n${rows.map((r) => i2 + r).join('\n')}\n`;
export const block = (name, ents, rows, ind = '', i2 = ind + '  ') => `${ind}sprite ${name} {\n${inner(ents, rows, i2)}${ind}}`;
export const cost = (K, w, hh) => 1 + K + hh * Math.ceil(w / (K <= 2 ? 32 : K <= 4 ? 16 : 8));

// The picture `s` as a sprite by the settings `o` ({crop, W, H, fit, samp,
// bg…, tier (0: the fewest that hold its colours), pal (colours) or none}),
// over `old`'s key: {K, ents, rows}.
export function make(s, o, old) {
  let px = opaque(fitIn(s, o.crop, o.W, o.H, o.fit, o.samp != 'sharp'), o);
  const n = new Set(px.filter((c) => c >= 0)).size, see = +px.includes(-1);
  const K = o.tier || TIERS.find((k) => k >= n + see) || 16, m = quantise(px, Math.max(1, K - see), o.pal);
  px = px.map((c) => (c < 0 ? c : m.get(c)));
  return { K, ...keyed(px, o.W, K, old) };
}

// --- The dialog.

const CSS = `.menu.psi-imp{width:min(820px,calc(100vw - 16px))}.menu.psi-imp.sheet{width:auto}.psi-imp-c{display:grid;grid-template-columns:minmax(0,1fr) minmax(0,1fr);gap:16px}.sheet .psi-imp-c{grid-template-columns:minmax(0,1fr)}.psi-imp label{display:inline-flex;align-items:center;gap:4px}.psi-imp input[type=number]{width:60px}.psi-imp .row{flex-wrap:wrap}
#psi-imp-src{display:block;touch-action:none;cursor:crosshair;background:#0a0b0c;border-radius:4px;max-width:100%}#psi-imp-src:focus-visible{outline:2px solid var(--acc)}
#psi-imp-grid{display:block;max-width:100%}.psi-imp .psi-chk{border-radius:4px;overflow:auto;max-height:40vh}#psi-imp-pv{display:block;width:384px;max-width:100%;image-rendering:pixelated;background:#000;border-radius:4px}
#psi-imp-cost{flex-basis:100%;margin:0}#psi-imp-tier button{min-height:40px}#psi-imp-tier button[aria-pressed=true]{outline:3px solid var(--acc);outline-offset:1px}@media (pointer:coarse){.psi-imp select,.psi-imp input,.psi-imp button{min-height:44px}}`;
let styled = false;
const SIZES = { '8x8': [8, 8], '16x16': [16, 16], '24x24': [24, 24], '32x32': [32, 32], '64x32': [64, 32] };
const clamp = (v, a, b) => Math.max(a, Math.min(b, Math.round(+v) || a));
const lineEnd = (s, p) => { const n = s.indexOf('\n', p); return n < 0 ? s.length : n; };
// Where `s` and `t` differ: [start, end in s, the text from t], or null.
function diff(s, t) {
  if (s === t) return null;
  let a = 0, e = 0;
  while (a < s.length && a < t.length && s[a] === t[a]) a++;
  while (e < s.length - a && e < t.length - a && s[s.length - 1 - e] === t[t.length - 1 - e]) e++;
  return [a, s.length - e, t.slice(a, t.length - e)];
}
// Where a new sprite goes (at the top level, E911): after the last sprite,
// table or palette, else after the header lines, else first. [at, text before, after].
function place(s) {
  let end = -1;
  for (const o of [...sprites(s), ...palettes(s)]) end = Math.max(end, o.e);
  for (const m of s.matchAll(/^table[ \t]+\w+[ \t]*=/gm)) { let p = m.index; while (/,[ \t]*(?:(--|\/\/).*)?$/.test(s.slice(p, lineEnd(s, p))) && lineEnd(s, p) < s.length) p = lineEnd(s, p) + 1; end = Math.max(end, lineEnd(s, p)); }
  if (end >= 0) return [lineEnd(s, end), '\n', ''];
  let at = -1;
  for (let p = 0; p < s.length; p = lineEnd(s, p) + 1) {
    const l = s.slice(p, lineEnd(s, p)).trim();
    if (/^(psi|name|about|control|look|loop|fps|smooth)\b/.test(l)) at = lineEnd(s, p);
    else if (l && !/^(--|\/\/)/.test(l)) break;
  }
  return at < 0 ? [0, '', '\n'] : [at, '\n', ''];
}

// The dialog for `file` over the editor (`kit`: {ta, put, peek, still(src,
// pic): a scratch formula's info() and (`pic`) its first frame, or its
// error; name: the sprite to replace, else the caret's, if any}).
export async function open(file, kit) {
  let s;
  try { s = await read(file); } catch (e) { toast(e.said ? e.message : 'This picture can’t be read here', 'bad'); return; }
  if (!styled) { document.head.append(h('style', CSS)); styled = true; }
  // A working copy of 512 a side at most (an area average: a crop moves at once).
  if (s.w > WORK || s.h > WORK) { const z = Math.min(WORK / s.w, WORK / s.h), w = Math.max(1, Math.round(s.w * z)), hh = Math.max(1, Math.round(s.h * z)); s = { w, h: hh, d: resample(s, 0, 0, s.w, s.h, w, hh, true) }; }
  const { ta } = kit, src0 = ta.value, p0 = ta.selectionStart;
  const target = kit.name ? sprites(src0).find((o) => o.name == kit.name) : sprites(src0).find((o) => o.ok && o.s <= p0 && p0 <= o.e);
  const R = store.get('psi.import', {}), o = { size: '32x32', W: 32, H: 32, lock: true, fit: 'fit', samp: 'smooth', tol: 10, thr: 50, one: 0xffffff, ...R, crop: { x: 0, y: 0, w: s.w, h: s.h }, tier: 0, pal: null };
  if (s.w <= 64 && s.h <= 32) o.size = 'as';
  let alpha = false;
  for (let q = 3; q < s.d.length; q += 4) if (s.d[q] < 255) { alpha = true; break; }
  o.bg = alpha ? 'alpha' : 'keep';
  o.bgc = s.d[0] << 16 | s.d[1] << 8 | s.d[2];
  const pals = palettes(src0).filter((p) => !p.bad);
  let rep = !!target, added = false, cand = null, res = null, raf = 0;
  const taken = (n) => new RegExp(`\\b${n}\\b`).test(src0);
  let name = 'pic';
  for (let k = 2; taken(name); k++) name = 'pic' + k;

  const sel = (id, label, opts, v, fn) => h('label', label, h('select', { id, onchange: (e) => { fn(e.target.value); run(); } }, opts.map(([k, t]) => h('option', { value: k, selected: k == v }, t))));
  const num = (id, label, max, k) => h('input', { type: 'number', id, inputMode: 'numeric', min: 1, max, value: o[k], 'aria-label': label, onchange: (e) => sized(k, e.target.value) });
  const cv = h('canvas', { id: 'psi-imp-src', tabIndex: 0, 'aria-label': 'The picture: drag to crop it; arrow keys move the crop, Alt and arrows change its size' });
  const wI = num('psi-imp-w', 'Width', 64, 'W'), hI = num('psi-imp-h', 'Height', 32, 'H');
  const sizeS = h('select', { id: 'psi-imp-size', 'aria-label': 'Size', onchange: () => { o.size = sizeS.value; run(); } }, [['as', 'As it is'], ...Object.keys(SIZES).map((k) => [k, k.replace('x', ' × ')]), ['own', 'Your size']].map(([k, t]) => h('option', { value: k }, t)));
  const lock = h('input', { type: 'checkbox', id: 'psi-imp-lock', checked: o.lock, onchange: () => { o.lock = lock.checked; } });
  const tiers = h('div.row', { id: 'psi-imp-tier', role: 'group', 'aria-label': 'Colours' });
  const bgx = h('span.row');
  const nameI = h('input', { type: 'text', id: 'psi-imp-name', value: name, maxLength: 24, 'aria-label': 'Name', spellcheck: false, oninput: () => run() });
  const repC = target && h('label', h('input', { type: 'checkbox', id: 'psi-imp-rep', checked: true, onchange: (e) => { rep = e.target.checked; nameI.value = rep ? target.name : name; run(); } }), `Replace ${target.name}`);
  const drawC = h('input', { type: 'checkbox', id: 'psi-imp-draw', checked: true, onchange: () => run() }), drawL = h('label', drawC, 'Draw it in the middle');
  const call = h('p.cap', { id: 'psi-imp-call' }), note = h('p.cap', { id: 'psi-imp-note', hidden: true }, 'The key holds the colours themselves: changing the palette later won’t change the sprite.');
  const grid = h('canvas', { id: 'psi-imp-grid' }), pv = h('canvas', { id: 'psi-imp-pv', width: 128, height: 32, 'aria-label': 'On the panel' });
  const costL = h('p.cap', { id: 'psi-imp-cost', 'aria-live': 'polite' });
  const go = h('button.pri', { id: 'psi-imp-go', onclick: () => add() });
  const el = h('div.menu.wide.psi-imp', { id: 'psi-imp', role: 'dialog', 'aria-label': 'A sprite from a picture' },
    h('div.mh', 'A sprite from a picture'), h('p.cap', 'Use pictures you made or may share.'),
    h('div.psi-imp-c', h('div', cv, h('div.row', h('button.sm', { id: 'psi-imp-all', onclick: () => { o.crop = { x: 0, y: 0, w: s.w, h: s.h }; run(); } }, 'All of it'), h('span.cap', { id: 'psi-imp-crop' })),
    h('div.row', h('label', 'Size', sizeS), wI, '×', hI, h('label', lock, 'Keep its shape')),
    h('div.row', sel('psi-imp-fit', 'Fit', [['fill', 'Crop to fill'], ['fit', 'Fit inside'], ['stretch', 'Stretch']], o.fit, (v) => { o.fit = v; }),
      sel('psi-imp-samp', 'Sampling', [['smooth', 'Smooth'], ['sharp', 'Sharp']], o.samp, (v) => { o.samp = v; })),
    h('div.row', h('span', 'Colours'), tiers),
    h('div.row', sel('psi-imp-pal', 'Palette', [['auto', 'Its own colours'], ['den', 'Panel colours'], ...pals.map((p) => ['p:' + p.name, 'The palette ' + p.name])], 'auto', (v) => { o.pal = v == 'auto' ? null : v == 'den' ? DEN : pals.find((p) => 'p:' + p.name == v).ents.map((x) => rgbOf(x.hex)); note.hidden = v[0] != 'p'; }),
      sel('psi-imp-bg', 'Background', [['keep', 'Keep it'], ['alpha', 'See-through where it is'], ['pick', 'See-through: a colour'], ['one', 'One colour']], o.bg, (v) => { o.bg = v; })), bgx, note,
    h('div.row', h('label', 'Name', nameI), repC, drawL), call),
    h('div', h('div.psi-chk', grid), h('p.cap', 'On the panel'), pv)),
    // (The cost with the buttons, in sight however far the sheet is scrolled.)
    h('div.row.acts', costL, h('button', { id: 'psi-imp-cancel', onclick: closeMenu }, 'Cancel'), go));
  // Enter: the dialog's button (Escape: closeMenu, the editor's text as it was).
  el.addEventListener('keydown', (e) => { if (e.key == 'Enter' && e.target.localName != 'button' && e.target.localName != 'select' && !go.disabled) { e.preventDefault(); add(); } });

  // The size: a preset, as it is (64 × 32 at most, its shape kept), or typed (the other side following its shape).
  function sized(k, v) {
    o[k] = clamp(v, 1, k == 'W' ? 64 : 32);
    if (o.lock) { const a = o.crop.w / o.crop.h; if (k == 'W') o.H = clamp(o.W / a, 1, 32); else o.W = clamp(o.H * a, 1, 64); }
    o.size = 'own';
    run();
  }
  function dims() {
    const c = o.crop;
    if (o.size == 'as') { const z = Math.min(1, 64 / c.w, 32 / c.h); [o.W, o.H] = [clamp(c.w * z, 1, 64), clamp(c.h * z, 1, 32)]; } else if (SIZES[o.size]) [o.W, o.H] = SIZES[o.size];
    sizeS.value = o.size;
    wI.value = o.W;
    hI.value = o.H;
  }
  function bgRow() {
    const col = (k, l) => h('input', { type: 'color', id: 'psi-imp-' + k, 'aria-label': l, value: '#' + (o[k] ?? 0).toString(16).padStart(6, '0'), onchange: (e) => { o[k] = rgbOf(e.target.value); run(); } });
    const rng = (k, l) => h('label', l, h('input', { type: 'range', id: 'psi-imp-' + k, min: 0, max: 100, value: o[k], oninput: (e) => { o[k] = +e.target.value; run(); } }));
    clear(bgx, o.bg == 'pick' ? [col('bgc', 'The background colour'), rng('tol', 'Near it')] : o.bg == 'one' ? [col('one', 'The colour'), rng('thr', 'Brighter than')] : null);
  }
  let bgWas = null;

  // The crop on the picture: dragged inside it, by a corner (44 px for a
  // finger), or drawn anew; arrow keys move it (Shift: by 10), Alt and arrows size it.
  let z = 1, drag = null;
  function paintSrc() {
    z = Math.min(360 / s.w, 200 / s.h, Math.max(1, (innerWidth - 48) / s.w));
    const dp = devicePixelRatio || 1, W = Math.round(s.w * z), H = Math.round(s.h * z), g = cv.getContext('2d'), c = o.crop;
    cv.width = W * dp; cv.height = H * dp; cv.style.width = W + 'px'; cv.style.height = H + 'px';
    g.scale(dp, dp);
    const b = h('canvas', { width: s.w, height: s.h });
    b.getContext('2d').putImageData(new ImageData(new Uint8ClampedArray(s.d), s.w, s.h), 0, 0);
    g.imageSmoothingEnabled = z < 1;
    g.drawImage(b, 0, 0, W, H);
    g.fillStyle = '#000a';
    g.fillRect(0, 0, W, c.y * z); g.fillRect(0, (c.y + c.h) * z, W, H); g.fillRect(0, c.y * z, c.x * z, c.h * z); g.fillRect((c.x + c.w) * z, c.y * z, W, c.h * z);
    g.strokeStyle = '#fff'; g.lineWidth = 2; g.strokeRect(c.x * z, c.y * z, c.w * z, c.h * z);
    g.fillStyle = '#fff';
    for (const [x, y] of corners()) g.fillRect(x - 5, y - 5, 10, 10);
    el.querySelector('#psi-imp-crop').textContent = `${c.w} × ${c.h} of ${s.w} × ${s.h}`;
  }
  const corners = () => { const c = o.crop; return [[c.x, c.y], [c.x + c.w, c.y], [c.x, c.y + c.h], [c.x + c.w, c.y + c.h]].map(([x, y]) => [x * z, y * z]); };
  const at = (e) => { const r = cv.getBoundingClientRect(); return [(e.clientX - r.left), (e.clientY - r.top)]; };
  function setCrop(x0, y0, x1, y1) {
    const a = (v, m) => Math.max(0, Math.min(m, Math.round(v)));
    let [l, r] = [a(Math.min(x0, x1), s.w), a(Math.max(x0, x1), s.w)], [t, b] = [a(Math.min(y0, y1), s.h), a(Math.max(y0, y1), s.h)];
    if (r - l < 1) { if (r < s.w) r = l + 1; else l = r - 1; }
    if (b - t < 1) { if (b < s.h) b = t + 1; else t = b - 1; }
    o.crop = { x: l, y: t, w: r - l, h: b - t };
  }
  cv.onpointerdown = (e) => {
    if (e.button) return;
    e.preventDefault();
    cv.setPointerCapture(e.pointerId);
    const [x, y] = at(e), c = { ...o.crop }, k = corners().findIndex(([u, v]) => Math.abs(u - x) <= 22 && Math.abs(v - y) <= 22);
    const inside = x >= c.x * z && x <= (c.x + c.w) * z && y >= c.y * z && y <= (c.y + c.h) * z;
    drag = { id: e.pointerId, x, y, c, k, mode: k >= 0 ? 'corner' : inside ? 'move' : 'new' };
  };
  cv.onpointermove = (e) => {
    if (drag?.id !== e.pointerId) return;
    const [x, y] = at(e), c = drag.c, dx = (x - drag.x) / z, dy = (y - drag.y) / z;
    if (drag.mode == 'move') { const nx = Math.max(0, Math.min(s.w - c.w, c.x + dx)), ny = Math.max(0, Math.min(s.h - c.h, c.y + dy)); setCrop(nx, ny, nx + c.w, ny + c.h); } else if (drag.mode == 'new') setCrop(drag.x / z, drag.y / z, x / z, y / z);
    else { const k = drag.k, fx = k & 1 ? c.x : c.x + c.w, fy = k & 2 ? c.y : c.y + c.h; setCrop(fx, fy, (k & 1 ? c.x + c.w : c.x) + dx, (k & 2 ? c.y + c.h : c.y) + dy); }
    run();
  };
  cv.onpointerup = cv.onpointercancel = (e) => { if (drag?.id === e.pointerId) drag = null; };
  cv.onkeydown = (e) => {
    const d = { ArrowLeft: [-1, 0], ArrowRight: [1, 0], ArrowUp: [0, -1], ArrowDown: [0, 1] }[e.key];
    if (!d) return;
    e.preventDefault();
    const c = o.crop, k = e.shiftKey ? 10 : 1, dx = d[0] * k, dy = d[1] * k;
    if (e.altKey) setCrop(c.x, c.y, c.x + c.w + dx, c.y + c.h + dy);
    else { const nx = Math.max(0, Math.min(s.w - c.w, c.x + dx)), ny = Math.max(0, Math.min(s.h - c.h, c.y + dy)); setCrop(nx, ny, nx + c.w, ny + c.h); }
    run();
  };

  // The sprite, the Formula with it, what it costs: each change, once a frame.
  const run = () => { raf ||= requestAnimationFrame(update); };
  let errAt = '';
  function update() {
    raf = 0;
    dims();
    if (bgWas !== o.bg) { bgWas = o.bg; bgRow(); }
    paintSrc();
    store.set('psi.import', { size: o.size == 'as' ? R.size : o.size, W: o.W, H: o.H, lock: o.lock, fit: o.fit, samp: o.samp, tol: o.tol, thr: o.thr, one: o.one });
    const on = rep && target;
    res = make(s, o, on ? target.key.ents : []);
    const K = res.ents.length;
    clear(tiers, TIERS.map((k) => h('button.sm', { type: 'button', 'data-k': k, 'aria-pressed': String(k == res.K), onclick: () => { o.tier = k; run(); } }, `${k} colours · ${cost(k, o.W, o.H)}`)));
    // The grid, on a checker where it is see-through.
    const cs = Math.max(3, Math.min(16, Math.floor(384 / o.W), Math.floor(240 / o.H))), dp = devicePixelRatio || 1, g = grid.getContext('2d'), col = new Map(res.ents.map((e) => [e.ch, HEX.test(e.v) ? six(e.v) : null]));
    grid.width = o.W * cs * dp; grid.height = o.H * cs * dp; grid.style.width = o.W * cs + 'px'; grid.style.height = o.H * cs + 'px';
    g.scale(dp, dp);
    res.rows.forEach((r, v) => [...r].forEach((ch, u) => { const c = col.get(ch); if (c) { g.fillStyle = c; g.fillRect(u * cs, v * cs, cs, cs); } }));
    grid.dataset.rows = res.rows.join('\n');
    // The new name: free and a name; replacing, the sprite's own.
    const nm = on ? target.name : nameI.value.trim();
    nameI.disabled = !!on;
    if (on) nameI.value = target.name;
    const x = (128 - o.W) >> 1, y = (32 - o.H) >> 1, cur = ta.value, hasDraw = /^[ \t]*draw[ \t]*\{/m.test(cur);
    drawL.hidden = !!on || hasDraw;
    call.textContent = on ? '' : `Draw it with spr(${nm}, ${x}, ${y}) in draw { }` + (hasDraw ? '' : drawC.checked ? ': it goes in with the sprite.' : '.');
    const blk = (n) => block(n, res.ents, res.rows);
    // Replacing: its lines between its head and its `}`, at their indent (its name, place and those lines kept).
    if (on) {
      const t = sprites(cur).find((q) => q.name == target.name);
      cand = t && cur.slice(0, cur.indexOf('\n', t.s) + 1) + inner(res.ents, res.rows, t.ind) + cur.slice(cur.lastIndexOf('\n', t.e - 1) + 1);
    } else {
      const [p, pre, post] = place(cur);
      cand = cur.slice(0, p) + pre + blk(nm) + (!hasDraw && drawC.checked ? `\ndraw { spr(${nm}, ${x}, ${y}) }` : '') + post + cur.slice(p);
    }
    const sc = still(blk('pic'), x, y), c = cand && kit.still(cand);
    if (sc?.rgb) { const im = new ImageData(128, 32); for (let p = 0, q = 0; p < 12288; p += 3, q += 4) im.data.set([sc.rgb[p], sc.rgb[p + 1], sc.rgb[p + 2], 255], q); pv.getContext('2d').putImageData(im, 0, 0); }
    const pic = `This picture: ${cost(K, o.W, o.H).toLocaleString('en')} numbers.`;
    const bad = !on && !/^[A-Za-z_]\w*$/.test(nm) ? 'A name starts with a letter, then letters, digits or _.' : c?.err && `It can’t go in: ${c.err}`;
    go.textContent = on ? `Replace ${target.name}` : 'Add the sprite';
    go.disabled = !!bad || !c;
    costL.classList.toggle('bad', !!bad);
    if (bad) {
      costL.textContent = `${pic} ${bad}`;
      const m = bad;
      if (c?.err && errAt !== m) { errAt = m; import('./psierr.js').then((P) => { const k = P.codeOf(m); if (k && costL.textContent.endsWith(m)) costL.append(` (${P.label(k)})`); }, () => {}); }
    } else {
      errAt = '';
      costL.textContent = `${pic} The Formula would use ${c.i[19].toLocaleString('en')} of 1,024 · ${c.i[20]} of 127 colours`;
    }
    kit.peek(bad ? undefined : cand);
  }
  // The sprite alone on the panel: drawn at its place, black round it.
  const still = (b, x, y) => kit.still(`${b}\ndraw { spr(pic, ${x}, ${y}) }\ngray(0)`, true);
  function add() {
    if (go.disabled || !cand) return;
    const d = diff(ta.value, cand);
    added = true;
    closeMenu();
    if (d) kit.put({ s: d[0], e: d[1] }, d[2]);
  }
  el.onclose = () => { cancelAnimationFrame(raf); if (!added) kit.peek(); };
  const r = (kit.at || ta).getBoundingClientRect();
  update();
  popup(el, r.left, r.bottom + 4);
  update();
}
