// The no-key line (docs/api.md "Security"): while the panel has no API
// key, one line at the top of Settings says what that means and links to
// the switch (Settings › WiFi › Security). Never over the other views:
// setup already offered a key. Imported by Settings while `api_key_set`
// is false; gone once a key is set.

import { $, h } from './util.js';

export function line(box) {
  if ($('#keynag')) return;
  box.append(h('p.cap.keyline', { id: 'keynag', role: 'note' }, 'No key: anyone on your WiFi can change this panel. ',
    h('a', { id: 'keynag-set', href: '#settings/wifi', onclick: () => setTimeout(() => $('#sec-key')?.scrollIntoView({ block: 'center' }), 600) }, 'Set\u00a0a\u00a0key')));
}

export const hide = () => $('#keynag')?.remove();
