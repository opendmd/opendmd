// The two places a card can be: a device on the network (DeviceSource, the
// firmware's JSON API and file routes on the same origin) or a folder the
// user picked (LocalSource, File objects). Views only ever talk to this
// interface:
//
//   list(dir) -> [{name, dir, size}]      readText(path) -> string | null
//   readBytes(path) -> Uint8Array          writeFile(path, data), deleteFile(path)
//   saveSet(name, text)                    device-only: status(), config(), ...
//
// Paths are card-absolute (`/gifs/Consoles/X.gif`). Nothing is cached here;
// a file's bytes live only as long as the caller holds them.

import { enc, base } from './util.js';

export class HttpError extends Error {
  constructor(status, message) { super(message); this.status = status; }
}

async function check(res) {
  if (res.ok) return res;
  let msg = res.statusText;
  try { msg = (await res.json()).error || msg; } catch { /* not JSON */ }
  throw new HttpError(res.status, `${res.status}: ${msg}`);
}

export class DeviceSource {
  kind = 'device';
  canWrite = true;

  async req(method, url, body, timeout = 90000) {
    const ctl = new AbortController();
    const t = setTimeout(() => ctl.abort(), timeout);
    try {
      return await check(await fetch(url, { method, body, signal: ctl.signal, cache: 'no-store' }));
    } catch (e) {
      if (e.name === 'AbortError') throw new HttpError(0, `${method} ${url}: timed out`);
      throw e;
    } finally { clearTimeout(t); }
  }
  json(method, url, body, timeout) { return this.req(method, url, body, timeout).then((r) => r.json()); }

  status() { return this.json('GET', '/api/status', undefined, 8000); }
  config() { return this.json('GET', '/api/config'); }
  // A playlist change (set, platforms, shuffle) rebuilds the index on the
  // device, which takes seconds; the firmware waits up to 60 s for it.
  putConfig(patch) { return this.json('PUT', '/api/config', JSON.stringify(patch)); }
  skip() { return this.json('POST', '/api/skip'); }
  pause() { return this.json('POST', '/api/pause'); }
  reindex() { return this.json('POST', '/api/reindex'); }
  reboot() { return this.json('POST', '/api/reboot'); }
  wifiScan() { return this.json('GET', '/api/wifi/scan', undefined, 20000); }
  // One file onto the panel now: 404 if it does not open, 415 if not a GIF.
  play(path) { return this.json('POST', '/api/play', JSON.stringify({ path })); }
  // {path, exists, size, is_dir} from one `stat`, i.e. one scan of the parent
  // directory on the device: ~20 ms in a bucket, ~600 ms in gifs/Consoles.
  stat(path) { return this.json('GET', '/api/stat?path=' + encodeURIComponent(path)); }
  // The playback cache: {total, prepared, state: "ready"|"counting"}; the
  // card paths of every prepared copy; and forgetting a GIF's copy.
  cache() { return this.json('GET', '/api/cache', undefined, 15000); }
  async cacheList() { return (await this.json('GET', '/api/cache/list', undefined, 180000)).paths; }
  forget(path) { return this.json('POST', '/api/cache/forget', JSON.stringify({ path })); }
  // Rename a GIF: its prepared copy, Smooth motion line and failure line
  // follow on the device; playlists and overrides are the caller's.
  move(from, to) { return this.json('POST', '/api/files/move', JSON.stringify({ from, to })); }
  // GIFs that can never be prepared ([{path, reason}], the device's
  // /cache/.failed), and recording one. Not in every firmware yet.
  cacheFailed() { return this.json('GET', '/api/cache/failed'); }
  cacheFail(path, reason) { return this.json('POST', '/api/cache/failed', JSON.stringify({ path, reason })); }
  // A playlist's per-GIF overrides, parsed: {set, playlist, overrides: [...]}.
  overrides(set) { return this.json('GET', '/api/overrides?set=' + encodeURIComponent(set)); }
  // The schedule (rules + night mode), whole; and suspending it for a while.
  schedule() { return this.json('GET', '/api/schedule'); }
  putSchedule(s) { return this.json('PUT', '/api/schedule', JSON.stringify(s)); }
  wake(minutes) { return this.json('POST', '/api/wake', JSON.stringify({ minutes })); }

  async list(dir) {
    const r = await this.json('GET', '/api/files?dir=' + encodeURIComponent(dir));
    return r.entries;
  }
  async readText(path) {
    try { return await (await this.req('GET', enc(path))).text(); } catch (e) {
      if (e.status === 404) return null;
      throw e;
    }
  }
  async readBytes(path) { return new Uint8Array(await (await this.req('GET', enc(path))).arrayBuffer()); }
  // A manifest path (`/filelist.*`, `/sets/*`) is installed by the device's
  // main task (the `.idx` goes first, then a reindex if it is active).
  writeFile(path, data) { return this.json('PUT', '/api/files' + enc(path), data); }
  deleteFile(path) { return this.json('DELETE', '/api/files' + enc(path)); }
  saveSet(name, text) { return this.json('PUT', '/api/sets/' + encodeURIComponent(name), text); }
}

// Card-absolute path for a picked file. A folder pick gives
// `webkitRelativePath` = `<picked folder>/gifs/Consoles/X.gif`; if the folder
// picked was one of the card's top-level folders itself, keep it.
const ROOTS = new Set(['gifs', 'cache', 'sets', 'www', 'logos']);
function localPath(f, cardPath) {
  const rel = f.webkitRelativePath;
  if (!rel) return cardPath(f.name);
  const seg = rel.split('/');
  return '/' + (ROOTS.has(seg[0]) ? seg : seg.slice(1)).join('/');
}

export class LocalSource {
  kind = 'local';

  constructor(fileList, cardPath) {
    this.files = new Map();
    this.dirs = new Map([['/', new Map()]]);
    this.label = '';
    this.handle = null; // a writable directory handle, once the user grants one
    for (const f of fileList) {
      if (f.name.startsWith('.')) continue;
      if (!this.label && f.webkitRelativePath) this.label = f.webkitRelativePath.split('/')[0];
      this.#add(localPath(f, cardPath), f);
    }
    this.canWrite = true; // via a directory handle, or as downloads
  }

  #add(path, file) {
    this.files.set(path, file);
    const seg = path.split('/').filter(Boolean);
    let d = '/';
    for (let i = 0; i < seg.length; i++) {
      const last = i === seg.length - 1;
      const m = this.dirs.get(d);
      m.set(seg[i], { name: seg[i], dir: !last, size: last ? file.size : null });
      d = (d === '/' ? '' : d) + '/' + seg[i];
      if (!last && !this.dirs.has(d)) this.dirs.set(d, new Map());
    }
  }

  get count() { return this.files.size; }

  async list(dir) {
    const m = this.dirs.get(dir.replace(/\/+$/, '') || '/');
    if (!m) throw new HttpError(404, `${dir}: not in the picked folder`);
    return [...m.values()];
  }
  async readText(path) { const f = this.files.get(path); return f ? f.text() : null; }
  async stat(path) {
    const f = this.files.get(path), d = !f && this.dirs.has(path);
    return { path, exists: !!f || d, size: f ? f.size : null, is_dir: d };
  }
  async readBytes(path) {
    const f = this.files.get(path);
    if (!f) throw new HttpError(404, `${path}: not in the picked folder`);
    return new Uint8Array(await f.arrayBuffer());
  }

  // Writes go to a directory the user grants (File System Access API,
  // Chromium) — the card root — or, failing that, become downloads.
  async writeFile(path, data) {
    const blob = data instanceof Blob ? data : new Blob([data]);
    if (!this.handle && window.showDirectoryPicker) {
      try { this.handle = await window.showDirectoryPicker({ mode: 'readwrite' }); } catch { /* declined */ }
    }
    if (this.handle) {
      const seg = path.split('/').filter(Boolean);
      let d = this.handle;
      for (const s of seg.slice(0, -1)) d = await d.getDirectoryHandle(s, { create: true });
      const w = await (await d.getFileHandle(seg.at(-1), { create: true })).createWritable();
      await w.write(blob);
      await w.close();
    } else {
      const a = document.createElement('a');
      a.href = URL.createObjectURL(blob);
      a.download = base(path);
      a.click();
      setTimeout(() => URL.revokeObjectURL(a.href), 5000);
    }
    this.#add(path, new File([blob], base(path)));
    return { ok: true, path, bytes: blob.size };
  }
  async deleteFile(path) {
    if (!this.handle) throw new HttpError(405, 'the picked folder is read-only here');
    const seg = path.split('/').filter(Boolean);
    let d = this.handle;
    for (const s of seg.slice(0, -1)) d = await d.getDirectoryHandle(s);
    await d.removeEntry(seg.at(-1));
    this.files.delete(path);
    this.dirs.get('/' + seg.slice(0, -1).join('/'))?.delete(seg.at(-1));
    return { ok: true };
  }
  saveSet(name, text) { return this.writeFile(`/sets/${name.replace(/\.txt$/, '')}.txt`, text); }
}

// Device mode is whatever answers `GET /api/status` like the firmware.
export async function detect() {
  try {
    const ctl = new AbortController();
    setTimeout(() => ctl.abort(), 4000);
    const r = await fetch('/api/status', { signal: ctl.signal, cache: 'no-store' });
    if (!r.ok) return null;
    const s = await r.json();
    return s && 'now_playing' in s ? { src: new DeviceSource(), status: s } : null;
  } catch { return null; }
}
