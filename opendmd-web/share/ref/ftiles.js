// The formula gallery (/formulas/): a tile's Formula plays live while the
// pointer is over the tile or the keyboard's focus is in it, drawn by
// psiled.js (the Psi reference's and the home page's drawing) in place of
// its thumbnail, which comes back when the pointer leaves or the focus
// moves on. One plays at a time. tools/site.py copies this file beside the
// gallery's page (the Worker's page from the store keeps the built page's
// head, so its tiles play too).
// The wasm loads at the first hover or focus, never before. Nothing
// happens on a screen with no hover (touch: the thumbnails as they are)
// or when the visitor prefers less motion. Nothing goes to the console.
// A tile's source is in its picture's link: `#bundled=<slug>` (the file
// beside this page, src/<slug>.psi) or `#formula=<source>`.

import { psi, use, frameAt, panel, paint } from '../psiled.js';

const less = matchMedia('(prefers-reduced-motion: reduce)');
const hover = matchMedia('(hover: hover)');
const tiles = document.querySelector('main');

let now = null; // the tile playing: {tile, pic, x, raf}
let want = null; // the tile asked for last (the pointer's or the focus's)
let sample = null; // the picture a look draws over, once one is asked for

async function source(tile) {
  const a = tile.querySelector('a.fpic');
  const q = new URLSearchParams((a && new URL(a.href).hash.slice(1)) || '');
  if (q.has('formula')) return q.get('formula');
  const slug = q.get('bundled');
  if (!slug || !/^[\w-]+$/.test(slug)) return null;
  const r = await fetch(`src/${slug}.psi`);
  return r.ok ? r.text() : null;
}

function stop() {
  if (!now) return;
  cancelAnimationFrame(now.raf);
  now.x.led.remove();
  now.pic.hidden = false;
  delete now.tile.dataset.live;
  now = null;
}

async function play(tile) {
  want = tile;
  if (now?.tile === tile) return;
  stop();
  if (less.matches) return;
  const pic = tile.querySelector('a.fpic .led');
  if (!pic) return;
  try {
    const [L, src] = await Promise.all([psi(), source(tile)]);
    if (want !== tile || now || !src || !use(L.P, src)) return;
    const info = L.P.info();
    // Neither the time nor the clock: the thumbnail is all of it.
    if (!(info[5] & 24)) return;
    const over = !!(info[5] & 32);
    if (over && !sample) sample = (await import('../demo/js/psikit.js')).sample;
    if (want !== tile || now) return;
    const x = panel('');
    const me = { tile, pic, x, raf: 0 };
    now = me;
    let t0 = 0, last = -1;
    const tick = (t) => {
      if (now !== me) return;
      t0 ||= t;
      const ms = frameAt(info, t - t0);
      if (ms !== last) {
        last = ms;
        // Another tile may have compiled since: this one's source again.
        if (!use(L.P, src)) { stop(); return; }
        paint(L, info, x, ms, over ? sample() : null);
        if (!x.led.isConnected) {
          pic.hidden = true;
          pic.after(x.led);
          tile.dataset.live = 'on';
        }
      }
      me.raf = requestAnimationFrame(tick);
    };
    me.raf = requestAnimationFrame(tick);
  } catch { stop(); }
}

function leave(tile) {
  if (want === tile) want = null;
  if (now?.tile === tile) stop();
}

if (tiles && !less.matches) {
  const tileOf = (e) => e.target.closest?.('li.ftile');
  if (hover.matches) {
    tiles.addEventListener('pointerover', (e) => { const t = tileOf(e); if (t && e.pointerType !== 'touch') play(t); });
    tiles.addEventListener('pointerout', (e) => {
      const t = tileOf(e);
      if (t && !t.contains(e.relatedTarget)) leave(t);
    });
  }
  tiles.addEventListener('focusin', (e) => { const t = tileOf(e); if (t) play(t); });
  tiles.addEventListener('focusout', (e) => {
    const t = tileOf(e);
    if (t && !t.contains(e.relatedTarget)) leave(t);
  });
  less.addEventListener('change', () => { if (less.matches) stop(); });
}
