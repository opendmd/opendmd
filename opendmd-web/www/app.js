// Entry point: load the wasm module, find out whether a device is serving us
// (GET /api/status) or we are a static page on a laptop (local mode), and
// route between the five views (Make art is the Studio, js/studio.js, whose
// stylesheet and wasm load when it is first opened). Each view is a module with
// `mount(el) -> {show, hide}`, mounted on first visit.

import init from './pkg/opendmd_web.js';
import { app } from './js/state.js';
import { detect, LocalSource } from './js/source.js';
import { $, h, clear, emit, toast } from './js/util.js';

await init();

const VIEWS = ['now', 'library', 'studio', 'sets', 'settings'];
const mounted = {};
let current = '';

async function route() {
  const name = VIEWS.includes(location.hash.slice(1)) ? location.hash.slice(1) : 'now';
  if (name === current) return;
  if (current) mounted[current]?.hide?.();
  current = name;
  for (const v of VIEWS) $('#view-' + v).classList.toggle('on', v === name);
  for (const a of document.querySelectorAll('#nav a')) a.classList.toggle('on', a.dataset.view === name);
  if (!app.src) return;
  if (!mounted[name]) {
    const mod = await import(`./js/${name}.js`);
    mounted[name] = await mod.mount($('#view-' + name));
  }
  mounted[name].show?.();
}

function setSource(src) {
  app.src = src;
  for (const k of Object.keys(mounted)) delete mounted[k];
  for (const v of VIEWS) clear($('#view-' + v));
  current = '';
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
  $('#where').textContent = s.hostname || 'OpenDMD player';
  app.refreshConfig().catch((e) => toast('config: ' + e.message, 'bad'));
  route();
} else {
  $('#where').textContent = 'Local mode';
  for (const v of VIEWS) $('#view-' + v).append(h('div', localPicker()));
  route();
  // Re-offer the picker from any view until something is picked.
  addEventListener('hashchange', () => { if (!app.src) route(); });
}
