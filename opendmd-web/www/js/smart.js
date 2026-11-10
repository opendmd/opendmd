// Smart playlists: a playlist built from a search (the Library's words:
// `platform:md rating>=4`, opendmd_core::query) and kept in step with it.
// The display never reads a search: the playlist is an ordinary
// /sets/<name>.txt, written here whenever the search finds something else
// (saved from the editor, a change of tags or stars, new GIFs, Refresh),
// so the schedule, Follow and a card read on a PC see a playlist like any
// other. Its search lives in /sets/.smart, one line per playlist
// (`path\tq=…\tfrom=…\tat=…`: the words, the playlist it takes GIFs from —
// none for every GIF on the card —, when it was last written).

import { app, listOr } from './state.js';
import { art } from './artmod.js';
import { savePlaylist } from './gifs.js';
import { emit, lines, isSys, byNice, gone } from './util.js';

export const FILE = '/sets/.smart';
const lc = (s) => s.toLowerCase();

// The art module (js/artmod.js: the search and the lists are parsed there),
// once something needs it: a card with no smart playlist never loads it.
let A = null;
const mod = async () => (A ??= await art());

let map = null, mapKey;
// path (lowercased) -> {path, q, from, at}
export async function load(fresh) {
  if (!map || fresh || mapKey !== app.cardKey) {
    mapKey = app.cardKey;
    const text = await app.sideText(FILE);
    map = new Map(!text ? [] : (await mod()).parseSidecar(text).map(({ path, f }) => [lc(path), { path, q: f.q || '', from: f.from || '', at: +f.at || 0 }]));
  }
  return map;
}
export const get = (p) => map?.get(lc(p)) || null;

const write = async () => app.writeSide(FILE, (await mod()).renderSidecar([...map.values()].map((s) => ({ path: s.path, f: { q: s.q, from: s.from, at: String(s.at) } }))));

// Each change is made to the file as it is now (another browser may have
// written it since).
export async function put(path, q, from = '') {
  await load(true);
  map.set(lc(path), { path, q, from, at: Date.now() / 1000 | 0 });
  await write();
}
export async function drop(path) {
  await load(true);
  if (map.delete(lc(path))) await write();
}
// The editor saving a smart playlist changed by hand (`save` writes it):
// it stops being smart, and a refresh already under way does not write
// the search's list back over it.
const byHand = new Set();
export async function handSave(path, save) {
  byHand.add(lc(path));
  try { await save(); await drop(path); } finally { byHand.delete(lc(path)); }
}
// A playlist renamed (`move`) or duplicated: its search goes with it.
export async function copy(from, to, move) {
  await load(true);
  const s = map.get(lc(from));
  if (!s) return;
  if (move) map.delete(lc(from));
  map.set(lc(to), { ...s, path: to });
  await write();
}

// Every GIF on the card, a path a line, folder by folder as the Library
// lists them (each folder once a session: tree.js); `fresh` lists them again.
let listed = false;
export async function everyGif(fresh) {
  const { listCat, forget } = await import('./tree.js');
  const dirs = (await listOr('/gifs')).filter((d) => d.dir).map((d) => d.name).sort(byNice);
  const out = [];
  for (const c of [...dirs.filter((d) => !isSys(d)), ...dirs.filter(isSys)]) {
    if (fresh) forget(c);
    const l = await listCat(c);
    // (A folder that could not be read: no playlist written without its GIFs.)
    if (!l) throw new Error('a folder couldn’t be read: try again');
    for (const it of l) out.push(it.path);
  }
  listed = true;
  return out.join('\n');
}

// The GIFs a search finds in `from` (a playlist's path; '' every GIF).
// Throws {message} for words it cannot read.
export async function find(q, from, fresh) {
  const text = from ? await app.text(from, fresh) : await everyGif(fresh);
  if (text == null) throw new Error('that playlist isn’t on the card');
  const t = await (await import('./tags.js')).tags();
  const s = new (await mod()).Search(q);
  try { return [...s.run(text, t.book)]; } finally { s.free(); }
}

// Write the playlist again from its search: {added, removed}, or null when
// it is not a smart playlist. The file is only written when its lines change.
export async function refresh(path, fresh) {
  await load();
  const s = get(path);
  if (!s) return null;
  const paths = await find(s.q, s.from, fresh), text = await app.text(path, true);
  // Still this search's playlist: not deleted, saved by hand or given
  // another search meanwhile (asked again after each wait).
  const mine = () => { const c = get(path); return !byHand.has(lc(path)) && !!c && c.q === s.q && c.from === s.from; };
  if (text == null || !mine()) return null;
  const old = lines(text).filter((p) => !gone(p));
  const was = new Set(old.map(lc)), now = new Set(paths.map(lc));
  const diff = { added: paths.filter((p) => !was.has(lc(p))).length, removed: old.filter((p) => !now.has(lc(p))).length };
  const changed = paths.join('\n') !== old.join('\n');
  if (changed) {
    await savePlaylist(path, paths.join('\n') + '\n');
    // Open in the editor with nothing changed by hand: it shows the new lines.
    const d = app.draft;
    if (d.path === path && d.paths.join('\n') === d.saved) { d.paths = paths; d.saved = d.smartPaths = paths.join('\n'); emit('draft'); }
  }
  // (When it was last written, or looked at by hand: the row's "updated".)
  // (The line as the map has it now: a load since has replaced `s`.)
  if ((changed || fresh) && mine()) { get(path).at = Date.now() / 1000 | 0; await write(); }
  return diff;
}

// No refresh running (a playlist about to be deleted or renamed must not be
// written back by one).
export const idle = () => (running || Promise.resolve()).catch(() => {});

// Every smart playlist `which(search)` picks (a Search: `usesTags`,
// `keyed`), one at a time. Resolves to how many changed. `cheap`: only
// those whose GIFs this page has read already (opening Playlists lists
// nothing more of the card for them).
let running = null;
export async function refreshAll(which = () => true, fresh, cheap) {
  if (running) await running.catch(() => {});
  return (running = (async () => {
    let n = 0;
    for (const s of [...(await load()).values()]) {
      let w;
      try { w = new (await mod()).Search(s.q); } catch { continue; }
      const pick = which(w) && !(cheap && (s.from ? !app.texts.has(s.from) : !listed));
      w.free();
      if (!pick) continue;
      const d = await refresh(s.path, fresh).catch(() => null);
      if (d && (d.added || d.removed)) n++;
    }
    return n;
  })());
}

// What a smart playlist's line says, for a row or the editor's header:
// "Mega Drive · ★ 4+ · updated 10 Nov" (once `load` or `find` has run).
export async function summary(s) {
  let d = '';
  try { const w = new (await mod()).Search(s.q); d = w.describe(); w.free(); } catch (e) { d = e.message || String(e); }
  return d + (s.at ? ' · updated ' + new Date(s.at * 1000).toLocaleDateString(undefined, { day: 'numeric', month: 'short' }) : '');
}
