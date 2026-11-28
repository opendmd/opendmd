// Add GIFs' second step: a picture (PNG, JPEG, WebP…) or a GIF that is not
// 128 × 32, fitted to the panel before its details are asked for. One sheet
// per file that needs it, in turn:
//
// - how it fits: Pixels (the picture's own pixels, an enlarged one's
//   undone), Pixels ½ (half that), or Fit (the whole picture scaled into the
//   panel) — the art module guesses one (`autoFit`), Auto guesses again;
// - what shows: a 128 × 32 frame over the picture, dragged (or a tap moves
//   it there), started over the busiest part;
// - a preview on the panel's dots, and Next: the art module writes the GIF
//   for the panel (its own palette of the colours used, every frame whole:
//   the encoder the website's converter uses), which Add GIFs then names,
//   adds and prepares like any other.
//
// A GIF can always be added as it is instead ("Add as it is"): the panel
// plays a GIF of any size, showing its middle 128 × 32, and keeps every
// frame — a long GIF (the website's Studio cuts one to 256 frames) or one
// past what the art module keeps (anim.rs MAX_FRAMES) goes on the card
// unchanged.
//
// Also a Formula's "Save as GIF…" (js/formulas.js): its frames, drawn by
// the Psi module, written as a GIF the same way (`framesGif`).
//
// Loaded only when a picked file needs it; the art module (pkg-art/,
// js/artmod.js) does the pixel work.

import { art } from './artmod.js';
import { PanelPreview, fitScale } from './preview.js';
import { dlgStyle } from './details.js';
import { h, clear } from './util.js';

const W = 128, H = 32;
const MAX_PX = 4e6; // bigger pictures are scaled down on the way in
const FITS = [['pixels', 'Pixels'], ['half', 'Pixels ½'], ['fit', 'Fit']];
const later = () => new Promise((r) => setTimeout(r, 0));

const CSS = `.fit-view{position:relative;max-width:100%;background:#000;cursor:crosshair;margin:12px auto 0;overflow:hidden}
.fit-view canvas{display:block;width:100%;image-rendering:pixelated}
.fit-win{position:absolute;box-shadow:0 0 0 2px var(--acc),0 0 0 9999px #0006;cursor:move;touch-action:none}
.fit-seg{display:inline-flex;border:1px solid var(--line);border-radius:var(--r);overflow:hidden}
.fit-seg button{border:0;border-radius:0;min-height:32px;padding:4px 12px;color:var(--mut)}
.fit-seg button+button{border-left:1px solid var(--line)}
.fit-seg button.on,.fit-seg button.on:hover{background:var(--line);color:var(--fg)}
#fit .row{margin-top:12px}.fit-pv{max-width:512px;margin:12px auto 0}`;
let styled = false;

// Call `step` (a little wasm work; true when done) until done, giving the
// page back every ~12 ms. False if `alive()` said stop.
async function run(step, alive, progress) {
  for (;;) {
    const t = performance.now();
    do { if (step()) return true; } while (performance.now() - t < 12);
    progress?.();
    await later();
    if (alive && !alive()) return false;
  }
}

// A GIF's width and height from its header (null: not a GIF).
export function gifSize(b) {
  if (b.length < 10 || b[0] !== 0x47 || b[1] !== 0x49 || b[2] !== 0x46) return null;
  return [b[6] | (b[7] << 8), b[8] | (b[9] << 8)];
}

// Does `file` need fitting first: a picture, or a GIF of another size.
export async function needsFit(file) {
  const s = gifSize(new Uint8Array(await file.slice(0, 10).arrayBuffer()));
  return !s || s[0] !== W || s[1] !== H;
}

// The files Add GIFs adds ([{file, init}]), each that needs it fitted in
// turn; a sheet's Cancel leaves that one out.
export async function fitAll(files, inits = []) {
  const out = [], todo = [];
  for (let i = 0; i < files.length; i++) if (await needsFit(files[i])) todo.push(i);
  for (let i = 0; i < files.length; i++) {
    const k = todo.indexOf(i);
    const file = k < 0 ? files[i] : await sheet(files[i], todo.length > 1 ? ` · ${k + 1} of ${todo.length}` : '');
    if (file) out.push({ file, init: inits[i] });
  }
  return out;
}

const stem = (name) => name.replace(/\.[^.]*$/, '') || 'Picture';

// One file's sheet: resolves to the GIF to add (made here, or the file as
// it is), or null (Cancel).
async function sheet(file, of) {
  const m = await art();
  dlgStyle();
  if (!styled) { document.head.append(h('style', CSS)); styled = true; }
  const S = { st: null, fit: 'pixels', x: 0, y: 0, gif: null, mode: 'fit', anim: false, frames: 1 };
  const what = h('p.muted', { id: 'fit-what' }, `${file.name}: reading…`);
  const seg = (id, items, pick) => {
    const el = h('div.fit-seg', { id, role: 'group' }, items.map(([v, t]) => h('button', { 'data-v': v, onclick: () => pick(v) }, t)));
    el.set = (v) => { for (const b of el.children) b.classList.toggle('on', b.dataset.v === v); };
    return el;
  };
  const mode = seg('fit-mode', [['fit', 'Fit to the panel'], ['asis', 'Add as it is']], (v) => setMode(v));
  const modeRow = h('div.row', { hidden: true }, mode);
  const cv = h('canvas', { id: 'fit-src' });
  const win = h('div.fit-win', { id: 'fit-win' });
  const view = h('div.fit-view', { id: 'fit-view' }, cv, win);
  const fits = seg('fit-fit', FITS, (v) => { S.fit = v; restage(true); });
  const cap = h('p.cap', { id: 'fit-cap' });
  const panel = h('canvas.panel', { id: 'fit-panel' });
  const edit = h('div', { id: 'fit-edit', hidden: true }, view,
    h('div.row', fits, h('button.sm', { id: 'fit-auto', onclick: () => { S.fit = S.st.autoFit(); restage(true); } }, 'Auto')), cap, h('div.fit-pv', panel));
  const asis = h('p.cap', { id: 'fit-asis', hidden: true });
  const next = h('button.pri', { id: 'fit-next', disabled: true, onclick: () => finish() }, 'Next');
  const card = h('div.card', h('h2', 'Fit it to the panel' + of), what, modeRow, edit, asis,
    h('div.row.dl-foot', h('button', { id: 'fit-cancel', onclick: () => done(null) }, 'Cancel'), next));
  const el = h('div.dlg', { id: 'fit', role: 'dialog', 'aria-modal': 'true' }, card);
  let done, alive = true;
  const result = new Promise((res) => { done = (v) => { alive = false; el.remove(); removeEventListener('keydown', esc); pv.dispose(); S.st?.free(); S.st = null; res(v); }; });
  const esc = (e) => e.key === 'Escape' && done(null);
  addEventListener('keydown', esc);
  document.body.append(el);
  const pv = new PanelPreview(panel, { onError: (e) => { cap.textContent = 'preview: ' + e; } });
  pv.p.setTransition('none', 0);

  // Dragging the frame; a tap elsewhere on the picture moves it there.
  let drag = null;
  function toStage(e) { const r = cv.getBoundingClientRect(); return [(e.clientX - r.left) * (S.st.width / r.width), (e.clientY - r.top) * (S.st.height / r.height)]; }
  win.addEventListener('pointerdown', (e) => { e.preventDefault(); e.stopPropagation(); win.setPointerCapture(e.pointerId); drag = { p: toStage(e), x: S.x, y: S.y }; });
  win.addEventListener('pointermove', (e) => {
    if (!drag) return;
    const [sx, sy] = toStage(e);
    S.x = Math.round(drag.x + sx - drag.p[0]); S.y = Math.round(drag.y + sy - drag.p[1]);
    frame();
  });
  win.addEventListener('pointerup', () => { if (drag) { drag = null; render(); } });
  view.addEventListener('click', (e) => {
    if (!S.st || e.target === win) return;
    const [sx, sy] = toStage(e);
    S.x = Math.round(sx - W / 2); S.y = Math.round(sy - H / 2);
    frame(); render();
  });

  const bytes = new Uint8Array(await file.arrayBuffer());
  const size = gifSize(bytes);
  if (size) {
    // A GIF: as it is, always possible.
    modeRow.hidden = false;
    asis.textContent = `Added unchanged: the panel shows the middle 128 × 32 of its ${size[0]} × ${size[1]}, every frame as it is.`;
  }
  const ok = await load(bytes, size);
  if (!alive) return result;
  if (!ok) {
    // Nothing the browser or the art module can read: a GIF may still go on as it is.
    if (size) setMode('asis');
    next.disabled = !size;
    return result;
  }
  setMode(S.capped ? 'asis' : 'fit');
  return result;

  async function load(b, gif) {
    try {
      if (gif && m.gifFrames(b) > 1) {
        const st = m.Studio.fromGif(b);
        const all = st.inFile;
        if (!(await run(() => st.decode(2), () => alive, () => { what.textContent = `${file.name}: reading frames… ${st.sourceFrames.toLocaleString()} of ${all.toLocaleString()}`; }))) { st.free(); return false; }
        const [ow, oh, ww, wh, reduce, damaged, capped] = st.info();
        Object.assign(S, { st, anim: true, frames: st.sourceFrames, capped: !!capped });
        what.textContent = `${file.name}: ${ow} × ${oh} · ${st.sourceFrames.toLocaleString()} frames`
          + (reduce > 1 ? ` · fitted at ${ww} × ${wh} to fit in memory` : '')
          + (capped ? ` · fitting keeps only the first ${st.sourceFrames.toLocaleString()} of ${all.toLocaleString()} frames: add it as it is to keep them all` : '')
          + (damaged ? ' · the end of the file is damaged' : '');
      } else {
        let bm;
        try { bm = await createImageBitmap(file, { colorSpaceConversion: 'none', premultiplyAlpha: 'none' }); } catch {
          what.textContent = `${file.name}: not a picture this browser can read`;
          return false;
        }
        const { width: ow, height: oh } = bm;
        const k = Math.min(1, Math.sqrt(MAX_PX / (ow * oh)));
        const w = Math.max(1, Math.round(ow * k)), hh = Math.max(1, Math.round(oh * k));
        const c = h('canvas', { width: w, height: hh });
        const g = c.getContext('2d', { willReadFrequently: true });
        g.imageSmoothingQuality = 'high';
        g.drawImage(bm, 0, 0, w, hh);
        bm.close?.();
        if (!alive) return false;
        S.st = new m.Studio(g.getImageData(0, 0, w, hh).data, w, hh);
        what.textContent = `${file.name}: ${ow} × ${oh}`;
      }
    } catch (e) {
      what.textContent = `${file.name}: it could not be read (${e.message || e})`;
      return false;
    }
    if (!alive) return false;
    S.fit = S.st.autoFit();
    return true;
  }

  function setMode(v) {
    S.mode = v;
    mode.set(v);
    edit.hidden = v !== 'fit' || !S.st;
    asis.hidden = v !== 'asis';
    next.disabled = v === 'fit' && !S.st;
    next.textContent = v === 'asis' ? 'Next' : 'Fit and next';
    el.dataset.mode = v;
    if (v === 'fit' && S.st && !S.staged) restage(true);
  }

  // The stage for the fit; `auto` also puts the frame over the busiest part.
  function restage(auto) {
    const st = S.st;
    st.prepare(S.fit, false);
    S.staged = true;
    if (auto) [S.x, S.y] = st.autoWindow('still');
    cv.width = st.width; cv.height = st.height;
    cv.getContext('2d').putImageData(new ImageData(new Uint8ClampedArray(st.pixels().buffer), st.width, st.height), 0, 0);
    const k = st.scale;
    cap.textContent = `${st.width} × ${st.height}` + (S.fit === 'fit' ? ' · the whole picture, fitted to the panel' : k > 1 ? ` · it was enlarged ${k}×: using its real pixels` : '')
      + ' · drag the frame to choose what shows';
    fits.set(S.fit);
    frame();
    render();
  }

  function frame() {
    const st = S.st, w = st.width, hh = st.height;
    S.x = Math.max(0, Math.min(S.x, w - W)); S.y = Math.max(0, Math.min(S.y, hh - H));
    const pct = (v, n) => (100 * v) / n + '%';
    // (As wide as the sheet, and no taller than about half the window.)
    const room = Math.min(card.clientWidth - 48 || 600, (innerHeight * 0.45 * w) / hh), z = fitScale(room, w);
    view.style.width = (z ? (w * z) / devicePixelRatio : room) + 'px';
    Object.assign(win.style, { left: pct(S.x, w), top: pct(S.y, hh), width: pct(Math.min(W, w), w), height: pct(Math.min(H, hh), hh) });
    Object.assign(el.dataset, { fit: S.fit, x: S.x, y: S.y, stage: `${w}x${hh}`, frames: S.frames, animated: S.anim ? 1 : '' });
  }

  // The preview: a picture's GIF as it will be written; an animation's first frame.
  function render() {
    const st = S.st;
    if (S.anim) {
      st.look(S.x, S.y, false, 100, 0);
      pv.p.load('fit.gif', st.frameGif(0));
    } else {
      S.gif = st.render('still', S.x, S.y, 40, 1500, false, 100, 0);
      pv.p.load('fit.gif', S.gif);
    }
    el.dataset.state = 'ready';
  }

  async function finish() {
    if (S.mode === 'asis') { done(file); return; }
    const st = S.st;
    next.disabled = true;
    el.dataset.state = 'saving';
    let out = S.gif;
    if (S.anim) {
      st.look(S.x, S.y, false, 100, 0);
      st.setEdit(0, st.sourceFrames - 1, 100, false, false, new Uint32Array(0));
      st.saveBegin(st.info()[7]);
      if (!(await run(() => st.saveStep(8), () => alive, () => { const [d, n] = st.saveProgress(); clear(cap, `Writing the GIF… ${Math.round((100 * d) / n)}%`); }))) return;
      out = st.saved();
    }
    done(new File([out], stem(file.name) + '.gif', { type: 'image/gif' }));
  }
}

// A Formula's loop as a GIF (js/formulas.js "Save as GIF…"): `n` frames of
// 128 × 32 RGB drawn by `frame(k)`, `delay` ms each (`cut`: not the whole
// loop), written as the art module writes any animation for the panel.
export async function framesGif({ n, delay, cut, frame }) {
  const m = await art();
  const st = m.Studio.fromFrames(frame(0), n, delay, cut);
  try {
    await run(() => st.push(frame(st.sourceFrames)));
    st.prepare(st.autoFit(), false);
    const [x, y] = st.autoWindow('still');
    st.look(x, y, false, 100, 0);
    st.setEdit(0, st.sourceFrames - 1, 100, false, false, new Uint32Array(0));
    st.saveBegin(0);
    await run(() => st.saveStep(8));
    return st.saved();
  } finally { st.free(); }
}
