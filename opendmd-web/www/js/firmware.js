// Settings › Device › Firmware and Security (imported when the Device tab is
// first open; its own size line). Firmware: the running version (GET
// /api/firmware), the updater's log behind "Update log", and "Update from
// file…": a signed bundle (.odfw, docs/ota.md) checked here for its header
// (the other board's or table's is refused; an older version than the
// running one asks again: `older`), PUT with
// progress, then /api/status polled until the player is back on the new
// version; "Check for updates" is js/update.js, which reuses both. Security:
// the optional API key (docs/api.md "Security"), made here, shown once.
// And the dialog a 401 opens to ask for the key (source.js).

import { app } from './state.js';
import { auth } from './source.js';
import { h, clear, on, store, saver } from './util.js';

const fact = (k, v) => v != null && [h('dt', k), h('dd', v)];
const sleep = (ms) => new Promise((r) => setTimeout(r, ms));
const A = 'ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789';

// Resolves to the key typed (stored), or '' on Cancel.
export const askKey = () => new Promise((res) => {
  const i = h('input', { type: 'password', id: 'key-in', style: 'width:100%;box-sizing:border-box' });
  const d = h('dialog.card', { id: 'key-dlg', style: 'color:inherit;border:0;margin:auto;width:min(360px,90vw)' }, h('form', { method: 'dialog' },
    h('h2', 'This display needs its API key'), i, h('p.row', h('button.pri', { value: 1 }, 'OK'), h('button', 'Cancel'))));
  d.onclose = () => { const k = d.returnValue && i.value.trim(); d.remove(); if (k) store.set('apiKey', k); res(k); };
  document.body.append(d);
  d.showModal();
});

// Whether version `a` is older than `b`, semver-ish: the dot-separated
// numbers compared as numbers (a leading v ignored, a missing part is 0),
// and a pre-release (`-beta.3`) before its release, its dot-separated parts
// compared as numbers where both are, a number before a word, words by
// their characters (opendmd_core::ota::version_older, the same rule):
// 1.0.0-beta.9 < 1.0.0-beta.16 < 1.0.0 < 1.0.1-beta.1 < 1.0.1. The device
// accepts any validly signed bundle, older ones included (a way back); this
// only decides whether the page asks twice, and what an update is.
export function older(a, b) {
  const parse = (s) => {
    const [core, pre] = String(s).trim().replace(/^v/i, '').split('+')[0].split(/-(.*)/s);
    return [core.split('.').map((x) => parseInt(x, 10) || 0), pre == null ? null : pre.split('.')];
  };
  const [an, ap] = parse(a), [bn, bp] = parse(b), num = (x) => /^\d+$/.test(x);
  for (let i = 0; i < Math.max(an.length, bn.length); i++) {
    const d = (an[i] || 0) - (bn[i] || 0);
    if (d) return d < 0;
  }
  if (!ap) return false;
  if (!bp) return true;
  for (let i = 0; ; i++) {
    const x = ap[i], y = bp[i];
    if (x == null || y == null) return x == null && y != null;
    if (x === y) continue;
    return num(x) && num(y) ? +x < +y : num(x) || (!num(y) && x < y);
  }
}

// A PUT as an XMLHttpRequest, for its upload progress: resolves to the
// status and the reply's `error` (or its text), rejects only when the player
// stops answering. update.js sends the web app with it too.
export const put = (path, body, progress) => new Promise((res, rej) => {
  const x = new XMLHttpRequest();
  x.open('PUT', path);
  for (const [k, v] of Object.entries(auth({}))) x.setRequestHeader(k, v);
  x.upload.onprogress = (e) => progress(e.loaded / e.total);
  x.onload = () => {
    let why = x.statusText;
    try { why = JSON.parse(x.responseText).error || why; } catch { /* not JSON */ }
    res({ s: x.status, why, text: x.responseText });
  };
  x.onerror = () => rej(new Error('The upload failed: the display stopped answering'));
  x.send(body);
});

export const board = (v) => ({ '4mb': 'DevKit (4 MB)' })[v] || v;
const td = new TextDecoder();

// A bundle's flash layout (its magic): [variant, partition table], or null.
// Any other magic starting ODFW (the 8mb layout of 1.0.0-beta.16 and earlier)
// is a bundle for a layout this firmware does not build (otherLayout).
const LAYOUTS = { ODFW34mb: ['4mb', 3], 'ODFW1\0\0\0': ['4mb', 2] };
export const fwLayout = (b) => (b.length >= 140 && LAYOUTS[td.decode(b.subarray(0, 8))]) || null;
export const fwVariant = (b) => fwLayout(b)?.[0] || null;
export const otherLayout = (b) => b.length >= 140 && !fwLayout(b) && td.decode(b.subarray(0, 4)) === 'ODFW';
// The player's table (a firmware that does not say: v2).
export const playerLayout = (f) => f?.layout ?? 2;
// Why firmware for table `want` can't go on a player on `have` over WiFi.
export const layoutWhy = (want, have) => (want === have ? null : want > have
  ? 'This firmware needs a one-time USB reinstall on this display: use the installer at opendmd.dev/install (Update OpenDMD keeps your WiFi and settings)'
  : 'This file is for an older flash layout than this display has: get the latest from opendmd.dev/install');

// The bundle's version from its 140-byte header, or null if it is not one.
export const fwHead = (b) => fwVariant(b) && td.decode(b.subarray(8, 40)).replace(/\0+$/, '');

export async function sendFw(file, progress) {
  const { s, why } = await put('/api/firmware', file, progress);
  if (s !== 200) {
    throw new Error(s === 422 ? `The display refused this file: ${why}` : s === 408 ? 'The upload was cut short. Nothing was installed; try again'
      : s === 413 ? 'This file is too big to be OpenDMD firmware' : s === 503 ? `The display can’t take it now (${why}). Try again in a moment`
        : `Couldn’t update (${s}: ${why})`);
  }
}

// After a firmware PUT: /api/status polled until the player answers with
// version `v` (docs/ota.md: ~33 s from the end of the upload, playback ~45 s).
export async function restart(v, say) {
  say('Restarting… the panel comes back in about 45 s');
  for (const t = Date.now(); ;) {
    await sleep(2000);
    try { if ((await app.src.status()).version === v) return; } catch { /* still restarting */ }
    if (Date.now() - t > 180000) throw new Error(`The display has not come back with ${v} after 3 minutes. Check it has power, then reload this page (Update log says what the updater did)`);
  }
}

export function fwCards(fw, sec) {
  // --- Firmware
  const facts = h('dl.facts', { id: 'fw-facts' });
  const msg = h('p.cap', { id: 'fw-msg', style: 'margin-top:12px' });
  const bar = h('i'), barBox = h('div.bar', { hidden: true, style: 'margin-top:12px' }, bar);
  const file = h('input', { type: 'file', accept: '.odfw', id: 'fw-file', hidden: true, onchange: pick });
  const btn = h('button', { id: 'fw-go', onclick: () => file.click() }, 'Update from file…');
  const say = (t, bad) => { msg.textContent = t; msg.className = bad ? 'cap bad' : 'cap'; };
  // "Check for updates": js/update.js, imported on the first click.
  const busy = (b) => { btn.disabled = chk.disabled = b; };
  const ui = { say, bar, barBox, busy, load: () => load() };
  const chk = h('button', { id: 'fw-check', onclick: () => import('./update.js').then((m) => m.check(ui)) }, 'Check for updates');
  const logText = h('div.log', { id: 'fw-log-text', style: 'white-space:pre-wrap' });
  const log = h('details', { id: 'fw-log', hidden: true }, h('summary', 'Update log'), logText);
  let running = null, variant = null, layout = 2;
  async function load() {
    try {
      const f = await app.src.json('GET', '/api/firmware');
      running = f.version;
      variant = f.variant;
      layout = playerLayout(f);
      clear(facts, fact('Version', f.version), fact('Board', board(variant)), fact('Flash layout', `v${layout}`),
        fact('Build', f.build), fact('Waiting to install', f.pending), fact('Last update', f.last));
      logText.textContent = f.log || '';
      log.hidden = !f.log;
    } catch (e) { clear(facts, fact('Version', 'unknown: ' + e.message)); }
  }
  async function pick() {
    const f = file.files[0];
    file.value = '';
    if (!f) return;
    const head = new Uint8Array(await f.slice(0, 140).arrayBuffer()), v = fwHead(head), b = board(variant);
    if (otherLayout(head)) return say(`This file is for another flash layout than this display's. Use the ${b || board('4mb')} update file`, 1);
    if (!v) return say('This is not an OpenDMD firmware file (.odfw)', 1);
    if (variant && fwVariant(head) !== variant) return say(`This file is for the ${board(fwVariant(head))}, and this display runs the ${b} firmware. Use the ${b} update file`, 1);
    const why = variant && layoutWhy(fwLayout(head)[1], layout);
    if (why) return say(why, 1);
    const ask = running && older(v, running) ? `Firmware ${v}: this is older than the firmware on the panel (${running}). Install it anyway?`
      : `Install firmware ${v}?`;
    if (!confirm(`${ask} The panel goes dark for about a minute.`)) return;
    busy(true);
    barBox.hidden = false;
    try {
      await sendFw(f, (p) => { bar.style.width = `${100 * p}%`; say(`Sending… ${Math.round(100 * p)}%`); });
      barBox.hidden = true;
      await restart(v, say);
      say(`Updated to ${v}`);
      app.refreshStatus().catch(() => {});
      load();
    } catch (e) { say(e.message, 1); }
    busy(false);
    barBox.hidden = true;
  }
  // "Get beta updates": a device setting (`beta_updates`), so every browser
  // sees the same choice; update.js reads beta.json with it on.
  const fwh = h('h3', 'Firmware'), save = saver(fwh);
  const beta = h('input.sw', { type: 'checkbox', id: 'fw-beta' });
  const betaBox = h('div', { id: 'fw-beta-box', hidden: true }, h('label', { style: 'margin-top:12px' }, beta, 'Get beta updates'),
    h('p.cap', 'Betas are early versions of the next release: new things sooner, and they may have problems. You can switch this off at any time. That doesn’t take the display back to an older version: it keeps its beta until the next regular release comes out.'));
  const bsync = (c) => { betaBox.hidden = !c || !('beta_updates' in c); beta.checked = !!c?.beta_updates; };
  beta.onchange = () => save(async () => {
    try { await app.src.putConfig({ beta_updates: beta.checked }); } catch (e) { bsync(app.config); throw e; }
    await app.refreshConfig();
  });
  on('config', bsync);
  bsync(app.config);
  fw.append(fwh, facts, log, barBox, msg, ui.row = h('div.row', { style: 'margin-top:12px' }, chk, btn, file), betaBox);
  fw.hidden = false;
  load();

  // --- Security
  const head = h('h3', 'Security'), run = saver(head);
  const sw = h('input.sw', { type: 'checkbox', id: 'sec-key' });
  const shown = h('div', { id: 'sec-shown' });
  const sync = (c) => { sw.checked = !!c?.api_key_set; };
  sw.onchange = () => run(async () => {
    let k = '';
    if (sw.checked) while (k.length < 24) for (const b of crypto.getRandomValues(new Uint8Array(32))) if (b < 248 && k.length < 24) k += A[b % 62];
    try { await app.src.putConfig({ api_key: k }); } catch (e) { sync(app.config); throw e; }
    store.set('apiKey', k);
    const out = h('input', { type: 'text', id: 'sec-key-val', readOnly: true, value: k, style: 'flex:1;min-width:0' });
    clear(shown, k && [h('div.row', { style: 'margin-top:12px' }, out,
      h('button.sm', { onclick: () => { out.select(); document.execCommand('copy'); } }, 'Copy')),
    h('p.cap', 'Other browsers and scripts need this key. It is not shown again.')]);
    await app.refreshConfig();
  });
  on('config', sync);
  sync(app.config);
  sec.append(head, h('p.cap', 'Keeps casual visitors and other web pages out. It does not protect against someone on the same WiFi: the display speaks plain HTTP.'),
    h('label', { style: 'margin-top:12px' }, sw, 'Require a key for the API'), shown);
  sec.hidden = false;
}
