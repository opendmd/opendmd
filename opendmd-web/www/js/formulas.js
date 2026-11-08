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
// A scratch formula's top row, RGB (js/fsprite.js's palette strip: the panel's own colours), then what was held compiled again.
const row = (src) => T(() => { const was = held; held = null; const r = P.compile(src) ? (P.frame(0, 10, 9, 30, Q(), Q(), Q()), new Uint8Array(P.mem.buffer, P.pixels(), 384).slice()) : null; take(was); return r; }, null);
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
// else (local mode, no card) its inline path (js/psishow.js `fpath`),
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
// The errors whose sentence carries a fix (by code, js/psierr.js): which `…` in it is the fix, for Fix it.
// (E218's is the text the `…` after it quotes; E201/E202 only as "did you
// mean": a habit from another language, `lerp`, names a whole call.)
const FIX = { 5: 1, 201: 1, 202: 1, 205: 1, 206: 1, 207: 1, 218: 0 };
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
    // (A formula that never repeats, loop 0: its time since the preview began.
    // `hold`: paused there, ms; the editor's time row.)
    const g = v.d || D, n = v.loopMs === 0 ? Infinity : Math.max(1, Math.floor((v.loopMs || 8000) / g)), e = v.hold ?? now - v.t0, k = Math.floor(e / g), i = k % n;
    // Over a GIF the picture changes with the GIF too.
    const key = v.over && U ? k : i;
    if (key === v.i && !v.dirty && !v.f.w?.on) continue;
    v.i = key; v.dirty = false;
    const rgb = draw(v.f, i * g, false, e);
    if (rgb) { v.pv.p.showRgb(rgb); v.canvas.dataset.frames = -~v.canvas.dataset.frames; v.tick?.(); }
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
// Earlier versions (js/fhist.js), by a key per Formula: yours by its id, one
// from the gallery by its name, a new one its own; the site's page's its own.
const hist = (fn) => import('./fhist.js').then(fn, () => {});
const hkey = (f) => (PAGE ? PAGE.key : f.id != null ? 'm' + f.id : (f.hk ??= 'n' + Date.now().toString(36)));

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
const STARTER = `-- x: 0-1 across, y: 0-1 down
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
// it in app.remix): into the editor, to change it, show it, add it to a
// playlist. A gallery one by name (a bundled one is here already; else
// fetched from the site) opens as a draft not saved yet: a link anyone can
// craft writes nothing to the card, Save does. An edited one sent by the
// site's page that opened this one (postMessage: its origin opendmd.dev, or
// this page's own for the demo and the tests), never in the link, is saved
// under Your formulas as it arrives (js/fstore.js: on the panel, else this
// browser): its person pressed Open on my panel there. One that doesn't
// work isn't saved: said, and kept as a draft not saved yet.
async function remixed() {
  const r = app.remix;
  if (!r) return;
  app.remix = null;
  location.replace('#formulas');
  const ex = r.gallery && EX.find((x) => x.stem === r.gallery);
  let f;
  try { f = ex ? { name: ex.name, src: ex.src } : r.gallery ? await fetchFormula(ROOT(), r.gallery) : await sent(); } catch (e) { toast(`Couldn’t open that Formula: ${e.message || e}`, 'bad'); return; }
  f = { ...f, ...r.f };
  if (r.gallery) { f.hk = 'g' + r.gallery; hist((m) => m.keep(f.hk, f, 'g')); }
  if (r.gallery && !ex) {
    await psi();
    f.name = ctlOf(f)?.name || f.name;
    f.unsaved = true;
    toast(`“${f.name || 'This formula'}” is open. Save keeps it under Your formulas`, 'ok');
  } else if (!ex) {
    try { f = await arrived(f); hist((m) => m.keep(hkey(f), f, 'r')); } catch (e) { toast(`“${f.name || 'This formula'}” isn’t saved: ${e.message || e}`, 'bad'); f.unsaved = true; }
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

function editor() {
  // The name is the formula's `name` line (typed here, written there).
  const name = h('input', { type: 'text', id: 'psi-name', placeholder: 'Untitled formula', maxLength: 60, 'aria-label': 'Name', oninput: () => { B.draft.name = name.value; stash(); }, onchange: () => setTimeout(nameIn) });
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
  const help = h('button.sm', { id: 'psi-help', 'aria-haspopup': 'dialog', onclick: (e) => { const b = e.currentTarget; hint().then((m) => m.cheat(b, ta)); } }, 'Cheat sheet');
  const vers = h('button.sm', { id: 'psi-vers-btn', 'aria-haspopup': 'dialog', onclick: (e) => { const b = e.currentTarget; hist((m) => m.menu(b, { key: hkey(B.draft), cur: ta.value, back, site: !!PAGE })); } }, 'Earlier versions…');
  const canvas = h('canvas.panel', { id: 'psi-ed-panel', tabIndex: 0, 'aria-label': 'Preview (, and . step a frame)' });
  // The preview's time: paused (pv.hold, ms), a frame back or on, or dragged
  // through: the loop, or the first 10 s of one that goes on.
  const tr = h('input', { type: 'range', id: 'psi-t', min: 0, max: 0, value: 0, 'aria-label': 'Time', oninput: () => seek(+tr.value * grain()) });
  const tnow = h('span.mono', { id: 'psi-t-now' });
  const pz = h('button.sm', { id: 'psi-pause', onclick: () => seek(pv.hold == null ? elapsed() : null) });
  const stp = (k) => h('button.sm', { id: k < 0 ? 'psi-t-back' : 'psi-t-on', 'aria-label': k < 0 ? 'A frame back' : 'A frame on', title: k < 0 ? 'A frame back (,)' : 'A frame on (.)', onclick: () => step(k) }, k < 0 ? '‹' : '›');
  const trow = h('div.psi-time', pz, stp(-1), tr, stp(1), tnow);
  const cap = h('div.cap', { id: 'psi-ed-cap' }), heavy = h('p.cap', { id: 'psi-heavy', hidden: true },
    'Heavy for the panel: noise, len, sqrt, atan2 and dividing by a changing value cost the most');
  const sl = h('div.psi-sls', { id: 'psi-ed-sl' });
  // A formula that reads a built-in input: its controls (js/fconds.js).
  const cond = h('div', { hidden: true });
  let W = null;
  const under = overRow('psi-ed-under');
  const look = LOOK() && h('button', { id: 'psi-ed-look', hidden: true, onclick: () => { if (ready()) setLook({ ...live }, () => keep()); } }, 'Set as the panel’s look');
  const touch = matchMedia('(pointer:coarse)').matches;
  // Colours (#rgb, #rrggbb): with the caret in one, a swatch at its line's
  // end opens the browser's colour picker; the colour picked goes in its
  // place, live while picking and one undo for the lot. `pend`, from the
  // picker opening to its change (or a click or key back in the text, or
  // focus elsewhere): the colour's range, the text it was and is now.
  // (Safari never focuses the picker: `pend` keeps the swatch up.)
  let ks = [], kt = null, pend = null;
  const hex = () => { const v = kp.value, w = pend.was, x = w.length == 4 && /^#(.)\1(.)\2(.)\3$/.test(v) ? '#' + v[1] + v[3] + v[5] : v; return /[A-F]/.test(w) ? x.toUpperCase() : x; };
  const open = () => { if (!pend && kt) { const was = ta.value.slice(kt.s, kt.e); pend = { s: kt.s, e: kt.e, was, now: was }; } };
  // The pick in, unless the text under it changed meanwhile (a `use` line pasted): then dropped.
  // (Tried with peek(): the text is left as it was until the pick lands.)
  const picked = () => { if (!pend) return; if (ta.value.slice(pend.s, pend.e) !== pend.was) { pend = null; peek(); return; } pend.now = hex(); peek(ta.value.slice(0, pend.s) + pend.now + ta.value.slice(pend.e)); };
  function done() {
    const p = pend;
    pend = null;
    if (p && p.now !== p.was && ta.value.slice(p.s, p.e) === p.was) { if (!touch) ta.focus({ preventScroll: true }); put(p, p.now); } else if (p) peek();
    swatch();
  }
  const away = () => setTimeout(() => { const f = document.activeElement; if (pend && f && f !== document.body && f !== kp && f !== ta) done(); else swatch(); });
  const kp = h('input', { type: 'color', id: 'psi-kp', hidden: true, title: 'Pick a colour', 'aria-label': 'Pick a colour',
    onpointerdown: open, onclick: open, oninput: picked, onchange: () => { picked(); done(); }, onblur: away });
  // One card: the preview (stuck to the top on a phone), the sliders, the
  // error over the code, the code, the actions.
  const top = h('div.psi-top', canvas, trow, cap, under);
  top.addEventListener('keydown', (e) => { if ((e.key === ',' || e.key === '.') && e.target !== tr && !e.ctrlKey && !e.metaKey && !e.altKey) { e.preventDefault(); step(e.key === '.' ? 1 : -1); } });
  const el = h('div', { id: 'psi-ed' }, head, h('div.card.psi-edb', top, h('div.psi-sl2', sl, heavy), cond, err,
    h('div.psi-code', h('div.psi-src', gut, h('div.psi-wrap', hl, ta, kp)), strip,
      h('div.row.acts', help, vers, h('span.cap', touch ? 'Tap a number, then drag the bar under the code to try other values; tap a colour to pick another' : 'Drag a number (or Alt + ↑ ↓) to try other values; click a colour to pick another · Tab indents: Esc, then Tab, leaves the code (or goes to the colour)'))),
    h('div.psi-acts', PAGE ? [h('div.row.acts', PAGE.acts), h('p.cap', PAGE.note)] : [h('div.row.acts', showBtn('psi-ed-show', () => live), plBtn('psi-ed-pl', () => ({ ...live }), () => keep()), look, h('button', { id: 'psi-ed-gif', onclick: () => { if (ready()) toGif(B.draft); } }, 'Save as GIF…'),
      h('button', { id: 'psi-ed-code', onclick: () => copyCode({ src: ta.value }) }, 'Copy code'),
      h('button', { id: 'psi-ed-submit', onclick: (e) => { if (ready()) submit(e.currentTarget, B.draft); } }, 'Submit to the gallery…')), h('p.cap', NOTE())])));
  let pv = null, toks = [], bad = null, num = null, good = null;
  // The site's What costs most and Tidy up (js/fcost.js, never on the panel): the last compile, told.
  let fc = null, fcs = [null];
  if (PAGE || app.demo) import('./fcost.js').then((m) => { fc = m.cost({ ta, gut, code: el.querySelector('.psi-code'), help, put }); fc.seen(...fcs); }, () => {});
  // What the preview plays: the last text that compiled, with the sliders as they are.
  const live = { get name() { return B.draft.name; }, get src() { return good; }, get a() { return B.draft.a; }, get b() { return B.draft.b; }, get c() { return B.draft.c; }, get w() { return cond.hidden ? null : W?.w; } };

  // The draft kept in this browser (the web app's; the site's page tells its own of each change).
  const stash = PAGE ? () => PAGE.changed?.() : debounce(() => store.set('psi.draft', B.draft), 300);
  const grain = () => pv.d || D, len = () => (pv.loopMs === 0 ? FOREVER : pv.loopMs || 8000), elapsed = () => pv.hold ?? performance.now() - pv.t0;
  // Held at `ms`, or (null) playing on from where it was held.
  function seek(ms) {
    if (ms == null) { pv.t0 = performance.now() - pv.hold; pv.hold = null; } else pv.hold = Math.max(0, ms);
    pv.dirty = true;
    kick();
    tick();
  }
  function step(k) {
    const g = grain(), n = Math.max(1, Math.floor(len() / g));
    let i = Math.floor(elapsed() / g) + k;
    if (pv.loopMs !== 0) i = (i % n + n) % n;
    seek(Math.max(0, i) * g);
  }
  let shown = '';
  function tick() {
    const g = grain(), n = Math.max(1, Math.floor(len() / g)), k = Math.floor(elapsed() / g), i = pv.loopMs === 0 ? k : k % n, held = pv.hold != null;
    // (Only what changed: it runs every frame.)
    if (shown === (shown = `${n} ${i} ${g} ${held}`)) return;
    tr.max = n - 1;
    tr.value = Math.min(i, n - 1);
    tnow.textContent = `t = ${(i * g / 1000).toFixed(2)} s`;
    pz.textContent = held ? '▶' : '❚❚';
    pz.setAttribute('aria-label', held ? 'Play' : 'Pause');
    top.classList.toggle('held', held);
  }
  // Back to an earlier version (js/fhist.js; the site's page's draft): only
  // the lines that differ, as typed, so undo takes it away again (true when it can).
  function back(v) {
    for (const k of 'abc') if (typeof v[k] == 'number') B.draft[k] = v[k];
    const s = ta.value, [a, e] = span(s, v.src);
    ta.focus({ preventScroll: true });
    const typed = document.activeElement === ta;
    if (a < s.length - e || a < v.src.length - e) put({ s: a, e: s.length - e }, v.src.slice(a, v.src.length - e));
    if (v.name && !ctlOf({ src: v.src })?.name) { B.draft.name = name.value = v.name; }
    paint();
    compile();
    return typed;
  }
  // Numbers (a unary minus included), comments and the error, drawn under the (transparent) text.
  function paint() {
    const s = ta.value, m = marks(s, bad);
    toks = m.toks;
    ks = m.cols;
    hl.innerHTML = m.html + '\n';
    const n = s.split('\n').length, el = bad && s.slice(0, bad[0]).split('\n').length;
    gut.innerHTML = Array.from({ length: n }, (_, k) => (k + 1 === el ? `<b>${k + 1}</b>` : k + 1)).join('\n');
    fc?.paint(n);
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
      fcs = [f.src, est(i)];
      fc?.seen(...fcs);
    } else {
      fcs = [null];
      fc?.seen(null);
      bad = miss ? [miss.s, miss.e] : T(() => P.errorAt(), [0, 0]);
      top.classList.toggle('stale', good != null);
      const line = ta.value.slice(0, bad[0]).split('\n').length, at = bad, was = ta.value;
      el.dataset.err = line;
      const msg = miss ? miss.why : T(() => P.error(), '');
      // "Line N" takes the caret there; Fix it (only on the text it was for).
      clear(err, h('button.txt', { id: 'psi-err-line', onclick: () => goLine(at[0]) }, `Line ${line}`), ': ', msg.split('`').map((t, k) => (k % 2 ? h('code', t) : t)));
      // Its code, linked to the language reference's entry (the editor's
      // own messages, `miss`, have none); one with a suggestion, Fix it.
      const shown = err.textContent;
      if (!miss) errRef(msg).then((r) => {
        if (!r || err.textContent !== shown) return;
        const c = +r.code.slice(1), q = msg.split('`'), x = (c != 201 && c != 202 || /did you mean/.test(msg)) && q[2 * FIX[c] + 1], o = c == 218 && q[3], k = o ? ta.value.lastIndexOf(o, at[1]) : 0, to = o ? [k, k + o.length] : at;
        if (x && k >= 0) err.append(' ', h('button.sm', { id: 'psi-fix', title: `Write ${x.replace('…', '')}`, onclick: () => { if (ta.value === was) fix(to, x, c); } }, 'Fix it'));
        err.append(' ', h('a.ecode', { href: r.href, target: '_blank', rel: 'noopener', title: 'What this error means' }, r.code));
      });
      paint();
    }
    stash();
    return miss;
  }
  // Before saving or sharing: every use line pasted, or nothing happens.
  const ready = () => { const m = compile(true); if (m) toast(`First fix line ${ta.value.slice(0, m.s).split('\n').length}: ${m.why}`, 'bad'); return !m; };
  ta.addEventListener('input', () => { paint(); scrubbable(); build(); HN?.update(true); });
  ta.addEventListener('blur', () => HN?.close());
  // The names at the caret and the cheat sheet (js/fhint.js), from the first key.
  let HN = null, hm = null;
  const hint = () => (hm ??= import('./fhint.js').then((m) => { HN = m.hint({ ta, put, measure, after: strip, touch }); HN.update(true); return m; }));
  // A suggestion put in (`…` where the caret goes, a placeholder chosen).
  function fix(at, x, c) {
    const k = x.indexOf('…'), t = x.replace('…', ''), q = k >= 0 ? [k, k] : c == 206 ? [t.length - 2, t.length - 1] : c == 207 ? [6, 7] : [t.length, t.length];
    ta.focus({ preventScroll: true });
    put({ s: at[0], e: at[1] }, t);
    ta.setSelectionRange(at[0] + q[0], at[0] + q[1]);
  }
  function goLine(p) {
    ta.focus({ preventScroll: true });
    ta.setSelectionRange(p, p);
    const r = ta.getBoundingClientRect(), y = r.top + parseFloat(getComputedStyle(ta).lineHeight) * (ta.value.slice(0, p).split('\n').length - 1);
    scrollBy({ top: y - innerHeight / 2, behavior: 'smooth' });
  }
  ta.addEventListener('scroll', () => { hl.scrollLeft = ta.scrollLeft; swatch(); });

  // Draggable numbers: under the pointer (a mouse or pen, straight in the
  // text), Alt+↑↓ at the caret, and on touch the strip under the code.
  // Edits go through insertText so the browser's undo has them.
  const at = (pos) => toks.find((t) => t.s <= pos && pos <= t.e);
  // Always as typed (the browser's undo keeps it: setRangeText would wipe
  // its history). Not focused (a phone's strip, a picker): focused for it
  // with no keyboard (inputmode none), then back where focus was.
  // (The focus moved for it blurs a field, whose change may ask for this
  // same edit again meanwhile: `putting`, that one is dropped.)
  let putting = false;
  function put(t, text) {
    if (putting) return;
    putting = true;
    const f = document.activeElement, away = f !== ta;
    try {
      if (away) { ta.inputMode = 'none'; ta.focus({ preventScroll: true }); }
      ta.setSelectionRange(t.s, t.e);
      document.execCommand('insertText', false, text);
      t.e = t.s + text.length;
      ta.setSelectionRange(t.e, t.e);
      if (away) { if (f && f !== document.body) f.focus({ preventScroll: true }); else ta.blur(); ta.inputMode = ''; }
    } finally { putting = false; }
  }
  // A text tried without touching the textarea (a number dragged, a colour
  // or a stroke picked: each lands once, with put()): drawn in the code and
  // played (Show on panel follows when it lands). peek() goes back to the
  // text as it is: the last version that worked, else nothing playing.
  function peek(src) {
    if (src == null) { paint(); if (good) play(pv, live, info(good)); else if (pv) pv.f = null; return; }
    hl.innerHTML = marks(src).html + '\n';
    if (T(() => P.compile(src), false)) { hold(src); play(pv, { ...live, src }, info(src)); }
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
    swatch();
  }
  // The colour swatch: beside the colour the caret is in, or hidden; left
  // as it is while the picker is open.
  function swatch() {
    blocks();
    const f = document.activeElement, p = ta.selectionStart;
    if (pend) return;
    if (f !== ta) { if (f !== kp) kp.hidden = true; return; }
    kt = p === ta.selectionEnd ? ks.find((t) => t.s <= p && p <= t.e) : null;
    if ((kp.hidden = !kt)) return;
    const v = ta.value.slice(kt.s, kt.e), row = ta.value.slice(0, kt.s).split('\n').length - 1, s = getComputedStyle(ta), lh = parseFloat(s.lineHeight);
    let col = 0;
    for (const c of ta.value.split('\n')[row]) col += c === '\t' ? 2 - (col % 2) : 1;
    kp.value = v.length == 4 ? '#' + [...v.slice(1)].map((c) => c + c).join('') : v;
    // Past the line's end (held inside the box), so it covers no text there is room for.
    kp.style.left = `${Math.max(0, Math.min(ta.clientLeft + parseFloat(s.paddingLeft) + (col + 1) * measure() - ta.scrollLeft, ta.clientWidth - kp.offsetWidth))}px`;
    kp.style.top = `${ta.clientTop + parseFloat(s.paddingTop) + (row + 0.5) * lh - kp.offsetHeight / 2}px`;
  }
  // Sprites' pictures and palettes (js/fsprite.js, imported for a formula with one): a button at the one the caret is in.
  let SP = null;
  function blocks() {
    if (SP) return SP.caret?.();
    if (/^[ \t]*(sprite[ \t]+\w+[ \t]*\{|palette[ \t]+\w+[ \t]*=)/m.test(ta.value)) SP = import('./fsprite.js').then((m) => { SP = m.blocks({ ta, put, peek, measure, row }); SP.caret(); }, () => { SP = null; });
  }
  ta.addEventListener('blur', away);
  // Back in the text: the pick so far goes in as one undo step.
  ta.addEventListener('pointerdown', () => { if (pend) done(); });
  ta.addEventListener('keydown', () => { if (pend) done(); });
  ta.addEventListener('focus', () => { if (pend) done(); });
  document.addEventListener('selectionchange', () => { if (document.activeElement === ta) { scrubbable(); HN?.update(); if (/^\s*use\s/m.test(ta.value)) build(); } });
  ta.addEventListener('keydown', (e) => {
    hint();
    // (Esc arms leaving by Tab, the list's Esc too.)
    const was = esc;
    esc = e.key === 'Escape';
    if (HN?.key(e)) return;
    if (e.altKey && (e.key === 'ArrowUp' || e.key === 'ArrowDown') && at(ta.selectionStart)) {
      e.preventDefault();
      bump((e.key === 'ArrowUp' ? 1 : -1) * (e.shiftKey ? 10 : 1), at(ta.selectionStart));
    } else keys(e, was);
  });
  // Typing code: Enter keeps the line's indent (after `{`, a line further in
  // and the `}` under it), `}` on an indent steps back, Tab and Shift+Tab
  // put two spaces in or take them out (Esc, then Tab, leaves the field);
  // Ctrl/Cmd+S saves, Ctrl/Cmd+Enter shows it on the panel. As typed
  // (insertText), so undo has them.
  let esc = false;
  function keys(e, was) {
    const s = ta.value, a = ta.selectionStart, z = ta.selectionEnd, l0 = s.lastIndexOf('\n', a - 1) + 1, ind = /^[ \t]*/.exec(s.slice(l0, a))[0], k = e.key, mod = e.ctrlKey || e.metaKey;
    const ins = (t, from = a, to = z) => { e.preventDefault(); ta.setSelectionRange(from, to); document.execCommand('insertText', false, t); };
    // (The site's page saves nothing: Ctrl+S there is the browser's.)
    if (mod && k.toLowerCase() === 's' && !PAGE) { e.preventDefault(); save().catch(() => {}); } else if (mod && k === 'Enter') { const b = el.querySelector('#psi-ed-show'); if (b) { e.preventDefault(); b.click(); } } else if (mod || e.altKey || e.isComposing) return;
    else if (k === 'Enter' && !e.shiftKey) {
      const end = s.indexOf('\n', z), rest = s.slice(z, end < 0 ? s.length : end), shut = /^\s*\}/.test(rest);
      if (!/\{[ \t]*$/.test(s.slice(l0, a))) return ins('\n' + ind);
      ins(`\n${ind}  ` + (shut ? '\n' + ind : (s.match(/\{/g) || []).length > (s.match(/\}/g) || []).length ? `\n${ind}}` : ''));
      ta.setSelectionRange(a + ind.length + 3, a + ind.length + 3);
    } else if (k === '}' && a === z && a > l0 && ind.length === a - l0) {
      // (On the line over a `}` already there, as Enter after `{` leaves: onto it.)
      const n = s.slice(a).match(/^\n([ \t]*)\}/);
      if (n && l0 && n[1] === ind.slice(0, -2) && (s.match(/\{/g) || []).length <= (s.match(/\}/g) || []).length) ins(`\n${n[1]}}`, l0 - 1, a + n[0].length);
      else ins(ind.slice(0, -2) + '}', l0);
    }
    else if (k === 'Tab' && !was) {
      if (a === z && !e.shiftKey) return ins('  ');
      const end = s.indexOf('\n', z > a && s[z - 1] === '\n' ? z - 1 : z), e1 = end < 0 ? s.length : end, b = s.slice(l0, e1), t = e.shiftKey ? b.replace(/^( {1,2}|\t)/gm, '') : b.replace(/^/gm, '  ');
      e.preventDefault();
      if (t === b) return;
      ins(t, l0, e1);
      if (a === z) { const q = Math.max(l0, a - b.length + t.length); ta.setSelectionRange(q, q); } else ta.setSelectionRange(l0, l0 + t.length);
    }
  }
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
    drag.now = drag.was;
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
    if (text !== drag.now) { drag.now = text; peek(ta.value.slice(0, drag.t.s) + text + ta.value.slice(drag.t.e)); }
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
    // One undo step for the whole drag: what it came to, typed in once.
    if (drag.moved) { if (drag.now !== drag.was) { if (drag.el !== pad) ta.focus({ preventScroll: true }); put(drag.t, drag.now); } else peek(); }
    drag = null;
    scrubbable();
  };
  ta.addEventListener('pointerup', end);
  // (A number in the selection, as an undo leaves it: dragged, it is the number's drag, not the text's.)
  ta.addEventListener('dragstart', (e) => { if (drag) e.preventDefault(); });
  ta.addEventListener('pointercancel', end);
  // (Or the capture lost, or the window left mid-drag: it ends where it got to.)
  ta.addEventListener('lostpointercapture', end);
  pad.addEventListener('lostpointercapture', end);
  addEventListener('blur', () => { if (drag) end({ pointerId: drag.id }); });
  pad.addEventListener('pointerdown', (e) => { if (num) { e.preventDefault(); dragFrom(num, e); } });
  pad.addEventListener('pointermove', dragMove);
  pad.addEventListener('pointerup', end);
  pad.addEventListener('pointercancel', end);

  // The text replaced by `t`: only the part that changed, typed in (put():
  // one undo step, the history kept), the caret where it was (moved along
  // when the change is before it); then compiled at once.
  function edit(t) {
    if (putting) return;
    const s = ta.value, [a, e] = span(s, t), b = s.length - e, d = t.length - s.length, c = [ta.selectionStart, ta.selectionEnd];
    if (a >= b && a >= t.length - e) { paint(); compile(); return; }
    put({ s: a, e: b }, t.slice(a, t.length - e));
    const at = (k) => (k <= a ? k : k >= b ? k + d : t.length - e);
    ta.setSelectionRange(at(c[0]), at(c[1]));
    paint();
    compile();
  }
  // How much of `s` and `t` is the same at the start and at the end.
  function span(s, t) {
    let a = 0, e = 0;
    while (a < s.length && a < t.length && s[a] === t[a]) a++;
    while (e < s.length - a && e < t.length - a && s[s.length - 1 - e] === t[t.length - 1 - e]) e++;
    return [a, e];
  }
  // The name field's text as the `name` line, unless it is that already.
  const nameIn = () => { if (P && (ctlOf({ src: ta.value })?.name || '') !== declText(name.value, MAX_NAME)) named(name.value); };
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
    // The name is its `name` line (written on the field's change: here too, in case none came).
    nameIn();
    const src = ta.value, k0 = hkey(f);
    let r;
    const done = said(async () => {
      const rec = await S.put({ id: f.id, file: f.file, name: f.name, src, a: f.a, b: f.b, c: f.c });
      f.id = rec.id;
      hist((m) => { m.move(k0, hkey(f)); m.keep(hkey(f), { ...f, src }, 's'); });
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
      if (!pv) { pv = view(canvas); pv.tick = tick; }
      pv.hold = null;
      compile();
      tick();
      scrubbable();
      if (matchMedia('(pointer:fine)').matches) ta.focus({ preventScroll: true });
    },
    // After a restart: the module is new, compile the text again.
    again() { if (B.editing) compile(); },
    // The site's page: the text as typed (`paste`: every `use` line pasted
    // first, null when one can't be), whether it works, its info() and controls().
    now(paste) { if (paste && !ready()) return null; const src = ta.value, ok = good === src; return { name: B.draft.name, src, a: B.draft.a, b: B.draft.b, c: B.draft.c, ok, info: ok ? info(src) : [], ctl: ok ? ctlOf({ src }) : null }; },
    // The name field's change, if it hasn't landed (leaving the editor).
    flush: () => nameIn(),
    // Its settings sheet (Settings › Look's Change…).
    settings: () => settings(setBtn),
    back,
  };
}

// The site's Formula page (/f/): the editor alone, on the demo's files,
// with `f` in it. `o`: {acts (its buttons), note (the line under them),
// key (its Earlier versions'), changed() (on every edit)}. Returns {P,
// now(paste), back(v)} (pkg-psi; the text now; a draft or version put back).
export async function editorPage(el, f, o) {
  PAGE = o;
  await psi();
  EX = await fetch(new URL('../demo/formulas.json', location.href)).then((r) => r.json()).then((x) => (Array.isArray(x) ? x.filter((e) => typeof e?.src == 'string') : []), () => []);
  B = { root: el, editing: true, tiles() {}, draft: null };
  B.ed = editor();
  el.append(B.ed.el);
  el.classList.add('psied');
  B.ed.open(B.draft = opened(f));
  return { P, now: (paste) => B.ed.now(paste), back: B.ed.back };
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
    if (ext?.hk) f.hk = ext.hk;
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
  B.ed.flush();
  B.editing = false;
  B.root.classList.remove('psied');
  closeMenu();
  B.tiles();
  if (nav) { if (B.pushed) history.back(); else location.replace('#formulas'); }
  B.pushed = false;
}
