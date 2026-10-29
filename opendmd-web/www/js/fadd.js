// Add to my panel: the site's gallery, a Formula's page or its short link
// hands a Formula to the panel's web app (`#formula=…&add=1`, or
// `#bundled=<stem>&add=1`; imported by js/formulas.js then, on a device;
// its own size line). One of yours or the gallery's is saved on the panel
// (js/fstore.js: /formulas/<stem>.psi; the same text already there is not
// saved twice), a bundled one needs nothing (`@formula:<stem>`); then a
// small sheet: Play now (POST /api/play), Add to a playlist (the Formulas
// view's picker) and Use as the look (one that declares `look`). What goes
// wrong is said in plain words, in the same sheet.

import { app } from './state.js';
import { h, toast } from './util.js';
import { popup, closeMenu } from './overrides.js';
import { declText, path as inline, MAX_NAME } from './psikit.js';
import * as S from './fstore.js';

// The panel keeps a Formula's text without its comment and blank lines, up
// to 8 KB (fx_psi::read_text, MAX_FORMULA_SOURCE).
const KEPT = 8192;
const kept = (src) => src.split('\n').map((l) => l.trim()).filter((l) => l && !l.startsWith('--') && !l.startsWith('//'))
  .reduce((n, l) => n + new TextEncoder().encode(l).length + 1, 0);

// `a`: psikit `parse` of the link ({f, cut, bundled}); `x`: the view's parts.
export async function add(a, x) {
  const { f, cut, bundled } = a;
  for (const k of 'abc') if (f[k] == null) delete f[k];
  if (bundled) {
    const ex = x.EX.find((e) => e.stem === bundled);
    if (!ex) return fail('This panel doesn’t come with that Formula yet: update OpenDMD to get it.');
    for (const k of 'abc') if (f[k] != null) ex[k] = f[k];
    x.fit(x.ctlOf(ex), ex);
    x.show(ex, false);
    return sheet({ ...ex, name: x.nameOf(ex) }, x, 'It comes with your panel', 'Nothing to save: it’s built in.');
  }
  const src0 = f.src.replace(/\r\n?/g, '\n');
  if (cut) return fail('This Formula is too long for the panel: its link was cut short.', x, f);
  if (kept(src0) > KEPT) return fail(`This Formula is too long for the panel: about ${Math.ceil(kept(src0) / 1024)} KB without its comments, and the panel takes 8 KB.`, x, f);
  if (!x.info(src0).length) {
    const at = x.P.errorAt?.(), line = at ? src0.slice(0, at[0]).split('\n').length : 0;
    return fail(`This Formula doesn’t work, so it wasn’t saved: ${line ? `line ${line}: ` : ''}${x.P.error().replace(/`/g, '')}`, x, f);
  }
  if (!S.onCard()) {
    const sd = (await app.refreshStatus?.().catch(() => null) || app.status)?.sd;
    return fail(sd?.mounted ? 'The panel can’t read its card just now, and Formulas are saved there. Settings › Device says more.'
      : 'There’s no card in the panel: Formulas are saved on it. Put the card in and try again.', x, f);
  }
  // Its name: its own `name` line, else the link's, written in as one (as
  // the editor's save does), so the panel and playlists show it.
  const ctl = x.ctlOf({ src: src0 }), given = f.name && f.name !== 'Shared formula' ? f.name : '';
  const name = ctl?.name || given || 'Shared formula';
  const src = ctl?.name || !given ? src0 : x.P.declare(src0, 'name', 0, `name "${declText(given, MAX_NAME)}"`) || src0;
  const vals = { a: f.a, b: f.b, c: f.c };
  let rec = S.list().find((r) => r.src === src || r.src === src0), title = 'Already on your panel', note = 'It was saved there before: nothing new was added.';
  if (rec) rec = { ...rec, ...Object.fromEntries(Object.entries(vals).filter(([, v]) => v != null)) };
  else {
    try {
      rec = await S.put({ name, src, ...vals });
    } catch (e) {
      return fail(`Couldn’t save it on the panel: ${e.status == 507 || /full|space/i.test(e.message) ? 'its card is full' : e.message || 'no answer'}.`, x, f);
    }
    title = 'Added to your panel';
    note = 'Saved under Your formulas.';
  }
  x.show(S.list().find((r) => r.file === rec.file) || rec, true);
  return sheet({ ...rec, name }, x, title, note);
}

// The sheet: what happened, and the three ways on.
function sheet(f, x, title, note) {
  const ctl = x.ctlOf(f), line = S.line(f, ctl, inline(f, ctl));
  const name = f.name || 'Formula';
  const b = (id, text, fn) => h('button.mi', { id, onclick: fn }, text);
  popup(h('div.menu.wide', { id: 'psi-added', role: 'dialog', 'aria-label': title },
    h('div.mh', title), h('p', { id: 'psi-added-name', style: 'margin:4px 8px' }, h('b', name)), h('p.cap', { style: 'margin:0 8px 8px' }, note),
    b('psi-added-play', 'Play now', () => { closeMenu(); play(line, name); }),
    b('psi-added-pl', 'Add to a playlist…', (e) => x.toList(f, e.currentTarget)),
    ctl?.look && b('psi-added-look', 'Use as the look', () => { closeMenu(); x.setLook(f); }),
    b('psi-added-done', 'Done', closeMenu)), innerWidth / 2 - 220, 96);
}

// Play now: on the panel until the playlist moves on.
async function play(path, name) {
  try {
    await app.src.json('POST', '/api/play', { path }, 8000);
    toast(`On the panel: ${name}`, 'ok');
  } catch (e) {
    const s = e.status, why = e.message?.replace(/^\d+: /, '');
    toast(s == 404 ? `The panel can’t find “${name}”: update OpenDMD to play it` : s == 400 ? `The panel can’t play “${name}”: ${why}`
      : s == 413 ? 'This Formula is too long for the panel' : s == 503 ? 'The panel has nothing playing to interrupt just now'
        : s ? 'Play now: ' + e.message : 'Couldn’t reach the panel', 'bad');
  }
}

// It couldn't be added: why, and (one with text) Open in the editor.
function fail(why, x, f) {
  popup(h('div.menu.wide', { id: 'psi-added', role: 'alertdialog', 'aria-label': 'Not added' },
    h('div.mh', 'Couldn’t add it to your panel'), h('p.bad', { id: 'psi-added-why', style: 'margin:4px 8px 8px' }, why),
    f && h('button.mi', { id: 'psi-added-edit', onclick: () => { closeMenu(); x.edit(f); } }, 'Open it in the editor'),
    h('button.mi', { id: 'psi-added-done', onclick: closeMenu }, 'Close')), innerWidth / 2 - 220, 96);
}
