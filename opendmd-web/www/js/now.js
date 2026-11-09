// Now: what the panel is doing. On a device, what is on screen pushed while
// this view is up (GET /api/events, with a full status read at the start,
// on each new file for Recently played and when the playlist or card
// changes), or /api/status polled through the app's scheduler where there
// is no stream (every POLL_MS; the slim `?now=1` between full reads, and
// less often while a file goes to or from the card), the remote control (skip,
// pause, brightness; sending a GIF to the panel is the Library's) and a
// panel-look preview of the file on screen, fetched once per change and
// kept in step with the panel by clock (elapsed_ms at the device's t_ms,
// offset estimated NTP-style; CLAUDE.md, "The Now preview is in step"). In
// local mode, a simulator over the picked folder that plays one file at a
// time with the firmware's dwell.

import { app, playlistLabel, platName } from './state.js';
import { PanelPreview, lookControls } from './preview.js';
import { bannerFor } from './schedule.js';
import { followLine } from './follow.js';
import { Overrides, moreButton, fav } from './overrides.js';
import { h, clear, on, store, debounce, toast, label, niceName, flagName, isFlag, isFormula, brightSlider, drawn } from './util.js';

// The full status every FULL_MS (the card, the schedule, the filter…) and
// whenever Recently played is wanted; the slim one (`?now=1`, one segment:
// the clock and what is on screen) between. Every SLOW_MS while `transfer`
// says a file is going to or from the card: the device serves one request
// at a time, and an upload holds its receive buffers.
const POLL_MS = 500, FULL_MS = 5000, SLOW_MS = 1500;
// The stream: an event at least every 3 s, so none for QUIET_MS is a dead
// one; after a failure it is tried again in RETRY_MS (polling meanwhile),
// after a live one ends in AGAIN_MS.
const QUIET_MS = 10000, RETRY_MS = 30000, AGAIN_MS = 3000;

// "Mega Drive · Consoles · Slide · by pixelkid42": the platform, category
// and flags by name and the author, each once (no "Arcade · Arcade", no
// "Kenney · by kenney"; OTHER is no platform); "Formula" for one, nothing
// for a title card.
const squash = (t) => t.toLowerCase().replace(/[^a-z0-9]/g, '');
function tags(p, e) {
  if (drawn(p)) return isFormula(p) ? 'Formula' : '';
  const by = e.flags.find((f) => !isFlag(f)), seen = new Set();
  return [platName(e.platform), e.category && niceName(e.category), ...e.flags.filter(isFlag).map(flagName), by]
    .filter((t) => t && !seen.has(squash(t)) && seen.add(squash(t))).map((t) => (t === by ? 'by ' + by : t)).join(' · ');
}

// ★: the GIF on screen a favourite in the playlist playing (not a title card or a Formula).
function favButton() {
  const ov = new Overrides(), b = h('button.more', { id: 'now-fav', hidden: true, onclick: () => ov.set(b.p, fav) });
  b.ov = ov;
  b.set = (p, list) => {
    b.hidden = !(b.p = p) || drawn(p) || !list;
    if (list && list != ov.path) ov.load(list);
    const on = ov.get(p)?.w > 1;
    b.textContent = on ? '★' : '☆';
    b.title = b.ariaLabel = `Favourite in “${ov.name}”`;
    b.ariaPressed = on;
  };
  ov.onChange = () => b.set(b.p, ov.path);
  return b;
}

// Edit, as the Library's: a GIF (its original) in Create Art, a Formula in its editor (the saved or example one, else the
// panel's one-line text); [where, go], or null for what the panel draws.
function editFor(p) {
  const q = p?.startsWith('@formula?') && new URLSearchParams(p.slice(9));
  if (q) return q.get('psb') && ['Formulas', async () => {
    // The saved or example Formula that compiles to what the panel plays.
    await (await import('./looks.js')).psi();
    const { path, num } = await import('./psikit.js');
    const f = [...[store.get('psi.mine', [])].flat(), ...await fetch('formulas.json').then((r) => r.json(), () => [])].find((x) => typeof x?.src == 'string' && path({ src: x.src }) == p.split('&')[0]);
    if (!f) { toast('That Formula isn’t saved here: open it where it was made'); return; }
    // (In memory, never in the address: js/formulas.js opens app.psiOpen.)
    app.psiOpen = { src: f.src, name: f.name || 'Formula' };
    for (const k of 'abc') if (num(q.get(k)) != null) app.psiOpen[k] = num(q.get(k));
    app.go('formulas/edit');
  }];
  const e = p?.[0] == '/' && app.wasm.parsePath(p);
  return e && ['Create Art', () => { app.artFrom = { path: p, e }; app.go('create'); }];
}
function editLine() {
  const b = h('button.sm', { id: 'now-edit', hidden: true }), why = h('span.muted', { id: 'now-live' }), el = h('div', { style: 'margin-top:8px' }, b, why);
  el.set = (p, t) => {
    const x = editFor(p);
    b.hidden = !x;
    if (x) [b.textContent, b.onclick] = ['✎ Edit in ' + x[0], x[1]];
    why.textContent = !x && p ? t + ': drawn by the panel itself, not a GIF' : '';
  };
  return el;
}

// Recently played, newest first, `at` on screen; a tap plays one again.
function recentCard(prev, pick, fb) {
  const list = h('div.list', { id: 'now-recent' });
  const card = h('div.card', { hidden: true }, h('h3', { style: 'margin-top:0' }, 'Recently played'), list);
  card.set = (paths, at) => {
    card.hidden = !paths.length;
    prev.disabled = at + 1 >= paths.length;
    clear(list, paths.map((p, k) => {
      const e = app.wasm.parsePath(p);
      const x = editFor(p);
      return h('div' + (k === at ? '.cur' : ''), { onclick: () => pick(p), 'data-path': p }, h('span.grow', label(e)), h('span.muted', tags(p, e)),
        x && h('button.more', { title: 'Edit in ' + x[0], 'aria-label': 'Edit in ' + x[0], onclick: (ev) => { ev.stopPropagation(); x[1](); } }, '✎'),
        fb.ov.path && !drawn(p) && moreButton(fb.ov, p));
    }));
  };
  return card;
}

export async function mount(el) {
  return app.device ? mountDevice(el) : mountLocal(el);
}

function mountDevice(el) {
  const canvas = h('canvas.panel', { id: 'now-panel' });
  // Settings › Panel › Mirror the panel (this browser's choice).
  const mirror = () => store.get('follow', true);
  const cap = h('div.cap', { id: 'now-cap' });
  const drift = h('span.mono.muted', { id: 'now-drift' });
  const title = h('b', { id: 'now-title' }, '…');
  const unfilter = h('button.sm', { id: 'now-unfilter', hidden: true, onclick: () => app.src.putConfig({ platforms: '' }).then(() => { app.refreshConfig(); poll(); }, (e) => toast(e.message, 'bad')) }, 'Clear filter');
  // The file's path: never shown (the title says what it is); kept for tools/web-smoke.py.
  const meta = h('span', { id: 'now-path', hidden: true });
  const followEl = h('span.tag.fl', { id: 'now-follow-line', hidden: true });
  const tagsEl = h('span', { id: 'now-tags' });
  const ord = h('span', { id: 'now-ord' });
  // What plays next, once the display has picked it (`now_playing.next`).
  const nextEl = h('span', { id: 'now-next' });
  const pauseBtn = h('button', { id: 'now-pause', onclick: togglePause }, 'Pause');
  // (Disabled, not hidden, until Recently played has one to go back to:
  // the buttons keep their places.)
  const prevBtn = h('button', { id: 'now-prev', disabled: true, onclick: back }, 'Previous');
  const againBtn = h('button', { id: 'now-again', onclick: () => pick(app.status?.now_playing?.path) }, 'Replay');
  const fb = favButton();
  const rc = recentCard(prevBtn, pick, fb);
  const ed = editLine();
  const bright = brightSlider('now-bright', app.config?.brightness ?? 96);
  bright.ceil(app.config?.max_brightness);
  const conn = h('span.dot', { id: 'now-conn' });
  // A Formula kept on the panel (`now_playing.kept_s`, Formulas › Keep on the panel): how long, and the way back.
  const keptText = h('span.grow', { id: 'now-kept-text' });
  const keptRow = h('div.row', { id: 'now-kept', hidden: true, style: 'margin-top:8px' }, keptText,
    h('button.sm', { id: 'now-unkeep', onclick: skip }, 'Back to the playlist'));

  const banner = h('div.banner', { id: 'now-banner', hidden: true });
  // A card the panel can't read (exFAT, as most cards over 32 GB come): say
  // so, with the way out (js/card.js, imported when first used).
  const sdText = h('span.grow.bad', { id: 'now-sd-text' });
  const sdLine = h('div.row', { id: 'now-sd', hidden: true, style: 'margin-top:12px' }, sdText,
    h('button.sm', { id: 'now-format', onclick: () => import('./card.js').then((m) => m.formatCard()) }, 'Format card…'));
  el.append(
    banner,
    h('div.card', canvas, cap, sdLine,
      h('div.nowt', h('div', title, ' ', fb, ' ', unfilter), h('div.muted', tagsEl, ' ', followEl), h('div.muted', ord, nextEl, ' ', conn), meta, ed, keptRow),
      h('div.transport', h('button.pri', { id: 'now-skip', onclick: skip }, 'Skip'), pauseBtn, prevBtn, againBtn),
      h('label', { style: 'margin-top:12px' }, 'Brightness', bright.el)),
    rc,
    lookControls(drift));

  const preview = new PanelPreview(canvas, { wrap: el, onError: (m) => { cap.textContent = 'Preview: ' + m; } });
  let shownPath = '';
  let failures = 0;
  let clocks = [];   // polled: [{rtt, off, hi, t}]: off = device ms - local ms, at most `hi`
  let pushed = [];   // from the stream: [{off, t}]: off at least this (less the trip)
  let last = null;   // the last status sample: {path, elapsed, t, frame, rate} (device clock)
  let base = null;   // this file's first sample, for the rate its position runs at
  let recentOrd = -1; // the file `recent` was fetched with; -2: firmware without it

  let mirrored = mirror();
  if (!mirrored) cap.textContent = 'Not mirroring the panel (Settings › Panel)';
  const sendBright = debounce(async (v) => {
    try { await app.src.putConfig({ brightness: v }); if (app.config) app.config.brightness = v; } catch (e) { toast('brightness: ' + e.message, 'bad'); }
  }, 300);
  bright.input.addEventListener('input', () => sendBright(+bright.input.value));
  on('config', (c) => { bright.ceil(c.max_brightness); if (document.activeElement !== bright.input) bright.set(c.brightness); });

  // (The stream brings a change by itself.)
  const soonPoll = (ms) => live || app.soon('now', ms);
  async function skip() {
    try { await app.src.skip(); soonPoll(400); } catch (e) { toast(e.message, 'bad'); }
  }

  // Previous: back through Recently played, on the device.
  const soon = () => { recentOrd = Math.min(recentOrd, -1); app.soon('now', 400); };
  async function back() {
    try { await app.src.skip(true); soon(); } catch (e) { toast(e.message, 'bad'); }
  }
  async function pick(p) { if (p && await app.play(p)) soon(); }

  async function togglePause() {
    try { const r = await app.src.pause(); if (r.paused !== undefined) app.status.paused = r.paused; render(app.status); } catch (e) { toast(e.message, 'bad'); }
  }

  // The preview fetches the file's prepared copy first (app.previewBytes):
  // fetching from gifs/Consoles costs the device a ~330 ms directory scan
  // that the panel feels; a copy's small directory costs ~6 ms.
  async function follow_(path) {
    if (!mirror() || !path || path === shownPath) return;
    shownPath = path;
    cap.textContent = 'Loading…';
    try {
      if (await preview.show(path, (p) => app.previewBytes(p))) { cap.textContent = ''; sync(); soonPoll(); }
    } catch (e) { cap.textContent = 'Preview: ' + e.message; }
  }

  // One status sample, requested at local t0 and answered at t1; an event
  // (t0 null) came at t1, its trip unknown: the device's lead is at least
  // t_ms - t1.
  function clock(s, t0, t1) {
    if (typeof s.t_ms !== 'number') { last = null; return; }
    // A reboot restarts the device clock: forget the old samples. (Not an
    // offset jump: one reply stuck behind a file transfer has a big one.)
    const t = [...clocks, ...pushed].reduce((a, c) => Math.max(a, c.t), -Infinity);
    if (s.t_ms + 5000 < t) clocks = pushed = [];
    const all = t0 == null ? pushed : clocks;
    all.push(t0 == null ? { off: s.t_ms - t1, t: s.t_ms } : { rtt: t1 - t0, off: s.t_ms - (t0 + t1) / 2, hi: s.t_ms - t0, t: s.t_ms });
    if (all.length > 8) all.shift();
    const np = s.now_playing;
    // The panel's position runs a little slower than its clock (10 ms
    // sleep ticks); measure how much from its own reports, over >= 2 s.
    const key = `${np.ordinal} ${np.path} ${s.paused}`;
    if (base?.key !== key) base = { key, t: s.t_ms, e: np.elapsed_ms };
    const span = s.t_ms - base.t;
    const rate = span >= 2000 ? Math.min(1.1, Math.max(0.5, (np.elapsed_ms - base.e) / span)) : 1;
    last = { path: np.path, elapsed: np.elapsed_ms, t: s.t_ms, frame: np.frame, rate };
  }

  // The device's lead on this clock: the polled sample with the shortest
  // round trip; with events too, the middle of what both allow (an event's
  // lead at least, a read's at most), the read's when they disagree.
  function offset() {
    const best = clocks.reduce((a, b) => (b.rtt < a.rtt ? b : a), clocks[0]);
    if (!pushed.length) return best?.off;
    const lo = Math.max(...pushed.map((p) => p.off)), hi = Math.min(...clocks.map((c) => c.hi));
    return !best ? lo : lo <= hi ? (lo + hi) / 2 : best.off;
  }

  // Put the preview in step with the last sample, if it is of this file.
  function sync() {
    if (!last || !last.frame || preview.path !== last.path) return;
    const off = offset();
    if (off == null) return;
    const best = clocks.reduce((a, b) => (b.rtt < a.rtt ? b : a), clocks[0]);
    const d = preview.syncTo(last.elapsed, last.t - off, last.rate);
    if (Number.isNaN(d)) return;
    // data-*: for tools/web-smoke.py (k = syncs of this file so far).
    const ds = drift.dataset;
    ds.k = ds.path === last.path ? +ds.k + 1 : 1;
    const rtt = best ? best.rtt : NaN;
    Object.assign(ds, { path: last.path, drift: d.toFixed(1), rtt: rtt.toFixed(1), rate: last.rate.toFixed(4), paused: preview.paused, live });
    drift.textContent = `Drift ${Math.round(d)} ms` + (best ? ` · RTT ${Math.round(rtt)} ms` : '');
  }

  function render(s) {
    if (!s) return;
    const np = s.now_playing;
    // A virtual file (`@…`: Follow RePlayOS's cards, a Formula) shows its title only.
    const virt = !!np.path && drawn(np.path);
    const e = app.wasm.parsePath(virt ? '' : np.path || '');
    // Why the playlist is empty; "No card" once a mount has failed.
    const sd = s.sd, unreadable = sd.mounted && sd.readable === false;
    const empty = !np.path && !s.entries && s.index_state != 'building' && !unreadable && (sd.mounted || sd.attempts || sd.error);
    const pf = empty && sd.mounted && app.config?.platforms?.trim();
    unfilter.hidden = !pf;
    title.textContent = virt ? np.title : np.path ? label(e) : !empty ? 'Nothing playing' : !sd.mounted ? 'No card'
      : pf ? `No GIFs match the platform filter (${pf})` : 'The playlist is empty';
    tagsEl.textContent = np.path ? tags(np.path, e) : '';
    meta.textContent = virt ? '' : np.path;
    const fl = followLine(s);
    followEl.hidden = !fl;
    followEl.textContent = followEl.title = fl ?? '';
    // (`ordinal` counts the GIFs started since start-up, not a place in the playlist.)
    ord.textContent = `${np.ordinal.toLocaleString()} played · ${s.entries.toLocaleString()} in the playlist`;
    // (A GIF's; a Formula or a title card next says nothing.)
    const nx = np.next;
    nextEl.textContent = nx && !drawn(nx) ? ` · Next: ${label(app.wasm.parsePath(nx))}` : '';
    const kept = np.kept_s, left = Math.ceil(Math.max(0, kept * 1000 - np.elapsed_ms) / 1000);
    keptRow.hidden = kept == null;
    keptText.textContent = kept == null ? '' : kept ? `Kept on the panel: ${(left / 60) | 0}:${String(left % 60).padStart(2, '0')} left` : 'Kept on the panel until you change it';
    pauseBtn.textContent = s.paused ? 'Resume' : 'Pause';
    againBtn.disabled = !np.path;
    fb.set(np.path, s.manifest && app.activePath);
    ed.set(np.path, title.textContent);
    const b = bannerFor(s, () => soonPoll(300));
    banner.hidden = !b;
    banner.dataset.mode = s.mode || 'normal';
    if (b) clear(banner, b);
    sdLine.hidden = !s.sd.mounted || s.sd.readable !== false;
    sdText.textContent = s.sd.formatting ? 'Formatting the card…' : 'This card can’t be read: it needs FAT32';
    if (preview.paused !== s.paused) preview.setPaused(s.paused);
    follow_(np.path);
  }

  const good = () => {
    if (failures && preview.path) cap.textContent = '';
    failures = 0;
    conn.className = 'dot ok';
    conn.title = live ? 'Live' : 'Live (polling)';
  };
  // A failed poll leaves the preview playing on its last sync. Run by the
  // app's scheduler (js/state.js), one at a time: the device has one httpd
  // task, and a reply queued behind the preview's file fetch must not have
  // others piling up behind it.
  let lastFull = -1e9, lastEv = null;
  async function poll() {
    const t0 = performance.now();
    try {
      // Recently played changes only when a file starts: asked for then.
      const ask = recentOrd > -2 && app.status?.now_playing?.ordinal !== recentOrd;
      // (With the stream, a read is only ever for what it does not carry.)
      const full = live || ask || t0 - lastFull >= FULL_MS;
      const s = full || !app.src.now ? await app.refreshStatus(ask) : Object.assign(app.status, await app.src.now());
      if (full) lastFull = t0;
      clock(s, t0, performance.now());
      // An event that came while this was asked is newer.
      if (lastEv?.t_ms > s.t_ms) merge(s, lastEv);
      render(s);
      if (ask) {
        recentOrd = s.recent ? s.now_playing.ordinal : -2;
        if (s.recent) rc.set(s.recent, s.recent_at);
      }
      sync();
      good();
    } catch (e) {
      conn.className = 'dot bad';
      conn.title = ++failures > 1 ? 'Offline' : 'Retrying';
      if (preview.path) cap.textContent = 'Panel not reachable';
    } finally {
      // Polling (no stream): less often while a file goes to or from the card.
      if (!live && shown) app.watch('now', app.status?.transfer ? SLOW_MS : POLL_MS, poll);
    }
  }

  // An event onto the status: the card's part merged (it carries only some
  // of it), the rest as it is.
  function merge(s, ev) {
    for (const k in ev) s[k] = k == 'sd' ? { ...s.sd, ...ev.sd } : ev[k];
  }

  // The stream (GET /api/events): while it is live the view shows its
  // events and reads the status only for what they do not carry; a refusal,
  // an error or QUIET_MS without an event goes back to polling, and it is
  // tried again later; firmware without it (404) just polls.
  let shown = false, live = false, stream = null, noStream = false, quietT = 0, againT = 0;
  function listen() {
    clearTimeout(againT);
    if (stream || noStream || !shown || document.hidden || !app.src.events) return;
    const ac = stream = new AbortController();
    const quiet = () => { clearTimeout(quietT); quietT = setTimeout(() => ac.abort(), QUIET_MS); };
    quiet();
    app.src.events((ev, at) => {
      if (stream !== ac || !app.status) return;
      quiet();
      if (!live) {
        // (Re)connected: one full read, then only what the events lack.
        live = true;
        app.watch('now', Infinity, poll);
        app.soon('now');
      }
      onEvent(ev, at);
    }, ac.signal).then(() => end(ac), (e) => end(ac, e));
  }
  function end(ac, e) {
    if (stream !== ac) return;
    clearTimeout(quietT);
    stream = lastEv = null;
    if (e?.status == 404) noStream = true;
    const ms = live ? AGAIN_MS : Math.max(RETRY_MS, (e?.after || 0) * 1000);
    live = false;
    if (shown) {
      app.watch('now', POLL_MS, poll);
      againT = setTimeout(listen, ms);
    }
  }
  function unlisten() {
    clearTimeout(quietT);
    clearTimeout(againT);
    const ac = stream;
    stream = lastEv = null;
    live = false;
    ac?.abort();
  }
  // A laptop's keys while Now is up: Space pauses or resumes, → skips, ←
  // is Previous. Not while typing or on a slider, nor Space on a control
  // reached with the keyboard (it presses that one); a button the mouse
  // left focused lets go first, so the key isn't a second press of it.
  function onKey(ev) {
    const t = ev.target, f = { ' ': togglePause, ArrowRight: skip, ArrowLeft: () => prevBtn.disabled || back() }[ev.key];
    if (!f || ev.ctrlKey || ev.metaKey || ev.altKey || ev.repeat || t.closest?.('input,select,textarea,[contenteditable],dialog,[role=dialog],.menu')) return;
    const ctl = t.closest?.('button,a,summary');
    if (ev.key == ' ' && ctl?.matches(':focus-visible')) return;
    ev.preventDefault();
    ctl?.blur();
    f();
  }

  // Hidden: no stream (and the scheduler reads nothing); back: polling until it is live again.
  const onVis = () => { if (document.hidden) unlisten(); else { app.watch('now', POLL_MS, poll); listen(); } };

  function onEvent(ev, at) {
    const s = app.status, was = s.now_playing?.ordinal;
    // The playlist or the card changed: the full status (the other views'
    // too, and a card swap's checks).
    const big = ['entries', 'manifest', 'index_state'].some((k) => ev[k] !== s[k])
      || ['mounted', 'readable', 'formatting'].some((k) => ev.sd?.[k] !== s.sd?.[k]);
    lastEv = ev;
    merge(s, ev);
    if (app.config && ev.brightness != null && ev.brightness !== app.config.brightness) {
      app.config.brightness = ev.brightness;
      if (document.activeElement !== bright.input) bright.set(ev.brightness);
    }
    clock(ev, null, at);
    render(s);
    sync();
    good();
    if (big || ev.now_playing.ordinal !== was && recentOrd > -2) app.soon('now');
  }

  render(app.status);
  return {
    show() {
      if (fb.ov.path) fb.ov.load(fb.ov.path); // the Library may have changed it
      // Mirroring switched in Settings since.
      if (mirrored !== (mirrored = mirror())) { preview.clear(); shownPath = ''; cap.textContent = mirrored ? '' : 'Not mirroring the panel (Settings › Panel)'; }
      shown = true;
      // A status read at the start (polling until the stream is live).
      app.watch('now', POLL_MS, poll);
      app.soon('now');
      document.addEventListener('visibilitychange', onVis);
      document.addEventListener('keydown', onKey);
      listen();
    },
    hide() {
      shown = false;
      document.removeEventListener('visibilitychange', onVis);
      document.removeEventListener('keydown', onKey);
      unlisten();
      app.unwatch('now');
    },
  };
}

function mountLocal(el) {
  const canvas = h('canvas.panel', { id: 'now-panel' });
  const title = h('b', { id: 'now-title' }, 'Local simulator');
  const line = h('span.muted', { id: 'now-path' });
  const listSel = h('select', { id: 'sim-list' });
  const plats = h('input', { type: 'text', id: 'sim-platforms', placeholder: 'MD,SNES — blank for all', size: 18 });
  const shuffle = h('input', { type: 'checkbox', id: 'sim-shuffle' });
  const seed = h('input', { type: 'number', id: 'sim-seed', value: 7, min: 0, style: 'width:90px' });
  const pauseBtn = h('button', { id: 'now-pause', disabled: true, onclick: () => { preview.setPaused(!preview.paused); pauseBtn.textContent = preview.paused ? 'Resume' : 'Pause'; } }, 'Pause');
  const nextBtn = h('button', { id: 'now-skip', disabled: true, onclick: () => next() }, 'Next');
  const prevBtn = h('button', { id: 'now-prev', disabled: true, onclick: () => cur + 1 < hist.length && show(hist[++cur]) }, 'Previous');
  const fb = favButton();
  const rc = recentCard(prevBtn, (p) => show(p, 1), fb);
  const ed = editLine();
  let list = [];
  let i = -1;
  // Recently played, by the device's rules (opendmd-core history.rs).
  let hist = [];
  let cur = 0;

  el.append(
    h('div.card', canvas,
      h('div.row.capl', h('span.grow', title, ' ', fb, ' ', line), h('span.cap', 'One file in memory at a time, with the firmware’s 3–30 s dwell')), ed,
      h('div.row', { style: 'margin-top:12px' },
        h('label', 'Playlist', listSel), h('label', 'Platforms', plats), h('label', shuffle, 'Shuffle'), h('label', 'Seed', seed),
        h('button.pri', { id: 'sim-play', onclick: play }, 'Play'), prevBtn, pauseBtn, nextBtn)),
    rc,
    lookControls());

  const preview = new PanelPreview(canvas, {
    wrap: el,
    onDwellEnd: () => next(),
    onError: (m) => { toast(m, 'bad'); next(); },
  });
  preview.setDwell(3, 30);

  const src = app.src;
  const manifests = [...src.files.keys()].filter((p) => /^\/filelist\.[^/]*$|^\/sets\/[^/]+\.txt$/.test(p) && !/\/filelist\.odm\.|\.(idx|part|tmp|bak|meta)$/.test(p));
  for (const m of manifests) listSel.append(h('option', { value: m }, playlistLabel(m)));
  listSel.append(h('option', { value: '' }, 'Every GIF in the folder'));

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
  // were not extracted) and Formulas (the player draws them; this preview
  // can't), but give up after a full lap.
  async function next(tries = 0) {
    if (!list.length || tries > list.length) return;
    i = (i + 1) % list.length;
    const path = list[i];
    if (path[0] !== '@' ? !src.files.has(path) || drawn(path) : path.startsWith('@formula')) return next(tries + 1);
    line.textContent = `· ${i + 1} of ${list.length}`;
    await show(path, 1);
  }

  // `rec`: a new start, not Previous.
  async function show(path, rec) {
    if (rec) {
      if (cur) hist.unshift(...hist.splice(cur, 1));
      cur = 0;
      hist = [path, ...hist.filter((p) => p !== path)].slice(0, 16);
    }
    fb.set(path, listSel.value);
    rc.set(hist, cur);
    ed.set(path, title.textContent = label(app.wasm.parsePath(path)));
    await preview.show(path, (p) => app.previewBytes(p));
  }

  // The demo's player (js/demo.js) sets the playlist, the platforms and
  // shuffle from its settings, and starts it.
  if (app.demo) app.demo.drive({ listSel, plats, shuffle, play, preview });
  return { show() {}, hide() {} };
}

