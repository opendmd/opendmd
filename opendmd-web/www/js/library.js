// Library: browse the card's GIFs by category, search titles and filter by
// platform with the core's tag parser, preview an entry (from its prepared
// copy when there is one — the same frames, a cheaper fetch), and select
// entries for a playlist. On a device, "Send to panel" plays the previewed
// GIF now (POST /api/play; the device finds the prepared copy itself), and
// "Add GIFs" uploads files into the category shown, prepares each one in the
// browser, and offers to add them to the playlist in play. Listings come
// from GET /api/files, one directory at a time.

import { app, playlistLabel } from './state.js';
import { PanelPreview } from './preview.js';
import { prepareBytes } from './prepare.js';
import { VList } from './vlist.js';
import { h, clear, emit, store, toast, label, lines } from './util.js';

export async function mount(el) {
  const canvas = h('canvas.panel', { id: 'lib-panel' });
  const cap = h('div.cap', { id: 'lib-cap' }, 'click an entry to preview it');
  const cat = h('select', { id: 'lib-cat' });
  const search = h('input', { type: 'search', id: 'lib-search', placeholder: 'search titles', enterKeyHint: 'search' });
  const chips = h('div.chips.scroll', { id: 'lib-chips' });
  const count = h('span.muted', { id: 'lib-count' });
  const selCount = h('b');
  const selbar = h('div.selbar.row.sp', { hidden: true, id: 'lib-selbar' },
    h('span', selCount, ' selected'),
    h('div.row',
      h('button', { id: 'lib-all', onclick: selectAll }, 'Select shown'),
      h('button.pri', { id: 'lib-to-set', onclick: toSet }, 'Add to playlist'),
      h('button', { onclick: () => { app.selection.clear(); sync(); } }, 'Clear')));

  let items = [];
  let shown = [];
  let current = '';
  let sendPath = '';
  const sendBtn = app.device && h('button', { id: 'lib-play', disabled: true, onclick: () => app.play(sendPath) }, 'Send to panel');
  const platforms = new Set();

  // --- Add GIFs (device only)
  const addInput = h('input', { type: 'file', multiple: true, accept: '.gif,image/gif', hidden: true, id: 'lib-add-files', onchange: () => { addGifs([...addInput.files]); addInput.value = ''; } });
  const addBar = h('i');
  const addMsg = h('div.muted', { id: 'lib-add-progress' });
  const addErr = h('div.log.bad', { id: 'lib-add-errors' });
  const addCard = h('div.card', { id: 'lib-add', hidden: true }, h('div.bar', { style: 'margin-bottom:8px' }, addBar), addMsg, addErr);
  const addBtn = app.device && h('label.btn', { id: 'lib-add-btn', title: 'Upload GIFs into the category shown' }, 'Add GIFs…', addInput);

  const list = new VList({
    rowHeight: 40,
    render(row, it) {
      const box = h('input', { type: 'checkbox', checked: app.selection.has(it.path), onclick: (e) => { e.stopPropagation(); toggle(it.path, box.checked); } });
      clear(row, box,
        h('span.t', label(it.e), it.e.platform && h('span.tag', it.e.platform), ...it.e.flags.map((f) => h('span.tag', f))),
        h('span.n.mono', it.name));
      row.classList.toggle('cur', it.path === current);
    },
    onClick: (it) => preview(it),
  });
  list.el.id = 'lib-list';

  el.append(h('div.libgrid',
    h('div',
      h('div.card',
        h('div.row', cat, h('div.grow', { style: 'min-width:140px' }, search), count, addBtn),
        h('div', { style: 'margin-top:8px' }, chips)),
      addCard, list.el, selbar),
    h('div.side', h('div.card', canvas, cap, sendBtn && h('div.row', { style: 'margin-top:8px' }, sendBtn)))));

  const pv = new PanelPreview(canvas, { onError: (m) => { cap.textContent = 'preview: ' + m; } });

  async function preview(it) {
    current = it.path;
    list.refresh();
    if (sendBtn) { sendPath = it.path; sendBtn.disabled = false; sendBtn.title = it.path; }
    cap.textContent = `${label(it.e)} · loading…`;
    try {
      if (await pv.show(it.path, (p) => app.previewBytes(p))) cap.textContent = `${label(it.e)} · ${it.name}`;
    } catch (e) { cap.textContent = 'preview: ' + e.message; }
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

  async function categories() {
    clear(cat);
    let dirs = [];
    try { dirs = (await app.src.list('/gifs')).filter((d) => d.dir).map((d) => d.name).sort(); } catch (e) { toast('listing /gifs: ' + e.message, 'bad'); }
    for (const d of dirs) cat.append(h('option', { value: d }, d));
    const want = store.get('lib.cat', 'Consoles');
    if ([...cat.options].some((o) => o.value === want)) cat.value = want;
    await load();
  }

  async function load() {
    store.set('lib.cat', cat.value);
    count.textContent = 'loading…';
    const dir = `/gifs/${cat.value}`;
    let files = [];
    try { files = (await app.src.list(dir)).filter((f) => !f.dir).map((f) => f.name); } catch (e) { toast(`listing ${dir}: ${e.message}`, 'bad'); }
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
    clear(chips, top.map(([p, n]) => h('button.chip' + (platforms.has(p) ? '.on' : ''), {
      'data-platform': p,
      onclick: () => { if (!platforms.delete(p)) platforms.add(p); renderChips(); apply(); },
    }, p, h('small', n))));
  }

  function apply() {
    const q = app.wasm.normalizeTitle(search.value);
    const qn = search.value.trim().toLowerCase();
    shown = items.filter((it) => (!platforms.size || platforms.has(it.e.platform))
      && (!q || it.norm.includes(q) || (qn && it.name.toLowerCase().includes(qn))));
    count.textContent = `${shown.length.toLocaleString()} of ${items.length.toLocaleString()}`;
    list.set(shown);
  }

  // Upload each GIF into the category shown and prepare it straight away:
  // the GIF (PUT /api/files/gifs/…), then its prepared copy, encoded here.
  // One file at a time; a GIF that will not encode is still added (it plays
  // as it is) and any stale copy of an older file by that name is forgotten.
  async function addGifs(picked) {
    const gifs = picked.filter((f) => /\.gif$/i.test(f.name));
    if (!gifs.length) { toast('choose .gif files', 'bad'); return; }
    const category = cat.value;
    if (!category) { toast('pick a category first', 'bad'); return; }
    addCard.hidden = false;
    addErr.replaceChildren();
    const added = [];
    for (let i = 0; i < gifs.length; i++) {
      const f = gifs[i], path = `/gifs/${category}/${f.name}`;
      addMsg.textContent = `Adding ${i + 1} of ${gifs.length} to ${category} · ${f.name}`;
      addBar.style.width = `${(100 * i) / gifs.length}%`;
      try {
        const bytes = new Uint8Array(await f.arrayBuffer());
        await app.src.writeFile(path, bytes);
        added.push(path);
        try { await prepareBytes(path, bytes); } catch (e) {
          addErr.append(h('div', `${f.name}: added, but it could not be prepared (${e.message || e}); it will play as it is`));
          try { await app.src.forget(path); } catch { /* nothing to forget */ }
        }
      } catch (e) { addErr.append(h('div', `${f.name}: ${e.message || e}`)); }
    }
    addBar.style.width = '100%';
    addMsg.textContent = `${added.length} of ${gifs.length} added to ${category}.`;
    if (added.length) {
      await load();
      await offerPlaylist(added);
    }
  }

  // Add the new GIFs to the playlist in play, if the user wants them there.
  async function offerPlaylist(paths) {
    const path = app.activePath;
    const name = playlistLabel(path);
    if (!confirm(`Add ${paths.length} new GIF${paths.length > 1 ? 's' : ''} to the playlist “${name}”?`)) return;
    try {
      const text = await app.text(path, true);
      const have = new Set(lines(text || ''));
      const add = paths.filter((p) => !have.has(p));
      if (!add.length) { toast(`already in “${name}”`); return; }
      const next = (text && !text.endsWith('\n') ? text + '\n' : text || '') + add.join('\n') + '\n';
      if (path.startsWith('/sets/')) await app.src.saveSet(path.slice(6).replace(/\.txt$/, ''), next);
      else await app.src.writeFile(path, next);
      app.texts.set(path, next);
      await app.refreshStatus().catch(() => {});
      toast(`${add.length} added to “${name}”`, 'ok');
    } catch (e) { toast(`adding to “${name}”: ${e.message}`, 'bad'); }
  }

  cat.onchange = load;
  search.oninput = apply;

  await categories();
  sync();
  return { show() { sync(); } };
}
