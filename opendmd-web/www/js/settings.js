// Settings: the device's NVS config (GET/PUT /api/config — a PUT is a
// partial patch, applied live except WiFi, which needs a reboot), in five
// tabs, `#settings/play|look|time|replay|device` (bare `#settings` is the first);
// only the open tab is in the page. Playback (playlist, platforms, shuffle,
// how long each GIF stays), Panel at `look` (brightness and its maximum
// js/power.js, colour, glow, a formula over every GIF, mirroring on Now, the
// transition, Start-up js/boot.js), Schedule (time, location js/location.js, schedule, night mode), RePlayOS
// (Follow RePlayOS, js/follow.js) and
// Device (status, the playback cache, network — its own Save; a new name
// is live at once —, panel timing js/panel.js, firmware and security
// js/firmware.js, About). Everything saves as it changes.

import { app, playlistLabel, setPath, NO_FORMULAS } from './state.js';
import { timeCard, scheduleCard } from './schedule.js';
import { followCard } from './follow.js';
import { PanelPreview } from './preview.js';
import { popup, closeMenu } from './overrides.js';
import { keepSelect, h, clear, on, note, store, debounce, saver, brightSlider } from './util.js';

const thumbs = () => h('label', { style: 'margin-top:12px' }, h('input', { type: 'checkbox', id: 'st-thumbs', checked: store.get('thumbs', true), onchange: (e) => store.set('thumbs', e.target.checked) }), 'Thumbnails in the Library and playlists');
const TABS = [['play', 'Playback'], ['look', 'Panel'], ['time', 'Schedule'], ['replay', 'RePlayOS'], ['device', 'Device']];
// The bundled Formulas (formulas.json: their sources), fetched once a session.
let exQ = null;
const examples = () => (exQ ??= fetch('formulas.json').then((r) => r.json()).catch(() => { exQ = null; return []; }));

export async function mount(el) {
  // (The demo's player answers too: js/demo.js.)
  if (!app.src.config) {
    el.append(h('h1.ptitle', 'Settings'), h('div.card', h('p.muted', 'Device settings, WiFi and reboot live on the display. This page is in local mode: open it from the display (http://opendmd.local/) to use them'), thumbs()));
    return {};
  }
  const c = app.config || await app.refreshConfig();
  const bright = brightSlider('st-brightness', c.brightness);
  bright.ceil(c.max_brightness);
  const f = {
    brightness: bright.input,
    dwell_min: h('input', { type: 'number', min: 1, id: 'st-dwell-min', value: c.dwell_min, style: 'width:80px', 'aria-label': 'shortest, seconds' }),
    dwell_max: h('input', { type: 'number', min: 1, id: 'st-dwell-max', value: c.dwell_max, style: 'width:80px', 'aria-label': 'longest, seconds' }),
    shuffle: h('input', { type: 'checkbox', id: 'st-shuffle', checked: c.shuffle }),
    platforms: h('input', { type: 'hidden', value: c.platforms }),
    set: h('select', { id: 'st-set' }),
    transition: h('select', { id: 'st-trans' }, [['dissolve', 'Dissolve'], ['wipe_left', 'Wipe'], ['blinds', 'Blinds'], ['push', 'Push'], ['fade', 'Fade'], ['none', 'None']].map(([v, l]) => h('option', { value: v }, l))),
    transition_ms: h('select', { id: 'st-trans-ms', 'data-num': 1 }, [[250, 'Quick'], [400, 'Normal'], [700, 'Slow']].map(([v, l]) => h('option', { value: v }, l))),
    look: h('select', { id: 'st-look' }, [['colour', 'Full colour'], ['amber', 'Amber'], ['red', 'Red'], ['green', 'Green'], ['blue', 'Blue'], ['white', 'White']].map(([v, l]) => h('option', { value: v }, l))),
    glow: h('select', { id: 'st-glow' }, [['off', 'Off'], ['short', 'Short'], ['long', 'Long']].map(([v, l]) => h('option', { value: v }, l))),
    formula_keep: keepSelect('st-keep', c.formula_keep),
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
  const playH = h('h2', 'Playback'), lookH = h('h2', 'Panel'), transH = h('h2', 'Transition');
  const play = saver(playH), looks = saver(lookH), trans = saver(transH);
  // The reboot banner (WiFi changes), filled by Device's network card.
  const banner = h('div.banner', { hidden: true, id: 'st-reboot' });
  // Saved as changed: the slider when it settles, typed fields when left.
  let held = 0; // a slider change not saved yet: another card's config must not undo it
  const later = debounce(() => save(['brightness'], looks).then(() => { held = 0; }), 400);
  f.brightness.addEventListener('input', () => { held = 1; later(); });
  for (const k of ['shuffle', 'set']) f[k].onchange = () => save([k], play);
  for (const k of ['look', 'glow']) f[k].onchange = () => save([k], looks);
  // A Formula over every GIF (js/looks.js, imported with the tab): one of
  // this browser's or a bundled one that declares `look` (its own line says
  // how it goes over the GIFs: the Formula's settings, in Formulas); one
  // that can't be drawn over GIFs is listed, not offered. The one set
  // elsewhere (another browser, an edit since) stays listed, read from its
  // path, with the panel's word if it can't draw it any more. Change… opens
  // its settings. Applied as it changes.
  const lf = h('select', { id: 'st-look-formula' });
  const lfErr = h('p.cap.bad', { id: 'st-look-formula-err', hidden: true });
  const lfWhy = h('p.cap', { id: 'st-look-why', hidden: true });
  const lfEdit = h('button.sm', { id: 'st-look-change', hidden: true, onclick: () => { app.psiEdit = { f: { ...lfs[lf.value] }, settings: 1 }; app.go('formulas/edit'); } }, 'Change…');
  let lfs = [], L = null, M = null;
  async function lookFormulas() {
    if (app.noFormulas) return; // a build without Formulas: no Formula looks (the tab says so)
    let ex;
    let mine;
    try { [ex, L, mine] = await Promise.all([examples(), import('./looks.js'), import('./fstore.js').then((S) => S.loaded().then(() => S.list()))]); M = await L.psi(); } catch (e) { lfErr.hidden = false; lfErr.textContent = 'Formulas could not be loaded: ' + (e.message || e); return; }
    const cf = app.config || {}, grp = (label, list) => list.length && h('optgroup', { label }, list.map(opt));
    const opt = (x) => h('option', { value: x.k, disabled: !!x.off }, x.name + (!x.off ? '' : /too heavy/.test(x.off) ? ' (too heavy)' : /too long/.test(x.off) ? ' (too long)' : ' (can’t be a look)'));
    lfs = [...(Array.isArray(mine) ? mine : []).map((x) => ({ ...x, mine: 1 })), ...(Array.isArray(ex) ? ex : []).map((x) => ({ ...x }))]
      .filter((x) => typeof x?.src == 'string' && L.declares(M, x)).map((x) => ({ ...x, name: L.named(M, x) }));
    let cur = -1;
    if (cf.look_formula) {
      cur = lfs.findIndex((x) => L.path(M, x) === cf.look_formula);
      if (cur < 0) { lfs.unshift(L.fromPath(M, cf.look_formula)); cur = 0; }
    }
    lfs.forEach((x, k) => { x.k = k; x.off = k !== cur && L.as(M, x).why; });
    clear(lf, h('option', { value: '' }, 'None'), lfs.filter((x) => x.saved).map(opt),
      grp('Your Formulas', lfs.filter((x) => x.mine)), grp('Bundled Formulas', lfs.filter((x) => !x.mine && !x.saved)));
    lf.value = cur < 0 ? '' : String(cur);
    say();
  }
  // How the chosen one goes on (its look line), or why the panel can't.
  function say() {
    const x = lfs[lf.value], a = x && L.as(M, x), l = x && L.decl(M, x.src);
    lfEdit.hidden = !x?.src;
    const t = !x ? '' : x.saved && !x.src ? 'Set elsewhere: open it where it was made to change it' : a.why || [`${L.BLENDS.find(([v]) => v == l?.look?.[0])?.[1] || 'Over'} at ${l?.look?.[1] ?? 100}%`, a.i[5] & 8 && ((a.i[13] ?? a.i[6]) > 8000 ? 'moves about 25 times a second (a heavy look)' : '')].filter(Boolean).join(' · ');
    lfWhy.hidden = !t; lfWhy.textContent = t; lfWhy.classList.toggle('bad', !!a?.why && !!x.src);
  }
  const saveLook = () => looks(async () => {
    const x = lfs[lf.value];
    try {
      if (!x) { await app.src.putConfig({ look_formula: '' }); await app.refreshConfig(); return; }
      await L.use(x);
    } catch (e) {
      // Refused (400/413): the picker goes back to what the panel has.
      await app.refreshConfig().catch(() => {});
      await lookFormulas();
      throw e;
    }
  });
  lf.onchange = () => { say(); if (!lfs[lf.value] || !L.as(M, lfs[lf.value]).why) saveLook(); };
  on('config', lookFormulas);
  const lookErr = (st) => { const e = st?.formula_look?.error; lfErr.hidden = !e; lfErr.textContent = e ? `The panel can’t draw this formula any more: ${e}` : ''; };
  on('status', lookErr);
  lookErr(app.status);

  // Platforms: one row, "All (60)" or the ones picked, and a sheet to
  // choose them (counted per manifest text; the rare ones folded away).
  const platVal = h('span.grow', { id: 'st-plat-val' });
  let counted = [];
  async function platCounts() {
    const text = await app.text(setPath(app.config?.set)).catch(() => '');
    if (counted[0] !== text) {
      const n = new Map();
      for (const e of text ? app.wasm.parseManifest(text) : []) if (e.platform) { const p = e.platform.toUpperCase(); n.set(p, -~n.get(p)); }
      counted = [text, [...n].sort((a, b) => b[1] - a[1])];
    }
    return counted[1];
  }
  const chosen = () => f.platforms.value.toUpperCase().split(',').map((p) => p.trim()).filter(Boolean);
  async function platRow() {
    const n = (await platCounts()).length, s = chosen();
    platVal.textContent = s.length ? s.join(', ') : `All (${n})`;
  }
  async function platSheet(btn) {
    const all = [...await platCounts()], sel = new Set(chosen());
    // A saved name the playlist lacks shows with 0, so it can be cleared.
    for (const p of sel) all.some(([q]) => q == p) || all.push([p, 0]);
    let more = false;
    const q = h('input', { type: 'search', id: 'st-plat-q', placeholder: 'Find a platform', oninput: draw });
    const box = h('div.chips', { id: 'st-plats' });
    const only = h('button.pri', { id: 'st-plat-only', onclick: () => platSet([...sel]) });
    function draw() {
      const t = q.value.trim().toUpperCase(), hit = all.filter(([p]) => p.includes(t));
      const show = hit.filter(([p, k]) => more || t || k >= 5 || sel.has(p)), rest = hit.length - show.length;
      clear(box, show.map(([p, k]) => h('button.chip' + (sel.has(p) ? '.on' : ''), { 'data-platform': p, 'aria-pressed': String(sel.has(p)), onclick: () => { sel.delete(p) || sel.add(p); draw(); } }, p, h('small', k.toLocaleString()))),
        rest > 0 && h('button.chip.txt', { id: 'st-plat-more', onclick: () => { more = true; draw(); } }, `+${rest} more`));
      only.textContent = sel.size ? `Only these (${sel.size})` : 'Only these';
      only.disabled = !sel.size;
    }
    draw();
    const r = btn.getBoundingClientRect();
    popup(h('div.menu.wide', { id: 'st-plat-sheet', role: 'dialog', 'aria-label': 'Platforms' }, h('div.mh', h('b', 'Platforms to play')), q, box,
      h('div.row.sp', h('button', { id: 'st-plat-all', onclick: () => platSet([]) }, `All platforms (${all.length})`), only)), r.right - 440, r.bottom + 4);
  }
  function platSet(list) {
    closeMenu();
    f.platforms.value = list.join(',');
    platRow();
    save(['platforms'], play);
  }
  on('config', (cf) => { f.platforms.value = cf.platforms; platRow(); });
  platRow();
  f.dwell_min.onchange = f.dwell_max.onchange = () => save(['dwell_min', 'dwell_max'], play);
  f.formula_keep.onchange = () => save(['formula_keep'], play);
  on('config', (cf) => { f.formula_keep.value = cf.formula_keep || 'turn'; });
  f.transition.onchange = f.transition_ms.onchange = () => save(['transition', 'transition_ms'], trans);
  on('config', (cf) => { bright.ceil(cf.max_brightness); if (!held) bright.set(cf.brightness); });
  // Now's preview of the panel: this browser's choice (it costs the player a download per GIF).
  const mirror = h('input', { type: 'checkbox', id: 'st-mirror', checked: store.get('follow', true), onchange: () => store.set('follow', mirror.checked) });

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
  // Start-up and Maximum brightness (js/boot.js and js/power.js, under the
  // Brightness slider; loaded when Panel is first open): modules of their own.
  const boot = h('div.card', { id: 'st-boot-card' }), power = h('div', { id: 'st-power-card', style: 'margin-top:12px' });
  const time = timeCard();
  const sched = scheduleCard();
  on('status', time.render);

  const tab = (k, ...kids) => h('div.stab', { id: 'st-tab-' + k, role: 'tabpanel', 'aria-labelledby': 'st-t-' + k }, kids);
  const BUILD = {
    play: () => tab('play',
      h('div.card', playH, note('A playlist, platform or shuffle change takes a few seconds', 'The display reads the playlist again; the panel holds its frame meanwhile.'),
        h('div.form.f2',
          h('label.stack', 'Playlist', f.set),
          h('div.stack', 'Platforms', h('div.row', platVal, h('button', { id: 'st-plat-btn', 'aria-haspopup': 'dialog', onclick: (e) => platSheet(e.currentTarget) }, 'Choose…'))),
          h('label.stack', 'Each GIF stays', h('div.row', f.dwell_min, '–', f.dwell_max, 's')),
          !app.noFormulas && h('label.stack', 'A Formula you show on the panel stays', f.formula_keep),
          h('label', f.shuffle, 'Shuffle')))),
    look: () => {
      import('./power.js').then((m) => m.powerCard(power, lookH));
      import('./boot.js').then((m) => m.bootCard(boot, examples));
      return tab('look',
        h('div.card', lookH, h('p.cap', 'How bright the panel is, and how it draws every GIF'),
          h('div.form.f2', h('div', h('label.stack', 'Brightness', bright.el), power),
            h('div', h('label.stack', 'Colour', f.look), h('label.stack', { style: 'margin-top:12px' }, 'Phosphor glow', f.glow))),
          ...(app.noFormulas ? [h('p.cap', { id: 'st-look-none', style: 'margin-top:12px' }, `No Formula over the GIFs: ${NO_FORMULAS.replace(/^F/, 'f')}`)]
            : [(lookFormulas(), h('label.stack', { style: 'margin-top:12px' }, 'Formula over every GIF', h('div.row', lf, lfEdit))), lfWhy, lfErr,
              h('p.cap', 'A Formula drawn over everything the panel plays, before the colour and glow: scanlines, an old TV, a colour grade. Each says how it goes over the GIFs; in Formulas, any Formula’s settings can make it one.')]),
          h('label', { style: 'margin-top:12px' }, mirror, 'Mirror the panel on Now'), h('p.cap', 'Now shows what the panel is playing; off saves the display a download for each GIF (this browser only)')),
        h('div.card', { id: 'st-trans-card' }, transH, h('p.cap', 'How the panel moves from one GIF to the next'),
          h('div.form.f2', h('label.stack', 'Style', f.transition), h('label.stack', 'Speed', f.transition_ms)),
          h('div.row', { style: 'margin-top:12px' }, h('button', { id: 'st-trans-preview', onclick: previewTrans }, 'Preview')),
          pvBox),
        boot);
    },
    // Location (js/location.js): on a player that keeps one (`lat` in its config).
    time: () => { const loc = 'lat' in c && h('div.grp', { id: 'loc' }); if (loc) import('./location.js').then((m) => m.locationCard(loc)); return tab('time', time.el, loc, sched.el); },
    replay: () => tab('replay', followCard().el),
    device: () => {
      const t = tab('device');
      import('./device.js').then((m) => { dev = m.deviceTab(t, { f, save, banner, sched, thumbs, live: () => visible && open === 'device' }); });
      return t;
    },
  };
  const built = {};
  const panel = h('div');
  const tabs = h('div.tabs', { id: 'st-tabs', role: 'tablist' }, TABS.map(([k, t]) => h('button.tab', { role: 'tab', id: 'st-t-' + k, 'aria-controls': 'st-tab-' + k, 'data-tab': k, onclick: () => location.replace('#settings/' + k) }, t)));
  el.append(h('h1.ptitle', 'Settings'), banner, tabs, panel);
  let open = '', visible = true, dev;
  function go(k) {
    if (!BUILD[k]) k = 'play';
    for (const b of tabs.children) { b.classList.toggle('on', b.dataset.tab === k); b.ariaSelected = b.dataset.tab === k; }
    if (k === open) return;
    if (open === 'device') dev?.hide();
    open = k;
    panel.replaceChildren(built[k] ??= BUILD[k]());
    if (k === 'device') dev?.show();
    tick();
  }
  // Device's status, every 5 s while it is open (the app's scheduler).
  function tick() {
    if (visible && open === 'device') app.watch('device', 5000); else app.unwatch('device');
  }

  // A playlist change while the player is still reading one would queue
  // behind it: the status every 2 s until it is done.
  on('status', (s) => {
    f.set.disabled = s.index_state === 'building';
    f.set.title = f.set.disabled ? 'the display is still reading the playlist' : '';
    if (f.set.disabled) app.watch('reindex', 2000); else app.unwatch('reindex');
  });

  // The fields `keys`, read now (a config event may redraw them before the
  // save runs), PUT through `run` where they differ from the config then.
  function save(keys, run) {
    const vals = {};
    for (const k of keys) {
      const el_ = f[k];
      vals[k] = el_.type === 'checkbox' ? el_.checked : el_.type === 'number' || el_.type === 'range' || el_.dataset.num ? +el_.value : el_.value.trim();
    }
    return run(async () => {
      const cur = app.config;
      const patch = {};
      for (const k of keys) {
        if (k === 'wifi_pass') { if (f[k].value) patch[k] = f[k].value; } else if (vals[k] !== cur[k]) patch[k] = vals[k];
      }
      if (!Object.keys(patch).length) return false;
      if ((patch.dwell_max ?? cur.dwell_max) < (patch.dwell_min ?? cur.dwell_min)) throw new Error('the longest must be at least the shortest');
      const r = await app.src.putConfig(patch);
      await app.refreshConfig();
      f.after?.(r, patch, keys);
    });
  }

  time.render(app.status);
  return {
    show() { visible = true; app.refreshStatus().catch(() => {}); loadPlaylists(); if (open === 'device') dev?.show(); sched.load(); tick(); },
    sub: go,
    hide() { visible = false; tick(); if (open === 'device') dev?.hide(); },
  };
}
