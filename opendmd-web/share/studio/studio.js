// The Studio (/studio/): draw pixel art for the panel, 128 x 32, keep it as
// a draft in this browser and download it as a GIF. Client-only: nothing is
// uploaded; the page fetches nothing but its own files and, at the first
// download, the demo's art module (its GIF writer, `encodeIndexed`).
//
// S0: one frame, one layer, the pencil, the eraser and fill, the starter
// palettes, the panel's seam and a grid, the LED view (the site's own .led
// look), undo (500 steps, this visit), drafts kept by store.js (the record
// envelope; LocalStore on IndexedDB) and saved as you go.
import { LocalStore, Conflict, record, newId, packPx, unpackPx } from './store.js';
import { STARTER, DEFAULT, builtin } from './palettes.js';

const W = 128, H = 32, N = W * H;
const STEPS = 500;
const ZOOMS = [2, 3, 4, 6, 8, 12, 16, 24, 32];
const LS = 'odmd.studio.';
const $ = (id) => document.getElementById(id);
const root = $('studio'), cv = $('s-canvas'), ctx = cv.getContext('2d'), work = $('s-work'), scroll = $('s-scroll');

let store = null;
let rec = null;          // the piece's record (its pixels packed only when saved)
let px = new Uint8Array(N); // layer 1, frame 1: palette keys (0: off)
let pal = builtin(DEFAULT), rgb = new Uint32Array(256);
let key = 1, tool = 'pencil', zoom = 0, ledView = false;
let undo = [], redo = [], stroke = null;
let lastHash = null, timer = 0, saving = null, again = false;

// Pixel icons (12 x 12), drawn as SVG.
const ICONS = {
  pencil: '.........##.|........#..#|.......#..#.|......#..#..|.....#..#...|....#..#....|...#..#.....|..#..#......|.#..#.......|.###........|.##.........|............',
  eraser: '............|......####..|.....#...##.|....#...#.#.|...#...#..#.|..#...#..#..|.#####..#...|.#...#.#....|.#...##.....|.#####......|............|............',
  fill: '.....#......|....#.#.....|...#...#....|..#.....#...|.#########..|.#########.#|..#######..#|...#####..##|....###...##|.....#.....#|............|............',
  undo: '............|....#.......|...##.......|..#######...|...##....#..|....#.....#.|..........#.|.........#..|....#####...|............|............|............',
  redo: '............|.......#....|.......##...|...#######..|..#....##...|.#.....#....|.#..........|..#.........|...#####....|............|............|............',
};
function icons() {
  for (const e of document.querySelectorAll('[data-i]')) {
    let s = '';
    ICONS[e.dataset.i].split('|').forEach((r, y) => [...r].forEach((c, x) => { if (c === '#') s += `<rect x="${x}" y="${y}" width="1" height="1"/>`; }));
    e.insertAdjacentHTML('afterbegin', `<svg viewBox="0 0 12 12" width="24" height="24" shape-rendering="crispEdges" fill="currentColor" aria-hidden="true">${s}</svg>`);
  }
}

// --- the palette ---
const hexOf = (n) => '#' + n.toString(16).padStart(6, '0');
function usePalette(p) {
  pal = p;
  rgb = new Uint32Array(256);
  for (const c of p.body.colours) rgb[c.k] = parseInt(c.hex.slice(1), 16);
  const sw = $('s-sw');
  sw.replaceChildren();
  sw.classList.toggle('many', p.body.colours.length > 24);
  const one = (k, name, hex) => {
    const b = document.createElement('button');
    b.className = 's-sw' + (k === 0 ? ' off' : '');
    b.style.background = hex;
    b.dataset.k = k;
    b.title = k ? `${name ? name + ' · ' : ''}${hex}` : 'Off: the LED dark';
    b.setAttribute('role', 'radio');
    b.setAttribute('aria-label', k ? name || hex : 'Off');
    sw.append(b);
  };
  one(0, 'Off', '#000000');
  for (const c of p.body.colours) one(c.k, c.name, c.hex);
  $('s-pal').value = p.id;
  $('s-pal-about').textContent = `${p.body.colours.length} colours and off · ${p.body.source}`;
  pick(rgb[key] || !key ? key : 1);
}
function pick(k) {
  key = k;
  for (const b of $('s-sw').children) {
    const on = +b.dataset.k === k;
    b.classList.toggle('on', on);
    b.setAttribute('aria-checked', on);
  }
}
const dist = (a, b) => {
  const dr = (a >> 16) - (b >> 16), dg = ((a >> 8) & 255) - ((b >> 8) & 255), db = (a & 255) - (b & 255);
  return 2 * dr * dr + 4 * dg * dg + 3 * db * db;
};
function nearest(p, c) {
  let best = p.body.colours[0].k, d = Infinity;
  for (const x of p.body.colours) {
    const e = dist(parseInt(x.hex.slice(1), 16), c);
    if (e < d) { d = e; best = x.k; }
  }
  return best;
}
// Another palette: every colour drawn moves to its nearest there (one undo step).
function switchPalette(p) {
  const map = new Uint8Array(256);
  for (let k = 1; k < 256; k++) map[k] = rgb[k] || pal.body.colours.some((c) => c.k === k) ? nearest(p, rgb[k]) : 0;
  const was = pal.id, k0 = key ? nearest(p, rgb[key]) : 0;
  begin();
  for (let i = 0; i < N; i++) if (px[i]) set(i, map[px[i]]);
  end({ pal: [was, p.id] });
  rec.body.palette = p.id;
  usePalette(p);
  pick(k0);
  paint();
  queueSave();
}

// --- drawing ---
function paint() {
  const img = ctx.createImageData(W, H), d = img.data;
  for (let i = 0; i < N; i++) {
    const c = px[i] ? rgb[px[i]] : 0;
    d[i * 4] = c >> 16; d[i * 4 + 1] = (c >> 8) & 255; d[i * 4 + 2] = c & 255; d[i * 4 + 3] = 255;
  }
  ctx.putImageData(img, 0, 0);
  const used = new Set(px).size;
  $('s-kind').textContent = `STILL · 128×32 · ${used} COLOUR${used === 1 ? '' : 'S'}`;
}
function begin() { stroke = new Map(); }
function set(i, k) {
  if (px[i] === k) return;
  if (!stroke.has(i)) stroke.set(i, px[i]);
  px[i] = k;
}
function end(extra = {}) {
  const s = stroke;
  stroke = null;
  if (!s || (!s.size && !extra.pal)) return;
  const i = Int32Array.from(s.keys());
  undo.push({ i, b: Uint8Array.from(s.values()), a: Uint8Array.from(i, (j) => px[j]), ...extra });
  if (undo.length > STEPS) undo.shift();
  redo = [];
  buttons();
  queueSave();
}
function line(x0, y0, x1, y1, k) {
  const dx = Math.abs(x1 - x0), dy = -Math.abs(y1 - y0), sx = x0 < x1 ? 1 : -1, sy = y0 < y1 ? 1 : -1;
  let e = dx + dy;
  for (;;) {
    if (x0 >= 0 && x0 < W && y0 >= 0 && y0 < H) set(y0 * W + x0, k);
    if (x0 === x1 && y0 === y1) break;
    const e2 = 2 * e;
    if (e2 >= dy) { e += dy; x0 += sx; }
    if (e2 <= dx) { e += dx; y0 += sy; }
  }
}
function fill(x, y, k) {
  const from = px[y * W + x];
  if (from === k) return;
  const todo = [y * W + x];
  while (todo.length) {
    const i = todo.pop();
    if (px[i] !== from) continue;
    set(i, k);
    const cx = i % W;
    if (cx > 0) todo.push(i - 1);
    if (cx < W - 1) todo.push(i + 1);
    if (i >= W) todo.push(i - W);
    if (i < N - W) todo.push(i + W);
  }
}
function step(list, other, dir) {
  const s = list.pop();
  if (!s) return;
  const v = dir ? s.a : s.b;
  for (let n = 0; n < s.i.length; n++) px[s.i[n]] = v[n];
  if (s.pal) {
    const p = builtin(s.pal[dir ? 1 : 0]);
    if (p) { rec.body.palette = p.id; usePalette(p); }
  }
  other.push(s);
  buttons();
  paint();
  queueSave();
}
function buttons() {
  $('s-undo').disabled = !undo.length;
  $('s-redo').disabled = !redo.length;
}

// Pointers: one draws; a second one cancels the stroke and pans (zoomed in).
const down = new Map();
let last = null, pan = null;
function at(e) {
  const r = cv.getBoundingClientRect();
  return [Math.floor((e.clientX - r.left) / r.width * W), Math.floor((e.clientY - r.top) / r.height * H)];
}
const inside = ([x, y]) => x >= 0 && x < W && y >= 0 && y < H;
cv.addEventListener('contextmenu', (e) => e.preventDefault());
cv.addEventListener('pointerdown', (e) => {
  cv.setPointerCapture(e.pointerId);
  down.set(e.pointerId, e);
  if (down.size === 2) {
    if (stroke) {
      for (const [i, b] of stroke) px[i] = b;
      stroke = null;
      paint();
    }
    const [a, b] = [...down.values()];
    pan = { x: (a.clientX + b.clientX) / 2, y: (a.clientY + b.clientY) / 2, l: scroll.scrollLeft, t: scroll.scrollTop };
    return;
  }
  if (down.size > 1) return;
  const p = at(e);
  if (!inside(p)) return;
  if (e.button === 2 || e.altKey) { pick(px[p[1] * W + p[0]]); return; }
  if (e.button !== 0) return;
  begin();
  if (tool === 'fill') {
    fill(p[0], p[1], key);
    end();
  } else {
    line(p[0], p[1], p[0], p[1], tool === 'eraser' ? 0 : key);
    last = p;
  }
  paint();
});
cv.addEventListener('pointermove', (e) => {
  const p = at(e);
  hover(p);
  if (!down.has(e.pointerId)) return;
  down.set(e.pointerId, e);
  if (pan && down.size === 2) {
    const [a, b] = [...down.values()];
    scroll.scrollLeft = pan.l - ((a.clientX + b.clientX) / 2 - pan.x);
    scroll.scrollTop = pan.t - ((a.clientY + b.clientY) / 2 - pan.y);
    return;
  }
  if (!stroke || !last) return;
  line(last[0], last[1], p[0], p[1], tool === 'eraser' ? 0 : key);
  last = p;
  paint();
});
const up = (e) => {
  down.delete(e.pointerId);
  if (!down.size) pan = null;
  if (stroke && tool !== 'fill') end();
  stroke = null;
  last = null;
};
cv.addEventListener('pointerup', up);
cv.addEventListener('pointercancel', up);
cv.addEventListener('pointerleave', () => { $('s-cursor').hidden = true; $('s-xy').innerHTML = 'X <b>–</b> Y <b>–</b>'; });
function hover([x, y]) {
  const c = $('s-cursor');
  if (!inside([x, y]) || ledView) { c.hidden = true; return; }
  const s = work.clientWidth / W;
  c.hidden = false;
  c.style.cssText = `left:${x * s}px;top:${y * s}px;width:${s}px;height:${s}px`;
  $('s-xy').innerHTML = `X <b>${x}</b> Y <b>${y}</b>`;
}

function useTool(t) {
  tool = t;
  for (const b of document.querySelectorAll('.s-tool')) {
    b.classList.toggle('on', b.dataset.tool === t);
    b.setAttribute('aria-pressed', b.dataset.tool === t);
  }
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
}
function toggle(id, on) {
  const b = $(id);
  b.classList.toggle('on', on);
  b.setAttribute('aria-pressed', on);
}
function view(led) {
  ledView = led;
  work.classList.toggle('led', led);
  toggle('s-v-px', !led);
  toggle('s-v-led', led);
  localStorage.setItem(LS + 'view', led ? 'led' : 'px');
}

// --- keeping it: autosave into the store ---
function status(what, text) {
  root.dataset.saved = what;
  $('s-saved').textContent = text || { kept: 'Kept in this browser', saving: 'Saving…', none: 'Not kept: this browser won’t keep drafts here' }[what];
}
function queueSave() {
  clearTimeout(timer);
  timer = setTimeout(save, 400);
  if (store) status('saving');
}
async function save() {
  clearTimeout(timer);
  if (!store || !rec) return;
  if (saving) { again = true; return saving; }
  saving = (async () => {
    const cel = await packPx(px);
    const body = { ...rec.body, layers: rec.body.layers.map((l, n) => (n ? l : { ...l, cels: [cel, ...l.cels.slice(1)] })) };
    const next = { ...rec, name: $('s-name').value.trim() || 'Untitled', body };
    try {
      const h = await store.put(next, { baseRev: lastHash ? rec.rev : null, baseHash: lastHash });
      rec = { ...next, hash: h.hash, modified: h.modified };
      lastHash = h.hash;
      localStorage.setItem(LS + 'last', rec.id);
      status('kept');
    } catch (e) {
      if (!(e instanceof Conflict)) { status('failed', `Not kept: ${e.message}`); return; }
      // Changed in another tab: this one's work is kept as a copy that says where it came from.
      const was = e.head;
      rec = { ...next, id: newId('piece'), rev: 1, name: `${next.name} (copy)`,
        from: [{ id: was.id, rev: was.rev, name: was.name || '', author: was.author || { name: '', account: null }, licence: was.licence ?? null }] };
      lastHash = null;
      $('s-name').value = rec.name;
      const h = await store.put(rec, {});
      rec.hash = lastHash = h.hash;
      root.dataset.id = rec.id;
      localStorage.setItem(LS + 'last', rec.id);
      status('kept', 'Changed in another tab: yours is kept as a copy');
    }
    await drafts();
  })();
  await saving;
  saving = null;
  if (again) { again = false; await save(); }
}

async function open(r) {
  rec = r;
  px = await unpackPx(r.body.layers[0].cels[0], N);
  lastHash = r.hash;
  undo = []; redo = [];
  buttons();
  $('s-name').value = r.name;
  usePalette(builtin(r.body.palette) || builtin(DEFAULT));
  paint();
  localStorage.setItem(LS + 'last', r.id);
  root.dataset.id = r.id;
  status('kept');
}
async function fresh() {
  const r = record('piece', 'Untitled', { w: W, h: H, palette: pal?.id || DEFAULT, plays: 0, frames: [{ ms: 1000 }],
    layers: [{ name: 'Drawing', moves: false, cels: [await packPx(new Uint8Array(N))] }] });
  await open({ ...r, hash: '' });
  lastHash = null;
  if (store) await save();
  else status('none');
  $('s-name').select();
}

// The drafts, newest first, each drawn as LEDs.
const ago = (iso) => {
  const s = (Date.now() - Date.parse(iso)) / 1000;
  return s < 60 ? 'just now' : s < 3600 ? `${Math.floor(s / 60)} min ago` : s < 86400 ? `${Math.floor(s / 3600)} h ago` : new Date(iso).toLocaleDateString();
};
async function drafts() {
  const ul = $('s-list');
  if (!store) { ul.replaceChildren(); return; }
  const { items } = await store.list({ type: 'piece' });
  const lis = await Promise.all(items.map(async (h) => {
    const r = h.id === rec?.id ? { ...rec, body: rec.body } : await store.get(h.id);
    const li = document.createElement('li');
    li.className = h.id === rec?.id ? 'on' : '';
    const b = document.createElement('button');
    b.className = 's-open';
    b.dataset.id = h.id;
    const led = document.createElement('span');
    led.className = 'led';
    const c = document.createElement('canvas');
    c.width = W; c.height = H;
    led.append(c);
    const t = document.createElement('span');
    t.className = 's-meta';
    t.innerHTML = '<b></b><small></small>';
    t.firstChild.textContent = h.name;
    t.lastChild.textContent = h.id === rec?.id ? 'Open now' : `Edited ${ago(h.modified)}`;
    b.append(led, t);
    const del = document.createElement('button');
    del.className = 's-del';
    del.dataset.id = h.id;
    del.title = `Delete ${h.name}`;
    del.setAttribute('aria-label', `Delete ${h.name}`);
    del.textContent = '×';
    li.append(b, del);
    try {
      const keys = h.id === rec?.id ? px : await unpackPx(r.body.layers[0].cels[0], N);
      const p = builtin(r.body.palette) || builtin(DEFAULT), m = new Uint32Array(256);
      for (const x of p.body.colours) m[x.k] = parseInt(x.hex.slice(1), 16);
      const g = c.getContext('2d'), img = g.createImageData(W, H);
      for (let i = 0; i < N; i++) { const v = keys[i] ? m[keys[i]] : 0; img.data.set([v >> 16, (v >> 8) & 255, v & 255, 255], i * 4); }
      g.putImageData(img, 0, 0);
    } catch { /* a draft that does not read: listed, not drawn */ }
    return li;
  }));
  ul.replaceChildren(...lis);
}

// --- the GIF ---
let art = null;
async function download() {
  const b = $('s-gif');
  b.disabled = true;
  try {
    await save();
    art ??= import('../demo/js/artmod.js').then((m) => m.art());
    const m = await art;
    // The GIF's table: only the colours used, off first, the same colour once.
    const table = [], index = new Map(), of = new Uint8Array(256);
    for (const k of [...new Set(px)].sort((a, c) => a - c)) {
      const c = k ? rgb[k] : 0;
      if (!index.has(c)) { index.set(c, table.length); table.push(c); }
      of[k] = index.get(c);
    }
    const frame = Uint8Array.from(px, (k) => of[k]);
    const gif = m.encodeIndexed(Uint32Array.from(table), frame, Uint32Array.from(rec.body.frames, (f) => f.ms), rec.body.plays);
    if (store) {
      // A download freezes this version (rev) of the draft; drawing on starts the next.
      await store.snapshot(rec.id);
      const cur = await store.get(rec.id);
      rec = { ...rec, rev: cur.rev };
      root.dataset.rev = cur.rev;
    }
    const name = (rec.name.replace(/[^\p{L}\p{N} _-]+/gu, '').trim().replace(/\s+/g, '_') || 'drawing') + '.gif';
    const a = document.createElement('a');
    a.href = URL.createObjectURL(new Blob([gif], { type: 'image/gif' }));
    a.download = name;
    document.body.append(a);
    a.click();
    a.remove();
    setTimeout(() => URL.revokeObjectURL(a.href), 30000);
    root.dataset.gif = gif.length;
    status('kept', `Downloaded ${name}: ${table.length} colour${table.length === 1 ? '' : 's'}, ${(gif.length / 1024).toFixed(1)} KB`);
  } catch (e) {
    $('s-err').hidden = false;
    $('s-err').textContent = `The GIF could not be made: ${e.message || e}`;
  } finally {
    b.disabled = false;
  }
}

// --- start ---
async function main() {
  icons();
  const sel = $('s-pal');
  for (const p of STARTER) sel.append(new Option(p.name, p.id));
  sel.addEventListener('change', () => switchPalette(builtin(sel.value)));
  $('s-sw').addEventListener('click', (e) => { const b = e.target.closest('.s-sw'); if (b) pick(+b.dataset.k); });
  for (const b of document.querySelectorAll('.s-tool')) b.addEventListener('click', () => useTool(b.dataset.tool));
  $('s-undo').addEventListener('click', () => step(undo, redo, false));
  $('s-redo').addEventListener('click', () => step(redo, undo, true));
  $('s-gif').addEventListener('click', download);
  $('s-new').addEventListener('click', async () => { await save(); await fresh(); await drafts(); });
  $('s-list').addEventListener('click', async (e) => {
    const o = e.target.closest('.s-open'), d = e.target.closest('.s-del');
    if (o && o.dataset.id !== rec.id) { await save(); const r = await store.get(o.dataset.id); if (r) await open(r); await drafts(); }
    if (d && confirm(`Delete ${d.title.slice(7)}? It can’t be brought back.`)) {
      await store.remove(d.dataset.id);
      if (d.dataset.id === rec.id) {
        const { items } = await store.list({ type: 'piece' });
        const r = items.length && await store.get(items[0].id);
        if (r) await open(r); else await fresh();
      }
      await drafts();
    }
  });
  $('s-name').addEventListener('input', queueSave);
  $('s-v-px').addEventListener('click', () => view(false));
  $('s-v-led').addEventListener('click', () => view(true));
  $('s-g-grid').addEventListener('click', () => { const on = !$('s-g-grid').classList.contains('on'); toggle('s-g-grid', on); $('s-grid').hidden = !on; });
  $('s-g-seam').addEventListener('click', () => { const on = !$('s-g-seam').classList.contains('on'); toggle('s-g-seam', on); $('s-seam').hidden = !on; });
  $('s-z-in').addEventListener('click', () => { zoom = Math.min(ZOOMS.length, zoom ? zoom + 1 : Math.max(1, ZOOMS.findIndex((z) => z > work.clientWidth / W) + 1)); layout(); });
  $('s-z-out').addEventListener('click', () => { zoom = zoom > 1 && ZOOMS[zoom - 2] > fitScale() ? zoom - 1 : 0; layout(); });
  new ResizeObserver(layout).observe(scroll);
  document.addEventListener('keydown', (e) => {
    if (e.target.closest('input, select, textarea')) return;
    const k = e.key.toLowerCase();
    if ((e.ctrlKey || e.metaKey) && k === 'z') { e.preventDefault(); e.shiftKey ? step(redo, undo, true) : step(undo, redo, false); }
    else if ((e.ctrlKey || e.metaKey) && k === 'y') { e.preventDefault(); step(redo, undo, true); }
    else if (!e.ctrlKey && !e.metaKey && !e.altKey && { b: 1, e: 1, g: 1 }[k]) useTool({ b: 'pencil', e: 'eraser', g: 'fill' }[k]);
  });
  addEventListener('pagehide', () => { if (root.dataset.saved === 'saving') save(); });
  view(localStorage.getItem(LS + 'view') === 'led');
  layout();
  try { store = await LocalStore.open(); } catch { store = null; }
  let r = null;
  if (store) {
    const lastId = localStorage.getItem(LS + 'last');
    r = lastId && await store.get(lastId);
    if (!r) { const { items } = await store.list({ type: 'piece' }); r = items.length && await store.get(items[0].id); }
  }
  if (r) await open(r); else await fresh();
  await drafts();
  root.removeAttribute('aria-busy');
}
main().catch((e) => {
  $('s-err').hidden = false;
  $('s-err').textContent = `The Studio could not start: ${e.message || e}`;
  root.removeAttribute('aria-busy');
});
