// Preparing files for playback. A GIF's prepared copy is a GIF re-encoded
// so the player plays it with less memory and opens it faster. The device makes them
// itself when idle; this is the fast path (and the only one past 256
// colours): encoded here with the core's encoder (byte-identical to
// opendmd-pack; it lives in the second wasm module, js/artmod.js, loaded on
// first use), uploaded to where the player looks (cachePathFor). The GIF
// itself is never touched.
//
// `start()` prepares whatever the playlist in play is missing, one file at
// a time: every transfer costs the playing device heap, so there is no
// prefetch. It lives here, not in a view, so it keeps going while the user
// looks at something else; progress goes out as `prepare` events. Stopping
// and starting again carries on where it left off, because what is missing
// is worked out afresh from the device's list of copies each time.
//
// Two kinds of trouble, neither of which stops the run:
// - damaged: the encoder refused the GIF (bad LZW, truncated, not a GIF;
//   past 256 colours is quantised, not an error). It will never work, so
//   it is recorded and skipped from then on: `POST /api/cache/failed` where
//   the device has it (the list is `/cache/.failed`, shared with its own cache
//   builder), else in this browser's storage, per device. The builder's own
//   verdicts are read too (`GET /api/cache/failed`, or the file itself),
//   bar the ones this encoder can do (colours, size).
// - network: a GET or PUT that failed is tried again after 2 s, then once
//   more at the end of the run; what still fails is listed together.

import { app } from './state.js';
import { emit, store } from './util.js';
import { art } from './artmod.js';

export const job = {
  running: false, stopping: false, phase: 'idle',
  todo: 0, done: 0, current: '', errors: [], t0: 0,
  damaged: [], net: [], tried: 0,
};

const tell = () => emit('prepare', job);
const sleep = (ms) => new Promise((r) => setTimeout(r, ms));

// Encode one GIF's bytes and upload its prepared copy. Throws on failure.
export async function prepareBytes(gifPath, bytes) {
  const dest = app.wasm.cachePathFor(gifPath);
  if (!dest) throw new Error('only GIFs can be prepared');
  const { copy } = (await art()).prepareGif(bytes);
  await app.src.writeFile(dest, copy);
  return copy.length;
}

// The entries of the playlist in play (under the device's platform filter,
// as its count is) that have no prepared copy yet.
export async function missing() {
  const text = await app.text(app.activePath, true);
  if (text == null) throw new Error('the playlist could not be read');
  const paths = app.wasm.filterManifest(text, JSON.stringify({ platforms: app.config?.platforms || '' }));
  const have = new Set((await app.src.cacheList()).map((p) => p.toLowerCase()));
  return paths.filter((p) => { const c = app.wasm.cachePathFor(p); return c && !have.has(c.toLowerCase()); });
}

// --- the damaged list -----------------------------------------------------
const localKey = () => 'damaged.' + location.host;
let shared = false; // the device has /api/cache/failed

// Lowercased path -> reason, for every GIF known to be damaged.
async function damagedList() {
  const out = new Map();
  let rows = null;
  try { rows = await app.src.cacheFailed(); shared = true; } catch (e) {
    shared = false;
    if (e.status !== 404) throw e;
    const text = await app.src.readText('/cache/.failed').catch(() => null);
    rows = (text || '').split('\n').map((l) => { const [path, reason = ''] = l.split('\t'); return { path: path.trim(), reason }; });
  }
  for (const r of rows) {
    // The builder also gives up on what this encoder can do: past 256
    // colours, a file too large (or needing more memory) to prepare there,
    // one that vanished.
    if (r.path && !/colou?rs|too large|not found/.test(r.reason)) out.set(r.path.toLowerCase(), r.reason);
  }
  for (const [p, why] of Object.entries(store.get(localKey(), {}))) out.set(p.toLowerCase(), why);
  return out;
}

async function recordDamaged(path, reason) {
  if (shared) {
    try { await app.src.cacheFail(path, reason); return; } catch { shared = false; /* no POST: keep it here */ }
  }
  store.set(localKey(), { ...store.get(localKey(), {}), [path]: reason });
}

// --- the run ----------------------------------------------------------------
// One file: 'ok', 'damaged' (recorded) or 'net' (after its retry, if any).
async function one(path, retry) {
  for (let i = 0; ; i++) {
    let bytes;
    try {
      bytes = await app.src.readBytes(path);
      const m = await art();
      let copy;
      try { copy = m.prepareGif(bytes).copy; } catch (e) {
        const reason = String(e.message || e);
        job.damaged.push({ path, reason });
        await recordDamaged(path, reason);
        return 'damaged';
      }
      await app.src.writeFile(app.wasm.cachePathFor(path), copy);
      return 'ok';
    } catch (e) {
      if (!retry || i || job.stopping) return 'net';
      await sleep(2000);
    }
  }
}

export async function start() {
  if (job.running) return;
  Object.assign(job, { running: true, stopping: false, phase: 'checking', todo: 0, done: 0, current: '', errors: [], t0: 0, damaged: [], net: [], tried: 0 });
  tell();
  try {
    const [list, bad] = await Promise.all([missing(), damagedList()]);
    const todo = [];
    for (const p of list) {
      const why = bad.get(p.toLowerCase());
      if (why != null) job.damaged.push({ path: p, reason: why, known: true });
      else todo.push(p);
    }
    Object.assign(job, { phase: 'preparing', todo: todo.length, t0: performance.now() });
    tell();
    const later = [];
    for (const path of todo) {
      if (job.stopping) break;
      job.current = path;
      tell();
      const r = await one(path, true);
      job.tried++;
      if (r === 'ok') job.done++;
      else if (r === 'net') { later.push(path); job.net.push(path); }
    }
    // Network failures get one more go at the end, once the rest is done.
    for (const path of later) {
      if (job.stopping) break;
      job.current = path;
      tell();
      const r = await one(path, false);
      if (r !== 'net') job.net.splice(job.net.indexOf(path), 1);
      if (r === 'ok') job.done++;
      tell();
    }
  } catch (e) {
    job.errors.push(e.message || String(e));
  }
  Object.assign(job, { running: false, phase: job.stopping ? 'stopped' : 'finished', current: '' });
  tell();
}

export function stop() { if (job.running) { job.stopping = true; tell(); } }

// Seconds left at the pace so far, or null before the first file is done.
export function eta() {
  const n = job.tried;
  if (!n || !job.t0) return null;
  return ((performance.now() - job.t0) / 1000 / n) * (job.todo - n);
}
