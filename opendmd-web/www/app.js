// Entry point: load the wasm module, find out whether a device is serving us
// (GET /api/status) or we are a static page on a laptop (local mode), and
// route between the five views (Formulas, js/formulas.js, has a stylesheet
// and wasm of its own that load when it is first opened). Each view is a
// module with `mount(el) -> {show, hide}`, mounted on first visit. Making
// art from the web is the website's (its Studio): `#create` and `#studio`,
// the addresses of the Create Art view the app once had, lead to the
// Library, where Add GIFs and Find art online are.

import { init } from './js/web.js';
import { app, NO_FORMULAS } from './js/state.js';
import { detect, demoCard, LocalSource } from './js/source.js';
import { $, h, clear, emit, on, toast } from './js/util.js';
import { closeMenu } from './js/overrides.js';

await init();

const VIEWS = ['now', 'library', 'sets', 'formulas', 'settings'];
// name -> the mount promise; ready[name] once it has resolved.
const mounted = {}, ready = {};
let current = '', shown = '';

// `#view` or `#view/sub` (Settings' tabs, the Playlists editor): a view's
// `show(sub)` runs when it comes up, `sub(sub)` on every route to it. A
// hash change while a view is still mounting (or before the source is
// known) mounts whatever the hash says once it can.
async function route() {
  // Old addresses: Create Art's, and the editor's when Formulas were a Library tab.
  if (/^#(create|studio)(\/|$)/.test(location.hash)) { location.replace('#library'); return; }
  if (location.hash === '#library/formula') { location.replace('#formulas/edit'); return; }
  // A Formula from the site (its Open on my panel: `#gallery=<slug>` or a
  // short link's `#remix=<id>`, never the text itself): Formulas fetches it
  // and opens it in the editor (js/formulas.js keeps it in app.remix).
  // A GIF from the website (its Studio's Send to panel, `#import`): sent
  // by postMessage (js/importgif.js), then the Library's Add GIFs, filled in.
  if (/^#import(&|$)/.test(location.hash)) {
    app.gifIn = import('./js/importgif.js').then((m) => m.received());
    app.gifIn.catch(() => {}); // (said by the Library, which may mount after a refusal)
    location.replace('#library');
    return;
  }
  const ref = /^#(gallery=|remix(&|$))/.test(location.hash) && (await import('./js/psikit.js')).from(location.hash);
  if (ref) app.remix = ref;
  const [v, sub = ''] = ref ? ['formulas', ''] : location.hash.slice(1).split('/');
  const name = VIEWS.includes(v) ? v : 'now';
  if (name !== current) {
    if (shown) ready[shown]?.hide?.();
    shown = '';
    closeMenu(); // a menu or sheet belongs to the view it opened in
    // Safari keeps focus (and the keyboard) in a field a nav tap hides.
    document.activeElement?.blur?.();
    current = name;
    for (const x of VIEWS) $('#view-' + x).classList.toggle('on', x === name);
    for (const a of document.querySelectorAll('#nav a')) a.classList.toggle('on', a.dataset.view === name);
  }
  if (!app.src) return;
  // A player built without Formulas: their view is one plain note (no editor, no wasm).
  const load = name === 'formulas' && app.noFormulas ? Promise.resolve({ mount: noFormulas })
    : import(`./js/${name}.js`);
  const m = await (mounted[name] ??= load.then((mod) => mod.mount($('#view-' + name))).then((r) => (ready[name] = r || {})));
  if (current !== name || app.src == null) return;
  if (shown !== name) { shown = name; m.show?.(sub); }
  m.sub?.(sub);
}

// The Formulas view on a player built without them.
function noFormulas(el) {
  el.append(h('div.card', { id: 'f-none' }, h('h2', 'Formulas'), h('p', NO_FORMULAS + '.'),
    h('p.cap', 'This display plays your GIFs and playlists; a playlist’s Formulas are skipped.')));
}

function setSource(src) {
  // (The view up lets go first: its watchers and streams belong to the old source.)
  if (shown) ready[shown]?.hide?.();
  app.src = src;
  for (const k of Object.keys(mounted)) delete mounted[k], delete ready[k];
  for (const v of VIEWS) clear($('#view-' + v));
  current = shown = '';
  emit('source', src);
  route();
}

function localPicker() {
  const card = $('#t-local').content.cloneNode(true);
  const pick = (e) => {
    const files = e.target.files;
    if (!files.length) return;
    const src = new LocalSource(files, app.wasm.cardPath);
    $('#where').textContent = `${src.label || 'Picked files'} · ${src.count} files`;
    toast(`${src.count} files picked — nothing is read until it plays`);
    setSource(src);
  };
  card.querySelector('#pick-dir').addEventListener('change', pick);
  card.querySelector('#pick-files').addEventListener('change', pick);
  return card;
}

addEventListener('hashchange', route);
const found = await detect();
if (found) {
  app.src = found.src;
  app.status = found.status;
  const s = found.status;
  // The device's name; its address and version are in Settings → About.
  $('#where').textContent = s.hostname || 'OpenDMD display';
  // What the panel is doing, on every view (js/pill.js).
  (await import('./js/pill.js')).pill();
  // Power-safe (the ESP32-S3's brownout-loop guard, `reset_note` "power-safe: …"):
  // the panel was not started this boot; one banner until a restart starts it.
  const powerSafe = (st) => {
    const ps = /^power-safe:/.test(st?.reset_note || '');
    if (ps && !$('#powersafe')) $('#main').prepend(h('div.banner', { id: 'powersafe', role: 'alert' }, "The panel wasn't started: the display's power supply kept dipping and it kept resetting (a weak USB supply can do this too). Check the 5 V supply, then restart the display."));
    else if (!ps) $('#powersafe')?.remove();
  };
  on('status', powerSafe);
  powerSafe(s);
  app.refreshConfig().catch((e) => toast('Couldn’t read the settings: ' + e.message, 'bad'));
  route();
} else if ((app.demo = await demoCard(app.wasm.cardPath))) {
  // The web demo: its own card, in memory; a card folder of yours can replace it.
  // (`?desk`: inside the site's desk scene, whose page has the way back and no folder to pick.)
  const desk = new URLSearchParams(location.search).has('desk');
  $('#where').textContent = desk ? 'The display on the desk · runs in your browser' : 'Demo · runs in your browser, nothing is uploaded';
  if (!desk) {
    const own = h('label.btn.sm.demo-own', 'Use your own card…', h('input', { type: 'file', webkitdirectory: true, hidden: true, onchange: (e) => {
      const files = e.target.files;
      if (!files.length) return;
      const src = new LocalSource(files, app.wasm.cardPath);
      app.demo = null;
      own.remove();
      $('#where').textContent = `${src.label || 'Picked files'} · ${src.count} files`;
      setSource(src);
    } }));
    // The way back to the site, and to installing it on a display.
    $('#top').append(own, h('span.demo-site', h('a', { href: '../' }, '‹ opendmd.dev'), h('a', { href: '../install/' }, 'Install')));
    $('#top strong').replaceChildren(h('a', { href: '../', style: 'color:inherit;text-decoration:none' }, 'OpenDMD'));
  }
  // Its player's settings and status (js/demo.js).
  await (await import('./js/demo.js')).simulate(app.demo);
  setSource(app.demo);
} else {
  $('#where').textContent = 'Local mode';
  for (const v of VIEWS) $('#view-' + v).append(h('div', localPicker()));
  route();
  // Re-offer the picker from any view until something is picked.
  addEventListener('hashchange', () => { if (!app.src) route(); });
}
