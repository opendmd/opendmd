// The web demo's player (imported by app.js for the demo only; its own size
// line, never loaded from a device). The demo's card is a LocalSource in
// memory; this gives it the device's settings API — GET/PUT /api/config,
// the schedule, the status Settings reads —, answered in the page and kept
// for this browser session (sessionStorage), so Settings runs its device
// code unchanged. What the settings change shows on the demo's panel (Now's
// simulator, which hands its controls over in `drive`): brightness (the
// schedule's and night mode's too), colour, glow and transition (preview.js
// applies the config to every preview), the Formula look (drawn with Psi
// over every frame, `Preview.setOver`), how long each GIF stays, the
// playlist, platforms and shuffle, and Now's schedule banner (a Formula
// look is checked before it is saved, as the player checks it). What only a
// player can do (firmware, WiFi, restarting, testing the panel) keeps its
// controls and says in a line that the demo can't.

import { app, setPath } from './state.js';
import { HttpError } from './source.js';
import { bannerFor, ZONES } from './schedule.js';
import { h, emit, on, label, toast } from './util.js';

const KEY = 'odmd.demo', SAFE = 144, DAYS = ['Mon', 'Tue', 'Wed', 'Thu', 'Fri', 'Sat', 'Sun'];
const PANEL = { clk_phase: true, latch_blanking: 1, driver: 'shiftreg', i2s_speed: 8, min_refresh: 60 }, BOOT = ['driver', 'i2s_speed', 'min_refresh'];
const cant = (what) => new HttpError(501, `The demo can’t ${what}`);
const bad = (m) => { throw new HttpError(400, m); };
const pad = (n) => String(n).padStart(2, '0');
const hhmm = (m) => `${pad((m / 60) | 0)}:${pad(m % 60)}`;
const hm = (t) => { const m = /^(\d\d?):(\d\d)$/.exec(t || ''); return !m ? null : +m[1] == 24 && +m[2] == 0 ? 0 : +m[1] < 24 && +m[2] < 60 ? m[1] * 60 + +m[2] : null; };

// The browser's zone when the list has it, else its offset as a POSIX string.
function zone() {
  let z;
  try { z = Intl.DateTimeFormat().resolvedOptions().timeZone; } catch { /* none */ }
  const k = ZONES.find(([n]) => n.split(',')[0] == z);
  if (k) return { tz: k[1], tz_name: k[0] };
  const o = -new Date().getTimezoneOffset(), a = Math.abs(o);
  return { tz: `UTC${o > 0 ? '-' : '+'}${(a / 60) | 0}${a % 60 ? ':' + pad(a % 60) : ''}`, tz_name: 'Custom' };
}

// What GET /api/config gives a fresh player, and the state kept with it.
const fresh = () => ({
  c: { wifi_ssid: 'HomeWiFi', wifi_pass_set: true, wifi_eap_user: '', wifi_eap_identity: '', wifi_eap_ttls: 'mschapv2', brightness: 96, max_brightness: SAFE, dwell_min: 3, dwell_max: 30, shuffle: false, platforms: '', set: '',
    hostname: 'opendmd', ...zone(), transition: 'dissolve', transition_ms: 400, auto_prepare: 'idle', card_speed: 'safe', beta_updates: false, follow: false, follow_host: '',
    follow_code_set: false, attract_set: null, attract: '', look: 'colour', glow: 'off', look_formula: '', boot_show: 'short', boot_formula: '', panel_sync: true, panel_split: true, formula_keep: 'turn', api_key_set: false },
  sched: { rules: [], night: null },
  wake: 0,
  // Panel timing (opendmd_core::panel_timing's Tuner, as tools/mock-device.py
  // has it): saved, running, a trial ({until}) and what waits for a restart.
  pt: { saved: { ...PANEL }, run: { ...PANEL }, trial: null, staged: null },
});
let S;
try { S = JSON.parse(sessionStorage.getItem(KEY)); } catch { /* none */ }
if (!S?.c) S = fresh();
// (A session kept from before a setting existed gets its default; one that
// has gone is dropped.)
const F = fresh().c;
S.c = Object.fromEntries(Object.entries({ ...F, ...S.c }).filter(([k]) => k in F));
const keep = () => { try { sessionStorage.setItem(KEY, JSON.stringify(S)); } catch { /* private mode */ } };

// The player's clock in its zone: the list's name for it (Intl knows those),
// else the POSIX string's standard offset (no summer time).
function clock(at = Date.now()) {
  const z = ZONES.find(([, tz]) => tz === S.c.tz);
  try {
    if (z) {
      const p = {};
      for (const x of new Intl.DateTimeFormat('en-GB', { timeZone: z[0].split(',')[0], hourCycle: 'h23', weekday: 'short', hour: '2-digit', minute: '2-digit', month: 'numeric', day: 'numeric' }).formatToParts(at)) p[x.type] = x.value;
      return { day: DAYS.indexOf(p.weekday) + 1, min: +p.hour * 60 + +p.minute, date: +p.month * 32 + +p.day };
    }
  } catch { /* the offset, then */ }
  const m = /^(?:<[^>]*>|[A-Za-z]+)([+-]?)(\d+)(?::(\d+))?/.exec(S.c.tz || '');
  const u = new Date(at + (m ? (m[1] == '-' ? 1 : -1) * (m[2] * 60 + +(m[3] || 0)) : -new Date(at).getTimezoneOffset()) * 60000);
  return { day: u.getUTCDay() || 7, min: u.getUTCHours() * 60 + u.getUTCMinutes(), date: (u.getUTCMonth() + 1) * 32 + u.getUTCDate() };
}

// opendmd_core::schedule: the first rule whose window holds the day, date
// (month * 32 + day; a window past midnight belongs to its start) and
// minute; night mode every day (not one from sunset to sunrise: the demo's
// player has no location); neither while woken.
const mdOf = (s) => +s.slice(0, 2) * 32 + +s.slice(3);
const inDates = (r, x) => !r || (mdOf(r[0]) <= mdOf(r[1]) ? mdOf(r[0]) <= x && x <= mdOf(r[1]) : x >= mdOf(r[0]) || x <= mdOf(r[1]));
const yesterday = (x) => (x % 32 > 1 ? x - 1 : ((m) => m * 32 + [31, 29, 31, 30, 31, 30, 31, 31, 30, 31, 30, 31][m - 1])(x >> 5 == 1 ? 12 : (x >> 5) - 1));
const inside = (f, t, days, d, m, dates, x) => {
  const on = (y, z) => (!days?.length || days.includes(y)) && inDates(dates, z), y = d == 1 ? 7 : d - 1;
  return f == t ? on(d, x) : f < t ? on(d, x) && f <= m && m < t : (on(d, x) && m >= f) || (on(y, yesterday(x)) && m < t);
};
function evaluate() {
  const { day, min, date } = clock(), r = { rule: null, set: null, until: null, mode: 'normal', mode_until: null, b: null };
  if (S.wake <= Date.now()) S.wake = 0;
  if (S.wake) return r;
  const i = S.sched.rules.findIndex((x) => inside(hm(x.from), hm(x.to), x.days, day, min, x.dates, date)), x = S.sched.rules[i], n = S.sched.night;
  if (x) Object.assign(r, { rule: i, set: x.set ?? null, until: x.to, b: x.brightness ?? null });
  if (n && !n.sun && inside(hm(n.from), hm(n.to), null, day, min, null, date)) Object.assign(r, { mode: n.mode, mode_until: n.to, b: n.mode == 'off' ? 0 : n.brightness ?? 8 });
  return r;
}

// --- panel timing
const T = () => S.pt;
const same = (a, b) => BOOT.every((k) => a[k] === b[k]);
const eff = (t) => ({ ...t, clk_phase: t.clk_phase || ['mbi5124', 'dp3246'].includes(t.driver) });
const base = () => ({ ...(T().trial ? T().run : T().staged || T().saved) });
const liveOf = (t) => ({ clk_phase: t.clk_phase, latch_blanking: t.latch_blanking });
function revert() { const t = T(); t.staged = null; if (t.trial) Object.assign(t.run, liveOf(t.saved)); t.trial = null; }
function panelFields() {
  const t = T();
  if (t.trial && Date.now() >= t.trial.until) { revert(); keep(); }
  return { panel: { ...t.saved }, panel_now: eff(t.run), panel_next: t.staged ? { ...t.staged } : same(t.saved, t.run) ? null : { ...t.saved },
    panel_trial_s: t.trial ? Math.max(0, Math.ceil((t.trial.until - Date.now()) / 1000)) : null };
}
function panel(patch) {
  const t = T(), pc = patch.panel_confirm, p = patch.panel;
  panelFields();
  if ('panel' in patch && p === null) { t.saved = { ...PANEL }; t.trial = t.staged = null; Object.assign(t.run, liveOf(t.saved)); return; }
  if (p) {
    for (const k in p) if (!(k in PANEL)) bad(`unknown panel setting ${k}`);
    const target = { ...base(), ...p };
    if (pc) { t.saved = target; t.trial = t.staged = null; Object.assign(t.run, liveOf(target)); } else if (same(target, t.run)) { t.run = target; t.trial = { until: Date.now() + 60000 }; t.staged = null; } else t.staged = target;
  } else if (pc) {
    if (!t.trial) throw new HttpError(409, 'no panel trial is running (it may have ended)');
    t.trial = null;
    t.saved = { ...t.run };
  } else if (pc === false) revert();
}

// --- the playlist in play: its card path, and how many GIFs the filter leaves
const playing = (r = evaluate()) => setPath(r.set ?? S.c.set);
const has = (path) => app.src.files.has(path);
async function count(set = playing(), platforms = S.c.platforms) {
  const text = await app.src.readText(set);
  return text == null ? 0 : app.wasm.filterManifest(text, JSON.stringify({ platforms })).length;
}

let sim = null, cur = { path: '', at: 0, n: 0 }, entries = 0, started = 0, uptime0 = Date.now() - 4 * 3600e3, version = '';

function status(recent) {
  const r = evaluate(), c = S.c, w = S.wake && clock(S.wake);
  const s = {
    version: version.replace(/^v/, '') || 'demo', variant: '4mb', uptime_secs: Math.round((Date.now() - uptime0) / 1000), t_ms: Math.round(performance.now()),
    ip: '192.0.2.10', wifi: 'sta', hostname: c.hostname, mdns: c.hostname,
    now_playing: { ordinal: cur.n, path: cur.path, title: cur.path && label(app.wasm.parsePath(cur.path)), elapsed_ms: Math.round(performance.now() - cur.at), frame: 1 },
    paused: !!sim?.preview.paused, entries, manifest: '/sdcard' + playing(r), index_state: 'ready', www_version: version || null,
    heap: { free: 61440, floor: 48128, idf_min: 45056, transitions_cut: 0 }, refresh_hz: 100, deadline_misses: 0, worst_lateness_ms: 0,
    misses: { steady: 0, switch: 0, threshold_ms: 4, window_ms: 2000 }, gifs_started: started,
    sd: { mounted: true, readable: true, capacity_bytes: 31914983424, clock_khz: c.card_speed == 'fast' ? 40000 : 20000, failures: 0, attempts: 1 },
    mode: r.mode, mode_until: r.mode_until, time: hhmm(clock().min), tz: c.tz_name, time_synced_s: 60, wake_until: w ? hhmm(w.min) : null,
    schedule: { rule: r.rule, set: r.set, until: r.until },
    context: { enabled: c.follow, state: 'unreachable', error: 'the demo can’t reach your network' },
    look: c.look, glow: c.glow, glow_held: 0, formula_look: c.look_formula ? { error: lookErr || null, held: 1 } : null,
    max_brightness: c.max_brightness, brightness_safe_max: SAFE, safe_start_until: null,
  };
  if (recent) Object.assign(s, { recent: cur.path ? [cur.path] : [], recent_at: 0 });
  return s;
}

function putConfig(patch) {
  const c = { ...S.c }, b = patch.brightness, mx = patch.max_brightness;
  for (const [k, v] of Object.entries(patch)) {
    if (k == 'wifi_ssid' || k == 'wifi_pass' || /^wifi_eap_/.test(k) && v !== c[k]) throw cant('join a WiFi network: on a display, this saves it for the next restart');
    if (k == 'hostname') {
      const n = String(v).trim().toLowerCase();
      if (!/^[a-z0-9]([a-z0-9-]{0,30}[a-z0-9])?$/.test(n)) bad('hostname must be 1-32 lowercase letters, digits and hyphens, not starting or ending with a hyphen');
      c.hostname = n;
    } else if (k == 'api_key') {
      // (firmware.js keeps a new key for this site's requests: the demo has none to send.)
      c.api_key_set = !!v;
      setTimeout(dropKey);
    }
    else if (k == 'follow_code') c.follow_code_set = c.follow_code_set || !!v;
    else if (k == 'dwell_min' || k == 'dwell_max') c[k] = Math.max(1, +v || 1);
    else if (k == 'formula_keep' && !['turn', '1m', '5m', 'hold'].includes(v)) bad('formula_keep must be turn, 1m, 5m or hold');
    else if (k == 'set' && v && !has(setPath(v))) throw new HttpError(404, 'set not found');
    else if (k in c && k != 'brightness' && k != 'max_brightness') c[k] = v;
    else if (!/^(brightness|max_brightness|panel|panel_confirm)$/.test(k)) bad(`unknown field ${k}`);
  }
  if (c.dwell_max < c.dwell_min) bad('dwell_max must be >= dwell_min');
  if (mx != null) { if (!(mx >= 16 && mx <= 255)) bad(`max_brightness must be 16..=255, not ${mx}`); c.max_brightness = mx; }
  if (b != null) { if (b > c.max_brightness) bad(`brightness ${b} is above the maximum ${c.max_brightness}; raise Maximum brightness first`); c.brightness = b; }
  c.brightness = Math.min(c.brightness, c.max_brightness);
  if ('panel' in patch || 'panel_confirm' in patch) panel(patch);
  S.c = c;
  keep();
  apply();
  return { ok: true, entries, brightness: c.brightness, max_brightness: c.max_brightness, reboot_required: false, card_speed_applied: 'now', ...panelFields() };
}

function putSchedule(s) {
  const rs = Array.isArray(s?.rules) ? s.rules : [], n = s?.night ?? null;
  if (rs.length > 16) bad(`at most 16 rules (got ${rs.length})`);
  rs.forEach((r, i) => {
    if (hm(r.from) == null || hm(r.to) == null) bad(`rule ${i + 1}: times are HH:MM`);
    if (r.dates != null && !(Array.isArray(r.dates) && r.dates.length == 2 && r.dates.every((d) => /^\d\d-\d\d$/.test(d)))) bad(`rule ${i + 1}: dates are MM-DD ("10-31")`);
    if (r.set != null && !has(setPath(r.set))) throw new HttpError(404, `set not found: ${r.set}`);
  });
  if (n && (!n.sun && (hm(n.from) == null || hm(n.to) == null) || !['off', 'dim'].includes(n.mode))) bad('night: from and to are HH:MM, mode off or dim');
  S.sched = { rules: rs, night: n };
  keep();
  apply();
  return { ok: true };
}

// The rest of what the device's views ask, by route.
function json(method, url, body) {
  const r = method + ' ' + url.split('?')[0];
  if (r == 'GET /api/name-check') return { taken: false };
  if (r == 'POST /api/card/full-speed') return { applied: 'now' };
  if (r == 'POST /api/follow/test') return { ok: false, reachable: false, error: 'the demo can’t reach your network' };
  if (r == 'GET /api/firmware') return { version: status().version, variant: '4mb', flash_size: 4 << 20, build: 'the web demo' };
  if (r == 'GET /api/www') return { ok: true, version };
  if (r == 'POST /api/panel/test') throw cant('show test pictures: it has no panel');
  throw cant(`do that (${r})`);
}

// --- what the settings do to the demo's panel
const style = h('style', '#view-now #now-panel{filter:brightness(var(--demo-b,1))}');
document.head.append(style);
let driven = '', lookOn = '', lookErr = '', psi = null;
const dropKey = () => { try { localStorage.removeItem('odmd.apiKey'); } catch { /* none */ } };
let live = true;
const pkg = () => (psi ??= import('./wasm.js').then((W) => W.load('pkg-psi', 'demo')).then(([m, x]) => ({ ...m, mem: x.memory })));
function apply() {
  if (!live) return;
  const r = evaluate(), c = S.c, b = Math.min(r.b ?? c.brightness, c.max_brightness);
  // Brightness 96 (a new player's) draws the preview as it is.
  document.documentElement.style.setProperty('--demo-b', b ? ((b / 96) ** 0.45).toFixed(3) : 0);
  app.status = status();
  emit('status', app.status);
  banner();
  if (!sim) return;
  sim.preview.setDwell(c.dwell_min, c.dwell_max);
  const capl = sim.preview.canvas.parentElement.querySelector('.capl .cap');
  if (capl) capl.textContent = `One file in memory at a time; each stays ${c.dwell_min}–${c.dwell_max} s (Settings › Playback)`;
  const set = playing(r), want = [set, c.platforms, c.shuffle].join('\n');
  if (want != driven) {
    driven = want;
    sim.listSel.value = has(set) ? set : '/filelist.txt';
    sim.plats.value = c.platforms;
    sim.shuffle.checked = c.shuffle;
    sim.play();
    count(sim.listSel.value).then((n) => { entries = n; });
  }
  look(c.look_formula);
}

// A look's Formula and its values: a bundled one by name (`@formula:<stem>`,
// formulas.json, as the player has them built in: its source) or inline
// (`psb=`, the compiled Formula the app sends, base64url): `[src, q, psb]`.
let exs = null;
async function lookOf(path) {
  if (!path.startsWith('@formula:')) {
    const q = new URLSearchParams(path.slice(9)), b = q.get('psb') || '';
    let psb = null;
    try { psb = b ? Uint8Array.from(atob(b.replace(/-/g, '+').replace(/_/g, '/')), (c) => c.charCodeAt(0)) : null; } catch { psb = new Uint8Array(); }
    return ['', q, psb];
  }
  const [n, q = ''] = path.slice(9).split('?');
  const ex = await (exs ??= fetch('formulas.json').then((r) => r.json()).catch(() => (exs = null, [])));
  return [ex.find((x) => x.stem == n)?.src || '', new URLSearchParams(q), null];
}
// The look loaded into pkg-psi (asLook / asLookPsb: the player's FormulaLook
// composition), or why not.
const asLook = (P, src, psb) => (psb ? P.asLookPsb(psb) : src ? !!P.asLook(src) : false) ? '' : (psb || src ? P.error() : 'there is no bundled Formula by that name');

// The Formula look on the demo's panel: Psi (an instance of its own) over
// every frame, at the panel's clock and the formula's sliders, as its `look`
// line asks (pkg-psi asLook: the player's FormulaLook composition).
async function look(path) {
  if (path == lookOn) return;
  lookOn = path;
  lookErr = '';
  if (!path) { sim.preview.p.setOver(undefined); return; }
  try {
    const P = await pkg(), [src, q, psb] = await lookOf(path);
    if (lookOn != path || !sim) return;
    const Q = (k, v = q.get(k)) => (/^-?(\d+\.?\d*|\.\d+)$/.test(v || '') && Math.abs(+v) < 32768 ? Math.round(+v * 65536) : -2147483648); // unset (also empty, junk, out of range): the formula's default, as fx_psi's query
    const why = asLook(P, src, psb);
    if (why) throw new Error(why);
    const loop = P.info()[4] || 0;
    sim.preview.p.setOver((px) => {
      const t = Math.floor(performance.now()), d = new Date();
      new Uint8Array(P.mem.buffer, P.under(), 12288).set(px);
      P.frameOver(loop ? t % loop : t >>> 0, d.getHours(), d.getMinutes(), d.getSeconds(), Q('a'), Q('b'), Q('c'));
      px.set(new Uint8Array(P.mem.buffer, P.pixels(), 12288));
    });
  } catch (e) {
    lookErr = e.message;
    sim.preview.p.setOver(undefined);
  }
  app.status = status();
  emit('status', app.status);
}

// The player's checks of a Formula look before it is saved (fx_psi's
// FormulaLook::compile): it compiles, lets the GIF show through, ≤ 12 ms.
async function lookCheck(path) {
  const P = await pkg(), [s, , b] = await lookOf(path), back = lookOn && (await lookOf(lookOn));
  const no = asLook(P, s, b), i = !no && P.info();
  const why = no || (!(i[5] & 32) ? 'this Formula covers the GIF entirely: choose a lower amount or another blend'
    : i[6] > 12000 ? `this formula is too heavy to draw over GIFs (about ${Math.ceil(i[6] / 1000)} ms a frame; the limit is 12 ms) — simplify it` : '');
  // (One instance: the look on the panel goes back to its own formula.)
  if (back) asLook(P, back[0], back[2]);
  if (why) bad('look_formula: ' + why);
}

// Now's banner while the schedule has the panel (schedule.js' bannerFor).
function banner() {
  const v = document.getElementById('view-now');
  if (!v?.firstChild) return;
  let el = document.getElementById('now-banner');
  if (!el) v.prepend(el = h('div.banner', { id: 'now-banner' }));
  const b = bannerFor(app.status, apply);
  el.hidden = !b;
  el.dataset.mode = app.status.mode;
  if (b) el.replaceChildren(...b.filter(Boolean));
}

// What only a player can do: its controls stay, with a line saying so; a
// press says it again.
const CANT = [
  ['#st-fw', 'The demo can’t install firmware: on a display, this updates it over WiFi.', '#fw-go,#fw-check'],
  ['#st-net', 'The demo can’t scan for or join WiFi networks; a new display name is shown, not announced.', '#st-scan'],
  ['#st-status', 'The demo can’t restart: on a display, Reboot does.', '#st-reboot-btn'],
  ['#st-panel', 'The demo has no panel to test or restart: changes here are tried and kept as on a display, with nothing to see.', '#pt-test,#pt-restart'],
];
function lines(root) {
  for (const [g, text, btns] of CANT) {
    const el = root.querySelector(g);
    if (!el?.firstChild || el.querySelector('.demo-cant')) continue;
    el.firstChild.after(h('p.cap.demo-cant', text));
    for (const b of el.querySelectorAll(btns)) {
      if (b.id == 'pt-test') b.hidden = false;
      b.addEventListener('click', (e) => { e.stopImmediatePropagation(); e.preventDefault(); toast(text); }, true);
    }
  }
}

export async function simulate(src) {
  try { version = (await (await fetch('version.txt')).text()).trim().split('\n')[0]; } catch { /* unknown */ }
  Object.assign(src, {
    config: async () => ({ ...S.c, ...panelFields() }),
    putConfig: async (p) => { if (p.look_formula && p.look_formula != S.c.look_formula) await lookCheck(p.look_formula); return putConfig(p); },
    status: async (recent) => status(recent),
    schedule: async () => structuredClone(S.sched),
    putSchedule: async (s) => putSchedule(s),
    wake: async (minutes) => { S.wake = minutes > 0 ? Date.now() + minutes * 60000 : 0; keep(); apply(); return { ok: true }; },
    json: async (m, u, b) => json(m, u, b),
    reboot: async () => { throw cant('restart: on a display, Reboot restarts it'); },
    wifiScan: async () => { throw cant('scan for WiFi networks'); },
    reindex: async () => ({ ok: true, entries: entries = await count() }),
    cache: async () => { const n = [...src.files.keys()].filter((p) => /^\/gifs\/.*\.gif$/i.test(p)).length; return { total: n, prepared: n, state: 'ready', auto: { mode: S.c.auto_prepare, state: 'done', failed: 0 } }; },
    cacheList: async () => [...src.files.keys()].map((p) => app.wasm.cachePathFor(p)).filter(Boolean),
    cacheFailed: async () => [],
    // Now's simulator: its playlist, platforms, shuffle, dwell and panel are the settings'.
    drive(s) {
      sim = s;
      driven = lookOn = '';
      const show = s.preview.show.bind(s.preview);
      s.preview.show = (p, get) => { cur = { path: p, at: performance.now(), n: cur.n + 1 }; started++; return show(p, get); };
      apply();
    },
  });
  app.config = await src.config();
  app.status = status();
  count().then((n) => { entries = n; }, () => {});
  const mo = new MutationObserver(() => lines(document.getElementById('view-settings')));
  mo.observe(document.getElementById('view-settings'), { childList: true, subtree: true });
  // The schedule's clock moves on: applied again every 20 s (the app's scheduler).
  app.watch('demo', 20000, async () => apply());
  // "Use your own card…": the folder is local mode, with no player; this one stops.
  on('source', (s) => {
    if (s === src || !live) return;
    live = false;
    app.unwatch('demo');
    mo.disconnect();
    sim?.preview.p.setOver(undefined);
    sim = null;
    style.remove();
    document.documentElement.style.removeProperty('--demo-b');
    document.getElementById('now-banner')?.remove();
    app.config = app.status = null;
    dropKey();
  });
}
