// The Studio: "Make art" in Library. A picture in (a file, a drop, a paste,
// an address, or "Find art for a game"), a 128x32 GIF out, in five steps on
// one sheet: Import, Frame, Look, Preview, Save.
//
// Loaded on first use, with its own wasm module (pkg-art/: the pixel work
// and the GIF writer, opendmd-art-web), so the app everyone loads does not
// grow. The picture is decoded by the browser into RGBA and handed to a
// wasm `Studio`, which keeps it; every change of a control re-renders the
// GIF there and the preview plays it. Save goes through Library's Add GIFs
// path (the GIF into a category, its prepared copy beside it) — or, with no
// device, downloads the GIF. The file name is built from the tag grammar
// and read back with the core's parser, so what the Library will list is
// what the user sees before saving.
//
// "Find art for a game" searches a name index per platform on the card
// (art/<PLATFORM>.json, from tools/art-index.py) and loads the pictures from
// the libretro-thumbnails repositories on raw.githubusercontent.com, which
// lets pages read them. Nothing here says where anything is stored.

import { app } from './state.js';
import { PanelPreview } from './preview.js';
import { h, clear, toast, debounce } from './util.js';
import { art as artModule } from './artmod.js';

const PLATFORMS = ['MD', 'SNES', 'GB', 'GBC', 'GBA', 'SATURN', 'NDS', 'NES', 'SMS', 'PCE', 'PS1', 'N64', 'ARCADE'];
const RAW = 'https://raw.githubusercontent.com/libretro-thumbnails/';
const KINDS = [[1, 'Named_Titles', 'Title screen'], [2, 'Named_Snaps', 'In the game'], [4, 'Named_Boxarts', 'Box'], [8, 'Named_Logos', 'Logo']];
const SPEEDS = [['Slow', 50], ['Normal', 40], ['Fast', 30]];
const HOLDS = [['No pause', 0], ['Pause 1.5 s', 1500], ['Pause 3 s', 3000]];
const MAX_PX = 4e6; // bigger pictures are scaled down on the way in

let art = null; // the wasm module
let sheet = null; // the one Studio, built on first open

// Open the Studio. `o`: {entry?, category, categories, names(category) ->
// file names, save(category, name, bytes) -> path, saved(category, paths)}.
export async function open(o) {
  if (!art) {
    [art] = await Promise.all([artModule(), css()]);
  }
  sheet ??= build();
  sheet.open(o);
  return sheet;
}

function css() {
  return new Promise((res, rej) => {
    const l = h('link', { rel: 'stylesheet', href: './studio.css', onload: res, onerror: () => rej(new Error('could not load the Studio')) });
    document.head.append(l);
  });
}

// Fetch a picture from libretro-thumbnails. Some entries there are links:
// the file then holds the name of the real one (a few dozen bytes of text).
async function thumb(url, hops = 3) {
  const r = await fetch(url);
  if (!r.ok) throw new Error(r.status);
  const b = await r.blob();
  if (b.size < 512 && hops) {
    const head = new Uint8Array(await b.slice(0, 1).arrayBuffer());
    if (head[0] !== 0x89) return thumb(new URL((await b.text()).trim().split('/').map(encodeURIComponent).join('/'), url).href, hops - 1);
  }
  return b;
}

// Lowercase joining words stay lowercase in a name; everything else starts
// with a capital, as the pack's names do.
const SMALL = new Set(['of', 'the', 'a', 'an', 'and', 'in', 'on', 'to', 'vs', 'de', 'no']);
const words = (t) => t.normalize('NFKD').replace(/[\u0300-\u036f]/g, '').replace(/[^A-Za-z0-9]+/g, ' ').trim().split(' ').filter(Boolean)
  .map((w, i) => (i && SMALL.has(w.toLowerCase()) ? w.toLowerCase() : w[0].toUpperCase() + w.slice(1)));

// PLATFORM[_LOGO]_Title_Words_NN.gif — the variant last, so a title ending
// in a number ("Streets of Rage 2") stays a title.
export function fileName(platform, title, logo, n) {
  const w = words(title);
  return [platform, logo && 'LOGO', ...(w.length ? w : ['Untitled']), String(n).padStart(2, '0')].filter(Boolean).join('_') + '.gif';
}

function build() {
  const S = { studio: null, fit: 'pixels', motion: 'still', x: 0, y: 0, gif: null, opts: null, names: new Map(), index: new Map(), urls: [] };
  const id = (x) => 'st-' + x;
  const seg = (name, items, onpick) => {
    const el = h('div.st-seg', { id: id(name), role: 'group' }, items.map(([v, t]) => h('button', { 'data-v': v, onclick: () => onpick(v) }, t)));
    el.set = (v) => { for (const b of el.children) b.classList.toggle('on', b.dataset.v === String(v)); };
    return el;
  };
  const select = (name, items, value) => h('select', { id: id(name) }, items.map(([t, v]) => h('option', { value: v, selected: v === value }, t)));

  // --- 1 Import
  const file = h('input', { type: 'file', accept: 'image/*', hidden: true, id: id('file'), onchange: () => { if (file.files[0]) load(file.files[0]); file.value = ''; } });
  const url = h('input', { type: 'text', id: id('url'), inputMode: 'url', placeholder: 'Picture address (https://…)' });
  const msg = h('div.cap', { id: id('msg'), 'aria-live': 'polite' });
  const plat = h('select', { id: id('find-plat') });
  const query = h('input', { type: 'search', id: id('find-q'), placeholder: 'Game title', enterKeyHint: 'search', onkeydown: (e) => e.key === 'Enter' && find() });
  const hits = h('div.st-hits', { id: id('hits') });
  const thumbs = h('div.st-thumbs', { id: id('thumbs') });
  const imp = h('section.card', { id: id('import') }, h('h3', '1 · Import'),
    h('div.row', h('label.btn', 'Choose a picture…', file), h('span.muted', 'or drop or paste one here')),
    h('div.row.st-url', h('div.grow', url), h('button', { id: id('url-go'), onclick: () => fromUrl(url.value.trim()) }, 'Load')),
    h('div.st-find', h('b', 'Find art for a game'),
      h('div.row', plat, h('div.grow', query), h('button', { id: id('find-go'), onclick: () => find() }, 'Search')), hits, thumbs),
    msg);

  // --- 2 Frame
  const cv = h('canvas', { id: id('src') });
  const win = h('div.st-win', { id: id('win') });
  const band = h('div.st-band');
  const view = h('div.st-view', { id: id('view') }, cv, band, win);
  const fit = seg('fit', [['pixels', 'Pixels'], ['half', 'Pixels ½'], ['fit', 'Fit']], (v) => { S.fit = v; restage(true); });
  const motion = seg('motion', [['still', 'Still'], ['pan', 'Pan →'], ['scroll', 'Scroll ↓']], (v) => {
    S.motion = v;
    if (!S.moved) [S.x, S.y] = S.studio.autoWindow(v);
    frame(); render();
  });
  const speed = select('speed', SPEEDS, 40);
  const hold = select('hold', HOLDS, 1500);
  const fcap = h('div.cap', { id: id('fcap') });
  const frm = h('section.card.st-step', { id: id('frame') }, h('h3', '2 · Frame'), view, fcap,
    h('div.row', fit, h('button.sm', { id: id('auto'), onclick: () => restage(true) }, 'Auto')),
    h('div.row', motion, speed, hold));

  // --- 3 Look
  const colours = select('colours', [['Colours: auto', 0], ['256 colours', 256], ['64 colours', 64], ['16 colours', 16]], 0);
  const key = h('input', { type: 'checkbox', id: id('key') });
  const lift = h('input', { type: 'checkbox', id: id('lift') });
  const gain = h('input', { type: 'range', id: id('gain'), min: 30, max: 150, step: 5, value: 100 });
  const gout = h('output.mono', '100%');
  const look = h('section.card.st-step', { id: id('look') }, h('h3', '3 · Look'),
    h('div.row', colours, h('label', key, 'Background to black')),
    h('div.row', h('label', lift, 'Brighten dark outlines (logos)')),
    h('div.row', h('label', 'Brightness', gain, gout)));

  // --- 4 Preview
  const panel = h('canvas.panel', { id: id('panel') });
  const stats = h('div.cap', { id: id('stats') });
  const prev = h('section.card.st-step', { id: id('preview') }, h('h3', '4 · Preview'), panel, stats);

  // --- 5 Save
  const splat = h('select', { id: id('plat') });
  const title = h('input', { type: 'text', id: id('title'), placeholder: 'Title' });
  const logo = h('input', { type: 'checkbox', id: id('logo') });
  const nn = h('input', { type: 'number', id: id('nn'), min: 1, max: 99, value: 1, style: 'width:72px' });
  const cat = h('select', { id: id('cat') });
  const name = h('div.cap', { id: id('name') });
  const warn = h('div.cap.bad', { id: id('warn') });
  const saveBtn = h('button.pri', { id: id('save'), onclick: () => save() }, 'Save');
  const sv = h('section.card.st-step', { id: id('save-step') }, h('h3', '5 · Save'),
    h('div.form',
      h('label.stack', 'Platform', splat), h('label.stack', 'Title', title),
      h('label.stack', 'Number', nn), h('label.stack', { id: id('cat-l') }, 'Category', cat)),
    h('div.row', { style: 'margin-top:8px' }, h('label', logo, 'It is a logo')),
    name, warn, h('div.row', { style: 'margin-top:8px' }, saveBtn));

  const close = h('button', { id: id('close'), 'aria-label': 'Close', onclick: () => hide() }, '✕');
  const box = h('div.st-box', h('div.row.sp', h('h2', 'Make art'), close), h('div.st-grid', imp, frm, look, prev, sv));
  const el = h('div.st-sheet', { id: 'studio', hidden: true }, box);
  document.body.append(el);

  for (const p of PLATFORMS) {
    plat.append(h('option', { value: p }, art.consoleName(p) || p));
    splat.append(h('option', { value: p }, p));
  }
  let pv = null;

  // --- Loading a picture
  async function load(blob, from = {}) {
    msg.textContent = 'Reading the picture…';
    let bm;
    try { bm = await createImageBitmap(blob, { colorSpaceConversion: 'none', premultiplyAlpha: 'none' }); } catch {
      msg.textContent = 'That is not a picture this browser can read.';
      return false;
    }
    const { width: ow, height: oh } = bm;
    let [w, hh] = [ow, oh];
    const k = Math.min(1, Math.sqrt(MAX_PX / (w * hh)));
    w = Math.max(1, Math.round(w * k)); hh = Math.max(1, Math.round(hh * k));
    const c = h('canvas', { width: w, height: hh });
    const g = c.getContext('2d', { willReadFrequently: true });
    g.imageSmoothingQuality = 'high';
    g.drawImage(bm, 0, 0, w, hh);
    bm.close?.();
    const data = g.getImageData(0, 0, w, hh).data;
    S.studio?.free();
    S.studio = new art.Studio(data, w, hh);
    S.fit = S.studio.autoFit();
    lift.checked = S.fit === 'fit';
    logo.checked = !!from.logo;
    if (from.platform) setPlatform(from.platform);
    if (from.title) title.value = from.title;
    msg.textContent = `${from.label || 'Picture'}: ${ow} × ${oh}`;
    el.classList.add('has');
    restage(true);
    await names();
    return true;
  }

  async function fromUrl(u) {
    if (!u) return;
    msg.textContent = 'Loading…';
    let r;
    try { r = await fetch(u); } catch {
      msg.textContent = 'That site doesn’t allow it — save the image and drop it here.';
      return;
    }
    if (!r.ok) { msg.textContent = `That address did not give a picture (${r.status}).`; return; }
    await load(await r.blob(), { label: 'From the address' });
  }

  // Build the stage for the fit; `auto` also re-picks window and motion.
  function restage(auto) {
    if (!S.studio) return;
    S.studio.prepare(S.fit, lift.checked);
    if (auto) {
      S.motion = S.studio.autoMotion();
      [S.x, S.y] = S.studio.autoWindow(S.motion);
      S.moved = false;
    }
    const st = S.studio;
    cv.width = st.width; cv.height = st.height;
    cv.getContext('2d').putImageData(new ImageData(new Uint8ClampedArray(st.pixels().buffer), st.width, st.height), 0, 0);
    const k = st.scale;
    fcap.textContent = `${st.width} × ${st.height}` + (S.fit === 'fit' ? ' · fitted to the panel' : k > 1 ? ` · the picture was enlarged ${k}×; using its real pixels` : '');
    fit.set(S.fit);
    frame();
    render();
  }

  // Draw the window (and, when it moves, where it travels).
  function frame() {
    const st = S.studio;
    if (!st) return;
    const W = st.width, H = st.height;
    S.x = Math.max(0, Math.min(S.x, W - 128)); S.y = Math.max(0, Math.min(S.y, H - 32));
    const pct = (v, of) => (100 * v) / of + '%';
    const box = (e, x, y, w, hh) => Object.assign(e.style, { left: pct(x, W), top: pct(y, H), width: pct(w, W), height: pct(hh, H) });
    // Whole-number zoom when there is room for 2x or more, else the width.
    const room = frm.clientWidth - 30 || 600, z = Math.floor(room / W);
    view.style.width = (z >= 2 ? W * z : room) + 'px';
    box(win, S.motion === 'pan' ? 0 : S.x, S.motion === 'scroll' ? 0 : S.y, 128, 32);
    band.hidden = S.motion === 'still';
    if (S.motion === 'pan') box(band, 0, S.y, W, 32);
    else if (S.motion === 'scroll') box(band, S.x, 0, 128, H);
    motion.set(S.motion);
    Object.assign(el.dataset, { x: S.x, y: S.y, fit: S.fit, motion: S.motion, stage: `${W}x${H}`, scale: st.scale });
  }

  function render() {
    if (!S.studio) return;
    const st = S.studio;
    S.gif = st.render(S.motion, S.x, S.y, +speed.value, +hold.value, key.checked, +gain.value, +colours.value);
    const secs = st.durationMs / 1000;
    stats.textContent = `${st.frames} frame${st.frames > 1 ? 's' : ''} · ${secs < 10 ? secs.toFixed(1) : Math.round(secs)} s · ${st.colours} colour${st.colours > 1 ? 's' : ''}`;
    el.dataset.frames = st.frames;
    pv?.show(fileNameNow(), async () => S.gif);
    naming();
  }
  const later = debounce(render, 120);

  // Dragging the window; a tap elsewhere on the picture moves it there.
  let drag = null;
  const toStage = (e) => { const r = cv.getBoundingClientRect(); return [(e.clientX - r.left) * (S.studio.width / r.width), (e.clientY - r.top) * (S.studio.height / r.height)]; };
  win.addEventListener('pointerdown', (e) => { e.preventDefault(); e.stopPropagation(); win.setPointerCapture(e.pointerId); drag = { p: toStage(e), x: S.x, y: S.y }; });
  win.addEventListener('pointermove', (e) => {
    if (!drag) return;
    const [sx, sy] = toStage(e);
    S.x = Math.round(drag.x + sx - drag.p[0]); S.y = Math.round(drag.y + sy - drag.p[1]);
    S.moved = true;
    frame();
  });
  win.addEventListener('pointerup', () => { if (drag) { drag = null; render(); } });
  view.addEventListener('click', (e) => {
    if (!S.studio || e.target === win) return;
    const [sx, sy] = toStage(e);
    S.x = Math.round(sx - 64); S.y = Math.round(sy - 16);
    S.moved = true;
    frame(); render();
  });

  for (const c of [speed, hold, colours, key]) c.onchange = render;
  lift.onchange = () => restage(false);
  gain.oninput = () => { gout.textContent = gain.value + '%'; later(); };

  // --- Find art for a game
  async function index(p) {
    if (!S.index.has(p)) {
      const r = await fetch(`./art/${p}.json`);
      if (!r.ok) throw new Error(r.status);
      const d = await r.json();
      d.norm = d.names.map((n) => app.wasm.normalizeTitle(art.cleanGameName(n)));
      S.index.set(p, d);
    }
    return S.index.get(p);
  }

  async function find() {
    const p = plat.value, q = query.value.trim();
    clear(thumbs);
    if (!q) return;
    clear(hits, h('span.muted', 'Searching…'));
    let d;
    try { d = await index(p); } catch { clear(hits, h('span.bad', 'The game list could not be loaded.')); return; }
    const qn = app.wasm.normalizeTitle(q);
    const qw = q.toLowerCase().split(/[^\p{L}\p{N}]+/u).filter(Boolean).map((w) => app.wasm.normalizeTitle(w));
    const found = [];
    d.norm.forEach((n, i) => {
      const s = n === qn ? 0 : n.startsWith(qn) ? 1 : n.includes(qn) ? 2 : qw.every((w) => n.includes(w)) ? 3 : -1;
      if (s >= 0) found.push([s, /\((USA|World)/.test(d.names[i]) ? 0 : /\(Europe/.test(d.names[i]) ? 1 : 2, d.names[i].length, i]);
    });
    found.sort((a, b) => a[0] - b[0] || a[1] - b[1] || a[2] - b[2]);
    if (!found.length) { clear(hits, h('span.muted', `No ${art.consoleName(p) || p} game matches “${q}”.`)); return; }
    clear(hits, found.slice(0, 24).map(([, , , i]) => {
      const region = d.names[i].match(/\(([^)]*)\)/)?.[1] || '';
      return h('button.st-hit', { 'data-i': i, onclick: (e) => pick(p, d, i, e.currentTarget) }, art.cleanGameName(d.names[i]), region && h('small', region));
    }));
  }

  async function pick(p, d, i, btn) {
    for (const b of hits.children) b.classList?.toggle('on', b === btn);
    for (const u of S.urls) URL.revokeObjectURL(u);
    S.urls = [];
    const kinds = parseInt(d.kinds[i], 16), game = art.cleanGameName(d.names[i]);
    clear(thumbs, KINDS.filter(([bit]) => kinds & bit).map(([bit, dir, label]) => {
      const img = h('img', { alt: label });
      const tile = h('button.st-thumb', { 'data-kind': dir, disabled: true, title: label }, img, h('small', label));
      thumb(`${RAW}${d.repo}/master/${dir}/${encodeURIComponent(d.names[i])}.png`).then((b) => {
        img.src = URL.createObjectURL(b);
        S.urls.push(img.src);
        tile.disabled = false;
        tile.onclick = () => load(b, { platform: p, title: game, logo: bit === 8, label: `${game} · ${label}` });
      }).catch(() => tile.remove());
      return tile;
    }));
  }

  // --- Save
  function setPlatform(p) {
    if (![...splat.options].some((o) => o.value === p)) splat.append(h('option', { value: p }, p));
    splat.value = p;
  }
  const fileNameNow = () => fileName(splat.value, title.value, logo.checked, +nn.value || 1);

  // The next free number for this platform and title in the category.
  async function names() {
    const c = cat.value || 'Consoles';
    if (!S.names.has(c)) S.names.set(c, new Set((await S.opts.names(c).catch(() => [])).map((n) => n.toLowerCase())));
    const have = S.names.get(c);
    let n = 1;
    while (have.has(fileName(splat.value, title.value, logo.checked, n).toLowerCase()) && n < 99) n++;
    nn.value = n;
    naming();
  }
  const renumber = debounce(names, 250);

  // The name, and what the Library will list it as (read back with the
  // core's parser): a word the grammar takes for a tag shows up here.
  function naming() {
    const f = fileNameNow(), c = cat.value || 'Consoles';
    const e = app.wasm.parsePath(`/gifs/${c}/${f}`);
    const shown = `${e.title}${e.variant != null ? ' ' + String(e.variant).padStart(2, '0') : ''}`;
    clear(name, 'File ', h('b.mono', f), app.device ? ` in ${c}` : '', ` · listed as “${shown}”`, e.flags.length ? ` (${e.flags.join(', ').toLowerCase()})` : '');
    const typed = words(title.value).join(' ');
    warn.textContent = typed && app.wasm.normalizeTitle(e.title) !== app.wasm.normalizeTitle(typed)
      ? `The Library will read the title as “${e.title}”: a word in it is taken for a tag. Try another wording.` : '';
    Object.assign(el.dataset, { name: f, title: e.title, platform: e.platform || '' });
  }
  for (const c of [splat, title, logo, cat]) c.addEventListener(c === title ? 'input' : 'change', () => { naming(); renumber(); });
  nn.oninput = naming;

  async function save() {
    if (!S.gif) return;
    const f = fileNameNow(), c = cat.value;
    if (!app.device) {
      const a = h('a', { href: URL.createObjectURL(new Blob([S.gif], { type: 'image/gif' })), download: f });
      a.click();
      setTimeout(() => URL.revokeObjectURL(a.href), 5000);
      toast(`downloaded ${f}`, 'ok');
      return;
    }
    if (S.names.get(c)?.has(f.toLowerCase()) && !confirm(`${f} is already in ${c}. Replace it?`)) return;
    saveBtn.disabled = true;
    try {
      const problems = [];
      const path = await S.opts.save(c, f, S.gif, (m) => problems.push(m));
      S.names.get(c)?.add(f.toLowerCase());
      el.dataset.saved = path;
      toast(`${f} saved to ${c}`, 'ok');
      for (const m of problems) toast(m, 'bad');
      await S.opts.saved(c, [path]);
      await names();
    } catch (e) { toast(`saving ${f}: ${e.message || e}`, 'bad'); } finally { saveBtn.disabled = false; }
  }

  // --- Drop and paste, while open
  const onPaste = (e) => {
    const f = [...(e.clipboardData?.files || [])].find((x) => x.type.startsWith('image/'));
    if (f) { e.preventDefault(); load(f, { label: 'Pasted' }); return; }
    const t = e.clipboardData?.getData('text/plain')?.trim();
    if (t && /^https?:\/\//.test(t) && e.target === document.body) { url.value = t; fromUrl(t); }
  };
  el.addEventListener('dragover', (e) => { e.preventDefault(); el.classList.add('drop'); });
  el.addEventListener('dragleave', (e) => { if (e.target === el) el.classList.remove('drop'); });
  el.addEventListener('drop', (e) => {
    e.preventDefault();
    el.classList.remove('drop');
    const f = [...e.dataTransfer.files].find((x) => x.type.startsWith('image/'));
    if (f) load(f, { label: 'Dropped' });
    else { const u = e.dataTransfer.getData('text/uri-list').split('\n')[0].trim(); if (u) { url.value = u; fromUrl(u); } }
  });
  el.addEventListener('click', (e) => { if (e.target === el) hide(); });
  const onKey = (e) => { if (e.key === 'Escape') hide(); };

  function hide() {
    el.hidden = true;
    document.body.classList.remove('st-open');
    removeEventListener('paste', onPaste);
    removeEventListener('keydown', onKey);
  }

  return {
    el,
    load,
    open(o) {
      S.opts = o;
      const e = o.entry;
      clear(cat, (app.device ? o.categories : [o.category]).filter(Boolean).map((c) => h('option', { value: c }, c)));
      cat.value = e?.category || (o.categories.includes('Consoles') ? 'Consoles' : o.category);
      cat.parentElement.hidden = !app.device;
      saveBtn.textContent = app.device ? 'Save to the card' : 'Download GIF';
      if (e) {
        setPlatform(e.platform || splat.value || 'MD');
        title.value = e.title || '';
        if (PLATFORMS.includes(e.platform)) plat.value = e.platform;
        query.value = e.title || '';
      }
      el.hidden = false;
      document.body.classList.add('st-open');
      addEventListener('paste', onPaste);
      addEventListener('keydown', onKey);
      if (!pv) {
        pv = new PanelPreview(panel, { onError: (m) => { stats.textContent = 'preview: ' + m; } });
        pv.p.setTransition('none', 0);
      }
      if (S.studio) frame();
      naming();
      if (e && query.value) find();
    },
  };
}
