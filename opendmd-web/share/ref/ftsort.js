// The formula gallery (/formulas/): sorting and filtering its tiles. The
// page comes newest first (tools/gallery_page.py `order`, and the Worker's
// page from its store); this shows the form above the lists and keeps its
// state in the address's query, so a view can be linked:
//   ?q=<words>  name, what it does or who made it, every word
//   ?sort=old | name  (newest when absent)
//   ?show=community | panel | looks  (all when absent)
//   ?controls=1  only those with controls
// Each tile says what it is in its data- attributes (gallery_page.py
// `tile`): data-added (ISO time, absent when not known: last either way),
// data-look, data-panel, data-controls. Tiles move inside their own list
// and are hidden, never rebuilt, so ftiles.js's hover-play keeps working.
// tools/site.py copies this file beside the page.

const form = document.getElementById('fsort');
const lists = [...document.querySelectorAll('main ul.ftiles')];
const none = document.querySelector('main .fs-none');
const tiles = lists.flatMap((ul) => [...ul.children]);
const DEFAULTS = { q: '', sort: 'new', show: 'all', controls: '' };

// gallery_page.py's `name.casefold()` order, near enough (the Worker's byName).
const fold = (s) => s.toLowerCase().replace(/ß/g, 'ss').replace(/ς/g, 'σ').replace(/ſ/g, 's');
const cmp = (x, y) => (x < y ? -1 : x > y ? 1 : 0);
const nameOf = (li) => li.querySelector('h3')?.firstChild?.textContent ?? '';
const slugOf = (li) => new URLSearchParams(new URL(li.querySelector('a.fpic').href).hash.slice(1)).get('gallery') ?? '';
const key = new Map(tiles.map((li) => [li, {
  name: fold(nameOf(li)),
  slug: slugOf(li),
  added: li.dataset.added || '',
  text: fold([nameOf(li), li.querySelector('.fdesc')?.textContent, li.querySelector('.fby')?.textContent].join(' ')),
}]));
// Code-point order, as Python compares strings.
const cp = (x, y) => {
  const p = Array.from(x), q = Array.from(y);
  for (let i = 0; i < Math.min(p.length, q.length); i++) {
    const d = p[i].codePointAt(0) - q[i].codePointAt(0);
    if (d) return d;
  }
  return p.length - q.length;
};
const byName = (a, b) => cp(key.get(a).name, key.get(b).name) || cp(key.get(a).slug, key.get(b).slug);
const SORTS = {
  // The undated last, by name, in both date orders.
  new: (a, b) => (!key.get(a).added - !key.get(b).added) || cmp(key.get(b).added, key.get(a).added) || byName(a, b),
  old: (a, b) => (!key.get(a).added - !key.get(b).added) || cmp(key.get(a).added, key.get(b).added) || byName(a, b),
  name: byName,
};

function read() {
  const q = new URLSearchParams(location.search);
  const s = { ...DEFAULTS };
  s.q = (q.get('q') || '').slice(0, 100);
  if (q.get('sort') in SORTS) s.sort = q.get('sort');
  if (['community', 'panel', 'looks'].includes(q.get('show'))) s.show = q.get('show');
  if (q.get('controls') === '1') s.controls = '1';
  return s;
}

const kidsShown = (ul) => [...ul.children].some((li) => !li.hidden);

function apply(s) {
  const words = fold(s.q).split(/\s+/).filter(Boolean);
  const filtered = words.length || s.show !== 'all' || s.controls;
  let shown = 0;
  for (const ul of lists) {
    const part = ul.closest('.fsec')?.dataset.part;
    const kids = [...ul.children].sort(SORTS[s.sort]);
    // Only what changed moves (a moved tile would lose the focus).
    kids.forEach((li, i) => { if (ul.children[i] !== li) ul.insertBefore(li, ul.children[i]); });
    for (const li of kids) {
      const ok = !!((!words.length || words.every((w) => key.get(li).text.includes(w)))
        && (s.show === 'all' || (s.show === 'community' ? part === 'community' : s.show === 'panel' ? li.dataset.panel : li.dataset.look))
        && (!s.controls || li.dataset.controls));
      if (!ok && li.dataset.live) li.dispatchEvent(new PointerEvent('pointerout', { bubbles: true })); // stop its play
      li.hidden = !ok;
      shown += ok ? 1 : 0;
    }
  }
  let note = false; // the empty community list's note shown
  for (const sec of document.querySelectorAll('main .fsec')) {
    const ul = sec.querySelector('ul.ftiles');
    sec.hidden = ul ? !kidsShown(ul) : !!filtered && s.show !== 'community';
    note ||= !ul && !sec.hidden;
  }
  if (none) none.hidden = shown > 0 || !filtered || note;
  form.elements.count.value = filtered ? `${shown} of ${tiles.length} Formulas` : `${tiles.length} Formulas`;
}

function write(s) {
  const q = new URLSearchParams();
  for (const k of Object.keys(DEFAULTS)) if (s[k] && s[k] !== DEFAULTS[k]) q.set(k, s[k]);
  const u = q.toString();
  history.replaceState(history.state, '', location.pathname + (u ? `?${u}` : '') + location.hash);
}

function fromForm() {
  const f = form.elements;
  return { q: f.q.value, sort: f.sort.value, show: f.show.value, controls: f.controls.checked ? '1' : '' };
}

function toForm(s) {
  const f = form.elements;
  f.q.value = s.q;
  f.sort.value = s.sort;
  f.show.value = s.show;
  f.controls.checked = !!s.controls;
}

if (form && lists.length) {
  const s = read();
  toForm(s);
  apply(s);
  form.hidden = false;
  const changed = () => { const s = fromForm(); apply(s); write(s); };
  form.addEventListener('input', changed);
  form.addEventListener('change', changed);
  form.addEventListener('submit', (e) => e.preventDefault());
  none?.querySelector('button')?.addEventListener('click', () => {
    toForm(DEFAULTS);
    changed();
    form.elements.q.focus();
  });
}
