// A picture pasted into the Studio (/studio/: the paste event's image, or
// the Paste button's navigator.clipboard.read()), loaded only then. Its
// size is read from its first bytes before anything decodes it: one of
// 128 x 32 or smaller comes in exactly, as a floating paste (every colour
// kept, see-through pixels off; the piece's `from` gains "Pasted"); a
// bigger one is said, with Fit it in Import; one too big to bring in at all,
// or that isn't the picture it claims to be (an SVG, a file named .png that
// isn't one), is refused with a sentence. The Studio's own copy (Copy puts
// a PNG on the system clipboard too) pastes as its own, with no "Pasted".
import { W, H } from './doc.js';

const MOST = 40e6; // pixels: past this not even Import takes it
let S = null;
export function mount(studio) { S = studio; }

// {w, h} from a PNG's, GIF's, JPEG's, WebP's or BMP's first bytes; null for anything else.
export function size(b) {
  const be = (i, n) => { let v = 0; for (let j = 0; j < n; j++) v = v * 256 + b[i + j]; return v; };
  const le = (i, n) => { let v = 0; for (let j = n - 1; j >= 0; j--) v = v * 256 + b[i + j]; return v; };
  const at = (i, s) => String.fromCharCode(...b.subarray(i, i + s.length)) === s;
  if (b.length >= 24 && be(0, 4) === 0x89504e47 && at(12, 'IHDR')) return { w: be(16, 4), h: be(20, 4) };
  if (b.length >= 10 && (at(0, 'GIF87a') || at(0, 'GIF89a'))) return { w: le(6, 2), h: le(8, 2) };
  if (b.length >= 26 && at(0, 'BM')) return { w: le(18, 4), h: Math.abs(le(22, 4) | 0) };
  if (b.length >= 30 && at(0, 'RIFF') && at(8, 'WEBP')) {
    if (at(12, 'VP8X')) return { w: 1 + le(24, 3), h: 1 + le(27, 3) };
    if (at(12, 'VP8L')) { const v = le(21, 4); return { w: 1 + (v & 0x3fff), h: 1 + ((v >>> 14) & 0x3fff) }; }
    if (at(12, 'VP8 ')) return { w: le(26, 2) & 0x3fff, h: le(28, 2) & 0x3fff };
    return null;
  }
  if (b[0] === 0xff && b[1] === 0xd8) {
    for (let i = 2; i + 9 < b.length;) {
      if (b[i] !== 0xff) return null;
      const m = b[i + 1];
      if (m === 0xff) { i++; continue; }
      if (m >= 0xc0 && m <= 0xcf && m !== 0xc4 && m !== 0xc8 && m !== 0xcc) return { w: be(i + 7, 2), h: be(i + 5, 2) };
      if (m === 0xd8 || (m >= 0xd0 && m <= 0xd7) || m === 0x01) { i += 2; continue; }
      i += 2 + be(i + 2, 2);
    }
  }
  return null;
}

// A pasted (or picked) image file: placed, offered to Import, or refused.
export async function paste(file) {
  let b = new Uint8Array(await file.slice(0, 256 << 10).arrayBuffer()), z = size(b);
  // (A JPEG's size comes after its metadata, which may be long: read on, up to 4 MB.)
  if (!z && b[0] === 0xff && b[1] === 0xd8 && file.size > b.length) z = size((b = new Uint8Array(await file.slice(0, 4 << 20).arrayBuffer())));
  if (!z || !z.w || !z.h) { S.say('That isn’t a picture the Studio can read: paste a PNG, GIF, JPEG, WebP or BMP.'); return 'refused'; }
  if (z.w * z.h > MOST) { S.say(`That picture is ${z.w.toLocaleString('en')} × ${z.h.toLocaleString('en')}: too big to bring in.`); return 'refused'; }
  if (z.w > W || z.h > H) {
    S.offer(`This picture is ${z.w} × ${z.h}, bigger than the panel.`, 'Fit it in Import', () => S.imp().then((m) => m.bring(file, 'Pasted')));
    return 'big';
  }
  let px;
  try {
    const bm = await createImageBitmap(file, { premultiplyAlpha: 'none', colorSpaceConversion: 'none' });
    if (bm.width !== z.w || bm.height !== z.h) { bm.close(); throw new Error('size'); }
    const c = new OffscreenCanvas(z.w, z.h), g = c.getContext('2d', { willReadFrequently: true });
    g.drawImage(bm, 0, 0);
    bm.close();
    px = g.getImageData(0, 0, z.w, z.h, { colorSpace: 'srgb' }).data;
  } catch {
    S.say('That picture couldn’t be read: it isn’t what it says it is.');
    return 'refused';
  }
  // Colours as the Studio's clipboard keeps them: 0xRRGGBB + 1, 0 for off.
  const c = new Uint32Array(z.w * z.h);
  for (let i = 0; i < c.length; i++) if (px[i * 4 + 3] >= 128) c[i] = ((px[i * 4] << 16) | (px[i * 4 + 1] << 8) | px[i * 4 + 2]) + 1;
  const own = S.clip && S.clip.w === z.w && S.clip.h === z.h && S.clip.c.every((v, i) => v === c[i]);
  if (!own && !S.addFrom({ external: true, note: 'Pasted' }, true)) return 'refused';
  if (!S.paste({ w: z.w, h: z.h, c })) return 'refused';
  if (!own) S.addFrom({ external: true, note: 'Pasted' });
  return 'placed';
}

// A pasted address: offered to Import.
export function address(u) {
  S.offer('That’s an address.', 'Import from this address', () => S.imp().then((m) => {
    history.pushState({ imp: 1 }, '', '#import');
    m.show(new URLSearchParams({ url: u }), true);
  }));
}
