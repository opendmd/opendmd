// Browser port of process/art.py: the pieces the Studio needs, on
// ImageData-like {width, height, data: Uint8ClampedArray RGBA}.
// Throwaway prototype — the real thing would move the heavy parts to wasm.

export const W = 128, H = 32;

export function img(width, height) { return { width, height, data: new Uint8ClampedArray(width * height * 4) }; }

// Undo a nearest-neighbour integer upscale: where strong colour edges fall,
// modulo k. Returns {k, px, py}.
export function detectScale(im, kmax = 8, thr = 60) {
  const axis = (horizontal) => {
    const n = horizontal ? im.width : im.height, m = horizontal ? im.height : im.width;
    const e = new Float64Array(n);
    const d = im.data;
    for (let i = 1; i < n; i++) for (let j = 0; j < m; j += 1) {
      const a = horizontal ? (j * im.width + i) * 4 : (i * im.width + j) * 4;
      const b = horizontal ? a - 4 : a - im.width * 4;
      if (Math.abs(d[a] - d[b]) + Math.abs(d[a + 1] - d[b + 1]) + Math.abs(d[a + 2] - d[b + 2]) > thr) e[i]++;
    }
    const total = e.reduce((s, v) => s + v, 0);
    if (total < 8) return [1, 0];
    for (let k = kmax; k > 1; k--) {
      const h = new Float64Array(k);
      for (let i = 0; i < n; i++) h[i % k] += e[i];
      const ph = h.indexOf(Math.max(...h));
      if (h[ph] / total > 0.8) return [k, ph];
    }
    return [1, 0];
  };
  const [kx, px] = axis(true), [ky, py] = axis(false);
  const k = kx > 1 && ky > 1 ? Math.min(kx, ky) : 1;
  return { k, px: px % k, py: py % k };
}

export function native(im) {
  const { k, px, py } = detectScale(im);
  if (k === 1) return { im, k };
  const w = Math.floor((im.width - px) / k), h = Math.floor((im.height - py) / k);
  const out = img(w, h);
  for (let y = 0; y < h; y++) for (let x = 0; x < w; x++) {
    const s = ((py + y * k + (k >> 1)) * im.width + px + x * k + (k >> 1)) * 4;
    out.data.set(im.data.subarray(s, s + 4), (y * w + x) * 4);
  }
  return { im: out, k };
}

// Interest per pixel: luma gradient + "not the dominant colour".
function detail(im) {
  const { width: w, height: h, data: d } = im;
  const counts = new Map();
  for (let i = 0; i < d.length; i += 4) { const key = (d[i] >> 4) << 8 | (d[i + 1] >> 4) << 4 | d[i + 2] >> 4; counts.set(key, (counts.get(key) || 0) + 1); }
  let bg = 0, best = -1;
  for (const [k, v] of counts) if (v > best) { best = v; bg = k; }
  const L = (i) => 0.299 * d[i] + 0.587 * d[i + 1] + 0.114 * d[i + 2];
  const out = new Float32Array(w * h);
  for (let y = 0; y < h; y++) for (let x = 0; x < w; x++) {
    const i = (y * w + x) * 4;
    if (d[i + 3] === 0) continue;
    const gx = x + 1 < w ? Math.abs(L(i + 4) - L(i)) : 0, gy = y + 1 < h ? Math.abs(L(i + w * 4) - L(i)) : 0;
    const key = (d[i] >> 4) << 8 | (d[i + 1] >> 4) << 4 | d[i + 2] >> 4;
    out[y * w + x] = Math.min(gx + gy, 96) + (key !== bg ? 24 : 0);
  }
  return out;
}

// Top-left of the w x h window with the most detail (integral image).
export function bestWindow(im, w, h, bias = 0.15) {
  w = Math.min(w, im.width); h = Math.min(h, im.height);
  const d = detail(im), iw = im.width + 1;
  const S = new Float64Array(iw * (im.height + 1));
  for (let y = 0; y < im.height; y++) for (let x = 0; x < im.width; x++)
    S[(y + 1) * iw + x + 1] = d[y * im.width + x] + S[y * iw + x + 1] + S[(y + 1) * iw + x] - S[y * iw + x];
  let best = -1, bx = 0, by = 0;
  const cx = (im.width - w) / 2, cy = (im.height - h) / 2;
  for (let y = 0; y + h <= im.height; y++) for (let x = 0; x + w <= im.width; x++) {
    const t = S[(y + h) * iw + x + w] - S[y * iw + x + w] - S[(y + h) * iw + x] + S[y * iw + x];
    const s = t * (1 - bias * Math.hypot((x - cx) / im.width, (y - cy) / im.height));
    if (s > best) { best = s; bx = x; by = y; }
  }
  return { x: bx, y: by };
}

export function crop(im, x, y, w, h, bg = [0, 0, 0]) {
  const out = img(w, h);
  for (let yy = 0; yy < h; yy++) for (let xx = 0; xx < w; xx++) {
    const sx = x + xx, sy = y + yy, o = (yy * w + xx) * 4;
    if (sx >= 0 && sy >= 0 && sx < im.width && sy < im.height) {
      const s = (sy * im.width + sx) * 4, a = im.data[s + 3] / 255;
      for (let c = 0; c < 3; c++) out.data[o + c] = im.data[s + c] * a + bg[c] * (1 - a);
    } else for (let c = 0; c < 3; c++) out.data[o + c] = bg[c];
    out.data[o + 3] = 255;
  }
  return out;
}

// Pixel-art-aware integer downscale: the most common colour of each block.
export function downMode(im, f) {
  const w = Math.floor(im.width / f), h = Math.floor(im.height / f), out = img(w, h);
  for (let y = 0; y < h; y++) for (let x = 0; x < w; x++) {
    const m = new Map();
    let best = 0, bestC = 0;
    for (let j = 0; j < f; j++) for (let i = 0; i < f; i++) {
      const s = ((y * f + j) * im.width + x * f + i) * 4, d = im.data;
      const c = d[s] << 24 | d[s + 1] << 16 | d[s + 2] << 8 | d[s + 3];
      const n = (m.get(c) || 0) + 1; m.set(c, n);
      if (n > best) { best = n; bestC = c; }
    }
    const o = (y * w + x) * 4;
    out.data[o] = bestC >>> 24; out.data[o + 1] = bestC >>> 16 & 255; out.data[o + 2] = bestC >>> 8 & 255; out.data[o + 3] = bestC & 255;
  }
  return out;
}

// Smooth sources (logos, photos): the browser's own area-ish resample.
export function resizeSmooth(bitmapOrCanvas, w, h) {
  const c = new OffscreenCanvas(w, h), g = c.getContext('2d');
  g.imageSmoothingEnabled = true; g.imageSmoothingQuality = 'high';
  g.drawImage(bitmapOrCanvas, 0, 0, w, h);
  const d = g.getImageData(0, 0, w, h);
  return { width: w, height: h, data: d.data };
}

// The palette for a whole animation: exact when it fits in `max`, else
// `max` of the colours used, most-used first, then whichever is furthest
// from everything picked (weighted by log uses) — keeps rare accents that
// median cut / NeuQuant drop. Returns {palette: [[r,g,b]], index(frame) -> Uint8Array}.
export function buildPalette(frames, max = 256) {
  const counts = new Map();
  for (const f of frames) for (let i = 0; i < f.data.length; i += 4) {
    const c = f.data[i] << 16 | f.data[i + 1] << 8 | f.data[i + 2];
    counts.set(c, (counts.get(c) || 0) + 1);
  }
  let cols = [...counts];
  let pal;
  if (cols.length <= max) pal = cols.map(([c]) => c);
  else {
    const rgb = cols.map(([c]) => [c >> 16, c >> 8 & 255, c & 255]), wgt = cols.map(([, n]) => Math.log1p(n));
    let first = 0;
    cols.forEach(([, n], i) => { if (n > cols[first][1]) first = i; });
    const picked = [first], dmin = rgb.map((p) => dist2(p, rgb[first]));
    while (picked.length < max) {
      let bi = 0, bv = -1;
      for (let i = 0; i < rgb.length; i++) { const v = Math.sqrt(dmin[i]) * wgt[i]; if (v > bv) { bv = v; bi = i; } }
      picked.push(bi);
      for (let i = 0; i < rgb.length; i++) dmin[i] = Math.min(dmin[i], dist2(rgb[i], rgb[bi]));
    }
    pal = picked.map((i) => cols[i][0]);
  }
  const palette = pal.map((c) => [c >> 16, c >> 8 & 255, c & 255]);
  const lut = new Map();
  const nearest = (c) => {
    let v = lut.get(c);
    if (v === undefined) {
      const p = [c >> 16, c >> 8 & 255, c & 255];
      let bd = Infinity; v = 0;
      palette.forEach((q, i) => { const dd = dist2(p, q); if (dd < bd) { bd = dd; v = i; } });
      lut.set(c, v);
    }
    return v;
  };
  const index = (f) => {
    const out = new Uint8Array(f.width * f.height);
    for (let i = 0, j = 0; i < f.data.length; i += 4, j++) out[j] = nearest(f.data[i] << 16 | f.data[i + 1] << 8 | f.data[i + 2]);
    return out;
  };
  return { palette, index };
}
const dist2 = (a, b) => (a[0] - b[0]) ** 2 + (a[1] - b[1]) ** 2 + (a[2] - b[2]) ** 2;

// Frames for a framing mode over a prepared source image (already native
// pixels or already fitted). mode: 'still' | 'pan' (left->right, ping-pong)
// | 'scroll' (top->bottom, 1.5 s holds at the ends, like the pack).
export function frames(src, mode, at, stepMs = 40) {
  if (mode === 'still') return [{ im: crop(src, at.x, at.y, W, H), delay: 5000 }];
  const out = [];
  if (mode === 'pan') {
    const span = Math.max(0, src.width - W);
    const xs = [...Array(span + 1).keys(), ...[...Array(Math.max(0, span - 1)).keys()].map((i) => span - 1 - i)];
    for (const x of xs) out.push({ im: crop(src, x, at.y, W, H), delay: stepMs });
  } else {
    const span = Math.max(0, src.height - H);
    for (let y = 0; y <= span; y++) out.push({ im: crop(src, at.x, y, W, H), delay: stepMs });
  }
  out[0].delay = 1500;
  out[out.length - 1].delay = 1500;
  return out;
}

// Card file name per the tag grammar: PLATFORM_Title_Words_NN.gif.
export function fileName(platform, title, n) {
  const words = title.normalize('NFKD').replace(/[^\p{L}\p{N} ]+/gu, ' ').trim().split(/\s+/).filter(Boolean)
    .map((w) => w[0].toUpperCase() + w.slice(1));
  const plat = platform.toUpperCase().replace(/[^A-Z0-9-]/g, '');
  return `${plat}_${words.join('_')}_${String(n).padStart(2, '0')}.gif`;
}
