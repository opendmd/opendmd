// Formulas' shared parts, one copy for the web app's Formulas view
// (js/formulas.js) and the site's share page (/f/, which imports this file
// from the demo): the share link's encoder and decoder, the sliders' names,
// what a formula is in a sentence, the code's highlighting and the sample
// picture. No imports; the DOM only in `picker`.

export const MAX_SRC = 4096, MAX_NAME = 60;

// A share link: `<base>#formula=<src>&name=<name>&a=…` (URLSearchParams),
// with only the sliders the formula reads (`mask`: info()[5], bits 0-2).
export function link(f, base, mask = 7) {
  const q = new URLSearchParams({ formula: f.src });
  if (f.name) q.set('name', f.name);
  for (const [j, k] of ['a', 'b', 'c'].entries()) if (mask & (1 << j) && f[k]) q.set(k, +(+f[k]).toFixed(2));
  return base + '#' + q;
}
// And back: `{f, cut, has}` — the source and the name cut to what a
// formula can have (`cut` says the source was), the sliders kept to 0..1,
// `has` whether there was a formula at all.
export function parse(s) {
  const q = new URLSearchParams(s.replace(/^#/, '')), src = q.get('formula') || '', name = q.get('name') || 'Shared formula';
  const f = { name: name.slice(0, MAX_NAME), src: src.slice(0, MAX_SRC) };
  for (const k of 'abc') if (q.has(k)) f[k] = Math.min(1, Math.max(0, +q.get(k) || 0));
  return { f, cut: f.src !== src, has: q.has('formula') };
}

// A formula's own slider names: `-- a: speed`; a picker's choices after an `=`:
// `-- a: shape = cube, ball` (up to 6 words, else it is a slider).
const said = (src, k) => (src.match(new RegExp(`^\\s*(?:--|//)\\s*${k}\\s*[:=]\\s*(.+)$`, 'm')) || [])[1] || '';
export const sliderName = (src, k) => said(src, k).split('=')[0].trim() || k;
export function choices(src, k) {
  const c = said(src, k).split('=')[1]?.split(',').map((w) => w.trim()).filter(Boolean);
  return c?.length && c.length < 7 ? c : null;
}
// A picker: a button for each choice (a radio group: arrows, Home, End). Choice
// j of n sets the slider to j/(n−1) (one alone: 0); the one shown is round(v·(n−1)).
export function picker(names, label, v, set) {
  const n = names.length - 1, g = document.createElement('div');
  g.className = 'psi-pick';
  g.role = 'radiogroup';
  g.ariaLabel = label;
  const bs = names.map((t, j) => Object.assign(document.createElement('button'), { type: 'button', role: 'radio', textContent: t, onclick: () => to(j) }));
  const mark = (k) => bs.forEach((b, j) => { b.ariaChecked = j == k; b.tabIndex = j == k ? 0 : -1; });
  const to = (j) => { mark(j); bs[j].focus(); set(n && j / n); };
  g.onkeydown = (e) => {
    const k = bs.indexOf(document.activeElement), d = { ArrowRight: 1, ArrowDown: 1, ArrowLeft: -1, ArrowUp: -1 }[e.key];
    const j = d ? k + d : e.key == 'Home' ? 0 : e.key == 'End' ? n : -1;
    if (j >= 0 && j <= n && k >= 0) { e.preventDefault(); to(j); }
  };
  mark(Math.round(Math.min(1, Math.max(0, v || 0)) * n));
  g.append(...bs);
  return g;
}
// What it is, from info(): its loop, the clock, its cost on the panel.
const cost = (us) => (us <= 5000 ? 'light' : us <= 8000 ? 'medium' : 'heavy');
export const about = (i) => i.length ? [i[5] & 8 && `Loops every ${i[4] / 1000} s`, i[5] & 8 && !(i[5] & 32) && i[10] && i[10] != 20 && `${1000 / i[10]} frames a second`, i[5] & 16 && 'Follows the clock', !(i[5] & 24) && 'Still', i[8] && `Draws ${i[9]} lines and dots a frame`, `${cost(i[6])} for the panel`].filter(Boolean).join(' · ') : '';
export const TIP = (i) => `${i[0]} steps, about ${(i[6] / 1000).toFixed(1)} ms a frame on the panel (light: under 5 ms; medium: under 8)`;

// The code as highlighted HTML (numbers `.n` — a unary minus included —,
// comments `.c`, colours `.k` with `--k` their colour, for a swatch, the
// block words and the draw stage's `.w`; the
// error `bad` = [start, end] `.e`; everything else escaped text) and where
// its numbers are.
// (`line`, `dot` and `ink` only in a formula with a draw block: before it
// they were names a formula could set, and may still be; `fps` only before a
// number, as `fps 100`.)
const WORDS = /^(loop|frame|layer|part|draw|for|table|line|dot|ink)$/, DRAWS = /^(line|dot|ink)$/;
export function marks(s, bad) {
  const draws = /^\s*draw\s*\{/m.test(s.replace(/(--|\/\/).*/g, ''));
  const toks = [], cls = new Array(s.length).fill('');
  for (const m of s.matchAll(/(--|\/\/).*|'[ -~]'|#[\da-f]{3}([\da-f]{3})?\b|[a-z_]\w*|\d+\.?\d*|\.\d+/gi)) {
    const k = m[1] ? 'c' : m[0][0] == '#' ? `k" style="--k:${m[0]}` : /[\d.']/.test(m[0][0]) ? 'n' : (WORDS.test(m[0]) && (draws || !DRAWS.test(m[0])) || m[0] == 'fps' && /^[ \t]+\d/.test(s.slice(m.index + 3))) ? 'w' : '';
    let a = m.index;
    // A character in quotes ('A') is a number, but not one to drag.
    if (k === 'n' && m[0][0] !== "'") {
      // `-2` after an operator, a bracket or the start of a line is one number.
      if (s[a - 1] === '-' && /[-(,=*/+?:<>!%;\n]|^$/.test(s.slice(0, a - 1).trimEnd().slice(-1))) a--;
      toks.push({ s: a, e: m.index + m[0].length });
    }
    if (k) cls.fill(k, a, m.index + m[0].length);
  }
  if (bad) for (let i = bad[0]; i < Math.max(bad[1], bad[0] + 1) && i < s.length; i++) cls[i] = 'e ' + cls[i];
  const esc = (t) => t.replace(/&/g, '&amp;').replace(/</g, '&lt;');
  let html = '', i = 0;
  while (i < s.length) {
    let j = i;
    while (j < s.length && cls[j] === cls[i]) j++;
    html += cls[i] ? `<span class="${cls[i]}">${esc(s.slice(i, j))}</span>` : esc(s.slice(i, j));
    i = j;
  }
  if (bad && bad[0] >= s.length) html += '<span class="e"> </span>';
  return { html, toks };
}

// The sample picture a Formula that draws over a GIF (info()[5] & 32) is
// shown over when there is no GIF: a sky, a sun and hills, 128 × 32 RGB, top
// row first. opendmd-gallery's `sample()` (the site's thumbnails) and
// psi-check draw the same.
let SAMPLE = null;
export function sample() {
  if (!SAMPLE) {
    SAMPLE = new Uint8Array(12288);
    for (let i = 0; i < 4096; i++) {
      const x = i % 128, y = i >> 7;
      SAMPLE.set(y > 23 - 4 * Math.sin(x / 13) - 2 * Math.sin(x / 5) ? [30, 110 + (x * 7 + y * 13) % 50, 40]
        : (x - 98) ** 2 + 1.5 * (y - 10) ** 2 < 50 ? [255, 215, 80] : [40 + y * 6, 60 + y * 4, 170 - y * 3], i * 3);
    }
  }
  return SAMPLE;
}

