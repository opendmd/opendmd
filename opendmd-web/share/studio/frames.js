// The Studio's frames and layers (/studio/), loaded once a piece has more
// than one of either, or one is added, or Play is pressed: the frames strip
// (one thumbnail a frame with its time; Play, onion skin, the frame's time,
// the plays), the layer rows (shown or hidden, held still or a cel a frame,
// renamed, moved up and down, duplicated, deleted). Each change is one undo
// step (doc.js shape): the piece's structure before and after, and the cels
// only one side has.
import { N, W, H, hexOf } from './doc.js';
import { CAPS } from './store.js';

let S = null, playing = 0, onion = false, plays = 0;
const $ = (id) => document.getElementById(id);
const el = (tag, cls, text) => { const e = document.createElement(tag); if (cls) e.className = cls; if (text !== undefined) e.textContent = text; return e; };
const D = () => S.doc;
const k8 = () => {
  const used = new Set([...D().frames.map((f) => f.key), ...D().layers.map((l) => l.key)]);
  for (;;) { const k = [...crypto.getRandomValues(new Uint8Array(4))].map((x) => x.toString(16).padStart(2, '0')).join(''); if (!used.has(k)) return k; }
};
const copy = (l, f) => D().celAt(l, f).px.slice();

export function mount(studio) {
  S = studio;
  const css = el('link');
  css.rel = 'stylesheet';
  css.href = new URL('frames.css', import.meta.url).href;
  // (The strip's controls shown once their styles are in: no flash of a long unstyled row.)
  css.onload = css.onerror = () => { bar.hidden = false; S.root.dataset.frames = 'yes'; };
  document.head.append(css);
  const on = el('canvas');
  on.id = 's-onion';
  on.width = W; on.height = H;
  $('s-canvas').after(on);
  // Play, onion skin, this frame's time and the plays, before the cells.
  const bar = el('div', 's-play');
  bar.hidden = true;
  bar.innerHTML = '<button class="s-pill s-pri" id="s-playb" title="Play (Space)">Play</button>'
    + '<button class="s-pill" id="s-onionb" aria-pressed="false" title="A ghost of the frame before, over this one">Onion skin</button>'
    + '<button class="s-pill s-fmorebtn" id="s-fmore-b" aria-expanded="false" aria-controls="s-fmore">Frame…</button>'
    + '<span class="s-fmore" id="s-fmore"><label class="s-time">Frame time <input id="s-ms" class="s-in" type="number" min="20" max="60000" step="10"> ms</label>'
    + '<button class="s-pill" id="s-msall" title="Every frame takes this frame’s time">All frames</button>'
    + '<label class="s-time" title="How many times it plays where a GIF’s loop count is kept, as in a browser; 0: forever. On the panel it plays for as long as its playlist keeps it on">Plays <input id="s-plays" class="s-in" type="number" min="0" max="65535"></label>'
    + '<span class="s-fmenu"><button class="s-pill" data-fa="dup" title="Duplicate this frame">Duplicate</button><button class="s-pill" data-fa="left" title="Move this frame earlier">◀</button>'
    + '<button class="s-pill" data-fa="right" title="Move this frame later">▶</button><button class="s-pill" data-fa="del" title="Delete this frame">Delete</button></span></span>';
  $('s-strip').prepend(bar);
  $('s-playb').onclick = toggle;
  $('s-fmore-b').onclick = () => { const o = $('s-fmore').classList.toggle('open'); $('s-fmore-b').setAttribute('aria-expanded', o); };
  $('s-onionb').onclick = () => { onion = !onion; $('s-onionb').setAttribute('aria-pressed', onion); $('s-onionb').classList.toggle('on', onion); S.paint(); };
  $('s-ms').onchange = () => retime(+$('s-ms').value, false);
  $('s-msall').onclick = () => retime(+$('s-ms').value, true);
  $('s-plays').onchange = () => setPlays(+$('s-plays').value);
  bar.querySelector('.s-fmenu').onclick = (e) => {
    const a = e.target.closest('[data-fa]')?.dataset.fa;
    if (a === 'dup') addFrame(true); else if (a === 'del') delFrame(); else if (a) moveFrame(a === 'left' ? -1 : 1);
  };
  $('s-cells').onclick = (e) => { const b = e.target.closest('[data-f]'); if (b) show(+b.dataset.f); };
  $('s-lrows').onclick = layerAct;
  $('s-lrows').onchange = (e) => { const i = e.target.closest('[data-l]'); if (i && e.target.matches('input')) rename(+i.dataset.l, e.target.value); };
  Object.assign(S.fx, { painted, changed: () => render(), opened: () => { stop(); render(); }, stop, step: (d) => show(D().F + d) });
  render();
}

// --- what is shown ---
function thumb(c, keys, cols) {
  const g = c.getContext('2d'), img = g.createImageData(W, H);
  for (let i = 0; i < N; i++) { const v = keys[i] ? cols.get(keys[i]) ?? 0 : 0; img.data.set([v >> 16, (v >> 8) & 255, v & 255, 255], i * 4); }
  g.putImageData(img, 0, 0);
}
const canvas = () => { const c = el('canvas'); c.width = W; c.height = H; return c; };
let drawing = 0;
function render() {
  const d = D(), f = d.F, n = ++drawing;
  $('s-cells').replaceChildren(...d.frames.map((fr, i) => {
    const b = el('button', 's-cell' + (i === f ? ' on' : '')), c = canvas();
    b.dataset.f = i;
    if (i === f) b.setAttribute('aria-current', 'true');
    b.title = `Frame ${i + 1} · ${fr.ms} ms`;
    b.append(c, el('b', '', String(i + 1)), el('small', '', `${fr.ms} ms`));
    return b;
  }));
  $('s-cells').children[f]?.scrollIntoView?.({ block: 'nearest', inline: 'nearest' });
  // The thumbnails, frame by frame (each read as it is drawn).
  (async () => {
    for (let i = 0; i < d.frames.length && n === drawing && d === D(); i++) {
      await d.load(i);
      const c = $('s-cells').children[i]?.querySelector('canvas');
      if (c) thumb(c, d.composite(i), d.cols);
    }
  })().catch(() => {});
  $('s-ms').value = d.frames[f].ms;
  $('s-plays').value = d.rest.plays || 0;
  S.root.dataset.frame = `${f + 1}/${d.frames.length}`;
  const rows = d.layers.map((l, i) => {
    const li = el('li', i === d.L ? 'on' : ''), c = canvas(), name = el('input', 's-in s-lname');
    li.dataset.l = i;
    name.value = l.name;
    name.maxLength = CAPS.layerName;
    name.setAttribute('aria-label', 'Layer name');
    const b = (a, text, title) => { const x = el('button', 's-pill', text); x.dataset.la = a; x.title = title; return x; };
    const eye = b('hide', l.hide ? 'Hidden' : 'Shown', l.hide ? 'Show this layer' : 'Hide this layer (it is then in no GIF and no send)');
    eye.setAttribute('aria-pressed', !l.hide);
    const mode = b('moves', l.moves ? 'A CEL A FRAME' : 'HELD STILL', l.moves ? 'Hold it still: this frame’s picture on every frame' : 'Give it a picture for each frame');
    mode.classList.add('s-mode');
    thumb(c, l.cels[l.moves ? f : 0].px || new Uint16Array(N), d.cols);
    li.append(eye, c, name, mode, b('up', 'Move up', 'Move up'), b('down', 'Move down', 'Move down'), b('dupl', 'Duplicate', 'Duplicate this layer'), b('dell', 'Delete', 'Delete this layer'));
    return li;
  }).reverse(); // the top layer first
  $('s-lrows').replaceChildren(...rows);
  S.root.dataset.layers = d.layers.map((l) => `${l.name}${l.hide ? '-' : ''}`).join('|');
  S.root.dataset.active = d.L;
}
// After each paint: the onion skin, and this frame's thumbnail and the active layer's.
// The onion skin: a ghost of the frame before, over this one, where it
// differs: the layer drawn on if it moves (what an animator steps), else
// the whole picture. Off pixels are see-through.
function painted() {
  const d = D(), on = $('s-onion');
  if (!on) return;
  on.hidden = !onion || d.frames.length < 2 || playing;
  if (!on.hidden) {
    const p = (d.F + d.frames.length - 1) % d.frames.length, own = d.layers[d.L].moves;
    d.load(p).then(() => {
      const here = own ? d.px : d.composite(), prev = own ? d.celAt(d.L, p).px : d.composite(p);
      const g = on.getContext('2d'), img = g.createImageData(W, H);
      for (let i = 0; i < N; i++) {
        const k = prev[i];
        if (!k || k === here[i]) continue;
        const v = d.cols.get(k) ?? 0;
        img.data.set([v >> 16, (v >> 8) & 255, v & 255, 255], i * 4);
      }
      g.putImageData(img, 0, 0);
    }).catch(() => {});
  }
  const c = $('s-cells').children[d.F]?.querySelector('canvas');
  if (c) thumb(c, d.composite(), d.cols);
  const r = $('s-lrows').querySelector(`[data-l="${d.L}"] canvas`);
  if (r) thumb(r, d.px, d.cols);
}
// A stroke or a drag in progress (a second finger on the strip): nothing moves under it.
const busy = () => !!(D().stroke || S.dragging);
async function show(f) {
  const d = D();
  if (f < 0 || f >= d.frames.length || S.sel?.fl || busy()) return;
  d.F = f;
  await d.load();
  S.showSel();
  S.paint();
  render();
}

// --- Play: frame after frame, each its time; `plays` times round, or until stopped ---
function toggle() { if (playing) stop(); else play(); }
function play() {
  const d = D();
  if (d.frames.length < 2 || busy() || S.sel?.fl) return; // not over a stroke, nor while a paste floats
  S.done();
  const n = ++playing, start = d.F;
  plays = 0;
  $('s-playb').textContent = 'Stop';
  S.root.dataset.playing = 'yes';
  const next = async () => {
    if (n !== playing || d !== D()) return;
    let f = d.F + 1;
    if (f >= d.frames.length) { f = 0; if (d.rest.plays && ++plays >= d.rest.plays) { stop(); return; } }
    d.F = f;
    await d.load();
    if (n !== playing) return;
    S.paint();
    S.root.dataset.shown = f; // (what a check watches)
    setTimeout(next, d.frames[f].ms);
  };
  d.F = start;
  setTimeout(next, d.frames[start].ms);
}
function stop() {
  if (!playing) return;
  playing = 0;
  $('s-playb').textContent = 'Play';
  delete S.root.dataset.playing;
  render();
}

// --- changes: each one step ---
let chain = Promise.resolve();
function change(f) {
  if (busy()) return chain;
  stop();
  S.done();
  chain = chain.then(async () => {
    const r = await f();
    if (!r) return;
    S.after(r);
    S.buttons();
    S.showSel();
    S.paint();
  }).catch((e) => S.say(`That can’t be done: ${e.message || e}`));
  return chain;
}
const no = (why) => { S.say(why); return null; };
// A new frame after this one: blank, or (dup) this one again. A piece none of
// whose layers moves makes the layer drawn on move, in the same step.
export function addFrame(dup = false) {
  return change(async () => {
    const d = D(), a = d.skel(), f = d.F, made = {};
    if (a.frames.length >= CAPS.frames) return no(`A piece has ${CAPS.frames} frames at most.`);
    S.say('');
    const key = k8(), nf = { key, ms: a.frames[f].ms };
    if (!a.layers.some((l) => l.moves)) {
      const l = a.layers[d.L];
      l.moves = true;
      for (const fr of a.frames) made[`${l.key}/${fr.key}`] = copy(d.L, 0);
    }
    a.frames.splice(f + 1, 0, nf);
    a.layers.forEach((l, i) => { if (l.moves) made[`${l.key}/${key}`] = dup ? (made[`${l.key}/${a.frames[f].key}`] || copy(i, f)).slice() : new Uint16Array(N); });
    await d.load();
    const r = await d.shape(a, made);
    d.F = f + 1;
    await d.load();
    return r;
  });
}
function delFrame() {
  return change(async () => {
    const d = D(), a = d.skel();
    if (a.frames.length < 2) return no('A piece has one frame at least.');
    S.say('');
    a.frames.splice(d.F, 1);
    return d.shape(a);
  });
}
function moveFrame(by) {
  return change(async () => {
    const d = D(), a = d.skel(), f = d.F, to = f + by;
    if (to < 0 || to >= a.frames.length) return null;
    a.frames.splice(to, 0, ...a.frames.splice(f, 1));
    return d.shape(a);
  });
}
// A GIF keeps times in hundredths of a second: a frame's time is held to tens of ms.
function retime(ms, all) {
  return change(async () => {
    const d = D(), a = d.skel();
    if (!Number.isFinite(ms) || ms < CAPS.msMin || ms > CAPS.msMax) { render(); return no(`A frame lasts ${CAPS.msMin} to ${CAPS.msMax.toLocaleString('en')} ms.`); }
    ms = Math.round(ms / 10) * 10;
    S.say('');
    for (const [i, fr] of a.frames.entries()) if (all || i === d.F) fr.ms = ms;
    if (a.frames.every((fr, i) => fr.ms === d.frames[i].ms)) return null;
    return d.shape(a);
  });
}
function setPlays(n) {
  return change(async () => {
    const d = D(), a = d.skel();
    if (!Number.isInteger(n) || n < 0 || n > 65535) { render(); return no('Plays is a whole number: 0 for forever.'); }
    if (n === a.plays) return null;
    a.plays = n;
    return d.shape(a);
  });
}
// A new layer above the one drawn on: blank, or (dup) that one again.
export function addLayer(dup = false, at = D().L) {
  return change(async () => {
    const d = D(), a = d.skel(), made = {};
    if (a.layers.length >= CAPS.layers) return no(`A piece has ${CAPS.layers} layers at most.`);
    S.say('');
    await d.load();
    const src = a.layers[at], key = k8(), l = dup ? { ...src, key, name: `${src.name} copy`.slice(0, CAPS.layerName) } : { key, name: `Layer ${a.layers.length + 1}`, moves: false };
    delete l.cels;
    if (!dup) delete l.hide;
    for (const [i, fr] of (l.moves ? a.frames : [null]).entries()) {
      if (dup && l.moves) await d.load(i);
      made[fr ? `${key}/${fr.key}` : key] = dup ? copy(at, fr ? i : d.F) : new Uint16Array(N);
    }
    a.layers.splice(at + 1, 0, l);
    const r = await d.shape(a, made);
    d.L = at + 1;
    return r;
  });
}
function delLayer(at) {
  return change(async () => {
    const d = D(), a = d.skel();
    if (a.layers.length < 2) return no('A piece has one layer at least.');
    S.say('');
    a.layers.splice(at, 1);
    if (d.L >= at && d.L > 0) d.L--;
    return d.shape(a);
  });
}
function moveLayer(at, by) {
  return change(async () => {
    const d = D(), a = d.skel(), to = at + by;
    if (to < 0 || to >= a.layers.length) return null;
    a.layers.splice(to, 0, ...a.layers.splice(at, 1));
    return d.shape(a);
  });
}
// Held still <-> a cel a frame: moving, every frame starts as the held
// picture; held, the picture of the frame shown stays.
function toggleMoves(at) {
  return change(async () => {
    const d = D(), a = d.skel(), l = a.layers[at], made = {};
    await d.loadAll();
    if (l.moves) made[l.key] = copy(at, d.F);
    else for (const fr of a.frames) made[`${l.key}/${fr.key}`] = copy(at, 0);
    l.moves = !l.moves;
    return d.shape(a, made);
  });
}
function toggleHide(at) {
  return change(async () => {
    const d = D(), a = d.skel(), l = a.layers[at];
    if (l.hide) delete l.hide; else l.hide = true;
    return d.shape(a);
  });
}
function rename(at, name) {
  return change(async () => {
    const d = D(), a = d.skel(), v = name.trim().slice(0, CAPS.layerName);
    if (v === a.layers[at].name) return null;
    a.layers[at].name = v;
    return d.shape(a);
  });
}
function layerAct(e) {
  const li = e.target.closest('[data-l]'), a = e.target.closest('[data-la]')?.dataset.la;
  if (!li || busy()) return;
  const at = +li.dataset.l;
  if (!a) { if (!e.target.matches('input') && !S.sel?.fl) { D().L = at; render(); } return; }
  ({ hide: () => toggleHide(at), moves: () => toggleMoves(at), up: () => moveLayer(at, 1), down: () => moveLayer(at, -1),
    dupl: () => addLayer(true, at), dell: () => delLayer(at) })[a]?.();
}
export { toggle, hexOf };
