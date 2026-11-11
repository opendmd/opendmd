// The Studio's built-in palette (CC0): a read-only record with a fixed id
// (store.js builtinId), so a piece can point at it by id. Den 16 is
// OpenDMD's own; the starter palettes come with the starter content
// (studio-starter/), as records of this same shape. Key 0 is never a colour here: it is
// "off", the LED dark (and, in a stamp, see-through), so every palette's
// keys start at 1. Editing one (later) makes a copy whose `from` names it.
import { builtinId } from './store.js';

const T = '2026-11-11T00:00:00Z';
function pal(n, name, source, list) {
  const colours = list.map(([hex, nm], i) => ({ k: i + 1, hex: '#' + hex, ...(nm ? { name: nm } : {}) }));
  return { type: 'palette', id: builtinId('palette', n), rev: 1, hash: '', name, tags: ['starter'],
    author: { name: 'OpenDMD', account: null }, licence: 'CC0-1.0', from: [], created: T, modified: T,
    body: { colours, source } };
}

const DEN = pal(1, 'Den 16', 'OpenDMD', [
  ['0a0e2a', 'Night'], ['161d52', 'Navy'], ['2e3a8c', 'Indigo'], ['6447c8', 'Violet'], ['33d1ff', 'Cyan'],
  ['8a4f1d', 'Bark'], ['14532d', 'Pine'], ['2f9e4f', 'Moss'], ['f7a531', 'Amber'], ['e8601c', 'Ember'],
  ['c92a2a', 'Barn'], ['ff4f8b', 'Pink'], ['f1ebdd', 'Cream'], ['7c7f8a', 'Ash'], ['ffffff', 'White']]);

export const STARTER = [DEN];
export const DEFAULT = DEN.id;
const BY = new Map(STARTER.map((p) => [p.id, p]));
export const builtin = (id) => BY.get(id) || null;
