// The Studio (/studio/): draw pixel art for the panel, 128 x 32, keep it as
// a draft in this browser and download it as a GIF. Client-only: nothing is
// uploaded; the page fetches nothing but its own files and, at the first
// download, the demo's art module (its GIF writer).
//
// The piece itself, with no DOM, is doc.js: its layers and frames, what is
// shown, its colours and the undo steps; this file draws it and keeps it.
// A piece of any number of layers and frames opens and shows as it is; the
// controls draw on one cel (the top layer, the first frame) until the frame
// strip and the layer rows come (frames.js). Tools (tools.js): pencil,
// eraser, line, rectangle and ellipse (edge or filled), fill, a shade for
// fills and filled shapes (solid, light, half, heavy), a selection to move,
// copy, flip and delete, a colour picker tool; brushes 1 to 3; each shape
// seen as it is dragged and kept when the pointer lifts. Colours: the
// piece's own (body.colours, any 24-bit colour; past 255 the pixels take two
// bytes), drawn from swatch sets (Den 16, the starter palettes, the user's
// own). Undo (doc.js: 500 steps) is a journal kept in the store, so it
// survives a reload; snapshots are named revisions. The picker and the
// palette tools load when first opened (colour.js), the GIF at the first
// download (out.js). Import (import.js: Create Art's converter in a sheet,
// for pictures, GIFs and videos from anywhere) adds a piece through
// `piece()` below, the one way in, or a layer to this one; a pasted picture
// (paste.js) floats in exactly. A drop routes by what it is: a .dmdart or
// .dmdpack to files.js, a picture, GIF or video (or a link) to Import.
import { LocalStore, Conflict, CAPS, record, newId, packPx, check } from './store.js';
import { DEFAULT, builtin, ready, starter, isStarter } from './palettes.js';
import { Doc, W, H, N, hexOf, toInt } from './doc.js';
import * as T from './tools.js';

const ZOOMS = [2, 3, 4, 6, 8, 12, 16, 24, 32];
const LS = 'odmd.studio.';
// A still's time, as the web app's (opendmd-art-web pipeline.rs STILL_MS).
const STILL_MS = 5000;
const AMBER = 0xf7a531; // Den 16's Amber, the bright one: a new piece's colour
// localStorage may be refused (a private window, a full disk): settings are then forgotten.
const ls = {
  get: (k) => { try { return localStorage.getItem(LS + k); } catch { return null; } },
  set: (k, v) => { try { localStorage.setItem(LS + k, v); } catch { /* not kept */ } },
};
const $ = (id) => document.getElementById(id);
const el = (tag, cls, text) => { const e = document.createElement(tag); if (cls) e.className = cls; if (text !== undefined) e.textContent = text; return e; };
const root = $('studio'), cv = $('s-canvas'), ctx = cv.getContext('2d'), work = $('s-work'), scroll = $('s-scroll');
const coarse = matchMedia('(pointer: coarse)').matches;

let store = null;
let rec = null;            // the piece's record (its pixels packed only when saved)
let D = new Doc({ w: W, h: H, frames: [{ ms: STILL_MS }], layers: [{ name: '', moves: false, cels: [''] }] }); // the piece (doc.js)
D.layers[0].cels[0].px = new Uint16Array(N);
let swatch = builtin(DEFAULT); // the swatch set shown (body.palette)
const mine = new Map();    // the user's own palettes, by id
let starters = [];         // the starter palettes, once fetched
let tool = 'pencil', prev = 'pencil', size = 1, zoom = 0, ledView = false, shade = 'solid', busy = false;
const filled = { rect: false, ellipse: false };
let sel = null, clip = null, drag = null;
let clipNew = false;       // the Studio's copy is the newest on the clipboard: made since this window last lost focus
let lastHash = null, timer = 0, nameDirty = false;
let kept = false;          // in the store (a new piece is not, until it is drawn on or named)
let journaled = false, unlock = null, closed = '', persisted = null;
let chain = Promise.resolve(), queued = null;

// Pixel icons (12 x 12), drawn as SVG.
const ICONS = {
  stamp: '............|....####....|....####....|.....##.....|.....##.....|..########..|.##########.|.##########.|............|.##########.|............|............',
  pencil: '.........##.|........#..#|.......#..#.|......#..#..|.....#..#...|....#..#....|...#..#.....|..#..#......|.#..#.......|.###........|.##.........|............',
  eraser: '............|......####..|.....#...##.|....#...#.#.|...#...#..#.|..#...#..#..|.#####..#...|.#...#.#....|.#...##.....|.#####......|............|............',
  fill: '.....#......|....#.#.....|...#...#....|..#.....#...|.#########..|.#########.#|..#######..#|...#####..##|....###...##|.....#.....#|............|............',
  line: '............|..........#.|.........#..|........#...|.......#....|......#.....|.....#......|....#.......|...#........|..#.........|.#..........|............',
  rect: '............|............|.##########.|.#........#.|.#........#.|.#........#.|.#........#.|.#........#.|.#........#.|.##########.|............|............',
  rectf: '............|............|.##########.|.##########.|.##########.|.##########.|.##########.|.##########.|.##########.|.##########.|............|............',
  ellipse: '............|............|....####....|..##....##..|.#........#.|.#........#.|.#........#.|.#........#.|..##....##..|....####....|............|............',
  ellipsef: '............|............|....####....|..########..|.##########.|.##########.|.##########.|.##########.|..########..|....####....|............|............',
  select: '............|.##.##.##.#.|.#..........|............|.#........#.|.#........#.|............|.#........#.|..........#.|.#.##.##.##.|............|............',
  pick: '.........##.|........####|.......#####|......#####.|.....#.##...|....#...#...|...#...#....|..#...#.....|.#...#......|.#..#.......|..##........|............',
  undo: '............|....#.......|...##.......|..#######...|...##....#..|....#.....#.|..........#.|.........#..|....#####...|............|............|............',
  redo: '............|.......#....|.......##...|...#######..|..#....##...|.#.....#....|.#..........|..#.........|...#####....|............|............|............',
};
function icon(e, name) {
  let s = '';
  ICONS[name].split('|').forEach((r, y) => [...r].forEach((c, x) => { if (c === '#') s += `<rect x="${x}" y="${y}" width="1" height="1"/>`; }));
  e.querySelector('svg')?.remove();
  e.insertAdjacentHTML('afterbegin', `<svg viewBox="0 0 12 12" width="24" height="24" shape-rendering="crispEdges" fill="currentColor" aria-hidden="true">${s}</svg>`);
}

// --- colours: the piece's own (doc.js) ---
const listOf = (body) => body.colours || (builtin(body.palette) || mine.get(body.palette) || builtin(DEFAULT)).body.colours;
const capSaid = () => say(`A piece holds ${CAPS.colours} colours at most`);
function keyFor(c) { const k = D.keyFor(c); if (!k) capSaid(); return k; }
function pickColour(c) {
  const k = D.pickColour(c);
  if (k) { swatches(); pick(k); } else capSaid();
}
function pick(k) {
  const key = D.pick(k);
  for (const b of $('s-sw').children) {
    const on = +b.dataset.k === key;
    b.classList.toggle('on', on);
    b.setAttribute('aria-checked', on);
  }
  root.dataset.key = key;
  root.dataset.colour = key ? hexOf(D.rgbOf(key)) : 'off';
}
function swatch1(k, hex, name) {
  const b = el('button', 's-sw' + (k === 0 ? ' off' : ''));
  b.style.background = hex;
  if (k !== null) { b.dataset.k = k; b.setAttribute('role', 'radio'); } else b.dataset.hex = hex;
  b.title = k === 0 ? 'Off: the LED dark' : `${name ? name + ' · ' : ''}${hex}`;
  b.setAttribute('aria-label', k === 0 ? 'Off' : name || hex);
  return b;
}
// The piece's colours, off first; then the one picked, if it is not drawn yet.
function swatches() {
  const sw = $('s-sw'), list = D.colourList(), key = D.key, more = key && D.pending.has(key) ? [swatch1(key, hexOf(D.pending.get(key)))] : [];
  for (const b of more) { b.classList.add('new'); b.title += ' · joins this piece when you draw with it'; }
  sw.replaceChildren(swatch1(0, '#000000'), ...list.map((c) => swatch1(c.k, c.hex)), ...more);
  sw.classList.toggle('many', list.length > 23);
  root.dataset.pending = D.pending.size; // colours picked or pasted, not drawn with yet
  pick(key);
}
// The swatch set shown, and what it is.
// A palette that is not one (no colours) is never taken in: `swatch` moves only once it has shown.
function useSwatches(p) {
  const set = $('s-set');
  set.replaceChildren(...p.body.colours.map((c) => swatch1(null, c.hex, c.name)));
  swatch = p;
  set.classList.toggle('many', p.body.colours.length > 24);
  $('s-pal').value = p.id;
  $('s-pal-about').textContent = `${p.body.colours.length} colours · ${builtin(p.id) ? `${p.body.source || 'OpenDMD'} · read-only` : 'yours'}`;
  root.dataset.swatches = p.id;
}
function menu() {
  const m = $('s-pal');
  m.replaceChildren(new Option(builtin(DEFAULT).name, DEFAULT));
  for (const [label, list] of [['Starter', starters], ['Yours', [...mine.values()]]]) {
    if (!list.length) continue;
    const g = document.createElement('optgroup');
    g.label = label;
    for (const p of list) g.append(new Option(p.name, p.id));
    m.append(g);
  }
  m.value = swatch.id;
}

// --- drawing ---
// What is shown: the layers composited, and a selection being moved over
// the cel drawn on (under any layer above it).
function view() {
  if (!sel?.fl) return D.composite();
  const v = D.px.slice();
  if (sel.cut) T.rect(sel.ox, sel.oy, sel.ox + sel.w - 1, sel.oy + sel.h - 1, true, (x, y) => { v[y * W + x] = 0; });
  floatOnto((i, k) => { v[i] = k; });
  return D.composite(D.F, v);
}
function floatOnto(put) {
  for (let j = 0; j < sel.h; j++) {
    for (let i = 0; i < sel.w; i++) {
      const k = sel.fl[j * sel.w + i], x = sel.x + i, y = sel.y + j;
      if (k && x >= 0 && x < W && y >= 0 && y < H) put(y * W + x, k);
    }
  }
}
function paint() {
  const v = view(), img = ctx.createImageData(W, H), d = img.data, rgbs = new Set();
  let off = false;
  for (let i = 0; i < N; i++) {
    const c = v[i] ? D.rgbOf(v[i]) : 0;
    if (v[i]) rgbs.add(c); else off = true;
    d[i * 4] = c >> 16; d[i * 4 + 1] = (c >> 8) & 255; d[i * 4 + 2] = c & 255; d[i * 4 + 3] = 255;
  }
  ctx.putImageData(img, 0, 0);
  const used = new Set(v);
  used.delete(0);
  // The GIF's colours: those drawn, and off (black) if any pixel is.
  const n = used.size, table = rgbs.size + (off && !rgbs.has(0) ? 1 : 0);
  const nf = D.frames.length, plays = D.rest.plays, t = D.frames.reduce((a, f) => a + f.ms, 0) / 1000;
  $('s-kind').textContent = `${nf > 1 ? `${plays ? `${plays}×` : 'LOOP'} · ${nf} FRAMES · ${+t.toFixed(2)} S` : 'STILL'} · 128×32 · ${n} COLOUR${n === 1 ? '' : 'S'}`;
  fx.painted?.();
  $('s-used').textContent = `Colours used: ${n}`;
  $('s-many').hidden = table <= 256;
  root.dataset.used = n;
  root.dataset.table = table;
  outline();
}
const plot = (k) => (x, y) => { if (x >= 0 && x < W && y >= 0 && y < H) D.set(y * W + x, k); };
// Fills and filled shapes set only the pixels the shade picks.
const shaded = (put) => (x, y) => { if (T.shade[shade](x, y)) put(x, y); };
// A shape seen while it is dragged: what it drew last undone, then drawn again.
function again(draw) {
  D.revert();
  D.begin();
  draw();
  paint();
}
// A second finger, or the pointer lost: nothing is made.
function cancel() {
  D.revert();
  if (drag?.was) Object.assign(sel, drag.was);
  if (drag?.marquee) sel = null;
  drag = null;
  showSel();
  paint();
}
// A stroke made: one step, with any picked colour it is the first to draw in.
function end() {
  const r = D.end();
  if (!r) return;
  if (r.step.c) swatches();
  after(r);
}
// A step done (doc.js push): kept in the journal (and the store's).
function after(r) {
  kept = true;
  jot({ step: r.step, cur: r.cur, low: r.low });
  buttons();
  queueSave();
  fx.changed?.(r.step);
}
function undo(fwd) {
  if (D.stroke || drag || busy) return; // not during a stroke
  if (!fwd && sel?.fl) { sel = null; D.letGo(D.key); swatches(); showSel(); paint(); return; } // a selection being moved (or a paste): put back, its new colours let go
  fx.stop?.();
  const c0 = D.cur, s = D.steps[fwd ? D.cur : D.cur - 1];
  let w;
  try { w = D.step(fwd); } catch (e) { say(`That step can’t be ${fwd ? 'redone' : 'undone'}: ${e.message || e}`); return; }
  const fin = () => {
    if (s.c || s.body) swatches();
    jot({ cur: D.low + D.cur - 1 });
    buttons();
    paint();
    queueSave();
    if (s.t || s.body || D.frames.length > 1 || D.layers.length > 1) frames().then(() => fx.changed?.(s), () => {}); else fx.changed?.(s);
  };
  if (w) {
    busy = true;
    w.then(fin, (e) => say(`That step can’t be ${fwd ? 'redone' : 'undone'}: ${e.message || e}`)).finally(() => { busy = false; });
  } else if (D.cur !== c0) fin();
}
function buttons() {
  $('s-undo').disabled = !D.cur;
  $('s-redo').disabled = D.cur >= D.steps.length;
  root.dataset.steps = `${D.cur}/${D.steps.length}`;
}
function jot(o) {
  if (!journaled || !store || !kept || closed) return;
  const id = rec.id;
  run(() => store.jot(id, o)).catch(() => {});
}

// --- the selection: a box; dragged, its pixels lift and float until it is done ---
function showSel() {
  $('s-selbar').hidden = tool !== 'select';
  root.dataset.sel = sel ? `${sel.x},${sel.y},${sel.w},${sel.h}${sel.fl ? ',float' : ''}` : '';
  root.dataset.pending = D.pending.size;
  outline();
}
function outline() {
  const o = $('s-sel');
  if (!sel || ledView) { o.hidden = true; return; }
  const s = work.clientWidth / W;
  o.hidden = false;
  o.style.cssText = `left:${sel.x * s}px;top:${sel.y * s}px;width:${sel.w * s}px;height:${sel.h * s}px`;
}
function lift() {
  if (sel.fl) return;
  sel.fl = new Uint16Array(sel.w * sel.h);
  for (let j = 0; j < sel.h; j++) for (let i = 0; i < sel.w; i++) sel.fl[j * sel.w + i] = D.px[(sel.y + j) * W + sel.x + i];
  Object.assign(sel, { cut: true, ox: sel.x, oy: sel.y });
}
// The floating pixels put down (one step; off in them is see-through); the box stays, on the panel.
function drop() {
  if (!sel?.fl) return;
  D.begin();
  if (sel.cut) T.rect(sel.ox, sel.oy, sel.ox + sel.w - 1, sel.oy + sel.h - 1, true, plot(0));
  floatOnto((i, k) => D.set(i, k));
  end();
  const x = Math.max(0, sel.x), y = Math.max(0, sel.y), w = Math.min(W, sel.x + sel.w) - x, h = Math.min(H, sel.y + sel.h) - y;
  sel = w > 0 && h > 0 ? { x, y, w, h } : null;
}
function done() { drop(); sel = null; showSel(); paint(); }
function act(a) {
  if (a === 'stamp') { if (sel) stamps().then((m) => m.saveSel(), () => {}); return; }
  if (a === 'paste') {
    if (!clip) return;
    fx.stop?.();
    if (tool !== 'select') useTool('select');
    drop();
    // Its colours join the piece in the step that puts it down, their keys
    // given out at once; it lands in the middle of what is in view.
    const cs = [...new Set(clip.c)].filter((c) => c), ks = D.keysFor(cs.map((c) => c - 1)), to = new Map(cs.map((c, n) => [c, ks[n]]));
    if (D.full) { D.letGo(D.key); say(`With its colours this piece would have more than ${CAPS.colours} colours: nothing was pasted.`); return false; } // (never half a paste)
    const fl = Uint16Array.from(clip.c, (c) => (c ? to.get(c) : 0));
    const r = scroll.getBoundingClientRect(), c = cv.getBoundingClientRect(), s = c.width / W;
    const mx = (Math.max(r.left, c.left) + Math.min(r.right, c.right)) / 2, my = (Math.max(r.top, c.top, 0) + Math.min(r.bottom, c.bottom, innerHeight)) / 2;
    const x = Math.round((mx - c.left) / s - clip.w / 2), y = Math.round((my - c.top) / s - clip.h / 2);
    sel = { x: Math.max(0, Math.min(W - clip.w, x)), y: Math.max(0, Math.min(H - clip.h, y)), w: clip.w, h: clip.h, fl, cut: false };
  } else if (a === 'done') done();
  else if (sel) {
    if (a === 'copy' || a === 'cut') {
      const keys = sel.fl || Uint16Array.from({ length: sel.w * sel.h }, (_, n) => D.px[(sel.y + Math.floor(n / sel.w)) * W + sel.x + n % sel.w]);
      clip = { w: sel.w, h: sel.h, c: Uint32Array.from(keys, (k) => (k ? D.rgbOf(k) + 1 : 0)) }; // colours, so it pastes into any piece
      clipNew = true;
      png(clip);
    }
    if (a === 'cut' || a === 'del') {
      // A box lifted from the panel: its place cleared. A paste not put down yet: gone.
      if (sel.fl) sel = sel.cut ? { x: sel.ox, y: sel.oy, w: sel.w, h: sel.h } : null;
      if (sel) { D.begin(); T.rect(sel.x, sel.y, sel.x + sel.w - 1, sel.y + sel.h - 1, true, plot(0)); end(); }
      sel = null;
      D.letGo(D.key);
      swatches();
    }
    if (a === 'fliph' || a === 'flipv') {
      lift();
      const f = sel.fl.slice(), { w, h } = sel;
      for (let j = 0; j < h; j++) for (let i = 0; i < w; i++) sel.fl[j * w + i] = a === 'fliph' ? f[j * w + (w - 1 - i)] : f[(h - 1 - j) * w + i];
    }
  }
  showSel();
  paint();
  return true;
}
// The Studio's copy on the system clipboard too, as a PNG, where the browser allows it.
function png(c) {
  if (!navigator.clipboard?.write || typeof ClipboardItem === 'undefined') return;
  const cv = new OffscreenCanvas(c.w, c.h), d = new ImageData(c.w, c.h);
  c.c.forEach((v, i) => { if (v) d.data.set([(v - 1) >> 16, ((v - 1) >> 8) & 255, (v - 1) & 255, 255], i * 4); });
  cv.getContext('2d').putImageData(d, 0, 0);
  navigator.clipboard.write([new ClipboardItem({ 'image/png': cv.convertToBlob({ type: 'image/png' }) })]).catch(() => {});
}
// Paste: an image on the system clipboard (paste.js), an address (Import), else the Studio's own copy.
function pasted(e) {
  if (e.target.closest?.('input, select, textarea') || document.querySelector('dialog[open]')) return;
  e.preventDefault();
  if (clip && clipNew) { act('paste'); return; }
  const f = [...(e.clipboardData?.files || [])].find((x) => x.type.startsWith('image/')), t = e.clipboardData?.getData('text/plain')?.trim() || '';
  if (f) pasteMod().then((m) => m.paste(f), () => {});
  else if (/^https:\/\/\S+$/.test(t)) pasteMod().then((m) => m.address(t), () => {});
  else act('paste');
}
// The Paste button: the system clipboard where the browser lets it be read, else the Studio's own copy.
async function pasteButton() {
  if (clip && clipNew) { act('paste'); return; }
  try {
    for (const it of await navigator.clipboard.read()) {
      const type = it.types.find((x) => x.startsWith('image/'));
      if (type) { const m = await pasteMod(); await m.paste(new File([await it.getType(type)], 'pasted', { type })); return; }
    }
  } catch { /* not allowed, or none: the Studio's own */ }
  act('paste');
}
const within = ([x, y]) => sel && x >= sel.x && x < sel.x + sel.w && y >= sel.y && y < sel.y + sel.h;

// --- pointers: one draws; a second one cancels it, then pans and pinches ---
const down = new Map();
let pan = null;
function at(e) {
  const r = cv.getBoundingClientRect();
  return [Math.floor((e.clientX - r.left) / r.width * W), Math.floor((e.clientY - r.top) / r.height * H)];
}
const inside = ([x, y]) => x >= 0 && x < W && y >= 0 && y < H;
const clamp = ([x, y]) => [Math.max(0, Math.min(W - 1, x)), Math.max(0, Math.min(H - 1, y))];
const brushed = () => /pencil|eraser|line/.test(tool);
// The shape tools as the drag says: from its start to `q`, Shift held or not.
function shape(q, shift) {
  const [x0, y0] = drag.p, k = D.key;
  let [x1, y1] = q;
  if (shift) [x1, y1] = tool === 'line' ? T.snap(x0, y0, x1, y1) : T.square(x0, y0, x1, y1);
  drag.q = q;
  const f = tool === 'rect' ? T.rect : T.ellipse;
  again(() => {
    if (tool === 'line') T.line(x0, y0, x1, y1, T.brush(size, plot(k)));
    else if (filled[tool]) f(x0, y0, x1, y1, true, shaded(plot(k)));
    else f(x0, y0, x1, y1, false, T.brush(size, plot(k)));
  });
}
const mid = (a, b) => ({ x: (a.clientX + b.clientX) / 2, y: (a.clientY + b.clientY) / 2, d: Math.hypot(a.clientX - b.clientX, a.clientY - b.clientY) || 1 });
cv.addEventListener('contextmenu', (e) => e.preventDefault());
cv.addEventListener('pointerdown', (e) => {
  fx.stop?.(); // drawing stops Play
  cv.setPointerCapture(e.pointerId);
  down.set(e.pointerId, e);
  if (down.size === 2) {
    cancel(); // a second finger: the stroke, the shape, the fill or the move is not made
    const [a, b] = [...down.values()];
    pan = { ...mid(a, b), l: scroll.scrollLeft, t: scroll.scrollTop, s: work.clientWidth / W };
    return;
  }
  if (down.size > 1) return;
  const p = at(e);
  if (!inside(p)) return;
  if (e.button === 2 || e.altKey || tool === 'pick') {
    pick(view()[p[1] * W + p[0]]); // what is shown: a selection being moved too
    if (tool === 'pick' && e.button === 0) useTool(prev);
    return;
  }
  if (e.button !== 0) return;
  if (tool === 'stamp') { fx.place?.(p); return; } // stamps.js: one step
  if (tool === 'fill') { drag = { p, fill: true }; return; }
  if (tool === 'select') {
    if (within(p)) { lift(); drag = { p, was: { x: sel.x, y: sel.y } }; } else { drop(); sel = { x: p[0], y: p[1], w: 1, h: 1 }; drag = { p, marquee: true }; }
    showSel();
    paint();
    return;
  }
  drag = { p, q: p };
  if (brushed() && tool !== 'line') {
    D.begin();
    T.brush(size, plot(tool === 'eraser' ? 0 : D.key))(p[0], p[1]);
    paint();
  } else shape(p, e.shiftKey);
});
cv.addEventListener('pointermove', (e) => {
  const p = at(e);
  hover(p);
  if (!down.has(e.pointerId)) return;
  down.set(e.pointerId, e);
  if (pan && down.size === 2) { pinch(); return; }
  if (!drag || drag.fill || down.size > 1) return;
  const q = clamp(p);
  if (tool === 'select') {
    if (drag.marquee) {
      const [x0, y0] = drag.p;
      sel = { x: Math.min(x0, q[0]), y: Math.min(y0, q[1]), w: Math.abs(q[0] - x0) + 1, h: Math.abs(q[1] - y0) + 1 };
    } else { sel.x = drag.was.x + p[0] - drag.p[0]; sel.y = drag.was.y + p[1] - drag.p[1]; }
    showSel();
    paint();
  } else if (tool === 'pencil' || tool === 'eraser') {
    if (!D.stroke) return;
    T.line(drag.q[0], drag.q[1], p[0], p[1], T.brush(size, plot(tool === 'eraser' ? 0 : D.key)));
    drag.q = p;
    paint();
  } else shape(q, e.shiftKey);
});
// What a pointer makes, it makes as it lifts.
const up = (e) => {
  down.delete(e.pointerId);
  if (!down.size) pan = null;
  if (drag && !down.size && e.type === 'pointerup') {
    if (drag.fill) {
      D.begin();
      const [x, y] = drag.p, k = D.key, on = T.shade[shade];
      for (const i of T.flood(D.px, W, H, x, y)) if (on(i % W, (i / W) | 0)) D.set(i, k);
    }
    end();
    if (drag.marquee && sel?.w === 1 && sel.h === 1) sel = null; // a click: nothing selected
    drag = null;
    showSel();
    paint();
  } else if (!down.size && (drag || D.stroke)) cancel();
};
cv.addEventListener('pointerup', up);
cv.addEventListener('pointercancel', up);
cv.addEventListener('pointerleave', () => { $('s-cursor').hidden = true; $('s-xy').innerHTML = 'X <b>–</b> Y <b>–</b>'; });
function hover([x, y]) {
  const c = $('s-cursor');
  if (!inside([x, y]) || ledView) { c.hidden = true; return; }
  const s = work.clientWidth / W, n = brushed() ? size : 1, lo = n === 3 ? 1 : 0;
  c.hidden = false;
  c.style.cssText = `left:${(x - lo) * s}px;top:${(y - lo) * s}px;width:${n * s}px;height:${n * s}px`;
  $('s-xy').innerHTML = `X <b>${x}</b> Y <b>${y}</b>`;
  fx.hover?.([x, y]);
}
// Two fingers: their middle pans, their spread zooms (what is under them stays put).
function pinch() {
  const [a, b] = [...down.values()], m = mid(a, b), want = pan.s * m.d / pan.d;
  let z = 0;
  if (want > fitScale() * 1.1) { z = 1; ZOOMS.forEach((s, n) => { if (Math.abs(s - want) < Math.abs(ZOOMS[z - 1] - want)) z = n + 1; }); }
  if (z !== zoom) {
    const r = work.getBoundingClientRect(), cx = (m.x - r.left) / r.width, cy = (m.y - r.top) / r.height;
    zoom = z;
    layout();
    const r2 = work.getBoundingClientRect();
    scroll.scrollLeft += r2.left + cx * r2.width - m.x;
    scroll.scrollTop += r2.top + cy * r2.height - m.y;
    Object.assign(pan, { x: m.x, y: m.y, l: scroll.scrollLeft, t: scroll.scrollTop });
    return;
  }
  scroll.scrollLeft = pan.l - (m.x - pan.x);
  scroll.scrollTop = pan.t - (m.y - pan.y);
}

function useTool(t, fill) {
  if (t !== 'select' && sel) done();
  if (tool === 'stamp' && t !== 'stamp') fx.unstamp?.();
  if (t === 'stamp') stamps().then((m) => m.tool(), () => {});
  if (t !== 'pick') prev = t;
  if (fill !== undefined) filled[t] = fill;
  else if (t === tool && t in filled) filled[t] = !filled[t]; // the shape's button again: edge <-> filled
  tool = t;
  for (const b of document.querySelectorAll('.s-tool')) {
    const d = b.dataset.tool;
    b.classList.toggle('on', d === t);
    b.setAttribute('aria-pressed', d === t);
    if (d in filled) icon(b, d + (filled[d] ? 'f' : ''));
  }
  root.dataset.tool = t + (filled[t] ? '-filled' : '');
  // Edge or filled, for a shape (a phone has no Shift); the shade, for a fill or a filled shape.
  $('s-fillmode').hidden = !(t in filled);
  for (const b of $('s-fillmode').children) { const on = +b.dataset.fill === +!!filled[t]; b.classList.toggle('on', on); b.setAttribute('aria-pressed', on); }
  $('s-shade').hidden = !(t === 'fill' || filled[t]);
  showSel();
}
function useShade(sh) {
  shade = sh;
  for (const b of $('s-shade').children) { const on = b.dataset.shade === sh; b.classList.toggle('on', on); b.setAttribute('aria-pressed', on); }
  root.dataset.shade = sh;
}
function useSize(n) {
  size = n;
  for (const b of document.querySelectorAll('.s-size [data-size]')) { b.classList.toggle('on', +b.dataset.size === n); b.setAttribute('aria-pressed', +b.dataset.size === n); }
  root.dataset.brush = n;
}

// --- the view: zoom (Fit, then whole steps), the grid, the seam, LEDs ---
let fitScale = () => 1;
function layout() {
  const cs = getComputedStyle(scroll);
  const avail = scroll.clientWidth - parseFloat(cs.paddingLeft) - parseFloat(cs.paddingRight) - 24;
  fitScale = () => Math.max(1, avail / W);
  let s = zoom ? ZOOMS[zoom - 1] : fitScale();
  if (!zoom && s > 4) s = Math.min(8, Math.floor(s));
  work.style.width = `${s * W}px`;
  work.style.height = `${s * H}px`;
  $('s-ruler').style.width = `${s * W}px`;
  const g = $('s-grid');
  g.style.backgroundSize = `${s}px ${s}px, ${s}px ${s}px, ${8 * s}px ${8 * s}px, ${8 * s}px ${8 * s}px`;
  g.classList.toggle('dense', s < 5);
  $('s-z').textContent = zoom ? `${s}×` : 'FIT';
  $('s-z-out').disabled = !zoom;
  $('s-z-in').disabled = zoom === ZOOMS.length;
  outline();
}
function zoomBy(d) {
  if (d > 0) zoom = Math.min(ZOOMS.length, zoom ? zoom + 1 : Math.max(1, ZOOMS.findIndex((z) => z > work.clientWidth / W) + 1));
  else zoom = zoom > 1 && ZOOMS[zoom - 2] > fitScale() ? zoom - 1 : 0;
  layout();
}
function toggle(id, on) {
  const b = $(id);
  b.classList.toggle('on', on);
  b.setAttribute('aria-pressed', on);
}
function viewLeds(led) {
  ledView = led;
  work.classList.toggle('led', led);
  toggle('s-v-px', !led);
  toggle('s-v-led', led);
  ls.set('view', led ? 'led' : 'px');
  outline();
}

// --- keeping it: autosave into the store ---
const TEXT = { saving: 'Saving…', none: 'Not kept: this browser won’t keep drafts here', new: 'Not kept yet: draw something' };
function status(what, text) {
  if (closed) { what = 'failed'; text = closed; } // a newer Studio took the drafts: said until a reload
  root.dataset.saved = what;
  $('s-saved').textContent = text || (what === 'kept' ? 'Kept in this browser' : TEXT[what]);
}
function say(text) { $('s-err').hidden = !text; $('s-err').textContent = text || ''; }
// A sentence with one thing to do about it.
function offer(text, label, go) {
  say(text);
  const b = el('button', 's-pill', label);
  b.onclick = () => { say(''); go(); };
  $('s-err').append(' ', b);
}
function queueSave() {
  clearTimeout(timer);
  if (!store || !kept || closed) return;
  timer = setTimeout(save, 400);
  status('saving');
}
// Every write goes through one chain, one at a time, so a save, a download's
// snapshot, a delete and the next save never overlap; one that fails says so
// and the chain goes on.
function run(job) {
  const p = chain.then(job);
  chain = p.catch(() => {});
  return p;
}
function save() {
  clearTimeout(timer);
  queued ??= run(() => { queued = null; return saveNow(); });
  return queued;
}
const nameNow = () => $('s-name').value.trim().slice(0, CAPS.name) || 'Untitled';
const stamp = () => new Date().toISOString().replace(/\.\d+Z$/, 'Z');
// What a piece is made of, as the store keeps it: a download's `shot` (doc.js snap), or now.
async function saveNow(shot) {
  if (!store || !rec || !kept || closed) return;
  try {
    const sn = shot || D.snap();
    let at = sn.at, note = null; // where the journal is as these pixels are taken
    let next = { ...rec, name: nameNow(), body: await D.packed(sn, { palette: swatch.id }) };
    for (let tries = 0; ; tries++) {
      try {
        const h = await store.put(next, { base: rec.ver ?? null, at: journaled ? at : undefined });
        rec = { ...next, hash: h.hash, modified: h.modified, ver: h.ver };
        lastHash = h.hash;
        nameDirty = false;
        break;
      } catch (e) {
        if (!(e instanceof Conflict)) throw e;
        const was = e.head;
        // Its pixels are as this tab last had them: only its rev or its name
        // moved (a download, a rename, in another tab). Go on from there,
        // with the other tab's name unless this one was renamed too.
        if (!was.gone && was.hash === lastHash && tries < 3) {
          next = { ...next, rev: was.rev, name: nameDirty ? next.name : was.name };
          if (!nameDirty) $('s-name').value = was.name;
          rec = { ...rec, ver: was.ver };
          continue;
        }
        // Its pixels changed in another tab: this one's work is kept as a copy that says where it came from.
        next = { ...next, id: newId('piece'), rev: 1, created: stamp(), name: `${next.name.slice(0, CAPS.name - 7).trimEnd()} (copy)`,
          from: was.hash ? [{ id: was.id, rev: was.rev, hash: was.hash, name: was.name || '', author: was.author || { name: '', account: null },
            licence: was.licence ?? null }] : [] };
        rec = { ...rec, ver: null };
        lastHash = null;
        D.steps = []; D.cur = 0; D.low = 1; at = 0; buttons(); // the copy starts a history of its own
        await lock(next.id);
        $('s-name').value = next.name;
        root.dataset.id = next.id;
        note = 'Changed in another tab: yours is kept as a copy';
      }
    }
    ls.set('last', rec.id);
    root.dataset.rev = rec.rev;
    await keep();
    status('kept', note);
    if (note) await drafts(); else one();
  } catch (e) {
    status('failed', `Not kept: ${e.message || e}`);
  }
}
// After the first saved stroke, ask the browser to keep the drafts for good; say whether it will.
async function keep() {
  if (persisted !== null || !navigator.storage?.persist) return;
  persisted = await navigator.storage.persisted() || await navigator.storage.persist();
  root.dataset.persist = persisted ? 'yes' : 'no';
  // Refused: said once, in this browser.
  if (!persisted && !ls.get('loose')) { $('s-loose').hidden = false; ls.set('loose', '1'); }
}
// A piece open in one tab keeps its journal (navigator.locks); a second tab
// on it still draws and saves, with this visit's undo only.
async function lock(id) {
  unlock?.();
  unlock = null;
  journaled = !navigator.locks;
  if (navigator.locks) {
    journaled = await new Promise((got) => {
      navigator.locks.request(`opendmd-studio:${id}`, { ifAvailable: true }, (l) => {
        got(!!l);
        return l && new Promise((free) => { unlock = free; });
      }).catch(() => got(false));
    });
  }
  root.dataset.journal = journaled ? 'yes' : 'no';
}

// A stored record opened, and its journal replayed (doc.js open): from where
// the record stands (`at`) to where undo was (`cur`). A journal that is not
// this record's, or of another shape (an earlier Studio's), is dropped.
async function open(r) {
  check(r); // a record written past check() (another tab, an older Studio, a hand) is not opened
  await Doc.open(r, listOf, null); // throws on a draft that does not read, before the open one's lock is let go
  await lock(r.id);
  const j = journaled && store && r.hash ? await store.steps(r.id) : null;
  const d = await Doc.open(r, listOf, j);
  sel = null;
  D = d;
  D.held = () => sel?.fl;
  showSel();
  rec = r;
  useSwatches(builtin(r.body.palette) || mine.get(r.body.palette) || builtin(DEFAULT));
  kept = true;
  lastHash = r.hash;
  nameDirty = false;
  if (D.said === 'dropped') await run(() => store.forget(r.id)); // written elsewhere since, or an older shape: no longer this record's
  if (D.said === 'moved') queueSave();
  buttons();
  $('s-name').value = r.name;
  D.key = D.keyOf(AMBER) || keyFor(AMBER);
  swatches();
  paint();
  ls.set('last', r.id);
  root.dataset.id = r.id;
  root.dataset.rev = r.rev;
  if (root.dataset.saved !== 'saving') status('kept');
  // More than one frame or layer: the strip and the layer rows (frames.js).
  if (D.frames.length > 1 || D.layers.length > 1) await frames().catch(() => {});
  fx.opened?.();
  await snaps();
}
// A new piece, with no colours until it is drawn on: the one way a piece
// comes in (an importer, a template, a file, a link). `spec` is a whole
// piece, {name, tags, plays, frames: [{ms}], layers: [{name, moves, hide,
// cels: [keys…]}], colours, from}, keys as Uint8Array or Uint16Array, or the
// one-cel form {name, keys, colours, from}. Its layers and frames get stable
// keys as they are made; it is checked (store.js check) before it opens.
const k8 = () => [...crypto.getRandomValues(new Uint8Array(4))].map((x) => x.toString(16).padStart(2, '0')).join('');
export async function piece({ name = 'Untitled', tags = [], plays, keys = new Uint8Array(N), colours = [], from = [],
  frames = [{ ms: STILL_MS }], layers = [{ name: 'Drawing', moves: false, cels: [keys] }] } = {}) {
  const wide = colours.some((c) => c.k > 255);
  const r = record('piece', name, { w: W, h: H, palette: swatch.id, ...(plays ? { plays } : {}), colours, ...(wide ? { keys: 2 } : {}),
    frames: frames.map((f) => ({ key: k8(), ms: f.ms ?? STILL_MS })),
    layers: await Promise.all(layers.map(async (l) => ({ key: k8(), name: l.name ?? '', moves: !!l.moves, ...(l.hide ? { hide: true } : {}),
      cels: await Promise.all(l.cels.map((c) => packPx(c, wide))) }))) }, { from, tags });
  check(r);
  await open({ ...r, hash: '' });
  lastHash = null;
  kept = false; // stored at its first stroke (or name)
  status(store ? 'new' : 'none');
  return r;
}
async function fresh() {
  await piece();
  if (!coarse) $('s-name').select(); // not on a phone: its keyboard would cover the panel
}
// A draft that will not open: said, and a new piece instead.
async function tryOpen(r) {
  try { await open(r); return true; } catch {
    say(`“${typeof r.name === 'string' ? r.name : 'A draft'}” can’t be opened: its picture is damaged. It stays in Drafts; this is a new piece.`);
    return false;
  }
}

// The drafts, newest first, each drawn as LEDs; after a save, only the open one is drawn again.
const ago = (iso) => {
  const s = (Date.now() - Date.parse(iso)) / 1000;
  return s < 60 ? 'just now' : s < 3600 ? `${Math.floor(s / 60)} min ago` : s < 86400 ? `${Math.floor(s / 3600)} h ago` : new Date(iso).toLocaleDateString();
};
function thumb(c, keys, m) {
  const g = c.getContext('2d'), img = g.createImageData(W, H);
  for (let i = 0; i < N; i++) { const v = keys[i] ? m.get(keys[i]) ?? 0 : 0; img.data.set([v >> 16, (v >> 8) & 255, v & 255, 255], i * 4); }
  g.putImageData(img, 0, 0);
}
function one() {
  const li = $('s-list').querySelector(`.s-open[data-id="${rec.id}"]`)?.parentElement;
  if (!li) { drafts(); return; }
  li.className = 'on';
  li.querySelector('b').textContent = nameNow();
  li.querySelector('small').textContent = 'Open now';
  li.querySelector('.s-del').hidden = false;
  thumb(li.querySelector('canvas'), D.composite(), D.cols);
  root.dataset.drawn = (+root.dataset.drawn || 0) + 1;
}
async function drafts() {
  const ul = $('s-list');
  if (!store) { ul.replaceChildren(); return; }
  const { items } = await store.list({ type: 'piece' });
  if (rec && !kept) items.unshift({ id: rec.id, name: nameNow(), modified: rec.modified });
  const lis = await Promise.all(items.map(async (h) => {
    if (typeof h.name !== 'string') h = { ...h, name: 'Untitled' }; // a record that is not one: listed, never trusted
    const li = el('li', h.id === rec?.id ? 'on' : ''), b = el('button', 's-open'), led = el('span', 'led'), c = el('canvas'), t = el('span', 's-meta');
    const open = h.id === rec?.id, del = el('button', 's-del', '×');
    b.dataset.id = del.dataset.id = h.id;
    c.width = W; c.height = H;
    led.append(c);
    t.append(el('b', '', h.name), el('small', '', open ? (kept ? 'Open now' : 'Open now, not kept yet') : `Edited ${ago(h.modified)}`));
    b.append(led, t);
    del.title = `Delete ${h.name}`;
    del.setAttribute('aria-label', `Delete ${h.name}`);
    del.hidden = open && !kept;
    li.append(b, del);
    try {
      if (open) thumb(c, D.composite(), D.cols);
      else { const d = new Doc((await store.get(h.id)).body, listOf); await d.load(0); thumb(c, d.composite(0), d.cols); }
    } catch { /* a draft that does not read: listed, not drawn */ }
    return li;
  }));
  ul.replaceChildren(...lis);
  root.dataset.drawn = 0;
}

// --- snapshots: named revisions (store.snapshot), restored as one undo step ---
// What the piece says about itself: who made it, its licence, what it was made from.
function prov() {
  const r = rec, from = (r?.from || []).map((f) => (f.external ? f.note || 'an imported picture' : f.name || 'a record')).filter(Boolean);
  $('s-prov').textContent = r ? `Made by ${r.author?.name || '—'} · Licence ${r.licence === 'CC0-1.0' ? 'CC0' : r.licence === 'CC-BY-4.0' ? 'CC BY 4.0' : 'not chosen'}`
    + (from.length ? ` · Made from ${from.slice(0, 3).join(', ')}${from.length > 3 ? ` and ${from.length - 3} more` : ''}` : '') : '';
}
// A record deleted (here or from the Library); the open piece's place taken by the newest other, or a new one.
async function forget(id) {
  if (id === rec.id) { clearTimeout(timer); kept = false; }
  await run(() => store.remove(id));
  if (id === rec.id) {
    const { items } = await store.list({ type: 'piece' });
    const r = items.length && await store.get(items[0].id);
    if (!r || !await tryOpen(r)) await fresh();
  }
  await drafts();
}
// Another record opened in the editor (from the Library, a file).
async function openRec(r) {
  done();
  await save();
  if (!await tryOpen(r)) await fresh();
  await drafts();
}
async function snaps() {
  prov();
  const list = store && kept && rec ? (await store.revs(rec.id)).reverse() : [];
  $('s-snaps').replaceChildren(...list.map((h) => {
    const li = el('li'), t = el('span', 's-meta'), b = el('button', 's-pill', 'Restore');
    t.append(el('b', '', h.label || 'Downloaded'), el('small', '', `${ago(h.modified)} · version ${h.rev} · Restore brings back this version, name and all`));
    b.dataset.rev = h.rev;
    b.title = 'Brings back this version, name and all: what is here now is kept as a snapshot first';
    li.append(t, b);
    return li;
  }));
  $('s-snaps-none').hidden = list.length > 0;
}
function freeze(label, shot) {
  return run(async () => {
    if (!store || !kept) return null;
    clearTimeout(timer);
    await saveNow(shot);
    if (root.dataset.saved === 'failed') return null;
    const h = await store.snapshot(rec.id, label);
    rec = { ...rec, rev: h.rev, ver: h.ver };
    root.dataset.rev = h.rev;
    return h;
  });
}
async function restore(rev) {
  const r = await store.get(rec.id, rev);
  if (!r) return;
  done();
  await freeze(`Before restoring ${r.label || `version ${rev}`}`.slice(0, CAPS.name));
  try {
    after(await D.restore(r.body, { palette: swatch.id }));
    // The whole record as it was then: its name, tags, note, who made it, its licence, what it was made from.
    const { name, tags, note, author, licence, from } = r;
    rec = { ...rec, name, tags, note, author, licence, from };
    $('s-name').value = name;
    nameDirty = true;
    prov();
    queueSave();
  } catch (e) {
    say(`That snapshot can’t be restored: ${e.message || e}`);
  }
  swatches();
  paint();
  await snaps();
}

// --- the GIF and Send to panel: out.js, loaded at the first download or send ---
let outMod = null;
async function out(what, ...a) {
  outMod ??= import('./out.js');
  try { await (await outMod)[what](studio, ...a); } catch (e) { outMod = null; say(`The GIF could not be made: ${e.message || e}`); }
}
const download = (fit) => out('download', fit);

// --- the frames strip and the layer rows: frames.js, loaded once a piece has more than one of either, or one is added ---
const fx = {}; // what frames.js hooks in: painted(), changed(step), opened(), stop()
let framesMod = null;
const frames = () => (framesMod ??= import('./frames.js').then((m) => { m.mount(studio); return m; }).catch((e) => { framesMod = null; say(`The frames could not load: ${e.message || e}`); throw e; }));
let stampsMod = null, libMod = null, filesMod = null;
const lazy = (get, set, name) => () => (get() ?? set(import(`./${name}.js`).then((m) => { m.mount?.(studio); return m; })
  .catch((e) => { set(null); say(`That could not load: ${e.message || e}`); throw e; })));
const stamps = lazy(() => stampsMod, (v) => (stampsMod = v), 'stamps');
const library = lazy(() => libMod, (v) => (libMod = v), 'library');
const files = lazy(() => filesMod, (v) => (filesMod = v), 'files');
let impMod = null, pasteMod_ = null;
const imp = lazy(() => impMod, (v) => (impMod = v), 'import');
const pasteMod = lazy(() => pasteMod_, (v) => (pasteMod_ = v), 'paste');
// The Library is a view of this page (#library): the editor comes back when
// it is left. Import (#import, maybe ?url=, ?q=&platform=, ?shared=1) is a sheet over the editor.
function route() {
  const lib = location.hash === '#library', im = /^#import(\?|$)/.test(location.hash);
  if (lib) { done(); library().then((m) => m.show(), () => {}); } else fx.unlib?.();
  if (im) { if (!/open|ready/.test(root.dataset.import)) { done(); imp().then((m) => m.show(new URLSearchParams(location.hash.slice(8))), () => {}); } } else impMod?.then((m) => m.close(), () => {});
  root.dataset.view = lib ? 'library' : 'editor';
}
const withFrames = (f) => () => { if (D.stroke || drag) return; done(); frames().then(f, () => {}); };

// --- the colour picker and the palette tools: colour.js, loaded when first opened ---
let colourMod = null;
const studio = {
  get store() { return store; }, run, mine, say, swatch: () => swatch, colours: () => D.colourList(), current: () => (D.key ? D.rgbOf(D.key) : null),
  pickColour, piece: () => rec && kept && lastHash ? { id: rec.id, rev: rec.rev, hash: lastHash, name: nameNow() } : null,
  use(p) { if (p) { useSwatches(p); queueSave(); } menu(); },
  // for out.js
  get doc() { return D; }, get rec() { return rec; }, get kept() { return kept; }, get saved() { return root.dataset.saved; },
  root, listOf, done, freeze, snaps, status, nameNow,
  // for frames.js
  fx, after, paint, buttons, showSel, get sel() { return sel; }, get dragging() { return !!drag; },
  newPiece: piece, // a whole piece in (import, templates, files, links); piece() above is the open one's reference
  // for stamps.js, library.js and files.js
  forget, fresh, useSwatches, menu, files, stamps,
  // The open piece read again as stored (a file's version taken), its unsaved state already saved.
  async reload(r) { clearTimeout(timer); if (!await tryOpen(r)) await fresh(); await drafts(); },
  // The open piece's name, tags or note changed from the Library: through the editor, which saves it.
  setHead(o) { if ('name' in o) { $('s-name').value = o.name; nameDirty = true; } rec = { ...rec, ...o }; kept = true; queueSave(); },
  get tool() { return tool; }, useTool, swatches, keep() { kept = true; queueSave(); }, openRec, drafts, save,
  get journaled() { return journaled; }, jot, starters: () => starters, builtin,
  // for import.js and paste.js
  offer, imp, get clip() { return clip; }, paste(c) { clip = c; return act('paste'); },
  // A whole body for this piece (an import's new layer), as one undo step.
  async replace(body) {
    done();
    after(await D.restore(body, { palette: swatch.id }));
    swatches();
    paint();
    await frames().catch(() => {});
    fx.changed?.(D.steps[D.cur - 1]);
  },
  // What the piece was made from gains `e` (once: the same id and hash, or note); false at the cap.
  // `dry`: only whether it could be (a stamp asks before it places, and adds once it has).
  addFrom(e, dry = false) {
    const f = rec.from || [];
    if (f.some((x) => (e.id ? x.id === e.id && x.hash === e.hash : x.external && x.note === e.note))) return true;
    if (f.length >= CAPS.from) { say(`A piece names ${CAPS.from} things it was made from at most, and this one is full: this can’t be added to it.`); return false; }
    if (dry) return true;
    rec = { ...rec, from: [...f, e] };
    kept = true;
    queueSave();
    prov();
    return true;
  },
  // Made by and licence, as an export sets them.
  meta({ author, licence }) { rec = { ...rec, author: { name: author, account: rec.author?.account ?? null }, licence }; kept = true; queueSave(); prov(); },
};
async function colourTools(which, b) {
  colourMod ??= import('./colour.js');
  const m = await colourMod, host = $(which === 'picker' ? 's-picker' : 's-palx');
  host.hidden = !host.hidden;
  b.setAttribute('aria-expanded', !host.hidden);
  if (!host.hidden) m[which](host, studio);
}
async function unused() {
  done();
  const r = await D.unused();
  if (!r) return;
  after(r);
  swatches();
  paint();
}

// --- start ---
async function main() {
  for (const e of document.querySelectorAll('[data-i]')) icon(e, e.dataset.i);
  $('s-pal').addEventListener('change', () => studio.use(builtin($('s-pal').value) || mine.get($('s-pal').value)));
  $('s-sw').addEventListener('click', (e) => { const b = e.target.closest('.s-sw'); if (b) pick(+b.dataset.k); });
  $('s-set').addEventListener('click', (e) => { const b = e.target.closest('.s-sw'); if (b) pickColour(toInt(b.dataset.hex)); });
  $('s-add').addEventListener('click', (e) => colourTools('picker', e.currentTarget));
  $('s-pals').addEventListener('click', (e) => colourTools('palettes', e.currentTarget));
  $('s-unused').addEventListener('click', unused);
  for (const b of document.querySelectorAll('.s-tool')) b.addEventListener('click', () => useTool(b.dataset.tool));
  for (const b of document.querySelectorAll('.s-size [data-size]')) b.addEventListener('click', () => useSize(+b.dataset.size));
  $('s-selbar').addEventListener('click', (e) => { const b = e.target.closest('[data-act]'); if (b) { if (b.dataset.act === 'paste' && navigator.clipboard?.read) pasteButton(); else act(b.dataset.act); } });
  $('s-undo').addEventListener('click', () => undo(false));
  $('s-redo').addEventListener('click', () => undo(true));
  $('s-gif').addEventListener('click', () => download(false));
  $('s-send').addEventListener('click', () => { done(); out('send'); });
  $('s-libb').addEventListener('click', () => { location.hash = '#library'; });
  $('s-imp-b').addEventListener('click', () => { done(); history.pushState({ imp: 1 }, '', '#import'); route(); });
  addEventListener('hashchange', route);
  document.addEventListener('paste', pasted);
  addEventListener('blur', () => { clipNew = false; }); // (something else may be copied now)
  // A drop: Studio files to files.js; a picture, GIF or video, or a link, to Import.
  // (Never left to the browser, which would open the file in place of the Studio.)
  document.addEventListener('dragover', (e) => e.preventDefault());
  document.addEventListener('drop', (e) => {
    e.preventDefault();
    const fs = [...e.dataTransfer.files], ours = fs.filter((f) => /\.(dmdart|dmdpack)$/i.test(f.name)), rest = fs.filter((f) => !ours.includes(f));
    const u = (e.dataTransfer.getData('text/uri-list') || '').split('\n')[0].trim();
    // On the open Import sheet: a picture or video into it; any other sheet takes no drop.
    if (document.querySelector('dialog[open]')) { if (rest.length && /open|ready/.test(root.dataset.import)) imp().then((m) => m.bring(rest[0], 'Your file'), () => {}); return; }
    if (root.dataset.view === 'library') { history.replaceState(null, '', location.pathname + location.search); route(); } // (to the editor first)
    if (ours.length) {
      files().then((m) => m.open(ours), () => {});
      if (rest.length) say(`${rest.length === 1 ? 'The picture' : 'The pictures'} dropped with ${ours.length === 1 ? 'that file' : 'those files'} ${rest.length === 1 ? 'was' : 'were'} left: drop ${rest.length === 1 ? 'it' : 'one'} on its own to import it.`);
    } else if (rest.length) imp().then((m) => m.bring(rest[0], 'Your file'), () => {});
    else if (/^https:\/\//i.test(u)) imp().then((m) => { history.pushState({ imp: 1 }, '', '#import'); m.show(new URLSearchParams({ url: u }), true); }, () => {});
  });
  $('s-stampsb').addEventListener('click', () => stamps().then((m) => m.show(), () => {}));
  $('s-savef').addEventListener('click', () => { done(); files().then((m) => m.saveOpen(), () => {}); });
  $('s-openf').addEventListener('click', () => files().then((m) => m.pick(), () => {}));
  $('s-addf').addEventListener('click', withFrames((m) => m.addFrame()));
  $('s-addl').addEventListener('click', withFrames((m) => m.addLayer()));
  // A phone's bench is tabs: Colours, Layers, Piece.
  root.dataset.tab = 'colours';
  $('s-tabs').addEventListener('click', (e) => {
    const b = e.target.closest('[data-tab]');
    if (!b) return;
    root.dataset.tab = b.dataset.tab;
    if (b.dataset.tab === 'stamps') stamps().then((m) => m.show(), () => {});
    for (const t of $('s-tabs').children) t.setAttribute('aria-selected', t === b);
  });
  $('s-fit').addEventListener('click', () => download(true));
  $('s-fillmode').addEventListener('click', (e) => { const b = e.target.closest('[data-fill]'); if (b) useTool(tool, b.dataset.fill === '1'); });
  $('s-shade').addEventListener('click', (e) => { const b = e.target.closest('[data-shade]'); if (b) useShade(b.dataset.shade); });
  $('s-snap').addEventListener('click', async () => {
    const n = $('s-snapname');
    if (!kept) { say('Draw something first: a snapshot keeps a piece as it is.'); return; }
    done();
    if (await freeze(n.value.trim() || `Snapshot, ${new Date().toLocaleString()}`)) { n.value = ''; await snaps(); }
  });
  $('s-snaps').addEventListener('click', (e) => { const b = e.target.closest('[data-rev]'); if (b) restore(+b.dataset.rev); });
  $('s-new').addEventListener('click', async () => { done(); await save(); await fresh(); await drafts(); });
  $('s-list').addEventListener('click', async (e) => {
    const o = e.target.closest('.s-open'), d = e.target.closest('.s-del');
    if (o && o.dataset.id !== rec.id) { done(); await save(); const r = await store.get(o.dataset.id); if (r && !await tryOpen(r)) await fresh(); await drafts(); }
    if (d && confirm(`Delete ${d.title.slice(7)}? It can’t be brought back.`)) await forget(d.dataset.id);
  });
  $('s-name').addEventListener('input', () => { kept = true; nameDirty = true; queueSave(); });
  $('s-v-px').addEventListener('click', () => viewLeds(false));
  $('s-v-led').addEventListener('click', () => viewLeds(true));
  $('s-g-grid').addEventListener('click', () => { const on = !$('s-g-grid').classList.contains('on'); toggle('s-g-grid', on); $('s-grid').hidden = !on; });
  $('s-g-seam').addEventListener('click', () => { const on = !$('s-g-seam').classList.contains('on'); toggle('s-g-seam', on); $('s-seam').hidden = !on; });
  $('s-z-in').addEventListener('click', () => zoomBy(1));
  $('s-z-out').addEventListener('click', () => zoomBy(-1));
  new ResizeObserver(layout).observe(scroll);
  const TOOLS = { b: 'pencil', e: 'eraser', l: 'line', r: 'rect', o: 'ellipse', g: 'fill', m: 'select', i: 'pick', s: 'stamp' };
  const CLIP = { c: 'copy', x: 'cut', v: 'paste' };
  const shapes = () => drag?.q && /line|rect|ellipse/.test(tool);
  document.addEventListener('keydown', (e) => {
    // Not in a field, nor behind an open sheet (Send to panel): the piece there is the one going out.
    if (e.target.closest('input, select, textarea') || document.querySelector('dialog[open]')) return;
    const k = e.key.toLowerCase(), mod = e.ctrlKey || e.metaKey;
    if (mod && k === 'z') { e.preventDefault(); undo(e.shiftKey); }
    else if (mod && k === 'y') { e.preventDefault(); undo(true); }
    else if (mod && k === 'v') { /* the paste event (pasted): it says what the clipboard holds */ }
    else if (mod && CLIP[k] && sel) { e.preventDefault(); act(CLIP[k]); }
    else if (mod && k === 'a') { e.preventDefault(); useTool('select'); drop(); sel = { x: 0, y: 0, w: W, h: H }; showSel(); paint(); }
    else if (mod || e.altKey || drag) { if (k === 'shift' && shapes()) shape(drag.q, true); }
    else if (e.shiftKey && (k === 'h' || k === 'v') && sel) act(k === 'h' ? 'fliph' : 'flipv');
    else if (TOOLS[k]) useTool(TOOLS[k], TOOLS[k] in filled ? e.shiftKey : undefined);
    else if (k === 'd') useShade(T.SHADES[(T.SHADES.indexOf(shade) + 1) % T.SHADES.length]); // D cycles the shade
    else if (k === '1' || k === '2' || k === '3') useSize(+k);
    else if ((k === 'delete' || k === 'backspace') && sel) { e.preventDefault(); act('del'); }
    else if ((k === 'enter' || k === 'escape') && sel) act('done');
    else if (k === 'escape' && tool === 'stamp') useTool('pencil');
    else if (k === ',' || k === '.') fx.step?.(k === '.' ? 1 : -1); // the frame before, after
    else if (k === ' ') { e.preventDefault(); if (D.frames.length > 1) frames().then((m) => m.toggle(), () => {}); }
    else if (k === 'n') withFrames((m) => m.addFrame())();
    else if (k === '+' || k === '=') zoomBy(1);
    else if (k === '-') zoomBy(-1);
  });
  document.addEventListener('keyup', (e) => { if (e.key === 'Shift' && shapes()) shape(drag.q, false); });
  // The last save, as the tab is hidden or closed.
  document.addEventListener('visibilitychange', () => { if (document.visibilityState === 'hidden') { if (sel?.fl) done(); if (root.dataset.saved === 'saving') save(); } });
  viewLeds(ls.get('view') === 'led');
  // A phone opens close up (about 8x), panned and zoomed with two fingers.
  if (coarse || innerWidth < 600) {
    zoom = ZOOMS.indexOf(8) + 1;
    $('s-touch').hidden = false;
  }
  layout();
  useTool('pencil');
  useSize(1);
  useShade('solid');
  await ready;
  try {
    store = await LocalStore.open(undefined, () => { closed = 'Not kept: the Studio was opened anew in another tab. Reload this one to go on'; status('failed'); });
  } catch (e) {
    store = null;
    say(`Drafts can’t be kept here: ${e.message || e}`);
  }
  if (store) for (const h of (await store.list({ type: 'palette' })).items) { const p = await store.get(h.id); try { check(p); mine.set(p.id, p); } catch { /* not a palette: left out */ } }
  useSwatches(swatch);
  menu();
  let r = null;
  if (store) {
    const lastId = ls.get('last');
    r = lastId && await store.get(lastId);
    if (!r) { const { items } = await store.list({ type: 'piece' }); r = items.length && await store.get(items[0].id); }
  }
  if (!r || !await tryOpen(r)) await fresh();
  await drafts();
  root.removeAttribute('aria-busy');
  route();
  // The starter palettes, once the page is up; read-only, as the starter pack lists them.
  starter().then((list) => {
    starters = list;
    for (const p of list) mine.delete(p.id);
    menu();
    if (isStarter(rec?.body.palette) && swatch.id !== rec.body.palette) useSwatches(builtin(rec.body.palette));
    root.dataset.starter = list.length;
  }).catch(() => { root.dataset.starter = 'none'; });
}
main().catch((e) => {
  say(`The Studio could not start: ${e.message || e}`);
  root.removeAttribute('aria-busy');
});
