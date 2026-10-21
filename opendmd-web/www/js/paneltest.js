// Settings → Advanced → Panel timing → Test panel timing (its own size
// line, imported on the first click): POST /api/panel/test shows the
// device's test pictures with one live setting switching between A and B;
// the pick is saved through /api/config. The test ends by itself after 2 min.

import { app } from './state.js';
import { h, clear, toast } from './util.js';

const post = (b) => app.src.json('POST', '/api/panel/test', b);
const L = { clk_phase: ['Clock edge', (v) => (v ? 'Rising' : 'Falling')], latch_blanking: ['Latch blanking', (v) => v] };

export function panelTest(box) {
  const say = (...k) => { box.hidden = !k[0]; clear(box, ...k); };
  const b = (t, fn) => h('button.sm', { onclick: fn }, t);
  const stop = (m) => { post({ stop: true }).catch(() => {}); say(m && h('p', m)); };
  async function run(k) {
    let r;
    try { r = await post({ setting: k }); } catch (e) { say(); return toast('panel test: ' + e.message, 'bad'); }
    // Saved at once, no trial: both values have just been seen side by side.
    // The save ends the test on the device.
    const pick = (s) => async () => {
      const v = r[s][k], [n, f] = L[k];
      try {
        await app.src.putConfig({ panel: { [k]: v }, panel_confirm: true });
        say(h('p', `Saved: ${n} ${f(v)}.`));
        await app.refreshConfig();
      } catch (e) { stop(); toast('panel timing: ' + e.message, 'bad'); }
    };
    say(h('p', (k === 'clk_phase' ? '' : 'One more setting. ') + 'Watch the panel. Does A or B look cleaner?'),
      h('p.cap', 'Each picture shows under A, then B: the letter is in the bottom corner.'),
      h('div.row', b('A looks clean', pick('a')), b('B looks clean', pick('b')),
        b('Both look the same / fine', () => stop('Nothing changed.')),
        b('Both have dots', () => (k === 'clk_phase' ? run('latch_blanking')
          : stop('Neither helped. Try Driver chip below, then switch the player off and on. If the dots stay, it may be the panel.'))),
        b('Stop', () => stop())));
  }
  run('clk_phase');
}
