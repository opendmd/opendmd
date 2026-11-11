// The Studio's files (/studio/), loaded when one is first saved or opened:
// `.dmdart` (a record, and its palette if it is the user's own), `.dmdpack`
// (a pack and every member at the revision it lists), and a backup (a pack
// of everything in the Library). One format, `{format, root, records}`
// (docs/studio-format.md).
//
// Saving asks, once each time, who made it (remembered in this browser) and
// its licence; it raises the record's rev, as a download does, and writes
// that revision. Opening a file is the first time records from other
// people arrive, so nothing in one is trusted: its size and record count
// are capped before it is read, every record goes through check() and its
// hash is worked out again, ids are each once, a pack's members are all in
// it and lead nowhere round, and nothing is stored until the user has seen
// what comes in and said Add. Merging is by id: the same id and hash is
// skipped; a higher rev is offered as an update; the same id with other
// content comes in as a copy whose `from` names the file's record; a
// starter record is never replaced.
import { FORMAT, LICENCES, CAPS, check, hashOf, record, newId, isBuiltin } from './store.js';
import { builtin, isStarter, starter } from './palettes.js';

export const LIMITS = { bytes: 16 << 20, records: 4096 + 64 };
const LS = 'odmd.studio.author';
const EXT = { pack: '.dmdpack' };
let S = null;
const $ = (id) => document.getElementById(id);
const el = (tag, cls, text) => { const e = document.createElement(tag); if (cls) e.className = cls; if (text !== undefined) e.textContent = text; return e; };
const now = () => new Date().toISOString().replace(/\.\d+Z$/, 'Z');
const stem = (n) => String(n).replace(/[^\p{L}\p{N} _-]+/gu, '').trim().replace(/\s+/g, '_').slice(0, 100) || 'studio';
const ls = { get: () => { try { return localStorage.getItem(LS) || ''; } catch { return ''; } }, set: (v) => { try { localStorage.setItem(LS, v); } catch { /* not kept */ } } };
const want = (r) => (r.type === 'piece' || r.type === 'template' ? 4096 * (r.body.keys === 2 ? 2 : 1) : 0);
const mineId = (id) => !isBuiltin(id) && !isStarter(id);

export function mount(studio) { S = studio; }

// --- a sheet over the page: built once, shown with its content ---
function sheet(title, body, acts) {
  let d = $('s-fs');
  if (!d) {
    d = el('dialog');
    d.id = 's-fs';
    d.style.cssText = 'width:min(34em,92vw);max-height:86vh;padding:18px 20px;border:1px solid var(--line);border-radius:14px;background:var(--bg);color:var(--fg)';
    document.body.append(d);
  }
  const h = el('div', 's-hd'), row = el('div', 's-row');
  h.append(el('h2', '', title));
  for (const a of acts) row.append(a);
  d.replaceChildren(h, ...body, row);
  if (!d.open) d.showModal();
  return d;
}
const button = (text, pri, on) => { const b = el('button', pri ? 's-pill s-pri' : 's-pill', text); b.type = 'button'; b.onclick = on; return b; };

// Who made it and its licence, asked once for an export: {author, licence}, or null if cancelled.
function ask(rec) {
  return new Promise((done) => {
    const name = el('input', 's-in'), lic = el('select', 's-in'), err = el('p', 's-cap s-bad');
    name.id = 's-fs-author';
    name.maxLength = 80;
    name.value = rec.author?.name || ls.get();
    lic.id = 's-fs-licence';
    lic.append(new Option('Not chosen', ''), new Option('CC0 (anyone may use it, no credit needed)', 'CC0-1.0'), new Option('CC BY 4.0 (anyone may use it, crediting you)', 'CC-BY-4.0'));
    lic.value = rec.licence || '';
    err.hidden = true;
    const lab = (t, e) => { const l = el('label', 's-f', t); l.style.cssText = 'display:grid;gap:4px;margin-top:10px;font-size:14px'; l.append(e); return l; };
    const body = [lab('Made by', name), lab('Licence', lic), err];
    if ((rec.from || []).some((f) => f.external === true)) body.splice(2, 0, el('p', 's-cap s-warn', 'Part of this came from an imported picture. Choose a licence only if it is yours to license.'));
    const d = sheet('Save as file', body, [
      button('Save', true, () => {
        const author = name.value.trim().slice(0, 80), licence = lic.value || null;
        if (licence === 'CC-BY-4.0' && !author) { err.hidden = false; err.textContent = 'CC BY means crediting you: say who made it.'; name.focus(); return; }
        if (author) ls.set(author);
        d.close();
        done({ author, licence });
      }),
      button('Cancel', false, () => { d.close(); done(null); })]);
    d.onclose = () => done(null);
  });
}
function write(name, root, records) {
  const text = JSON.stringify({ format: FORMAT, root, records });
  const a = el('a');
  a.href = URL.createObjectURL(new Blob([text], { type: 'application/json' }));
  a.download = name;
  document.body.append(a);
  a.click();
  a.remove();
  setTimeout(() => URL.revokeObjectURL(a.href), 30000);
  S.root.dataset.file = `${name}:${text.length}`;
  return text;
}
// A record's palette goes with it if it is the user's own.
async function withPalette(r) {
  const p = typeof r.body.palette === 'string' && mineId(r.body.palette) ? await S.store.get(r.body.palette) : null;
  return p ? [r, p] : [r];
}

// The open piece, saved as a file: asked, then frozen (its rev rises), that revision written.
export async function saveOpen() {
  if (!S.store || !S.kept) { S.say('Draw something first: a file keeps a piece as it is kept.'); return; }
  const m = await ask(S.rec);
  if (!m) return;
  S.meta(m);
  const h = await S.freeze('');
  if (!h) { S.say('It could not be saved first, so no file was made.'); return; }
  const r = await S.store.get(S.rec.id, h.rev - 1), { ver, label, ...clean } = r;
  write(`${stem(r.name)}.dmdart`, r.id, await withPalette(clean));
  await S.snaps();
  S.status('kept', `Saved ${stem(r.name)}.dmdart`);
}
// Any record from the Library: the same, for one not open in the editor (a starter record as it is).
export async function save(id) {
  if (S.rec?.id === id) return saveOpen();
  let r = (S.store && await S.store.get(id)) || builtin(id);
  if (!r) return;
  if (mineId(id)) {
    const m = await ask(r);
    if (!m) return;
    const h = await S.run(() => S.store.put({ ...r, author: { name: m.author, account: r.author?.account ?? null }, licence: m.licence }, { base: r.ver }));
    const next = await S.run(() => S.store.snapshot(id));
    r = await S.store.get(id, next.rev - 1) || { ...r, ...h };
  }
  const { ver, label, ...clean } = r;
  let records = await withPalette(clean);
  if (r.type === 'pack') {
    const ms = [];
    for (const mb of r.body.members) {
      let x = builtin(mb.id);
      if (!x && S.store) { const head = await S.store.get(mb.id); x = head && head.rev === mb.rev ? head : await S.store.get(mb.id, mb.rev); }
      if (!x) { S.say(`This pack names a record this browser no longer has at the version it lists: ${mb.id}.`); return; }
      const { ver: _, label, ...m } = x;
      ms.push(m);
    }
    records = [clean, ...ms];
  }
  write(`${stem(r.name)}${EXT[r.type] || '.dmdart'}`, r.id, records);
}
// Everything in the Library (the user's own records, as they are now), as one pack.
export async function backup() {
  if (!S.store) return;
  const all = [];
  for (const type of ['palette', 'stamp', 'font', 'template', 'piece', 'pack']) {
    for (const h of (await S.store.list({ type })).items) { if (!mineId(h.id)) continue; const r = await S.store.get(h.id); if (r) { const { ver, ...c } = r; all.push(c); } }
  }
  const day = now().slice(0, 10);
  const pack = record('pack', `Backup, ${day}`, { members: all.map((r) => ({ id: r.id, rev: r.rev })) }, { tags: ['backup'] });
  pack.hash = await hashOf(pack.body);
  write(`OpenDMD_Studio_backup_${day}.dmdpack`, pack.id, [pack, ...all]);
  S.say('');
}

// --- opening a file ---
export function pick() {
  const i = el('input');
  i.type = 'file';
  i.accept = '.dmdart,.dmdpack,application/json';
  i.onchange = () => { if (i.files[0]) open(i.files[0]); };
  i.click();
}
// What a file holds, checked through: {root, records}, or a sentence why not.
export async function read(file) {
  const no = (why) => { throw new Error(why); };
  if (file.size > LIMITS.bytes) no(`That file is ${(file.size / 1048576).toFixed(1)} MB: a Studio file is ${LIMITS.bytes >> 20} MB at most.`);
  let f;
  try { f = JSON.parse(await file.text()); } catch { no('That isn’t a Studio file: it doesn’t read as one.'); }
  if (!f || typeof f !== 'object' || Array.isArray(f) || f.format !== FORMAT || typeof f.root !== 'string' || !Array.isArray(f.records)) no('That isn’t a Studio file (.dmdart or .dmdpack).');
  if (!f.records.length || f.records.length > LIMITS.records) no(`That file holds ${f.records.length} records: a Studio file holds 1 to ${LIMITS.records}.`);
  const by = new Map();
  for (const r of f.records) {
    try { check(r); } catch (e) { no(`A record in it isn’t one (${e.message.replace('not a record: ', '')}): nothing was taken from the file.`); }
    if (by.has(r.id)) no('The file names one record twice: nothing was taken from it.');
    let h = '';
    try { h = await hashOf(r.body, want(r)); } catch (e) { no(`“${r.name}” in it doesn’t read (${e.message}): nothing was taken from the file.`); }
    if (h !== r.hash) no(`“${r.name}” in it isn’t what its hash says: nothing was taken from the file.`);
    by.set(r.id, r);
  }
  const root = by.get(f.root);
  if (!root) no('The file doesn’t hold the record it says it is: nothing was taken from it.');
  // A pack's members are all in the file, at the rev it lists, and no pack holds itself, however far round.
  const seen = new Set(), walk = (r, path) => {
    if (r.type !== 'pack') return;
    if (path.has(r.id)) no('A pack in the file holds itself: nothing was taken from it.');
    path.add(r.id);
    for (const m of r.body.members) {
      const x = by.get(m.id);
      if (!x || x.rev !== m.rev) no(`A pack in the file names a record the file doesn’t hold (${m.id}): nothing was taken from it.`);
      walk(x, path);
    }
    path.delete(r.id);
    seen.add(r.id);
  };
  for (const r of by.values()) if (r.type === 'pack' && !seen.has(r.id)) walk(r, new Set());
  return { root, records: [...by.values()] };
}
// How each record would come in: new, same (skipped), update (offered), copy, starter (never replaced).
async function plan(records) {
  await starter().catch(() => {});
  const out = [];
  for (const r of records) {
    const st = builtin(r.id);
    if (st || isBuiltin(r.id) || isStarter(r.id)) { out.push({ r, how: st && st.hash === r.hash ? 'same' : 'starter' }); continue; }
    const have = S.store ? await S.store.get(r.id) : null;
    out.push({ r, have, how: !have ? 'new' : have.hash === r.hash ? 'same' : r.rev > have.rev ? 'update' : 'copy' });
  }
  return out;
}
const SAID = { new: 'new', same: 'already here', update: 'a newer version', copy: 'comes in as a copy (yours differs)', starter: 'a starter record: left as it is' };
const ORDER = ['palette', 'font', 'stamp', 'template', 'piece', 'pack'];
// A file opened: read, shown, and only on Add stored.
export async function open(file) {
  let got;
  try { got = await read(file); } catch (e) { S.say(e.message); S.root.dataset.opened = 'refused'; return null; }
  const p = await plan(got.records), backupRoot = got.root.type === 'pack' && got.root.tags.includes('backup');
  const rows = p.filter((x) => !(backupRoot && x.r === got.root)).map((x) => {
    const li = el('li'), b = el('b', '', x.r.name || 'Untitled'), small = el('small', '', ` · ${x.r.type} · ${SAID[x.how]}`);
    li.append(b, small);
    if (x.how === 'update') {
      const c = el('input');
      c.type = 'checkbox';
      c.checked = true;
      c.dataset.id = x.r.id;
      const l = el('label', '', ' update');
      l.prepend(c);
      li.append(' ', l);
    }
    li.dataset.how = x.how;
    return li;
  });
  const ul = el('ul', 's-list s-fs-list');
  ul.style.cssText = 'max-height:46vh;overflow:auto';
  ul.append(...rows);
  const n = (h) => p.filter((x) => x.how === h).length;
  const what = el('p', 's-cap', `${file.name}: ${n('new')} new, ${n('update')} newer, ${n('copy')} as copies, ${n('same') + n('starter')} already here.`);
  return new Promise((done) => {
    const d = sheet('Open a file', [what, ul], [
      button('Add', true, async () => { d.onclose = null; d.close(); done(await add(p, got, backupRoot, ul)); }),
      button('Cancel', false, () => { d.close(); })]);
    d.onclose = () => { S.root.dataset.opened = 'cancelled'; done(null); };
    S.root.dataset.opened = 'asked';
  });
}
async function add(p, got, backupRoot, ul) {
  const skip = new Set([...ul.querySelectorAll('input[type=checkbox]')].filter((c) => !c.checked).map((c) => c.dataset.id));
  const go = p.filter((x) => (x.how === 'new' || x.how === 'update' || x.how === 'copy') && !skip.has(x.r.id) && !(backupRoot && x.r === got.root));
  // Copies take new ids; what in the file points at them points at the copies.
  const ids = new Map(go.filter((x) => x.how === 'copy').map((x) => [x.r.id, newId(x.r.type)]));
  const to = (id) => ids.get(id) || id;
  let n = 0, err = '', first = null;
  for (const type of ORDER) {
    for (const x of go.filter((y) => y.r.type === type)) {
      const { ver, ...r0 } = x.r, body = { ...r0.body };
      if (typeof body.palette === 'string') body.palette = to(body.palette);
      if (Array.isArray(body.members)) body.members = body.members.map((m) => ({ ...m, id: to(m.id) }));
      let r = { ...r0, body };
      if (x.how === 'copy') r = { ...r, id: ids.get(r0.id), rev: 1, created: now(), from: [{ id: r0.id, rev: r0.rev, hash: r0.hash, name: r0.name, author: r0.author, licence: r0.licence }, ...r0.from].slice(0, CAPS.from) };
      try {
        const h = await S.run(() => S.store.put(r, { base: x.how === 'update' ? x.have.ver : null }));
        n++;
        if (r0.id === got.root.id) first = h.id;
      } catch (e) { err ||= `“${r.name}”: ${e.message || e}`; }
    }
  }
  S.root.dataset.opened = `added:${n}`;
  S.say(err ? `Some of it could not be added: ${err}` : '');
  S.status('kept', `Added ${n} from the file`);
  S.fx.libChanged?.();
  if (first && got.root.type === 'piece') await S.openRec(await S.store.get(first));
  else await S.drafts();
  return n;
}
export { LICENCES };
