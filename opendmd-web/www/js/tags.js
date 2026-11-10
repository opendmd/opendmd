// The user's tags and star ratings of GIFs: one file for the whole card,
// /sets/.tags (opendmd_core::tags through the wasm's TagBook: parsed,
// rendered and searched in Rust, in the art module: js/artmod.js). Loaded
// once per card, written whole after
// a change (one write at a time, changes made meanwhile in the next); a
// change made on another browser or a PC since it was read is read again
// first and this page's changes applied over it. Tags are the user's own
// words: what a name says (platform, folder, Logo…, author) is a GIF's
// details, never called a tag.

import { app } from './state.js';
import { art } from './artmod.js';
import { actions, popup, closeMenu } from './overrides.js';
import { h, clear, emit, toast } from './util.js';

export const FILE = '/sets/.tags';
const bytes = (t) => new TextEncoder().encode(t).length;

class Store {
  constructor(m) {
    this.m = m;
    this.book = new m.TagBook('');
    this.size = 0;
    this.ops = [];
    this.ok = false;
    this.saving = null;
    this.again = false;
  }

  async load() {
    const text = await app.sideText(FILE);
    this.book.free();
    this.book = new this.m.TagBook(text);
    this.size = bytes(text);
    this.ops = [];
    this.ok = true;
    emit('tags');
    return this;
  }

  // {r, t} (r 0: unrated) or null.
  get(p) { return this.book.get(p) || null; }
  // Every tag in use, [[tag, n]], most first.
  names() { return [...this.book.tags()]; }

  // Change the book (`fn(book)`), remember the change, save.
  change(fn) {
    if (!this.ok) { toast('your tags couldn’t be read: try again', 'bad'); return Promise.resolve(); }
    fn(this.book);
    this.ops.push(fn);
    emit('tags');
    return this.save();
  }
  set(p, r, t) { return this.change((b) => b.set(p, r, t)); }
  // `patch(old {r, t}) -> {r, t}` on each of `paths`; resolves to an undo.
  async bulk(paths, patch) {
    const was = paths.map((p) => [p, this.get(p) || { r: 0, t: [] }]);
    await this.change((b) => { for (const [p, o] of was) { const n = patch(o); b.set(p, n.r, n.t); } });
    return () => this.change((b) => { for (const [p, o] of was) b.set(p, o.r, o.t); });
  }
  // [{from, to | null}]: lines follow a rename, go with a delete. False if none moved.
  relink(pairs) {
    const hit = pairs.some(({ from }) => this.get(from));
    return hit ? this.change((b) => { for (const { from, to } of pairs) if (to) b.rename(from, to); else b.forget(from); }).then(() => true) : Promise.resolve(false);
  }

  async save() {
    if (this.saving) { this.again = true; return this.saving; }
    this.saving = (async () => {
      try {
        // Changed elsewhere since it was read (its size says so): read
        // again, this page's changes over it.
        const st = await app.src.stat(FILE).catch(() => null);
        if (st && (st.exists ? st.size : 0) !== this.size) {
          const text = (st.exists && (await app.src.readText(FILE))) || '';
          this.book.free();
          this.book = new this.m.TagBook(text);
          for (const fn of this.ops) fn(this.book);
        }
        const ops = this.ops.length, out = this.book.render();
        await app.writeSide(FILE, out);
        this.size = bytes(out);
        this.ops.splice(0, ops);
        // Smart playlists that read tags or stars are written again.
        import('./smart.js').then((m) => m.refreshAll((s) => s.usesTags)).catch(() => {});
      } catch (e) { toast(`saving your tags: ${e.message}`, 'bad'); }
    })();
    await this.saving;
    this.saving = null;
    if (this.again) { this.again = false; return this.save(); }
  }
}

// The card's tags, read once per card (`app.tags` too: the tree's search
// reads them).
let mine = null, key;
export async function tags() {
  if (!mine || key !== app.cardKey) {
    key = app.cardKey;
    mine = art().then((m) => new Store(m).load()).then((s) => (app.tags = s), (e) => { mine = null; throw e; });
  }
  return mine;
}

// --- In the Library ---------------------------------------------------------

const STAR = '★', NO_STAR = '☆';

// A row's glyphs: its stars (`★4`) and up to two of its tags, "+n" for more.
export function bits(o) {
  if (!o) return null;
  const t = o.t || [];
  return h('span.ut', o.r ? h('i.stars', { title: `${o.r} star${o.r > 1 ? 's' : ''}` }, STAR + o.r) : null,
    t.slice(0, 2).map((x) => h('span.utag', x)), t.length > 2 && h('span.utag.more', '+' + (t.length - 2)));
}

// Details' stars and tags for `path`, drawn again on every change.
// `onTag(tag)`: a tag's chip was picked (the Library searches for it).
export function detail(box, path, onTag) {
  const s = mine && app.tags;
  if (!s) { clear(box); tags().then(() => detail(box, path, onTag)).catch(() => {}); return; }
  const o = s.get(path) || { r: 0, t: [] };
  const stars = h('div.stars', { id: 'lib-d-stars', role: 'radiogroup', 'aria-label': 'Stars' },
    [1, 2, 3, 4, 5].map((n) => h('button.star' + (n <= o.r ? '.on' : ''), {
      role: 'radio', 'aria-checked': String(n === o.r), 'aria-label': `${n} star${n > 1 ? 's' : ''}`, title: n === o.r ? 'Tap again to clear' : '',
      'data-n': n, onclick: () => { s.set(path, n === o.r ? 0 : n, o.t); detail(box, path, onTag); box.querySelector(`[data-n="${n}"]`)?.focus(); },
    }, n <= o.r ? STAR : NO_STAR)));
  const list = h('datalist', { id: 'lib-tag-list' }, s.names().map(([t]) => h('option', { value: t })));
  const input = h('input', { type: 'text', id: 'lib-d-tag', placeholder: '+ Add tag', enterKeyHint: 'done', 'aria-label': 'Add a tag', maxLength: 24 });
  input.setAttribute('list', 'lib-tag-list');
  const add = () => {
    const t = s.m.cleanTag(input.value);
    if (!t) { if (input.value.trim()) toast('a tag is letters, digits, spaces and _ . + \' - (24 at most)', 'bad'); return; }
    if (o.t.includes(t)) { input.value = ''; return; }
    s.set(path, o.r, [...o.t, t]);
    detail(box, path, onTag);
    box.querySelector('#lib-d-tag')?.focus();
  };
  input.addEventListener('keydown', (e) => { if (e.key === 'Enter') { e.preventDefault(); add(); } });
  input.addEventListener('change', add);
  const chips = o.t.map((t) => h('span.uchip', { 'data-tag': t },
    h('button.txt', { title: `GIFs tagged ${t}`, onclick: () => onTag?.(t) }, t),
    h('button.x', { 'aria-label': `Take off ${t}`, onclick: () => { s.set(path, o.r, o.t.filter((x) => x !== t)); detail(box, path, onTag); } }, '✕')));
  clear(box, stars, h('div.utags', { id: 'lib-d-utags' }, chips, input, list));
}

// The bulk bar's Tag…: the tags on any of `paths`, each on all of them (✓),
// some (–) or none; a tap turns one on for all, then off for all, then (if
// it was) back to some. "+ Add tag" adds one. Apply writes once and says so,
// with an Undo.
export async function bulkTag(btn, paths, done) {
  const s = await tags(), n = new Map();
  for (const p of paths) for (const t of s.get(p)?.t || []) n.set(t, (n.get(t) || 0) + 1);
  const was = new Map([...n].map(([t, c]) => [t, c === paths.length ? 'all' : 'some'])), now = new Map(was);
  const box = h('div.chips', { id: 'bulk-tags' });
  const draw = () => clear(box, [...now].map(([t, st]) => h('button.chip' + (st === 'all' ? '.on' : st === 'some' ? '.part' : ''), {
    'data-tag': t, 'aria-pressed': st === 'all' ? 'true' : st === 'some' ? 'mixed' : 'false',
    onclick: () => { now.set(t, st === 'all' ? 'none' : st === 'none' && was.get(t) === 'some' ? 'some' : 'all'); draw(); },
  }, { all: '✓ ', some: '– ', none: '' }[st], t)));
  const input = h('input', { type: 'text', id: 'bulk-tag-new', placeholder: '+ Add tag', maxLength: 24 });
  input.setAttribute('list', 'bulk-tag-list');
  const plus = () => {
    const t = s.m.cleanTag(input.value);
    if (t) { now.set(t, 'all'); input.value = ''; draw(); } else if (input.value.trim()) toast('a tag is letters, digits, spaces and _ . + \' - (24 at most)', 'bad');
  };
  input.addEventListener('keydown', (e) => { if (e.key === 'Enter') { e.preventDefault(); plus(); } });
  draw();
  const el = h('div.menu', { id: 'bulk-tag', role: 'dialog', 'aria-label': 'Tags' }, h('div.mh', h('b', `Tags of ${paths.length.toLocaleString()} GIFs`)),
    box, h('div.row', { style: 'margin-top:8px' }, input, h('datalist', { id: 'bulk-tag-list' }, s.names().map(([t]) => h('option', { value: t })))),
    h('div.row', { style: 'margin-top:12px' }, h('button.pri', { id: 'bulk-tag-ok', onclick: async () => {
      plus();
      closeMenu();
      const undo = await s.bulk(paths, (o) => ({ r: o.r, t: [...o.t.filter((t) => now.get(t) !== 'none'), ...[...now].filter(([t, st]) => st === 'all' && !o.t.includes(t)).map(([t]) => t)] }));
      done?.();
      toast(`Tagged ${paths.length.toLocaleString()} GIFs`, 'ok', ['Undo', () => undo().then(() => done?.())]);
    } }, 'Apply')));
  const r = btn.getBoundingClientRect();
  popup(el, r.right - 320, r.bottom + 4);
}

// The bulk bar's Rate…: five stars, or none, for all of `paths`.
export async function bulkRate(btn, paths, done) {
  const s = await tags();
  const rate = async (n) => {
    closeMenu();
    const undo = await s.bulk(paths, (o) => ({ r: n, t: o.t }));
    done?.();
    toast(n ? `Rated ${paths.length.toLocaleString()} GIFs ${STAR}${n}` : `Stars cleared from ${paths.length.toLocaleString()} GIFs`, 'ok', ['Undo', () => undo().then(() => done?.())]);
  };
  actions(btn, 'bulk-rate', [...[5, 4, 3, 2, 1].map((n) => [{ 'data-n': n }, STAR.repeat(n) + NO_STAR.repeat(5 - n), () => rate(n)]),
    [{ 'data-n': 0 }, 'Clear stars', () => rate(0)]], `Stars for ${paths.length.toLocaleString()} GIFs`);
}

// Tags kept for GIFs not on the card (moved or deleted on a PC), given every
// GIF the card has: [{path, to}] — `to` the one GIF elsewhere with the same
// file name, if there is exactly one.
export function strays(paths) {
  const s = app.tags, low = (p) => p.toLowerCase(), name = (p) => low(p.slice(p.lastIndexOf('/') + 1));
  if (!s) return [];
  const all = new Set(paths.map(low)), byName = new Map();
  for (const p of paths) byName.set(name(p), byName.has(name(p)) ? null : p);
  return [...s.book.paths()].filter((p) => !all.has(low(p))).map((p) => ({ path: p, to: byName.get(name(p)) || null }));
}
