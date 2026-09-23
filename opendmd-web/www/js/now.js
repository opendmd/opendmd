// Now: what the panel is doing. On a device, /api/status every 500 ms while
// this view is up, the remote control (skip, pause, brightness, send a path
// to the panel with POST /api/play) and a panel-look preview of the file on
// screen, fetched once per change and kept in step with the panel by clock
// (elapsed_ms at the device's t_ms, offset estimated NTP-style; CLAUDE.md,
// "The Now preview is in step"). In local mode, a simulator over the picked
// folder that plays one file at a time with the firmware's dwell.

import { app, playlistLabel } from './state.js';
import { PanelPreview, lookControls } from './preview.js';
import { h, clear, on, store, debounce, toast, bytes, uptime, label } from './util.js';

const POLL_MS = 500;
const CAP = 'preview · in step with the panel';

function tags(e) {
  return [e.platform && h('span.tag', e.platform), e.category && h('span.tag', e.category),
    ...e.flags.map((f) => h('span.tag', f))];
}

export async function mount(el) {
  return app.device ? mountDevice(el) : mountLocal(el);
}

function mountDevice(el) {
  const canvas = h('canvas.panel', { id: 'now-panel' });
  const follow = h('input', { type: 'checkbox', id: 'now-follow', checked: store.get('follow', true) });
  const cap = h('div.cap', { id: 'now-cap' }, CAP);
  const drift = h('span.mono.muted', { id: 'now-drift' });
  const title = h('h2', { id: 'now-title' }, '…');
  const meta = h('div.muted.mono', { id: 'now-path' });
  const ord = h('span.muted', { id: 'now-ord' });
  const pauseBtn = h('button', { id: 'now-pause', onclick: togglePause }, 'Pause');
  const bright = h('input', { type: 'range', min: 1, max: 255, step: 1, id: 'now-bright', value: app.config?.brightness ?? 96 });
  const brightOut = h('output.mono', bright.value);
  const grid = h('div.grid', { id: 'now-stats' });
  const conn = h('span', { id: 'now-conn' });
  const playPath = h('input', { type: 'text', id: 'now-play-path', placeholder: '/gifs/… (card path)', onkeydown: (e) => e.key === 'Enter' && send() });

  el.append(
    h('div.card', canvas, h('div.row.sp', cap, h('label.cap', follow, 'preview follows the panel'))),
    h('div.card',
      h('div.row.sp', h('div.grow', title, h('div', { id: 'now-tags' }), meta), h('div', ord, ' ', conn)),
      h('div.row', { style: 'margin-top:10px' },
        h('button.pri', { id: 'now-skip', onclick: skip }, 'Skip'), pauseBtn,
        h('label', 'Brightness', bright, brightOut)),
      h('div.row', { style: 'margin-top:10px' }, h('div.grow', playPath), h('button', { id: 'now-play', onclick: send }, 'Send to panel'))),
    h('div.card', h('h3', { style: 'margin-top:0' }, 'Health'), grid),
    lookControls(drift));

  const preview = new PanelPreview(canvas, { onError: (m) => { cap.textContent = 'preview: ' + m; } });
  let shownPath = '';
  let timer = 0;
  let failures = 0;
  let clocks = [];   // [{rtt, off}]: off = device ms - local ms
  let last = null;   // the last status sample: {path, elapsed, t, frame, rate} (device clock)
  let base = null;   // this file's first sample, for the rate its position runs at

  follow.onchange = () => { store.set('follow', follow.checked); if (follow.checked) render(app.status); else { preview.clear(); shownPath = ''; } };
  const sendBright = debounce(async (v) => {
    try { await app.src.putConfig({ brightness: v }); if (app.config) app.config.brightness = v; } catch (e) { toast('brightness: ' + e.message, 'bad'); }
  }, 300);
  bright.oninput = () => { brightOut.textContent = bright.value; sendBright(+bright.value); };
  on('config', (c) => { if (document.activeElement !== bright) { bright.value = c.brightness; brightOut.textContent = c.brightness; } });

  async function skip() {
    try { await app.src.skip(); setTimeout(poll, 400); } catch (e) { toast(e.message, 'bad'); }
  }
  async function send() {
    const p = playPath.value.trim();
    if (p && await app.play(p)) setTimeout(poll, 400);
  }
  async function togglePause() {
    try { const r = await app.src.pause(); app.status.paused = r.paused; render(app.status); } catch (e) { toast(e.message, 'bad'); }
  }

  // The preview fetches the file's prepared copy first (app.previewBytes):
  // fetching from gifs/Consoles costs the device a ~330 ms directory scan
  // that the panel feels; a copy's small directory costs ~6 ms.
  async function follow_(path) {
    if (!follow.checked || !path || path === shownPath) return;
    shownPath = path;
    cap.textContent = 'preview — not frame-synced · loading…';
    cap.textContent = 'preview · loading…';
    try {
      if (await preview.show(path, (p) => app.previewBytes(p))) { cap.textContent = CAP; sync(); poll(); }
    } catch (e) { cap.textContent = 'preview: ' + e.message; }
  }

  // One status sample, requested at local t0 and answered at t1.
  function clock(s, t0, t1) {
    if (typeof s.t_ms !== 'number') { last = null; return; }
    const off = s.t_ms - (t0 + t1) / 2;
    // A reboot restarts the device clock: forget the old samples. (Not an
    // offset jump: one reply stuck behind a file transfer has a big one.)
    if (clocks.length && s.t_ms + 5000 < clocks.at(-1).t) clocks = [];
    clocks.push({ rtt: t1 - t0, off, t: s.t_ms });
    if (clocks.length > 8) clocks.shift();
    const np = s.now_playing;
    // The panel's position runs a little slower than its clock (10 ms
    // sleep ticks); measure how much from its own reports, over >= 2 s.
    const key = `${np.ordinal} ${np.path} ${s.paused}`;
    if (base?.key !== key) base = { key, t: s.t_ms, e: np.elapsed_ms };
    const span = s.t_ms - base.t;
    const rate = span >= 2000 ? Math.min(1.1, Math.max(0.5, (np.elapsed_ms - base.e) / span)) : 1;
    last = { path: np.path, elapsed: np.elapsed_ms, t: s.t_ms, frame: np.frame, rate };
  }

  // Put the preview in step with the last sample, if it is of this file.
  function sync() {
    if (!last || !last.frame || preview.path !== last.path) return;
    const best = clocks.reduce((a, b) => (b.rtt < a.rtt ? b : a));
    const d = preview.syncTo(last.elapsed, last.t - best.off, last.rate);
    if (Number.isNaN(d)) return;
    // data-*: for tools/web-smoke.py (k = syncs of this file so far).
    const ds = drift.dataset;
    ds.k = ds.path === last.path ? +ds.k + 1 : 1;
    Object.assign(ds, { path: last.path, drift: d.toFixed(1), rtt: best.rtt.toFixed(1), rate: last.rate.toFixed(4), paused: preview.paused });
    drift.textContent = `drift ${Math.round(d)} ms · rtt ${Math.round(best.rtt)} ms`;
  }

  function stat(name, value, cls = '') { return h('div.stat', h('b' + cls, value), h('span', name)); }

  function render(s) {
    if (!s) return;
    const np = s.now_playing;
    const e = app.wasm.parsePath(np.path || '');
    title.textContent = np.path ? label(e) : '(nothing playing)';
    clear(el.querySelector('#now-tags'), np.path ? tags(e) : null);
    meta.textContent = np.path;
    ord.textContent = `#${np.ordinal} · ${s.entries.toLocaleString()} entries`;
    pauseBtn.textContent = s.paused ? 'Resume' : 'Pause';
    const set = playlistLabel(s.manifest);
    clear(grid,
      stat('uptime', uptime(s.uptime_secs)),
      stat(`${s.wifi} · ${s.hostname}.local`, s.ip || '–'),
      stat('heap free', bytes(s.heap.free)),
      stat('heap floor (sampled)', bytes(s.heap.floor), s.heap.floor < 20000 ? '.bad' : ''),
      stat('refresh', s.refresh_hz + ' Hz'),
      stat('deadline misses', s.deadline_misses.toLocaleString()),
      stat('worst lateness', s.worst_lateness_ms + ' ms'),
      stat('files started', s.gifs_started.toLocaleString()),
      stat('playlist', set),
      stat('card', s.sd.mounted ? bytes(s.sd.capacity_bytes) : 'not mounted', s.sd.mounted ? '' : '.bad'));
    if (preview.paused !== s.paused) preview.setPaused(s.paused);
    follow_(np.path);
  }

  // A failed poll leaves the preview playing on its last sync. One at a
  // time: the device has one httpd task, and a reply queued behind the
  // preview's file fetch must not have others piling up behind it.
  let busy = false;
  async function poll() {
    if (document.hidden || busy) return;
    busy = true;
    try {
      const t0 = performance.now();
      const s = await app.refreshStatus();
      clock(s, t0, performance.now());
      render(s);
      sync();
      if (failures && preview.path) cap.textContent = CAP;
      failures = 0;
      conn.className = 'ok';
      conn.textContent = '● live';
    } catch (e) {
      conn.className = 'bad';
      conn.textContent = ++failures > 1 ? '● offline' : '● retrying';
      if (preview.path) cap.textContent = 'preview · panel not reachable';
    } finally { busy = false; }
  }

  render(app.status);
  return {
    show() { poll(); clearInterval(timer); timer = setInterval(poll, POLL_MS); },
    hide() { clearInterval(timer); },
  };
}

function mountLocal(el) {
  const canvas = h('canvas.panel', { id: 'now-panel' });
  const title = h('h2', { id: 'now-title' }, 'Local simulator');
  const line = h('div.muted.mono', { id: 'now-path' });
  const listSel = h('select', { id: 'sim-list' });
  const plats = h('input', { type: 'text', id: 'sim-platforms', placeholder: 'MD,SNES — blank for all', size: 18 });
  const shuffle = h('input', { type: 'checkbox', id: 'sim-shuffle' });
  const seed = h('input', { type: 'number', id: 'sim-seed', value: 7, min: 0, style: 'width:90px' });
  const pauseBtn = h('button', { id: 'now-pause', disabled: true, onclick: () => { preview.setPaused(!preview.paused); pauseBtn.textContent = preview.paused ? 'Resume' : 'Pause'; } }, 'Pause');
  const nextBtn = h('button', { id: 'now-skip', disabled: true, onclick: () => next() }, 'Next');
  let list = [];
  let i = -1;

  el.append(
    h('div.card', canvas, h('div.cap', 'local simulator — one file in memory at a time, the firmware’s 3–30 s dwell')),
    h('div.card', title, line,
      h('div.row', { style: 'margin-top:10px' },
        h('label', 'playlist', listSel), h('label', 'platforms', plats), h('label', shuffle, 'shuffle'), h('label', 'seed', seed),
        h('button.pri', { id: 'sim-play', onclick: play }, 'Play'), nextBtn, pauseBtn)),
    lookControls());

  const preview = new PanelPreview(canvas, {
    onDwellEnd: () => next(),
    onError: (m) => { toast(m, 'bad'); next(); },
  });
  preview.setDwell(3, 30);

  const src = app.src;
  const manifests = [...src.files.keys()].filter((p) => /^\/filelist\.[^/]*$|^\/sets\/[^/]+\.txt$/.test(p) && !/\/filelist\.odm\.|\.(idx|part|tmp|bak)$/.test(p));
  for (const m of manifests) listSel.append(h('option', { value: m }, playlistLabel(m)));
  listSel.append(h('option', { value: '' }, 'every GIF in the folder'));

  async function play() {
    let text;
    if (listSel.value) text = await src.readText(listSel.value);
    else text = [...src.files.keys()].filter((p) => /\.gif$/i.test(p)).sort().join('\n');
    list = app.wasm.filterManifest(text, JSON.stringify({ platforms: plats.value, shuffle: shuffle.checked, seed: +seed.value }));
    if (!list.length) { toast('the filter matched nothing', 'bad'); return; }
    i = -1;
    nextBtn.disabled = pauseBtn.disabled = false;
    next();
  }

  // Skip entries the folder does not have (a manifest can name files that
  // were not extracted), but give up after a full lap.
  async function next(tries = 0) {
    if (!list.length || tries > list.length) return;
    i = (i + 1) % list.length;
    const path = list[i];
    if (!src.files.has(path)) return next(tries + 1);
    const e = app.wasm.parsePath(path);
    title.textContent = label(e);
    line.textContent = `[${i + 1}/${list.length}] ${path}`;
    await preview.show(path, (p) => app.previewBytes(p));
  }

  return { show() {}, hide() {} };
}

