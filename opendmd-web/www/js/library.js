// Library: the GIFs on the player as a tree of folders (js/tree.js), and
// Details for the one picked. + Add: Add GIFs (your own GIFs and pictures,
// fitted to the panel first where they need it: js/fit.js) and Find art
// online (the website's Studio, opendmd.dev/studio/#import: finding art on
// the web is the website's). See "The Library's layout" in CLAUDE.md. The user's tags
// (js/tags.js; the built-in one, Favourite, a heart) show on the rows and
// in Details, and Select's More ▾ favourites, tags, moves, renames or
// deletes many GIFs at once (js/tags.js, js/refs.js).

import { app, platName } from './state.js';
import { Overrides, glyphs, menu, moreButton, actions, popup, closeMenu } from './overrides.js';
import { PanelPreview } from './preview.js';
import { Tree, chipRow, forget as forgetDir, recheck } from './tree.js';
import { forget as forgetThumb, check as checkThumb } from './thumbs.js';
import { authorOf, addOne, offerPlaylist, targetMenu } from './gifs.js';
import { h, clear, on, store, toast, label, niceName, flagName, isFlag, SITE, unreachable, enc, base } from './util.js';

export async function mount(el) {
  let current = null, selecting = false;
  const plats = new Set();

  const search = h('input', { type: 'search', id: 'lib-search', placeholder: 'Search GIFs', enterKeyHint: 'search', oninput: () => apply() });
  const count = h('span.muted', { id: 'lib-count' });
  const fold = h('button.sm', { id: 'lib-fold', title: 'Close every folder', 'aria-label': 'Close every folder', onclick: () => tree.closeAll() }, '⊟');
  // Checkboxes only in Select mode (the button, or a long press on a row).
  const selBtn = h('button.sm', { id: 'lib-select', onclick: () => selMode(!selecting) }, 'Select');
  const chips = h('div.chips.scroll', { id: 'lib-chips' });
  const addInput = h('input', { type: 'file', multiple: true, accept: '.gif,image/*', hidden: true, id: 'lib-add-files', onchange: () => { addGifs([...addInput.files]); addInput.value = ''; } });
  // Finding art on the web is the website's: its Studio's Import, with the
  // search's words and its one platform chip (the demo's own site beside it).
  const online = () => {
    const q = search.value.trim(), p = new URLSearchParams({ ...(q && { q }), ...(plats.size === 1 && { platform: [...plats][0] }) });
    return new URL('studio/', app.demo ? new URL('../', location.href) : SITE).href + '#import' + (p.size ? '?' + p : '');
  };
  const findLink = (id, cls, text) => h('a', { id, className: cls, href: online(), target: '_blank', rel: 'noopener' }, text);
  // + Add ▾: Add GIFs… (your own files), Find art online ↗.
  const addBtn = app.device && h('button', { id: 'lib-add-btn', 'aria-haspopup': 'menu', onclick: (e) => {
    const r = e.currentTarget.getBoundingClientRect();
    popup(h('div.menu', { id: 'lib-add-menu', role: 'menu' },
      h('button.mi', { id: 'lib-add-gifs', onclick: () => { closeMenu(); addInput.click(); } }, 'Add GIFs…'),
      h('p.cap', { style: 'margin:0 8px 4px' }, 'GIFs and pictures from this device'),
      // (Closed once the click has opened the tab.)
      Object.assign(findLink('lib-find-online', 'btn mi', 'Find art online ↗'), { onclick: () => setTimeout(closeMenu) }),
      h('p.cap', { style: 'margin:0 8px 4px' }, 'On opendmd.dev: search, convert, then send it here')), r.right - 250, r.bottom + 4);
  } }, '+ Add ▾');
  // Nothing on the player matches: the same words, online.
  const webQ = h('b'), webGo = findLink('lib-web-go', 'txt', 'Find art online ↗'), webMsg = h('span');
  const webHint = h('div.lib-web', { id: 'lib-web', hidden: true }, webMsg, webGo);
  const addBar = h('i');
  const addMsg = h('span', { id: 'lib-add-progress' });
  const addErr = h('div.log.bad', { id: 'lib-add-errors' });
  const addCard = h('div.lib-progress', { id: 'lib-add', hidden: true }, h('div.bar', addBar), h('div.cap', addMsg), addErr);

  const selCount = h('b');
  // Selected but not in the list as it shows now (another folder, closed,
  // or not what the search finds): the bar says how many.
  const selOff = h('span.muted', { id: 'lib-sel-off' });
  // Into a playlist picked from a menu; the selection clears once it is in
  // (and Select ends with a new playlist, off in the editor). Every action
  // on the selection clears it once done (js/refs.js, bulk() below).
  // One row, on a phone too: Done, the count, All, Add to ▾ and More ▾
  // (None and the bulk actions); the long labels on a laptop (`.lg`/`.sh`).
  const two = (lg, sh) => [h('span.lg', lg), h('span.sh', sh)];
  const moreSel = h('button', { id: 'lib-sel-more', 'aria-haspopup': 'menu', 'aria-label': 'More', onclick: (e) => bulkMenu(e.currentTarget) }, two('More ▾', '⋯'));
  const toSetBtn = h('button.pri', { id: 'lib-to-set', 'aria-haspopup': 'menu', onclick: (e) => targetMenu(e.currentTarget, [...app.selection], (fresh) => { if (fresh) selMode(false); else { app.selection.clear(); sync(); } }) }, two('Add to playlist ▾', 'Add to ▾'));
  const selbar = h('div.selbar', { hidden: true, id: 'lib-selbar' },
    h('button.sm', { id: 'lib-sel-done', title: 'Done: end Select', 'aria-label': 'Done' }, two('Done', '✕')),
    h('span.grow', selCount, ' selected', selOff),
    h('button.sm', { id: 'lib-all', title: 'Select all shown', onclick: selectAll }, two('Select all shown', 'All')),
    toSetBtn, moreSel);
  selbar.firstChild.onclick = () => selMode(false);
  // The user's tags (js/tags.js), once the card has any or Details asks.
  let tg = null;
  const tagsMod = () => import('./tags.js').then(async (m) => { await m.tags(); tg = m; return m; });
  on('tags', () => { tree.refresh(); favs(); });

  const ov = new Overrides();
  ov.onChange = () => tree.refresh();

  const tree = new Tree({
    picked: app.selection,
    ticking: () => selecting,
    open: store.get('lib.open', null) ?? ['Consoles'],
    onOpen: (o) => store.set('lib.open', o),
    onPick: (it) => preview(it),
    onMenu: (it, x, y) => menu(ov, it.path, x, y),
    onHold: (it) => { selMode(true); toggle(it.path, true); },
    onTick: () => sync(),
    onError: (m) => toast(m, 'bad'),
    extra(it) {
      const u = tg && app.tags.get(it.path);
      return [glyphs(ov.get(it.path), tg?.isFav(u)), tg && tg.bits(u), platName(it.e.platform) && h('span.tag', { title: it.e.platform }, platName(it.e.platform)), moreButton(ov, it.path)];
    },
    onUpdate: counted,
  });
  tree.el.id = 'lib-list';

  const canvas = h('canvas.panel', { id: 'lib-panel' });
  const cap = h('div.cap', { id: 'lib-cap' }, 'Pick a GIF to see it here');
  const dTitle = h('h2', { id: 'lib-d-title' });
  const meta = h('div.cap', { id: 'lib-d-meta' });
  const sendBtn = app.device && h('button.pri', { id: 'lib-play', disabled: true, onclick: () => app.play(current.path) }, 'Play now');
  const plBtn = h('button', { id: 'lib-d-add', 'aria-haspopup': 'menu', onclick: (e) => targetMenu(e.currentTarget, [current.path]) }, 'Add to playlist…');
  const moreBtn = h('button.more', { id: 'lib-d-more', 'aria-label': 'More', 'aria-haspopup': 'menu', onclick: (e) => detailMenu(e.currentTarget) }, '…');
  const smoothBox = app.device && h('div', { id: 'lib-smooth', hidden: true });
  const tagBox = h('div', { id: 'lib-d-tags' });
  const gifOnly = [meta, moreBtn];
  const body = h('div', { id: 'lib-d-body', hidden: true },
    h('div.dhead', h('div.grow', dTitle, meta), moreBtn),
    tagBox,
    h('div.row.acts', sendBtn, plBtn), smoothBox);
  const side = h('div.side', { id: 'lib-side' },
    h('div.card', { id: 'lib-detail' },
      h('button.sm.close', { id: 'lib-d-close', 'aria-label': 'Close', onclick: () => side.classList.remove('open') }, '✕'),
      canvas, cap, body));

  // A phone's detail sheet: closed by its ✕, by Select, and by scrolling
  // the list under it (it stays open on a laptop, where it is a column).
  const phone = matchMedia('(max-width:760px)');
  const closeSheet = () => side.classList.remove('open');
  let sheetAt = null;
  const onScroll = (e) => {
    if (!phone.matches || !side.classList.contains('open') || side.contains(e.target)) return;
    const y = e.target.scrollTop ?? scrollY;
    sheetAt ??= [e.target, y];
    if (sheetAt[0] !== e.target) sheetAt = [e.target, y];
    else if (Math.abs(y - sheetAt[1]) > 48) closeSheet();
  };
  addEventListener('scroll', onScroll, { capture: true, passive: true });

  // On a phone the chips scroll sideways: fade an edge while more is off that side.
  // ♥ Favourites: one chip beside the platforms once something is a favourite.
  const FQ = /(^|\s)tag:favourite(?=\s|$)/;
  const favChip = h('button.chip', { id: 'lib-fav-chip', hidden: true, onclick: () => {
    const q = search.value.trim();
    search.value = FQ.test(q) ? q.replace(FQ, '').trim() : (q + ' tag:favourite').trim();
    apply();
  } }, '♥ Favourites');
  const favs = () => {
    favChip.hidden = !app.tags?.names().some(([t]) => t === 'favourite');
    favChip.classList.toggle('on', FQ.test(search.value));
  };
  const goneMsg = h('span.grow');
  const goneBox = h('div.lib-web', { id: 'lib-gone', hidden: true }, goneMsg,
    h('button.txt', { id: 'lib-gone-move', onclick: () => strays(true) }, 'Move their tags'),
    h('button.txt', { id: 'lib-gone-forget', onclick: () => strays(false) }, 'Forget them'));
  const r2 = h('div.lib-r2', { id: 'lib-r2' }, favChip, chips);
  const edge = () => {
    r2.classList.toggle('more', r2.scrollLeft + r2.clientWidth < r2.scrollWidth - 2);
    r2.classList.toggle('less', r2.scrollLeft > 2);
  };
  r2.addEventListener('scroll', edge, { passive: true });
  const ro = new ResizeObserver(edge);
  for (const x of [r2, chips]) ro.observe(x);

  const grid = h('div.libgrid',
    h('div',
      h('div.card.libhead',
        h('div.lib-q', search), count, fold, selBtn, addBtn, addInput,
        r2, addCard),
      webHint, goneBox, tree.el, selbar),
    side);
  el.append(grid);

  // (In plain words: the player's own error names the file's path.)
  const NO = 'Preview not available for this GIF', pv = new PanelPreview(canvas, { onError: () => { cap.textContent = NO; } });

  async function preview(it) {
    current = it;
    tree.cur = it.path;
    it.pick?.();
    tree.refresh();
    side.classList.add('open');
    sheetAt = null;
    body.hidden = false;
    const e = it.e, gif = it.path[0] != '@';
    dTitle.textContent = label(e);
    for (const x of gifOnly) x.hidden = !gif;
    moreBtn.hidden ||= !app.device; // (its items are the card's)
    tagBox.hidden = !gif;
    if (gif) (tg ? Promise.resolve(tg) : tagsMod()).then((m) => { if (current === it) m.detail(tagBox, it.path, (t) => { search.value = 'tag:' + (/[\s,:]/.test(t) ? `"${t}"` : t); apply(); }, ov); }).catch(() => {});
    const a = authorOf(e), sp = (id, t) => t && h('span', { id }, t);
    clear(meta, [sp('lib-d-platform', platName(e.platform)), sp('lib-d-category', e.category && niceName(e.category)),
      a && ['by ', sp('lib-d-author', a)], ...e.flags.filter(isFlag).map(flagName)].filter(Boolean).flatMap((x, i) => (i ? [' · ', x] : [x])));
    if (sendBtn) { sendBtn.disabled = false; sendBtn.dataset.path = it.path; }
    cap.textContent = 'Loading…';
    delete cap.dataset.shown;
    if (smoothBox) Object.assign(smoothBox, { hidden: true }).dataset.state = 'analysing';
    let bytes;
    try {
      if (await pv.show(it.path, async (p) => (bytes = await app.previewBytes(p)))) {
        if (it.pick) pv.syncTo(0, performance.now());
        cap.textContent = '';
        cap.dataset.shown = it.name;
        if (gif) checkThumb(it.path, bytes);
        if (smoothBox && gif) import('./smooth.js').then((m) => m.detail(smoothBox, it, bytes, pv, cap)).catch(() => {});
      }
    } catch (err) { cap.textContent = unreachable(err) ? 'Panel not reachable' : NO; }
  }

  function detailMenu(btn) {
    actions(btn, 'lib-d-menu', [
      app.device && ['lib-d-edit', 'Edit details…', edit],
      // (To change the GIF itself: download it and open it in the website's Studio.)
      app.device && ['lib-d-dl', 'Download GIF', () => h('a', { href: enc(current.path), download: base(current.path) }).click()],
      app.device && ['lib-d-del', 'Delete…', remove, '.danger']], label(current.e));
  }

  function toggle(path, onOff) {
    if (onOff) app.selection.add(path); else app.selection.delete(path);
    sync();
  }
  function selectAll() { for (const it of tree.res) app.selection.add(it.path); sync(); }
  function selMode(v) {
    selecting = v;
    if (!v) app.selection.clear();
    else closeSheet(); // (a phone: the detail sheet would cover what is picked)
    selBtn.classList.toggle('on', v);
    sync();
  }
  function sync() {
    const n = app.selection.size;
    if (n) selecting = true;
    selCount.textContent = n;
    selbar.hidden = !selecting;
    toSetBtn.disabled = !n;
    unseen();
    tree.refresh();
  }
  function unseen() {
    let k = 0;
    if (app.selection.size) {
      const shown = new Set(tree.res.map((x) => x.path));
      for (const p of app.selection) if (!shown.has(p)) k++;
    }
    selOff.textContent = k ? ` · ${k.toLocaleString()} not shown` : '';
  }

  // The search, the platforms: the tree filters; counted() shows the result.
  // Clearing the box clears the platforms picked with it.
  let was = '';
  function apply() {
    const q = search.value.trim();
    if (was && !q) plats.clear();
    was = q;
    tree.filter(search.value, plats);
  }
  function counted(st) {
    const text = search.value.trim();
    // The GIF in Details gone from its folder (the card's listing, read again, lacks it): Details closes.
    const cat = current?.path.startsWith('/gifs/') && current.path.split('/')[2], l = cat && tree.items(cat);
    if (l && !l.some((x) => x.path === current.path)) {
      current = null;
      tree.cur = '';
      body.hidden = true;
      closeSheet();
      pv.clear();
      cap.textContent = 'Pick a GIF to see it here';
    }
    count.textContent = st.err || (st.busy ? (st.text ? 'Searching…' : 'Loading…') : `${st.shown.toLocaleString()} of ${st.pool.toLocaleString()}${st.bad ? ' · some folders couldn’t be read: type again to retry' : ''}`);
    count.classList.toggle('bad', !!st.err);
    favs();
    unseen();
    // Every folder listed (a search): tags kept for GIFs no longer here.
    if (!st.busy && st.text && !st.bad && tg && lostKey !== (lostKey = `${st.pool}|${app.tags.size}`)) {
      const all = tree.real().flatMap((c) => (tree.items(c) || []).map((x) => x.path));
      lost = tg.strays(all);
      const moved = lost.filter((x) => x.to).length;
      goneMsg.textContent = `${lost.length.toLocaleString()} tagged GIF${lost.length > 1 ? 's are' : ' is'} no longer on the card${moved ? ` (${moved.toLocaleString()} found in another folder)` : ''}.`;
      goneBox.hidden = !lost.length;
      goneBox.querySelector('#lib-gone-move').hidden = !moved;
    }
    grid.classList.toggle('noplat', chipRow(chips, st.plats, plats, 'data-platform', apply, (v) => platName(v) || 'Other') < 2 && favChip.hidden);
    grid.classList.toggle('lq', !!text);
    webQ.textContent = text;
    webMsg.replaceChildren('Nothing here matches “', webQ, '”. ');
    webGo.href = online();
    webHint.hidden = !text || st.busy || st.matched > 0;
    tree.el.hidden = !webHint.hidden; // no empty box under the hint
  }

  // Tags of GIFs gone from the card: onto the one GIF elsewhere with the
  // same name (`move`), or forgotten.
  let lost = [], lostKey = '';
  async function strays(move) {
    const pairs = lost.map((x) => ({ from: x.path, to: move ? x.to : null })).filter((x) => !move || x.to);
    await app.tags.relink(pairs);
    toast(move ? `Tags of ${pairs.length.toLocaleString()} GIFs moved to where they are now` : `Tags of ${pairs.length.toLocaleString()} GIFs forgotten`, 'ok');
    goneBox.hidden = true;
    apply();
  }

  // Select's More ▾: ♥ Favourite, Tag… (js/tags.js); Move to folder…,
  // Rename…, Delete… (js/refs.js). Each clears the selection once done (a
  // tag or favourite's Undo, later, leaves a new one alone).
  function bulkMenu(btn) {
    const paths = [...app.selection].filter((p) => p.startsWith('/gifs/'));
    let once = false;
    const done = () => { if (once) tree.refresh(); else { once = true; app.selection.clear(); sync(); } };
    const none = ['lib-none', 'Select none', () => { app.selection.clear(); sync(); }];
    if (!paths.length) { actions(btn, 'lib-bulk', [none], 'Nothing selected'); return; }
    actions(btn, 'lib-bulk', [none,
      ['lib-bulk-fav', '♥ Favourite', () => tagsMod().then((m) => m.bulkFav(paths, done)), '.sep'],
      ['lib-bulk-tag', 'Tag…', () => tagsMod().then((m) => m.bulkTag(btn, paths, done))],
      app.device && ['lib-bulk-move', 'Move to folder…', () => bulk('bulkMove', paths)],
      app.device && ['lib-bulk-rename', 'Rename…', () => bulk('bulkRename', paths)],
      app.device && ['lib-bulk-del', 'Delete…', () => bulk('bulkDelete', paths), '.danger']], `${paths.length.toLocaleString()} GIFs`);
  }
  // Done, the selection clears; stopped halfway, what was not reached (the
  // GIF that failed and those after it) stays selected, to try again.
  async function bulk(fn, paths) {
    const shown = new Set(tree.res.map((x) => x.path));
    const r = await (await import('./details.js'))[fn](paths, { ...opts(), hidden: paths.filter((p) => !shown.has(p)).length });
    if (!r) return;
    // (Only what this batch was given: a GIF ticked while it ran stays.)
    const left = new Set(r.left);
    for (const p of paths) if (!left.has(p)) app.selection.delete(p);
    const { done } = r;
    if (!done.length) { sync(); return; }
    if (current && done.some((x) => x.from === current.path)) {
      current = null;
      tree.cur = '';
      body.hidden = true;
      side.classList.remove('open');
      pv.clear();
      cap.textContent = 'Pick a GIF to see it here';
    }
    await ov.load(ov.path);
    await tree.load();
    sync();
  }

  async function reveal(path) {
    search.value = '';
    apply();
    await tree.load();
    const it = await tree.show(path);
    if (it) preview(it);
  }

  const opts = () => {
    // The folder open (one), else the picked GIF's, else the first open.
    const open = [...tree.open], cur = current?.path.startsWith('/gifs/') && current.path.split('/')[2];
    const c = (open.length === 1 ? open[0] : cur) || open[0] || '';
    return { category: c, categories: tree.real(), platforms: [...new Set(tree.rows.filter((r) => r.path).map((r) => r.e.platform).filter(Boolean))] };
  };
  // Add GIFs: the files (GIFs, pictures), each that is not a 128 × 32 GIF
  // fitted to the panel first (js/fit.js), then their details (js/details.js).
  // One at a time: files dropped or pasted while it is open wait their turn.
  let adding = Promise.resolve();
  const addGifs = (picked, inits) => (adding = adding.then(() => addNow(picked, inits)).catch(() => {}));
  async function addNow(picked, inits = []) {
    const ok = picked.map((f, i) => [f, inits[i]]).filter(([f]) => /\.gif$/i.test(f.name) || f.type.startsWith('image/'));
    if (!ok.length) { toast('choose GIFs or pictures', 'bad'); return; }
    const fit = await import('./fit.js');
    const got = await fit.fitAll(ok.map((x) => x[0]), ok.map((x) => x[1]));
    if (!got.length) return;
    const plan = await (await import('./details.js')).addDialog(got.map((x) => x.file), { ...opts(), inits: got.map((x) => x.init) });
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
    const where = [...new Set(plan.map((p) => niceName(p.category)))].join(', ');
    addMsg.textContent = `${added.length} of ${plan.length} added to ${where}`;
    if (!addErr.firstChild) setTimeout(() => { addCard.hidden = true; }, 6000);
    if (added.length) {
      import('./smart.js').then((m) => m.refreshAll()).catch(() => {}); // smart playlists that now find them
      await reveal(added[0]);
      await offerPlaylist(added);
    }
  }

  async function edit() {
    const to = await (await import('./details.js')).editDialog(current.path, opts());
    if (to) { forgetThumb(current.path); forgetThumb(to); forgetDir(current.path.split('/')[2]); await ov.load(ov.path); await reveal(to); }
  }
  async function remove() {
    if (!(await (await import('./details.js')).removeGif(current.path, label(current.e)))) return;
    forgetThumb(current.path);
    forgetDir(current.path.split('/')[2]);
    current = null;
    tree.cur = '';
    body.hidden = true;
    side.classList.remove('open');
    pv.clear();
    cap.textContent = 'Pick a GIF to see it here';
    await ov.load(ov.path);
    await tree.load();
  }

  async function playlists() {
    const lists = await app.playlists().catch(() => []);
    if (!tg && app.setNames?.has('.tags')) tagsMod().then(() => tree.refresh()).catch(() => {});
    const has = (p) => p && lists.some((x) => x.path === p);
    const want = [ov.path, store.get('lib.ov'), app.activePath].find(has) || lists[0]?.path || '/filelist.txt';
    await ov.load(want); // each time: Now's ♥ or the Playlists editor may have changed it
    // (Favourites from before the tag, a weight alone: into the tag, js/tags.js adopt.)
    if ([...ov.map.values()].some((o) => o.w > 1)) tagsMod().then((m) => m.adopt(ov)).catch(() => {});
  }

  await tree.load();
  // (Not waited for: the rows' glyphs follow when the playlist's overrides come.)
  playlists();
  sync();
  const again = () => {
    if (tree.dirty) tree.load().then(() => { const it = current && tree.rows.find((x) => x.path === current.path); if (it) preview(it); });
    else tree.refresh(); // the thumbnails switch may have changed
  };
  // Drop or paste (on a device): picture and GIF files go to Add GIFs; an
  // address or text is not fetched here (the website does that): the
  // Find art online hint says so.
  const media = (fs) => [...(fs || [])].filter((f) => f.type.startsWith('image/') || /\.gif$/i.test(f.name));
  const notHere = () => {
    webMsg.textContent = 'Only files from this device are added here; for a picture on a web page, ';
    webGo.href = online();
    webHint.hidden = false;
    webHint.scrollIntoView?.({ block: 'nearest' });
  };
  const take = (dt) => {
    const fs = media(dt?.files);
    if (fs.length) addGifs(fs);
    else if (dt && /^(https?:|www\.)/i.test((dt.getData('text/uri-list') || dt.getData('text/plain') || '').trim())) notHere();
    else return false;
    return true;
  };
  const onPaste = (e) => {
    // (Text into a field stays text; an image, or an address pasted on the page, is ours.)
    if (media(e.clipboardData?.files).length || !/INPUT|TEXTAREA/.test(e.target.tagName)) { if (take(e.clipboardData)) e.preventDefault(); }
  };
  if (app.device) {
    // (Anywhere on the page while the Library shows, its sheets included: a
    // file dropped there would otherwise replace the page.)
    // (Once per page: a view mounted again lets the last one's go.)
    el._drop?.abort();
    const signal = (el._drop = new AbortController()).signal;
    const on = () => el.classList.contains('on'), over = (v) => grid.classList.toggle('drop', v);
    addEventListener('dragover', (e) => { if (on()) { e.preventDefault(); over(true); } }, { signal });
    addEventListener('dragleave', (e) => { if (!e.relatedTarget) over(false); }, { signal });
    addEventListener('drop', (e) => { if (!on()) return; e.preventDefault(); over(false); take(e.dataTransfer); }, { signal });
  }
  // Another card (swapped while the page was open): its folders, if shown.
  on('card', () => { if (el.classList.contains('on')) { playlists(); again(); } });
  return {
    show() {
      recheck();
      sync();
      playlists();
      again();
      // A GIF sent from the website (its Studio's Send to panel: app.js,
      // js/importgif.js) or a Formula's Save as GIF: Add GIFs, filled in.
      const gif = app.gifIn;
      app.gifIn = null;
      gif?.then(({ file, init }) => (app.device ? addGifs([file], [init]) : toast('Send it to your panel’s own address: this isn’t one', 'bad')),
        (e) => toast(`Couldn’t add that GIF: ${e.message || e}`, 'bad'));
      // A card swapped since the last look shows in the status: one request.
      if (app.device) { app.refreshStatus().catch(() => {}); addEventListener('paste', onPaste); }
    },
    hide() { removeEventListener('paste', onPaste); },
    // #library/<Category>: that folder open at the top, the others closed.
    async sub(sub) {
      if (!sub) return;
      if (search.value) { search.value = ''; apply(); }
      await tree.only(decodeURIComponent(sub));
    },
  };
}
