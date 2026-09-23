// Sets: the playlists — the card's own (filelist.txt is "All files", the
// default; filelist.hall "Halloween", filelist.xmas "Christmas") and the
// ones in /sets/*.txt. Open one to edit it, build a new one from filters (the
// core's Filter, run here over the whole manifest) or from the Library
// selection, reorder, save with PUT /api/sets/<name> and activate with
// PUT /api/config {"set": value} — "" for All files, a root manifest's file
// name for the card's others, a set's name for the rest. Entries are GIF
// paths; the device plays a file's prepared copy by itself.
// The `.idx` sidecar rule is the device's business: a manifest PUT deletes
// the stale index first and reindexes if the set is the active one.

import { app, playlistLabel } from './state.js';
import { Overrides, glyphs, menu, moreButton } from './overrides.js';
import { VList } from './vlist.js';
import { h, clear, on, toast, label, lines } from './util.js';

// Not `filelist.*`: the firmware reads such a `set` as a root manifest.
const NAME_RE = /^(?!filelist\.)[A-Za-z0-9][A-Za-z0-9 _.-]{0,40}$/;

export async function mount(el) {
  const setsList = h('div.list', { id: 'sets-list' });
  // --- builder
  const baseSel = h('select', { id: 'b-base' });
  const bPlat = h('input', { type: 'text', id: 'b-platforms', placeholder: 'MD,SNES' });
  const bCats = h('div.chips', { id: 'b-cats' });
  const bTitle = h('input', { type: 'text', id: 'b-title', placeholder: 'mario, sonic' });
  const bFlags = h('input', { type: 'text', id: 'b-flags', placeholder: 'INVERT,LOGO', value: '' });
  const bShuffle = h('input', { type: 'checkbox', id: 'b-shuffle' });
  const bSeed = h('input', { type: 'number', id: 'b-seed', value: 1, min: 0, style: 'width:90px' });
  const bLimit = h('input', { type: 'number', id: 'b-limit', placeholder: 'all', min: 1, style: 'width:90px' });
  const bCount = h('span.muted', { id: 'b-count' });
  // --- editor
  const name = h('input', { type: 'text', id: 'set-name', placeholder: 'name, e.g. consoles', value: app.draft.name });
  const eCount = h('span.muted', { id: 'set-count' });
  const eSearch = h('input', { type: 'search', id: 'set-search', placeholder: 'filter entries' });
  const activeNote = h('span', { id: 'set-active' });
  let view = [];
  const picked = new Set();

  // Favourite / hide / play longer, for the playlist open in the editor
  // (a saved one: a draft has nowhere to keep them yet).
  const ov = new Overrides();
  ov.onChange = () => list.refresh();
  const list = new VList({
    rowHeight: 40,
    render(row, i) {
      const p = app.draft.paths[i];
      const e = app.wasm.parsePath(p);
      const o = ov.get(p);
      clear(row, h('span.muted.mono', { style: 'width:44px;text-align:right' }, i + 1),
        h('span.t', label(e), e.platform && h('span.tag', e.platform)),
        glyphs(o),
        h('span.n', e.category || ''),
        moreButton(ov, p),
        h('button.sm', { title: 'move up', onclick: (ev) => { ev.stopPropagation(); move(i, -1); } }, '↑'),
        h('button.sm', { title: 'move down', onclick: (ev) => { ev.stopPropagation(); move(i, 1); } }, '↓'),
        h('button.sm.danger', { title: 'remove', onclick: (ev) => { ev.stopPropagation(); remove(i); } }, '✕'));
      row.classList.toggle('cur', picked.has(i));
      row.classList.toggle('hid', !!o?.never);
      row.dataset.path = p;
    },
    onClick: (i) => { if (!picked.delete(i)) picked.add(i); list.refresh(); },
    onMenu: (i, _k, x, y) => menu(ov, app.draft.paths[i], x, y),
  });
  list.el.id = 'set-entries';

  el.append(h('div.cols',
    h('div',
      h('div.card', h('div.row.sp', h('h2', 'Playlists'), activeNote), setsList,
        h('p.cap', 'Activating rebuilds the index on the device (a few seconds; the panel holds its frame). “All files” plays when no other playlist is chosen.')),
      h('div.card', h('h2', 'Build from filters'),
        h('div.form',
          h('label.stack', 'from playlist', baseSel),
          h('label.stack', 'platforms (any of)', bPlat),
          h('label.stack', 'title contains (any of)', bTitle),
          h('label.stack', 'drop flags', bFlags)),
        h('h3', 'categories (any of; none = all)'), bCats,
        h('div.row', { style: 'margin-top:10px' }, h('label', bShuffle, 'shuffle'), h('label', 'seed', bSeed), h('label', 'limit', bLimit)),
        h('div.row.sp', { style: 'margin-top:10px' }, bCount,
          h('div.row', h('button', { id: 'b-append', onclick: () => build(true) }, 'Append to editor'),
            h('button.pri', { id: 'b-use', onclick: () => build(false) }, 'Replace editor'))))),
    h('div.card',
      h('div.row', h('label.stack.grow', 'playlist name', name), eCount),
      h('div.row', { style: 'margin:8px 0' }, h('div.grow', eSearch),
        h('button.sm', { onclick: shuffle }, 'Shuffle'), h('button.sm', { onclick: sortTitle }, 'Sort A–Z'),
        h('button.sm', { onclick: () => movePicked(-1e9) }, 'Picked to top'),
        h('button.sm.danger', { onclick: removePicked }, 'Remove picked'),
        h('button.sm.danger', { onclick: () => { app.draft.paths = []; picked.clear(); refresh(); } }, 'Clear')),
      h('p.cap', 'Right-click (or long-press, or …) an entry to make it a favourite, hide it, or play it longer in this playlist.'),
      list.el,
      h('div.row', { style: 'margin-top:10px' },
        h('button.pri', { id: 'set-save', onclick: () => save(false) }, 'Save'),
        app.device && h('button', { id: 'set-save-activate', onclick: () => save(true) }, 'Save & activate')))));

  // --- sets list ----------------------------------------------------------
  let known = [];
  async function loadSets() {
    const lists = await app.playlists();
    known = lists.map((p) => p.path);
    const active = app.activeSet;
    const activePl = lists.find((p) => p.value === active);
    activeNote.textContent = app.device ? `playing: ${activePl ? activePl.label : playlistLabel(active || 'filelist.txt')}` : '';
    clear(setsList, lists.map((p) => {
      const isActive = app.device && p.value === active;
      return h('div', { 'data-path': p.path, 'data-value': p.value },
        h('span.grow', p.label, !p.value && h('span.tag', 'default'), isActive && h('span.tag.act', 'active')),
        h('button.sm', { onclick: () => open(p) }, 'Open'),
        app.device && !isActive && h('button.sm', { 'data-act': 1, disabled: app.building, onclick: () => activate(p) }, 'Activate'),
        !p.root && h('button.sm.danger', { onclick: () => del(p) }, 'Delete'));
    }));
    const keep = baseSel.value;
    clear(baseSel, lists.map((p) => h('option', { value: p.path }, p.label)));
    baseSel.value = keep && known.includes(keep) ? keep : '/filelist.txt';
  }

  async function open(p) {
    try {
      const text = await app.text(p.path, true);
      if (text == null) throw new Error('not served by this source');
      app.draft.paths = lines(text);
      app.draft.name = p.root ? '' : p.value;
      app.draft.path = p.path;
      ov.load(p.path);
      name.value = app.draft.name;
      picked.clear();
      refresh();
      toast(`${p.label}: ${app.draft.paths.length} entries`);
    } catch (e) { toast(`${p.label}: ${e.message}`, 'bad'); }
  }

  async function activate(p) {
    if (app.building) { toast('the device is building an index; try again when it is ready', 'bad'); return; }
    const set = p.value, n = p.label;
    try {
      toast(`activating ${n} — the device is rebuilding its index…`);
      const r = await app.src.putConfig({ set });
      await app.refreshStatus();
      await app.refreshConfig(); // -> loadSets
      toast(`active: ${n} · ${r.entries} entries · index ${r.index ?? 'unchanged'} ${r.index_ms ? `(${r.index_ms} ms)` : ''}`, 'ok');
    } catch (e) { toast('activate: ' + e.message, 'bad'); }
  }

  async function del(p) {
    const path = p.path, set = p.value;
    const isActive = app.device && app.activeSet === set;
    if (!confirm(isActive ? `"${set}" is playing. Switch the player back to All files and delete it?` : `Delete the playlist "${set}"?`)) return;
    try {
      if (isActive) { await app.src.putConfig({ set: '' }); await app.refreshStatus(); await app.refreshConfig(); }
      await app.src.deleteFile(path);
      app.texts.delete(path);
      toast(`deleted "${set}"`, 'ok');
      loadSets();
    } catch (e) { toast('delete: ' + e.message, 'bad'); }
  }

  // --- builder ------------------------------------------------------------
  async function categories() {
    let dirs = [];
    try { dirs = (await app.src.list('/gifs')).filter((d) => d.dir).map((d) => d.name).sort(); } catch { /* none */ }
    clear(bCats, dirs.map((d) => h('button.chip', { 'data-cat': d, onclick: (e) => { e.target.closest('.chip').classList.toggle('on'); count(); } }, d)));
  }
  const spec = () => ({
    platforms: bPlat.value, titles: bTitle.value, exclude_flags: bFlags.value,
    categories: [...bCats.querySelectorAll('.chip.on')].map((c) => c.dataset.cat),
    shuffle: bShuffle.checked, seed: +bSeed.value || 0, ...(+bLimit.value > 0 ? { limit: +bLimit.value } : {}),
  });
  async function run() {
    const text = await app.text(baseSel.value);
    if (text == null) throw new Error(`${baseSel.value} is not served by this source`);
    return app.wasm.filterManifest(text, JSON.stringify(spec()));
  }
  async function count() {
    try { bCount.textContent = `${(await run()).length.toLocaleString()} match`; } catch (e) { bCount.textContent = e.message; }
  }
  async function build(append) {
    try {
      const paths = await run();
      if (append) app.draft.paths.push(...paths);
      else { app.draft.paths = paths; app.draft.path = ''; ov.load(''); }
      picked.clear();
      refresh();
      toast(`${paths.length} entries ${append ? 'appended' : 'in the editor'}`, 'ok');
    } catch (e) { toast(e.message, 'bad'); }
  }
  for (const i of [baseSel, bPlat, bTitle, bFlags, bShuffle, bSeed, bLimit]) i.addEventListener('change', count);

  // --- editor -------------------------------------------------------------
  function refresh() {
    const q = app.wasm.normalizeTitle(eSearch.value);
    const all = app.draft.paths.map((_, i) => i);
    view = q ? all.filter((i) => app.wasm.normalizeTitle(app.wasm.parsePath(app.draft.paths[i]).title).includes(q)) : all;
    eCount.textContent = `${app.draft.paths.length.toLocaleString()} entries${q ? ` · ${view.length} shown` : ''}`;
    list.set(view);
  }
  function move(i, d) {
    const j = i + d;
    const p = app.draft.paths;
    if (j < 0 || j >= p.length) return;
    [p[i], p[j]] = [p[j], p[i]];
    picked.clear();
    refresh();
  }
  function movePicked() {
    const p = app.draft.paths;
    const top = [...picked].sort((a, b) => a - b).map((i) => p[i]);
    app.draft.paths = [...top, ...p.filter((_, i) => !picked.has(i))];
    picked.clear();
    refresh();
  }
  function remove(i) { app.draft.paths.splice(i, 1); picked.clear(); refresh(); }
  function removePicked() { app.draft.paths = app.draft.paths.filter((_, i) => !picked.has(i)); picked.clear(); refresh(); }
  function shuffle() {
    app.draft.paths = app.wasm.filterManifest(app.draft.paths.join('\n'), JSON.stringify({ shuffle: true, seed: Date.now() % 1e9 }));
    refresh();
  }
  function sortTitle() {
    const key = new Map(app.draft.paths.map((p) => [p, app.wasm.normalizeTitle(label(app.wasm.parsePath(p)))]));
    app.draft.paths.sort((a, b) => key.get(a).localeCompare(key.get(b)));
    refresh();
  }

  async function save(andActivate) {
    const n = name.value.trim().replace(/\.txt$/, '');
    if (!NAME_RE.test(n)) { toast('playlist names are letters, digits, space, _ . - (no slashes, not filelist.*)', 'bad'); return; }
    if (!app.draft.paths.length) { toast('the playlist is empty', 'bad'); return; }
    const path = `/sets/${n}.txt`;
    if (known.includes(path) && app.draft.name !== n && !confirm(`A playlist named "${n}" exists. Replace it?`)) return;
    try {
      const text = app.draft.paths.join('\n') + '\n';
      const r = await app.src.saveSet(n, text);
      app.draft.name = n;
      app.texts.set(path, text);
      if (app.draft.path !== path) { app.draft.path = path; ov.load(path); }
      toast(`saved "${n}" · ${app.draft.paths.length} entries${r.index ? ` · reindexed (${r.index_ms} ms)` : ''}`, 'ok');
      if (andActivate && app.activeSet !== n) await activate({ value: n, label: n });
      await loadSets();
    } catch (e) { toast('save: ' + e.message, 'bad'); }
  }

  name.oninput = () => { app.draft.name = name.value.trim(); };
  eSearch.oninput = refresh;
  on('draft', () => { name.value = app.draft.name; refresh(); });
  if (app.draft.path) ov.load(app.draft.path);
  on('config', () => loadSets());
  on('status', () => { for (const b of setsList.querySelectorAll('[data-act]')) b.disabled = app.building; });

  await Promise.all([loadSets(), categories()]);
  refresh();
  count();
  return { show() { name.value = app.draft.name; refresh(); if (app.device) app.refreshStatus().catch(() => {}); } };
}
