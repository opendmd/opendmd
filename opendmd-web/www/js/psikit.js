// Formulas' shared parts, one copy for the web app's Formulas view
// (js/formulas.js) and the site's share page (/f/, which imports this file
// from the demo): the share link's encoder and decoder, the controls a
// formula declares (read from the compiler: pkg-psi's controls()), what a
// formula is in a sentence, the code's highlighting and the sample
// picture. No imports; the DOM only in `picker` and `control`.

export const MAX_SRC = 4096, MAX_NAME = 60;

// A share link: `<base>#formula=<src>&name=<name>&a=…` (URLSearchParams),
// with the values set that differ from their control's default (`ctl`:
// controls(); a slot left out plays at the default) of the controls the
// formula declares or reads (without `ctl`: every value set).
export function link(f, base, ctl) {
  const q = new URLSearchParams({ formula: f.src });
  if (f.name) q.set('name', f.name);
  for (const k of 'abc') if (f[k] != null && moved(ctl, f, k)) q.set(k, +(+f[k]).toFixed(2));
  return base + '#' + q;
}
// Whether `f[k]` is worth sending: its slot is read and the value isn't the
// declared default (to 4 places, as a path writes it). Without `ctl`, yes.
const moved = (ctl, f, k) => {
  if (!ctl) return true;
  const c = ctl.controls.find((c) => c.slot == k);
  return !!c && +(+f[k]).toFixed(4) !== +(+c.default).toFixed(4);
};
// And back: `{f, cut, has}` — the source and the name cut to what a
// formula can have (`cut` says the source was), the values as numbers
// (`fit` holds them to the formula's ranges), `has` whether there was a
// formula at all.
export function parse(s) {
  const q = new URLSearchParams(s.replace(/^#/, '')), src = q.get('formula') || '', name = q.get('name') || 'Shared formula';
  const f = { name: name.slice(0, MAX_NAME), src: src.slice(0, MAX_SRC) };
  for (const k of 'abc') if (num(q.get(k)) != null) f[k] = num(q.get(k));
  return { f, cut: f.src !== src, has: q.has('formula') };
}
// Text for a declaration's quotes (`name`, `about`, a label): without `" <
// > \`, control characters or ones that hide or reorder text (the compiler
// refuses them), cut to `n` characters.
export const declText = (v, n) => Array.from(v.replace(/["<>\\\u0000-\u001f\u007f-\u009f\u200b-\u200f\u2028-\u202e\u2060-\u2069\ufeff]/g, '')).slice(0, n).join('').trim();
// A slider's value from a link or a path (`a=-0.5`), as the panel reads one
// (fx_psi `query`): a decimal number, maybe below zero, under 32,768; else
// undefined — unset, so the formula's default.
export const num = (v) => (/^-?(\d+\.?\d*|\.\d+)$/.test(v || '') && Math.abs(+v) < 32768 ? +v : undefined);

// The device's path text (opendmd_psi::compact, which psi-web's tests hold this
// to): comments out, runs of spaces made one, lines joined with `; `; a
// character in quotes ('"') one token; text in quotes on a declaration line
// (`name "…"`, not `name = …`) kept as written.
export function compact(src) {
  let out = '';
  for (const l of src.split('\n')) {
    const text = /^[^\w]*(psi|name|about|control|look)\b(?![ \t\f\r\v]*=)/.test(l);
    let code = '', sp = false;
    for (let i = 0; i < l.length;) {
      const c = l[i];
      let t = c;
      if (c == "'" && l[i + 2] == "'" && l[i + 1] >= ' ' && l[i + 1] <= '~') t = l.slice(i, i + 3);
      else if (c == '"' && text) { const e = l.indexOf('"', i + 1); t = l.slice(i, e < 0 ? l.length : e + 1); }
      else if ((c == '-' || c == '/') && l[i + 1] == c) break;
      else if (' \t\f\r\v'.includes(c)) { sp = true; i++; continue; }
      else t = String.fromCodePoint(l.codePointAt(i));
      if (sp && code) code += ' ';
      sp = false;
      code += t;
      i += t.length;
    }
    if (code) out += (out ? '; ' : '') + code;
  }
  return out;
}

// The panel's path for a formula (fx_psi's `@fx:psi?src=…`; what Show on
// panel, a playlist line and the look send): compact(src) lightly encoded
// (a space is `+`; `% & + # ?`, controls and non-ASCII as %XX), then the
// values worth sending (`moved`). Over the panel's 2,048 B its `about` line
// is left out first (psi-web's tests hold this to the compiler's compact).
export function path(f, ctl) {
  let c = compact(f.src), p = enc(c);
  if (p.length > 2048) p = enc(noAbout(c));
  for (const k of 'abc') if (f[k] != null && moved(ctl, f, k)) p += `&${k}=${+(+f[k]).toFixed(4)}`;
  return p;
}
function enc(c) {
  let p = '@fx:psi?src=';
  for (const b of new TextEncoder().encode(c)) {
    const ch = String.fromCharCode(b);
    p += b == 32 ? '+' : b > 32 && b < 127 && !'%&+#?'.includes(ch) ? ch : '%' + (b < 16 ? '0' : '') + b.toString(16).toUpperCase();
  }
  return p;
}
// Compact text without its `about "…"` statement: the declarations come
// first, one statement each, their text in quotes holding no `"`.
function noAbout(c) {
  for (let i = 0; i < c.length;) {
    const w = /^\w*/.exec(c.slice(i))[0];
    if (!/^(psi|name|about|control|look)$/.test(w) || /^ ?=/.test(c.slice(i + w.length))) break;
    let j = i, q = false;
    while (j < c.length && (q || c.slice(j, j + 2) != '; ')) q = c[j++] == '"' ? !q : q;
    if (w == 'about') return c.slice(0, i) + c.slice(j + 2);
    i = j + 2;
  }
  return c;
}

// A slider's value for pkg-psi's frame(): Q16.16, or UNSET (the slot plays at
// the formula's declared default, as on the panel when a path has no `&a=`).
export const UNSET = -2147483648;
export const Q = (v) => (v == null ? UNSET : Math.round(v * 65536));

// What a formula declares (`name`, `about`, `look`, its controls: pkg-psi's
// controls(), the compiler's reading), or null when it doesn't compile. A
// slot read without a `control` line is a 0..1 slider named after it.
export function controls(P, src) {
  try { const j = P.controls(src); return j ? JSON.parse(j) : null; } catch { return null; }
}
// `f`'s values held to its controls' ranges (a choice's to its words), as the
// formula holds them; one read without a control line to 0..1, as before.
export function fit(ctl, f) {
  for (const c of ctl?.controls || []) {
    const v = f[c.slot];
    if (v == null) continue;
    f[c.slot] = c.kind == 'choice' ? Math.floor(Math.min(c.words.length - 1, Math.max(0, v))) : Math.min(c.hi, Math.max(c.lo, v));
  }
  return f;
}
// A control's row: its label and a slider (`lo` to `hi` in hundredths, or whole
// numbers when its ends and default are) or a picker, showing `f[slot]` (its
// default while unset); `set` after a change. `id` goes on the input.
export function control(c, f, set, id) {
  const k = c.slot, name = document.createElement('span');
  name.textContent = c.label;
  if (c.kind == 'choice') {
    const row = document.createElement('div');
    row.className = 'psi-sl';
    row.append(name, Object.assign(picker(c.words, c.label, f[k] ?? c.default, (v) => { f[k] = v; set(); }), { id }));
    return row;
  }
  const whole = [c.lo, c.hi, c.default].every(Number.isInteger) && c.hi - c.lo >= 2;
  const dp = whole ? 0 : 2, out = document.createElement('output'), lab = document.createElement('label');
  const r = Object.assign(document.createElement('input'), { type: 'range', min: c.lo, max: c.hi, step: whole ? 1 : (c.hi - c.lo) / 100, value: f[k] ?? c.default, id });
  out.className = 'mono';
  out.textContent = (+r.value).toFixed(dp);
  r.oninput = () => { f[k] = +r.value; out.textContent = f[k].toFixed(dp); set(); };
  lab.className = 'psi-sl';
  lab.append(name, r, out);
  return lab;
}
// A picker: a button for each choice (a radio group: arrows, Home, End). The
// value is the choice's number, 0 for the first (what `pick(mode, …)` reads).
export function picker(names, label, v, set) {
  const n = names.length - 1, g = document.createElement('div');
  g.className = 'psi-pick';
  g.role = 'radiogroup';
  g.ariaLabel = label;
  const bs = names.map((t, j) => Object.assign(document.createElement('button'), { type: 'button', role: 'radio', textContent: t, onclick: () => to(j) }));
  const mark = (k) => bs.forEach((b, j) => { b.ariaChecked = j == k; b.tabIndex = j == k ? 0 : -1; });
  const to = (j) => { mark(j); bs[j].focus(); set(j); };
  g.onkeydown = (e) => {
    const k = bs.indexOf(document.activeElement), d = { ArrowRight: 1, ArrowDown: 1, ArrowLeft: -1, ArrowUp: -1 }[e.key];
    const j = d ? k + d : e.key == 'Home' ? 0 : e.key == 'End' ? n : -1;
    if (j >= 0 && j <= n && k >= 0) { e.preventDefault(); to(j); }
  };
  // A value between choices (a link) snaps to the one the formula reads: floor.
  const k0 = Math.floor(Math.min(n, Math.max(0, v || 0)));
  mark(k0);
  if (v != null && v !== k0) set(k0);
  g.append(...bs);
  return g;
}
// What it is, from info(): its loop, the clock, its cost on the panel.
const cost = (us) => (us <= 5000 ? 'light' : us <= 8000 ? 'medium' : 'heavy');
// Its µs a frame on the panel: with its still cache (i[11]) unless it draws over a GIF (a look has none).
export const est = (i) => (i[5] & 32 || i[11] == null ? i[6] : i[11]);
export const about = (i) => i.length ? [i[5] & 8 && (i[4] ? `Loops every ${i[4] / 1000} s` : 'Goes on without repeating'), i[5] & 8 && !(i[5] & 32) && i[10] && i[10] != 20 && `${1000 / i[10]} frames a second`, i[5] & 16 && 'Follows the clock', !(i[5] & 24) && 'Still', i[8] && `Draws ${i[9]} lines and dots a frame`, `${cost(est(i))} for the panel`].filter(Boolean).join(' · ') : '';
export const TIP = (i) => `${i[0]} steps, about ${(est(i) / 1000).toFixed(1)} ms a frame on the panel (light: under 5 ms; medium: under 8)`;

// The code as highlighted HTML (numbers `.n` — a unary minus included —,
// comments `.c`, colours `.k` with `--k` their colour, for a swatch, the
// block words and the draw stage's `.w`; the
// error `bad` = [start, end] `.e`; everything else escaped text) and where
// its numbers are.
// (`line`, `dot` and `ink` only in a formula with a draw block: before it
// they were names a formula could set, and may still be; `fps` only before a
// number, as `fps 100`; `forever` only after `loop`. A sprite block's rows
// are its picture, `.p`, never numbers to drag.)
// psi-builtins: highlighter (generated from opendmd-psi/src/builtins.rs: PSI_BLESS=1 cargo test --features docs --test builtins)
const WORDS = /^(loop|frame|layer|part|draw|for|table|line|dot|ink)$/, DRAWS = /^(line|dot|ink)$/, DECLS = /^(psi|name|about|control|look)$/;
// psi-builtins end
export function marks(s, bad) {
  const draws = /^\s*draw\s*\{/m.test(s.replace(/(--|\/\/).*/g, ''));
  const toks = [], cls = new Array(s.length).fill(''), pic = [];
  for (const b of s.matchAll(/^([ \t]*sprite[ \t]+\w+[ \t]*\{[^\n]*\n)([\s\S]*?)^[ \t]*\}/gm)) {
    let at = b.index + b[1].length;
    for (const line of b[2].split('\n')) {
      const r = line.replace(/(--|\/\/).*/, '').trim();
      if (r && !/^key\s/.test(r)) { const a = at + line.indexOf(r); pic.push([a, a + r.length]); }
      at += line.length + 1;
    }
  }
  const inPic = (i) => pic.some(([a, e]) => i >= a && i < e);
  for (const m of s.matchAll(/(--|\/\/).*|"[^"\n]*"?|'[ -~]'|#[\da-f]{3}([\da-f]{3})?\b|[a-z_]\w*|\d+(?:\.(?!\.)\d*)?|(?<!\.)\.\d+/gi)) {
    const k = m[1] ? 'c' : m[0][0] == '"' ? 's' : m[0][0] == '#' ? `k" style="--k:${m[0]}` : /[\d.']/.test(m[0][0]) ? 'n' : (WORDS.test(m[0]) && (draws || !DRAWS.test(m[0])) || DECLS.test(m[0]) && /(^|;)[ \t]*$/.test(s.slice(0, m.index).split('\n').pop()) && !/^[ \t]*[=(.[]/.test(s.slice(m.index + m[0].length)) || m[0] == 'fps' && /^[ \t]+\d/.test(s.slice(m.index + 3)) || m[0] == 'forever' && /\bloop[ \t]+$/.test(s.slice(0, m.index)) || m[0] == 'sprite' && /^[ \t]+\w+[ \t]*\{/.test(s.slice(m.index + 6)) || m[0] == 'palette' && /^[ \t]+\w+[ \t]*=/.test(s.slice(m.index + 7))) ? 'w' : '';
    let a = m.index;
    if (inPic(a)) continue;
    // A character in quotes ('A') is a number, but not one to drag.
    if (k === 'n' && m[0][0] !== "'") {
      // `-2` after an operator, a bracket or the start of a line is one number.
      if (s[a - 1] === '-' && /[-(,=*/+?:<>!%;\n]|^$/.test(s.slice(0, a - 1).trimEnd().slice(-1))) a--;
      toks.push({ s: a, e: m.index + m[0].length });
    }
    if (k) cls.fill(k, a, m.index + m[0].length);
  }
  for (const [a, e] of pic) cls.fill('p', a, e);
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

