// The web demo's display (imported by app.js for the demo only; its own size
// line, never loaded from a device): a simulated OpenDMD player, so every
// view runs its device code (`kind = 'device'`). Its card is the LocalSource
// in memory; its panel is a canvas of its own with the wasm Preview (the
// demo's one true panel: Now's preview mirrors it, as it does a real one),
// playing the playlist with the firmware's dwell, transition, colour, glow
// and Formula look (drawn with Psi over every frame, `Preview.setOver`), and
// Formulas (Show on panel, a playlist's, the start-up show's). It answers
// the device API in the page: settings, the schedule and night mode, the
// status and its stream, skip, pause, play, the card's files, moves, the
// playback cache's records, overrides — kept for this browser session
// (sessionStorage) where a player keeps them. What only a player can do
// (firmware, WiFi, restarting from Settings, testing the panel, formatting
// a card) keeps its controls and says in a line that the demo can't; a
// playlist's per-GIF overrides are kept on the card but not played.
//
// Its world (`window.opendmdDemo`, for the site's desk scene): power, the
// card, the network, in compressed time. Off: every call fails like a
// dropped connection, the panel goes dark. On: a fresh start (its clock and
// uptime from zero), the start-up show, then the playlist, and the network
// a little later. The card out: a restart, then the status card; back in:
// mounted again, the playlist plays.

import { app, setPath } from './state.js';
import { HttpError, sent, queued } from './source.js';
import { ZONES } from './schedule.js';
import { fileFor } from './overrides.js';
import { Q as Qn, num, b64 } from './psikit.js';
import { load } from './wasm.js';
import web from './web.js';
import { h, emit, on, bus, label, toast, isFormula, formulaName } from './util.js';

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
const F0 = fresh().c;
S.c = Object.fromEntries(Object.entries({ ...F0, ...S.c }).filter(([k]) => k in F0));
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

// --- the card: the LocalSource in memory (its own methods, `L`; app.src may
// not be set yet when apply() first runs). With no card in, a card call is a 404.
let R = null, L = {}, failed = new Map();
const need = () => { if (!W.card) throw new HttpError(404, 'no card'); };
const playing = (r = evaluate()) => setPath(r.set ?? S.c.set);
const has = (path) => R.files.has(path);
async function count(set = playing(), platforms = S.c.platforms) {
  const text = W.card && await L.readText(set);
  return text ? app.wasm.filterManifest(text, JSON.stringify({ platforms })).length : 0;
}
const copyOf = (p) => { const c = app.wasm.cachePathFor(p); return c && has(c) ? c : ''; };

// --- the player and its world
const E = new Uint8Array(), MS = { short: 1300, full: 5600 }, IP = '192.0.2.10', KEEP = { '1m': 60, '5m': 300, hold: 0 };
const sleep = (ms) => new Promise((r) => setTimeout(r, ms));
const down = () => Object.assign(new TypeError('Failed to fetch'), { link: true });
const Q = (v) => Qn(num(v));
const W = { power: true, card: true, up: true, b: 96 }, subs = new Set(), pc = h('canvas.demo-panel');
const snap = () => ({ ...W, canvas: pc });
const tell = () => { const s = snap(); for (const f of subs) f(s); };
// mode: 'play' (the playlist), 'boot' (the start-up show), 'card' (the
// status card: no card), 'off'. F: the Formula on the panel (its pkg-psi
// instance, values, grid, loop, length; `boot`: the start-up show's line).
let K = 5, pv = null, mode = 'play', gen = 0, list = [], li = -1, hist = [], hc = 0, F = null, held = null, going = null, seed = 7, tok = 0, bt = 0, raf = 0, running = true, cardT = 0;
let cur = { path: '', at: 0, n: 0 }, pausedAt = 0, pausedFor = 0, title = '', kept = null, ovr = '', bootErr = '';
let entries = 0, started = 0, up0 = Date.now() - 4 * 3600e3, t0 = 0, version = '';
const ver = () => version.replace(/^v/, '') || 'demo';
const elapsed = () => (pausedAt || performance.now()) - cur.at - pausedFor;
const dwell = () => (S.c.dwell_min + Math.random() * (S.c.dwell_max - S.c.dwell_min)) * 1000;

function setPaused(v) {
  if (v == pv.paused) return;
  pv.setPaused(v);
  if (v) pausedAt = performance.now(); else { pausedFor += performance.now() - pausedAt; pausedAt = 0; }
}
// The panel at K canvas px per LED, in the renderer's own look (not this page's preview sliders).
function newPanel() {
  pv?.free();
  pc.width = 128 * K;
  pv = new web.Preview(pc, 2.2, 1, web.defaultBloom(), false);
}
function dress() {
  const c = S.c;
  pv.setTransition(c.transition ?? 'dissolve', c.transition_ms ?? 400);
  pv.setDisplay(c.look ?? '', c.glow ?? '');
  if (mode == 'play') pv.setDwell(c.dwell_min, c.dwell_max); else pv.setDwell(0, 0);
}
// A virtual file (the start-up show, the status card), played by the page's clock.
const virt = (p) => { pv.load(p, E); pv.syncTo(0, (held = [p, E, performance.now()])[2], 1); };

let fq = null;
const fpkg = () => (fq ??= load('pkg-psi', 'demo-f').then(([m, x]) => ({ ...m, mem: x.memory })).catch((e) => { fq = null; throw e; }));
// A Formula's text, as the player finds it: bundled by name, yours on the
// card, this page's last Show on panel, or inline (the saved or bundled
// Formula it was compiled from, js/nowpsi.js).
async function fsrc(p) {
  const h0 = p.split('?')[0];
  if (h0 == '@formula:~sent') return (await import('./psishow.js')).sent()?.src;
  if (h0.startsWith('@formula:')) return (await bundled()).find((x) => x.stem == h0.slice(9))?.src;
  if (h0[0] == '/') { need(); return L.readText(h0); }
  return (await (await import('./nowpsi.js')).find(p))?.src;
}
async function formula(p) {
  const src = await fsrc(p), M = await fpkg();
  if (!src || !M.compile(src)) throw new HttpError(404, 'not on the panel');
  M.k = src;
  const q = new URLSearchParams(p.split('?')[1]), i = M.info();
  return { M, src, v: [...'abc'].map((k) => Q(q.get(k))), g: i[10] || 20, lp: i[4] === 0 ? 2 ** 32 : i[4] || 8000, n: -1, name: /^\s*name\s+"([^"\n]*)"/m.exec(src)?.[1] };
}
function drawF() {
  const n = Math.floor(Math.max(0, elapsed()) / F.g), M = F.M, d = new Date();
  if (n === F.n) return;
  F.n = n;
  if (M.k !== F.src && !M.compile((M.k = F.src))) throw 0;
  M.frame((n * F.g) % F.lp, d.getHours(), d.getMinutes(), d.getSeconds(), ...F.v);
  const px = new Uint8Array(M.mem.buffer, M.pixels(), 12288);
  if (F.boot) pv.showBoot(px, F.boot, n * F.g); else pv.showRgb(px);
}
function tick() {
  raf = running && live ? requestAnimationFrame(tick) : 0;
  if (!W.power || !raf) return;
  if (F) try { drawF(); } catch { F = null; if (mode == 'play') advance(); }
  const r = pv.frame();
  if (mode == 'play' && (F ? !F.boot && elapsed() >= F.dur : r === 2 || r < 0)) advance();
}
const advance = () => (going ??= next().finally(() => { going = null; }));

// Play `p` now (`rec`: a new start, into Recently played by the device's
// rules, opendmd-core history.rs; not for Previous).
async function show(p, rec) {
  // (Last caller wins: a slow read never replaces a later show.)
  const t = ++tok, g = gen, f = isFormula(p) && await formula(p), b = f ? E : (need(), await L.readBytes(copyOf(p) || p).catch(() => { throw new HttpError(404, 'not on the card'); }));
  if (t != tok || g != gen || mode != 'play') return;
  // (The panel's sent Formula is not a file to play again: Recently played leaves it out, as the player does.)
  if (rec && !p.startsWith('@formula:~sent')) {
    if (hc) hist.unshift(...hist.splice(hc, 1));
    hc = 0;
    hist = [p, ...hist.filter((x) => x !== p)].slice(0, 16);
  }
  const sent = p.startsWith('@formula:~sent');
  kept = sent ? KEEP[S.c.formula_keep] ?? null : null;
  if ((F = f || null)) F.dur = kept == null ? dwell() : kept ? kept * 1000 : Infinity;
  else { pv.load(p, b); held = [p, b]; }
  title = f ? f.name || formulaName(p) : label(app.wasm.parsePath(p));
  // (A Formula shown over the GIF stays until the playlist moves on.)
  if (ovr) { ovr = ''; look(S.c.look_formula); }
  cur = { path: p, at: performance.now(), n: cur.n + 1 };
  pausedFor = 0;
  pausedAt = pv.paused ? cur.at : 0;
  started++;
}
// The next in the playlist: GIFs the card lacks and Formulas that can't be played are passed over, a lap at most.
async function next(tries = 0) {
  if (mode != 'play' || !list.length || tries >= list.length) return;
  li = (li + 1) % list.length;
  const p = list[li];
  if (!isFormula(p) && (p[0] != '/' || !has(p))) return next(tries + 1);
  try { await show(p, 1); } catch { return next(tries + 1); }
}
// The playlist in play, read again; `go`: started from its top (a new one), else on after the GIF on screen (its file written).
async function rebuild(go) {
  const t = ++bt, g = gen, p = playing(), text = W.card ? await L.readText(has(p) ? p : '/filelist.txt').catch(() => null) : null;
  if (t != bt || g != gen) return;
  list = text ? app.wasm.filterManifest(text, JSON.stringify({ platforms: S.c.platforms, shuffle: S.c.shuffle, seed })) : [];
  entries = list.length;
  li = go ? -1 : list.indexOf(cur.path);
  if (go && mode == 'play') { going = null; await advance(); }
}
function start() {
  mode = 'play';
  F = null;
  dress();
  driven = '';
  apply();
}
function noCard() {
  mode = 'card';
  F = null;
  dress();
  virt('@boot?' + new URLSearchParams({ len: 'card', v: ver(), ip: IP, note: 'Insert a microSD card' }));
}
// A start: the clock and uptime from zero, the show (or its Formula), then
// the card or the status card; the network answers a little later.
async function boot() {
  const g = ++gen;
  W.up = false;
  mode = 'boot';
  F = held = null;
  ovr = bootErr = title = '';
  hist = [];
  list = [];
  hc = started = 0;
  t0 = performance.now();
  up0 = Date.now();
  cur = { path: '', at: t0, n: 0 };
  seed = (Math.random() * 1e6) | 0;
  setPaused(false);
  pausedFor = pausedAt = 0;
  pv.clear();
  dress();
  look(S.c.look_formula);
  tell();
  setTimeout(() => { if (g == gen) { W.up = true; tell(); app.soon('now'); } }, 2600);
  await sleep(350);
  const len = S.c.boot_show, q = '@boot?' + new URLSearchParams({ len, v: ver(), ip: IP });
  if (g == gen && MS[len]) {
    try { if (S.c.boot_formula) (F = await formula(S.c.boot_formula)).boot = q; } catch { bootErr = 'it couldn’t be read'; }
    if (g != gen) return;
    cur.at = performance.now();
    if (!F) virt(q);
    await sleep(MS[len]);
  }
  if (g != gen) return;
  if (W.card) start(); else noCard();
}

const world = {
  get state() { return snap(); },
  on(f) { subs.add(f); f(snap()); return () => subs.delete(f); },
  power(v) {
    if (!v == !W.power || !live) return;
    W.power = !!v;
    if (v) { boot(); return; }
    gen++;
    mode = 'off';
    F = null;
    W.up = false;
    pv.clear();
    tell();
  },
  // Out while it plays: its reads fail, three in a minute restart the
  // player, and it comes up with no card (the status card), trying the
  // card again (every minute on a display; here, soon).
  // (Back in before the restart: none.)
  card(v) {
    if (!v == !W.card || !live) return;
    W.card = !!v;
    tell();
    const g = gen;
    clearTimeout(cardT);
    if (W.power) cardT = setTimeout(() => { if (g == gen) { if (!v) boot(); else if (mode == 'card') start(); } }, v ? 1500 : 1000);
  },
  // Everything stops (the panel, the status stream, the app's reads) while false: the desk scene off screen.
  run(v) {
    if (!live) return;
    running = !!v;
    app.hold(!running);
    if (running && !raf && live) raf = requestAnimationFrame(tick);
  },
  // Now's mirror of the panel drawn (true) or not (a page that shows this panel itself).
  mirror(v) { app.noMirror = !v; emit('mirror'); },
  // Canvas px per LED (1..12); what is on the panel starts again at the new size.
  scale(k) {
    if (!live) return;
    K = Math.min(12, Math.max(1, Math.round(k) || 5));
    const p = pv.paused;
    newPanel();
    dress();
    if (p) pv.setPaused(true);
    lookOn = '';
    look(ovr || S.c.look_formula);
    if (!W.power || F || !held) return;
    pv.load(held[0], held[1]);
    if (held[2]) pv.syncTo(0, held[2], 1);
    else { cur.at = performance.now(); pausedFor = 0; pausedAt = p ? cur.at : 0; }
  },
};

function status(recent) {
  const r = evaluate(), c = S.c, w = S.wake && clock(S.wake), p = mode == 'play' ? cur.path : '';
  const s = {
    version: ver(), variant: '4mb', uptime_secs: Math.round((Date.now() - up0) / 1000), t_ms: Math.round(performance.now() - t0),
    ip: IP, wifi: 'sta', hostname: c.hostname, mdns: c.hostname,
    now_playing: { ordinal: cur.n, path: p, title: p && title, elapsed_ms: Math.round(p ? elapsed() : 0), frame: p ? 1 : 0, ...(p && kept != null && { kept_s: kept }) },
    paused: !!pv?.paused, entries: W.card ? entries : 0, manifest: '/sdcard' + playing(r), index_state: W.card ? 'ready' : 'unavailable', www_version: version || null,
    heap: { free: 61440, floor: 48128, idf_min: 45056, transitions_cut: 0 }, refresh_hz: 100, deadline_misses: 0, worst_lateness_ms: 0,
    misses: { steady: 0, switch: 0, threshold_ms: 4, window_ms: 2000 }, gifs_started: started,
    sd: W.card ? { mounted: true, readable: true, card_id: 'demo', capacity_bytes: 31914983424, clock_khz: c.card_speed == 'fast' ? 40000 : 20000, failures: 0, attempts: 1 }
      : { mounted: false, readable: false, capacity_bytes: 0, failures: 0, attempts: 2, error: 'no card' },
    mode: r.mode, mode_until: r.mode_until, time: hhmm(clock().min), tz: c.tz_name, time_synced_s: 60, wake_until: w ? hhmm(w.min) : null,
    schedule: { rule: r.rule, set: r.set, until: r.until },
    context: { enabled: c.follow, state: 'unreachable', error: 'the demo can’t reach your network' },
    look: c.look, glow: c.glow, glow_held: 0, formula_look: ovr || c.look_formula ? { error: lookErr || null, held: 1 } : null,
    boot_formula: bootErr ? { error: bootErr } : null,
    max_brightness: c.max_brightness, brightness_safe_max: SAFE, safe_start_until: null,
  };
  if (recent) Object.assign(s, { recent: hist.slice(), recent_at: hc });
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

// Show on panel (js/psishow.js): the compiled Formula as the body. On its
// own: played as `@formula:~sent` (its text is the page's last send, read
// once the answer is in), for as long as Keep on the panel says; over the
// GIF: drawn as the look until the playlist moves on. `live`: a change to
// the one shown, or a 409 once it has moved on.
async function playFormula(url, body) {
  const q = new URLSearchParams(url.split('?')[1]), live = q.has('live'), o = q.has('over');
  q.delete('live');
  q.delete('over');
  const v = String(q), sent = cur.path.startsWith('@formula:~sent');
  if (mode != 'play') throw new HttpError(503, 'nothing playing');
  if (o) {
    if (live && !ovr) throw new HttpError(409, 'it has moved on');
    look((ovr = '@formula?psb=' + b64(new Uint8Array(await body.arrayBuffer())) + (v && '&' + v)));
    return {};
  }
  if (live && !sent) throw new HttpError(409, 'it has moved on');
  const p = '@formula:~sent' + (v && '?' + v);
  setTimeout(async () => {
    try {
      if (!live) return await show(p, 1);
      const f = await formula(p);
      if (F && cur.path.startsWith('@formula:~sent')) { Object.assign(F, { ...f, dur: F.dur }); cur.path = p; title = f.name || 'Formula'; }
    } catch { /* gone meanwhile */ }
  });
  return {};
}

// The rest of what the device's views ask, by route.
async function json(method, url, body) {
  const r = method + ' ' + url.split('?')[0];
  if (r == 'GET /api/name-check') return { taken: false };
  if (r == 'POST /api/card/full-speed') return { applied: 'now' };
  if (r == 'POST /api/follow/test') return { ok: false, reachable: false, error: 'the demo can’t reach your network' };
  if (r == 'GET /api/firmware') return { version: status().version, variant: '4mb', flash_size: 4 << 20, build: 'the web demo' };
  if (r == 'GET /api/www') return { ok: true, version };
  if (r == 'POST /api/panel/test') throw cant('show test pictures');
  // Show on panel's "off" for a Formula over the GIF.
  if (r == 'POST /api/play' && !body?.path && body?.over) { ovr = ''; look(S.c.look_formula); return { ok: true }; }
  if (r == 'POST /api/play/formula') return playFormula(url, body);
  // A folder and everything in it (an emptied GIF folder's prepared copies).
  if (r.startsWith('DELETE /api/files/')) {
    need();
    const p = decodeURIComponent(r.slice(17));
    for (const k of [...R.files.keys()]) if (k.startsWith(p + '/')) await L.deleteFile(k);
    await L.deleteFile(p);
    return { ok: true };
  }
  if (r == 'POST /api/card/format') throw cant('format a card');
  throw cant('do that');
}

// --- what the settings do to the demo's panel (and to Now's mirror of it, so the plain page shows them too)
const style = h('style', '#view-now #now-panel,.demo-panel{filter:brightness(var(--demo-b,1))}');
document.head.append(style);
let driven = '', lookOn = '', lookErr = '', psi = null, overFn, mir = null;
const dropKey = () => { try { localStorage.removeItem('odmd.apiKey'); } catch { /* none */ } };
let live = true;
const pkg = () => (psi ??= load('pkg-psi', 'demo').then(([m, x]) => ({ ...m, mem: x.memory })));
const over = (f) => { if (!live) return; overFn = f; pv.setOver(f); mir?.setOver(f); };
function apply() {
  if (!live) return;
  const r = evaluate(), c = S.c, b = Math.min(r.b ?? c.brightness, c.max_brightness);
  // Brightness 96 (a new player's) draws the panel as it is.
  document.documentElement.style.setProperty('--demo-b', b ? ((b / 96) ** 0.45).toFixed(3) : 0);
  if (b != W.b) { W.b = b; tell(); }
  if (W.up) { app.status = status(); emit('status', app.status); }
  dress();
  const want = [playing(r), c.platforms, c.shuffle].join('\n');
  if (want != driven) { driven = want; if (mode == 'play') rebuild(1).catch(() => {}); }
  look(ovr || c.look_formula);
}

// A look's Formula and its values: a bundled one by name (`@formula:<stem>`,
// formulas.json, as the player has them built in: its source) or inline
// (`psb=`, the compiled Formula the app sends, base64url): `[src, q, psb]`.
let exs = null;
const bundled = () => (exs ??= fetch('formulas.json').then((r) => r.json()).catch(() => (exs = null, [])));
async function lookOf(path) {
  if (!path.startsWith('@formula:')) {
    const q = new URLSearchParams(path.slice(9)), b = q.get('psb') || '';
    let psb = null;
    try { psb = b ? Uint8Array.from(atob(b.replace(/-/g, '+').replace(/_/g, '/')), (c) => c.charCodeAt(0)) : null; } catch { psb = new Uint8Array(); }
    return ['', q, psb];
  }
  const [n, q = ''] = path.slice(9).split('?');
  return [(await bundled()).find((x) => x.stem == n)?.src || '', new URLSearchParams(q), null];
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
  if (!path) { over(undefined); return; }
  try {
    const P = await pkg(), [src, q, psb] = await lookOf(path);
    if (lookOn != path || !live) return;
    // (Unset, also empty, junk, out of range: the formula's default, as fx_psi's query.)
    const why = asLook(P, src, psb), a = Q(q.get('a')), b = Q(q.get('b')), c = Q(q.get('c'));
    if (why) throw new Error(why);
    const loop = P.info()[4] || 0;
    over((px) => {
      const t = Math.floor(performance.now()), d = new Date();
      new Uint8Array(P.mem.buffer, P.under(), 12288).set(px);
      P.frameOver(loop ? t % loop : t >>> 0, d.getHours(), d.getMinutes(), d.getSeconds(), a, b, c);
      px.set(new Uint8Array(P.mem.buffer, P.pixels(), 12288));
    });
  } catch (e) {
    lookErr = e.message;
    over(undefined);
  }
  if (W.up) { app.status = status(); emit('status', app.status); }
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

// What only a player can do: its controls stay, with a line saying so; a
// press says it again.
const CANT = [
  ['#st-fw', 'The demo can’t install firmware: on a display, this updates it over WiFi.', '#fw-go,#fw-check'],
  ['#st-net', 'The demo can’t scan for or join WiFi networks; a new display name is shown, not announced.', '#st-scan'],
  ['#st-status', 'The demo can’t restart: on a display, Reboot does.', '#st-reboot-btn'],
  ['#st-panel', 'The demo can’t test or restart its panel: changes here are tried and kept as on a display, with nothing to see.', '#pt-test,#pt-restart'],
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

// Everything the app asks goes over the "network", answered after a moment;
// nothing answers while the display is off or starting. Retried as
// js/source.js's DeviceSource retries the same call (`sent`, `queued`).
const NET = { config: 1, putConfig: 1, skip: 1, pause: 1, play: 1, putSchedule: 1, wake: 1, forget: 1, cacheFail: 1, move: 2, writeFile: 2, deleteFile: 2, saveSet: 2,
  list: 3, readText: 3, readBytes: 3, cacheList: 3, cacheFailed: 3, status: 0, now: 0, events: 0, reindex: 0, reboot: 0, wifiScan: 0, stat: 0, cache: 0, overrides: 0, schedule: 0, json: 0, req: 0 };
function gate(src) {
  for (const [k, w] of Object.entries(NET)) {
    const f = src[k].bind(src), g = async (...a) => { await sleep(8); if (!W.up) throw down(); return f(...a); };
    src[k] = w == 3 ? (...a) => queued(() => g(...a)) : w ? (...a) => sent(() => g(...a), w == 1) : g;
  }
}

// The card's files: a GIF written anew loses its "can't be prepared"
// line; the playlist in play, written, is read again.
async function wf(p, d, o) {
  need();
  if (o?.createNew && has(p)) throw new HttpError(409, 'exists');
  const r = await L.writeFile(p, d);
  failed.delete(p);
  if (p == playing()) rebuild().catch(() => {});
  return r;
}

export async function simulate(src) {
  try { version = (await (await fetch('version.txt')).text()).trim().split('\n')[0]; } catch { /* unknown */ }
  R = src;
  for (const k of ['list', 'readText', 'readBytes', 'stat', 'writeFile', 'deleteFile']) L[k] = src[k].bind(src);
  Object.assign(src, {
    kind: 'device',
    config: async () => ({ ...S.c, ...panelFields() }),
    putConfig: async (p) => { if (p.look_formula && p.look_formula != S.c.look_formula) await lookCheck(p.look_formula); return putConfig(p); },
    status: async (recent) => status(recent),
    now: async () => { const s = status(); return { t_ms: s.t_ms, now_playing: s.now_playing, paused: s.paused }; },
    // GET /api/events: on change (looked at four times a second) and every 3 s.
    async events(f, signal) {
      for (let k = '', at = 0; live && !signal?.aborted; await sleep(250)) {
        if (!W.up) throw down();
        if (!running) continue;
        const s = status(), e = { brightness: S.c.brightness };
        for (const x of ['t_ms', 'now_playing', 'paused', 'entries', 'manifest', 'index_state', 'sd', 'mode', 'mode_until', 'wake_until', 'schedule', 'context']) e[x] = s[x];
        const key = JSON.stringify({ ...e, t_ms: 0, now_playing: { ...e.now_playing, elapsed_ms: 0 } }), t = performance.now();
        if (key != k || t - at >= 3000) { k = key; at = t; f(e, t); }
      }
    },
    schedule: async () => structuredClone(S.sched),
    putSchedule: async (s) => putSchedule(s),
    wake: async (minutes) => { S.wake = minutes > 0 ? Date.now() + minutes * 60000 : 0; keep(); apply(); return { ok: true }; },
    json,
    req: json,
    reboot: async () => { throw cant('restart: on a display, Reboot restarts it'); },
    wifiScan: async () => { throw cant('scan for WiFi networks'); },
    reindex: async () => ({ ok: true, entries: entries = await count() }),
    skip: async (back) => {
      if (mode != 'play') throw new HttpError(503, 'nothing playing');
      if (!back) await next();
      else if (hc + 1 < hist.length) await show(hist[++hc]);
      else throw new HttpError(404, 'nothing earlier');
      return { ok: true };
    },
    pause: async () => { if (mode != 'play') throw new HttpError(503, 'nothing playing'); setPaused(!pv.paused); return { paused: pv.paused }; },
    play: async (p) => {
      if (!isFormula(p)) { need(); if (!has(p)) throw new HttpError(404, 'not found'); if (!/\.gif$/i.test(p)) throw new HttpError(415, 'not a GIF'); }
      if (mode != 'play') throw new HttpError(503, 'nothing playing');
      await show(p, 1);
      return { ok: true };
    },
    writeFile: (p, d, o) => wf(p, d, o),
    // (A GIF's prepared copy goes with it.)
    async deleteFile(p) { need(); const c = copyOf(p); if (c) await L.deleteFile(c); return L.deleteFile(p); },
    saveSet(n, t) { return wf(`/sets/${n.replace(/\.txt$/, '')}.txt`, t); },
    list: async (d) => (need(), L.list(d)),
    readText: async (p) => (W.card ? L.readText(p) : null),
    // (What the panel has just read, Now's mirror reads again: given from memory, not fetched twice.)
    readBytes: async (p) => (need(), held?.[0] == p && held[1].length ? held[1] : L.readBytes(p)),
    stat: async (p) => (need(), L.stat(p)),
    // A rename: its prepared copy, Smooth motion line and "can't be prepared" line follow.
    async move(from, to) {
      need();
      if (!has(from)) throw new HttpError(404, 'not found');
      if (has(to)) throw new HttpError(409, 'exists');
      const c = copyOf(from);
      for (const [a, b] of [[from, to], c && [c, app.wasm.cachePathFor(to)]].filter(Boolean)) { await L.writeFile(b, await L.readBytes(a)); await L.deleteFile(a); }
      if (failed.has(from)) { failed.set(to, failed.get(from)); failed.delete(from); }
      const sm = await L.readText('/sets/.smooth');
      if (sm) await L.writeFile('/sets/.smooth', sm.split('\n').map((l) => (l.trim().toLowerCase() == from.toLowerCase() ? to : l)).join('\n'));
      return { ok: true, from, to };
    },
    forget: async (p) => { need(); const c = copyOf(p); if (c) await L.deleteFile(c); return { ok: true }; },
    overrides: async (set) => { need(); const t = await L.readText(fileFor(setPath(set))); return { set, playlist: setPath(set), overrides: t ? [...app.wasm.parseOverrides(t)] : [] }; },
    cache: async () => { need(); const n = [...R.files.keys()].filter((p) => /^\/gifs\/.*\.gif$/i.test(p)).length; return { total: n, prepared: n, state: 'ready', auto: { mode: S.c.auto_prepare, state: 'done', failed: 0 } }; },
    cacheList: async () => (need(), [...R.files.keys()].map((p) => app.wasm.cachePathFor(p)).filter(Boolean)),
    cacheFailed: async () => [...failed].map(([path, reason]) => ({ path, reason })),
    cacheFail: async (path, reason) => { failed.set(path, reason); return { ok: true }; },
  });
  gate(src);
  newPanel();
  raf = requestAnimationFrame(tick);
  // (`?desk`: a page that shows this panel itself, the site's desk scene: no mirror on Now.)
  if (new URLSearchParams(location.search).has('desk')) app.noMirror = true;
  app.config = await src.config();
  start();
  window.opendmdDemo = world;
  const mo = new MutationObserver(() => lines(document.getElementById('view-settings')));
  mo.observe(document.getElementById('view-settings'), { childList: true, subtree: true });
  // Now's mirror gets the look too (a new one, once Now is up again).
  const hook = () => { const m = document.getElementById('now-panel')?.preview?.p; if (m && m != mir) (mir = m).setOver(overFn); };
  bus.addEventListener('status', hook);
  // The schedule's clock moves on: applied again every 20 s (the app's scheduler).
  app.watch('demo', 20000, async () => apply());
  // "Use your own card…": the folder is local mode, with no player; this one stops.
  on('source', (s) => {
    if (s === src || !live) return;
    live = false;
    gen++;
    app.unwatch('demo');
    app.hold(false);
    app.noMirror = false;
    mo.disconnect();
    bus.removeEventListener('status', hook);
    cancelAnimationFrame(raf);
    overFn = undefined;
    mir = null;
    pv.free();
    pv = psi = fq = null;
    style.remove();
    document.documentElement.style.removeProperty('--demo-b');
    app.config = app.status = null;
    delete window.opendmdDemo;
    dropKey();
  });
}
