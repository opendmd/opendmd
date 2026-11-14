// The 3D desk scene, in the manner of "HD-2D" games (the style only): a low-poly den with
// pixel-art textures and sprites (2 mm a texel everywhere), real lights (daylight, the lamp, the
// tube, and the panel's own picture), bloom, depth of field and a vignette. Units are centimetres;
// the desk top is y 0, the display stands at z 0 facing +z, the wall is behind it. Every shape and
// texture is drawn here or in tools/desk-art-hd.py: no game art, names, logos or marks.
// The page (../desk.js) owns the state and the buttons; this draws it: start(ui) -> scene. The
// panel is the app's simulated player (window.opendmdDemo in the frame): its LEDs, one pixel each
// (`frame()`), on the LED face and in the four lights the face throws on the room.
import * as THREE from './three.js';
import { EffectComposer, Pass, FullScreenQuad, UnrealBloomPass } from './three.js';
import ATLAS, { ATLAS_W, ATLAS_H } from './atlas.js';
import { rearSvg, carrierSvg, devkitSvg, ribbonSvg, raster, MUST, TIN } from './back.js';
import { PARTS } from '../parts.js';

const T = 0.2;                 // one texel, cm (2 mm: half an LED pitch)
const ZW = -13, ZF = 34;       // the wall; the desk's front edge
const RISE = 7;                // the wall's things hang this much higher than in the flat drawing (the eye is above)
const LIFT = 0.9;              // the display's body stands this far off the desk, on its feet
const FOV = 26;
const P = { wall: '#f6ecd2', dot: '#e6d6b0', floor: '#c9b08a', ol: '#3a2a32', wood: '#d59a55', wood_l: '#edc07a', wood_d: '#a0683a', dark: '#2c2a3a', label: '#fbf3dc',
  scr: '#1e3247', lit: '#ffd45a', hot: '#ee5d4e', cool: '#3cb6b0', grey: '#dcd6c8', grey2: '#a9a39a', teal: '#2f9c95', red: '#d9483f', mustard: '#f0b33c', plum: '#7d4f8f', blue: '#3f6fb5' };
const clamp = (v, a, b) => Math.max(a, Math.min(b, v));
const lerp = (a, b, t) => a + (b - a) * t;
const ease = (t) => (t < 0.5 ? 4 * t * t * t : 1 - (-2 * t + 2) ** 3 / 2);
const mk = (w, h) => Object.assign(document.createElement('canvas'), { width: w, height: h });
const V = (x, y, z) => new THREE.Vector3(x, y, z);

// Texels stay square and sharp at any size, without shimmer: sample between texel centres only
// across one screen pixel at each texel edge.
const AA = `#ifdef USE_MAP
  vec2 ts_ = vec2( textureSize( map, 0 ) ); vec2 tx_ = vMapUv * ts_; vec2 sm_ = floor( tx_ + 0.5 );
  vec2 uv_ = ( sm_ + clamp( ( tx_ - sm_ ) / max( fwidth( tx_ ), 1e-5 ), -0.5, 0.5 ) ) / ts_;
  vec4 sampledDiffuseColor = textureGrad( map, uv_, dFdx( vMapUv ), dFdy( vMapUv ) );
  diffuseColor *= sampledDiffuseColor;
#endif`;
const aaPatch = (sh) => { sh.fragmentShader = sh.fragmentShader.replace('#include <map_fragment>', AA); };
const aa = (m) => { m.onBeforeCompile = aaPatch; return m; };

export async function start(ui) {
  const { S: P0, q, still, act, host, stage, onSlow, onFirst } = ui;
  const ac = new AbortController(), sig = { signal: ac.signal };
  const canvas = Object.assign(document.createElement('canvas'), { className: 'gl' });
  canvas.setAttribute('aria-hidden', 'true');
  host.prepend(canvas);
  const renderer = new THREE.WebGLRenderer({ canvas, antialias: false, alpha: false, stencil: false, depth: false, powerPreference: 'high-performance' });
  renderer.shadowMap.enabled = true;
  renderer.shadowMap.autoUpdate = false;
  renderer.info.autoReset = false;
  renderer.setClearColor(0x05060c);
  let disposed = false;
  // the context goes with the scene (after its lost event, so a listener sees it), however it ends
  const drop = () => { ac.abort(); renderer.dispose(); const gone = () => canvas.remove(); canvas.addEventListener('webglcontextlost', gone, { once: true }); setTimeout(gone, 500); renderer.forceContextLoss(); };
  try {
  const gpu = (() => { const gl = renderer.getContext(), e = gl.getExtension('WEBGL_debug_renderer_info'); return e ? gl.getParameter(e.UNMASKED_RENDERER_WEBGL) : '?'; })();
  const soft = /llvmpipe|swiftshader|software|softpipe/i.test(gpu);
  const maxAniso = soft ? 1 : Math.min(4, renderer.capabilities.getMaxAnisotropy());   // (Mesa's llvmpipe returns NaN texels with anisotropy on)
  const scene = new THREE.Scene();
  const camera = new THREE.PerspectiveCamera(FOV, 1.4, 20, 900);

  // ---------------------------------------------------------------- textures
  const pix = (t, rep) => { t.colorSpace = THREE.SRGBColorSpace; t.magFilter = THREE.LinearFilter; t.minFilter = THREE.LinearMipmapLinearFilter; t.generateMipmaps = true; t.anisotropy = maxAniso;
    if (rep) { t.wrapS = t.wrapT = THREE.RepeatWrapping; t.repeat.set(...rep); } return t; };
  const atlas = pix(await new THREE.TextureLoader().loadAsync(new URL('atlas.webp', import.meta.url).href));
  const tile = (w, h, draw, rep) => { const cv = mk(w, h), x = cv.getContext('2d'); draw(x, (c, X, Y, W = 1, H = 1) => { x.fillStyle = c; x.fillRect(X, Y, W, H); }); return pix(new THREE.CanvasTexture(cv), rep); };
  let seed = 7; const rnd = (n) => (seed = (seed * 16807) % 2147483647) % n;
  const wallTex = (rep) => tile(8, 8, (x, r) => { r(P.wall, 0, 0, 8, 8); r(P.dot, 3, 3); }, rep);
  const woodTex = (rep, base, lo, hi, gr) => tile(256, 112, (x, r) => {  // boards along x: joints, butts, grain
    r(base, 0, 0, 256, 112);
    for (let b = 0; b < 4; b++) { r(lo, 0, b * 28, 256, 1); r(hi, 0, b * 28 + 1, 256, 1); for (let X = rnd(90); X < 256; X += 90 + rnd(80)) r(lo, X, b * 28 + 2, 1, 26); }
    for (let i = 0; i < 70; i++) r(gr, rnd(244), 3 + rnd(108), 4 + rnd(8), 1);
  }, rep);

  // ---------------------------------------------------------------- materials, builders
  const lam = (color, o = {}) => new THREE.MeshLambertMaterial({ color, ...o });
  const cut = (o = {}) => aa(lam('#fff', { map: atlas, alphaTest: 0.5, side: THREE.DoubleSide, ...o }));
  const SPR = cut();                                           // the desk's sprites
  const wallMats = [];                                         // everything on the wall dissolves when the eye goes behind it
  const wm = (m) => { m.alphaHash = true; wallMats.push(m); return m; };
  const SPRW = wm(cut());
  const mats = {};                                             // plain colours, shared
  const col = (c) => mats[c] ??= lam(c);
  const bills = [];                                            // sprites that stand up and face the eye
  const mesh = (g, m, cast = true, recv = true) => { const o = new THREE.Mesh(g, m); o.castShadow = cast; o.receiveShadow = recv; return o; };
  const at = (o, x, y, z, parent = scene) => { o.position.set(x, y, z); parent.add(o); return o; };
  const box = (w, h, d, m, cast, recv) => mesh(new THREE.BoxGeometry(w, h, d), typeof m == 'string' ? col(m) : m, cast, recv);
  const uvs = (g, [x, y, w, h]) => { const u0 = x / ATLAS_W, u1 = (x + w) / ATLAS_W, v1 = 1 - y / ATLAS_H, v0 = 1 - (y + h) / ATLAS_H, a = g.attributes.uv; a.setXY(0, u0, v1); a.setXY(1, u1, v1); a.setXY(2, u0, v0); a.setXY(3, u1, v0); return g; };
  const quad = (name, m = SPR, foot = false) => { const s = ATLAS[name], g = uvs(new THREE.PlaneGeometry(s[2] * T, s[3] * T), s); if (foot) g.translate(0, s[3] * T / 2, 0); return mesh(g, m); };
  const ax = (name, shift = 0) => (ATLAS[name][4] + ATLAS[name][2] / 2 - 320) * T + shift;      // where v2's drawing had it, across
  const wy = (row) => (300 - row) * T + RISE;                                                    // and up the wall
  // a sprite standing on (x, y, z), turning to the eye
  const stand = (name, x, y, z, parent = scene, m = SPR) => { const o = at(quad(name, m, true), x, y, z, parent); bills.push(o); return o; };
  // a thing lying on the desk: a low box with its picture on top
  const flat = (name, x, z, h, side, rot = 0) => { const s = ATLAS[name], g = new THREE.Group(); at(box(s[2] * T - .2, h, s[3] * T - .2, side), 0, h / 2, 0, g);
    const t = at(quad(name), 0, h + .02, 0, g); t.rotation.x = -Math.PI / 2; t.castShadow = false; g.rotation.y = rot; return at(g, x, 0, z); };
  const rod = (a, b, th, m, parent = scene) => { const d = b.clone().sub(a), o = box(th, d.length(), th, m); o.position.copy(a).addScaledVector(d, .5); o.quaternion.setFromUnitVectors(V(0, 1, 0), d.normalize()); parent.add(o); return o; };
  const shade = (w, d, x, y, z, o = .4, parent = scene) => {  // a soft contact shade lying on the desk
    shade.t ??= (() => { const cv = mk(64, 64), c = cv.getContext('2d'), g = c.createRadialGradient(32, 32, 0, 32, 32, 32); g.addColorStop(0, 'rgba(26,12,20,1)'); g.addColorStop(.55, 'rgba(26,12,20,.55)'); g.addColorStop(1, 'rgba(26,12,20,0)'); c.fillStyle = g; c.fillRect(0, 0, 64, 64); return new THREE.CanvasTexture(cv); })();
    const m = mesh(new THREE.PlaneGeometry(w, d), new THREE.MeshBasicMaterial({ map: shade.t, transparent: true, opacity: o, depthWrite: false }), false, false); m.rotation.x = -Math.PI / 2; m.renderOrder = 1; return at(m, x, y, z, parent); };

  // ---------------------------------------------------------------- the room
  const wallG = new THREE.Group(); scene.add(wallG);
  at(mesh(new THREE.PlaneGeometry(420, 170), wm(aa(lam('#fff', { map: wallTex([420 / 1.6, 170 / 1.6]) }))), false), 0, 55, ZW, wallG);
  { // where the desk meets the wall the corner is shaded (three dithered steps, as in the flat drawing)
    const t = tile(2, 18, (x, r) => { x.clearRect(0, 0, 2, 18); for (let y = 0; y < 18; y++) for (let X = 0; X < 2; X++) { const k = y < 4 ? ((X + y) % 2 ? .0 : .16) : y < 10 ? .16 : y < 13 ? ((X + y) % 2 ? .16 : .34) : .34; r(`rgba(120,90,50,${k})`, X, y); } }, [210, 1]);
    t.wrapT = THREE.ClampToEdgeWrapping;
    at(mesh(new THREE.PlaneGeometry(420, 3.6), wm(aa(new THREE.MeshBasicMaterial({ map: t, transparent: true, depthWrite: false }))), false, false), 0, 1.8, ZW + .05, wallG);
  }
  const WOOD = aa(lam('#fff', { map: woodTex([190 / 51.2, (ZF - ZW) / 22.4], P.wood, P.wood_d, P.wood_l, '#c58c4b') }));
  at(mesh(new THREE.BoxGeometry(190, 3, ZF - ZW), [col(P.wood_d), col(P.wood_d), WOOD, col(P.wood_d), col('#b97f45'), col(P.wood_d)], true, true), 0, -1.5, (ZF + ZW) / 2);
  at(box(190, .5, .5, P.wood_l, false), 0, -.25, ZF + .05);
  at(box(184, 10, 1.6, P.wood_d), 0, -8, ZF - 3);
  for (const x of [-91, 91]) at(box(3, 70, ZF - ZW - 4, P.wood_d), x, -37, (ZF + ZW) / 2);
  shade(190, 9, 0, .03, ZW + 3.5, .3);
  // the rest of the room, seen (blurred) when the eye goes round: a floor, the far wall (no rug: from the back it showed
  // under the desk as a bright shape in front of everything)
  const fl = mesh(new THREE.PlaneGeometry(700, 500), aa(lam('#fff', { map: woodTex([700 / 51.2, 500 / 22.4], P.floor, '#a68f6c', '#d9c4a0', '#bba27d') })), false); fl.rotation.x = -Math.PI / 2; at(fl, 0, -72, 150);
  const far = mesh(new THREE.PlaneGeometry(700, 260), aa(lam('#fff', { map: wallTex([700 / 1.6, 260 / 1.6]) })), false); far.rotation.y = Math.PI; at(far, 0, 58, 330);

  // on the wall
  for (const n of ['p_sun', 'p_space', 'p_shapes', 'clock', 'board', 'dial']) { const s = ATLAS[n]; at(quad(n, SPRW), ax(n), wy(s[5] + s[3] / 2), ZW + (n[0] == 'p' ? .25 : .5), wallG); }
  const knob = mesh(new THREE.CylinderGeometry(.8, .8, .5, 8), wm(lam(P.grey)), false); knob.rotation.x = Math.PI / 2; at(knob, ax('dial'), wy(214), ZW + .8, wallG);
  at(box(.2, .06, .7, wm(lam(P.ol)), false), 0, .28, -.4, knob);
  const WOODW = wm(lam(P.wood)), WOODD = wm(lam(P.wood_d));
  const plank = (x0, x1, row) => { const w = (x1 - x0) * T, x = ((x0 + x1) / 2 - 320) * T, y = wy(row); at(box(w, .8, 5.4, WOODW), x, y - .4, ZW + 2.7, wallG); for (const s of [-1, 1]) at(box(.6, 2, 4, WOODD), x + s * (w / 2 - 1.6), y - 1.6, ZW + 2, wallG); return y; };
  { const y = plank(196, 444, 224); let i = 0; for (const n of ['s_carts', 's_cass', 's_boom', 's_hand', 's_fbox', 's_cactus', 's_stick']) stand(n, ax(n), y, ZW + 2.2 + [1, .2, 0, 1.6, .6, 2, 1.2][i++], wallG, SPRW); }
  [['l1', 14, 98, 120], ['l2', 14, 98, 176], ['l3', 14, 98, 232], ['r1', 548, 630, 120], ['r2', 548, 630, 176], ['r3', 548, 630, 232]].forEach(([n, x0, x1, row]) => stand(n, ax(n), plank(x0, x1, row), ZW + 2.8, wallG, SPRW));

  // on the desk, at its back
  stand('slab', ax('slab'), 0, -7); shade(11, 4, ax('slab'), .04, -6.5);
  stand('plant', ax('plant'), 0, -8); shade(6, 3.4, ax('plant'), .04, -7.6);
  stand('pile', ax('pile', 3), 0, -6); stand('dome', ax('dome', 3), 0, -7.5); stand('cactus', ax('cactus', 3), 0, -8.5);
  shade(6, 3, ax('pile', 3), .04, -5.6); shade(8, 3.4, ax('dome', 3), .04, -7); shade(4, 2.6, ax('cactus', 3), .04, -8.2);
  // the lamp: a base, two arms, a shade, a bulb that is a real light
  const lampG = new THREE.Group(); scene.add(lampG);       // hidden while the eye is round the back: it would hang in front of it
  const LB = V(-35.5, 0, -5.5), LJ = V(-38, 12.5, -6.5), LH = V(-30.5, 23, -1.5), LT = V(-15, 0, 9), LD = LT.clone().sub(LH).normalize();
  { const b = mesh(new THREE.CylinderGeometry(2.4, 2.8, .9, 10), col(P.dark)); at(b, LB.x, .45, LB.z, lampG); shade(9, 7, LB.x, .04, LB.z + .3, .5); }
  rod(V(LB.x, .9, LB.z), LJ, .5, P.ol, lampG); rod(LJ, LH, .5, P.ol, lampG);
  for (const p of [LJ, V(LB.x, 1.2, LB.z)]) at(box(1, 1, 1, P.mustard), p.x, p.y, p.z, lampG);
  const shadeM = lam(P.mustard, { side: THREE.DoubleSide, flatShading: true, emissive: '#ffb648', emissiveIntensity: 0 });
  const lampShade = mesh(new THREE.CylinderGeometry(1.5, 5.4, 5.2, 8, 1, true), shadeM, false); lampShade.quaternion.setFromUnitVectors(V(0, -1, 0), LD); lampShade.position.copy(LH).addScaledVector(LD, 2.2); lampG.add(lampShade);
  const bulbM = new THREE.MeshBasicMaterial({ color: '#000' });
  const bulb = mesh(new THREE.SphereGeometry(1.2, 8, 6), bulbM, false, false); bulb.position.copy(LH).addScaledVector(LD, 3.4); lampG.add(bulb);
  const lamp = new THREE.SpotLight('#ffc98a', 0, 0, 1.02, .75, 2); lamp.position.copy(bulb.position); lamp.target.position.copy(LT); scene.add(lamp, lamp.target);
  lamp.castShadow = true; lamp.shadow.mapSize.set(1024, 1024); lamp.shadow.camera.near = 3; lamp.shadow.camera.far = 140; lamp.shadow.bias = -.0015; lamp.shadow.radius = 3;
  // the computer: a base, its tube on top (the screen glows), its keys in front
  const CX = 37.6;
  at(box(14.6, 2.8, 12, P.grey), CX, 1.4, -7.4); { const f = at(quad('base'), CX, 1.5, -1.38); f.castShadow = false; }
  at(box(12, 9.2, 10, P.grey), CX - .2, 7.4, -7.6); at(box(9, 7, 3, P.grey2), CX - .2, 7.4, -13 + 1.4); { const f = at(quad('tube'), CX - .2, 7.3, -2.58); f.castShadow = false; }
  shade(20, 6, CX, .04, -2, .45);
  const crtCv = mk(43, 36), crtX = crtCv.getContext('2d'), crtTex = new THREE.CanvasTexture(crtCv); crtTex.colorSpace = THREE.SRGBColorSpace; crtTex.magFilter = THREE.NearestFilter; crtTex.minFilter = THREE.LinearFilter; crtTex.generateMipmaps = false;
  const crtM = new THREE.MeshBasicMaterial({ map: crtTex });
  at(mesh(new THREE.PlaneGeometry(43 * T, 36 * T), crtM, false, false), CX - .2 + (4 + 21.5 - 31) * T, 7.3 + (24 - 4 - 18) * T, -2.55);
  const tube = at(new THREE.PointLight('#46c8be', 0, 60, 2), CX - 1.5, 7.5, 3.5);
  let crtOn = -1;
  const crtDraw = (on) => { if (on === crtOn) return; crtOn = on; const r = (c, x, y, w, h) => { crtX.fillStyle = c; crtX.fillRect(x, y, w, h); };
    r('#0c2b2c', 0, 0, 43, 36); r('#7df0c8', 3, 4, 16, 2); r('#ffd45a', 3, 9, 26, 2); r('#7df0c8', 3, 14, 21, 2); r('#ee5d4e', 3, 19, 11, 2); if (on) r('#7df0c8', 3, 25, 4, 5);
    for (let y = 1; y < 36; y += 2) r('rgba(0,0,0,.22)', 0, y, 43, 1); crtTex.needsUpdate = true; };
  flat('keys', CX + 1.2, 6.5, 1, P.grey2, -.05);
  // in front of the display
  stand('pad', ax('pad'), 0, 11); shade(4.4, 2.4, ax('pad'), .04, 11.2);
  stand('stick', ax('stick'), 0, 17); shade(4.6, 3, ax('stick'), .04, 17.2);
  flat('cas1', -11, 11.5, 1.1, P.plum, .12); flat('cas2', -7.2, 16.5, 1.1, P.dark, -.2);
  flat('flop1', 1.6, 11.6, .32, P.mustard, -.1); flat('flop2', 5, 15.2, .32, P.blue, .16);
  flat('note', ax('note', 3), 14, .06, P.label, .2);
  { const p = box(7, .34, .34, P.lit); p.rotation.y = .5; at(p, ax('note', 3) + 4.5, .2, 18); const e = box(.5, .36, .36, P.hot); e.rotation.y = .5; at(e, ax('note', 3) + 7.7, .2, 16.25); }
  { const g = new THREE.Group(), m = lam(P.red, { flatShading: true }); at(mesh(new THREE.CylinderGeometry(1.3, 1.2, 3.1, 8), m), 0, 1.55, 0, g); at(mesh(new THREE.CylinderGeometry(1.1, 1.1, .1, 8), col('#3a2418'), false), 0, 3.06, 0, g);
    at(box(.4, 1.6, 1.2, m), 1.6, 1.6, 0, g); at(box(2.3, .5, .05, P.label, false), 0, 2.1, 1.28, g); g.rotation.y = -.5; at(g, -35, 0, 13); shade(5, 4, -35, .04, 13.3, .5); }
  // the power strip, its switch, the charger and the lead round to the display's back
  const SX = 20.5, SZ = 16.5;
  flat('strip', SX, SZ, 2.2, P.grey);
  const swM = new THREE.MeshBasicMaterial({ color: '#ff5a4a' }); at(box(.9, .3, .9, swM, false, false), SX + 6.1, 2.3, SZ - .1);
  shade(17, 6, SX, .04, SZ + .4, .45);
  const PLX = SX - 2.9, PLZ = SZ - .1, plug = new THREE.Group(); at(plug, PLX, 2.2, PLZ);
  at(box(2.4, 2.6, 2.4, P.label), 0, 1.3, 0, plug); at(box(.8, .4, .5, P.ol, false), 0, 2.4, 1.0, plug);
  const cable = (pts, rad, m, segs = 48) => mesh(new THREE.TubeGeometry(new THREE.CatmullRomCurve3(pts, false, 'catmullrom', .35), segs, rad, 5), m);
  { const g = box(100, .5, .5, P.grey2); at(g, SX + 6.5 + 50, .3, SZ + 2.2); g.rotation.y = -.03; }                 // the strip's own lead, off to the right

  // ---------------------------------------------------------------- the display
  const rig = new THREE.Group(); at(rig, 0, LIFT, 0);
  const BW = 51.8, BH = 13.4, BD = 1.6;
  const pmats = {};                                           // per part: its materials, so one part can stay lit while the rest dim
  const pm = (part, c, o = {}) => { const m = (o.map ? aa : (x) => x)(lam(c, o)); (pmats[part] ??= []).push([m, m.color.clone(), m.emissive.clone()]); return m; };
  const pmb = (part, c) => { const m = new THREE.MeshBasicMaterial({ color: c }); (pmats[part] ??= []).push([m, m.color.clone(), null]); return m; };
  const pbox = {};                                            // per part: its box in the world, for the labels' hot spots
  const pb = (part, o) => { o.updateWorldMatrix(true, false); (pbox[part] ??= new THREE.Box3()).expandByObject(o); return o; };
  at(box(BW, BH, BD, pm('body', '#2a2c33')), 0, BH / 2, 0, rig);
  at(box(.16, BH + .04, BD + .04, pm('body', '#050506'), false), 0, BH / 2, 0, rig);
  for (const s of [-1, 1]) {
    at(box(3.2, .9, 7.2, pm('body', MUST)), s * 18.4, -.45, 0, rig); at(box(3.2, .3, .3, pm('body', MUST)), s * 18.4, .15, BD / 2 + .15, rig); at(box(3.2, 1.7, .3, pm('body', MUST)), s * 18.4, .85, -BD / 2 - .15, rig);
    at(box(3.24, .06, 1.7, pm('body', '#8a5f1c'), false), s * 18.4, 0, 0, rig);
  }
  shade(62, 13, 0, .035, .4, .62); shade(52, 5, 0, .036, .2, .5);
  // the LED face: the player's own 128 x 32 frame, one dot an LED
  const f128 = mk(128, 32), fx = f128.getContext('2d', { willReadFrequently: true });   // (on the CPU: it is uploaded every frame)
  const ledTex = new THREE.CanvasTexture(f128); ledTex.colorSpace = THREE.SRGBColorSpace; ledTex.magFilter = ledTex.minFilter = THREE.NearestFilter; ledTex.generateMipmaps = false;
  const led = new THREE.ShaderMaterial({ uniforms: { map: { value: ledTex }, on: { value: 0 }, gain: { value: 1.2 }, amb: { value: 1 } },
    vertexShader: 'varying vec2 vUv; void main(){ vUv = uv; gl_Position = projectionMatrix * modelViewMatrix * vec4( position, 1.0 ); }',
    fragmentShader: `uniform sampler2D map; uniform float on, gain, amb; varying vec2 vUv;
void main(){
  vec2 g = vUv * vec2( 128., 32. ), f = fract( g ) - .5;
  vec3 c = texture2D( map, ( floor( g ) + .5 ) / vec2( 128., 32. ) ).rgb;
  float fw = max( fwidth( g.x ), fwidth( g.y ) ), d = length( f );
  float far = smoothstep( .17, .45, fw );                                // too small to draw dots (under about 5 screen pixels an LED): the cell's mean instead
  float dotm = 1. - smoothstep( .36 - fw, .36 + fw, d );
  float m = mix( dotm + .22 * exp( -d * d * 7. ), .52, far );
  vec3 o = c * gain * on * m;
  o += amb * ( vec3( .006, .006, .008 ) + mix( dotm, .4, far ) * vec3( .022, .022, .027 ) );   // unlit: the face and its dots in the room's light
  o *= 1. - .7 * ( 1. - smoothstep( 0., fw + .04, abs( g.x - 64. ) ) );                       // the seam between the two panels
  gl_FragColor = vec4( o, 1. );
}` });
  at(mesh(new THREE.PlaneGeometry(51.2, 12.8), led, false, false), 0, BH / 2, BD / 2 + .01, rig);
  // the light it throws: four lamps in front of it, each the colour of its quarter of the picture
  const glow = [-19.2, -6.4, 6.4, 19.2].map((x) => at(new THREE.PointLight('#000', 0, 170, 1.7), x, BH / 2 + 5.5, 14, rig));
  const gcol = glow.map(() => [0, 0, 0]);

  // the back: millimetres from its top left, as seen from behind
  const BX = (mm) => BW / 2 - mm / 10, BY = (mm) => BH - mm / 10, BZ = -BD / 2;
  const bbox = (part, x, y, w, h, off, th, m, cast = false) => pb(part, at(box(w / 10, h / 10, th, m, cast, true), BX(x + w / 2), BY(y + h / 2), BZ - off - th / 2, rig));
  const bquad = (part, x, y, w, h, off, m) => { const o = mesh(new THREE.PlaneGeometry(w / 10, h / 10), m, false, true); o.rotation.y = Math.PI; return at(o, BX(x + w / 2), BY(y + h / 2), BZ - off, rig); };
  const PX = +(q.get('backpx') || 2);                         // texels a millimetre on the back
  const ctex = async (svg) => { const t = pix(new THREE.CanvasTexture(await raster(svg, PX))); return t; };
  const [rearT, carT, devT, ribT] = await Promise.all([ctex(rearSvg()), ctex(carrierSvg()), ctex(devkitSvg()), ctex(ribbonSvg())]);
  for (const [i, part] of [[0, 'body'], [1, 'panels']]) { const o = bquad(part, i * 259, 0, 259, 134, .01, pm(part, '#fff', { map: rearT })); const a = o.geometry.attributes.uv; for (let k = 0; k < 4; k++) a.setX(k, (i + a.getX(k)) / 2); }
  pbox.panels = new THREE.Box3(V(BX(500), LIFT + BY(56), BZ - .2), V(BX(300), LIFT + BY(20), BZ));
  for (const y of [9, 109]) bbox('body', 237, y, 44, 16, 0, .25, pm('body', MUST), true);
  // carrier, DevKit, card
  bbox('carrier', 8, 22, 42, 90, .5, .16, pm('carrier', '#1f4fae'), true); bquad('carrier', 8, 22, 42, 90, .67, pm('carrier', '#fff', { map: carT }));
  for (const [x, y] of [[12, 26], [46, 26], [46, 108]]) bbox('carrier', x - 1.6, y - 1.6, 3.2, 3.2, 0, .5, pm('carrier', '#8d8f96'));
  bbox('carrier', 12.6, 43, 3.4, 54, .66, .85, pm('carrier', '#0b0c0e')); bbox('carrier', 42, 43, 3.4, 54, .66, .85, pm('carrier', '#0b0c0e'));
  bbox('carrier', 9, 97, 14, 14, .66, .9, pm('carrier', '#2e9b5a'));
  { const cap = mesh(new THREE.CylinderGeometry(.46, .46, 1, 8), pm('carrier', '#23252b'), false); cap.rotation.x = Math.PI / 2; pb('carrier', at(cap, BX(43), BY(104.5), BZ - 1.16, rig)); }
  bbox('devkit', 15, 44, 28, 52, 1.51, .16, pm('devkit', '#17181c'), true); bquad('devkit', 15, 44, 28, 52, 1.68, pm('devkit', '#fff', { map: devT }));
  bbox('devkit', 19.5, 52.5, 19, 18, 1.67, .3, pm('devkit', '#c4c8ce')); bbox('devkit', 23.5, 88.5, 11, 7.5, 1.67, .32, pm('devkit', TIN));
  const pwr = pmb('devkit', '#ff4030'); bbox('devkit', 36, 82, 1.4, 1.4, 1.67, .08, pwr);
  bbox('sd', 19, 25, 20, 15, .66, .2, pm('sd', TIN));
  const card = new THREE.Group(); at(card, BX(29), BY(19), BZ - .9, rig);
  at(box(1.2, 1.6, .1, pm('sd', '#191a1e'), false), 0, 0, 0, card); at(box(.9, .6, .02, pm('sd', MUST), false), 0, .25, -.06, card);
  pbox.sd.expandByPoint(V(BX(23), LIFT + BY(9), BZ - 1)).expandByPoint(V(BX(35), LIFT + BY(40), BZ - .6));
  const loose = new THREE.Group(); at(loose, 30.2, .08, -4.6); loose.rotation.set(-Math.PI / 2, 0, .5); loose.visible = false;
  at(box(1.2, 1.6, .1, col('#191a1e')), 0, 0, 0, loose); at(box(.9, .6, .02, col(MUST), false), 0, .25, .06, loose);
  // the ribbon across the seam, between two blocks
  for (const x of [236, 266]) bbox('ribbon', x, 50, 16, 34, 0, .9, pm('ribbon', '#121317'), true);
  { const g = new THREE.PlaneGeometry(2.4, 2.6, 8, 1), p = g.attributes.position; for (let i = 0; i < p.count; i++) p.setZ(i, .22 * Math.cos(p.getX(i) / 1.2 * Math.PI / 2)); g.computeVertexNormals();
    const o = mesh(g, pm('ribbon', '#fff', { map: ribT, side: THREE.DoubleSide }), false); o.rotation.y = Math.PI; pb('ribbon', at(o, BX(259), BY(67), BZ - .75, rig)); }
  // the power leads: red and black, from the carrier's terminal to each panel's header
  const lead = (c, pts) => pb('leads', at(cable(pts.map(([x, y, o = .3]) => V(BX(x), BY(y), BZ - o)), .12, pm('leads', c), 64), 0, 0, 0, rig));
  lead('#d23a30', [[19, 108, 1.2], [19, 113, .5], [20, 117], [24, 118], [124, 118], [128, 117], [129, 113], [129, 76], [129, 72, .9]]);
  lead('#1d1e23', [[13, 108, 1.2], [13, 116, .5], [14, 121], [18, 122], [130, 122], [134, 121], [135, 117], [135, 76], [135, 72, .9]]);
  lead('#d23a30', [[388, 62, .9], [384, 62], [142, 62], [138, 63], [137, 66], [137, 72, .9]]);
  lead('#1d1e23', [[388, 70, .9], [384, 70], [145, 70], [141, 70], [141, 73, .9]]);
  for (const [x, y, w, h] of [[60, 115.5, 2.6, 9], [100, 115.5, 2.6, 9], [190, 59.5, 2.6, 13], [330, 59.5, 2.6, 13]]) bbox('leads', x, y, w, h, .1, .5, pm('leads', '#e9e4d6'));
  pbox.leads = new THREE.Box3(V(BX(134), LIFT + BY(125), BZ - .6), V(BX(54), LIFT + BY(112), BZ));
  // the USB-C lead: its plug in the DevKit, down to the desk and round the end to the charger
  bbox('usb', 22.5, 96.5, 13, 13, 1.45, .7, pm('usb', '#2c2e35'), true); bbox('usb', 25.5, 108, 7, 6, 1.5, .6, pm('usb', '#202228'));
  { const x = BX(29), z = BZ - 1.8;
    at(cable([V(x, LIFT + BY(112), z), V(x, LIFT + BY(124), z), V(x + .3, .9, z - .9), V(x + 1.5, .32, z - 2.2), V(27, .32, -5.4), V(30.4, .32, -2), V(30.6, .32, 4), V(27.5, .32, 10), V(PLX + 4, .4, 12.6), V(PLX + .6, 1.2, PLZ - 2.6), V(PLX, 3.4, PLZ - 1.5), V(PLX, 4.4, PLZ - .4)], .2, pm('usb', '#3b3d44'), 96), 0, 0, 0); }
  pbox.usb.expandByPoint(V(BX(22), LIFT + BY(134), BZ - 2.2));
  const hotLight = at(new THREE.PointLight('#ffb040', 0, 9, 1.6), 0, 0, 0);

  // ---------------------------------------------------------------- the lights
  const hemi = new THREE.HemisphereLight('#fff', '#000', 1); scene.add(hemi);
  const sun = new THREE.DirectionalLight('#fff1dc', 0); sun.position.set(-70, 120, 95); sun.target.position.set(0, 5, -4); scene.add(sun, sun.target);
  sun.castShadow = true; sun.shadow.mapSize.set(2048, 1024); Object.assign(sun.shadow.camera, { left: -85, right: 85, top: 52, bottom: -30, near: 20, far: 320 }); sun.shadow.bias = -.0008; sun.shadow.normalBias = .05; sun.shadow.radius = 2.5;
  const fill = new THREE.DirectionalLight('#fff', 0); scene.add(fill, fill.target);   // the eye's own light when it looks at the back
  const C1 = new THREE.Color(), C2 = new THREE.Color();
  // the page's state (light, back, W: read through) and this scene's own (k: how far round the eye is)
  const S = { get light() { return P0.light; }, get back() { return P0.back; }, get W() { return P0.W; }, k: 0, hot: '', dirty: 1, shadow: 1, bloom: 1, expo: 1, thr: 1, bst: .2 };
  function relight() {
    const L = S.light / 100, t = L ** 1.15, dk = 1 - L, on = clamp((L - .03) * 4, 0, 1), bk = S.k;
    hemi.color.copy(C1.set('#2a3466')).lerp(C2.set('#fff6e6'), t); hemi.groundColor.copy(C1.set('#0c0e20')).lerp(C2.set('#d8b890'), t);
    hemi.intensity = lerp(.3, 2.35, t);
    sun.intensity = 1.25 * t;
    lamp.intensity = on * (260 + 5600 * dk);
    shadeM.emissiveIntensity = on * (.25 + .75 * dk); bulbM.color.setRGB(3.2 * on, 2.5 * on, 1.5 * on);
    tube.intensity = 170 * (.3 + .7 * dk); crtM.color.setScalar(1 + .9 * dk); swM.color.setRGB(1 + 1.4 * dk, .35, .29);
    fill.intensity = bk * lerp(2.3, 1.0, t);
    led.uniforms.amb.value = .08 + .92 * t;
    knob.rotation.y = -(L * 260 - 130) * Math.PI / 180;
    S.bst = (.2 + .3 * dk) * S.bloom; S.thr = lerp(.74, 1.02, t); bloom.strength = S.bst; bloom.threshold = S.thr;
    S.expo = lerp(1, .16, t);                                  // by day the eye is stopped down: the panel's own light barely shows on the room
    fin.uniforms.vig.value = .34 + .5 * dk; fin.uniforms.grain.value = .014 + .01 * dk;
    renderer.setClearColor(C1.set('#04050b').lerp(C2.set('#f1e7cf'), t * .9));
    S.shadow = S.dirty = 1;
  }

  // ---------------------------------------------------------------- the picture: scene -> depth of field -> bloom -> grade
  // One pass draws the room to a target with its depth, makes a half-size blurred copy whose reach
  // follows each pixel's distance from the plane in focus, and joins the two.
  const QUAD_V = 'varying vec2 vUv; void main(){ vUv = uv; gl_Position = vec4( position.xy, 0., 1. ); }';
  const COC = `uniform sampler2D tDepth; uniform float near, far, focus, aper, tilt, maxc;
float vz( vec2 uv ){ float d = texture2D( tDepth, uv ).x; return near * far / ( far - d * ( far - near ) ); }
float coc( vec2 uv ){ float z = vz( uv ); float c = aper * ( 1. / focus - 1. / z ); float t = tilt * smoothstep( .2, .5, abs( uv.y - .5 ) ); return clamp( c + sign( c ) * t, -maxc, maxc ); }`;
  class Room extends Pass {
    constructor() {
      super(); this.dof = true; this.samples = 0;
      this.u = { tColor: { value: null }, tDepth: { value: null }, tHalf: { value: null }, tBlur: { value: null }, near: { value: 1 }, far: { value: 1 }, focus: { value: 100 }, aper: { value: 1.7 }, tilt: { value: .0014 }, maxc: { value: .011 }, asp: { value: 1 } };
      const m = (fs) => new THREE.ShaderMaterial({ uniforms: this.u, vertexShader: QUAD_V, fragmentShader: fs, depthTest: false, depthWrite: false });
      this.mDown = m(`uniform sampler2D tColor; varying vec2 vUv; ${COC} void main(){ gl_FragColor = vec4( min( texture2D( tColor, vUv ).rgb, 6. ), coc( vUv ) ); }`);
      this.mBlur = m(`uniform sampler2D tHalf; uniform float maxc, asp; varying vec2 vUv;
void main(){
  vec4 c0 = texture2D( tHalf, vUv ); vec3 acc = c0.rgb; float wsum = 1., csum = abs( c0.a );
  float a = fract( 52.9829189 * fract( dot( gl_FragCoord.xy, vec2( .06711056, .00583715 ) ) ) ) * 6.2832;
  for ( int i = 0; i < 20; i ++ ) {
    float r = sqrt( ( float( i ) + .5 ) / 20. ) * maxc; a += 2.39996;
    vec4 s = texture2D( tHalf, vUv + vec2( cos( a ) / asp, sin( a ) ) * r );
    float sc = s.a > c0.a ? min( abs( s.a ), abs( c0.a ) ) : abs( s.a );     // what lies behind cannot smear over what is sharper in front of it
    float w = smoothstep( r - .0012, r + .0004, sc );
    acc += s.rgb * w; wsum += w; csum += abs( s.a ) * w;
  }
  gl_FragColor = vec4( acc / wsum, csum / wsum );
}`);
      this.mJoin = m(`uniform sampler2D tColor, tBlur; varying vec2 vUv; ${COC} void main(){ vec4 b = texture2D( tBlur, vUv ); float c = max( abs( coc( vUv ) ), b.a * .8 ); gl_FragColor = vec4( mix( texture2D( tColor, vUv ).rgb, b.rgb, smoothstep( .0009, .0046, c ) ), 1. ); }`);
      this.mCopy = m('uniform sampler2D tColor; varying vec2 vUv; void main(){ gl_FragColor = vec4( texture2D( tColor, vUv ).rgb, 1. ); }');
      this.fs = new FullScreenQuad(this.mCopy);
    }
    setSize(w, h) {
      this.rt?.dispose(); this.h1?.dispose(); this.h2?.dispose();
      this.rt = new THREE.WebGLRenderTarget(w, h, { type: THREE.HalfFloatType, samples: this.samples, depthTexture: new THREE.DepthTexture(w, h) });
      const half = () => new THREE.WebGLRenderTarget(Math.ceil(w / 2), Math.ceil(h / 2), { type: THREE.HalfFloatType, depthBuffer: false });
      this.h1 = half(); this.h2 = half(); this.u.asp.value = w / h;
    }
    render(r, write) {
      r.setRenderTarget(this.rt); r.render(scene, camera);
      const u = this.u; u.tColor.value = this.rt.texture; u.tDepth.value = this.rt.depthTexture; u.near.value = camera.near; u.far.value = camera.far;
      const go = (m, to) => { this.fs.material = m; r.setRenderTarget(to); this.fs.render(r); };
      if (this.dof) { go(this.mDown, this.h1); u.tHalf.value = this.h1.texture; go(this.mBlur, this.h2); u.tBlur.value = this.h2.texture; go(this.mJoin, write); } else go(this.mCopy, write);
    }
  }
  const room = new Room();
  const bloom = new UnrealBloomPass(new THREE.Vector2(256, 256), .5, .62, .92);
  bloom.materialHighPassFilter.fragmentShader = bloom.materialHighPassFilter.fragmentShader.replace(/float v = [^;]+;/, 'float v = max( texel.r, max( texel.g, texel.b ) );');  // a pure red LED glows as much as a white one
  bloom.highPassUniforms.smoothWidth.value = .5;
  class Grade extends Pass {
    constructor() {
      super();
      this.uniforms = { tDiffuse: { value: null }, vig: { value: .3 }, grain: { value: .02 }, cell: { value: 2 } };
      this.fs = new FullScreenQuad(new THREE.ShaderMaterial({ uniforms: this.uniforms, vertexShader: QUAD_V, depthTest: false, depthWrite: false, fragmentShader: `uniform sampler2D tDiffuse; uniform float vig, grain, cell; varying vec2 vUv;
float b2( vec2 a ){ a = floor( a ); return fract( a.x / 2. + a.y * a.y * .75 ); }
float b4( vec2 a ){ return b2( .5 * a ) * .25 + b2( a ); }
float b8( vec2 a ){ return b4( .5 * a ) * .25 + b2( a ); }
void main(){
  vec3 c = texture2D( tDiffuse, vUv ).rgb;
  vec2 p = ( vUv - vec2( .5, .54 ) ) * vec2( 1.05, 1.3 );
  c *= 1. - vig * smoothstep( .32, .98, length( p ) * 1.42 );
  float m = max( c.r, max( c.g, c.b ) );                                  // a shoulder that keeps the hue: nothing clips to white
  if ( m > .8 ) c *= ( .8 + .2 * ( 1. - exp( -( m - .8 ) / .2 ) ) ) / m;
  c = mix( c * 12.92, 1.055 * pow( max( c, 1e-5 ), vec3( 1. / 2.4 ) ) - .055, step( .0031308, c ) );
  c += ( b8( gl_FragCoord.xy / cell ) - .5 ) * grain;                    // an ordered dither, a cell about a texel
  gl_FragColor = vec4( c, 1. );
}` }));
    }
    render(r, write, read) { this.uniforms.tDiffuse.value = read.texture; r.setRenderTarget(this.renderToScreen ? null : write); this.fs.render(r); }
  }
  const grade = new Grade(), fin = grade;
  const composer = new EffectComposer(renderer);
  composer.addPass(room); composer.addPass(bloom); composer.addPass(grade);

  // ---------------------------------------------------------------- quality: full size first, less if frames are slow
  // px: `?px=N` draws the room N times smaller and scales it up with square pixels (the other look: see README)
  const pxMode = +(q.get('px') || 0);
  const LEVELS = [{ pr: 2, dof: 1 }, { pr: 1.5, dof: 1 }, { pr: 1, dof: 1 }, { pr: 1, dof: 0 }, { pr: .75, dof: 0 }];
  let level = clamp(+(q.get('q') ?? 0), 0, 4), pinned = q.has('q') && q.get('scene') != 'try', G = { w: 1, h: 1 };
  function resize() {
    const w = host.clientWidth, h = host.clientHeight; if (!w || !h) return;
    G = { w, h };
    const dpr = devicePixelRatio || 1, lv = LEVELS[level], pr = pxMode ? 1 / pxMode : Math.min(dpr, lv.pr);
    room.dof = !!lv.dof && !pxMode; room.samples = pr < 1.5 && !pxMode ? 4 : 0;
    renderer.setPixelRatio(pr); renderer.setSize(w, h, false); composer.setPixelRatio(pr); composer.setSize(w, h);
    canvas.style.imageRendering = pxMode ? 'pixelated' : '';
    grade.uniforms.cell.value = Math.max(1, Math.round(pr * (pxMode ? 1 : 2)));
    camera.aspect = w / h; camera.updateProjectionMatrix();
    S.dirty = 1; pose(true);
  }

  // ---------------------------------------------------------------- the eye: fixed, a little above; it drifts with the pointer and goes round to the back
  const eye = { x: 0, y: 0, tx: 0, ty: 0 }, tgt = V(0, 0, 0), right = V(0, 0, 0), tmp = V(0, 0, 0);
  let turnT = -1e9, wallOp = 1, moved = true, lastPose = 0;
  const TURN = 1900;
  function pose(force) {
    const now = performance.now(), f = still ? 1 : clamp((now - turnT) / TURN, 0, 1), k = S.pin ?? (S.back ? ease(f) : 1 - ease(f));
    const moving = k !== S.k || Math.abs(eye.tx - eye.x) + Math.abs(eye.ty - eye.y) > .002;
    if (!moving && !force) { lastPose = now; return (moved = false); }
    moved = true; S.k = k;
    const ea = 1 - Math.exp(-clamp(now - lastPose, 0, 100) / 160); lastPose = now;   // (eased in time, not frames: it settles in about a second at any frame rate)
    eye.x += (eye.tx - eye.x) * ea; eye.y += (eye.ty - eye.y) * ea;
    const tv = Math.tan(FOV / 2 * Math.PI / 180), th = tv * camera.aspect;
    // the front: 51 cm either side of the display's middle and 31 above and below, unless that leaves
    // the LEDs under 5 device pixels each (a small window at 1x): then closer, down to 34 cm a side
    const fit = clamp(51.2 * G.w * (devicePixelRatio || 1) / (5 * 128) / 2, 34, 51) / 51;
    const dF = Math.max(51 * fit / th, 31 * fit / tv), dB = Math.max(29.5 / th, 10.5 / tv);
    const az = k * Math.PI, el = (lerp(12.5, 10, k) + Math.sin(k * Math.PI) * 9) * Math.PI / 180, d = lerp(dF, dB, k) * (1 + .12 * Math.sin(k * Math.PI));
    tgt.set(0, lerp(19 - (1 - fit) * 8, 7.4, k), lerp(-4, 0, k));
    camera.position.set(tgt.x + d * Math.sin(az) * Math.cos(el), tgt.y + d * Math.sin(el), tgt.z + d * Math.cos(az) * Math.cos(el));
    camera.lookAt(tgt);
    const dr = 1 - Math.sin(k * Math.PI);                      // the drift rests while it turns
    right.setFromMatrixColumn(camera.matrixWorld, 0); camera.position.addScaledVector(right, eye.x * 7 * dr); camera.position.y += -eye.y * 3 * dr;
    camera.lookAt(tgt); camera.updateMatrixWorld();
    room.u.aper.value = lerp(1.7, .7, k);                    // the back sharper: its parts are what is looked at
    room.u.focus.value = camera.position.distanceTo(tmp.set(clamp(camera.position.x, -20, 20) * .4, tgt.y, 0));
    for (const b of bills) b.rotation.y = Math.atan2(camera.position.x - b.getWorldPosition(tmp).x, camera.position.z - tmp.z);
    const op = clamp((camera.position.z - (ZW - 3)) / 9, 0, 1);  // behind the wall: it dissolves, and what hangs on it
    if (op !== wallOp) { wallOp = op; wallG.visible = op > .01; for (const m of wallMats) m.opacity = op; }
    fill.position.copy(camera.position); fill.target.position.copy(tgt);
    lampG.visible = k < .55;
    if (k > 0 && k < 1) { relight(); S.shadow = 1; }
    S.dirty = 1; spots();
    return true;
  }
  if (!still) {
    stage.addEventListener('pointermove', (e) => { if (e.pointerType == 'touch') return; const b = host.getBoundingClientRect(); eye.tx = S.back ? 0 : clamp((e.clientX - b.left) / b.width * 2 - 1, -1, 1); eye.ty = S.back ? 0 : clamp((e.clientY - b.top) / b.height * 2 - 1, -1, 1); }, sig);
    stage.addEventListener('pointerleave', () => { eye.tx = eye.ty = 0; }, sig);
  }

  // ---------------------------------------------------------------- what can be touched
  // In the picture (aria-hidden, pointer only: the page's buttons do the same by keyboard): the lamp,
  // the dimmer, the charger, the loose card. At the back: the page's part links, placed over the parts.
  const over = Object.assign(document.createElement('div'), { className: 'over' }); host.append(over);
  const spotsL = [];
  const spot = (id, label, box3, when, fn) => { const e = Object.assign(document.createElement('button'), { type: 'button', id, className: 'spot', tabIndex: -1, onclick: fn }); e.setAttribute('aria-label', label); over.append(e); spotsL.push([e, box3, when]); return e; };
  const cs = [0, 1, 2, 3, 4, 5, 6, 7].map(() => V(0, 0, 0)), R = [0, 0, 0, 0];
  const proj = (b) => {
    let x0 = 1e9, y0 = 1e9, x1 = -1e9, y1 = -1e9;
    for (let i = 0; i < 8; i++) { const v = cs[i].set(i & 1 ? b.max.x : b.min.x, i & 2 ? b.max.y : b.min.y, i & 4 ? b.max.z : b.min.z).project(camera), x = (v.x * .5 + .5) * G.w, y = (-v.y * .5 + .5) * G.h; x0 = Math.min(x0, x); y0 = Math.min(y0, y); x1 = Math.max(x1, x); y1 = Math.max(y1, y); }
    R[0] = x0; R[1] = y0; R[2] = x1 - x0; R[3] = y1 - y0; return R;
  };
  const atBack = () => S.back && S.k > .98, atFront = () => !S.back && S.k < .02;
  function spots() {
    for (const [e, b, when] of spotsL) {
      const show = when();
      e.hidden = !show; if (!show) continue;
      const [x, y, w, h] = proj(b);
      e.style.transform = `translate(${x.toFixed(1)}px,${y.toFixed(1)}px)`; e.style.width = w.toFixed(1) + 'px'; e.style.height = h.toFixed(1) + 'px';
    }
    const back = atBack();
    ui.parts(back);
    if (back) { for (const [id] of PARTS) ui.box(id, ...proj(pbox[id])); ui.place(); }
  }
  const lampBox = new THREE.Box3().setFromObject(lampShade).expandByScalar(.6), plugBox = new THREE.Box3(V(PLX - 2, 2, PLZ - 2), V(PLX + 2, 6, PLZ + 2)), looseBox = new THREE.Box3(V(28.8, 0, -6), V(31.6, .6, -3.2));
  spot('s-lamp', 'The lamp: room lights', lampBox, atFront, act.cycle);
  spot('s-dial', 'Room lights: bright, evening, off', new THREE.Box3().setFromObject(knob).expandByScalar(1.2), atFront, act.cycle);
  spot('s-plug', 'The charger: unplug it, or plug it back in', plugBox, atFront, act.power);
  spot('s-loose', 'The microSD card, on the desk: put it back', looseBox, () => S.W && !S.W.card && (atFront() || atBack()), act.card);
  // A part is pointed at: it keeps its colours and glows amber; the rest of the display go dim.
  function hot(id) {
    S.hot = id;
    for (const part in pmats) for (const [m, c, e] of pmats[part]) { m.color.copy(c).multiplyScalar(!id || part === id ? 1 : .36); if (e) m.emissive.copy(id && part === id ? C1.set('#5a3200') : e); }
    hotLight.intensity = id ? 9 : 0;
    if (id && pbox[id]) { pbox[id].getCenter(hotLight.position); hotLight.position.z -= 4; }
    S.dirty = 1;
  }

  // ---------------------------------------------------------------- the state, drawn
  const tweens = [];
  const tween = (ms, fn, delay = 0) => { tweens.push({ t0: performance.now() + delay, ms: still ? 1 : ms, fn }); };
  let wasPower = true, wasCard = true, ledOn = 0, ledTo = 0;
  function show() {
    const W = S.W;
    relight(); spots();
    S.dirty = 1;
    if (!W) return;
    led.uniforms.gain.value = 1.2 * (W.b ? (W.b / 96) ** .45 : 0);
    ledTo = W.power ? 1 : 0; pwr.color.setRGB(W.power ? 3 : .12, W.power ? .5 : .02, W.power ? .3 : .02); pmats.devkit.find((x) => x[0] === pwr)[1].copy(pwr.color);
    if (W.power !== wasPower) { wasPower = W.power; const a = plug.position.y, b = W.power ? 2.2 : 4.3, ra = plug.rotation.z, rb = W.power ? 0 : .32;
      tween(400, (t) => { const e = 1 - (1 - t) ** 3; plug.position.y = lerp(a, b, e); plug.position.x = PLX - (plug.position.y - 2.2) * .5; plug.rotation.z = lerp(ra, rb, e); S.shadow = 1; }); }
    if (W.card !== wasCard) { wasCard = W.card; const a = card.position.y, b = BY(19) + (W.card ? 0 : 2.4);
      if (W.card) { card.visible = true; loose.visible = false; }
      tween(550, (t) => { card.position.y = lerp(a, b, ease(t)); if (t == 1 && !W.card) { card.visible = false; loose.visible = true; spots(); } S.shadow = 1; }); }
  }
  function turn(back) { const now = performance.now(), left = clamp(1 - (now - turnT) / TURN, 0, 1); turnT = now - left * TURN; eye.tx = eye.ty = 0; if (!back) hot(''); S.dirty = 1; }

  // ---------------------------------------------------------------- the panel: the player's LEDs, one pixel each
  let dev = null, raw = null;
  const q4 = new Float64Array(12);
  function panel(n) {
    if (!raw) return;
    fx.drawImage(raw, 0, 0);
    ledTex.needsUpdate = true;
    if (n & 1) return;
    // each quarter's mean colour (the LEDs themselves: one pixel each)
    const px = fx.getImageData(0, 0, 128, 32).data, g = led.uniforms.gain.value / 1.2 * ledOn, lum = [0, 0, 0, 0];
    q4.fill(0);
    for (let i = 0; i < px.length; i += 4) { const j = ((i >> 2) & 127) >> 5; q4[j * 3] += px[i]; q4[j * 3 + 1] += px[i + 1]; q4[j * 3 + 2] += px[i + 2]; }
    let sum = 0, pic = 0;
    for (let i = 0; i < 4; i++) {
      const c = gcol[i]; for (let k = 0; k < 3; k++) { const v = (q4[i * 3 + k] / 1024 / 255) ** 2.2; c[k] += (v - c[k]) * .3; }
      sum += lum[i] = Math.min(1, Math.max(c[0], c[1], c[2]) * 2.4) ** .6;
      pic += (.2126 * c[0] + .7152 * c[1] + .0722 * c[2]) / 4;
    }
    // A bright, nearly white picture with the lights off washed the room into haze and over the things
    // in front: the four lights together are capped, the eye stops down as the picture's mean rises,
    // and the bloom's threshold rises with it (a dark warm picture is as before).
    const cap = Math.min(1, 2.2 / Math.max(sum, 1e-4)), stop = 1 - .45 * clamp((pic - .2) / .6, 0, 1);
    for (let i = 0; i < 4; i++) { const c = gcol[i], mx = Math.max(c[0], c[1], c[2], 1e-4); glow[i].color.setRGB(c[0] / mx, c[1] / mx, c[2] / mx); glow[i].intensity = 300 * lum[i] * cap * g * S.expo * stop; }
    bloom.threshold = S.thr + .3 * pic * ledOn; bloom.strength = S.bst * (1 - .45 * pic * ledOn);
  }

  // ---------------------------------------------------------------- the loop, and the governor
  // The governor reads windows of frame intervals (45 frames or 1.5 s, whichever ends first, after a
  // warm-up of 30 frames or 0.6 s): every interval counts, however long (a machine under 4 fps is the
  // slowest of all). "Slow" is against the display's own cadence (the window's 10th percentile, the
  // lowest seen, taken as at most 34 ms: a 30 or 40 Hz display, or a 30 fps cap, is not slow), at
  // least 21 ms. Slow: one level down; more than twice slow (about 45 ms at 60 Hz): straight to the
  // last level; over 80 ms at any level, or slow at the last: the flat scene takes over. The first
  // window after a change is thrown away (it read low), unless it is far over. Comfortably fast for two
  // windows: one level back up, twice a visit at most (no see-saw). Healthy for 30 s: `onHealthy`.
  const stats = { n: 0, js: 0, max: 0, level, fps: 0, ms: [], cad: 0 };
  const WIN = 45, WIN_MS = 1500, FAR = 80;
  let raf = 0, n = 0, prev = 0, win = [], winT = 0, warm = 30, warmT = 0, skip = 0, first = true, blink = -1, running = false;
  let cad = 34, good = 0, ups = 0, healthyT = 0, told = false;
  function govern(now) {
    if (warm > 0 && now < warmT) { warm--; winT = now; win = []; return; }
    if (win.length < WIN && now - winT < WIN_MS || win.length < 8) return;
    win.sort((a, b) => a - b);
    const med = win[win.length >> 1], p10 = win[Math.floor(win.length * .1)];
    win = []; winT = now;
    cad = Math.min(cad, clamp(p10, 6, 34)); stats.cad = cad;
    if (skip > 0) { skip--; if (med <= FAR) return; }
    stats.fps = 1000 / med; stats.ms.push(+med.toFixed(1));
    if (pinned || pxMode) return;
    const slow = Math.max(21, cad * 1.3);
    if (med > FAR || (med > slow && level == 4)) { onSlow?.(med); return; }
    if (med > slow) { level = med > 2.1 * slow ? 4 : level + 1; good = 0; healthyT = 0; changed(now); return; }
    good = med < slow * .6 ? good + 1 : 0;
    if (good >= 2 && level > 0 && ups < 2) { level--; ups++; good = 0; changed(now); return; }
    healthyT ||= now;
    if (!told && now - healthyT > 30000) { told = true; ui.onHealthy?.(); }
  }
  function changed(now) { stats.level = level; warm = 20; warmT = now + 400; skip = 1; win = []; resize(); }
  function draw(now) {
    raf = requestAnimationFrame(draw);
    const t0 = performance.now(), dt = prev ? now - prev : 0; prev = now;
    pose();
    for (let i = tweens.length - 1; i >= 0; i--) { const tw = tweens[i], t = clamp((now - tw.t0) / tw.ms, 0, 1); if (now >= tw.t0) { tw.fn(t); S.dirty = 1; } if (t == 1) tweens.splice(i, 1); }
    if (ledOn !== ledTo) { ledOn = clamp(ledOn + Math.sign(ledTo - ledOn) * (dt || 16) / 120, 0, 1); led.uniforms.on.value = ledOn; if (!ledOn) for (const g of glow) g.intensity = 0; S.dirty = 1; }
    const playing = S.W && S.W.power;
    if (playing) panel(n++);
    // (the tube's cursor rests while the panel is off: nothing to draw)
    const bl = still || !playing ? 1 : Math.floor(now / 550) & 1;
    if (bl !== blink) { blink = bl; crtDraw(bl); S.dirty = 1; }
    if (!(playing || S.dirty)) { prev = 0; return; }        // nothing moved and the panel is dark: the last picture stands
    if (S.shadow) { renderer.shadowMap.needsUpdate = true; S.shadow = 0; }
    S.dirty = 0;
    stats.calls = renderer.info.render.calls; stats.tris = renderer.info.render.triangles; renderer.info.reset();
    composer.render();
    const js = performance.now() - t0; stats.n++; stats.js += js; if (js > stats.max) stats.max = js;
    if (first) { first = false; canvas.classList.add('on'); onFirst?.(); warmT = now + 600; }
    if (dt > 0) win.push(Math.min(dt, 1000));
    govern(now);
  }
  function run(v) {
    if (v == running) return;
    running = v; cancelAnimationFrame(raf); raf = 0;
    if (v) { prev = 0; win = []; warm = Math.max(warm, 10); warmT = performance.now() + 300; healthyT = 0; S.dirty = 1; raf = requestAnimationFrame(draw); }
  }
  canvas.addEventListener('webglcontextlost', (e) => { e.preventDefault(); if (!disposed) onSlow?.(-1); }, sig);
  resize(); crtDraw(1);
  renderer.compile(scene, camera);
  if (q.has('hud')) { const hud = Object.assign(document.createElement('pre'), { className: 'hud' }); host.append(hud); const t = setInterval(() => { hud.textContent = `${gpu}\n${stats.fps.toFixed(0)} fps (a window's median; the display's cadence ${stats.cad.toFixed(1)} ms) · level ${level} · ${canvas.width}x${canvas.height} · dof ${room.dof ? 'on' : 'off'} · msaa ${room.samples} · scene js ${(stats.js / Math.max(1, stats.n)).toFixed(2)} ms`; }, 500); ac.signal.addEventListener('abort', () => clearInterval(t)); }
  // where things are on screen, in the scene's pixels (for the site's tests): the LED face, a patch of wall
  const RECTS = { face: new THREE.Box3(V(-25.6, LIFT + .3, BD / 2), V(25.6, LIFT + BH - .3, BD / 2 + .02)), wall: new THREE.Box3(V(26, 28, ZW), V(29, 32, ZW)) };
  return {
    rect: (name) => [...proj(RECTS[name])],
    kind: 'hd', turnMs: TURN, stats, S, composer, renderer, camera, led, glow, bloom,
    get level() { return level; }, set level(v) { level = v; pinned = true; resize(); },
    info: () => ({ gpu, soft, calls: stats.calls, tris: stats.tris, size: [canvas.width, canvas.height], level, dof: room.dof, msaa: room.samples }),
    show, turn, hot, run, layout: resize,
    // the player: its panel drawn at one canvas pixel an LED (only the flat scene looks at it), and its LEDs
    device(d) { if (d === dev) return; dev = d; d.scale(1); raw = d.frame(); show(); },
    dispose() { disposed = true; run(false); dev?.frame(false); over.remove(); ui.parts(false); drop(); },
  };
  } catch (e) { disposed = true; host.querySelector('.over')?.remove(); host.querySelector('.hud')?.remove(); drop(); throw e; }
}
