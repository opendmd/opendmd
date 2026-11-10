// Adding a GIF to the card and to a playlist: shared by the Library, its
// dialogs (js/details.js) and Create Art (js/studio.js), which so need not
// load the Library's tree and thumbnails.

import { app, playlistLabel, dirty } from './state.js';
import { prepareBytes } from './prepare.js';
import { actions } from './overrides.js';
import { h, emit, toast, lines, isFlag, gone } from './util.js';

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

// One write (or delete) of a playlist at a time, per path (as the card's
// FAT names them: case-blind): `fn` starts once the one before it has
// answered, so the last asked is what the card keeps.
const turns = new Map();
function inTurn(path, fn) {
  const k = path.toLowerCase(), run = (turns.get(k) || Promise.resolve()).then(fn);
  const tail = run.catch(() => {});
  turns.set(k, tail);
  tail.then(() => { if (turns.get(k) === tail) turns.delete(k); });
  return run;
}

export async function savePlaylist(path, text) {
  await inTurn(path, async () => {
    if (path.startsWith('/sets/')) await app.src.saveSet(path.slice(6).replace(/\.txt$/, ''), text);
    else await app.src.writeFile(path, text);
    app.texts.set(path, text);
  });
  if (path === app.activePath) await app.refreshStatus().catch(() => {});
}
export const deletePlaylist = (path) => inTurn(path, () => app.src.deleteFile(path));
// Read after any write of it still under way.
export const readPlaylist = (path) => inTurn(path, () => app.text(path, true));

// (The card's FAT names files case-insensitively.)
const lc = (p) => p.toLowerCase();

// The GIFs not yet in it, at the end; true once done (or nothing to add).
export async function appendTo(path, paths) {
  const name = playlistLabel(path);
  try {
    const text = await app.text(path, true);
    const have = new Set(lines(text || '').map(lc));
    const add = paths.filter((p) => !have.has(lc(p)) && have.add(lc(p))), n = paths.length - add.length;
    if (!add.length) { toast(`already in “${name}”`); return true; }
    const body = (text && !text.endsWith('\n') ? text + '\n' : text || '') + add.join('\n') + '\n';
    await savePlaylist(path, body);
    // Open in the editor: it gains them too (its Save would put the old list
    // back), and what it would put back on Cancel is what the card has now.
    const d = app.draft;
    if (d.path === path) {
      const mine = new Set(d.paths.map(lc));
      d.paths.push(...add.filter((p) => !mine.has(lc(p))));
      d.saved = lines(body).filter((p) => !gone(p)).join('\n');
      emit('draft');
    }
    toast(`${add.length.toLocaleString()} added to “${name}”${n ? ` · ${n.toLocaleString()} already in it` : ''}`, 'ok');
    return true;
  } catch (e) { toast(`adding to “${name}”: ${e.message}`, 'bad'); return false; }
}

// A new playlist of these GIFs, in the editor with its name to fill in.
export function newDraft(paths) {
  app.draft = { name: '', paths: [...paths], path: '', saved: '', focus: true };
  emit('draft');
  app.go('sets/edit');
}

// Add these GIFs to which playlist? Under `btn`: the one playing first (one
// of yours), then the rest of yours with their counts (the one open in the
// editor marked), then New playlist…. Not the card's own (All files,
// Halloween, Christmas): the editor saves those as a new playlist.
// `done(fresh)` after a playlist took them (`fresh`: a new one, in the editor).
export async function targetMenu(btn, paths, done = () => {}) {
  const lists = (await app.playlists().catch(() => [])).filter((p) => !p.root);
  const playing = app.device ? lists.find((p) => p.path === app.activePath) : null;
  const item = (p, text) => {
    const n = h('small.muted');
    app.text(p.path).then((t) => { n.textContent = ' ' + lines(t || '').length.toLocaleString(); }, () => {});
    return [{ 'data-path': p.path }, [text, n, p.path === app.draft.path && h('span.tag', 'Editing')], async () => { if (await appendTo(p.path, paths)) done(); }];
  };
  const k = paths.length;
  actions(btn, 'pl-target', [playing && item(playing, `Playing: ${playing.label}`), ...lists.filter((p) => p !== playing).map((p) => item(p, p.label)),
    ['pl-new', 'New playlist…', () => { newDraft(paths); done(true); }]], `Add ${k === 1 ? 'it' : k.toLocaleString() + ' GIFs'} to`);
}

export async function offerPlaylist(paths) {
  const n = paths.length;
  if (confirm(`Add ${n} new GIF${n > 1 ? 's' : ''} to the playlist “${playlistLabel(app.activePath)}”?`)) await appendTo(app.activePath, paths);
}

