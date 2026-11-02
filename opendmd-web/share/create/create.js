// Create Art on the site (/create/): the web app's own Studio (js/studio.js
// `mount`, from the demo's files), with every source of pictures — Game art,
// GifCities (js/find.js), your own file, drop, paste or address — and every
// editing step. Save downloads the GIF (the Studio with no card).
//
// `?q=<words>&platform=<P>` searches at once: Game art on that platform when
// it has one, else GifCities (the Studio's `app.artFind`, as the web app's
// Library hands its words over).

import init from '../demo/pkg/opendmd_web.js';
import { app } from '../demo/js/state.js';

const $ = (id) => document.getElementById(id);

// #create is aria-busy until the Studio is up: what a test waits for.
start().catch((e) => { $('c-cap').hidden = false; $('c-cap').textContent = 'This page could not load: ' + (e.message || e); })
  .finally(() => $('create').removeAttribute('aria-busy'));

async function start() {
  await init();
  app.site = true;
  const { mount } = await import('../demo/js/studio.js');
  const st = await mount($('c-ed'), { css: false });
  const q = new URLSearchParams(location.search);
  const words = (q.get('q') || '').trim().slice(0, 100);
  if (words) app.artFind = { q: words, platform: (q.get('platform') || '').toUpperCase() };
  await st.show();
}
