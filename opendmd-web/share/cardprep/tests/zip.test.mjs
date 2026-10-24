// node --test opendmd-web/share/cardprep/tests/: the zip reader against
// zips built here byte by byte (data descriptors, stored and deflated
// entries, a comment, zip64 records and extra fields, UTF-8 and CP437
// names, a bad CRC, a cut-off file, a password-protected entry), and, given
// CARD_ZIP (tools/card-zip.py's card.zip), against a zip Python wrote.

import { test } from 'node:test';
import assert from 'node:assert/strict';
import { deflateRawSync } from 'node:zlib';

// A bit-at-a-time CRC-32, the reference the table-driven one is checked
// against (node:zlib's crc32 needs node 20.15+; this suite runs on 20.12).
function zcrc(bytes) {
  let c = 0xffffffff;
  for (const b of bytes) { c ^= b; for (let k = 0; k < 8; k++) c = c & 1 ? 0xedb88320 ^ (c >>> 1) : c >>> 1; }
  return ~c >>> 0;
}
import { readFileSync } from 'node:fs';
import { readZip, entryBytes, crc32, ZipError } from '../zip.js';

const u16 = (n) => [n & 0xff, (n >>> 8) & 0xff];
const u32 = (n) => [n & 0xff, (n >>> 8) & 0xff, (n >>> 16) & 0xff, (n >>> 24) & 0xff];
const u64 = (n) => [...u32(n % 0x100000000), ...u32(Math.floor(n / 0x100000000))];

// A zip, written the long way. Each file: {name, data, method = 8,
// descriptor = true, nameBytes, utf8, flags}. Options: comment, zip64 (every
// size and offset in the central directory as 0xFFFFFFFF with the real
// values in a zip64 extra field, and a zip64 end record), crcFor (override).
function makeZip(files, { comment = '', zip64 = false, crcFor = {} } = {}) {
  const out = [], cen = [];
  let off = 0;
  const push = (bytes) => { out.push(Uint8Array.from(bytes)); off += bytes.length; };
  for (const f of files) {
    const data = typeof f.data === 'string' ? new TextEncoder().encode(f.data) : f.data;
    const method = f.method ?? 8;
    const body = method === 8 ? deflateRawSync(data) : data;
    const name = f.nameBytes ?? new TextEncoder().encode(f.name);
    const desc = f.descriptor ?? true;
    const flags = (f.flags ?? 0) | (desc ? 8 : 0) | (f.utf8 ? 0x800 : 0);
    const crc = crcFor[f.name] ?? zcrc(data);
    const lho = off;
    push([...u32(0x04034b50), ...u16(20), ...u16(flags), ...u16(method), ...u16(0), ...u16(0),
      ...u32(desc ? 0 : crc), ...u32(desc ? 0 : body.length), ...u32(desc ? 0 : data.length),
      ...u16(name.length), ...u16(0), ...name]);
    push([...body]);
    if (desc) push([...u32(0x08074b50), ...u32(crc), ...u32(body.length), ...u32(data.length)]);
    const extra = zip64 ? [...u16(1), ...u16(24), ...u64(data.length), ...u64(body.length), ...u64(lho)] : [];
    cen.push([...u32(0x02014b50), ...u16(45), ...u16(45), ...u16(flags), ...u16(method), ...u16(0), ...u16(0),
      ...u32(crc), ...u32(zip64 ? 0xffffffff : body.length), ...u32(zip64 ? 0xffffffff : data.length),
      ...u16(name.length), ...u16(extra.length), ...u16(0), ...u16(0), ...u16(0), ...u32(0),
      ...u32(zip64 ? 0xffffffff : lho), ...name, ...extra]);
  }
  const cdOff = off;
  for (const c of cen) push(c);
  const cdSize = off - cdOff;
  const c = new TextEncoder().encode(comment);
  if (zip64) {
    const recOff = off;
    push([...u32(0x06064b50), ...u64(44), ...u16(45), ...u16(45), ...u32(0), ...u32(0),
      ...u64(files.length), ...u64(files.length), ...u64(cdSize), ...u64(cdOff)]);
    push([...u32(0x07064b50), ...u32(0), ...u64(recOff), ...u32(1)]);
    push([...u32(0x06054b50), ...u16(0), ...u16(0), ...u16(0xffff), ...u16(0xffff), ...u32(0xffffffff), ...u32(0xffffffff),
      ...u16(c.length), ...c]);
  } else {
    push([...u32(0x06054b50), ...u16(0), ...u16(0), ...u16(files.length), ...u16(files.length), ...u32(cdSize), ...u32(cdOff),
      ...u16(c.length), ...c]);
  }
  return new Blob(out);
}

const gifLike = (n, seed = 1) => {
  const b = new Uint8Array(n);
  let x = seed;
  for (let i = 0; i < n; i++) { x = (x * 1103515245 + 12345) >>> 0; b[i] = i % 7 === 0 ? x >>> 24 : i & 0x3f; }
  return b;
};
const text = (b) => new TextDecoder().decode(b);

test('crc32 matches the reference, at every length around the 4-byte step', () => {
  for (let n = 0; n < 40; n++) assert.equal(crc32(gifLike(n, n)), zcrc(gifLike(n, n)));
  const big = gifLike(300000, 9);
  assert.equal(crc32(big), zcrc(big));
  assert.equal(crc32(new TextEncoder().encode('123456789')), 0xcbf43926, 'the standard check value');
});

test('data descriptors: sizes and CRCs come from the central directory', async () => {
  const big = gifLike(200000);
  const zip = makeZip([{ name: 'gifs/', data: '', method: 0 }, { name: 'gifs/A/x.gif', data: big }, { name: 'filelist.txt', data: '/gifs/A/x.gif\n' }]);
  const { entries } = await readZip(zip);
  assert.deepEqual(entries.map((e) => [e.name, e.dir, e.usize]), [['gifs/', true, 0], ['gifs/A/x.gif', false, 200000], ['filelist.txt', false, 14]]);
  assert.ok(entries.every((e) => e.flags & 8));
  assert.deepEqual(await entryBytes(zip, entries[1]), big);
  assert.equal(text(await entryBytes(zip, entries[2])), '/gifs/A/x.gif\n');
});

test('stored entries, and entries without descriptors', async () => {
  const zip = makeZip([{ name: 'a.txt', data: 'stored here', method: 0 }, { name: 'b.txt', data: 'deflated, no descriptor', descriptor: false }]);
  const { entries } = await readZip(zip);
  assert.equal(text(await entryBytes(zip, entries[0])), 'stored here');
  assert.equal(text(await entryBytes(zip, entries[1])), 'deflated, no descriptor');
});

test('a comment at the end is skipped over', async () => {
  const zip = makeZip([{ name: 'a.txt', data: 'hello' }], { comment: 'PK\x05\x06 looks like a record but is a comment '.repeat(50) });
  const { entries } = await readZip(zip);
  assert.equal(text(await entryBytes(zip, entries[0])), 'hello');
});

test('zip64: the end record, the locator and the extra field', async () => {
  const data = gifLike(5000);
  const zip = makeZip([{ name: 'gifs/A/one.gif', data }, { name: 'gifs/A/two.gif', data: 'two', method: 0 }], { zip64: true });
  const { entries } = await readZip(zip);
  assert.equal(entries.length, 2);
  assert.equal(entries[0].usize, 5000);
  assert.deepEqual(await entryBytes(zip, entries[0]), data);
  assert.equal(text(await entryBytes(zip, entries[1])), 'two');
});

test('names: UTF-8 with flag bit 11, CP437 without', async () => {
  const zip = makeZip([
    { name: 'gifs/Ñ/café.gif', data: 'x', utf8: true },
    { name: 'cp437', nameBytes: Uint8Array.from([0x67, 0x2f, 0x82, 0x94, 0xe1, 0xff]), data: 'y' },
  ]);
  const { entries } = await readZip(zip);
  assert.equal(entries[0].name, 'gifs/Ñ/café.gif');
  assert.equal(entries[1].name, 'g/éöß ');
});

test('a bad CRC is the entry\'s error, not the zip\'s', async () => {
  const zip = makeZip([{ name: 'good.gif', data: 'fine' }, { name: 'bad.gif', data: 'broken' }], { crcFor: { 'bad.gif': 1234 } });
  const { entries } = await readZip(zip);
  assert.equal(text(await entryBytes(zip, entries[0])), 'fine');
  await assert.rejects(entryBytes(zip, entries[1]), (e) => e instanceof ZipError && /damaged/.test(e.message) && e.kind === 'damaged');
});

test('damaged deflate data is refused', async () => {
  const zip = makeZip([{ name: 'x.gif', data: gifLike(10000) }]);
  const bytes = new Uint8Array(await zip.arrayBuffer());
  for (let i = 60; i < 400; i += 3) bytes[i] ^= 0x5a;
  const broken = new Blob([bytes]);
  const { entries } = await readZip(broken);
  await assert.rejects(entryBytes(broken, entries[0]), ZipError);
});

test('a cut-off download, and not a zip at all', async () => {
  const zip = makeZip([{ name: 'x.gif', data: gifLike(10000) }]);
  await assert.rejects(readZip(zip.slice(0, zip.size - 30)), ZipError);
  await assert.rejects(readZip(zip.slice(0, Math.floor(zip.size / 2))), ZipError);
  await assert.rejects(readZip(new Blob(['GIF89a not a zip at all, just some text'])), ZipError);
  await assert.rejects(readZip(new Blob([])), ZipError);
});

test('password-protected and unknown methods are refused per entry', async () => {
  const zip = makeZip([{ name: 'locked.gif', data: 'x', flags: 1 }, { name: 'ok.gif', data: 'ok' }]);
  const { entries } = await readZip(zip);
  assert.equal(entries[0].encrypted, true);
  await assert.rejects(entryBytes(zip, entries[0]), (e) => /password/.test(e.message) && e.kind === 'unsupported');
  assert.equal(text(await entryBytes(zip, entries[1])), 'ok');
  await assert.rejects(entryBytes(zip, { ...entries[1], method: 9 }), (e) => /method 9/.test(e.message) && e.kind === 'unsupported');
});

test('the fixture zip tools/card-zip.py writes (CARD_ZIP)', { skip: !process.env.CARD_ZIP }, async () => {
  const zip = new Blob([readFileSync(process.env.CARD_ZIP)]);
  const { entries } = await readZip(zip);
  assert.ok(entries.filter((e) => !e.dir).every((e) => e.flags & 8), 'every entry has a data descriptor');
  for (const e of entries.filter((x) => !x.dir)) {
    const b = await entryBytes(zip, e);
    assert.equal(b.length, e.usize, e.name);
  }
  assert.ok(entries.some((e) => e.method === 0) && entries.some((e) => e.method === 8));
});
