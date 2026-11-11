// The Studio (/studio/): draw pixel art for the panel, 128 x 32, keep it as
// a draft in this browser and download it as a GIF. Client-only: nothing is
// uploaded; the page fetches nothing but its own files and, at the first
// download, the demo's art module (its GIF writer).
//
// S1: one frame, one layer. Tools (tools.js): pencil, eraser, line, rectangle
// and ellipse (edge or filled), fill, dither fill, a selection to move, copy,
// flip and delete, a colour picker tool; brushes 1 to 3; each shape seen as
// it is dragged and kept when the pointer lifts. Colours: the piece's own
// (body.colours, any 24-bit colour; past 255 the pixels take two bytes),
// drawn from swatch sets (Den 16, the starter palettes, the user's own).
// Undo (500 steps) is a journal kept in the store, so it survives a reload;
// snapshots are named revisions. The picker and the palette tools load
// when first opened (colour.js). An importer (Create Art's GIF and picture
// conversion, later) adds a piece through `piece()` below, the one way in.
import { LocalStore, Conflict, CAPS, record, newId, packPx, unpackPx } from './store.js';
import { DEFAULT, builtin, ready, starter, isStarter } from './palettes.js';
import * as T from './tools.js';

const W = 128, H = 32, N = W * H;
const STEPS = 500;
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
let px = new Uint16Array(N); // layer 1, frame 1: colour keys (0: off)
let cols = new Map();      // the piece's own colours: key -> 0xRRGGBB
let pending = new Map();   // colours picked, not drawn with yet: key -> 0xRRGGBB
let swatch = builtin(DEFAULT); // the swatch set shown (body.palette)
const mine = new Map();    // the user's own palettes, by id
let starters = [];         // the starter palettes, once fetched
let key = 0, tool = 'pencil', prev = 'pencil', size = 1, zoom = 0, ledView = false;
const filled = { rect: false, ellipse: false };
let steps = [], cur = 0, low = 1; // the journal: steps[i] is step low + i; the first `cur` are done
let stroke = null, sel = null, clip = null, drag = null;
let lastHash = null, timer = 0, nameDirty = false;
let kept = false;          // in the store (a new piece is not, until it is drawn on or named)
let journaled = false, unlock = null, closed = '', persisted = null;
let chain = Promise.resolve(), queued = null;

// Pixel icons (12 x 12), drawn as SVG.
const ICONS = {
  pencil: '.........##.|........#..#|.......#..#.|......#..#..|.....#..#...|....#..#....|...#..#.....|..#..#......|.#..#.......|.###........|.##.........|............',
  eraser: '............|......####..|.....#...##.|....#...#.#.|...#...#..#.|..#...#..#..|.#####..#...|.#...#.#....|.#...##.....|.#####......|............|............',
  fill: '.....#......|....#.#.....|...#...#....|..#.....#...|.#########..|.#########.#|..#######..#|...#####..##|....###...##|.....#.....#|............|............',
  dither: '............|.#.#.#.#.#..|..#.#.#.#.#.|.#.#.#.#.#..|..#.#.#.#.#.|.#.#.#.#.#..|..#.#.#.#.#.|.#.#.#.#.#..|..#.#.#.#.#.|.#.#.#.#.#..|............|............',
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

// --- colours: the piece's own ---
const hexOf = (n) => '#' + n.toString(16).padStart(6, '0');
const toInt = (h) => parseInt(h.slice(1), 16);
const colourList = () => [...cols].sort((a, b) => a[0] - b[0]).map(([k, c]) => ({ k, hex: hexOf(c) }));
const listOf = (body) => body.colours || (builtin(body.palette) || mine.get(body.palette) || builtin(DEFAULT)).body.colours;
const mapOf = (list) => new Map(list.map((c) => [c.k, toInt(c.hex)]));
function keyOf(c) { for (const [k, v] of cols) if (v === c) return k; return 0; }
// A new colour's key: the lowest no colour has, none picked but not yet
// drawn has, and no step in the journal names, undone or not: a step can be
// redone while a paste floats, and its key must still be its colour's.
function freeKey() {
  const used = new Set([...cols.keys(), ...pending.keys()]);
  for (const s of steps) for (const c of [...(s.cb || []), ...(s.ca || [])]) used.add(c.k);
  for (let k = 1; k < 65536; k++) if (!used.has(k)) return k;
  return 0;
}
// A colour picked (a swatch, the picker, a paste): its key, the piece's if
// it has the colour; else a new one, pending: it joins the piece's colours
// in the step that first draws with it, so a pick is never a step of its own.
function keyFor(c) {
  let k = keyOf(c);
  if (k) return k;
  for (const [pk, pc] of pending) if (pc === c) return pk;
  if (cols.size + pending.size >= CAPS.colours || !(k = freeKey())) { say(`A piece holds ${CAPS.colours} colours at most`); return 0; }
  pending.set(k, c);
  return k;
}
const rgbOf = (k) => cols.get(k) ?? pending.get(k) ?? 0;
// Pending colours no longer wanted (not `keep`, not in a paste being placed) are let go.
function letGo(keep) {
  const held = new Set(sel?.fl || []);
  for (const pk of [...pending.keys()]) if (pk !== keep && !held.has(pk)) pending.delete(pk);
}
function pickColour(c) {
  letGo(0);
  const k = keyFor(c);
  if (k) { swatches(); pick(k); }
}
function pick(k) {
  key = cols.has(k) || pending.has(k) ? k : 0;
  for (const b of $('s-sw').children) {
    const on = +b.dataset.k === key;
    b.classList.toggle('on', on);
    b.setAttribute('aria-checked', on);
  }
  root.dataset.key = key;
  root.dataset.colour = key ? hexOf(rgbOf(key)) : 'off';
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
  const sw = $('s-sw'), list = colourList(), more = key && pending.has(key) ? [swatch1(key, hexOf(pending.get(key)))] : [];
  for (const b of more) { b.classList.add('new'); b.title += ' · joins this piece when you draw with it'; }
  sw.replaceChildren(swatch1(0, '#000000'), ...list.map((c) => swatch1(c.k, c.hex)), ...more);
  sw.classList.toggle('many', list.length > 23);
  root.dataset.pending = pending.size; // colours picked or pasted, not drawn with yet
  pick(key);
}
// The swatch set shown, and what it is.
function useSwatches(p) {
  swatch = p;
  const set = $('s-set');
  set.replaceChildren(...p.body.colours.map((c) => swatch1(null, c.hex, c.name)));
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
// What is shown: the pixels, and a selection being moved over them.
function view() {
  if (!sel?.fl) return px;
  const v = px.slice();
  if (sel.cut) T.rect(sel.ox, sel.oy, sel.ox + sel.w - 1, sel.oy + sel.h - 1, true, (x, y) => { v[y * W + x] = 0; });
  floatOnto((i, k) => { v[i] = k; });
  return v;
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
    const c = v[i] ? rgbOf(v[i]) : 0;
    if (v[i]) rgbs.add(c); else off = true;
    d[i * 4] = c >> 16; d[i * 4 + 1] = (c >> 8) & 255; d[i * 4 + 2] = c & 255; d[i * 4 + 3] = 255;
  }
  ctx.putImageData(img, 0, 0);
  const used = new Set(v);
  used.delete(0);
  // The GIF's colours: those drawn, and off (black) if any pixel is.
  const n = used.size, table = rgbs.size + (off && !rgbs.has(0) ? 1 : 0);
  $('s-kind').textContent = `STILL · 128×32 · ${n} COLOUR${n === 1 ? '' : 'S'}`;
  $('s-used').textContent = `Colours used: ${n}`;
  $('s-many').hidden = table <= 256;
  root.dataset.used = n;
  root.dataset.table = table;
  outline();
}
function begin() { stroke = new Map(); }
function set(i, k) {
  if (px[i] === k) return;
  if (!stroke.has(i)) stroke.set(i, px[i]);
  px[i] = k;
}
const plot = (k) => (x, y) => { if (x >= 0 && x < W && y >= 0 && y < H) set(y * W + x, k); };
// A shape seen while it is dragged: what it drew last undone, then drawn again.
function again(draw) {
  if (stroke) for (const [i, b] of stroke) px[i] = b;
  begin();
  draw();
  paint();
}
// A second finger, or the pointer lost: nothing is made.
function cancel() {
  if (stroke) { for (const [i, b] of stroke) px[i] = b; stroke = null; }
  if (drag?.was) Object.assign(sel, drag.was);
  if (drag?.marquee) sel = null;
  drag = null;
  showSel();
  paint();
}
// A stroke made: one step, with any picked colour it is the first to draw in.
function end(more = {}) {
  const s = stroke;
  stroke = null;
  if (!s || (!s.size && !more.ca)) return;
  const i = Int32Array.from(s.keys()), a = Uint16Array.from(i, (j) => px[j]);
  const join = [...new Set(a)].filter((k) => pending.has(k));
  if (join.length && !more.ca) {
    const cb = colourList();
    for (const k of join) { cols.set(k, pending.get(k)); pending.delete(k); }
    more = { ...more, cb, ca: colourList() };
    swatches();
  }
  push({ i, b: Uint16Array.from(s.values()), a, ...more });
}
// A step done: kept in the journal (and the store's), the redo steps dropped.
function push(s) {
  steps.length = cur;
  s.n = low + cur;
  steps.push(s);
  cur++;
  let cut = 0;
  if (steps.length > STEPS) { steps.shift(); cur--; low++; cut = low; }
  kept = true;
  jot({ step: s, cur: low + cur - 1, low: cut });
  buttons();
  queueSave();
}
function apply(s, fwd) {
  if (s.i) { const v = fwd ? s.a : s.b; for (let n = 0; n < s.i.length; n++) px[s.i[n]] = v[n]; }
  if (s.ca) {
    const was = cols;
    cols = mapOf(fwd ? s.ca : s.cb);
    // The colour picked, undone out of the piece, stays picked.
    if (key && !cols.has(key) && was.has(key)) pending.set(key, was.get(key));
    if (cols.has(key)) pending.delete(key);
    swatches();
  }
}
function undo(fwd) {
  if (stroke || drag) return; // not during a stroke
  if (!fwd && sel?.fl) { sel = null; letGo(key); swatches(); showSel(); paint(); return; } // a selection being moved (or a paste): put back, its new colours let go
  if (fwd ? cur >= steps.length : !cur) return;
  apply(steps[fwd ? cur : cur - 1], fwd);
  cur += fwd ? 1 : -1;
  jot({ cur: low + cur - 1 });
  buttons();
  paint();
  queueSave();
}
function buttons() {
  $('s-undo').disabled = !cur;
  $('s-redo').disabled = cur >= steps.length;
  root.dataset.steps = `${cur}/${steps.length}`;
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
  root.dataset.pending = pending.size;
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
  for (let j = 0; j < sel.h; j++) for (let i = 0; i < sel.w; i++) sel.fl[j * sel.w + i] = px[(sel.y + j) * W + sel.x + i];
  Object.assign(sel, { cut: true, ox: sel.x, oy: sel.y });
}
// The floating pixels put down (one step; off in them is see-through); the box stays, on the panel.
function drop() {
  if (!sel?.fl) return;
  begin();
  if (sel.cut) T.rect(sel.ox, sel.oy, sel.ox + sel.w - 1, sel.oy + sel.h - 1, true, plot(0));
  floatOnto(set);
  end();
  const x = Math.max(0, sel.x), y = Math.max(0, sel.y), w = Math.min(W, sel.x + sel.w) - x, h = Math.min(H, sel.y + sel.h) - y;
  sel = w > 0 && h > 0 ? { x, y, w, h } : null;
}
function done() { drop(); sel = null; showSel(); paint(); }
function act(a) {
  if (a === 'paste') {
    if (!clip) return;
    if (tool !== 'select') useTool('select');
    drop();
    // Its colours join the piece in the step that puts it down; it lands in the middle of what is in view.
    const fl = Uint16Array.from(clip.c, (c) => (c ? keyFor(c - 1) : 0));
    const r = scroll.getBoundingClientRect(), c = cv.getBoundingClientRect(), s = c.width / W;
    const mx = (Math.max(r.left, c.left) + Math.min(r.right, c.right)) / 2, my = (Math.max(r.top, c.top, 0) + Math.min(r.bottom, c.bottom, innerHeight)) / 2;
    const x = Math.round((mx - c.left) / s - clip.w / 2), y = Math.round((my - c.top) / s - clip.h / 2);
    sel = { x: Math.max(0, Math.min(W - clip.w, x)), y: Math.max(0, Math.min(H - clip.h, y)), w: clip.w, h: clip.h, fl, cut: false };
  } else if (a === 'done') done();
  else if (sel) {
    if (a === 'copy' || a === 'cut') {
      const keys = sel.fl || Uint16Array.from({ length: sel.w * sel.h }, (_, n) => px[(sel.y + Math.floor(n / sel.w)) * W + sel.x + n % sel.w]);
      clip = { w: sel.w, h: sel.h, c: Uint32Array.from(keys, (k) => (k ? rgbOf(k) + 1 : 0)) }; // colours, so it pastes into any piece
    }
    if (a === 'cut' || a === 'del') {
      // A box lifted from the panel: its place cleared. A paste not put down yet: gone.
      if (sel.fl) sel = sel.cut ? { x: sel.ox, y: sel.oy, w: sel.w, h: sel.h } : null;
      if (sel) { begin(); T.rect(sel.x, sel.y, sel.x + sel.w - 1, sel.y + sel.h - 1, true, plot(0)); end(); }
      sel = null;
      letGo(key);
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
  const [x0, y0] = drag.p, k = key;
  let [x1, y1] = q;
  if (shift) [x1, y1] = tool === 'line' ? T.snap(x0, y0, x1, y1) : T.square(x0, y0, x1, y1);
  drag.q = q;
  const f = tool === 'rect' ? T.rect : T.ellipse;
  again(() => {
    if (tool === 'line') T.line(x0, y0, x1, y1, T.brush(size, plot(k)));
    else if (filled[tool]) f(x0, y0, x1, y1, true, plot(k));
    else f(x0, y0, x1, y1, false, T.brush(size, plot(k)));
  });
}
const mid = (a, b) => ({ x: (a.clientX + b.clientX) / 2, y: (a.clientY + b.clientY) / 2, d: Math.hypot(a.clientX - b.clientX, a.clientY - b.clientY) || 1 });
cv.addEventListener('contextmenu', (e) => e.preventDefault());
cv.addEventListener('pointerdown', (e) => {
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
  if (tool === 'fill' || tool === 'dither') { drag = { p, fill: true }; return; }
  if (tool === 'select') {
    if (within(p)) { lift(); drag = { p, was: { x: sel.x, y: sel.y } }; } else { drop(); sel = { x: p[0], y: p[1], w: 1, h: 1 }; drag = { p, marquee: true }; }
    showSel();
    paint();
    return;
  }
  drag = { p, q: p };
  if (brushed() && tool !== 'line') {
    begin();
    T.brush(size, plot(tool === 'eraser' ? 0 : key))(p[0], p[1]);
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
    if (!stroke) return;
    T.line(drag.q[0], drag.q[1], p[0], p[1], T.brush(size, plot(tool === 'eraser' ? 0 : key)));
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
      begin();
      const [x, y] = drag.p, k = key, dot = tool === 'dither';
      for (const i of T.flood(px, W, H, x, y)) if (!dot || T.checker(i % W, (i / W) | 0)) set(i, k);
    }
    end();
    if (drag.marquee && sel?.w === 1 && sel.h === 1) sel = null; // a click: nothing selected
    drag = null;
    showSel();
    paint();
  } else if (!down.size && (drag || stroke)) cancel();
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
  // Edge or filled, for a shape (a phone has no Shift).
  $('s-fillmode').hidden = !(t in filled);
  for (const b of $('s-fillmode').children) { const on = +b.dataset.fill === +!!filled[t]; b.classList.toggle('on', on); b.setAttribute('aria-pressed', on); }
  showSel();
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
// What a piece is made of, as the store keeps it: a download's `shot`, or now.
async function bodyOf(shot) {
  const list = shot ? shot.colours : colourList(), wide = list.some((c) => c.k > 255);
  const cel = await packPx(shot ? shot.px : px, wide);
  const { keys, ...b } = rec.body;
  return { ...b, ...(wide ? { keys: 2 } : {}), palette: swatch.id, colours: list,
    layers: b.layers.map((l, n) => (n ? l : { ...l, cels: [cel, ...l.cels.slice(1)] })) };
}
async function saveNow(shot) {
  if (!store || !rec || !kept || closed) return;
  try {
    let at = shot ? shot.at : low + cur - 1, note = null; // where the journal is as these pixels are taken
    let next = { ...rec, name: nameNow(), body: await bodyOf(shot) };
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
        steps = []; cur = 0; low = 1; at = 0; buttons(); // the copy starts a history of its own
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

// A stored record opened, and its journal replayed: from where the record
// stands (`at`) to where undo was (`cur`).
async function open(r) {
  const b = r.body, keys = await unpackPx(b.layers[0].cels[0], N, b.keys === 2); // throws on a draft that does not read
  sel = null;
  showSel();
  rec = r;
  px = Uint16Array.from(keys);
  cols = mapOf(listOf(b));
  pending = new Map();
  useSwatches(builtin(b.palette) || mine.get(b.palette) || builtin(DEFAULT));
  kept = true;
  lastHash = r.hash;
  nameDirty = false;
  steps = []; cur = 0; low = 1;
  await lock(r.id);
  if (journaled && store && r.hash) {
    const j = await store.steps(r.id), lo = j.list[0]?.n, hi = lo + j.list.length - 1, ok = (n) => n >= lo - 1 && n <= hi;
    if (j.list.length && j.hash === r.hash && j.list.every((s, n) => s.n === lo + n) && ok(j.at) && ok(j.cur)) {
      steps = j.list; low = lo; cur = j.cur - lo + 1;
      for (let n = j.at + 1; n <= j.cur; n++) apply(steps[n - lo], true);
      for (let n = j.at; n > j.cur; n--) apply(steps[n - lo], false);
      if (j.at !== j.cur) queueSave();
    } else if (j.list.length) await run(() => store.forget(r.id)); // written elsewhere since: no longer this record's
  }
  buttons();
  $('s-name').value = r.name;
  key = keyOf(AMBER) || keyFor(AMBER);
  swatches();
  paint();
  ls.set('last', r.id);
  root.dataset.id = r.id;
  root.dataset.rev = r.rev;
  if (root.dataset.saved !== 'saving') status('kept');
  await snaps();
}
// A new piece, with no colours until it is drawn on: the one way a piece
// comes in (an importer, later, hands over its pixels, colours and `from`
// here). Its layers and frames get stable keys as they are made.
const k8 = () => [...crypto.getRandomValues(new Uint8Array(4))].map((x) => x.toString(16).padStart(2, '0')).join('');
export async function piece({ name = 'Untitled', keys = new Uint8Array(N), colours = [], from = [] } = {}) {
  const wide = colours.some((c) => c.k > 255);
  const r = record('piece', name, { w: W, h: H, palette: swatch.id, plays: 0, colours, ...(wide ? { keys: 2 } : {}), frames: [{ key: k8(), ms: STILL_MS }],
    layers: [{ key: k8(), name: 'Drawing', moves: false, cels: [await packPx(keys, wide)] }] }, { from });
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
    say(`“${r.name}” can’t be opened: its picture is damaged. It stays in Drafts; this is a new piece.`);
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
  thumb(li.querySelector('canvas'), px, cols);
  root.dataset.drawn = (+root.dataset.drawn || 0) + 1;
}
async function drafts() {
  const ul = $('s-list');
  if (!store) { ul.replaceChildren(); return; }
  const { items } = await store.list({ type: 'piece' });
  if (rec && !kept) items.unshift({ id: rec.id, name: nameNow(), modified: rec.modified });
  const lis = await Promise.all(items.map(async (h) => {
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
      if (open) thumb(c, px, cols);
      else { const r = await store.get(h.id); thumb(c, await unpackPx(r.body.layers[0].cels[0], N, r.body.keys === 2), mapOf(listOf(r.body))); }
    } catch { /* a draft that does not read: listed, not drawn */ }
    return li;
  }));
  ul.replaceChildren(...lis);
  root.dataset.drawn = 0;
}

// --- snapshots: named revisions (store.snapshot), restored as one undo step ---
async function snaps() {
  const list = store && kept && rec ? (await store.revs(rec.id)).reverse() : [];
  $('s-snaps').replaceChildren(...list.map((h) => {
    const li = el('li'), t = el('span', 's-meta'), b = el('button', 's-pill', 'Restore');
    t.append(el('b', '', h.label || 'Downloaded'), el('small', '', `${ago(h.modified)} · version ${h.rev}`));
    b.dataset.rev = h.rev;
    b.title = 'Restore it: what is here now is kept as a snapshot first';
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
  const keys = await unpackPx(r.body.layers[0].cels[0], N, r.body.keys === 2);
  done();
  await freeze(`Before restoring ${r.label || `version ${rev}`}`.slice(0, CAPS.name));
  const cb = colourList(), ca = listOf(r.body).map(({ k, hex }) => ({ k, hex }));
  begin();
  for (let i = 0; i < N; i++) set(i, keys[i]);
  cols = mapOf(ca);
  end({ cb, ca });
  swatches();
  paint();
  await snaps();
}

// --- the GIF ---
// The GIF writers: the site's own wasm module (pkg-studio, opendmd-studio-web; never on the panel), loaded at the first download.
let writer = null;
const gifWriter = () => (writer ??= import('../demo/js/wasm.js').then((w) => w.load('pkg-studio')).then(([m]) => m)
  .catch((e) => { writer = null; throw e; }));
// fit: 256 colours picked for spread, the rest as their nearest (a GIF the panel can prepare and smooth).
async function download(fit) {
  const b = $(fit ? 's-fit' : 's-gif');
  b.disabled = true;
  try {
    const m = await gifWriter();
    done();
    // What is downloaded is what is frozen: these pixels, saved and snapshotted in the chain, then read back.
    const shot = { px: px.slice(), colours: colourList(), at: low + cur - 1 };
    const h = await freeze('', shot), frozen = h && await store.get(rec.id, h.rev - 1);
    const body = frozen?.body || { ...rec.body, colours: shot.colours };
    const keys = frozen ? await unpackPx(body.layers[0].cels[0], N, body.keys === 2) : shot.px, cm = mapOf(listOf(body));
    // The GIF's table: only the colours used, off first, the same colour once.
    const table = [], index = new Map(), of = new Map();
    for (const k of [...new Set(keys)].sort((a, c) => a - c)) {
      const c = k ? cm.get(k) ?? 0 : 0;
      if (!index.has(c)) { index.set(c, table.length); table.push(c); }
      of.set(k, index.get(c));
    }
    const delays = Uint32Array.from(body.frames, (f) => f.ms);
    // Past 256 colours: a whole picture over 256 of them picked for spread, then the rest exactly (or, fitted, only that picture).
    const rgb = () => Uint32Array.from(keys, (k) => (k ? cm.get(k) ?? 0 : 0));
    const gif = table.length <= 256 ? m.encodeIndexed(Uint32Array.from(table), Uint8Array.from(keys, (k) => of.get(k)), delays, body.plays)
      : fit ? m.encodeFitted(rgb(), delays, body.plays) : m.encodeWide(rgb(), delays, body.plays);
    const stem = nameNow() + (fit && table.length > 256 ? ' 256' : '');
    const name = (stem.replace(/[^\p{L}\p{N} _-]+/gu, '').trim().replace(/\s+/g, '_') || 'drawing') + '.gif';
    const a = el('a');
    a.href = URL.createObjectURL(new Blob([gif], { type: 'image/gif' }));
    a.download = name;
    document.body.append(a);
    a.click();
    a.remove();
    setTimeout(() => URL.revokeObjectURL(a.href), 30000);
    root.dataset.gif = gif.length;
    say('');
    const n = fit ? Math.min(table.length, 256) : table.length;
    if (root.dataset.saved !== 'failed') status(kept ? 'kept' : 'new', `Downloaded ${name}: ${n} colour${n === 1 ? '' : 's'}, ${(gif.length / 1024).toFixed(1)} KB`);
    await snaps();
  } catch (e) {
    say(`The GIF could not be made: ${e.message || e}`);
  } finally {
    b.disabled = false;
  }
}

// --- the colour picker and the palette tools: colour.js, loaded when first opened ---
let colourMod = null;
const studio = {
  get store() { return store; }, run, mine, say, swatch: () => swatch, colours: colourList, current: () => (key ? rgbOf(key) : null),
  pickColour, piece: () => rec && kept && lastHash ? { id: rec.id, rev: rec.rev, hash: lastHash, name: nameNow() } : null,
  use(p) { if (p) { useSwatches(p); queueSave(); } menu(); },
};
async function colourTools(which, b) {
  colourMod ??= import('./colour.js');
  const m = await colourMod, host = $(which === 'picker' ? 's-picker' : 's-palx');
  host.hidden = !host.hidden;
  b.setAttribute('aria-expanded', !host.hidden);
  if (!host.hidden) m[which](host, studio);
}
function unused() {
  done();
  const used = new Set(px), cb = colourList(), ca = cb.filter((c) => used.has(c.k));
  if (ca.length === cb.length) return;
  cols = mapOf(ca);
  push({ cb, ca });
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
  $('s-selbar').addEventListener('click', (e) => { const b = e.target.closest('[data-act]'); if (b) act(b.dataset.act); });
  $('s-undo').addEventListener('click', () => undo(false));
  $('s-redo').addEventListener('click', () => undo(true));
  $('s-gif').addEventListener('click', () => download(false));
  $('s-fit').addEventListener('click', () => download(true));
  $('s-fillmode').addEventListener('click', (e) => { const b = e.target.closest('[data-fill]'); if (b) useTool(tool, b.dataset.fill === '1'); });
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
    if (d && confirm(`Delete ${d.title.slice(7)}? It can’t be brought back.`)) {
      const id = d.dataset.id;
      if (id === rec.id) { clearTimeout(timer); kept = false; }
      await run(() => store.remove(id));
      if (id === rec.id) {
        const { items } = await store.list({ type: 'piece' });
        const r = items.length && await store.get(items[0].id);
        if (!r || !await tryOpen(r)) await fresh();
      }
      await drafts();
    }
  });
  $('s-name').addEventListener('input', () => { kept = true; nameDirty = true; queueSave(); });
  $('s-v-px').addEventListener('click', () => viewLeds(false));
  $('s-v-led').addEventListener('click', () => viewLeds(true));
  $('s-g-grid').addEventListener('click', () => { const on = !$('s-g-grid').classList.contains('on'); toggle('s-g-grid', on); $('s-grid').hidden = !on; });
  $('s-g-seam').addEventListener('click', () => { const on = !$('s-g-seam').classList.contains('on'); toggle('s-g-seam', on); $('s-seam').hidden = !on; });
  $('s-z-in').addEventListener('click', () => zoomBy(1));
  $('s-z-out').addEventListener('click', () => zoomBy(-1));
  new ResizeObserver(layout).observe(scroll);
  const TOOLS = { b: 'pencil', e: 'eraser', l: 'line', r: 'rect', o: 'ellipse', g: 'fill', d: 'dither', m: 'select', i: 'pick' };
  const CLIP = { c: 'copy', x: 'cut', v: 'paste' };
  const shapes = () => drag?.q && /line|rect|ellipse/.test(tool);
  document.addEventListener('keydown', (e) => {
    if (e.target.closest('input, select, textarea')) return;
    const k = e.key.toLowerCase(), mod = e.ctrlKey || e.metaKey;
    if (mod && k === 'z') { e.preventDefault(); undo(e.shiftKey); }
    else if (mod && k === 'y') { e.preventDefault(); undo(true); }
    else if (mod && CLIP[k] && (sel || k === 'v')) { e.preventDefault(); act(CLIP[k]); }
    else if (mod && k === 'a') { e.preventDefault(); useTool('select'); drop(); sel = { x: 0, y: 0, w: W, h: H }; showSel(); paint(); }
    else if (mod || e.altKey || drag) { if (k === 'shift' && shapes()) shape(drag.q, true); }
    else if (e.shiftKey && (k === 'h' || k === 'v') && sel) act(k === 'h' ? 'fliph' : 'flipv');
    else if (TOOLS[k]) useTool(TOOLS[k], TOOLS[k] in filled ? e.shiftKey : undefined);
    else if (k === '1' || k === '2' || k === '3') useSize(+k);
    else if ((k === 'delete' || k === 'backspace') && sel) { e.preventDefault(); act('del'); }
    else if ((k === 'enter' || k === 'escape') && sel) act('done');
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
  await ready;
  try {
    store = await LocalStore.open(undefined, () => { closed = 'Not kept: the Studio was opened anew in another tab. Reload this one to go on'; status('failed'); });
  } catch (e) {
    store = null;
    say(`Drafts can’t be kept here: ${e.message || e}`);
  }
  if (store) for (const h of (await store.list({ type: 'palette' })).items) { const p = await store.get(h.id); if (p) mine.set(p.id, p); }
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
