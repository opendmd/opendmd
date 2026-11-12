// Import (/studio/), loaded by Import…, a picture or video dropped on the
// Studio, a paste too big to place, or `#import` in the address: a sheet
// over the editor holding the converter, Create Art's page in a same-origin
// frame (`/create/?in=studio`), so the Studio's own CSP never changes and
// none of the converter is fetched until the sheet first opens.
//
// Each opening is a fresh frame, removed when the sheet closes (one frame
// navigated twice would put its own entries in the page's history, and Back
// would step the frame). It says it is ready, then gets what the Studio
// holds: a dropped or pasted file, or an address (a search or a shared file
// go in its address). An address that came in a link (#import?url=) is
// shown and fetched only once "Fetch it" is pressed.
// Use in the Studio posts the frames Download GIF would write, in full
// colour (up to the piece's 4,096, two-byte keys past 255; fitted to 256 only
// when asked in the frame). Every
// message is checked here: from this frame's window, this origin and this
// opening's nonce, after its ready; and its shape, sizes, frame count,
// frame times and colours, so a message that lies is refused with a
// sentence and nothing is half-made. The frames become a new piece, or a
// new layer in this piece in one undo step; the piece's `from` gains
// {external: true, note, title, platform, author, logo}.
import { CAPS, packPx, unpackPx, checkBody, plain as text } from './store.js';
import { W, H, N, hexOf } from './doc.js';

const READY = 'opendmd-studio-import-ready', IN = 'opendmd-studio-import-in', FRAMES = 'opendmd-studio-import';
let S = null, d = null, fr = null, k = '', ready = false, held = null, busy = false;
const $ = (id) => document.getElementById(id);
const el = (tag, cls, text) => { const e = document.createElement(tag); if (cls) e.className = cls; if (text !== undefined) e.textContent = text; return e; };
const plain = (v) => !!v && typeof v === 'object' && Object.getPrototypeOf(v) === Object.prototype;
const str = (v, n) => typeof v === 'string' && v.length <= n;

export function mount(studio) {
  S = studio;
  addEventListener('message', on);
}

function build() {
  if (d) return;
  const css = el('style', '', `#s-imp{width:min(1200px,96vw);height:92vh;max-width:none;max-height:none;padding:0;border:1px solid var(--line);border-radius:14px;background:var(--bg);color:var(--fg);overflow:hidden}
#s-imp[open]{display:flex;flex-direction:column}#s-imp::backdrop{background:rgba(0,0,0,.6)}
#s-imp .s-hd{display:flex;flex-wrap:wrap;align-items:center;gap:8px 14px;padding:10px 16px;border-bottom:1px solid var(--line)}
#s-imp .s-hd h2{margin:0;font-size:18px}#s-imp label{display:inline-flex;align-items:center;gap:6px;font-size:14px;min-height:36px}
#s-imp iframe{flex:1;width:100%;border:0;background:var(--bg)}#s-imp-url{margin:6px 16px;word-break:break-all}
@media (max-width:700px){#s-imp{width:100vw;height:100dvh;border:0;border-radius:0;margin:0}
#s-imp .s-hd{gap:0 10px;padding:6px 12px}#s-imp .s-hd h2{font-size:16px}#s-imp .s-sp{display:none}#s-imp-x{order:2;margin-left:auto}
#s-imp .s-imp-as{order:3;flex:1 0 100%;display:flex;gap:12px}#s-imp label{min-height:32px;font-size:13px}}`);
  d = el('dialog');
  d.id = 's-imp';
  d.setAttribute('aria-label', 'Import');
  const hd = el('div', 's-hd'), as = el('span', 's-imp-as');
  as.setAttribute('role', 'radiogroup');
  as.setAttribute('aria-label', 'Use it as');
  for (const [v, t] of [['new', 'A new piece'], ['layer', 'A new layer in this piece']]) {
    const l = el('label'), r = el('input');
    Object.assign(r, { type: 'radio', name: 's-imp-as', value: v, checked: v === 'new' });
    l.append(r, t);
    as.append(l);
  }
  const x = el('button', 's-pill', 'Close');
  x.id = 's-imp-x';
  x.type = 'button';
  x.onclick = () => d.close();
  hd.append(el('h2', '', 'Import'), as, el('span', 's-sp'), x);
  const ask = el('p', 's-cap');
  ask.id = 's-imp-url';
  ask.hidden = true;
  d.append(css, hd, ask);
  // Closed (Close, Escape, Back): the converter is let go, so nothing it was doing can land later.
  d.onclose = () => {
    if (d.open) return; // (opened again before this close was told)
    fr?.remove();
    fr = null;
    k = '';
    ready = false;
    held = null;
    S.root.dataset.import = 'closed';
    if (location.hash.startsWith('#import')) {
      if (history.state?.imp) history.back();
      else history.replaceState(null, '', location.pathname + location.search);
    }
  };
  document.body.append(d);
}
const say = (t) => S.say(t); // (the Studio's notice, in this sheet while it is open)

// The sheet opened, from `#import?…` (route): an https address, a search, a shared file.
// `asked`: the address is one the user just gave here (a paste, a drop), not a link's.
export function show(q = new URLSearchParams(), asked = false) {
  build();
  k = [...crypto.getRandomValues(new Uint8Array(12))].map((x) => x.toString(16).padStart(2, '0')).join('');
  ready = false;
  held = null;
  const p = new URLSearchParams({ in: 'studio', k });
  const u = (q.get('url') || '').trim();
  let why = '';
  const box = $('s-imp-url');
  box.hidden = true;
  if (u) {
    let ok = false;
    try { ok = new URL(u).protocol === 'https:'; } catch { /* not an address */ }
    if (!ok) why = 'Only an https:// address can be imported: that one was left out.';
    else if (asked) held = { url: u };
    else {
      // From a link: nothing is fetched until it is asked for.
      const fetchBtn = el('button', 's-pill s-pri', 'Fetch it');
      fetchBtn.type = 'button';
      fetchBtn.id = 's-imp-fetch';
      const me = k;
      fetchBtn.onclick = () => { if (k !== me) return; box.hidden = true; give({ url: u }); };
      box.replaceChildren('Import the picture at ', el('b', '', u), '? ', fetchBtn);
      box.hidden = false;
    }
  }
  const words = (q.get('q') || '').trim().slice(0, 100);
  if (words) { p.set('q', words); if (q.get('platform')) p.set('platform', q.get('platform').slice(0, 16)); }
  if (q.get('shared')) p.set('shared', '1');
  // A fresh frame for each opening, its address set before it is in the page.
  fr?.remove();
  fr = el('iframe');
  fr.id = 's-imp-f';
  fr.title = 'Find or bring a picture, a GIF or a video, and make it fit the panel';
  fr.src = `../create/?${p}`;
  d.append(fr);
  for (const r of d.querySelectorAll('input[name=s-imp-as]')) r.checked = r.value === 'new';
  if (!d.open) d.showModal();
  if (why) say(why); // (once the sheet is open: said in it)
  S.root.dataset.import = 'open';
}
// A file for Import (dropped on the Studio, a paste too big to place): the sheet opens with it.
export function bring(file, label = 'Your file') {
  if (d?.open && k) { give({ file, label }); return; } // (dropped on the open sheet: into this opening)
  if (!location.hash.startsWith('#import')) history.pushState({ imp: 1 }, '', '#import');
  show();
  held = { file, label };
}
// Something for the frame: now if it is ready, else at its ready.
function give(x) {
  if (ready && fr) fr.contentWindow.postMessage({ type: IN, k, ...x }, location.origin);
  else held = x;
}
export function close() { if (d?.open) d.close(); }

function on(e) {
  if (!d?.open || !k || !fr || e.source !== fr.contentWindow || e.origin !== location.origin) return;
  const m = e.data;
  if (!plain(m) || m.k !== k) return;
  if (m.type === READY) {
    ready = true;
    S.root.dataset.import = 'ready';
    if (held) { fr.contentWindow.postMessage({ type: IN, k, ...held }, location.origin); held = null; }
    return;
  }
  if (m.type !== FRAMES || !ready || busy) return;
  busy = true;
  const me = k;
  take(m, () => d.open && k === me).then((done) => {
    if (!done) return;
    d.close();
    // (Said by the frame's message: past a piece's colours it was fitted to 256, not refused.)
    const fitted = m.fitted === true ? `That had more than ${CAPS.colours.toLocaleString('en')} colours, so it was fitted to 256.` : '';
    S.say([fitted, m.total > m.n ? `That had ${m.total.toLocaleString('en')} frames: the first ${m.n} are kept (a piece has ${CAPS.frames} at most). Import’s timeline trims it to the part you want.` : ''].filter(Boolean).join(' '), 'info');
  }, (err) => refuse(err.message || String(err))).finally(() => { busy = false; });
}
function refuse(why) {
  say(`That can’t be used: ${why}. Nothing was changed.`);
  S.root.dataset.imported = 'refused';
}

// What a frames message says, checked against what it carries: the frames
// as keys (0: off), their colours (0xRRGGBB, in order of first use), times.
function read(m) {
  if (m.w !== W || m.h !== H) throw new Error(`its frames aren’t ${W} × ${H}`);
  if (!Number.isInteger(m.n) || m.n < 1 || m.n > CAPS.frames) throw new Error(`it has ${String(m.n).slice(0, 12)} frames: a piece has 1 to ${CAPS.frames}`);
  if (!Number.isInteger(m.total) || m.total < m.n || m.total > 1e6 || (m.total > m.n && m.n < CAPS.frames)) throw new Error('its frame count doesn’t add up');
  if (!Array.isArray(m.ms) || m.ms.length !== m.n) throw new Error('its frame times don’t match its frames');
  const ms = m.ms.map((t) => (typeof t === 'number' && Number.isFinite(t) ? Math.round(t / 10) * 10 : NaN));
  const odd = ms.findIndex((t) => !(t >= CAPS.msMin && t <= CAPS.msMax));
  if (odd >= 0) throw new Error(`a frame of ${String(m.ms[odd]).slice(0, 12)} ms: ${CAPS.msMin} to ${CAPS.msMax.toLocaleString('en')} ms`);
  if (!(m.px instanceof ArrayBuffer) || m.px.byteLength !== m.n * N * 4) throw new Error('its pictures don’t match its frame count');
  if (!Number.isInteger(m.plays) || m.plays < 0 || m.plays > 65535) throw new Error('its play count isn’t one');
  if (!str(m.title, CAPS.name) || !str(m.platform, 16) || !str(m.author, 80) || !str(m.note, CAPS.note) || typeof m.logo !== 'boolean') throw new Error('its naming fields aren’t right');
  const px = new Uint8Array(m.px), keys = new Map(), cels = [];
  for (let f = 0; f < m.n; f++) {
    const c = new Uint16Array(N);
    for (let i = 0, o = f * N * 4; i < N; i++, o += 4) {
      if (px[o + 3] < 128) continue; // see-through: off
      const v = (px[o] << 16) | (px[o + 1] << 8) | px[o + 2];
      if (!v) continue; // black: off, as the panel shows it
      let key = keys.get(v);
      if (key === undefined) {
        if (keys.size >= CAPS.colours) throw new Error(`it has more than ${CAPS.colours.toLocaleString('en')} colours, more than a piece holds: tick Fit to 256 colours and use it again`);
        keys.set(v, (key = keys.size + 1));
      }
      c[i] = key;
    }
    cels.push(c);
  }
  // (One byte a pixel when the colours fit in 255, as the Studio writes them; else two.)
  return { ms, cels: keys.size > 255 ? cels : cels.map((c) => Uint8Array.from(c)), colours: [...keys.keys()] };
}

// `now()`: this opening is still the one open (else nothing is made: false).
async function take(m, now) {
  const { ms, cels, colours } = read(m);
  const n = cels.length;
  // (The naming fields as plain text, as a file's are.)
  m = { ...m, title: text(m.title), platform: text(m.platform), author: text(m.author), note: text(m.note) };
  const entry = { external: true, note: m.note || 'Imported', ...(m.title ? { title: m.title } : {}), ...(m.platform ? { platform: m.platform } : {}),
    ...(m.author ? { author: m.author } : {}), ...(m.logo ? { logo: true } : {}) };
  const as = d.querySelector('input[name=s-imp-as]:checked')?.value;
  if (as !== 'layer') {
    await S.save();
    if (!now()) return false;
    try {
      await S.newPiece({ name: (m.title || 'Imported').slice(0, CAPS.name), plays: m.plays, frames: ms.map((t) => ({ ms: t })),
        layers: [{ name: 'Imported', moves: n > 1, cels: n > 1 ? cels : [cels[0]] }], colours: colours.map((c, i) => ({ k: i + 1, hex: hexOf(c) })), from: [entry] });
    } catch (e) { throw new Error(/too big/.test(e.message) ? 'it is too big for a piece (4 MB at most): trim it in the timeline, or use fewer colours' : e.message); }
    S.keep();
    await S.save(); // (kept before anything else: a reload right after has it)
    await S.drafts();
    S.root.dataset.imported = `piece:${n}`;
    return true;
  }
  // A new layer on top of this piece, one undo step: a still held, an
  // animation moving (a one-frame piece takes its frames and times; one that
  // moves already repeats them along its own).
  const D = S.doc;
  if (D.stroke || S.dragging) throw new Error('a stroke is in progress: finish it first');
  if (D.layers.length >= CAPS.layers) throw new Error(`this piece has ${CAPS.layers} layers, the most it can: use it as a new piece`);
  if (!S.addFrom(entry, true)) throw new Error('this piece names as many things it was made from as it can');
  S.done();
  const ks = D.keysFor(colours);
  if (D.full) { D.letGo(D.key); throw new Error(`with its colours this piece would have more than ${CAPS.colours.toLocaleString('en')}`); }
  const was = await D.packed(D.snap()), more = new Map(ks.map((x, i) => [x, colours[i]]));
  const have = new Set(was.colours.map((c) => c.k));
  const all = [...was.colours, ...[...more].filter(([x]) => !have.has(x)).map(([x, c]) => ({ k: x, hex: hexOf(c) }))].sort((a, b) => a.k - b.k);
  const wide = all.some((c) => c.k > 255), map = Uint16Array.from([0, ...ks]);
  // (Every cel as wide as the colours now need.)
  const again = async (z) => (wide === (was.keys === 2) ? z : packPx(Uint16Array.from(await unpackPx(z, N, was.keys === 2)), wide));
  const grow = D.frames.length === 1 && n > 1;
  const frames = grow ? ms.map((t, i) => (i ? { ms: t } : { ...was.frames[0], ms: t })) : was.frames;
  const layers = [];
  for (const l of was.layers) {
    const cs = await Promise.all(l.cels.map(again));
    layers.push({ ...l, cels: grow && l.moves ? frames.map(() => cs[0]) : cs });
  }
  const mine = (c) => packPx(Uint16Array.from(c, (x) => map[x]), wide);
  layers.push({ name: 'Imported', moves: n > 1, cels: n > 1 ? await Promise.all(frames.map((_, f) => mine(cels[f % n]))) : [await mine(cels[0])] });
  const { keys, ...rest } = was;
  const body = { ...rest, ...(wide ? { keys: 2 } : {}), colours: all, frames, layers, ...(grow && m.plays ? { plays: m.plays } : {}) };
  try { checkBody('piece', body); } catch (e) { D.letGo(D.key); throw new Error(e.message); }
  if (JSON.stringify(body).length > CAPS.piece - (64 << 10)) { D.letGo(D.key); throw new Error('the piece would be too big (4 MB at most): trim it in the timeline first'); }
  if (!now()) { D.letGo(D.key); return false; }
  await S.replace(body);
  S.addFrom(entry);
  await S.save();
  S.root.dataset.imported = `layer:${n}`;
  return true;
}
