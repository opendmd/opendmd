// Library: the card's GIFs by category (tabs, plus Live and Effects), a
// search over titles and the platform chips, and a Details panel for the
// one picked: its preview (from its prepared copy when there is one — the
// same frames, a cheaper fetch), what its name says (title, platform,
// category, author: the name *is* the metadata), and what can be done with
// it — Send to panel (POST /api/play), Add to playlist, Edit in Make art,
// Favourite / Hide in a playlist, Smooth motion, Edit details (a rename on
// the card) and Delete. On a narrow screen the panel is a sheet.
//
// "Add GIFs" asks for a title, platform, category and author per file
// (js/details.js, loaded then, with the Edit details dialog and Delete),
// uploads each under the name those make, prepares it in the browser and
// offers to add them to the playlist in play. "Make art" is its own view
// (js/studio.js). Listings come from GET /api/files, one directory at a time.

import { app, playlistLabel } from './state.js';
import { Overrides, glyphs, menu, moreButton, playlistSelect, popup, closeMenu } from './overrides.js';
import { PanelPreview } from './preview.js';
import { prepareBytes } from './prepare.js';
import { VList } from './vlist.js';
import { h, clear, emit, on, store, toast, label, lines } from './util.js';

// Keyword and season flags (opendmd_core::manifest KEYWORD_FLAGS, SEASONS);
// any other flag is the author's handle.
const TAGS = /^(INVERT|INTRO|LOGO|SLIDE|STORY|ANIM|STATIC|HALLOWEEN|XMAS)$/i;
export const authorOf = (e) => e.flags.find((f) => !TAGS.test(f)) || '';

// Upload one GIF into a category and prepare it straight away: the GIF
// (PUT /api/files/gifs/…), then its prepared copy, encoded here. A GIF that
// will not encode is still added (it plays as it is; `warn` says so) and any
// stale copy of an older file by that name is forgotten.
export async function addOne(category, name, bytes, warn) {
  const path = `/gifs/${category}/${name}`;
  await app.src.writeFile(path, bytes);
  try { await prepareBytes(path, bytes); } catch (e) {
    warn?.(`${name}: added, but it could not be prepared (${e.message || e}); it will play as it is`);
    try { await app.src.forget(path); } catch { /* nothing to forget */ }
  }
  emit('gifs', category);
  return path;
}

// Save a playlist's text: a set through PUT /api/sets, a root playlist as a
// file; the device reindexes if it is the one in play.
export async function savePlaylist(path, text) {
  if (path.startsWith('/sets/')) await app.src.saveSet(path.slice(6).replace(/\.txt$/, ''), text);
  else await app.src.writeFile(path, text);
  app.texts.set(path, text);
  if (path === app.activePath) await app.refreshStatus().catch(() => {});
}

// Append GIFs to a playlist (those not in it already).
export async function appendTo(path, paths) {
  const name = playlistLabel(path);
  try {
    const text = await app.text(path, true);
    const have = new Set(lines(text || ''));
    const add = paths.filter((p) => !have.has(p));
    if (!add.length) { toast(`already in “${name}”`); return; }
    await savePlaylist(path, (text && !text.endsWith('\n') ? text + '\n' : text || '') + add.join('\n') + '\n');
    toast(`${add.length} added to “${name}”`, 'ok');
  } catch (e) { toast(`adding to “${name}”: ${e.message}`, 'bad'); }
}

// Add new GIFs to the playlist in play, if the user wants them there.
export async function offerPlaylist(paths) {
  const n = paths.length;
  if (confirm(`Add ${n} new GIF${n > 1 ? 's' : ''} to the playlist “${playlistLabel(app.activePath)}”?`)) await appendTo(app.activePath, paths);
}

export async function mount(el) {
  let cat = store.get('lib.cat', 'Consoles');
  let cats = []; // [value, label]
  let items = [], shown = [], current = null;
  const platforms = new Set();

  // --- The header: search, Add GIFs, Make art; the category tabs; the platforms.
  const search = h('input', { type: 'search', id: 'lib-search', placeholder: 'Search GIFs', enterKeyHint: 'search', oninput: () => web || apply(), onkeydown: (e) => web && e.key === 'Enter' && R.search(search.value) });
  const count = h('span.muted', { id: 'lib-count' });
  // Your GIFs, or GIFs and pictures from the web (js/find.js, loaded then):
  // the same search box drives either, and the list comes back as it was.
  let web = null, R = null, libQ = '', webQ = '', libTop = 0;
  const srcs = h('div.chips', { id: 'lib-src' }, [['', 'Your GIFs'], ['gifcities', 'GifCities'], ['gameart', 'Game art']]
    .map(([v, t]) => h('button.chip' + (v ? '' : '.on'), { 'data-src': v, onclick: () => useSource(v) }, t)));
  const goBtn = h('button', { id: 'lib-go', hidden: true, onclick: () => R.search(search.value) }, 'Search');
  const findBox = h('div.card', { id: 'lib-find', hidden: true });
  const tabs = h('div.tabs', { id: 'lib-cats', role: 'tablist' });
  const chips = h('div.chips.scroll', { id: 'lib-chips' });
  const addInput = h('input', { type: 'file', multiple: true, accept: '.gif,image/gif', hidden: true, id: 'lib-add-files', onchange: () => { addGifs([...addInput.files]); addInput.value = ''; } });
  const addBtn = app.device && h('label.btn', { id: 'lib-add-btn', title: 'Upload GIFs to the card' }, 'Add GIFs…', addInput);
  const artBtn = h('button.pri', { id: 'lib-art', title: 'Make a GIF for the panel from a picture', onclick: () => app.go('studio') }, '✎ Make art');
  // Uploads in progress: a thin bar and a line.
  const addBar = h('i');
  const addMsg = h('span', { id: 'lib-add-progress' });
  const addErr = h('div.log.bad', { id: 'lib-add-errors' });
  const addCard = h('div.lib-progress', { id: 'lib-add', hidden: true }, h('div.bar', addBar), h('div.cap', addMsg), addErr);

  const selCount = h('b');
  const selbar = h('div.selbar.row.sp', { hidden: true, id: 'lib-selbar' },
    h('span', selCount, ' selected'),
    h('div.row',
      h('button', { id: 'lib-all', onclick: selectAll }, 'Select shown'),
      h('button.pri', { id: 'lib-to-set', onclick: toSet }, 'Add to playlist'),
      h('button', { onclick: () => { app.selection.clear(); sync(); } }, 'Clear')));

  // Favourite / hide / play longer act on one playlist: the one in play,
  // unless another is chosen in the Details panel.
  const ov = new Overrides();
  ov.onChange = () => { list.refresh(); syncOv(); };

  const list = new VList({
    rowHeight: 44,
    render(row, it) {
      const box = h('input', { type: 'checkbox', checked: app.selection.has(it.path), 'aria-label': 'select', onclick: (e) => { e.stopPropagation(); toggle(it.path, box.checked); } });
      const o = ov.get(it.path);
      clear(row, box, h('span.t', label(it.e)), it.e.platform && h('span.tag', it.e.platform), glyphs(o), moreButton(ov, it.path));
      row.classList.toggle('cur', it.path === current?.path);
      row.classList.toggle('hid', !!o?.never);
      row.dataset.path = it.path;
    },
    onClick: (it) => preview(it),
    onMenu: (it, _i, x, y) => menu(ov, it.path, x, y),
  });
  list.el.id = 'lib-list';

  // --- Details
  const canvas = h('canvas.panel', { id: 'lib-panel' });
  const cap = h('div.cap.mono', { id: 'lib-cap' }, 'Pick a GIF to see it here');
  const dTitle = h('h2', { id: 'lib-d-title' });
  const dd = (id) => h('dd', { id });
  const [dPlat, dCat, dAuthor, dTags] = ['lib-d-platform', 'lib-d-category', 'lib-d-author', 'lib-d-tags'].map(dd);
  const tagsDt = h('dt', 'Tags');
  const facts = h('dl.facts', { id: 'lib-d-facts' }, h('dt', 'Platform'), dPlat, h('dt', 'Category'), dCat, h('dt', 'Author'), dAuthor, tagsDt, dTags);
  const sendBtn = app.device && h('button.pri', { id: 'lib-play', disabled: true, onclick: () => app.play(current.path) }, 'Send to panel');
  const plBtn = h('button', { id: 'lib-d-add', onclick: (e) => pickPlaylist(e.currentTarget) }, 'Add to playlist…');
  const editArt = h('button', { id: 'lib-d-art', title: 'Open it in Make art', onclick: () => { app.artFrom = { path: current.path, e: current.e }; app.go('studio'); } }, '✎ Edit in Make art');
  const favBtn = h('button', { id: 'lib-d-fav', onclick: () => put((o) => ({ w: o.w > 1 ? null : 3, never: null })) });
  const hideBtn = h('button', { id: 'lib-d-hide', onclick: () => put((o) => ({ never: o.never ? null : true, w: null })) });
  const ovHolder = h('label.cap', { id: 'lib-ovfor' });
  // Smooth motion for the previewed GIF (device only; js/smooth.js, loaded
  // on the first preview): shown only when there is motion to smooth.
  const smoothBox = app.device && h('div', { id: 'lib-smooth', hidden: true });
  // "Live" and "Effects" (js/live.js, loaded when chosen): the clock, a
  // scrolling text, the effects.
  const liveBox = h('div', { id: 'lib-live', hidden: true });
  const editBtn = app.device && h('button', { id: 'lib-d-edit', onclick: () => edit() }, 'Edit details…');
  const delBtn = app.device && h('button.danger', { id: 'lib-d-del', onclick: () => remove() }, 'Delete…');
  const gifOnly = [facts, editArt, editBtn, delBtn].filter(Boolean);
  const body = h('div', { id: 'lib-d-body', hidden: true },
    dTitle, facts,
    h('div.row.acts', sendBtn, plBtn, editArt),
    smoothBox,
    h('div.row.acts', favBtn, hideBtn, ovHolder),
    app.device && h('div.row.acts.end', editBtn, delBtn));
  const side = h('div.side', { id: 'lib-side' },
    h('div.card', { id: 'lib-detail' },
      h('button.sm.close', { id: 'lib-d-close', 'aria-label': 'Close', onclick: () => side.classList.remove('open') }, '✕'),
      canvas, cap, body, liveBox));

  const grid = h('div.libgrid',
    h('div',
      h('div.card.libhead',
        h('div.row', h('div.grow.lib-q', search), goBtn, count, addBtn, artBtn),
        srcs, tabs, chips, addCard),
      findBox, list.el, selbar),
    side);
  el.append(grid);

  const pv = new PanelPreview(canvas, { onError: (m) => { cap.textContent = 'preview: ' + m; } });

  async function preview(it) {
    current = it;
    it.pick?.();
    list.refresh();
    side.classList.add('open');
    body.hidden = false;
    const e = it.e, gif = it.path[0] != '@';
    dTitle.textContent = label(e);
    for (const x of gifOnly) x.hidden = !gif;
    dPlat.textContent = e.platform || '—';
    dCat.textContent = e.category || '—';
    dAuthor.textContent = authorOf(e) || '—';
    const tags = e.flags.filter((f) => TAGS.test(f));
    dTags.textContent = tags.join(' · ');
    tagsDt.hidden = dTags.hidden = !tags.length;
    syncOv();
    if (sendBtn) { sendBtn.disabled = false; sendBtn.title = it.path; }
    cap.textContent = 'loading…';
    if (smoothBox) Object.assign(smoothBox, { hidden: true }).dataset.state = 'analysing';
    let bytes;
    try {
      if (await pv.show(it.path, async (p) => (bytes = await app.previewBytes(p)))) {
        // An effect may run faster than the page's frames: play it by the clock.
        if (it.pick) pv.syncTo(0, performance.now());
        cap.textContent = it.name;
        if (smoothBox && gif) import('./smooth.js').then((m) => m.detail(smoothBox, it, bytes, pv, cap)).catch(() => {});
      }
    } catch (err) { cap.textContent = 'preview: ' + err.message; }
  }

  // Favourite and Hide, in the chosen playlist.
  function put(patch) {
    if (!ov.path) { toast('save the playlist first', 'bad'); return; }
    const o = ov.get(current.path) || {};
    ov.set(current.path, { ...o, ...patch(o) });
  }
  function syncOv() {
    const o = (current && ov.get(current.path)) || {};
    favBtn.textContent = o.w > 1 ? '★ Favourite' : '☆ Favourite';
    hideBtn.textContent = o.never ? '⊘ Hidden' : '⊘ Hide';
    favBtn.classList.toggle('on', o.w > 1);
    hideBtn.classList.toggle('on', !!o.never);
  }

  // "Add to playlist…": a playlist from a menu, or a new one.
  async function pickPlaylist(btn) {
    const it = current;
    const lists = await app.playlists().catch(() => []);
    const r = btn.getBoundingClientRect();
    popup(h('div.menu', { id: 'lib-pl-menu', role: 'menu' },
      h('div.mh', h('b', label(it.e)), h('span.muted', ' — add to')),
      lists.map((p) => h('button.mi', { 'data-path': p.path, onclick: () => { closeMenu(); appendTo(p.path, [it.path]); } }, p.label)),
      h('button.mi', { id: 'lib-pl-new', onclick: () => { closeMenu(); app.selection.add(it.path); toSet(); } }, 'New playlist…')), r.left, r.bottom + 4);
  }

  function toggle(path, onOff) {
    if (onOff) app.selection.add(path); else app.selection.delete(path);
    sync();
  }
  function selectAll() { for (const it of shown) app.selection.add(it.path); sync(); }
  function sync() {
    selCount.textContent = app.selection.size;
    selbar.hidden = !app.selection.size;
    list.refresh();
  }
  function toSet() {
    const have = new Set(app.draft.paths);
    for (const p of app.selection) if (!have.has(p)) app.draft.paths.push(p);
    toast(`${app.selection.size} entries added to the playlist being edited`, 'ok');
    emit('draft');
    app.go('sets');
  }

  async function useSource(id) {
    if ((web?.id || '') === id) return;
    let m = null;
    if (id) try { m = await import('./find.js'); await m.style(); } catch { toast('the search could not be loaded', 'bad'); return; }
    for (const b of srcs.children) b.classList.toggle('on', b.dataset.src === id);
    if (!m) {
      web = null;
      webQ = search.value;
      search.value = libQ;
      search.placeholder = 'Search GIFs';
      grid.classList.remove('web');
      findBox.hidden = goBtn.hidden = true;
      list.el.scrollTop = libTop;
      list.refresh();
      return;
    }
    if (!web) { libQ = search.value; libTop = list.el.scrollTop; search.value = webQ; }
    web = m.sources.find((s) => s.id === id);
    R ??= m.mountResults(findBox, { onPick: toArt });
    R.use(web);
    search.placeholder = web.placeholder;
    grid.classList.add('web');
    findBox.hidden = goBtn.hidden = false;
    if (search.value.trim()) R.search(search.value);
  }
  // A web GIF or picture into Make art (there to frame, then save).
  async function toArt(hit) {
    app.artFrom = { blob: await hit.fetch(), label: hit.title, title: hit.hint, source: hit.source, platform: hit.platform, logo: hit.logo };
    app.go('studio');
  }

  // --- Categories and the list
  const real = () => cats.filter(([v]) => v[0] != '@').map(([v]) => v);
  async function categories() {
    let dirs = [];
    try { dirs = (await app.src.list('/gifs')).filter((d) => d.dir).map((d) => d.name).sort(); } catch (e) { toast('listing /gifs: ' + e.message, 'bad'); }
    cats = [...dirs.map((d) => [d, d]), ['@', 'Live'], ['@fx', 'Effects']];
    if (!cats.some(([v]) => v === cat)) cat = dirs.includes('Consoles') ? 'Consoles' : cats[0][0];
    renderTabs();
    await load();
  }
  function renderTabs() {
    clear(tabs, cats.map(([v, t]) => h('button.tab' + (v === cat ? '.on' : ''), {
      role: 'tab', 'data-cat': v, 'aria-selected': String(v === cat),
      onclick: () => { if (v !== cat) { cat = v; renderTabs(); load(); } },
    }, t)));
    tabs.querySelector('.on')?.scrollIntoView({ block: 'nearest', inline: 'nearest' });
  }

  async function load() {
    store.set('lib.cat', cat);
    if (!(liveBox.hidden = cat[0] != '@')) {
      items = (await import('./live.js'))[cat == '@' ? 'items' : 'effects'](liveBox, preview, () => list.refresh());
      platforms.clear();
      renderChips();
      return apply();
    }
    count.textContent = 'loading…';
    const want = cat, dir = `/gifs/${cat}`;
    let files = [];
    try { files = (await app.src.list(dir)).filter((f) => !f.dir).map((f) => f.name); } catch (e) { toast(`listing ${dir}: ${e.message}`, 'bad'); }
    // Another category was chosen while this one listed (a big one takes seconds on the device).
    if (want != cat) return;
    items = files.filter((n) => /\.gif$/i.test(n)).map((name) => {
      const path = `${dir}/${name}`;
      const e = app.wasm.parsePath(path);
      return { path, e, name, norm: app.wasm.normalizeTitle(e.title) };
    }).sort((a, b) => a.norm.localeCompare(b.norm) || (a.e.variant ?? 0) - (b.e.variant ?? 0));
    platforms.clear();
    renderChips();
    apply();
  }

  function renderChips() {
    const counts = new Map();
    for (const it of items) if (it.e.platform) counts.set(it.e.platform, (counts.get(it.e.platform) || 0) + 1);
    const top = [...counts].sort((a, b) => b[1] - a[1]).slice(0, 24);
    chips.hidden = top.length < 2;
    clear(chips, top.map(([p, n]) => h('button.chip' + (platforms.has(p) ? '.on' : ''), {
      'data-platform': p,
      onclick: () => { if (!platforms.delete(p)) platforms.add(p); renderChips(); apply(); },
    }, p, h('small', n))));
  }

  function apply() {
    const text = web ? libQ : search.value; // (the box holds a web search meanwhile)
    const q = app.wasm.normalizeTitle(text);
    const qn = text.trim().toLowerCase();
    shown = items.filter((it) => (!platforms.size || platforms.has(it.e.platform))
      && (!q || it.norm.includes(q) || (qn && it.name.toLowerCase().includes(qn))));
    count.textContent = `${shown.length.toLocaleString()} of ${items.length.toLocaleString()}`;
    list.set(shown);
  }

  // Show a GIF: its category's tab, the row, its details.
  async function reveal(path) {
    const c = path.slice(6, path.lastIndexOf('/'));
    search.value = '';
    if (cats.some(([v]) => v === c)) cat = c;
    await categories();
    if (c !== cat && cats.some(([v]) => v === c)) { cat = c; renderTabs(); await load(); }
    const it = items.find((x) => x.path === path);
    if (it) { list.el.scrollTop = shown.indexOf(it) * list.rh; preview(it); }
  }

  // --- Add GIFs: a name for each (the dialog), then one file at a time.
  const opts = () => ({ category: cat[0] == '@' ? '' : cat, categories: real(), platforms: [...new Set(items.map((x) => x.e.platform).filter(Boolean))] });
  async function addGifs(picked) {
    const gifs = picked.filter((f) => /\.gif$/i.test(f.name));
    if (!gifs.length) { toast('choose .gif files', 'bad'); return; }
    const plan = await (await import('./details.js')).addDialog(gifs, opts());
    if (!plan) return;
    addCard.hidden = false;
    addErr.replaceChildren();
    const added = [];
    const warn = (m) => addErr.append(h('div', m));
    for (let i = 0; i < plan.length; i++) {
      const { file, category, name } = plan[i];
      addMsg.textContent = `Adding ${i + 1} of ${plan.length} · ${name}`;
      addBar.style.width = `${(100 * i) / plan.length}%`;
      try { added.push(await addOne(category, name, new Uint8Array(await file.arrayBuffer()), warn)); } catch (e) { warn(`${name}: ${e.message || e}`); }
    }
    addBar.style.width = '100%';
    const where = [...new Set(plan.map((p) => p.category))].join(', ');
    addMsg.textContent = `${added.length} of ${plan.length} added to ${where}.`;
    if (added.length) {
      await reveal(added[0]);
      await offerPlaylist(added);
    }
  }

  // Edit details (a rename) and Delete, from js/details.js.
  async function edit() {
    const to = await (await import('./details.js')).editDialog(current.path, opts());
    if (to) { await ov.load(ov.path); await reveal(to); }
  }
  async function remove() {
    if (!(await (await import('./details.js')).removeGif(current.path, label(current.e)))) return;
    current = null;
    body.hidden = true;
    side.classList.remove('open');
    pv.clear();
    cap.textContent = 'Pick a GIF to see it here';
    await ov.load(ov.path);
    await categories();
  }

  async function playlists() {
    const lists = await app.playlists().catch(() => []);
    if (!lists.length) lists.push({ path: '/filelist.txt', label: 'All files' });
    const want = ov.path && lists.some((p) => p.path === ov.path) ? ov.path
      : lists.some((p) => p.path === app.activePath) ? app.activePath : lists[0].path;
    clear(ovHolder, 'in', playlistSelect(lists, want, (p) => ov.load(p)));
    if (want !== ov.path) await ov.load(want);
  }

  // The Studio saved a GIF, or the card's GIFs changed some other way.
  let dirty = false;
  on('gifs', () => { dirty = true; });
  await categories();
  await playlists();
  sync();
  return {
    show() {
      sync();
      playlists();
      if (dirty) { dirty = false; categories(); }
    },
  };
}
