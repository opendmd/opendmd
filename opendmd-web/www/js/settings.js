// Settings: the device's NVS config (GET/PUT /api/config — a PUT is a
// partial patch, applied live except WiFi and hostname, which need a
// reboot), a WiFi scan, reindex, reboot, the facts from status, and
// the playback cache (how many files are prepared; prepare the rest).

import { app, playlistLabel } from './state.js';
import * as prep from './prepare.js';
import { timeCard, scheduleCard } from './schedule.js';
import { followCard } from './follow.js';
import { PanelPreview } from './preview.js';
import { h, clear, on, toast, bytes, uptime, base } from './util.js';

export async function mount(el) {
  if (!app.device) {
    el.append(h('div.card', h('h2', 'Settings'), h('p.muted', 'Device settings, WiFi and reboot live on the player. This page is in local mode: open it from the player (http://opendmd.local/) to use them.')));
    return {};
  }
  const c = app.config || await app.refreshConfig();
  const f = {
    brightness: h('input', { type: 'range', min: 1, max: 255, id: 'st-brightness', value: c.brightness }),
    dwell_min: h('input', { type: 'number', min: 1, id: 'st-dwell-min', value: c.dwell_min, style: 'width:100px' }),
    dwell_max: h('input', { type: 'number', min: 1, id: 'st-dwell-max', value: c.dwell_max, style: 'width:100px' }),
    shuffle: h('input', { type: 'checkbox', id: 'st-shuffle', checked: c.shuffle }),
    platforms: h('input', { type: 'text', id: 'st-platforms', value: c.platforms, placeholder: 'blank = all, or MD,SNES' }),
    set: h('select', { id: 'st-set' }),
    wifi_ssid: h('input', { type: 'text', id: 'st-ssid', value: c.wifi_ssid, autocomplete: 'off' }),
    wifi_pass: h('input', { type: 'password', id: 'st-pass', placeholder: c.wifi_pass_set ? 'unchanged (one is saved)' : 'none saved', autocomplete: 'new-password' }),
    hostname: h('input', { type: 'text', id: 'st-hostname', value: c.hostname }),
    transition: h('select', { id: 'st-trans' }, [['dissolve', 'Dissolve'], ['wipe_left', 'Wipe'], ['blinds', 'Blinds'], ['push', 'Push'], ['fade', 'Fade'], ['none', 'None']].map(([v, l]) => h('option', { value: v }, l))),
    transition_ms: h('select', { id: 'st-trans-ms', 'data-num': 1 }, [[250, 'Quick'], [400, 'Normal'], [700, 'Slow']].map(([v, l]) => h('option', { value: v }, l))),
    look: h('select', { id: 'st-look' }, [['colour', 'Full colour'], ['amber', 'Amber'], ['red', 'Red'], ['green', 'Green'], ['blue', 'Blue'], ['white', 'White']].map(([v, l]) => h('option', { value: v }, l))),
    glow: h('select', { id: 'st-glow' }, ['off', 'short', 'long'].map((v) => h('option', { value: v }, v))),
  };
  f.look.value = c.look ?? 'colour';
  f.glow.value = c.glow ?? 'off';
  // A value set some other way (another wipe direction, another duration)
  // is shown as it is rather than silently replaced.
  const pick = (sel, v, label) => { if (![...sel.options].some((o) => o.value == v)) sel.append(h('option', { value: v }, label)); sel.value = v; };
  pick(f.transition, c.transition ?? 'dissolve', String(c.transition).replace('_', ' '));
  pick(f.transition_ms, c.transition_ms ?? 400, `${c.transition_ms} ms`);
  // "Preview" plays the choice in this page only: the panel's file, started
  // again from its first frame, transitioning in from the frame on screen.
  const pvCanvas = h('canvas.panel', { id: 'st-trans-panel' });
  const pvBox = h('div', { hidden: true, style: 'margin-top:12px' }, pvCanvas);
  let pv = null;
  async function previewTrans() {
    pvBox.hidden = false;
    pv ??= new PanelPreview(pvCanvas);
    pv.p.setTransition(f.transition.value, +f.transition_ms.value);
    const path = app.status?.now_playing?.path;
    if (!pv.path) {
      if (!path || !await pv.show(path, (p) => app.previewBytes(p))) return;
      await new Promise((r) => setTimeout(r, 800));
    }
    pv.p.replay();
  }
  const brightOut = h('output.mono', c.brightness);
  f.brightness.oninput = () => { brightOut.textContent = f.brightness.value; };
  const nets = h('div.chips', { id: 'st-nets' });
  const banner = h('div.banner', { hidden: true, id: 'st-reboot' }, 'Saved. WiFi and hostname changes take effect after a reboot. ',
    h('button.sm', { onclick: reboot }, 'Reboot now'));
  const facts = h('div.grid', { id: 'st-facts' });
  // One quiet line, only when the card has had trouble (sdhealth on the device).
  const cardNote = h('p.cap', { id: 'st-card-note', hidden: true, style: 'margin-top:10px' });

  // The playlist choices, rebuilt whenever the config changes or the view
  // is shown: a playlist made or deleted elsewhere must not linger here.
  async function loadPlaylists() {
    const set = app.config?.set ?? '';
    const lists = await app.playlists().catch(() => []);
    if (!lists.some((p) => p.value === '')) lists.unshift({ value: '', label: 'All files' });
    if (!lists.some((p) => p.value === set)) lists.push({ value: set, label: playlistLabel(set) });
    clear(f.set, lists.map((p) => h('option', { value: p.value }, p.value ? p.label : `${p.label} (default)`)));
    f.set.value = set;
  }
  await loadPlaylists();
  on('config', loadPlaylists);
  // --- playback cache
  const cacheLine = h('b', { id: 'pc-count' }, '…');
  const cacheBtn = h('button.pri', { id: 'pc-go', onclick: () => (prep.job.running ? prep.stop() : prep.start()) }, 'Prepare remaining files');
  const cacheBar = h('i');
  const cacheMsg = h('div.muted', { id: 'pc-progress' });
  const cacheErr = h('div.log.bad', { id: 'pc-errors' });
  const cacheSum = h('div', { id: 'pc-summary' });
  // The device prepares files by itself when idle (auto_prepare): one
  // sentence, and whether it may also do so, slowly, while playing.
  const autoLine = h('span', { id: 'pc-auto' });
  const always = h('input', { type: 'checkbox', id: 'pc-always', onchange: async () => {
    try { await app.src.putConfig({ auto_prepare: always.checked ? 'always' : 'idle' }); await app.refreshConfig(); loadCache(); } catch (e) { toast('save: ' + e.message, 'bad'); }
  } });
  const autoBox = h('div', { hidden: true, style: 'margin-top:10px' }, autoLine, h('label', always, 'also while playing (slower)'));

  // Start-up: its own module and size line (js/boot.js), loaded here.
  const boot = h('div.card', { id: 'st-boot-card' });
  import('./boot.js').then((m) => m.bootCard(boot));
  const time = timeCard();
  const sched = scheduleCard();
  const follow = followCard();
  on('status', time.render);

  el.append(banner,
    h('div.card', h('h2', 'Playback'), h('p.cap', 'Applied live. A playlist, platform or shuffle change rebuilds the index on the card (a few seconds).'),
      h('div.form',
        h('label.stack', 'brightness', h('div.row', f.brightness, brightOut)),
        h('label.stack', 'colour', f.look),
        h('label.stack', 'phosphor glow', f.glow),
        h('label.stack', 'dwell min / max (s)', h('div.row', f.dwell_min, f.dwell_max)),
        h('label.stack', 'platforms', f.platforms),
        h('label.stack', 'playlist', f.set),
        h('label', f.shuffle, 'shuffle')),
      h('div.row', { style: 'margin-top:12px' }, h('button.pri', { id: 'st-save-play', onclick: () => save(['brightness', 'look', 'glow', 'dwell_min', 'dwell_max', 'shuffle', 'platforms', 'set']) }, 'Save playback'))),
    h('div.card', { id: 'st-trans-card' }, h('h2', 'Transition'), h('p.cap', 'How the panel moves from one GIF to the next.'),
      h('div.form', h('label.stack', 'style', f.transition), h('label.stack', 'speed', f.transition_ms)),
      h('div.row', { style: 'margin-top:12px' }, h('button', { id: 'st-trans-preview', onclick: previewTrans }, 'Preview'),
        h('button.pri', { id: 'st-save-trans', onclick: () => save(['transition', 'transition_ms']) }, 'Save transition')),
      pvBox),
    boot, time.el, sched.el, follow.el,
    h('div.card', { id: 'pc' }, h('h2', 'Playback cache'),
      h('p.cap', 'Prepared copies play with less memory and start faster. Originals stay as they are.'),
      h('div.row.sp', h('span', cacheLine), cacheBtn),
      h('div.bar', { style: 'margin:10px 0' }, cacheBar), cacheMsg, cacheSum, cacheErr, autoBox),
    h('div.card', h('h2', 'Network'), h('p.cap', 'WiFi and hostname changes need a reboot. With no saved network the player raises the open OpenDMD-Setup access point.'),
      h('div.form',
        h('label.stack', 'WiFi network (SSID)', f.wifi_ssid),
        h('label.stack', 'WiFi password', f.wifi_pass),
        h('label.stack', 'hostname (.local)', f.hostname)),
      h('div.row', { style: 'margin-top:10px' }, h('button', { id: 'st-scan', onclick: scan }, 'Scan for networks'), nets),
      h('div.row', { style: 'margin-top:12px' }, h('button.pri', { id: 'st-save-net', onclick: () => save(['wifi_ssid', 'wifi_pass', 'hostname']) }, 'Save network'))),
    h('div.card', h('h2', 'Device'), facts, cardNote,
      h('p.cap', { style: 'margin-top:10px' }, 'Firmware updates are installed over USB with the browser flasher.'),
      h('div.row', { style: 'margin-top:12px' },
        h('button', { id: 'st-reindex', onclick: reindex }, 'Rebuild index'),
        h('button.danger', { id: 'st-reboot-btn', onclick: reboot }, 'Reboot'))));

  function renderFacts(s) {
    if (!s) return;
    const fact = (k, v) => h('div.stat', h('b', { style: 'font-size:14px' }, v), h('span', k));
    clear(facts, fact('firmware', s.version), fact('flash variant', s.variant), fact('panel', `128×32 · ${s.refresh_hz} Hz`),
      fact('WiFi', `${s.wifi} · ${s.ip}`), fact('hostname', s.hostname + '.local'), fact('uptime', uptime(s.uptime_secs)),
      fact('playlist', playlistLabel(s.manifest)), fact('entries', s.entries.toLocaleString()),
      fact('index', s.index_state ?? '–'), fact('web app', s.www_version ?? '–'),
      fact('card', s.sd.mounted ? bytes(s.sd.capacity_bytes) : 'not mounted'));
    const sd = s.sd;
    const errs = sd.failures || (!sd.mounted && sd.reboots) || 0;
    cardNote.textContent = errs ? `Card errors: ${errs}, retrying`
      : sd.fallback ? 'Card running at reduced speed after errors' : '';
    cardNote.hidden = !cardNote.textContent;
    // A set change while an index is being built would queue behind it.
    f.set.disabled = s.index_state === 'building';
    f.set.title = f.set.disabled ? 'the device is building an index' : '';
    clearTimeout(recheck);
    if (f.set.disabled) recheck = setTimeout(() => app.refreshStatus().catch(() => {}), 2000);
  }
  let recheck = 0;
  on('status', renderFacts);

  async function save(keys) {
    const cur = app.config;
    const patch = {};
    for (const k of keys) {
      const el_ = f[k];
      const v = el_.type === 'checkbox' ? el_.checked : el_.type === 'number' || el_.type === 'range' || el_.dataset.num ? +el_.value : el_.value.trim();
      if (k === 'wifi_pass') { if (el_.value) patch[k] = el_.value; continue; }
      if (v !== cur[k]) patch[k] = v;
    }
    if (patch.dwell_max < (patch.dwell_min ?? cur.dwell_min) || (patch.dwell_min > (patch.dwell_max ?? cur.dwell_max))) { toast('dwell max must be ≥ dwell min', 'bad'); return; }
    if (!Object.keys(patch).length) { toast('nothing changed'); return; }
    try {
      const r = await app.src.putConfig(patch);
      await app.refreshConfig();
      f.wifi_pass.value = '';
      f.wifi_pass.placeholder = app.config.wifi_pass_set ? 'unchanged (one is saved)' : 'none saved';
      if (r.reboot_required || 'hostname' in patch) banner.hidden = false;
      toast(`saved ${Object.keys(patch).filter((k) => k !== 'wifi_pass').concat('wifi_pass' in patch ? ['password'] : []).join(', ')}${r.index ? ` · index ${r.index} (${r.index_ms} ms) · ${r.entries} entries` : ''}`, 'ok');
    } catch (e) { toast('save: ' + e.message, 'bad'); }
  }

  async function scan() {
    clear(nets, h('span.muted', 'scanning…'));
    try {
      const aps = await app.src.wifiScan();
      clear(nets, aps.sort((a, b) => b.rssi - a.rssi).map((ap) => h('button.chip', {
        onclick: () => { f.wifi_ssid.value = ap.ssid; f.wifi_pass.focus(); },
      }, ap.ssid || '(hidden)', h('small', `${ap.rssi} dBm${ap.secure ? ' · secure' : ' · open'}`))));
    } catch (e) { clear(nets, h('span.bad', e.message)); }
  }

  async function reindex() {
    try {
      toast('rebuilding the index… the panel holds its frame');
      const r = await app.src.reindex();
      toast(`index ${r.index}: ${r.entries} entries in ${r.index_ms} ms`, 'ok');
    } catch (e) { toast('reindex: ' + e.message, 'bad'); }
  }

  async function waitForBoot(msg) {
    for (let i = 0; i < 45; i++) {
      await new Promise((r) => setTimeout(r, 2000));
      try { const s = await app.refreshStatus(); msg(`back up: ${s.version} ${s.variant}`); return; } catch { msg(`rebooting… ${2 * (i + 1)} s`); }
    }
    msg('the device has not come back after 90 s — check its serial log');
  }

  async function reboot() {
    if (!confirm('Reboot the player?')) return;
    try { await app.src.reboot(); toast('rebooting…'); banner.hidden = true; waitForBoot((m) => toast(m)); } catch (e) { toast('reboot: ' + e.message, 'bad'); }
  }

  // --- playback cache ---------------------------------------------------
  // GET /api/cache is cheap to ask; the device counts in the background and
  // answers "counting" meanwhile, so poll until it is ready.
  let cachePoll = 0, visible = true;
  async function loadCache() {
    clearTimeout(cachePoll);
    try {
      const r = await app.src.cache();
      if (r.prepared == null) cacheLine.textContent = `${r.total.toLocaleString()} files · counting prepared files…`;
      else cacheLine.textContent = `${r.prepared.toLocaleString()} of ${r.total.toLocaleString()} files prepared` + (r.state === 'counting' ? ' · recounting…' : '');
      const a = r.auto, run = a?.state === 'running';
      autoBox.hidden = !a;
      if (a) { autoLine.textContent = autoText(a, r) + '. '; always.checked = a.mode === 'always'; }
      if ((run || r.state === 'counting') && visible) cachePoll = setTimeout(loadCache, run ? 4000 : 2000);
    } catch (e) { cacheLine.textContent = 'cache: ' + e.message; }
  }
  function autoText(a, r) {
    const n = (x) => x.toLocaleString(), e = a.eta_s, nt = sched.night();
    if (a.mode === 'off') return 'Automatic preparing is off';
    if (a.state === 'done') return a.failed ? `Automatic preparing is done · ${n(a.failed)} could not be prepared` : 'All files are prepared';
    const m = (t) => +t.slice(0, 2) * 60 + +t.slice(3), len = nt?.mode == 'off' && ((m(nt.to) - m(nt.from) + 1440) % 1440 || 1440) * 60;
    return (a.state !== 'running' ? (a.reason && a.reason !== 'playing' ? `Automatic preparing is waiting (${a.reason})` : 'Files are prepared automatically when the panel is idle')
      : a.policy === 'drip' ? 'Preparing files slowly while playing' : 'Preparing files automatically while the panel is idle')
      + (r.prepared != null ? ` · ${n(r.prepared)} of ${n(r.total)}` : '')
      + (e == null ? '' : len ? ` · about ${Math.ceil(e / len)} night${e > len ? 's' : ''} left` : ` · about ${e < 5400 ? mins(e) : Math.round(e / 3600) + ' hours'} of idle time left`);
  }
  function mins(s) { return s < 90 ? `${Math.max(1, Math.round(s))} s` : s < 5400 ? `${Math.round(s / 60)} min` : `${(s / 3600).toFixed(1)} h`; }
  function renderJob(j) {
    cacheBtn.textContent = j.running ? (j.stopping ? 'Stopping after this file…' : 'Stop') : 'Prepare remaining files';
    cacheBtn.disabled = j.stopping;
    const n = j.tried, dmg = j.damaged.length, net = j.net.length, num = (x) => x.toLocaleString();
    const files = (k, what) => `${num(k)} ${what}${k === 1 ? '' : 's'}`;
    cacheBar.style.width = j.todo ? `${(100 * n) / j.todo}%` : j.phase === 'finished' ? '100%' : '0';
    const left = prep.eta();
    const skipped = dmg ? ` · ${files(dmg, 'damaged file')} skipped (${dmg === 1 ? 'it' : 'they'} can't be played either)` : '';
    cacheMsg.textContent = j.phase === 'checking' ? 'Finding the files that are not prepared yet…'
      : j.phase === 'preparing' ? `${num(n)} of ${num(j.todo)} · ${num(j.done)} prepared` + (dmg ? ` · ${num(dmg)} damaged skipped` : '')
        + (net ? ` · ${num(net)} couldn't download` : '') + (left != null && n < j.todo ? ` · about ${mins(left)} left` : '') + (j.current ? ` · ${base(j.current)}` : '')
      : j.phase === 'finished' ? (j.todo ? files(j.done, 'file') + ' prepared' : dmg ? 'Every other file is already prepared' : 'Every file is already prepared') + skipped + '.'
      : j.phase === 'stopped' ? `Stopped after ${num(n)} of ${num(j.todo)}` + skipped + '. Start again to carry on.' : '';
    // Once a run is over: what was skipped, and what could not be fetched.
    const list = (id, head, rows) => h('details', { id }, h('summary', head), h('div.log', rows.map((r) => h('div', r))));
    clear(cacheSum, !j.running && dmg > 0 && list('pc-damaged', 'Damaged files', j.damaged.map((d) => `${d.path} — ${d.reason}`)),
      !j.running && net > 0 && h('div', { id: 'pc-net' }, h('span.bad', `Couldn't download ${files(net, 'file')} — `),
        h('button.sm', { onclick: () => prep.start() }, 'try again'), list('pc-net-list', 'Which files', j.net)));
    clear(cacheErr, j.errors.map((e) => h('div', e)));
    if (!j.running && j.phase !== 'idle') loadCache();
  }
  on('prepare', renderJob);

  renderFacts(app.status);
  time.render(app.status);
  renderJob(prep.job);
  loadCache();
  return { show() { visible = true; app.refreshStatus().catch(() => {}); loadPlaylists(); loadCache(); sched.load(); }, hide() { visible = false; clearTimeout(cachePoll); } };
}
