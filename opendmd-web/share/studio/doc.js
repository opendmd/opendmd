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
// `c` is a colour delta, {add: [{k, hex}], del: [{k, hex}]}: what the step
// put into the piece's colours and took out of them.
import { CAPS, packPx, unpackPx, memo } from './store.js';

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
  return n;
}
const okColours = (l) => Array.isArray(l) && l.length <= CAPS.colours && l.every((c) => obj(c) && Number.isInteger(c.k) && c.k > 0 && c.k < 65536
  && typeof c.hex === 'string' && HEX.test(c.hex));
const okBody = (b) => obj(b) && Array.isArray(b.frames) && b.frames.length > 0 && b.frames.length <= CAPS.frames
  && b.frames.every(obj) && Array.isArray(b.layers) && b.layers.length > 0 && b.layers.length <= CAPS.layers
  && b.layers.every((l) => obj(l) && Array.isArray(l.cels) && l.cels.length === (l.moves ? b.frames.length : 1) && l.cels.every((c) => typeof c === 'string'))
  && (b.colours === undefined || okColours(b.colours));

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
    const { frames, layers, colours, keys, ...rest } = body, wide = keys === 2, seen = new Set();
    const key = (k) => { let x = typeof k === 'string' ? k : ''; while (!x || seen.has(x)) x = k8(); seen.add(x); return x; };
    this.rest = rest;
    this.frames = frames.map((f) => ({ ...f, key: key(f.key) }));
    this.layers = layers.map((l) => ({ ...l, key: key(l.key), cels: l.cels.map((z) => new Cel(z, wide, this.#cached(z))) }));
    this.cols = new Map(this.coloursOf(body).map((c) => [c.k, toInt(c.hex)]));
    this.L = Math.min(this.L, this.layers.length - 1);
    this.F = Math.min(this.F, this.frames.length - 1);
    memo(this.layers.reduce((n, l) => n + l.cels.length, 0) + 8);
  }
  #cached(z) { const p = this.unz.get(z); return p ? p.slice() : null; }
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
      while (this.cur < j.cur - lo + 1) await this.step(true);
      while (this.cur > j.cur - lo + 1) await this.step(false);
    } catch {
      return 'damaged';
    }
    return j.at !== j.cur ? 'moved' : 'kept';
  }
  // A journal step of this shape, naming cels this piece has.
  valid(s) {
    if (!obj(s) || !Number.isInteger(s.n)) return false;
    if (s.body !== undefined) return obj(s.body) && okBody(s.body.b) && okBody(s.body.a) && s.i === undefined && s.c === undefined;
    if (s.c !== undefined && !(obj(s.c) && okColours(s.c.add) && okColours(s.c.del))) return false;
    if (s.i === undefined) return s.c !== undefined && s.l === undefined;
    return typeof s.l === 'string' && typeof s.f === 'string' && this.layers.some((l) => l.key === s.l) && this.frames.some((f) => f.key === s.f)
      && s.i instanceof Int32Array && s.b instanceof Uint16Array && s.a instanceof Uint16Array && s.i.length === s.b.length
      && s.i.length === s.a.length && s.i.every((x) => x >= 0 && x < N);
  }

  // --- cels ---
  celAt(l, f) { const L = this.layers[l]; return L.cels[L.moves ? f : 0]; }
  // The keys of every cel frame `f` shows, read now if they are not yet.
  async load(f = this.F) {
    await Promise.all(this.layers.map(async (l, n) => {
      const c = this.celAt(n, f);
      if (!c.px) { c.px = Uint16Array.from(await unpackPx(c.z, N, c.zw)); this.#remember(c.z, c.px); }
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
    const go = () => { this.#apply(s, fwd); this.cur += fwd ? 1 : -1; };
    const wait = this.#needs(s, fwd);
    if (!wait) { go(); return null; }
    return wait.then(go);
  }
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
    return this.celAt(l, f).px ? null : this.load(f);
  }
  #apply(s, fwd) {
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
  // The colour picked, undone out of the piece, stays picked.
  #kept(was) {
    const k = this.key;
    if (k && !this.cols.has(k) && was.has(k)) this.pending.set(k, was.get(k));
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
        const keys = px || (c.v === v && c.px) || Uint16Array.from(await unpackPx(c.z, N, c.zw));
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
    const s = { body: { b, a: body } };
    await this.#needs(s, true);
    this.#take(body);
    this.#kept(was);
    this.reconcile();
    await this.load();
    return this.push(s);
  }
}
