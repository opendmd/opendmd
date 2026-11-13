// The web app's panel drawn for the home page's screenshots whose panel
// plays (home.js imports it when the first of them nears the screen;
// tools/site.py copies it to the site's root): the same LEDs as in the
// shot, so a Formula takes over from the still unseen. As the web app's
// preview draws its panel (opendmd-render's LedRenderer, at the web app's
// look: gamma 2.2, gain 1, the default
// bloom): each LED a small lit dot with a soft glow, added in linear
// light over a near-black mask with the unlit packages a shade lighter,
// `r` canvas pixels a LED. Decoration: the screenshot's alt text says
// what plays. {led, cv, draw(rgb)}: draw a 128 x 32 RGB frame.
export function leds(r) {
  const W = 128 * r, H = 32 * r, k = 2 * r, o = (r >> 1) - r, c = r / 2 - o;
  const rx = 0.42 * r / 2, ry = rx * 1.1, bloom = 0.75 * r, g = 2.2;
  const ker = new Float32Array(k * k);
  for (let j = 0; j < k; j++) {
    for (let i = 0; i < k; i++) {
      const dx = i + 0.5 - c, dy = j + 0.5 - c, d = Math.hypot(dx, dy), n = Math.hypot(dx / rx, dy / ry);
      const core = n <= 1.2e-7 ? 1 : Math.min(1, Math.max(0, 0.5 + (d / n - d)));
      const t = d / bloom, halo = d < bloom ? 0.22 * (1 - t) * (1 - t) : 0;
      ker[j * k + i] = core + halo * (1 - core);
    }
  }
  const mask = (5 / 255) ** g, extra = (10 / 255) ** g - mask, back = new Float32Array(W * H * 3).fill(mask);
  const pkg = Math.min(r, Math.max(1, Math.round(0.5 * r))), pad = (r - pkg) >> 1;
  for (let y = 0; y < 32; y++) {
    for (let j = 0; j < pkg; j++) {
      const row = (y * r + pad + j) * W;
      for (let x = 0; x < 128; x++) for (let i = 0; i < pkg; i++) back.fill(mask + extra, (row + x * r + pad + i) * 3, (row + x * r + pad + i) * 3 + 3);
    }
  }
  const dec = Float32Array.from({ length: 256 }, (_, v) => (v / 255) ** g);
  const enc = Uint8Array.from({ length: 4097 }, (_, i) => Math.floor((i / 4096) ** (1 / g) * 255 + 0.5));
  const acc = new Float32Array(W * H * 3);
  const cv = Object.assign(document.createElement('canvas'), { width: W, height: H });
  cv.setAttribute('aria-hidden', 'true');
  const led = Object.assign(document.createElement('span'), { className: 'lit' });
  led.append(cv);
  const ctx = cv.getContext('2d'), img = ctx.createImageData(W, H), out = img.data;
  const draw = (rgb) => {
    acc.set(back);
    for (let y = 0; y < 32; y++) {
      for (let x = 0; x < 128; x++) {
        const p = (y * 128 + x) * 3, R = dec[rgb[p]], G = dec[rgb[p + 1]], B = dec[rgb[p + 2]];
        if (!(R || G || B)) continue;
        const ox = x * r + o, oy = y * r + o;
        const i0 = Math.max(0, -ox), i1 = Math.min(k, W - ox), j0 = Math.max(0, -oy), j1 = Math.min(k, H - oy);
        for (let j = j0; j < j1; j++) {
          let a = ((oy + j) * W + ox + i0) * 3, q = j * k + i0;
          for (let i = i0; i < i1; i++, a += 3, q++) {
            const v = ker[q];
            acc[a] += R * v; acc[a + 1] += G * v; acc[a + 2] += B * v;
          }
        }
      }
    }
    for (let a = 0, b = 0; b < out.length; a += 3, b += 4) {
      out[b] = enc[(Math.min(1, acc[a]) * 4096) | 0];
      out[b + 1] = enc[(Math.min(1, acc[a + 1]) * 4096) | 0];
      out[b + 2] = enc[(Math.min(1, acc[a + 2]) * 4096) | 0];
      out[b + 3] = 255;
    }
    ctx.putImageData(img, 0, 0);
    cv.dataset.frames = -~cv.dataset.frames;
  };
  return { led, cv, draw };
}
