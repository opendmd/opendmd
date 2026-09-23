// What the user said about one GIF in one playlist: favourite, play less,
// hide, play longer, loop N times. Shown as small glyphs on Library and
// Playlist rows; changed from a menu (right-click, long-press, or the row's
// "…" button). Each playlist keeps its own: the same GIF can be a favourite
// in one and hidden in another.
//
// On the card it is a small text file beside the playlist, parsed and
// written by the core (wasm parseOverrides/renderOverrides); the device reads
// it back as GET /api/overrides?set=. Saving writes the whole file, and the
// device rebuilds that playlist's index (a few seconds when it is the one
// playing). None of that shows in the page's words.

import { app, playlistLabel, setValue } from './state.js';
import { h, clear, toast, label, base } from './util.js';

const FAV = 3, LESS = 0.5;

// The overrides file beside a playlist: /sets/x.txt -> /sets/x.meta,
// /filelist.txt -> /filelist.meta, /filelist.hall -> /filelist.hall.meta.
const fileFor = (p) => (p.endsWith('.txt') ? p.slice(0, -4) : p) + '.meta';

export class Overrides {
  constructor() {
    this.path = '';      // the playlist's card path
    this.map = new Map(); // gif path -> {path, never?, w?, dwell?, dwell_x?, loops?}
    this.saving = null;
    this.again = false;
    this.onChange = () => {};
  }

  get name() { return playlistLabel(this.path); }

  async load(path) {
    this.path = path;
    this.map = new Map();
    if (!path) return this;
    let list = [];
    try {
      if (app.device) list = (await app.src.overrides(setValue(path))).overrides;
      else {
        const text = await app.src.readText(fileFor(path));
        if (text) list = [...app.wasm.parseOverrides(text)];
      }
    } catch (e) { toast(`${this.name}: ${e.message}`, 'bad'); }
    if (this.path === path) for (const o of list) this.map.set(o.path, o);
    this.onChange();
    return this;
  }

  get(p) { return this.map.get(p); }

  // Replace one GIF's override (an empty one removes it) and save.
  set(p, o) {
    const keep = Object.fromEntries(Object.entries(o).filter(([k, v]) => k !== 'path' && v != null && v !== false));
    if (Object.keys(keep).length) this.map.set(p, { path: p, ...keep });
    else this.map.delete(p);
    this.onChange();
    return this.save();
  }

  // One write at a time; changes made meanwhile go out in the next.
  async save() {
    if (this.saving) { this.again = true; return this.saving; }
    const file = fileFor(this.path);
    const text = app.wasm.renderOverrides([...this.map.values()]);
    this.saving = (async () => {
      try {
        if (text || !app.device) await app.src.writeFile(file, text);
        else await app.src.deleteFile(file);
        toast(`saved for “${this.name}”${app.device && app.activePath === this.path ? ' · the player is catching up (a few seconds)' : ''}`, 'ok');
      } catch (e) { toast(`saving for “${this.name}”: ${e.message}`, 'bad'); }
    })();
    await this.saving;
    this.saving = null;
    if (this.again) { this.again = false; return this.save(); }
  }
}

// The little marks on a row.
export function glyphs(o) {
  if (!o) return null;
  const g = [];
  if (o.never) g.push(['⊘', 'Hidden from this playlist', 'hid']);
  if (o.w > 1) g.push(['★', 'Favourite: plays more often', 'fav']);
  else if (o.w != null && o.w < 1) g.push(['▽', 'Plays less often', 'less']);
  if (o.dwell) g.push([`◔${o.dwell}s`, `Plays for ${o.dwell} seconds`, 'long']);
  else if (o.dwell_x) g.push([`◔${o.dwell_x}×`, `Plays ${o.dwell_x}× as long`, 'long']);
  if (o.loops) g.push([`↻${o.loops}`, `Loops ${o.loops} times`, 'loop']);
  return g.length ? h('span.ov', g.map(([t, title, c]) => h('i.' + c, { title }, t))) : null;
}

let open = null;
export function closeMenu() { open?.remove(); open = null; }
addEventListener('keydown', (e) => { if (e.key === 'Escape') closeMenu(); });
addEventListener('pointerdown', (e) => { if (open && !open.contains(e.target)) closeMenu(); }, true);
addEventListener('resize', closeMenu);

// The menu for one GIF, at (x, y) — a sheet along the bottom on a phone.
export function menu(ov, path, x, y) {
  closeMenu();
  if (!ov.path) { toast('save the playlist first', 'bad'); return; }
  const o = ov.get(path) || {};
  const put = (patch) => { closeMenu(); ov.set(path, { ...o, ...patch }); };
  const secs = h('input', { type: 'number', min: 1, max: 86400, placeholder: 's', id: 'ov-secs', value: o.dwell || '', style: 'width:72px' });
  const loops = h('input', { type: 'number', min: 1, max: 1000, placeholder: '×', id: 'ov-loops', value: o.loops || '', style: 'width:64px' });
  const item = (id, on, text, act) => h('button.mi' + (on ? '.on' : ''), { id, onclick: act }, h('span.ck', on ? '✓' : ''), text);
  const e = app.wasm.parsePath(path);
  open = h('div.menu', { id: 'ov-menu', role: 'menu' },
    h('div.mh', h('b', label(e)), h('span.muted', ` in “${ov.name}”`)),
    item('ov-fav', o.w > 1, '★ Favourite — plays more often', () => put({ w: o.w > 1 ? null : FAV, never: null })),
    item('ov-less', o.w != null && o.w < 1, '▽ Play less often', () => put({ w: o.w != null && o.w < 1 ? null : LESS, never: null })),
    item('ov-hide', o.never, '⊘ Hide from this playlist', () => put({ never: o.never ? null : true, w: null })),
    h('div.mr', h('span', 'Play longer'),
      h('button.sm' + (o.dwell_x === 2 ? '.on' : ''), { id: 'ov-2x', onclick: () => put({ dwell_x: o.dwell_x === 2 ? null : 2, dwell: null }) }, '2×'),
      secs, h('button.sm', { id: 'ov-secs-set', onclick: () => { const v = +secs.value; if (v >= 1 && v <= 86400) put({ dwell: Math.round(v), dwell_x: null }); else toast('seconds: 1 to 86400', 'bad'); } }, 'Set')),
    h('div.mr', h('span', 'Loop'), loops, h('span.muted', 'times'),
      h('button.sm', { id: 'ov-loops-set', onclick: () => { const v = +loops.value; if (v >= 1 && v <= 1000) put({ loops: Math.round(v) }); else toast('loops: 1 to 1000', 'bad'); } }, 'Set')),
    item('ov-reset', false, 'Reset', () => put({ never: null, w: null, dwell: null, dwell_x: null, loops: null })));
  open.querySelector('#ov-reset').disabled = !ov.get(path);
  document.body.append(open);
  if (innerWidth > 600) {
    const r = open.getBoundingClientRect();
    open.style.left = Math.max(8, Math.min(x, innerWidth - r.width - 8)) + 'px';
    open.style.top = Math.max(8, Math.min(y, innerHeight - r.height - 8)) + 'px';
  } else open.classList.add('sheet');
}

// The "…" button a row carries so the menu is discoverable.
export function moreButton(ov, path) {
  return h('button.sm.more', {
    title: 'Favourite, hide, play longer…', 'aria-label': `options for ${base(path)}`,
    onclick: (ev) => { ev.stopPropagation(); const r = ev.currentTarget.getBoundingClientRect(); menu(ov, path, r.left - 200, r.bottom + 4); },
  }, '…');
}

// A playlist chooser for views that act on "the playlist" (Library).
export function playlistSelect(lists, value, onchange) {
  const sel = h('select', { id: 'lib-ovset', title: 'Favourites and hiding apply to this playlist', onchange: () => onchange(sel.value) });
  clear(sel, lists.map((p) => h('option', { value: p.path }, p.label)));
  sel.value = value;
  return sel;
}
