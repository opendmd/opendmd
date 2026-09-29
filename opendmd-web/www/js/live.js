// Library → Live (loaded when that category is chosen): things the panel
// draws itself rather than GIFs — the clock and a scrolling text. They are
// items like any other: preview, send to the panel, add to a playlist (the
// playlist names them as `@clock…` / `@text?msg=…` lines, which the player
// and this preview draw with the same code).

import { app } from './state.js';
import { h } from './util.js';

const CLOCKS = [['@clock', 'hours and minutes'], ['@clock?seconds=1', 'with seconds'], ['@clock?date=1', 'with the date'],
  ['@clock?style=small&seconds=1&date=1', 'small, seconds and date']];

// The items, and the text's form in `box`; `preview(item)` shows one,
// `refresh()` redraws the list after the text changed.
export function items(box, preview, refresh) {
  const item = (path, name) => {
    const e = app.wasm.parsePath(path);
    return { path, e, name, norm: app.wasm.normalizeTitle(e.title) };
  };
  const msg = h('input', { type: 'text', id: 'live-msg', value: 'OpenDMD', maxLength: 200 });
  const speed = h('select', { id: 'live-speed' }, [[60, 'slow'], [40, 'normal'], [25, 'fast']].map(([v, l]) => h('option', { value: v }, l)));
  speed.value = 40;
  const text = {};
  const set = () => Object.assign(text, item(`@text?msg=${encodeURIComponent(msg.value.trim() || 'OpenDMD')}${speed.value != 40 ? '&speed=' + speed.value : ''}`, 'scrolling text'));
  // The row follows the typing; the preview shows it on Enter (or a click
  // on the row). A refresh on blur would land between a click's mousedown
  // and mouseup on another row and swallow the click.
  msg.oninput = () => { set(); refresh(); };
  msg.onkeyup = (e) => e.key == 'Enter' && preview(text);
  speed.onchange = () => { set(); refresh(); preview(text); };
  set();
  box.replaceChildren(h('div.form', { style: 'margin-top:10px' }, h('label.stack', 'scrolling text', msg), h('label.stack', 'speed', speed)),
    h('p.cap', 'The clock shows the player’s own time. Add these to a playlist like any GIF.'));
  return [...CLOCKS.map(([p, n]) => item(p, n)), text];
}
