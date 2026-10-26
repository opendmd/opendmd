# Psi for models: writing Formulas

A compact, exact reference for a language model (or a person) writing
**Formulas** in **Psi**, OpenDMD's language for a 128 × 32 LED panel. Every
code block marked `psi` below is compiled by the tests
(`opendmd-psi/tests/models.rs`); every worked example's frames are checked
against golden hashes, so this document cannot drift from the language.
Users see the words *Formula* and *GIF*; Psi is the language's name.

**The feedback loop.** Write a candidate, then run

```text
make psi-check F=candidate.psi [ARGS="--out sheet.png --frames 8 --a 0.5"]
```

It prints `ok: …` (steps, layers, registers, values), the loop and what it reads,
and `panel: ~N ms a frame: light|medium|heavy`, and writes a PNG contact
sheet of frames across one loop (over a sample picture — sky, sun, hills —
when the formula draws over what is under it). On a compile error it prints
`error: line L, column C: <sentence>`, the line and carets, and fails:
the tool itself exits 1 (2 for a missing file or a bad option), and
`make psi-check` then exits 2, as make does for any failed command.
Look at the sheet: the numbers say it runs, only the picture says it is
right. Aim for **≤ 8 ms** (light or medium).

## 1. The model

A formula is evaluated **for every pixel, every frame** (50 frames a
second). It is a few lines of arithmetic ending in a colour. There are no
loops, arrays, strings, recursion or state between frames: each frame is a
pure function of the pixel's position, the time, the clock, three sliders,
and (optionally) the picture under it.

Coordinates: `x` is 0 at the left column to 1 at the right (`x = px/127`);
`y` is 0 at the **bottom** row to 1 at the top (`y = py/31`). The panel is
4 times wider than tall, so the built-ins about the middle measure in panel
heights: `cx` is −2 at the left edge to 2 at the right, `cy` −0.5 at the
bottom to 0.5 at the top, and `r` (the distance from the middle) and
`angle` are round: `r < 0.4` is a disc, not an ellipse. Or work in pixels
(`len(px - 63.5, py - 15.5)`). `ty` is the row counted from the top (0 to
31), as the fonts count.

Angles and waves are in **turns**: `sin(0.25)` is 1; `sin(v)` repeats every 1.
Time `t` is seconds into the loop, always a whole number of 20 ms frames.

## 2. Grammar

```text
formula   = { sep } { stmt sep } colourline { sep }      -- or layers instead of colourline
stmt      = "loop" NUMBER                     -- 0.1..3600 s, once, default 8
          | "frame" "{" { assign sep } "}"    -- values once a frame (no x, y, px, py, i, ur…)
          | NAME "=" expr                     -- each name set once in its scope
          | "layer" [BLEND] "{" { assign | frameblock } colourline "}"
          | "part" NAME "{" formula-body "}"  -- another formula inside this one
colourline= "rgb" "(" e "," e "," e [ "," cover ] ")"
          | "rgb" "(" ce [ "," cover ] ")"       -- a colour, maybe covering less
          | "hsv" "(" e "," e "," e [ "," cover ] ")"
          | "pal" "(" PALETTE "," e [ "," cover ] ")"
          | "gray" "(" e [ "," cover ] ")"
          | PART [ "(" e "," e [ "," cover ] ")" ]   -- a part's colour, maybe elsewhere
          | ce                                 -- a colour expression (see 6)
          | e                                  -- a bare value: gray(e)
ce        = an expr with a COLOUR or an "rgb(" e "," e "," e ")" in it, read
            once a channel
expr      = or [ "?" expr ":" expr ]           -- both arms are always evaluated
or        = and { ("or" | "||") and }
and       = not { ("and" | "&&") not }
not       = ("not" | "!") not | cmp
cmp       = sum { ("<" | "<=" | ">" | ">=" | "==" | "!=") sum }
sum       = term { ("+" | "-") term }
term      = unary { ("*" | "/" | "%") unary }
unary     = ("-" | "+") unary | atom
atom      = NUMBER | CHAR | COLOUR | NAME | FUNC "(" args ")" | PART "." NAME
          | PART "(" e "," e ")" "." NAME | "(" expr ")"
args      = e { "," e }                        -- digit/glyph: e "," e "," e [ "," FONT ]
                                               -- pick: e "," e { "," e }
sep       = newline | ";"                      -- newlines inside ( ) are spaces
NUMBER    = digits [ "." digits ] | "." digits -- no sign, no exponent
CHAR      = "'" one printable ASCII character "'"   -- its code: 'A' is 65
COLOUR    = "#" 6 hex digits | "#" 3 hex digits   -- #ff8800, #f80 (each digit twice); colour lines only
FONT      = "normal" | "small" | "bold"      -- bold: digit only
NAME      = [A-Za-z_][A-Za-z0-9_]*
comment   = "--" or "//" to the end of the line
```

`and`/`or` bind looser than comparisons, `not` looser than comparisons too
(`not x < 0.5` is `not (x < 0.5)`). An expression continues onto the next
line after an operator or inside brackets. The colour line must be the last
line (of the formula, of a part, or of a layer). With `layer` blocks, every
colour goes inside a layer.

## 3. Numbers

Q16.16 fixed point: −32,768 to 32,767.99998, steps of 1/65,536. Arithmetic
**saturates** (no wrap-around). `a / 0` is the largest number of `a`'s sign
(0 for 0/0); `a % 0` is 0; a character in single quotes is its ASCII code
(`'A'` is 65, `'0'` 48, `' '` 32; one character, no strings); `%` is floored (`-1 % 3` is 2; `v % 1` is
`frac(v)`). Comparisons and logic give 1 or 0; any non-zero is true.
Division by a constant is exact once a frame or a row (`floor(minute/10)`
is 5 from 50 to 59); per pixel it is a multiply by 1/k, which `floor`
takes rounded up, so `floor(n/k)` is exact for whole numbers with
n·k < 65,536 (`floor(px/10)`, `floor(i/12)`); elsewhere `px/3` may be off
by a step or two of 1/65,536. The same formula gives the same bytes on the panel, in the browser and on a
computer.

## 4. Values

| name | meaning | varies per |
|---|---|---|
| `x`, `y` | 0..1 across (left→right) and up (bottom→top) | pixel / row |
| `px`, `py` | column 0..127, row 0..31 from the bottom (whole numbers) | pixel / row |
| `i` | pixel number 0..4095 from the top left: `(31 − py)·128 + px` | pixel |
| `t` | seconds into the loop (whole 20 ms frames) | frame |
| `hour`, `minute`, `second` | the clock: 0..23, 0..59, 0..59 (whole) | frame |
| `a`, `b`, `c` | sliders, 0..1; a comment `-- a: speed` names one | frame |
| `w`, `h` | 128 and 32 (constants) | — |
| `ur`, `ug`, `ub` | red, green, blue of the picture under the formula, 0..1 | pixel |
| `ul` | its brightness (Rec. 601: `(77R + 150G + 29B + 128)/256`), 0..1 | pixel |
| `cx` | across from the middle in panel heights, −2..2: `(x − 0.5)·4` | pixel |
| `cy` | up from the middle, −0.5..0.5: `y − 0.5` | row |
| `r` | the distance from the middle, in panel heights: `len(cx, cy)` (0.5 touches the top and bottom) | pixel |
| `angle` | the angle about the middle in turns, −½..½, 0 to the right, ¼ up: `atan2(cy, cx)` | pixel |
| `ty` | the row from the **top**, 0..31 (whole numbers): `31 − py` | row |

`cx cy r angle ty` are exactly the formulas after their colons: the same
steps, the same cost, the same bytes. Alone (not over a GIF), `ur ug ub ul`
are 0. Built-in names, function names, palette names and keywords (`loop
frame layer part use`) cannot be set (except `digit`, `glyph`, `bit`,
`pick`, `cx`, `cy`, `r`, `angle` and `ty`, added later, so formulas saved
with those names still work: a name a formula sets is that name; don't use
them as names in new ones).

## 5. Functions

| function | result |
|---|---|
| `sin(v)`, `cos(v)` | −1..1, period 1 (turns); a 256-entry table, interpolated |
| `tri(v)` | triangle in step with sin: 0 at 0, 1 at ¼, 0 at ½, −1 at ¾ |
| `saw(v)` | −1 at 0 rising to 1 at the end of each turn |
| `sqr(v)` | 1 for the first half of each turn, −1 for the second |
| `abs(v)`, `floor(v)`, `frac(v)` (or `fract(v)`) | size; whole part (towards −∞); `v − floor(v)` |
| `sqrt(v)` | square root; 0 for v ≤ 0 |
| `min(a, b)`, `max(a, b)`, `clamp(v, lo, hi)` | limits |
| `mix(a, b, k)` | `a + (b − a)·k` (k is not clamped) |
| `step(edge, v)` | 1 when `v ≥ edge`, else 0 — note the order: edge first |
| `smoothstep(e0, e1, v)` | 0..1, smooth (3k² − 2k³); `e0 > e1` gives a falling step |
| `len(dx, dy)` | √(dx² + dy²) |
| `atan2(dy, dx)` | angle in turns, −½..½, 0 along +x, ¼ along +y |
| `noise(x, y, z)` | smooth value noise 0..1, one hill per unit: scale inputs (`x*8`) |
| `hash(a, b)` | a pseudo-random 0..1 from the exact bits of a and b (`hash(px, py)`: per pixel) |
| `select(c, a, b)`, `c ? a : b` | `a` when c ≠ 0, else `b` (both are computed) |
| `digit(n, col, row)` | 1 if pixel (col, row) of digit n is lit in the panel's font, else 0 (see below) |
| `glyph(ch, col, row)` | the same for a character code, 32..126: `glyph('A', col, row)` |
| `digit(…, small)`, `glyph(…, small)` | the same in the small font |
| `digit(…, bold)` | the same in the bold 3 × 5 digits (no `glyph`: digits only) |
| `bit(n, i)` | bit i of floor(n), 1 or 0 (two's complement); i floored, i < 0 gives 0 |
| `pick(k, a, b, …)` | `a` when floor(k) is 0, `b` when 1, …; **0** when it is below 0 or past the last (any number of values) |

**Picking.** `pick(k, a, b, c)` is the chain `k < 0 ? 0 : k < 1 ? a : k <
2 ? b : k < 3 ? c : 0`, byte for byte, but one step a pixel when `k` varies
per pixel and the values it picks from don't (numbers, characters,
`frame { }` values, anything per row): a digit's value from the clock, a
letter of a word (`pick(cell, 'H', 'I')`), a colour (section 6). When the
values vary per pixel it is that chain of compares and selects.

**Digits and text.** `digit` and `glyph` read the panel's own bitmap fonts
(OpenDMD's `@clock` and title cards; X11 misc-fixed, printable ASCII), and
`digit` also a bold set of digits:

| font | cell (col × row) | a digit's pixels | |
|---|---|---|---|
| `normal` (the default) | 5 × 7: col 0..4, row 0..6 | cols 0..3, rows 0..5 | the clock's digits |
| `small` | 4 × 6: col 0..3, row 0..5 | cols 0..2, rows 0..4 | 3 × 5 digits |
| `bold` (`digit` only) | 4 × 6: col 0..3, row 0..5 | cols 0..2, rows 0..4 | chunky 3 × 5 digits, square 0 |

- **Coordinates** are whole columns and rows of the cell from its **top
  left** (row 0 is the top: the opposite of `py`), floored, so a glyph
  drawn `k` times the size is `glyph(ch, floor(u/k), floor(v/k))` with
  `u`, `v` the pixel's offset from the cell's top left. Outside the cell
  the result is 0, so neighbouring cells, gaps and margins need no test.
  The cell includes the gap to the next character: cells tile at 5 px
  (`small`, `bold`: 4).
- **The character** is floored: `digit` takes 0..9, `glyph` 32..126; any
  other (negative, 10, 127, …) draws nothing.
- Each is **one step** per pixel (a table read, ~100 ns), and once a frame
  or a row when its values are.

## 6. Colours and coverage

The last line is the colour:

| colour | meaning |
|---|---|
| `rgb(r, g, b)` | each 0..1, clamped, rounded to 0..255 |
| `hsv(h, s, v)` | hue in turns (wraps: `frac`), saturation and value clamped 0..1 |
| `pal(P, k)` | palette P at k (clamped 0..1), **32 steps** (bands on slow ramps) |
| `gray(v)` | black to white; a bare value `v` means `gray(v)` |
| `#ff8800`, `#f80` | a colour: red, green, blue as two hex digits each (or one, doubled) |
| a colour expression | arithmetic on colours, read once a channel (below) |
| `rgb(c, k)` | colour expression `c` covering `k` of the pixel |

**Colours.** `#rrggbb` names a colour by its bytes: the channel `cc` is
exactly `cc/255`, so `#ff8000` is drawn as 255, 128, 0. A colour goes only
in a colour line, never in a name (`v = #f80` is an error: colours are not
values). A colour line with a colour in it (a `#…`, or an `rgb(r, g, b)`
inside it) is a **colour expression**: it is read three times, once for
red, once for green, once for blue, with each `#…` that channel's value and
each `rgb(r, g, b)` inside it that channel's value, and the rest as usual.
So

- `#f80 * k` is `rgb(k, k*0.533, 0)`: the colour at brightness `k`;
- `mix(#102040, #ff8800, k)` goes from one colour to the other as `k`
  goes from 0 to 1 (not clamped);
- `pick(n, #f00, #0f0, #00f)` and `c ? #fff : #888` choose a colour;
- `mix(rgb(ur, ug, ub), #b08040, a)` mixes the picture under with a colour;
- `#f80*0.5 + #008*k` adds two colours, each channel clamped at the end.

Each step runs once per channel: a colour expression costs what the
three channels written out as `rgb(…)` cost (`#f80 * k` is one multiply:
a channel of 1 is free, of 0 folds away), and it holds the same values
at once (an `rgb(r, g, b)` inside reads only its channel's value in each
reading). `hsv`, `pal` and `gray` take values only: they can't go inside a
colour expression or be multiplied (`pal(fire, k) * #fff` is an error);
`rgb(r, g, b) * k` can. Coverage goes on the outer `rgb` only:
`rgb(mix(#000, #f80, k), 0.5)`, not `mix(rgb(#f80, 0.5), …)`.

Palettes: `fire` (black, red, orange, yellow, near white), `ice` = `ocean`
(black, navy, blue, cyan, near white), `amber` (black to warm amber),
`rainbow` (**black at 0**, red, orange, yellow, green, blue, violet, magenta,
dark at 1 — not cyclic: for a seamless rainbow use `hsv`), `chrono` (black,
purple, blue, white at ~0.75, purple, dark), `gray`.

Every colour takes one more, optional value: **coverage**, how much of the
pixel it covers, 0..1 (clamped): `rgb(r, g, b, k)`, `hsv(h, s, v, k)`,
`pal(P, v, k)`, `gray(v, k)`, and for a colour expression `rgb(c, k)`
(`rgb(#ff3333, k)`). Where it covers less than all, what is below
shows through: `result = below + (colour − below)·k`, in 8-bit channels.

## 7. Layers

```psi
loop 4
layer { hsv(x + t/4, 1, 1) }
layer multiply { gray(py % 2 ? 1 : 0.6) }
```

`layer [over|add|multiply|screen|max|min] { … }` blocks stack bottom to top,
`over` by default. Per 8-bit channel, `d` below, `s` the layer's colour:
over `s`; add `min(255, d + s)`; multiply `d·s/255`; screen `d + s − d·s/255`;
max; min. Then coverage moves `d` towards that result. A layer has its own
names (two layers may both set `v`) and may have its own `frame { }`; it
sees names set before it outside the layers, but may not set them again.
`loop` and `part` go outside layers. Layers do not nest. A layer under an
opaque `over` layer (no coverage, or a constant ≥ 1) is dropped (free).

## 8. Over a GIF

A formula can draw over the GIF playing. The bottom of the stack is the
picture under the formula: the bottom layer, a blend other than `over`, or a
coverage below 1, let it show through, and `ur ug ub ul` read it at the
pixel. A formula that does neither replaces the GIF (and is refused as a
GIF look). Alone, the picture under it is black. Looks are capped at 8 ms.

## 9. Parts: one formula inside another

```psi
part p {
  loop 8
  wave = sin(x*2 + t/8) + sin(y*1.5 - t/4)
  hsv(wave/4 + t/8, 0.9, 1)
}
layer { p }
layer add { p(1 - x, y, 0.5) }
```

- `part NAME { … }` holds a whole formula: `loop`, `frame { }`, names,
  layers, parts, and (optionally) a colour line. It draws nothing until
  used. It sees **only its own names** and the built-ins: outer names are
  invisible inside, and its names are invisible outside except as `p.v`.
  So any formula can be pasted in unchanged.
- **As a colour** (a whole last line of the formula or of a layer): `p`;
  `p(x', y')` draws it as if this pixel were at (x', y') — inside, `x`, `y`
  are those, `px = floor(x'·127 + ½)`, `py = floor(y'·31 + ½)`, `i` from
  them; `p(x', y', k)` also covers `k` of the pixel (like every colour's
  extra value). `p(x, y)` is just `p`.
- **As values**: `p.v` is the value `v` set at the part's top level;
  `p(x', y').v` the same at another place. A part used only for values needs
  no colour line.
- **Its own time**: inside a part, `t` is `t % (its loop)` (its `loop`, or
  8 s like any formula), unless the formula's loop is no longer than the
  part's. Make the formula's loop a multiple of its parts' loops.
- A part with layers keeps its blends: draw it with `layer { p }` (no blend
  name) or as the last line; with coverage, each of its layers covers that
  much less.
- `ur ug ub ul` inside a part read the picture at the real pixel.
- Each place a part is drawn at is its own steps (counted in the 256);
  drawn where the pixel is, a part costs what it cost alone.
- In the web editor, the line `use Plasma as p` pastes the saved formula or
  example called Plasma in as `part p { … }` (never the formula being
  edited; `use` lines inside what it pastes become comments); anything
  shared or saved is always the pasted, self-contained text. A model should write `part`
  blocks directly.

Masks and crossfades need nothing more: a crossfade is a second layer whose
coverage moves with `t`; a mask is a coverage taken from another part's
value (examples 23 and 24).

## 10. Limits

| limit | value | error says |
|---|---|---|
| source | 4,096 characters (a shared link past ~1,500 may be cut by apps) | too long |
| steps | 256 in all: per frame + per row + per pixel + one per layer | too long for the panel (N steps) |
| live per-pixel values at once | 12 | keeps too many values at once |
| constants + per-frame/per-row values | ~119 | too many numbers and steps |
| nesting | 24 levels of brackets and operators (a part level counts 3) | nested too deeply / parts go too deep |
| `loop` | 0.1..3600 s | between 0.1 and 3600 seconds |
| re-reading parts drawn at other places | bounded | draws its parts in too many places |

Unused names and parts cost nothing; constants fold (`2*3` is 6; `x/8` is
a multiply).

## 11. The cost model

The panel's processor runs each step for 4,096 pixels; the estimate is
`(160 + Σ per-pixel step costs) ns × 4096` (+ ~10 µs per per-row step).
**Hoisting is automatic**: a value is computed as rarely as its inputs
change — constants once, `t`/clock/slider-only values once a frame,
`y`/`py`-only values once per row (32 times), the rest per pixel. So
`sin(t/8)` is free and `sin(y*4)` nearly free; only per-pixel work counts.

| per pixel | ns | | per pixel | ns |
|---|---|---|---|---|
| + − compare min max step and or `%`2ⁿ | 50 | | `sin` `cos` | 110 |
| `*` | 86 | | `tri` `saw` `sqr` `select` `clamp` | 60 |
| `%` (other) | 113 | | `mix` | 130 |
| neg abs floor frac not | 40 | | `smoothstep` | 214 + a division if its edges vary |
| `/` by a varying value, `1/v` | 685 | | `hash` | 220 |
| `sqrt` | 539 | | `atan2` | 529 |
| `len` | 642 | | `noise` | 855 |
| colour `rgb` | 120 | | colour `hsv` | 195 |
| `ur` `ug` `ub` | 35 | | `ul` | 125 |
| `digit` `glyph` | ~100 (estimated) | | `bit` | 55 |
| `pick` (values the same for every pixel) | 60 (estimated) | | `pick` of per-pixel values | its compares and selects |
| each extra layer | 150 (multiply 210, screen 290; +75 with coverage) | | showing the picture under | 50 |

Ratings: **light ≤ 5 ms**, **medium ≤ 8 ms**, **heavy** beyond (runs, but
the gallery and GIF looks take ≤ 8 ms). Division by a constant is a
multiply. One `noise` or `len` is fine; four are heavy. Per-pixel `px`
arithmetic costs the same as `x` arithmetic.

## 12. Idioms

- **Seamless loops**: with `loop N`, move things by `t/N` (or `t*k/N`, k
  whole) inside periodic functions (`sin`, `frac`, `tri`, hue).
- **Circles**: `r` (or `len(px - 63.5, py - 15.5)` in pixels); a disc
  `step(r, 0.3)`; soft edge in pixels: `smoothstep(rad + 0.5, rad - 0.5, d)`.
  A circle about another point: `len(cx - 1, cy)`. Spokes and sweeps:
  `angle` (`frac(angle - t/4)` turns once a loop).
- **Bands and cells**: `floor(px/8)`, `px % 8`, `frac(x*4)`; checker:
  `(floor(px/8) + floor(py/8)) % 2`.
- **Random per column/row/pixel**: `hash(px, 7)`, `hash(py, 3)`,
  `hash(px, py)`; changing over time in steps: `hash(px, py + floor(t*12)*61)`.
- **Per-frame values**: put clock maths in `frame { }` (it's hoisted anyway,
  but `frame` also catches mistakes and documents intent).
- **Lighten/darken a GIF**: `layer multiply { gray(k) }`; tint:
  `layer multiply { rgb(…) }`; glow: `layer add { … }` or `screen`.
- **Shapes over a GIF**: `layer { rgb(r, g, b, shape) }` where `shape` is
  0..1: only the shape covers.
- **Crossfade**: `k = smoothstep(0.25, 0.75, sin(t/N)*0.5 + 0.5)`; `layer { A }`,
  `layer { B(x, y, k) }`.
- **Mirror / tile / zoom a part**: `p(abs(x - 0.5)*2, y)`, `p(frac(x*4), y)`,
  `p((x - 0.5)/2 + 0.5, (y - 0.5)/2 + 0.5)`.
- **Digits and text**: work out, from `px` and `ty`, which cell the pixel
  is in, which character goes there (`pick`, character codes in quotes),
  and the column and row inside the cell; `digit`/`glyph` is the pixel, 0
  or 1, ready for `hsv`'s value, a coverage or `#…*`. Scale by dividing
  the offsets. The next block writes HELLO at twice the size in the
  middle; examples 26 and 27 put it to work.

```psi
-- 5 letters x 10 px (the 5-px cell, doubled) = 50 px, from column 39
u = px - 39; v = ty - 9              -- offsets from the text's top left
ch = pick(floor(u/10), 'H', 'E', 'L', 'L', 'O')   -- which letter (0 outside)
gray(glyph(ch, floor(u/2) % 5, floor(v/2)))
```

  (`pick` gives 0 outside the word and character 0 draws nothing, so
  either side is blank; `%` is floored, so `floor(u/2) % 5` is the column
  even left of the text.)
- **Colours**: name them by their bytes and do the arithmetic on the
  colour: `#80b3ff * glow` (light blue at brightness `glow`),
  `mix(#1a1033, #ff8020, k)` (a two-colour gradient), `pick(k, #f00, #0f0)`.
  `rgb(#ff3333, k)` covers `k` of the pixel.
- **Your own patterns**: pack rows of pixels into a whole number and read
  them back with `bit(row_bits, column)`, e.g. an 8-pixel row pattern
  `bit(165, px % 8)`.
- **Ranges**: keep `hsv`'s value and `rgb`'s channels in 0..1 by design
  (`*0.5 + 0.5` maps −1..1 to 0..1); out-of-range is clamped, so overshoot
  just flattens.

## 13. Common mistakes

Each block below fails to compile with the sentence in its first comment.

```psi error
-- error: there is no power operator
gray(x ^ 2)
```

Write `x * x`. Likewise there is no `pow`, `exp`, `log`, `pi` or `tan`
(angles are turns: use `0.5` for half a turn).

```psi error
-- error: unknown function `pow`
gray(pow(x, 2))
```

```psi error
-- error: to multiply, write `2 * x`
gray(2x)
```

```psi error
-- error: `v` is already set on line 2
v = x
v = v * 2
gray(v)
```

Each name is set once: pick a new name for each step.

```psi error
-- error: inside frame { } only values that are the same for every pixel
frame { k = x * 2 }
gray(k)
```

```psi error
-- error: makes the colour, so it can only be the whole last line
v = rgb(x, y, 0)
gray(0)
```

Colours are not values: compute numbers, then end with one colour.

```psi error
-- error: the colour has to be the last line
rgb(x, 0, 0)
v = 1
```

```psi error
-- error: with layers, every colour goes inside one
layer { gray(x) }
gray(y)
```

```psi error
-- error: the last value of `digit` is a font: normal (the default), small or bold
gray(digit(3, px % 5, 31 - py, 2))
```

```psi error
-- error: a character goes in quotes, one at a time
gray(glyph('AB', px % 5, ty))
```

There are no strings: one `glyph` per character cell (the idiom above).

```psi error
-- error: unknown palette `blue`
pal(blue, x)
```

```psi error
-- error: a colour can't be a value
glow = #ffcc00
glow * x
```

A colour has no name: write it where it is used, in the colour line
(`#ffcc00 * x`).

```psi error
-- error: a colour can't be a value
hsv(#ff8800)
```

`hsv`, `pal` and `gray` take values; a colour goes on its own or in `rgb(#ff8800, k)`.

```psi error
-- error: a colour is # and 3 or 6 hex digits
#ff880
```

```psi error
-- error: `pick` takes a number, then the values to pick from
gray(pick(x))
```

```psi error
-- error: `t` is built in
t = 2
gray(t)
```

```psi error
-- error: `p` is a part: draw it as a whole last line
part p { gray(x) }
gray(p * 0.5)
```

To scale a part, give it coverage (`p(x, y, 0.5)` in a layer) or use one of
its values (`p.v`).

```psi error
-- error: unknown name `k`
k = 0.5
part p { gray(k) }
p
```

A part sees only its own names: put `k` inside it.

```psi error
-- error: this part has layers of its own
part p { layer { gray(x) }
  layer multiply { gray(y) } }
layer add { p }
```

Mistakes that compile but look wrong (the contact sheet shows them):

- `y` is **up**: `y = 0` is the bottom row.
- Forgetting the 4:1 aspect: `len(x - 0.5, y - 0.5)` makes ellipses
  (`r` doesn't).
- `cx` and `cy` are in panel heights: `cx` reaches ±2, `cy` only ±0.5.
- `pick` counts from **0**: `pick(k, a, b)` is `a` for `0 <= k < 1`; and
  it is 0 past the end, not the last value.
- `step(v, edge)` instead of `step(edge, v)`: the comparison is reversed.
- A loop that jumps: something moved by `t/3` under `loop 4`.
- `pal(rainbow, frac(…))` shows a black band where it wraps; use `hsv`.
- `hash(x, y)` is random per *bit* of x: use `hash(px, py)` for per-pixel.
- `sin` is in turns: `sin(x*6.28)` is about six waves across, not one.
- Slow `pal` ramps band (32 steps); `rgb`/`hsv` are smooth.
- Bright everything: `add` and `screen` layers only lighten.

## 14. Worked examples

Each: what it is for, then the formula. Frames are checked against golden
hashes (at 10:09:30, sliders a = 0.5, b = 0.25, c = 0.75; those that read
what is under drawn over a test picture).

### 1. Gradient

The smallest formula: a value as the last line is its gray.

```psi example=gradient
-- left to right: black to white
gray(x)
```

### 2. Still sky

A colour that depends only on `y`: computed once per row, nearly free.

```psi example=sunset-sky
-- a still sky: deep blue at the top to orange at the horizon
rgb(1 - y*0.8, 0.45 - y*0.3, 0.2 + y*0.6)
```

### 3. Scrolling rainbow

Seamless motion: `t/4` with `loop 4` comes back to where it started.

```psi example=scrolling-rainbow
loop 4
-- the rainbow slides left once a loop (t/4 with loop 4: seamless)
hsv(x + t/4, 1, 1)
```

### 4. Checkerboard

Cells from whole pixels: `floor` and `%`.

```psi example=checkerboard
-- 8x8-pixel squares; floor makes whole cells, % 2 alternates them
cell = (floor(px/8) + floor(py/8)) % 2
mix(#001a4d, #e54d4d, cell)          -- navy where cell is 0, red where 1
```

### 5. Plasma

Several waves summed into a hue: the classic.

```psi example=plasma
loop 8
wave = sin(x*2 + t/8) + sin(y*1.5 - t/4) + sin(x*1.5 + y + t/8)
hsv(wave/6 + t/8, 0.9, 1)
```

### 6. Rings

`r`, the distance from the middle, is round on the 4:1 panel.

```psi example=rings
loop 4
-- r: the distance from the middle, in panel heights
pal(rainbow, frac(r*1.5 - t/2))
```

### 7. Noise fire

`noise` scaled and moving up, fading with height, through a palette.

```psi example=noise-fire
loop 4
-- noise rises (y*3 - t*... moves it up); hotter at the bottom
heat = noise(x*10, y*3 - t, t/2) * (1.4 - y*1.2)
pal(fire, clamp(heat, 0, 1))
```

### 8. Rain

Per-column randomness with `hash(px, …)`.

```psi example=rain
loop 4
-- one made-up random number per column: its speed and offset
r = hash(px, 7)
drop = frac(y + t*(1 + floor(r*3))/4 + r)
glow = max(0, 1 - drop*6) * step(0.5, r)
rgb(glow*0.4, glow*0.7, glow)
```

### 9. Starfield

Per-row randomness: three speeds of streaks.

```psi example=starfield
loop 8
-- each row: no star, or a slow, medium or fast one
r = hash(py, 3)
speed = floor(r*4)
p = frac(x*2 + t*speed/4 + r*5)
gray(max(0, 1 - p*(14 - speed*3)) * speed/3)
```

### 10. Bouncing ball

A shape whose position is worked out once a frame, with a soft edge.

```psi example=bouncing-ball
loop 2
-- the ball's centre in pixels: across and back with tri, two bounces with abs(sin)
frame { bx = 63.5 + 55*tri(t/2); by = 4.5 + 22*abs(sin(t/2)) }
d = len(px - bx, py - by)
-- 1 inside 3.5 px, 0 past 4.5 px: a soft-edged disc
ball = smoothstep(4.5, 3.5, d)
rgb(ball, ball*0.6, 0.15 + 0.1*y)
```

### 11. Spotlight

A soft light on a path; values kept in range with `clamp`.

```psi example=spotlight
loop 8
-- a soft disc moving along a figure of eight, over a dim floor
dx = cx - 0.35*sin(t/8)*4; dy = cy - 0.3*sin(t/4)
light = clamp(1.3 - (dx*dx + dy*dy)*4, 0.12, 1)
rgb(light, light*0.9, light*0.7)
```

### 12. Stripes with sliders

Sliders `a` and `b`, named by comments, shaping the pattern.

```psi example=slider-stripes
loop 4
-- a: stripe width
-- b: tilt
size = 6 + a*24
band = sin((px + py*(b*4 - 2))/size - t/2)
rgb(1, band*0.5 + 0.5, band*0.5 + 0.5)
```

### 13. Seconds bar

Following the clock: a bar filling every minute, with ticks.

```psi example=seconds-bar
-- a bar that fills across the panel every minute; the ticks mark 5 s
frame { s = second/60 }
fill = step(x, s)
tick = step(px % 21, 0.5) * step(py, 3)
rgb(fill*0.2 + tick, fill*0.8 + tick, fill + tick)
```

### 14. Dot clock

Hours as lit dots and minutes as a bar: comparisons as masks.

```psi example=dot-clock
-- top: twelve dots, the hours lit; bottom: a bar, one pixel a minute
frame { hr = hour % 12; mn = minute }
dot = step(abs(px % 10 - 4.5), 2.5) * step(abs(py - 22), 3) * step(px, 119)
lit = step(floor(px/10), hr - 0.5)
bar = step(abs(py - 8), 2) * step(px - 4, mn) * step(4, px)
rgb(dot*(0.15 + lit*0.85), dot*(0.1 + lit*0.5) + bar*0.8, bar)
```

### 15. Sky by the time of day

A whole-day cycle from `hour` and `minute`.

```psi example=day-sky
-- follows the clock: dark at night, blue by day, warm near sunrise and sunset
frame { day = (hour + minute/60)/24; sun = -cos(day) }
light = clamp(sun*1.5 + 0.3, 0, 1)
warm = clamp(1 - abs(sun)*3, 0, 1) * (1 - y)
rgb(0.02 + light*(0.6 - 0.4*y) + warm*0.9, 0.02 + light*(0.8 - 0.35*y) + warm*0.35, 0.1 + light*(0.9 - 0.1*y))
```

### 16. Layers

A picture, scanlines multiplied on, a glow added with coverage.

```psi example=layered-glow
loop 4
-- a plasma, scanlines multiplied on, then a warm glow added in the middle
dx = x - 0.5
layer {
  wave = sin(x*2 + t/4) + sin(y*1.5 - t/2)
  hsv(wave/4 + t/4, 0.9, 1)
}
layer multiply { gray(py % 2 ? 1 : 0.6) }
layer add { rgb(#ffcc66, 0.35 - (dx*dx*4 + cy*cy)*1.2) }
```

### 17. Scanlines over a GIF

The cheapest look: a `multiply` layer that leaves white rows as they are.

```psi example=scanlines-look
-- over a GIF: every other row darker; white leaves the GIF as it is
layer multiply { gray(py % 2 ? 1 : 0.55) }
```

### 18. Old photo over a GIF

Reading the picture under: its colour and brightness, mixed by slider `a`
(a colour expression: `mix` of two colours, each `rgb(…)` of values).

```psi example=sepia-look
-- over a GIF: its brightness, tinted; a: how much (0: the GIF as it is)
mix(rgb(ur, ug, ub), rgb(ul*1.1 + 0.08, ul*0.92 + 0.04, ul*0.65), a)
```

### 19. Glitter over a GIF

Sparkles `screen`ed on: they only lighten.

```psi example=glitter-look
loop 2
-- over a GIF: a new handful of sparkles twelve times a second, screened on
s = hash(px, py + floor(t*12)*61)
sparkle = step(0.985, s) * (0.6 + 0.4*sin(t*6 + s*40))
layer screen { rgb(sparkle, sparkle*0.9, sparkle*0.6) }
```

### 20. A badge over a GIF

A shape with coverage: only the dot covers the GIF.

```psi example=corner-badge
loop 2
-- over a GIF: a pulsing dot in the top-right corner; elsewhere the GIF shows
d = len(px - 121, py - 25)
k = smoothstep(3.5, 2.5, d)
layer { rgb(#ff3333, k*(0.7 + 0.3*sin(t/2))) }
```

### 21. Kaleidoscope (a part drawn elsewhere)

A pasted formula drawn at folded coordinates.

```psi example=mirrored
loop 8
part plasma {
  loop 8
  wave = sin(x*2 + t/8) + sin(y*1.5 - t/4) + sin(x*1.5 + y + t/8)
  hsv(wave/6 + t/8, 0.9, 1)
}
-- folded twice: one quarter, mirrored into every corner
plasma(abs(x - 0.5)*2, abs(y - 0.5)*2)
```

### 22. Tiles (a part repeated)

`frac` repeats a part across the panel.

```psi example=tiled
loop 4
part ring {
  loop 4
  d = len((x - 0.5)*1, y - 0.5)
  pal(ice, frac(d*3 - t/4))
}
-- four square tiles across: frac repeats the part's 0..1 every quarter
ring(frac(x*4), y)
```

### 23. Mask (a part's value as coverage)

One part's value decides where another shows.

```psi example=masked
loop 4
part stripes {
  loop 4
  s = clamp(sin((px + py)/10 - t/2)*1.5 + 0.5, 0, 1)
  gray(s)
}
part plasma {
  loop 8
  wave = sin(x*2 + t/8) + sin(y*1.5 - t/4)
  hsv(wave/4 + t/8, 0.9, 1)
}
-- the plasma shows where the stripes are white, dark blue elsewhere
layer { #000033 }
layer { plasma(x, y, stripes.s) }
```

### 24. Crossfade (two parts over time)

A second layer whose coverage moves with `t`.

```psi example=crossfade
loop 16
part bars {
  loop 4
  bar = max(0, 1 - abs(cy - 0.35*sin(t/4))*5)
  bar * #ff9933
}
part rings {
  loop 4
  pal(rainbow, frac(r*1.5 - t/2))
}
-- 0 to 1 and back over the loop, resting at each end
k = smoothstep(0.25, 0.75, sin(t/16)*0.5 + 0.5)
layer { bars }
layer { rings(x, y, k) }
```

### 25. Picture in picture

A part squeezed into a window, its `px`/`py` following.

```psi example=picture-in-picture
loop 4
part candy {
  loop 4
  -- px and py inside follow the place it is drawn at
  band = sin((px + py)/12 - t/2)
  s = clamp(band*1.5 + 0.5, 0, 1)
  rgb(1, s, s)
}
-- a dark blue background, and the candy stripes in a window on the right
inside = step(88, px) * step(px, 123) * step(4, py) * step(py, 27)
layer { #0d1433 }
layer { candy((px - 88)/35, (py - 4)/23, inside) }
```

### 26. A name in lights

Text: which letter from `px` (`pick` of character codes), its column and
row from the offsets, `glyph` for the pixel, here at twice the size. The
pick is one step a pixel however long the word (as a chain of `?:` each
letter cost a comparison and a select).

```psi example=name-sign
loop 4
-- "OpenDMD" in the panel's font at twice the size, centred: 7 letters of 10 px
u = px - 30; v = ty - 9               -- the pixel from the text's top left
ch = pick(floor(u/10), 'O', 'p', 'e', 'n', 'D', 'M', 'D')   -- which letter
ink = glyph(ch, floor(u/2) % 5, floor(v/2))   -- each font pixel 2 x 2
hsv(x - t/4, 0.8, ink)                -- a rainbow sliding through, once a loop
```

### 27. Big clock (the showcase)

What only a formula can do, readable in two seconds: the real time, in
digits that fill the panel, with a seconds bar that crosses the bottom
row once a minute. The digits are named once a frame (`h1 h2 m1 m2`);
per pixel, which digit comes from `px` and its value from `pick`, the
font's column and row from the pixel's offset in the digit (each font
pixel 8 × 6 px: the bold font's 3 × 5 digits make 24 × 30), and `digit`
is the pixel. In row 0 the seconds bar replaces the digits.

```psi example=big-clock
loop 60
-- Big clock: the time in four big digits, and a seconds bar along the bottom
frame {
  h1 = floor(hour/10); h2 = hour % 10       -- the hours' two digits
  m1 = floor(minute/10); m2 = minute % 10   -- the minutes' two digits
  end = (second + 1)*128/60                 -- where the seconds bar ends
}
-- a digit: the bold font's 3 x 5 pixels, each 8 px wide, 6 tall (24 x 30)
q = px >= 64 ? px - 8 : px     -- the minutes sit 8 px further right
k = floor(q/32)                -- which digit: one every 32 px (0 1, then 2 3)
n = pick(k, h1, h2, m1, m2)    -- and its value
col = floor(q % 32 / 8)        -- the font's column, 0 to 2 (then the gap)
row = floor(ty/6)              -- its row from the top, 0 to 4 (5: below)
bar = py < 1                   -- the bottom row is the seconds bar
lit = bar ? px < end : digit(n, col, row, bold)
-- a rainbow drifting through the digits, tilted by row; the bar is white
hue = py/64 + hour/24 + t/10
hsv(x + hue, bar ? 0 : 1, lit)
```

### 28. Dusk (a gradient between two colours)

Two colours named by their bytes, mixed by a value: a colour expression.

```psi example=dusk
-- orange at the horizon to deep blue at the top, a little brighter in the middle
glow = 1 - r*0.3
mix(#ff8030, #101850, y) * glow
```

### 29. Radar (the angle about the middle)

`angle` sweeps round the middle once a turn. The disc is `cx*cx + cy*cy <
0.2` (r² < 0.2): `r` alone would cost a `len` on top of the `atan2`.

```psi example=radar
loop 4
-- a beam sweeping round once a loop, fading behind it, in a disc
behind = frac(t/4 - angle)             -- 0 at the beam, growing behind it
glow = max(0, 1 - behind*3) * (cx*cx + cy*cy < 0.2)
#33ff66 * glow
```

### 30. A word in colours (pick of colours)

`pick` chooses the letter and, in the colour line, its colour.

```psi example=rgb-word
-- "RGB" at twice the size, each letter its own colour
u = px - 49; v = ty - 9
cell = floor(u/10)
ink = glyph(pick(cell, 'R', 'G', 'B'), floor(u/2) % 5, floor(v/2))
pick(cell, #ff3333, #33ff55, #3377ff) * ink
```

## 15. In the product (not built)

This reference and `psi-check` are tooling only: nothing in OpenDMD calls a
model. If the owner wants it later, the natural place is the Formulas
editor: a "Describe an effect…" box whose text, with this document as the
system prompt, asks a model for a formula; the editor compiles the answer
locally (errors and cost come from the same compiler), shows it in the
preview, and could loop a few times on compile errors before showing
anything. The decision (provider, keys, cost, privacy) is the owner's.
