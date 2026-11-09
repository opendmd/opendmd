// Settings → Start-up (imported by settings.js; its own size line): what
// the panel shows while it starts — the OpenDMD name, or a Formula
// (boot_formula: a bundled one or yours, one light enough for the show,
// named as the look names one, js/looks.js) — how long (boot_show), and
// previews of the show and of the no-card screen, drawn by the same core
// as the panel (with this device's version and address in the line). A
// Formula's preview is pkg-psi's frame (an instance of its own, `?boot`)
// with the main module's line over it (`Preview.showBoot`), as the panel
// draws `@boot?formula=…`.

import { app } from './state.js';
import { PanelPreview } from './preview.js';
import { Q } from './psikit.js';
import { load } from './wasm.js';
import { h, toast, note, saver, clear, on } from './util.js';

// The show's length (opendmd_core::boot), and how heavy a Formula may be
// for it (boot::FORMULA_MAX_EST_US: its frame the slow way).
const MS = { short: 1300, full: 5600 }, HEAVY = 12000;
let B = null;
const own = () => (B ??= load('pkg-psi', 'boot').then(([m, x]) => ({ ...m, mem: x.memory })).catch((e) => { B = null; throw e; }));

export function bootCard(card, examples) {
  const sel = h('select', { id: 'st-boot' }, [['off', 'Off'], ['short', 'Short'], ['full', 'Full']].map(([v, l]) => h('option', { value: v }, l)));
  sel.value = app.config?.boot_show ?? 'short';
  const fsel = h('select', { id: 'st-boot-formula' }, h('option', { value: '' }, 'OpenDMD (built in)'));
  const err = h('p.cap.bad', { id: 'st-boot-err', hidden: true });
  const canvas = h('canvas.panel', { id: 'st-boot-panel' });
  const box = h('div', { hidden: true, style: 'margin-top:12px' }, canvas);
  let pv = null, fs = [], L = null, M = null, raf = 0;
  // The Formulas that can be the show: yours and the bundled ones; one that
  // doesn't work, is too heavy or too long is listed, not offered. One set
  // elsewhere stays listed, by its name.
  async function list() {
    if (app.noFormulas) return;
    let ex, mine;
    try { [ex, L, mine] = await Promise.all([examples(), import('./looks.js'), import('./fstore.js').then((S) => S.loaded().then(() => S.list()))]); M = await L.psi(); } catch { return; }
    const cur = app.config?.boot_formula || '';
    fs = [...(Array.isArray(mine) ? mine : []).map((x) => ({ ...x, mine: 1 })), ...(Array.isArray(ex) ? ex : [])]
      .filter((x) => typeof x?.src == 'string').map((x) => ({ ...x, name: L.named(M, x), p: L.path(M, x) }));
    for (const x of fs) x.off = !M.compile(x.src) ? 'doesn’t work yet' : M.info()[6] > HEAVY ? 'too heavy' : /^@formula\?/.test(x.p) && x.p.length > 2048 ? 'too long' : '';
    let k = cur ? fs.findIndex((x) => x.p === cur) : -1;
    if (cur && k < 0) { fs.unshift({ ...L.fromPath(M, cur), p: cur }); k = 0; }
    const opt = (x) => { const i = fs.indexOf(x), off = i !== k && x.off; return h('option', { value: i, disabled: !!off }, x.name + (off ? ` (${off})` : '')); };
    const grp = (label, xs) => xs.length && h('optgroup', { label }, xs.map(opt));
    clear(fsel, h('option', { value: '' }, 'OpenDMD (built in)'), fs.filter((x) => x.saved).map(opt),
      grp('Your Formulas', fs.filter((x) => x.mine)), grp('Bundled Formulas', fs.filter((x) => !x.mine && !x.saved)));
    fsel.value = k < 0 ? '' : String(k);
  }
  const stop = () => { cancelAnimationFrame(raf); raf = 0; };
  // The Formula's frames on its own grid, the line over them, for the show's length.
  async function formula(x, path, ms) {
    const P = await own();
    if (!P.compile(x.src)) { toast('This Formula doesn’t work yet', 'bad'); return; }
    const g = P.info()[10] || 20, t0 = performance.now();
    let last = -1;
    const step = (now) => {
      const n = Math.floor(Math.max(0, Math.min(now - t0, ms - 10)) / g);
      if (n !== last) {
        last = n;
        const d = new Date();
        P.frame(n * g, d.getHours(), d.getMinutes(), d.getSeconds(), Q(x.a), Q(x.b), Q(x.c));
        pv.p.showBoot(new Uint8Array(P.mem.buffer, P.pixels(), 12288), path, n * g);
      }
      raf = now - t0 < ms ? requestAnimationFrame(step) : 0;
    };
    raf = requestAnimationFrame(step);
  }
  async function play(len, note) {
    const s = app.status || {};
    const q = new URLSearchParams({ len, v: s.version || '', ip: s.ip || '' });
    if (note) q.set('note', note);
    box.hidden = false;
    pv ??= new PanelPreview(canvas);
    stop();
    const x = len != 'card' && fs[fsel.value];
    if (x?.src) return formula(x, '@boot?' + q, MS[len]);
    // Drawn every 10 ms: played by the page's clock, as Formulas are.
    if (await pv.show('@boot?' + q, (p) => app.previewBytes(p))) pv.syncTo(0, performance.now());
  }
  const head = h('h2', 'Start-up'), run = saver(head);
  sel.onchange = () => run(async () => {
    await app.src.putConfig({ boot_show: sel.value });
    await app.refreshConfig();
  });
  fsel.onchange = () => run(async () => {
    const x = fs[fsel.value];
    try {
      await app.src.putConfig({ boot_formula: x ? x.p : '' });
      await app.refreshConfig();
    } catch (e) {
      // Refused (400/413): the picker goes back to what the panel has.
      await app.refreshConfig().catch(() => {});
      await list();
      toast(e.status == 400 ? `The panel can’t use it at start-up: ${e.message.replace(/^400: (boot_formula: )?/, '')}` : e.message, 'bad');
      return false;
    }
  });
  // The last start: the Formula would not play, so the name did.
  const said = (st) => { const e = st?.boot_formula?.error; err.hidden = !e; err.textContent = e ? `At the last start the panel showed OpenDMD instead: ${e}` : ''; };
  on('status', said);
  said(app.status);
  on('config', list);
  list();
  card.append(head,
    note('What the panel shows while it starts', 'The name lights up, dot by dot, over the version, or a Formula plays instead, with the network to join or the address over it when there’s something to say. Then the first GIF (Short adds about a second, Full about five). With no card in, the panel says so, and shows its address.'),
    h('div.form.f2', ...(app.noFormulas ? [] : [h('label.stack', 'Start-up', fsel)]), h('label.stack', 'Length', sel)), err,
    h('div.row', { style: 'margin-top:12px' },
      h('button', { id: 'st-boot-preview', onclick: () => (sel.value === 'off' ? toast('Off: the panel stays dark until the first GIF') : play(sel.value)) }, 'Preview'),
      h('button', { id: 'st-boot-nocard', onclick: () => { stop(); play('card', 'Insert a microSD card'); } }, 'No card screen')),
    box);
}
