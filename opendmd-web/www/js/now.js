// Now: what the panel is doing. On a device, /api/status every 2 s, the
// remote control (skip, pause, brightness, send a path to the panel with
// POST /api/play) and a panel-look preview of the
// file on screen, fetched once per change and played here — not frame-synced
// with the panel. In local mode, a simulator over the picked folder that
// plays one file at a time with the firmware's dwell.

import { app } from './state.js';
import { PanelPreview, lookControls } from './preview.js';
import { h, clear, on, store, debounce, toast, bytes, uptime, label } from './util.js';

const POLL_MS = 2000;

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
  const cap = h('div.cap', 'preview — not frame-synced');
  const title = h('h2', { id: 'now-title' }, '…');
  const meta = h('div.muted.mono', { id: 'now-path' });
  const ord = h('span.muted', { id: 'now-ord' });
  const pauseBtn = h('button', { id: 'now-pause', onclick: togglePause }, 'Pause');
  const bright = h('input', { type: 'range', min: 1, max: 255, step: 1, id: 'now-bright', value: app.config?.brightness ?? 96 });
  const brightOut = h('output.mono', bright.value);
  const grid = h('div.grid', { id: 'now-stats' });
  const conn = h('span', { id: 'now-conn' });
  const playPath = h('input', { type: 'text', id: 'now-play-path', placeholder: '/gifs/… or /odm/… (card path)', onkeydown: (e) => e.key === 'Enter' && send() });

  el.append(
    h('div.card', canvas, h('div.row.sp', cap, h('label.cap', follow, 'preview follows the panel'))),
    h('div.card',
      h('div.row.sp', h('div.grow', title, h('div', { id: 'now-tags' }), meta), h('div', ord, ' ', conn)),
      h('div.row', { style: 'margin-top:10px' },
        h('button.pri', { id: 'now-skip', onclick: skip }, 'Skip'), pauseBtn,
        h('label', 'Brightness', bright, brightOut)),
      h('div.row', { style: 'margin-top:10px' }, h('div.grow', playPath), h('button', { id: 'now-play', onclick: send }, 'Send to panel'))),
    h('div.card', h('h3', { style: 'margin-top:0' }, 'Health'), grid),
    lookControls());

  const preview = new PanelPreview(canvas, { onError: (m) => { cap.textContent = 'preview: ' + m; } });
  let shownPath = '';
  let timer = 0;
  let failures = 0;

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

  // The preview prefers the file's bucketed .odm when filelist.odm.txt says
  // there is one: fetching from gifs/Consoles costs the device a ~330 ms
  // directory scan that the panel feels; a bucket costs ~6 ms.
  async function follow_(path) {
    if (!follow.checked || !path || path === shownPath) return;
    shownPath = path;
    await app.odmIndex();
    const fetchPath = app.hasOdm(path) ? app.wasm.odmPathFor(path) : path;
    cap.textContent = 'preview — not frame-synced · loading…';
    try {
      if (await preview.show(fetchPath, (p) => app.src.readBytes(p))) {
        cap.textContent = `preview — not frame-synced · ${fetchPath.endsWith('.odm') ? '.odm' : 'GIF'}`;
      }
    } catch (e) { cap.textContent = 'preview: ' + e.message; }
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
    const set = s.manifest.replace(/^\/sdcard/, '');
    clear(grid,
      stat('uptime', uptime(s.uptime_secs)),
      stat(`${s.wifi} · ${s.hostname}.local`, s.ip || '–'),
      stat('heap free', bytes(s.heap.free)),
      stat('heap floor (sampled)', bytes(s.heap.floor), s.heap.floor < 20000 ? '.bad' : ''),
      stat('refresh', s.refresh_hz + ' Hz'),
      stat('deadline misses', s.deadline_misses.toLocaleString()),
      stat('worst lateness', s.worst_lateness_ms + ' ms'),
      stat('files started', s.gifs_started.toLocaleString()),
      stat('manifest', set),
      stat('card', s.sd.mounted ? bytes(s.sd.capacity_bytes) : 'not mounted', s.sd.mounted ? '' : '.bad'));
    follow_(np.path);
  }

  async function poll() {
    if (document.hidden) return;
    try {
      render(await app.refreshStatus());
      failures = 0;
      conn.className = 'ok';
      conn.textContent = '● live';
    } catch (e) {
      conn.className = 'bad';
      conn.textContent = ++failures > 1 ? '● offline' : '● retrying';
    }
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
  const manifests = [...src.files.keys()].filter((p) => /^\/filelist\.[^/]*$|^\/sets\/[^/]+\.txt$/.test(p));
  for (const m of manifests) listSel.append(h('option', { value: m }, m));
  listSel.append(h('option', { value: '' }, 'every GIF / .odm in the folder'));

  async function play() {
    let text;
    if (listSel.value) text = await src.readText(listSel.value);
    else text = [...src.files.keys()].filter((p) => /\.(gif|odm)$/i.test(p)).sort().join('\n');
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
    await preview.show(path, (p) => src.readBytes(p));
  }

  return { show() {}, hide() {} };
}

