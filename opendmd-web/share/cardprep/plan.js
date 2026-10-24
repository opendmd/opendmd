// What goes on the card, and what is still to write: pure functions over
// the zip's entries and a listing of the card, shared by the page and its
// node tests. Paths here are card-relative with no leading slash
// (`gifs/Arcade/X.gif`); playlists and cache/.failed use the card-absolute
// form (`/gifs/Arcade/X.gif`), as the player does. FAT compares names
// case-insensitively, so every lookup in a listing is lowercased.

// The playlists the player reads at the card's root, and their names in the app.
const PLAYLIST = /^filelist\.[^/]+$/i;
const PLAYLIST_NAMES = { txt: 'All files', hall: 'Halloween', xmas: 'Christmas' };
export const playlistName = (p) => PLAYLIST_NAMES[p.split('.').pop().toLowerCase()] ?? p;
// The retired pre-rendered format's playlist: the player ignores it.
const RETIRED = /^filelist\.odm\.txt$/i;
const BOOT = /^logos\/boot\.gif$/i;
const GIF = /^gifs\/[^/]+\/.+\.gif$/i;
export const ORIGINAL_REASON = 'more than 256 colours: plays as it is';
export const playsAsIs = (reason) => /^\s*more than 256 colou?rs/i.test(String(reason ?? ''));

// The card inside a zip: the folder holding `filelist.txt` beside a
// `gifs/` folder (the zip's root for the DMDoS pack; one folder down for a
// zip of a folder). null when the zip is not a card.
export function cardLayout(entries) {
  const files = entries.filter((e) => !e.dir);
  const lower = new Set(files.map((e) => e.name.toLowerCase()));
  const roots = files.map((e) => e.name).filter((n) => /(^|\/)filelist\.txt$/i.test(n))
    .map((n) => n.slice(0, n.length - 'filelist.txt'.length))
    .filter((pre) => [...lower].some((n) => n.startsWith(pre.toLowerCase() + 'gifs/')))
    .sort((a, b) => a.split('/').length - b.split('/').length);
  if (!roots.length) return null;
  const prefix = roots[0];
  const out = { prefix, gifs: [], playlists: [], boot: null, dropped: [], program: false, categories: new Set(), bytes: 0 };
  for (const e of files) {
    if (!e.name.startsWith(prefix)) continue;
    const path = e.name.slice(prefix.length);
    const item = { path, entry: e };
    if (GIF.test(path)) {
      out.gifs.push(item);
      out.categories.add(path.split('/')[1]);
    } else if (PLAYLIST.test(path) && !RETIRED.test(path)) out.playlists.push(item);
    else if (BOOT.test(path)) out.boot = item;
    else {
      out.dropped.push(path);
      if (/\.exe$/i.test(path)) out.program = true;
      continue;
    }
    out.bytes += e.usize;
  }
  // The playlists in the app's order: All files first.
  out.playlists.sort((a, b) => (b.path.toLowerCase() === 'filelist.txt') - (a.path.toLowerCase() === 'filelist.txt')
    || a.path.localeCompare(b.path));
  return out;
}

// The lines of a playlist that are paths (`/gifs/…`), lowercased, no leading slash.
export function playlistPaths(text) {
  return text.split('\n').map((l) => l.trim().replace(/^\/+/, '').toLowerCase()).filter(Boolean);
}

// GIFs in playing order: those in the playlists (All files first), then the
// rest in the zip's order. `texts`: playlist path -> text.
export function ordered(layout, texts) {
  const rank = new Map();
  for (const pl of layout.playlists) {
    for (const p of playlistPaths(texts.get(pl.path) ?? '')) if (!rank.has(p)) rank.set(p, rank.size);
  }
  const big = Number.MAX_SAFE_INTEGER;
  const gifs = layout.gifs.map((g, i) => ({ g, i, r: rank.get(g.path.toLowerCase()) ?? big }))
    .sort((a, b) => a.r - b.r || a.i - b.i).map((x) => x.g);
  return gifs;
}

// What goes last, after every GIF: the boot logo, then the playlists (a
// card without its playlists is one the page hasn't finished).
export const lastFiles = (layout) => (layout.boot ? [layout.boot, ...layout.playlists] : layout.playlists);

// How many playlist lines name a GIF the zip doesn't have.
export function missingLines(layout, texts) {
  const have = new Set(layout.gifs.map((g) => g.path.toLowerCase()));
  let n = 0;
  for (const pl of layout.playlists) {
    for (const p of playlistPaths(texts.get(pl.path) ?? '')) if (p.startsWith('gifs/') && !have.has(p)) n++;
  }
  return n;
}

// The work, given the card as it is. `have`: lowercased path -> size of
// every file found; `noCopy`: lowercased `/gifs/…` paths cache/.failed
// lists (they play as they are, or nothing can play them); `copyPath`: `/gifs/…` -> `/cache/…` or undefined (the
// main module's cachePathFor). A GIF is written unless the card has it at
// the zip's size (a 0-byte or short file is from an interrupted run); its
// lighter version is made unless the card has a non-empty one, or
// cache/.failed lists the GIF — and always when the GIF itself is (re)written, so it
// never belongs to another file.
export function planWork(files, last, have, noCopy, copyPath) {
  const items = [];
  let gifs = 0, copies = 0, already = 0;
  for (const f of files) {
    const e = f.entry;
    const writeGif = have.get(f.path.toLowerCase()) !== e.usize;
    const cp = copyPath('/' + f.path);
    const copy = cp ? cp.replace(/^\//, '') : null;
    const makeCopy = !!copy && (writeGif || (!(have.get(copy.toLowerCase()) > 0) && !noCopy.has('/' + f.path.toLowerCase())));
    if (!writeGif && !makeCopy) { already++; continue; }
    if (writeGif) gifs++;
    if (makeCopy) copies++;
    items.push({ path: f.path, copy: makeCopy ? copy : null, writeGif,
      e: { lho: e.lho, csize: e.csize, usize: e.usize, crc: e.crc, method: e.method, flags: e.flags } });
  }
  // The last files, written as they are, the same way: unless the card has them at the zip's size.
  const rest = last.filter((p) => have.get(p.path.toLowerCase()) !== p.entry.usize);
  return { items, gifs, copies, already, last: rest };
}

// cache/.failed after a run: the old text, without lines for GIFs that now
// have a lighter version (`drop`), plus a `path<TAB>reason` line for each
// GIF that got none (`add`: [path, reason]; a GIF past 256 colours plays
// as it is, ORIGINAL_REASON, as opendmd-pack and the player record it),
// once per path, sorted as opendmd-pack writes them. null when nothing
// changes.
export function failedText(old, add, drop) {
  const key = (l) => l.split('\t')[0].trim().toLowerCase();
  const dropSet = new Set([...drop].map((p) => p.toLowerCase()));
  let lines = old ? old.split('\n') : [];
  if (lines.length && lines[lines.length - 1] === '') lines.pop();
  const before = lines.length;
  lines = lines.filter((l) => !dropSet.has(key(l)));
  const known = new Set(lines.map(key));
  const fresh = new Map();
  for (const [p, why] of add) if (!known.has(p.toLowerCase())) fresh.set(p, String(why).replace(/\s+/g, ' ').trim());
  if (!fresh.size && lines.length === before) return null;
  const sorted = [...fresh].sort((a, b) => (a[0] < b[0] ? -1 : a[0] > b[0] ? 1 : 0));
  return [...lines, ...sorted.map(([p, why]) => `${p}\t${why}`)].map((l) => l + '\n').join('');
}

// The GIFs cache/.failed lists (lowercased paths).
export function failedPaths(text) {
  const out = new Set();
  for (const l of (text || '').split('\n')) {
    const p = l.split('\t')[0].trim();
    if (p) out.add(p.toLowerCase());
  }
  return out;
}

// Names at a folder's top that say "this is a card" (or OS litter on one).
const CARD_NAMES = /^(gifs|cache|sets|logos|update|odm|filelist\..*|system volume information|\$recycle\.bin|\.trashes|\.trash-\d+|\.spotlight-v100|\.fseventsd|\.ds_store|\._.*|desktop\.ini|thumbs\.db|.*\.crswap|lost\+found|\.temporaryitems|indexervolumeguid)$/i;
// The names at the top of a picked folder that don't belong on a card.
export function strangers(names) {
  return names.filter((n) => !CARD_NAMES.test(n));
}

// Bytes on a FAT32 card with 32 KB clusters (what a card the player formats uses).
export const onDisk = (n) => Math.ceil(n / 32768) * 32768;
