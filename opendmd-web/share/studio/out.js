// The Studio's way out (/studio/), loaded at the first download: the piece
// flattened to its frames (every shown layer, composited as doc.js shows
// it; a hidden layer is not in the GIF), the GIF's colour table, the GIF
// written by the site's own wasm module (pkg-studio, opendmd-studio-web;
// never on the panel) and downloaded.
import { Doc, N } from './doc.js';

let writer = null;
const gifWriter = () => (writer ??= import('../demo/js/wasm.js').then((w) => w.load('pkg-studio')).then(([m]) => m)
  .catch((e) => { writer = null; throw e; }));

// A body's frames, each as the keys it shows, back to back; and its colours.
export async function flatten(body, coloursOf) {
  const d = new Doc(body, coloursOf);
  await d.loadAll();
  const keys = new Uint16Array(d.frames.length * N);
  d.frames.forEach((_, f) => keys.set(d.composite(f), f * N));
  return { keys, cm: d.cols, delays: Uint32Array.from(d.frames, (f) => f.ms), plays: body.plays || 0 };
}

// The GIF of the piece as it is now, frozen first (saved and snapshotted in
// the chain, then read back: what goes out is what is kept). fit: 256
// colours picked for spread, the rest as their nearest (a GIF the panel can
// prepare and smooth).
async function make(S, fit) {
  const m = await gifWriter();
  S.done();
  const D = S.doc, shot = D.snap();
  const h = await S.freeze('', shot), frozen = h && await S.store.get(S.rec.id, h.rev - 1);
  const body = frozen?.body || await D.packed(shot, { palette: S.swatch().id });
  const { keys, cm, delays, plays } = await flatten(body, S.listOf);
  // The GIF's table: only the colours used, off first, the same colour once.
  const table = [], index = new Map(), of = new Map();
  for (const k of [...new Set(keys)].sort((a, c) => a - c)) {
    const c = k ? cm.get(k) ?? 0 : 0;
    if (!index.has(c)) { index.set(c, table.length); table.push(c); }
    of.set(k, index.get(c));
  }
  // Past 256 colours: a whole picture over 256 of them picked for spread, then the rest exactly (or, fitted, only that picture).
  const rgb = () => Uint32Array.from(keys, (k) => (k ? cm.get(k) ?? 0 : 0));
  const gif = table.length <= 256 ? m.encodeIndexed(Uint32Array.from(table), Uint8Array.from(keys, (k) => of.get(k)), delays, plays)
    : fit ? m.encodeFitted(rgb(), delays, plays) : m.encodeWide(rgb(), delays, plays);
  const n = fit ? Math.min(table.length, 256) : table.length;
  return { gif, n, name: fileName(S.nameNow() + (fit && table.length > 256 ? ' 256' : '')) };
}
// A file name of 120 characters at most, as the panel's app takes it (js/importgif.js).
const fileName = (stem) => (stem.replace(/[^\p{L}\p{N} _-]+/gu, '').trim().replace(/\s+/g, '_').slice(0, 116) || 'drawing') + '.gif';
const kb = (n) => `${(n / 1024).toFixed(1)} KB`;

export async function download(S, fit) {
  const b = document.getElementById(fit ? 's-fit' : 's-gif');
  b.disabled = true;
  try {
    const { gif, n, name } = await make(S, fit);
    const a = document.createElement('a');
    a.href = URL.createObjectURL(new Blob([gif], { type: 'image/gif' }));
    a.download = name;
    document.body.append(a);
    a.click();
    a.remove();
    setTimeout(() => URL.revokeObjectURL(a.href), 30000);
    S.root.dataset.gif = gif.length;
    S.say('');
    if (S.saved !== 'failed') S.status(S.kept ? 'kept' : 'new', `Downloaded ${name}: ${n} colour${n === 1 ? '' : 's'}, ${kb(gif.length)}`);
    await S.snaps();
  } catch (e) {
    S.say(`The GIF could not be made: ${e.message || e}`);
  } finally {
    b.disabled = false;
  }
}

// --- Send to panel: the panel's own web app opened at `#import` (its
// address asked once, ../panel.js, as /f/ and /create/ ask it), which says
// it is ready and gets the GIF by postMessage, transferred, with its name
// and the fields its Add GIFs takes: the message released firmware's web
// app already takes from Create Art (js/importgif.js: READY, IMPORT,
// MAX_GIF). The GIF is made when the sheet opens, so one too big is said
// before any window opens; Send opens the window in the click.
const READY = 'opendmd-import-ready', IMPORT = 'opendmd-import', MAX_GIF = 4 * 1024 * 1024;
const SHEET = `<form class="s-sheet" id="s-p-form"><div class="s-hd"><h2>Send to panel</h2><span class="s-sp"></span><button class="s-pill" type="button" id="s-p-close">Close</button></div>
<p class="s-cap" id="s-p-what">Making the GIF…</p>
<label class="s-f">Title <input class="s-in" id="s-p-title" maxlength="80" autocomplete="off" autofocus></label>
<label class="s-f">Platform <input class="s-in" id="s-p-platform" maxlength="16" autocomplete="off" spellcheck="false" placeholder="like SNES"></label>
<label class="s-f">Made by <input class="s-in" id="s-p-author" maxlength="40" autocomplete="off" spellcheck="false" placeholder="letters and digits"></label>
<label class="s-chk"><input type="checkbox" id="s-p-logo"> It is a logo</label>
<div class="s-row"><button class="s-cta" id="s-p-send" type="submit" disabled>Send</button></div></form>
<p class="s-cap" id="s-p-where" hidden>Your panel: <b id="s-p-where-addr"></b> · <button class="s-pill" id="s-p-change" type="button">Change</button></p>
<form id="s-p-ask" hidden><label for="s-p-addr">Your panel’s address</label>
<div class="s-row"><input class="s-in" id="s-p-addr" autocomplete="off" autocapitalize="off" spellcheck="false" placeholder="opendmd.local"><button class="s-pill s-pri" id="s-p-go">Send</button></div>
<p class="s-cap s-bad" id="s-p-addr-err" hidden></p></form>`;
let panel = null, made = null;
export async function send(S) {
  panel ??= import('../panel.js');
  const P = await panel;
  let dlg = document.getElementById('s-p');
  if (!dlg) {
    dlg = document.createElement('dialog');
    dlg.id = 's-p';
    dlg.className = 's-dialog';
    dlg.style.cssText = 'width:min(32em,92vw);padding:18px 20px;border:1px solid var(--line);border-radius:14px;background:var(--bg);color:var(--fg)';
    dlg.innerHTML = SHEET;
    document.body.append(dlg);
    for (const l of dlg.querySelectorAll('.s-f')) l.style.cssText = 'display:grid;gap:4px;margin-top:10px;font-size:14px';
    dlg.querySelector('.s-chk').style.cssText = 'display:flex;align-items:center;gap:8px;min-height:40px;margin-top:10px;font-size:14px';
    document.getElementById('s-p-close').onclick = () => dlg.close();
    const { ask } = P.addressForm('s-p', (a) => go(S, P, a), 'GIF');
    // Send is the sheet's default: Enter in a field sends (when the GIF is ready), it never closes the sheet.
    document.getElementById('s-p-form').onsubmit = (e) => { e.preventDefault(); if (!made) return; const a = P.addr(); if (a) go(S, P, a); else ask(); };
  }
  // The naming fields: an import's (its `from` entry), else the piece's name.
  const ext = (S.rec?.from || []).find((f) => f && f.external === true && (f.title || f.platform || f.author || f.logo)) || {};
  const val = (id, v) => { document.getElementById(id).value = typeof v === 'string' ? v : ''; };
  val('s-p-title', ext.title || S.nameNow());
  val('s-p-platform', ext.platform);
  val('s-p-author', ext.author);
  document.getElementById('s-p-logo').checked = ext.logo === true;
  const what = document.getElementById('s-p-what'), btn = document.getElementById('s-p-send');
  btn.disabled = true;
  what.textContent = 'Making the GIF…';
  if (!dlg.open) dlg.showModal();
  S.say(''); // (what was said before: not this sheet's)
  made = null;
  try {
    const m = await make(S, false);
    if (m.gif.length > MAX_GIF) throw new Error(`This GIF is ${kb(m.gif.length)}, over the panel’s ${MAX_GIF >> 20} MB: make it shorter or use fewer colours, or download it.`);
    made = m;
    what.textContent = `${S.doc.frames.length > 1 ? `${S.doc.frames.length} frames` : 'A still'} · ${m.n} colour${m.n === 1 ? '' : 's'} · ${kb(m.gif.length)}`;
    btn.disabled = false;
    S.root.dataset.sendGif = m.gif.length;
    await S.snaps();
  } catch (e) {
    what.textContent = '';
    S.say(e.message || String(e), { at: btn });
  }
}
// In the click or the form's submit: the window opens at once, the GIF follows on its ready.
function go(S, P, a) {
  const v = (id) => document.getElementById(id).value.trim();
  if (!made) return;
  const buf = made.gif.slice().buffer; // (a copy: the one sent is transferred)
  const title = v('s-p-title').slice(0, 80);
  const data = { type: IMPORT, gif: buf, name: fileName(title || S.nameNow()), title,
    platform: v('s-p-platform').replace(/[^A-Za-z0-9]/g, '').slice(0, 16), author: v('s-p-author').replace(/[^A-Za-z0-9]/g, '').slice(0, 40),
    logo: document.getElementById('s-p-logo').checked };
  const ok = P.handOff(a, 'import', READY, { data, transfer: [buf] });
  S.say(ok ? '' : 'Your browser didn’t open the panel’s web app: allow pop-ups for this site, then press Send again, or download the GIF.', { at: document.getElementById('s-p-send') });
  // (The panel's app says nothing back once it has the GIF: what is known is that its page opened.)
  if (ok) S.status(S.kept ? 'kept' : 'new', `Opened ${a}: add the GIF there`);
}
