// App-wide state shared by the views: the source, the device's last status
// and config, the manifests (fetched once, on demand), the playlists, the
// Library selection and the playlist being edited.
//
// Users see GIFs and playlists. A GIF's prepared copy (a pre-rendered file
// the player plays with less memory) is a cache: the device plays it when it
// exists, previews fetch it first and fall back to the GIF, and nothing here
// ever shows its path.

import wasm from './web.js';
import { emit, toast, base } from './util.js';

// Root manifests with a name people know them by. Anything else shows its
// file name.
const LABELS = { 'filelist.txt': 'All files', 'filelist.hall': 'Halloween', 'filelist.xmas': 'Christmas' };
// Manifests of prepared-copy paths from before copies were found
// automatically (filelist.odm.txt, and the like). The device ignores them;
// so does the UI.
const OBSOLETE = /^filelist\.odm\./i;

// What the Formulas view says on a player built without them (the
// firmware's own words for a Formula sent there, its 501).
export const NO_FORMULAS = 'Formulas aren’t in this firmware build';

export function playlistLabel(name) {
  const n = base(name);
  if (LABELS[n]) return LABELS[n];
  // A set named on the card with underscores reads as words (`Sprites_and_Clock`).
  return /^filelist\./.test(n) ? n : n.replace(/\.txt$/, '').replace(/(?!^)_/g, ' ');
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

// A directory's entries; a 404 (a picked folder without it, a directory
// gone) is []. A busy device's 503 or a dropped request is retried inside
// `list` (source.js `queued`, the one retry layer, ~45 s): an empty list
// would show Playlists or a Library folder with nothing in it until a reload.
export async function listOr(dir) {
  try { return await app.src.list(dir); } catch (e) {
    if (e && e.status === 404) return [];
    throw e;
  }
}

// The playlist being edited has changes that are not on the card.
export const dirty = (d = app.draft) => d.paths.join('\n') !== (d.saved ?? '');

function cardKey(a) {
  if (!a.device) return a.src ? 'l' + (a.src.label || '') : null;
  const sd = a.status?.sd;
  return sd?.card_id ? 'i' + sd.card_id : sd?.capacity_bytes ? 'c' + sd.capacity_bytes : null;
}

export const app = {
  wasm,
  src: null,
  status: null,
  stAsked: 0, // refreshStatus: requests made / the latest one applied
  stApplied: 0,
  config: null,
  selection: new Set(),
  // The playlist being edited: `saved` is its lines as opened or last saved.
  draft: { name: '', paths: [], path: '', saved: '' },
  texts: new Map(),
  // A page of the site's own (its Create Art page, share/create/), not the
  // web app: no source, no card; Game art's index is beside it.
  site: false,

  get device() { return this.src?.kind === 'device'; },
  // A player built without Formulas (`/api/status` `features`, the optional
  // parts compiled in, has no "formulas"; firmware from before the field
  // has them): the Formulas view is a note, no Formula look, and a
  // playlist's Formula lines are marked skipped (the player skips them).
  get noFormulas() { const f = this.status?.features; return !!this.device && Array.isArray(f) && !f.includes('formulas'); },

  // Which card the things kept in this browser belong to (thumbnails, the
  // folder counts, files the prepare job could not do): the status's
  // `sd.card_id` on a device (another card, or this one formatted again,
  // is another key; firmware before it: the card's size), the folder's
  // name locally. null until the status names a card.
  get cardKey() { return cardKey(this); },

  // A manifest's text, fetched once per session (filelist.txt is 450 KB);
  // callers at the same moment share the one fetch.
  tq: new Map(),
  async text(path, fresh = false) {
    if (fresh || !this.texts.has(path)) {
      let q = !fresh && this.tq.get(path);
      if (!q) this.tq.set(path, q = this.src.readText(path).finally(() => this.tq.delete(path)));
      this.texts.set(path, await q);
    }
    return this.texts.get(path);
  },

  // The playlists: the card's root manifests (filelist.txt, .hall, .xmas —
  // not their .idx or temp files, not the obsolete one) and /sets/*.txt.
  // /sets is only listed when the root shows it: a card that never had a
  // playlist has no directory to 404 on. Each is {path, value, label, root}:
  // `value` is what `set` holds to play it ("" for All files, the default).
  // Callers at the same moment (a config change refreshes several views)
  // share one listing: the device has four sockets.
  // Callers share a listing already in flight; `fresh` asks for one begun now
  // (a caller that has just changed the card must not take a listing that
  // began before the change: it would bring a deleted playlist back).
  playlists(fresh) {
    if (fresh) this.pl = null;
    const mine = (this.pl ??= this.listPlaylists().finally(() => { if (this.pl === mine) this.pl = null; }));
    return mine.then((l) => [...l]);
  },
  async listPlaylists() {
    const root = await listOr('/');
    const sets = root.some((f) => f.dir && f.name === 'sets') ? await listOr('/sets') : [];
    const roots = root.filter((f) => !f.dir && /^filelist\./.test(f.name) && !OBSOLETE.test(f.name) && !/\.(idx|part|tmp|bak|meta)$/.test(f.name))
      .map((f) => f.name).sort((a, b) => (a === 'filelist.txt' ? -1 : b === 'filelist.txt' ? 1 : a.localeCompare(b)));
    const names = sets.filter((f) => !f.dir && /\.txt$/.test(f.name)).map((f) => f.name.replace(/\.txt$/, '')).sort();
    return [
      ...roots.map((n) => ({ path: '/' + n, value: n === 'filelist.txt' ? '' : n, label: playlistLabel(n), root: true })),
      ...names.map((n) => ({ path: `/sets/${n}.txt`, value: n, label: playlistLabel(n), root: false })),
    ];
  },

  // (A device, or the demo's player: js/demo.js.)
  async refreshStatus(recent) {
    // (Counted as a read even with no player: a watcher waiting on it must
    // not be due again at once.)
    fullAt = performance.now();
    if (!this.src?.config) return null;
    const was = this.cardKey, seq = ++this.stAsked;
    const st = await this.src.status(recent);
    // Replies can land out of order (the Now poll, a return to the tab and a
    // view's own refresh are each a request): one asked before the latest
    // applied is its caller's to read, not the app's current status. Taken
    // as current it would bring the old card back after a swap.
    if (seq < this.stApplied) return st;
    this.stApplied = seq;
    this.status = st;
    // Another card (swapped with the power off, or formatted): what was
    // read from the old one goes, and the views list the new one.
    const now = this.cardKey;
    if (was && now && was !== now) {
      this.texts.clear();
      this.selection.clear();
      emit('card', now);
    }
    emit('status', this.status);
    return this.status;
  },
  async refreshConfig() {
    if (!this.src?.config) return null;
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
    // A virtual file (a title card, the power-on effect: `@…`) is drawn by
    // the preview's own engine from its path; there is nothing to fetch.
    if (path.startsWith('@')) return new Uint8Array();
    const copy = wasm.cachePathFor(path);
    if (copy) {
      try { return await this.src.readBytes(copy); } catch (e) { if (e.status !== 404) throw e; }
    }
    return this.src.readBytes(path);
  },

  // POST /api/play: the panel shows `path` now, then resumes its schedule.
  async play(path) {
    try {
      await this.src.play(path);
      toast(`on the panel: ${path.startsWith('@formula?') ? 'Formula' : base(path)}`, 'ok');
      return true;
    } catch (e) {
      toast(e.status === 501 ? e.message.replace(/^501: /, '') : e.status === 404 ? `${path}: not on the card` : e.status === 415 ? `${path}: not a GIF` : 'play: ' + e.message, 'bad');
      return false;
    }
  },

  go(view) { location.hash = '#' + view; },

  // The one status scheduler. A view asks for the status every `ms` while it
  // is up (`watch`; again to change the rate) and lets go (`unwatch`). One
  // timer runs, for whichever is due first, so the fastest asked sets the
  // pace; one read is in flight at a time; none while the page is hidden.
  // Without `read` a watcher wants the full status, and any full read
  // (`refreshStatus`, whoever asked) counts for it; with one (Now: its
  // slim reads and clock samples) the read is its own. `ms` Infinity: only
  // when `soon` asks (in `wait` ms).
  watch(key, ms, read) { const w = watchers.get(key); watchers.set(key, { at: -1e9, ...w, ms, read }); pace(); },
  unwatch(key) { watchers.delete(key); pace(); },
  soon(key, wait = 0) { const w = watchers.get(key); if (w) { w.soon = performance.now() + wait; pace(); } },
};

const watchers = new Map();
let fullAt = -1e9, paceT = 0, reading = false;
const dueAt = (w) => Math.min(w.soon ?? Infinity, (w.read ? w.at : fullAt) + w.ms);
function pace() {
  clearTimeout(paceT);
  if (reading || document.hidden) return;
  const t = performance.now();
  let next = null, at = Infinity;
  for (const w of watchers.values()) {
    if (!w.read && w.soon <= fullAt) w.soon = null; // (a full read since did it)
    if (dueAt(w) < at) [next, at] = [w, dueAt(w)];
  }
  if (at > t) { if (at < Infinity) paceT = setTimeout(pace, at - t); return; }
  reading = true;
  next.at = t;
  next.soon = null;
  (next.read || (() => app.refreshStatus()))().catch(() => {}).finally(() => { reading = false; pace(); });
}

// Back to this tab (a card may have been swapped meanwhile): one full
// status first, then the watchers' reads as they fall due.
document.addEventListener('visibilitychange', () => {
  if (!document.hidden && app.device) watchers.set('back', { at: -1e9, ms: Infinity, soon: performance.now() });
  pace();
});
