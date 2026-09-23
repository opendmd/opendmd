// Settings: the device's NVS config (GET/PUT /api/config — a PUT is a
// partial patch, applied live except WiFi and hostname, which need a
// reboot), a WiFi scan, reindex, reboot, OTA, and the facts from status.

import { app } from './state.js';
import { h, clear, toast, bytes, uptime } from './util.js';

export async function mount(el) {
  if (!app.device) {
    el.append(h('div.card', h('h2', 'Settings'), h('p.muted', 'Device settings, WiFi, OTA and reboot live on the player. This page is in local mode: open it from the player (http://opendmd.local/) to use them.')));
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
  };
  const brightOut = h('output.mono', c.brightness);
  f.brightness.oninput = () => { brightOut.textContent = f.brightness.value; };
  const nets = h('div.chips', { id: 'st-nets' });
  const banner = h('div.banner', { hidden: true, id: 'st-reboot' }, 'Saved. WiFi and hostname changes take effect after a reboot. ',
    h('button.sm', { onclick: reboot }, 'Reboot now'));
  const facts = h('div.grid', { id: 'st-facts' });
  const otaFile = h('input', { type: 'file', accept: '.bin', id: 'st-ota' });
  const otaBar = h('i');
  const otaMsg = h('span.muted');

  const { sets } = await app.sets();
  f.set.append(h('option', { value: '' }, 'filelist.txt (stock)'), ...sets.map((s) => h('option', { value: s }, s)));
  f.set.value = c.set;

  el.append(banner,
    h('div.card', h('h2', 'Playback'), h('p.cap', 'Applied live. A set, platform or shuffle change rebuilds the index on the card (a few seconds).'),
      h('div.form',
        h('label.stack', 'brightness', h('div.row', f.brightness, brightOut)),
        h('label.stack', 'dwell min / max (s)', h('div.row', f.dwell_min, f.dwell_max)),
        h('label.stack', 'platforms', f.platforms),
        h('label.stack', 'active set', f.set),
        h('label', f.shuffle, 'shuffle')),
      h('div.row', { style: 'margin-top:12px' }, h('button.pri', { id: 'st-save-play', onclick: () => save(['brightness', 'dwell_min', 'dwell_max', 'shuffle', 'platforms', 'set']) }, 'Save playback'))),
    h('div.card', h('h2', 'Network'), h('p.cap', 'WiFi and hostname changes need a reboot. With no saved network the player raises the open OpenDMD-Setup access point.'),
      h('div.form',
        h('label.stack', 'WiFi network (SSID)', f.wifi_ssid),
        h('label.stack', 'WiFi password', f.wifi_pass),
        h('label.stack', 'hostname (.local)', f.hostname)),
      h('div.row', { style: 'margin-top:10px' }, h('button', { id: 'st-scan', onclick: scan }, 'Scan for networks'), nets),
      h('div.row', { style: 'margin-top:12px' }, h('button.pri', { id: 'st-save-net', onclick: () => save(['wifi_ssid', 'wifi_pass', 'hostname']) }, 'Save network'))),
    h('div.card', h('h2', 'Device'), facts,
      h('div.row', { style: 'margin-top:12px' },
        h('button', { id: 'st-reindex', onclick: reindex }, 'Rebuild index'),
        h('button.danger', { id: 'st-reboot-btn', onclick: reboot }, 'Reboot'))),
    h('div.card', h('h2', 'Firmware update'),
      h('p.cap', 'Upload an app image (target/…/opendmd-firmware.bin from a release build). The device checks the flash variant and reboots into it.'),
      h('div.row', otaFile, h('button', { id: 'st-ota-go', onclick: ota }, 'Upload & reboot'), otaMsg),
      h('div.bar', { style: 'margin-top:10px' }, otaBar)));

  function renderFacts(s) {
    if (!s) return;
    const fact = (k, v) => h('div.stat', h('b', { style: 'font-size:14px' }, v), h('span', k));
    clear(facts, fact('firmware', s.version), fact('flash variant', s.variant), fact('panel', `128×32 · ${s.refresh_hz} Hz`),
      fact('WiFi', `${s.wifi} · ${s.ip}`), fact('hostname', s.hostname + '.local'), fact('uptime', uptime(s.uptime_secs)),
      fact('manifest', s.manifest.replace(/^\/sdcard/, '')), fact('entries', s.entries.toLocaleString()),
      fact('card', s.sd.mounted ? bytes(s.sd.capacity_bytes) : 'not mounted'));
  }

  async function save(keys) {
    const cur = app.config;
    const patch = {};
    for (const k of keys) {
      const el_ = f[k];
      const v = el_.type === 'checkbox' ? el_.checked : el_.type === 'number' || el_.type === 'range' ? +el_.value : el_.value.trim();
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
      try { const s = await app.refreshStatus(); msg(`back up: ${s.version} ${s.variant}`); renderFacts(s); return; } catch { msg(`rebooting… ${2 * (i + 1)} s`); }
    }
    msg('the device has not come back after 90 s — check its serial log');
  }

  async function reboot() {
    if (!confirm('Reboot the player?')) return;
    try { await app.src.reboot(); toast('rebooting…'); banner.hidden = true; waitForBoot((m) => toast(m)); } catch (e) { toast('reboot: ' + e.message, 'bad'); }
  }

  async function ota() {
    const file = otaFile.files[0];
    if (!file) { toast('pick a .bin first'); return; }
    if (!confirm(`Flash ${file.name} (${bytes(file.size)}) and reboot?`)) return;
    try {
      await app.src.ota(file, (p) => { otaBar.style.width = `${(p * 100).toFixed(1)}%`; otaMsg.textContent = `uploading ${(p * 100).toFixed(0)}%`; });
      otaMsg.textContent = 'written, rebooting…';
      waitForBoot((m) => { otaMsg.textContent = m; });
    } catch (e) { otaMsg.textContent = ''; toast('OTA: ' + e.message, 'bad'); }
  }

  renderFacts(app.status);
  return { show() { app.refreshStatus().then(renderFacts, () => {}); } };
}
