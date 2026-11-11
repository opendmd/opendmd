// The Studio's drawing, as pixels (/studio/): every tool's shape as the
// points it lights, through `plot(x, y)` (the caller clips and keeps the
// undo step), integers only, so a check can work out the same pixels.

// Bresenham's line, both ends included.
export function line(x0, y0, x1, y1, plot) {
  const dx = Math.abs(x1 - x0), dy = -Math.abs(y1 - y0), sx = x0 < x1 ? 1 : -1, sy = y0 < y1 ? 1 : -1;
  let e = dx + dy;
  for (;;) {
    plot(x0, y0);
    if (x0 === x1 && y0 === y1) return;
    const e2 = 2 * e;
    if (e2 >= dy) { e += dy; x0 += sx; }
    if (e2 <= dx) { e += dx; y0 += sy; }
  }
}

// Shift on a line: the end moved to the nearest of the eight directions
// (flat or upright when the slope is under 2 in 5 of the other way, else 45°).
export function snap(x0, y0, x1, y1) {
  const dx = x1 - x0, dy = y1 - y0, ax = Math.abs(dx), ay = Math.abs(dy);
  if (ay * 5 < ax * 2) return [x1, y0];
  if (ax * 5 < ay * 2) return [x0, y1];
  const d = Math.max(ax, ay);
  return [x0 + Math.sign(dx) * d, y0 + Math.sign(dy) * d];
}

// Shift on a shape: a square box, as big as the drag's longer side.
export function square(x0, y0, x1, y1) {
  const d = Math.max(Math.abs(x1 - x0), Math.abs(y1 - y0));
  return [x0 + (x1 < x0 ? -d : d), y0 + (y1 < y0 ? -d : d)];
}

// A brush of size 1, 2 or 3 at (x, y): 1 the pixel, 2 it and the three to
// its right and below, 3 the pixel and the eight round it.
export function brush(size, plot) {
  const lo = size === 3 ? -1 : 0, hi = size === 1 ? 0 : 1;
  return (x, y) => { for (let j = lo; j <= hi; j++) for (let i = lo; i <= hi; i++) plot(x + i, y + j); };
}

// A rectangle over the box with these corners, its edge or filled.
export function rect(x0, y0, x1, y1, filled, plot) {
  const l = Math.min(x0, x1), r = Math.max(x0, x1), t = Math.min(y0, y1), b = Math.max(y0, y1);
  for (let y = t; y <= b; y++) {
    for (let x = l; x <= r; x++) if (filled || y === t || y === b || x === l || x === r) plot(x, y);
  }
}

// An ellipse inside the box with these corners (Alois Zingl's integer
// rasteriser, "A Rasterizing Algorithm for Drawing Curves", 2012:
// plotEllipseRect), its edge or filled (each row's span, edge to edge).
export function ellipse(x0, y0, x1, y1, filled, plot) {
  const rows = new Map();
  const put = filled ? (x, y) => {
    const s = rows.get(y);
    if (!s) rows.set(y, [x, x]); else { s[0] = Math.min(s[0], x); s[1] = Math.max(s[1], x); }
  } : plot;
  let a = Math.abs(x1 - x0);
  const b = Math.abs(y1 - y0);
  let b1 = b & 1;
  let dx = 4 * (1 - a) * b * b, dy = 4 * (b1 + 1) * a * a, err = dx + dy + b1 * a * a;
  if (x0 > x1) { x0 = x1; x1 += a; }
  if (y0 > y1) y0 = y1;
  y0 += (b + 1) >> 1;
  y1 = y0 - b1;
  a *= 8 * a;
  b1 = 8 * b * b;
  do {
    put(x1, y0); put(x0, y0); put(x0, y1); put(x1, y1);
    const e2 = 2 * err;
    if (e2 <= dy) { y0++; y1--; err += dy += a; }
    if (e2 >= dx || 2 * err > dy) { x0++; x1--; err += dx += b1; }
  } while (x0 <= x1);
  while (y0 - y1 <= b) { // a flat ellipse's tips
    put(x0 - 1, y0); put(x1 + 1, y0++); put(x0 - 1, y1); put(x1 + 1, y1--);
  }
  for (const [y, [l, r]] of rows) for (let x = l; x <= r; x++) plot(x, y);
}

// The pixels a fill at (x, y) reaches: those of its key, edge to edge (not
// corner to corner), as indices into the w x h picture `px`.
export function flood(px, w, h, x, y) {
  const from = px[y * w + x], seen = new Uint8Array(w * h), out = [], todo = [y * w + x];
  while (todo.length) {
    const i = todo.pop();
    if (seen[i] || px[i] !== from) continue;
    seen[i] = 1;
    out.push(i);
    const cx = i % w;
    if (cx > 0) todo.push(i - 1);
    if (cx < w - 1) todo.push(i + 1);
    if (i >= w) todo.push(i - w);
    if (i < w * h - w) todo.push(i + w);
  }
  return out;
}

// Shade: which pixels of a fill or a filled shape are set, by where they
// are on the panel (anchored at its (0, 0), not at the shape, so two shaded
// areas side by side line up); the others are left as they were. Light: x
// and y both even (1 in 4). Half: x + y even (every other). Heavy: all but x
// and y both odd (3 in 4).
export const SHADES = ['solid', 'light', 'half', 'heavy'];
export const shade = {
  solid: () => true,
  light: (x, y) => !(x & 1) && !(y & 1),
  half: (x, y) => !((x + y) & 1),
  heavy: (x, y) => !(x & 1 && y & 1),
};
