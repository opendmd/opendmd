// Your Formulas, and how a playlist line or the look names a Formula
// (imported by js/formulas.js and Settings › Look; its own size line).
//
// On a device with a card a saved Formula lives on the panel, a file in
// /formulas/ (`<stem>.psi`, the stem made from its name at the first save
// and kept, so a rename never breaks a playlist); the player plays it from
// there, `/formulas/<stem>.psi?a=…`. Elsewhere (local mode, the demo, a
// device with no card) this browser keeps them, as before. A bundled
// Formula is named, `@formula:<stem>`; anything else goes inline,
// `@formula?src=…` (psikit `path`). Users see "Formulas", never files.

import { app } from './state.js';
import { store } from './util.js';
import { vals } from './psikit.js';

export const DIR = '/formulas';
let card = false, cache = [];

// This browser's own list (local mode, the demo, no card; and what is
// copied onto a card once).
const local = () => { const m = store.get('psi.mine', []); return Array.isArray(m) ? m.filter((x) => typeof x?.src == 'string') : []; };
// Values moved on the panel's copy (its text holds the defaults): by file.
const VK = 'psi.vals';
const valsOf = (file) => store.get(VK, {})[file] || {};
function keepVals(f) {
  const v = store.get(VK, {}), o = {};
  for (const k of 'abc') if (f[k] != null) o[k] = f[k];
  v[f.file] = o;
  store.set(VK, v);
}

// Whether saved Formulas live on the panel now (after `load`).
export const onCard = () => card;

// A Formula's name as its `name "…"` line gives it (no compile), else the stem.
const declared = (src) => /^\s*name\s+"([^"\n]*)"/m.exec(src)?.[1];
export const stemTitle = (s) => { const t = s.replace(/[-_]+/g, ' ').trim(); return t ? t[0].toUpperCase() + t.slice(1) : 'Formula'; };
// `My road!` -> `my-road`: [a-z0-9-], at most 40.
export const slug = (n) => n.toLowerCase().normalize('NFKD').replace(/[^a-z0-9]+/g, '-').replace(/^-+|-+$/g, '').slice(0, 40).replace(/-+$/, '') || 'formula';

// Your Formulas, read where they live: [{id, name, src, a, b, c, file?}].
export async function load() {
  once = once || Promise.resolve();
  card = false;
  if (app.device) {
    const sd = (app.status || await app.src.status().catch(() => null))?.sd;
    card = !!sd?.mounted && sd.readable !== false;
  }
  if (!card) return (cache = local());
  // (Asked first: a listing of a folder that isn't there is a 404.)
  const rows = (await app.src.stat(DIR).catch(() => null))?.exists ? await app.src.list(DIR).catch(() => []) : [];
  const out = [];
  for (const r of rows) {
    if (r.dir || !/\.psi$/i.test(r.name)) continue;
    const file = `${DIR}/${r.name}`, src = await app.src.readText(file).catch(() => null);
    if (typeof src != 'string') continue;
    out.push({ ...valsOf(file), id: file, file, src, name: declared(src) || stemTitle(r.name.replace(/\.psi$/i, '')) });
  }
  out.sort((a, b) => a.name.localeCompare(b.name));
  return (cache = out);
}

// The list as last loaded.
export const list = () => (card ? cache : local());
// Loaded once (Settings › Look; the Formulas view loads it when it opens).
let once = null;
export const loaded = () => (once ??= load().catch(() => (once = null, local())));

// Save `f` ({id?, name, src, a, b, c, file?}): on the panel a file (a new
// one named from its name, unique), else this browser's list. The record kept.
export async function put(f) {
  if (!card) {
    const l = local(), rec = { id: f.id ?? Date.now(), name: f.name, src: f.src, a: f.a, b: f.b, c: f.c }, k = l.findIndex((x) => x.id === rec.id);
    if (k < 0) l.unshift(rec); else l[k] = rec;
    if (!store.set('psi.mine', l)) throw new Error('this browser’s storage is full or blocked');
    return (cache = l, rec);
  }
  let file = f.file;
  if (!file) {
    const s = slug(f.name || ''), used = new Set(cache.map((x) => x.file));
    file = `${DIR}/${s}.psi`;
    for (let n = 2; used.has(file); n++) file = `${DIR}/${s}-${n}.psi`;
  }
  await app.src.writeFile(file, f.src);
  const rec = { id: file, file, name: f.name, src: f.src, a: f.a, b: f.b, c: f.c };
  keepVals(rec);
  const k = cache.findIndex((x) => x.file === file);
  if (k < 0) cache.unshift(rec); else cache[k] = rec;
  return rec;
}

// Delete one.
export async function drop(f) {
  if (!card) { store.set('psi.mine', local().filter((x) => x.id !== f.id)); cache = local(); return; }
  await app.src.deleteFile(f.file);
  cache = cache.filter((x) => x.file !== f.file);
}

// Once per card: the Formulas saved in this browser copied onto the panel
// (those whose text is already there skipped). How many were copied.
export async function copyUp() {
  const key = 'psi.copied.' + (app.cardKey || 'card');
  if (!card || store.get(key, false)) return 0;
  const have = new Set(cache.map((x) => x.src));
  let n = 0;
  for (const f of local().reverse()) {
    if (have.has(f.src)) continue;
    await put({ name: f.name || 'My formula', src: f.src, a: f.a, b: f.b, c: f.c });
    have.add(f.src);
    n++;
  }
  store.set(key, true);
  return n;
}

// The line a playlist or the look names `f` by: a bundled one by name, one
// on the panel by its file, with the values that differ from its defaults
// (`ctl`, pkg-psi controls()); else `inline`, its `@formula?src=…` path.
export function line(f, ctl, inline) {
  const v = vals(f, ctl), q = v ? '?' + v.slice(1) : '';
  if (f.stem) return `@formula:${f.stem}${q}`;
  if (card && f.file) return f.file + q;
  return inline;
}
