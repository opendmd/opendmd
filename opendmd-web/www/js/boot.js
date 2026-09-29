// Settings → Start-up (imported by settings.js; its own size line): the
// boot_show choice, and previews of the show and of the no-card screen,
// drawn by the same core as the panel (with this device's version and
// address in the status line).

import { app } from './state.js';
import { PanelPreview } from './preview.js';
import { h, toast } from './util.js';

export function bootCard(card) {
  const sel = h('select', { id: 'st-boot' }, [['off', 'Off'], ['short', 'Short'], ['full', 'Full']].map(([v, l]) => h('option', { value: v }, l)));
  sel.value = app.config?.boot_show ?? 'short';
  const canvas = h('canvas.panel', { id: 'st-boot-panel' });
  const box = h('div', { hidden: true, style: 'margin-top:12px' }, canvas);
  let pv = null;
  async function play(len, note) {
    const s = app.status || {};
    const q = new URLSearchParams({ len, v: s.version || '', ip: s.ip || '' });
    if (note) q.set('note', note);
    box.hidden = false;
    pv ??= new PanelPreview(canvas);
    // Drawn every 10 ms: played by the page's clock, as the effects are.
    if (await pv.show('@boot?' + q, (p) => app.previewBytes(p))) pv.syncTo(0, performance.now());
  }
  async function save() {
    if (sel.value === app.config.boot_show) { toast('nothing changed'); return; }
    try {
      await app.src.putConfig({ boot_show: sel.value });
      await app.refreshConfig();
      toast('saved: start-up show ' + sel.value, 'ok');
    } catch (e) { toast('save: ' + e.message, 'bad'); }
  }
  card.append(h('h2', 'Start-up'),
    h('p.cap', 'What the panel shows while it starts: a warp into the name and the version, then the first GIF (Short adds about a second, Full about five). With no card in, the panel says so, and shows its address.'),
    h('div.form', h('label.stack', 'start-up show', sel)),
    h('div.row', { style: 'margin-top:12px' },
      h('button', { id: 'st-boot-preview', onclick: () => (sel.value === 'off' ? toast('Off: the panel stays dark until the first GIF') : play(sel.value)) }, 'Preview'),
      h('button', { id: 'st-boot-nocard', onclick: () => play('card', 'Insert a microSD card') }, 'No card screen'),
      h('button.pri', { id: 'st-save-boot', onclick: save }, 'Save start-up')),
    box);
}
