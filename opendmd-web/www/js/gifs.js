// Adding a GIF to the card and to a playlist: shared by the Library, its
// dialogs (js/details.js) and Create Art (js/studio.js), which so need not
// load the Library's tree and thumbnails.

import { app, playlistLabel } from './state.js';
import { prepareBytes } from './prepare.js';
import { emit, toast, lines, isFlag } from './util.js';

export const authorOf = (e) => e.flags.find((f) => !isFlag(f)) || '';

// The next name to try when one is taken (a names list gone stale): the
// GIF's number up by one, else a `_2` before `.gif`.
export const nextName = (n) => {
  const m = /^(.*_)(\d+)((?:_[^_]*)?\.gif)$/i.exec(n);
  return m ? m[1] + String(+m[2] + 1).padStart(m[2].length, '0') + m[3] : n.replace(/(\.gif)$/i, '_2$1');
};

// Adds the GIF under `name`, or the next free name if a GIF has it (the
// device refuses to replace one: 409): the path it was written to.
export async function addOne(category, name, bytes, warn, prep = prepareBytes) {
  let path;
  for (let k = 0; ; k++) {
    path = `/gifs/${category}/${name}`;
    try { await app.src.writeFile(path, bytes, { createNew: true }); break; } catch (e) {
      if (e.status !== 409 || k >= 20) throw e;
      name = nextName(name);
    }
  }
  import('./thumbs.js').then((m) => m.forget(path)); // its thumbnail, if one was kept
  try { await prep(path, bytes); } catch (e) {
    warn?.(`${name}: added, but it could not be prepared (${e.message || e}); it will play as it is`);
    try { await app.src.forget(path); } catch { /* nothing to forget */ }
  }
  emit('gifs', category);
  return path;
}

export async function savePlaylist(path, text) {
  if (path.startsWith('/sets/')) await app.src.saveSet(path.slice(6).replace(/\.txt$/, ''), text);
  else await app.src.writeFile(path, text);
  app.texts.set(path, text);
  if (path === app.activePath) await app.refreshStatus().catch(() => {});
}

export async function appendTo(path, paths) {
  const name = playlistLabel(path);
  try {
    const text = await app.text(path, true);
    const have = new Set(lines(text || ''));
    const add = paths.filter((p) => !have.has(p));
    if (!add.length) { toast(`already in “${name}”`); return; }
    await savePlaylist(path, (text && !text.endsWith('\n') ? text + '\n' : text || '') + add.join('\n') + '\n');
    toast(`${add.length} added to “${name}”`, 'ok');
  } catch (e) { toast(`adding to “${name}”: ${e.message}`, 'bad'); }
}

export async function offerPlaylist(paths) {
  const n = paths.length;
  if (confirm(`Add ${n} new GIF${n > 1 ? 's' : ''} to the playlist “${playlistLabel(app.activePath)}”?`)) await appendTo(app.activePath, paths);
}

