// The status pill in the header, on every view: what the panel is playing,
// or why it isn't (paused, switching playlist, no card, not reachable). A
// tap goes to Now. Fed by every full status (app.refreshStatus), Now's
// stream and polls (`now`), and the link's state (`link`, js/source.js
// dfetch). Off Now, a full status every PILL_MS keeps it true.

import { app } from './state.js';
import { $, h, on, label, drawn, isFormula } from './util.js';

const PILL_MS = 20000;

export function pill() {
  const dot = h('i.dot'), text = h('span'), b = h('button.pill', { id: 'pill', onclick: () => app.go('now') }, dot, text);
  $('#top').append(b);
  let s = app.status, down = false;
  const draw = () => {
    const np = s?.now_playing || {}, sd = s?.sd || {}, p = np.path || '';
    const [k, t] = down ? ['bad', 'Panel not reachable']
      : !sd.mounted && (sd.attempts || sd.error) ? ['bad', 'No card']
      : sd.readable === false ? ['bad', 'Card can’t be read']
      : s?.index_state === 'building' ? ['busy', 'Switching playlist…']
      : !p ? ['', 'Nothing playing']
      : [s.paused ? 'busy' : 'ok', (s.paused ? 'Paused · ' : '') + (drawn(p) ? np.title || (isFormula(p) ? 'Formula' : 'Title card') : label(app.wasm.parsePath(p)))];
    dot.className = 'dot ' + k;
    text.textContent = t;
    b.title = b.ariaLabel = t + ' · go to Now';
  };
  on('status', (x) => { s = x; down = false; draw(); });
  on('now', (x) => { s = x; draw(); });
  on('link', (ok) => { if (down !== !ok) { down = !ok; draw(); } });
  // Off Now (which reads the status itself), a slow full read.
  addEventListener('hashchange', pace);
  function pace() { if (/^#now|^#?$/.test(location.hash)) app.unwatch('pill'); else app.watch('pill', PILL_MS); }
  pace();
  draw();
}
