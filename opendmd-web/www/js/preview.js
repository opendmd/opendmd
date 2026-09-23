// Panel-look previews. Each <canvas> gets one wasm `Preview`, which owns the
// LED renderer for its life and swaps only the file underneath it. All
// previews share one look (gain/bloom/gamma/flat), kept in localStorage, and
// one requestAnimationFrame loop that ticks only the previews that are on
// screen.

import { Preview, defaultBloom } from '../pkg/opendmd_web.js';
import { h, store } from './util.js';

const DEFAULT_LOOK = { gamma: 2.2, gain: 1, bloom: defaultBloom(), flat: false };
export const look = { ...DEFAULT_LOOK, ...store.get('look', {}) };
const live = new Set();
let raf = 0;

function loop() {
  raf = 0;
  for (const p of live) if (p.canvas.offsetParent !== null) p.tick();
  if (live.size) raf = requestAnimationFrame(loop);
}

export class PanelPreview {
  // The LED scale is fixed at construction from the canvas's on-screen width
  // (device pixels / 128, clamped to 6..10: below 6 there is no room for a
  // dot and its bloom, above 10 costs frame time for nothing).
  constructor(canvas, { onDwellEnd, onError } = {}) {
    this.canvas = canvas;
    const css = canvas.getBoundingClientRect().width || 900;
    canvas.width = 128 * Math.max(6, Math.min(10, Math.round((css * devicePixelRatio) / 128)));
    this.p = new Preview(canvas, look.gamma, look.gain, look.bloom, look.flat);
    this.onDwellEnd = onDwellEnd;
    this.onError = onError;
    this.token = 0;
    live.add(this);
    if (!raf) raf = requestAnimationFrame(loop);
  }

  tick() {
    const r = this.p.frame();
    if (r === 2) this.onDwellEnd?.();
    else if (r < 0) this.onError?.(this.p.error);
  }

  // Fetch-and-play with last-caller-wins: a slow fetch for an old path never
  // replaces a newer one. `get` returns the file's bytes.
  async show(path, get) {
    const t = ++this.token;
    const bytes = await get(path);
    if (t !== this.token) return false;
    this.p.load(path, bytes);
    return true;
  }
  clear() { this.token++; this.p.clear(); }
  get path() { return this.p.path; }
  setPaused(v) { this.p.setPaused(v); }
  get paused() { return this.p.paused; }
  // The panel was `elapsed` ms into this file at local time `at`
  // (performance.now()); returns the drift before correction (NaN: nothing loaded).
  syncTo(elapsed, at) { return this.p.syncTo(elapsed, at); }
  get synced() { return this.p.synced; }
  setDwell(min, max) { this.p.setDwell(min, max); }
  apply() { this.p.setLook(look.gamma, look.gain, look.bloom, look.flat); }
}

export function setLook(patch) {
  Object.assign(look, patch);
  store.set('look', look);
  for (const p of live) p.apply();
}

// The collapsible "panel look" section: the simulator's --led-gain, --bloom
// and --gamma, plus flat (nearest-neighbour, no LED simulation).
// `extra` (optional) goes at the end of the row: the Now view's drift readout.
export function lookControls(extra) {
  const slider = (key, min, max, step) => {
    const out = h('output.mono', look[key].toFixed(2));
    const input = h('input', {
      type: 'range', min, max, step, value: look[key], id: 'look-' + key,
      oninput: () => { out.textContent = (+input.value).toFixed(2); setLook({ [key]: +input.value }); },
    });
    return h('label', key, input, out);
  };
  const flat = h('input', { type: 'checkbox', id: 'look-flat', checked: look.flat, onchange: () => setLook({ flat: flat.checked }) });
  return h('details.card', h('summary', 'Panel look'),
    h('div.row', slider('gain', 0, 4, 0.05), slider('bloom', 0, 1, 0.01), slider('gamma', 0.5, 4, 0.05),
      h('label', flat, 'flat'),
      h('button.sm', { onclick: (e) => { setLook(DEFAULT_LOOK); const d = lookControls(extra); d.open = true; e.target.closest('details').replaceWith(d); } }, 'reset'), extra));
}
