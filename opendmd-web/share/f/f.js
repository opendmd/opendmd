// A Formula's page on the site: the web app's own Formulas editor (live
// preview as you type, errors, sliders, the cheat sheet and settings:
// js/formulas.js `editorPage`, from the demo's files), with the site's ways
// on: Open on my panel, Copy code, Submit to the gallery (js/fsubmit.js,
// the web app's form too).
//
// Which Formula: never its text in the link (anyone can craft a link). A
// gallery one by name, `/f/#gallery=<slug>` (the file beside the gallery,
// formulas/src/<slug>.psi), slider values as `&a=…`; or one the Psi
// reference opened, `/f/#open` (left in this site's own storage by the
// reference, read once).
//
// Open on my panel: the Formula saved on the panel (under Your formulas,
// by its own web app as it arrives) and open in its editor. As it came from the gallery, a top-level navigation to
// `http://<addr>/#gallery=<slug>` (the panel has it built in or fetches it
// from here); changed, a window opened on `http://<addr>/#remix`, whose app
// says it is ready (postMessage) and gets the Formula back the same way:
// nothing in the link, nothing kept on the site. An https page can open
// or navigate to http, never fetch from it. A gallery tile's Open on my
// panel comes marked `&panel=1`: with the address known it goes straight on.
//
// Never lost: the text as changed is kept in this browser per Formula (its
// gallery name, or an #open one's by its text), offered back on return (Continue where you left
// off), and Earlier versions… has it as it came, as it went to the panel
// and as it was left (js/fhist.js).

import { init } from '../demo/js/web.js';
import { from, unheader, vals, num, READY, REMIX, MAX_SRC } from '../demo/js/psikit.js';
import { addr, addressForm, handOff } from '../panel.js';

const $ = (id) => document.getElementById(id);
const store = {
  get(k) { try { return localStorage.getItem('odmd.' + k); } catch { return null; } },
  drop(k) { try { localStorage.removeItem('odmd.' + k); } catch { /* private mode */ } },
};

const ref = from(location.hash);
const opening = !ref?.gallery && /^#open(&|$)/.test(location.hash);
// Arrived to go to the panel (a gallery tile's Open on my panel).
const intent = /(^#|&)panel=1(&|$)/.test(location.hash);
const q = (f, ctl) => vals(f, ctl).slice(1);
// Another link pasted into this tab: start again with it.
addEventListener('hashchange', () => location.reload());

if (!ref?.gallery && !opening) {
  $('f-none').hidden = false;
} else {
  // Open on my panel from the gallery, the address known: straight on (the
  // page itself still loads, without the mark, for the way back).
  if (intent && addr() && ref?.gallery) {
    history.replaceState(null, '', location.pathname + location.search + location.hash.replace(/&panel=1\b/, ''));
    const v = new URLSearchParams(Object.entries(ref.f)).toString();
    location.href = `http://${addr()}/#gallery=${ref.gallery}${v ? '&' + v : ''}`;
  }
  // #f is aria-busy until start() has wired everything: what a test waits for.
  start().catch((e) => { $('f-cap').hidden = false; $('f-cap').textContent = 'This page could not load: ' + (e.message || e); })
    .finally(() => $('f').removeAttribute('aria-busy'));
}

// The Formula the link names: {name, src}, or null.
async function load() {
  if (opening) {
    const t = store.get('psi.open');
    store.drop('psi.open');
    let o = null;
    try { o = JSON.parse(t || 'null'); } catch { /* none */ }
    return typeof o?.src == 'string' ? { name: String(o.name || ''), src: o.src.slice(0, MAX_SRC) } : null;
  }
  const r = await fetch(`../formulas/src/${ref.gallery}.psi`).catch(() => null);
  if (!r?.ok) return null;
  return { name: '', src: unheader(await r.text()).slice(0, MAX_SRC) };
}

async function start() {
  const H = await import('../demo/js/fhist.js');
  let got = await load();
  // An #open one is its text's own (`o<hash>`), the last one opened kept
  // for a reload (its text is read once): the draft's start, the draft offered.
  const key = ref?.gallery ? 'g' + ref.gallery : got ? H.opened('o' + H.hash(got.src)) : H.opened(), d = key && H.draft(key);
  if (!got && opening && d?.base) got = { name: d.name, src: d.base };
  if (!got || !got.src.trim()) {
    $('f-none').hidden = false;
    $('f-none-why').textContent = opening ? 'There’s no formula waiting here: open one from the Psi language reference or the formula gallery.'
      : 'The gallery has no formula by that name. Pick another from the formula gallery.';
    return;
  }
  // As it came: unchanged, a gallery one goes to the panel by its name.
  const orig = got.src;
  $('f').hidden = false;
  await init();
  const { editorPage } = await import('../demo/js/formulas.js');
  const btn = (id, text, cls = '') => Object.assign(document.createElement('button'), { id, textContent: text, className: cls });
  const open = btn('f-open', 'Open on my panel', 'pri'), copyCode = btn('f-copy-code', 'Copy code'), submit = btn('f-submit', 'Submit to the gallery…');
  const ed = await editorPage($('f-ed'), { ...got, ...(ref?.f || {}) }, {
    acts: [open, copyCode, submit], key,
    note: 'Open on my panel saves it on your own display, under Your formulas, and opens it there, where you can change it, show it and add it to a playlist. To share it, submit it to the gallery. A Formula with phase or smooth needs OpenDMD 1.1 on the panel: if it says the name is unknown, update the panel.',
    changed: () => { clearTimeout(changed.t); changed.t = setTimeout(changed, 150); },
  });
  let subForm = null;
  // The draft as it was: kept, and offered while the page is as it loaded.
  H.keep(key, got, ref?.gallery ? 'g' : 'o');
  const old = d && d.src !== orig ? d : null;
  if (d && d.src !== orig) H.keep(key, d, 'd');
  if (old) $('f-ed').before(H.offer(old, () => ed.back(old)));
  let asked = !!old;
  function changed() {
    const x = ed.now();
    if (x.src !== orig) { asked = false; H.draft(key, { src: x.src, name: x.name, a: x.a, b: x.b, c: x.c, base: orig }); } else if (!asked) H.draft(key, null);
    const name = x.ctl?.name || x.name || 'Formula';
    $('f-name').textContent = name;
    document.title = `${name} · OpenDMD`;
    $('f-about').textContent = x.ctl?.about || '';
    $('f-about').hidden = !x.ctl?.about;
    subForm?.refresh();
  }
  changed();
  const say = (t) => { $('f-cap').hidden = !t; $('f-cap').textContent = t || ''; };
  // `phase` read, or `smooth` alone on its line (OpenDMD 1.1's words), unless the formula sets its own.
  const NEW = (s) => /(?<![\w.])phase\b|(^|;)[ \t]*smooth[ \t]*(;|$)/m.test(s) && !/(^|;)[ \t]*(phase|smooth)[ \t]*=[^=]/m.test(s);
  const OLD = 'Your panel’s web app is older than OpenDMD 1.1, and doesn’t know phase or smooth: update the panel (its Settings › Update), then press Open on my panel again.';

  // Open on my panel: the address asked once, then remembered (../panel.js).
  // (Called in the click or the form's submit: a window may open.)
  function go(a) {
    say('');
    const x = ed.now(true);
    if (!x) return;
    const v = q(x, x.ctl);
    H.keep(key, x, 'p');
    if (ref?.gallery && x.src === orig) { location.href = `http://${a}/#gallery=${ref.gallery}${v ? '&' + v : ''}`; return; }
    // Its app says it is ready (once it has loaded): the Formula, to that window only.
    const data = { type: REMIX, name: x.ctl?.name || x.name || '', src: x.src, a: x.a, b: x.b, c: x.c };
    // A panel's app from before OpenDMD 1.1 (no `feat`) can't compile `phase` or `smooth`: asked to update instead.
    const send = (r) => (!r.feat && NEW(x.src.replace(/(--|\/\/).*/g, '')) ? (say(OLD), null) : { data });
    if (!handOff(a, 'remix', READY, send)) say('Your browser didn’t open the display’s web app: allow pop-ups for opendmd.dev, then press Open on my panel again.');
  }
  const { ask } = addressForm('f', go, 'formula');
  open.onclick = () => { if (addr()) go(addr()); else ask(); };

  const fsub = await import('../demo/js/fsubmit.js');
  const flash = (b, text) => { const was = b.textContent; b.textContent = text; setTimeout(() => { b.textContent = was; }, 1500); };
  copyCode.onclick = async () => flash(copyCode, (await fsub.copy(ed.now().src)) ? 'Copied' : 'Select and copy it');
  // Submit to the gallery: the web app's form, in its card here.
  const openSub = () => {
    if (!subForm) { subForm = fsub.form(ed.P, () => ed.now()); $('f-sub').append(subForm.el); }
    $('f-sub').hidden = false;
    subForm.refresh();
  };
  submit.onclick = () => { if (!ed.now(true)) return; openSub(); $('f-sub').scrollIntoView({ behavior: 'smooth', block: 'start' }); subForm.focus(); };
  if (/(^#|&)submit(=|&|$)/.test(location.hash)) openSub();
  // Open on my panel from the gallery with no address yet: ask for it.
  if (intent && !addr()) ask();
}
