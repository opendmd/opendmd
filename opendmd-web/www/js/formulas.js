// The Formulas view (`#formulas`; `#gallery=`/`#remix` from the site): maths that draws on
// the panel, in Psi (Ψ). The bundled ones (formulas.json: `[{src}]`, written
// by make web from opendmd-web/formulas/'s `-- bundle` files; each says its
// own name and about), a link to the site's gallery for the rest, the ones saved here,
// details, and the editor (`#formulas/edit`). Drawn by pkg-psi (the
// firmware's crate: the panel's frames byte for byte) through the LED
// preview (`Preview.showRgb`); on a device, Show on panel (js/psishow.js).
// A formula that reads what is under it (`ur ug ub ul`, layers that let it
// show through) is drawn over a GIF: one picked from the Library or a file
// (decoded by pkg-art), else a sample picture; on a device it can be shown
// over the panel's GIF. A Formula's settings (js/fsheet.js: its name, about,
// controls and look, as edits of its declaration lines) make it one that
// can be the panel's look (js/looks.js). The site's Formula page (/f/)
// mounts the editor alone (`editorPage`), with its own actions.

import { app } from './state.js';
import { PanelPreview } from './preview.js';
import { h, clear, on, store, toast, debounce, saver, base, SITE, drawn, keepSelect } from './util.js';
import { popup, closeMenu, actions } from './overrides.js';
import { controls, control, fit, Q, about, TIP, est, marks, sample, declText, fetchFormula, READY, REMIX, MAX_NAME, MAX_PATH, MAX_SRC, compiler } from './psikit.js';
import * as S from './fstore.js';
import { load } from './wasm.js';

let P = null, mod = null, gen = 0;
// The site's Formula page's options (`editorPage`), else null: the web app.
let PAGE = null;
// The site whose gallery and reference these are: the demo's and /f/'s
// own, else opendmd.dev.
const ROOT = () => (app.demo || PAGE ? new URL('../', location.href).href : SITE);
const psi = () => (mod ??= load('pkg-psi', gen || '').then(([m, x]) => { P = { ...m, mem: x.memory }; compiler(P); })
  .catch((e) => { mod = null; throw e; }));
// A trap (a bug) leaves the module unusable: a fresh instance takes over.
function restart() {
  P = mod = held = null;
  over = false;
  gen++;
  psi().then(() => { toast('Formulas restarted'); B?.ed.again(); for (const v of views) v.dirty = true; kick(); });
}
function T(fn, d) {
  try { return P ? fn() : d; } catch (e) {
    if (!(e instanceof WebAssembly.RuntimeError)) throw e;
    restart();
    return d;
  }
}

// The frame clock is the panel's: 50 frames a second, `t` a whole number of 20 ms frames,
// or the formula's `fps` line (info()[10]: 10 or 40 ms). Over a GIF it is the GIF's: a look
// doesn't follow its line.
const D = 20;
const grid = (i) => (i[5] & 32 ? 0 : i[10]) || D;
// What the module holds: compiling is cheap (tens of µs), so whoever draws compiles first.
// `over`: it draws over what is under it.
let held = null, over = false;
const hold = (src) => { held = src; over = src != null && !!(P.info()[5] & 32); return true; };
const take = (src) => T(() => src != null && (src === held || (P.compile(src.slice(0, MAX_SRC)) && hold(src))), false);
// `still`: a thumbnail, drawn at 10:09:30 like the site's gallery (opendmd-gallery's
// preview clock), so one that follows the clock doesn't come out dark at night.
// `gms`: where the GIF under it is (its own clock: ms since the preview started).
// The built-in inputs are their defaults but in the editor's preview while its
// controls are in use (`f.w`, js/fconds.js: the world and the clock it sets).
let wd = 0;
function draw(f, ms, still, gms = ms) {
  if (!take(f.src)) return null;
  const d = new Date(), w = !still && f.w?.on && f.w;
  return T(() => {
    if (w) w.apply(P); else if (wd) P.worldReset();
    wd = !!w;
    const [hh, mm, ss] = still ? [10, 9, 30] : w?.hms || [d.getHours(), d.getMinutes(), d.getSeconds()];
    if (over) new Uint8Array(P.mem.buffer, P.under(), 12288).set(underAt(gms));
    P[over ? 'frameOver' : 'frame'](ms, hh, mm, ss, Q(f.a), Q(f.b), Q(f.c));
    return new Uint8Array(P.mem.buffer, P.pixels(), 12288);
  }, null);
}
const info = (src) => (take(src) && T(() => P.info(), null)) || [];
// What it declares (pkg-psi controls(): its name, about, look, controls).
const ctlOf = (f) => T(() => controls(P, f.src), null);
// Its name and line about it: its own lines, else the record's.
const nameOf = (f) => ctlOf(f)?.name || f.name || '';
const isOver = (f) => !!(info(f.src)[5] & 32);

// --- What is under: the GIF picked (`U`: its name, RGB frames and when each
// ends), else a sample picture (js/psikit.js: a sky, a sun and hills).
let U = null;
function underAt(ms) {
  if (!U) return sample();
  const t = ms % U.total;
  let k = 0;
  while (U.end[k] <= t) k++;
  return U.f[k];
}
const underName = () => (U ? U.name : 'a sample picture');
const underSet = [];
function setUnder(u) {
  U = u;
  for (const v of views) v.dirty = true;
  for (const fn of underSet) fn();
  kick();
}
// A GIF's frames, by pkg-art (the Studio's decoder: the frames the Library's preview shows).
async function loadGif(name, bytes) {
  toast(`Reading “${name}”…`);
  try {
    const m = await (await import('./artmod.js')).art();
    const st = m.Studio.fromGif(bytes);
    while (!st.decode(24)) await new Promise((r) => setTimeout(r));
    const d = st.delays(), f = [], end = [];
    let t = 0;
    for (let i = 0; i < d.length; i++) {
      const a = st.thumb(i), b = new Uint8Array(12288);
      for (let p = 0, q = 0; q < 12288; p += 4, q += 3) { b[q] = a[p]; b[q + 1] = a[p + 1]; b[q + 2] = a[p + 2]; }
      f.push(b);
      end.push(t += Math.max(20, d[i] || 100));
    }
    st.free();
    setUnder({ name, f, end, total: t });
  } catch (e) { toast(`Couldn’t read that GIF: ${e.message || e}`, 'bad'); }
}
// Which GIF: one of the playlist's (the Library's titles), a file, or the sample.
async function pickGif(btn) {
  const r = btn.getBoundingClientRect();
  const q = h('input', { type: 'search', id: 'psi-gq', placeholder: 'Find a GIF', 'aria-label': 'Find a GIF' });
  const list = h('div', { id: 'psi-glist', style: 'max-height:40vh;overflow:auto' });
  const file = h('input', { type: 'file', accept: '.gif,image/gif', hidden: true, onchange: () => { const x = file.files[0]; if (x) { closeMenu(); x.arrayBuffer().then((b) => loadGif(x.name.replace(/\.gif$/i, ''), new Uint8Array(b))); } } });
  let all = [];
  const show = () => {
    const t = q.value.trim().toLowerCase();
    clear(list, all.filter((e) => e.t.toLowerCase().includes(t)).slice(0, 40).map((e) => h('button.mi', { onclick: () => { closeMenu(); app.previewBytes(e.p).then((b) => loadGif(e.t, b), (er) => toast(`Couldn’t read that GIF: ${er.message}`, 'bad')); } }, e.t)));
  };
  q.oninput = show;
  popup(h('div.menu.wide', { id: 'psi-gif-pick', role: 'dialog', 'aria-label': 'Draw it over a GIF' }, h('div.mh', 'Draw it over a GIF'), app.src && q, app.src && list,
    h('button.mi', { id: 'psi-gif-file', onclick: () => file.click() }, 'A GIF from this computer…'), file,
    U && h('button.mi', { id: 'psi-gif-none', onclick: () => { closeMenu(); setUnder(null); } }, 'The sample picture')), r.left, r.bottom + 4);
  if (app.src) {
    const text = await app.text(app.activePath).catch(() => '');
    all = (text ? app.wasm.parseManifest(text) : []).filter((e) => !drawn(e.path)).map((e) => ({ t: e.title || base(e.path), p: e.path }));
    show();
  }
}
// The row that says what a formula is drawn over, with the picker.
function overRow(id) {
  const name = h('b', { id: id + '-name' }), el = h('div.row.cap', { id, hidden: true }, 'Drawn over ', name, h('button.sm', { id: id + '-pick', onclick: (e) => pickGif(e.currentTarget) }, 'Pick a GIF…'));
  const upd = () => { name.textContent = underName(); };
  underSet.push(upd);
  upd();
  return el;
}

// On a device (or the demo's player): a Formula that declares `look` kept
// as the panel's look (Settings › Look), over every GIF (js/looks.js).
const setLook = (f, keep) => import('./looks.js').then((L) => L.useSaid(f, keep));

// Add to playlist: a line in one of the card's playlists (a player's, the
// demo's, a card folder's) naming the Formula (js/fstore.js `line`): a
// bundled one by name, one of yours on the panel by its file (the editor
// saves it first: `keep`), with the values that differ from its defaults;
// else (local mode, no card) the path Show on panel sends (js/psishow.js),
// and `&name=` for one without a `name` line, all in the player's 6,144 B.
async function toList(f, btn, keep) {
  if (!info(f.src).length) { toast('Fix the formula first: it doesn’t work yet', 'bad'); return; }
  if (keep && S.onCard()) f = await keep().catch(() => null);
  if (!f) return;
  const c = ctlOf(f), p = (await import('./psishow.js')).fpath(f, c), q = f.name && !c?.name ? p + '&' + new URLSearchParams({ name: f.name.slice(0, MAX_NAME) }) : p;
  const l = S.line(f, c, q.length > MAX_PATH ? p : q);
  if (l.length > MAX_PATH) { toast('This formula is too long for a playlist', 'bad'); return; }
  const lists = await app.playlists().catch(() => []);
  if (!lists.length) lists.push({ path: '/filelist.txt', label: 'All files' });
  const { appendTo } = await import('./gifs.js');
  actions(btn, 'psi-pl-menu', lists.map((x) => [{ 'data-path': x.path }, x.label, () => appendTo(x.path, [l])]), 'Add to');
}
const plBtn = (id, f, keep) => app.src && h('button', { id, onclick: (e) => toList(f(), e.currentTarget, keep) }, 'Add to playlist…');

const NOTE = () => (app.device ? 'Show on panel plays it there until the playlist moves on (one that draws over a GIF, over the panel’s GIF); save it as a GIF to keep it.' : 'Formulas play here in the browser. Save one as a GIF to play it on an OpenDMD panel.');

let PS = null;
// An error's code and the reference's entry for it (js/psierr.js, imported
// at the first error); the demo's reference is its own site's.
const errRef = (msg) => import('./psierr.js').then((m) => m.refOf(msg, ROOT()), () => null);
// Use as the look: a player's, or the demo's (js/demo.js).
const LOOK = () => app.device || !!app.demo;
const shows = [], showBtn = (id, f) => app.device && shows[shows.push([h('button', { id, onclick: () => PS?.show(f()) }, 'Show on panel'), f]) - 1][0];

// A preview: the LED canvas, the formula it plays and its own clock.
const views = new Set();
let raf = 0;
function loop(now) {
  let on = false;
  for (const v of views) {
    if (!v.f || v.canvas.offsetParent === null) continue;
    on = true;
    // (A formula that never repeats, loop 0: its time since the preview began.)
    const g = v.d || D, n = v.loopMs === 0 ? Infinity : Math.max(1, Math.floor((v.loopMs || 8000) / g)), k = Math.floor((now - v.t0) / g), i = k % n;
    // Over a GIF the picture changes with the GIF too.
    const key = v.over && U ? k : i;
    if (key === v.i && !v.dirty && !v.f.w?.on) continue;
    v.i = key; v.dirty = false;
    const rgb = draw(v.f, i * g, false, now - v.t0);
    if (rgb) { v.pv.p.showRgb(rgb); v.canvas.dataset.frames = -~v.canvas.dataset.frames; }
  }
  // Nothing on screen: stop (kick() starts it again).
  raf = on ? requestAnimationFrame(loop) : 0;
}
export const kick = () => { raf ||= requestAnimationFrame(loop); };
function view(canvas) {
  const v = { canvas, pv: new PanelPreview(canvas), f: null, t0: performance.now(), i: -1, dirty: true };
  v.pv.p.setTransition('none', 0);
  views.add(v);
  return v;
}
function play(v, f, i) {
  if (v.f?.src !== f.src) v.t0 = performance.now();
  v.f = f; v.loopMs = i[4]; v.over = i[5] & 32; v.d = grid(i); v.dirty = true;
  kick();
}

// The controls a formula declares (its `control` lines, and a 0..1 slider for
// a, b or c read without one), in `box`; `set` on a change.
function sliders(box, f, set) {
  const ctl = ctlOf(f);
  clear(box, (ctl?.controls || []).map((c) => control(c, f, set, `${box.id}-${c.slot}`)));
}

// --- Saving and sharing
// Yours: on the panel (a device with a card) or in this browser (js/fstore.js).
const mine = () => S.list();
// Once per card the ones saved in this browser went onto the panel (as the
// view loaded: S.copyUp): said once.
const copied = (n) => n && toast(`${n} Formula${n > 1 ? 's' : ''} saved in this browser ${n > 1 ? 'are' : 'is'} on the panel now`, 'ok');
const stored = () => { const d = store.get('psi.draft', null); return typeof d?.src == 'string' ? d : null; };
// A draft with changes not saved: its text or name differs from where it started.
const sig = (f) => f.src + '\n\0' + (f.name || '');
const dirty = (d) => d?.base != null && sig(d) !== d.base;

// A Formula as `{name, src, a, b, c}` ready for the editor: its values held
// to its controls, its `name` line its name.
function opened(x) {
  const f = { name: (x.name || '').slice(0, MAX_NAME), src: (x.src || '').slice(0, MAX_SRC) }, c = P && controls(P, f.src);
  for (const k of 'abc') if (x[k] != null) f[k] = x[k];
  fit(c, f);
  if (c?.name) f.name = c.name;
  f.base = sig(f);
  return f;
}
async function copy(text) {
  try { await navigator.clipboard.writeText(text); return true; } catch { /* not a secure context: the old way */ }
  const t = h('textarea', { value: text, readOnly: true, style: 'position:fixed;opacity:0' });
  document.body.append(t);
  t.select();
  const ok = document.execCommand('copy');
  t.remove();
  return ok;
}
// The online gallery: every Formula OpenDMD has made (the panel bundles ten)
// and the ones people share.
const GALLERY = () => ROOT() + 'formulas/';
// Copy code: the text, as it is.
const copyCode = async (f) => { const ok = await copy(f.src); toast(ok ? 'Code copied' : 'Couldn’t copy it: select the code and copy it', ok ? 'ok' : 'bad'); };
// Submit to the gallery (js/fsubmit.js, the site's page's own form): in a sheet.
async function submit(btn, f) {
  const r = btn.getBoundingClientRect(), x = () => ({ ...f, ok: !!info(f.src).length, info: info(f.src), ctl: ctlOf(f) });
  const m = await import('./fsubmit.js');
  const s = m.form(P, x);
  popup(h('div.menu.wide', { id: 'psi-submit-sheet', role: 'dialog', 'aria-label': 'Submit to the gallery' }, h('div.mh', 'Submit to the gallery'), s.el), r.left, r.bottom + 4);
}

// "Save as GIF…": one loop into Create Art: 20 ms frames (a GIF can't do 10: `fps 100`
// saves at 50), 40 with `fps 25` or past 240, cut at 240.
// Over a GIF: the GIF's length, or the formula's loop if that is longer and it moves.
// One that never repeats (loop 0): its first FOREVER ms, and a toast says so.
const FOREVER = 10000;
function toGif(f) {
  const i = info(f.src), lp = i[4];
  if (!i.length) { toast('Fix the formula first: it doesn’t work yet', 'bad'); return; }
  const ms = lp || FOREVER;
  if (!lp) toast(`This formula goes on without repeating: saved as its first ${FOREVER / 1000} s`);
  const o = i[5] & 32, len = !o ? ms : i[5] & 24 ? Math.max(ms, U?.total || 0) : U?.total || D;
  let d = Math.max(D, grid(i));
  if (len / d > 240) d = 40;
  const all = Math.max(1, Math.floor(len / d)), n = Math.min(all, 240);
  app.artFrom = { title: (f.name || 'Formula') + (o && U ? ' over ' + U.name : ''), frames: { n, delay: d, cut: all > n, frame: (k) => draw(f, lp ? k * d % lp : k * d, false, k * d) } };
  app.go('create');
}

// --- The tab: the gallery, the details, the editor
const STARTER = `-- x: 0-1 across, y: 0-1 up
-- t: seconds. Drag numbers!
loop 8
v = sin(x*2 - t/8)
hsv(x/2 + t/8, 1, v/2 + 0.5)
`;
let B = null; // the built elements and state
let EX = [];

export async function mount(el) {
  build(el);
  await B.ready;
  return {
    show() { if (!B.editing) B.tiles(); kick(); remixed(); },
    sub(s) {
      if (s !== 'edit') { if (B.editing) close(false); } else if (!B.editing || app.psiOpen || app.psiEdit) edit();
      kick();
      remixed();
    },
  };
}

// `#gallery=<slug>` or `#remix` (the site's Open on my panel, app.js keeps
// it in app.remix): the Formula saved under Your formulas as it arrives
// (js/fstore.js: on the panel, else this browser), then into the editor:
// change it, show it, add it to a playlist. A gallery one by name (a
// bundled one is here already: nothing to save; else fetched from the
// site); an edited one sent by the site's page that opened this one
// (postMessage: its origin opendmd.dev, or this page's own for the demo
// and the tests), never in the link. One that doesn't work isn't saved:
// said, and kept as a draft not saved yet.
async function remixed() {
  const r = app.remix;
  if (!r) return;
  app.remix = null;
  location.replace('#formulas');
  const ex = r.gallery && EX.find((x) => x.stem === r.gallery);
  let f;
  try { f = ex ? { name: ex.name, src: ex.src } : r.gallery ? await fetchFormula(ROOT(), r.gallery) : await sent(); } catch (e) { toast(`Couldn’t open that Formula: ${e.message || e}`, 'bad'); return; }
  f = { ...f, ...r.f };
  if (!ex) {
    try { f = await arrived(f); } catch (e) { toast(`“${f.name || 'This formula'}” isn’t saved: ${e.message || e}`, 'bad'); f.unsaved = true; }
  }
  app.psiOpen = f;
  app.go('formulas/edit');
}
// A Formula from the site, saved under Your formulas (once: the same text
// already there is opened instead), its name written in as its `name` line
// as the editor's Save does. The record, as saved.
async function arrived(f) {
  if (!info(f.src).length) throw new Error(`it doesn’t work yet: ${T(() => P.error(), '').replace(/`/g, '')}`);
  const c = ctlOf(f), name = c?.name || f.name.trim() || `My formula ${mine().length + 1}`;
  const src = (c?.name ? f.src : T(() => P.declare(f.src, 'name', 0, `name "${declText(name, MAX_NAME)}"`), '') || f.src).slice(0, MAX_SRC);
  const vals = {};
  for (const k of 'abc') if (f[k] != null) vals[k] = f[k];
  const had = mine().find((x) => x.src === src || x.src === f.src);
  if (had) { toast(`“${name}” is already under Your formulas`, 'ok'); return { ...had, ...vals }; }
  const rec = await S.put({ name, src, ...vals });
  toast(`“${name}” is saved ${S.onCard() ? 'on your panel' : 'in this browser'}, under Your formulas`, 'ok');
  return rec;
}
// The edited Formula from the page that opened this one: READY to it, then
// its answer, `{type: REMIX, name, src, a, b, c}`, from opendmd.dev only.
function sent() {
  const from = [new URL(SITE).origin, location.origin], o = window.opener;
  if (!o) return Promise.reject(new Error('open it again from opendmd.dev'));
  return new Promise((res, rej) => {
    const t = setTimeout(() => { removeEventListener('message', on); rej(new Error('opendmd.dev didn’t send it: open it again from there')); }, 10000);
    function on(e) {
      const d = e.data;
      if (e.source !== o || !from.includes(e.origin) || d?.type !== REMIX || typeof d.src != 'string' || (d.name != null && typeof d.name != 'string')) return;
      clearTimeout(t);
      removeEventListener('message', on);
      const f = { name: (d.name || '').slice(0, MAX_NAME), src: d.src.slice(0, MAX_SRC) };
      for (const k of 'abc') if (typeof d[k] == 'number' && Number.isFinite(d[k]) && Math.abs(d[k]) < 32768) f[k] = d[k];
      res(f);
    }
    addEventListener('message', on);
    for (const f of from) o.postMessage({ type: READY }, f);
  });
}

// An in-page question (a sheet on a phone): `yes` or `no`, Escape is neither.
function ask(text, [y, yes], [n, no], at) {
  const r = at?.getBoundingClientRect?.() || { left: innerWidth / 2 - 125, bottom: 120 };
  const b = (t, fn, c = '') => h('button.mi' + c, { onclick: () => { closeMenu(); fn(); } }, t);
  popup(h('div.menu', { id: 'psi-ask', role: 'alertdialog', 'aria-label': text }, h('p', { style: 'margin:4px 8px' }, text), b(y, yes, '.danger'), b(n, no)), r.left, r.bottom + 4);
}
const unsaved = () => { const d = B.draft || stored(); return dirty(d) && d; };

function build(root) {
  const gal = h('div', { id: 'psi-gal' });
  const canvas = h('canvas.panel', { id: 'psi-panel' });
  const cap = h('div.cap', { id: 'psi-cap' });
  const title = h('h2', { id: 'psi-d-title' }), desc = h('p.cap', { id: 'psi-d-desc' });
  const sl = h('div.psi-sls', { id: 'psi-sl' });
  const main = h('button.pri', { id: 'psi-edit' }), gifBtn = h('button', { id: 'psi-gif' }, 'Save as GIF…');
  const codeBtn = h('button', { id: 'psi-code-btn' }, 'Copy code'), del = h('button.danger', { id: 'psi-del' }, 'Delete…');
  let cur = null;
  const show = showBtn('psi-show', () => cur);
  const look = LOOK() && h('button', { id: 'psi-look', onclick: () => setLook({ ...cur, name: nameOf(cur) }) }, 'Set as the panel’s look');
  const sets = h('button', { id: 'psi-settings' }, 'Settings…');
  const pl = plBtn('psi-pl', () => cur);
  const under = overRow('psi-under');
  // How long Show on panel (and Play now) keeps a Formula there: the panel's `formula_keep`.
  const keep = app.device && app.src.config && keepSelect('psi-keep', app.config?.formula_keep);
  if (keep) {
    keep.onchange = () => app.src.putConfig({ formula_keep: keep.value }).then(() => app.refreshConfig(), (e) => toast('Keep on the panel: ' + e.message, 'bad'));
    on('config', (c) => { keep.value = c.formula_keep || 'turn'; });
  }
  const side = h('div.side', { id: 'psi-side' }, h('div.card', { id: 'psi-detail' },
    h('button.sm.close', { 'aria-label': 'Close', onclick: () => side.classList.remove('open') }, '✕'),
    canvas, cap, under, h('div.dhead', h('div.grow', title, desc)), sl,
    h('div.row.acts', show, main, gifBtn), keep && h('label.row', { id: 'psi-keep-row' }, 'Keep on the panel', keep),
    h('div.row.acts', pl, look, sets, codeBtn, del), h('p.cap', NOTE())));
  const ed = editor();
  root.append(h('h1.ptitle', 'Formulas'), h('div.libgrid', gal, side), ed.el);
  addEventListener('keydown', (e) => { if (e.key === 'Escape' && side.classList.contains('open') && !document.querySelector('.menu')) side.classList.remove('open'); });
  const pv = view(canvas);
  function pick(f, mineToo) {
    cur = f;
    side.classList.add('open');
    const i = info(f.src), c = ctlOf(f);
    title.textContent = nameOf(f);
    desc.textContent = c?.about || f.desc || '';
    cap.textContent = i.length ? about(i) : `This formula doesn’t work any more: ${T(() => P.error(), '')}`;
    cap.title = i.length ? TIP(i) : '';
    sliders(sl, f, () => { pv.dirty = true; PS?.follow(f); });
    if (show) show.disabled = !i.length;
    under.hidden = !(i[5] & 32);
    if (look) look.hidden = !i.length || !c?.look;
    if (pl) pl.hidden = !i.length;
    PS?.mark();
    main.textContent = mineToo ? 'Edit' : 'Remix';
    const mk = () => (mineToo ? { ...f } : { name: nameOf(f) + ' remix', src: c?.name ? P.declare(f.src, 'name', 0, `name "${Array.from(c.name + ' remix').slice(0, MAX_NAME).join('')}"`) : f.src, a: f.a, b: f.b, c: f.c });
    main.onclick = (e) => go(mk(), e.currentTarget);
    // Its settings: in the editor (an example's on a remix).
    sets.onclick = (e) => go(mk(), e.currentTarget, 1);
    sets.hidden = !i.length;
    gifBtn.onclick = () => toGif(f);
    codeBtn.onclick = () => copyCode(f);
    del.hidden = !mineToo;
    del.onclick = (e) => ask(`Delete “${f.name}”?`, ['Delete', () => S.drop(f).then(() => { side.classList.remove('open'); cur = null; tiles(); }, (x) => toast('Couldn’t delete it: ' + x.message, 'bad'))], ['Keep it', () => {}], e.currentTarget);
    for (const t of gal.querySelectorAll('.psi-tile')) t.classList.toggle('cur', t._f === f);
    if (i.length) play(pv, f, i); else { pv.f = null; pv.pv.clear(); }
  }
  function tile(f, m) {
    const c = h('canvas', { width: 128, height: 32 });
    const d = ctlOf(f), t = h('button.psi-tile', { 'data-name': nameOf(f), onclick: () => pick(f, m) }, c, h('b', nameOf(f)), (d?.about || f.desc) && h('span.cap', d?.about || f.desc), d?.look && h('span.psi-badge', 'look'));
    t._f = f;
    const rgb = draw(f, Math.floor((info(f.src)[4] || 8000) * 0.3 / D) * D, true);
    if (rgb) {
      const img = new ImageData(128, 32);
      for (let p = 0, q = 0; p < 12288; p += 3, q += 4) img.data.set([rgb[p], rgb[p + 1], rgb[p + 2], 255], q);
      c.getContext('2d').putImageData(img, 0, 0);
    }
    return t;
  }
  function tiles() {
    const ms = mine(), d = unsaved();
    clear(gal,
      d && h('div.card.row', { id: 'psi-cont' }, h('span.grow', 'Not saved yet: “', h('b', d.name || 'Untitled formula'), '”'),
        h('button.pri', { id: 'psi-cont-go', onclick: () => { B.draft = d; B.pushed = true; app.go('formulas/edit'); } }, 'Continue editing'),
        h('button.txt', { id: 'psi-cont-drop', onclick: () => { B.draft = null; store.set('psi.draft', null); tiles(); } }, 'Discard')),
      h('div.card.psi-intro', h('p', 'Formulas are a few lines of maths that draw on the panel, every pixel at once. Pick one to play with it, change a number and watch.'),
        h('div.row', h('button.pri', { id: 'psi-new', onclick: (e) => go({ name: '', src: STARTER }, e.currentTarget) }, '+ New formula'),
          h('span.cap', 'Written in Psi ', h('span.psi-mark', { title: 'Psi, the language formulas are written in' }, 'Ψ')))),
      ms.length > 0 && h('div.card', h('h3', 'Your formulas'), h('div.psi-tiles', { id: 'psi-mine' }, ms.map((f) => tile(f, true)))),
      h('div.card', h('h3', 'Bundled'), h('p.cap', 'These come with the panel: clocks, screensavers, and looks that draw over what plays (pick a GIF to try them on).'),
        h('div.psi-tiles', { id: 'psi-ex' }, EX.map((f) => tile(f, false))),
        h('div.row', { style: 'margin-top:12px' }, h('a.btn', { id: 'psi-more', href: GALLERY(), target: app.demo ? null : '_blank', rel: 'noopener' }, 'More Formulas'),
          h('span.cap', 'In the online gallery: tunnels, plasma, fire and more. Open one there and send it to your panel.'))));
    if (cur) for (const t of gal.querySelectorAll('.psi-tile')) t.classList.toggle('cur', t._f.src === cur.src && t._f.name === cur.name);
    // Two columns: the details are never empty (a sheet on a phone waits for a pick).
    else if (EX[0] && matchMedia('(min-width:1001px)').matches) pick(EX[0], false);
  }
  B = { root, ed, tiles, pick, editing: false, draft: null };
  B.ready = Promise.all([psi(), app.device && import('./psishow.js').then((m) => { PS = m; m.init(shows, isOver, ctlOf); }), new Promise((res) => document.head.append(h('link', { rel: 'stylesheet', href: './formulas.css', onload: res, onerror: res }))), fetch('formulas.json').then((r) => r.json()).then((x) => { EX = Array.isArray(x) ? x.filter((f) => typeof f?.src == 'string') : []; }), S.load().then(async () => { await psi(); B.copied = await S.copyUp(); const bad = await S.prepare(() => toast('Getting your Formulas ready for the panel…')); if (bad) toast(`Couldn’t get ${bad} of your Formulas ready for the panel`, 'bad'); }, (e) => toast('Couldn’t read your Formulas on the panel: ' + (e.message || e), 'bad'))])
    // (Each says its own name: `[{src}]`.)
    .then(() => { EX = EX.map((f) => ({ ...f, name: nameOf(f) })); tiles(); copied(B.copied); })
    .catch((e) => { clear(gal, h('div.card', h('p.bad', 'Formulas could not be loaded: ' + (e.message || e)))); });
}

// --- The editor (`#formulas/edit`)
// psi-builtins: cheat sheet (generated from opendmd-psi/src/builtins.rs: PSI_BLESS=1 cargo test --features docs --test builtins)
const VARS = [
  ['x, y', 'where the pixel is: 0 to 1 across from the left, and down from the top'],
  ['px, py', 'the pixel’s column 0–127 from the left and row 0–31 from the top'],
  ['t', 'seconds into the loop'],
  ['hour, minute, second', 'the time of day'],
  ['a, b, c', 'the controls, in the order the control lines declare them (0 to 1 when undeclared); any built-in name may be set as your own'],
  ['w, h', '128 and 32'],
  ['i', 'the pixel’s number, 0–4095'],
  ['cx, cy', 'from the middle: −2 to 2 across, −0.5 to 0.5 down'],
  ['r, angle', 'distance and angle (turns, clockwise from the right) from the middle'],
  ['ur, ug, ub', 'the colour under the formula: the GIF it is drawn over, 0 to 1'],
  ['ul', 'how bright that is'],
  ['under(x, y)', 'the picture under at pixel x, y (the nearest, held to the panel): a colour, as #…: under(px + 2, py) shifts it, under(floor(px/4)*4, floor(py/4)*4) pixelates it; under alone is this pixel\'s'],
  ['daylight', 'how light it is outside where the panel is, 0 at night to 1 by day (twilight between); the panel\'s location in Settings, else 1'],
  ['sun.alt', 'the sun\'s height in turns: 0 on the horizon, 0.25 straight up, below 0 at night; 0.1 without a location'],
  ['moon.phase, season, yearday', 'the moon (0 new, 0.5 full), the time of year (0 midwinter, 0.5 midsummer) and the day of the year (0 on 1 January)'],
  ['night, bright', '1 while night mode dims the panel; the panel\'s brightness, 0 to 1'],
  ['replay.playing, replay.platform, replay.game', 'RePlayOS: 1 while a game plays; its platform as a number (0 none); a number 0 to 1 for the game, the same each time it plays']
];
const FUNCS = [
  ['sin(v) cos(v)', 'a wave: one turn per 1, between −1 and 1'],
  ['tri(v) saw(v) sqr(v)', 'triangle, sawtooth and square waves'],
  ['abs floor frac sqrt', 'size, whole part, fraction, square root'],
  ['min max clamp(v, lo, hi)', 'limits'],
  ['mix(a, b, k)', 'from a to b as k goes 0 to 1'],
  ['step(edge, v)', '1 past the edge, else 0'],
  ['smoothstep(e0, e1, v)', 'a soft step'],
  ['len(dx, dy)', 'distance'],
  ['atan2(dy, dx)', 'angle, in turns'],
  ['noise(x, y, z)', 'smooth random hills, 0 to 1'],
  ['hash(a, b)', 'random 0 to 1'],
  ['select(c, a, b)', 'a when c is true (not 0), else b; or c ? a : b'],
  ['pick(k, a, b, …)', 'a at k = 0, b at 1, …; else 0'],
  ['since(replay.game)', 'seconds since a built-in input last changed (32767 when it never has): a flash is max(0, 1 - since(replay.game))'],
  ['was(replay.game)', 'a built-in input\'s value before its last change: mix(was(k), k, min(1, since(k))) fades from the old to the new'],
  ['digit(n, col, row)', 'the panel’s font: 1 where digit n has a pixel; col 0–4, row 0–6 from the top; add , small or , bold (chunky digits) for another font'],
  ['glyph(\'A\', col, row)', 'the same for a character'],
  ['bit(n, i)', 'bit i of a whole number: 1 or 0'],
  ['< <= > >= == !=', 'compare: 1 or 0; and, or, not'],
  ['%', 'remainder: v % 1 is the fraction'],
  [';', 'a new line, on the same line'],
  ['loop 8', 'the loop’s length in seconds (first line)'],
  ['loop forever', 'goes on without repeating: t grows from when it starts (use frac, %, sin, noise of it); seed is 0 to 1, new each time it plays'],
  ['fps 100', 'frames a second: 25, 50 (else) or 100, for fast motion; a light formula only'],
  ['layer add { … }', 'a layer with its own names and colour, on what is below: over, add, multiply, screen, max or min'],
  ['layer rate 4 { … }', 'a slow bottom layer: redrawn every 4th frame, a band of rows a frame (or when the clock or a control it reads changes); the layers above draw over it every frame', 'layer rate 4 {\n  \n}\n'],
  ['fn ring(cx, r) = abs(len(px - cx, py - 16) - r) < 1', 'a function: a value worked out from its values, used as ring(64, 8); with { … } instead of = …, lines that draw, called on a line of their own in draw', 'fn f(a) = '],
  ['fn p(x, y) { … }', 'a function whose values are x and y: px, py, r, angle … inside follow them; its last line a colour, p(1 - x, y) draws it mirrored', 'fn p(x, y) {\n  \n}\n'],
  ['use Plasma as p', 'puts in one of your formulas or an example, as a function fn p(x, y) { … }', 'use Plasma as p\n']
];
const DRAW = [
  ['draw { … }', 'once a frame, before the pixels: values, loops, lines and dots into ink', 'draw {\n  \n}\n'],
  ['for k < 12 { … }', 'inside draw: k = 0, 1, … 11; two deep at most, 2048 turns a frame', 'for k < 12 {\n  \n}'],
  ['line(x0, y0, x1, y1, v0, v1)', 'a line, in pixels from the top left; v0, v1 its ink at each end, 0 to 1 (else 1)'],
  ['dot(x, y, v)', 'one pixel of ink v'],
  ['cls(#102)', 'paint: the whole panel one colour (paint is drawn over the colour line; the last painted wins)'],
  ['pset(x, y, #f80)', 'paint one pixel; line(x0, y0, x1, y1, c) and dot(x, y, c) paint too when their last value is a colour'],
  ['rect(x0, y0, x1, y1, c) rectfill(…)', 'paint a box (its corners included), or a filled one'],
  ['circ(x, y, r, c) circfill(…)', 'paint a circle, or a filled one'],
  ['trifill(x0, y0, x1, y1, x2, y2, c)', 'paint a filled triangle'],
  ['print("TEXT", x, y, c)', 'paint text in the panel\'s font (add small); or a number: print(minute, x, y, c, small, 2) with at least 2 digits'],
  ['spr(s, x, y, flip)', 'paint sprite s with its top left at x, y (its key gives colours: key . 0, R #f00); flip 1 across, 2 down, 3 both'],
  ['pal(fire, v), gray(v)', 'colours to paint with besides #…: a palette\'s or a gray, v 0 to 1'],
  ['if c { … } else { … }', 'inside draw: only when c is not 0', 'if  {\n  \n}'],
  ['ink', 'what draw put at the pixel, 0 to 1: pal(ice, ink), #40c0ff * ink'],
  ['ink(16)', 'the same in 16 levels, a whole number: what was drawn with ink k/16 reads k, exactly (pick(ink(16), …)); 1 to 255 levels'],
  ['table T = 1, -2, 0.5', 'numbers, 256 a table and 1024 in all (tables and sprites)', 'table T = '],
  ['T[k]', 'number k of table T, from 0 (0 outside it)'],
  ['sprite s { … }', 'a small picture in characters: first its key, key . 0, X 1 (each character and the number it reads as, up to 16), then rows up to 64 wide and 32 tall', 'sprite s {\n  key . 0, X 1\n  .X.\n}\n'],
  ['s[u, v]', 'sprite s at column u, row v from its top left: its key’s number (0 outside)']
];
const COLS = [
  ['hsv(hue, sat, val)', 'colour: hue in turns, 0 to 1 each'],
  ['rgb(r, g, b)', 'red, green, blue: 0 to 1'],
  ['pal(fire, v)', 'a palette: fire, ice, ocean, chrono, amber, rainbow, gray'],
  ['palette sea = #024, #08c, #fff', 'your own palette, up to 32 colours (8 palettes; pal and cycle read 3): pal(sea, 0) the first, pal(sea, 1) the last, pal(sea, 0.5) the middle one', 'palette p = #000, #f80, #fff\n'],
  ['scene s when c { v = … }', 'values worked out only while c (once a frame or a row) is not 0; after it its names are 0 while it is off; the estimate counts two scenes at once', 'scene s when  {\n  \n}\n'],
  ['palette sky = tween(day, night, w)', 'a palette mixed once a frame, day towards night by w (0 to 1, of t or sliders); tweens nest; tween(a, b, w), tween(c, d, v) fills the low and high 16 colours', 'palette p = tween(, , )\n'],
  ['palette sea = deep #024, foam #fff', 'a colour may have a name: sea.foam is where pal finds it (sea.step from one colour to the next, sea.n how many)'],
  ['pal(fire, v) * k', 'a palette colour, dimmed (or lit) by k'],
  ['cycle(fire, v, s)', 'palette cycling: the palette turned by s (one turn per 1: t/4 turns it every 4 s), at v; wraps where pal stops; s may change by row; cheap when v is the same every frame'],
  ['gray(v)', 'white, 0 to 1'],
  ['#ff8800', 'a colour: alone, or #f80 * v, mix(#000, #f80, v)'],
  ['rgb(r, g, b, k)', 'a last value: how much it covers, 0 to 1 (what is under shows through); rgb(#f80, k)']
];
const DECLS = [
  ['name "Sunrise"', 'the formula’s name (60 characters at most)', 'name ""\n'],
  ['about "…"', 'one line about it (120 at most)', 'about ""\n'],
  ['control speed 0..1 = 0.5 "Speed"', 'a slider: its name in the formula, range, default and label (three controls at most)', 'control speed 0..1 = 0.5 "Speed"\n'],
  ['control mode = calm, wild', 'buttons: the value is the choice’s number, 0 for the first (pick(mode, …)); two words make a switch', 'control mode = calm, wild\n'],
  ['look screen 40', 'it can be the panel’s look: drawn over the GIF with a blend and an amount (look alone: over, 100)', 'look\n']
];
// psi-builtins end

function editor() {
  // The name is the formula's `name` line (typed here, written there).
  const name = h('input', { type: 'text', id: 'psi-name', placeholder: 'Untitled formula', maxLength: 60, 'aria-label': 'Name', oninput: () => { B.draft.name = name.value; named(name.value); stash(); } });
  const setBtn = h('button', { id: 'psi-set-btn', 'aria-haspopup': 'dialog', title: 'Formula settings: name, about, controls, look', onclick: (e) => settings(e.currentTarget) }, '⚙ Settings');
  // (The site's page: no way back to a gallery, nothing saved there.)
  const head = h('div.card.psi-edhead', !PAGE && h('button', { id: 'psi-back', onclick: () => close(true) }, '‹ Formulas'), name, setBtn,
    !PAGE && h('button.pri', { id: 'psi-save', onclick: () => save().catch(() => {}) }, 'Save'));
  const said = saver(head);
  const ta = h('textarea', { id: 'psi-src', spellcheck: false, autocapitalize: 'off', autocomplete: 'off', wrap: 'off', 'aria-label': 'Formula', 'aria-describedby': 'psi-err' });
  ta.setAttribute('autocorrect', 'off');
  const hl = h('pre.psi-hl', { 'aria-hidden': 'true' }), gut = h('div.psi-gut', { 'aria-hidden': 'true' });
  const err = h('div.cap.bad', { id: 'psi-err', 'aria-live': 'polite' });
  const val = h('b.mono', { id: 'psi-num' });
  const pad = h('div.psi-pad', { id: 'psi-pad', role: 'slider', 'aria-label': 'Drag to change the number' }, '◀ ', val, ' ▶');
  const nudge = (k) => h('button.sm', { 'aria-label': k > 0 ? 'More' : 'Less', onpointerdown: (e) => e.preventDefault(), onclick: () => bump(k) }, k > 0 ? '+' : '−');
  const strip = h('div.psi-strip', { id: 'psi-scrub', hidden: true }, nudge(-1), pad, nudge(1));
  const help = h('button.sm', { id: 'psi-help', 'aria-haspopup': 'dialog', onclick: (e) => cheat(e.currentTarget) }, 'Cheat sheet');
  const canvas = h('canvas.panel', { id: 'psi-ed-panel' });
  const cap = h('div.cap', { id: 'psi-ed-cap' }), heavy = h('p.cap', { id: 'psi-heavy', hidden: true },
    'Heavy for the panel: noise, len, sqrt, atan2 and dividing by a changing value cost the most');
  const sl = h('div.psi-sls', { id: 'psi-ed-sl' });
  // A formula that reads a built-in input: its controls (js/fconds.js).
  const cond = h('div', { hidden: true });
  let W = null;
  const under = overRow('psi-ed-under');
  const look = LOOK() && h('button', { id: 'psi-ed-look', hidden: true, onclick: () => { if (ready()) setLook({ ...live }, () => keep()); } }, 'Set as the panel’s look');
  const touch = matchMedia('(pointer:coarse)').matches;
  // One card: the preview (stuck to the top on a phone), the sliders, the
  // error over the code, the code, the actions.
  const top = h('div.psi-top', canvas, cap, under);
  const el = h('div', { id: 'psi-ed' }, head, h('div.card.psi-edb', top, h('div.psi-sl2', sl, heavy), cond, err,
    h('div.psi-code', h('div.psi-src', gut, h('div.psi-wrap', hl, ta)), strip,
      h('div.row.acts', help, h('span.cap', touch ? 'Tap a number, then drag the bar under the code to try other values' : 'Drag a number (or Alt + ↑ ↓) to try other values'))),
    h('div.psi-acts', PAGE ? [h('div.row.acts', PAGE.acts), h('p.cap', PAGE.note)] : [h('div.row.acts', showBtn('psi-ed-show', () => live), plBtn('psi-ed-pl', () => ({ ...live }), () => keep()), look, h('button', { id: 'psi-ed-gif', onclick: () => { if (ready()) toGif(B.draft); } }, 'Save as GIF…'),
      h('button', { id: 'psi-ed-code', onclick: () => copyCode({ src: ta.value }) }, 'Copy code'),
      h('button', { id: 'psi-ed-submit', onclick: (e) => { if (ready()) submit(e.currentTarget, B.draft); } }, 'Submit to the gallery…')), h('p.cap', NOTE())])));
  let pv = null, toks = [], bad = null, num = null, good = null;
  // What the preview plays: the last text that compiled, with the sliders as they are.
  const live = { get name() { return B.draft.name; }, get src() { return good; }, get a() { return B.draft.a; }, get b() { return B.draft.b; }, get c() { return B.draft.c; }, get w() { return cond.hidden ? null : W?.w; } };

  // The draft kept in this browser (the web app's; the site's page tells its own of each change).
  const stash = PAGE ? () => PAGE.changed?.() : debounce(() => store.set('psi.draft', B.draft), 300);
  // Numbers (a unary minus included), comments and the error, drawn under the (transparent) text.
  function paint() {
    const s = ta.value, m = marks(s, bad);
    toks = m.toks;
    hl.innerHTML = m.html + '\n';
    const n = s.split('\n').length, el = bad && s.slice(0, bad[0]).split('\n').length;
    gut.innerHTML = Array.from({ length: n }, (_, k) => (k + 1 === el ? `<b>${k + 1}</b>` : k + 1)).join('\n');
    ta.style.height = `calc(${Math.max(8, n + 1) * 1.5}em + 18px)`;
  }
  const build = debounce(() => compile(), 150);
  // `use Plasma as p`: one of yours or an example put in as a function,
  // `fn p(x, y) { … }` (pkg-psi asFunction; the shared text stands alone),
  // once the caret is off the line — every one (`all`) before saving or
  // sharing. Never the formula itself; at most 16 a time. One it can't put
  // in: {s, e, why}.
  function paste(all) {
    for (let k = 0; ; k++) {
      const s = ta.value, cl = s.slice(0, ta.selectionStart).split('\n').length, me = name.value.trim().toLowerCase();
      const m = [...s.matchAll(/^([ \t]*)use[ \t]+(.+?)[ \t]+as[ \t]+([A-Za-z_]\w*)[ \t]*$/gm)]
        .find((m) => all || document.activeElement !== ta || s.slice(0, m.index).split('\n').length !== cl);
      if (!m) return;
      const n = m[2].trim(), at = { s: m.index, e: m.index + m[0].length }, i = m[1];
      const f = [...mine(), ...EX].find((x) => x.name.toLowerCase() === n.toLowerCase() && (x.id == null || x.id !== B.draft.id) && x.name.toLowerCase() !== me);
      if (!f || k > 15) return { ...at, why: k > 15 ? 'too many formulas put in at once' : n.toLowerCase() === me ? 'a formula can’t be put inside itself' : `there’s no formula called “${n}” in yours or the bundled ones` };
      const fn = T(() => P.asFunction(f.src, m[3]), '');
      if (!fn) return { ...at, why: `“${f.name}” can’t be put in as a function: only a formula with one colour line can (no layers, drawing, tables, sprites or palettes)` };
      const t = `${i}-- ${f.name}\n${fn.replace(/^/gm, i)}`, c = ta.selectionStart;
      // Before saving or sharing: focused, so one undo takes it back. Not on
      // opening a draft or a link (no phone keyboard, the name keeps focus).
      if (all) ta.focus({ preventScroll: true });
      put(at, t);
      // The caret stays where it was (one undo takes the paste back).
      const q = c > m.index ? c + t.length - m[0].length : c;
      ta.setSelectionRange(q, q);
      const lp = (x) => +(x.match(/^loop\s+([\d.]+)/m) || [0, 8])[1];
      toast(`“${f.name}” is in as ${m[3]}(x, y): draw it with ${m[3]}(x, y) as a last line` + (lp(f.src) > lp(s) ? `; its loop (${lp(f.src)} s) is longer than this formula’s — try loop ${lp(f.src)}` : ''));
    }
  }
  // A use line it couldn't paste: said, and returned (nothing is saved or shared).
  function compile(all) {
    const f = B.draft, miss = paste(all);
    f.src = ta.value;
    // (The module restarting: the next compile catches up.)
    const ok = miss ? false : T(() => P.compile(f.src), null);
    if (ok == null) return;
    if (ok) hold(f.src); else held = null;
    if (ok) {
      bad = null;
      err.textContent = '';
      const i = info(f.src);
      cap.textContent = about(i);
      cap.title = TIP(i);
      heavy.hidden = est(i) <= 8000;
      under.hidden = !(i[5] & 32);
      const c = ctlOf(f);
      if (look) look.hidden = !c?.look;
      // Its `name` line, typed in the text: the name.
      if (c?.name != null) f.name = c.name;
      if (document.activeElement !== name) name.value = f.name || '';
      el.dataset.steps = i[0];
      delete el.dataset.err;
      good = f.src;
      top.classList.remove('stale');
      sliders(sl, f, () => { stash(); pv.dirty = true; PS?.follow(live); });
      // (What it reads, while the module holds it.)
      const bi = i[5] & 128 && JSON.parse(T(() => P.builtins(), '[]'));
      cond.hidden = !bi;
      if (bi) import('./fconds.js').then((m) => { (W ??= m.conditions(P, cond)).keys(bi, i[5] & 16); });
      play(pv, live, i);
      PS?.follow(live);
    } else {
      bad = miss ? [miss.s, miss.e] : T(() => P.errorAt(), [0, 0]);
      top.classList.toggle('stale', good != null);
      const line = ta.value.slice(0, bad[0]).split('\n').length;
      el.dataset.err = line;
      const msg = miss ? miss.why : T(() => P.error(), '');
      clear(err, `Line ${line}: `, msg.split('`').map((t, k) => (k % 2 ? h('code', t) : t)));
      // Its code, linked to the language reference's entry (the editor's
      // own messages, `miss`, have none).
      const shown = err.textContent;
      if (!miss) errRef(msg).then((r) => { if (r && err.textContent === shown) err.append(' ', h('a.ecode', { href: r.href, target: '_blank', rel: 'noopener', title: 'What this error means' }, r.code)); });
      paint();
    }
    stash();
    return miss;
  }
  // Before saving or sharing: every use line pasted, or nothing happens.
  const ready = () => { const m = compile(true); if (m) toast(`First fix line ${ta.value.slice(0, m.s).split('\n').length}: ${m.why}`, 'bad'); return !m; };
  ta.addEventListener('input', () => { paint(); scrubbable(); build(); });
  ta.addEventListener('scroll', () => { hl.scrollLeft = ta.scrollLeft; });

  // Draggable numbers: under the pointer (a mouse or pen, straight in the
  // text), Alt+↑↓ at the caret, and on touch the strip under the code.
  // Edits go through insertText so the browser's undo has them.
  const at = (pos) => toks.find((t) => t.s <= pos && pos <= t.e);
  function put(t, text, quiet) {
    // Focused: as typing (the browser's undo keeps it); not (a phone's strip
    // with the keyboard down), or mid-drag: straight in, no keyboard popping up.
    if (document.activeElement === ta && !quiet) {
      ta.setSelectionRange(t.s, t.e);
      document.execCommand('insertText', false, text);
    } else {
      ta.setRangeText(text, t.s, t.e, 'end');
      ta.dispatchEvent(new Event('input'));
    }
    t.e = t.s + text.length;
    ta.setSelectionRange(t.e, t.e);
  }
  // Below zero is fine (a minus that would make `--`, a comment, gets brackets).
  function fmt(t, v, d) {
    const x = (Math.abs(v) < 10 ** -d / 2 ? 0 : v).toFixed(d);
    return x[0] === '-' && ta.value[t.s - 1] === '-' ? `(${x})` : x;
  }
  const decs = (t) => (ta.value.slice(t.s, t.e).split('.')[1] || '').replace(/\D/g, '').length;
  const num0 = (t) => parseFloat(ta.value.slice(t.s, t.e).replace(/[()]/g, ''));
  function bump(k, t = num) {
    if (!t) return;
    const d = decs(t), step = d ? 10 ** -d : 1;
    put(t, fmt(t, num0(t) + k * step, d));
    paint();
    num = at(t.s) || at(t.s + 1);
    scrubbable();
  }
  function scrubbable() {
    num = ta.selectionStart === ta.selectionEnd || document.activeElement !== ta ? at(ta.selectionStart) : null;
    strip.hidden = !num || !touch;
    if (num) pad.setAttribute('aria-valuenow', val.textContent = ta.value.slice(num.s, num.e));
  }
  document.addEventListener('selectionchange', () => { if (document.activeElement === ta) { scrubbable(); if (/^\s*use\s/m.test(ta.value)) build(); } });
  ta.addEventListener('keydown', (e) => {
    if (e.altKey && (e.key === 'ArrowUp' || e.key === 'ArrowDown') && at(ta.selectionStart)) {
      e.preventDefault();
      bump((e.key === 'ArrowUp' ? 1 : -1) * (e.shiftKey ? 10 : 1), at(ta.selectionStart));
    }
  });
  // Where a pointer is in the text (monospace, no wrapping, tabs of 2).
  function posAt(e) {
    const r = ta.getBoundingClientRect(), s = getComputedStyle(ta);
    const cw = measure(), lh = parseFloat(s.lineHeight);
    const x = (e.clientX - r.left - parseFloat(s.paddingLeft) - ta.clientLeft + ta.scrollLeft) / cw;
    const y = e.clientY - r.top - parseFloat(s.paddingTop) - ta.clientTop + ta.scrollTop;
    const lines = ta.value.split('\n'), row = Math.floor(y / lh);
    if (row < 0 || row >= lines.length || x < 0) return -1;
    let col = 0, k = 0;
    for (const l = lines[row]; k < l.length; k++) {
      const w = l[k] === '\t' ? 2 - (col % 2) : 1;
      if (x < col + w) break;
      col += w;
    }
    return k < lines[row].length ? lines.slice(0, row).reduce((a, l) => a + l.length + 1, 0) + k : -1;
  }
  let cw = 0, cwFont = '';
  function measure() {
    const f = getComputedStyle(ta).fontSize;
    if (f !== cwFont) {
      const m = h('span', { style: 'position:absolute;visibility:hidden;white-space:pre' }, '0'.repeat(20));
      hl.append(m);
      cw = m.getBoundingClientRect().width / 20;
      m.remove();
      cwFont = f;
    }
    return cw;
  }
  // A drag starts once the pointer has moved 4 px sideways: a click or a
  // selection is the browser's as usual.
  let drag = null;
  function dragFrom(t, e) {
    drag = { t, x0: e.clientX, was: ta.value.slice(t.s, t.e), v0: num0(t), d: decs(t), moved: false, id: e.pointerId, el: e.currentTarget };
  }
  function dragMove(e) {
    if (!drag || e.pointerId !== drag.id) return false;
    const dx = e.clientX - drag.x0;
    if (!drag.moved) {
      if (Math.abs(dx) < 4) return true;
      drag.moved = true;
      drag.el.setPointerCapture(e.pointerId);
    }
    const step = (drag.d ? 10 ** -drag.d : 1) * (e.shiftKey ? 10 : 1);
    const text = fmt(drag.t, drag.v0 + Math.round(dx / 4) * step, drag.d);
    if (text !== ta.value.slice(drag.t.s, drag.t.e)) put(drag.t, text, true);
    if (drag.el === pad) pad.setAttribute('aria-valuenow', val.textContent = text);
    return true;
  }
  ta.addEventListener('pointermove', (e) => {
    if (dragMove(e)) return;
    if (e.pointerType !== 'touch') { const p = posAt(e); ta.style.cursor = p >= 0 && at(p) ? 'ew-resize' : ''; }
  });
  ta.addEventListener('pointerdown', (e) => {
    if (e.pointerType === 'touch' || e.button) return;
    const p = posAt(e), t = p >= 0 && at(p);
    if (t) dragFrom(t, e);
  });
  const end = (e) => {
    if (!drag || e.pointerId !== drag.id) return;
    if (drag.moved) {
      // One undo step for the whole drag: back to where it started, then the result as typed.
      const t = drag.t, now = ta.value.slice(t.s, t.e);
      put(t, drag.was, true);
      ta.focus({ preventScroll: true });
      put(t, now);
    }
    drag = null;
    scrubbable();
  };
  ta.addEventListener('pointerup', end);
  ta.addEventListener('pointercancel', end);
  pad.addEventListener('pointerdown', (e) => { if (num) { e.preventDefault(); dragFrom(num, e); } });
  pad.addEventListener('pointermove', dragMove);
  pad.addEventListener('pointerup', end);
  pad.addEventListener('pointercancel', end);

  function cheat(btn) {
    const r = btn.getBoundingClientRect();
    const ins = (t) => () => { closeMenu(); const s = ta.selectionStart; ta.focus(); ta.setSelectionRange(s, ta.selectionEnd); document.execCommand('insertText', false, t); };
    const rows = (title, list) => [h('h3', title), h('dl.psi-dl', list.map(([k, v, x]) => [h('dt', h('button.txt.mono', { onclick: ins(x || k.split(/[ ,(]/)[0]) }, k)), h('dd', v)]))];
    popup(h('div.menu.wide', { id: 'psi-cheat', role: 'dialog', 'aria-label': 'Cheat sheet' }, h('div.mh', 'Cheat sheet'),
      rows('At the top: what the formula is', DECLS), rows('Values', VARS), rows('Functions', FUNCS), rows('Drawing', DRAW), rows('Colour (the last line, of the formula or a layer)', COLS),
      h('p.cap', 'Phases are in turns: sin(0.25) is 1. Tap a name to put it in. -- starts a comment.')), r.left, r.bottom + 4);
  }

  // The text replaced by `t` (only the part that changed, so the caret
  // and scroll stay), then compiled at once.
  function edit(t) {
    const s = ta.value;
    let a = 0, e = 0;
    while (a < s.length && a < t.length && s[a] === t[a]) a++;
    while (e < s.length - a && e < t.length - a && s[s.length - 1 - e] === t[t.length - 1 - e]) e++;
    ta.setRangeText(t.slice(a, t.length - e), a, s.length - e, 'preserve');
    paint();
    compile();
  }
  // The `name` line set to `v` (taken out when empty).
  function named(v) {
    const t = declText(v, MAX_NAME);
    if (P) edit(P.declare(ta.value, 'name', 0, t ? `name "${t}"` : ''));
  }
  // The settings sheet (js/fsheet.js), over this text: edits go in as typed.
  async function settings(btn) {
    if (!good) { toast('Fix the formula first: it doesn’t work yet', 'bad'); return; }
    const back = () => { if (good) play(pv, live, info(good)); };
    (await import('./fsheet.js')).sheet({
      P, f: B.draft, look: LOOK(), get: () => ta.value, put: edit, keep,
      moved: () => { stash(); pv.dirty = true; sliders(sl, B.draft, () => { stash(); pv.dirty = true; PS?.follow(live); }); PS?.follow(live); },
      preview: (src) => (src ? play(pv, { ...live, src }, info(src)) : back()),
    }, btn);
  }

  // Saved where yours live (js/fstore.js): on the panel, a file it plays.
  function save() {
    if (!ready()) return Promise.reject(new Error('it doesn’t work yet'));
    const f = B.draft;
    f.name = name.value.trim() || `My formula ${mine().length + 1}`;
    name.value = f.name;
    // The name is its `name` line.
    if (!ctlOf(f)?.name) named(f.name);
    const src = ta.value;
    let r;
    const done = said(async () => {
      const rec = await S.put({ id: f.id, file: f.file, name: f.name, src, a: f.a, b: f.b, c: f.c });
      f.id = rec.id;
      if (rec.file) f.file = rec.file;
      f.src = src;
      f.base = sig({ src, name: f.name });
      stash();
      r = { ...live, file: f.file };
    });
    return done.then(() => { if (!r) throw new Error('not saved'); return r; });
  }
  // The Formula as saved (its file, on the panel), for a playlist line or
  // the look: saved first when it changed or never was.
  const keep = () => (B.draft.file && !dirty(B.draft) && B.draft.src === good ? Promise.resolve({ ...live, file: B.draft.file }) : save());

  return {
    el,
    open(f) {
      PS?.drop(live);
      B.draft = f;
      f.base ??= sig(f);
      name.value = f.name || '';
      ta.value = f.src;
      bad = null;
      good = null;
      W?.reset();
      paint();
      pv ??= view(canvas);
      compile();
      scrubbable();
      if (matchMedia('(pointer:fine)').matches) ta.focus({ preventScroll: true });
    },
    // After a restart: the module is new, compile the text again.
    again() { if (B.editing) compile(); },
    // The site's page: the text as typed (`paste`: every `use` line pasted
    // first, null when one can't be), whether it works, its info() and controls().
    now(paste) { if (paste && !ready()) return null; const src = ta.value, ok = good === src; return { name: B.draft.name, src, a: B.draft.a, b: B.draft.b, c: B.draft.c, ok, info: ok ? info(src) : [], ctl: ok ? ctlOf({ src }) : null }; },
    // Its settings sheet (Settings › Look's Change…).
    settings: () => settings(setBtn),
  };
}

// The site's Formula page (/f/): the editor alone, on the demo's files,
// with `f` in it. `o`: {acts (its buttons), note (the line under them),
// changed() (on every edit)}. Returns {P, now(paste)} (pkg-psi; the text now).
export async function editorPage(el, f, o) {
  PAGE = o;
  await psi();
  EX = await fetch(new URL('../demo/formulas.json', location.href)).then((r) => r.json()).then((x) => (Array.isArray(x) ? x.filter((e) => typeof e?.src == 'string') : []), () => []);
  B = { root: el, editing: true, tiles() {}, draft: null };
  B.ed = editor();
  el.append(B.ed.el);
  el.classList.add('psied');
  B.ed.open(B.draft = opened(f));
  return { P, now: (paste) => B.ed.now(paste) };
}

// Into the editor from the gallery (the way back is the browser's); a
// draft not saved yet is only replaced when the user says so.
function go(f, at, sheet) {
  const d = unsaved(), to = (x) => () => { B.draft = x; B.pushed = true; B.sheet = sheet && x === f; app.go('formulas/edit'); };
  f.base = sig(f);
  if (d) ask(`Replace “${d.name || 'Untitled formula'}”? It isn’t saved yet.`, ['Replace it', to(f)], ['Keep editing it', to(d)], at);
  else to(f)();
}
function edit() {
  if (!P) return; // the module didn't load: the gallery says so; a link waits for a reload
  B.draft ??= stored() || { name: '', src: STARTER };
  let later;
  // From Settings › Look's Change…: that Formula (yours as saved), its settings open.
  const pe = app.psiEdit, m = pe?.f.mine && mine().find((x) => x.id === pe.f.id);
  app.psiEdit = null;
  if (pe) B.sheet = 1;
  const ext = app.psiOpen;
  app.psiOpen = null;
  if (ext || pe) {
    const f = m ? { ...m } : opened(ext || pe.f), d = unsaved();
    f.base ??= sig(f);
    // Yours as saved (the site's, saved as it arrived): Save keeps to its file.
    if (ext?.id != null) { f.id = ext.id; if (ext.file) f.file = ext.file; }
    // Not saved (it doesn't work yet): a draft not saved yet, kept over a reload.
    if (ext?.unsaved) f.base = '';
    B.pushed = !!pe;
    if (d && sig(d) !== sig(f)) later = () => ask(`Open “${f.name}”? “${d.name || 'Untitled formula'}” isn’t saved yet and will be replaced.`, ['Open it', () => { B.ed.open(f); if (pe) B.ed.settings(); }], ['Keep mine', () => {}]);
    else B.draft = f;
    if (later) B.sheet = 0;
    else if (ext?.unsaved) store.set('psi.draft', f);
  }
  B.editing = true;
  B.root.classList.add('psied');
  B.ed.open(B.draft);
  scrollTo(0, 0);
  later?.();
  if (B.sheet) { B.sheet = 0; setTimeout(() => B.ed.settings(), 0); }
}
function close(nav) {
  B.editing = false;
  B.root.classList.remove('psied');
  closeMenu();
  B.tiles();
  if (nav) { if (B.pushed) history.back(); else location.replace('#formulas'); }
  B.pushed = false;
}
