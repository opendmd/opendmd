// The status pill in the header: what the panel is playing, or why it
// isn't (paused, switching or reading a playlist, no card, not reachable).
// A tap goes to Now. On Now itself, which shows what plays, only a state
// that isn't playing shows. Fed by every full status (app.refreshStatus),
// Now's stream and polls (`now`) and the link's state (`link`, js/source.js
// dfetch). Off Now, the slim status (`?now=1`, ~300 B) every PILL_MS keeps
// it true; a full one when the link comes back (and, js/state.js, when the
// tab does).

import { app } from './state.js';
import { $, h, on, emit, label, drawn, isFormula } from './util.js';

const PILL_MS = 20000;

export function pill() {
  const dot = h('i.dot'), text = h('span', { role: 'status', 'aria-live': 'polite' });
  const b = h('button.pill', { id: 'pill', onclick: () => app.go('now') }, dot, text);
  $('#top').append(b);
  let s = app.status, down = false;
  const onNow = () => /^#now|^#?$/.test(location.hash);
  const draw = () => {
    const np = s?.now_playing || {}, sd = s?.sd || {}, p = np.path || '';
    // (A playlist chosen from this page: "Switching"; any other reread — a
    // weight saved, another browser's change — is said as what it is.)
    const [k, t] = down ? ['bad', 'Panel not reachable']
      : !sd.mounted && (sd.attempts || sd.error) ? ['bad', 'No card']
      : sd.readable === false ? ['bad', 'Card can’t be read']
      : s?.index_state === 'building' ? ['busy', app.switching ? `Switching to “${app.switching}”…` : 'Reading the playlist…']
      : !p ? ['', 'Nothing playing']
      : s.paused ? ['busy', 'Paused']
      : ['ok', drawn(p) ? np.title || (isFormula(p) ? 'Formula' : 'Title card') : label(app.wasm.parsePath(p))];
    dot.className = 'dot ' + k;
    text.textContent = t;
    b.title = t + ' · go to Now';
    b.hidden = onNow() && k === 'ok';
  };
  on('status', (x) => { s = x; down = false; draw(); });
  on('now', (x) => { s = x; draw(); });
  on('link', (ok) => {
    if (down === !ok) return;
    down = !ok;
    draw();
    if (ok && app.device) app.refreshStatus().catch(() => {});
  });
  // Off Now (which reads the status itself), the slim read now and then.
  const slim = async () => { if (!app.status || !app.src.now) return app.refreshStatus(); Object.assign(app.status, await app.src.now()); emit('now', app.status); };
  addEventListener('hashchange', pace);
  function pace() { if (onNow()) app.unwatch('pill'); else app.watch('pill', PILL_MS, slim); draw(); }
  pace();
}
