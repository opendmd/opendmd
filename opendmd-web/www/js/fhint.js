// The Formula editor's help at the caret (js/formulas.js editor(), imported
// at the first key typed or the Cheat sheet's first click): names that fit
// what is typed — the built-ins and the formula's own — in a list under the
// caret (on touch a row of chips under the code), Tab or Enter to put one
// in; inside a call, its values and what it does, the one being typed in
// bold; and the cheat sheet. The built-ins' rows and names are generated
// from opendmd-psi (tests/builtins.rs).

import { h, clear } from './util.js';
import { popup, closeMenu } from './overrides.js';

// psi-builtins: cheat sheet (generated from opendmd-psi/src/builtins.rs: PSI_BLESS=1 cargo test --features docs --test builtins)
const VARS = [
  ['x, y', 'where the pixel is: x 0 at the left to 1 at the right, y 0 at the top to 1 at the bottom'],
  ['px, py', 'the pixel’s column 0–127 from the left and row 0–31 from the top'],
  ['t', 'seconds into the loop'],
  ['phase', 'where the frame is in the loop, 0 to 1: t / the loop’s length (OpenDMD 1.1: update a panel that says it is unknown)'],
  ['hour, minute, second', 'the time of day'],
  ['a, b, c', 'the controls, in the order the control lines declare them (undeclared: a slider 0 to 1 at 0); any built-in name may be set as your own'],
  ['w, h', '128 and 32'],
  ['i', 'the pixel’s number, 0–4095'],
  ['cx, cy', 'from the middle: −2 to 2 across, −0.5 to 0.5 down'],
  ['r, angle', 'distance and angle (turns, clockwise from the right) from the middle'],
  ['ur, ug, ub', 'the colour under the formula: the GIF it is drawn over, 0 to 1'],
  ['ul', 'how bright that is'],
  ['under(x, y)', 'the picture under the formula at pixel x, y (the nearest, held to the panel), as a #… colour. under(px + 2, py) shifts it; under(floor(px/4)*4, floor(py/4)*4) pixelates it. under alone is this pixel\'s'],
  ['daylight', 'how light it is outside where the panel is, 0 at night to 1 by day (twilight between); the panel\'s location in Settings, else 1'],
  ['sun.alt', 'the sun\'s height in turns: 0 on the horizon, 0.25 straight up, below 0 at night; 0.1 without a location'],
  ['moon.phase, season, yearday', 'the moon (0 new, 0.5 full), the time of year (0 midwinter, 0.5 midsummer) and the day of the year (0 on 1 January)'],
  ['night, bright', '1 while night mode dims the panel; the panel\'s brightness, 0 to 1'],
  ['replay.playing, replay.platform, replay.game', 'RePlayOS: 1 while a game plays; its platform as a number (0 none); a number 0 to 1 for the game, the same each time it plays']
];
const FUNCS = [
  ['sin(v) cos(v)', 'a wave: one turn per 1, between −1 and 1'],
  ['tri(v) saw(v) sqr(v)', 'triangle, sawtooth and square waves'],
  ['abs floor frac sqrt', 'size, whole part, fraction, square root'],
  ['min max clamp(v, lo, hi)', 'limits'],
  ['mix(a, b, k)', 'from a to b as k goes 0 to 1'],
  ['step(edge, v)', '1 past the edge, else 0'],
  ['smoothstep(e0, e1, v)', 'a soft step'],
  ['len(dx, dy)', 'distance'],
  ['atan2(dy, dx)', 'angle, in turns'],
  ['noise(x, y, z)', 'smooth random hills, 0 to 1'],
  ['hash(a, b)', 'random 0 to 1'],
  ['select(c, a, b)', 'a when c is true (not 0), else b; or c ? a : b'],
  ['pick(k, a, b, …)', 'a at k = 0, b at 1, …; else 0'],
  ['since(replay.game)', 'seconds since a built-in input last changed, 32767 if it never has. max(0, 1 - since(replay.game)) flashes on a change'],
  ['was(replay.game)', 'a built-in input\'s value before its last change. mix(was(k), k, min(1, since(k))) fades from the old to the new'],
  ['digit(n, col, row)', '1 where digit n has a pixel in the panel’s font; col 0–4, row 0–6 from the top. Add , small or , bold (chunky digits) for another font'],
  ['glyph(\'A\', col, row)', 'the same for a character'],
  ['bit(n, i)', 'bit i of a whole number: 1 or 0'],
  ['< <= > >= == !=', 'compare: 1 or 0; and, or, not'],
  ['%', 'remainder: v % 1 is the fraction'],
  [';', 'a new line, on the same line'],
  ['loop 8', 'the loop’s length in seconds, and the least time it plays'],
  ['loop forever', 'goes on without repeating: t grows from when it starts (use frac, %, sin, noise of it); seed is 0 to 1, new each time it plays'],
  ['smooth', 'drawn every refresh of the panel (100 to 125 a second), not every 2nd; for fast motion, a light formula only (OpenDMD 1.1; before, fps 100)', 'smooth\n'],
  ['layer add { … }', 'a layer with its own names and colour, on what is below: over, add, multiply, screen, max or min'],
  ['layer rate 4 { … }', 'a slow bottom layer: a band of its rows is redrawn each frame, so every row every 4th frame, and all of it when the clock or a control it reads changes. The layers above draw over it every frame', 'layer rate 4 {\n  \n}\n'],
  ['fn ring(cx, r) = abs(len(px - cx, py - 16) - r) < 1', 'a function: a value worked out from its values, used as ring(64, 8). With { … } instead of = …, it holds lines that draw, called on a line of their own in draw', 'fn f(a) = '],
  ['fn p(x, y) { … }', 'a function whose values are x and y, so px, py, r, angle … inside follow them. With a colour as its last line, p(1 - x, y) draws it mirrored', 'fn p(x, y) {\n  \n}\n'],
  ['use Plasma as p', 'puts in one of your formulas or an example, as a function fn p(x, y) { … }', 'use Plasma as p\n']
];
const DRAW = [
  ['draw { … }', 'once a frame, before the pixels: values, loops, lines and dots into ink', 'draw {\n  \n}\n'],
  ['for k < 12 { … }', 'inside draw: k = 0, 1, … 11; two deep at most, 2048 turns a frame', 'for k < 12 {\n  \n}'],
  ['line(x0, y0, x1, y1, v0, v1)', 'a line, in pixels from the top left; v0, v1 its ink at each end, 0 to 1 (else 1)'],
  ['dot(x, y, v)', 'one pixel of ink v'],
  ['cls(#102)', 'paint the whole panel one colour. Paint is drawn over the colour line, and the last painted wins'],
  ['pset(x, y, #f80)', 'paint one pixel; line(x0, y0, x1, y1, c) and dot(x, y, c) paint too when their last value is a colour'],
  ['rect(x0, y0, x1, y1, c) rectfill(…)', 'paint a box (its corners included), or a filled one'],
  ['circ(x, y, r, c) circfill(…)', 'paint a circle, or a filled one'],
  ['trifill(x0, y0, x1, y1, x2, y2, c)', 'paint a filled triangle'],
  ['print("TEXT", x, y, c)', 'paint text in the panel\'s font (add small); or a number: print(minute, x, y, c, small, 2) with at least 2 digits'],
  ['spr(s, x, y, flip)', 'paint sprite s with its top left at x, y, in its key\'s colours (key . 0, R #f00); flip 1 across, 2 down, 3 both'],
  ['pal(fire, v), gray(v)', 'colours to paint with besides #…: a palette\'s or a gray, v 0 to 1'],
  ['if c { … } else { … }', 'inside draw: only when c is not 0', 'if  {\n  \n}'],
  ['ink', 'what draw put at the pixel, 0 to 1: pal(ice, ink), #40c0ff * ink'],
  ['ink(16)', 'the same in 16 levels, as a whole number. Ink k/16 reads exactly k, for pick(ink(16), …); 1 to 255 levels'],
  ['table T = 1, -2, 0.5', 'numbers, 256 a table and 1024 in all (tables and sprites)', 'table T = '],
  ['T[k]', 'number k of table T, from 0 (0 outside it)'],
  ['sprite s { … }', 'a small picture in characters. First the key, key . 0, X 1, gives each character the number it reads as (up to 16); then rows up to 64 wide and 32 tall', 'sprite s {\n  key . 0, X 1\n  .X.\n}\n'],
  ['s[u, v]', 'sprite s at column u, row v from its top left: its key’s number (0 outside)']
];
const COLS = [
  ['hsv(hue, sat, val)', 'colour: hue in turns, 0 to 1 each'],
  ['rgb(r, g, b)', 'red, green, blue: 0 to 1'],
  ['pal(fire, v)', 'a palette: fire, ice, ocean, chrono, amber, rainbow, gray'],
  ['palette sea = #024, #08c, #fff', 'your own palette of up to 32 colours (8 palettes; pal and cycle read 3). pal(sea, 0) is the first, pal(sea, 1) the last, pal(sea, 0.5) the middle one', 'palette p = #000, #f80, #fff\n'],
  ['scene s when c { v = … }', 'values worked out only while c is not 0 (c once a frame or a row). After it, its names read 0 while it is off. The estimate counts two scenes at once', 'scene s when  {\n  \n}\n'],
  ['palette sky = tween(day, night, w)', 'a palette mixed once a frame, day towards night by w (0 to 1, of t or sliders); tweens nest; tween(a, b, w), tween(c, d, v) fills the low and high 16 colours', 'palette p = tween(, , )\n'],
  ['palette sea = deep #024, foam #fff', 'a colour may have a name: sea.foam is where pal finds it (sea.step from one colour to the next, sea.n how many)'],
  ['pal(fire, v) * k', 'a palette colour, dimmed (or lit) by k'],
  ['cycle(fire, v, s)', 'palette cycling: the palette read at v, turned by s (one turn per 1, so t/4 turns it every 4 s). It wraps where pal stops; s may change by row; cheap when v is the same every frame'],
  ['gray(v)', 'white, 0 to 1'],
  ['#ff8800', 'a colour: alone, or #f80 * v, mix(#000, #f80, v)'],
  ['rgb(r, g, b, k)', 'a last value: how much it covers, 0 to 1 (what is under shows through); rgb(#f80, k)']
];
const DECLS = [
  ['name "Sunrise"', 'the formula’s name (60 characters at most)', 'name ""\n'],
  ['about "…"', 'one line about it (120 at most)', 'about ""\n'],
  ['control speed 0..1 = 0.5 "Speed"', 'a slider: its name in the formula, range, default and label (three controls at most)', 'control speed 0..1 = 0.5 "Speed"\n'],
  ['control mode = calm, wild', 'buttons: the value is the choice’s number, 0 for the first (pick(mode, …)); two words make a switch', 'control mode = calm, wild\n'],
  ['look screen 40', 'it can be the panel’s look, drawn over the GIF with a blend and an amount (look alone is over, 100)', 'look\n']
];
const NAMES = [
  ['x', 0],
  ['y', 0],
  ['px', 1],
  ['py', 1],
  ['i', 7],
  ['t', 2],
  ['hour', 4],
  ['minute', 4],
  ['second', 4],
  ['a', 5],
  ['b', 5],
  ['c', 5],
  ['ur', 10],
  ['ug', 10],
  ['ub', 10],
  ['ul', 11],
  ['ink', 72],
  ['cx', 8],
  ['cy', 8],
  ['r', 9],
  ['angle', 9],
  ['phase', 3],
  ['w', 6],
  ['h', 6],
  ['daylight', 13],
  ['sun.alt', 14],
  ['moon.phase', 15],
  ['season', 15],
  ['yearday', 15],
  ['night', 16],
  ['bright', 16],
  ['replay.playing', 17],
  ['replay.platform', 17],
  ['replay.game', 17],
  ['sin(v)', 18],
  ['cos(v)', 18],
  ['tri(v)', 19],
  ['saw(v)', 19],
  ['sqr(v)', 19],
  ['abs(v)', 20],
  ['floor(v)', 20],
  ['frac(v)', 20],
  ['fract(v)', 20],
  ['sqrt(v)', 20],
  ['min(a, b)', 21],
  ['max(a, b)', 21],
  ['step(edge, v)', 23],
  ['hash(a, b)', 28],
  ['len(dx, dy)', 25],
  ['atan2(dy, dx)', 26],
  ['clamp(v, lo, hi)', 21],
  ['mix(a, b, k)', 22],
  ['smoothstep(e0, e1, v)', 24],
  ['noise(x, y, z)', 27],
  ['select(c, a, b)', 29],
  ['bit(n, i)', 35],
  ['digit(n, col, row)', 33],
  ['glyph(\'A\', col, row)', 34],
  ['pick(k, a, b, …)', 30],
  ['rgb(r, g, b)', 48],
  ['hsv(hue, sat, val)', 47],
  ['pal(fire, v)', 49],
  ['gray(v)', 56],
  ['cycle(fire, v, s)', 55],
  ['line(x0, y0, x1, y1, v0, v1)', 61],
  ['dot(x, y, v)', 62],
  ['pset(x, y, #f80)', 64],
  ['rect(x0, y0, x1, y1, c)', 65],
  ['rectfill(x0, y0, x1, y1, c)', 65],
  ['circ(x, y, r, c)', 66],
  ['circfill(x, y, r, c)', 66],
  ['trifill(x0, y0, x1, y1, x2, y2, c)', 67],
  ['cls(#102)', 63],
  ['spr(s, x, y, flip)', 69],
  ['print("TEXT", x, y, c)', 68],
  ['since(replay.game)', 31],
  ['was(replay.game)', 32],
  ['under(x, y)', 12],
  ['tween(day, night, w)', 52],
  ['seed', 40],
  ['scene', 51],
  ['when', 51],
  ['if', 71],
  ['else', 71],
  ['key', 69],
  ['loop', 39],
  ['layer', 42],
  ['part', 20],
  ['use', 46],
  ['draw', 59],
  ['for', 60],
  ['table', 74],
  ['fn', 44],
  ['fps', 41],
  ['smooth', 41],
  ['forever', 40],
  ['sprite', 76],
  ['palette', 50],
  ['rate', 43],
  ['name', 78],
  ['about', 79],
  ['control', 80],
  ['look', 82],
  ['fire', 49],
  ['ice', 49],
  ['ocean', 49],
  ['chrono', 49],
  ['amber', 49],
  ['rainbow', 49],
  ['over', 42],
  ['add', 42],
  ['multiply', 42],
  ['screen', 82],
  ['normal', -1],
  ['small', 33],
  ['bold', 33]
];
// psi-builtins end
const ROWS = [VARS, FUNCS, COLS, DRAW, DECLS].flat();
const item = ([l, r], d) => { const k = l.indexOf('('); return { n: k < 0 ? l : l.slice(0, k), sig: l, f: k > 0, d: d ?? ROWS[r]?.[1] ?? '' }; };
let BUILT = null;

// The formula's own names, before the built-ins they hide: values set,
// controls, functions and their values, loop counts, tables, sprites,
// palettes and their entries.
function own(s) {
  const o = [], add = (n, d, sig = n) => o.push(item([sig], d)), t = s.replace(/(--|\/\/).*/g, ''), all = (re, f) => { for (const m of t.matchAll(re)) f(m); };
  all(/^[ \t]*fn[ \t]+([A-Za-z_]\w*)[ \t]*\(([^)]*)\)/gm, (m) => { add(m[1], 'your function', `${m[1]}(${m[2].trim()})`); for (const a of m[2].split(',')) if (a.trim()) add(a.trim(), `a value of ${m[1]}`); });
  all(/(?:^|;)[ \t]*([A-Za-z_]\w*)[ \t]*=(?!=)/gm, (m) => add(m[1], 'your value'));
  all(/^[ \t]*control[ \t]+([A-Za-z_]\w*)/gm, (m) => add(m[1], 'a control'));
  all(/\bfor[ \t]+([A-Za-z_]\w*)[ \t]*</g, (m) => add(m[1], 'a loop’s count'));
  all(/^[ \t]*table[ \t]+([A-Za-z_]\w*)/gm, (m) => add(m[1], 'your table', m[1] + '[k]'));
  all(/^[ \t]*sprite[ \t]+([A-Za-z_]\w*)/gm, (m) => add(m[1], 'your sprite', m[1] + '[u, v]'));
  all(/^[ \t]*palette[ \t]+([A-Za-z_]\w*)[ \t]*=([^\n]*(?:,[ \t]*\n[^\n]*)*)/gm, (m) => {
    add(m[1], 'your palette');
    for (const e of [...[...m[2].matchAll(/([A-Za-z_]\w*)[ \t]+#/g)].map((e) => e[1]), 'step', 'n']) add(`${m[1]}.${e}`, `palette ${m[1]}’s ${e}`);
  });
  return o;
}

// The call the caret is in: its name and which value (commas back to the
// open bracket, on this line), or null.
function call(s, c) {
  let d = 0, k = 0;
  for (let i = c - 1; i >= 0; i--) {
    const ch = s[i];
    if ('\n;{}'.includes(ch)) return null;
    if (ch === ')' || ch === ']') d++;
    else if (ch === '(' || ch === '[') {
      if (d) { d--; continue; }
      const m = ch === '(' && /[A-Za-z_][\w.]*$/.exec(s.slice(0, i));
      if (m) return { n: m[0], k };
      // (A bracket of its own: the call is further out.)
      k = 0;
    } else if (ch === ',' && !d) k++;
  }
  return null;
}

// The list (one for the page, whichever editor has it: CUR), kept in place
// as the page scrolls, closed when it resizes.
let BOX = null, CUR = null;
function boxed() {
  if (BOX) return BOX;
  BOX = h('div.psi-ac', { id: 'psi-ac', hidden: true, onpointerdown: (e) => e.preventDefault(), onclick: (e) => { const d = e.target.closest('[data-i]'); if (d) CUR?.pick(+d.dataset.i); } });
  document.body.append(BOX);
  addEventListener('scroll', () => { if (!BOX.hidden) CUR?.show(); }, { passive: true, capture: true });
  addEventListener('resize', () => CUR?.close());
  return BOX;
}

// `o`: {ta, put(range, text), measure() (a character's width), after (the
// element the chips go after), touch}.
export function hint(o) {
  const { ta, touch } = o;
  BUILT ??= NAMES.map((n) => item(n));
  const box = touch ? h('div.psi-chips', { id: 'psi-chips', hidden: true }) : boxed();
  if (touch) o.after.after(box);
  // (A combobox's list, for a screen reader: the option chosen, the call's values described.)
  Object.entries({ 'aria-autocomplete': 'list', 'aria-expanded': 'false', 'aria-controls': box.id }).forEach(([k, v]) => ta.setAttribute(k, v));
  const desc = ta.getAttribute('aria-describedby') || '';
  let items = [], sel = 0, at = -1, word = '';
  // The name being typed (at the caret, outside comments and quotes), or ''.
  const typed = () => {
    const s = ta.value, c = ta.selectionStart, l = s.slice(s.lastIndexOf('\n', c - 1) + 1, c);
    if (c !== ta.selectionEnd || /--|\/\//.test(l) || l.split('"').length % 2 == 0 || /[\w.]/.test(s[c] || '')) return '';
    return (/(?:^|[^\w.#'])([A-Za-z_][\w.]*)$/.exec(l) || [])[1] || '';
  };
  const esc = (t) => t.replace(/&/g, '&amp;').replace(/</g, '&lt;');
  // A signature with value `k` in bold.
  function sig(it, k) {
    const m = /^([^(]*\()(.*)\)$/.exec(it.sig);
    if (!m) return esc(it.sig);
    const a = m[2].split(/, (?![^(]*\))/), j = Math.min(k, a.length - 1);
    return esc(m[1]) + a.map((x, i) => (i === j ? `<b>${esc(x)}</b>` : esc(x))).join(', ') + ')';
  }
  function place(el) {
    const s = ta.value, c = ta.selectionStart, r = ta.getBoundingClientRect(), st = getComputedStyle(ta), lh = parseFloat(st.lineHeight);
    const l = s.slice(s.lastIndexOf('\n', c - 1) + 1, c - (items.length ? word.length : 0));
    let col = 0;
    for (const ch of l) col += ch === '\t' ? 2 - (col % 2) : 1;
    const x = r.left + ta.clientLeft + parseFloat(st.paddingLeft) + col * o.measure() - ta.scrollLeft;
    let y = r.top + ta.clientTop + parseFloat(st.paddingTop) + (s.slice(0, c).split('\n').length) * lh - ta.scrollTop;
    if (y + el.offsetHeight > innerHeight - 4) y -= lh + el.offsetHeight;
    el.style.left = Math.max(4, Math.min(x, innerWidth - el.offsetWidth - 4)) + 'px';
    el.style.top = y + 'px';
  }
  function show() {
    const s = ta.value, c = ta.selectionStart, f = !items.length && document.activeElement === ta && c === ta.selectionEnd && call(s, c);
    const it = f && [...own(s), ...BUILT].find((x) => x.f && x.n === f.n);
    const tip = it ? `<span class="mono">${sig(it, f.k)}</span>${it.d ? ' ' + esc(it.d) : ''}` : '';
    if (!touch && CUR !== me) return;
    box.hidden = !items.length && !tip;
    ta.setAttribute('aria-expanded', !!items.length);
    ta.setAttribute('aria-describedby', tip ? `${desc} ${box.id}` : desc);
    if (items.length) ta.setAttribute('aria-activedescendant', `${box.id}-${sel}`); else ta.removeAttribute('aria-activedescendant');
    if (box.hidden) return;
    box.setAttribute('role', items.length ? 'listbox' : 'tooltip');
    if (touch) {
      clear(box, items.map((x, i) => h('button.sm.mono', { id: `${box.id}-${i}`, role: 'option', 'aria-selected': i === sel, onpointerdown: (e) => e.preventDefault(), onclick: () => accept(i) }, x.n)));
      if (tip) box.append(h('span.cap', { innerHTML: tip }));
      return;
    }
    box.innerHTML = items.length ? items.map((x, i) => `<div role="option" id="${box.id}-${i}" aria-selected="${i === sel}" data-i="${i}"><span class="mono">${esc(x.sig)}</span></div>`).join('') + (items[sel].d ? `<p class="cap">${esc(items[sel].d)}</p>` : '') : `<p class="cap">${tip}</p>`;
    place(box);
  }
  function accept(i) {
    const x = items[i], c = ta.selectionStart, p = word, f = x.f && ta.value[c] !== '(';
    items = [];
    if (document.activeElement !== ta) ta.focus({ preventScroll: true });
    o.put({ s: c - p.length, e: c }, x.n + (f ? '()' : ''));
    if (f) ta.setSelectionRange(ta.selectionStart - 1, ta.selectionStart - 1);
    at = -1;
    show();
  }
  // On typing (`t`): the names the word at the caret starts (yours first,
  // then the shortest; one typed in full first); a caret moved: the list
  // closes, the call's values stay.
  function update(t) {
    const c = ta.selectionStart;
    CUR = me;
    if (document.activeElement !== ta) return close();
    if (t) {
      word = typed();
      // (`smooth` is a line of its own: offered only at a line's start.)
      const seen = new Set(), head = /(^|[\n;])[ \t]*$/.test(ta.value.slice(0, c - word.length));
      items = word.length < 2 ? [] : [...own(ta.value), ...BUILT.filter((x) => head || x.n != 'smooth')].filter((x) => x.n.startsWith(word) && !seen.has(x.n) && seen.add(x.n))
        .sort((a, b) => (b.n === word) - (a.n === word) || a.n.length - b.n.length).slice(0, 8);
      if (items.length === 1 && items[0].n === word) items = [];
      sel = 0;
      at = c;
    } else if (c !== at) items = [];
    show();
  }
  function close() {
    items = [];
    at = -1;
    if (touch || CUR === me) box.hidden = true;
    ta.setAttribute('aria-expanded', false);
    ta.setAttribute('aria-describedby', desc);
    ta.removeAttribute('aria-activedescendant');
  }
  const me = {
    update, close, show, pick: accept,
    // A key while the list is open: ↑ ↓ choose, Tab or Enter put it in (Enter
    // goes on as a new line when the name is typed in full), Esc closes it.
    // True when it was the list's.
    key(e) {
      if (!items.length || e.altKey || e.ctrlKey || e.metaKey || e.isComposing || e.keyCode === 229) return false;
      const k = e.key, n = items.length;
      if (k === 'ArrowDown' || k === 'ArrowUp') sel = (sel + (k === 'ArrowDown' ? 1 : n - 1)) % n;
      else if (k === 'Escape') close();
      else if (k === 'Tab' && !e.shiftKey || k === 'Enter' && !e.shiftKey && items[sel].n !== word) { e.preventDefault(); accept(sel); return true; }
      else return false;
      e.preventDefault();
      show();
      return true;
    },
  };
  return me;
}

// The cheat sheet: a tap on a name puts it in at the caret. `root`: the
// site's (its guide's tour is linked at the foot).
export function cheat(btn, ta, root) {
  const r = btn.getBoundingClientRect();
  const ins = (t) => () => { closeMenu(); const s = ta.selectionStart; ta.focus(); ta.setSelectionRange(s, ta.selectionEnd); document.execCommand('insertText', false, t); };
  const rows = (title, list) => [h('h3', title), h('dl.psi-dl', list.map(([k, v, x]) => [h('dt', h('button.txt.mono', { onclick: ins(x || k.split(/[ ,(]/)[0]) }, k)), h('dd', v)]))];
  popup(h('div.menu.wide', { id: 'psi-cheat', role: 'dialog', 'aria-label': 'Cheat sheet' }, h('div.mh', 'Cheat sheet'),
    rows('At the top: what the formula is', DECLS), rows('Values', VARS), rows('Functions', FUNCS), rows('Drawing', DRAW), rows('Colour (the last line, of the formula or a layer)', COLS),
    h('p.cap', 'Phases are in turns: sin(0.25) is 1. Tap a name to put it in. -- starts a comment. New to Formulas? ',
      h('a', { href: root + 'guide/psi-1.html', target: '_blank', rel: 'noopener' }, 'Learn Psi'))), r.left, r.bottom + 4);
}
