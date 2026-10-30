// The share page (/f/#formula=…&name=…&a=…, or a short link's /f/<id>?a=…):
// one formula, playing in the
// web app's LED preview, its code, its sliders, and the ways on: add it to
// my panel, open it on my display, try it in the demo, copy, submit to the
// gallery, a QR code.
// Add to my panel (also the gallery's tiles: `…&add=1`, or `?add=1` on a
// short link) and Open on my display are the same hand-off: a top-level
// navigation to the panel's own web app, `http://<addr>/#formula=…` (or
// `#bundled=<stem>`), which Add marks `&add=1`: the panel saves it and
// offers Play now, Add to a playlist, Use as the look. An https page can
// navigate to http, never fetch from it (until S8's Local Network Access).
// The demo's files do the work: the share link's encoder and decoder
// (js/psikit.js, the app's own), the LED preview (js/preview.js and its
// wasm) and Psi (pkg-psi), so a frame here is the panel's and looks like
// the app's. Everything a link brings is shown as text, never as HTML.

import init from '../demo/pkg/opendmd_web.js';
import { link, parse, controls, control, fit, Q, about, TIP, est, marks, sample, declText } from '../demo/js/psikit.js';
import { qr } from './qr.js';

const $ = (id) => document.getElementById(id);
const store = {
  get(k) { try { return localStorage.getItem('odmd.' + k); } catch { return null; } },
  set(k, v) { try { localStorage.setItem('odmd.' + k, v); } catch { /* private mode */ } },
};
// The gallery's repository, where Submit opens GitHub's "new file" page.
const REPO = 'https://github.com/opendmd/opendmd.github.io';
// GitHub refuses very long addresses: past this, the file is copied instead.
const MAX_URL = 6000;
const D = 20; // the panel's frame: 50 a second

// A short link's page (/f/<id>, written by the site's Worker) carries its
// Formula in #f-data and slider values in the query; a long link carries
// both in the hash.
const data = (() => { try { return JSON.parse($('f-data')?.textContent || 'null'); } catch { return null; } })();
const { f, cut, has, bundled } = parse(data ? '#' + new URLSearchParams([['formula', data.src], ...(data.name ? [['name', data.name]] : []),
  ...[...new URLSearchParams(location.search)].filter(([k]) => /^[abc]$/.test(k))]) : location.hash);
// Its short link's id, once it has one.
let shortId = data?.id || null;
// Arrived to be added to the panel (a gallery tile's Add to my panel).
const intent = /(^#|&)add=1(&|$)/.test(location.hash) || new URLSearchParams(location.search).get('add') === '1';
// The panel's address, asked once (Add or Open), then remembered.
const addr = () => store.get('display');
// `#formula=…` or `#bundled=<stem>` (the panel's own share links), marked
// for Add.
const toPanel = (a, hash, add) => { location.href = `http://${a}/${hash}${add ? '&add=1' : ''}`; };
// Another link pasted into this tab: start again with it.
addEventListener('hashchange', () => location.reload());
const here = () => new URL('./', location.href).href;

// A bundled Formula by name: the gallery's copy of its file, the header cut.
const named = bundled && fetch(`../formulas/src/${bundled}.psi`).then((r) => (r.ok ? r.text() : '')).catch(() => '')
  .then((t) => { const l = t.split('\n'); let n = 0; while (n < l.length && /^-- (author|licen[cs]e|bundle)/.test(l[n])) n++; f.src = l.slice(n).join('\n'); });
if (!has || (!bundled && !f.src.trim())) {
  $('f-none').hidden = false;
} else {
  // Add to my panel from the gallery, the address known: straight on (the
  // page itself still loads, without the mark, for the way back).
  if (intent && addr()) {
    history.replaceState(null, '', location.pathname + location.search.replace(/[?&]add=1\b/, '').replace(/^&/, '?') + location.hash.replace(/&add=1\b/, ''));
    toPanel(addr(), link({ ...f, stem: bundled }, ''), true);
  }
  // #f is aria-busy until start() has wired everything (the preview, the
  // buttons, the highlighted code): what a test waits for, not #f-name.
  start().catch((e) => { $('f-cap').textContent = 'This page could not load: ' + (e.message || e); })
    .finally(() => $('f').removeAttribute('aria-busy'));
}

async function start() {
  if (named) {
    await named;
    if (!f.src.trim()) { $('f-none').hidden = false; return; }
    f.stem = bundled;
  }
  $('f').hidden = false;
  $('f-name').textContent = f.name;
  document.title = `${f.name} · OpenDMD`;
  // The code as plain text at once, with the name: readable while the wasm
  // loads (or if it never does); highlighted below once Psi is up.
  $('f-code').textContent = f.src + '\n';
  if (cut) $('f-cap').textContent = 'This link was too long: only the first 16,384 characters of the formula are here.';

  await init();
  const [{ PanelPreview }, P] = await Promise.all([import('../demo/js/preview.js'), import('../demo/pkg-psi/opendmd_psi_web.js')]);
  const mem = (await P.default()).memory;
  const ok = P.compile(f.src);
  const i = ok ? P.info() : [];
  const bad = ok ? null : P.errorAt();
  $('f-code').innerHTML = marks(f.src, bad).html + '\n'; // marks() escapes the text
  const ctl = ok ? controls(P, f.src) : null;
  fit(ctl, f);
  // What it says of itself (its `name` and `about` lines, read by the
  // compiler): its title and the line under it.
  if (ctl?.name) {
    f.name = ctl.name;
    $('f-name').textContent = f.name;
    document.title = `${f.name} · OpenDMD`;
  }
  $('f-about').textContent = ctl?.about || '';
  $('f-about').hidden = !ctl?.about;
  // The slots it declares or reads: the link's values.
  const mask = (i[5] || 0) | (ctl?.controls || []).reduce((m, c) => m | (1 << 'abc'.indexOf(c.slot)), 0);
  // One that draws over a GIF is shown over the sample picture, as in the app.
  const over = !!(mask & 32);
  if (ok) {
    $('f-cap').textContent = (cut ? $('f-cap').textContent + ' ' : '') + about(i);
    $('f-cap').title = TIP(i);
  } else {
    const line = f.src.slice(0, bad[0]).split('\n').length;
    const msg = P.error();
    $('f-cap').textContent = `This formula doesn’t work: line ${line}: ${msg.replace(/`/g, '')}`;
    $('f-cap').classList.add('bad');
    // Its code, linked to the language reference's entry.
    import('../demo/js/psierr.js').then((m) => {
      const r = m.refOf(msg, new URL('../', here()).href);
      if (!r) return;
      const a = document.createElement('a');
      a.className = 'ecode'; a.href = r.href; a.textContent = r.code; a.title = 'What this error means';
      $('f-cap').append(' ', a);
    }).catch(() => {});
  }

  // The preview: the frame at a whole number of 20 ms frames into the loop.
  const pv = new PanelPreview($('f-panel'));
  pv.p.setTransition('none', 0);
  // (A formula that goes on without repeating, loop 0: no wrap.)
  const n = ok && i[4] === 0 ? Infinity : Math.max(1, Math.floor((i[4] || 8000) / D));
  let t0 = performance.now(), last = -1, dirty = true;
  const tick = (now) => {
    const k = Math.floor((now - t0) / D) % n;
    if (ok && (k !== last || dirty)) {
      last = k; dirty = false;
      const d = new Date();
      if (over) new Uint8Array(mem.buffer, P.under(), 12288).set(sample());
      P[over ? 'frameOver' : 'frame'](k * D, d.getHours(), d.getMinutes(), d.getSeconds(), Q(f.a), Q(f.b), Q(f.c));
      pv.p.showRgb(new Uint8Array(mem.buffer, P.pixels(), 12288));
      $('f-panel').dataset.frames = -~$('f-panel').dataset.frames;
    }
    requestAnimationFrame(tick);
  };
  requestAnimationFrame(tick);

  // Its controls (its `control` lines; a 0..1 slider for a, b or c read without one).
  for (const c of ctl?.controls || []) $('f-sl').append(control(c, f, () => { dirty = true; links(); }, 'f-sl-' + c.slot));

  // The links: this page, the demo, the display (the device's own share
  // links open the formula in its editor: `#formula=…`).
  const long = () => link(f, here(), ctl);
  const hash = () => long().slice(here().length);
  // The short link (/f/<id>, with the sliders that moved as a query), once
  // there is one; else the long one.
  const vals = () => { const q = new URLSearchParams(link(f, '', ctl).slice(1)); q.delete('formula'); q.delete('name'); return String(q); };
  const own = () => (shortId ? `${location.origin}/f/${shortId}${vals() ? '?' + vals() : ''}` : long());
  let qrFor = '';
  function links() {
    $('f-demo').href = new URL('../demo/', location.href).href + hash();
    clearTimeout(links.t);
    links.t = setTimeout(() => { if (own() !== qrFor) drawQr(qrFor = own()); }, 300);
    sub();
  }

  // Add to my panel (saved there) and Open on my display (its editor): the
  // address asked once, then remembered.
  const where = () => { const a = addr(); $('f-where').hidden = !a; $('f-where-addr').textContent = a || ''; };
  where();
  let adding = true;
  const go = (a) => toPanel(a, hash(), adding);
  $('f-add').onclick = () => { adding = true; if (addr()) go(addr()); else ask(); };
  $('f-open').onclick = () => { adding = false; if (addr()) go(addr()); else ask(); };
  $('f-change').onclick = () => ask();
  function ask() {
    $('f-ask').hidden = false;
    $('f-go').textContent = adding ? 'Add' : 'Open';
    $('f-ask-what').textContent = adding ? 'The Formula is saved on your display; then you can play it now, add it to a playlist, or use it as the look.'
      : 'The Formula opens in the display’s editor, where you can change it and save it.';
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
      $('f-addr-err').textContent = `${a} doesn’t look like a display on your own network (like opendmd.local or 192.168.1.50): the formula would go to that site. Press ${adding ? 'Add' : 'Open'} again if you’re sure.`;
      return;
    }
    store.set('display', a);
    where();
    go(a);
  };

  const flash = (btn, text) => { const was = btn.textContent; btn.textContent = text; setTimeout(() => { btn.textContent = was; }, 1500); };
  $('f-copy-code').onclick = async (e) => flash(e.currentTarget, (await copy(f.src)) ? 'Copied' : 'Select and copy it');
  // Copy link: a short link, made by the site (POST /api/f stores the
  // source; the same source is always the same link), with a `name` line
  // added when only the long link named it. Offline or refused: the long
  // link, which needs nothing from the site.
  async function shorten() {
    if (shortId) return true;
    let src = f.src;
    if (ok && !ctl?.name && f.name && f.name !== 'Shared formula') src = P.declare(src, 'name', 0, `name "${declText(f.name, 40)}"`);
    try {
      const r = await fetch('/api/f', { method: 'POST', headers: { 'Content-Type': 'text/plain; charset=utf-8' }, body: src });
      const j = r.ok ? await r.json() : null;
      if (!j?.id) return false;
      shortId = j.id;
      links();
      return true;
    } catch { return false; }
  }
  $('f-copy-link').onclick = async (e) => {
    const btn = e.currentTarget;
    let made = true;
    const url = shorten().then((s) => { made = s; return own(); });
    flash(btn, (await copyLater(url)) ? (made ? 'Copied' : 'Copied a long link') : 'Select and copy it');
  };

  // The QR code: this page's link (with the sliders as they are).
  function drawQr(url) {
    const m = qr(url), box = $('f-qr');
    box.replaceChildren();
    $('f-qr-cap').textContent = m ? 'Scan to open this page on your phone.' : 'This link is too long for a QR code: copy it instead.';
    if (!m) return;
    const NS = 'http://www.w3.org/2000/svg', s = m.length + 8;
    const svg = document.createElementNS(NS, 'svg'), path = document.createElementNS(NS, 'path');
    svg.setAttribute('viewBox', `0 0 ${s} ${s}`);
    svg.setAttribute('role', 'img');
    svg.setAttribute('aria-label', 'QR code of this page’s link');
    let d = '';
    m.forEach((row, y) => row.forEach((dark, x) => { if (dark) d += `M${x + 4} ${y + 4}h1v1h-1z`; }));
    path.setAttribute('d', d);
    svg.append(path);
    box.append(svg);
  }

  // Submit to the gallery: the file (the gallery's header, `-- author:` and
  // `-- licence:`, then the formula with its `name` and `about` lines set
  // from the fields: pkg-psi's `declare`, the editor's own edit), checked
  // here as the gallery's checker will, then GitHub's "new file" page with
  // it filled in. Its sliders start where its `control` lines say.
  const slug = (s) => s.normalize('NFKD').replace(/[̀-ͯ]/g, '').toLowerCase().replace(/[^a-z0-9]+/g, '-').replace(/^-+|-+$/g, '').slice(0, 48).replace(/-+$/, '');
  // The characters the checker refuses (controls, invisible, direction-changing, private use).
  const BAD = /[\0-\x08\x0a-\x1f\x7f-\xa0\xad\u034f\u061c\u115f\u1160\u17b4\u17b5\u180b-\u180e\u2000-\u200f\u2028-\u202f\u205f-\u206f\u3000\u3164\ufe00-\ufe0f\ufeff\uffa0\ufff0-\uffff\ue000-\uf8ff\udb40-\udb43\udb80-\udbff]/;
  let touched = false;
  $('s-name').value = f.name.slice(0, 40);
  $('s-desc').value = ctl?.about || '';
  $('s-author').value = store.get('author') || '';
  $('s-file').value = slug(f.name);
  for (const id of ['s-name', 's-desc', 's-author', 's-lic', 's-file']) $(id).addEventListener('input', () => { if (id === 's-file') touched = true; sub(); });
  $('s-name').addEventListener('input', () => { if (!touched) $('s-file').value = slug($('s-name').value); sub(); });
  $('s-author').addEventListener('change', () => store.set('author', $('s-author').value.trim()));
  const v = (id) => $(id).value.trim().replace(/\s+/g, ' ');
  function file() {
    let src = f.src.replace(/\r\n?/g, '\n').replace(/\n*$/, '\n');
    if (ok) {
      for (const [w, id, n] of [['name', 's-name', 40], ['about', 's-desc', 120]]) {
        const t = declText(v(id), n);
        src = P.declare(src, w, 0, t ? `${w} "${t}"` : '');
      }
    }
    return `-- author: ${v('s-author')}\n-- licence: ${$('s-lic').value}\n` + src;
  }
  function problems() {
    const out = [], fields = [['name', 's-name', 40], ['description', 's-desc', 120], ['author', 's-author', 40]];
    for (const [what, id, max] of fields) {
      const v = $(id).value.trim();
      if (!v) out.push(`Give it a ${what === 'author' ? 'name or handle for you' : what}.`);
      else if ([...v].length > max) out.push(`The ${what} can be ${max} characters at most.`);
      if (/[<>]/.test(v)) out.push(`The ${what} can’t contain < or >.`);
      else if (id !== 's-author' && /["\\]/.test(v)) out.push(`The ${what} can’t contain " or \\.`);
      if (BAD.test(v)) out.push(`The ${what} has an invisible or special character in it.`);
    }
    if (!/^[a-z0-9]+(-[a-z0-9]+)*$/.test($('s-file').value) || ['readme', 'index'].includes($('s-file').value)) out.push('The file name can be lower-case letters, digits and single hyphens.');
    if (!ok) out.push('The formula has to work first.');
    else if (est(i) > 8000) out.push('This formula is too heavy for the panel: the gallery takes ones under 8 ms a frame.');
    if (f.src.replace(/\r\n?/g, '\n').split('\n').some((l) => BAD.test(l.replace(/\t/g, '')))) out.push('The formula has an invisible or special character in it.');
    if (new TextEncoder().encode(file()).length > 4096) out.push('The file would be over 4 KB: the gallery takes shorter formulas.');
    return out;
  }
  function sub() {
    if ($('f-sub').hidden) return;
    const text = file(), ps = problems();
    $('s-text').textContent = text;
    $('s-err').textContent = ps.join(' ');
    const url = `${REPO}/new/main/gallery?filename=${encodeURIComponent($('s-file').value + '.psi')}&value=${encodeURIComponent(text)}`;
    const long = url.length > MAX_URL;
    $('s-go').href = ps.length ? '#' : long ? `${REPO}/new/main/gallery?filename=${encodeURIComponent($('s-file').value + '.psi')}` : url;
    $('s-go').toggleAttribute('aria-disabled', ps.length > 0);
    $('s-long').hidden = !long;
  }
  $('s-go').onclick = (e) => { if (problems().length) e.preventDefault(); };
  $('s-copy').onclick = async (e) => flash(e.currentTarget, (await copy(file())) ? 'Copied' : 'Select and copy it');
  const openSub = () => { $('f-sub').hidden = false; sub(); };
  $('f-submit').onclick = () => { openSub(); $('f-sub').scrollIntoView({ behavior: 'smooth', block: 'start' }); $('s-desc').focus({ preventScroll: true }); };
  if (/(^#|&)submit(=|&|$)/.test(location.hash)) openSub();
  links();
  // Add to my panel from the gallery with no address yet: ask for it.
  if (intent && !addr()) ask();
}

// copy() of text still to come (the short link): a ClipboardItem holding a
// promise keeps the click's permission across the wait (Safari needs
// that); else copy it once it is here.
async function copyLater(p) {
  if (window.ClipboardItem && navigator.clipboard?.write) {
    try {
      await navigator.clipboard.write([new ClipboardItem({ 'text/plain': p.then((t) => new Blob([t], { type: 'text/plain' })) })]);
      return true;
    } catch { /* below */ }
  }
  return copy(await p);
}

async function copy(text) {
  try { await navigator.clipboard.writeText(text); return true; } catch { /* the old way */ }
  const t = Object.assign(document.createElement('textarea'), { value: text, readOnly: true });
  t.style.cssText = 'position:fixed;opacity:0';
  document.body.append(t);
  t.select();
  const done = document.execCommand('copy');
  t.remove();
  return done;
}
