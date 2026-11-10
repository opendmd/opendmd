// Playlists: the list (#sets), and the editor as a screen of its own
// (#sets/edit: on a phone it replaces the list, with a ‹ back; on a laptop
// the right column), js/pledit.js, imported the first time a playlist
// opens. Play PUT /api/config {set}; Delete DELETE /api/files/sets/…;
// Rename and Duplicate write the playlist (and its overrides, and a smart
// playlist's search: js/smart.js) anew. Smart playlists are written again
// from their searches when the view is shown (at most once a minute).

import { app, SET_NAME, SET_NAME_RULE } from './state.js';
import { actions, closeMenu, Overrides, fileFor } from './overrides.js';
import { h, clear, emit, on, toast, lines, note, isSys, byNice, gone } from './util.js';

const smartMod = () => import('./smart.js');

export async function mount(el) {
  const setsList = h('div.list', { id: 'sets-list' });
  const se = h('div.se', h('p.muted.sehint', 'Open a playlist to see and change what is in it.'));
  el.append(h('div.cols.sets', { id: 'sets' },
    h('div.sl', h('div.card', h('div.row.sp', { style: 'margin-bottom:8px' }, h('h2', { style: 'margin:0' }, 'Playlists'), h('button', { id: 'sets-new', onclick: newList }, '+ New playlist')), setsList,
      note('“All files” plays when no other playlist is chosen', 'Changing the playlist takes a few seconds; the panel holds its frame meanwhile.'))),
    se));

  // The editor, once something opens.
  let edQ = null, edi = null, known = [], lists = [];
  const editor = () => (edQ ??= import('./pledit.js').then((m) => (edi = m.editor(se, { el, closeEd, activate, loadSets, known: () => known, lists: () => lists }))));

  // #sets/edit: pushed when opened from the list, so ‹ goes back.
  let pushed = false;
  async function goEd() { (await editor()).header(); if (!el.classList.contains('ed')) { pushed = true; location.hash = '#sets/edit'; } }
  function closeEd() { if (pushed) history.back(); else location.replace('#sets'); }
  async function newList() {
    if (edi && !(await edi.guard())) return;
    app.draft = { name: '', paths: [], path: '', saved: '' };
    (await editor()).opened();
    goEd();
  }

  let ctok = 0, ltok = 0;
  async function loadSets() {
    // Each load lists the card afresh and only the latest asked is shown: the
    // load a config change started before a Delete must not land after the
    // one the Delete started, and bring the deleted row back.
    const t = ++ltok, got = await app.playlists(true), sm = await smartMod();
    await sm.load(true).catch(() => {});
    if (t !== ltok) return;
    lists = got;
    known = lists.map((p) => p.path);
    const active = app.activeSet;
    const rows = lists.sort((a, b) => a.root - b.root || byNice(a.label, b.label)).map((p) => {
      const isActive = app.device && p.value === active, n = h('span.n'), sp = sm.get(p.path);
      return [p, n, h('div' + (ed() && p.path === app.draft.path ? '.cur' : ''), { 'data-path': p.path, 'data-value': p.value },
        h('button.grow.pl' + (!p.root && isSys(p.value) ? '.sys' : ''), { onclick: () => open(p) },
          p.label, !p.value && h('span.tag', 'Default'), sp && h('span.tag.smart', { title: 'Built from a search: ' + sm.summary(sp) }, 'Smart'), isActive && h('span.tag.act', 'Playing'), n),
        h('button.more', { 'aria-label': 'More', 'aria-haspopup': 'menu', onclick: (e) => actions(e.currentTarget, 'sets-menu', [
          app.device && !isActive && [{ 'data-do': 'activate' }, 'Play', () => activate(p), '', app.building],
          sp && [{ 'data-do': 'refresh' }, 'Refresh', () => refreshOne(p)],
          !p.root && [{ 'data-do': 'rename' }, 'Rename…', () => rename(p)],
          [{ 'data-do': 'duplicate' }, 'Duplicate', () => duplicate(p)],
          !p.root && [{ 'data-do': 'delete' }, 'Delete…', () => del(p), '.danger']], p.label) }, '…'))];
    });
    // None of the user's own yet: how to start one.
    const none = !lists.some((p) => !p.root) && h('div.empty', { id: 'sets-empty' },
      h('p.cap', 'No playlists of your own yet. Start one here, or pick GIFs in the Library and add them to a new playlist.'),
      h('div.row', h('button.pri', { id: 'sets-empty-new', onclick: newList }, '+ New playlist'), h('button', { id: 'sets-empty-lib', onclick: () => app.go('library') }, 'Go to the Library')));
    clear(setsList, rows.map((r) => r[2]), none);
    counts(rows);
  }
  async function counts(rows) {
    // A device's root lists (~400 KB) are not fetched to be counted.
    for (const t = ++ctok; rows.length;) {
      const [p, n] = rows.shift();
      if (app.device && p.root && !app.texts.has(p.path)) {
        if (p.value === app.activeSet && app.status?.entries != null) n.textContent = app.status.entries.toLocaleString();
        continue;
      }
      const text = await app.text(p.path).catch(() => null);
      if (t !== ctok) return;
      if (text != null) n.textContent = lines(text).filter((p) => !gone(p)).length.toLocaleString();
    }
  }

  async function refreshOne(p) {
    try {
      const r = await (await smartMod()).refresh(p.path, true);
      toast(r && (r.added || r.removed) ? `“${p.label}”: +${r.added} −${r.removed}` : `“${p.label}” is up to date`, 'ok');
    } catch (e) { toast(`refresh: ${e.message || e}`, 'bad'); }
    loadSets();
  }
  // Every smart playlist from its search, quietly (on show, once a minute).
  let auto = 0;
  async function refreshAll() {
    if (performance.now() - auto < 60000) return;
    auto = performance.now();
    const sm = await smartMod();
    if (!(await sm.load()).size) return;
    if (await sm.refreshAll(undefined, false, true).catch(() => 0)) loadSets();
  }

  async function open(p) {
    if (edi && !(await edi.guard())) return;
    try {
      const text = await app.text(p.path, true);
      if (text == null) throw new Error('not served by this source');
      const paths = lines(text).filter((p) => !gone(p));
      const smart = (await smartMod()).get(p.path);
      app.draft = { paths, name: p.root ? '' : p.value, path: p.path, saved: paths.join('\n'), smart, smartPaths: paths.join('\n') };
      (await editor()).opened();
      for (const r of setsList.children) r.classList.toggle('cur', r.dataset.path === p.path);
      goEd();
    } catch (e) { toast(`${p.label}: ${e.message}`, 'bad'); }
  }

  async function activate(p) {
    if (app.building) { toast('the display is still reading a playlist; try again in a moment', 'bad'); return; }
    const set = p.value, n = p.label;
    try {
      toast(`switching to “${n}”…`);
      const r = await app.src.putConfig({ set });
      await app.refreshStatus();
      await app.refreshConfig();
      toast(`Playing “${n}” · ${(r.entries ?? app.status?.entries ?? 0).toLocaleString()} GIFs`, 'ok');
    } catch (e) { toast('play: ' + e.message, 'bad'); }
  }

  async function del(p) {
    const path = p.path, set = p.value;
    const isActive = app.device && app.activeSet === set;
    if (!confirm(isActive ? `"${p.label}" is playing. Switch the display back to All files and delete it?` : `Delete the playlist "${p.label}"?`)) return;
    try {
      await (await smartMod()).idle();
      if (isActive) { await app.src.putConfig({ set: '' }); await app.refreshStatus(); await app.refreshConfig(); }
      await app.src.deleteFile(path);
      app.texts.delete(path);
      await (await smartMod()).drop(path);
      toast(`deleted "${set}"`, 'ok');
      loadSets();
    } catch (e) { toast('delete: ' + e.message, 'bad'); }
  }

  // Rename and Duplicate (no request renames a playlist): its lines and its
  // overrides written under the new name; a rename then moves what names
  // the old one (the display's playlist, the schedule's rules, the editor)
  // and deletes it. A name taken gets the next number: "Halloween 2".
  const lc = (s) => s.toLowerCase();
  const taken = (n) => known.some((k) => lc(k) === lc(`/sets/${n}.txt`));
  const free = (n) => { let i = 2; while (taken(`${n} ${i}`)) i++; return `${n} ${i}`; };
  async function copyTo(p, n, move) {
    if (move) await (await smartMod()).idle();
    const to = `/sets/${n}.txt`, text = await app.text(p.path, true);
    if (text == null) throw new Error('not served by this source');
    const ov = await new Overrides().load(p.path);
    if (!ov.ok) throw new Error('its favourites could not be read');
    await app.src.saveSet(n, text);
    app.texts.set(to, text);
    const meta = app.wasm.renderOverrides([...ov.map.values()]);
    if (meta) await app.src.writeFile(fileFor(to), meta);
    await (await smartMod()).copy(p.path, to, move);
    if (!move) return;
    if (app.device && app.activeSet === p.value) { await app.src.putConfig({ set: n }); await app.refreshStatus(); await app.refreshConfig(); }
    if (app.src.schedule) {
      const s = await app.src.schedule();
      if (s.rules?.some((r) => r.set === p.value)) await app.src.putSchedule({ ...s, rules: s.rules.map((r) => (r.set === p.value ? { ...r, set: n } : r)) });
    }
    await app.src.deleteFile(p.path);
    // (Even one that says nothing any more, such as earlier firmware's lines only.)
    await app.src.deleteFile(fileFor(p.path)).catch((e) => { if (e.status !== 404 && e.name !== 'NotFoundError') throw e; });
    app.texts.delete(p.path);
    // Open in the editor: it follows, its overrides read from the new name.
    const d = app.draft;
    if (d.path === p.path) { d.path = to; d.name = n; d.smart = (await smartMod()).get(to); emit('draft'); }
  }
  async function rename(p) {
    const n = (prompt(`Rename “${p.label}” to`, p.value) || '').trim().replace(/\.txt$/, '');
    if (!n || n === p.value) return;
    if (!SET_NAME.test(n)) { toast(SET_NAME_RULE, 'bad'); return; }
    const same = lc(n) === lc(p.value);
    if (!same && taken(n)) { toast(`a playlist named “${n}” exists`, 'bad'); return; }
    try {
      // A change of case alone names the same file on the card: by way of another name.
      if (same) { const t = free(n); await copyTo(p, t, true); await copyTo({ path: `/sets/${t}.txt`, value: t }, n, true); }
      else await copyTo(p, n, true);
      toast(`Renamed “${p.label}” to “${n}”`, 'ok');
    } catch (e) { toast('rename: ' + e.message, 'bad'); }
    loadSets();
  }
  async function duplicate(p) {
    const n = free(p.root ? p.label : p.value);
    try { await copyTo(p, n, false); toast(`Duplicated as “${n}”`, 'ok'); } catch (e) { toast('duplicate: ' + e.message, 'bad'); }
    loadSets();
  }

  on('config', () => loadSets());
  const ed = () => el.classList.contains('ed');

  await loadSets();
  return {
    show() { edi?.show(); if (app.device) app.refreshStatus().catch(() => {}); refreshAll(); },
    // Another view: the editor's changes are asked about on the way.
    hide() { if (ed()) edi?.left(); },
    async sub(sub) {
      el.classList.toggle('ed', sub == 'edit');
      if (!ed()) { pushed = false; closeMenu(); edQ?.then((e) => e.left()); } else { (await editor()).header(); scrollTo(0, 0); }
      for (const r of setsList.children) r.classList.toggle('cur', ed() && r.dataset.path === app.draft.path);
      edQ?.then((e) => e.shown());
    },
  };
}
