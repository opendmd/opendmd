// The home page's live panels (site/pages/home.html: every element with a
// `data-formula`, the hero's and the Formulas door's): a bundled Formula
// each (the hero's the first of its show, hero.js), drawn at the
// browser's frame rate through psiled.js, the Psi reference's own drawing. tools/site.py copies this file to the site's
// root and loads it on the home page only.
// A panel's recorded still (its `.led` picture, site-art.py) is the
// poster: shown at once, and kept when the visitor prefers less motion,
// when the wasm or the formula cannot load, or when it does not compile.
// Nothing loads until the page has (and the browser is idle); a still
// gives way only once its first frame is drawn. A panel stops drawing
// while it is off-screen or the tab is hidden. Nothing goes to the console.

import { psi, use, frameAt, panel, paint } from './psiled.js';

const less = matchMedia('(prefers-reduced-motion: reduce)');
const boxes = [...document.querySelectorAll('[data-formula]')].filter((b) => b.querySelector('.led'));

if (boxes.length && !less.matches) {
  const idle = () => (window.requestIdleCallback ? requestIdleCallback(go, { timeout: 2000 }) : setTimeout(go, 200));
  if (document.readyState === 'complete') idle(); else addEventListener('load', idle, { once: true });
}

// Each panel its own Psi (one compiled formula apiece), so both can play
// at once without compiling again every frame.
// The hero's panel plays a little show instead (`data-show`: hero.js,
// fetched only now).
function go() { boxes.forEach((b, i) => (b.dataset.show ? import('./hero.js').then((m) => m.show(b, i, less)) : live(b, i)).catch(() => {})); }

async function live(box, slot) {
  const pic = box.querySelector('.led');
  const [L, src] = await Promise.all([psi(slot), fetch(box.dataset.formula).then((r) => (r.ok ? r.text() : null))]);
  if (!src || less.matches || !use(L.P, src)) return;
  const info = L.P.info();
  // A formula that never repeats starts somewhere else each time, as on
  // the panel (Q16.16: 65,536 is 1).
  L.P.setSeed(Math.floor(Math.random() * 65536));
  const x = panel(pic.querySelector('img')?.alt || '');
  let ran = 0, prev = 0, last = -1, raf = 0, seen = true;
  const tick = (now) => {
    raf = 0;
    // Its own time: only while it plays (a hidden tab's gap not counted).
    ran += prev ? Math.min(now - prev, 100) : 0;
    prev = now;
    const ms = frameAt(info, ran);
    if (ms !== last) {
      last = ms;
      try { paint(L, info, x, ms); } catch { return; }
      if (!x.led.isConnected) {
        pic.replaceWith(x.led);
        box.dataset.live = 'on';
      }
    }
    run();
  };
  // Drawing while on screen, the tab shown and motion welcome.
  const run = () => {
    const on = seen && !document.hidden && !less.matches;
    if (on && !raf) raf = requestAnimationFrame(tick);
    if (!on && raf) { cancelAnimationFrame(raf); raf = 0; }
    if (!on) prev = 0;
  };
  new IntersectionObserver(([e]) => { seen = e.isIntersecting; run(); }).observe(box);
  document.addEventListener('visibilitychange', run);
  less.addEventListener('change', run);
  run();
}
