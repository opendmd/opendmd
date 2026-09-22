// Deliberately thin. This file reads the files the user picked, wires the
// controls to the wasm module, and runs requestAnimationFrame. Everything
// else — filtering, scheduling, GIF decoding, the LED look, the status line —
// is Rust in `../src/lib.rs`.

import init, { DmdApp } from './pkg/opendmd_web.js';

await init();
DmdApp.init();

const $ = (id) => document.getElementById(id);
const status = $('status');

// Files the user picked: [name, Uint8Array] pairs, read once and kept in JS
// until a play starts, then handed to wasm.
let names = [];
let blobs = [];
let app = null;
let raf = 0;

// Chromium and Safari support it; Firefox does too but hide it where it isn't.
if ('webkitdirectory' in document.createElement('input')) {
  $('dir-label').hidden = false;
}

async function load(fileList) {
  const files = [...fileList].filter((f) => f.name.toLowerCase().endsWith('.gif'));
  if (!files.length) {
    status.textContent = 'no .gif files in that selection';
    return;
  }
  status.textContent = `reading ${files.length} files…`;
  names = files.map((f) => f.webkitRelativePath || f.name);
  blobs = await Promise.all(files.map(async (f) => new Uint8Array(await f.arrayBuffer())));
  status.textContent = `${files.length} GIFs in memory — press play`;
  $('play').disabled = false;
}

$('files').addEventListener('change', (e) => load(e.target.files));
$('dir').addEventListener('change', (e) => load(e.target.files));

// The three sliders and the flat checkbox are one control group: read them all
// and let Rust rebuild the renderer.
const look = () => [
  parseFloat($('gamma').value),
  parseFloat($('gain').value),
  parseFloat($('bloom').value),
  $('flat').checked,
];

for (const id of ['gain', 'bloom', 'gamma']) {
  const el = $(id);
  const out = $(`${id}-out`);
  const show = () => { out.textContent = parseFloat(el.value).toFixed(2); };
  el.addEventListener('input', () => { show(); app?.setLook(...look()); });
  show();
}
$('bloom').value = DmdApp.defaultBloom();
$('bloom').dispatchEvent(new Event('input'));
$('flat').addEventListener('change', () => app?.setLook(...look()));

$('play').addEventListener('click', () => {
  cancelAnimationFrame(raf);
  const [gamma, gain, bloom, flat] = look();
  try {
    app = new DmdApp(
      'panel', 'status', names, blobs,
      $('platforms').value, $('shuffle').checked, Number($('seed').value),
      gamma, gain, bloom, flat,
    );
  } catch (e) {
    status.textContent = String(e);
    return;
  }
  $('next').disabled = $('pause').disabled = false;
  $('pause').textContent = 'pause';
  const loop = () => { if (app.frame()) raf = requestAnimationFrame(loop); };
  raf = requestAnimationFrame(loop);
});

$('next').addEventListener('click', () => app?.skip());
$('pause').addEventListener('click', () => {
  $('pause').textContent = app?.togglePause() ? 'resume' : 'pause';
});
