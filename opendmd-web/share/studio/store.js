// The Studio's records and the one place they are kept (/studio/).
//
// Everything the Studio makes is a record with the same envelope: a stable
// random id (never a name, a position or a counter), `rev` (rises only at a
// snapshot or an export, never on an autosave), `hash` (hashOf: SHA-256 of
// the canonical body, pixels uncompressed), name, tags, an optional one-line
// `note`, created, modified, `author` ({name, account}; account is null until
// there are accounts, then a string), `licence` (null: not chosen, which is
// not a licence) and `from` (what it was made from: {id, rev, hash, name,
// author, licence}, or {external: true, note}; only `external === true` is
// external). Records point at each other by id. Pixels are palette keys,
// zlib-deflated (CompressionStream 'deflate'), in base64: one byte a pixel,
// or, in a piece whose body says `keys: 2` (one past 255 colours), two,
// high byte first. A piece's own colours are `body.colours` ({k, hex}, keys
// from 1; 0 is off); without them (S0 drafts, templates) they are its
// palette's. `body.palette` is the swatch set it was drawn from.
//
// The app talks to the Store interface and nothing else:
//   get(id, rev?)            the record (at a frozen revision), or null
//   list({type, tag, text})  {items: heads (no body), next}
//   put(rec, {base})         refused (Conflict) unless `base` is the stored
//                            record's `ver` (an opaque store version every
//                            head carries, moved on by any write: another
//                            tab's rename too); required when it exists
//   snapshot(id, label?)     freeze the record as it is (into revs, named
//                            `label`) and move it on to the next rev; nothing
//                            new if unchanged and unnamed. Returns the head
//                            it goes on with
//   remove(id)               leaves a tombstone
//   revs(id)                 its frozen revisions, oldest first
//   usedBy(id)               the records that point at it
//   changes(since)           every head written after `since`, for a later sync
// LocalStore keeps them in IndexedDB now; a RemoteStore (the site's Worker)
// and a SyncedStore over both can come later behind the same interface.
// Conflicts are settled by the store's version and the hash, never by clock
// time. The undo journal (LocalStore's own: steps(), jot(), its object store)
// never leaves the browser.

export const FORMAT = 'opendmd-studio/1';
export const TYPES = { piece: 'pc', palette: 'pl', stamp: 'st', font: 'ft', shade: 'sh', template: 'tp', pack: 'pk' };
export const LICENCES = ['CC0-1.0', 'CC-BY-4.0'];
// Caps, set now so nothing grows without bound.
export const CAPS = { frames: 256, layers: 16, bytes: 1 << 20, name: 120, note: 200, tags: 32, from: 64, colours: 4096 };
const VERSION = 2; // the IndexedDB schema (LocalStore.open's steps)
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

// Canonical JSON (keys sorted, no spaces, UTF-8): what the hash covers.
// Only JSON's own values, and integers for numbers, so any language can
// write the same text (Python: json.dumps(v, sort_keys=True,
// separators=(',', ':'), ensure_ascii=False)); anything else is refused.
// No floats, ever (docs/studio-format.md): a record that needs a fraction
// says it in whole units (ms, not s; a percentage as 0..100).
export function canon(v) {
  if (v === null || typeof v === 'boolean' || typeof v === 'string') return JSON.stringify(v);
  if (typeof v === 'number') {
    if (!Number.isSafeInteger(v)) throw new Error(`canon: ${v} is not an integer`);
    return String(v);
  }
  if (Array.isArray(v)) return `[${v.map(canon).join(',')}]`;
  if (typeof v === 'object' && Object.getPrototypeOf(v) === Object.prototype) {
    return `{${Object.keys(v).sort().map((k) => `${JSON.stringify(k)}:${canon(v[k])}`).join(',')}}`;
  }
  throw new Error(`canon: ${v === undefined ? 'undefined' : typeof v} is not JSON`);
}
// The hash: the body with every pixel field (each string in a `cels` array,
// each `px` string, anywhere in it) replaced by its pixels uncompressed, as
// lowercase hex; then canon, then SHA-256, as 'sha256-' + lowercase hex. The
// compressed bytes differ between browsers; the pixels do not. A two-byte
// piece's cels are its bytes as stored (high byte first), so the rule is the
// same and every one-byte hash is what it was; `keys: 2` is in the body.
const raw = new Map(); // cel string -> its hex: an unchanged cel is inflated once
async function rawHex(c) {
  let h = raw.get(c);
  if (h === undefined) {
    h = hex(await inflate(c));
    if (raw.size > 64) raw.delete(raw.keys().next().value);
    raw.set(c, h);
  }
  return h;
}
export async function pixelForm(v) {
  if (Array.isArray(v)) return Promise.all(v.map(pixelForm));
  if (!v || typeof v !== 'object') return v;
  const out = {};
  for (const [k, x] of Object.entries(v)) {
    if (k === 'cels' && Array.isArray(x)) out[k] = await Promise.all(x.map((c) => (typeof c === 'string' ? rawHex(c) : pixelForm(c))));
    else if (k === 'px' && typeof x === 'string') out[k] = await rawHex(x);
    else out[k] = await pixelForm(x);
  }
  return out;
}
export async function hashOf(body) {
  const text = canon(await pixelForm(body));
  return 'sha256-' + hex(new Uint8Array(await crypto.subtle.digest('SHA-256', new TextEncoder().encode(text))));
}

// Pixels: palette keys, zlib-deflated, base64; `wide`: two bytes a key, high first.
async function through(bytes, stream) {
  return new Uint8Array(await new Response(new Blob([bytes]).stream().pipeThrough(stream)).arrayBuffer());
}
export async function packPx(keys, wide = false) {
  let b = keys;
  if (wide) {
    b = new Uint8Array(keys.length * 2);
    keys.forEach((k, i) => { b[2 * i] = k >> 8; b[2 * i + 1] = k & 255; });
  } else if (!(keys instanceof Uint8Array)) {
    if (keys.some((k) => k > 255)) throw new Error('a key past 255 in a one-byte picture');
    b = Uint8Array.from(keys);
  }
  const z = await through(b, new CompressionStream('deflate'));
  let s = '';
  for (let i = 0; i < z.length; i += 0x8000) s += String.fromCharCode(...z.subarray(i, i + 0x8000));
  return btoa(s);
}
const inflate = (b64) => through(Uint8Array.from(atob(b64), (c) => c.charCodeAt(0)), new DecompressionStream('deflate'));
// n keys: a Uint8Array, or (wide) a Uint16Array.
export async function unpackPx(b64, n, wide = false) {
  const px = await inflate(b64);
  if (px.length !== n * (wide ? 2 : 1)) throw new Error('a picture of the wrong size');
  return wide ? Uint16Array.from({ length: n }, (_, i) => (px[2 * i] << 8) | px[2 * i + 1]) : px;
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
  if (!rec.author || typeof rec.author.name !== 'string' || rec.author.name.length > 80
    || (rec.author.account !== null && typeof rec.author.account !== 'string')) bad('author');
  if (rec.note !== undefined && (typeof rec.note !== 'string' || rec.note.length > CAPS.note)) bad('note');
  if (rec.licence !== null && !LICENCES.includes(rec.licence)) bad('licence');
  // An entry naming a record says which content it was: its hash too.
  if (!Array.isArray(rec.from) || rec.from.length > CAPS.from || rec.from.some((f) => !f || typeof f !== 'object'
    || (f.id !== undefined && (typeof f.id !== 'string' || !/^sha256-[0-9a-f]{64}$/.test(f.hash))))) bad('from');
  const b = rec.body;
  if (!b || typeof b !== 'object') bad('body');
  if (rec.type === 'piece') {
    if (b.w !== 128 || b.h !== 32) bad('size');
    if (!Array.isArray(b.frames) || !b.frames.length || b.frames.length > CAPS.frames) bad('frames');
    if (!Array.isArray(b.layers) || !b.layers.length || b.layers.length > CAPS.layers) bad('layers');
    // A layer that moves has a cel a frame; one held still, one cel.
    if (b.layers.some((l) => !Array.isArray(l.cels) || l.cels.length !== (l.moves ? b.frames.length : 1))) bad('cels');
    if (b.keys !== undefined && b.keys !== 1 && b.keys !== 2) bad('keys');
  }
  // Colours: keys 1 to 255 (65535 in a two-byte piece), each once, #rrggbb, each colour once.
  const cs = rec.type === 'piece' || rec.type === 'palette' ? b.colours : undefined;
  if (cs !== undefined) {
    const top = rec.type === 'piece' && b.keys !== 2 ? 255 : 65535, seen = new Set(), hexes = new Set();
    if (!Array.isArray(cs) || cs.length > CAPS.colours || cs.some((c) => !c || !Number.isInteger(c.k) || c.k < 1 || c.k > top
      || seen.size === seen.add(c.k).size || !/^#[0-9a-f]{6}$/.test(c.hex) || hexes.size === hexes.add(c.hex).size)) bad('colours');
  }
  if (JSON.stringify(rec).length > CAPS.bytes) bad('too big');
}

const head = (rec) => { const { body, ...h } = rec; return h; };

// --- IndexedDB ---
const req = (r) => new Promise((ok, no) => { r.onsuccess = () => ok(r.result); r.onerror = () => no(r.error); });
const done = (tx) => new Promise((ok, no) => { tx.oncomplete = ok; tx.onerror = tx.onabort = () => no(tx.error); });

export class LocalStore {
  // `closed` is called if a newer Studio (in another tab) needs the database:
  // this one lets go, and every call after that fails.
  static async open(name = 'opendmd-studio', closed = () => {}) {
    const r = indexedDB.open(name, VERSION);
    r.onupgradeneeded = (e) => {
      const db = r.result;
      // One step a version, from whatever this browser has.
      switch (e.oldVersion) {
        case 0:
          db.createObjectStore('records', { keyPath: 'id' }).createIndex('seq', 'seq');
          db.createObjectStore('revs', { keyPath: ['id', 'rev'] });
          db.createObjectStore('journal', { autoIncrement: true }).createIndex('id', 'id');
          db.createObjectStore('refs', { keyPath: ['to', 'from'] }).createIndex('from', 'from');
          db.createObjectStore('meta', { keyPath: 'k' });
        // falls through
        case 1: // the journal keyed by piece and step (S0 never wrote one)
          db.deleteObjectStore('journal');
          db.createObjectStore('journal', { keyPath: ['id', 'n'] });
        // falls through to the next version's step, when there is one
        default:
      }
    };
    const db = await new Promise((ok, no) => {
      r.onsuccess = () => ok(r.result);
      r.onerror = () => no(r.error);
      r.onblocked = () => no(new Error('an older Studio is open in another tab: close it, then reload'));
    });
    db.onversionchange = () => { db.close(); closed(); };
    return new LocalStore(db);
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

  // `at`: where the undo journal stands in what is written (LocalStore's own).
  async put(rec, { base = null, at } = {}) {
    const { ver, ...clean } = rec;
    const out = { ...clean, hash: await hashOf(rec.body), modified: now() };
    check(out);
    const tx = this.db.transaction(['records', 'refs', 'meta'], 'readwrite');
    const recs = tx.objectStore('records');
    const was = await req(recs.get(out.id));
    if (was && (was.gone || base !== String(was.seq))) {
      tx.abort();
      throw new Conflict(head(strip(was)));
    }
    out.seq = await this.#seq(tx);
    recs.put(out);
    this.#refs(tx, out);
    if (at !== undefined) this.#j(tx, out.id, { at, hash: out.hash });
    await done(tx);
    return head(strip(out));
  }

  async snapshot(id, label = '') {
    const tx = this.db.transaction(['records', 'revs', 'meta'], 'readwrite');
    const recs = tx.objectStore('records'), revs = tx.objectStore('revs');
    const cur = await req(recs.get(id));
    if (!cur || cur.gone) { tx.abort(); throw new Error('no such record'); }
    const last = cur.rev > 1 ? await req(revs.get([id, cur.rev - 1])) : null;
    if (last && last.hash === cur.hash && !label) { await done(tx); return head(strip(cur)); }
    const { seq, ...frozen } = cur;
    revs.put(label ? { ...frozen, label: label.slice(0, CAPS.name) } : frozen);
    const next = { ...cur, rev: cur.rev + 1, seq: await this.#seq(tx) };
    recs.put(next);
    await done(tx);
    return head(strip(next)); // the record as it goes on (the frozen one: get(id, rev - 1))
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
    await this.forget(id);
  }

  // --- the undo journal (this browser's alone: never in a record, a file or a sync) ---
  // A piece's steps are numbered from 1, back to back; `cur` is the last one
  // done (the ones after it can be redone), `at` the one the stored record
  // stands at and `hash` that record's (put writes both), so a reload opens
  // the record and replays (or unwinds) from `at` to `cur`, or, if the record
  // was written without it (another tab), drops the journal.
  async steps(id) {
    const tx = this.db.transaction(['journal', 'meta']);
    const list = await req(tx.objectStore('journal').getAll(IDBKeyRange.bound([id, 0], [id, Infinity])));
    const m = (await req(tx.objectStore('meta').get('j:' + id))) || {};
    return { list: list.map(({ id: _, ...s }) => s), cur: m.cur || 0, at: m.at || 0, hash: m.hash || null };
  }
  // A new step `n` (the ones from n on dropped first) and those below `low`
  // forgotten; or, with no step, only the cursor moved (an undo, a redo).
  async jot(id, { step, cur, low = 0 }) {
    const tx = this.db.transaction(['journal', 'meta'], 'readwrite'), j = tx.objectStore('journal');
    if (step) {
      j.delete(IDBKeyRange.bound([id, step.n], [id, Infinity]));
      j.put({ ...step, id });
      if (low > 0) j.delete(IDBKeyRange.bound([id, 0], [id, low], false, true));
    }
    this.#j(tx, id, { cur });
    await done(tx);
  }
  async forget(id) {
    const tx = this.db.transaction(['journal', 'meta'], 'readwrite');
    tx.objectStore('journal').delete(IDBKeyRange.bound([id, 0], [id, Infinity]));
    tx.objectStore('meta').delete('j:' + id);
    await done(tx);
  }
  #j(tx, id, set) {
    const meta = tx.objectStore('meta'), r = meta.get('j:' + id);
    r.onsuccess = () => meta.put({ ...(r.result || { cur: 0, at: 0 }), ...set, k: 'j:' + id });
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
    return { items: all.map((r) => head(strip(r))), next: all.length ? all[all.length - 1].seq : since };
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

// The stored copy without the store's own bookkeeping; its version as `ver` (opaque).
function strip(r) { const { seq, ...rest } = r; return seq === undefined ? rest : { ...rest, ver: String(seq) }; }
