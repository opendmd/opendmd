// What Download GIF would write, read back into frames for the Studio's
// Import (create.js, framed): the converter's own 128x32 GIF, every frame
// composited as the player shows it, each with its time. Loaded only when
// Use in the Studio is pressed.
//
// The browser's own decoders keep no frame times and the art module's
// reader fits what it reads (a picture of 2x2 blocks would come back at
// half size), so the GIF is read here, exactly: LZW, local and global
// tables, transparency, disposal, interlace, the loop count.

export const W = 128, H = 32, MAX = 256;
const bad = (why) => { throw new Error(`That GIF could not be read (${why})`); };

// `b` (Uint8Array) -> {n, total, ms: [n], plays, px: Uint8Array(n * W * H * 4)}:
// the first MAX frames as RGBA (alpha 0: off), `total` how many it has;
// `plays` 0 forever, else how many times (no loop block: once).
export function read(b) {
  const u16 = (i) => b[i] | (b[i + 1] << 8);
  if (b.length < 13 || !/^GIF8[79]a$/.test(String.fromCharCode(...b.subarray(0, 6)))) bad('not a GIF');
  if (u16(6) !== W || u16(8) !== H) bad('not 128 × 32');
  let p = 13, gct = null, plays = 1, total = 0;
  const table = (f) => { const n = 3 << ((f & 7) + 1), t = b.subarray(p, p + n); p += n; if (t.length < n) bad('cut short'); return t; };
  if (b[10] & 0x80) gct = table(b[10]);
  const cv = new Uint8Array(W * H * 4), out = [], ms = [];
  let gce = { disp: 0, delay: 0, tr: -1 };
  const blocks = () => { const parts = []; while (p < b.length && b[p]) { parts.push(b.subarray(p + 1, p + 1 + b[p])); p += b[p] + 1; } p++; return parts; };
  while (p < b.length) {
    const c = b[p++];
    if (c === 0x3b) break;
    if (c === 0x21) {
      const l = b[p++];
      if (l === 0xf9 && b[p] >= 4) { const f = b[p + 1]; gce = { disp: (f >> 2) & 7, delay: u16(p + 2), tr: f & 1 ? b[p + 4] : -1 }; }
      const parts = blocks();
      if (l === 0xff && parts.length > 1 && String.fromCharCode(...parts[0]) === 'NETSCAPE2.0' && parts[1][0] === 1) {
        const n = parts[1][1] | (parts[1][2] << 8);
        plays = n ? n + 1 : 0;
      }
      continue;
    }
    if (c !== 0x2c) bad('damaged');
    const x0 = u16(p), y0 = u16(p + 2), fw = u16(p + 4), fh = u16(p + 6), f = b[p + 8];
    p += 9;
    if (!fw || !fh || fw * fh > W * H) bad('a frame bigger than the picture');
    const ct = f & 0x80 ? table(f) : gct;
    if (!ct) bad('no colour table');
    const min = b[p++];
    if (min < 2 || min > 8) bad('damaged');
    const idx = lzw(blocks(), min, fw * fh);
    const keep = gce.disp === 3 ? cv.slice() : null;
    // Interlaced rows come in four passes: every 8th from 0, every 8th from 4, every 4th from 2, every 2nd from 1.
    const rows = [];
    if (f & 0x40) for (const [s, d] of [[0, 8], [4, 8], [2, 4], [1, 2]]) for (let r = s; r < fh; r += d) rows.push(r);
    else for (let r = 0; r < fh; r++) rows.push(r);
    rows.forEach((r, n) => {
      const y = y0 + r;
      if (y >= H) return;
      for (let i = 0; i < fw; i++) {
        const x = x0 + i, k = idx[n * fw + i];
        if (x >= W || k === gce.tr || k * 3 + 2 >= ct.length) continue;
        const o = (y * W + x) * 4;
        cv[o] = ct[k * 3]; cv[o + 1] = ct[k * 3 + 1]; cv[o + 2] = ct[k * 3 + 2]; cv[o + 3] = 255;
      }
    });
    total++;
    // (A delay of 0 or 1 hundredths plays at 100 ms, as browsers play it.)
    if (out.length < MAX) { out.push(cv.slice()); ms.push(gce.delay > 1 ? gce.delay * 10 : 100); }
    if (gce.disp === 2) for (let y = y0; y < Math.min(H, y0 + fh); y++) cv.fill(0, (y * W + x0) * 4, (y * W + Math.min(W, x0 + fw)) * 4);
    else if (keep) cv.set(keep);
    gce = { disp: 0, delay: 0, tr: -1 };
  }
  if (!out.length) bad('no frames');
  const px = new Uint8Array(out.length * W * H * 4);
  out.forEach((fr, i) => px.set(fr, i * W * H * 4));
  return { n: out.length, total, ms, plays, px };
}

// The image data's codes to exactly `n` indices: data that ends first is refused.
function lzw(parts, min, n) {
  const out = new Uint8Array(n), clear = 1 << min, eoi = clear + 1;
  const pre = new Int32Array(4096), suf = new Uint8Array(4096), len = new Int32Array(4096);
  for (let i = 0; i < clear; i++) { pre[i] = -1; suf[i] = i; len[i] = 1; }
  let size = min + 1, next = eoi + 1, old = -1, o = 0, acc = 0, bits = 0;
  const put = (code) => { // the string of `code`, written from its end back
    let l = len[code], c = code;
    if (o + l > n) l = n - o;
    for (let j = len[code] - 1; j >= 0; j--, c = pre[c]) if (j < l) out[o + j] = suf[c];
    o += l;
  };
  const first = (code) => { while (pre[code] !== -1) code = pre[code]; return suf[code]; };
  for (const part of parts) for (const byte of part) {
    acc |= byte << bits;
    bits += 8;
    while (bits >= size) {
      const code = acc & ((1 << size) - 1);
      acc >>>= size;
      bits -= size;
      if (code === clear) { size = min + 1; next = eoi + 1; old = -1; continue; }
      if (code === eoi) { if (o < n) bad('cut short'); return out; }
      if (o >= n) return out;
      if (old === -1) { if (code >= clear) bad('damaged'); put(code); old = code; continue; }
      if (code > next || (code === next && next >= 4096)) bad('damaged');
      const k = code < next ? first(code) : first(old);
      if (next < 4096) { pre[next] = old; suf[next] = k; len[next] = len[old] + 1; next++; if (next === 1 << size && size < 12) size++; }
      put(code);
      old = code;
    }
  }
  if (o < n) bad('cut short');
  return out;
}
