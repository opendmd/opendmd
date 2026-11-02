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
// Library links here with its words).

import init from '../demo/pkg/opendmd_web.js';
import { app } from '../demo/js/state.js';
import { READY, IMPORT, MAX_GIF } from '../demo/js/importgif.js';
import { addr, addressForm, handOff } from '../panel.js';

const $ = (id) => document.getElementById(id);
const say = (t) => { $('c-cap').hidden = !t; $('c-cap').textContent = t || ''; };

// #create is aria-busy until the Studio is up: what a test waits for.
start().catch((e) => say('This page could not load: ' + (e.message || e)))
  .finally(() => $('create').removeAttribute('aria-busy'));

async function start() {
  await init();
  app.site = true;
  const { mount } = await import('../demo/js/studio.js');
  const send = Object.assign(document.createElement('button'), { id: 'c-send', className: 'pri', textContent: 'Send to my panel' });
  const st = await mount($('c-ed'), { css: false, acts: [send] });
  $('st-save').classList.remove('pri'); // (Send is the page's first way; Download the other)
  const q = new URLSearchParams(location.search);
  const words = (q.get('q') || '').trim().slice(0, 100);
  if (words) app.artFind = { q: words, platform: (q.get('platform') || '').toUpperCase() };
  await st.show();

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
