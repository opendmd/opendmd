// The user's tags of GIFs: one file for the whole card,
// /sets/.tags (opendmd_core::tags through the wasm's TagBook: parsed,
// rendered and searched in Rust, in the art module: js/artmod.js). Loaded
// once per card, written whole after
// a change (one write at a time, changes made meanwhile in the next); a
// change made on another browser or a PC since it was read is read again
// first and this page's changes applied over it. Tags are the user's own
// words: what a name says (platform, folder, Logo…, author) is a GIF's
// details, never called a tag. One is built in: `favourite` (FAV), the
// heart. Star ratings (`r=` in the file, from before) are kept as they
// are but no longer shown or set: tags only.

import { app } from './state.js';
import { art } from './artmod.js';
import { Overrides, popup, closeMenu } from './overrides.js';
import { h, clear, emit, toast, lines } from './util.js';

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
    mine = art().then((m) => new Store(m).load()).then((s) => { app.tags = s; emit('tags'); return s; }, (e) => { mine = null; throw e; });
  }
  return mine;
}

// --- Favourite: the built-in tag -------------------------------------------

export const FAV = 'favourite';
// Its weight in a playlist's overrides (js/overrides.js `fav`): three times as often.
const FAV_W = 3;
export const isFav = (o) => !!o?.t?.includes(FAV);
// Every tag in use for a picker, `favourite` first even when none has it.
const offered = (s) => [FAV, ...s.names().map(([t]) => t).filter((t) => t !== FAV)];

// `paths` favourites or not: the tag, and, so that a favourite still plays
// more often, its weight in the playlist playing (the player reads that,
// not the tags; `ov`: that playlist's overrides if the caller has them
// loaded; `s`: the tags, when the caller has them: the tag is then changed
// before this returns its promise, for a redraw at once). -> an undo that
// puts back the tags and the weights exactly as they were, or null if it
// was refused (said in a toast).
export async function setFav(paths, on, ov, s) {
  const now = !!s;
  let plan = null;
  if (!now) {
    s = await tags();
    if ((plan = await weighing(paths, on, ov))?.no) { toast(plan.no, 'bad'); return null; }
  }
  const undo = await s.bulk(paths, (o) => ({ r: o.r, t: on ? (o.t.includes(FAV) ? o.t : [...o.t, FAV]) : o.t.filter((t) => t !== FAV) }));
  if (now && (plan = await weighing(paths, on, ov))?.no) { await undo(); toast(plan.no, 'bad'); return null; }
  const back = await plan?.go();
  return async () => { await undo(); await back?.(); };
}

// The weights a favourite change makes in the playlist playing: only for
// GIFs in it, only when it shuffles (in order, a weight changes nothing:
// no write, no reindex), and never past the 1,024 overrides the player
// keeps for a playlist. -> null (nothing to write), {no: why}, or {go()},
// which writes them and resolves to an undo of exactly that.
const MAX_OV = 1024;
export async function weighing(paths, on, ov) {
  if (!app.device || !app.activePath || app.config?.shuffle === false) return null;
  const text = await app.text(app.activePath).catch(() => null);
  if (text == null) return null;
  const has = new Set(lines(text).map((l) => l.toLowerCase()));
  if (ov?.path !== app.activePath) ov = await new Overrides().load(app.activePath);
  if (!ov.ok) return null;
  const todo = paths.filter((p) => has.has(p.toLowerCase()) && (ov.get(p)?.w > 1) !== on);
  if (!todo.length) return null;
  const n = ov.map.size + todo.filter((p) => !ov.get(p)).length;
  if (on && n > MAX_OV) return { no: `“${ov.name}” can give at most ${MAX_OV.toLocaleString()} GIFs settings of their own, and this would make it ${n.toLocaleString()}: favourite fewer at once` };
  const was = new Map(todo.map((p) => [p, ov.get(p)?.w ?? null]));
  return {
    async go() {
      await ov.setMany(todo.map((p) => [p, (o) => ({ ...o, w: on ? FAV_W : null })]), true);
      return () => ov.setMany([...was].map(([p, w]) => [p, (o) => ({ ...o, w })]), true);
    },
  };
}

// Favourites from before the tag (a weight above 1 in the playlist
// playing, as Now's ☆ wrote): adopted into the tag when that playlist's
// overrides are read, once a session each, so the heart, Details and the
// Favourites chip all see them.
const adopted = new Set();
export async function adopt(ov) {
  if (!ov?.ok || ov.path !== app.activePath) return;
  const ps = [...ov.map.values()].filter((o) => o.w > 1 && !adopted.has(o.path)).map((o) => o.path);
  if (!ps.length) return;
  const s = await tags();
  for (const p of ps) adopted.add(p);
  const add = ps.filter((p) => !isFav(s.get(p)));
  if (add.length) await s.bulk(add, (o) => ({ r: o.r, t: [...o.t, FAV] }));
}

// --- In the Library ---------------------------------------------------------

// A row's glyphs: ♥ for a favourite and up to two of its other tags, "+n" for more.
export function bits(o) {
  if (!o) return null;
  const t = (o.t || []).filter((x) => x !== FAV);
  return h('span.ut', isFav(o) && h('i.fav', { title: 'Favourite' }, '♥'),
    t.slice(0, 2).map((x) => h('span.utag', x)), t.length > 2 && h('span.utag.more', '+' + (t.length - 2)));
}

// Details' heart and tags for `path`, drawn again on every change.
// `onTag(tag)`: a tag's chip was picked (the Library searches for it);
// `ov`: the overrides the Library has loaded (setFav's).
export function detail(box, path, onTag, ov) {
  const s = mine && app.tags;
  if (!s) { clear(box); tags().then(() => detail(box, path, onTag, ov)).catch(() => {}); return; }
  const o = s.get(path) || { r: 0, t: [] }, fav = isFav(o);
  const heart = h('button.chip.heart' + (fav ? '.on' : ''), { id: 'lib-d-fav', 'data-path': path, 'aria-pressed': String(fav), title: fav ? 'A favourite: plays more often' : 'Make it a favourite: plays more often',
    onclick: () => { setFav([path], !fav, ov, s).then(() => detail(box, path, onTag, ov)); detail(box, path, onTag, ov); box.querySelector('#lib-d-fav')?.focus(); } }, fav ? '♥ Favourite' : '♡ Favourite');
  const list = h('datalist', { id: 'lib-tag-list' }, offered(s).map((t) => h('option', { value: t })));
  const input = h('input', { type: 'text', id: 'lib-d-tag', placeholder: '+ Add tag', enterKeyHint: 'done', 'aria-label': 'Add a tag', maxLength: 24 });
  input.setAttribute('list', 'lib-tag-list');
  const add = () => {
    const t = s.m.cleanTag(input.value);
    if (!t) { if (input.value.trim()) toast('a tag is letters, digits, spaces and _ . + \' - (24 at most)', 'bad'); return; }
    if (o.t.includes(t)) { input.value = ''; return; }
    if (t === FAV) setFav([path], true, ov, s); else s.set(path, o.r, [...o.t, t]);
    detail(box, path, onTag, ov);
    box.querySelector('#lib-d-tag')?.focus();
  };
  input.addEventListener('keydown', (e) => { if (e.key === 'Enter') { e.preventDefault(); add(); } });
  input.addEventListener('change', add);
  const chips = o.t.filter((t) => t !== FAV).map((t) => h('span.uchip', { 'data-tag': t },
    h('button.txt', { title: `GIFs tagged ${t}`, onclick: () => onTag?.(t) }, t),
    h('button.x', { 'aria-label': `Take off ${t}`, onclick: () => { s.set(path, o.r, o.t.filter((x) => x !== t)); detail(box, path, onTag, ov); } }, '✕')));
  clear(box, h('div.utags', { id: 'lib-d-utags' }, heart, chips, input, list));
}

// The bulk bar's Tag…: the tags on any of `paths`, each on all of them (✓),
// some (–) or none; a tap turns one on for all, then off for all, then (if
// it was) back to some. "+ Add tag" adds one. Apply writes once and says so,
// with an Undo.
export async function bulkTag(btn, paths, done) {
  const s = await tags(), n = new Map();
  for (const p of paths) for (const t of s.get(p)?.t || []) n.set(t, (n.get(t) || 0) + 1);
  const was = new Map([[FAV, 'none'], ...[...n].map(([t, c]) => [t, c === paths.length ? 'all' : 'some'])]), now = new Map(was);
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
    box, h('div.row', { style: 'margin-top:8px' }, input, h('datalist', { id: 'bulk-tag-list' }, offered(s).map((t) => h('option', { value: t })))),
    h('div.row', { style: 'margin-top:12px' }, h('button.pri', { id: 'bulk-tag-ok', onclick: async () => {
      plus();
      closeMenu();
      // (Favourite on or off for them all: their weights follow, as setFav's; refused, nothing changes.)
      const nf = now.get(FAV), plan = nf !== was.get(FAV) && nf !== 'some' ? await weighing(paths, nf === 'all') : null;
      if (plan?.no) { toast(plan.no, 'bad'); return; }
      const undo = await s.bulk(paths, (o) => ({ r: o.r, t: [...o.t.filter((t) => now.get(t) !== 'none'), ...[...now].filter(([t, st]) => st === 'all' && !o.t.includes(t)).map(([t]) => t)] }));
      const back = await plan?.go();
      done?.();
      toast(`Tagged ${paths.length.toLocaleString()} GIFs`, 'ok', ['Undo', () => undo().then(() => back?.()).then(() => done?.())]);
    } }, 'Apply')));
  const r = btn.getBoundingClientRect();
  popup(el, r.right - 320, r.bottom + 4);
}

// The bulk bar's ♥ Favourite: all of `paths` favourites (or, if they all
// are, none), with an Undo.
export async function bulkFav(paths, done) {
  const s = await tags(), on = !paths.every((p) => isFav(s.get(p))), n = paths.length.toLocaleString();
  const undo = await setFav(paths, on);
  if (!undo) return;
  done?.();
  toast(on ? `${n} GIFs are favourites` : `${n} GIFs are no longer favourites`, 'ok', ['Undo', () => undo().then(() => done?.())]);
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
