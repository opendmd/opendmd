// Library: browse the card by category, search titles and filter by
// platform with the core's tag parser, preview an entry, and select entries
// for a set or the converter. On a device, "Send to panel" plays the
// previewed entry now (POST /api/play), its .odm when there is one. Listings come from GET /api/files (one
// directory at a time); the .odm view comes from filelist.odm.txt, never
// from listing the bucket directories.

import { app } from './state.js';
import { PanelPreview } from './preview.js';
import { VList } from './vlist.js';
import { h, clear, on, emit, store, toast, label } from './util.js';

export async function mount(el) {
  const canvas = h('canvas.panel', { id: 'lib-panel' });
  const cap = h('div.cap', { id: 'lib-cap' }, 'click an entry to preview it');
  const kind = h('select', { id: 'lib-kind' }, h('option', { value: 'gifs' }, 'GIFs'), h('option', { value: 'odm' }, '.odm (filelist.odm.txt)'));
  const cat = h('select', { id: 'lib-cat' });
  const search = h('input', { type: 'search', id: 'lib-search', placeholder: 'search titles', enterKeyHint: 'search' });
  const chips = h('div.chips.scroll', { id: 'lib-chips' });
  const count = h('span.muted', { id: 'lib-count' });
  const selCount = h('b');
  const selbar = h('div.selbar.row.sp', { hidden: true, id: 'lib-selbar' },
    h('span', selCount, ' selected'),
    h('div.row',
      h('button', { id: 'lib-all', onclick: selectAll }, 'Select shown'),
      h('button.pri', { id: 'lib-to-set', onclick: toSet }, 'Add to set'),
      h('button', { id: 'lib-to-convert', onclick: toConvert }, 'Convert'),
      h('button', { onclick: () => { app.selection.clear(); sync(); } }, 'Clear')));

  let items = [];
  let shown = [];
  let current = '';
  let besideOdm = new Set();
  let sendPath = '';
  const sendBtn = app.device && h('button', { id: 'lib-play', disabled: true, onclick: () => app.play(sendPath) }, 'Send to panel');
  const platforms = new Set();

  const list = new VList({
    rowHeight: 40,
    render(row, it) {
      const box = h('input', { type: 'checkbox', checked: app.selection.has(it.path), onclick: (e) => { e.stopPropagation(); toggle(it.path, box.checked); } });
      clear(row, box,
        h('span.t', label(it.e), it.e.platform && h('span.tag', it.e.platform), ...it.e.flags.map((f) => h('span.tag', f)),
          it.odm && h('span.tag.odm', '.odm')),
        h('span.n.mono', it.name));
      row.classList.toggle('cur', it.path === current);
    },
    onClick: (it) => preview(it),
  });
  list.el.id = 'lib-list';

  el.append(h('div.libgrid',
    h('div',
      h('div.card',
        h('div.row', kind, cat, h('div.grow', { style: 'min-width:140px' }, search), count),
        h('div', { style: 'margin-top:8px' }, chips)),
      list.el, selbar),
    h('div.side', h('div.card', canvas, cap, sendBtn && h('div.row', { style: 'margin-top:8px' }, sendBtn)))));

  const pv = new PanelPreview(canvas, { onError: (m) => { cap.textContent = 'preview: ' + m; } });

  // '' = none; else where the .odm is: the path itself, the bucket, or beside.
  function odmOf(path) {
    if (/\.odm$/i.test(path)) return path;
    if (app.hasOdm(path)) return app.wasm.odmPathFor(path);
    return besideOdm.has(path) ? path.slice(0, -4) + '.odm' : '';
  }

  async function preview(it) {
    current = it.path;
    list.refresh();
    // A GIF with a known bucketed .odm previews from the .odm: the same
    // frames, and a cheap open on the device instead of a big-directory scan.
    const path = it.odm || it.path;
    if (sendBtn) { sendPath = path; sendBtn.disabled = false; sendBtn.title = path; }
    cap.textContent = `${label(it.e)} · loading ${path}`;
    try {
      if (await pv.show(path, (p) => app.src.readBytes(p))) cap.textContent = `${label(it.e)} · ${path}`;
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
    toast(`${app.selection.size} entries added to the set being edited`, 'ok');
    emit('draft');
    app.go('sets');
  }
  function toConvert() {
    app.pendingConvert.push(...[...app.selection].filter((p) => /\.gif$/i.test(p)));
    emit('convert-add');
    app.go('convert');
  }

  async function categories() {
    clear(cat);
    if (kind.value === 'gifs') {
      let dirs = [];
      try { dirs = (await app.src.list('/gifs')).filter((d) => d.dir).map((d) => d.name).sort(); } catch (e) { toast('listing /gifs: ' + e.message, 'bad'); }
      for (const d of dirs) cat.append(h('option', { value: d }, d));
    } else {
      const odm = await app.odmIndex();
      if (!odm.size) toast('no filelist.odm.txt on this card', 'bad');
      const cats = new Set([...odm].map((p) => app.wasm.parsePath(p).category || ''));
      for (const d of [...cats].sort()) cat.append(h('option', { value: d }, d || '(none)'));
    }
    const want = store.get('lib.cat', 'Consoles');
    if ([...cat.options].some((o) => o.value === want)) cat.value = want;
    await load();
  }

  async function load() {
    store.set('lib.cat', cat.value);
    count.textContent = 'loading…';
    await app.odmIndex();
    let paths;
    if (kind.value === 'gifs') {
      const dir = `/gifs/${cat.value}`;
      // A GIF's .odm may sit beside it (`opendmd-pack --layout beside`):
      // list the GIF once and mark it, rather than showing both.
      let files = [];
      try { files = (await app.src.list(dir)).filter((f) => !f.dir).map((f) => f.name); } catch (e) { toast(`listing ${dir}: ${e.message}`, 'bad'); }
      const beside = new Set(files.filter((n) => /\.odm$/i.test(n)).map((n) => n.slice(0, -4).toLowerCase()));
      paths = files.filter((n) => /\.gif$/i.test(n)).map((n) => `${dir}/${n}`);
      besideOdm = new Set(paths.filter((p) => beside.has(p.slice(p.lastIndexOf('/') + 1, -4).toLowerCase())));
    } else {
      paths = [...app.odm].filter((p) => (app.wasm.parsePath(p).category || '') === cat.value);
    }
    items = paths.map((path) => {
      const e = app.wasm.parsePath(path);
      return { path, e, name: path.slice(path.lastIndexOf('/') + 1), norm: app.wasm.normalizeTitle(e.title), odm: odmOf(path) };
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

  kind.onchange = categories;
  cat.onchange = load;
  search.oninput = apply;
  on('odm', () => { for (const it of items) it.odm = odmOf(it.path); list.refresh(); });

  await categories();
  sync();
  return { show() { sync(); } };
}

