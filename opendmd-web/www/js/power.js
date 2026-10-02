// Settings → Power (imported by settings.js; its own size line): the
// Maximum brightness setting, which also sets how far the Playback
// brightness slider goes, and a warning above the tested level.

import { app } from './state.js';
import { h, on, toast } from './util.js';

export function powerCard(card) {
  const bright = document.getElementById('st-brightness');
  const num = h('input', { type: 'number', min: 16, max: 255, id: 'st-maxbright', style: 'width:100px' });
  const safe = () => app.status?.brightness_safe_max;
  const q = h('span.q', { tabIndex: 0 }, '?');
  const note = h('p.cap', { id: 'st-maxbright-note' }, 'Limits how bright the panel can go', q);
  const warn = h('div.banner', { id: 'st-maxbright-warn', hidden: true });
  const details = h('details', { id: 'st-power' }, h('summary', 'Advanced'),
    h('label.stack', { style: 'margin-top:12px' }, 'Maximum brightness', num), note, warn,
    h('div.row', { style: 'margin-top:12px' }, h('button', { id: 'st-save-power', onclick: save }, 'Save maximum')));
  function show() {
    const s = safe();
    q.title = `Above ${s ?? 'the tested level'} the panel can reset on the supplied power supply, especially on white screens.`;
    warn.hidden = !(s && +num.value > s);
    warn.textContent = `Above the tested limit (${s}). The display may reset. Only raise this with a stronger power supply.`;
    if (!warn.hidden) details.open = true;
  }
  function sync(c) {
    const m = c.max_brightness ?? 255;
    num.value = m;
    bright.max = m;
    if (+bright.value > m) { bright.value = m; bright.dispatchEvent(new Event('input')); }
    show();
  }
  async function save() {
    const v = +num.value;
    if (!(v >= 16 && v <= 255)) { toast('maximum brightness must be 16 to 255', 'bad'); return; }
    if (v === app.config.max_brightness) { toast('nothing changed'); return; }
    try {
      await app.src.putConfig({ max_brightness: v });
      await app.refreshConfig();
      toast('saved: maximum brightness ' + v, 'ok');
    } catch (e) { toast('save: ' + e.message, 'bad'); }
  }
  num.oninput = show;
  on('config', sync);
  on('status', show);
  card.append(h('h2', 'Power'), details);
  sync(app.config);
}
