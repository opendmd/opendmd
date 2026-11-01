# Psi in brief (for models)

Psi draws a 128 × 32 LED panel, 50 frames a second. A **formula** is a
few lines of maths worked out for every pixel; its **last line is the
colour**. The full reference is `docs/psi-for-models.md`; this card is
what you need for most formulas. Check with `psi check FILE`.

## The pixel, the time, the colour

- `x`, `y`: 0 to 1 across and **down** (y = 0 is the top row). `px` 0..127,
  `py` 0..31 are the same in pixels; `i` = `py*128 + px`. `cx`, `cy` are
  centred (`cx` −2..2, `cy` −0.5..0.5, square units), `r` = distance from
  the middle, `angle` its angle in turns. `w`, `h` = 128, 32.
- `t`: seconds into the loop. `loop 8` (the default) repeats every 8 s; make
  motion repeat with it: `sin(t/8)`, `frac(t/4)`. `loop forever` never
  repeats (then `seed` is new each play).
- **Angles and waves are turns, not radians**: `sin(v)` goes round once as
  `v` goes 0 → 1. No pi: `sin(x)` is one wave across, `sin(t/8)` one a loop.
  `sin cos tri saw sqr` are −1..1 waves.
- The clock: `hour`, `minute`, `second`. Sliders: `a`, `b`, `c` (or names
  from `control` lines).
- Colour line, one of: `rgb(r, g, b)` (0..1 each), `hsv(h, s, v)` (h in
  turns), `gray(v)`, `pal(P, v)` (palettes `fire ice ocean chrono amber
  rainbow gray`), `cycle(P, v, speed)`, a colour expression with `#rrggbb`
  or `#rgb` in it (`#f80 * k`, `mix(#102040, #ff8800, y)`, `c ? #fff : #444`),
  or a bare value (its gray). Each colour takes one more value, how much of
  the pixel it covers: `rgb(r, g, b, k)`.
- Functions: `abs floor frac sqrt min max clamp mix step smoothstep len
  atan2 hash noise select pick bit`; `%` is the remainder; `c ? a : b`;
  comparisons give 1 or 0, with `and or not`. `noise(x, y, z)` smooth
  hills 0..1; `hash(a, b)` random 0..1; `fract` is `frac`. Text:
  `digit(n, col, row)`, `glyph('A', col, row)` (font cells 5 × 7; a last
  value `small`, or `bold` for digits; `normal` by default).
- Names: `v = …` sets a name; set it again to change it. Numbers only:
  a colour can't be kept in a name (write a function instead).

```psi
loop 8
-- plasma: three waves, hue turning with time
v = sin(x*2 + t/8) + sin(y*1.5 - t/4) + sin(r*2 - t/8)
hsv(v/6 + t/8, 0.9, 1)
```

## Functions

`fn name(a, b) = value`, or `fn name(a, b) { … }` (names set, then its
last line: a value or a colour). Each call is written out where it is.
A function whose values are named `x, y` draws elsewhere: inside it `px`,
`py`, `r` … follow them.

```psi
loop 8
fn sky(v) = mix(#102040, #ff8040, v)
fn ring(x, y) {
  d = len((x - 0.5)*4, y - 0.5)
  pal(ice, frac(d*3 - t/4))
}
-- the ring folded into four mirrored quarters
ring(abs(x - 0.5)*2, abs(y - 0.5)*2)
```

## Scenes: drawing once a frame

`draw { … }` runs once a frame, before the pixels, in **pixel
coordinates** (x 0..127, y 0..31 down). Paint in colours — the last
painted wins and it shows over everything:
`cls(#col)`, `pset(x, y, #col)`, `line(x0, y0, x1, y1, #col)`,
`rect(x0, y0, x1, y1, #col)`, `rectfill(…)`, `circ(x, y, r, #col)`,
`circfill(…)`, `trifill(x0, y0, x1, y1, x2, y2, #col)`,
`spr(S, x, y)` (a sprite with colours in its key), `print("TEXT", x, y, #col)`.
Colours in draw are `#…`, `pal(P, v)` or `gray(v)`.
`for k < 12 { … }` loops (a written number; two deep), `if c { … } else
{ … }`. Values in draw are the same for every pixel (no `x`, `px`).
Without a colour line, the paint goes over black (or over the GIF).
`line(…)`/`dot(…)` with a number instead of a colour draw *ink*, read by
the pixels as `ink` (0..1) for glows.

```psi
loop 4
fn palm(x0, s) {
  line(x0, 31, x0, 31 - 10*s, #a62)
  circfill(x0, 31 - 10*s, 3*s, #2a4)
}
draw {
  cls(#102040)
  circfill(100, 8, 5, #ffd040)
  for k < 4 {
    palm(12 + k*30 + sin(t/4 + k/4)*3, 1 + k/4)
  }
  rectfill(0, 29, 127, 31, #d0a060)
}
```

## Over a GIF

A formula with `look` at its top may be drawn over every GIF. The picture
under it: `ur ug ub` (0..1) and `ul` (brightness) at the pixel;
`under(x, y)` the picture at another pixel, as a colour; a coverage below
1, or a `layer add { … }` / `screen` / `multiply` blend, lets it show.
`look screen 40` says how it goes on (blend, amount).

```psi
look
loop 2
-- the GIF shaken a pixel or two, a little dimmed at the edges
v = 1 - abs(cx)*0.15
under(px + floor(sin(t*2)*2), py) * v
```

## Layers, sliders, sprites

- `layer [over|add|multiply|screen|max|min] { … colour }` stack bottom to top.
- At the top: `name "Sunset"`, `about "…"`, `control speed 0..4 = 1
  "Speed"` (a slider, read as `speed`), `look`.
- `fps 100` for fast motion (light formulas only); `layer rate 4 { … }` a
  slow bottom layer redrawn every 4th frame.
- `table T = 0.2, 0.9, 0.5` numbers, read as `T[k]`.
- `sprite S { key . 0, R #f00, W #fff … rows }`; in draw `spr(S, x, y)`.
- `palette P = #000, #f80, #fff` your own palette for `pal(P, v)`.

## Cost: keep it light

The panel has ~8 ms a frame (12 for a look over a GIF). `psi check` says
the cost and names the heaviest lines. Per pixel, `noise`, `atan2`, `len`
and `/` cost most; a value of only `t`, the clock and sliders is worked out
once a frame, of `y` once a row — free. Drawing costs by the shapes, not the
pixels: prefer `draw` for scenes (objects, sprites, text) and per-pixel
maths for fields (plasma, fire, sky).

## Mistakes to avoid

- There are no parts: write a function. Radians: `sin(x*6.28)` is six waves, not one. No `pi`, `pow`, `vec3`,
  `iTime`, `fragCoord`, `uv`: use `t`, `px`/`x`, `x*x`, `rgb(…)`.
- A colour in a name or a number slot: `c = #f80` and `hsv(#f80)` are
  errors; write the colour where it is used, or a function.
- `if` outside draw: write `c ? a : b`.
- y counts **down**: the sky is `y < 0.5`, the ground `py > 24`.
- Whole colours (`hsv pal gray`) don't go into sums: `hsv(…) * 0.5` is an
  error; give it a lower value instead, or use `#…`/`rgb` to mix.

```psi error
-- error: unknown function `pow` — Psi has no `pow`
gray(pow(x, 2))
```
