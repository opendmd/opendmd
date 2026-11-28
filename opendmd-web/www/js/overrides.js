// Per-GIF overrides per playlist (play more or less often, play longer,
// loop N times): the row glyphs and the menu. A favourite is a tag
// (js/tags.js, the heart), which writes its weight here as well; the
// menu has no Favourite of its own. On the card, a `.meta` file
// beside the playlist (wasm parseOverrides/renderOverrides; GET
// /api/overrides?set=); the page never says so.

import { app, playlistLabel, setValue } from './state.js';
import { h, on, emit, toast, label } from './util.js';

const FAV = 3, LESS = 0.5;

// Favourite's weight on/off (js/tags.js setFav; Now's heart).
export const fav = (o = {}) => ({ ...o, w: o.w > 1 ? null : FAV });

export const fileFor = (p) => (p.endsWith('.txt') ? p.slice(0, -4) : p) + '.meta';

// Every Overrides of this page (weakly): one that saves tells the others
// with the same playlist, which read it again (`overrides`).
const live = new Set();
on('overrides', ({ path, from }) => {
  for (const r of live) {
    const x = r.deref();
    if (!x) live.delete(r);
    else if (x !== from && x.path === path && !x.mine(path).length && !x.saving) x.load(path);
  }
});

// One change `o` (or a function of the current entry) to `p` in `map`.
function put(map, p, o) {
  if (o.call) o = o(map.get(p) || {});
  const keep = Object.fromEntries(Object.entries(o).filter(([k, v]) => k !== 'path' && v != null && v !== false));
  if (Object.keys(keep).length) map.set(p, { path: p, ...keep });
  else map.delete(p);
}

// A playlist's overrides as the card has them now; throws if they can't be read.
async function read(path) {
  if (!path) return [];
  if (app.device) return (await app.src.overrides(setValue(path))).overrides;
  const text = await app.src.readText(fileFor(path));
  return text ? [...app.wasm.parseOverrides(text)] : [];
}

export class Overrides {
  constructor() {
    this.path = '';      // the playlist's card path
    this.map = new Map(); // gif path -> {path, w?, dwell?, dwell_x?, loops?}
    this.ops = [];       // this page's changes not saved yet: [playlist path, gif path, o]
    this.saving = null;
    this.loading = null;
    this.ok = true;
    this.again = false;
    this.loud = new Set(); // the playlists whose next save says "Saved"
    this.onChange = () => {};
    live.add(new WeakRef(this));
  }

  get name() { return playlistLabel(this.path); }
  // This playlist's changes not saved yet.
  mine(path = this.path) { return this.ops.filter((x) => x[0] === path); }

  // The old map stays until the new list arrives; a change waits for it,
  // and is refused if the list could not be read (it would wipe the rest).
  async load(path) {
    this.path = path;
    const run = this.loading = read(path).catch((e) => { toast(`${this.name}: ${e.message}`, 'bad'); });
    const list = await run;
    if (this.loading === run && this.path === path) {
      this.ok = !!list;
      this.map = new Map(list?.map((o) => [o.path, o]));
      for (const [, p, o] of this.mine(path)) put(this.map, p, o);
      this.onChange();
    }
    return this;
  }

  get(p) { return this.map.get(p); }

  // Replace one GIF's override (an empty one removes it; `o` may be a
  // function of the current one) and save. `quiet`: no "Saved" toast (a
  // heart's weight: the heart is the feedback).
  set(p, o, quiet) { return this.setMany([[p, o]], quiet); }
  async setMany(changes, quiet) {
    await this.loading;
    if (!this.ok) { toast(`Couldn’t read “${this.name}”: try again`, 'bad'); return this.load(this.path); }
    for (const [p, o] of changes) { this.ops.push([this.path, p, o]); put(this.map, p, o); }
    if (!quiet) this.loud.add(this.path);
    this.onChange();
    return this.save();
  }

  // One write at a time, a playlist at a time; changes made meanwhile go
  // out in the next. The file is read again first, and only this page's
  // changes to that playlist go over it (another view of this page,
  // another browser, may have written it since: a copy read earlier would
  // undo theirs), as js/tags.js does. A save that fails keeps its changes
  // (shown, and in the next save) and says so, with Retry.
  async save() {
    if (this.saving) { this.again = true; return this.saving; }
    const path = this.ops[0]?.[0];
    if (path == null) return;
    const ops = this.mine(path), loud = this.loud.delete(path);
    let failed = false;
    this.saving = (async () => {
      try {
        const map = new Map((await read(path)).map((o) => [o.path, o]));
        for (const [, p, o] of ops) put(map, p, o);
        const text = app.wasm.renderOverrides([...map.values()]);
        if (text || !app.device) await app.src.writeFile(fileFor(path), text);
        else await app.src.deleteFile(fileFor(path)).catch((e) => { if (e.status !== 404) throw e; });
        this.ops = this.ops.filter((x) => !ops.includes(x));
        if (this.path === path) {
          for (const [, p, o] of this.mine(path)) put(map, p, o);
          this.map = map;
          this.onChange();
        }
        emit('overrides', { path, from: this });
        if (loud) toast(`Saved for “${playlistLabel(path)}”${app.device && app.activePath === path ? ' · the display is catching up (a few seconds)' : ''}`, 'ok');
      } catch (e) {
        failed = true;
        if (loud) this.loud.add(path);
        toast(`Couldn’t save for “${playlistLabel(path)}”: ${e.message}. The change is kept for the next save`, 'bad', ['Retry', () => this.save()]);
      }
    })();
    await this.saving;
    this.saving = null;
    // (Another playlist's changes waiting, or more made meanwhile; not again at once after a failure.)
    if (!failed && (this.again || this.ops.length)) { this.again = false; return this.save(); }
    this.again = false;
  }
}

// (A weight above 1 has no glyph of its own: a favourite is its tag, whose
// ♥ the tags' glyphs show; js/tags.js adopt.)
export function glyphs(o) {
  if (!o) return null;
  const g = [];
  if (o.w > 1) { /* the tag's ♥ */ } else if (o.w != null && o.w < 1) g.push(['▽', 'Plays less often', 'less']);
  if (o.dwell) g.push([`◔${o.dwell}s`, `Plays for ${o.dwell} seconds`, 'long']);
  else if (o.dwell_x) g.push([`◔${o.dwell_x}×`, `Plays ${o.dwell_x}× as long`, 'long']);
  if (o.loops) g.push([`↻${o.loops}`, `Loops ${o.loops} times`, 'loop']);
  return g.length ? h('span.ov', g.map(([t, title, c]) => h('i.' + c, { title }, t))) : null;
}

// The element that had the focus when a menu opened gets it back on close;
// `el.onclose` runs then.
let open = null, opener = null;
export function closeMenu() {
  if (!open) return;
  open.remove();
  open.onclose?.();
  open = null;
  if (opener?.isConnected) opener.focus({ preventScroll: true });
  opener = null;
}
addEventListener('keydown', (e) => { if (e.key === 'Escape') closeMenu(); });
addEventListener('pointerdown', (e) => { if (open && !open.contains(e.target)) closeMenu(); }, true);
// (Not a phone keyboard coming up: only a change of width.)
let wide = innerWidth;
addEventListener('resize', () => { if (wide != (wide = innerWidth)) closeMenu(); });

// The menu for one GIF, at (x, y) — a sheet along the bottom on a phone.
// `extra`: optional [id, text, action, off] items at the end (the editor's
// Move and Remove); with no saved playlist, the only ones.
export function menu(ov, path, x, y, extra) {
  closeMenu();
  if (!ov.path && !extra) { toast('Save the playlist first', 'bad'); return; }
  const o = ov.get(path) || {};
  const put = (patch) => { closeMenu(); ov.set(path, (x) => ({ ...x, ...patch })); };
  const secs = h('input', { type: 'number', min: 1, max: 86400, placeholder: 's', id: 'ov-secs', value: o.dwell || '', style: 'width:72px' });
  const loops = h('input', { type: 'number', min: 1, max: 1000, placeholder: '×', id: 'ov-loops', value: o.loops || '', style: 'width:64px' });
  const item = (id, on, text, act) => h('button.mi' + (on ? '.on' : ''), { id, onclick: act }, h('span.ck', on ? '✓' : ''), text);
  const e = app.wasm.parsePath(path);
  const head = h('div.mh', h('b', label(e)), ov.path && h('span.muted', ` in “${ov.name}”`));
  // Play longer and Loop: their fields in a second step of the same menu.
  const longer = () => open.replaceChildren(head,
    item('ov-2x', o.dwell_x === 2, 'Twice as long', () => put({ dwell_x: o.dwell_x === 2 ? null : 2, dwell: null })),
    h('div.mr', h('span', 'Play for'), secs, h('span.muted', 'seconds'),
      h('button.sm', { id: 'ov-secs-set', onclick: () => { const v = +secs.value; if (v >= 1 && v <= 86400) put({ dwell: Math.round(v), dwell_x: null }); else toast('Seconds: 1 to 86400', 'bad'); } }, 'Set')),
    h('div.mr', h('span', 'Loop'), loops, h('span.muted', 'times'),
      h('button.sm', { id: 'ov-loops-set', onclick: () => { const v = +loops.value; if (v >= 1 && v <= 1000) put({ loops: Math.round(v) }); else toast('Loops: 1 to 1000', 'bad'); } }, 'Set')));
  open = h('div.menu', { id: 'ov-menu', role: 'menu' }, head,
    ov.path && [item('ov-less', o.w != null && o.w < 1, '▽ Play less often', () => put({ w: o.w != null && o.w < 1 ? null : LESS })),
    item('ov-longer', o.dwell || o.dwell_x || o.loops, '◔ Play longer or loop…', longer),
    item('ov-reset', false, 'Reset', () => put({ w: null, dwell: null, dwell_x: null, loops: null }))],
    extra?.map(([id, t, act, off]) => Object.assign(item(id, false, t, () => { closeMenu(); act(); }), { disabled: !!off })));
  const r = open.querySelector('#ov-reset');
  if (r) r.disabled = !ov.get(path);
  popup(open, x, y);
}

// Show a `.menu` element at (x, y), or as a sheet along the bottom on a
// phone; one at a time, closed by Escape or a tap elsewhere.
export function popup(el, x, y) {
  if (open !== el) closeMenu();
  opener ??= document.activeElement;
  open = el;
  document.body.append(el);
  // Focus inside: the first control, or (a touch screen: no keyboard popping up) the sheet itself.
  el.tabIndex = -1;
  (matchMedia('(pointer:coarse)').matches ? el : el.querySelector('input,select,button:not(:disabled)') || el).focus({ preventScroll: true });
  if (innerWidth > 600) {
    const r = el.getBoundingClientRect();
    el.style.left = Math.max(8, Math.min(x, innerWidth - r.width - 8)) + 'px';
    el.style.top = Math.max(8, Math.min(y, innerHeight - r.height - 8)) + 'px';
  } else el.classList.add('sheet');
}

// A menu of actions under `btn`: items [id or attributes, text, act,
// '.cls', disabled] (falsy ones skipped).
export function actions(btn, id, items, head) {
  const r = btn.getBoundingClientRect();
  popup(h('div.menu', { id, role: 'menu' }, head && h('div.mh', head),
    items.filter(Boolean).map(([i, t, act, c = '', off]) => h('button.mi' + c, { ...(i?.at ? { id: i } : i), disabled: off, onclick: () => { closeMenu(); act(); } }, t))), r.right - 250, r.bottom + 4);
}

export function moreButton(ov, path, extra) {
  return h('button.sm.more', {
    title: 'Play less often, longer or loop…', 'aria-label': `Options for ${label(app.wasm.parsePath(path))}`,
    onclick: (ev) => { ev.stopPropagation(); const r = ev.currentTarget.getBoundingClientRect(); menu(ov, path, r.left - 200, r.bottom + 4, extra); },
  }, '…');
}
