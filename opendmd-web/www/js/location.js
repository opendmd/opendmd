// Settings › Schedule › Location (imported by settings.js with the tab, on a
// player whose /api/config has `lat`/`lon`; its own size line): latitude and
// longitude to 0.01°, "Use this browser's location" (asked only on the
// click, rounded here before it is sent; browsers answer only a secure
// page, which the panel's http:// page is not, so there it says to type
// it in) and Clear. PUT /api/config
// `{"lat", "lon"}` both numbers or both null. Formulas' `daylight`, `sun.alt`
// and `season` follow it (the panel works out the sky); never on the card.

import { app } from './state.js';
import { h, toast, saver } from './util.js';

const r2 = (v) => Math.round(v * 100) / 100;

export function locationCard(el) {
  const num = (id, label, max) => h('input', { type: 'number', id, min: -max, max, step: 0.01, style: 'width:120px', placeholder: label === 'Latitude' ? '51.51' : '-0.13', 'aria-label': label });
  const lat = num('loc-lat', 'Latitude', 90), lon = num('loc-lon', 'Longitude', 180);
  const head = h('h3', 'Location'), run = saver(head);
  const now = h('p.cap', { id: 'loc-now' });
  function fill() {
    const c = app.config || {}, set = c.lat != null;
    lat.value = set ? c.lat : '';
    lon.value = set ? c.lon : '';
    clr.hidden = !set;
    now.textContent = set ? `Set: ${Math.abs(c.lat)}°${c.lat < 0 ? 'S' : 'N'}, ${Math.abs(c.lon)}°${c.lon < 0 ? 'W' : 'E'}` : 'Not set: Formulas that follow the sun see a sunny midday.';
  }
  const put = (a, b) => run(async () => {
    await app.src.putConfig({ lat: a, lon: b });
    await app.refreshConfig();
    fill();
  });
  // Saved once both are filled in and in range.
  function save() {
    const a = lat.value.trim(), b = lon.value.trim(), c = app.config || {};
    if (!a || !b) return;
    const x = r2(+a), y = r2(+b);
    if (!(Math.abs(x) <= 90 && Math.abs(y) <= 180)) { toast('Latitude is −90 to 90 and longitude −180 to 180', 'bad'); return; }
    if (x !== c.lat || y !== c.lon) put(x, y);
  }
  lat.onchange = lon.onchange = save;
  const here = h('button', {
    id: 'loc-here', hidden: !navigator.geolocation,
    onclick: () => navigator.geolocation.getCurrentPosition((p) => put(r2(p.coords.latitude), r2(p.coords.longitude)),
      () => toast(isSecureContext ? 'Couldn’t get this browser’s location: type it in instead' : 'Browsers share their location only with secure (https) pages, and the panel’s isn’t one: type it in (a map app shows it)', 'bad'), { timeout: 15000, maximumAge: 600000 }),
  }, 'Use this browser’s location');
  const clr = h('button.txt', { id: 'loc-clear', onclick: () => put(null, null) }, 'Clear');
  el.append(head,
    h('p.cap', 'Where the panel is, roughly (to about a kilometre), so Formulas can follow the real sun: daylight, dawn and dusk, the seasons. It stays on your panel: it isn’t written to the card or shared.'),
    h('div.form.f2', h('label.stack', 'Latitude (north +)', lat), h('label.stack', 'Longitude (east +)', lon)),
    h('div.row', { style: 'margin-top:12px' }, here, clr), now);
  fill();
}
