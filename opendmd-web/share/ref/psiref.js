// The Psi language reference's live parts (the site's /dev/psi/, from
// docs/psi-spec.md; tools/site.py copies this file beside the page):
// - the examples: every `code.psi-ex` in the generated tables (one a
//   cheat-sheet row, opendmd-psi/src/reference.rs) and every ```psi block
//   of the Cookbook, drawn as the panel draws them by the demo's Psi
//   (pkg-psi, through ../../psiled.js, as the home page's panel is), each
//   with an "Open in editor" link to the demo's editor.
//   Nothing loads until the first one scrolls near: then the wasm, once,
//   and only the examples on screen are drawn (one frame for a still one,
//   or for every one when the reader prefers less motion);
// - the code blocks highlighted as the editor highlights them (psikit.js);
// - a search box that filters the built-ins' rows and the error entries,
//   and a compact cheat-sheet view of both.
// Everything here is text the page already has: nothing is fetched but the
// demo's files.

import { marks, sample } from '../../demo/js/psikit.js';
import { psi, use, frameAt, panel, paint } from '../../psiled.js';

const main = document.querySelector('main');
const SITE = new URL('../../', location.href).href;
const still = matchMedia('(prefers-reduced-motion: reduce)').matches;
const el = (tag, props = {}, ...kids) => { const e = Object.assign(document.createElement(tag), props); e.append(...kids); return e; };

// --- the sections -------------------------------------------------------------

// Each error entry (its h4 and what follows, to the next heading) in a
// <section>, so it filters and folds as one.
const errs = [];
for (const h of main.querySelectorAll(':scope > h4[id^=E]')) {
  const box = el('section', { className: 'psi-err' });
  h.before(box);
  const part = [h];
  for (let n = h.nextElementSibling; n && !/^H[1-4]$/.test(n.tagName); n = n.nextElementSibling) part.push(n);
  box.append(...part);
  errs.push(box);
}
// main's children grouped under their h2: [h2, [elements…]] (the first
// group's head is the h1); worked out again for each search, as the live
// examples are added after.
const grouped = () => {
  const out = [];
  for (const c of main.children) {
    if (c.tagName === 'H2' || !out.length) out.push([c, []]);
    else out.at(-1)[1].push(c);
  }
  return out;
};
const section = (id) => grouped().find(([h]) => h.id === id) || [null, []];
const [, refEls] = section('what-each-does');
const [, errEls] = section('error-codes');
const [, cookEls] = section('cookbook');

// --- the code: highlighted ------------------------------------------------------

for (const c of main.querySelectorAll('pre code.language-psi')) {
  c.innerHTML = marks(c.textContent.replace(/\n$/, '')).html; // marks() escapes the text
  c.parentElement.classList.add('psi-hl');
}

// --- drawing: only what is on screen ----------------------------------------------

const near = new Set();
let L = null, loading = null, t0 = 0, raf = 0;
const seen = new IntersectionObserver((es) => {
  for (const e of es) {
    const x = examples.find((x) => x.box === e.target);
    if (e.isIntersecting) near.add(x); else near.delete(x);
  }
  if (near.size) start();
}, { rootMargin: '200px 0px' });

// --- the examples ---------------------------------------------------------------

// A code element's text, its <br>s as line ends.
const text = (code) => [...code.childNodes].map((n) => (n.nodeName === 'BR' ? '\n' : n.textContent)).join('');
const editor = (src, name) => SITE + 'demo/#' + new URLSearchParams(name ? { formula: src, name } : { formula: src });

const examples = [];
function live(src, name, at, big) {
  const x = panel('What this formula draws');
  const box = el('div', { className: 'psi-live' + (big ? ' big' : '') },
    x.led, el('a', { className: 'psi-open', href: editor(src, name) }, 'Open in editor'));
  at(box);
  Object.assign(x, { src, box, info: null, drawn: false });
  examples.push(x);
  seen.observe(box);
}

for (const code of main.querySelectorAll('code.psi-ex')) {
  const row = code.closest('tr');
  row?.closest('table')?.classList.add('psi-ex-t');
  const name = row && ('Example: ' + row.cells[0].textContent.trim()).slice(0, 60);
  live(text(code), name, (b) => code.after(b), false);
}

for (const pre of cookEls.filter((e) => e.tagName === 'PRE')) {
  const code = pre.querySelector('code.language-psi');
  if (code) live(code.textContent.replace(/\n$/, ''), null, (b) => pre.before(b), true);
}

function start() {
  if (!loading) {
    loading = psi().then((l) => {
      L = l;
      L.P.setSeed(20000);
      t0 = performance.now();
      main.dataset.psiLive = 'ready';
    }).catch((e) => { main.dataset.psiLive = 'failed'; console.warn('Psi did not load:', e); });
  }
  loading.then(() => { if (L && !raf) raf = requestAnimationFrame(tick); });
}

let last = 0;
function tick(now) {
  raf = 0;
  if (!near.size) return;
  // 25 frames a second is plenty for a reference page.
  if (now - last >= 40) {
    last = now;
    const d = new Date();
    for (const x of near) {
      if (x.drawn && (still || x.still)) continue;
      draw(x, now - t0, d);
    }
  }
  if ([...near].some((x) => !x.drawn || !(still || x.still))) raf = requestAnimationFrame(tick);
}

function draw(x, el_ms, d) {
  const { P } = L;
  if (!use(P, x.src)) { x.box.replaceChildren(el('p', { className: 'cap bad' }, P.error())); x.drawn = x.still = true; return; }
  if (!x.info) {
    x.info = P.info();
    // Neither the time nor the clock: one frame is all of it.
    x.still = !(x.info[5] & 24);
  }
  paint(L, x.info, x, frameAt(x.info, el_ms), x.info[5] & 32 ? sample() : null, d);
  x.drawn = true;
}

// --- search and the cheat sheet ----------------------------------------------------

const rows = refEls.flatMap((t) => (t.tagName === 'TABLE' ? [...t.tBodies[0].rows] : []));
const q = el('input', { type: 'search', id: 'psi-q', placeholder: 'Search built-ins and errors', autocomplete: 'off', spellcheck: false });
q.setAttribute('aria-label', 'Search the built-ins and the error codes');
q.setAttribute('aria-keyshortcuts', '/');
const cheat = el('button', { type: 'button', id: 'psi-cheat', className: 'btn' }, 'Cheat sheet');
cheat.setAttribute('aria-pressed', 'false');
const count = el('span', { id: 'psi-n', className: 'cap' });
count.setAttribute('aria-live', 'polite');
const bar = el('div', { className: 'psi-tools' }, q, cheat, count);
bar.setAttribute('role', 'search');
main.querySelector('h1').after(bar);

// The parts the cheat sheet keeps: the built-ins' tables and the error
// entries, with their headings.
const keep = new Set([...refEls, ...errEls].filter((e) => /^(H3|H4|TABLE)$/.test(e.tagName) || e.classList.contains('psi-err')));

function apply() {
  const words = q.value.trim().toLowerCase().split(/\s+/).filter(Boolean);
  const hit = (e) => { const t = e.textContent.toLowerCase(); return words.every((w) => t.includes(w)); };
  const on = words.length > 0, sheet = cheat.getAttribute('aria-pressed') === 'true', all = !on && !sheet;
  main.classList.toggle('cheat', sheet);
  let nr = 0, ne = 0;
  for (const r of rows) { const ok = !on || hit(r); r.hidden = !ok; nr += ok; }
  for (const s of errs) { const ok = !on || hit(s); s.hidden = !ok; ne += ok; }
  // Everything else: shown unless searching (or folded into the cheat
  // sheet); a heading with nothing left under it goes too.
  for (const [h, els] of grouped()) {
    let any = false, head = null, under = false;
    const close = () => { if (head) head.hidden = !(all || under); };
    for (const e of els) {
      if (e === bar) continue;
      if (/^H[34]$/.test(e.tagName)) { close(); head = e; under = false; continue; }
      let show;
      if (e.tagName === 'TABLE' && rows.includes(e.tBodies[0]?.rows[0])) show = [...e.tBodies[0].rows].some((r) => !r.hidden);
      else if (e.classList.contains('psi-err')) show = !e.hidden;
      else show = all || (!on && keep.has(e));
      e.hidden = !show;
      if (show) { any = true; under = true; }
    }
    close();
    if (h.tagName === 'H2') h.hidden = !(all || any);
  }
  count.textContent = on ? `${nr} built-in${nr == 1 ? '' : 's'}, ${ne} error${ne == 1 ? '' : 's'}` : '';
}
q.addEventListener('input', apply);
q.addEventListener('keydown', (e) => { if (e.key === 'Escape') { q.value = ''; apply(); } });
cheat.addEventListener('click', () => { cheat.setAttribute('aria-pressed', String(cheat.getAttribute('aria-pressed') !== 'true')); apply(); });
addEventListener('keydown', (e) => {
  if (e.key === '/' && !/^(INPUT|TEXTAREA|SELECT)$/.test(document.activeElement?.tagName) && !e.ctrlKey && !e.metaKey && !e.altKey) { e.preventDefault(); q.focus(); }
});
// In the cheat sheet an error's title opens its whole entry.
for (const s of errs) {
  s.querySelector('h4').addEventListener('click', () => {
    if (!main.classList.contains('cheat')) return;
    cheat.setAttribute('aria-pressed', 'false');
    q.value = '';
    apply();
    location.hash = s.querySelector('h4').id;
    s.scrollIntoView();
  });
}
// A link to an entry (#E203) shows it, whatever was filtered.
addEventListener('hashchange', () => {
  const t = document.getElementById(location.hash.slice(1));
  if (t && t.closest('[hidden]')) { q.value = ''; cheat.setAttribute('aria-pressed', 'false'); apply(); t.scrollIntoView(); }
});
main.dataset.psiRef = 'ready';
