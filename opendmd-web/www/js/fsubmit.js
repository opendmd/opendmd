// Submit to the gallery: one form for the web app's editor (a sheet) and the
// site's Formula page (a card), imported when it first opens (its own size
// line). The file is the gallery's header (`-- author:`, `-- licence:`)
// and the Formula with its `name` and `about` lines set from the fields
// (pkg-psi's `declare`, the editor's own edit), checked here as the
// gallery's checker will; then GitHub's "new file" page opens with it filled
// in (the gallery is a public repository, added to by pull request and
// checked by a maintainer). Sharing a Formula is this: there are no
// anonymous links.

import { h, store } from './util.js';
import { declText, est } from './psikit.js';

// The gallery's repository, where Submit opens GitHub's "new file" page.
const REPO = 'https://github.com/opendmd/opendmd.github.io';
// GitHub refuses very long addresses: past this, the file is copied instead.
const MAX_URL = 6000;
// The gallery's cost limit (opendmd-gallery MAX_US, the Worker's check.ts): psikit `est`, the tier (the JIT's estimate where it takes the Formula), in µs a frame.
const MAX_US = 10000;
// The characters the checker refuses (controls, invisible, direction-changing, private use).
const BAD = /[\0-\x08\x0a-\x1f\x7f-\xa0\xad\u034f\u061c\u115f\u1160\u17b4\u17b5\u180b-\u180e\u2000-\u200f\u2028-\u202f\u205f-\u206f\u3000\u3164\ufe00-\ufe0f\ufeff\uffa0\ufff0-\uffff\ue000-\uf8ff\udb40-\udb43\udb80-\udbff]/;
const slug = (s) => s.normalize('NFKD').replace(/[̀-ͯ]/g, '').toLowerCase().replace(/[^a-z0-9]+/g, '-').replace(/^-+|-+$/g, '').slice(0, 48).replace(/-+$/, '');

// `P`: pkg-psi; `get()`: the Formula now, {src, name, ok, info, ctl}.
// Returns {el, refresh()}: the form, and its file again (after an edit).
export function form(P, get) {
  const x0 = get(), name0 = x0.ctl?.name || x0.name || '';
  const inp = (id, max, more = {}) => h('input', { type: 'text', id, maxLength: max, autocomplete: 'off', ...more });
  const name = inp('s-name', 40, { value: name0.slice(0, 40) }), desc = inp('s-desc', 120, { placeholder: 'One line: what it shows', value: x0.ctl?.about || '' });
  const author = inp('s-author', 40, { autocomplete: 'nickname', placeholder: '@you', value: store.get('author', '') || '' });
  const lic = h('select', { id: 's-lic' }, h('option', { value: 'CC0-1.0' }, 'CC0: anyone may use it, no credit needed'), h('option', { value: 'CC-BY-4.0' }, 'CC BY 4.0: anyone may use it, with credit'));
  const file = inp('s-file', 48, { spellcheck: false, value: slug(name0) });
  const err = h('p.cap.bad', { id: 's-err', 'aria-live': 'polite' }), text = h('pre.psi-file', { id: 's-text', 'aria-label': 'The file' });
  const go = h('a.btn.pri', { id: 's-go', target: '_blank', rel: 'noopener', onclick: (e) => { if (problems(get()).length) e.preventDefault(); } }, 'Open on GitHub');
  const flash = (b, t) => { const was = b.textContent; b.textContent = t; setTimeout(() => { b.textContent = was; }, 1500); };
  const cp = h('button', { id: 's-copy', onclick: async (e) => flash(e.currentTarget, (await copy(fileText(get()))) ? 'Copied' : 'Select and copy it') }, 'Copy the file');
  const long = h('p.cap', { id: 's-long', hidden: true }, 'This one is too long to hand to GitHub in a link: copy the file, then open GitHub, name the file as above and paste it in.');
  const lab = (f, t) => h('label', { for: f }, t);
  const el = h('div.sub', { id: 's-form' },
    h('p.cap', 'Formulas in the gallery are files in a public GitHub repository, added by pull request. Fill this in and GitHub opens with the file ready: it makes your own copy and the pull request for you (you need a free GitHub account). A maintainer checks it before it appears.'),
    h('div.grid', lab('s-name', 'Name'), name, lab('s-desc', 'Description'), desc, lab('s-author', 'Your name or GitHub handle'), author,
      lab('s-lic', 'Licence'), lic, lab('s-file', 'File name'), h('span.file', 'gallery/', file, '.psi')),
    err, text, h('div.row.acts', go, cp), long);
  let touched = false;
  for (const i of [name, desc, author, lic, file]) i.addEventListener('input', () => { if (i === file) touched = true; refresh(); });
  name.addEventListener('input', () => { if (!touched) file.value = slug(name.value); refresh(); });
  author.addEventListener('change', () => store.set('author', author.value.trim()));
  const v = (i) => i.value.trim().replace(/\s+/g, ' ');
  function fileText(x) {
    let src = x.src.replace(/\r\n?/g, '\n').replace(/\n*$/, '\n');
    if (x.ok) {
      for (const [w, i, n] of [['name', name, 40], ['about', desc, 120]]) {
        const t = declText(v(i), n);
        src = P.declare(src, w, 0, t ? `${w} "${t}"` : '');
      }
    }
    return `-- author: ${v(author)}\n-- licence: ${lic.value}\n` + src;
  }
  function problems(x) {
    const out = [];
    for (const [what, i, max] of [['name', name, 40], ['description', desc, 120], ['author', author, 40]]) {
      const t = i.value.trim();
      if (!t) out.push(`Give it a ${what === 'author' ? 'name or handle for you' : what}.`);
      else if ([...t].length > max) out.push(`The ${what} can be ${max} characters at most.`);
      if (/[<>]/.test(t)) out.push(`The ${what} can’t contain < or >.`);
      else if (i !== author && /["\\]/.test(t)) out.push(`The ${what} can’t contain " or \\.`);
      if (BAD.test(t)) out.push(`The ${what} has an invisible or special character in it.`);
    }
    if (!/^[a-z0-9]+(-[a-z0-9]+)*$/.test(file.value) || ['readme', 'index'].includes(file.value)) out.push('The file name can be lower-case letters, digits and single hyphens.');
    if (!x.ok) out.push('The formula has to work first.');
    else if (est(x.info) > MAX_US) out.push('This formula is too heavy for the panel: the gallery takes ones up to 10 ms a frame.');
    if (x.src.replace(/\r\n?/g, '\n').split('\n').some((l) => BAD.test(l.replace(/\t/g, '')))) out.push('The formula has an invisible or special character in it.');
    if (new TextEncoder().encode(fileText(x)).length > 20480) out.push('The file would be over 20 KB: the gallery takes shorter formulas.');
    const n = x.ok ? P.program(x.src).length : 0;
    if (n > 4500) out.push(`This formula is too long for the panel to take in a link: ${n} bytes once prepared, 4500 at most. Cut tables or repeated steps (comments and spaces cost nothing).`);
    return out;
  }
  function refresh() {
    const x = get(), t = fileText(x), ps = problems(x);
    text.textContent = t;
    err.textContent = ps.join(' ');
    const base = `${REPO}/new/main/gallery?filename=${encodeURIComponent(file.value + '.psi')}`, url = `${base}&value=${encodeURIComponent(t)}`;
    const tooLong = url.length > MAX_URL;
    go.href = ps.length ? '#' : tooLong ? base : url;
    go.toggleAttribute('aria-disabled', ps.length > 0);
    long.hidden = !tooLong;
  }
  refresh();
  return { el, refresh, focus: () => desc.focus({ preventScroll: true }) };
}

export async function copy(text) {
  try { await navigator.clipboard.writeText(text); return true; } catch { /* not a secure context: the old way */ }
  const t = h('textarea', { value: text, readOnly: true, style: 'position:fixed;opacity:0' });
  document.body.append(t);
  t.select();
  const ok = document.execCommand('copy');
  t.remove();
  return ok;
}
