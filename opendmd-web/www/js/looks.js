// Any Formula as the panel's look (Settings › Look, and Formulas' "Use as
// the look"; imported by both, its own size line): a formula drawn over
// every GIF with a blend, at an amount. pkg-psi (an instance of its own,
// `?look`) writes the text the panel is sent — opendmd_psi::look: the
// formula pasted in as a part, a layer with the blend; over at 100 % is
// the formula itself — and says what it costs. The config keeps that path
// with the formula's name, blend and amount — look_formula_name `Plasma ·
// Screen 40%` (the name alone for Over 100%, as before) — so Settings
// shows them again; the player stores the name as display text.

import { app } from './state.js';
import { h } from './util.js';
import { fpath } from './psishow.js';

export { fpath };

export const BLENDS = [['over', 'Over'], ['screen', 'Screen'], ['add', 'Add'], ['multiply', 'Multiply'], ['max', 'Lighten'], ['min', 'Darken']];
let mod = null;
export const psi = () => (mod ??= import('../pkg-psi/opendmd_psi_web.js?look').then(async (m) => { await m.default(); return m; }).catch((e) => { mod = null; throw e; }));

// `f` as a look: {src, i} (the text and what it is), or {why} — the
// panel's own refusals (fx_psi: it hides the GIF; over its 8 ms).
export function as(m, f, blend, amt) {
  const src = f?.src && m.look(f.src, blend, amt), i = src && m.info();
  return !src ? { why: f?.src ? m.error() : 'pick a Formula' }
    : !(i[5] & 32) ? { why: 'this Formula covers the GIF entirely: choose a lower amount or another blend' }
      : i[6] > 8000 ? { why: `this Formula is too heavy to draw over every GIF (about ${Math.ceil(i[6] / 1000)} ms a frame; the limit is 8 ms)` }
        : { src, i };
}
// One with layers of its own goes over only.
export const overOnly = (m, f) => !m.look(f.src, 'screen', 40) && /only go over/.test(m.error());
// Where a formula starts: as it is when it draws over the GIF already;
// else screened at 40 % (the GIF's bright parts stay, the formula shows in
// the dark ones), or over at 30 % when that can't be (too heavy, layers).
export function choice(m, f) {
  if (m.compile(f.src) && m.info()[5] & 32) return ['over', 100];
  return as(m, f, 'screen', 40).why ? ['over', 30] : ['screen', 40];
}

// The saved name, blend and amount, from look_formula_name.
const SUFFIX = / · (\w+) (\d{1,3})%$/;
export function named(cf) {
  const n = cf.look_formula_name || '', m = SUFFIX.exec(n), b = m && BLENDS.find((x) => x[1] == m[1]);
  return b && m[2] <= 100 ? [n.slice(0, m.index), b[0], +m[2]] : [n, 'over', 100];
}
// …and back: the name, cut to fit 60 characters with its blend and amount.
const label = (name, blend, amt) => {
  const s = ` · ${BLENDS.find((x) => x[0] == blend)[1]} ${amt}%`, n = Array.from(name || 'Formula');
  return blend == 'over' && amt == 100 && !SUFFIX.test(name) ? n.slice(0, 60).join('') : n.slice(0, 60 - s.length).join('') + s;
};

// Kept as the panel's look: the config, then read again.
export async function use(f, blend, amt) {
  const x = as(await psi(), f, blend, amt);
  if (x.why) throw new Error(x.why);
  await app.src.putConfig({ look_formula: fpath({ ...f, src: x.src }), look_formula_name: label(f.name, blend, amt) });
  await app.refreshConfig();
}

// The saved look's formula (its path's text — opendmd_psi::look's, lines
// joined by `; ` — taken apart again) and sliders: for one not found among
// this browser's Formulas.
const inner = (s) => (/^(?:loop [\d.]+; )?part look \{; (.*); \}; (?:layer \w+ \{ )?look(?:\(x, y, [\d.]+\))?(?: \})?$/.exec(s) || [])[1];
export function saved(cf) {
  const q = new URLSearchParams(cf.look_formula.slice(8)), src = q.get('src') || '', [name, b, k] = named(cf), f = { name: name || 'A formula', saved: 1 };
  f.src = b == 'over' && k == 100 ? src : inner(src) || src;
  for (const k of 'abc') if (q.has(k)) f[k] = +q.get(k);
  return f;
}

// The two controls, `id`-blend and `id`-amt; `change(blend, amount, settled)`
// (settled: not mid-drag). set(blend, amount, overOnly).
export function controls(id, change) {
  const sel = h('select', { id: id + '-blend' }, BLENDS.map(([v, l]) => h('option', { value: v }, l)));
  const r = h('input', { type: 'range', min: 0, max: 100, step: 5, id: id + '-amt', 'aria-label': 'Amount' }), out = h('output.mono');
  const why = h('p.cap.bad', { id: id + '-why', hidden: true });
  const tell = (e) => { out.textContent = r.value + '%'; change(sel.value, +r.value, e.type != 'input'); };
  sel.onchange = r.oninput = r.onchange = tell;
  return {
    el: h('div.form.f2', h('label.stack', 'Blend', sel), h('label.stack', 'Amount', h('span.bri', h('span.rng', r), out))), why,
    set(b, k, only) {
      sel.value = b; r.value = k; out.textContent = k + '%';
      for (const o of sel.options) o.disabled = only && o.value != 'over';
      sel.title = only ? 'This Formula has layers of its own: it goes over the GIF' : '';
    },
    say(t) { why.hidden = !t; why.textContent = t || ''; },
  };
}
