// The Studio's way out (/studio/), loaded at the first download: the piece
// flattened to its frames (every shown layer, composited as doc.js shows
// it; a hidden layer is not in the GIF), the GIF's colour table, the GIF
// written by the site's own wasm module (pkg-studio, opendmd-studio-web;
// never on the panel) and downloaded.
import { Doc, N } from './doc.js';

let writer = null;
const gifWriter = () => (writer ??= import('../demo/js/wasm.js').then((w) => w.load('pkg-studio')).then(([m]) => m)
  .catch((e) => { writer = null; throw e; }));

// A body's frames, each as the keys it shows, back to back; and its colours.
export async function flatten(body, coloursOf) {
  const d = new Doc(body, coloursOf);
  await d.loadAll();
  const keys = new Uint16Array(d.frames.length * N);
  d.frames.forEach((_, f) => keys.set(d.composite(f), f * N));
  return { keys, cm: d.cols, delays: Uint32Array.from(d.frames, (f) => f.ms), plays: body.plays || 0 };
}

// fit: 256 colours picked for spread, the rest as their nearest (a GIF the panel can prepare and smooth).
export async function download(S, fit) {
  const b = document.getElementById(fit ? 's-fit' : 's-gif');
  b.disabled = true;
  try {
    const m = await gifWriter();
    S.done();
    // What is downloaded is what is frozen: these pixels, saved and snapshotted in the chain, then read back.
    const D = S.doc, shot = D.snap();
    const h = await S.freeze('', shot), frozen = h && await S.store.get(S.rec.id, h.rev - 1);
    const body = frozen?.body || await D.packed(shot, { palette: S.swatch().id });
    const { keys, cm, delays, plays } = await flatten(body, S.listOf);
    // The GIF's table: only the colours used, off first, the same colour once.
    const table = [], index = new Map(), of = new Map();
    for (const k of [...new Set(keys)].sort((a, c) => a - c)) {
      const c = k ? cm.get(k) ?? 0 : 0;
      if (!index.has(c)) { index.set(c, table.length); table.push(c); }
      of.set(k, index.get(c));
    }
    // Past 256 colours: a whole picture over 256 of them picked for spread, then the rest exactly (or, fitted, only that picture).
    const rgb = () => Uint32Array.from(keys, (k) => (k ? cm.get(k) ?? 0 : 0));
    const gif = table.length <= 256 ? m.encodeIndexed(Uint32Array.from(table), Uint8Array.from(keys, (k) => of.get(k)), delays, plays)
      : fit ? m.encodeFitted(rgb(), delays, plays) : m.encodeWide(rgb(), delays, plays);
    const stem = S.nameNow() + (fit && table.length > 256 ? ' 256' : '');
    const name = (stem.replace(/[^\p{L}\p{N} _-]+/gu, '').trim().replace(/\s+/g, '_') || 'drawing') + '.gif';
    const a = document.createElement('a');
    a.href = URL.createObjectURL(new Blob([gif], { type: 'image/gif' }));
    a.download = name;
    document.body.append(a);
    a.click();
    a.remove();
    setTimeout(() => URL.revokeObjectURL(a.href), 30000);
    S.root.dataset.gif = gif.length;
    S.say('');
    const n = fit ? Math.min(table.length, 256) : table.length;
    if (S.saved !== 'failed') S.status(S.kept ? 'kept' : 'new', `Downloaded ${name}: ${n} colour${n === 1 ? '' : 's'}, ${(gif.length / 1024).toFixed(1)} KB`);
    await S.snaps();
  } catch (e) {
    S.say(`The GIF could not be made: ${e.message || e}`);
  } finally {
    b.disabled = false;
  }
}
