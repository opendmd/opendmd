// App-wide state shared by the views: the source, the device's last status
// and config, the manifests (fetched once, on demand), the playlists, the
// Library selection and the playlist being edited.
//
// Users see GIFs and playlists. A GIF's prepared copy (a pre-rendered file
// the player plays with less memory) is a cache: the device plays it when it
// exists, previews fetch it first and fall back to the GIF, and nothing here
// ever shows its path.

import * as wasm from '../pkg/opendmd_web.js';
import { emit, toast, base } from './util.js';

// Root manifests with a name people know them by. Anything else shows its
// file name.
const LABELS = { 'filelist.txt': 'All files', 'filelist.hall': 'Halloween', 'filelist.xmas': 'Christmas' };
// Manifests of prepared-copy paths from before copies were found
// automatically (filelist.odm.txt, and the like). The device ignores them;
// so does the UI.
const OBSOLETE = /^filelist\.odm\./i;

export function playlistLabel(name) {
  const n = base(name);
  if (LABELS[n]) return LABELS[n];
  return /^filelist\./.test(n) ? n : n.replace(/\.txt$/, '');
}

// The `set` value (config, schedule, GET /api/overrides) for a playlist's
// card path: "" for /filelist.txt, the file name for another root playlist,
// the name for /sets/<name>.txt.
export function setValue(path) {
  const n = base(path);
  if (!path.startsWith('/sets/')) return n === 'filelist.txt' ? '' : n;
  return n.replace(/\.txt$/, '');
}

// A `set` value's card path (the inverse).
export function setPath(set) {
  return !set ? '/filelist.txt' : set.startsWith('filelist.') ? '/' + set : `/sets/${set}.txt`;
}

export const app = {
  wasm,
  src: null,
  status: null,
  config: null,
  selection: new Set(),
  draft: { name: '', paths: [], path: '' },
  texts: new Map(),

  get device() { return this.src?.kind === 'device'; },

  // A manifest's text, fetched once per session (filelist.txt is 450 KB).
  async text(path, fresh = false) {
    if (fresh || !this.texts.has(path)) this.texts.set(path, await this.src.readText(path));
    return this.texts.get(path);
  },

  // The playlists: the card's root manifests (filelist.txt, .hall, .xmas —
  // not their .idx or temp files, not the obsolete one) and /sets/*.txt.
  // /sets is only listed when the root shows it: a card that never had a
  // playlist has no directory to 404 on. Each is {path, value, label, root}:
  // `value` is what `set` holds to play it ("" for All files, the default).
  async playlists() {
    let root = [], sets = [];
    try { root = await this.src.list('/'); } catch { /* a pick of gifs/ alone */ }
    if (root.some((f) => f.dir && f.name === 'sets')) {
      try { sets = await this.src.list('/sets'); } catch { /* gone meanwhile */ }
    }
    const roots = root.filter((f) => !f.dir && /^filelist\./.test(f.name) && !OBSOLETE.test(f.name) && !/\.(idx|part|tmp|bak|meta)$/.test(f.name))
      .map((f) => f.name).sort((a, b) => (a === 'filelist.txt' ? -1 : b === 'filelist.txt' ? 1 : a.localeCompare(b)));
    const names = sets.filter((f) => !f.dir && /\.txt$/.test(f.name)).map((f) => f.name.replace(/\.txt$/, '')).sort();
    return [
      ...roots.map((n) => ({ path: '/' + n, value: n === 'filelist.txt' ? '' : n, label: playlistLabel(n), root: true })),
      ...names.map((n) => ({ path: `/sets/${n}.txt`, value: n, label: n, root: false })),
    ];
  },

  async refreshStatus() {
    if (!this.device) return null;
    this.status = await this.src.status();
    emit('status', this.status);
    return this.status;
  },
  async refreshConfig() {
    if (!this.device) return null;
    this.config = await this.src.config();
    emit('config', this.config);
    return this.config;
  },
  // The configured `set`: '' = All files (/filelist.txt), 'filelist.*' =
  // that root manifest, anything else = /sets/<set>.txt.
  get activeSet() { return this.config?.set ?? ''; },
  // The card path of the playlist in play: status `manifest`, else the config.
  get activePath() {
    const m = this.status?.manifest?.replace(/^\/sdcard/, '');
    if (m) return m;
    return setPath(this.activeSet);
  },
  get building() { return this.status?.index_state === 'building'; },

  // A file's bytes for a preview: its prepared copy when there is one (the
  // same frames; on the device a ~15-file directory instead of, say, the
  // 3,781-file gifs/Consoles, whose scan the panel would feel), else the
  // file itself. A missing copy costs one 404.
  async previewBytes(path) {
    const copy = wasm.odmPathFor(path);
    if (copy) {
      try { return await this.src.readBytes(copy); } catch (e) { if (e.status !== 404) throw e; }
    }
    return this.src.readBytes(path);
  },

  // POST /api/play: the panel shows `path` now, then resumes its schedule.
  async play(path) {
    try {
      await this.src.play(path);
      toast(`on the panel: ${base(path)}`, 'ok');
      return true;
    } catch (e) {
      toast(e.status === 404 ? `${path}: not on the card` : e.status === 415 ? `${path}: not a GIF` : 'play: ' + e.message, 'bad');
      return false;
    }
  },

  go(view) { location.hash = '#' + view; },
};
