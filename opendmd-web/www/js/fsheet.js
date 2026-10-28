// A Formula's settings (the editor's Settings sheet; imported when it
// first opens, its own size line): a form over its declaration lines —
// `name`, `about`, up to three `control`s, `look` — where every change is
// an edit of that line (pkg-psi `declare` and `look`: put in if missing,
// taken out when back to nothing), so the text stays the one record and a
// user who reads it sees the sheet follow. The look's part (the switch,
// Blend and Amount, the panel's word, Set as the panel's look) is
// js/looks.js'.

import { h, clear, toast } from './util.js';
import { popup, closeMenu } from './overrides.js';
import { controls as decl, control, declText } from './psikit.js';

const text = declText;
// A name in the formula (a control's, a choice's words).
const word = (v) => v.trim().replace(/\W+/g, '_').replace(/^(\d)/, '_$1');
const n = (v, d) => (Number.isFinite(+v) && v !== '' ? String(+v) : d);

// `x`: {P (the editor's pkg-psi), get() (the text), put(text) (into the
// editor, compiled), f (the values), moved() (a value changed), preview(src
// | null) (the look over the picture, or the formula again), look (a
// player or the demo: Set as the panel's look)}.
export async function sheet(x, btn) {
  let L, M;
  try { L = await import('./looks.js'); M = await L.psi(); } catch (e) { toast('Settings: ' + (e.message || e), 'bad'); return; }
  const P = x.P, d = () => decl(P, x.get());
  const line = (w, k, l) => x.put(P.declare(x.get(), w, k, l));
  const field = (id, label, max, w) => {
    const i = h('input', { type: 'text', id, maxLength: max, placeholder: w == 'name' ? 'Untitled formula' : 'One line about it' });
    i.oninput = () => { const t = text(i.value, max); if (w == 'name') x.f.name = t; line(w, 0, t ? `${w} "${t}"` : ''); };
    return [h('label.stack', label, i), i];
  };
  const [nameL, name] = field('psi-set-name', 'Name', 60, 'name'), [aboutL, about] = field('psi-set-about', 'About', 120, 'about');
  const ctls = h('div', { id: 'psi-set-ctls' }), add = h('button.sm', { id: 'psi-set-add', onclick: () => { line('control', d().own, `control k${d().own + 1} 0..1 = 0.5`); rows(); } }, '+ Add a control');
  const sw = h('input', { type: 'checkbox', id: 'psi-set-look' });
  const lc = L.controls('psi-set-lk', (b, k, done) => { const g = { ...vals(), src: L.set(M, vals(), b, k).src }; if (done) x.put(g.src); show(g); });
  const box = h('div', { id: 'psi-set-mix' }, lc.el, h('p.cap', 'Blend: how it goes over the GIF. Amount: how much of it shows.'));
  const use = x.look && h('button.pri', { id: 'psi-set-use' }, 'Set as the panel’s look');
  const err = h('p.cap.bad', { id: 'psi-set-err', hidden: true });
  const vals = () => ({ src: x.get(), a: x.f.a, b: x.f.b, c: x.f.c, name: d()?.name || x.f.name });
  sw.onchange = () => { x.put(L.set(M, vals(), ...(sw.checked ? L.choice(M, vals()) : ['', 0])).src); look(); };
  if (use) use.onclick = () => { closeMenu(); L.useSaid(vals()); };
  // The look's part as the text has it now.
  function look() {
    const c = d(), on = !!c?.look;
    sw.checked = on; box.hidden = !on;
    if (on) lc.set(c.look[0], c.look[1], L.overOnly(M, vals()));
    show(vals());
  }
  function show(f) {
    const on = !!decl(M, f.src)?.look, a = on && L.as(M, f);
    lc.say(a || null);
    if (use) use.disabled = !on || !!a.why;
    x.preview(on && a.src ? a.src : null);
  }
  // A row for each of the formula's own controls: as it shows, then its line's parts.
  function rows() {
    const c = d();
    err.hidden = !!c;
    err.textContent = c ? '' : 'Fix the formula first: its settings show once it works';
    add.disabled = !c || c.own >= 3;
    clear(ctls, (c?.controls || []).slice(0, c?.own).map((k, j) => {
      const ch = k.kind == 'choice', inp = (id, v, ph, w = '72px') => h('input', { type: 'text', id: `psi-set-c${j}-${id}`, value: v, placeholder: ph, style: `width:${w}`, 'aria-label': ph });
      const nm = inp('name', k.name, 'name', '96px'), lb = inp('label', k.label == k.name ? '' : k.label, 'label', '120px');
      const kind = h('select', { id: `psi-set-c${j}-kind`, 'aria-label': 'kind' }, h('option', { value: 's' }, 'Slider'), h('option', { value: 'c', selected: ch }, 'Choice'));
      const lo = inp('lo', ch ? 0 : k.lo, 'from'), hi = inp('hi', ch ? 1 : k.hi, 'to'), df = inp('def', ch ? 0.5 : k.default, 'default');
      const ws = inp('words', ch ? k.words.join(', ') : 'on, off', 'words, with commas', '180px');
      const write = () => {
        const l = text(lb.value, 24), w = ws.value.split(',').map(word).filter(Boolean).join(', ');
        line('control', j, `control ${word(nm.value) || 'k'}${kind.value == 'c' ? ` = ${w}` : ` ${n(lo.value, 0)}..${n(hi.value, 1)} = ${n(df.value, 0)}`}${l && l != nm.value.trim() ? ` "${l}"` : ''}`);
      };
      for (const i of [nm, lb, lo, hi, df, ws]) i.onchange = write;
      kind.onchange = () => { write(); rows(); };
      return h('div.psi-set-c', { id: `psi-set-c${j}` }, control(k, x.f, x.moved, `psi-set-c${j}-v`),
        h('div.row', nm, kind, ch ? ws : [lo, '..', hi, '=', df], lb, h('button.sm', { id: `psi-set-c${j}-del`, 'aria-label': 'Remove', onclick: () => { line('control', j, ''); rows(); } }, '✕')));
    }));
  }
  const el = h('div.menu.wide', { id: 'psi-set', role: 'dialog', 'aria-label': 'Formula settings' }, h('div.mh', 'Formula settings'), err, nameL, aboutL,
    h('h3', 'Controls'), h('p.cap', 'Sliders and buttons the Formula reads by name (three at most).'), ctls, h('div.row', add),
    h('h3', 'Look'), h('label', sw, 'Can be the panel’s look: drawn over every GIF'), box, lc.why, use && h('div.row', use));
  el.onclose = () => x.preview(null);
  const c = d();
  name.value = c?.name ?? x.f.name ?? '';
  about.value = c?.about ?? '';
  rows();
  look();
  const r = btn.getBoundingClientRect();
  popup(el, r.left, r.bottom + 4);
}
