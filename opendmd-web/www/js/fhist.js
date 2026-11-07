// Earlier versions of a Formula (the editor's Earlier versions…, imported
// when it opens or a version is kept): a ring of the last 10 per Formula in
// this browser (`odmd.psi.v.<key>`: newest first, {t, src, name, a, b, c,
// w}), kept on each save, as one arrives (the gallery, opendmd.dev) and,
// on the site's page, as it goes to the panel. `w` says why. Each lists
// what going back would change (a line diff) and goes back through the
// editor's own edit, so undo has it. The site's page keeps its draft here
// too (`odmd.psi.fd.<key>`: {t, src, name, a, b, c, base}, base the text
// the page loaded). Storage may be blocked or full: the oldest go first.

import { h, toast } from './util.js';
import { popup, closeMenu } from './overrides.js';

const N = 10, MAX = 32768, RINGS = 30, DRAFTS = 20, V = 'odmd.psi.v.', FD = 'odmd.psi.fd.';
const get = (k, d) => { try { return JSON.parse(localStorage.getItem(k)) ?? d; } catch { return d; } };
const keys = (p) => { try { return Object.keys(localStorage).filter((k) => k.startsWith(p)); } catch { return []; } };
// Written, else the oldest kept version anywhere dropped until it fits (never `k`'s newest).
function put(k, v) {
  for (let n = 0; n < 40; n++) {
    try { localStorage.setItem(k, JSON.stringify(v)); return true; } catch { /* full, or blocked */ }
    let o = null;
    for (const r of keys(V)) { const l = get(r, []); if (Array.isArray(l) && l.length > (r === k ? 1 : 0) && (!o || l.at(-1).t < o.t)) o = { r, t: l.at(-1).t, l }; }
    if (!o) return false;
    o.l.pop();
    try { o.l.length ? localStorage.setItem(o.r, JSON.stringify(o.l)) : localStorage.removeItem(o.r); } catch { return false; }
    if (o.r === k && Array.isArray(v) && v.length > 1) v = v.slice(0, -1);
  }
  return false;
}
// The oldest of more than `n` of `p…` (by `t` of what each holds) dropped.
function trim(p, n, t) {
  const ks = keys(p);
  if (ks.length <= n) return;
  for (const k of ks.map((k) => [k, t(get(k, null))]).sort((a, b) => a[1] - b[1]).slice(0, ks.length - n)) try { localStorage.removeItem(k[0]); } catch { /* blocked */ }
}

export const list = (key) => { const l = get(V + key, []); return Array.isArray(l) ? l.filter((v) => typeof v?.src == 'string') : []; };
// Kept: {src, name, a, b, c, t?}, `w` why.
export function keep(key, f, w) {
  const l = list(key), i = f?.src ? l.findIndex((v) => v.src === f.src) : 0;
  // (One as it arrives once; another the same as one kept moves up.)
  if (i >= 0 && (!f?.src || 'gro'.includes(w))) return;
  if (i >= 0) l.splice(i, 1);
  const v = { t: f.t || Date.now(), src: f.src.slice(0, MAX), name: f.name || '', w };
  for (const k of 'abc') if (typeof f[k] == 'number') v[k] = f[k];
  put(V + key, [v, ...l].sort((a, b) => b.t - a.t).slice(0, N));
  trim(V, RINGS, (l) => l?.[0]?.t || 0);
}
// A new Formula saved: its versions go with it.
export function move(from, to) {
  if (from === to) return;
  const l = list(from);
  if (!l.length) return;
  put(V + to, [...l, ...list(to)].sort((a, b) => b.t - a.t).slice(0, N));
  try { localStorage.removeItem(V + from); } catch { /* blocked */ }
}
// The site's draft: read, or written (null: dropped).
export function draft(key, d) {
  if (d === undefined) { const x = get(FD + key, null); return typeof x?.src == 'string' ? x : null; }
  if (!d) { try { localStorage.removeItem(FD + key); } catch { /* blocked */ } return; }
  put(FD + key, { ...d, t: Date.now(), src: d.src.slice(0, MAX) });
  trim(FD, DRAFTS, (x) => x?.t || 0);
}

// Lines of `a` (now) against `b` (that version): [[' ' | '-' | '+', line]].
// The same lines at either end left out of the comparison; past 200 × 200
// lines between, all of one out and all of the other in.
export function diff(a, b) {
  const x = a.split('\n'), y = b.split('\n');
  let p = 0, s = 0;
  while (p < x.length && p < y.length && x[p] === y[p]) p++;
  while (s < x.length - p && s < y.length - p && x[x.length - 1 - s] === y[y.length - 1 - s]) s++;
  const X = x.slice(p, x.length - s), Y = y.slice(p, y.length - s), m = X.length, n = Y.length, o = x.slice(0, p).map((l) => [' ', l]);
  if (m * n > 40000) o.push(...X.map((l) => ['-', l]), ...Y.map((l) => ['+', l]));
  else {
    const L = Array.from({ length: m + 1 }, () => new Uint16Array(n + 1));
    for (let i = m - 1; i >= 0; i--) for (let j = n - 1; j >= 0; j--) L[i][j] = X[i] === Y[j] ? L[i + 1][j + 1] + 1 : Math.max(L[i + 1][j], L[i][j + 1]);
    let i = 0, j = 0;
    while (i < m || j < n) {
      if (i < m && j < n && X[i] === Y[j]) { o.push([' ', X[i]]); i++; j++; } else if (i < m && (j >= n || L[i + 1][j] >= L[i][j + 1])) o.push(['-', X[i++]]); else o.push(['+', Y[j++]]);
    }
  }
  return o.concat(x.slice(x.length - s).map((l) => [' ', l]));
}
// The changed lines with one around each, `…` between, 40 at most.
function shown(d) {
  const keepAt = (i) => d.slice(Math.max(0, i - 1), i + 2).some((x) => x[0] !== ' ');
  const out = [];
  let gap = false, n = 0;
  d.forEach((x, i) => {
    if (!keepAt(i)) { gap = out.length > 0; return; }
    if (gap) out.push(h('div.cap', '…'));
    gap = false;
    if (++n <= 40) out.push(h('div' + (x[0] === '+' ? '.a' : x[0] === '-' ? '.d' : ''), (x[0] === ' ' ? '  ' : x[0] === '+' ? '+ ' : '− ') + x[1]));
  });
  if (n > 40) out.push(h('div.cap', `… and ${n - 40} more lines`));
  return out;
}
const WHY = { g: 'As it came from the gallery', r: 'As it came from opendmd.dev', o: 'As it was opened', d: 'Your changes', s: 'Saved', p: 'Sent to your panel' };
const day = (d) => Math.floor((d - d.getTimezoneOffset() * 6e4) / 864e5);
export function when(t) {
  const d = new Date(t), k = day(new Date()) - day(d), hm = d.toTimeString().slice(0, 5);
  return (k === 0 ? 'Today ' : k === 1 ? 'Yesterday ' : d.toLocaleDateString(undefined, { day: 'numeric', month: 'short' }) + ' ') + hm;
}

// Grown past the window's foot: up again (a sheet on a phone stays put).
const fit = (m) => { const r = m.getBoundingClientRect(); if (!m.classList.contains('sheet') && r.bottom > innerHeight - 8) m.style.top = Math.max(8, innerHeight - r.height - 8) + 'px'; };
// The menu, under `btn`: o = {key, cur (the text now), back(version), site}.
export function menu(btn, o) {
  const r = btn.getBoundingClientRect(), vs = list(o.key);
  const item = (v, k) => {
    const d = diff(o.cur, v.src), add = d.filter((x) => x[0] === '+').length, del = d.filter((x) => x[0] === '-').length;
    const box = h('div', { id: `psi-v-${k}-d`, hidden: true }, add + del ? h('div.psi-diff', shown(d)) : h('p.cap', 'The same as now.'),
      h('button.pri', { id: `psi-v-${k}-go`, disabled: !(add + del), onclick: () => { closeMenu(); o.back(v); toast(`Back to ${v.w === 's' ? 'the one saved ' + when(v.t) : (WHY[v.w] || 'it').toLowerCase()}: undo takes it away again`, 'ok'); } }, 'Go back to this'));
    const tag = WHY[v.w] || '', lead = v.w === 's' || v.w === 'd' || !tag ? when(v.t) : tag;
    return [h('button.mi', { id: `psi-v-${k}`, 'aria-expanded': 'false', onclick: (e) => { box.hidden = !box.hidden; e.currentTarget.setAttribute('aria-expanded', !box.hidden); fit(box.closest('.menu')); } },
      h('span.grow', h('b', lead), h('span.cap', ` ${lead === tag ? when(v.t) : tag}${v.name ? ' · ' + v.name : ''}`)), h('span.cap', add + del ? `+${add} −${del}` : 'as now')), box];
  };
  popup(h('div.menu.wide', { id: 'psi-vers', role: 'dialog', 'aria-label': 'Earlier versions' }, h('div.mh', 'Earlier versions'),
    vs.length ? vs.map(item) : h('p.cap', { style: 'margin:4px 8px' }, o.site ? 'None yet: one is kept as a formula arrives here, and each time it goes to your panel.' : 'None yet: one is kept each time you save, and as a formula arrives.'),
    vs.length > 0 && h('p.cap', { style: 'margin:4px 8px' }, 'Kept in this browser, the last 10 of this formula. − lines go, + lines come back.')), r.left, r.bottom + 4);
}

// The site's offer, over its editor: the draft `d` back, or not now (it
// stays under Earlier versions either way).
export function offer(d, go) {
  const el = h('div.card.row', { id: 'f-cont' }, h('span.grow', `Your changes from ${when(d.t)} are still here.`),
    h('button.pri', { id: 'f-cont-go', onclick: () => { el.remove(); go(); } }, 'Continue where you left off'),
    h('button.txt', { id: 'f-cont-no', onclick: () => el.remove() }, 'Not now'));
  return el;
}
