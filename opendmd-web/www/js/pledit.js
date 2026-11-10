// The playlist editor (#sets/edit; on a phone it replaces the list, on a
// laptop it is the right column), imported by sets.js the first time a
// playlist opens: the entries with their thumbnails (drag ⠿, Alt+↑/↓ or
// a row's Move items, ✕, Select and Arrange), **Add GIFs** — the card's
// folders as a tree (js/tree.js): tick GIFs or whole folders, or add
// everything a search shows; beside the entries on a laptop, in their
// place on a phone —, Add by filter (js/plbuild.js: a search, a sheet; a
// smart playlist when the search is kept, js/smart.js), Save PUT
// /api/sets/<name>, play PUT /api/config. Changes not saved are asked
// about on the way out (`guard`); the ones that take many GIFs at once
// have an Undo in their toast.

import { app, playlistLabel, platName, dirty, SET_NAME, SET_NAME_RULE } from './state.js';
import { Overrides, glyphs, menu, moreButton, actions } from './overrides.js';
import { VList } from './vlist.js';
import { Tree, chipRow } from './tree.js';
import { attach, watch, thumbsOn } from './thumbs.js';
import { h, clear, emit, on, store, toast, label, note, isFormula, drawn } from './util.js';
import { inTurn } from './gifs.js';

// ctx: {el (the view), closeEd(), activate(p), loadSets(), known() (playlist paths), lists() (playlists)}
export function editor(se, ctx) {
  const { el } = ctx;
  const name = h('input', { type: 'text', id: 'set-name', placeholder: 'e.g. consoles', value: app.draft.name });
  const nameL = h('span'), nameRow = h('label.stack', { id: 'set-name-row', style: 'margin-bottom:12px' }, nameL, name);
  const title = h('h2.grow', { id: 'set-title' });
  // A smart playlist (js/smart.js): its search, Edit search, Refresh now.
  const smartCap = h('span.grow');
  const smartRow = h('div.row.cap', { id: 'set-smart', hidden: true }, smartCap,
    h('button.sm', { id: 'set-smart-edit', onclick: (e) => openBuilder(e.currentTarget, app.draft.smart) }, 'Edit search'),
    h('button.sm', { id: 'set-smart-refresh', onclick: refreshNow }, 'Refresh now'));
  const saveB = h('button.pri', { id: 'set-save', onclick: () => save(false) }), playB = app.device && h('button', { id: 'set-save-activate', onclick: () => save(true) });
  const empty = h('div', { id: 'set-empty' },
    h('p.cap', 'Nothing in it yet. + Add GIFs picks them from your folders; Add by filter takes every GIF that matches.'),
    h('div.row', h('button.pri', { id: 'set-empty-add', onclick: () => adding(true) }, '+ Add GIFs'),
      h('button', { id: 'set-empty-build', onclick: (e) => openBuilder(e.currentTarget) }, 'Add by filter…')));
  const eCount = h('span.muted', { id: 'set-count' });
  const eSearch = h('input', { type: 'search', id: 'set-search', placeholder: 'Filter GIFs', oninput: () => refresh() });
  let view = [], have = new Set(), selecting = false;
  const picked = new Set();
  // Select: a tick on every row; the chip says how many are picked, and unpicks them.
  const selB = h('button', { id: 'set-select', 'aria-pressed': 'false', onclick: () => select(!selecting) }, 'Select');
  const pickChip = h('button.chip.on', { id: 'set-picked', hidden: true, title: 'Unpick them', onclick: () => { picked.clear(); refresh(); } });

  const ov = new Overrides();
  ov.onChange = () => list.refresh();
  // One of the user's Formulas on the card says its name in its `name`
  // line: read once, the row drawn again.
  const fnames = new Map();
  const fname = (p) => {
    const f = p.split('?')[0];
    if (f[0] != '/' || !/\.psi$/i.test(f)) return null;
    if (!fnames.has(f)) {
      fnames.set(f, null);
      app.src.readText(f).then((t) => { const n = /^\s*name\s+"([^"\n]*)"/m.exec(t || '')?.[1]; if (n) { fnames.set(f, n); list.refresh(); } }, () => {});
    }
    return fnames.get(f);
  };
  const list = new VList({
    rowHeight: 40,
    render(row, i) {
      const p = app.draft.paths[i];
      const e = app.wasm.parsePath(p);
      const skipped = app.noFormulas && isFormula(p);
      const o = ov.get(p), cv = thumbsOn() && !drawn(p) && h('canvas.th', { width: 128, height: 32 });
      clear(row, selecting && h('input', { type: 'checkbox', checked: picked.has(i), tabIndex: -1, 'aria-label': 'Pick' }),
        h('button.grip', { title: 'Drag to move (or Alt+↑/↓)', 'aria-label': 'Move', onpointerdown: (ev) => drag(ev, row, i) }, '⠿'),
        cv, h('span.t', fname(p) || label(e)), glyphs(o), (platName(e.platform) || isFormula(p)) && h('span.tag', platName(e.platform) || (skipped ? 'Formula: skipped on this display' : 'Formula')),
        h('button.sm.rm', { title: 'Remove', 'aria-label': 'Remove', onclick: (ev) => { ev.stopPropagation(); remove(i); } }, '✕'),
        moreButton(ov, p, rowItems(i)));
      if (cv) attach(cv, p);
      row.classList.toggle('cur', picked.has(i));
      row.classList.toggle('skip', skipped);
      row.dataset.path = p;
    },
    onClick: (i, _k, e) => { if (e.target.closest('.grip')) return; if (!picked.delete(i)) picked.add(i); list.refresh(); chip(); },
    onMenu: (i, _k, x, y) => menu(ov, app.draft.paths[i], x, y, rowItems(i)),
  });
  // A row's own items in its menu: where it goes, and out.
  const rowItems = (i) => {
    const k = view.indexOf(i), last = app.draft.paths.length - 1;
    return [['set-up', 'Move up', () => moveTo(i, view[k - 1]), k < 1],
      ['set-down', 'Move down', () => moveTo(i, view[k + 1]), k < 0 || k >= view.length - 1],
      ['set-to-top', 'Move to top', () => moveTo(i, 0), i === 0],
      ['set-to-end', 'Move to end', () => moveTo(i, last), i === last],
      ['set-rm', 'Remove from playlist', () => remove(i)]];
  };
  function chip() {
    pickChip.hidden = !picked.size;
    pickChip.textContent = `${picked.size.toLocaleString()} picked ✕`;
  }
  function select(v) {
    selecting = v;
    selB.classList.toggle('on', v);
    selB.setAttribute('aria-pressed', String(v));
    if (!v) picked.clear();
    refresh();
  }
  list.el.id = 'set-entries';
  watch(list.el);
  list.el.addEventListener('keydown', (e) => {
    const row = e.target.closest('.vrow'), d = { ArrowUp: -1, ArrowDown: 1 }[e.key];
    if (!e.altKey || !d || !row) return;
    e.preventDefault();
    const k = row._i, t = k + d;
    if (t < 0 || t >= view.length) return;
    moveTo(view[k], view[t]);
    list.scrollTo(t);
    [...list.el.querySelectorAll('.vrow')].find((r) => r._i === t && !r.hidden)?.querySelector('.grip').focus();
  });
  const arrange = (b) => actions(b, 'set-arrange-menu', [
    ['set-shuffle', 'Shuffle', shuffle],
    ['set-sort', 'Sort A–Z', sortTitle],
    ['set-top', 'Picked to top', () => movePicked(false), '', !picked.size],
    ['set-end', 'Picked to end', () => movePicked(true), '', !picked.size],
    ['set-rm-picked', 'Remove picked', removePicked, '.danger', !picked.size],
    ['set-clear', 'Clear', () => undoable(() => { app.draft.paths = []; }, 'Cleared'), '.danger', !app.draft.paths.length]]);

  // Add GIFs: the folders, ticks, a search (and its platforms).
  const ticked = new Set(), aPl = new Set();
  const addQ = h('input', { type: 'search', id: 'set-add-q', placeholder: 'Search GIFs', enterKeyHint: 'search', oninput: () => tree.filter(addQ.value, aPl) });
  const aChips = h('div.chips.scroll', { id: 'set-add-chips' });
  const addTicked = h('button.pri', { id: 'set-add-ticked', onclick: () => add([...ticked]) });
  const addAll = h('button', { id: 'set-add-all', onclick: () => add(tree.res.map((x) => x.path)) });
  const tree = new Tree({
    picked: ticked,
    ticking: () => true,
    rowHeight: 40,
    open: store.get('pl.open', []),
    onOpen: (o) => store.set('pl.open', o),
    mark: (it) => have.has(it.path),
    extra: (it) => platName(it.e.platform) && h('span.tag', platName(it.e.platform)),
    onTick: () => foot(),
    onError: (m) => toast(m, 'bad'),
    onUpdate: (st) => { chipRow(aChips, st.plats, aPl, 'data-platform', () => tree.filter(addQ.value, aPl)); foot(st); },
  });
  tree.el.id = 'set-add-list';
  let shown = 0;
  function foot(st) {
    if (st) shown = st.busy ? 0 : st.shown;
    addTicked.textContent = `Add ${ticked.size.toLocaleString()} ticked`;
    addTicked.disabled = !ticked.size;
    addAll.textContent = `Add all ${shown.toLocaleString()} shown`;
    addAll.disabled = !shown;
  }
  const pane = h('div', { id: 'set-add-pane' },
    h('div.row', { style: 'margin-bottom:8px' }, h('h3.pane-h.grow', 'Add GIFs'), h('button.sm', { id: 'set-add-done', onclick: () => adding(false) }, 'Done')),
    h('div', { style: 'margin-bottom:8px' }, addQ), aChips, tree.el,
    h('div.row', { id: 'set-add-foot' }, addTicked, addAll));
  let treeQ = null;
  function adding(v) {
    el.classList.toggle('adding', v);
    if (v && (!treeQ || tree.dirty)) treeQ = tree.load();
    if (v) tree.refresh();
  }
  function add(paths) {
    const n = paths.length, fresh = paths.filter((p) => !have.has(p) && have.add(p));
    app.draft.paths.push(...fresh);
    ticked.clear();
    foot();
    refresh();
    list.scrollTo(app.draft.paths.length - 1);
    toast(`${fresh.length.toLocaleString()} added${n > fresh.length ? ` · ${(n - fresh.length).toLocaleString()} already in it` : ''}`, 'ok');
  }

  // The sheet (js/plbuild.js) on first use; what it builds lands here.
  let bq = null;
  async function openBuilder(b, smart) {
    bq ??= import('./plbuild.js').then((m) => m.builder({ lists: ctx.lists, known: ctx.known, done: built }));
    (await bq)(b, smart);
  }
  function built(paths, append, smart) {
    const d = app.draft, msg = `${paths.length.toLocaleString()} GIFs ${append ? 'added' : 'in the list'}`;
    if (append) { d.paths.push(...paths); picked.clear(); refresh(); toast(msg, 'ok'); }
    else undoable(() => { d.paths = paths; }, msg); // the same playlist, new contents
    if (smart) { d.smart = smart; d.smartPaths = paths.join('\n'); }
    header();
  }
  async function refreshNow() {
    const d = app.draft;
    if (!d.path || dirty(d)) { toast('save the playlist first', 'bad'); return; }
    try {
      const r = await (await import('./smart.js')).refresh(d.path, true);
      if (r) { d.smartPaths = d.paths.join('\n'); d.smart = (await import('./smart.js')).get(d.path); }
      header();
      toast(r && (r.added || r.removed) ? `+${r.added} −${r.removed}` : 'Up to date', 'ok');
    } catch (e) { toast(`refresh: ${e.message || e}`, 'bad'); }
  }

  se.append(h('div.card', { id: 'set-editor' },
    h('div.row', { style: 'margin-bottom:8px' }, h('button.sm.back', { id: 'set-back', onclick: ctx.closeEd }, '‹ Playlists'), title, eCount),
    smartRow, nameRow,
    h('div.edpanes',
      h('div', { id: 'set-ent-pane' },
        h('div.row', { style: 'margin-bottom:8px' }, h('div.grow', { style: 'min-width:140px' }, eSearch),
          h('button', { id: 'set-add', onclick: () => adding(true) }, '+ Add GIFs'),
          h('button', { id: 'set-build', 'aria-haspopup': 'dialog', onclick: (e) => openBuilder(e.currentTarget) }, 'Add by filter…'),
          selB, h('button', { id: 'set-arrange', 'aria-haspopup': 'menu', onclick: (e) => arrange(e.currentTarget) }, 'Arrange ▾'), pickChip),
        note('Drag ⠿ to reorder; tap GIFs to pick them', 'Alt+↑/↓ moves the focused GIF; ✕ removes it; its … menu moves it, favourites it or plays it longer; Arrange works on the GIFs picked'),
        empty, list.el),
      pane),
    h('div.edfoot.row', saveB, playB)));

  // A built-in playlist is saved under a new name: "Halloween 2", or the next number free.
  const freeName = (base) => { const known = ctx.known(); let i = 2; while (known.includes(`/sets/${base} ${i}.txt`)) i++; return `${base} ${i}`; };
  // The header: the playlist's name; a built-in one is saved as a new playlist.
  function header() {
    const d = app.draft, root = d.path && !d.path.startsWith('/sets/');
    title.textContent = d.path ? playlistLabel(d.path) : 'New playlist';
    nameRow.hidden = d.path && !root;
    nameL.textContent = root ? 'Save as a new playlist' : 'Name';
    name.value = root ? (d.copyName ??= freeName(playlistLabel(d.path))) : d.name;
    saveB.textContent = root ? 'Save as new' : 'Save';
    if (playB) playB.textContent = root ? 'Save as new & play' : 'Save & play';
    smartRow.hidden = !d.smart;
    if (d.smart) import('./smart.js').then((m) => m.summary(d.smart)).then((t) => { smartCap.textContent = 'Built from a search: ' + t; });
  }
  // A playlist (or a new one) is in app.draft: show it.
  function opened() {
    ov.load(app.draft.path);
    picked.clear();
    refresh();
  }

  function refresh() {
    const q = app.wasm.normalizeTitle(eSearch.value);
    const all = app.draft.paths.map((_, i) => i);
    view = q ? all.filter((i) => app.wasm.normalizeTitle(app.wasm.parsePath(app.draft.paths[i]).title).includes(q)) : all;
    eCount.textContent = `${app.draft.paths.length.toLocaleString()} GIFs${q ? ` · ${view.length} shown` : ''}`;
    empty.hidden = !!app.draft.paths.length;
    list.el.hidden = !empty.hidden;
    list.set(view);
    have = new Set(app.draft.paths);
    chip();
    if (el.classList.contains('adding')) tree.refresh();
  }
  // A change that takes many GIFs at once, with one step back in its toast
  // (while the same playlist is open).
  function undoable(fn, msg) {
    const d = app.draft, was = [...d.paths];
    fn();
    picked.clear();
    refresh();
    toast(msg, 'ok', ['Undo', () => { if (app.draft !== d) return; d.paths = was; picked.clear(); refresh(); }]);
  }
  function moveTo(i, j) {
    const p = app.draft.paths;
    p.splice(j, 0, ...p.splice(i, 1));
    picked.clear();
    refresh();
  }
  function drag(ev, row, i) {
    if (ev.button) return;
    const g = ev.currentTarget, y0 = ev.clientY, k0 = row._i, at = row.style.transform;
    let k = k0;
    g.setPointerCapture(ev.pointerId);
    row.classList.add('drag');
    g.onpointermove = (e) => {
      const dy = e.clientY - y0;
      k = Math.max(0, Math.min(view.length - 1, k0 + Math.round(dy / list.rh)));
      row.style.transform = `translateY(${k0 * list.rh + dy}px)`;
    };
    g.onpointerup = g.onpointercancel = () => {
      g.onpointermove = g.onpointerup = g.onpointercancel = null;
      row.classList.remove('drag');
      row.style.transform = at;
      if (k !== k0) moveTo(i, view[k]);
    };
  }
  function movePicked(end) {
    const p = app.draft.paths;
    const them = [...picked].sort((a, b) => a - b).map((i) => p[i]), rest = p.filter((_, i) => !picked.has(i));
    app.draft.paths = end ? [...rest, ...them] : [...them, ...rest];
    picked.clear();
    refresh();
  }
  function remove(i) {
    const t = fname(app.draft.paths[i]) || label(app.wasm.parsePath(app.draft.paths[i]));
    undoable(() => app.draft.paths.splice(i, 1), `“${t}” taken out`);
  }
  function removePicked() {
    const n = picked.size;
    undoable(() => { app.draft.paths = app.draft.paths.filter((_, i) => !picked.has(i)); }, `${n.toLocaleString()} taken out`);
  }
  function shuffle() {
    undoable(() => { app.draft.paths = app.wasm.filterManifest(app.draft.paths.join('\n'), JSON.stringify({ shuffle: true, seed: Date.now() % 1e9 })); }, 'Shuffled');
  }
  function sortTitle() {
    const key = new Map(app.draft.paths.map((p) => [p, app.wasm.normalizeTitle(label(app.wasm.parsePath(p)))]));
    app.draft.paths.sort((a, b) => key.get(a).localeCompare(key.get(b)));
    refresh();
  }

  // True once saved.
  async function save(andActivate) {
    const n = name.value.trim().replace(/\.txt$/, '');
    if (!n) { toast('give the playlist a name', 'bad'); name.focus(); return false; }
    if (!SET_NAME.test(n)) { toast(SET_NAME_RULE, 'bad'); return false; }
    if (!app.draft.paths.length) { toast('the playlist is empty', 'bad'); return false; }
    const path = `/sets/${n}.txt`;
    if (ctx.known().includes(path) && app.draft.name !== n && !confirm(`A playlist named "${n}" exists. Replace it?`)) return false;
    // Changed by hand, a smart playlist stops following its search.
    const d = app.draft, hand = d.smart && d.paths.join('\n') !== d.smartPaths;
    if (hand && !confirm('This playlist is built from a search; saving your changes by hand stops that. Save it as it is now?')) return false;
    try {
      const body = d.paths.join('\n'), text = body + '\n', sm = d.smart && (await import('./smart.js'));
      const put = () => inTurn(path, () => app.src.saveSet(n, text));
      if (hand) await sm.handSave(path, put); else await put();
      if (hand) d.smart = null; else if (d.smart) { await sm.put(path, d.smart.q, d.smart.from); d.smart = sm.get(path); d.smartPaths = body; }
      d.name = n;
      d.saved = body;
      app.texts.set(path, text);
      if (d.path !== path) { d.path = path; ov.load(path); }
      header();
      toast(`Saved “${n}” · ${d.paths.length.toLocaleString()} GIFs`, 'ok');
      if (andActivate && app.activeSet !== n) await ctx.activate({ value: n, label: n });
      await ctx.loadSets();
      return true;
    } catch (e) { toast('save: ' + e.message, 'bad'); return false; }
  }
  // Before the editor lets go of the playlist (another view, the list,
  // another playlist): changes not saved are saved, or put back — with an
  // Undo that brings them back into the editor. False if the save did not
  // go through (the editor stays).
  async function guard() {
    const d = app.draft;
    if (!dirty(d)) return true;
    const t = d.path ? playlistLabel(d.path) : name.value.trim() || 'New playlist';
    if (confirm(`Save changes to “${t}”?`)) return save(false);
    const was = d.paths;
    d.paths = d.saved ? d.saved.split('\n') : [];
    refresh();
    toast(`Changes to “${t}” put back`, 'ok', ['Undo', async () => {
      if (app.draft !== d && !(await guard())) return;
      app.draft = d;
      d.paths = was;
      emit('draft');
      app.go('sets/edit');
    }]);
    return true;
  }
  addEventListener('beforeunload', (e) => { if (dirty()) e.preventDefault(); });

  name.oninput = () => { const d = app.draft, v = name.value.trim(); if (!d.path) d.name = v; else if (!d.path.startsWith('/sets/')) d.copyName = v; };
  // Another playlist put in app.draft (a new one from the Library): its overrides.
  on('draft', () => { if (ov.path !== app.draft.path) { ov.load(app.draft.path); picked.clear(); } header(); refresh(); });
  if (app.draft.path) ov.load(app.draft.path);
  header();
  refresh();
  return {
    header, opened, refresh, guard,
    show() { refresh(); if (ov.path) ov.load(ov.path); },
    // Leaving the editor: the Add pane closes with it, and changes are asked about.
    left() { adding(false); list.refresh(); guard().then((ok) => ok || app.go('sets/edit')); },
    shown() {
      list.refresh();
      // A new playlist from the Library: its name first.
      if (app.draft.focus && el.classList.contains('ed')) { delete app.draft.focus; name.focus(); }
    },
  };
}
