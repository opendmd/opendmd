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

import init from '../demo/pkg/opendmd_web.js';
import { from, unheader, vals, num, READY, REMIX, MAX_SRC } from '../demo/js/psikit.js';

const $ = (id) => document.getElementById(id);
const store = {
  get(k) { try { return localStorage.getItem('odmd.' + k); } catch { return null; } },
  set(k, v) { try { localStorage.setItem('odmd.' + k, v); } catch { /* private mode */ } },
  drop(k) { try { localStorage.removeItem('odmd.' + k); } catch { /* private mode */ } },
};

const ref = from(location.hash);
const opening = !ref?.gallery && /^#open(&|$)/.test(location.hash);
// Arrived to go to the panel (a gallery tile's Open on my panel).
const intent = /(^#|&)panel=1(&|$)/.test(location.hash);
// The panel's address, asked once, then remembered.
const addr = () => store.get('display');
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
  const got = await load();
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
    acts: [open, copyCode, submit],
    note: 'Open on my panel saves it on your own panel, under Your formulas, and opens it there, where you can change it, show it and add it to a playlist. To share it, submit it to the gallery.',
    changed: () => { clearTimeout(changed.t); changed.t = setTimeout(changed, 150); },
  });
  let subForm = null;
  function changed() {
    const x = ed.now();
    const name = x.ctl?.name || x.name || 'Formula';
    $('f-name').textContent = name;
    document.title = `${name} · OpenDMD`;
    $('f-about').textContent = x.ctl?.about || '';
    $('f-about').hidden = !x.ctl?.about;
    subForm?.refresh();
  }
  changed();
  const say = (t) => { $('f-cap').hidden = !t; $('f-cap').textContent = t || ''; };

  // Open on my panel: the address asked once, then remembered.
  const where = () => { const a = addr(); $('f-where').hidden = !a; $('f-where-addr').textContent = a || ''; };
  where();
  // (Called in the click or the form's submit: a window may open.)
  function go(a) {
    say('');
    const x = ed.now(true);
    if (!x) return;
    const v = q(x, x.ctl);
    if (ref?.gallery && x.src === orig) { location.href = `http://${a}/#gallery=${ref.gallery}${v ? '&' + v : ''}`; return; }
    const panel = new URL(`http://${a}/`).origin, w = window.open(`http://${a}/#remix`, '_blank');
    if (!w) { say('Your browser didn’t open the panel’s page: allow pop-ups for opendmd.dev, then press Open on my panel again.'); return; }
    // Its app says it is ready (once it has loaded): the Formula, to that window only.
    const on = (e) => {
      if (e.source !== w || e.origin !== panel || e.data?.type !== READY) return;
      removeEventListener('message', on);
      w.postMessage({ type: REMIX, name: x.ctl?.name || x.name || '', src: x.src, a: x.a, b: x.b, c: x.c }, panel);
    };
    addEventListener('message', on);
    setTimeout(() => removeEventListener('message', on), 120000);
  }
  open.onclick = () => { if (addr()) go(addr()); else ask(); };
  $('f-change').onclick = () => ask();
  function ask() {
    $('f-ask').hidden = false;
    $('f-addr').value = addr() || 'opendmd.local';
    $('f-addr').focus();
    $('f-addr').select();
  }
  $('f-ask').onsubmit = (e) => {
    e.preventDefault();
    // A host name or IPv4 address (an IPv6 one in brackets), maybe a port;
    // `http://` and a trailing slash typed with it are fine.
    const a = $('f-addr').value.trim().replace(/^https?:\/\//i, '').replace(/\/+$/, '');
    const okAddr = /^(\[[0-9a-f:.]+\]|[a-z0-9]([a-z0-9-]*[a-z0-9])?(\.[a-z0-9]([a-z0-9-]*[a-z0-9])?)*)(:\d{1,5})?$/i.test(a);
    $('f-addr-err').hidden = okAddr;
    $('f-addr-err').textContent = okAddr ? '' : 'That doesn’t look like an address: a name like opendmd.local, or numbers like 192.168.1.50.';
    if (!okAddr) return;
    // On this network: a .local name, a name with no dots, a private or
    // link-local address. Anything else could be anyone's: ask once.
    const host = a.replace(/:\d+$/, '').toLowerCase();
    const near = /\.local$/.test(host) || /^[^[.]+$/.test(host) || /^(10\.|192\.168\.|169\.254\.|127\.|172\.(1[6-9]|2\d|3[01])\.)/.test(host)
      || /^\[(fe80|fc|fd|::1\])/.test(host);
    if (!near && $('f-ask').dataset.sure !== a) {
      $('f-ask').dataset.sure = a;
      $('f-addr-err').hidden = false;
      $('f-addr-err').textContent = `${a} doesn’t look like a panel on your own network (like opendmd.local or 192.168.1.50): the formula would go to that site. Press Open again if you’re sure.`;
      return;
    }
    store.set('display', a);
    where();
    go(a);
  };

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
