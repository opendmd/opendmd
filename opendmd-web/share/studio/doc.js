// The Studio's piece as a model, with no DOM (/studio/): its cels by layer
// and frame, what is shown (the composite), its own colours and their keys,
// a stroke in progress and the undo steps. studio.js draws it and keeps it;
// a check can drive it with no page at all.
//
// A piece's layers go bottom to top (layers[0] is the bottom); a pixel shows
// the topmost shown layer's key that is not 0, and 0 everywhere is off. A
// layer that moves has a cel a frame; one held still has one, shown on every
// frame. `hide: true` leaves a layer out of what is shown and of what is
// sent; a shown layer says nothing (an optional field at its default is
// absent: docs/studio-format.md).
//
// Cels stay packed (as stored) until they are first shown or drawn on, and
// a save packs again only the cels changed since their last packing.
//
// The undo journal (store.js steps(), jot()) is this browser's alone, so its
// shape is free to change; a journal of another shape is dropped when the
// piece opens. A step is one of:
//   {n, l, f, i, b, a, c?}  pixels of the cel of layer key `l` at frame key
//                           `f` (a held layer's one cel whatever `f`): at
//                           indices `i` (Int32Array), keys before `b` and
//                           after `a` (Uint16Array)
//   {n, c}                  only colours (Remove unused)
//   {n, body: {b, a}}       the whole piece before and after (a snapshot
//                           restored): bodies as stored, cels packed
//   {n, t: {b, a, z}}       its structure before and after (frames and
//                           layers added, removed, moved, retimed, renamed,
//                           shown or hidden, held or moving; plays): `b` and
//                           `a` are skeletons ({plays, frames, layers} with
//                           no cels), `z` the packed cels only one side has,
//                           by cel id ([z, two bytes a key]): a layer's key,
//                           and `/` and the frame's for a layer that moves
// `c` is a colour delta, {add: [{k, hex}], del: [{k, hex}]}: what the step
// put into the piece's colours and took out of them.
import { CAPS, packPx, unpackPx, memo, checkBody } from './store.js';

export const W = 128, H = 32, N = W * H;
// The journal's cap: 500 steps or about 8 MB, whichever comes first.
export const STEPS = 500, STEP_BYTES = 8 << 20;

export const hexOf = (n) => '#' + n.toString(16).padStart(6, '0');
export const toInt = (h) => parseInt(h.slice(1), 16);
const obj = (v) => !!v && typeof v === 'object' && !Array.isArray(v);
const k8 = () => [...crypto.getRandomValues(new Uint8Array(4))].map((x) => x.toString(16).padStart(2, '0')).join('');
const HEX = /^#[0-9a-f]{6}$/;

// A cel: packed as stored (`z`, `zw` true if two bytes a key) and, once
// needed, unpacked (`px`). `v` counts its changes, `zv` the change `z` is of.
class Cel {
  constructor(z, zw, px = null) { this.z = z; this.zw = zw; this.px = px; this.v = 0; this.zv = 0; }
}

// A step's size, near enough, for the journal's cap.
function size(s) {
  let n = 64;
  if (s.i) n += s.i.length * 8;
  if (s.c) n += (s.c.add.length + s.c.del.length) * 24;
  if (s.body) n += JSON.stringify(s.body).length * 2;
  if (s.t) n += JSON.stringify(s.t).length * 2;
  return n;
}
// A skeleton is a piece's structure: checked as a body would be, with empty cels.
const okSkel = (k) => obj(k) && Array.isArray(k.frames) && Array.isArray(k.layers) && k.layers.every(obj) && okBody({ w: W, h: H,
  ...(k.plays ? { plays: k.plays } : {}), frames: k.frames, layers: k.layers.map((l) => ({ ...l, cels: Array(l.moves ? k.frames.length : 1).fill('') })) });
const stable = (v) => JSON.stringify(v, (k, x) => (obj(x) ? Object.fromEntries(Object.keys(x).sort().map((y) => [y, x[y]])) : x));
const big = (px) => px.some((k) => k > 255);
const cid = (l, f) => (l.moves ? `${l.key}/${f.key}` : l.key);
const okColours = (l) => Array.isArray(l) && l.length <= CAPS.colours && l.every((c) => obj(c) && Number.isInteger(c.k) && c.k > 0 && c.k < 65536
  && typeof c.hex === 'string' && HEX.test(c.hex));
const okBody = (b) => { try { checkBody('piece', b); return true; } catch { return false; } };
// Give every frame and layer of a body a key, once (a body written before keys).
const keyed = (b) => {
  const seen = new Set(), key = (k) => { let x = typeof k === 'string' ? k : ''; while (!x || seen.has(x)) x = k8(); seen.add(x); return x; };
  return { ...b, frames: b.frames.map((f) => ({ ...f, key: key(f.key) })), layers: b.layers.map((l) => ({ ...l, key: key(l.key) })) };
};

export class Doc {
  // `body`: a piece's body as stored. `coloursOf(body)`: its colours, or its
  // palette's for a piece with none of its own (S0).
  constructor(body, coloursOf = (b) => b.colours || []) {
    this.coloursOf = coloursOf;
    this.key = 0;              // the colour picked
    this.pending = new Map();  // colours picked, not drawn with yet: key -> 0xRRGGBB
    this.stroke = null;        // a stroke in progress: index -> key before
    this.held = () => null;    // keys held elsewhere (a paste floating), kept and remapped with the rest
    this.steps = []; this.cur = 0; this.low = 1; // steps[i] is step low + i; the first `cur` are done
    this.L = 0; this.F = 0;    // the layer and frame drawn on
    this.full = false;         // the last colour asked for did not fit (CAPS.colours)
    this.stats = { usedBuilds: 0 };
    this.unz = new Map();      // packed cel -> its keys, for cels just packed or unpacked
    this.#take(body);
  }

  // The body's structure and colours taken in; its cels packed, not read yet.
  #take(body) {
    // Taken in the form it is written in: plays 0 as no plays, `keys: 1` as
    // none, the colours by key (an optional field at its default is absent).
    const { frames, layers, colours, keys, plays, ...rest } = keyed(body), wide = keys === 2;
    this.rest = plays ? { ...rest, plays } : rest;
    this.frames = frames;
    this.cols = new Map((this.coloursOf(body) || []).map((c) => [c.k, toInt(c.hex)]));
    // The colours first: a cel from the cache is cleaned against them too.
    this.layers = layers.map((l) => ({ ...l, cels: l.cels.map((z) => new Cel(z, wide, this.#cached(z))) }));
    this.L = Math.min(this.L, this.layers.length - 1);
    this.F = Math.min(this.F, this.frames.length - 1);
    memo(this.layers.reduce((n, l) => n + l.cels.length, 0) + 8);
  }
  #cached(z) { const p = this.unz.get(z); return p ? this.#clean(p.slice()) : null; }
  #remember(z, px) {
    if (this.unz.size > Math.max(64, 2 * this.layers.reduce((n, l) => n + l.cels.length, 0))) this.unz.delete(this.unz.keys().next().value);
    this.unz.set(z, px.slice());
  }

  // A stored piece, opened: frame 0 read; its journal `j` (store.steps())
  // replayed from where the record stands to where undo was, if it is this
  // record's and of this shape. `said`: 'none', 'kept', 'moved' (the record
  // needs saving) or 'dropped' (forget it).
  static async open(rec, coloursOf, j) {
    const fresh = async () => {
      const d = new Doc(rec.body, coloursOf);
      d.L = Math.max(0, d.layers.findLastIndex((l) => !l.hide)); // drawn on: the top shown layer (no layer controls yet)
      await d.load();
      return d;
    };
    let d = await fresh(), said = j?.list.length ? await d.#replay(j, rec) : 'none';
    if (said === 'damaged') { d = await fresh(); said = 'dropped'; } // a step that would not apply: from the record alone
    d.said = said;
    return d;
  }
  async #replay(j, rec) {
    if (!j.list.every((s) => this.valid(s))) return 'dropped';
    const lo = j.list[0].n, hi = lo + j.list.length - 1, ok = (n) => Number.isInteger(n) && n >= lo - 1 && n <= hi;
    if (!rec.hash || j.hash !== rec.hash || !j.list.every((s, n) => s.n === lo + n) || !ok(j.at) || !ok(j.cur)) return 'dropped';
    this.steps = j.list; this.low = lo; this.cur = j.at - lo + 1;
    try {
      // Every cel the journal holds reads, as the width it says, whether it comes into view or not.
      for (const st of j.list) {
        if (st.t) for (const [z, w] of Object.values(st.t.z)) await unpackPx(z, N, w);
        if (st.body) for (const b of [st.body.a, st.body.b]) for (const l of b.layers) for (const z of l.cels) await unpackPx(z, N, b.keys === 2);
      }
      while (this.cur < j.cur - lo + 1) await this.step(true);
      while (this.cur > j.cur - lo + 1) await this.step(false);
      // What the journal led to must be a piece too: a journal can lie.
      await this.load();
      checkBody('piece', await this.packed(this.snap()));
      if (!this.#sound()) throw new Error('a key with no colour');
    } catch {
      return 'damaged';
    }
    return j.at !== j.cur ? 'moved' : 'kept';
  }
  // Every key a cel read so far holds is one of the piece's colours.
  #sound() {
    for (const l of this.layers) for (const c of l.cels) if (c.px) for (const k of c.px) if (k && !this.cols.has(k)) return false;
    return true;
  }
  // A cel just read: a key the piece has no colour for is off (as it shows).
  #clean(px) {
    for (let i = 0; i < px.length; i++) if (px[i] && !this.cols.has(px[i])) px[i] = 0;
    return px;
  }
  // A journal step of this shape, naming cels this piece has.
  valid(s) {
    if (!obj(s) || !Number.isInteger(s.n)) return false;
    if (s.body !== undefined) return obj(s.body) && okBody(s.body.b) && okBody(s.body.a) && s.i === undefined && s.c === undefined;
    if (s.t !== undefined) return obj(s.t) && okSkel(s.t.b) && okSkel(s.t.a) && obj(s.t.z) && s.i === undefined && s.c === undefined
      && Object.values(s.t.z).every((e) => Array.isArray(e) && e.length === 2 && typeof e[0] === 'string' && typeof e[1] === 'boolean');
    if (s.c !== undefined && !(obj(s.c) && okColours(s.c.add) && okColours(s.c.del))) return false;
    if (s.i === undefined) return s.c !== undefined && s.l === undefined;
    // (Whether the piece has that cel is known only as the steps before it are applied.)
    return typeof s.l === 'string' && typeof s.f === 'string'
      && s.i instanceof Int32Array && s.b instanceof Uint16Array && s.a instanceof Uint16Array && s.i.length === s.b.length
      && s.i.length === s.a.length && s.i.every((x) => x >= 0 && x < N);
  }

  // --- cels ---
  celAt(l, f) { const L = this.layers[l]; return L.cels[L.moves ? f : 0]; }
  // The keys of every cel frame `f` shows, read now if they are not yet.
  async load(f = this.F) {
    await Promise.all(this.layers.map(async (l, n) => {
      const c = this.celAt(n, f);
      if (!c.px) { c.px = Uint16Array.from(await unpackPx(c.z, N, c.zw)); this.#clean(c.px); this.#remember(c.z, c.px); }
    }));
  }
  async loadAll() { for (let f = 0; f < this.frames.length; f++) await this.load(f); }
  get cel() { return this.celAt(this.L, this.F); }
  get px() { return this.cel.px; } // the keys drawn on
  // What frame `f` shows: each pixel the topmost shown layer's key that is
  // not 0; with `swap`, keys in place of the cel drawn on (a selection
  // lifted from it, floating). Not to be written to.
  composite(f = this.F, swap = null) {
    const own = this.celAt(this.L, this.F), cels = this.layers.filter((l) => !l.hide).map((l) => l.cels[l.moves ? f : 0])
      .map((c) => (swap && c === own ? swap : c.px));
    if (cels.length === 1) return cels[0];
    const out = new Uint16Array(N);
    for (const p of cels) for (let i = 0; i < N; i++) if (p[i]) out[i] = p[i];
    return out;
  }

  // --- colours: the piece's own ---
  colourList() { return [...this.cols].sort((a, b) => a[0] - b[0]).map(([k, c]) => ({ k, hex: hexOf(c) })); }
  keyOf(c) { for (const [k, v] of this.cols) if (v === c) return k; return 0; }
  rgbOf(k) { return this.cols.get(k) ?? this.pending.get(k) ?? 0; }
  // New colours' keys, one allocator for a whole paste (or a stamp, an
  // import): the lowest keys no colour has, none picked but not drawn yet
  // has, and no step in the journal names, undone or not (a step can be
  // redone while a paste floats, and its keys must still be its colours').
  // The set of keys in use is built once, here.
  freeKeys() {
    this.stats.usedBuilds++;
    const used = new Set([...this.cols.keys(), ...this.pending.keys()]);
    const all = (list) => { for (const c of list || []) used.add(c.k); };
    for (const s of this.steps) {
      if (s.c) { all(s.c.add); all(s.c.del); }
      if (s.body) { all(this.coloursOf(s.body.b)); all(this.coloursOf(s.body.a)); }
    }
    let k = 0;
    return () => { do k++; while (used.has(k)); return k < 65536 ? k : 0; };
  }
  // Colours (0xRRGGBB) to keys: the piece's key for one it has; else the
  // pending key for one already picked; else a new one, pending: it joins
  // the piece's colours in the step that first draws with it, so a pick is
  // never a step of its own. 0 for one past CAPS.colours (`full` says so).
  keysFor(list) {
    const have = new Map(), out = [];
    for (const [k, c] of this.cols) have.set(c, k);
    for (const [k, c] of this.pending) if (!have.has(c)) have.set(c, k);
    let next = null;
    this.full = false;
    for (const c of list) {
      let k = have.get(c);
      if (k === undefined) {
        next ??= this.freeKeys();
        k = this.cols.size + this.pending.size < CAPS.colours ? next() : 0;
        if (k) { this.pending.set(k, c); have.set(c, k); } else this.full = true;
      }
      out.push(k);
    }
    return out;
  }
  keyFor(c) { return this.keysFor([c])[0]; }
  // Pending colours no longer wanted (not `keep`, not held by a paste) let go.
  letGo(keep = 0) {
    const held = new Set(this.held() || []);
    for (const k of [...this.pending.keys()]) if (k !== keep && !held.has(k)) this.pending.delete(k);
  }
  // A colour picked (a swatch, the picker): its key, and picked.
  pickColour(c) {
    this.letGo(0);
    const k = this.keyFor(c);
    if (k) this.key = k;
    return k;
  }
  pick(k) { this.key = this.cols.has(k) || this.pending.has(k) ? k : 0; return this.key; }
  // After the piece's colours changed: a pending colour the piece now has is
  // dropped, and what used its key (the colour picked, a paste floating, a
  // stroke in progress) uses the piece's key; one whose key the piece now
  // gives another colour moves to a free key. So no colour is ever listed
  // twice, nor a key given two colours.
  reconcile() {
    const have = new Map(), map = new Map();
    for (const [k, c] of this.cols) have.set(c, k);
    let next = null;
    for (const [k, c] of [...this.pending]) {
      if (have.has(c)) { this.pending.delete(k); if (have.get(c) !== k) map.set(k, have.get(c)); } else if (this.cols.has(k)) {
        next ??= this.freeKeys();
        const to = next();
        this.pending.delete(k);
        if (to) { this.pending.set(to, c); map.set(k, to); }
      }
    }
    if (!map.size) return map;
    if (map.has(this.key)) this.key = map.get(this.key);
    const h = this.held();
    if (h) for (let i = 0; i < h.length; i++) if (map.has(h[i])) h[i] = map.get(h[i]);
    if (this.stroke) { const p = this.px; for (const i of this.stroke.keys()) if (map.has(p[i])) p[i] = map.get(p[i]); }
    return map;
  }

  // --- a stroke: pixels set on the cel drawn on, one step when it ends ---
  begin() { this.stroke = new Map(); }
  set(i, k) {
    const p = this.px;
    if (p[i] === k) return;
    if (!this.stroke.has(i)) this.stroke.set(i, p[i]);
    p[i] = k;
  }
  // What the stroke drew, undone (a shape dragged again, a stroke cancelled).
  revert() {
    if (!this.stroke) return;
    const p = this.px;
    for (const [i, b] of this.stroke) p[i] = b;
    this.stroke = null;
  }
  // The stroke made: one step, with the picked colours it is the first to
  // draw with. Returns what the journal is told (push), or null.
  end() {
    const s = this.stroke, p = this.px;
    this.stroke = null;
    if (!s || !s.size) return null;
    const i = Int32Array.from(s.keys()), a = Uint16Array.from(i, (j) => p[j]);
    const step = { l: this.layers[this.L].key, f: this.frames[this.F].key, i, b: Uint16Array.from(s.values()), a };
    const join = [...new Set(a)].filter((k) => this.pending.has(k));
    if (join.length) {
      step.c = { add: join.sort((x, y) => x - y).map((k) => ({ k, hex: hexOf(this.pending.get(k)) })), del: [] };
      for (const k of join) { this.cols.set(k, this.pending.get(k)); this.pending.delete(k); }
    }
    this.cel.v++;
    return this.push(step);
  }
  // The colours no cel uses, taken out: one step (or null: none to take).
  async unused() {
    await this.loadAll();
    const used = new Set();
    for (const l of this.layers) for (const c of l.cels) for (const k of c.px) used.add(k);
    const del = this.colourList().filter((c) => !used.has(c.k));
    if (!del.length) return null;
    for (const c of del) this.cols.delete(c.k);
    this.reconcile();
    return this.push({ c: { add: [], del } });
  }
  // A step done: the redo steps dropped, the oldest let go past the cap.
  // Returns {step, cur, low} for the journal (store.jot): `low`, if not 0,
  // the first step still kept.
  push(s) {
    if (new Set(this.cols.values()).size !== this.cols.size) throw new Error('a colour listed twice');
    this.steps.length = this.cur;
    s.n = this.low + this.cur;
    this.steps.push(s);
    this.cur++;
    let total = this.steps.reduce((t, x) => t + size(x), 0), cut = 0;
    while (this.steps.length > STEPS || (total > STEP_BYTES && this.steps.length > 1)) {
      total -= size(this.steps.shift());
      this.cur--; this.low++; cut = this.low;
    }
    return { step: s, cur: this.low + this.cur - 1, low: cut };
  }

  // --- undo and redo ---
  // A step undone (fwd false) or redone: null when done at once, else a
  // promise (a cel to read first, a snapshot's body).
  step(fwd) {
    if (fwd ? this.cur >= this.steps.length : !this.cur) return null;
    const s = this.steps[fwd ? this.cur : this.cur - 1];
    const go = () => { this.#apply(s, fwd); this.cur += fwd ? 1 : -1; return this.#shown(); };
    const wait = this.#needs(s, fwd);
    if (!wait) return go();
    return wait.then(go);
  }
  // null if every cel the frame drawn on shows is read, else the promise of reading them.
  #shown() { return this.layers.every((l, n) => this.celAt(n, this.F).px) ? null : this.load(); }
  // What has to be read before step `s` can be applied: null if nothing.
  #needs(s, fwd) {
    if (s.body) {
      const b = fwd ? s.body.a : s.body.b, f = Math.min(this.F, b.frames.length - 1);
      const zs = b.layers.map((l) => l.cels[l.moves ? f : 0]);
      return zs.every((z) => this.unz.has(z)) ? null : Promise.all(zs.map(async (z) => {
        if (!this.unz.has(z)) this.#remember(z, Uint16Array.from(await unpackPx(z, N, b.keys === 2)));
      }));
    }
    if (!s.i) return null;
    const l = this.layers.findIndex((x) => x.key === s.l), f = this.frames.findIndex((x) => x.key === s.f);
    if (l < 0 || f < 0) return Promise.reject(new Error('a step for a cel this piece does not have'));
    this.L = l; this.F = f; // undone or redone where it was drawn: in view
    return this.celAt(l, f).px ? null : this.load(f);
  }
  #apply(s, fwd) {
    if (s.t) {
      // A structure step leads from the piece as it stands, or it is not this piece's.
      if (stable(this.skel()) !== stable(fwd ? s.t.b : s.t.a)) throw new Error('a step for another piece');
      this.#reshape(fwd ? s.t.a : s.t.b, s.t.z);
      return;
    }
    if (s.body) {
      const was = this.cols;
      this.#take(fwd ? s.body.a : s.body.b);
      this.#kept(was);
      this.reconcile();
      return;
    }
    if (s.i) {
      const c = this.celAt(this.layers.findIndex((x) => x.key === s.l), this.frames.findIndex((x) => x.key === s.f)), v = fwd ? s.a : s.b;
      for (let n = 0; n < s.i.length; n++) c.px[s.i[n]] = v[n];
      c.v++;
    }
    if (s.c) {
      const was = new Map(this.cols);
      for (const x of fwd ? s.c.del : s.c.add) this.cols.delete(x.k);
      for (const x of fwd ? s.c.add : s.c.del) this.cols.set(x.k, toInt(x.hex));
      this.#kept(was);
      this.reconcile();
    }
  }
  // A colour taken out of the piece by an undo or a redo stays pending if
  // something still holds its key: the colour picked, a paste floating, a
  // stroke in progress.
  #kept(was) {
    const held = new Set(this.held() || []);
    held.add(this.key);
    if (this.stroke) for (const i of this.stroke.keys()) held.add(this.px[i]);
    for (const [k, c] of was) if (k && held.has(k) && !this.cols.has(k)) this.pending.set(k, c);
  }

  // --- structure (frames.js makes the changes; here they are applied and undone) ---
  skel() {
    return { plays: this.rest.plays || 0, frames: this.frames.map((f) => ({ ...f })), layers: this.layers.map(({ cels, ...l }) => ({ ...l })) };
  }
  // The piece given the structure `a` (a skeleton), as one step. `made`: the
  // keys of each cel `a` has and the piece has not (by cel id); the cels it
  // has and `a` has not are kept, packed, in the step. Returns what the
  // journal is told.
  async shape(a, made = {}) {
    if (!okSkel(a)) throw new Error('not a piece');
    const b = this.skel(), wide = this.colourList().some((c) => c.k > 255), z = {}, have = this.#cels(), want = new Set();
    for (const l of a.layers) for (const f of l.moves ? a.frames : [null]) want.add(f ? cid(l, f) : l.key);
    for (const id of want) {
      if (have.has(id)) continue;
      if (!made[id]) throw new Error(`no cel ${id}`);
      const w = wide || big(made[id]), zz = await packPx(made[id], w);
      this.#remember(zz, made[id]);
      z[id] = [zz, w];
    }
    for (const [id, c] of have) {
      if (want.has(id)) continue;
      // (Packed as wide as its keys need: a 256th colour may have been drawn since it was packed.)
      if (c.v !== c.zv && c.px) { const w = c.zw || big(c.px); c.z = await packPx(c.px, w); c.zw = w; c.zv = c.v; }
      z[id] = [c.z, c.zw];
    }
    const s = { t: { b, a, z } };
    this.#reshape(a, z);
    await this.load();
    return this.push(s);
  }
  #cels() {
    const have = new Map();
    for (const l of this.layers) l.cels.forEach((c, i) => have.set(l.moves ? `${l.key}/${this.frames[i].key}` : l.key, c));
    return have;
  }
  #reshape(k, z) {
    const have = this.#cels(), lk = this.layers[this.L]?.key, fk = this.frames[this.F]?.key;
    const frames = k.frames.map((f) => ({ ...f }));
    const layers = k.layers.map((l) => ({ ...l, cels: (l.moves ? frames : [null]).map((f) => {
      const id = f ? cid(l, f) : l.key, c = have.get(id), e = z[id];
      if (c) return c;
      if (!e) throw new Error('a step for a cel this piece does not have');
      return new Cel(e[0], e[1], this.#cached(e[0]));
    }) }));
    this.frames = frames;
    this.layers = layers;
    const { plays, ...rest } = this.rest;
    this.rest = k.plays ? { ...rest, plays: k.plays } : rest;
    const l = layers.findIndex((x) => x.key === lk), f = frames.findIndex((x) => x.key === fk);
    this.L = l >= 0 ? l : Math.min(this.L, layers.length - 1);
    this.F = f >= 0 ? f : Math.min(this.F, frames.length - 1);
  }

  // --- keeping it ---
  // What a save or a download takes, now (the drawing may go on while it is packed).
  snap() {
    return {
      frames: this.frames.map((f) => ({ ...f })), colours: this.colourList(), at: this.low + this.cur - 1,
      layers: this.layers.map(({ cels, ...l }) => ({ ...l, cels: cels.map((c) => ({ c, v: c.v, px: c.px && c.v !== c.zv ? c.px.slice() : null })) })),
    };
  }
  // A snap as a body, the cels packed again only where they changed (or
  // where the piece went from one byte a key to two, or back).
  async packed(sn, more = {}) {
    const wide = sn.colours.some((c) => c.k > 255);
    const layers = [];
    for (const { cels, ...l } of sn.layers) {
      const out = [];
      for (const { c, v, px } of cels) {
        if (!px && c.zw === wide) { out.push(c.z); continue; }
        const keys = px || (c.v === v && c.px) || this.#clean(Uint16Array.from(await unpackPx(c.z, N, c.zw)));
        const z = await packPx(keys, wide);
        if (c.v === v) { c.z = z; c.zw = wide; c.zv = v; this.#remember(z, keys); }
        out.push(z);
      }
      layers.push({ ...l, cels: out });
    }
    return { ...this.rest, ...more, ...(wide ? { keys: 2 } : {}), colours: sn.colours, frames: sn.frames, layers };
  }
  // A snapshot's body put in place of the piece, as one step: returns what
  // the journal is told.
  async restore(body, more = {}) {
    const b = await this.packed(this.snap(), more), was = this.cols;
    const s = { body: { b, a: keyed(body) } }; // keyed once, here: a redo finds the same keys
    body = s.body.a;
    await this.#needs(s, true);
    this.#take(body);
    this.#kept(was);
    this.reconcile();
    await this.load();
    return this.push(s);
  }
}
