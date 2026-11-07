// Pictures and palettes in the Formula editor (js/formulas.js imports it for
// a formula with a `sprite name {` block or a `palette name =` line). With
// the caret in one, a button at its first line opens its editor: a
// sprite's picture as a grid coloured from its key, to paint with a key
// character (tap or drag), rows and columns added or taken away, a key
// colour added; or a palette's strip, read as the panel reads it (a scratch
// formula, `pal(p, x)` across a row, on the editor's own compiler), with a
// handle per colour (dragged to reorder, tapped to change, named, added,
// taken away). The text is the source of truth: every change rewrites the
// block through the editor's put() (insertText: the browser's undo has each
// stroke as one step, the preview compiles it), and the editor reads the
// text again each time. Its own size line (tools/web-size.py).

import { h, clear } from './util.js';
import { popup, closeMenu } from './overrides.js';

const HEX = /^#([\da-f]{3}|[\da-f]{6})$/i;
// A line's code, its comment blanked (the same length: offsets hold).
const code = (l) => l.replace(/(--|\/\/).*/, (c) => ' '.repeat(c.length));
const lineEnd = (s, p) => { const n = s.indexOf('\n', p); return n < 0 ? s.length : n; };
const touch = () => matchMedia('(pointer:coarse)').matches;
const six = (v) => (v.length == 4 ? '#' + [...v.slice(1)].map((c) => c + c).join('') : v).toLowerCase();
// A colour picked, written as the one it replaces was: #rgb when it fits, upper case kept.
const as = (v, w) => { const x = w?.length == 4 && /^#(.)\1(.)\2(.)\3$/.test(v) ? '#' + v[1] + v[3] + v[5] : v; return /[A-F]/.test(w || '') ? x.toUpperCase() : x; };

// The sprite blocks: {k: 's', name, s, e (the block), key {s, e, le, ents [{ch, v}]},
// rows [{s, e, le, t}] (the text of each, its line's end), ind}.
function sprites(s) {
  const out = [];
  for (const b of s.matchAll(/^([ \t]*)sprite[ \t]+(\w+)[ \t]*\{[^\n]*\n([\s\S]*?)^[ \t]*\}/gm)) {
    const o = { k: 's', name: b[2], s: b.index, e: b.index + b[0].length, key: null, rows: [] };
    let at = b.index + b[0].indexOf('\n') + 1;
    for (const line of b[3].split('\n').slice(0, -1)) {
      const r = code(line).trim(), a = at + line.indexOf(r), le = at + line.length;
      if (/^key\s/.test(r)) o.key = { s: a, e: a + r.length, le, ents: r.slice(4).split(',').map((x) => x.trim().match(/^(\S)\s+(\S+)$/)).filter(Boolean).map((m) => ({ ch: m[1], v: m[2] })) };
      else if (r) o.rows.push({ s: a, e: a + r.length, le, t: r });
      at += line.length + 1;
    }
    o.ind = o.rows[0] ? s.slice(s.lastIndexOf('\n', o.rows[0].s - 1) + 1, o.rows[0].s) : b[1] + '  ';
    out.push(o);
  }
  return out;
}
// The palettes of colours (not tweens): {k: 'p', name, s, e (to the end of
// its statement: lines on while one ends with a comma, up to a `;`), ents
// [{s, e, lab, hex}]}; `bad` when an entry is not a colour.
function palettes(s) {
  const out = [];
  for (const m of s.matchAll(/^[ \t]*palette[ \t]+(\w+)[ \t]*=/gm)) {
    const a = m.index + m[0].length;
    let e = a;
    for (let p = a; ;) {
      const le = lineEnd(s, p), c = code(s.slice(p, le)), sc = c.indexOf(';');
      if (sc >= 0) { e = p + sc; break; }
      e = p + c.trimEnd().length;
      if (!c.trimEnd().endsWith(',') || le >= s.length) break;
      p = le + 1;
    }
    const r = s.slice(a, e).replace(/(--|\/\/)[^\n]*/g, (c) => ' '.repeat(c.length)), ents = [];
    let at = a, bad = false;
    for (const x of r.split(',')) {
      const t = x.trim(), q = t.match(/^(?:([A-Za-z_]\w*)[ \t]+)?(#[\da-f]{3}(?:[\da-f]{3})?)$/i), s0 = at + x.indexOf(t);
      if (q) ents.push({ s: s0, e: s0 + t.length, lab: q[1] || '', hex: q[2] }); else bad = true;
      at += x.length + 1;
    }
    out.push({ k: 'p', name: m[1], s: m.index, e, ents, bad: bad || !ents.length });
  }
  return out;
}
const find = (s, k, name) => (k == 's' ? sprites(s) : palettes(s)).find((o) => o.name == name);
// The edits [start, end, text] made to `s`, last first.
const apply = (s, ed) => ed.sort((x, y) => y[0] - x[0]).reduce((t, [a, b, x]) => t.slice(0, a) + x + t.slice(b), s);
// Where `s` and `t` differ: [start, end in s, the text from t], or null.
function diff(s, t) {
  if (s === t) return null;
  let a = 0, e = 0;
  while (a < s.length && a < t.length && s[a] === t[a]) a++;
  while (e < s.length - a && e < t.length - a && s[s.length - 1 - e] === t[t.length - 1 - e]) e++;
  return [a, s.length - e, t.slice(a, t.length - e)];
}

// A sprite rewritten: its rows `rows` (more go in after the last, fewer
// take the last lines away), a key entry `add` after the key.
function spriteText(s, o, rows, add) {
  const ed = [], n = o.rows.length, m = rows.length;
  if (add) ed.push([o.key.e, o.key.e, add]);
  for (let i = 0; i < Math.min(n, m); i++) if (o.rows[i].t !== rows[i]) ed.push([o.rows[i].s, o.rows[i].e, rows[i]]);
  if (m > n) { const at = n ? o.rows[n - 1].le : o.key.le; ed.push([at, at, rows.slice(n).map((r) => '\n' + o.ind + r).join('')]); }
  if (m < n) ed.push([o.rows[m - 1].le, o.rows[n - 1].le, '']);
  return apply(s, ed);
}
// A palette rewritten with the entries `L` ({lab, hex}): each in its place,
// more after the last, fewer from the end (the layout over lines kept).
function palText(s, o, L) {
  const ed = [], n = o.ents.length, m = L.length, tx = (x) => (x.lab ? x.lab + ' ' : '') + x.hex;
  for (let i = 0; i < Math.min(n, m); i++) ed.push([o.ents[i].s, o.ents[i].e, tx(L[i])]);
  if (m > n) ed.push([o.ents[n - 1].e, o.ents[n - 1].e, L.slice(n).map((x) => ', ' + tx(x)).join('')]);
  if (m < n) ed.push([o.ents[m - 1].e, o.ents[n - 1].e, '']);
  return apply(s, ed);
}

const CSS = `#psi-blk{position:absolute;z-index:2;min-height:24px;padding:0 8px;font-size:12px;background:var(--bg3);border-color:var(--acc);white-space:nowrap}#psi-blk[hidden]{display:none}
#psi-blk canvas{width:40px;height:10px;image-rendering:pixelated;border-radius:2px}@media (pointer:coarse){#psi-blk{min-height:32px}}
.menu.psi-big{width:min(760px,calc(100vw - 16px))}.menu.psi-big.sheet{width:auto}
.psi-grid{overflow:auto;max-height:52vh;background:#0a0b0c;border-radius:4px}.psi-grid canvas{display:block;touch-action:none;cursor:crosshair}.psi-grid.pan canvas{touch-action:pan-x pan-y;cursor:grab}
.psi-keys{display:flex;flex-wrap:wrap;gap:4px;align-items:center}.psi-keys button{min-width:44px;min-height:40px;justify-content:center;font:700 15px ui-monospace,monospace;text-shadow:0 0 3px #000,0 0 2px #000;color:#fff}
.psi-keys button[aria-pressed=true]{outline:3px solid var(--acc);outline-offset:1px}.psi-keys input[type=color],.psi-pe input[type=color]{width:44px;height:40px;padding:2px;border:1px solid var(--line);border-radius:4px;background:var(--bg3);cursor:pointer}
.psi-chk{background:repeating-conic-gradient(#555 0 25%,#2a2a2a 0 50%) 0 0/10px 10px}
.psi-strip2 canvas{display:block;width:100%;height:32px;image-rendering:pixelated;border-radius:4px;background:#000}.psi-ticks{position:relative;height:10px;margin:2px 0 6px}.psi-ticks i{position:absolute;top:0;width:2px;height:10px;margin-left:-1px;background:var(--mut)}
.psi-pes{display:flex;flex-wrap:wrap;gap:6px}.psi-pe{display:flex;align-items:center;gap:4px;padding:3px;border:1px solid var(--line);border-radius:var(--r);background:var(--bg3)}
.psi-pe input[type=color]{touch-action:none;cursor:grab}.psi-pe input[type=text]{width:88px;padding:4px 6px}.psi-pe.drag{opacity:.6;outline:2px dashed var(--acc)}.psi-pe .n{min-width:20px;color:var(--mut);font-size:12px;text-align:right}`;

// The editor's half (`kit`: {ta, put(t, text, quiet), measure(), row(src):
// a scratch formula's top row, RGB, or null}): `caret()` after the caret
// moves or the text changes.
export function blocks(kit) {
  const { ta } = kit;
  document.head.append(h('style', CSS));
  const mini = h('canvas', { width: 128, height: 1 });
  const btn = h('button.sm', { id: 'psi-blk', type: 'button', hidden: true, onpointerdown: (e) => e.preventDefault(), onclick: () => cur && open(cur) });
  ta.parentElement.append(btn);
  let cur = null, self = false, sync = null, dlg = null;

  // The text made `t`: live (`was` null: straight in, no undo step), or as one
  // undo step from `was` (the text as the gesture began): back to it, then
  // the change typed in, the text focused for it (no phone keyboard).
  function write(t, was) {
    self = true;
    try {
      if (was == null) { const d = diff(ta.value, t); if (d) kit.put({ s: d[0], e: d[1] }, d[2], true); return; }
      const b = diff(ta.value, was);
      if (b) kit.put({ s: b[0], e: b[1] }, b[2], true);
      const d = diff(was, t);
      if (!d) return;
      const back = document.activeElement;
      ta.inputMode = 'none';
      ta.focus({ preventScroll: true });
      kit.put({ s: d[0], e: d[1] }, d[2]);
      back?.focus?.({ preventScroll: true });
      ta.inputMode = '';
    } finally { self = false; }
  }
  // The browser's undo, from the editor's button (a phone has no Ctrl+Z).
  function undo() {
    const back = document.activeElement;
    ta.inputMode = 'none';
    ta.focus({ preventScroll: true });
    document.execCommand('undo');
    back?.focus?.({ preventScroll: true });
    ta.inputMode = '';
  }
  // The text changed by someone else (typing, an undo): the open editor reads it again.
  ta.addEventListener('input', () => { if (!self && sync && dlg?.isConnected) sync(); });

  // A palette's colours as the panel reads them: 128 across, RGB.
  const strip = (o, L) => kit.row(`palette ${o.name} = ${L.map((x) => x.hex).join(', ')}\npal(${o.name}, x)`);
  function paint(cv, rgb) {
    const g = cv.getContext('2d'), im = g.createImageData(128, 1);
    for (let i = 0; i < 128; i++) im.data.set([rgb[i * 3], rgb[i * 3 + 1], rgb[i * 3 + 2], 255], i * 4);
    g.putImageData(im, 0, 0);
  }
  let miniOf = '';

  // The button: at the end of the first line of the block the caret is in.
  function caret() {
    const f = document.activeElement, p = ta.selectionStart, s = ta.value;
    cur = f === ta || f === btn ? [...sprites(s), ...palettes(s)].find((o) => o.s <= p && p <= o.e && (o.k == 's' ? o.key : !o.bad)) : null;
    if ((btn.hidden = !cur)) return;
    const l = s.slice(0, cur.s).split('\n'), first = code(s.slice(cur.s, lineEnd(s, cur.s))).trimEnd(), st = getComputedStyle(ta);
    let col = 0;
    for (const c of l.at(-1) + first) col += c === '\t' ? 2 - (col % 2) : 1;
    clear(btn, cur.k == 's' ? 'Edit the picture' : [mini, 'Edit the palette']);
    if (cur.k == 'p') { const k = cur.ents.map((x) => x.hex).join(), rgb = k !== miniOf && strip(cur, cur.ents); if (rgb) { paint(mini, rgb); miniOf = k; } }
    const lh = parseFloat(st.lineHeight);
    btn.style.left = `${Math.max(0, Math.min(ta.clientLeft + parseFloat(st.paddingLeft) + (col + 2) * kit.measure() - ta.scrollLeft, ta.clientWidth - btn.offsetWidth))}px`;
    btn.style.top = `${ta.clientTop + parseFloat(st.paddingTop) + (l.length - 1) * lh + (lh - btn.offsetHeight) / 2}px`;
  }
  function open(o) {
    const r = btn.getBoundingClientRect();
    dlg = o.k == 's' ? picture(o) : palette(o);
    popup(dlg, r.left, r.bottom + 4);
    dlg.onclose = () => { sync = dlg = null; };
  }
  const foot = (...x) => h('div.row.acts', ...x, h('button.sm', { id: 'psi-blk-undo', onclick: undo }, 'Undo'), h('button.sm.pri', { onclick: closeMenu }, 'Done'));

  // --- A sprite's picture.
  function picture(o0) {
    const name = o0.name, grid = h('div.psi-grid'), cv = h('canvas', { id: 'psi-pic' }), keys = h('div.psi-keys', { role: 'group', 'aria-label': 'Paint with' });
    const size = h('span.cap', { id: 'psi-pic-size' });
    grid.append(cv);
    let o = o0, rows = [], W = 0, H = 0, cs = 20, pick = null, map = {}, empty = '.';
    // The key: each character's colour (a number's a gray; 0 is see-through).
    function read() {
      o = find(ta.value, 's', name);
      if (!o?.key) { closeMenu(); return false; }
      const ents = o.key.ents, hi = Math.max(1, ...ents.map((k) => +k.v || 0));
      map = {};
      for (const k of ents) map[k.ch] = HEX.test(k.v) ? six(k.v) : +k.v ? `rgb(${Array(3).fill(Math.round(255 * Math.min(1, +k.v / hi))).join()})` : null;
      empty = ents.find((k) => !HEX.test(k.v) && +k.v === 0)?.ch ?? ents[0]?.ch ?? '.';
      W = Math.max(1, ...o.rows.map((r) => r.t.length));
      H = o.rows.length;
      rows = o.rows.map((r) => r.t.padEnd(W, empty));
      if (!H) { rows = [empty.repeat(W)]; H = 1; }
      if (!(pick in map)) pick = ents.find((k) => map[k.ch])?.ch ?? empty;
      return true;
    }
    function draw() {
      cs = touch() ? 22 : Math.max(14, Math.min(28, Math.floor(700 / W)));
      const d = devicePixelRatio || 1, g = cv.getContext('2d');
      cv.width = W * cs * d; cv.height = H * cs * d;
      cv.style.width = `${W * cs}px`; cv.style.height = `${H * cs}px`;
      g.scale(d, d);
      for (let v = 0; v < H; v++) for (let u = 0; u < W; u++) {
        const c = rows[v][u], k = map[c];
        if (k) { g.fillStyle = k; g.fillRect(u * cs, v * cs, cs, cs); } else {
          for (let q = 0; q < 4; q++) { g.fillStyle = (q + (q >> 1)) & 1 ? '#2a2a2a' : '#555'; g.fillRect(u * cs + (q & 1) * cs / 2, v * cs + (q >> 1) * cs / 2, cs / 2, cs / 2); }
          if (!(c in map)) { g.fillStyle = '#e55'; g.font = `bold ${cs * 0.6}px monospace`; g.textAlign = 'center'; g.fillText(c, u * cs + cs / 2, v * cs + cs * 0.72); }
        }
        g.strokeStyle = '#0006'; g.strokeRect(u * cs + 0.5, v * cs + 0.5, cs - 1, cs - 1);
      }
      size.textContent = `${W} × ${H}`;
      cv.setAttribute('aria-label', `The picture, ${W} by ${H}`);
      for (const [b, n] of [[addC, W >= 64], [delC, W <= 1], [addR, H >= 32], [delR, H <= 1]]) b.disabled = n;
    }
    function swatches() {
      clear(keys, o.key.ents.map((k) => h('button' + (map[k.ch] ? '' : '.psi-chk'), { type: 'button', 'data-ch': k.ch, 'aria-pressed': String(k.ch === pick), title: map[k.ch] ? `${k.ch}: ${k.v}` : `${k.ch}: see-through`,
        style: map[k.ch] ? `background:${map[k.ch]}` : '', onclick: () => { pick = k.ch; swatches(); } }, k.ch)),
      o.key.ents.length < 16 && h('label.row', { title: 'Add a colour to the key' }, '+ Colour', h('input', { type: 'color', id: 'psi-pic-add', 'aria-label': 'Add a colour to the key', value: '#ffffff', onchange: (e) => addKey(e.target.value) })));
    }
    // A new key character: a letter the key doesn't have.
    function addKey(v) {
      if (!read()) return;
      const ch = [...'ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz*+=%&@$'].find((c) => !(c in map) && !rows.some((r) => r.includes(c)));
      if (!ch) return;
      write(spriteText(ta.value, o, rows, `, ${ch} ${v}`), ta.value);
      read(); pick = ch; swatches(); draw();
    }
    // Rows and columns: added (see-through, or the key's first) or taken away, at the bottom and the right.
    const resize = (dw, dh) => {
      if (!read()) return;
      let r = rows.map((x) => (dw > 0 ? x + empty : dw < 0 ? x.slice(0, -1) : x));
      if (dh > 0) r.push(empty.repeat(r[0].length)); else if (dh < 0) r = r.slice(0, -1);
      write(spriteText(ta.value, o, r), ta.value);
      read(); draw();
    };
    const addC = h('button.sm', { id: 'psi-pic-c+', onclick: () => resize(1, 0) }, '+ Column'), delC = h('button.sm', { id: 'psi-pic-c-', onclick: () => resize(-1, 0) }, '− Column');
    const addR = h('button.sm', { id: 'psi-pic-r+', onclick: () => resize(0, 1) }, '+ Row'), delR = h('button.sm', { id: 'psi-pic-r-', onclick: () => resize(0, -1) }, '− Row');
    // A stroke: each cell as it is reached (live), one undo step at the end.
    let stroke = null;
    const cell = (e) => { const b = cv.getBoundingClientRect(); return [Math.floor((e.clientX - b.left) / cs), Math.floor((e.clientY - b.top) / cs)]; };
    function dab(e) {
      const [u, v] = cell(e);
      if (u < 0 || v < 0 || u >= W || v >= H || rows[v][u] === pick) return;
      rows[v] = rows[v].slice(0, u) + pick + rows[v].slice(u + 1);
      write(spriteText(ta.value, find(ta.value, 's', name), rows));
      draw();
    }
    cv.onpointerdown = (e) => {
      if (grid.classList.contains('pan') && e.pointerType == 'touch' || e.button) return;
      if (!read()) return;
      e.preventDefault();
      cv.setPointerCapture(e.pointerId);
      stroke = { id: e.pointerId, was: ta.value };
      dab(e);
    };
    cv.onpointermove = (e) => { if (stroke?.id === e.pointerId) dab(e); };
    cv.onpointerup = cv.onpointercancel = (e) => {
      if (stroke?.id !== e.pointerId) return;
      const was = stroke.was;
      stroke = null;
      write(ta.value, was);
    };
    sync = () => { if (!stroke && read()) { swatches(); draw(); } };
    const pan = touch() && h('button.sm', { 'aria-pressed': 'false', onclick: (e) => { const on = grid.classList.toggle('pan'); e.currentTarget.ariaPressed = String(on); } }, 'Scroll');
    read();
    swatches();
    const el = h('div.menu.wide.psi-big', { id: 'psi-pic-ed', role: 'dialog', 'aria-label': `The picture of ${name}` }, h('div.mh', `The picture of ${name}`),
      keys, grid, h('div.row', size, addC, delC, addR, delR, pan),
      h('p.cap', touch() ? 'Pick a character from the key, then tap or drag on the picture. Scroll moves the picture instead.' : 'Pick a character from the key, then click or drag on the picture.'), foot());
    draw();
    return el;
  }

  // --- A palette's strip and its colours.
  function palette(o0) {
    const name = o0.name, cv = h('canvas', { id: 'psi-pal-strip', width: 128, height: 1 }), ticks = h('div.psi-ticks'), list = h('div.psi-pes', { id: 'psi-pal-list' });
    const note = h('p.cap.bad', { hidden: true });
    let o = o0, L = [];
    const read = () => { o = find(ta.value, 'p', name); if (!o || o.bad) { closeMenu(); return false; } L = o.ents.map((x) => ({ ...x })); return true; };
    function show() {
      const rgb = strip(o, L);
      note.hidden = !!rgb;
      note.textContent = rgb ? '' : 'The panel can’t read this palette yet';
      if (rgb) paint(cv, rgb);
      cv.dataset.rgb = rgb ? Array.from(rgb).join() : '';
      clear(ticks, L.map((x, i) => h('i', { style: `left:${L.length > 1 ? (i * 100) / (L.length - 1) : 0}%` })));
    }
    const commit = (was) => { write(palText(ta.value, find(ta.value, 'p', name), L), was); if (read()) { show(); chips(); } };
    // (On the window: the chip moved in the list loses the pointer's capture.)
    let drag = null;
    function move(e) {
      const d = drag;
      if (d?.id !== e.pointerId) return;
      if (!d.moved) { if (Math.hypot(e.clientX - d.x, e.clientY - d.y) < 6) return; d.moved = true; d.c.classList.add('drag'); }
      e.preventDefault();
      const t = document.elementsFromPoint(e.clientX, e.clientY).find((n) => n.classList?.contains('psi-pe') && n !== d.c), all = [...list.children];
      if (t) list.insertBefore(d.c, all.indexOf(t) > all.indexOf(d.c) ? t.nextSibling : t);
    }
    function up(e) {
      const d = drag;
      if (d?.id !== e.pointerId) return;
      removeEventListener('pointermove', move);
      removeEventListener('pointerup', up);
      removeEventListener('pointercancel', up);
      setTimeout(() => { if (drag === d) drag = null; });
      if (!d.moved) return;
      const order = [...list.querySelectorAll('.psi-pe')].map((n) => +n.dataset.i);
      if (order.some((j, q) => j !== q)) { L = order.map((j) => L[j]); commit(ta.value); } else chips();
    }
    function chips() {
      clear(list, L.map((x, i) => {
        let was = null;
        const w0 = x.hex, k = h('input', { type: 'color', value: six(x.hex), 'aria-label': `Colour ${i + 1}${x.lab ? ', ' + x.lab : ''}: drag to move it`, title: 'Tap to change it, drag to move it',
          onclick: (e) => { if (drag?.moved) e.preventDefault(); else was = ta.value; },
          oninput: () => { if (was == null) was = ta.value; L[i].hex = as(k.value, w0); write(palText(ta.value, find(ta.value, 'p', name), L)); show(); },
          onchange: () => { if (was != null) { const w = was; was = null; L[i].hex = as(k.value, w0); commit(w); } } });
        const lab = h('input', { type: 'text', value: x.lab, placeholder: 'name', maxLength: 24, 'aria-label': `Name of colour ${i + 1}`,
          onchange: () => { const v = lab.value.trim(); if (v && !/^[A-Za-z_]\w*$/.test(v)) { lab.value = x.lab; return; } L[i].lab = v; commit(ta.value); } });
        const del = h('button.sm', { type: 'button', 'aria-label': `Remove colour ${i + 1}`, disabled: L.length < 2, onclick: () => { L.splice(i, 1); commit(ta.value); } }, '×');
        const c = h('div.psi-pe', { 'data-i': i }, h('span.n', i + 1), k, lab, del);
        // Dragged (6 px), it moves along the list; let go, the palette is in that order.
        k.onpointerdown = (e) => {
          if (e.button) return;
          drag = { x: e.clientX, y: e.clientY, id: e.pointerId, moved: false, c };
          addEventListener('pointermove', move);
          addEventListener('pointerup', up);
          addEventListener('pointercancel', up);
        };
        return c;
      }), h('button.sm', { id: 'psi-pal-add', type: 'button', disabled: L.length >= 32, onclick: () => { L.push({ lab: '', hex: L.at(-1).hex }); commit(ta.value); try { list.querySelectorAll('input[type=color]')[L.length - 1]?.showPicker?.(); } catch { /* not allowed now: a tap on it opens it */ } } }, '+ Colour'));
    }
    sync = () => { if (!drag && read()) { show(); chips(); } };
    read();
    show();
    chips();
    return h('div.menu.wide.psi-big', { id: 'psi-pal-ed', role: 'dialog', 'aria-label': `The palette ${name}` }, h('div.mh', `The palette ${name}`),
      h('div.psi-strip2', cv), ticks, note, list,
      h('p.cap', 'The strip is the palette as the panel reads it, pal(' + name + ', x) from left to right. Tap a colour to change it, drag it to move it.'), foot());
  }

  return { caret, sprites, palettes };
}
