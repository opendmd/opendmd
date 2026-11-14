// The flat desk scene's drawing of the display's back (flat.js loads it when the page is idle, or
// at the first turn: it is not needed for the first picture). Millimetres, the drawing 518 x 134.
// --- the back, in millimetres (518 x 134: the body, its 3 mm lip included). Shapes only: no names,
// marks or codes. Two drawings: the panels' own backs, and what is fixed on them (it stands 5 mm off).
const r = (x, y, w, h, f, more = '') => `<rect x="${x}" y="${y}" width="${w}" height="${h}" fill="${f}" ${more}/>`;
const c = (x, y, rad, f, more = '') => `<circle cx="${x}" cy="${y}" r="${rad}" fill="${f}" ${more}/>`;
const g = (id, body, cls = '') => `<g class="p ${cls}" data-p="${id}">${body}</g>`;
const svg = (body) => `<svg viewBox="0 0 518 134" preserveAspectRatio="none">${body}</svg>`;
const GOLD = '#c9a24a', TIN = '#c9ccd1', MUST = '#f0b33c', INK = '#3a2a32';
const screw = (x, y) => c(x, y, 2.6, TIN, 'stroke="#55585f" stroke-width=".5"') + r(x - 1.8, y - .35, 3.6, .7, '#55585f') + r(x - .35, y - 1.8, .7, 3.6, '#55585f');
export function rear() {
  let s = '';
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
    s += g('panels', m, x0 > 3 ? 'm2' : '');
  }
  s += r(258.4, 3, 1.2, 128, '#050506');
  for (const y of [9, 109]) s += r(237, y, 44, 16, MUST, `rx="2" stroke="${INK}" stroke-width=".8"`) + r(238, y + 1, 42, 1.4, '#f7cf6f') + screw(245, y + 8) + screw(273, y + 8);  // joiners over the seam
  for (const x of [59, 427]) s += r(x, 117, 32, 17, MUST, `stroke="${INK}" stroke-width=".8"`) + r(x + 1, 118, 30, 1.4, '#f7cf6f') + screw(x + 16, 125);  // the feet's tabs
  return svg(s);
}
export function kit() {
  const lead = (d, col) => `<path d="${d}" fill="none" stroke="#050506" stroke-width="3.4"/><path d="${d}" fill="none" stroke="${col}" stroke-width="2.2"/><path d="${d}" fill="none" stroke="#fff" stroke-opacity=".25" stroke-width=".5" transform="translate(-.5 -.5)"/>`;
  const RED = '#d23a30', BLK = '#1d1e23';
  let s = `<defs><linearGradient id="can" x2="1" y2="1"><stop offset="0" stop-color="#eceef1"/><stop offset=".45" stop-color="#b4b8bf"/><stop offset="1" stop-color="#8e9299"/></linearGradient>
<linearGradient id="rib" x2="1"><stop offset="0" stop-color="#8f9299"/><stop offset=".5" stop-color="#dfe1e6"/><stop offset="1" stop-color="#8f9299"/></linearGradient></defs>`;
  // power: from the carrier's terminal to the first panel, and on to the second
  let p = lead('M19 111V114Q19 118 23 118H125Q129 118 129 114V74', RED) + lead('M13 111V118Q13 122 17 122H131Q135 122 135 118V74', BLK);
  p += lead('M389 62H141Q137 62 137 66', RED) + lead('M389 70H141', BLK);
  for (const [x, y, h] of [[60, 115.5, 9], [100, 115.5, 9], [190, 59.5, 13], [330, 59.5, 13]]) p += r(x, y, 2.6, h, '#e9e4d6', 'stroke="#000" stroke-width=".3"');
  for (const x of [127.8, 133.8]) p += r(x, 70, 2.4, 6, TIN);
  for (const y of [60.8, 68.8]) p += r(385, y, 6, 2.4, TIN);
  s += g('leads', p);
  // the ribbon across the seam
  let b = r(247, 54, 24, 26, 'url(#rib)', 'stroke="#000" stroke-width=".5"');
  for (let i = 1; i < 16; i++) b += r(247, 54 + i * 1.62, 24, .3, '#6f7279');
  b += r(247, 54, 24, 1.7, '#d9483f');
  for (const x of [236, 266]) b += r(x + 1.5, 51.5, 16, 34, '#000', 'opacity=".35" rx="1.2"') + r(x, 50, 16, 34, '#121317', 'rx="1.2" stroke="#2f3138" stroke-width=".6"') + r(x + (x < 250 ? 3 : 10), 54, 3, 26, '#1e2026') + r(x + (x < 250 ? 9 : 4), 52, 3, 1.5, '#2f3138');
  s += g('ribbon', b);
  // the carrier board: blue, plain
  let cb = r(10, 25, 42, 90, '#000', 'opacity=".4" rx="3"') + r(8, 22, 42, 90, '#1f4fae', 'rx="3" stroke="#0e2a63" stroke-width=".8"') + r(9, 23, 40, 1.2, '#4f7fdc', 'rx=".6"');
  cb += `<path d="M12 42V92M46 42V88M40 98H31V104M12 96h-1M24 40V30h-6" fill="none" stroke="#2f66d4" stroke-width=".9"/>`;
  for (const [x, y] of [[12, 26], [46, 26], [46, 108]]) cb += c(x, y, 2, '#0b0c0e', `stroke="${GOLD}" stroke-width=".8"`);
  cb += r(12.6, 43, 3.4, 54, '#0b0c0e') + r(42, 43, 3.4, 54, '#0b0c0e');
  cb += c(43, 100.5, 4.6, '#23252b', 'stroke="#0b0c0e" stroke-width=".6"') + c(43, 100.5, 3.2, '#3d4049') + `<path d="M40.2 98.4A3.6 3.6 0 0 1 45.8 98.4" fill="none" stroke="${TIN}" stroke-width="1.2"/>`;
  cb += r(9, 97, 14, 14, '#2e9b5a', 'rx="1" stroke="#145c33" stroke-width=".6"') + r(9.6, 97.6, 12.8, 1.2, '#57c887') + screw(13, 103) + screw(19, 103) + r(11.6, 109, 2.8, 2, '#0b0c0e') + r(17.6, 109, 2.8, 2, '#0b0c0e');
  s += g('carrier', cb);
  // the card, in its socket
  s += g('sd', `<g class="sd">` + `<path d="M23 11h9l3 3v13H23z" fill="#191a1e" stroke="#000" stroke-width=".4"/>` + r(24.3, 14.5, 9, 6, MUST) + r(24.3, 14.5, 9, 1, '#f7cf6f') + `</g>`
    + r(19, 25, 20, 15, TIN, 'stroke="#6f7279" stroke-width=".5" rx="1"') + r(20, 26, 18, 1, '#eef0f3') + r(21.5, 29, 5, 8.5, '#a3a6ad') + r(29, 36.4, 8, 1.2, '#8d8f96') + r(22, 24.4, 14, 1.2, '#2a2c33'));
  // the DevKit: black, an aerial, a plain shield can, two buttons, a USB-C socket
  let d = r(16.5, 46.5, 28, 52, '#000', 'opacity=".45" rx="1.5"') + r(15, 44, 28, 52, '#17181c', 'rx="1.5" stroke="#000" stroke-width=".5"') + r(15.6, 44.6, 26.8, 6.6, '#202228', 'rx="1"');
  d += `<path d="M19 50.4v-4h2.8v4h2.8v-4h2.8v4h2.8v-4h2.8v4h2.8v-4h2.2" fill="none" stroke="${GOLD}" stroke-width=".9"/>`;
  d += r(19.5, 52.5, 19, 18, 'url(#can)', 'stroke="#6f7279" stroke-width=".5" rx="1"') + r(20.5, 53.4, 17, .9, '#fff', 'opacity=".7"') + c(21.3, 68.7, .6, '#8e9299') + c(36.7, 68.7, .6, '#8e9299');
  for (let i = 0; i < 15; i++) d += r(15.9, 47.6 + i * 3.2, 1.4, 1.4, GOLD) + r(40.7, 47.6 + i * 3.2, 1.4, 1.4, GOLD);
  d += r(20.5, 74, 6, 6, '#08080a') + `<path d="M20.5 73.5h6M20.5 80.5h6" stroke="#8d8f96" stroke-width=".7" stroke-dasharray=".7 .8"/>` + r(30, 74.5, 6.5, 3.6, '#08080a') + r(28.5, 82, 2, 1.2, '#b89a6a') + r(31.5, 82, 2, 1.2, '#6b6e75');
  for (const x of [17.6, 35.9]) d += r(x, 88, 4.5, 4.2, TIN, 'stroke="#6f7279" stroke-width=".4"') + c(x + 2.25, 90.1, 1.1, '#17181c');
  d += r(23.5, 88.5, 11, 7.5, TIN, 'stroke="#6f7279" stroke-width=".5" rx="1.6"') + r(24.6, 89.4, 8.8, .8, '#eef0f3');
  s += g('devkit', d);
  // the USB-C lead
  let ub = `<path d="M29 110V141" stroke="#050506" stroke-width="5.4"/><path d="M29 110V141" stroke="#3b3d44" stroke-width="4"/><path d="M28 110V141" stroke="#fff" stroke-opacity=".14" stroke-width=".7"/>`;
  ub += r(25.5, 106, 7, 6, '#202228', 'rx="1" stroke="#000" stroke-width=".4"') + r(22.5, 94.5, 13, 13, '#2c2e35', 'rx="2" stroke="#000" stroke-width=".5"') + r(23.5, 95.3, 11, 1, '#4a4d57', 'rx=".5"');
  for (let i = 0; i < 3; i++) ub += r(25, 100.5 + i * 1.8, 8, .7, '#17181c');
  s += g('usb', ub);
  return svg(s);
}

