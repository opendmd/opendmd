// The Studio's Library (/studio/#library), loaded when it is first opened:
// a view of the same page over everything kept in this browser and the
// starter pack. Types down the side (Drafts, Stamps, Palettes, Fonts,
// Templates, Packs) with their counts, a search by name or tag, the
// storage used, Back up everything; a record's detail (its name, tags and
// note, who made it, its licence, what it was made from and what uses it,
// its versions) and what can be done with it: edit or open it, a new piece
// from a template, duplicate, save as a file, delete. Starter records are
// read-only; Duplicate makes an editable copy whose `from` names them.
import { Doc, W, H } from './doc.js';
import { record, newId, unpackPx, CAPS } from './store.js';
import { starter, starterAll, builtin, isStarter, DEFAULT } from './palettes.js';

let S = null, type = 'piece', q = '', open = null, all = {};
const TYPES = [['piece', 'Drafts'], ['stamp', 'Stamps'], ['palette', 'Palettes'], ['font', 'Fonts'], ['template', 'Templates'], ['pack', 'Packs']];
const $ = (id) => document.getElementById(id);
const el = (tag, cls, text) => { const e = document.createElement(tag); if (cls) e.className = cls; if (text !== undefined) e.textContent = String(text); return e; };
const button = (text, act, pri) => { const b = el('button', pri ? 's-pill s-pri' : 's-pill', text); b.type = 'button'; b.dataset.la = act; return b; };
const ro = (r) => isStarter(r.id) || !!builtin(r.id); // a starter or built-in record: read-only
const hexInt = (h) => parseInt(h.slice(1), 16);
const LICENCE = { 'CC0-1.0': 'CC0', 'CC-BY-4.0': 'CC BY 4.0' };
const ago = (iso) => { const t = Date.parse(iso); if (!t) return ''; const s = (Date.now() - t) / 1000; return s < 3600 ? `${Math.max(1, Math.floor(s / 60))} min ago` : s < 86400 ? `${Math.floor(s / 3600)} h ago` : new Date(t).toLocaleDateString(); };

export function mount(studio) {
  S = studio;
  const css = el('link');
  css.rel = 'stylesheet';
  css.href = new URL('library.css', import.meta.url).href;
  document.head.append(css);
  const lib = el('section', 's-lib');
  lib.id = 's-lib';
  lib.setAttribute('aria-labelledby', 's-lib-h');
  lib.innerHTML = '<div class="s-lhd"><h1 id="s-lib-h">Library</h1><span class="s-sp"></span><label class="s-hide" for="s-lq">Search</label>'
    + '<input id="s-lq" class="s-in" type="search" placeholder="Search by name or tag" autocomplete="off">'
    + '<button class="s-pill" type="button" id="s-lopen">Open a file…</button><button class="s-pill" type="button" id="s-lback">Back to the piece</button></div>'
    + '<div class="s-lbody"><nav class="s-lside" id="s-lside" aria-label="Kinds"></nav><div class="s-lmain"><div id="s-ldet" hidden></div><h2 id="s-lkind"></h2><div class="s-lgrid" id="s-lgrid"></div></div></div>';
  S.root.append(lib);
  $('s-lq').oninput = () => { q = $('s-lq').value.trim().toLowerCase(); render(); };
  $('s-lopen').onclick = () => S.files().then((m) => m.pick(), () => {});
  $('s-lback').onclick = () => { location.hash = ''; };
  $('s-lside').onclick = (e) => {
    const b = e.target.closest('[data-type]');
    if (b) { type = b.dataset.type; open = null; render(); }
    if (e.target.closest('#s-lbackup')) S.files().then((m) => m.backup(), () => {});
  };
  $('s-lgrid').onclick = (e) => { const b = e.target.closest('[data-id]'); if (b) detail(b.dataset.id); };
  $('s-ldet').onclick = act;
  $('s-ldet').onchange = edit;
  S.fx.libChanged = () => { if (S.root.dataset.view === 'library') show(); };
}

// --- what there is: the user's records and the starter pack's, by type ---
async function gather() {
  await starter().catch(() => {});
  all = {};
  for (const [t] of TYPES) {
    // (A record of the user's with a starter record's id is never shown for it: the starter one is.)
    const mine = [];
    if (S.store) for (const h of (await S.store.list({ type: t })).items) { if (isStarter(h.id) || builtin(h.id)) continue; const r = await S.store.get(h.id); if (r) mine.push(r); }
    const st = starterAll.filter((r) => r.type === t);
    all[t] = [...mine, ...(t === 'palette' ? [builtin(DEFAULT)] : []), ...st];
  }
}
const hit = (r) => !q || String(r.name).toLowerCase().includes(q) || (Array.isArray(r.tags) && r.tags.some((t) => String(t).toLowerCase().includes(q)));

export async function show() {
  await gather();
  await render();
  const est = await navigator.storage?.estimate?.().catch(() => null);
  const used = $('s-lstore');
  if (used && est) used.textContent = `Kept in this browser: ${(est.usage / 1024).toFixed(0)} KB`;
}

// A record's colours by key (its own, or its palette's).
const colours = (r) => new Map((r.body.colours || builtin(r.body.palette)?.body.colours || S.mine.get(r.body.palette)?.body.colours || []).map((c) => [c.k, hexInt(c.hex)]));
async function pic(r) {
  const b = r.body, c = el('canvas');
  try {
    if (r.type === 'piece' || r.type === 'template') {
      const d = new Doc(b, (x) => x.colours || builtin(x.palette)?.body.colours || S.mine.get(x.palette)?.body.colours || builtin(DEFAULT).body.colours);
      await d.load(0);
      paint(c, W, H, d.composite(0), d.cols);
    } else if (r.type === 'stamp') paint(c, b.w, b.h, await unpackPx(b.frames[0].px, b.w * b.h, b.keys === 2), colours(r));
    else if (r.type === 'font') paint(c, b.atlas.w, b.atlas.h, await unpackPx(b.atlas.px, b.atlas.w * b.atlas.h), new Map([[1, 0xf1ebdd]]));
    else if (r.type === 'palette') { const cs = b.colours; c.width = Math.max(1, cs.length); c.height = 1; const g = c.getContext('2d'); cs.forEach((x, i) => { g.fillStyle = x.hex; g.fillRect(i, 0, 1, 1); }); }
    else return el('span', 's-lpk', `${b.members.length} records`);
  } catch { /* a record that does not read: listed, not drawn */ }
  return c;
}
function paint(c, w, h, keys, cols) {
  c.width = w; c.height = h;
  const g = c.getContext('2d'), img = g.createImageData(w, h);
  for (let i = 0; i < w * h; i++) { const v = keys[i] ? cols.get(keys[i]) ?? 0 : 0; img.data.set([v >> 16, (v >> 8) & 255, v & 255, 255], i * 4); }
  g.putImageData(img, 0, 0);
}
function line(r) {
  const b = r.body;
  if (r.type === 'piece') return `${b.frames.length > 1 ? `${b.frames.length} frames` : 'Still'}${b.layers.length > 1 ? ` · ${b.layers.length} layers` : ''} · ${ago(r.modified)}`;
  if (r.type === 'stamp') return `${b.w}×${b.h}${b.frames.length > 1 ? ` · ${b.frames.length} frames` : ''}`;
  if (r.type === 'palette') return `${b.colours.length} colours`;
  if (r.type === 'font') return `${b.cell.w}×${b.cell.h}`;
  if (r.type === 'template') return b.frames.length > 1 ? `${b.frames.length} frames` : 'Still';
  return `${b.members.length} records`;
}
async function render() {
  const side = $('s-lside');
  side.replaceChildren(...TYPES.map(([t, label]) => {
    const b = el('button', 's-lt' + (t === type ? ' on' : ''));
    b.type = 'button';
    b.dataset.type = t;
    b.append(el('span', '', label), el('i', '', (all[t] || []).filter(hit).length));
    return b;
  }), el('p', 's-cap', ''), Object.assign(el('button', 's-pill', 'Back up everything'), { id: 's-lbackup', type: 'button' }));
  side.querySelector('.s-cap').id = 's-lstore';
  $('s-lkind').textContent = TYPES.find(([t]) => t === type)[1];
  const list = (all[type] || []).filter(hit);
  const cards = await Promise.all(list.map(async (r) => {
    const b = el('button', 's-lcard' + (r.id === open ? ' on' : '') + (r.id === S.rec?.id ? ' now' : ''));
    b.type = 'button';
    b.dataset.id = r.id;
    const p = el('span', 's-lpic');
    p.append(await pic(r));
    b.append(p, el('b', '', r.name), el('small', '', ro(r) ? `${line(r)} · starter` : line(r)));
    return b;
  }));
  $('s-lgrid').replaceChildren(...(cards.length ? cards : [el('p', 's-cap', q ? 'Nothing here matches.' : 'Nothing here yet.')]));
  S.root.dataset.lib = TYPES.map(([t]) => `${t}:${(all[t] || []).length}`).join(' ');
  S.root.dataset.libShown = list.length;
}

// --- a record's detail ---
// What uses a record: the store's rows, and every pack here that names it
// (its rows were written at the pack's save, which may have been before the record came).
async function usedIn(id) {
  if (!S.store) return [];
  const out = new Map((await S.store.usedBy(id)).map((h) => [h.id, h]));
  for (const h of (await S.store.list({ type: 'pack' })).items) {
    if (out.has(h.id)) continue;
    const p = await S.store.get(h.id);
    if (p?.body.members.some((m) => m.id === id)) out.set(h.id, p);
  }
  return [...out.values()];
}
const find = (id) => Object.values(all).flat().find((r) => r.id === id);
async function detail(id) {
  open = id;
  const r = find(id), d = $('s-ldet');
  if (!r) { d.hidden = true; return; }
  // (What it was made from, each a claim: in quotes and isolated, as the open sheet shows a file's.)
  const own = !ro(r), from = el('span'), q = (t) => ['“', el('bdi', '', t), '”'];
  for (const [i, f] of (r.from || []).entries()) from.append(...(i ? [', '] : []), ...(f.external ? (f.note ? q(f.note) : ['an imported picture']) : [...q(f.name || f.id), ` (version ${f.rev})`]));
  const used = own ? await usedIn(id) : [];
  const revs = S.store && own ? await S.store.revs(id) : [];
  const head = el('div', 's-hd');
  if (own) {
    const n = el('input', 's-in s-lname');
    n.value = r.name;
    n.maxLength = CAPS.name;
    n.dataset.f = 'name';
    n.setAttribute('aria-label', 'Name');
    head.append(n);
  } else head.append(el('h3', '', r.name));
  const facts = el('dl', 's-lfacts');
  const fact = (k, v) => { const dd = el('dd'); if (typeof v === 'string') dd.textContent = v; else dd.append(v); facts.append(el('dt', '', k), dd); };
  const field = (f, v, label, max) => { const i = el('input', 's-in'); i.value = v; i.maxLength = max; i.dataset.f = f; i.setAttribute('aria-label', label); return i; };
  fact('Tags', own ? field('tags', r.tags.join(', '), 'Tags', 400) : r.tags.join(', ') || '—');
  fact('Note', own ? field('note', r.note || '', 'Note', CAPS.note) : r.note || '—');
  fact('Made by', r.author?.name || '—');
  fact('Licence', LICENCE[r.licence] || 'not chosen');
  fact('Made from', from.childNodes.length ? from : '—');
  if (own) fact('Used in', used.map((u) => u.name).join(', ') || '—');
  // A pack's members, each a name, and the ones this browser doesn't hold (a file named them, or they were deleted) said as such.
  if (r.type === 'pack') {
    const here = new Map(Object.values(all).flat().map((x) => [x.id, x])), ms = r.body.members.map((m) => here.get(m.id) || builtin(m.id)), got = ms.filter(Boolean), gone = ms.length - got.length, h = el('span');
    if (got.length) h.append(got.slice(0, 12).map((x) => x.name || 'Untitled').join(', ') + (got.length > 12 ? ` and ${got.length - 12} more` : ''));
    if (gone) h.append(...(got.length ? [' · '] : []), el('span', 's-warn', `${gone} not in this browser`));
    h.dataset.missing = gone;
    fact('Holds', h.childNodes.length ? h : '—');
  }
  if (own && revs.length) fact('Versions', revs.map((v) => `${v.rev}${v.label ? ` (${v.label})` : ''}`).join(', ') + `, ${r.rev} now`);
  const acts = el('div', 's-row');
  if (r.type === 'piece' && own) acts.append(button(r.id === S.rec?.id ? 'Back to it' : 'Open', 'edit', true));
  if (r.type === 'template') acts.append(button('New piece from this', 'new', true));
  if (r.type === 'palette') acts.append(button('Use its colours as swatches', 'swatch'));
  if (r.type === 'stamp') acts.append(button('Use this stamp', 'stamp'));
  acts.append(button('Duplicate', 'dup'), button('Save as file', 'file'));
  if (r.type === 'pack' && S.sharing) acts.append(button('Share link', 'link')); // (off: studio.js SHARE)
  if (own) acts.append(button('Delete', 'del'));
  d.replaceChildren(head, facts, acts);
  d.dataset.id = id;
  d.hidden = false;
  for (const c of $('s-lgrid').children) c.classList?.toggle('on', c.dataset.id === id);
  S.root.dataset.libOpen = id;
}
// A name, tags or note changed: the open piece through the editor, any other record put.
async function edit(e) {
  const i = e.target.closest('[data-f]'), id = $('s-ldet').dataset.id, r = find(id);
  if (!i || !r || ro(r)) return;
  const v = i.value.trim(), o = i.dataset.f === 'tags' ? { tags: [...new Set(v.split(',').map((t) => t.trim()).filter(Boolean))].slice(0, CAPS.tags) }
    : i.dataset.f === 'note' ? { note: v.slice(0, CAPS.note) || undefined } : { name: v.slice(0, CAPS.name) || 'Untitled' };
  try {
    if (id === S.rec?.id) S.setHead(o);
    else await S.run(() => S.store.put({ ...r, ...o }, { base: r.ver }));
    S.say('');
  } catch (x) { S.say(`That could not be changed: ${x.message || x}`); }
  await show();
  await detail(id);
}
async function act(e) {
  const a = e.target.closest('[data-la]')?.dataset.la, id = $('s-ldet').dataset.id, r = find(id);
  if (!a || !r) return;
  if (a === 'edit') { location.hash = ''; if (id !== S.rec?.id) await S.openRec(await S.store.get(id)); }
  if (a === 'new') await fromTemplate(r);
  if (a === 'swatch') { S.useSwatches(r); S.menu(); location.hash = ''; }
  if (a === 'stamp') { location.hash = ''; const m = await S.stamps(); await m.show(); m.pickId(id); }
  if (a === 'file') (await S.files()).save(id);
  if (a === 'link') (await S.link()).sharePack(id);
  if (a === 'dup') await duplicate(r);
  if (a === 'del') {
    const used = await usedIn(id);
    const said = used.length ? `${r.name} is used in ${used.map((u) => u.name).join(', ')}: they keep what they have, but no longer point at it. Delete it? It can’t be brought back.` : `Delete ${r.name}? It can’t be brought back.`;
    if (!confirm(said)) return;
    await S.forget(id);
    open = null;
    $('s-ldet').hidden = true;
    await show();
  }
}
// A copy of one's own to change: a new id, rev 1, made from the original.
async function duplicate(r) {
  if (!S.store) return;
  const { ver, seq, label, ...c } = r, t = new Date().toISOString().replace(/\.\d+Z$/, 'Z');
  const copy = { ...c, id: newId(r.type), rev: 1, name: `${r.name} copy`.slice(0, CAPS.name), created: t, modified: t,
    from: [{ id: r.id, rev: r.rev, hash: r.hash, name: r.name, author: r.author, licence: r.licence }, ...(r.from || [])].slice(0, CAPS.from) };
  try {
    const h = await S.run(() => S.store.put(copy));
    type = r.type;
    await show();
    await detail(h.id);
  } catch (x) { S.say(`That could not be copied: ${x.message || x}`); }
}
// A new piece from a template: its layers and frames, its palette's colours as the piece's own, made from it.
async function fromTemplate(t) {
  await starter().catch(() => {});
  const b = t.body, cs = b.colours || builtin(b.palette)?.body.colours || S.mine.get(b.palette)?.body.colours;
  if (!cs) { S.say(`“${t.name}” names colours this browser doesn’t have, so no piece was made from it.`); return; }
  const wide = b.keys === 2, layers = [];
  for (const l of b.layers) layers.push({ name: l.name, moves: l.moves, ...(l.hide ? { hide: true } : {}), cels: await Promise.all(l.cels.map((z) => unpackPx(z, W * H, wide))) });
  location.hash = '';
  await S.save();
  await S.newPiece({ name: t.name, tags: t.tags.slice(0, CAPS.tags), frames: b.frames.map((f) => ({ ms: f.ms })), layers,
    colours: cs.map(({ k, hex }) => ({ k, hex })), from: [{ id: t.id, rev: t.rev, hash: t.hash, name: t.name, author: t.author, licence: t.licence }] });
  S.keep();
  await S.drafts();
}
