// Share links (/studio/), loaded by Share, a pack's Share link in the
// Library, or an address with #p= or #k= (docs/studio-format.md "Share
// links"). Share and the pack's link are off (studio.js SHARE, sharing
// shelved 2026-11-12): only an address with #p= or #k= loads this now. The fragment never reaches the server, and opening a link fetches
// nothing and stores nothing until the user says so.
//
// `#p=`: one piece, flattened (every shown layer composited, as a GIF has
// it): a version byte; frame count and plays; the colour table; the times;
// frame 0 as indices into the table, each later frame XOR the one before;
// the name, the author and the licence; the source record {id, rev, hash}.
// Deflated, base64url. It opens as a preview, the picture playing, who it
// says made it and its licence, and "Open as a new piece".
//
// `#k=`: a small pack, the .dmdpack file's JSON, deflated, base64url. It
// opens as a file does (files.js open): every record checked and hashed,
// the claims shown, nothing stored until Add, a record of the user's never
// replaced.
//
// A link is at most LINK characters: past that Share says so and offers the
// file. Coming in, a longer fragment is refused before anything is decoded;
// inflating stops at the most a link could hold; every byte is checked
// against what came before it, and the piece against check().
import { FORMAT, CAPS, LICENCES, inflate, record, check, plain } from './store.js';
import { W, H, N, hexOf } from './doc.js';

export const LINK = 8000;
const V = 1, MAX_RAW = 1 + 6 + CAPS.colours * 3 + CAPS.frames * 2 + CAPS.frames * N * 2 + 3 * 256 + 64, MAX_PACK = 1 << 20;
let S = null, playing = 0;
export function mount(studio) { S = studio; }

// --- bytes, deflate, base64url ---
async function deflate(bytes) {
  const s = new Blob([bytes]).stream().pipeThrough(new CompressionStream('deflate'));
  return new Uint8Array(await new Response(s).arrayBuffer());
}
const b64u = (z) => { let s = ''; for (let i = 0; i < z.length; i += 0x8000) s += String.fromCharCode(...z.subarray(i, i + 0x8000)); return btoa(s).replace(/\+/g, '-').replace(/\//g, '_').replace(/=+$/, ''); };
const unb64u = (t) => t.replace(/-/g, '+').replace(/_/g, '/') + '='.repeat((4 - (t.length % 4)) % 4);
const utf = new TextEncoder();
const damaged = (why) => { throw new Error(`This link is damaged (${why}): nothing was taken from it.`); };

// --- out: the open piece as #p= ---
export async function encode(name, author, licence, src, keys, cm, delays, plays) {
  const n = delays.length, table = [], at = new Map([[0, 0]]);
  for (const k of keys) if (!at.has(k)) { const c = cm.get(k) ?? 0, i = table.indexOf(c); at.set(k, i >= 0 ? i + 1 : table.push(c)); }
  const wide = table.length > 255, b = [];
  const u8 = (v) => b.push(v & 255), u16 = (v) => { u8(v >> 8); u8(v); }, str = (t, cap) => {
    // 255 bytes of UTF-8 at most, cut between characters (never inside one).
    let s = [...t.slice(0, cap)], x = utf.encode(s.join(''));
    while (x.length > 255) { s.pop(); x = utf.encode(s.join('')); }
    u8(x.length); b.push(...x);
  };
  u8(V); u16(n); u16(plays); u16(table.length);
  for (const c of table) { u8(c >> 16); u8(c >> 8); u8(c); }
  for (const d of delays) u16(Math.round(d / 10));
  const px = new Uint8Array(n * N * (wide ? 2 : 1)), w = wide ? 2 : 1;
  for (let i = 0; i < n * N; i++) { const v = at.get(keys[i]); if (wide) { px[2 * i] = v >> 8; px[2 * i + 1] = v; } else px[i] = v; }
  for (let i = px.length - 1; i >= N * w; i--) px[i] ^= px[i - N * w]; // (later frames as what changed)
  str(name, CAPS.name); str(author, 80); u8(licence ? LICENCES.indexOf(licence) + 1 : 0);
  if (src) {
    u8(1); str(src.id, 64); u16(src.rev >>> 16); u16(src.rev & 0xffff);
    for (let i = 0; i < 64; i += 2) u8(parseInt(src.hash.slice(7 + i, 9 + i), 16));
  } else u8(0);
  const head = Uint8Array.from(b.slice(0, 7 + table.length * 3 + n * 2)), tail = Uint8Array.from(b.slice(7 + table.length * 3 + n * 2));
  const all = new Uint8Array(head.length + px.length + tail.length);
  all.set(head); all.set(px, head.length); all.set(tail, head.length + px.length);
  return b64u(await deflate(all));
}
const here = () => location.origin + location.pathname;
async function give(text, big) {
  if (text.length > LINK) { S.offer(`That’s too big for a link (${text.length.toLocaleString('en')} characters; a link holds ${LINK.toLocaleString('en')}): save it as a file to share it.`, 'Save as file', big); return null; }
  try { await navigator.clipboard.writeText(text); S.say(''); S.status('kept', 'Link copied'); } catch {
    S.say('Copy this link: ');
    const i = Object.assign(document.createElement('input'), { value: text, readOnly: true, className: 's-in' });
    document.getElementById('s-err').append(i);
    i.select();
  }
  S.root.dataset.link = text;
  return text;
}
// Share: the open piece, as it shows (hidden layers left out), copied as a link.
export async function share() {
  S.done();
  const D = S.doc, { flatten } = await import('./out.js');
  const body = await D.packed(D.snap(), { palette: S.swatch().id });
  const { keys, cm, delays, plays } = await flatten(body, S.listOf);
  const src = S.piece(), r = S.rec || {};
  const code = await encode(S.nameNow(), r.author?.name || '', r.licence || null, src && src.hash ? { id: src.id, rev: src.rev, hash: src.hash } : null, keys, cm, delays, plays);
  return give(`${here()}#p=${code}`, () => S.files().then((m) => m.saveOpen()));
}
// A pack from the Library, as #k=: what its file would hold.
export async function sharePack(id) {
  const r = (S.store && await S.store.get(id)), m = await S.files(), records = r && await m.gather(r);
  if (!records) return null;
  const z = await deflate(utf.encode(JSON.stringify({ format: FORMAT, root: id, records })));
  return give(`${here()}#k=${b64u(z)}`, () => m.save(id));
}

// --- in ---
// What #p= holds, checked through: {spec, claims}, or it throws saying why.
export async function decodePiece(code) {
  let raw;
  try { raw = await inflate(unb64u(code), MAX_RAW); } catch { damaged('it doesn’t unpack'); }
  let p = 0;
  const need = (k) => { if (p + k > raw.length) damaged('it ends too soon'); };
  const u8 = () => { need(1); return raw[p++]; }, u16 = () => (u8() << 8) | u8();
  const str = (cap) => { const l = u8(); need(l); let t; try { t = new TextDecoder('utf-8', { fatal: true }).decode(raw.subarray(p, p + l)); } catch { damaged('a name that isn’t text'); } p += l; if (t.length > cap) damaged('a name too long'); return t; };
  if (u8() !== V) damaged('a version this Studio doesn’t know');
  const n = u16(), plays = u16(), c = u16();
  if (n < 1 || n > CAPS.frames) damaged(`${n} frames`);
  if (c > CAPS.colours) damaged(`${c} colours`);
  need(c * 3);
  const colours = [];
  for (let i = 0; i < c; i++) colours.push({ k: i + 1, hex: hexOf((raw[p] << 16) | (raw[p + 1] << 8) | raw[p + 2]) }), (p += 3);
  if (new Set(colours.map((x) => x.hex)).size !== c) damaged('a colour listed twice');
  const ms = [];
  for (let i = 0; i < n; i++) { const t = u16() * 10; if (t < CAPS.msMin || t > CAPS.msMax) damaged(`a frame of ${t} ms`); ms.push(t); }
  const wide = c > 255, w = wide ? 2 : 1;
  need(n * N * w);
  const px = raw.slice(p, p + n * N * w);
  p += n * N * w;
  for (let i = N * w; i < px.length; i++) px[i] ^= px[i - N * w];
  const cels = [];
  for (let f = 0; f < n; f++) {
    const cel = wide ? new Uint16Array(N) : new Uint8Array(N);
    for (let i = 0; i < N; i++) { const v = wide ? (px[(f * N + i) * 2] << 8) | px[(f * N + i) * 2 + 1] : px[f * N + i]; if (v > c) damaged('a pixel with no colour'); cel[i] = v; }
    cels.push(cel);
  }
  const name = plain(str(CAPS.name)), author = plain(str(80)), li = u8();
  if (li > LICENCES.length) damaged('a licence that isn’t one');
  let src = null;
  if (u8()) {
    const id = str(64), rev = u16() * 65536 + u16();
    need(32);
    const hash = 'sha256-' + [...raw.subarray(p, p + 32)].map((x) => x.toString(16).padStart(2, '0')).join('');
    p += 32;
    if (!/^pc_[0-9a-f]{32}$/.test(id) || rev < 1 || rev > CAPS.rev) damaged('a source that isn’t a piece');
    src = { id, rev, hash };
  }
  if (p !== raw.length) damaged('more than it says');
  const licence = li ? LICENCES[li - 1] : null;
  const spec = { name: name || 'Shared', plays, frames: ms.map((t) => ({ ms: t })), layers: [{ name: 'Drawing', moves: n > 1, cels: n > 1 ? cels : [cels[0]] }], colours };
  return { spec, name, author, licence, src };
}
// The address's #p= or #k=, opened: a preview of the piece, or the pack as a file.
export async function show(hash) {
  hash = hash.slice(0, 2).toLowerCase() + hash.slice(2); // (#P= as #p=)
  // Another sheet open: closed first, so this one is the one seen (a preview
  // open already is shown anew in place; a pack's sheet takes its place).
  for (const o of document.querySelectorAll(/^#p/.test(hash) ? 'dialog[open]:not(#s-lk)' : 'dialog[open]')) o.close();
  history.replaceState(null, '', location.pathname + location.search); // (a reload doesn't open it again)
  S.root.dataset.linkOpened = 'reading';
  const m = /^#([pk])=([A-Za-z0-9_-]+)$/.exec(hash);
  try {
    if (hash.length > LINK + 3) throw new Error(`This link is ${hash.length.toLocaleString('en')} characters, more than a Studio link holds (${LINK.toLocaleString('en')}): nothing was taken from it.`);
    if (!m) damaged('it isn’t a Studio link');
    if (m[1] === 'k') {
      let raw;
      try { raw = await inflate(unb64u(m[2]), MAX_PACK); } catch { damaged('it doesn’t unpack'); }
      S.root.dataset.linkOpened = 'pack';
      return (await S.files()).open([new File([raw], 'shared.dmdpack', { type: 'application/json' })], 'A shared link');
    }
    return preview(await decodePiece(m[2]));
  } catch (e) {
    S.say(e.message);
    S.root.dataset.linkOpened = 'refused';
    return null;
  }
}

const LIC = { 'CC0-1.0': 'CC0', 'CC-BY-4.0': 'CC BY 4.0' };
async function preview(x) {
  const { spec, name, author, licence, src } = x;
  // Checked as a record before it is shown (it is stored only on Open as a new piece).
  const probe = record('piece', spec.name, { w: W, h: H, colours: spec.colours, ...(spec.colours.length > 255 ? { keys: 2 } : {}), frames: spec.frames,
    layers: spec.layers.map((l) => ({ ...l, cels: l.cels.map(() => '') })) });
  try { check(probe); } catch (e) { damaged(e.message.replace('not a record: ', '')); }
  let d = document.getElementById('s-lk');
  if (!d) {
    d = document.createElement('dialog');
    d.id = 's-lk';
    d.style.cssText = 'width:min(40em,94vw);padding:18px 20px;border:1px solid var(--line);border-radius:14px;background:var(--bg);color:var(--fg)';
    document.body.append(d);
  }
  clearTimeout(playing); // (a link before this one stops playing)
  const h = document.createElement('h2'), said = document.createElement('p'), cv = document.createElement('canvas'), who = document.createElement('p'), row = document.createElement('div');
  const q = (t) => { const b = document.createElement('bdi'); b.textContent = t; return ['“', b, '”']; };
  // The heading is the Studio's; the name is the link's claim, quoted.
  h.textContent = 'A shared piece';
  h.style.margin = '0 0 4px';
  said.id = 's-lk-name';
  said.style.margin = '0 0 10px';
  said.append(...(name ? ['Called ', ...q(name)] : ['It has no name.']));
  Object.assign(cv, { width: W, height: H });
  cv.style.cssText = 'width:100%;image-rendering:pixelated;background:#000;border-radius:6px';
  who.className = 's-cap';
  who.id = 's-lk-who';
  who.append(`${spec.frames.length > 1 ? `${spec.frames.length} frames` : 'A still'} · by `, ...(author ? q(author) : ['no one named']),
    ` · licence ${LIC[licence] || 'not chosen'}, the link says. Nothing is kept until you open it.`);
  row.className = 's-row';
  const go = document.createElement('button'), no = document.createElement('button');
  go.className = 's-pill s-pri'; go.textContent = 'Open as a new piece'; go.id = 's-lk-open'; go.type = 'button';
  no.className = 's-pill'; no.textContent = 'Close'; no.type = 'button';
  row.append(go, no);
  d.replaceChildren(h, said, cv, who, row);
  // The picture playing, frame by frame, at its own times.
  const g = cv.getContext('2d'), img = g.createImageData(W, H), rgb = [0, ...spec.colours.map((c) => parseInt(c.hex.slice(1), 16))];
  let f = 0;
  const draw = () => {
    const cel = spec.layers[0].cels[spec.layers[0].moves ? f : 0];
    for (let i = 0; i < N; i++) { const v = rgb[cel[i]]; img.data[i * 4] = v >> 16; img.data[i * 4 + 1] = (v >> 8) & 255; img.data[i * 4 + 2] = v & 255; img.data[i * 4 + 3] = 255; }
    g.putImageData(img, 0, 0);
    if (spec.frames.length > 1) playing = setTimeout(() => { f = (f + 1) % spec.frames.length; draw(); }, spec.frames[f].ms);
  };
  draw();
  d.onclose = () => { clearTimeout(playing); if (S.root.dataset.linkOpened === 'shown') S.root.dataset.linkOpened = 'closed'; };
  no.onclick = () => d.close();
  go.onclick = async () => {
    go.disabled = true;
    // What it was made from: the record the link names, unless that id is one
    // of this browser's with other content (a link can't speak for it).
    let from = [{ external: true, note: 'A shared link', ...(name ? { title: name } : {}), ...(author ? { author } : {}) }];
    if (src) {
      const have = S.store && await S.store.get(src.id);
      // (Held here with that content: what this browser knows of it, not what the link says.)
      if (have && have.hash === src.hash) from = [{ id: have.id, rev: have.rev, hash: have.hash, name: have.name, author: have.author, licence: have.licence }];
      else if (!have) from = [{ id: src.id, rev: src.rev, hash: src.hash, name, author: { name: author, account: null }, licence }];
    }
    try {
      await S.save();
      await S.newPiece({ ...spec, from });
      S.keep();
      await S.save();
      await S.drafts();
      S.root.dataset.linkOpened = 'opened';
      d.close();
    } catch (e) { S.say(`That piece can’t be opened: ${e.message || e}`); go.disabled = false; }
  };
  if (!d.open) d.showModal();
  S.root.dataset.linkOpened = 'shown';
}
