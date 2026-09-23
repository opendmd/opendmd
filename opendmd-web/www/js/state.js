// App-wide state shared by the views: the source, the device's last status
// and config, the manifests (fetched once, on demand), which GIFs already
// have an .odm, the Library selection and the set being edited.

import * as wasm from '../pkg/opendmd_web.js';
import { emit, lines, toast } from './util.js';

export const app = {
  wasm,
  src: null,
  status: null,
  config: null,
  // Card-absolute .odm paths known to exist, from filelist.odm.txt. null
  // until loaded; an empty set if the source does not serve that file.
  odm: null,
  odmSource: '',
  selection: new Set(),
  draft: { name: '', paths: [] },
  pendingConvert: [],
  texts: new Map(),

  get device() { return this.src?.kind === 'device'; },

  // A manifest's text, fetched once per session (filelist.txt is 450 KB).
  async text(path, fresh = false) {
    if (fresh || !this.texts.has(path)) this.texts.set(path, await this.src.readText(path));
    return this.texts.get(path);
  },

  // The "has .odm" marker. One fetch of filelist.odm.txt, never a listing of
  // the 256 bucket directories per category.
  async odmIndex() {
    if (this.odm) return this.odm;
    // Look in the root listing first (a dozen entries) so a card without
    // the file costs no 404.
    let text = null;
    try {
      if ((await this.src.list('/')).some((f) => f.name === 'filelist.odm.txt')) text = await this.text('/filelist.odm.txt');
    } catch { /* treat as absent */ }
    this.odm = new Set(text ? lines(text).filter((p) => p.toLowerCase().endsWith('.odm')) : []);
    this.odmSource = text == null ? 'none' : 'filelist.odm.txt';
    return this.odm;
  },
  // The root listing, its manifests (filelist.txt, .odm.txt, .hall, .xmas —
  // not their .idx or temp files) and /sets/*.txt. /sets is only listed when
  // the root shows it: a card that never had a set has no directory to 404 on.
  async sets() {
    let root = [], sets = [];
    try { root = await this.src.list('/'); } catch { /* a pick of gifs/ alone */ }
    if (root.some((f) => f.dir && f.name === 'sets')) {
      try { sets = await this.src.list('/sets'); } catch { /* gone meanwhile */ }
    }
    const manifests = root.filter((f) => !f.dir && /^filelist\./.test(f.name) && !/\.(idx|part|tmp|bak)$/.test(f.name)).map((f) => f.name).sort();
    return { root, manifests, sets: sets.filter((f) => !f.dir && /\.txt$/.test(f.name)).map((f) => f.name.replace(/\.txt$/, '')).sort() };
  },
  hasOdm(gifPath) {
    const p = wasm.odmPathFor(gifPath);
    return !!(p && this.odm?.has(p));
  },
  addOdm(path) { (this.odm ??= new Set()).add(path); emit('odm'); },

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
  // The configured `set`: '' = the card's default manifest, 'filelist.*' =
  // that root manifest, anything else = /sets/<set>.txt.
  get activeSet() { return this.config?.set ?? ''; },
  // What `set: ""` plays: the status's resolved manifest while the default is
  // in play, else the firmware's rule (filelist.odm.txt if the card has it).
  defaultManifest(names) {
    if (this.device && !this.activeSet && this.status?.manifest) return this.status.manifest.replace(/^.*\//, '');
    return names.includes('filelist.odm.txt') ? 'filelist.odm.txt' : 'filelist.txt';
  },
  get building() { return this.status?.index_state === 'building'; },
  // POST /api/play: the panel shows `path` now, then resumes its schedule.
  async play(path) {
    try {
      await this.src.play(path);
      toast(`on the panel: ${path}`, 'ok');
      return true;
    } catch (e) {
      toast(e.status === 404 ? `${path}: not on the card` : e.status === 415 ? `${path}: not a GIF or .odm` : 'play: ' + e.message, 'bad');
      return false;
    }
  },

  go(view) { location.hash = '#' + view; },
};
