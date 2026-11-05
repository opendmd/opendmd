// The first-run nudge (docs/api.md "Security"): while the panel has no API
// key, one banner above every view says what that means and offers to set
// one (Settings › Device › Security). Not now hides it for this visit; it
// comes back next time until a key is set, and is never more than this
// banner. Imported by app.js only while `api_key_set` is false.

import { $, h } from './util.js';

export function show() {
  if ($('#keynag') || sessionStorage.getItem('odmd.keynag') === 'later') return;
  const b = h('div.banner', { id: 'keynag', role: 'note' },
    h('p', { style: 'margin:0 0 8px' }, 'Anyone on your WiFi can change this panel: its playlists, its settings, even its software. Set a key so only you can.'),
    h('div.row', h('button.pri.sm', { id: 'keynag-set', onclick: () => { hide(); location.hash = '#settings/device'; setTimeout(() => $('#sec-key')?.scrollIntoView({ block: 'center' }), 600); } }, 'Set a key'),
      h('button.sm', { id: 'keynag-later', onclick: () => { try { sessionStorage.setItem('odmd.keynag', 'later'); } catch { /* private mode */ } hide(); } }, 'Not now')));
  $('#main').prepend(b);
}

export const hide = () => $('#keynag')?.remove();
