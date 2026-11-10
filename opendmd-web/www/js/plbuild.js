// The playlist editor's "Add by filter" sheet (imported when it first
// opens): the GIFs of a playlist, or of the whole card, that a search finds
// — chips for platforms, folders, what to leave out, the user's tags and
// stars, more words of the Library's search, shuffled and cut — added to
// the playlist or in place of what is in it. With "Keep this search" the
// playlist becomes a smart one: js/smart.js writes it again whenever the
// search finds something else.

import { app } from './state.js';
import { art } from './artmod.js';
import { popup, closeMenu } from './overrides.js';
import { find, everyGif } from './smart.js';
import { h, clear, niceName, isSys, byNice, FLAGS } from './util.js';

const picks = (box, k) => [...box.querySelectorAll('.on')].map((c) => c.dataset[k]);
const quote = (v) => (/[\s,:]/.test(v) ? `"${v.replace(/"/g, '')}"` : v);
const list = (k, vs) => vs.length ? [`${k}:${vs.map(quote).join(',')}`] : [];
const STARS = [['', 'Any'], ['rating>=3', '★ 3 or more'], ['rating>=4', '★ 4 or more'], ['rating:5', '★ 5'], ['unrated', 'Not rated']];

// ctx: {lists(), known(), done(paths, append, smart|null)} — smart {q, from}.
export function builder(ctx) {
  const tog = (e) => { e.currentTarget.classList.toggle('on'); count(); };
  const bSeedL = h('label', { hidden: true }, 'Seed');
  const baseSel = h('select', { id: 'b-base', onchange: () => platChips() });
  const bPlats = h('div.chips', { id: 'b-plats' });
  const bPlat = h('input', { type: 'text', id: 'b-platforms', placeholder: 'MD,SNES', hidden: true });
  const bCats = h('div.chips', { id: 'b-cats' });
  const bTitle = h('input', { type: 'text', id: 'b-title', placeholder: 'mario, sonic' });
  const bFlags = h('div.chips', { id: 'b-flags' }, Object.entries(FLAGS).map(([k, n]) => h('button.chip', { 'data-flag': k.toLowerCase(), onclick: tog }, n)));
  const bTagsL = h('div.lbl', { hidden: true }, 'Tagged');
  const bTags = h('div.chips', { id: 'b-tags', hidden: true });
  const bStars = h('select', { id: 'b-stars' }, STARS.map(([v, t]) => h('option', { value: v }, t)));
  const bWords = h('input', { type: 'search', id: 'b-words', placeholder: 'e.g. by:pixelkid42 -tag:boss' });
  const bShuffle = h('input', { type: 'checkbox', id: 'b-shuffle', onchange: () => { bSeedL.hidden = !bShuffle.checked; } });
  const bSeed = h('input', { type: 'number', id: 'b-seed', value: 1, min: 0, style: 'width:90px' });
  const bLimit = h('input', { type: 'number', id: 'b-limit', placeholder: 'all', min: 1, style: 'width:90px' });
  const bKeep = h('input', { type: 'checkbox', id: 'b-keep', onchange: () => count() });
  const bCount = h('span.cap.bad', { id: 'b-count' });
  const bAdd = h('button.pri', { id: 'b-append', onclick: () => build(true) }), bUse = h('button', { id: 'b-use', onclick: () => build(false) });
  const el = h('div.menu.wide', { id: 'builder', role: 'dialog', 'aria-label': 'Add by filter' }, h('div.mh', h('b', 'Add GIFs by filter')),
    h('div.form',
      h('label.stack', 'Take GIFs from', baseSel),
      h('label.stack', 'Title has any of', bTitle)),
    h('div.lbl', 'Platforms'), bPlats, bPlat,
    h('div.lbl', 'Folders'), bCats,
    h('div.lbl', 'Leave out'), bFlags,
    bTagsL, bTags,
    h('div.form', { style: 'margin-top:12px' }, h('label.stack', 'Stars', bStars), h('label.stack', 'More search words', bWords)),
    h('div.row', { style: 'margin-top:16px' }, h('label', bShuffle, 'Shuffle'), bSeedL, h('label', 'At most', bLimit)),
    h('label', { style: 'margin-top:12px;display:block' }, bKeep, 'Keep this search: the playlist follows it as GIFs, tags and stars change'), bCount,
    h('div.row', { style: 'margin-top:12px' }, bAdd, bUse));
  bSeedL.append(bSeed);
  for (const i of [baseSel, bPlat, bTitle, bShuffle, bSeed, bLimit, bStars, bWords]) i.addEventListener('change', count);
  bWords.addEventListener('input', count);

  // The base playlist's text (every GIF on the card: '').
  const text = (b) => (b ? app.text(b) : everyGif());
  async function categories() {
    let dirs = [];
    try { dirs = (await app.src.list('/gifs')).filter((d) => d.dir).map((d) => d.name).sort(byNice); } catch { /* none */ }
    clear(bCats, dirs.map((d) => h('button.chip' + (isSys(d) ? '.sys' : ''), { 'data-cat': d, onclick: tog }, niceName(d))));
  }
  async function platChips() {
    const t = await text(baseSel.value).catch(() => null), n = new Map(), keep = picks(bPlats, 'platform');
    if (t) for (const e of app.wasm.parseManifest(t)) if (e.platform) n.set(e.platform, (n.get(e.platform) || 0) + 1);
    bPlat.hidden = !!n.size;
    clear(bPlats, [...n].sort((a, b) => b[1] - a[1]).map(([p, c]) => h('button.chip' + (keep.includes(p) ? '.on' : ''), { 'data-platform': p, onclick: tog }, p, h('small', c.toLocaleString()))));
    count();
  }
  async function tagChips() {
    const names = (await (await import('./tags.js')).tags()).names(), keep = picks(bTags, 'tag');
    bTags.hidden = bTagsL.hidden = !names.length;
    clear(bTags, names.slice(0, 40).map(([t, c]) => h('button.chip' + (keep.includes(t) ? '.on' : ''), { 'data-tag': t, onclick: tog }, t, h('small', c.toLocaleString()))));
  }

  // The search the sheet says, as the Library's search box would read it.
  const query = () => [
    ...list('platform', (bPlat.hidden ? picks(bPlats, 'platform') : bPlat.value.split(',')).map((s) => s.trim().toLowerCase()).filter(Boolean)),
    ...list('folder', picks(bCats, 'cat')),
    ...list('title', bTitle.value.split(',').map((s) => s.trim()).filter(Boolean)),
    ...list('-is', picks(bFlags, 'flag')),
    ...list('tag', picks(bTags, 'tag')),
    bStars.value, bWords.value.trim(),
    bShuffle.checked && `shuffle seed:${+bSeed.value || 0}`,
    +bLimit.value > 0 && `limit:${+bLimit.value}`,
  ].filter(Boolean).join(' ');
  const run = () => find(query(), baseSel.value);
  async function count() {
    bAdd.disabled = bKeep.checked;
    try {
      const n = (await run()).length, t = n.toLocaleString() + (n == 1 ? ' GIF' : ' GIFs');
      bAdd.textContent = 'Add ' + t;
      bUse.textContent = 'Replace with ' + t;
      bAdd.disabled = !n || bKeep.checked;
      bUse.disabled = !n;
      bCount.textContent = '';
    } catch (e) { bCount.textContent = e.message || String(e); bAdd.disabled = bUse.disabled = true; }
  }
  async function build(append) {
    try {
      const paths = await run(), q = bKeep.checked && !append && (await render(query()));
      closeMenu();
      ctx.done(paths, append, q != null && q !== false ? { q, from: baseSel.value } : null);
    } catch (e) { bCount.textContent = e.message || String(e); }
  }
  const render = async (q) => { const s = new (await art()).Search(q); try { return s.render(); } finally { s.free(); } };

  let built = false;
  // Open under `btn`; `smart` ({q, from}): the search of the playlist open,
  // in the words box, kept.
  return async function open(btn, smart) {
    const keep = smart ? smart.from : baseSel.value, known = ctx.known();
    clear(baseSel, h('option', { value: '' }, 'Every GIF on the card'), ctx.lists().map((p) => h('option', { value: p.path }, p.label)));
    baseSel.value = smart ? smart.from : keep && known.includes(keep) ? keep : '/filelist.txt';
    if (smart) {
      for (const c of el.querySelectorAll('.chip.on')) c.classList.remove('on');
      bTitle.value = bPlat.value = bLimit.value = '';
      bStars.value = '';
      bShuffle.checked = false;
      bSeedL.hidden = true;
      bWords.value = smart.q;
    }
    bKeep.checked = !!smart;
    const r = btn.getBoundingClientRect();
    popup(el, r.right - 440, r.bottom + 4);
    if (!built) { built = true; categories(); }
    tagChips().catch(() => {});
    platChips();
  };
}
