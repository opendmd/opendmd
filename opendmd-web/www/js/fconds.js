// The editor's built-in inputs (js/formulas.js imports it for a formula that
// reads one: `daylight`, `sun.alt`, `replay.game`, `since(…)`, `was(…)`):
// what the preview draws with, never the defaults' place (previews,
// thumbnails and the gallery keep the defaults). Time travel: a time of day
// and a date (Play: a day in 30 s) drive hour, minute, second and the sky
// (pkg-psi sky(): the panel's own integer maths) at the panel's location
// (Settings), else this browser's if asked, else 51.5°N 0°. Changes: Start a
// game, Back to the menu, Night on/off, and a slider for each input read;
// each is a change as the panel makes one (`was` the old value), its `since`
// kept here, growing with real time. Its own size line (tools/web-size.py).

import { app } from './state.js';
import { h, clear, toast } from './util.js';
import { control } from './psikit.js';

const SKY = ['daylight', 'sun.alt', 'moon.phase', 'season', 'yearday'];
const UNSET = -2147483648, NEVER = 32767 * 65536, DAY = 86400000;
const Q = (v) => Math.round(v * 65536);
const two = (n) => String(n).padStart(2, '0');
const r2 = (v) => Math.round(v * 100) / 100;

// The panel in `box`: `keys(list, clock)` for each compile (pkg-psi
// builtins(), and whether it reads the clock), `reset()` for another
// formula, `w` what formulas.js draws with (`on`, `apply(P)`, `hms`).
export function conditions(P, box) {
  let keys = [], ov = new Map(), vals = {}, clock = 0, last = 0;
  const tt = { on: false, play: false, ms: Date.now() };
  let where = null;
  const place = () => {
    const c = app.config;
    if (c?.lat != null && c?.lon != null) return [c.lat, c.lon, 'your panel’s location'];
    return where || [51.5, 0, '51.5°N 0° (London): no location set'];
  };
  const reads = (n) => keys.some((k) => k.name.startsWith(n));

  const w = {
    get on() { return tt.on || ov.size > 0; },
    hms: null,
    apply(P) {
      const now = performance.now();
      if (tt.play) {
        tt.ms += (now - (last || now)) * 2880;
        if (Math.floor(tt.ms / 60000) !== Math.floor(shown / 60000)) say();
      }
      last = now;
      P.worldReset();
      let s = null;
      if (tt.on) {
        const [lat, lon] = place();
        s = P.sky(tt.ms / 1000, -new Date(tt.ms).getTimezoneOffset() * 60, lat, lon);
        w.hms = [s[5], s[6], s[7]];
      } else w.hms = null;
      for (const k of keys) {
        const i = SKY.indexOf(k.name), o = ov.get(k.name);
        if (s && i >= 0) { if (s[i] !== UNSET) P.worldSet(k.id, s[i], s[i], NEVER); continue; }
        if (o) P.worldSet(k.id, Q(o.v), Q(o.was), Math.min(NEVER, Math.round((now - o.at) * 65.536)));
      }
    },
  };

  // A change, as the panel makes one: `was` the value it had, `since` from now.
  const k0 = (n) => keys.find((k) => k.name === n);
  function change(n, v) {
    const o = ov.get(n), had = o ? o.v : k0(n)?.default ?? 0;
    if (had === v && o) return;
    ov.set(n, { v, was: had, at: performance.now() });
    vals[n] = v;
  }
  const many = (m) => { for (const [n, v] of Object.entries(m)) change(n, v); sliders(); };

  // Time travel.
  const on = h('input', { type: 'checkbox', id: 'psi-tt', onchange: () => { tt.on = on.checked; if (!tt.on) tt.play = false; tt.ms = tt.on ? Date.now() : tt.ms; last = 0; show(); } });
  const time = h('input', { type: 'range', id: 'psi-tt-time', min: 0, max: 1439, 'aria-label': 'Time of day' });
  const date = h('input', { type: 'range', id: 'psi-tt-date', min: 0, max: 365, 'aria-label': 'Date' });
  const tOut = h('output.mono', { id: 'psi-tt-t' }), dOut = h('output.mono', { id: 'psi-tt-d' });
  const play = h('button.sm', { id: 'psi-tt-play', onclick: () => { tt.play = !tt.play; last = 0; say(); } });
  const at = h('span.cap', { id: 'psi-tt-where' });
  const here = h('button.sm.txt', { id: 'psi-tt-here', onclick: locate }, 'Use this browser’s location');
  const ttBox = h('div', { id: 'psi-tt-box', hidden: true },
    h('label.psi-sl', h('span', 'Time'), time, tOut), h('label.psi-sl', h('span', 'Date'), date, dOut), h('div.row', play, at, here));
  const ttRow = h('div', h('label', on, 'Time travel: the sun, the moon and the clock at another moment'), ttBox);
  // The moment from the scrubbers: this year's day `date`, minute `time`, local.
  const moment = () => { const d = new Date(new Date().getFullYear(), 0, 1 + +date.value); return d.getTime() + +time.value * 60000; };
  time.oninput = date.oninput = () => { tt.ms = moment(); say(); };
  let shown = 0;
  function say() {
    const d = new Date(tt.ms), y0 = new Date(d.getFullYear(), 0, 1);
    shown = tt.ms;
    time.value = d.getHours() * 60 + d.getMinutes();
    date.value = Math.round((new Date(d.getFullYear(), d.getMonth(), d.getDate()) - y0) / DAY);
    tOut.textContent = `${two(d.getHours())}:${two(d.getMinutes())}`;
    dOut.textContent = d.toLocaleDateString(undefined, { day: 'numeric', month: 'short' });
    play.textContent = tt.play ? 'Pause' : 'Play';
    const p = place();
    at.textContent = 'At ' + p[2];
    here.hidden = app.config?.lat != null || !navigator.geolocation;
  }
  // This browser's location, rounded to 0.01° here (~1 km), only when asked.
  function locate() {
    navigator.geolocation.getCurrentPosition((p) => { where = [r2(p.coords.latitude), r2(p.coords.longitude), 'this browser’s location']; say(); },
      () => toast(isSecureContext ? 'Couldn’t get this browser’s location' : 'Browsers share their location only with secure (https) pages: set it in Settings › Schedule instead', 'bad'), { timeout: 15000, maximumAge: 600000 });
  }

  // Changes: the buttons for the inputs it reads, and a slider for each.
  const acts = h('div.row.acts', { id: 'psi-cond-acts' });
  const sl = h('div.psi-sls', { id: 'psi-cond-sl' });
  function sliders() {
    clear(sl, keys.map((k) => {
      const sky = tt.on && SKY.includes(k.name);
      const c = { slot: k.name, label: k.name, kind: 'slider', lo: k.lo, hi: k.hi, default: k.default };
      const el = control(c, vals, () => { change(k.name, vals[k.name]); }, 'psi-cond-' + k.name.replace('.', '-'));
      const r = el.querySelector('input');
      if (k.whole) r.step = 1;
      r.disabled = sky;
      if (sky) el.title = 'Time travel sets it';
      return el;
    }));
  }
  function show() {
    const night = ov.get('night')?.v === 1;
    clear(acts,
      reads('replay.') && [
        h('button.sm', { id: 'psi-cond-game', onclick: () => many({ 'replay.playing': 1, 'replay.platform': 1 + Math.floor(Math.random() * 40), 'replay.game': r2(Math.random()) }) }, 'Start a game'),
        h('button.sm', { id: 'psi-cond-menu', onclick: () => many({ 'replay.playing': 0, 'replay.platform': 0, 'replay.game': 0 }) }, 'Back to the menu')],
      reads('night') && h('button.sm', { id: 'psi-cond-night', onclick: () => { many({ night: night ? 0 : 1 }); show(); } }, night ? 'Night off' : 'Night on'),
      h('button.sm.txt', { id: 'psi-cond-reset', onclick: () => { ov.clear(); vals = {}; tt.on = on.checked = tt.play = false; show(); } }, 'Back to the defaults'));
    ttRow.hidden = !clock && !keys.some((k) => SKY.includes(k.name));
    ttBox.hidden = !tt.on;
    say();
    sliders();
  }

  clear(box, h('div.psi-conds', { id: 'psi-cond' },
    h('h3', 'Try it out'), h('p.cap', { id: 'psi-cond-reads' }), ttRow, acts, sl,
    h('p.cap', 'Only this preview: the gallery and thumbnails show the defaults, and the panel its own world.')));
  return {
    w,
    keys(list, clk) {
      keys = list;
      clock = clk;
      box.querySelector('#psi-cond-reads').textContent = 'It reads ' + list.map((k) => [k.reads & 1 && k.name, k.reads & 2 && `since(${k.name})`, k.reads & 4 && `was(${k.name})`].filter(Boolean).join(', ')).join(', ');
      for (const n of ov.keys()) if (!k0(n) && !n.startsWith('replay.')) ov.delete(n);
      show();
    },
    reset() { ov.clear(); vals = {}; tt.on = on.checked = tt.play = false; },
  };
}
