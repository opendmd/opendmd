// Preparing files for playback. A GIF's prepared copy is a pre-rendered file
// the player plays with less memory and opens faster. The device makes them
// itself when idle; this is the fast path (and the only one past 256
// colours): encoded here with the core's encoder (byte-identical to
// opendmd-pack), uploaded to where the player looks (odmPathFor). The GIF
// itself is never touched.
//
// `start()` prepares whatever the playlist in play is missing, one file at
// a time: every transfer costs the playing device heap, so there is no
// prefetch. It lives here, not in a view, so it keeps going while the user
// looks at something else; progress goes out as `prepare` events. Stopping
// and starting again carries on where it left off, because what is missing
// is worked out afresh from the device's list of copies each time.

import { app } from './state.js';
import { emit, base } from './util.js';

export const job = {
  running: false, stopping: false, phase: 'idle',
  todo: 0, done: 0, failed: 0, current: '', errors: [], t0: 0,
};

const tell = () => emit('prepare', job);

// Encode one GIF's bytes and upload its prepared copy. Throws on failure.
export async function prepareBytes(gifPath, bytes) {
  const dest = app.wasm.odmPathFor(gifPath);
  if (!dest) throw new Error('only GIFs can be prepared');
  const { odm } = app.wasm.encodeGif(bytes);
  await app.src.writeFile(dest, odm);
  return odm.length;
}

// The entries of the playlist in play (under the device's platform filter,
// as its count is) that have no prepared copy yet.
export async function missing() {
  const text = await app.text(app.activePath, true);
  if (text == null) throw new Error('the playlist could not be read');
  const paths = app.wasm.filterManifest(text, JSON.stringify({ platforms: app.config?.platforms || '' }));
  const have = new Set((await app.src.cacheList()).map((p) => p.toLowerCase()));
  return paths.filter((p) => { const c = app.wasm.odmPathFor(p); return c && !have.has(c.toLowerCase()); });
}

export async function start() {
  if (job.running) return;
  Object.assign(job, { running: true, stopping: false, phase: 'checking', todo: 0, done: 0, failed: 0, current: '', errors: [], t0: 0 });
  tell();
  try {
    const list = await missing();
    Object.assign(job, { phase: 'preparing', todo: list.length, t0: performance.now() });
    tell();
    for (const path of list) {
      if (job.stopping) break;
      job.current = path;
      tell();
      try {
        await prepareBytes(path, await app.src.readBytes(path));
        job.done++;
      } catch (e) {
        job.failed++;
        job.errors.push(`${base(path)}: ${e.message || e}`);
      }
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
  const n = job.done + job.failed;
  if (!n || !job.t0) return null;
  return ((performance.now() - job.t0) / 1000 / n) * (job.todo - n);
}
