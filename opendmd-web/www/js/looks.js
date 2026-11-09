// A Formula as the panel's look (Settings › Panel; the Formula settings
// sheet, js/fsheet.js; imported by both, its own size line): a Formula
// that declares `look` (its `look [blend] [amount]` line: how it goes over
// every GIF). The line is the one record of the blend and amount: the
// sheet writes it (pkg-psi `look`, a text edit), Settings only picks which
// Formula, and the config keeps the Formula's path (look_formula), which
// the player draws as its line asks (fx_psi::FormulaLook). pkg-psi (an
// instance of its own, `?look`) composes the look as the player does
// (`asLook`) and says what it costs.

import { app } from './state.js';
import { h, toast } from './util.js';
import { path as fpath, controls as ctlOf, num, compiler, hasCompiler } from './psikit.js';
import { line, onCard, stemTitle } from './fstore.js';
import { load } from './wasm.js';

export const BLENDS = [['over', 'Over'], ['screen', 'Screen'], ['add', 'Add'], ['multiply', 'Multiply'], ['max', 'Lighten'], ['min', 'Darken']];
let mod = null;
export const psi = () => (mod ??= load('pkg-psi', 'look').then(([m]) => { if (!hasCompiler()) compiler(m); return m; }).catch((e) => { mod = null; throw e; }));

// What a formula declares (psikit `controls`).
export { ctlOf as decl };
// What the look setting names `f` by (js/fstore.js `line`: a bundled one by
// name, one of yours on the panel by its file, else its inline path; values
// at their defaults left out).
export const path = (m, f) => { const c = ctlOf(m, f.src); return line(f, c, fpath(f, c)); };
// Whether `f` declares `look` (one Settings › Panel lists).
export const declares = (m, f) => !!ctlOf(m, f?.src)?.look;
// Its name: its `name` line, else the one it was saved under.
export const named = (m, f) => ctlOf(m, f.src)?.name || f.name || 'Formula';

// `f` as the panel's look, as its `look` line says (none: as it is): {src,
// i} (the text the panel draws and what it is), or {why} — the panel's
// own refusals (an inline look over 2,048 B, a 413; fx_psi: it hides the
// GIF; over its 12 ms).
export function as(m, f) {
  const src = f?.src && m.asLook(f.src), i = src && m.info();
  return !src ? { why: f?.src ? m.error() : 'pick a Formula' }
    : /^@formula\?/.test(path(m, f)) && path(m, f).length > 2048 ? { why: 'this Formula is too long for the panel as a look: save it first' }
    : !(i[5] & 32) ? { why: 'this Formula covers the GIF entirely: choose a lower amount or another blend' }
      : i[6] > 12000 ? { why: `this Formula is too heavy to draw over every GIF (about ${Math.ceil(i[6] / 1000)} ms a frame; the limit is 12 ms)` }
        : { src, i };
}
// `f` with its look line set (blend '' takes it out).
export const set = (m, f, blend, amt) => ({ ...f, src: m.look(f.src, blend, amt) || f.src });
// One with layers of its own goes over only.
export const overOnly = (m, f) => !m.asLook(m.look(f.src, 'screen', 40)) && /goes over the GIF as a look/.test(m.error());
// Where a new look line starts: as it is when it draws over the GIF already;
// else screened at 40 % (the GIF's bright parts stay, the formula shows in
// the dark ones), or over at 30 % when that can't be (too heavy, layers).
export function choice(m, f) {
  if (m.compile(f.src) && m.info()[5] & 32) return ['over', 100];
  return as(m, set(m, f, 'screen', 40)).why ? ['over', 30] : ['screen', 40];
}

// Kept as the panel's look: the config, then read again. `keep` (the
// editor's): the Formula as saved, so the look names it on the panel.
export async function use(f, keep) {
  if (keep && onCard()) f = { ...f, ...await keep() };
  const m = await psi(), x = as(m, f);
  if (x.why) throw new Error(x.why);
  await app.src.putConfig({ look_formula: path(m, f) });
  await app.refreshConfig();
}

// The same, said in a toast (the Formulas view's Set as the panel's look).
export async function useSaid(f, keep) {
  try {
    await use(f, keep);
    toast(`“${f.name || 'Formula'}” is the panel’s look now: drawn over every GIF (Settings › Panel changes it)`, 'ok');
  } catch (e) { toast(e.status == 400 ? `The panel can’t use it: ${e.message.replace(/^400: (look_formula: )?/, '')}` : 'Set as the panel’s look: ' + e.message, 'bad'); }
}

// `f` as what the panel shows while it starts (boot_formula; the Formulas
// view's Use at start-up): one light enough for the show (12 ms a frame,
// the panel's limit), named as a look is; said in a toast.
export async function startUp(f, keep) {
  try {
    if (keep && onCard()) f = { ...f, ...await keep() };
    const m = await psi();
    if (!m.compile(f.src)) throw new Error(m.error() || 'it doesn’t work yet');
    if (m.info()[6] > 12000) throw new Error('this Formula is too heavy for the start-up show');
    const p = path(m, f);
    if (/^@formula\?/.test(p) && p.length > 2048) throw new Error('this Formula is too long for the panel at start-up: save it first');
    await app.src.putConfig({ boot_formula: p });
    await app.refreshConfig();
    toast(`“${f.name || 'Formula'}” plays when the panel starts now (Settings › Panel changes it)`, 'ok');
  } catch (e) { toast(e.status == 400 ? `The panel can’t use it at start-up: ${e.message.replace(/^400: (boot_formula: )?/, '')}` : 'Use at start-up: ' + e.message, 'bad'); }
}

// The look a path holds, as a formula (for one not found among this
// browser's Formulas): its text and values.
export function fromPath(m, p) {
  // A name or a file not among those listed: named from it.
  if (!p.startsWith('@formula?')) return { src: '', saved: 1, name: stemTitle(p.replace(/^@formula:|\?.*$/g, '').replace(/^.*\/|\.psi$/gi, '')) };
  // A compiled one sent from elsewhere: only its name is in it.
  const q = new URLSearchParams(p.slice(9)), b = unb64(q.get('psb') || '');
  return { src: '', saved: 1, name: (b && b[0] == 80 && new TextDecoder().decode(b.subarray(6, 6 + b[5]))) || q.get('name') || 'Formula' };
}

// The two controls, `id`-blend and `id`-amt; `change(blend, amount, settled)`
// (settled: not mid-drag). set(blend, amount, overOnly); say(as(…)): the
// refusal, or how fast the look moves on the panel (over 8 ms its frames
// are held twice as long: fx_psi's governor).
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
    say(x) {
      const t = x?.why || (x?.i?.[5] & 8 ? `On the panel it moves about ${(x.i[13] ?? x.i[6]) > 8000 ? '25 times a second (a heavy look: lighter ones move at 50)' : '50 times a second'}` : '');
      why.hidden = !t; why.textContent = t; why.classList.toggle('bad', !!x?.why);
    },
  };
}

// base64url (psikit `b64`) back to bytes, or null.
function unb64(s) {
  try { return Uint8Array.from(atob(s.replace(/-/g, '+').replace(/_/g, '/')), (c) => c.charCodeAt(0)); } catch { return null; }
}
