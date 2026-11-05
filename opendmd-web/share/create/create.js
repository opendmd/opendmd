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
// Bringing a GIF from elsewhere (docs/research/tenor.md §7-8): under Your
// own, links that open other sites' own search in a new tab with "pixel
// art" and the visitor's words, and a bookmarklet that opens the page it
// is pressed on here (?url=). Installed on Android, the page is a share
// target (manifest.webmanifest, sw.js): a shared file arrives as
// ?shared=1 (put aside by the worker), a shared link as ?url=.

import { init } from '../demo/js/web.js';
import { app } from '../demo/js/state.js';
import { READY, IMPORT, MAX_GIF } from '../demo/js/importgif.js';
import { addr, addressForm, handOff } from '../panel.js';
import { h } from '../demo/js/util.js';

const $ = (id) => document.getElementById(id);
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
start().catch((e) => say('This page could not load: ' + (e.message || e)))
  .finally(() => $('create').removeAttribute('aria-busy'));

async function start() {
  await init();
  app.site = true;
  const { mount } = await import('../demo/js/studio.js');
  const send = Object.assign(document.createElement('button'), { id: 'c-send', className: 'pri', textContent: 'Send to my panel' });
  const st = await mount($('c-ed'), { css: false, acts: [send], relay, video: () => import('./video.js') });
  $('st-save').classList.remove('pri'); // (Send is the page's first way; Download the other)
  const q = new URLSearchParams(location.search);
  const words = (q.get('q') || '').trim().slice(0, 100);
  if (words) app.artFind = { q: words, platform: (q.get('platform') || '').toUpperCase() };
  await st.show();
  elsewhere();
  const link = (q.get('url') || '').trim();
  if (/^https?:\/\//i.test(link)) st.open(link.replace(/^http:/i, 'https:'));
  else if (q.get('shared')) shared(st);
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
    if (!handOff(a, 'import', READY, gif)) say('Your browser didn’t open the panel’s page: allow pop-ups for opendmd.dev, then press Send to my panel again, or download the GIF.');
  }
  const { ask } = addressForm('c', go, 'GIF');
  send.onclick = () => { if (addr()) go(addr()); else ask(); };
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
    h('p.cap', 'On a computer, drag ', bm, ' to your bookmarks bar: pressed on a page with a GIF, it opens that page here.')));
}
