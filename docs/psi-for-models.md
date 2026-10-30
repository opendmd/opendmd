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

(or `psi check candidate.psi --out sheet.png`, with the `psi` command
installed: `make psi`).

It prints `ok: …` (steps, layers, registers, values), the loop and what it reads,
and `panel: ~N ms a frame: light|medium|heavy`, and writes a PNG contact
sheet of frames across one loop (over a sample picture — sky, sun, hills —
when the formula draws over what is under it). On a compile error it prints
`error[E###]: line L, column C: <sentence>` (the code links the language
reference's error index, `/dev/psi/#E###`), the line and carets, and fails:
the tool itself exits 1 (2 for a missing file or a bad option), and
`make psi-check` then exits 2, as make does for any failed command.
Look at the sheet: the numbers say it runs, only the picture says it is
right. Aim for **≤ 8 ms** (light or medium).

## 1. The model

A formula is evaluated **for every pixel, every frame** (50 frames a
second, or what its `fps` line asks). It is a few lines of arithmetic ending in a colour. Per pixel there
are no loops, arrays, strings, recursion or state between frames: each frame
is a pure function of the pixel's position, the time, the clock, three
sliders, and (optionally) the picture under it. For things that are not "each
pixel decides for itself" — stars, wireframes, particles — a `draw { }` block
runs **once a frame** before the pixels, loops over small `table`s and draws
lines and dots into a canvas the pixels read back as `ink` (section 10).

Coordinates: `x` is 0 at the left column to 1 at the right (`x = px/127`);
`y` is 0 at the **bottom** row to 1 at the top (`y = py/31`). The panel is
4 times wider than tall, so the built-ins about the middle measure in panel
heights: `cx` is −2 at the left edge to 2 at the right, `cy` −0.5 at the
bottom to 0.5 at the top, and `r` (the distance from the middle) and
`angle` are round: `r < 0.4` is a disc, not an ellipse. Or work in pixels
(`len(px - 63.5, py - 15.5)`). `ty` is the row counted from the top (0 to
31), as the fonts count.

Angles and waves are in **turns**: `sin(0.25)` is 1; `sin(v)` repeats every 1.
Time `t` is seconds into the loop, always a whole number of frames: 20 ms
ones, or 10 ms with `fps 100` (40 ms with `fps 25`). With `loop forever`
there is no loop: `t` is the seconds since the formula started, growing
without end (section 11).

## 2. Grammar

```text
formula   = { sep } { decl sep } { stmt sep } colourline { sep }   -- or layers instead of colourline
decl      = "psi" NUMBER                      -- the language version, first line only; never needed (unversioned is 1)
          | "name" TEXT                       -- what the formula is called (60 characters)
          | "about" TEXT                      -- one line about it (120 characters)
          | "control" NAME [ NUM ".." NUM ] [ "=" NUM ] [ TEXT ]   -- a slider: range (0..1), default (the low end), label
          | "control" NAME "=" WORD "," WORD { "," WORD } [ TEXT ] -- 2-6 words: buttons; the value is the word's number
          | "look" [ BLEND ] [ NUMBER ]       -- it may be the panel's look: blend (over), amount 0..100 (100)
stmt      = "loop" NUMBER                     -- 0.1..3600 s, once, default 8
          | "loop" "forever"                  -- no loop: t grows without end (section 11)
          | "fps" NUMBER                      -- 25, 50 or 100 frames a second, once, default 50
          | "frame" "{" { assign sep } "}"    -- values once a frame (no x, y, px, py, i, ur…)
          | NAME "=" expr                     -- may set a name again: a read sees the latest
          | "layer" [BLEND] "{" { assign | frameblock } colourline "}"
          | "part" NAME "{" formula-body "}"  -- another formula inside this one
          | "draw" "{" { dstmt sep } "}"      -- once a frame, before the pixels: one per formula
          | "table" NAME "=" ["-"] NUMBER { "," ["-"] NUMBER }   -- constants; a "," may end a line
dstmt     = NAME "=" expr                     -- the same for every pixel (no x, y, px, py, i, ur…, ink)
          | "for" NAME "<" NUMBER "{" { dstmt sep } "}"   -- NAME = 0 … NUMBER−1; 1..1024, two deep
          | "line" "(" e "," e "," e "," e [ "," e [ "," e ] ] ")"   -- x0, y0, x1, y1 [, v0 [, v1]]
          | "dot" "(" e "," e [ "," e ] ")"                            -- x, y [, v]
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
          | PART "(" e "," e ")" "." NAME | TABLE "[" expr "]" | SPRITE "[" e "," e "]" | "(" expr ")"
args      = e { "," e }                        -- digit/glyph: e "," e "," e [ "," FONT ]
                                               -- pick: e "," e { "," e }
sep       = newline | ";"                      -- newlines inside ( ) and [ ] are spaces
NUMBER    = digits [ "." digits ] | "." digits -- no sign, no exponent
CHAR      = "'" one printable ASCII character "'"   -- its code: 'A' is 65
COLOUR    = "#" 6 hex digits | "#" 3 hex digits   -- #ff8800, #f80 (each digit twice); colour lines only
TEXT      = '"' characters '"'                -- on name/about/control lines only; no " < > \ or tabs inside; label 24 characters
NUM       = [ "-" ] NUMBER
WORD      = NAME
FONT      = "normal" | "small" | "bold"      -- bold: digit only
NAME      = [A-Za-z_][A-Za-z0-9_]*
comment   = "--" or "//" to the end of the line
```

The declarations (`decl`) come first, before every other line; each once
(`control` up to three times). Section 4 says what they do.

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
| `t` | seconds into the loop (whole frames: 20 ms, 10 with `fps 100`); with `loop forever`, since the formula started, growing without end (section 11) | frame |
| `seed` | `loop forever` only: 0..1, the same for a whole play, a new one each time it plays (section 11) | frame |
| `hour`, `minute`, `second` | the clock: 0..23, 0..59, 0..59 (whole) | frame |
| `a`, `b`, `c` | the controls, in the order the `control` lines declare them (`a` is the first one's value; see below); one read without a `control` line is a slider 0..1 named after it | frame |
| `w`, `h` | 128 and 32 (constants) | — |
| `ur`, `ug`, `ub` | red, green, blue of the picture under the formula, 0..1 | pixel |
| `ul` | its brightness (Rec. 601: `(77R + 150G + 29B + 128)/256`), 0..1 | pixel |
| `cx` | across from the middle in panel heights, −2..2: `(x − 0.5)·4` | pixel |
| `cy` | up from the middle, −0.5..0.5: `y − 0.5` | row |
| `r` | the distance from the middle, in panel heights: `len(cx, cy)` (0.5 touches the top and bottom) | pixel |
| `angle` | the angle about the middle in turns, −½..½, 0 to the right, ¼ up: `atan2(cy, cx)` | pixel |
| `ty` | the row from the **top**, 0..31 (whole numbers): `31 − py` | row |
| `ink` | what the `draw { }` block put at this pixel, 0..1 (section 10) | pixel |

`cx cy r angle ty` are exactly the formulas after their colons: the same
steps, the same cost, the same bytes. Alone (not over a GIF), `ur ug ub ul`
are 0. Built-in names, function names, palette names and keywords (`loop
frame layer part use draw for table`) cannot be set (except `digit`,
`glyph`, `bit`, `pick`, `cx`, `cy`, `r`, `angle`, `ty`, `ink`, `line`,
`dot`, `seed` and `forever`, added later, so formulas saved with those names
still work: a name a formula sets is that name; don't use them as names in
new ones).

### What a formula says it is: the declarations

A formula may start with lines that say what it is. They are statements,
not comments, so they travel with it everywhere (the panel, a playlist, a
look, a share link), and the web app builds its controls from them.

```psi
name "Spin"
about "A wheel turning; its speed and spokes are controls"
control speed 0.5..4 = 1 "Speed"
control spokes = four, six, eight
look screen 40
loop 8
n = pick(spokes, 4, 6, 8)
k = abs(sin(angle*n + t*speed))
gray(smoothstep(0.8, 1, k) * (r < 0.45))
```

- **`control NAME lo..hi = default "Label"`** is a slider: `NAME` is a value
  in the formula, from `lo` to `hi` (the range is 0..1 when left out), the
  default the low end when left out, the label the name when left out (24
  characters). Wherever the formula reads it, the value is **held to the
  range** — whatever a link or a playlist line says — so the panel's cost
  estimate and its fast paths know it.
- **`control NAME = word, word, …`** (2 to 6 words) is a row of buttons:
  the value is the chosen word's number, 0 for the first (the default), so
  read it with `pick(NAME, …)`; two words make a switch (`NAME` is 0 or 1).
- The controls are the slots `a`, `b`, `c` in the order they are declared,
  three at most; the letters still read them (`a` is the first control's
  value). A link or a playlist line keeps values as `&a=…`; a slot it
  leaves out plays at the declared default.
- **`name "…"`** (60 characters) and **`about "…"`** (120): what it is
  called and one line about it. Text in quotes is allowed on these three
  lines only (no `"`, `<`, `>`, `\` or tabs inside).
- **`look [blend] [amount]`** says the formula may be the panel's look,
  drawn over every GIF with that blend (over, screen, add, multiply, max,
  min; over when none) and amount (0 to 100; 100 when none) — section 8.
  Played on its own (on the panel, in a playlist) the line is ignored.
- **`psi 1`**, on the first line, says the formula is written for version 1
  of the language. Never needed: a formula without it is version 1.
- A formula pasted into another as a part keeps its declarations; there only
  its `control` lines do anything (its names read the same slots).

```psi error
-- error: `name` goes at the top of the formula, before its other lines
loop 4
name "Late"
gray(x)
```

```psi error
-- error: a range goes from the lower number to the higher
control size 2..1 = 1
gray(size)
```

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
| `pal(P, k) * v` | the palette's colour with each channel times v (a light, a fog): one step |
| `cycle(P, v, s)` | palette cycling: entry `(floor(v·32) + floor(s·32)) mod 32` of P (wraps; P not `gray`); `s` per frame or per row |
| `gray(v)` | black to white; a bare value `v` means `gray(v)` |
| `#ff8800`, `#f80` | a colour: red, green, blue as two hex digits each (or one, doubled) |
| a colour expression | arithmetic on colours, read once a channel (below) |
| `rgb(c, k)` | colour expression `c` covering `k` of the pixel |

**Palettes of your own.** `palette sea = #001020, #0060a0, #40c0e0, #fff`
(a line at the top of the formula or a part; 1 to 32 colours, a `,` may
end a line; up to 3 palettes) is used as the built-ins are: `pal(sea, k)`,
`pal(sea, k) * v`, `pal(sea, k, cover)`. With n colours, `pal(sea,
i/(n−1))` is colour i: the first at 0, the last at 1 (`pal(sea, 1/3)` the
second of four), and k in between gives the nearest — bands, not a blend,
so a smooth ramp needs its steps written out. One step a pixel, the cost of
a built-in palette: a scene's colours by index without `pick`s. A colour
may have a name before it, `palette road = black #000, kerb #806, sky
#f48`: then `road.kerb` is exactly the k of that colour (i/(n−1)),
`road.step` is 1/(n−1) (one colour on: `road.kerb + road.step`) and
`road.n` the count — constants, no steps, and adding a colour renumbers
nothing you wrote.

```psi
palette dusk = #10082a, #4a1858, #c0306a, #ff8c40, #ffe0a0
pal(dusk, floor(y*5)/4)      -- five bands, bottom to top
```

**Palette cycling.** `cycle(P, v, s)` (also `* k` and `cycle(P, v, s,
cover)`, as `pal`) is palette P turned by `s`: entry `(floor(v·32) +
floor(s·32)) mod 32`, so both wrap (where `pal` clamps). `s` may not vary
along a row (E931: a value of `t`, a `frame { }` value, or a row value
like `py/40`, which turns the palette per row: copper bars). P is a
built-in palette or the formula's own, never `gray` (E932); each distinct
(P, s) takes one of the formula's three palette slots, with its own
`palette` lines (E933). The cost: the turn is a row step (a 32-entry copy,
~20 µs a frame), the pixel pays `frac(v)` and a palette look-up; with `v`
the pixel's position alone (no `t`, no slider) the still cache keeps
`frac(v)` and a frame is ~0.6-0.9 ms — the idiom for plasma, rings,
water, fire at `fps 100`. Write the motion into `s`, not `v`; keep `s` a
whole number of turns over the loop.

```psi
loop 8
fps 100
palette glow = #002, #208, #80f, #f0c, #f60, #ff0, #fff, #ff0, #f60, #f0c, #80f, #208
wave = sin(x*2) + sin(y*1.5) + sin(x*1.5 + y)
cycle(glow, wave/6, t/8)
```

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
colour expression (`pal(fire, k) * #fff` is an error); `rgb(r, g, b) * k`
can. The one exception is a palette **times values** as the whole colour:
`pal(fire, k) * light` (or `* a * b`) is the palette's colour with each
channel scaled, exactly `rgb` of its channels times `light`, in one step
(not a `multiply` layer); `pal(fire, k) * light + 0.1` is still an error. Coverage goes on the outer `rgb` only:
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
`loop`, `fps` and `part` go outside layers. Layers do not nest. A layer under an
opaque `over` layer (no coverage, or a constant ≥ 1) is dropped (free).

**Slow layers: `layer [blend] rate N { … }`** (N 2..200). The bottom
layer, or a run of layers from the bottom with the same N, is redrawn
every Nth frame of the formula's grid (its `fps`, else 50), a band of rows
a frame: row r (from the top) is in band `floor(r·N/32)`, and in frame k
it shows the slow layers as at frame `k − ((k − band) mod N)`, their `t`
taken back to it (the clock, sliders and `seed` as now). A slow layer that
reads no `t` is redrawn only when what it reads changes (once a second for
a clock's `second`). The layers above are drawn over it every frame. Rules
(E941-E947): `rate` only on the bottom run, one N, at least one layer above
it, the bottom one opaque (no blend, no coverage), no `ink` or `ur ug ub
ul` in a slow layer; inside a part `rate` is left out (as `fps` is). Cost: the slow layers' pixel steps
÷ N, plus a row copy; as a look the slow layers are drawn every frame.
Commutative blends (`multiply`, `min`, `max`, `add`, `screen`) let a
static mask sit at the bottom: Big clock's digits as `layer rate 50 {
gray(lit) }` under `layer min { hsv(…) }` draws the same frames as the
one-layer Big clock. Fast motion in a slow layer shows bands moving at
different times: use it for what changes slowly.

## 8. Over a GIF

A formula can draw over the GIF playing. The bottom of the stack is the
picture under the formula: the bottom layer, a blend other than `over`, or a
coverage below 1, let it show through, and `ur ug ub ul` read it at the
pixel. A formula that does neither replaces the GIF (and is refused as a
GIF look as it is). Alone, the picture under it is black. Looks are capped
at 12 ms (a look over 8 ms moves at 25 frames a second). **Any** formula
can be a look: a **`look screen 40`** line at its top (section 4) says how it
goes on — the player pastes the formula in as a part and draws it with that
blend and coverage, `part look { … }` then `layer screen { look(x, y, 0.4) }`
(`opendmd_psi::look`) — so a model never writes that wrapper itself. A
formula with layers of its own goes over only (`look` or `look over 60`). A look's frames follow the GIF's: a formula's
`fps` line is not used over a GIF.

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
  So any formula can be pasted in unchanged. A part's `fps` line is
  allowed and has no effect: the frame rate is the whole formula's.
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

## 10. Drawing: lines and dots, once a frame

```text
draw {                      -- once a frame, before the pixels; the canvas starts empty
  for k < 192 {             -- k = 0, 1, … 191 (a whole number 1..1024, written out)
    … values of k …         -- names set in a loop are its own
    line(x0, y0, x1, y1, v0, v1)   -- or dot(x, y, v)
  }
}
pal(ice, ink)               -- the pixels read the canvas as ink, 0..1
```

- **Coordinates are pixels**, x across from the left (0..127), y **up from
  the bottom** (0..31), as `px` and `py`; fractions round to the nearest
  pixel (½ goes up). Anything off the panel is clipped; ends are held to
  −512..639 × −512..543.
- **Ink** is 0..1 (default 1; `line` with one ink uses it at both ends): a
  line's ink runs evenly from `v0` at its start to `v1` at its end in 1/255
  steps. Where shapes overlap the brighter wins (`max`), so crossing lines
  never darken and near edges beat far ones without sorting. No
  antialiasing: one pixel wide, as the Starfield and Wireframe cube examples draw.
- **Inside `draw { }`** only values that are the same for every pixel: `t`,
  the clock, the sliders, `frame { }` values, tables, loop counters. A loop
  counts with a literal number; loops nest two deep; the loops' turns in a
  frame (each run of a body counts one, an inner loop's every run) are at
  most 2,048. A value inside a loop that doesn't use its counter is worked
  out once a frame anyway (hoisting, as everywhere). Names set in `draw` but
  outside its loops are seen after it, as `frame { }` values are.
- **`ink`** is what was drawn at this pixel. `ink` with no `draw { }` is an
  error; a `draw { }` that nothing shows (no `ink` in a colour that can be
  seen) is left out. One `draw { }` per formula (the formula's, or one
  part's: there is one canvas). Inside a part drawn elsewhere, `ink` is
  still the real pixel's: the canvas is not moved.
- **`ink(n)`** (n a whole number, 1 to 255) is the canvas in `n` levels, a
  whole number: what was drawn with ink `k/n` reads `k`, exactly, for every
  k ≤ n. Draw each kind of thing with its own tag and colour them with a
  `pick`: `line(…, 6/16)` for the hills, then `pal(P, pick(ink(16), …))`.
  One step a pixel (~90 ns), no `+ ½` offsets, no `floor`.
- **Tables**: `table V = -1, 1, 0.5` at the top of the formula (or a part),
  numbers only, 256 in all; `V[k]` is number `floor(k)` counting from 0, and
  0 outside the table. Any level: per pixel `V[px % 3]` is one step, like
  `pick`. A comma may end a line and the numbers go on.
- **Colour comes from the pixels**: `pal(ice, ink)`, `#40c0ff * ink`,
  `hsv(x, 1, ink)`, `layer screen { gray(ink) }` over a GIF — the canvas
  holds one number a pixel, the colour is the last line's.
- **Every frame starts empty**: nothing carries over from one frame to the
  next (trails are the panel's glow's job), so a frame is still a pure
  function of `t`.

```psi
-- twelve spokes turning, brighter at the rim
loop 4
draw {
  for k < 12 {
    an = k/12 + t/4
    line(64, 16, 64 + cos(an)*40, 16 + sin(an)*14, 0.2, 1)
  }
}
hsv(x, 0.6, ink)
```

## 11. Going on without repeating: `loop forever`

`loop forever` (instead of `loop N`) is a formula with no loop: `t` is the
seconds since it started and grows without end, so a drive, a sky or a
cloud field never comes back to where it was. Each time it plays it
starts at `t = 0`, with a new `seed` (0..1, the same for the whole play):
add `seed*1000` to a distance to start somewhere else each time.

`t` here is kept exactly, however long it plays (to 1/65,536 s, for
millions of years), but a value that grows without end can only go where it
can come back to an ordinary number. Once a frame — `t`, and what you make
of it with values that are the same for every pixel — it may be:

- added to, taken from, multiplied or divided by values the same all frame
  (`t*0.15 + seed*1000`, `t/8`, `t*a`): still growing;
- taken back to an ordinary number by `frac(…)`, `% n`, `sin cos tri saw
  sqr`, `noise(…)`, `hash(…)`, a comparison (`t > 20`, `step(30, t)`),
  `min`/`max`/`clamp`, `smoothstep(0, 20, t)` (a fade-in) or `floor(…)` then
  one of those. Each gives exactly what it gives for a short `t` (a `loop
  forever` formula draws the same frames as with `loop 3600` while its
  values fit a number), and carries on past it without stopping or
  jumping: `frac(t*10)` keeps ticking ten times a second after 55 minutes,
  where `loop 3600`'s `t*10` stops at 32,767.

And it may have **a part per pixel or per row added to it** (`t*0.5 + x*8`,
`u + ZH[py]`) when that goes straight into `frac`, `sin cos tri saw sqr`,
`noise` or `% n`, or is multiplied by a value the same all frame (`(t + x)*0.3`),
on the way there (the growing part's fraction is added in: exact).
Divided, it is an error (write `t/3 + x/3`); so is anything else: `t*x`, `sqrt(t)`, `t*t`, a table read at
`t`, a growing value as a coordinate in `draw { }`. A colour line held to
0..1 anyway takes it as it is (`gray(t/60)` is white from a minute on);
`hsv`'s hue takes its fraction.

```psi
loop forever
fps 100
frame { u = t*0.15 + seed*1000 }   -- a distance, in noise cells: grows without end
ahead = noise(u + y*0.5, 0, 0)     -- per row: the track a little further on
glow = smoothstep(0, 5, t)         -- a fade-in over the first five seconds
hsv(ahead + x*0.2, 0.8, glow*noise(x*8 + t, y*4, u))
```

Without an envelope or a blend at a loop's seam, a drive here has none. A
part pasted in with a `loop N` of its own still repeats every N seconds
(its `t` is `t % N`); a `loop forever` part inside a looped formula reads
the formula's `t`.

## 12. Limits

| limit | value | error says |
|---|---|---|
| source | 16,384 characters, comments included (a shared link past ~1,500 may be cut by apps; the gallery takes files of 4 KB) | too long |
| steps | 256 in all: per frame + per row + per pixel + one per layer | too long for the panel (N steps) |
| live per-pixel values at once | 12 | holds more than 12 values at once |
| constants + per-frame/per-row values held at once | ~119 (a value's slot is used again once nothing reads it any more: a per-frame value only the frame's steps read frees its slot for the rows' values, a row value the pixels don't read for the next one; constants and values the pixels read hold theirs all frame) | needs more than 128 numbers and once-a-frame values |
| nesting | 18 levels of brackets and operators (a part level counts 3) | nested too deeply / parts go too deep |
| `loop` | 0.1..3600 s, or `forever` | between 0.1 and 3600 seconds |
| values that grow without end (`loop forever`: `t` and what is made of it once a frame) | 15 at once | too many values that grow without end |
| `noise` of a growing value with a part per pixel or per row | 4 different growing values | four different ones at most |
| `fps` | 25, 50 or 100 | `fps` is 25, 50 or 100 frames a second |
| re-reading parts drawn at other places | bounded | draws its parts in too many places |
| `for` count | a whole number 1..1024, written out | `for` takes a name and a count |
| loop turns a frame (every run of a loop's body) | 2,048 | draws too much a frame |
| loops inside loops | two deep | two deep at most |
| `table` numbers | 256 in all (sprites share them) | a table holds numbers only, up to 256 in all |
| `palette` (your own) | 1 to 32 colours, 3 palettes; each takes 32 of the 256 table numbers | a palette is 1 to 32 colours / 3 palettes of its own at most / tables, sprites and palettes hold 256 numbers |
| `sprite` | key: 16 characters; rows: 64 wide, 32 tall; stored in the 256 table numbers (a 26 × 9 picture with 4 key characters takes 23) | `X` is not in the sprite's key / as wide as its first / 1 to 32 rows / tables, sprites and palettes hold 256 numbers in all |
| `draw { }` blocks | one (the formula's or one part's) | there can be one `draw { }` block |
| lines and dots drawn a frame | 2,048; and 16,384 line steps (pixels walked, on the panel or off; a line wholly off one side walks none): past either the rest are skipped, the same on every device | — |

Unused names and parts cost nothing; constants fold (`2*3` is 6; `x/8` is
a multiply).

## 13. The cost model

The panel's processor runs each step for 4,096 pixels; the estimate is
`(45 + Σ per-pixel step costs) ns × 4096` (+ ~10 µs per per-row step),
then × 1.15 for what playing it adds (the copy to the panel, the picture
under a look), where every colour costs 120 to write when it varies per
pixel (about 5 when it is one value along a row), so `gray(x)` is 165.
It is calibrated on the device (within ~12 % for layers and looks).
**Hoisting is automatic**: a value is computed as rarely as its inputs
change — constants once, `t`/clock/slider-only values once a frame,
`y`/`py`-only values once per row (32 times), the rest per pixel. So
`sin(t/8)` is free and `sin(y*4)` nearly free; only per-pixel work counts.
**Ranges are automatic too**: the panel knows each value's range from
`x y px py i` (0..1, 0..127, …), `t` (0 to the loop's length), the sliders
(0..1), the clock, `min`, `floor`, `sin`, `%`, comparisons and the
constants, and a `+ − *` whose result can't leave the number range skips
its overflow check (the cheaper rows below); a multiply by 1/2ⁿ is a shift.
`min(near, 10)*min(near, 10)` is cheaper than an unbounded `near*near`.

| per pixel | ns | | per pixel | ns |
|---|---|---|---|---|
| + − compare min max step and or `%`2ⁿ | 50 | | `sin` `cos` | 110 |
| `*` | 86 | | `tri` `saw` `sqr` `select` `clamp` | 60 |
| + − that can't overflow | 28 | | `*` that can't overflow | 41 (37 when the product is small) |
| `*` by 1/2ⁿ (`/2`, `/8`, `*0.25`) | 33 | | | |
| `%` (other) | 113 | | `mix` | 130 |
| neg abs floor frac not | 40 | | `smoothstep` | 214 + a division if its edges vary |
| `/` by a varying value | 360 (`1/v` 230) | | `hash` | 220 |
| `sqrt` | 229 | | `atan2` | 384 |
| `len` | 472 | | `noise` | 855 |
| `noise` of a growing value (section 11) | 870 | | | |
| colour `rgb` | 120 | | colour `hsv` | 195 |
| `ur` `ug` `ub` | 70 | | `ul` | 140 |
| `digit` `glyph` | 240 | | `bit` | 55 |
| `pick` (values the same for every pixel) | 100 | | `pick` of per-pixel values | its compares and selects |
| a sprite's pixel, `s[u, v]` | 120 | | | |
| each extra layer | its colour + 190 (multiply 245, screen 300, max/min 180; with coverage add 575, multiply 630, screen 600, max/min 450, over 390) | | showing the picture under | free |

**The draw stage** adds, a frame: ~420 ns for each step each time it runs
(a loop's body counts as often as it turns, its end too), a line ~1,100 ns +
320 a pixel walked (its longer side when that side's ends are constants, at
least 12 pixels otherwise), a dot ~400, and 10 µs to clear the canvas; per
pixel `ink` is 70 and `T[k]` 100 (measured on the device). **A loop of 32
turns or more runs as lanes**: each step of its body is worked out for
128 turns at once, at about the table's per-pixel cost a turn (`hash` 220,
`*` 86, `+` 50, `/` 685: the same functions, the same picture), then its
lines and dots are drawn turn by turn as above; so 192 streaks of ~15
steps each are ~1 ms of values (two lanes of 128) plus their lines, not
~2.5 ms. A loop inside another, or one under 32 turns, runs a turn at a
time.

**Common pairs are one step** (the same picture, worked out in one pass):
a checker `(floor(u) + floor(v)) % 2` (~75 ns saved), `a*k + c` (~20),
and the colour `pal(P, c ? A : B)` (~190 saved with `* k` of a row value,
~130 with a per-pixel `k`), `pal(P, pick(m, …))` (~50), `pal(P, ink)` and
`gray(ink)` (~70). Nothing to write differently: the panel finds them.

**Two things are worked out less often than per pixel, by themselves**
(the same pictures either way):

- **The column run.** A value of `x`/`px` and per-frame values only (`sin(x*3
  + t/8)`, `px*k + s`, a rotozoom's column wobble) is the same down a
  column: the panel works such values out on the first row of a frame and
  keeps them, so they cost a 32nd of the table's figure. Write a value that
  is a column's part plus a row's part with the brackets that say so:
  `u = (cu + cs*sc*dx) + (wob - sn*sc*dy)` is one add a pixel, not four
  steps. (It holds a register each for the whole row.)
- **The still cache.** A value of the pixel's position alone (`x y px py
  i` and constants: `len(cx, cy)`, `atan2(…)`, `1/len(…)`, `r`, `angle`) is
  the same every frame. The compiler picks the **two** such values that save
  the most (each at least ~0.4 µs a pixel), and the panel works them out
  once and keeps them (16 KB of its memory each) while it can spare the
  memory; then they and the steps only they need cost nothing a frame. It is
  an effect's only (not a look's: a look is estimated without it), and the
  panel may let it go at any time (then every step runs, slower, the same
  bytes). `psi check` prints both estimates. So a tunnel's depth and angle
  are free, but `depth*a` is not still (a slider moves): keep the sliders
  out of the values you want kept (`twist = 1 -- try 0 to 3`).
- **Ask the compiler.** `psi check` prints notes with their lines: what
  the still cache and the column run took, the heaviest value they could
  not take and what kept it out, a `pick` made per pixel by one per-pixel
  arm (`pick(m, a, b, c*px)` is ~2n+2 steps a pixel; `pick(m, a, b, c) +
  extra` is one), a left-to-right sum or product that works a frame or row
  part out per pixel (`px - 64 - X` → `px - (64 + X)`; products may round
  differently in the last bit), and a draw loop of 32+ turns run as lanes
  (costed for 128 turns a lane). `psi check --levels` gives every value's
  level and the chain to the input that set it; `psi check --same-as
  old.psi` proves a rewrite draws the same frames.
- **noise at a whole z** (`noise(x, y, 0)`, any whole constant third
  value) is ~470 ns, half the 855 of a moving one.

Ratings: **light ≤ 5 ms**, **medium ≤ 8 ms**, **heavy** beyond (runs at
25 frames a second or fewer; `fps 100` needs ≤ 5 ms, else it plays at 50; GIF looks take ≤ 12 ms, and the worked
examples here ≤ 10). Division by a constant is a
multiply. One `noise` or `len` is fine; four are heavy. Per-pixel `px`
arithmetic costs the same as `x` arithmetic.

## 14. Idioms

- **Without end**: `loop forever`; move by `t*k` inside `frac`, `sin`,
  `noise`; start somewhere else each play with `+ seed*1000` (section 11).
- **Fast motion**: `fps 100` (a line of its own, like `loop`) draws 100
  frames a second, for things that move several pixels a frame (streaks,
  wireframes): smoother, if the formula is light (≤ 5 ms; heavier ones play
  at 50). Most formulas need no `fps` line.
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
- **Particles** (stars, sparks, snow): `draw { for k < N { … } }` with each
  particle's start from `hash(k, …)` and its place from `t`: `frac(hash(k,
  1) + t/L)` cycles it once a loop. A streak is a `line` from where it was
  a moment ago to where it is, its ink fading towards the tail; a point is
  a `dot`. Hide a particle that has left the panel with ink 0 at both
  ends (`line(…, lit*shown, lit*shown)`): such a line, like one wholly off
  one side of the panel, walks no steps, but one partly on it walks all of
  them, off the panel too, and they count against the 16,384 a frame.
  Example 31.
- **A wireframe**: the corners in one `table` (x, y, z after each other),
  the edges in another (pairs of corner numbers); `for e < EDGES`, read the
  two corners (`V[E[e*2]*3 + 1]` is the first corner's y), turn them with
  per-frame `sin`/`cos` and project (`64 + x*36/(z + 128)`), then `line`
  with ink from the depth. Example 32.
- **Ranges**: keep `hsv`'s value and `rgb`'s channels in 0..1 by design
  (`*0.5 + 0.5` maps −1..1 to 0..1); out-of-range is clamped, so overshoot
  just flattens.

## 15. Common mistakes

Each block below fails to compile with the sentence in its first comment.

```psi error
-- error: grows without end in `loop forever`
loop forever
gray(frac(t*x))
```

`t*x` would be a value per pixel that grows without end. Make the growing
part a frame's and add the pixel's: `frac(t + x)`, `sin(x*3 + t/8)`.

```psi error
-- error: `seed` is for formulas that go on without repeating
gray(seed*x)
```

Only a `loop forever` formula has a `seed`.

```psi error
-- error: `Y` is not in the sprite's key
sprite s {
  key . 0, X 1
  .XY.
}
gray(s[px, ty])
```

Every character of a sprite's rows needs its number in the key line.

```psi error
-- error: a palette is 1 to 32 colours
palette sea = #003, 0.5, #fff
pal(sea, y)
```

A palette holds colours only (`#…`).

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

A value may be set again, and each use reads the latest above it: `v = x`
then `v = v * 2` is `x * 2` from there on (a layer, a part or a loop may
set a name from outside too: its own until it closes; `p.v` is the last `v`
on the part's own lines, its layers' left out). Parts, tables, sprites, palettes and controls keep their names:

```psi error
-- error: is already the name of a table
table T = 0.5, 1
T = 2
gray(T[0])
```

```psi error
-- error: `frame { }` is worked out once a frame
frame { k = x * 2 }
gray(k)
```

```psi error
-- error: is a whole colour: it can only be the last line
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
-- error: is a colour, not a number
glow = #ffcc00
glow * x
```

A colour has no name: write it where it is used, in the colour line
(`#ffcc00 * x`).

```psi error
-- error: is a colour, not a number
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

```psi error
-- error: `draw { }` runs once a frame
draw { dot(px, 16) }
gray(ink)
```

The draw block runs once a frame, not per pixel: draw at numbers worked out
from `t`, the sliders, tables and loop counters.

```psi error
-- error: `ink` needs a `draw { }` block to read from
gray(ink)
```

```psi error
-- error: `for` takes a name and a count
draw { for k < n { dot(k, 1) } }
gray(ink)
```

A loop's count is a number written out (the cost must be known): loop to
the most and leave some out (`dot(k, 1, k < n)`: ink 0 draws nothing and
costs no steps).

```psi error
-- error: is a loop counter: it can't be set
draw { for k < 4 { k = k*2
  dot(k, 1) } }
gray(ink)
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
- A `draw { }` that shows nothing: no `ink` in the colour (it is then left
  out), or y counted from the top (`line` and `dot` count **up** from the
  bottom, as `py`), or every line off the panel.
- `x`, `y`, `i` and `r` are built in: name a star's place `sx`, `hy`, …

## 16. Worked examples

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

Two controls, declared by `control` lines, shaping the pattern.

```psi example=slider-stripes
control width = 0.5 "Stripe width"
control tilt = 0.25 "Tilt"
loop 4
size = 6 + width*24
band = sin((px + py*(tilt*4 - 2))/size - t/2)
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

Reading the picture under: its colour and brightness, mixed by a control
(a colour expression: `mix` of two colours, each `rgb(…)` of values).

```psi example=sepia-look
control amount = 0.5 "How much"   -- 0: the GIF as it is
look
mix(rgb(ur, ug, ub), rgb(ul*1.1 + 0.08, ul*0.92 + 0.04, ul*0.65), amount)
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
lit = glyph(ch, floor(u/2) % 5, floor(v/2))   -- each font pixel 2 x 2
hsv(x - t/4, 0.8, lit)                -- a rainbow sliding through, once a loop
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
lit = glyph(pick(cell, 'R', 'G', 'B'), floor(u/2) % 5, floor(v/2))
pick(cell, #ff3333, #33ff55, #3377ff) * lit
```

### 31. Starfield, drawn (particles)

192 stars from `hash(k, …)`, each a streak from where it was a moment ago
to where it is, flying at the viewer through a field 4,096 deep once a
loop, at 100 frames a second; the ink fades from the dark tail to the head
and in from the distance (squared, so far stars stay dark); a star whose
head has left the panel is drawn far below it, where a line is skipped
without walking a step. Written for few steps a star (every step runs 192
times): the perspective is one division each end, the off-panel test picks
one row for both ends. ~5.0 ms estimated: 100 fps (the first version with
these streaks estimated 6.0; the one before the eye test measured 4.4 ms).

```psi example=starfield-drawn
loop 2.4
fps 100
-- Starfield: 192 stars flying at you, a pass through the field a loop,
-- each a streak from where it was a moment ago
-- a: streak length
frame { back = 1 + 1/64 + (144 + a*240)/4096 }
draw {
  for k < 192 {
    sx = hash(k, 7)*32 - 16; sy = hash(k, 3)*8 - 4  -- where it is, across and up
    near = frac(t/2.4 - hash(k, 11))          -- 0 far, 1 at you
    s = 1/(65/64 - near); st = 1/(back - near)  -- perspective, now and a moment ago
    hx = sx*s; hy = sy*s                      -- the head, from the middle
    -- a head off the panel: the line moved far below it, where it is skipped
    row = abs(hx) > 64 or abs(hy) > 16 ? -999 : 16
    lit = near*near                           -- squared: far stars stay dark longer
    line(64 + sx*st, row + sy*st, 64 + hx, row + hy, lit*0.2, lit)
  }
}
pal(ice, ink)
```

### 32. Wireframe cube (tables and a projection)

The corners in one table, the edges in another; each edge reads its two
corners, turns them about y then x by per-frame angles, projects them and
draws the line, its ink from the depth (near edges brighter, crossing
handled by the max rule), at 100 frames a second (~2.9 ms). The colour is
the last line's: one edit.

```psi example=wire-cube
loop 4.8
fps 100
-- Wireframe cube: turning about two axes and swinging across; the near
-- edges brighter. The colour is the last line's #40c0ff
-- a: size
table V = -1,-1,-1, 1,-1,-1, 1,1,-1, -1,1,-1, -1,-1,1, 1,-1,1, 1,1,1, -1,1,1
table E = 0,1, 1,2, 2,3, 3,0, 4,5, 5,6, 6,7, 7,4, 0,4, 1,5, 2,6, 3,7
frame {
  sa = sin(t/4.8); ca = cos(t/4.8); sb = sin(t/4.8 + 1/6); cb = cos(t/4.8 + 1/6)
  size = 34 + a*28; mid = 64 + sa*40
}
draw {
  for e < 12 {
    m = E[e*2]*3; n = E[e*2 + 1]*3            -- the edge's two corners
    x0 = V[m]*size; y0 = V[m + 1]*size; z0 = V[m + 2]*size
    x1 = V[n]*size; y1 = V[n + 1]*size; z1 = V[n + 2]*size
    xa = x0*ca + z0*sa; za = z0*ca - x0*sa; ya = y0*cb - za*sb; zb = y0*sb + za*cb  -- turned
    xc = x1*ca + z1*sa; zc = z1*ca - x1*sa; yc = y1*cb - zc*sb; zd = y1*sb + zc*cb
    line(mid + xa*36/(zb + 128), 16 + ya*36/(zb + 128), mid + xc*36/(zd + 128), 16 + yc*36/(zd + 128),
         clamp(0.594 - zb/148, 0.156, 1), clamp(0.594 - zd/148, 0.156, 1))
  }
}
#40c0ff * ink
```

### 33. Tunnel (the still cache)

The built-in tunnel's look: per pixel a depth (`128/len`, the tunnel twice
as tall as wide) and a place round the walls (`atan2`), a checker turning
and travelling, the palette cycling, lit by depth in eight steps with a
light going round. `near` and `round` depend on the pixel alone, so they
are the still cache's: ~4.6 ms a frame with it (`fps 100`; the spare
measured 4.0 ms as an effect, p90 4.1), ~14.4 without (a look, or the panel
short of memory: then the panel holds it at a slower rate). The twist is a number in the source, not a slider, so
that `round` stays still; `near` (16 − depth, 0 in the black middle) makes
the fade one `min` and the black middle a comparison.

```psi example=tunnel
loop 4.8
fps 100
-- Tunnel: the time warp. Each loop the walls turn once, travel two
-- checker lengths, and the palette goes round once.
-- From the vanishing point, the tunnel twice as tall as wide:
dx = px - 63.5; dy = (15.5 - py)*2
-- how far each pixel is from the black middle: 0 there (where the
-- tunnel is deeper than 16), ~15 at the edges
-- (this and `round` depend on the pixel alone: worked out once, kept)
near = 16 - min(128/len(dx, dy), 16)
-- round the walls: 16 checker cells a turn, twisting with depth
twist = 1                              -- try 0 to 3
round = atan2(dy, dx)*16 + (16 - near)*twist/8
-- the checker, turning and travelling
u = round + t/0.3; v = 16 + t/0.6 - near
cell = (floor(u) + floor(v)) % 2
-- lit by depth (full near, fading as a square, black in the middle) and
-- by a light going round with the walls, in eight steps
fade = min(near, 10)                    -- 10 near, to 0 where it turns black
side = 0.0635 + 0.0159*cos(u/16)
light = floor(fade*fade*side + (near > 0))/8
pal(chrono, cell ? frac(0.711 + t/4.8) : frac(0.336 + t/4.8)) * light
```

### 34. Rotozoom (the column run, a picker)

Mode 7: each pixel's place on a turning, breathing checker is a column's
part (`cs*sc*dx`, the column wobble) plus a row's (the row wobble, the
rest), so a pixel adds, floors and looks up; the noise under it is
`noise(u, v, 0)` (a whole z: the cheaper noise). Slider `a` is a picker
(`-- a: wobble = gentle, strong, off`: the editor shows three buttons, 0,
½ and 1; the formula reads `pick(floor(a*2 + 0.5), …)`). ~6.0 ms (50 fps).
The floor (the native's `floor=1`) is its own example, **Rotozoom floor**:
each row's scale by its distance (a per-row value, so the column parts are
gone), one layer, `pal(fire, …) * fog`, ~4.3 ms at `fps 100`.

```psi example=rotozoom
loop 9.6
-- Rotozoom: Mode 7. A checker turning once a loop, its zoom breathing
-- twice, gliding two checkers across and one down; under it a noise
-- texture turning the other way twice as fast; rows and columns wobble.
-- a: wobble = gentle, strong, off
frame {
  sn = sin(t/9.6); cs = cos(t/9.6)                 -- the turn
  s2 = -sin(t/4.8); c2 = cos(t/4.8)                -- the noise's, back twice
  zoom = 2                                         -- try 1 to 4
  sc = 1/((1 + sin(t/4.8)/2)*zoom*8)               -- checker cells a pixel
  sc2 = sc*0.75
  cu = t/0.6; cv = t/1.2                           -- the glide, in cells
  amp = pick(floor(a*2 + 0.5), 3, 4.5, 0)/8        -- the wobble, in texels
}
dx = px - 64; dy = 15 - py
-- the wobble: rows slide sideways, columns up and down, on travelling waves
wob = sin((31 - py)/32 + t/3.2)*amp
col = sin(px*0.0234375 - t/4.8)*amp
-- where each pixel is on the checker: its column's part plus its row's
u = (cu + cs*sc*dx) + (wob - sn*sc*dy)
v = (cv + sn*sc*dx + col) + cs*sc*dy
cell = (floor(u) + floor(v)) % 2
u2 = (cv + c2*sc2*dx) + (wob - s2*sc2*dy)
v2 = (cu + s2*sc2*dx + col) + c2*sc2*dy
pal(fire, (cell ? 0.44 : 0.19) + noise(u2, v2, 0)*0.6)
```

### 35. Clouds without end (`loop forever`)

A sky that drifts for ever and is a different sky each time it plays:
two noises of a distance that grows without end, one per pixel (the clouds)
and one per row (a haze band), and `seed` to start somewhere else.

```psi example=clouds-forever
loop forever
-- clouds drifting for ever, a new sky each time it plays
frame { u = t*0.05 + seed*1000 }        -- grows without end
puff = noise(x*6 + u, y*3, u*0.3)        -- per pixel: the clouds
haze = noise(u*2 + y, 9, 0)*0.3          -- per row: a band of haze
k = smoothstep(0.45, 0.75, puff + haze*0.5)
mix(#204080, #f0f0ff, k)
```

### 36. Invaders (a sprite sheet)

Two frames of an invader in one sprite (the second 8 rows below the
first), a row of nine across the panel marching side to side, the frame
switching twice a second; the key's numbers are brightness.

```psi example=invaders
loop 4
sprite inv {
  key . 0, X 1, o 0.6
  ..X.....X..
  ...X...X...
  ..XXXXXXX..
  .XXoXXXoXX.
  XXXXXXXXXXX
  X.XXXXXXX.X
  X.X.....X.X
  ...XX.XX...
  ..X.....X..
  X..X...X..X
  X.XXXXXXX.X
  XXXoXXXoXXX
  XXXXXXXXXXX
  .XXXXXXXXX.
  ..X.....X..
  .X.......X.
}
frame {
  sway = floor(tri(t/4)*6)               -- side to side, a pixel at a time
  f = floor(t*2) % 2                     -- which frame
}
u = px - 2 - sway
k = inv[u % 14, ty - 10 + f*8] * (u >= 0) * (u < 123)   -- nine, 14 px apart
hsv(0.33 - ty/100, 0.7, k)
```

### 37. Night city (a palette of its own, a sprite for its index)

Windows lit at random in a skyline: the colours are a palette of the
formula's own (sky, building, three window colours), each pixel picks its
place in it — a sprite of a building's pattern, a hash per window and a
per-row sky — with one `pal` step instead of three `pick`s.

```psi example=night-city
loop 8
palette city = #050812, #141a2c, #2a2f44, #ffd070, #fff4c0
sprite block {
  key . 2, w 3
  .w.w.w.
  .......
}
frame { blink = floor(t*2) }
col = floor(px/7); top = 10 + hash(col, 3)*16      -- each building's height
on = hash(col*31 + floor(ty/2), blink) > 0.6       -- this window lit just now
inside = ty > 31 - top
k = inside ? block[px % 7, ty % 2]*on + (1 - on)*2 : (py > 24 ? 0 : 1)
pal(city, k/4)
```

## 17. In the product (not built)

This reference and `psi check` are tooling only: nothing in OpenDMD calls a
model. If the owner wants it later, the natural place is the Formulas
editor: a "Describe an effect…" box whose text, with this document as the
system prompt, asks a model for a formula; the editor compiles the answer
locally (errors and cost come from the same compiler), shows it in the
preview, and could loop a few times on compile errors before showing
anything. The decision (provider, keys, cost, privacy) is the owner's.
