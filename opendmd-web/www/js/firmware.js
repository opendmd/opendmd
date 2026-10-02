// Settings → Advanced → Firmware and Security (imported when Advanced is
// first open; its own size line). Firmware: the running version (GET
// /api/firmware) and "Update from file…": a signed bundle (.odfw,
// docs/ota.md) checked here for its header, PUT with progress, then
// /api/status polled until the player is back on the new version. Security:
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
    h('h2', 'This player needs its API key'), i, h('p.row', h('button.pri', { value: 1 }, 'OK'), h('button', 'Cancel'))));
  d.onclose = () => { const k = d.returnValue && i.value.trim(); d.remove(); if (k) store.set('apiKey', k); res(k); };
  document.body.append(d);
  d.showModal();
});

// The PUT as an XMLHttpRequest, for its upload progress.
function send(file, progress) {
  return new Promise((res, rej) => {
    const x = new XMLHttpRequest();
    x.open('PUT', '/api/firmware');
    for (const [k, v] of Object.entries(auth({}))) x.setRequestHeader(k, v);
    x.upload.onprogress = (e) => progress(e.loaded / e.total);
    x.onload = () => {
      let why = x.statusText;
      try { why = JSON.parse(x.responseText).error || why; } catch { /* not JSON */ }
      const s = x.status;
      if (s === 200) res();
      else rej(new Error(s === 422 ? `The player refused this file: ${why}` : s === 408 ? 'The upload was cut short. Nothing was installed; try again'
        : s === 413 ? 'This file is too big to be OpenDMD firmware' : s === 503 ? `The player can’t take it now (${why}). Try again in a moment`
          : `Couldn’t update (${s}: ${why})`));
    };
    x.onerror = () => rej(new Error('The upload failed: the player stopped answering'));
    x.send(file);
  });
}

export function fwCards(fw, sec) {
  // --- Firmware
  const facts = h('dl.facts', { id: 'fw-facts' });
  const msg = h('p.cap', { id: 'fw-msg', style: 'margin-top:12px' });
  const bar = h('i'), barBox = h('div.bar', { hidden: true, style: 'margin-top:12px' }, bar);
  const file = h('input', { type: 'file', accept: '.odfw', id: 'fw-file', hidden: true, onchange: pick });
  const btn = h('button', { id: 'fw-go', onclick: () => file.click() }, 'Update from file…');
  const say = (t, bad) => { msg.textContent = t; msg.className = bad ? 'cap bad' : 'cap'; };
  async function load() {
    try {
      const f = await app.src.json('GET', '/api/firmware');
      clear(facts, fact('Version', f.version), fact('Build', f.build), fact('Waiting to install', f.pending), fact('Last update', f.last));
    } catch (e) { clear(facts, fact('Version', 'unknown: ' + e.message)); }
  }
  async function pick() {
    const f = file.files[0];
    file.value = '';
    if (!f) return;
    const head = new Uint8Array(await f.slice(0, 140).arrayBuffer()), td = new TextDecoder();
    if (f.size < 140 || td.decode(head.subarray(0, 5)) !== 'ODFW1') return say('This is not an OpenDMD firmware file (.odfw)', 1);
    const v = td.decode(head.subarray(8, 40)).replace(/\0+$/, '');
    if (!confirm(`Install firmware ${v}? The panel goes dark for about half a minute.`)) return;
    btn.disabled = true;
    barBox.hidden = false;
    try {
      await send(f, (p) => { bar.style.width = `${100 * p}%`; say(`Sending… ${Math.round(100 * p)}%`); });
      barBox.hidden = true;
      say('Restarting… the panel comes back in about 20 s');
      for (const t = Date.now(); ;) {
        await sleep(2000);
        try { if ((await app.src.status()).version === v) break; } catch { /* still restarting */ }
        if (Date.now() - t > 120000) throw new Error(`The player has not come back with ${v} after 2 minutes. Check it has power, then reload this page`);
      }
      say(`Updated to ${v}`);
      app.refreshStatus().catch(() => {});
      load();
    } catch (e) { say(e.message, 1); }
    btn.disabled = false;
    barBox.hidden = true;
  }
  fw.append(h('h3', 'Firmware'), facts, barBox, msg, h('div.row', { style: 'margin-top:12px' }, btn, file));
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
  sec.append(head, h('p.cap', 'Keeps casual visitors and other web pages out. It does not protect against someone on the same WiFi: the player speaks plain HTTP.'),
    h('label', { style: 'margin-top:12px' }, sw, 'Require a key for the API'), shown);
  sec.hidden = false;
}
