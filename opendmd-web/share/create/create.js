// Create Art on the site (/create/): the web app's own Studio (js/studio.js
// `mount`, from the demo's files), with every source of pictures — Game art,
// GifCities (js/find.js), your own file, drop, paste or address — and every
// editing step.
//
// Send to my panel: the panel's own web app opened at `#import` (the
// address asked once: ../panel.js), which says it is ready and gets the
// finished GIF by postMessage, transferred, with its title, platform,
// author and logo (js/importgif.js there: Add GIFs, filled in). Download
// GIF is always there too (then Add GIFs on the panel).
//
// `?q=<words>&platform=<P>` searches at once: Game art on that platform when
// it has one, else GifCities (the Studio's `app.artFind`; the web app's
// Library links here with its words). `?url=<address>` opens that picture.
//
// An address the browser may not fetch (a site with no CORS headers) comes
// through the site's relay, /api/fetch (site/worker/src/relay.ts): only
// for this page, pictures and videos only, nothing kept. Its refusals are
// said here in plain words. A video (MP4, WebM: from a file, a link or the
// relay) is played into frames by ./video.js, imported only then. A page's address (a Tenor, Reddit or Imgur
// link) comes back as the pictures and videos it names, best first, and
// the first that loads is used.
//
// Bringing a GIF from elsewhere (opendmd-notes docs/research/tenor.md §7-8): under Your
// own, links that open other sites' own search in a new tab with "pixel
// art" and the visitor's words, and a bookmarklet that opens the page it
// is pressed on here (?url=). Installed on Android, the page is a share
// target (manifest.webmanifest, sw.js): a shared file arrives as
// ?shared=1 (put aside by the worker), a shared link as ?url=.
//
// A video longer than video.js' MAX_S gets "Start at" (seconds) beside the
// line that says what was read: the same file read again from there.
//
// Inside the Studio (/studio/'s Import sheet frames this page as
// `?in=studio&k=<nonce>`, same origin, its parent the Studio): the page's
// own chrome, Send and Download give way to one action, Use in the Studio.
// It takes what Download GIF would write (`out()`), reads it back into
// frames (./gifread.js) and posts them to the Studio, transferred, with
// their times, the play count, the naming fields and where it came from;
// the Studio checks every message (studio/import.js). The page says it is
// ready the same way, and takes a file the Studio holds (dropped, pasted).
// Opened directly, `?in=studio` is the plain page.
//
// The hand-off (the cutover, behind a switch that stays off until the
// owner's call: `<html data-handoff="on">` in index.html): opened directly,
// the page hands off at once to the Studio's Import, carrying ?q=&platform=,
// ?url= and ?shared=1 (the share target's file, put aside by sw.js) as
// `/studio/#import?…`.

import { init } from '../demo/js/web.js';
import { app } from '../demo/js/state.js';
import { READY, IMPORT, MAX_GIF } from '../demo/js/importgif.js';
import { addr, addressForm, handOff } from '../panel.js';
import { h } from '../demo/js/util.js';

const $ = (id) => document.getElementById(id);
// Messages between this page and the Studio's Import (studio/import.js).
const S_READY = 'opendmd-studio-import-ready', IN = 'opendmd-studio-import-in', FRAMES = 'opendmd-studio-import';
const Q = new URLSearchParams(location.search);
const studioParent = () => { try { return parent !== window && parent.location.origin === location.origin && parent.location.pathname.startsWith('/studio/'); } catch { return false; } };
const framed = Q.get('in') === 'studio' && studioParent();
const handoff = !framed && document.documentElement.dataset.handoff === 'on';
if (framed) document.documentElement.classList.add('framed');
// The cutover: this address hands off to the Studio's Import, its query with it.
if (handoff) {
  const p = new URLSearchParams();
  for (const k of ['q', 'platform', 'url', 'shared']) if (Q.get(k)) p.set(k, Q.get(k));
  // (The share target's worker is put in place first, as the page would: it is what takes a share's POST.)
  await Promise.race([navigator.serviceWorker?.register('sw.js').catch(() => {}), new Promise((ok) => setTimeout(ok, 1500))]);
  location.replace(new URL('../studio/', location.href).href + '#import' + (p.size ? '?' + p : ''));
}
const WHY = {
  size: 'That file is too big (over 16 MB): pick a shorter or smaller one',
  type: 'That address isn’t a picture or a video',
  rate: 'Too many tries in a minute: wait a moment, then try again',
  busy: 'opendmd.dev is fetching a lot for everyone just now: try again later, or download the picture and add the file here',
  token: 'Reload this page, then try again',
  refused: 'That site refused to send it: download the picture, then add the file here',
};

// The relay's token (POST /api/fetch/token): asked once, again when it is
// near its end or the relay refuses it (a new network, say), then once more.
let token = null;
async function relayToken(fresh) {
  if (fresh || !token || Date.now() > token.until) {
    const r = await fetch('/api/fetch/token', { method: 'POST' });
    const d = await r.json().catch(() => ({}));
    if (!r.ok || !d.token) throw new Error(WHY[d.why] || WHY.refused);
    token = { t: d.token, until: Date.now() + (d.ttl - 60) * 1000 };
  }
  return token.t;
}
async function relay(u, deep, again) {
  let r;
  try { r = await fetch('/api/fetch', { method: 'POST', body: u, headers: { 'X-Relay-Token': await relayToken(again) } }); } catch (e) { throw e instanceof TypeError ? new Error('opendmd.dev could not be reached: check your connection') : e; }
  if (!r.ok) {
    const why = await r.json().then((d) => d.why, () => '');
    if (why === 'token' && !again) return relay(u, deep, true);
    throw new Error(WHY[why] || WHY.refused);
  }
  if (r.headers.get('X-Relayed') === 'page') {
    // A page: what it names, each asked directly first, then through the relay.
    const { media = [] } = await r.json();
    if (deep || !media.length) throw new Error(WHY.type);
    let err;
    for (const m of media.slice(0, 3)) {
      try {
        const d = await fetch(m).catch(() => null);
        const b = d?.ok ? await d.blob() : await relay(m, true);
        return Object.assign(b, { link: u });
      } catch (e) { err = e; }
    }
    throw err;
  }
  // (Past 16 MB the relay stops sending: the body ends in an error.)
  try { return await r.blob(); } catch { throw new Error(WHY.size); }
}
const say = (t) => { $('c-cap').hidden = !t; $('c-cap').textContent = t || ''; };

// #create is aria-busy until the Studio is up: what a test waits for.
if (!handoff) {
  start().catch((e) => say('This page could not load: ' + (e.message || e)))
    .finally(() => $('create').removeAttribute('aria-busy'));
}

async function start() {
  await init();
  app.site = true;
  const { mount } = await import('../demo/js/studio.js');
  const send = Object.assign(document.createElement('button'), { id: framed ? 'c-use' : 'c-send', className: 'pri', textContent: framed ? 'Use in the Studio' : 'Send to my panel' });
  const st = await mount($('c-ed'), { css: false, acts: [send], relay, video: reader });
  $('st-save').classList.remove('pri'); // (Send is the page's first way; Download the other)
  if (framed) $('st-save').hidden = true; // (the Studio downloads)
  startAt(st);
  const q = Q;
  const words = (q.get('q') || '').trim().slice(0, 100);
  if (words) app.artFind = { q: words, platform: (q.get('platform') || '').toUpperCase() };
  await st.show();
  elsewhere();
  const link = (q.get('url') || '').trim();
  // (Framed, an address in the page's own address came from a link: shown, fetched only when asked.)
  if (/^https?:\/\//i.test(link) && framed) {
    const u = link.replace(/^http:/i, 'https:');
    $('c-cap').hidden = false;
    $('c-cap').replaceChildren('Import the picture at ', h('b', u), '? ', h('button.pri', { id: 'c-fetch', onclick: () => { $('c-cap').hidden = true; st.open(u); } }, 'Fetch it'));
  } else if (/^https?:\/\//i.test(link)) st.open(link.replace(/^http:/i, 'https:'));
  else if (q.get('shared')) shared(st);
  if (framed) { inStudio(st, send); return; }
  navigator.serviceWorker?.register('sw.js').catch(() => {});

  // (In the click or the form's submit: the window opens at once, the GIF
  // follows when it is made — an animation's every frame is rendered first.)
  function go(a) {
    say('');
    if (!st.ready()) { say('Pick a picture and give it a title first.'); return; }
    const gif = st.out().then((o) => {
      if (!o) return null;
      if (o.gif.length > MAX_GIF) { say(`This GIF is over ${MAX_GIF >> 20} MB, too big to send: make it shorter or use fewer colours, or download it.`); return null; }
      const buf = o.gif.slice().buffer; // (a copy: the one sent is transferred, and Download keeps its own)
      return { data: { type: IMPORT, gif: buf, name: o.name, title: o.title, platform: o.platform, author: o.author, logo: o.logo }, transfer: [buf] };
    });
    if (!handOff(a, 'import', READY, gif)) say('Your browser didn’t open the display’s web app: allow pop-ups for opendmd.dev, then press Send to my panel again, or download the GIF.');
  }
  const { ask } = addressForm('c', go, 'GIF');
  send.onclick = () => { if (addr()) go(addr()); else ask(); };
}

// --- inside the Studio's Import ---
function inStudio(st, use) {
  const k = Q.get('k') || '', to = (m, t) => parent.postMessage({ ...m, k }, location.origin, t || []);
  // A file the Studio holds (dropped on it, pasted), or an address it was
  // asked to fetch: from the Studio's window alone.
  addEventListener('message', (e) => {
    if (e.source !== parent || e.origin !== location.origin || e.data?.type !== IN || e.data.k !== k) return;
    if (e.data.file instanceof Blob) st.load(e.data.file, { label: e.data.label === 'Pasted' ? 'Pasted' : 'Your file' });
    else if (typeof e.data.url === 'string' && /^https:\/\//i.test(e.data.url)) st.open(e.data.url);
  });
  use.onclick = async () => {
    say('');
    if (!$('studio').classList.contains('has')) { say('Pick a picture, a GIF or a video first.'); return; }
    // The Studio needs no file name: the converter's out() does, so a plain
    // one stands in while the GIF is written (any title, author and platform,
    // or none, are sent as they were typed).
    const t = $('st-title'), a = $('st-author'), was = [t.value, a.value];
    const set = (tv, av) => { t.value = tv; a.value = av; t.dispatchEvent(new Event('input')); };
    use.disabled = true;
    try {
      set('Imported', '');
      for (let i = 0; i < 60 && !st.ready(); i++) await new Promise((ok) => setTimeout(ok, 50));
      const o = await st.out();
      if (!o) { say('The GIF could not be made: try again.'); return; }
      const { read } = await import('./gifread.js'), g = read(o.gif);
      // (Times as the Studio keeps them: 20 ms to a minute.)
      const ms = g.ms.map((x) => Math.min(60000, Math.max(20, Math.round(x / 10) * 10)));
      to({ type: FRAMES, w: 128, h: 32, n: g.n, total: g.total, ms, plays: g.plays, px: g.px.buffer,
        title: was[0].trim().slice(0, 120), platform: o.platform, author: was[1].trim().slice(0, 80), logo: o.logo, note: from() }, [g.px.buffer]);
    } catch (e) { say(e.message || String(e)); } finally {
      use.disabled = false;
      set(...was);
    }
  };
  to({ type: S_READY });
}
// Where the picture came from, for the piece's `from`: "Game art: <title>",
// "GifCities: <words>", an address's host, "Your file", "Pasted", "Shared".
function from() {
  const msg = $('st-msg').textContent, src = document.querySelector('#st-src .on')?.dataset.v, i = msg.indexOf(': ');
  const label = i > 0 ? msg.slice(0, i) : '';
  const n = src === 'gameart' && label ? `Game art: ${label}` : src === 'gifcities' && label ? `GifCities: ${label}` : label && !/^(Video|GIF|Picture)$/.test(label) ? label : 'Your file';
  return n.slice(0, 200);
}

// The video reader (./video.js), loaded when a video first arrives. Past
// its MAX_S, "Start at" beside the line that says what was read: the same
// video read again from there.
let vid = null, long = () => {};
const reader = async () => {
  const m = await import('./video.js');
  return { frames: async (blob, o) => {
    const at = vid?.blob === blob ? vid.at : 0;
    vid = { blob, at, dur: 0 };
    const r = await m.frames(blob, { ...o, start: at });
    if (r) vid.dur = r.dur;
    long(r && r.dur > m.MAX_S);
    return r;
  } };
};
function startAt(st) {
  const s = h('input', { type: 'number', id: 'c-start-s', min: 0, step: 0.5, value: 0, 'aria-label': 'Start at, in seconds' });
  const box = h('span', { id: 'c-start', hidden: true }, ' · Start at ', s, ' s ', h('button.sm', { id: 'c-start-go', onclick: () => {
    if (!vid) return;
    vid.at = Math.max(0, Math.min(+s.value || 0, Math.max(0, vid.dur - 0.5)));
    st.load(vid.blob, { label: 'Video' });
  } }, 'Read from there'));
  $('st-msg').after(box);
  long = (on) => { box.hidden = !on; s.value = String(vid?.at || 0); };
  // Any other picture hides it.
  new MutationObserver(() => { if (!/video|^Reading/.test($('st-msg').textContent)) box.hidden = true; })
    .observe($('st-msg'), { childList: true, characterData: true, subtree: true });
}

// What was shared to the installed page (sw.js put it aside): loaded once.
async function shared(st) {
  history.replaceState(null, '', location.pathname);
  const box = await caches.open('create-shared').catch(() => null);
  const r = await box?.match('shared');
  if (!r) return;
  await box.delete('shared');
  st.load(await r.blob(), { label: 'Shared' });
}

// Other sites' own search, in a new tab: "pixel art" and the visitor's words.
const enc = (s) => encodeURIComponent(s.trim()).replace(/%20/g, '+');
const dash = (s) => s.trim().toLowerCase().replace(/[^a-z0-9]+/g, '-').replace(/^-|-$/g, '');
const SITES = [
  ['Tenor', (q) => `https://tenor.com/search/${dash(q)}-gifs`],
  ['Klipy', (q) => `https://klipy.com/search/${dash(q)}`],
  ['GIPHY', (q) => `https://giphy.com/search/${dash(q)}`],
  ['Google', (q) => `https://www.google.com/search?tbm=isch&tbs=itp:animated&q=${enc(q)}`],
  ['Bing', (q) => `https://www.bing.com/images/search?qft=+filterui:photo-animatedgif&q=${enc(q)}`],
  ['DuckDuckGo', (q) => `https://duckduckgo.com/?iax=images&ia=images&iaf=type:gif&q=${enc(q)}`],
  ['Reddit', (q, w) => `https://www.reddit.com/r/PixelArt/search/?restrict_sr=1&q=${enc(w || 'animated')}`],
  ['Tumblr', (q) => `https://www.tumblr.com/search/${encodeURIComponent(q.trim())}`],
  ['DeviantArt', (q) => `https://www.deviantart.com/search?q=${enc(q + ' gif')}`],
  ['Imgur', (q) => `https://imgur.com/search?q=${enc(q)}`],
];

function elsewhere() {
  const words = h('input', { type: 'search', id: 'c-words', placeholder: 'Your words (a game, a character…)', 'aria-label': 'Words to search for' });
  const links = SITES.map(([name]) => h('a.chip', { target: '_blank', rel: 'noopener noreferrer', 'data-site': name }, name));
  const set = () => {
    const w = words.value.trim().slice(0, 80);
    links.forEach((a, i) => { a.href = SITES[i][1](`pixel art ${w}`, w); });
  };
  words.addEventListener('input', set);
  set();
  const bm = h('a', { id: 'c-bm', href: `javascript:void(open('${location.origin}/create/?url='+encodeURIComponent(location.href)))` }, 'Open in Create Art');
  bm.addEventListener('click', (e) => e.preventDefault());
  $('st-own').append(h('div', { id: 'c-elsewhere' },
    h('div.lbl', 'Search for pixel art on'), h('div.row', h('div.grow', words)), h('div.chips', { id: 'c-chips' }, links),
    h('p.cap', 'Each opens in a new tab. Found one? Bring it back: paste its link or the picture here, or drag it onto this page.'),
    h('p.cap', 'On a computer, drag ', bm, ' to your bookmarks bar: pressed on a page with a GIF, it opens that page here.'),
    h('p.cap', framed ? 'Or draw it here in the Studio (close Import), or at ' : 'Or draw your own at ', h('a', { id: 'c-pixilart', href: 'https://www.pixilart.com/draw', target: '_blank', rel: 'noopener noreferrer' }, 'Pixilart'),
      ': make the canvas 128 × 32, then download it as a GIF and drop it here.')));
}
