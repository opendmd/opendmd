// What names a GIF by its path, kept true when GIFs move, are renamed or
// are deleted (imported by js/details.js and the Library's bulk bar):
// relink([{from, to | null}]) moves (POST /api/files/move: the device moves
// the prepared copy and renames the Smooth motion and failure lines
// itself) or deletes each GIF in turn, stopping at the first that fails,
// then, for those done: every playlist's lines and favourites (one read and
// at most one write each, however many GIFs), the user's tags
// (/sets/.tags), Smooth motion's list for a deleted GIF, the smart
// playlists, the editor's draft, the selection, thumbnails and folder
// listings, and a folder left empty goes.

import { app, setValue } from './state.js';
import { fileFor } from './overrides.js';
import { savePlaylist } from './gifs.js';
import { emit, base, dir, enc } from './util.js';

const lc = (s) => s.toLowerCase();

// Every playlist line, and every playlist's overrides, through
// `map(path) -> path | null` (null drops it).
async function rewrite(map) {
  for (const pl of await app.playlists(true).catch(() => [])) {
    const text = await app.text(pl.path, true).catch(() => null);
    if (text) {
      let hit = false;
      const out = text.split('\n').map((l) => {
        const p = l.trim();
        if (!p) return l;
        const n = map(p);
        if (n === p) return l;
        hit = true;
        return n;
      }).filter((l) => l !== null);
      if (hit) await savePlaylist(pl.path, out.join('\n'));
    }
    const list = app.device ? (await app.src.overrides(setValue(pl.path)).catch(() => null))?.overrides || [] : [];
    const next = list.map((o) => ({ ...o, path: map(o.path) })).filter((o) => o.path);
    if (next.length !== list.length || next.some((o, i) => o.path !== list[i].path)) {
      const meta = app.wasm.renderOverrides(next);
      if (meta) await app.src.writeFile(fileFor(pl.path), meta); else await app.src.deleteFile(fileFor(pl.path));
    }
  }
}

// Smooth motion's list (js/smooth.js) loses deleted GIFs' lines (a move's
// the device renames). Read only when it is there.
async function unsmooth(gone) {
  const text = await app.sideText('/sets/.smooth');
  if (!text) return;
  const all = text.split('\n').map((l) => l.trim()).filter(Boolean), keep = all.filter((l) => !gone.has(lc(l)));
  if (keep.length !== all.length) await app.writeSide('/sets/.smooth', keep.map((p) => p + '\n').join(''));
}

// A folder left empty goes (its folder, and its prepared copies' folder).
async function dropEmpty(path) {
  const d = dir(path);
  if (!/^\/gifs\/[^/]+$/.test(d) || (await app.src.list(d).catch(() => [{}])).length) return;
  try {
    await app.src.deleteFile(d);
    const c = base(d);
    if ((await app.src.list('/cache').catch(() => [])).some((f) => f.dir && f.name === c)) await app.src.json('DELETE', `/api/files${enc('/cache/' + c)}?recursive=1`);
  } catch { /* an older device keeps the empty folder */ }
}

// -> {done: [{from, to}], failed: {from, to, error} | null}. `step(n)`
// after each one done.
export async function relink(pairs, step) {
  const done = [];
  let failed = null;
  for (const p of pairs) {
    try {
      if (p.to) await app.src.move(p.from, p.to);
      else {
        await app.src.deleteFile(p.from);
        // (The device's DELETE takes the prepared copy; this is the cleanup on one that did not.)
        await app.src.forget(p.from).catch(() => {});
      }
      done.push(p);
      step?.(done.length);
    } catch (error) { failed = { ...p, error }; break; }
  }
  if (!done.length) return { done, failed };
  const map = new Map(done.map((p) => [lc(p.from), p.to]));
  const to = (p) => (map.has(lc(p)) ? map.get(lc(p)) : p);
  await rewrite(to);
  const t = app.tags || (app.setNames?.has('.tags') && (await (await import('./tags.js')).tags()));
  if (t) await t.relink(done);
  const gone = new Set(done.filter((p) => !p.to).map((p) => lc(p.from)));
  if (gone.size) await unsmooth(gone);
  // What the app holds by path follows too.
  const d = app.draft;
  d.paths = d.paths.map(to).filter(Boolean);
  for (const p of done) if (app.selection.delete(p.from) && p.to) app.selection.add(p.to);
  const th = await import('./thumbs.js').catch(() => null), dirs = new Map();
  for (const p of done) {
    th?.forget(p.from);
    if (p.to) th?.forget(p.to);
    dirs.set(dir(p.from), p.from);
    if (p.to) dirs.set(dir(p.to), p.to);
  }
  for (const [d, p] of dirs) { await dropEmpty(p); emit('gifs', d.split('/')[2]); }
  import('./smart.js').then((m) => m.refreshAll()).catch(() => {});
  emit('draft');
  return { done, failed };
}
