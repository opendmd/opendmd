// The flat desk scene: what phones, reduced motion and machines without a fast WebGL2 get, and
// what the 3D one hands over to. The den is three layers of pixel art (room/*.webp, drawn by
// tools/desk-art.py); the display is a CSS box whose back is an SVG in millimetres
// (its back's drawing: flat-back.js, fetched when the page is idle); the room's light is one small canvas multiplied over the art: ambient, the lamp, the tube and
// the panel's own picture. The page (desk.js) owns the state; this draws it (start(ui) -> scene).
const clamp = (v, a, b) => Math.max(a, Math.min(b, v));
const mk = (w, h) => Object.assign(document.createElement('canvas'), { width: w, height: h });
const ease = (t) => (t < 0.5 ? 4 * t * t * t : 1 - (-2 * t + 2) ** 3 / 2);
const TURN = 1150, CLOSE = 380 + 700;
// the parts' boxes on the back (mm, the drawing's 518 x 134)
const BOX = { panels: [300, 20, 200, 36], carrier: [8, 22, 42, 90], leads: [54, 113, 78, 12], ribbon: [236, 50, 46, 34], devkit: [15, 44, 28, 50], sd: [19, 8, 20, 32], usb: [22, 96, 14, 38] };

const INK = '#3a2a32', MUST = '#f0b33c';

export async function start(ui) {
  const { S, still, act, host, stage } = ui;
  const $ = (s) => host.querySelector(s);
  const scene = host, rig = $('#rig'), set = (k, v) => stage.style.setProperty(k, v);
  const phone = matchMedia('(max-width:820px)').matches, dpr = devicePixelRatio || 1;
  const ac = new AbortController(), sig = { signal: ac.signal };
  // --- sizes. k: device pixels per LED (whole, so the dots land on the screen's); an art pixel is
  // half of that rounded up, also whole. Chosen once; the layout follows the scene's box.
  let k = clamp(Math.floor(Math.min(stage.clientWidth * (phone ? 0.86 : 0.66) / 128, phone ? 99 : (innerHeight - 170) * 0.3 / 32) * dpr), 3, 12);
  if (!phone && k % 2 && k > 3) k += (k + 1) * 128 / dpr <= stage.clientWidth * 0.78 && scene.clientHeight / ((k + 1) / 2 / dpr) >= 250 && k < 12 ? 1 : -1;
  const u = k / dpr, a = Math.ceil(k / 2) / dpr;
  stage.classList.toggle('fine', k < 6);
  set('--u', u + 'px'); set('--a', a + 'px');
  const G = { w: 0, h: 0, cx: 0, oy: 0, mx: 0, my: 0 };  // the scene's box, and the art's origin (320, 400) in it
  const snap = (v) => Math.round(v * dpr) / dpr;
  const lm = $('#lm'), lx = lm.getContext('2d'), halo = $('#halo'), hx = halo.getContext('2d');
  let dirty = 1;
  function layout() {
    if (phone) set('--strip', snap(S.folded ? 46 * u : Math.min(172 * a, innerHeight * 0.34)) + 'px');
    const w = scene.clientWidth, h = phone ? parseFloat(stage.style.getPropertyValue('--strip')) : scene.clientHeight;
    const drop = phone ? (S.folded ? 74 - 6 * u / a : 28) : clamp(322 - h / a, 0, 28);  // art rows cut off the foot
    Object.assign(G, { w, h, cx: snap(w / 2), oy: snap(h + drop * a) });
    set('--cx', G.cx + 'px'); set('--oy', G.oy + 'px');
    // the close-up of the back: the whole of it on a desk, its working end on a phone
    const zs = phone ? w / (90 * u) : Math.min(1.75, w * 0.95 / (129.5 * u));
    const midY = G.oy - 74 * a - 19 * u;
    set('--zs', zs.toFixed(3)); set('--zx', (phone ? 22 * u * zs : 0) + 'px'); set('--zy', (h * (phone ? 0.52 : 0.5) - midY) + 'px');
    G.mx = Math.round(w * 0.25); G.my = Math.round(h * 0.4);  // the light map overhangs the scene: the close-up moves it
    set('--mx', G.mx + 'px'); set('--my', G.my + 'px');
    lm.width = Math.ceil((w + 2 * G.mx) / 8); lm.height = Math.ceil((h + 2 * G.my) / 8);
    dirty = 1;
    if (settled) placeParts();
    kick();
  }

  const rearEl = $('#rear'), kitEl = $('#kit');
  // the back's drawing: when the page is idle, or at the first turn, whichever comes first
  let drawn = null;
  const drawBack = () => (drawn ??= import('./flat-back.js').then((m) => {
    rearEl.innerHTML = m.rear();
    kitEl.insertAdjacentHTML('afterbegin', m.kit());
    if (ui.S.hot) hot(ui.S.hot);
  }));
  (window.requestIdleCallback || setTimeout)(() => drawBack(), { timeout: 3000 });
  kitEl.innerHTML = `<i class="pwr" style="left:${36.6 / 5.18}%;top:${82.6 / 1.34}%"></i>`
    + Object.entries(BOX).map(([id, [x, y, w, h]]) => `<i class="pbox" data-p="${id}" style="left:${x / 5.18}%;top:${y / 1.34}%;width:${w / 5.18}%;height:${h / 1.34}%"></i>`).join('');
  // the parts' links (the page's) over their boxes, once the close-up has settled
  let settled = false, settleT = 0;
  function placeParts() {
    const v = ui.view.getBoundingClientRect();
    for (const e of kitEl.querySelectorAll('.pbox')) { const b = e.getBoundingClientRect(); ui.box(e.dataset.p, b.left - v.left, b.top - v.top, b.width, b.height); }
    ui.place();
  }
  function hot(id) {
    rig.classList.toggle('dimmed', !!id);
    for (const e of rig.querySelectorAll('.p')) e.classList.toggle('on', e.dataset.p === id && (id != 'panels' || e.classList.contains('m2')));
  }
  // the charger in the strip and its lead (art pixels); the card, loose on the desk
  const cab = (d, cls) => `<path class="${cls}" d="${d}" fill="none" stroke="${INK}" stroke-width="2.6" stroke-linecap="round"/><path class="${cls}" d="${d}" fill="none" stroke="#6a6577" stroke-width="1.2" stroke-linecap="round"/>`;
  $('#cables').innerHTML = `<svg viewBox="180 300 300 80">` + cab('M438 314C444 334 409 326 409 345', 'c-front') + cab('M207 316C203 338 236 339 300 339S409 332 409 345', 'c-back')
    + `<g class="plug" id="plug"><rect x="402.5" y="343.5" width="13" height="15" fill="#fbf3dc" stroke="${INK}"/><rect x="403" y="344" width="12" height="1" fill="#fff"/><rect x="403" y="355" width="12" height="3" fill="#dcd6c8"/><rect x="407" y="347" width="4" height="2" fill="${INK}"/><rect x="398" y="338" width="22" height="26" fill="transparent"/></g></svg>`;
  $('#loose').innerHTML = `<svg viewBox="0 0 14 12"><g transform="rotate(-24 7 6)"><path d="M3.5 0h5l2 2v10h-7z" fill="#191a1e" stroke="${INK}" stroke-width=".8"/><rect x="4.6" y="3" width="4.8" height="3.4" fill="${MUST}"/></g></svg>`;
  { // the dither: an 8x8 ordered matrix, one cell an art pixel
    const B = [0, 32, 8, 40, 2, 34, 10, 42, 48, 16, 56, 24, 50, 18, 58, 26, 12, 44, 4, 36, 14, 46, 6, 38, 60, 28, 52, 20, 62, 30, 54, 22, 3, 35, 11, 43, 1, 33, 9, 41, 51, 19, 59, 27, 49, 17, 57, 25, 15, 47, 7, 39, 13, 45, 5, 37, 63, 31, 55, 23, 61, 29, 53, 21];
    const t = mk(8, 8), x = t.getContext('2d'), d = x.createImageData(8, 8);
    B.forEach((v, i) => d.data.set([14, 10, 30, v * 0.52], i * 4));
    x.putImageData(d, 0, 0);
    $('#grain').style.backgroundImage = `url(${t.toDataURL()})`;
  }
  $('#dial').onclick = $('#lamp').onclick = act.cycle;
  $('#plug').onclick = act.power;
  $('#loose').onclick = act.card;

  // --- the state, drawn
  let turnT = -1e9, face = 1, closeT = 0;
  const panel = $('#panel');
  panel.width = 128 * k; panel.height = 32 * k;
  const pctx = panel.getContext('2d');
  function show() {
    const W = S.W, L = S.light / 100;
    set('--L', L.toFixed(3));
    set('--lamp', clamp((L - 0.03) * 4, 0, 1).toFixed(2));
    dirty = 1;
    if (W) {
      const pb = W.b ? (W.b / 96) ** 0.45 : 0;
      set('--pb', pb.toFixed(3));
      panel.style.filter = pb > 1 ? `brightness(${pb.toFixed(3)})` : '';
      if (!W.power) pctx.clearRect(0, 0, panel.width, panel.height);
    }
    kick();
  }
  function turn(back) {
    if (back) drawBack();
    turnT = performance.now();
    clearTimeout(closeT); clearTimeout(settleT);
    settled = false;
    ui.parts(false);
    if (back) {
      closeT = setTimeout(() => stage.classList.add('close'), still ? 0 : 380);
      settleT = setTimeout(() => { if (S.back) { settled = true; ui.parts(true); placeParts(); } }, still ? 30 : Math.max(TURN, CLOSE) + 60);
    } else stage.classList.remove('close');
    kick();
  }

  // --- the pointer moves the eye a little: the wall slides one way, the near things the other, and
  // the display shows a side (its LED face never moves). Off for reduced motion, and in the close-up.
  const eye = { x: 0, y: 0, tx: 0, ty: 0 };
  if (!still && !phone) {
    stage.addEventListener('pointermove', (e) => { if (e.pointerType == 'touch') return; const b = scene.getBoundingClientRect(); eye.tx = clamp((e.clientX - b.left) / b.width * 2 - 1, -1, 1); eye.ty = clamp((e.clientY - b.top) / b.height * 2 - 1, -1, 1); kick(); }, sig);
    stage.addEventListener('pointerleave', () => { eye.tx = eye.ty = 0; kick(); }, sig);
  }
  function parallax() {
    const tx = S.back ? 0 : eye.tx, ty = S.back ? 0 : eye.ty;
    if (Math.abs(tx - eye.x) + Math.abs(ty - eye.y) < 0.002) return false;
    eye.x += (tx - eye.x) * 0.1; eye.y += (ty - eye.y) * 0.1;
    set('--fx', snap(eye.x * 7) + 'px'); set('--fy', snap(eye.y * 3) + 'px');
    set('--nx', snap(eye.x * -6) + 'px'); set('--ny', snap(eye.y * -2) + 'px');
    set('--ox', (eye.x * 44 * u).toFixed(1) + 'px');
    return true;
  }

  // --- light. The panel's picture is halved down to a few pixels (each step an exact box filter),
  // then stretched: over the wall and the desk in the light map, and as bloom and a sheen on the desk.
  const mips = [];
  for (let w = 64 * k; w >= 12; w = Math.floor(w / 2)) mips.push(mk(w, Math.max(1, Math.round(w / 4))));
  const last = mips[mips.length - 1], M = 3, glow = mk(last.width + 2 * M, last.height + 2 * M), gx = glow.getContext('2d');  // a clear margin: its edge fades out when stretched
  const blob = (R, Gc, B) => { const t = mk(64, 64), x = t.getContext('2d'), gr = x.createRadialGradient(32, 32, 0, 32, 32, 32); gr.addColorStop(0, `rgba(${R},${Gc},${B},1)`); gr.addColorStop(.45, `rgba(${R},${Gc},${B},.5)`); gr.addColorStop(1, `rgba(${R},${Gc},${B},0)`); x.fillStyle = gr; x.fillRect(0, 0, 64, 64); return t; };
  const WARM = blob(255, 196, 120), TEAL = blob(70, 200, 190), REDL = blob(255, 70, 50);
  const fade = hx.createLinearGradient(0, 25, 0, 38); fade.addColorStop(0, '#000'); fade.addColorStop(.35, 'rgba(0,0,0,.5)'); fade.addColorStop(1, 'transparent');
  // stretch the glow so the picture itself covers (x, y, w, h)
  function spread(ctx, x, y, w, h, al, flip) {
    if (al <= 0.004) return;
    const sx = w / last.width, sy = h / last.height;
    if (flip) { ctx.save(); ctx.translate(0, 2 * y + h); ctx.scale(1, -1); }
    for (; al > 0.004; al -= 1) { ctx.globalAlpha = Math.min(1, al); ctx.drawImage(glow, x - M * sx, y - M * sy, glow.width * sx, glow.height * sy); }  // over 1: laid again (gain)
    if (flip) ctx.restore();
  }
  function light() {
    const W = S.W, L = S.light / 100, dk = 1 - L, s = lm.width / (G.w + 2 * G.mx), sy = lm.height / (G.h + 2 * G.my);
    const X = (ax) => (G.mx + G.cx + (ax - 320) * a) * s, Y = (ay) => (G.my + G.oy + (ay - 400) * a) * sy;
    const el = (img, cx, cy, rx, ry, al) => { if (al > 0.004) { lx.globalAlpha = Math.min(1, al); lx.drawImage(img, X(cx - rx), Y(cy - ry), 2 * rx * a * s, 2 * ry * a * sy); } };
    // the panel's light: how hard it shines, and which way it faces
    const on = W && W.power ? Math.min(1.15, +stage.style.getPropertyValue('--pb')) : 0;
    const ff = Math.max(face, 0), fb = Math.max(-face, 0);
    if (on) {
      let src = W.canvas;
      for (const m of mips) { const x = m.getContext('2d'); x.clearRect(0, 0, m.width, m.height); x.drawImage(src, 0, 0, m.width, m.height); src = m; }
      gx.globalAlpha = 0.55; gx.drawImage(last, M, M);  // eased in time, so a cut does not flash the room
    } else gx.clearRect(0, 0, glow.width, glow.height);
    const t = L ** 1.15, amb = [16 + 222 * t, 18 + 216 * t, 40 + 188 * t].map(Math.round);
    lx.globalCompositeOperation = 'source-over'; lx.globalAlpha = 1;
    lx.fillStyle = `rgb(${amb})`; lx.fillRect(0, 0, lm.width, lm.height);
    lx.globalCompositeOperation = 'lighter';
    const lamp = clamp((L - 0.03) * 4, 0, 1);
    el(WARM, 196, 322, 96, 34, lamp * (0.2 + 0.62 * dk)); el(WARM, 174, 280, 60, 50, lamp * (0.16 + 0.5 * dk));
    el(TEAL, 486, 274, 54, 46, dk * 0.5); el(TEAL, 490, 334, 66, 24, dk * 0.42); el(REDL, 458, 360, 12, 9, dk * 0.5);
    const px = (G.mx + G.cx - 64 * u) * s, py = (G.my + G.oy - 74 * a - 35 * u) * sy, us = u * s, uy = u * sy;
    spread(lx, px - 34 * us, py - 36 * uy, 196 * us, 96 * uy, on * (1.5 * ff + 2.6 * fb));  // the wall, wide
    spread(lx, px - 9 * us, py - 12 * uy, 146 * us, 58 * uy, on * (1.3 * ff + 2 * fb));    // and close
    spread(lx, px - 16 * us, py + 34 * uy, 160 * us, 34 * uy, on * 2.6 * ff, 1);              // the desk in front
    // over the room: bloom round the panel, a sheen on the desk (2 u a pixel; the panel is at 12, 8)
    hx.globalCompositeOperation = 'source-over'; hx.clearRect(0, 0, 88, 38);
    spread(hx, 9, 26.5, 70, 11, on * (0.5 + 1.1 * dk) * ff, 1);
    hx.save(); hx.beginPath(); hx.rect(0, 25, 88, 13); hx.clip(); hx.globalCompositeOperation = 'destination-in'; hx.globalAlpha = 1; hx.fillStyle = fade; hx.fillRect(0, 25, 88, 13); hx.restore();
    spread(hx, 7, 4, 74, 24, on * (0.5 + 1.5 * dk) * (ff + fb * 1.3));
  }

  // --- the loop: it runs while there is something to draw (the panel on, the box turning, the eye
  // drifting, a change), and stops by itself otherwise; the page stops it out of sight
  const stats = { n: 0, js: 0, max: 0 };
  let raf = 0, n = 0, running = true, dev = null;
  function kick() { if (running && !raf) raf = requestAnimationFrame(draw); }
  function draw(now) {
    raf = 0;
    const t0 = performance.now(), W = S.W;
    let busy = parallax();
    const f = still ? 1 : clamp((now - turnT) / TURN, 0, 1), fc = Math.cos(Math.PI * (S.back ? ease(f) : 1 - ease(f)));
    if (fc != face) { face = fc; dirty = 1; busy = true; }
    const on = W && W.power;
    if (on && face > -0.999) {
      const cv = W.canvas;
      if (cv.width === panel.width) pctx.drawImage(cv, 0, 0); else pctx.drawImage(cv, 0, 0, panel.width, panel.height);
    }
    if ((on && !(n++ & 1)) || dirty) { dirty = 0; light(); }
    const dt = performance.now() - t0;
    stats.n++; stats.js += dt; if (dt > stats.max) stats.max = dt;
    if (on || busy || dirty) kick();
  }
  function run(v) { running = v; if (v) kick(); else { cancelAnimationFrame(raf); raf = 0; } }
  layout();
  return {
    kind: 'flat', turnMs: TURN, stats, k,
    show, turn, hot, run, layout,
    device(d) { if (d === dev) return; dev = d; d.scale(k); show(); },
    dispose() { run(false); ac.abort(); clearTimeout(closeT); clearTimeout(settleT); stage.classList.remove('close', 'fine'); },
  };
}
