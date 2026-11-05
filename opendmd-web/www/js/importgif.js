// A GIF from the site's Create Art (opendmd.dev/create/, its Send to my
// panel): that page opens this app at `#import` (app.js), this window says
// `READY` to its opener, and the page answers this window alone with
// `{type: IMPORT, gif (an ArrayBuffer, transferred: no copy, nothing in a
// link, nothing kept on the site), name, title, platform, author, logo}`.
// The Library then opens Add GIFs with the file and its fields filled in;
// nothing is on the card until the user says Add. As strict as the Formula
// hand-off (js/formulas.js `sent`): only from the opener, only from
// opendmd.dev or this page's own origin (the demo, the tests), the shape
// checked, the GIF capped. Loaded only on `#import`.

import { SITE } from './util.js';

export const READY = 'opendmd-import-ready', IMPORT = 'opendmd-import';
export const MAX_GIF = 4 * 1024 * 1024;
const WAIT = 30000; // the site renders an animation's every frame first

const text = (v, n, re) => (typeof v == 'string' && v.length <= n && (!re || re.test(v)) ? v : '');

// The GIF the opener sends: {file, init} (Add GIFs' fields), or a rejection
// with a sentence.
export function received() {
  const from = [new URL(SITE).origin, location.origin], o = window.opener;
  if (!o) return Promise.reject(new Error('open it again from Create Art on opendmd.dev'));
  return new Promise((res, rej) => {
    const end = (f, v) => { clearTimeout(t); removeEventListener('message', on); f(v); };
    const t = setTimeout(() => end(rej, new Error('opendmd.dev didn’t send it: send it again from there')), WAIT);
    function on(e) {
      const d = e.data;
      if (e.source !== o || !from.includes(e.origin) || d?.type !== IMPORT) return;
      if (!(d.gif instanceof ArrayBuffer)) return;
      if (d.gif.byteLength > MAX_GIF) { end(rej, new Error(`it is over ${MAX_GIF >> 20} MB`)); return; }
      const head = new Uint8Array(d.gif, 0, Math.min(4, d.gif.byteLength));
      if (String.fromCharCode(...head) !== 'GIF8') { end(rej, new Error('that isn’t a GIF')); return; }
      const name = text(d.name, 120, /^[^/\\\u0000-\u001f]+\.gif$/i) || 'Created.gif';
      // (What is missing or refused, Add GIFs guesses from the name, as for a picked file.)
      const init = Object.fromEntries(Object.entries({ title: text(d.title, 80), platform: text(d.platform, 16, /^[A-Za-z0-9]+$/),
        author: text(d.author, 40, /^[A-Za-z0-9]+$/) }).filter(([, v]) => v));
      if (d.logo === true) init.flags = ['LOGO'];
      end(res, { file: new File([d.gif], name, { type: 'image/gif' }), init });
    }
    addEventListener('message', on);
    for (const f of from) o.postMessage({ type: READY }, f);
  });
}
