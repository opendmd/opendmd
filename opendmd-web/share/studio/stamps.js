// The Studio's stamps (/studio/), loaded when the Stamp tool, the Stamps
// section or Save as stamp is first used: the picker (the starter pack's
// stamps and the user's own), the stamp following the pointer as a ghost,
// a click placing it as one step (its colours joining the piece through
// keysFor, in that step), and Save as stamp, a selection kept as a stamp
// record with its own colours. A stamp placed is flat pixels in the cel
// drawn on; the piece's `from` names it once, with its hash.
import { W, H } from './doc.js';
import { record, packPx, unpackPx, CAPS } from './store.js';
import { starter, starterAll, builtin, isStarter } from './palettes.js';

let S = null, picked = null, ghost = null, at = null, list = [], set = 'starter';
const $ = (id) => document.getElementById(id);
const el = (tag, cls, text) => { const e = document.createElement(tag); if (cls) e.className = cls; if (text !== undefined) e.textContent = text; return e; };
const hexInt = (h) => parseInt(h.slice(1), 16);

export function mount(studio) {
  S = studio;
  ghost = el('canvas');
  ghost.id = 's-ghost';
  ghost.width = W; ghost.height = H;
  ghost.hidden = true;
  ghost.style.cssText = 'position:absolute;inset:0;width:100%;height:100%;pointer-events:none;opacity:.6;image-rendering:pixelated';
  $('s-canvas').after(ghost);
  Object.assign(S.fx, { hover, place, unstamp: () => { ghost.hidden = true; } });
}

// A stamp as the picker and the placing need it: its first frame's keys, its colours (0xRRGGBB by key).
async function load(r) {
  const b = r.body, cs = b.colours || builtin(b.palette)?.body.colours || S.mine.get(b.palette)?.body.colours;
  if (!cs) return null;
  const px = await unpackPx(b.frames[0].px, b.w * b.h, b.keys === 2);
  return { r, w: b.w, h: b.h, px, cols: new Map(cs.map((c) => [c.k, hexInt(c.hex)])) };
}
async function stamps() {
  if (set === 'starter') {
    await starter().catch(() => {});
    return starterAll.filter((r) => r.type === 'stamp');
  }
  const out = [];
  for (const h of S.store ? (await S.store.list({ type: 'stamp' })).items : []) { const r = await S.store.get(h.id); if (r) out.push(r); }
  return out;
}
function thumb(s) {
  const c = el('canvas');
  c.width = s.w; c.height = s.h;
  const g = c.getContext('2d'), img = g.createImageData(s.w, s.h);
  s.px.forEach((k, i) => { if (k && s.cols.has(k)) { const v = s.cols.get(k); img.data.set([v >> 16, (v >> 8) & 255, v & 255, 255], i * 4); } });
  g.putImageData(img, 0, 0);
  return c;
}
// The picker, in the Stamps section: a set menu (the starter stamps, yours) and a grid.
export async function show() {
  const host = $('s-stampsx');
  $('s-stampsb').hidden = true;
  if (!host.querySelector('select')) {
    const m = el('select', 's-in s-sset');
    m.setAttribute('aria-label', 'Stamp set');
    m.append(new Option('Starter stamps', 'starter'), new Option('Your stamps', 'mine'));
    m.onchange = () => { set = m.value; show(); };
    const grid = el('div', 's-sgrid');
    grid.id = 's-sgrid';
    grid.onclick = (e) => { const b = e.target.closest('[data-n]'); if (b) pick(+b.dataset.n); };
    grid.style.cssText = 'display:grid;grid-template-columns:repeat(auto-fill,minmax(52px,1fr));gap:6px;margin-top:10px';
    host.replaceChildren(m, grid);
  }
  const rs = await stamps();
  list = (await Promise.all(rs.map((r) => load(r).catch(() => null)))).filter(Boolean);
  $('s-sgrid').replaceChildren(...list.map((s, n) => {
    const b = el('button', 's-sbtn' + (picked?.r.id === s.r.id ? ' on' : ''));
    b.dataset.n = n;
    b.title = s.r.name;
    b.setAttribute('aria-label', s.r.name);
    b.style.cssText = 'aspect-ratio:1;display:grid;place-items:center;padding:4px;border:1px solid var(--line);border-radius:8px;background:#000';
    const c = thumb(s);
    c.style.cssText = `image-rendering:pixelated;max-width:100%;max-height:100%;width:${Math.min(44, s.w * 4)}px`;
    b.append(c);
    return b;
  }));
  if (!list.length) $('s-sgrid').replaceChildren(el('p', 's-cap', set === 'mine' ? 'None yet: select a box on the panel, then Save as stamp.' : 'The starter stamps could not load.'));
  S.root.dataset.stamps = `${set}:${list.length}`;
}
function pick(n) {
  picked = list[n];
  S.say('', 'info'); // (the hint to pick one: done)
  S.root.dataset.stamp = picked?.r.id || '';
  for (const b of document.querySelectorAll('#s-sgrid .s-sbtn')) b.classList.toggle('on', +b.dataset.n === n);
  if (S.tool !== 'stamp') S.useTool('stamp');
}
// A stamp picked by id (the Library's Use this stamp).
export async function pickId(id) {
  const want = isStarter(id) ? 'starter' : 'mine';
  if (set !== want) { set = want; const m = document.querySelector('#s-stampsx select'); if (m) m.value = want; await show(); }
  const n = list.findIndex((s) => s.r.id === id);
  if (n >= 0) pick(n);
}
// The Stamp tool chosen with nothing picked: the picker, and a word.
export function tool() {
  if (!picked) { show(); S.say('Pick a stamp under Stamps, then click the panel to place it.', 'info'); }
}

// Where a stamp lands: centred on the pixel pointed at.
const origin = (s, [x, y]) => [x - (s.w >> 1), y - (s.h >> 1)];
function hover(p) {
  at = p;
  if (!ghost || S.tool !== 'stamp' || !picked || !p) { if (ghost) ghost.hidden = true; return; }
  const [ox, oy] = origin(picked, p), g = ghost.getContext('2d'), img = g.createImageData(W, H);
  picked.px.forEach((k, i) => {
    const x = ox + (i % picked.w), y = oy + Math.floor(i / picked.w);
    if (picked.cols.has(k) && x >= 0 && x < W && y >= 0 && y < H) { const v = picked.cols.get(k); img.data.set([v >> 16, (v >> 8) & 255, v & 255, 255], (y * W + x) * 4); }
  });
  g.putImageData(img, 0, 0);
  ghost.hidden = false;
}
// A click: the stamp's pixels set on the cel drawn on (off is see-through), its colours joining the piece, one step.
function place(p) {
  if (!picked) { tool(); return; }
  const D = S.doc, s = picked, r = s.r;
  if (D.stroke) return;
  const entry = { id: r.id, rev: r.rev, hash: r.hash, name: r.name, author: r.author, licence: r.licence };
  if (!S.addFrom(entry, true)) return;
  // A key the stamp has no colour for is off, as it shows.
  const used = [...new Set(s.px)].filter((k) => k && s.cols.has(k)), ks = D.keysFor(used.map((k) => s.cols.get(k) ?? 0)), to = new Map(used.map((k, i) => [k, ks[i]]));
  if (D.full) { S.say(`A piece holds ${CAPS.colours} colours at most`); D.letGo(D.key); return; }
  const [ox, oy] = origin(s, p);
  D.begin();
  s.px.forEach((k, i) => {
    const x = ox + (i % s.w), y = oy + Math.floor(i / s.w);
    if (to.has(k) && x >= 0 && x < W && y >= 0 && y < H) D.set(y * W + x, to.get(k));
  });
  const step = D.end();
  if (step) { S.addFrom(entry); S.swatches(); S.after(step); }
  S.paint();
  S.root.dataset.placed = (+S.root.dataset.placed || 0) + 1;
}

// The selection kept as a stamp of the user's own: its box, its colours renumbered from 1.
export async function saveSel() {
  const sel = S.sel, D = S.doc;
  if (!sel || !S.store) { S.say(S.store ? 'Select a box first.' : 'Stamps can’t be kept in this browser.'); return; }
  const keys = sel.fl || Uint16Array.from({ length: sel.w * sel.h }, (_, n) => D.px[(sel.y + Math.floor(n / sel.w)) * W + sel.x + (n % sel.w)]);
  const used = [...new Set(keys)].filter((k) => k).sort((a, b) => a - b);
  if (!used.length) { S.say('That box is empty: there is nothing to keep as a stamp.'); return; }
  const re = new Map(used.map((k, i) => [k, i + 1])), wide = used.length > 255;
  const colours = used.map((k, i) => ({ k: i + 1, hex: '#' + D.rgbOf(k).toString(16).padStart(6, '0') }));
  const px = await packPx(Uint16Array.from(keys, (k) => (k ? re.get(k) : 0)), wide);
  const p = S.piece(), from = p ? [{ id: p.id, rev: p.rev, hash: p.hash, name: p.name, author: S.rec.author, licence: S.rec.licence }] : [];
  const name = `${S.nameNow()} stamp`.slice(0, CAPS.name);
  const rec = record('stamp', name, { w: sel.w, h: sel.h, ...(wide ? { keys: 2 } : {}), colours, frames: [{ px }] }, { from });
  try {
    const h = await S.run(() => S.store.put(rec));
    S.say('');
    S.status('kept', `Kept as a stamp: ${name}`);
    set = 'mine';
    await show();
    const m = document.querySelector('#s-stampsx select');
    if (m) m.value = 'mine';
    S.root.dataset.savedStamp = h.id;
  } catch (e) {
    S.say(`That stamp could not be kept: ${e.message || e}`);
  }
}
