// Thumbnails for list rows (the Library's folders, the playlist editor):
// a GIF's first frame, 64×16 CSS px, and a short clip on hover or focus.
// Light on the device by design (opendmd-notes docs/research/library-playlists.md):
//  - only rows that are rendered ask, and a row gone before its turn is
//    skipped; at most two requests in flight; none while a list is flung;
//  - the prepared copy first (`cachePathFor`), the GIF on a 404, read as a
//    stream and cut off once the first image is in (~1 KB of a ~87 KB copy);
//  - kept in IndexedDB per card (`app.cardKey`: the status's `sd.card_id`),
//    keyed by path, with the file's size as the validator, so a GIF is
//    asked for once, across visits; capped and evicted least recently used;
//    the stores of the four cards seen last are kept.
// The browser's own GIF decoder draws them (createImageBitmap, an <img>).

import { app } from './state.js';
import { dfetch } from './source.js';
import { enc, store, on, drawn } from './util.js';

export const thumbsOn = () => store.get('thumbs', true);
// What the page has asked for, for the tests and the design note's numbers.
export const stats = { net: 0, bytes: 0, max: 0, idb: 0, mem: 0, busy: 0, r503: 0, failed: 0 };

const CAP = 24 << 20, CLIP_BYTES = 32 << 10, CLIP_FRAMES = 24;
const mem = new Map(); // path -> bitmap, the 200 last drawn
const inflight = new Set();
// Failures that may pass (a timeout, a dropped connection, a restart, a
// 500) are not kept: the row is asked for again when drawn again, at least
// 10 s later, three times a session; a reload starts over.
const tries = new Map(); // path -> [failures, when]
let queue = [], pausedTo = 0, timer = 0;
// Bumped when the card changes: what was being read from the old one is
// neither stored nor drawn.
let epoch = 0;
on('card', () => {
  epoch++;
  for (const bm of mem.values()) bm?.close();
  mem.clear();
  tries.clear();
  queue = [];
});

// The end of the `want`-th image in a GIF's first bytes: [end, images, done].
function walk(b, want) {
  let p = 13, n = 0, end = 0;
  if (b.length < 13) return [0, 0, false];
  if (b[10] & 128) p += 3 << ((b[10] & 7) + 1);
  for (;;) {
    if (p >= b.length) return [end, n, false];
    const c = b[p];
    if (c !== 0x21 && c !== 0x2C) return [end, n, true]; // the trailer
    let q = p + 2;
    if (c === 0x2C) {
      if (p + 10 > b.length) return [end, n, false];
      q = p + 10 + (b[p + 9] & 128 ? 3 << ((b[p + 9] & 7) + 1) : 0) + 1;
    }
    while (q < b.length && b[q]) q += b[q] + 1;
    if (q >= b.length) return [end, n, false];
    p = q + 1;
    if (c === 0x2C && (end = p, ++n >= want)) return [end, n, true];
  }
}
// The first `want` images (fewer within `cap`) as a GIF of their own.
function cutBytes(b, want, cap) {
  const [end] = walk(b.subarray(0, cap), want);
  if (!end) return null;
  const out = new Uint8Array(end + 1);
  out.set(b.subarray(0, end));
  out[end] = 0x3B;
  return new Blob([out], { type: 'image/gif' });
}

// Read `path` (its copy first) as far as `want` images: {gif, size}.
// `extend(first frame)`: true reads on to a clip's worth (a dark start).
async function fetchCut(path, want, cap, extend) {
  // (The web demo's display: its card is in memory, read as a folder's.)
  if (!app.device || app.src.memory) {
    const b = await app.previewBytes(path);
    if (extend && await extend(cutBytes(b, want, cap))) return { gif: cutBytes(b, CLIP_FRAMES, CLIP_BYTES), size: b.length, more: true };
    return { gif: cutBytes(b, want, cap), size: b.length };
  }
  const copy = app.wasm.cachePathFor(path);
  for (const p of copy ? [copy, path] : [path]) {
    stats.net++;
    let r;
    try { r = await dfetch(enc(p), {}, 20000); } catch { throw { status: 0 }; }
    if (r.status === 404 && p !== path) continue;
    // Busy (the stream queue full, the heap low): 503 + Retry-After.
    if (r.status === 503) { stats.r503++; throw { status: 503, after: +r.headers.get('Retry-After') || 1 }; }
    if (!r.ok) throw { status: r.status };
    const size = +r.headers.get('Content-Length') || 0, rd = r.body.getReader();
    let buf = new Uint8Array(size && size < cap ? size : 8192), got = 0, more = false, t = 0;
    for (;;) {
      // A body that stalls for 8 s gives its slot up.
      const { done, value } = await Promise.race([rd.read(), new Promise((_, no) => { t = setTimeout(no, 8000, { status: 0 }); })])
        .catch((e) => { rd.cancel().catch(() => {}); throw e; }).finally(() => clearTimeout(t));
      if (value) {
        if (got + value.length > buf.length) { const nb = new Uint8Array(Math.max(buf.length * 2, got + value.length)); nb.set(buf.subarray(0, got)); buf = nb; }
        buf.set(value, got);
        got += value.length;
        stats.bytes += value.length;
      }
      if (done || got >= cap || walk(buf.subarray(0, got), want)[2]) {
        // A dark start reads on to its clip, unless what came already holds
        // it (a fast link's first piece can be the whole clip, or the file).
        const first = walk(buf.subarray(0, got), 1)[0];
        if (!more && extend && first && first <= CLIP_BYTES && await extend(cutBytes(buf.subarray(0, got), want, cap))) {
          more = true;
          want = CLIP_FRAMES;
          cap = CLIP_BYTES;
          if (!done && got < cap && !walk(buf.subarray(0, got), want)[2]) continue;
        }
        // The rest stays on the card. The device notices at its next 2 KB
        // piece: the slot is held a moment longer, so it never serves more
        // than two of these at once.
        if (!done) { rd.cancel().catch(() => {}); await new Promise((r) => setTimeout(r, 80)); }
        return { gif: cutBytes(buf.subarray(0, got), want, cap), size: size || got, more };
      }
    }
  }
  throw { status: 404 }; // neither the copy nor the GIF
}

// IndexedDB: one database per card, records {k: path, size, still, clip, n, at}
// and {k: '#', total}. Any failure (a private window) just means no store.
let dbq, dbns;
const CARDS = 4;
function db() {
  const ns = app.cardKey;
  if (!ns) return Promise.resolve(null); // the card not known yet: no store
  if (ns !== dbns) {
    dbns = ns;
    dbq?.then((d) => d?.close());
    tidy(ns);
    dbq = new Promise((res) => {
      try {
        const r = indexedDB.open('odmd-thumbs-' + ns, 1);
        r.onupgradeneeded = () => r.result.createObjectStore('t', { keyPath: 'k' }).createIndex('at', 'at');
        r.onsuccess = () => res(r.result);
        r.onerror = r.onblocked = () => res(null);
      } catch { res(null); }
    });
  }
  return dbq;
}
// The stores of the cards seen last stay (a card swapped back finds its
// thumbnails); older ones go, and so do those named by the card's size
// alone (firmware before `sd.card_id`) once the device names its card.
function tidy(ns) {
  const seen = [ns, ...store.get('thumbs.cards', []).filter((x) => x !== ns)];
  store.set('thumbs.cards', seen.slice(0, CARDS));
  const keep = new Set(seen.slice(0, CARDS).map((x) => 'odmd-thumbs-' + x));
  indexedDB.databases?.().then((all) => {
    for (const { name } of all) {
      if (name?.startsWith('odmd-thumbs-') && !keep.has(name) && (seen.includes(name.slice(12)) || ns[0] === 'i' && /^odmd-thumbs-c\d+$/.test(name))) indexedDB.deleteDatabase(name);
    }
  }).catch(() => {});
}
const req = (r) => new Promise((res, rej) => { r.onsuccess = () => res(r.result); r.onerror = () => rej(r.error); });
async function get(path) {
  try {
    const d = await db();
    const rec = d && await req(d.transaction('t').objectStore('t').get(path));
    // Touched at most once a day: a read is not a write.
    if (rec && Date.now() - rec.at > 864e5) put({ ...rec });
    return rec;
  } catch { return null; }
}
async function put(rec) {
  try {
    const d = await db();
    if (!d) return;
    rec.at = Date.now();
    rec.n = 64 + (rec.still?.size || 0) + (rec.clip?.size || 0);
    const t = d.transaction('t', 'readwrite'), s = t.objectStore('t');
    const [old, meta] = await Promise.all([req(s.get(rec.k)), req(s.get('#'))]);
    let total = (meta?.total || 0) - (old?.n || 0) + rec.n;
    s.put(rec);
    if (total > CAP) {
      // The least recently used go, down to 80% of the cap.
      const c = s.index('at').openCursor();
      await new Promise((res) => {
        c.onsuccess = () => {
          const cur = c.result;
          if (!cur || total <= CAP * 0.8) return res();
          if (cur.value.k !== rec.k) { total -= cur.value.n || 0; cur.delete(); }
          cur.continue();
        };
        c.onerror = res;
      });
    }
    s.put({ k: '#', total });
  } catch { /* full, closed, private */ }
}

// A GIF changed or went (Add GIFs, Edit details, Delete): ask again.
export async function forget(path) {
  mem.delete(path);
  try { const d = await db(); d?.transaction('t', 'readwrite').objectStore('t').delete(path); } catch { /* none */ }
}

// The whole file was read anyway (the Details preview): if it is not the
// size the thumbnail came from, make it again from these bytes. Free.
export async function check(path, bytes) {
  if (!thumbsOn() || drawn(path) || !bytes?.length) return;
  const rec = await get(path);
  if (rec && rec.size === bytes.length) return;
  const still = cutBytes(bytes, 1, bytes.length);
  await put({ k: path, size: bytes.length, still });
  mem.delete(path);
  show(path, await bitmap(still));
}

const bitmap = (b) => (b ? createImageBitmap(b).catch(() => null) : null);
function paint(cv, bm) {
  const x = cv.getContext('2d');
  x.clearRect(0, 0, 128, 32);
  if (bm) {
    const s = Math.min(128 / bm.width, 32 / bm.height), w = bm.width * s, y = bm.height * s;
    x.drawImage(bm, (128 - w) / 2, (32 - y) / 2, w, y);
  }
  cv.dataset.done = bm ? 1 : 0;
}
function show(path, bm) {
  mem.delete(path);
  mem.set(path, bm);
  if (mem.size > 200) { const [k, v] = mem.entries().next().value; mem.delete(k); v?.close(); }
  for (const c of document.querySelectorAll('canvas.th')) if (c._p === path) paint(c, bm);
}
const live = (cv) => cv.isConnected && !cv.parentNode.hidden;

// A rendered row's canvas: drawn from memory, the store, or (queued) the device.
export function attach(cv, path) {
  cv._p = path;
  delete cv.dataset.done;
  if (mem.has(path)) { stats.mem++; const bm = mem.get(path); mem.delete(path); mem.set(path, bm); paint(cv, bm); return; }
  const e = epoch;
  get(path).then(async (rec) => {
    if (cv._p !== path) return;
    // Another card since the read began: the record is the old card's. The
    // rows are drawn again by the views' own refresh; this one waits for it.
    if (e !== epoch) { attach(cv, path); return; }
    if (rec) {
      const bm = await bitmap(rec.still);
      if (e !== epoch) { bm?.close(); attach(cv, path); return; } // (decoding took a card swap)
      stats.idb++;
      show(path, bm);
      return;
    }
    if (inflight.has(path)) return; // drawn when it comes
    const [n, at] = tries.get(path) || [0, 0];
    if (n >= 3 || Date.now() - at < 10000) { paint(cv, null); return; }
    queue.push({ cv, path });
    pump();
  });
}

// Only a row in view is fetched (the list's pooled rows above and below
// wait), two at a time.
function seen(cv) {
  const l = cv.closest('.vlist'), r = cv.getBoundingClientRect(), b = l?.getBoundingClientRect();
  return b && r.bottom > b.top && r.top < b.bottom;
}
// A read to try again: for each canvas showing its path now, not drawn (a
// row drawn again while it was out has a new one), else the one it had.
function again(q) {
  const cvs = [...document.querySelectorAll('canvas.th')].filter((c) => c._p === q.path && live(c) && !c.dataset.done);
  return cvs.length ? cvs.map((cv) => ({ cv, path: q.path })) : [q];
}
function pump() {
  clearTimeout(timer);
  const wait = pausedTo - performance.now();
  if (wait > 0) { timer = setTimeout(pump, wait); return; }
  queue = queue.filter((q) => q.run || (live(q.cv) && q.cv._p === q.path && !q.cv.dataset.done));
  while (stats.busy < 2) {
    const k = queue.findIndex((q) => q.run || (!inflight.has(q.path) && seen(q.cv)));
    if (k < 0) return;
    const [q] = queue.splice(k, 1);
    if (!q.run && mem.has(q.path)) { paint(q.cv, mem.get(q.path)); continue; }
    stats.busy++;
    stats.max = Math.max(stats.max, stats.busy);
    const was = epoch;
    (q.run ? q.run() : stillOf(q)).catch((e) => {
      if (was !== epoch) return; // the old card's
      // Busy: everything waits Retry-After, this row first again after.
      // (The row's canvas now: one made while this was out waited for it.)
      if (e?.status === 503) { pausedTo = performance.now() + e.after * 1000; if (!q.run) queue.unshift(...again(q)); return; }
      if (q.run) return;
      // Neither file there: kept as nothing. Anything else may pass.
      if (e?.status === 404) { show(q.path, null); put({ k: q.path, size: 0, still: null }); return; }
      stats.failed++;
      tries.set(q.path, [(tries.get(q.path)?.[0] || 0) + 1, Date.now()]);
      for (const c of document.querySelectorAll('canvas.th')) if (c._p === q.path) paint(c, null);
    }).finally(() => { stats.busy--; pump(); });
  }
}
// Mostly black (a fade-in): fewer than 16 of 256 dots lit at 32×8.
async function dark(gif) {
  const bm = await bitmap(gif);
  if (!bm) return false;
  const c = new OffscreenCanvas(32, 8).getContext('2d', { willReadFrequently: true });
  c.drawImage(bm, 0, 0, 32, 8);
  const d = c.getImageData(0, 0, 32, 8).data;
  let n = 0;
  for (let i = 0; i < d.length; i += 4) n += d[i] + d[i + 1] + d[i + 2] > 60;
  return n < 16;
}
// The first frame of `clip` that is not dark, as a PNG (WebCodecs' ImageDecoder).
async function litFrame(clip) {
  try {
    const dec = new ImageDecoder({ data: await clip.arrayBuffer(), type: 'image/gif' });
    await dec.tracks.ready;
    for (let i = 1; i < dec.tracks.selectedTrack.frameCount; i++) {
      const { image } = await dec.decode({ frameIndex: i });
      const c = new OffscreenCanvas(image.displayWidth, image.displayHeight);
      c.getContext('2d').drawImage(image, 0, 0);
      image.close();
      const png = await c.convertToBlob();
      if (!await dark(png)) return png;
    }
  } catch { /* no decoder, a frame it refused */ }
  return null;
}
async function stillOf({ path }) {
  inflight.add(path);
  const e = epoch;
  try {
    // A GIF that starts dark is read on to its clip (≤ 32 KB, kept for the
    // hover too) and shows its first lit frame where the browser can say.
    const r = await fetchCut(path, 1, 64 << 10, typeof ImageDecoder == 'function' && dark);
    const gif = r.more && await litFrame(r.gif) || r.gif, bm = await bitmap(gif);
    if (e !== epoch) return; // another card now
    // Bytes came and nothing decoded: kept as nothing (not asked again).
    await put({ k: path, size: r.size, still: bm ? gif : null, ...(r.more ? { clip: r.gif } : {}) });
    show(path, bm);
  } finally { inflight.delete(path); }
}

// Lists report their scrolling: a fling (over 1.5 px/ms) holds the queue
// until 150 ms after it slows.
export function watch(el) {
  let y = el.scrollTop, t = performance.now();
  el.addEventListener('scroll', () => {
    const now = performance.now(), v = Math.abs(el.scrollTop - y) / Math.max(1, now - t);
    y = el.scrollTop;
    t = now;
    if (v > 1.5) pausedTo = now + 150;
    clearTimeout(timer);
    timer = setTimeout(pump, 100); // rows scrolled into view without a redraw
  }, { passive: true });
  // The clip: 350 ms on a row (pointer or focus), one at a time; read
  // through the queue like a still, ahead of them.
  let on = null, hold = 0;
  const stop = () => { clearTimeout(hold); if (on) { on.img?.remove(); if (on.img) URL.revokeObjectURL(on.img.src); if (on.cv) on.cv.hidden = false; on = null; } };
  const start = (e) => {
    const row = e.target.closest?.('.vrow'), cv = row?.querySelector('canvas.th');
    if (on?.cv === cv) return;
    stop();
    if (!cv) return;
    const q = on = { cv, path: cv._p };
    q.run = async () => {
      let rec = await get(q.path);
      if (rec && !rec.still && !rec.clip) return; // it could not be read
      if (!rec?.clip) {
        const e = epoch, { gif, size } = await fetchCut(q.path, CLIP_FRAMES, CLIP_BYTES);
        if (e !== epoch) return;
        // Kept only beside a still: a record without one reads as "nothing
        // to show", and a row whose still has not come (or failed) would
        // then never ask for it again.
        const had = rec;
        rec = { ...(rec || {}), k: q.path, size, clip: gif || null };
        if (had) await put(rec);
      }
      // (The row may have been drawn again meanwhile: its canvas now.)
      const c = [...el.querySelectorAll('canvas.th')].find((x) => x._p === q.path && live(x));
      if (on !== q || !rec.clip || !c) return;
      q.cv = c;
      q.img = Object.assign(document.createElement('img'), { className: 'th', src: URL.createObjectURL(rec.clip), alt: '' });
      c.after(q.img);
      c.hidden = true;
    };
    // Still on the row (or focused) when the time is up.
    hold = setTimeout(() => { if (on === q && (row.matches(':hover') || row.contains(document.activeElement))) { queue.unshift(q); pump(); } }, 350);
  };
  el.addEventListener('pointerover', (e) => e.pointerType === 'mouse' && start(e));
  el.addEventListener('focusin', start);
  el.addEventListener('pointerleave', stop);
  el.addEventListener('focusout', (e) => { if (!el.contains(e.relatedTarget)) stop(); });
  el.addEventListener('scroll', stop, { passive: true });
}
