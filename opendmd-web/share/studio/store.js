// The Studio's records and the one place they are kept (/studio/).
//
// Everything the Studio makes is a record with the same envelope: a stable
// random id (never a name, a position or a counter), `rev` (rises only at a
// snapshot or an export, never on an autosave), `hash` (SHA-256 of the
// canonical body), name, tags, created, modified, `author` ({name, account};
// account stays null until there are accounts), `licence` (null: not chosen,
// which is not a licence) and `from` (what it was made from: {id, rev, name,
// author, licence}, or {external: true, note}). Records point at each other
// by id. Pixels are palette keys, deflated, in base64.
//
// The app talks to the Store interface and nothing else:
//   get(id, rev?)            the record (at a frozen revision), or null
//   list({type, tag, text})  {items: heads (no body), next}
//   put(rec, {baseRev, baseHash})   refused (Conflict) when the stored record
//                            moved on: another rev, or another tab's content
//   snapshot(id)             freeze the record as it is (into revs) and move
//                            it on to the next rev; nothing new if unchanged
//   remove(id)               leaves a tombstone
//   revs(id)                 its frozen revisions, oldest first
//   usedBy(id)               the records that point at it
//   changes(since)           every head written after `since`, for a later sync
// LocalStore keeps them in IndexedDB now; a RemoteStore (the site's Worker)
// and a SyncedStore over both can come later behind the same interface.
// Conflicts are settled by rev and hash, never by clock time. The undo
// journal (its own object store) never leaves the browser.

export const FORMAT = 'opendmd-studio/1';
export const TYPES = { piece: 'pc', palette: 'pl', stamp: 'st', font: 'ft', shade: 'sh', template: 'tp', pack: 'pk' };
export const LICENCES = ['CC0-1.0', 'CC-BY-4.0'];
// Caps, set now so nothing grows without bound.
export const CAPS = { frames: 256, layers: 16, bytes: 1 << 20, name: 120, tags: 32, from: 64 };
const ID = /^(pc|pl|st|ft|sh|tp|pk)_[0-9a-f]{32}$/;

export class Conflict extends Error {
  constructor(head) { super('changed elsewhere'); this.name = 'Conflict'; this.head = head; }
}

const hex = (b) => [...b].map((x) => x.toString(16).padStart(2, '0')).join('');

// A new id: UUIDv7's layout (48 bits of milliseconds, version 7, the
// variant, 74 random bits) with the type's prefix. Never reused.
export function newId(type) {
  const r = crypto.getRandomValues(new Uint8Array(16));
  let t = Date.now();
  for (let i = 5; i >= 0; i--) { r[i] = t % 256; t = Math.floor(t / 256); }
  r[6] = 0x70 | (r[6] & 0x0f);
  r[8] = 0x80 | (r[8] & 0x3f);
  return `${TYPES[type]}_${hex(r)}`;
}
// The built-in resources' fixed ids: time zero, which no new id has.
export const builtinId = (type, n) => `${TYPES[type]}_000000000000700080${n.toString(16).padStart(14, '0')}`;
export const isBuiltin = (id) => id.slice(3, 15) === '000000000000';

const now = () => new Date().toISOString().replace(/\.\d+Z$/, 'Z');

export function record(type, name, body, { author = '', licence = null, from = [], tags = [] } = {}) {
  const t = now();
  return { type, id: newId(type), rev: 1, hash: '', name, tags, author: { name: author, account: null },
    licence, from, created: t, modified: t, body };
}

// The body as canonical JSON (keys sorted, no spaces): what the hash covers.
export function canon(v) {
  if (Array.isArray(v)) return `[${v.map(canon).join(',')}]`;
  if (v && typeof v === 'object') return `{${Object.keys(v).sort().map((k) => `${JSON.stringify(k)}:${canon(v[k])}`).join(',')}}`;
  return JSON.stringify(v);
}
export async function hashOf(body) {
  return 'sha256-' + hex(new Uint8Array(await crypto.subtle.digest('SHA-256', new TextEncoder().encode(canon(body)))));
}

// Pixels: palette keys, zlib-deflated, base64.
async function through(bytes, stream) {
  return new Uint8Array(await new Response(new Blob([bytes]).stream().pipeThrough(stream)).arrayBuffer());
}
export async function packPx(keys) {
  const z = await through(keys, new CompressionStream('deflate'));
  let s = '';
  for (let i = 0; i < z.length; i += 0x8000) s += String.fromCharCode(...z.subarray(i, i + 0x8000));
  return btoa(s);
}
export async function unpackPx(b64, n) {
  const z = Uint8Array.from(atob(b64), (c) => c.charCodeAt(0));
  const px = await through(z, new DecompressionStream('deflate'));
  if (px.length !== n) throw new Error('a picture of the wrong size');
  return px;
}

// What a record's body points at (by id): its palette, its stamps.
export function refsOf(rec) {
  const b = rec.body || {}, out = new Set();
  if (typeof b.palette === 'string') out.add(b.palette);
  for (const l of b.layers || []) for (const s of l.stamps || []) if (typeof s.ref === 'string') out.add(s.ref);
  for (const m of b.members || []) if (typeof m.id === 'string') out.add(m.id);
  out.delete(rec.id);
  return [...out];
}

// The envelope and the caps; throws with what is wrong.
export function check(rec) {
  const bad = (why) => { throw new Error(`not a record: ${why}`); };
  if (!TYPES[rec.type] || !ID.test(rec.id) || rec.id.slice(0, 2) !== TYPES[rec.type]) bad('type or id');
  if (!Number.isInteger(rec.rev) || rec.rev < 1) bad('rev');
  if (typeof rec.name !== 'string' || rec.name.length > CAPS.name) bad('name');
  if (!Array.isArray(rec.tags) || rec.tags.length > CAPS.tags || rec.tags.some((t) => typeof t !== 'string')) bad('tags');
  if (!rec.author || typeof rec.author.name !== 'string' || rec.author.name.length > 80 || rec.author.account !== null) bad('author');
  if (rec.licence !== null && !LICENCES.includes(rec.licence)) bad('licence');
  if (!Array.isArray(rec.from) || rec.from.length > CAPS.from) bad('from');
  const b = rec.body;
  if (!b || typeof b !== 'object') bad('body');
  if (rec.type === 'piece') {
    if (b.w !== 128 || b.h !== 32) bad('size');
    if (!Array.isArray(b.frames) || !b.frames.length || b.frames.length > CAPS.frames) bad('frames');
    if (!Array.isArray(b.layers) || !b.layers.length || b.layers.length > CAPS.layers) bad('layers');
    if (b.layers.some((l) => !Array.isArray(l.cels) || l.cels.length !== b.frames.length)) bad('cels');
  }
  if (JSON.stringify(rec).length > CAPS.bytes) bad('too big');
}

const head = (rec) => { const { body, ...h } = rec; return h; };

// --- IndexedDB ---
const req = (r) => new Promise((ok, no) => { r.onsuccess = () => ok(r.result); r.onerror = () => no(r.error); });
const done = (tx) => new Promise((ok, no) => { tx.oncomplete = ok; tx.onerror = tx.onabort = () => no(tx.error); });

export class LocalStore {
  static async open(name = 'opendmd-studio') {
    const r = indexedDB.open(name, 1);
    r.onupgradeneeded = () => {
      const db = r.result;
      db.createObjectStore('records', { keyPath: 'id' }).createIndex('seq', 'seq');
      db.createObjectStore('revs', { keyPath: ['id', 'rev'] });
      db.createObjectStore('journal', { autoIncrement: true }).createIndex('id', 'id');
      db.createObjectStore('refs', { keyPath: ['to', 'from'] }).createIndex('from', 'from');
      db.createObjectStore('meta', { keyPath: 'k' });
    };
    return new LocalStore(await req(r));
  }
  constructor(db) { this.db = db; }

  async get(id, rev) {
    const tx = this.db.transaction(rev ? 'revs' : 'records');
    const r = await req(tx.objectStore(rev ? 'revs' : 'records').get(rev ? [id, rev] : id));
    return r && !r.gone ? strip(r) : null;
  }

  async list({ type, tag, text } = {}) {
    const all = await req(this.db.transaction('records').objectStore('records').getAll());
    const t = text?.toLowerCase();
    const items = all.filter((r) => !r.gone && (!type || r.type === type) && (!tag || r.tags.includes(tag))
      && (!t || r.name.toLowerCase().includes(t))).map((r) => head(strip(r)))
      .sort((a, b) => (a.modified < b.modified ? 1 : a.modified > b.modified ? -1 : 0));
    return { items, next: null };
  }

  async put(rec, { baseRev = null, baseHash = null } = {}) {
    const out = { ...rec, hash: await hashOf(rec.body), modified: now() };
    check(out);
    const tx = this.db.transaction(['records', 'refs', 'meta'], 'readwrite');
    const recs = tx.objectStore('records');
    const was = await req(recs.get(out.id));
    if (was && !was.gone && (baseRev !== was.rev || (baseHash && baseHash !== was.hash))) {
      tx.abort();
      throw new Conflict(head(strip(was)));
    }
    if (was?.gone) { tx.abort(); throw new Conflict(head(strip(was))); }
    out.seq = await this.#seq(tx);
    recs.put(out);
    this.#refs(tx, out);
    await done(tx);
    return head(strip(out));
  }

  async snapshot(id) {
    const tx = this.db.transaction(['records', 'revs', 'meta'], 'readwrite');
    const recs = tx.objectStore('records'), revs = tx.objectStore('revs');
    const cur = await req(recs.get(id));
    if (!cur || cur.gone) { tx.abort(); throw new Error('no such record'); }
    const last = cur.rev > 1 ? await req(revs.get([id, cur.rev - 1])) : null;
    if (last && last.hash === cur.hash) { await done(tx); return head(strip(last)); }
    const frozen = strip(cur);
    revs.put(frozen);
    recs.put({ ...cur, rev: cur.rev + 1, seq: await this.#seq(tx) });
    await done(tx);
    return head(frozen);
  }

  async remove(id) {
    const tx = this.db.transaction(['records', 'revs', 'refs', 'meta'], 'readwrite');
    const recs = tx.objectStore('records');
    const cur = await req(recs.get(id));
    if (cur && !cur.gone) {
      recs.put({ id, type: cur.type, rev: cur.rev, gone: true, modified: now(), seq: await this.#seq(tx) });
      tx.objectStore('revs').delete(IDBKeyRange.bound([id, 0], [id, Infinity]));
      this.#refs(tx, { id, body: {} });
    }
    await done(tx);
  }

  async revs(id) {
    const r = await req(this.db.transaction('revs').objectStore('revs').getAll(IDBKeyRange.bound([id, 0], [id, Infinity])));
    return r.map(head);
  }

  async usedBy(id) {
    const tx = this.db.transaction(['refs', 'records']);
    const links = await req(tx.objectStore('refs').getAll(IDBKeyRange.bound([id, ''], [id, '￿'])));
    const recs = await Promise.all(links.map((l) => req(tx.objectStore('records').get(l.from))));
    return recs.filter((r) => r && !r.gone).map((r) => head(strip(r)));
  }

  async changes(since = 0) {
    const all = await req(this.db.transaction('records').objectStore('records').index('seq').getAll(IDBKeyRange.lowerBound(since, true)));
    return { items: all.map((r) => { const { body, ...h } = r; return h; }), next: all.length ? all[all.length - 1].seq : since };
  }

  // A local, rising number for every write (changes(since)); kept in meta.
  async #seq(tx) {
    const meta = tx.objectStore('meta');
    const n = ((await req(meta.get('seq')))?.v || 0) + 1;
    meta.put({ k: 'seq', v: n });
    return n;
  }

  #refs(tx, rec) {
    const refs = tx.objectStore('refs');
    const idx = refs.index('from');
    const c = idx.openKeyCursor(IDBKeyRange.only(rec.id));
    const want = refsOf(rec);
    c.onsuccess = () => {
      const k = c.result;
      if (k) { refs.delete(k.primaryKey); k.continue(); } else for (const to of want) refs.put({ to, from: rec.id });
    };
  }
}

// The stored copy without the store's own bookkeeping.
function strip(r) { const { seq, ...rest } = r; return rest; }
