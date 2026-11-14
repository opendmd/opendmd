// The display's back, for the 3D scene: drawings in millimetres (v2's, shapes only: no names,
// marks or codes) rasterised to textures, on real geometry. 518 x 134 mm: the body, its lip included.
const r = (x, y, w, h, f, more = '') => `<rect x="${x}" y="${y}" width="${w}" height="${h}" fill="${f}" ${more}/>`;
const c = (x, y, rad, f, more = '') => `<circle cx="${x}" cy="${y}" r="${rad}" fill="${f}" ${more}/>`;
export const GOLD = '#c9a24a', TIN = '#c9ccd1', MUST = '#f0b33c', INK = '#3a2a32';
const screw = (x, y) => c(x, y, 2.6, TIN, 'stroke="#55585f" stroke-width=".5"') + r(x - 1.8, y - .35, 3.6, .7, '#55585f') + r(x - .35, y - 1.8, .7, 3.6, '#55585f');

// the two panels' backs, with the shade of what stands on them
export function rearSvg() {
  let s = r(0, 0, 518, 134, '#1a1b20');
  for (const x0 of [3, 259]) {
    let m = r(x0, 3, 256, 128, '#15161a') + r(x0 + 5, 8, 246, 118, '#0f2b25');
    for (const y of [26, 48, 86, 108]) m += r(x0 + 8, y, 240, .6, '#1a4a3c') + r(x0 + 8, y + 2, 240, .6, '#1a4a3c');
    for (const y of [16, 38, 76, 98]) for (const cell of [5, 67, 131.5, 196]) for (const dx of [9, 34]) {  // the driver chips
      const x = x0 + cell + dx;
      if ((y == 38 || y == 76) && (cell + dx < 30 || cell + dx > 220 || (cell + dx > 100 && cell + dx < 150))) continue;
      m += r(x, y, 13, 6, '#0a0b0d') + `<path d="M${x + 1} ${y - .7}h11M${x + 1} ${y + 6.7}h11" stroke="#8d8f96" stroke-width="1.1" stroke-dasharray=".9 .93"/>` + r(x + 1, y + 1, 1.2, 1.2, '#2a2c33');
      m += r(x + 15.5, y + 1, 2.2, 1.3, '#b89a6a') + r(x + 15.5, y + 3.6, 2.2, 1.3, '#6b6e75');
    }
    for (const x of [62, 126.5, 191]) m += r(x0 + x, 3, 5, 128, '#22242a') + r(x0 + x, 3, .7, 128, '#343740') + r(x0 + x + 4.3, 3, .7, 128, '#0b0c0f');  // the frame's ribs
    m += r(x0, 64.5, 256, 5, '#22242a') + r(x0, 64.5, 256, .7, '#343740') + r(x0, 68.8, 256, .7, '#0b0c0f');
    m += `<rect x="${x0 + 2.5}" y="5.5" width="251" height="123" fill="none" stroke="#22242a" stroke-width="5"/><rect x="${x0 + .35}" y="3.35" width="255.3" height="127.3" fill="none" stroke="#343740" stroke-width=".7"/>`;
    for (const [x, y] of [[14, 17], [242, 17], [14, 117], [242, 117], [129, 17], [129, 117]]) m += c(x0 + x, y, 6, '#272930', 'stroke="#0b0c0f" stroke-width=".7"') + c(x0 + x, y, 3, GOLD) + c(x0 + x, y, 1.3, '#3b2f14');  // screw bosses
    for (const x of [10, 236]) {  // the data headers, in and out
      m += r(x0 + x, 53, 10, 28, '#08080a', 'rx=".8" stroke="#2c2e35" stroke-width=".6"');
      for (let i = 0; i < 8; i++) m += r(x0 + x + 2.2, 55.6 + i * 3.1, 1.3, 1.3, GOLD) + r(x0 + x + 6.5, 55.6 + i * 3.1, 1.3, 1.3, GOLD);
    }
    m += r(x0 + 122, 58, 14, 18, '#e9e4d6', 'rx="1" stroke="#000" stroke-width=".5"') + r(x0 + 124.5, 61, 2.4, 12, '#8d8f96') + r(x0 + 131, 61, 2.4, 12, '#8d8f96');  // power in
    m += `<path d="M${x0 + 150} 29l5 3-5 3zM${x0 + 158} 29l5 3-5 3z" fill="#cfd8cf" opacity=".55"/>`;
    s += m;
  }
  s += r(258.4, 3, 1.2, 128, '#050506');
  // contact shade under what is fixed on the back (no light with a shadow reaches here)
  const sh = (x, y, w, h, o = .42) => `<rect x="${x}" y="${y}" width="${w}" height="${h}" rx="3" fill="#000" opacity="${o}" filter="url(#b)"/>`;
  s = `<defs><filter id="b" x="-20%" y="-20%" width="140%" height="140%"><feGaussianBlur stdDeviation="1.6"/></filter></defs>` + s
    + sh(7, 23, 46, 93, .6) + sh(235, 50, 18, 36) + sh(265, 50, 18, 36) + sh(236, 9, 46, 18, .35) + sh(236, 109, 46, 18, .35) + sh(58, 117, 34, 17, .35) + sh(426, 117, 34, 17, .35)
    + sh(11, 116, 126, 9, .3) + sh(125, 72, 13, 48, .3) + sh(136, 59, 256, 14, .3);
  return [s, 518, 134];
}
// the carrier board's face: blue, plain
export function carrierSvg() {
  let cb = r(8, 22, 42, 90, '#1f4fae', 'rx="3"') + r(9, 23, 40, 1.2, '#4f7fdc', 'rx=".6"');
  cb += `<path d="M12 42V92M46 42V88M40 98H31V104M24 40V30h-6" fill="none" stroke="#2f66d4" stroke-width=".9"/>`;
  for (const [x, y] of [[12, 26], [46, 26], [46, 108]]) cb += c(x, y, 2, '#0b0c0e', `stroke="${GOLD}" stroke-width=".8"`);
  cb += r(12.6, 43, 3.4, 54, '#0b0c0e') + r(42, 43, 3.4, 54, '#0b0c0e');
  for (let i = 0; i < 17; i++) cb += r(13.7, 44.4 + i * 3.1, 1.2, 1.2, GOLD) + r(43.1, 44.4 + i * 3.1, 1.2, 1.2, GOLD);
  return [cb, 42, 90, 8, 22];
}
// the DevKit's face: black, an aerial, pin rows, two chips, two buttons (the can and the socket are real boxes)
export function devkitSvg() {
  let d = r(15, 44, 28, 52, '#17181c', 'rx="1.5"') + r(15.6, 44.6, 26.8, 6.6, '#202228', 'rx="1"');
  d += `<path d="M19 50.4v-4h2.8v4h2.8v-4h2.8v4h2.8v-4h2.8v4h2.8v-4h2.2" fill="none" stroke="${GOLD}" stroke-width=".9"/>`;
  for (let i = 0; i < 15; i++) d += r(15.9, 47.6 + i * 3.2, 1.4, 1.4, GOLD) + r(40.7, 47.6 + i * 3.2, 1.4, 1.4, GOLD);
  d += r(20.5, 74, 6, 6, '#08080a') + `<path d="M20.5 73.5h6M20.5 80.5h6" stroke="#8d8f96" stroke-width=".7" stroke-dasharray=".7 .8"/>` + r(30, 74.5, 6.5, 3.6, '#08080a') + r(28.5, 82, 2, 1.2, '#b89a6a') + r(31.5, 82, 2, 1.2, '#6b6e75');
  for (const x of [17.6, 35.9]) d += r(x, 88, 4.5, 4.2, TIN, 'stroke="#6f7279" stroke-width=".4"') + c(x + 2.25, 90.1, 1.1, '#17181c');
  return [d, 28, 52, 15, 44];
}
export function ribbonSvg() {
  let b = `<defs><linearGradient id="rib" x2="1"><stop offset="0" stop-color="#8f9299"/><stop offset=".5" stop-color="#dfe1e6"/><stop offset="1" stop-color="#8f9299"/></linearGradient></defs>` + r(0, 0, 24, 26, 'url(#rib)');
  for (let i = 1; i < 16; i++) b += r(0, i * 1.62, 24, .3, '#6f7279');
  b += r(0, 0, 24, 1.7, '#d9483f');
  return [b, 24, 26];
}
// An SVG body -> a canvas, `px` texels a millimetre. Crisp edges: it is pixel art at this size.
export async function raster([body, w, h, x0 = 0, y0 = 0], px) {
  const src = `<svg xmlns="http://www.w3.org/2000/svg" viewBox="${x0} ${y0} ${w} ${h}" width="${w * px}" height="${h * px}" shape-rendering="crispEdges">${body}</svg>`;
  const img = new Image();
  img.src = 'data:image/svg+xml,' + encodeURIComponent(src);
  await img.decode();
  const cv = Object.assign(document.createElement('canvas'), { width: Math.round(w * px), height: Math.round(h * px) });
  cv.getContext('2d').drawImage(img, 0, 0, cv.width, cv.height);
  return cv;
}
