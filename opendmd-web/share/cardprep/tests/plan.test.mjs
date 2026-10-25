// node --test: the planner. What a zip puts on a card, and what is still to
// write on an empty card, a finished one, a half-written one (including the
// empty files an interrupted write leaves), one whose GIF is another file
// of the same name, and one whose cache/.failed already lists a GIF.

import { test } from 'node:test';
import assert from 'node:assert/strict';
import { cardLayout, ordered, lastFiles, missingLines, planWork, failedText, failedReasons, insideCard, strangers, playlistName, ORIGINAL_REASON, onDisk } from '../plan.js';

let lho = 0;
const E = (name, usize = 100) => ({ name, dir: name.endsWith('/'), method: 8, flags: 8, crc: 0, csize: usize, usize, lho: lho++ });
// The FNV-free stand-in for cachePathFor: the planner only needs a function.
const copyPath = (p) => (/^\/gifs\/[^/]+\/.+\.gif$/i.test(p) ? p.replace(/^\/gifs\/([^/]+)\//, '/cache/$1/00/') : undefined);

const PACK = [
  E('gifs/'), E('gifs/Consoles/'), E('gifs/Consoles/SNES_A.gif', 1000), E('gifs/Consoles/SNES_B.gif', 2000),
  E('gifs/Arcade/ARCADE_C.gif', 3000), E('gifs/Arcade/ARCADE_D.GIF', 4000), E('gifs/Spectrum/X.gif.mp4', 50),
  E('filelist.txt', 60), E('filelist.hall', 20), E('filelist.xmas', 20), E('filelist.odm.txt', 10), E('logos/boot.gif', 500),
  E('DMDos GUI.exe', 9000), E('icono.ico', 30), E('scan.py', 40),
];
const TEXTS = new Map([
  ['filelist.txt', '/gifs/Arcade/ARCADE_C.gif\n/gifs/Consoles/SNES_A.gif\n/gifs/Consoles/Missing.gif\n'],
  ['filelist.hall', '/gifs/Consoles/SNES_A.gif\n'],
  ['filelist.xmas', '/gifs/Nowhere/Gone.gif\r\n'],
]);

test('what the pack puts on the card, and what it leaves off', () => {
  const l = cardLayout(PACK);
  assert.equal(l.prefix, '');
  assert.deepEqual(l.gifs.map((g) => g.path), ['gifs/Consoles/SNES_A.gif', 'gifs/Consoles/SNES_B.gif', 'gifs/Arcade/ARCADE_C.gif', 'gifs/Arcade/ARCADE_D.GIF']);
  assert.deepEqual(l.playlists.map((p) => p.path), ['filelist.txt', 'filelist.hall', 'filelist.xmas']);
  assert.deepEqual(l.playlists.map((p) => playlistName(p.path)), ['All files', 'Halloween', 'Christmas']);
  assert.equal(l.boot.path, 'logos/boot.gif');
  assert.deepEqual(l.dropped.sort(), ['DMDos GUI.exe', 'filelist.odm.txt', 'gifs/Spectrum/X.gif.mp4', 'icono.ico', 'scan.py']);
  assert.equal(l.program, true);
  assert.deepEqual([...l.categories].sort(), ['Arcade', 'Consoles']);
  assert.equal(l.bytes, 1000 + 2000 + 3000 + 4000 + 60 + 20 + 20 + 500);
});

test('names no card has are left off it', () => {
  const l = cardLayout([E('filelist.txt'), E('gifs/A/ok.gif'), E('gifs/A/../../escape.gif'), E('gifs/./A/x.gif'),
    E('gifs/A//y.gif'), E('gifs/A\\z.gif'), E('gifs/A/tab\tname.gif')]);
  assert.deepEqual(l.gifs.map((g) => g.path), ['gifs/A/ok.gif']);
  assert.equal(l.dropped.length, 5);
});

test('a card one folder down in the zip; and zips that are not cards', () => {
  const wrapped = cardLayout([E('My card/filelist.txt'), E('My card/gifs/Other/A.gif'), E('notes/filelist.txt'), E('readme.txt')]);
  assert.equal(wrapped.prefix, 'My card/');
  assert.deepEqual(wrapped.gifs.map((g) => g.path), ['gifs/Other/A.gif']);
  assert.deepEqual(wrapped.playlists.map((p) => p.path), ['filelist.txt']);
  assert.equal(cardLayout([E('gifs/A/x.gif')]), null, 'no filelist.txt');
  assert.equal(cardLayout([E('filelist.txt'), E('pics/x.gif')]), null, 'no gifs/');
  assert.equal(cardLayout([E('a/filelist.txt'), E('b/gifs/A/x.gif')]), null, 'not beside each other');
});

test('playing order: All files first, then the other lists, then the rest; the boot logo and the playlists last', () => {
  const l = cardLayout(PACK);
  assert.deepEqual(ordered(l, TEXTS).map((g) => g.path),
    ['gifs/Arcade/ARCADE_C.gif', 'gifs/Consoles/SNES_A.gif', 'gifs/Consoles/SNES_B.gif', 'gifs/Arcade/ARCADE_D.GIF']);
  assert.deepEqual(lastFiles(l).map((f) => f.path), ['logos/boot.gif', 'filelist.txt', 'filelist.hall', 'filelist.xmas']);
  assert.equal(missingLines(l, TEXTS), 2);
});

test('an empty card: everything to write', () => {
  const l = cardLayout(PACK);
  const p = planWork(ordered(l, TEXTS), lastFiles(l), new Map(), new Map(), copyPath);
  assert.equal(p.items.length, 4);
  assert.equal(p.gifs, 4);
  assert.equal(p.copies, 4);
  assert.equal(p.items[0].copy, 'cache/Arcade/00/ARCADE_C.gif');
  assert.deepEqual(p.last.map((f) => f.path), ['logos/boot.gif', 'filelist.txt', 'filelist.hall', 'filelist.xmas']);
  assert.deepEqual(Object.keys(p.items[0].e).sort(), ['crc', 'csize', 'flags', 'lho', 'method', 'usize']);
});

const full = () => new Map([
  ['gifs/consoles/snes_a.gif', 1000], ['gifs/consoles/snes_b.gif', 2000], ['gifs/arcade/arcade_c.gif', 3000],
  ['gifs/arcade/arcade_d.gif', 4000], ['logos/boot.gif', 500], ['cache/consoles/00/snes_a.gif', 600],
  ['cache/consoles/00/snes_b.gif', 1200], ['cache/arcade/00/arcade_c.gif', 1800], ['cache/arcade/00/arcade_d.gif', 2400],
  ['filelist.txt', 60], ['filelist.hall', 20], ['filelist.xmas', 20],
]);

test('a finished card: nothing to write (names compared case-insensitively)', () => {
  const l = cardLayout(PACK);
  const p = planWork(ordered(l, TEXTS), lastFiles(l), full(), new Map(), copyPath);
  assert.equal(p.items.length, 0);
  assert.equal(p.last.length, 0);
  assert.equal(p.already, 4);
});

test('a half-written card: the empty and short files an interrupted run leaves are written again', () => {
  const l = cardLayout(PACK);
  const have = full();
  have.set('gifs/arcade/arcade_d.gif', 0); // created, never written
  have.delete('cache/arcade/00/arcade_d.gif');
  have.set('cache/consoles/00/snes_b.gif', 0); // the GIF is there, its lighter version empty
  have.set('gifs/arcade/arcade_c.gif', 1234); // another file of the same name
  have.delete('filelist.txt');
  have.set('logos/boot.gif', 0);
  const p = planWork(ordered(l, TEXTS), lastFiles(l), have, new Map(), copyPath);
  assert.deepEqual(p.items.map((i) => [i.path, i.writeGif, !!i.copy]), [
    ['gifs/Arcade/ARCADE_C.gif', true, true], // rewritten, so its lighter version is made again too
    ['gifs/Consoles/SNES_B.gif', false, true],
    ['gifs/Arcade/ARCADE_D.GIF', true, true],
  ]);
  assert.equal(p.gifs, 2);
  assert.equal(p.copies, 3);
  assert.deepEqual(p.last.map((x) => x.path), ['logos/boot.gif', 'filelist.txt']);
});

test('the pack copied by hand: only the lighter versions to make', () => {
  const l = cardLayout(PACK);
  const have = full();
  for (const k of [...have.keys()]) if (k.startsWith('cache/')) have.delete(k);
  const p = planWork(ordered(l, TEXTS), lastFiles(l), have, new Map(), copyPath);
  assert.equal(p.gifs, 0);
  assert.equal(p.copies, 4);
  assert.ok(p.items.every((i) => !i.writeGif));
});

test('cache/.failed: a GIF that plays as it is gets no second try; any other line is tried again; a GIF written anew is always', () => {
  const l = cardLayout(PACK);
  const have = full();
  have.delete('cache/arcade/00/arcade_c.gif');
  have.delete('cache/consoles/00/snes_b.gif');
  have.delete('cache/arcade/00/arcade_d.gif');
  const failed = failedReasons(`/gifs/Arcade/ARCADE_C.gif\t${ORIGINAL_REASON}\n/GIFS/consoles/SNES_B.gif\tinvalid LZW\n` +
    '/gifs/Arcade/ARCADE_D.GIF\ttoo large to prepare here\n');
  assert.deepEqual([...failed.keys()], ['/gifs/arcade/arcade_c.gif', '/gifs/consoles/snes_b.gif', '/gifs/arcade/arcade_d.gif']);
  let p = planWork(ordered(l, TEXTS), lastFiles(l), have, failed, copyPath);
  assert.deepEqual(p.items.map((i) => [i.path, i.writeGif, !!i.copy, i.retry]),
    [['gifs/Consoles/SNES_B.gif', false, true, true], ['gifs/Arcade/ARCADE_D.GIF', false, true, true]]);
  assert.equal(p.retries, 2);
  have.set('gifs/arcade/arcade_c.gif', 7);
  p = planWork(ordered(l, TEXTS), lastFiles(l), have, failed, copyPath);
  assert.deepEqual(p.items.map((i) => [i.path, i.writeGif, !!i.copy, i.retry]).slice(0, 1), [['gifs/Arcade/ARCADE_C.gif', true, true, false]]);
});

test('a GIF directly in gifs/ is a GIF too (no category)', () => {
  const l = cardLayout([E('filelist.txt'), E('gifs/Top.gif'), E('gifs/A/x.gif')]);
  assert.deepEqual(l.gifs.map((g) => g.path), ['gifs/Top.gif', 'gifs/A/x.gif']);
  assert.deepEqual([...l.categories], ['A']);
});

test('a picked folder named like one on a card', () => {
  assert.ok(insideCard('gifs') && insideCard('Cache') && insideCard('logos') && insideCard('sets'));
  assert.ok(!insideCard('OPENDMD') && !insideCard('\\') && !insideCard(''));
});

test('cache/.failed: lines added once, sorted as opendmd-pack writes them; lines for GIFs that now have one dropped', () => {
  assert.equal(failedText('', [], []), null);
  assert.equal(failedText('', [['/gifs/B/z.gif', ORIGINAL_REASON], ['/gifs/A/y.gif', 'bad\tdata\nhere']], []),
    `/gifs/A/y.gif\tbad data here\n/gifs/B/z.gif\t${ORIGINAL_REASON}\n`);
  const old = `/gifs/A/old.gif\tinvalid LZW\n/gifs/B/z.gif\tmore than 256 colours`;
  assert.equal(failedText(old, [['/gifs/b/Z.gif', ORIGINAL_REASON]], []), null, 'already listed (any case)');
  assert.equal(failedText('', [['/gifs/A/long.gif', 'x'.repeat(250)]], []), `/gifs/A/long.gif\t${'x'.repeat(200)}\n`, 'reasons capped at 200');
  assert.equal(failedText(old, [['/gifs/C/n.gif', ORIGINAL_REASON]], ['/GIFS/A/OLD.gif']),
    `/gifs/B/z.gif\tmore than 256 colours\n/gifs/C/n.gif\t${ORIGINAL_REASON}\n`);
});

test('a folder that is not a card', () => {
  assert.deepEqual(strangers(['gifs', 'cache', 'filelist.txt', 'filelist.hall', 'System Volume Information', '.Spotlight-V100',
    '._filelist.txt', '.DS_Store', 'logos', 'sets', 'x.gif.crswap', '.Trash-1000', 'update']), []);
  assert.deepEqual(strangers(['Documents', 'photo.jpg', 'gifs']), ['Documents', 'photo.jpg']);
});

test('space on a card with 32 KB clusters', () => {
  assert.equal(onDisk(0), 0);
  assert.equal(onDisk(1), 32768);
  assert.equal(onDisk(32768), 32768);
  assert.equal(onDisk(32769), 65536);
});
