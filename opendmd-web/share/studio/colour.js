// The Studio's colour picker and palette tools (/studio/), loaded the first
// time either opens. The picker: any 24-bit colour, by hue, saturation and
// value or by hex; "Add" puts it in the piece's colours. The palettes: the
// piece's colours saved as a palette of the user's own; theirs renamed or
// deleted (Den 16 and the starter palettes are read-only); one imported
// from pasted hex lines or a .hex or .gpl file (Lospec's downloads), read
// here, never fetched.
import { record, Conflict, CAPS } from './store.js';
import { builtin, DEFAULT } from './palettes.js';

const hexOf = (n) => '#' + n.toString(16).padStart(6, '0');
const el = (tag, cls, text) => { const e = document.createElement(tag); if (cls) e.className = cls; if (text !== undefined) e.textContent = text; return e; };
function button(text, act, pri) { const b = el('button', 's-pill' + (pri ? ' s-pri' : ''), text); b.dataset.a = act; return b; }

// HSV (h 0..360, s and v 0..1) <-> 0xRRGGBB.
export function rgbOf(h, s, v) {
  const f = (n) => { const k = (n + h / 60) % 6; return Math.round(255 * (v - v * s * Math.max(0, Math.min(k, 4 - k, 1)))); };
  return (f(5) << 16) | (f(3) << 8) | f(1);
}
export function hsvOf(c) {
  const r = (c >> 16) / 255, g = ((c >> 8) & 255) / 255, b = (c & 255) / 255, v = Math.max(r, g, b), d = v - Math.min(r, g, b);
  const h = !d ? 0 : v === r ? ((g - b) / d + 6) % 6 : v === g ? (b - r) / d + 2 : (r - g) / d + 4;
  return [h * 60, v ? d / v : 0, v];
}

// The picker, in `host`: a saturation and value square for the hue, the hue, the hex.
export function picker(host, S) {
  if (host.firstChild) return;
  let [h, s, v] = hsvOf(S.current() ?? 0xf7a531), exact = null; // exact: a hex typed, kept as it is
  const now = () => exact ?? rgbOf(h, s, v);
  const sv = el('canvas', 's-sv'), hue = el('input', 's-hue'), hex = el('input', 's-in s-hex'), prev = el('span', 's-prev');
  sv.width = 192; sv.height = 96;
  Object.assign(hue, { type: 'range', min: 0, max: 359, step: 1, title: 'Hue' });
  hue.setAttribute('aria-label', 'Hue');
  Object.assign(hex, { type: 'text', maxLength: 7, spellcheck: false, autocomplete: 'off' });
  hex.setAttribute('aria-label', 'Hex colour');
  sv.setAttribute('aria-label', 'Saturation, left to right, and brightness, bottom to top');
  const row = el('div', 's-row');
  row.append(prev, hex, button('Add colour', 'add', true), button('Close', 'close'));
  const box = el('div', 's-pk');
  box.append(sv, hue, row);
  host.replaceChildren(box);
  const g = sv.getContext('2d');
  const draw = (from) => {
    if (from !== 'hex') exact = null;
    const c = now();
    g.fillStyle = hexOf(rgbOf(h, 1, 1));
    g.fillRect(0, 0, sv.width, sv.height);
    let gr = g.createLinearGradient(0, 0, sv.width, 0);
    gr.addColorStop(0, '#fff'); gr.addColorStop(1, 'rgba(255,255,255,0)');
    g.fillStyle = gr; g.fillRect(0, 0, sv.width, sv.height);
    gr = g.createLinearGradient(0, 0, 0, sv.height);
    gr.addColorStop(0, 'rgba(0,0,0,0)'); gr.addColorStop(1, '#000');
    g.fillStyle = gr; g.fillRect(0, 0, sv.width, sv.height);
    const x = s * (sv.width - 1), y = (1 - v) * (sv.height - 1);
    g.strokeStyle = v > 0.5 ? '#000' : '#fff';
    g.strokeRect(x - 3.5, y - 3.5, 7, 7);
    if (from !== 'hue') hue.value = Math.round(h);
    if (from !== 'hex') hex.value = hexOf(c);
    prev.style.background = hexOf(c);
    host.dataset.colour = hexOf(c);
  };
  const at = (e) => {
    const r = sv.getBoundingClientRect();
    s = Math.max(0, Math.min(1, (e.clientX - r.left) / r.width));
    v = Math.max(0, Math.min(1, 1 - (e.clientY - r.top) / r.height));
    draw();
  };
  sv.addEventListener('pointerdown', (e) => { sv.setPointerCapture(e.pointerId); at(e); });
  sv.addEventListener('pointermove', (e) => { if (sv.hasPointerCapture(e.pointerId)) at(e); });
  hue.addEventListener('input', () => { h = +hue.value; draw('hue'); });
  hex.addEventListener('input', () => {
    const m = /^#?([0-9a-f]{6})$/i.exec(hex.value.trim());
    if (m) { exact = parseInt(m[1], 16); [h, s, v] = hsvOf(exact); draw('hex'); }
  });
  hex.addEventListener('keydown', (e) => { if (e.key === 'Enter') S.pickColour(now()); });
  row.addEventListener('click', (e) => {
    const a = e.target.closest('[data-a]')?.dataset.a;
    if (a === 'add') S.pickColour(now());
    if (a === 'close') { host.hidden = true; document.getElementById('s-add').setAttribute('aria-expanded', false); }
  });
  draw();
}

// Pasted text or a file's: a GIMP palette (.gpl: "R G B name" lines after
// its header), or hex colours, one or more a line (Lospec's .hex: "rrggbb").
// Each colour once, in order; `name` from a .gpl's Name line.
export function parsePalette(text) {
  const lines = text.split(/\r?\n/), gpl = /^GIMP Palette/i.test(lines[0]?.trim() || ''), out = [];
  let name = '';
  for (const raw of lines.slice(gpl ? 1 : 0)) {
    const l = raw.trim();
    if (gpl) {
      const n = /^Name:\s*(.+)$/i.exec(l);
      if (n) { name = n[1].trim(); continue; }
      const m = /^(\d{1,3})\s+(\d{1,3})\s+(\d{1,3})(?:\s|$)/.exec(l);
      if (m && m.slice(1).every((x) => +x < 256)) out.push((+m[1] << 16) | (+m[2] << 8) | +m[3]);
    } else for (const m of l.matchAll(/(?:^|[\s,;#])#?([0-9a-f]{6})(?![0-9a-f])/gi)) out.push(parseInt(m[1], 16));
  }
  return { name, colours: [...new Set(out)] };
}

// The palette tools, in `host`.
export function palettes(host, S) {
  const p = S.swatch(), own = S.mine.has(p.id);
  const msg = el('p', 's-cap');
  const mk = (cls, ...kids) => { const d = el('div', cls); d.append(...kids); return d; };
  const name = Object.assign(el('input', 's-in'), { type: 'text', maxLength: CAPS.name, value: p.name, autocomplete: 'off' });
  name.setAttribute('aria-label', 'Palette name');
  const text = Object.assign(el('textarea', 's-in s-paste'), { rows: 4, spellcheck: false,
    placeholder: 'Paste colours: hex, one a line (as Lospec’s .hex), or a GIMP .gpl palette' });
  text.setAttribute('aria-label', 'Colours to import');
  const file = Object.assign(el('input'), { type: 'file', accept: '.hex,.gpl,.txt,text/plain' });
  file.setAttribute('aria-label', 'A .hex or .gpl file');
  const iname = Object.assign(el('input', 's-in'), { type: 'text', maxLength: CAPS.name, placeholder: 'Name', autocomplete: 'off' });
  iname.setAttribute('aria-label', 'Imported palette’s name');
  host.replaceChildren(
    mk('s-row', button('Save this piece’s colours as a palette', 'save')),
    own ? mk('s-row', name, button('Rename', 'rename'), button('Delete', 'delete'))
      : el('p', 's-cap', `${p.name} is read-only: it comes with the Studio. Save this piece’s colours to make one of your own.`),
    el('h4', '', 'Import a palette'), text, mk('s-row', file, iname, button('Import', 'import', true)), msg);
  const put = async (rec, base = null) => {
    const h = await S.run(() => S.store.put(rec, { base }));
    const r = { ...rec, ...h };
    S.mine.set(r.id, r);
    S.use(r);
    return r;
  };
  const make = (nm, list, from) => record('palette', nm.slice(0, CAPS.name) || 'My palette',
    { colours: list.map((c, i) => ({ k: i + 1, hex: hexOf(c) })), source: 'Mine' }, { from });
  file.addEventListener('change', async () => {
    const f = file.files[0];
    if (!f) return;
    if (f.size > 1 << 20) { msg.textContent = 'That file is too big for a palette.'; return; }
    text.value = await f.text();
    if (!iname.value) iname.value = f.name.replace(/\.[^.]*$/, '');
  });
  host.onclick = async (e) => {
    const a = e.target.closest('[data-a]')?.dataset.a;
    if (!a) return;
    if (!S.store) { msg.textContent = 'Palettes can’t be kept in this browser.'; return; }
    try {
      if (a === 'save') {
        const list = S.colours().map((c) => parseInt(c.hex.slice(1), 16)), pc = S.piece();
        if (!list.length) { msg.textContent = 'This piece has no colours yet.'; return; }
        const r = await put(make(`${pc?.name || 'Untitled'} colours`, list, pc ? [{ ...pc, author: { name: '', account: null }, licence: null }] : []));
        palettes(host, S);
        host.lastChild.textContent = `Saved as “${r.name}”.`;
      } else if (a === 'rename') {
        const cur = S.mine.get(p.id);
        try { await put({ ...cur, name: name.value.trim().slice(0, CAPS.name) || cur.name }, cur.ver); } catch (x) {
          if (!(x instanceof Conflict)) throw x;
          const fresh = await S.store.get(p.id); // changed in another tab: renamed from what it is now
          await put({ ...fresh, name: name.value.trim().slice(0, CAPS.name) || fresh.name }, fresh.ver);
        }
        msg.textContent = 'Renamed.';
      } else if (a === 'delete') {
        if (!confirm(`Delete the palette “${p.name}”? Pieces keep their colours.`)) return;
        await S.run(() => S.store.remove(p.id));
        S.mine.delete(p.id);
        S.use(builtin(DEFAULT));
        palettes(host, S);
      } else if (a === 'import') {
        const got = parsePalette(text.value);
        if (!got.colours.length) { msg.textContent = 'No colours found: paste hex colours (one a line) or a .gpl palette.'; return; }
        if (got.colours.length > CAPS.colours) { msg.textContent = `A palette holds ${CAPS.colours} colours at most.`; return; }
        const nm = iname.value.trim() || got.name || 'Imported palette';
        const r = await put(make(nm, got.colours, [{ external: true, note: `Imported: ${(file.files[0]?.name || 'pasted colours').slice(0, 120)}` }]));
        text.value = ''; iname.value = ''; file.value = '';
        palettes(host, S);
        host.lastChild.textContent = `Imported “${r.name}”: ${got.colours.length} colours.`;
      }
    } catch (x) {
      msg.textContent = `Not kept: ${x.message || x}`;
    }
  };
}
