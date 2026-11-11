// The Studio's built-in palette (CC0): a read-only record with a fixed id
// (store.js builtinId), so a piece can point at it by id. Den 16 is
// OpenDMD's own; the starter palettes come with the starter content
// (studio-starter/starter.dmdpack, served beside the page), records of this
// same shape, read-only too: a starter record is one whose id the starter
// pack lists as a member. Key 0 is never a colour here: it is "off", the LED
// dark (and, in a stamp, see-through), so every palette's keys start at 1.
// Palettes are swatch sets to draw from: a colour picked from one joins the
// piece's own colours (body.colours).
import { builtinId, hashOf, check, FORMAT } from './store.js';

const T = '2026-11-11T00:00:00Z';
function pal(n, name, source, list) {
  const colours = list.map(([hex, nm], i) => ({ k: i + 1, hex: '#' + hex, ...(nm ? { name: nm } : {}) }));
  return { type: 'palette', id: builtinId('palette', n), rev: 1, hash: '', name, tags: [],
    author: { name: 'OpenDMD', account: null }, licence: 'CC0-1.0', from: [], created: T, modified: T,
    body: { colours, source } };
}

const DEN = pal(1, 'Den 16', 'OpenDMD', [
  ['0a0e2a', 'Night'], ['161d52', 'Navy'], ['2e3a8c', 'Indigo'], ['6447c8', 'Violet'], ['33d1ff', 'Cyan'],
  ['8a4f1d', 'Bark'], ['14532d', 'Pine'], ['2f9e4f', 'Moss'], ['f7a531', 'Amber'], ['e8601c', 'Ember'],
  ['c92a2a', 'Barn'], ['ff4f8b', 'Pink'], ['f1ebdd', 'Cream'], ['7c7f8a', 'Ash'], ['ffffff', 'White']]);

export const STARTER = [DEN];
export const DEFAULT = DEN.id;
// Their hashes, as any record's (store.js hashOf); site-smoke pins Den 16's.
export const ready = Promise.all(STARTER.map(async (p) => { p.hash = await hashOf(p.body); }));
const BY = new Map(STARTER.map((p) => [p.id, p]));
export const builtin = (id) => BY.get(id) || null;

// The starter pack's palettes, fetched once (after the page is up): each
// checked and its hash worked out again; one that is not right is left out.
let members = new Set(), starting = null;
export const isStarter = (id) => members.has(id);
export function starter() {
  starting ??= fetch(new URL('starter.dmdpack', import.meta.url)).then((r) => {
    if (!r.ok) throw new Error(`starter palettes: HTTP ${r.status}`);
    return r.json();
  }).then(async (b) => {
    const pack = b.format === FORMAT && b.records.find((x) => x.id === b.root && x.type === 'pack');
    if (!pack) throw new Error('starter palettes: not a pack');
    members = new Set(pack.body.members.map((m) => m.id));
    const out = [];
    for (const p of b.records) {
      if (p.type !== 'palette' || !members.has(p.id)) continue;
      try { check(p); if (await hashOf(p.body) === p.hash) { out.push(p); BY.set(p.id, p); } } catch { /* left out */ }
    }
    return out;
  }).catch((e) => { starting = null; throw e; });
  return starting;
}
