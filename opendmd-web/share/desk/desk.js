// The demo's page (/demo/): a display on a desk beside the web app (app.html?desk, in a frame).
// This module owns the page: the buttons, the room's lights, which way the display faces, the
// status line, the part links of its back, and the device (the app's simulated player:
// window.opendmdDemo, js/demo.js). A scene draws that state and nothing else, through one
// contract, so either can be mounted, and the 3D one can hand over to the flat one:
//   start(ui) -> { show(), turn(back), hot(id), device(dev), run(on), layout(), dispose(), turnMs }
// Which scene: the 3D one (hd/scene.js: three.js, fetched after first paint, behind a still) on a
// machine with a fast WebGL2; else the flat one (flat.js: layers of pixel art and a CSS box).
//   ?scene=hd | flat   force one (hd: on any WebGL2; only a lost context ends it)
//   ?scene=try         3D on any WebGL2, slow frames still step down and hand over
//   &q=0..4            the 3D scene's quality level (pinned with hd, the start with try)
//   ?hud=1             its GPU, fps, cadence and level    ?light=0..100 the room's lights    ?back=1 turned round
import { PARTS, GUIDE, partLabel, tipHtml, says } from './parts.js';

const $ = (s) => document.querySelector(s);
const q = new URLSearchParams(location.search);
const mm = (m) => matchMedia(m).matches;
const clamp = (v, a, b) => Math.max(a, Math.min(b, v));
const stage = $('#stage'), view = $('#view'), host = $('#scene'), frame = $('#app'), partsEl = $('#parts'), tip = $('#tip');
const bPower = $('#b-power'), bTurn = $('#b-turn'), bCard = $('#b-card'), bLight = $('#b-light'), say = $('#say'), grab = $('#grab');
const still = mm('(prefers-reduced-motion:reduce)'), dark = mm('(prefers-color-scheme:dark)');
const NARROW = matchMedia('(max-width:820px)');
// A slow 3D visit is remembered for a week (a lost context is not slowness); a healthy one clears it.
const KEY = 'opendmd-desk-flat', WEEK = 7 * 864e5;
// light: 0..100 (the system's theme: lights on in the light one, dimmed in the dark one)
const S = { light: clamp(+(q.get('light') ?? (dark ? 6 : 100)) || 0, 0, 100), back: false, W: null, hot: '', folded: false };
let dev = null, scene = null, seen = true, cardT = 0, mounting = 0;

// --- the back's parts: real links (a labelled group after the controls, outside the aria-hidden
// picture; the card's a button: it pulls the card); the scene places them over what they name,
// and shows them only while it is turned round
const partEl = {};
for (const p of PARTS) {
  const sd = p[0] == 'sd';
  const a = Object.assign(document.createElement(sd ? 'button' : 'a'), { className: 'part', id: 'p-' + p[0] });
  if (sd) { a.type = 'button'; a.onclick = () => act.card(); } else a.href = GUIDE + p[3];
  a.setAttribute('aria-label', partLabel(p, false));
  a.onpointerenter = a.onfocus = () => hot(p[0]);
  a.onpointerleave = a.onblur = () => hot('');
  partsEl.append(a);
  partEl[p[0]] = a;
}
function parts(show) {
  if (partsEl.hidden == !show) return;
  if (show) fitParts();
  partsEl.hidden = !show;
  if (!show && S.hot) hot('');
}
// (the group comes after the controls in the page, so Tab reaches the buttons first; it lies over the picture)
function fitParts() { partsEl.style.top = view.offsetTop + 'px'; partsEl.style.height = view.offsetHeight + 'px'; }
// A part is pointed at: its name sits by it (above if there is room), the scene lights it.
function hot(id) {
  S.hot = id;
  for (const k in partEl) partEl[k].classList.toggle('on', k === id);
  const p = PARTS.find((x) => x[0] === id);
  tip.classList.toggle('on', !!p);
  if (p) { tip.innerHTML = tipHtml(p, S.W && !S.W.card); place(); }
  scene?.hot(id);
}
function place() {
  if (!S.hot) return;
  const b = partEl[S.hot].getBoundingClientRect(), sc = view.getBoundingClientRect(), mid = b.left + b.width / 2 - sc.left;
  const x = clamp(mid - tip.offsetWidth / 2, 8, sc.width - tip.offsetWidth - 8), up = b.top - sc.top - tip.offsetHeight - 10;
  tip.classList.toggle('below', up < 6);
  tip.style.transform = `translate(${Math.round(x)}px,${Math.round(up < 6 ? b.bottom - sc.top + 10 : up)}px)`;
  tip.style.setProperty('--ax', clamp(mid - x, 14, tip.offsetWidth - 14) + 'px');
}
// where a part's link goes, in the view's pixels
function box(id, x, y, w, h) {
  const s = partEl[id].style;
  s.transform = `translate(${x.toFixed(1)}px,${y.toFixed(1)}px)`; s.width = w.toFixed(1) + 'px'; s.height = h.toFixed(1) + 'px';
}

// --- what the buttons (and the things in the picture) do
const act = {
  power: () => S.W && dev?.power(!S.W.power),
  // From the front the display turns round first, so the card is seen leaving (not under reduced motion).
  card: () => {
    if (!dev || !S.W) return;
    clearTimeout(cardT);
    if (!S.back && S.W.card && !still) { turn(true); cardT = setTimeout(() => S.back && dev.card(false), (scene?.turnMs ?? 1200) + 250); } else dev.card(!S.W.card);
  },
  turn: (b = !S.back) => turn(b),
  cycle: () => { S.light = S.light > 60 ? 30 : S.light > 4 ? 0 : 100; show(); },
};
// (a button's words say what it does next; its colour says which way things are)
const label = (b, on, x, z) => { b.classList.toggle('on', on); b.querySelector('span').textContent = on ? z : x; };
let said = '';
function show() {
  const W = S.W;
  stage.classList.toggle('back', S.back);
  stage.classList.toggle('off', !!W && !W.power);
  stage.classList.toggle('nocard', !!W && !W.card);
  bLight.value = S.light;
  label(bTurn, S.back, 'Turn it round', 'Turn it back');
  label(bPower, !!W && !W.power, 'Unplug the power', 'Plug it back in');
  label(bCard, !!W && !W.card, 'Pull the SD card', 'Put the card back');
  partEl.sd.setAttribute('aria-label', partLabel(PARTS[5], !!W && !W.card));
  const t = says(W, S.back);
  if (t != said) say.textContent = said = t;
  if (S.hot) hot(S.hot);
  scene?.show();
}
function turn(back) {
  if (back === S.back) return;
  S.back = back;
  if (back && S.folded) fold(false);
  if (!back) { clearTimeout(cardT); parts(false); }
  scene?.turn(back);
  show();
}
function fold(v) {
  S.folded = v;
  stage.classList.toggle('folded', v);
  grab.setAttribute('aria-expanded', !v); grab.setAttribute('aria-label', v ? 'Show the desk' : 'Hide the desk');
  if (v && S.back) turn(false);
  scene?.layout();
}
bPower.onclick = act.power;
bCard.onclick = act.card;
bTurn.onclick = () => turn(!S.back);
bLight.oninput = () => { S.light = +bLight.value; show(); };
grab.onclick = () => fold(!S.folded);
addEventListener('keydown', (e) => {
  if (e.key != 'Escape' || !S.back) return;
  const inParts = partsEl.contains(document.activeElement);
  turn(false);
  if (inParts) bTurn.focus();
});

// --- the device: the app's own simulated player, in the frame. Asked for until it answers (a slow
// first load of the app's wasm), again each time the frame loads.
let asking = 0;
frame.addEventListener('load', async () => {
  const me = ++asking, w = frame.contentWindow;
  while (me == asking && !w.opendmdDemo) await new Promise((f) => setTimeout(f, 100));
  if (me != asking) return;
  dev = w.opendmdDemo;
  dev.on((W) => { S.W = W; show(); });
  scene?.device(dev);
  vis();
});
frame.src = 'app.html?desk#now';

// --- out of sight (a hidden tab, the scene scrolled away): the scene and the player both stop
function vis() {
  const on = !document.hidden && seen;
  scene?.run(on);
  dev?.run(on);
}
document.addEventListener('visibilitychange', vis);
new IntersectionObserver((es) => { seen = es[es.length - 1].isIntersecting; vis(); }).observe(view);
new ResizeObserver(() => { scene?.layout(); if (!partsEl.hidden) fitParts(); place(); }).observe(view);

// --- which scene
const ui = { S, q, still, dark, act, host, view, stage, parts, box, place, partEl };
function fastGl() {
  try {
    const gl = document.createElement('canvas').getContext('webgl2', { failIfMajorPerformanceCaveat: true });
    if (!gl) return false;
    const e = gl.getExtension('WEBGL_debug_renderer_info'), name = e ? gl.getParameter(e.UNMASKED_RENDERER_WEBGL) : '';
    gl.getExtension('WEBGL_lose_context')?.loseContext();
    return !/swiftshader|llvmpipe|softpipe|software|basic render/i.test(name);
  } catch { return false; }
}
function why() {
  const f = q.get('scene');
  if (f == 'flat') return 'asked';
  if (f == 'hd' || f == 'try') { try { return document.createElement('canvas').getContext('webgl2') ? '' : 'no WebGL2'; } catch { return 'no WebGL2'; } }
  if (still) return 'reduced motion';
  if (NARROW.matches) return 'small screen';
  // the LED face must get about 5 device pixels an LED with the room cropped to 34 cm a side (hd/scene.js)
  if ((innerWidth - 416) * (devicePixelRatio || 1) < 850) return 'small scene';
  if (navigator.connection?.saveData) return 'save-data';
  if (navigator.deviceMemory && navigator.deviceMemory < 4) return 'little memory';
  try { if (+localStorage.getItem(KEY) > Date.now() - WEEK) return 'slow last time'; } catch {}
  if (!fastGl()) return 'no fast WebGL2';
  return '';
}
function mounted(s, me) {
  if (me != mounting) { s.dispose(); return; }
  scene = s;
  if (dev) s.device(dev);
  if (S.back) s.turn(true);
  show();
  vis();
}
function unmount() {
  scene?.dispose();
  scene = null;
  parts(false);
  host.className = 'scene';
  host.replaceChildren();
}
async function flat(reason) {
  const me = ++mounting;
  unmount();
  host.replaceChildren($('#t-flat').content.cloneNode(true));
  stage.dataset.scene = 'flat';
  stage.dataset.why = reason;
  try {
    mounted(await (await import('./flat.js')).start(ui), me);
  } catch (e) {
    console.warn('desk: the scene did not load', e);
    if (me != mounting) return;
    host.className = 'scene none';
    host.replaceChildren(Object.assign(document.createElement('p'), { textContent: 'The desk did not load. The app beside it still works.' }));
    stage.dataset.scene = 'none';
  }
}
async function go() {
  const reason = why();
  if (reason) return flat(reason);
  const me = ++mounting;
  unmount();
  host.classList.add('hd');                                   // the still shows at once (asked for first, then a frame painted)
  new Image().src = new URL(`hd/poster-${dark ? 'night' : 'day'}.webp`, import.meta.url).href;
  stage.dataset.scene = 'hd-loading';
  delete stage.dataset.why;
  await new Promise((f) => requestAnimationFrame(() => setTimeout(f, 0)));
  if (me != mounting) return;
  try {
    const { start } = await import('./hd/scene.js');
    if (me != mounting) return;
    mounted(await start({ ...ui,
      onFirst: () => { if (me == mounting) stage.dataset.scene = 'hd'; },
      onHealthy: () => { try { localStorage.removeItem(KEY); } catch {} },
      onSlow: (ms) => {
        if (me != mounting) return;
        if (ms >= 0) {
          if (q.get('scene') == 'hd') return;
          try { localStorage.setItem(KEY, Date.now()); } catch {}
        }
        flat(ms < 0 ? 'context lost' : 'slow frames');
      } }), me);
  } catch (e) {
    console.warn('desk: the 3D scene did not start', e);
    if (me == mounting) flat('error');
  }
}
// crossing a phone's width: the choice is made again (the flat scene's sizes are chosen at mount)
NARROW.addEventListener('change', () => { if (!q.get('scene')) go(); });
show();
window.desk = { S, act, get scene() { return scene; }, get dev() { return dev; }, flat };
// after first paint: the page, the still and the app's frame come first
requestAnimationFrame(() => setTimeout(go, 0));
