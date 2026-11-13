// The Studio's panels (/studio/), loaded once the window is the workspace's
// size (studio.css; FLOAT.md F1): Tools, Colours, Layers, Stamps and This
// piece, each docked in the left or right column, under the canvas, or
// floating over the stage.
// Under the canvas: a window that has 200 px to spare below the canvas at
// Fit and the frames, or has them once Fit gives up one step (never below
// 7x), has a row of panels there, Layers and Stamps to begin with, and the
// stage is no taller than the canvas (studio.css, data-under; studio.js
// keeps Fit to what leaves the row its 200 px). roomy() says so by the
// window's size alone, so nothing a panel does moves it. In a window
// without that room the row's panels sit in the right column, each by its
// own place in its stack (Colours, Layers, Stamps, This piece to begin
// with), and a panel's menu still says which it is docked to.
// A title bar drags (to within 28 px of the stage's edge: docked in that
// column, where it was dropped in the stack; onto the title bars of the row
// under the canvas, which is kept as it is until the drop: docked there;
// Escape puts it back); its name
// is a button the arrow keys move (16 px, Shift 64) and Enter or Space folds; its ⋮
// menu docks, floats, folds and closes it; a floating one resizes by its
// corner (dragged, or the arrow keys on it). The Panels menu shows or closes
// each, hides them all (H, or PANELS in the chin: for this tab's visit only)
// and resets the layout. Two arrangements are kept in this browser, for
// windows under 1600 px and wider; storage refused, it just forgets.
//
// Panels are placed with left / top only: no transform, filter or contain
// on one, as the notice (position: fixed) may sit inside one. When a panel
// that holds the notice's control loses its box the notice goes to the top
// (S.rehome). Below the workspace's size everything goes back as it was.
const IDS = ['tools', 'colours', 'layers', 'stamps', 'piece'];
const NAME = { tools: 'Tools', colours: 'Colours', layers: 'Layers', stamps: 'Stamps', piece: 'This piece' };
const SEL = { tools: '.s-tray', colours: '.s-colours', layers: '.s-layers', stamps: '.s-stamps', piece: '.s-drafts' };
const FW = { tools: 104, colours: 264, layers: 320, stamps: 400, piece: 320 }; // a floating panel's width, until resized
const MINW = { tools: 54 }, MINH = 90, TB = 36, EDGE = 28;
const ws = matchMedia('(min-width:1000px) and (min-height:620px)');
const SIDES = ['l', 'r', 'b'], ROW = 200, TRIM = 240; // (TRIM: the stage but for the picture, and the frames strip: studio.js STAGE_TRIM)
const el = (tag, cls, text) => { const e = document.createElement(tag); if (cls) e.className = cls; if (text !== undefined) e.textContent = text; return e; };
const num = (v, d) => (Number.isFinite(v) ? Math.max(-1e5, Math.min(1e5, v)) : d);

let S = null, root = null, on = false, bare = false, drag = null, menu = null, raf = 0;
const P = {}, docks = {};
let pbtn = null, chin = null, bench = null, selbar = null;
// The layout, a class at a time ('n' under 1600 px, 'w' wider): { order: [front to back], p: { id: { at, i, x, y, ax, c, h, w, ht } } }.
let lay = {}, kept = {};

export let ready = null;
export function mount(studio) {
  S = studio;
  root = S.root;
  const css = el('link');
  css.rel = 'stylesheet';
  css.href = new URL('panels.css', import.meta.url).href;
  ready = new Promise((ok) => { css.onload = css.onerror = ok; document.head.append(css); }).then(() => { if (ws.matches) up(); });
  ws.addEventListener('change', () => (ws.matches ? up() : down()));
  addEventListener('resize', () => { shut(); if (on && !raf) raf = requestAnimationFrame(() => { raf = 0; apply(); }); });
  addEventListener('hashchange', () => shut());
  // Back from the Library (the Studio its workspace height again): the room under the canvas as the window is now.
  new ResizeObserver(() => { if (on && !drag && under !== roomy() && root.dataset.view !== 'library') apply(); }).observe(root);
  root.addEventListener('pointerdown', grab);
  root.addEventListener('pointermove', move);
  root.addEventListener('pointerup', (e) => let_go(e, true));
  root.addEventListener('pointercancel', (e) => let_go(e, false));
  // (Only the Studio's own capture: a touch's implicit capture on the title bar is let go, and bubbles here, as the drag takes it.)
  root.addEventListener('lostpointercapture', (e) => { if (e.target === root) let_go(e, false); });
  root.addEventListener('pointerdown', (e) => { const p = on && e.target.closest?.('.s-panel.float'); if (p) front(p.dataset.panel); }, true);
  root.addEventListener('dblclick', (e) => { const hd = bar(e); if (hd) fold(hd.parentNode.dataset.panel); });
  root.addEventListener('keydown', keys);
  root.addEventListener('click', press);
  root.addEventListener('focusin', (e) => { const p = e.target.closest?.('.s-panel.float'); if (p) front(p.dataset.panel); });
  // Escape: a notice first (studio.js, before this), then a drag in progress.
  document.addEventListener('keydown', (e) => { if (e.key === 'Escape' && drag) { e.preventDefault(); e.stopImmediatePropagation(); let_go(e, false); } }, true);
  document.addEventListener('pointerdown', (e) => { if (menu && !menu.m.contains(e.target) && !menu.at.contains(e.target)) shut(); }, true);
}

// --- the layout: what each panel is, kept and read back ---
const fresh = () => ({ order: [...IDS], p: Object.fromEntries(IDS.map((id, i) => [id, { at: { tools: 'l', layers: 'b', stamps: 'b' }[id] || 'r', i, x: null, y: null, ax: 'l', c: 0, h: 0 }])) });
// Room under the canvas for a row of panels: what the window's height leaves below the canvas at Fit (with a
// right column at its narrowest, 16x at most: 374 px is the window's width but for the canvas then) and the frames
// strip is the row's 200 px, or is once Fit is a step smaller (7x at least): no window is left a wide empty band.
const roomy = () => {
  const s = Math.max(1, Math.min(16, Math.floor((vw() - 374) / 128))), spare = root.clientHeight - root.querySelector('.s-doc').offsetHeight - TRIM - 32 * s;
  return spare >= ROW || (s > 7 && spare + 32 >= ROW);
};
// Where a panel docked at `at` is shown: one kept for under the canvas is in the right column when there is no room there.
let under = false;
const shown = (at) => (at === 'b' && !under ? 'r' : at);
const cls = () => (innerWidth >= 1600 ? 'w' : 'n');
const cur = () => lay[cls()];
// What was stored, made safe: anything that does not read is the default; positions are clamped when shown.
function tidy(c) {
  const d = fresh();
  if (!c || typeof c !== 'object' || !c.p || typeof c.p !== 'object') return null;
  for (const id of IDS) {
    const q = c.p[id], p = d.p[id];
    if (!q || typeof q !== 'object') continue;
    if (['l', 'r', 'b', 'f'].includes(q.at)) p.at = q.at;
    p.i = num(q.i, p.i);
    p.x = num(q.x, null);
    p.y = num(q.y, null);
    p.ax = q.ax === 'r' ? 'r' : 'l';
    p.c = q.c ? 1 : 0;
    p.h = q.h ? 1 : 0;
    if (Number.isFinite(q.w)) p.w = q.w;
    if (Number.isFinite(q.ht)) p.ht = q.ht;
  }
  const o = Array.isArray(c.order) ? c.order.filter((id, n, a) => IDS.includes(id) && a.indexOf(id) === n) : [];
  d.order = [...o, ...IDS.filter((id) => !o.includes(id))];
  return d;
}
function load() {
  let o = null;
  try { o = JSON.parse(S.ls.get('panels')); } catch { /* none, or not ours */ }
  for (const k of ['n', 'w']) {
    const t = o?.v === 2 ? tidy(o[k]) : null;
    lay[k] = t || fresh();
    kept[k] = !!t;
  }
  try { bare = sessionStorage.getItem('odmd.studio.bare') === '1'; } catch { bare = false; }
}
function save(keep = true) {
  if (keep) kept[cls()] = true;
  const o = { v: 2 };
  for (const k of ['n', 'w']) if (kept[k]) o[k] = lay[k];
  S.ls.set('panels', JSON.stringify(o));
}
function setBare(b) {
  bare = b;
  try { sessionStorage.setItem('odmd.studio.bare', b ? '1' : '0'); } catch { /* this visit only */ }
  apply();
}

// --- the DOM: each panel's title bar, body and corner; the left column; the Panels button ---
function up() {
  if (on) return;
  on = true;
  bench = root.querySelector('.s-bench');
  selbar = root.querySelector('.s-selbar');
  docks.l = el('div', 's-dock s-dl');
  docks.r = bench;
  bench.classList.add('s-dock');
  selbar.before(docks.l);
  docks.b = el('div', 's-dock s-du');
  root.querySelector('.s-strip').after(docks.b);
  for (const id of IDS) {
    const p = P[id] = root.querySelector(SEL[id]);
    let hd = p.querySelector(':scope>.s-hd');
    if (!hd) { hd = el('div', 's-hd s-gen'); hd.append(el('h2')); p.prepend(hd); }
    const h2 = hd.querySelector('h2'), g = el('button', 's-grip');
    g.type = 'button';
    g.title = `${NAME[id]}: drag to move it (to a column's edge to dock it); arrow keys move it${id === 'tools' ? '' : ', Enter folds it'}`;
    if (hd.classList.contains('s-gen')) g.textContent = NAME[id]; else g.append(...h2.childNodes);
    h2.append(g);
    const b = (pa, text, label) => { const x = el('button', 's-pbtn', text); x.type = 'button'; x.dataset.pa = pa; x.setAttribute('aria-label', label); x.title = label; return x; };
    if (id !== 'tools') hd.append(b('col', '▾', `Fold ${NAME[id]}`));
    const m = b('menu', '⋮', `${NAME[id]}: dock, float, fold or close`);
    m.setAttribute('aria-haspopup', 'menu');
    m.setAttribute('aria-expanded', 'false');
    hd.append(m);
    if (id !== 'tools') hd.append(b('close', '×', `Close ${NAME[id]}`));
    const pb = el('div', 's-pb');
    pb.append(...[...p.childNodes].filter((n) => n !== hd));
    p.append(pb, b('size', '', `Resize ${NAME[id]}: drag, or the arrow keys`));
    p.lastChild.className = 's-rsz';
    p.classList.add('s-panel');
    p.dataset.panel = id;
  }
  pbtn = el('button', 's-pill', 'Panels ▾');
  pbtn.id = 's-panels';
  pbtn.type = 'button';
  pbtn.title = 'Show or close each panel, hide them all (H), reset the layout';
  pbtn.setAttribute('aria-haspopup', 'menu');
  pbtn.setAttribute('aria-expanded', 'false');
  root.querySelector('#s-libb').before(pbtn);
  chin = el('span', 's-seg');
  chin.setAttribute('role', 'group');
  chin.append(el('button', '', 'PANELS'));
  chin.firstChild.id = 's-g-panels';
  chin.firstChild.type = 'button';
  chin.firstChild.title = 'Show or hide every panel (H)';
  root.querySelector('#s-xy').before(chin);
  load();
  apply();
}
function down() {
  if (!on) return;
  shut();
  if (drag) let_go(null, false);
  on = false;
  for (const id of IDS) {
    const p = P[id], hd = p.querySelector(':scope>.s-hd'), pb = p.querySelector(':scope>.s-pb');
    for (const x of p.querySelectorAll(':scope>.s-hd>.s-pbtn, :scope>.s-rsz')) x.remove();
    const g = hd.querySelector('.s-grip');
    if (hd.classList.contains('s-gen')) hd.remove(); else g.replaceWith(...g.childNodes);
    pb.replaceWith(...pb.childNodes);
    p.classList.remove('s-panel', 'float', 'col', 'shut', 'drag');
    p.style.cssText = '';
    delete p.dataset.panel;
  }
  selbar.before(P.tools);
  bench.append(P.colours, P.layers, P.stamps, P.piece);
  bench.classList.remove('s-dock', 'empty', 'hot');
  docks.l.remove();
  docks.b.remove();
  root.removeAttribute('data-under');
  pbtn.remove();
  chin.remove();
  root.removeAttribute('data-bare');
  S.rehome();
}

// --- shown: every panel where its layout says ---
const vw = () => document.documentElement.clientWidth;
const width = (id) => Math.round(Math.max(MINW[id] || 200, Math.min(vw() - 16, cur().p[id].w ?? FW[id])));
function apply() {
  if (!on) return;
  const L = cur(), W = vw(), H = innerHeight, focus = document.activeElement;
  if (root.dataset.view !== 'library') under = roomy(); // (the Library has no stage to measure)
  for (const side of SIDES) {
    const d = docks[side], list = IDS.filter((id) => shown(L.p[id].at) === side).sort((a, b) => L.p[a].i - L.p[b].i).map((id) => P[id]);
    const now = [...d.children].filter((e) => e.classList.contains('s-panel'));
    if (list.length !== now.length || list.some((p, n) => now[n] !== p)) d.append(...list);
    d.classList.toggle('empty', !list.some((p) => !L.p[p.dataset.panel].h));
  }
  // (a drag that began with the row there keeps it, empty or not, until the drop: nothing moves under the pointer)
  const held = !!drag?.row && docks.b.classList.contains('empty');
  docks.b.classList.toggle('held', held);
  root.toggleAttribute('data-under', !bare && (held || !docks.b.classList.contains('empty')));
  for (const id of IDS) {
    const p = P[id], q = L.p[id], fl = q.at === 'f';
    p.classList.toggle('float', fl);
    p.classList.toggle('col', !!q.c && id !== 'tools');
    p.classList.toggle('shut', !!q.h);
    const fb = p.querySelector('[data-pa=col]');
    if (fb) { fb.textContent = q.c ? '▸' : '▾'; fb.setAttribute('aria-label', `${q.c ? 'Unfold' : 'Fold'} ${NAME[id]}`); fb.title = fb.getAttribute('aria-label'); fb.setAttribute('aria-expanded', !q.c); }
    if (!fl) { p.style.cssText = ''; continue; }
    if (p.parentNode !== root) root.append(p);
    const x = q.x ?? 80, w = width(id), left = Math.max(0, Math.min(W - w, q.ax === 'r' ? W - x - w : x)), top = Math.max(0, Math.min(H - TB, q.y ?? 200)); // (no place kept: over the stage)
    const room = Math.max(TB, H - top - 8), ht = q.ht != null ? Math.max(MINH, Math.min(room, q.ht)) : null;
    p.style.cssText = `left:${left}px;top:${top}px;width:${w}px;${ht ? `height:${ht}px` : `max-height:${room}px`};z-index:${19 - L.order.indexOf(id)}`;
  }
  root.toggleAttribute('data-bare', bare);
  const b = chin.firstChild;
  b.classList.toggle('on', !bare);
  b.setAttribute('aria-pressed', !bare);
  if (focus && focus !== document.activeElement && focus.isConnected && focus.getClientRects().length) focus.focus({ preventScroll: true });
  S.rehome();
}
// Where a floating panel is kept: from the nearer side, so one at the right stays there as the window widens.
function at(id, left, top) {
  const q = cur().p[id], w = width(id), W = vw();
  q.at = 'f';
  q.y = Math.round(top);
  if (left + w / 2 > W / 2) { q.ax = 'r'; q.x = Math.round(W - left - w); } else { q.ax = 'l'; q.x = Math.round(left); }
}
function front(id) {
  const o = cur().order;
  if (o[0] === id) return;
  o.splice(o.indexOf(id), 1);
  o.unshift(id);
  for (const k of IDS) if (P[k].classList.contains('float')) P[k].style.zIndex = 19 - o.indexOf(k);
  save();
}
function dock(id, side, y) {
  const L = cur(), q = L.p[id], list = IDS.filter((k) => k !== id && shown(L.p[k].at) === shown(side)).sort((a, b) => L.p[a].i - L.p[b].i);
  let n = list.length;
  if (y != null) n = list.filter((k) => { const r = P[k].getBoundingClientRect(); return r.height && (side === 'b' ? r.left + r.width / 2 < y : r.top + r.height / 2 < y); }).length; // (y: along the dock, so across the row under the canvas)
  list.splice(n, 0, id);
  list.forEach((k, i) => { L.p[k].i = i; });
  q.at = side;
}
function float(id) {
  const q = cur().p[id], r = P[id].getBoundingClientRect();
  if (q.x == null || q.y == null) at(id, r.left, Math.min(r.top, innerHeight - 320)); else q.at = 'f'; // (where it is, room below it to use it)
}
function fold(id) {
  if (id === 'tools') return; // (Tools is narrow: no fold, so never a column with no way back)
  const q = cur().p[id];
  q.c = q.c ? 0 : 1;
  save();
  apply();
}
function close(id) {
  cur().p[id].h = 1;
  save();
  apply();
}
// A panel a command needs (the Stamp tool, Save as stamp): shown and unfolded where it is, never moved.
export function show(id) {
  if (!on) return;
  const q = cur().p[id];
  if (!bare && !q.h && !q.c) return;
  q.h = q.c = 0;
  save();
  if (bare) setBare(false); else apply();
}
// H, PANELS: every panel hidden, or back.
export function toggle() { if (on && root.dataset.view !== 'library') setBare(!bare); }
// The notice said under a control in a floating panel: that panel to the front, so nothing in front covers it.
export function raise(id) { if (on && cur().p[id]?.at === 'f') front(id); }

// --- pointers: a title bar dragged, a corner pulled ---
const bar = (e) => {
  const hd = e.target.closest?.('.s-panel>.s-hd');
  return hd && !e.target.closest('button:not(.s-grip), input, select, textarea, a') ? hd : null;
};
function grab(e) {
  if (!on || e.button !== 0) return;
  const sz = e.target.closest?.('.s-panel.float>.s-rsz'), hd = sz ? null : bar(e);
  if (!sz && !hd) return;
  const p = (sz || hd).parentNode, id = p.dataset.panel, r = p.getBoundingClientRect();
  drag = { id, p, size: !!sz, pid: e.pointerId, x0: e.clientX, y0: e.clientY, dx: e.clientX - r.left, dy: e.clientY - r.top, w0: r.width, h0: r.height,
    was: JSON.stringify(cur().p[id]), moved: false, hot: null, cx: e.clientX, cy: e.clientY, row: root.hasAttribute('data-under') };
  if (p.classList.contains('float')) front(id);
}
function move(e) {
  if (!drag || e.pointerId !== drag.pid) return;
  drag.cx = e.clientX;
  drag.cy = e.clientY;
  if (!drag.moved) {
    if (!e.buttons) { drag = null; return; } // (let go outside the page before it moved)
    if (Math.hypot(e.clientX - drag.x0, e.clientY - drag.y0) < 4) return;
    drag.moved = true;
    root.setPointerCapture(e.pointerId); // (on the Studio, which never moves: a panel may change parents as it is dragged; once moving, so a click stays a click)
    drag.p.classList.add('drag');
    if (!drag.size) { drag.dx = Math.min(drag.dx, width(drag.id) - 24); front(drag.id); }
  }
  if (!drag.f) drag.f = requestAnimationFrame(() => frame(drag));
}
// A size kept within bounds: at least a title bar and a little, at most the window.
function size(id, w, h) {
  const q = cur().p[id], r = P[id].getBoundingClientRect();
  q.w = Math.round(Math.max(MINW[id] || 200, Math.min(vw() - r.left, w)));
  q.ht = Math.round(Math.max(MINH, Math.min(innerHeight - r.top - 8, h)));
  if (q.ax === 'r') q.x = Math.round(vw() - r.left - q.w); // (its left edge stays put)
}
function frame(d) {
  if (!d) return;
  d.f = 0;
  if (d.size) size(d.id, d.w0 + d.cx - d.x0, d.h0 + d.cy - d.y0);
  else {
    at(d.id, d.cx - d.dx, d.cy - d.dy);
    const s = root.querySelector('.s-stage').getBoundingClientRect(), u = docks.b.getBoundingClientRect();
    // (under the canvas: over that row's title bars, edge to edge, before either column's edge; with no row, the foot of
    // the stage, where its lit edge is; lower down over the row a panel floats)
    const row = under && d.cx >= s.left && d.cx <= s.right && (u.height ? d.cy >= u.top && d.cy <= u.top + 2 * EDGE : d.cy >= s.bottom - 2 * EDGE && d.cy <= s.bottom);
    d.hot = row ? 'b' : d.cx <= s.left + EDGE ? 'l' : d.cx >= s.right - EDGE ? 'r' : null;
    for (const k of SIDES) docks[k].classList.toggle('hot', d.hot === k);
    root.querySelector('.s-stage').dataset.hot = d.hot || '';
  }
  apply();
}
// Lifted (ok), or lost, cancelled, Escape: back where it was.
function let_go(e, ok) {
  const d = drag;
  if (!d || (e && e.pointerId !== undefined && e.pointerId !== d.pid)) return;
  drag = null;
  cancelAnimationFrame(d.f);
  try { if (root.hasPointerCapture(d.pid)) root.releasePointerCapture(d.pid); } catch { /* gone */ }
  if (d.moved && ok) frame(d);
  d.p.classList.remove('drag');
  for (const k of SIDES) docks[k].classList.remove('hot');
  root.querySelector('.s-stage').dataset.hot = '';
  if (!d.moved) return;
  if (ok) {
    if (d.hot) dock(d.id, d.hot, d.hot === 'b' ? d.cx : d.cy);
    save();
  } else cur().p[d.id] = JSON.parse(d.was);
  apply();
}

// --- keys: the name moves its panel, the corner resizes it ---
const ARROW = { ArrowLeft: [-1, 0], ArrowRight: [1, 0], ArrowUp: [0, -1], ArrowDown: [0, 1] };
function keys(e) {
  if (!on || e.ctrlKey || e.metaKey || e.altKey) return;
  const g = e.target.closest?.('.s-grip, .s-rsz'), v = ARROW[e.key];
  if (!g || (!v && !((e.key === 'Enter' || e.key === ' ') && g.classList.contains('s-grip')))) return;
  e.preventDefault();
  e.stopPropagation(); // (not the Studio's own keys)
  const p = g.closest('.s-panel'), id = p.dataset.panel, q = cur().p[id];
  if (!v) { fold(id); return; }
  const n = e.shiftKey ? 64 : 16, r = p.getBoundingClientRect();
  if (g.classList.contains('s-rsz')) size(id, r.width + v[0] * n, r.height + v[1] * n);
  else {
    if (q.at !== 'f') float(id);
    const r2 = q.at === 'f' && p.classList.contains('float') ? r : { left: q.ax === 'r' ? vw() - q.x - width(id) : q.x, top: q.y };
    at(id, r2.left + v[0] * n, r2.top + v[1] * n);
    front(id);
  }
  save();
  apply();
  g.focus({ preventScroll: true });
}

// --- the buttons: fold, the ⋮ menu, close; the Panels button; PANELS ---
function press(e) {
  if (!on) return;
  const b = e.target.closest?.('.s-pbtn, #s-panels, #s-g-panels');
  if (!b) return;
  if (b.id === 's-g-panels') { toggle(); return; }
  if (b.id === 's-panels') { if (menu?.at === b) shut(true); else panelsMenu(); return; }
  const id = b.closest('.s-panel').dataset.panel, q = cur().p[id];
  if (b.dataset.pa === 'col') fold(id);
  else if (b.dataset.pa === 'close') { close(id); pbtn.focus({ preventScroll: true }); } else if (menu?.at === b) shut(true);
  else {
    const set = (fn) => () => { fn(); save(); apply(); };
    open(b, [
      { label: 'Dock left', tick: q.at === 'l', go: set(() => dock(id, 'l')) },
      { label: 'Dock right', tick: q.at === 'r', go: set(() => dock(id, 'r')) },
      { label: under ? 'Dock under the canvas' : 'Dock under the canvas (in a larger window; right till then)', tick: q.at === 'b', go: set(() => dock(id, 'b')) },
      { label: 'Float', tick: q.at === 'f', go: set(() => float(id)) },
      '-',
      ...(id === 'tools' ? [] : [{ label: q.c ? 'Unfold' : 'Fold', go: () => fold(id) }]),
      { label: 'Close', go: () => close(id), away: true },
    ]);
  }
}
function panelsMenu() {
  const L = cur();
  open(pbtn, [
    ...IDS.map((id) => ({ label: NAME[id], tick: !bare && !L.p[id].h, go: () => {
      const q = L.p[id];
      if (bare) { q.h = 0; save(); setBare(false); } else { q.h = q.h ? 0 : 1; save(); apply(); }
    } })),
    '-',
    { label: bare ? 'Show all panels' : 'Hide all panels', kbd: 'H', go: toggle },
    { label: 'Reset layout', go: () => { lay[cls()] = fresh(); kept[cls()] = false; save(false); setBare(false); } },
  ]);
}
// A menu of buttons under its opener: arrow keys, Escape (focus back), a click outside or a choice closes it.
function open(at, items) {
  shut();
  const m = el('div', 's-pmenu');
  m.setAttribute('role', 'menu');
  for (const it of items) {
    if (it === '-') { m.append(el('hr')); continue; }
    const b = el('button');
    b.type = 'button';
    b.setAttribute('role', it.tick === undefined ? 'menuitem' : 'menuitemcheckbox');
    if (it.tick !== undefined) b.setAttribute('aria-checked', !!it.tick);
    b.append(el('i', '', it.tick ? '✓' : ''), it.label);
    if (it.kbd) b.append(el('kbd', '', it.kbd));
    b.onclick = () => {
      shut();
      it.go();
      (at.getClientRects().length && !it.away ? at : pbtn).focus({ preventScroll: true });
    };
    m.append(b);
  }
  m.addEventListener('keydown', (e) => {
    const bs = [...m.querySelectorAll('button')], n = bs.indexOf(document.activeElement);
    const to = { ArrowDown: n + 1, ArrowUp: n - 1, Home: 0, End: bs.length - 1 }[e.key];
    if (to !== undefined) { e.preventDefault(); bs[(to + bs.length) % bs.length].focus(); } else if (e.key === 'Escape') { e.preventDefault(); shut(true); } else if (e.key === 'Tab') shut(true); // (focus to its opener, then on as Tab goes)
    e.stopPropagation(); // (the Studio's own keys are not for a menu)
  });
  // Focus gone elsewhere (a notice's Escape, a click away): the menu closes.
  m.addEventListener('focusout', (e) => { if (menu?.m === m && !m.contains(e.relatedTarget)) shut(); });
  document.body.append(m);
  const r = at.getBoundingClientRect(), W = vw();
  m.style.left = `${Math.max(8, Math.min(r.left, W - m.offsetWidth - 8))}px`;
  m.style.top = `${r.bottom + 4 + m.offsetHeight <= innerHeight ? r.bottom + 4 : Math.max(8, r.top - m.offsetHeight - 4)}px`;
  at.setAttribute('aria-expanded', 'true');
  menu = { m, at };
  m.querySelector('button').focus({ preventScroll: true });
}
function shut(back) {
  if (!menu) return;
  const { m, at } = menu;
  menu = null;
  m.remove();
  at.setAttribute('aria-expanded', 'false');
  if (back) at.focus({ preventScroll: true });
}
