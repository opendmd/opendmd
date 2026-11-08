// Formulas' "Show on panel" (imported by js/formulas.js on a device): the
// formula, compiled, becomes the panel's live item (POST /api/play/formula,
// the compiled bytes as the body, so its length never rides on a path)
// until the playlist moves on; changes made meanwhile follow it there as
// live edits (`live=1`: the device swaps the formula in place, or says
// 409 once it has moved on), ~200 ms apart, one request at a time, the
// latest only. A formula that draws over a GIF goes over the panel's GIF
// instead (`over=1`, until the playlist moves on; pressed again, off).

import { app } from './state.js';
import { toast } from './util.js';
import { path, program, vals } from './psikit.js';

// The device's `fx_psi::path` (psikit `path`): Add to playlist's inline
// line, with no card.
export const fpath = (f, ctl) => path(f, ctl);
// The panel busy (503 + Retry-After: the player had not taken it, or a
// short heap): asked again, 5 times.
const TRIES = 5;

let shown = null, next = null, busy = false, last = 0, btns = [], isOver = () => false, ctlOf = () => null, ov = false;
// `o(f)`: it draws over the GIF; `c(f)`: its controls (for the path's values).
export function init(b, o, c) { btns = b; isOver = o; ctlOf = c; mark(); }
// Each button says whether its formula is the one on the panel.
export function mark() {
  for (const [b, f] of btns) {
    const x = f(), on = !!shown && x === shown, o = !!x?.src && isOver(x);
    b.textContent = on ? (ov ? 'Over the panel’s GIF ✓' : 'On the panel ✓') : o ? 'Show over the panel’s GIF' : 'Show on panel';
    b.title = on && ov ? 'Press again to take it off' : '';
    b.setAttribute('aria-pressed', on);
  }
}
// (Nothing to show before the editor's text has compiled once.)
export function show(f) {
  if (!f?.src) return;
  if (f === shown && ov) { shown = null; next = [{ src: '', over: 1 }, false]; } else { ov = isOver(f); shown = f; next = [f, false]; }
  pump();
  mark();
}
export function follow(f) { if (f === shown && !next) { next = [f, true]; pump(); } }
export function drop(f) { if (shown === f) { shown = null; mark(); } }

async function pump() {
  if (busy || !next) return;
  const wait = last + 200 - Date.now();
  if (wait > 0) { busy = true; setTimeout(() => { busy = false; pump(); }, wait); return; }
  const [f, live, tries = 0] = next, name = f.name || 'Formula', o = f.over || (f === shown && ov);
  next = null; busy = true; last = Date.now();
  try {
    if (f.src) {
      const q = new URLSearchParams(vals(f, ctlOf(f)).slice(1));
      if (live) q.set('live', 1);
      if (o) q.set('over', 1);
      await app.src.req('POST', '/api/play/formula?' + q, new Blob([program(f.src)], { type: 'application/octet-stream' }), 8000);
    } else await app.src.json('POST', '/api/play', { path: '', live, over: true }, 8000);
    if (!live) toast(!f.src ? 'Taken off the panel' : o ? `Over the panel’s GIF: ${name}` : `On the panel: ${name}`, 'ok');
    // Over the GIF, it needs the player's memory: say so if it is waiting for it.
    if (o && f.src && !live) setTimeout(() => app.refreshStatus?.().then((st) => { if (shown === f && st?.formula_look?.held === 0) toast('The display is short of memory just now: it draws this over the GIF as soon as there is room', 'bad'); }).catch(() => {}), 1500);
  } catch (e) {
    const s = e.status;
    // (A newer change goes instead, if one came.)
    if (s == 503 && e.after && tries < TRIES) {
      next ??= [f, live, tries + 1];
      last = Date.now() + e.after * 1000 - 200;
      busy = false;
      pump();
      return;
    }
    if (shown === f) { shown = null; if (next?.[0] === f) next = null; }
    // A change after the playlist moved on: it just stops following.
    const why = e.message?.replace(/^400: /, '');
    if (s != 409) toast(s == 415 || s == 404 || s == 405 ? 'Update OpenDMD to show formulas on the panel' : s == 400 ? `The panel can’t ${o ? 'draw' : 'show'} “${name}”${o ? ' over its GIF' : ''}: ${why}`
      : s == 413 ? 'This formula is too long to show on the panel' : s == 503 && e.after ? 'The display is busy just now: try again in a moment' : s == 503 ? `The display has nothing playing to ${o ? 'draw over' : 'interrupt'}` : s ? 'Show on panel: ' + e.message : 'Couldn’t reach the display', 'bad');
  }
  busy = false;
  mark();
  pump();
}
