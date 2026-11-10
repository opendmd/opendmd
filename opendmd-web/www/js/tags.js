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
import { emit, toast } from './util.js';

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
