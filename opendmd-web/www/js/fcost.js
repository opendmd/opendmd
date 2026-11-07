// What costs most and Tidy up: the site's Formula editor only (/f/ and the
// demo: js/formulas.js imports this when its PAGE or app.demo is set; the
// pack leaves it out, opendmd-pack www.rs SITE_ONLY). What costs most (by
// itself once a Formula is past light, else its button): a bar in the gutter
// beside each line for its share of the panel's work, and the compiler's
// notes as sentences over the code, each with a Line N button to go there.
// Tidy up lays the text out as `psi fmt` does, as one edit (one undo). The
// wasm (pkg-psi-lang: observe(), format(); the whole compiler, ~121 KB gz)
// loads only for one of these.
import { h, clear, toast, debounce } from './util.js';
import { load } from './wasm.js';

let L = null;
const lang = () => (L ??= load('pkg-psi-lang').then(([m]) => m).catch((e) => { L = null; throw e; }));
// µs a frame on the panel (psikit est()): under it, light, and nothing said about the cost.
const LIGHT = 5000;
// Notes about what went well (kept, a column at a time, lanes) are not news here.
// (A column run left out is not: said in its own words, below.)
const WELL = /^(still cache:|column run: (?!left out)|this loop of \d+ turns runs as lanes)/;
const LOST = { 'it is over 255 steps': 'there are too many of them', 'it holds more values at once than the registers': 'they would hold more values at once than the panel can',
  'its marker does not fit the program': 'the program has no room left for it', "the scenes' steps could not be laid out around it": 'the scenes’ steps can’t be laid out around them' };
const COST = /for every pixel|per pixel/;
const plain = (t) => t.replace(/^column run: left out \((.+?)\): .*/, (_, w) => `Some values here change only from column to column, but they are worked out for every pixel, not once a column: ${LOST[w] || w}`).replace('it would be kept in the still cache but', 'it could be worked out once and kept, but')
  .replace(/ \([^()]*psi check[^()]*\)/, '').replace(/^./, (c) => c.toUpperCase());
const said = (t) => plain(t).split('`').map((x, k) => (k % 2 ? h('code', x) : x));
const CSS = '.psi-gut{position:relative;z-index:0}.fc-bar{position:absolute;left:0;height:1.5em;background:var(--acc);opacity:.4;border-radius:0 2px 2px 0;z-index:-1}'
  + '.fc-notes{margin:0 0 8px;font-size:14px;display:flex;flex-direction:column;gap:4px}.fc-notes p{margin:0}.fc-notes code{color:var(--fg)}'
  + '.fc-notes button{min-height:28px;padding:2px 6px;margin-right:6px;color:var(--acc)}';

// The editor's parts: the textarea, its gutter, the code's block (the notes
// go at its top, under the error), the Cheat sheet button (Tidy up goes
// beside it), and its put() (an edit the browser's undo keeps).
export function cost({ ta, gut, code, help, put }) {
  if (!document.getElementById('fc-css')) document.head.append(h('style', { id: 'fc-css' }, CSS));
  const notes = h('div.cap.fc-notes', { id: 'psi-notes', hidden: true });
  const tidy = h('button.sm', { id: 'psi-tidy', hidden: true, title: 'Lay the code out the usual way: spaces and indents only', onclick: () => tidyUp() }, 'Tidy up');
  // On demand for a light one (a heavy one shows it by itself), until pressed again.
  const ask = h('button.sm', { id: 'psi-cost', hidden: true, 'aria-pressed': 'false', title: 'Which lines take the panel the most work', onclick: () => { asked = !asked; ask.setAttribute('aria-pressed', asked); if (asked) look(); else show(null); } }, 'What costs most');
  code.prepend(notes);
  help.after(ask, tidy);
  let src = null, us = 0, bars = [], lines = 0, asked = false;
  const go = (k) => {
    const s = ta.value.split('\n').slice(0, k).reduce((n, l) => n + l.length + 1, 0);
    ta.focus();
    ta.setSelectionRange(s, s);
  };
  const line = (k) => h('button.sm', { onclick: () => go(k) }, `Line ${k + 1}`);
  function show(o) {
    for (const b of gut.querySelectorAll('.fc-bar')) b.remove();
    lines = ta.value.split('\n').length;
    const heavy = !!o && (us > LIGHT || asked), top = o?.lines[0]?.[1] || 1;
    bars = heavy ? o.lines.filter((l) => l[1] >= 10).map(([k, p]) => h('i.fc-bar', { style: `top:calc(8px + ${k * 1.5}em);width:${Math.max(6, Math.round((p * 100) / top))}%` })) : [];
    gut.append(...bars);
    const most = heavy ? o.lines.slice(0, 3).filter((l) => l[1] >= 100) : [];
    const ns = (o?.notes || []).filter(([, t]) => !WELL.test(t) && (heavy || !COST.test(t)));
    // Light (shown on demand): plainly so, and nothing to change unless a note says otherwise.
    const tips = ns.some(([, t]) => COST.test(t));
    clear(notes, heavy && us <= LIGHT && h('p', tips ? 'Light for the panel.' : 'Light for the panel: nothing here needs to change.'),
      most.length > 0 && h('p', 'What costs most: ', most.map(([k, p]) => [line(k), `${Math.round(p / 10)}% `])), ns.map(([k, t]) => h('p', /^column run: left out/.test(t) ? null : line(k), said(t))));
    notes.hidden = !notes.firstChild;
  }
  // After the editor's own compile, a pause: never in the way of typing.
  const look = debounce(() => {
    const s = src;
    if (s == null) return;
    lang().then((m) => {
      if (s !== src) return;
      const t0 = performance.now(), j = m.observe(s);
      notes.dataset.ms = (performance.now() - t0).toFixed(1);
      show(j ? JSON.parse(j) : null);
    }, () => {});
  }, 300);
  async function tidyUp() {
    const m = await lang().catch(() => null);
    if (!m) return toast('Couldn’t load Tidy up: try again', 'bad');
    const old = ta.value, out = m.format(old);
    if (!out) return toast('Fix the error first, then tidy up', 'bad');
    if (out === old) return toast('Already tidy');
    // The one range that changed: one edit, one undo.
    let p = 0, q = 0;
    while (p < old.length && old[p] === out[p]) p++;
    while (q < old.length - p && q < out.length - p && old[old.length - 1 - q] === out[out.length - 1 - q]) q++;
    const c = ta.selectionStart, e = old.length - q;
    ta.focus();
    put({ s: p, e }, out.slice(p, out.length - q));
    const at = c <= p ? c : c >= e ? c + out.length - old.length : out.length - q;
    ta.setSelectionRange(at, at);
  }
  return {
    // A compile: the text and its estimate (µs a frame), or null (an error).
    seen(s, e) {
      src = s;
      us = e || 0;
      tidy.hidden = s == null;
      ask.hidden = s == null || us > LIGHT;
      if (s != null && (us > LIGHT || asked || L)) look();
      else show(null);
    },
    // The gutter drawn again (n lines): its bars, while the lines are where they were.
    paint(n) { if (n === lines) gut.append(...bars); else bars = []; },
  };
}
