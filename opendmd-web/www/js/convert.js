// Convert: GIF -> .odm in the browser with the core's encoder (the same one
// opendmd-pack runs, so the bytes are identical), uploaded to the bucketed
// layout /odm/<Category>/<bb>/<name>.odm one file at a time. One GIF and one
// .odm are in memory at once; the device sees one transfer at a time,
// because every transfer costs it heap while it is playing.

import { app } from './state.js';
import { VList } from './vlist.js';
import { h, clear, on, toast, bytes, base, dir } from './util.js';

export async function mount(el) {
  const queue = [];
  const localCat = h('select', { id: 'cv-cat' });
  const files = h('input', { type: 'file', multiple: true, accept: '.gif,image/gif', hidden: true, id: 'cv-files', onchange: () => { addFiles(files.files); files.value = ''; } });
  const skip = h('input', { type: 'checkbox', id: 'cv-skip', checked: true });
  const startBtn = h('button.pri', { id: 'cv-start', onclick: () => (running ? stop() : start()) }, 'Start');
  const bar = h('i');
  const prog = h('div.muted', { id: 'cv-progress' }, 'queue empty');
  const errors = h('div.log.bad', { id: 'cv-errors' });
  const manifestCard = h('div.card', { id: 'cv-manifest', hidden: true });
  let running = false;

  const list = new VList({
    rowHeight: 40,
    render(row, it) {
      clear(row, h('span.t.mono', { title: it.dest }, base(it.dest)),
        h('span.n.mono', dir(it.dest)),
        h('span.tag' + (it.state === 'done' ? '.odm' : ''), it.state),
        it.stats && h('span.muted', `${bytes(it.stats.gif_bytes)} → ${bytes(it.stats.odm_bytes)}`));
    },
  });
  list.el.id = 'cv-queue';
  list.el.style.height = '40vh';

  el.append(
    h('div.card', h('h2', 'Convert GIFs to .odm'),
      h('p.muted', 'An .odm plays with ~15 KB of heap instead of ~54 KB and opens from a ~15-file bucket instead of a 3,781-file directory. Converted here with the firmware’s own decoder; written to ',
        h('code', '/odm/<Category>/<bb>/<name>.odm'), '.'),
      h('div.row',
        h('button', { id: 'cv-add-sel', onclick: addSelection }, 'Add Library selection'),
        h('label.btn', 'Local GIFs…', files), h('label', 'category for local files', localCat),
        h('label', { title: 'Known from filelist.odm.txt; without one, the device is asked (GET /api/stat) about the one destination. '
          + 'A stat scans the parent directory — ~600 ms in gifs/Consoles, where the panel feels it — so only the bucketed .odm (a ~15-file directory, ~20 ms) is ever checked.' },
        skip, 'skip files that already have an .odm'))),
    h('div.card',
      h('div.row.sp', prog, h('div.row', startBtn, h('button', { onclick: clearDone }, 'Clear finished'))),
      h('div.bar', { style: 'margin:10px 0' }, bar), list.el, errors),
    manifestCard);

  try {
    const dirs = (await app.src.list('/gifs')).filter((d) => d.dir).map((d) => d.name).sort();
    for (const d of ['Unknown', ...dirs]) localCat.append(h('option', { value: d }, d));
  } catch { localCat.append(h('option', { value: 'Unknown' }, 'Unknown')); }

  function push(it) {
    it.dest = app.wasm.odmPathFor(it.path);
    if (!it.dest) return false;
    if (queue.some((q) => q.dest === it.dest)) return false;
    queue.push(Object.assign(it, { state: 'queued' }));
    return true;
  }
  function addPaths(paths) {
    const n = paths.filter((p) => push({ path: p })).length;
    toast(`${n} GIFs queued`);
    render();
  }
  function addSelection() {
    const sel = [...app.selection].filter((p) => /\.gif$/i.test(p));
    if (!sel.length) { toast('select GIFs in the Library first'); return; }
    addPaths(sel);
  }
  function addFiles(list_) {
    const n = [...list_].filter((f) => push({ path: `/gifs/${localCat.value}/${f.name}`, file: f })).length;
    toast(`${n} local GIFs queued`);
    render();
  }

  function render() {
    list.set(queue);
    const c = count();
    prog.textContent = queue.length ? `${c.done} converted · ${c.skip} skipped · ${c.error} failed · ${c.queued} queued` + (stats.t ? ` · ${rate()}` : '') : 'queue empty';
    bar.style.width = queue.length ? `${(100 * (queue.length - c.queued)) / queue.length}%` : '0';
    startBtn.textContent = running ? 'Stop after this file' : 'Start';
    startBtn.disabled = !running && !c.queued;
  }
  function count() {
    const c = { done: 0, skip: 0, error: 0, queued: 0 };
    for (const q of queue) c[q.state === 'skipped' ? 'skip' : q.state === 'done' ? 'done' : q.state === 'error' ? 'error' : 'queued']++;
    return c;
  }
  const stats = { t: 0, bytes: 0, files: 0 };
  function rate() {
    const s = (performance.now() - stats.t) / 1000;
    return `${bytes(stats.bytes / s)}/s up · ${((stats.files * 60) / s).toFixed(1)} files/min`;
  }
  function clearDone() {
    for (let i = queue.length - 1; i >= 0; i--) if (queue[i].state !== 'queued') queue.splice(i, 1);
    errors.replaceChildren();
    render();
  }

  // Does the bucket already hold this .odm? filelist.odm.txt answers it
  // when the card has one; otherwise one GET /api/stat of the destination.
  // A stat is a scan of the parent directory, cheap only because `dest` is
  // always bucketed (~15 entries): never stat anything in gifs/<Category>.
  async function exists(dest) {
    await app.odmIndex();
    if (app.odm.has(dest)) return true;
    if (app.odmSource !== 'none') return false;
    return (await app.src.stat(dest)).exists;
  }

  function stop() { running = false; render(); }
  async function start() {
    running = true;
    stats.t = performance.now(); stats.bytes = 0; stats.files = 0;
    const done = [];
    render();
    for (const it of queue) {
      if (!running) break;
      if (it.state !== 'queued') continue;
      try {
        if (skip.checked && await exists(it.dest)) { it.state = 'skipped'; render(); continue; }
        it.state = 'reading'; list.refresh();
        let gif = it.file ? new Uint8Array(await it.file.arrayBuffer()) : await app.src.readBytes(it.path);
        it.state = 'encoding'; list.refresh();
        const { odm, stats: st } = app.wasm.encodeGif(gif);
        gif = null;
        it.stats = st;
        it.state = 'uploading'; list.refresh();
        await app.src.writeFile(it.dest, odm);
        stats.bytes += odm.length; stats.files++;
        it.state = 'done';
        app.addOdm(it.dest);
        done.push(it.dest);
      } catch (e) {
        it.state = 'error';
        errors.append(h('div', `${it.path}: ${e.message || e}`));
      }
      render();
    }
    running = false;
    render();
    if (done.length) offerManifest(done.length);
  }

  // filelist.odm.txt = filelist.txt with every line whose .odm is known to
  // exist switched to it (opendmd-pack manifest --layout bucketed). The
  // device treats a PUT onto /filelist.* as a manifest install (.idx first).
  function offerManifest(n) {
    manifestCard.hidden = false;
    clear(manifestCard, h('h2', 'filelist.odm.txt'),
      h('p.muted', `${n} new .odm files. Regenerate /filelist.odm.txt from /filelist.txt so the new files are used by anything built from it.`,
        app.odmSource === 'none' ? h('b.bad', ' This source did not serve filelist.odm.txt, so only .odm files converted in this session are known: older ones would be left pointing at their GIFs.') : ''),
      h('button.pri', { id: 'cv-manifest-write', onclick: writeManifest }, 'Update filelist.odm.txt'));
  }
  async function writeManifest() {
    try {
      const text = await app.text('/filelist.txt', true);
      if (text == null) throw new Error('/filelist.txt is not served by this source');
      const r = app.wasm.odmManifest(text, [...app.odm]);
      if (!confirm(`Write /filelist.odm.txt: ${r.switched.toLocaleString()} lines point at .odm files?`)) return;
      await app.src.writeFile('/filelist.odm.txt', r.text);
      app.texts.set('/filelist.odm.txt', r.text);
      toast(`wrote /filelist.odm.txt (${r.switched} .odm lines)`, 'ok');
    } catch (e) { toast('filelist.odm.txt: ' + e.message, 'bad'); }
  }

  const drain = () => { const p = app.pendingConvert.splice(0); if (p.length) addPaths(p); };
  on('convert-add', drain);
  drain();
  render();
  return { show() { render(); } };
}
