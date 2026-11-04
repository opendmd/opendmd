# Psi language reference

**Psi** is the language OpenDMD's **Formulas** are written in: a few lines
of arithmetic, worked out for every pixel of the 128 × 32 panel, every
frame, ending in a colour. This page is the reference: what the language
is, every name it knows, its limits and its error codes. To learn it, start
with [the guide's Formulas page](guide/formulas.md); to try it, open the
[demo](/demo/)'s Formulas.

The tables of names, the limits and the error codes on this page are
generated from the compiler's own tables, and the tests fail when they
drift, so what this page says is what the panel does. On the site, the
search box filters the built-ins and the error codes (press `/`), and
**Cheat sheet** folds the page down to the tables and the error titles.

## Versions

This page describes **Psi 1**. A formula may say which version it is
written for with `psi 1` on its first line; it never needs to, because a
formula without the line is version 1, and always will be.

- Within version 1, new built-in names may be added. A name a formula sets
  is that formula's own, even where a later version made it a built-in, so
  a formula saved before a name existed means what it meant (the names
  under "Every name" that a formula may set).
- Anything that would change the pixels of a formula that compiled under
  version 1, or refuse one, is a new version. The tests hold every example
  and gallery formula to the same frames.
- A formula written for a later version than the panel has (`psi 2` on a
  version 1 panel) is refused with a clear sentence (`E601`) rather than
  failing somewhere in the middle.

## Text

- A formula is text, at most 16,384 characters, comments included.
- **Lines.** Each statement is a line; `;` ends a statement too (`a = 1;
  b = 2`). Inside `( )` and `[ ]` a line end is a space, and after an
  operator or a `,` the statement goes on onto the next line.
- **Comments** run from `--` or `//` to the end of the line.
- **Numbers** are digits with at most one point, no sign and no exponent:
  `12`, `0.5`, `.5`. A minus sign is an operator.
- **Characters** in single quotes are numbers: `'A'` is 65, one printable
  ASCII character at a time. There are no strings.
- **Colours** are `#` and 3 or 6 hex digits: `#f80` is `#ff8800`.
- **Names** are a letter or `_`, then letters, digits and `_`.
- **Text in quotes** (`"…"`) goes only on `name`, `about` and `control`
  lines, on one line, without `"`, `<`, `>`, `\`, tabs, control characters
  or characters that hide or reorder text.
- A **sprite**'s rows (`sprite NAME {` on a line of its own, then a key line
  and rows of characters, then `}` alone) are read as written, a line at a
  time.

## Grammar

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
          | NAME "=" expr                     -- may set a name again: a read sees the latest
          | "layer" [BLEND] "{" { assign } colourline "}"
          | "fn" NAME "(" [ NAME { "," NAME } ] ")" "=" expr-or-colourline   -- a function (section 9)
          | "fn" NAME "(" [ NAME { "," NAME } ] ")" "{" { assign | dstmt } last "}"
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
          | FN "(" args ")"                    -- a function whose last line is a colour
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
atom      = NUMBER | CHAR | COLOUR | NAME | FUNC "(" args ")" | FN "(" args ")"
          | TABLE "[" expr "]" | SPRITE "[" e "," e "]" | "(" expr ")"
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


The declarations (`decl`) come first, before every other line, each once
(`control` up to three times). The colour line is the last line of the
formula, of a function and of a layer; with layers, every colour is inside one.

## Numbers

Every value is a fixed-point number, from −32,768 to 32,767.99998 in steps
of 1/65,536 (Q16.16). Arithmetic saturates rather than wrapping. `a / 0` is
the largest number of `a`'s sign (0 for `0 / 0`); `a % 0` is 0; `%` is
floored (`-1 % 3` is 2). Comparisons and `and`, `or`, `not` give 1 or 0;
any number but 0 is true. Both sides of `? :` are worked out.

Where a function is defined by a table (`sin`, `noise`, `hash`, `atan2`,
the palettes, the fonts) the table is the definition. The normative
statement is that **a formula gives the same bytes on the panel, in the
browser and on a computer**; golden frames in the tests hold every platform
to it.

## How a formula runs

- **Levels.** Each value is worked out as rarely as what it reads changes:
  constants once, values of the time, the clock and the controls once a
  frame, values of `y`, `py` and `cy` once a row,
  the rest once a pixel. This changes only the cost, never the result.
- **Layers** (`layer [blend] [rate N] { … }`) are drawn bottom to top, each
  with its own names and colour, onto what is below with its blend (`over`,
  `add`, `multiply`, `screen`, `max`, `min`); a colour's last value says how
  much of the pixel it covers.
- **Functions** (`fn p(a, b) = …` or `fn p(a, b) { … }`) are written out
  where they are called: a value, a colour, or lines that draw. A function
  whose values are `x, y` is drawn at another place: `px py i cx cy r
  angle` inside follow them.
- **`draw { }`** runs once a frame before the pixels: loops (`for k < n`),
  lines and dots into a canvas the pixels read back as `ink`.
- **Time.** `t` is seconds into the loop (`loop N`, 8 when left out), always
  a whole number of frames (`fps 25`, `50` or `100`; 50 when left out); with
  `loop forever`, seconds since the formula started, growing without end.

## Every name

<!-- psi-builtins: names (generated from opendmd-psi/src/builtins.rs: PSI_BLESS=1 cargo test --features docs --test builtins) -->
| kind | names |
| --- | --- |
| values | `x`, `y`, `px`, `py`, `i`, `t`, `hour`, `minute`, `second`, `a`, `b`, `c`, `ur`, `ug`, `ub`, `ul`, `ink`, `seed` |
| values about the middle | `cx`, `cy`, `r`, `angle` |
| constants | `w`, `h` |
| functions (how many values) | `sin` (1), `cos` (1), `tri` (1), `saw` (1), `sqr` (1), `abs` (1), `floor` (1), `frac` (1), `fract` (1), `sqrt` (1), `min` (2), `max` (2), `step` (2), `hash` (2), `len` (2), `atan2` (2), `clamp` (3), `mix` (3), `smoothstep` (3), `noise` (3), `select` (3), `bit` (2), `digit(…)` (3 values and a font), `glyph(…)` (3 values and a font), `pick(k, a, b, …)` |
| colours (how many values, before the coverage) | `rgb` (3), `hsv` (3), `pal` (2), `gray` (1), `cycle` (3) |
| palettes | `fire`, `ice`, `ocean`, `chrono`, `amber`, `rainbow`, `gray` |
| fonts | `normal`, `small`, `bold` |
| blends | `over`, `add`, `multiply`, `screen`, `max`, `min` |
| keywords | `loop`, `layer`, `part`, `use`, `draw`, `for`, `table`, `fn` |
| drawing | `line`, `dot`, `pset`, `rect`, `rectfill`, `circ`, `circfill`, `trifill`, `cls`, `spr`, `print` |
| declarations | `psi`, `name`, `about`, `control`, `look` |
| words that are keywords in one place only | `fps`, `forever`, `sprite`, `palette`, `key`, `rate` |

A formula may set a name added after people had saved formulas, and then it is the formula's own: `cx`, `cy`, `r`, `angle`, `bit`, `digit`, `glyph`, `pick`, `cycle` and `seed`, `ink`, `line`, `dot`, `forever`.
<!-- psi-builtins end -->

## What each does

These are the editor's own words (its cheat sheet), so the editor and this
page say the same thing. Each row has a small example; on the site it is
drawn as the panel draws it, with a link to try it in the editor. Every
example is compiled by the tests.

<!-- psi-builtins: reference (generated from opendmd-psi/src/builtins.rs: PSI_BLESS=1 cargo test --features docs --test builtins) -->

#### Values

| write | what it does | example |
| --- | --- | --- |
| `x, y` | where the pixel is: 0 to 1 across from the left, and down from the top | <code class="psi-ex">rgb(x, y, 0.5)</code> |
| `px, py` | the pixel’s column 0–127 from the left and row 0–31 from the top | <code class="psi-ex">gray(px % 8 == 0 or py % 8 == 0)</code> |
| `t` | seconds into the loop | <code class="psi-ex">loop 4; hsv(x + t/4, 1, 1)</code> |
| `hour, minute, second` | the time of day | <code class="psi-ex">band = floor(y&#42;3); hsv(band/3, 0.8, x &lt; pick(band, second/60, minute/60, hour/24))</code> |
| `a, b, c` | the controls, in the order the control lines declare them (0 to 1 when undeclared); any built-in name may be set as your own | <code class="psi-ex">control size 0..1 = 0.5 "Size"; gray(r &lt; size&#42;2)</code> |
| `w, h` | 128 and 32 | <code class="psi-ex">gray(px == 0 or py == 0 or px == w - 1 or py == h - 1)</code> |
| `i` | the pixel’s number, 0–4095 | <code class="psi-ex">hsv(i/4096, 1, 1)</code> |
| `cx, cy` | from the middle: −2 to 2 across, −0.5 to 0.5 down | <code class="psi-ex">rgb(abs(cx)/2, abs(cy)&#42;2, 0.3)</code> |
| `r, angle` | distance and angle (turns, clockwise from the right) from the middle | <code class="psi-ex">loop 4; hsv(angle + t/4, 1, 1 - r/2)</code> |
| `ur, ug, ub` | the colour under the formula: the GIF it is drawn over, 0 to 1 | <code class="psi-ex">rgb(ub, ug, ur)</code> |
| `ul` | how bright that is | <code class="psi-ex">pal(fire, ul)</code> |
| `under(x, y)` | the picture under at pixel x, y (the nearest, held to the panel): a colour, as #…: under(px + 2, py) shifts it, under(floor(px/4)*4, floor(py/4)*4) pixelates it; under alone is this pixel's | <code class="psi-ex">loop 2; under(floor(px/4)&#42;4 + 2 + floor(sin(t/2)&#42;2), floor(py/4)&#42;4 + 2)</code> |

#### Functions and statements

| write | what it does | example |
| --- | --- | --- |
| `sin(v) cos(v)` | a wave: one turn per 1, between −1 and 1 | <code class="psi-ex">loop 4; gray(abs(y - 0.5 - 0.4&#42;sin(x&#42;2 + t/4)) &lt; 0.06)</code> |
| `tri(v) saw(v) sqr(v)` | triangle, sawtooth and square waves | <code class="psi-ex">loop 4; k = floor(y&#42;3); v = pick(k, tri(x&#42;2 - t/4), saw(x&#42;2 - t/4), sqr(x&#42;2 - t/4)); hsv(k/3, 1, 0.5 + v/2)</code> |
| `abs floor frac sqrt` | size, whole part, fraction, square root | <code class="psi-ex">loop 4; gray(frac(x&#42;4 - t/4) &#42; sqrt(abs(cy)&#42;2))</code> |
| `min max clamp(v, lo, hi)` | limits | <code class="psi-ex">gray(clamp(sin(x&#42;2)&#42;2, 0, 1))</code> |
| `mix(a, b, k)` | from a to b as k goes 0 to 1 | <code class="psi-ex">mix(&#35;f40, &#35;04f, x)</code> |
| `step(edge, v)` | 1 past the edge, else 0 | <code class="psi-ex">loop 4; gray(step(frac(t/4), x))</code> |
| `smoothstep(e0, e1, v)` | a soft step | <code class="psi-ex">gray(smoothstep(0.3, 0.7, x))</code> |
| `len(dx, dy)` | distance | <code class="psi-ex">gray(len(px - 63.5, py - 15.5) &lt; 12)</code> |
| `atan2(dy, dx)` | angle, in turns | <code class="psi-ex">hsv(atan2(py - 15.5, px - 63.5), 1, 1)</code> |
| `noise(x, y, z)` | smooth random hills, 0 to 1 | <code class="psi-ex">loop 8; gray(noise(x&#42;8, y&#42;2, t/2))</code> |
| `hash(a, b)` | random 0 to 1 | <code class="psi-ex">gray(hash(px, py) &gt; 0.9)</code> |
| `select(c, a, b)` | a when c is true (not 0), else b; or c ? a : b | <code class="psi-ex">gray(select(x &lt; 0.5, y, 1 - y))</code> |
| `pick(k, a, b, …)` | a at k = 0, b at 1, …; else 0 | <code class="psi-ex">pal(rainbow, pick(floor(x&#42;4), 0, 0.3, 0.6, 0.9))</code> |
| `digit(n, col, row)` | the panel’s font: 1 where digit n has a pixel; col 0–4, row 0–6 from the top; add , small or , bold (chunky digits) for another font | <code class="psi-ex">gray(digit(second % 10, floor(px/4) - 14, floor(py/4)))</code> |
| `glyph('A', col, row)` | the same for a character | <code class="psi-ex">gray(glyph('A', floor(px/4) - 14, floor(py/4)))</code> |
| `bit(n, i)` | bit i of a whole number: 1 or 0 | <code class="psi-ex">gray(bit(px, floor(py/4)))</code> |
| `< <= > >= == !=` | compare: 1 or 0; and, or, not | <code class="psi-ex">rgb(x &lt; 0.5, y &gt;= 0.5, 0.2)</code> |
| `%` | remainder: v % 1 is the fraction | <code class="psi-ex">gray((px % 16 &lt; 8) != (py % 16 &lt; 8))</code> |
| `;` | a new line, on the same line | <code class="psi-ex">u = x; v = y; rgb(u, v, 0.5)</code> |
| `loop 8` | the loop’s length in seconds (first line) | <code class="psi-ex">loop 2; gray(frac(x - t/2))</code> |
| `loop forever` | goes on without repeating: t grows from when it starts (use frac, %, sin, noise of it); seed is 0 to 1, new each time it plays | <code class="psi-ex">loop forever; hsv(frac(x + t/10), 1, 1)</code> |
| `fps 100` | frames a second: 25, 50 (else) or 100, for fast motion; a light formula only | <code class="psi-ex">fps 100; loop 1; gray(abs(x - frac(t)) &lt; 0.02)</code> |
| `layer add { … }` | a layer with its own names and colour, on what is below: over, add, multiply, screen, max or min | <code class="psi-ex">layer { gray(y&#42;0.5) }; layer add { rgb(x, 0, 0) }</code> |
| `layer rate 4 { … }` | a slow bottom layer: redrawn every 4th frame, a band of rows a frame (or when the clock or a control it reads changes); the layers above draw over it every frame | <code class="psi-ex">loop 4; layer rate 4 { gray(noise(x&#42;6, y&#42;2, t)) }; layer { rgb(1, 0.5, 0, abs(x - frac(t/4)) &lt; 0.01) }</code> |
| `fn ring(cx, r) = abs(len(px - cx, py - 16) - r) < 1` | a function: a value worked out from its values, used as ring(64, 8); with { … } instead of = …, lines that draw, called on a line of their own in draw | <code class="psi-ex">loop 4<br>fn ring(cx, r) = abs(len(px - cx, py - 15.5) - r) &lt; 1<br>fn tree(x0, s) {<br>  line(x0, 31, x0, 31 - 10&#42;s, &#35;a62)<br>  circfill(x0, 31 - 10&#42;s, 3&#42;s, &#35;2a4)<br>}<br>draw { for k &lt; 4 { tree(10 + k&#42;30, 1 + k/4) } }<br>pal(ice, (ring(64 + sin(t/4)&#42;30, 8) + ring(64, 12))&#42;0.8)</code> |
| `fn p(x, y) { … }` | a function whose values are x and y: px, py, r, angle … inside follow them; its last line a colour, p(1 - x, y) draws it mirrored | <code class="psi-ex">loop 8<br>fn p(x, y) {<br>  wave = sin(x&#42;2 + t/8) + sin(y&#42;1.5 - t/4)<br>  hsv(wave/4 + t/8, 0.9, r &lt; 0.6)<br>}<br>p(abs(x - 0.5)&#42;2, y)</code> |
| `use Plasma as p` | puts in one of your formulas or an example, as a function fn p(x, y) { … } | <code class="psi-ex">fn p(x, y) = hsv(x + t/8, 1, 1)<br>p(1 - x, y)</code> |

#### Drawing

| write | what it does | example |
| --- | --- | --- |
| `draw { … }` | once a frame, before the pixels: values, loops, lines and dots into ink | <code class="psi-ex">draw { line(0, 0, 127, 31) }; pal(fire, ink)</code> |
| `for k < 12 { … }` | inside draw: k = 0, 1, … 11; two deep at most, 2048 turns a frame | <code class="psi-ex">draw { for k &lt; 12 { line(k&#42;10 + 8, 4, k&#42;10 + 8, 27, 0.2, 1) } }; gray(ink)</code> |
| `line(x0, y0, x1, y1, v0, v1)` | a line, in pixels from the top left; v0, v1 its ink at each end, 0 to 1 (else 1) | <code class="psi-ex">loop 4; draw { line(64, 16, 64 + 40&#42;cos(t/4), 16 + 15&#42;sin(t/4), 0.2, 1) }; pal(ice, ink)</code> |
| `dot(x, y, v)` | one pixel of ink v | <code class="psi-ex">draw { for k &lt; 64 { dot(hash(k, 1)&#42;128, hash(k, 2)&#42;32, hash(k, 3)) } }; gray(ink)</code> |
| `cls(#102)` | paint: the whole panel one colour (paint is drawn over the colour line; the last painted wins) | <code class="psi-ex">draw { cls(&#35;102); circfill(64, 16, 10, &#35;f80) }</code> |
| `pset(x, y, #f80)` | paint one pixel; line(x0, y0, x1, y1, c) and dot(x, y, c) paint too when their last value is a colour | <code class="psi-ex">loop 4; draw { for k &lt; 64 { pset(hash(k, 1)&#42;128, frac(hash(k, 2) + t/4)&#42;32, &#35;fff) } }</code> |
| `rect(x0, y0, x1, y1, c) rectfill(…)` | paint a box (its corners included), or a filled one | <code class="psi-ex">draw { rectfill(10, 6, 50, 25, &#35;048); rect(10, 6, 50, 25, &#35;fff) }</code> |
| `circ(x, y, r, c) circfill(…)` | paint a circle, or a filled one | <code class="psi-ex">loop 4; draw { circfill(64 + 40&#42;sin(t/4), 16, 9, &#35;f80); circ(64, 16, 14, &#35;0ff) }</code> |
| `trifill(x0, y0, x1, y1, x2, y2, c)` | paint a filled triangle | <code class="psi-ex">draw { trifill(40, 28, 64, 4, 88, 28, &#35;0c4) }</code> |
| `print("TEXT", x, y, c)` | paint text in the panel's font (add small); or a number: print(minute, x, y, c, small, 2) with at least 2 digits | <code class="psi-ex">draw { print("HELLO", 4, 4, &#35;ff0); print(minute, 90, 20, &#35;0ff, small, 2) }</code> |
| `spr(s, x, y, flip)` | paint sprite s with its top left at x, y (its key gives colours: key . 0, R #f00); flip 1 across, 2 down, 3 both | <code class="psi-ex">sprite heart {<br>  key . 0, R &#35;e24<br>  .R.R.<br>  RRRRR<br>  .RRR.<br>  ..R..<br>}<br>draw { spr(heart, 60, 12); spr(heart, 70, 12, 2) }</code> |
| `pal(fire, v), gray(v)` | colours to paint with besides #…: a palette's or a gray, v 0 to 1 | <code class="psi-ex">draw { for k &lt; 16 { rectfill(k&#42;8, 0, k&#42;8 + 7, 31, pal(fire, k/15)) } }</code> |
| `if c { … } else { … }` | inside draw: only when c is not 0 | <code class="psi-ex">draw { for k &lt; 16 { if k % 2 == 0 { rectfill(k&#42;8, 4, k&#42;8 + 6, 27, &#35;f80) } else { circfill(k&#42;8 + 3, 16, 3, &#35;0ff) } } }</code> |
| `ink` | what draw put at the pixel, 0 to 1: pal(ice, ink), #40c0ff * ink | <code class="psi-ex">draw { line(0, 16, 127, 16) }; &#35;40c0ff &#42; ink</code> |
| `ink(16)` | the same in 16 levels, a whole number: what was drawn with ink k/16 reads k, exactly (pick(ink(16), …)); 1 to 255 levels | <code class="psi-ex">draw { for k &lt; 16 { line(k&#42;8, 0, k&#42;8 + 7, 31, k/16) } }; pal(rainbow, ink(16)/15)</code> |
| `table T = 1, -2, 0.5` | numbers, 256 in all | <code class="psi-ex">table T = 0.2, 0.6, 1, 0.6; gray(T&#91;floor(x&#42;4)&#93;)</code> |
| `T[k]` | number k of table T, from 0 (0 outside it) | <code class="psi-ex">table hue = 0, 0.1, 0.3, 0.6; hsv(hue&#91;px % 4&#93;, 1, 1)</code> |
| `sprite s { … }` | a small picture in characters: first its key, key . 0, X 1 (each character and the number it reads as, up to 16), then rows up to 64 wide and 32 tall | <code class="psi-ex">sprite s {<br>  key . 0, X 1<br>  .X.<br>  XXX<br>  .X.<br>}<br>gray(s&#91;floor(px/4) - 14, floor(py/4) - 2&#93;)</code> |
| `s[u, v]` | sprite s at column u, row v from its top left: its key’s number (0 outside) | <code class="psi-ex">sprite heart {<br>  key . 0, X 1<br>  .X.X.<br>  XXXXX<br>  .XXX.<br>  ..X..<br>}<br>rgb(1, 0.2, 0.3) &#42; heart&#91;px % 6, py % 5&#93;</code> |

#### Colours

| write | what it does | example |
| --- | --- | --- |
| `hsv(hue, sat, val)` | colour: hue in turns, 0 to 1 each | <code class="psi-ex">hsv(x, 1, y)</code> |
| `rgb(r, g, b)` | red, green, blue: 0 to 1 | <code class="psi-ex">rgb(x, y, 1 - x)</code> |
| `pal(fire, v)` | a palette: fire, ice, ocean, chrono, amber, rainbow, gray | <code class="psi-ex">pal(fire, y)</code> |
| `palette sea = #024, #08c, #fff` | your own palette, up to 32 colours (3 palettes): pal(sea, 0) the first, pal(sea, 1) the last, pal(sea, 0.5) the middle one | <code class="psi-ex">palette sea = &#35;024, &#35;08c, &#35;fff; pal(sea, y)</code> |
| `palette sea = deep #024, foam #fff` | a colour may have a name: sea.foam is where pal finds it (sea.step from one colour to the next, sea.n how many) | <code class="psi-ex">palette sea = deep &#35;024, mid &#35;08c, foam &#35;fff; pal(sea, sea.mid + sea.step&#42;(y &gt; 0.5))</code> |
| `pal(fire, v) * k` | a palette colour, dimmed (or lit) by k | <code class="psi-ex">pal(fire, y) &#42; x</code> |
| `cycle(fire, v, s)` | palette cycling: the palette turned by s (one turn per 1: t/4 turns it every 4 s), at v; wraps where pal stops; s may change by row; cheap when v is the same every frame | <code class="psi-ex">loop 4; cycle(fire, x, t/4)</code> |
| `gray(v)` | white, 0 to 1 | <code class="psi-ex">gray(x)</code> |
| `#ff8800` | a colour: alone, or #f80 * v, mix(#000, #f80, v) | <code class="psi-ex">&#35;f80 &#42; y</code> |
| `rgb(r, g, b, k)` | a last value: how much it covers, 0 to 1 (what is under shows through); rgb(#f80, k) | <code class="psi-ex">layer { pal(ice, x) }; layer { rgb(1, 0.5, 0, r &lt; 0.6) }</code> |

#### Declarations

| write | what it does | example |
| --- | --- | --- |
| `name "Sunrise"` | the formula’s name (60 characters at most) | <code class="psi-ex">name "Sunrise"; hsv(0.08, 1, y)</code> |
| `about "…"` | one line about it (120 at most) | <code class="psi-ex">about "A warm glow"; pal(fire, y)</code> |
| `control speed 0..1 = 0.5 "Speed"` | a slider: its name in the formula, range, default and label (three controls at most) | <code class="psi-ex">control speed 0..2 = 1 "Speed"; loop 4; gray(frac(x - t&#42;speed/4))</code> |
| `control mode = calm, wild` | buttons: the value is the choice’s number, 0 for the first (pick(mode, …)); two words make a switch | <code class="psi-ex">control mode = calm, wild; hsv(x, 1, pick(mode, 0.5, 1))</code> |
| `look screen 40` | it can be the panel’s look: drawn over the GIF with a blend and an amount (look alone: over, 100) | <code class="psi-ex">look screen 40; pal(fire, y)</code> |
<!-- psi-builtins end -->

## Limits

<!-- psi-builtins: limits (generated from opendmd-psi/src/builtins.rs: PSI_BLESS=1 cargo test --features docs --test builtins) -->
| limit | at most |
| --- | --- |
| a formula's text, comments included | 16,384 characters |
| steps (per frame, per row, per pixel, and one per layer) | 512 |
| values held a pixel at once | 12 |
| constants and per-frame and per-row values held at once | 241 |
| nesting of brackets, operators and function calls | 48 |
| `table`, `sprite` and `palette` numbers, in all | 256 |
| loop turns a frame in `draw { }` | 2,048 |
| lines and dots a frame | 2,048 |
| line steps a frame | 16,384 |
| `layer rate N` | N from 2 to 200 |
| `name` / `about` / a control's label | 60 / 120 / 24 characters |
<!-- psi-builtins end -->

How heavy a formula is for the panel is an estimate in milliseconds a frame:
**light** is 5 ms or less, **medium** 8 ms or less, past that **heavy**. The
panel refuses a formula past 40 ms a frame (8 ms for one drawn over every
GIF as a look, and for the gallery).

## How a formula travels

- **No link carries a formula's text.** The gallery's travel by name
  (`https://opendmd.dev/f/#gallery=<name>`, and to a panel's app
  `#gallery=<name>`); one changed on the site goes to the panel's app by
  `postMessage` from the page that opened it. Sharing is submitting to the
  gallery.
- **On the panel** a playlist line or the look names a built-in Formula
  (`@formula:<name>`) or one saved on the panel (its `.psi` file, played
  from the compiled `.psb` the app writes beside it); a formula sent to
  show it at once goes compiled, on one line (`@formula?psb=…`, base64url,
  6,144 bytes at most; 2,048 for a look set that way). The panel never
  compiles.
- **In the gallery** a formula is a file of two comment lines (`--
  author: …` and `-- licence: CC0-1.0`) and then the formula, 4,096 bytes
  at most.

## Tools

The `psi` command checks and formats formulas on a computer
(`cargo install --path opendmd-psi-cli` from the source, or `make psi`):

```text
$ psi check storm-front.psi --out -
ok: 41 of 256 steps (per frame 11, per row 5, per pixel 25), 2 layers, 3 of 12 registers, 38 of 128 values
name: Storm front
never repeats (loop forever) at 50 fps; reads t
panel: ~6.8 ms a frame: medium (light ≤ 5 ms, medium ≤ 8 ms; GIF looks take ≤ 12 ms)
JIT: yes, ~5.6 ms a frame compiled (the VM's ~6.8 ms is what the limits count)
length: 559 B as written (16,384 at most), 360 B compiled (493 B as a path, @formula?psb=…: 6,144 to show it, 2,048 for an inline look)
note: line 14: `d` is worked out for every pixel each frame (~666 ns a pixel): it would be kept in the still cache but it reads `fall`, which changes from frame to frame
heaviest lines (their share of the work a frame; make these lighter first):
  line 8 (68%): cl = noise(x*3 - t*0.15, (1 - y)*2, 0)*0.7 + noise(x*7 - …
  line 14 (20%): d = hash(px, floor(py/4 + hash(px, 1)*8 - fall))
  line 9 (8%): pal(ice, cl*0.8 + (1 - y)*0.3) * (0.35 + cl*0.5)

$ psi check broken.psi
error[E202]: line 3, column 6: unknown function `sine` — did you mean `sin`?
  gray(sine(v))
       ^^^^
```

`psi check` also writes a PNG contact sheet of frames across one loop
(`--out`, `--frames`, `--a`/`--b`/`--c`, `--clock`, `--hash`, `--listing`).
It says whether the panel's JIT takes the formula (and why not), and it
prints the compiler's **notes**, each with its line: a `pick` worked out
per pixel because one arm is, a sum or product that brackets better
(`px - 64 - X` works `- X` out for every pixel; `px - (64 + X)` once a
frame), what the still cache and the column run took, the heaviest value
they couldn't take and the value that kept it out, and a draw loop run as
lanes (32 turns or more, costed for 128 a lane). `--levels` lists every
value with how often it is worked out and the chain that makes it so
(`beam  per pixel: reads `off` (line 3) ← `px``). `--same-as other.psi`
proves a rewrite: the same program, or else the same frames (three seeds
× 120 frames over a loop, or a `loop forever` formula's first 5 minutes,
the clock moving, and the sliders at 0 and at 1; `--seeds`, `--frames`);
it exits 1 at the first frame that differs.
`psi fmt FILE` prints the formula in the one canonical layout (`--write`
rewrites the file, `--check` names the ones that aren't): one statement a
line, two spaces inside `{ }`, spaces round `=`, comparisons and `+ -`, and
`*`, `/`, `%` close up where they bind tighter (`x*3 + t/8`); comments kept
where they are. It changes only the spaces between the formula's words, so
the formula is the same program, and it says so before it writes.
`psi builtins --json` and `psi errors --json` give this page's tables as
data.

## Cookbook

Short recipes, each a whole formula that works as it is: open one in the
editor and change it. Three are Formulas from the
[gallery](/formulas/), word for word.

### Scrolling text

A message is a `table` of character codes (a character in single quotes is
its code). Each pixel works out which character of the message it is in and
where in that character, and `glyph` says whether the font has a pixel
there. The message moves a whole number of characters a loop, so it wraps
without a jump.

```psi
name "Scrolling text"
about "A message scrolling across, in the panel's own font, twice its size"
loop 6
-- the message: a character in quotes is its code ('A' is 65)
table msg = 'H', 'E', 'L', 'L', 'O', ' ', 'P', 'A', 'N', 'E', 'L', ' '
-- a character is 6 cells (5 and a gap), a cell 2 pixels; the 12 go by
-- once a loop: 72 cells in 6 s
u = floor(px/2 + t*12)
k = floor(u/6) % 12
row = floor((py - 9)/2)
hsv(x/2 + t/6, 0.6, glyph(msg[k], u % 6, row))
```

### A palette cycle

The 1990s trick: the picture never moves, only its palette turns.
`cycle(P, v, s)` reads palette `P` at `v` turned by `s`; with `s = t/8`
it turns once in an 8-second loop. Since `wave` reads only the pixel's
position, the panel works it out once and keeps it, so the formula is very
light.

<!-- psi-gallery: cycled-plasma -->
```psi
name "Cycled plasma"
about "Plasma the 1992 way: the picture holds still and only its palette turns"
loop 8
fps 100
palette glow = #002, #208, #80f, #f0c, #f60, #ff0, #fff, #ff0, #f60, #f0c, #80f, #208
-- three waves that never move (worked out once, then kept)
wave = sin(x*2) + sin((1 - y)*1.5) + sin(x*1.5 + (1 - y))
-- the palette turned once round in the loop
cycle(glow, wave/6, t/8)
```

### A clock face

`hour` and `minute` are the panel's clock. The hands' angles change once a
minute, so they are worked out once a frame rather than for
every pixel. Each hand is a thin box turned by that angle; the dial is a
ring where the distance from the middle is close to 15.

<!-- psi-gallery: clock -->
```psi
name "Clock"
about "A clock face with hour and minute hands"
-- Hands on a round dial; dx, dy: pixels from the middle
dx = px - 63.5
dy = (31 - py) - 15.5
hours = (hour%12 + minute/60) / 12
mins = minute/60
hour_hand = step(max(abs(dx*cos(hours) - dy*sin(hours)), abs(dx*sin(hours) + dy*cos(hours) - 4) - 3), 1)
minute_hand = step(max(abs(dx*cos(mins) - dy*sin(mins)), abs(dx*sin(mins) + dy*cos(mins) - 6.5) - 5.5), 0.8)
dial = step(abs(dx*dx + dy*dy - 218), 22)
pal(ice, dial*0.6 + hour_hand + minute_hand)
```

### Drawing over a GIF

A formula with a `look` line can be the panel's look, drawn over whatever
GIF plays. `ur`, `ug` and `ub` are the colour of the GIF under each pixel
and `ul` its brightness, so the formula can recolour it. Here the GIF goes
sepia, as much as the control says.

<!-- psi-gallery: old-photo -->
```psi
name "Old photo"
about "The GIF in warm sepia tones"
control amount = 0.8 "How much" -- 0: the GIF as it is
look
mix(rgb(ur, ug, ub), rgb(ul*1.1 + 0.08, ul*0.92 + 0.04, ul*0.65), amount)
```

### A slow background with `rate`

Noise is one of the heavier steps. A bottom layer marked `rate 4` is redrawn
every 4th frame (a band of rows each frame), while the layers above it are
drawn every frame, so something small can move smoothly over a heavy
background.

```psi
name "Slow clouds"
about "Clouds redrawn every 4th frame, with a spark racing over them every frame"
loop 8
-- the bottom layer: heavy noise, worked out every 4th frame
layer rate 4 {
  pal(ocean, noise(x*6 + t/2, y*2, t/4)*0.9)
}
-- the layer above is drawn every frame
layer {
  rgb(1, 0.8, 0.3, len((x - frac(t/8))*4, cy) < 0.12)
}
```

### A starfield: a draw loop

`draw { }` runs once a frame, before the pixels, and puts lines and dots
into the frame's ink; the colour line reads it back as `ink`. A `for` loop
draws one streak a star: `hash(k, …)` gives each star its own direction
and start, the same every frame. The gallery's Starfield is the full
version.

```psi
name "Warp stars"
about "64 stars streaking out from the middle: lines drawn once a frame"
loop 2
fps 100
draw {
  for k < 64 {
    dir = hash(k, 1)         -- its direction, in turns
    near = frac(t/2 + hash(k, 2)) -- 0 far away, 1 here
    d = near*near*80         -- how far out it is, in pixels
    tail = d*0.8             -- where it was a moment ago
    line(64 + cos(dir)*tail, 16 + sin(dir)*tail*0.4, 64 + cos(dir)*d, 16 + sin(dir)*d*0.4, 0, near)
  }
}
pal(ice, ink)
```

### Reading slider controls

A `control` line makes a slider (or, with words, a row of buttons) in the
editor, the gallery and the panel's settings, and gives its value a name.
Up to three controls; they read as values like any other. Whole-number
speeds keep the loop seamless.

```psi
name "Sliders"
about "A bar to steer: its speed, its width and its colour are controls"
control speed 0..4 = 1 "Speed"
control width 0.05..0.5 = 0.2 "Width"
control tone = warm, cold
loop 8
-- a control is a value like any other, read once a frame
pos = frac(t*speed/8)
bar = abs(x - pos) < width/2
hsv(pick(tone, 0.06, 0.55), 0.9, bar*(0.3 + y*0.7))
```

## Error codes

Every error the compiler gives has a code, for tools and links (each
entry's address is this page with `#` and the code, as `#E203`). The
editor shows the sentence with its code, linked to its entry here. The
common errors have a word on why they happen and a before and after; the
codes marked internal are never a formula's fault.


<!-- psi-errors (generated from opendmd-psi/src/errors.rs: PSI_BLESS=1 cargo test --features docs --test diagnostics) -->
### Characters and numbers (E0xx)

<h4 id="E001">E001 · a character Psi does not know</h4>

> I don't understand `…`

Psi has numbers, names, a few operators and brackets; anything else, such as `^`, `$` or a double quote outside a declaration, is not part of it.

Take the character out. Text in quotes only goes on `name`, `about` and `control` lines; for a character's code write it in single quotes, 'A'.

Before:

```psi
gray(x ^ 2)
```

After:

```psi
gray(x * x)
```

<h4 id="E002">E002 · a bad number</h4>

> `…` is not a number

Write the number with digits and at most one point, as 12 or 0.5.

<h4 id="E003">E003 · a bad colour</h4>

> a colour is # and 3 or 6 hex digits (0-9, a-f), as #f80 or #ff8800

Write # and then 3 or 6 hex digits.

<h4 id="E004">E004 · a number out of range</h4>

> numbers must be between -32768 and 32767

Use a smaller number, or scale it down with a multiply.

<h4 id="E005">E005 · a number run into a name</h4>

> `…` is not a number or a name (to multiply, write `… * …`)

Psi never multiplies by putting things side by side, as maths on paper does.

Put a `*` between the number and the name.

Before:

```psi
gray(2x)
```

After:

```psi
gray(2 * x)
```

<h4 id="E006">E006 · a bad character</h4>

> a character goes in quotes, one at a time: 'A', '7' or ':' (there are no strings)

Write one character between single quotes, as 'A'.

### Syntax (E1xx)

<h4 id="E101">E101 · something else was expected</h4>

> expected … here, found …

Two values side by side with nothing between them: usually an operator or a comma is missing.

Write what the message says is expected at that place.

Before:

```psi
v = x y
gray(v)
```

After:

```psi
v = x * y
gray(v)
```

<h4 id="E102">E102 · a ( never closed</h4>

> this `(` is never closed — add `)`

Every `(` needs its `)`: the message points at the one left open.

Add the `)` that closes it.

Before:

```psi
v = (x + 1
gray(v / 2)
```

After:

```psi
v = (x + 1)
gray(v / 2)
```

<h4 id="E103">E103 · a block never closed</h4>

> this `layer {` is never closed — add `}`

A block (`layer`, `draw`, `fn`, `for`, `if`, `sprite`) runs to its `}`; the formula ended first.

Add the `}` that closes the block.

Before:

```psi
layer {
  gray(x)
```

After:

```psi
layer {
  gray(x)
}
```

<h4 id="E104">E104 · the colour is not last</h4>

> the colour has to be the last line (move this to the end)

The last line is the colour, and only the last: lines after it would never be used.

Move the colour line to the end of the formula (or the layer or function).

Before:

```psi
gray(x)
v = 1
```

After:

```psi
v = 1
gray(x * v)
```

<h4 id="E105">E105 · <code>loop</code> inside a layer</h4>

> `loop` is the whole formula's: put it on a line of its own, outside the layers

Move the `loop` line out of the layer.

<h4 id="E106">E106 · a value is missing</h4>

> a value is missing here

An operator, a `=` or a `,` with nothing after it.

Write the value that goes there.

Before:

```psi
v =
gray(v)
```

After:

```psi
v = x
gray(v)
```

<h4 id="E107">E107 · a [ never closed</h4>

> this `[` is never closed — add `]`

Add the `]` that closes it.

<h4 id="E108">E108 · a bare value that is not the colour</h4>

> only the last line can be a bare value (the colour) — did you mean `name = …`?

A line that is only a value, not the last line: it would be worked out and thrown away, so it is most likely a name left out.

Give the value a name with `name = …`, or make it the last line.

Before:

```psi
x + 1
gray(x)
```

After:

```psi
v = x + 1
gray(v / 2)
```

<h4 id="E109">E109 · <code>loop</code> set twice</h4>

> `loop` is already set

Keep one `loop` line.

<h4 id="E110">E110 · <code>loop</code> without a time</h4>

> `loop` takes a number of seconds, e.g. `loop 8`, or `forever` for one that goes on without repeating

Write `loop` and a number of seconds, or `loop forever`.

<h4 id="E111">E111 · <code>use</code> left in a formula</h4>

> `use` puts a formula in by its name, as a function `fn p(x, y) { … }`, when you leave the line

Finish the line (the editor puts the formula in), or write the function out.

<h4 id="E112">E112 · an empty formula</h4>

> empty: write a colour, e.g. `hsv(x + t/4, 1, 1)`

Write a colour as the last line.

<h4 id="E113">E113 · no colour at the end</h4>

> the last line should be the colour, e.g. `rgb(r, g, b)`, `hsv(h, s, v)` or `pal(fire, k)`

End the formula with a colour line.

<h4 id="E114">E114 · the wrong number of values</h4>

> `…` takes … values, but got …

Each function takes a fixed number of values (the reference's tables say how many); colours may take one more, how much of the pixel they cover.

Give the function the number of values it takes.

Before:

```psi
gray(x, 2, 3)
```

After:

```psi
gray(x, 0.5)
```

<h4 id="E115">E115 · the wrong values for a character</h4>

> `…` takes 3 values (…, a column and a row; and a font, normal, small or bold, if you like), but got …

Give it the value, a column and a row, and a font if you like.

<h4 id="E116">E116 · the wrong values for <code>pick</code></h4>

> `pick` takes a number, then the values to pick from: `pick(k, a, b, …)`

Write `pick(k, a, b, …)`: a number, then the values.

<h4 id="E117">E117 · the bold font has digits only</h4>

> the bold font has digits only: use `digit(n, x, y, bold)`

Use `digit(…)` for the bold font.

<h4 id="E118">E118 · not a font</h4>

> the last value of `…` is a font: normal (the default), small or bold, found …

Write normal, small or bold, or leave the font out.

<h4 id="E119">E119 · the wrong values for <code>rgb</code> of a colour</h4>

> `rgb` of a colour takes the colour and, if you like, how much of the pixel it covers (0 to 1)

Write `rgb(colour)` or `rgb(colour, cover)`.

<h4 id="E120">E120 · coverage on an inner colour</h4>

> how much a colour covers goes on the whole line: `rgb(mix(…), k)`, not on an `rgb(…)` inside it

Move the coverage to the outer `rgb(…)`.

### Names (E2xx)

<h4 id="E201">E201 · an unknown name</h4>

> unknown name `…` (did you mean `…`?)

A name is used before it is set, or is spelled differently where it is set.

Set the name on an earlier line, or fix the spelling.

Before:

```psi
gray(level)
```

After:

```psi
level = x
gray(level)
```

<h4 id="E202">E202 · an unknown function</h4>

> unknown function `…` (did you mean `…`?)

Only the built-in functions can be called; the message suggests the nearest one.

Fix the spelling, or use a function from the cheat sheet.

Before:

```psi
gray(sine(x))
```

After:

```psi
gray(sin(x))
```

<h4 id="E203">E203 · a name that is already a function, table, sprite, palette or control</h4>

> `…` is already the name of a … (line …) — pick another name

A value may be set again (`v = v * 2` reads the `v` before it), but functions, tables, sprites, palettes and controls keep their names: neither a value nor another of them can take one. Give it a name of its own.

Give it a name of its own. (A value may be set again, `v = v*2`, and a layer or function may set a name from outside: only functions, tables, sprites, palettes and controls keep theirs.)

Before:

```psi
table T = 0.5, 1
T = 2
gray(T[0])
```

After:

```psi
table T = 0.5, 1
k = 2
gray(T[0] * k)
```

<h4 id="E204">E204 · a word of the language as a name</h4>

> `…` is a word of the language; pick another name

The language's own words (`loop`, `layer`, `part`, `use`, `draw`, `for`, `table`, `fn`) can't be names; any built-in value or function's name can (it is then yours).

Pick another name: `loop`, `layer`, `part`, `use`, `draw`, `for`, `table` and `fn` are the language's own words (any built-in value or function's name may be yours).

Before:

```psi
layer = 1
gray(x * layer)
```

After:

```psi
level = 1
gray(x * level)
```

<h4 id="E205">E205 · a function used as a value</h4>

> `…` is a function: write `…(…)`

A function's name alone is not a value: it needs its brackets and values.

Call it with brackets: `name(…)`.

Before:

```psi
gray(sin)
```

After:

```psi
gray(sin(x))
```

<h4 id="E206">E206 · a palette used as a value</h4>

> `…` is a palette: use it as `pal(…, k)` on the last line

Use it as `pal(name, k)` on the last line.

<h4 id="E207">E207 · a font used as a value</h4>

> `…` is a font: use it as `digit(n, x, y, …)`

Use it as the font of `digit(…)`.

<h4 id="E208">E208 · a name that is a palette's</h4>

> `…` is a palette's name; pick another name

Pick another name.

<h4 id="E209">E209 · a value called like a function</h4>

> `…` is a value, not a function

Leave the brackets off.

<h4 id="E210">E210 · a keyword inside a line</h4>

> `…` can only start a line

Move it to the start of a line.

<h4 id="E211">E211 · an unknown palette</h4>

> unknown palette `…` (there are fire, ocean, ice, chrono, amber, rainbow and gray…)

The palettes are fire, ice, ocean, chrono, amber, rainbow and gray, and the ones the formula makes with `palette`.

Use one of the listed palettes, or write your own with `palette name = #…, #…`.

Before:

```psi
pal(lava, x)
```

After:

```psi
pal(fire, x)
```

<h4 id="E212">E212 · a palette expected</h4>

> the first value of `…` is a palette: fire, ocean, ice, chrono, amber, rainbow or gray

Start with a palette name.

<h4 id="E213">E213 · an unknown blend</h4>

> unknown blend `…` (a layer goes over, add, multiply, screen, max or min)

Use one of the listed blends.

<h4 id="E214">E214 · <code>ink(n)</code> without a whole number</h4>

> `ink(n)` takes a whole number from 1 to 255: `ink(16)` is the canvas in 16 levels

Write the number of levels, as `ink(16)`, and draw with inks `k/16`.

### Placement and levels (E3xx)

<h4 id="E301">E301 · a <code>frame { }</code> block</h4>

> there are no `frame { }` blocks: a value is worked out once a frame by itself when it reads only the time, the clock and the controls; write its lines without the block

There are no `frame { }` blocks: a value that reads only the time, the clock and the controls is worked out once a frame by itself.

Take the lines out of the block, as they are.

Before:

```psi
frame {
  k = t / 8
}
gray(x * k)
```

After:

```psi
k = t / 8
gray(x * k)
```

<h4 id="E302">E302 · a colour used as a value</h4>

> `#…` is a colour, not a number: use it on the last line, as `#f80 * k` or `mix(#000, #f80, k)`

A colour such as `#f80` is not a number: it goes in the colour line, where it can be dimmed or mixed.

Use the colour in the colour line, not in a number's place.

Before:

```psi
v = #f80
gray(v)
```

After:

```psi
v = x
#f80 * v
```

<h4 id="E303">E303 · a colour outside a layer</h4>

> with layers, every colour goes inside one: put this line in `layer { … }`

Once a formula has a layer, each colour belongs to one: a colour line outside them would have nothing to go on.

Put the colour line inside a layer.

Before:

```psi
layer { gray(x) }
gray(y)
```

After:

```psi
layer { gray(x) }
layer add { gray(y) }
```

<h4 id="E304">E304 · a colour function in a sum of colours</h4>

> `…(…)` is a whole colour: it can't be added to others (only `#…` and `rgb(r, g, b)` can)

Use `#…` or `rgb(…)` in the sum, or give the function its own line.

<h4 id="E305">E305 · a colour that can't be summed</h4>

> `…(…)` is a whole colour: it can't go into a sum, only be scaled, `… * k`

Scale it on its own line, or use `#…` or `rgb(…)` colours in the sum.

<h4 id="E306">E306 · a colour function that is not the whole line</h4>

> `…(…)` is a whole colour: it can only be the last line (of the formula or a layer)

Make it the whole last line.

<h4 id="E307">E307 · a colour inside <code>rgb(r, g, b)</code></h4>

> `rgb(r, g, b)` takes three numbers, not colours: write a colour alone, as `#f80 * k`

Give `rgb(r, g, b)` three numbers, or write the colour on its own.

### Limits (E4xx)

<h4 id="E401">E401 · a formula that is too long to read</h4>

> too long: a formula can be … characters at most

Shorten the formula.

<h4 id="E402">E402 · too many steps</h4>

> this formula is too long for the panel: … steps (per frame …, draw …, per row …, per pixel …), the limit is … (or: … — simplify it)

The panel works a formula out for 4,096 pixels at every frame, so the steps a formula takes are limited. Values that change less often (once a row, once a frame) cost much less: move what you can out of the pixel.

Simplify the formula: fewer or simpler steps.

<h4 id="E403">E403 · too many live values</h4>

> this formula holds more than … values at once (the panel's registers): use each sooner

Set fewer names, or use each value sooner.

<h4 id="E404">E404 · too many numbers</h4>

> this formula needs … numbers and once-a-frame values, more than the … it can hold (steps: per frame …, draw …, per row …, per pixel …): use fewer

Use fewer different numbers and steps: the section with the most steps is the place to look.

<h4 id="E405">E405 · nested too deeply</h4>

> this is nested too deeply — split it into named steps

Split it into named steps, `name = …`.

<h4 id="E406">E406 · a loop time out of range</h4>

> the loop must be between 0.1 and 3600 seconds

Use a loop between 0.1 and 3600 seconds.

<h4 id="E407">E407 · a layout fault <span class="badge">internal</span></h4>

> this formula could not be laid out (a bug in Psi) — please report it

**Internal:** a bug in Psi: please report it, with the formula. It is never the formula's fault.

Report it with the formula.

<h4 id="E408">E408 · not enough memory</h4>

> not enough memory for this Formula right now

The panel didn't have the memory to compile this Formula just then (a GIF, a look or a transition was holding it): not the formula's fault. It plays when it is next due; a shorter formula needs less.

Try again, or make the Formula smaller.

### Layers (E5xx)

<h4 id="E509">E509 · a layer inside a layer</h4>

> a layer can't go inside another — close this one with `}` first

Close the first layer before starting another.

<h4 id="E510">E510 · a layer without a colour</h4>

> this layer ends without a colour: its last line is `rgb(…)`, `hsv(…)`, `pal(…)` or `gray(…)`

End the layer with a colour.

### Version (E6xx)

<h4 id="E601">E601 · a newer Psi</h4>

> this formula needs a newer OpenDMD: it is written for Psi …, this one has Psi …

Update OpenDMD, or write the formula for an older Psi.

<h4 id="E603">E603 · a bad <code>psi</code> version</h4>

> `psi` takes the language's version, a whole number: `psi 1`

Write `psi 1`.

<h4 id="E604">E604 · <code>psi</code> not on the first line</h4>

> `psi 1` goes on the formula's first line

Move `psi 1` to the first line.

### The draw stage, paint and functions (E7xx)

<h4 id="E701">E701 · <code>draw { }</code> or a table inside a layer</h4>

> `draw { }` goes at the top of the formula, not inside a layer

Move it out of the layer, to the top of the formula.

<h4 id="E702">E702 · a second <code>draw { }</code> block</h4>

> there can be one `draw { }` block (functions that draw are called inside it)

Merge the `draw { }` blocks into one.

<h4 id="E703">E703 · a bad <code>for</code> line</h4>

> `for` takes a name and a count, e.g. `for k < 12 { … }` (a whole number from 1 to 1024)

Write `for k < 12 { … }` with a whole count from 1 to 1024.

<h4 id="E704">E704 · too many loop turns</h4>

> this formula draws too much a frame (… loop turns in all; the limit is …) — fewer or shorter loops

Use fewer or shorter loops.

<h4 id="E705">E705 · <code>for</code> nested too deep</h4>

> `for` loops go two deep at most

Use at most two `for` loops, one inside the other.

<h4 id="E706">E706 · drawing outside <code>draw { }</code></h4>

> `…(…)` draws: it goes on a line of its own inside `draw { }`

`line` and `dot` draw into the frame's ink once a frame, so they live in `draw { }`; the pixels read what they drew as `ink`.

Put the line inside `draw { }`.

Before:

```psi
line(0, 0, 127, 31)
gray(1)
```

After:

```psi
draw {
  line(0, 0, 127, 31)
}
gray(ink)
```

<h4 id="E707">E707 · a table that is not numbers</h4>

> a table holds numbers only, up to 256 in all, as in `table T = 1, -2, 0.5`

Write only numbers in the table.

<h4 id="E708">E708 · a per-pixel value in <code>draw { }</code></h4>

> `draw { }` runs once a frame, before any pixel, but `…` (line …) is per pixel: it reads `…` (or: but this uses `…` (…))

`draw { }` runs once a frame, before the pixels: there is no pixel position there. Work in pixels, with numbers and the time.

Use only values that are the same everywhere inside `draw { }`.

Before:

```psi
draw { dot(x, 1) }
gray(ink)
```

After:

```psi
draw { dot(t * 16, 1) }
gray(ink)
```

<h4 id="E709">E709 · a loop counter set twice</h4>

> `…` is a loop counter: it can't be set (or: `…` is already a loop counter here)

Use another name.

<h4 id="E710">E710 · a table read wrongly</h4>

> `…[…]` reads a table, and `…` is not one

Read a table with `NAME[k]`.

<h4 id="E711">E711 · a table without a name</h4>

> a table needs a name, as in `table T = 1, 2, 3`

Name the table.

<h4 id="E712">E712 · the wrong values for <code>line</code> or <code>dot</code></h4>

> `line` takes x0, y0, x1, y1 (pixels from the top left) and 1 or 2 inks: … given (`dot`: x, y and an ink)

Give `line` four values and `dot` two.

<h4 id="E713">E713 · a line that does not belong in <code>draw { }</code></h4>

> inside draw { } go `name = value`, `for k < N { … }`, `if c { … }` and drawing: `line`, `dot`, `pset`, `rect`, `rectfill`, `circ`, `circfill`, `trifill`, `cls`, `spr`, `print`

Use only those lines inside `draw { }`; the colour line goes after it.

<h4 id="E714">E714 · <code>ink</code> without <code>draw { }</code></h4>

> `ink` needs a `draw { }` block to read from

Write a `draw { }` block that draws what `ink` reads.

<h4 id="E715">E715 · <code>for</code> outside <code>draw { }</code></h4>

> `for` goes inside `draw { }`: `draw { for k < 12 { … } }`

Put the loop inside `draw { }`.

<h4 id="E716">E716 · a table without <code>[k]</code></h4>

> `…` is a table: read one of its numbers, `…[k]` (k from 0)

Read a number of the table with `NAME[k]`.

<h4 id="E720">E720 · too many colours to paint with</h4>

> a formula paints with … colours at most (each of a palette's 32 counts)

Paint with fewer colours: a palette used in `pal(P, v)` takes 32 of the 127.

<h4 id="E721">E721 · not a colour to paint with</h4>

> a colour to paint with is `#…` (as #f80), `pal(palette, v)` or `gray(v)`

End the call with a colour: `#f80`, `pal(fire, v)` or `gray(v)` (for brightness into ink, `line` and `dot` take a number).

<h4 id="E722">E722 · <code>spr</code> of something that is not a sprite</h4>

> `spr` draws a sprite: `spr(name, x, y)`, its name a `sprite name { … }`'s

Name a sprite block, as `spr(ship, x, y)`.

<h4 id="E723">E723 · <code>spr</code> without its place</h4>

> … takes a sprite and its place

Give the sprite's place: `spr(ship, x, y)`, and maybe a flip (1 across, 2 down, 3 both).

<h4 id="E724">E724 · a text too long to print</h4>

> `print` writes 32 characters at most

Split the text into two `print`s.

<h4 id="E725">E725 · no room for a text</h4>

> this text doesn't fit: tables, sprites and texts hold 256 numbers in all

Use shorter texts, or fewer tables and sprites.

<h4 id="E726">E726 · a font <code>print</code> doesn't have</h4>

> `print`'s font is `normal`, `small` or (for a value) `bold`

Write `normal`, `small` or, for a number, `bold`.

<h4 id="E727">E727 · something after a paint call's values</h4>

> …: expected `)` here

Close the call after its colour (and, for `print`, its font and digits).

<h4 id="E740">E740 · a function written wrongly</h4>

> a function is `fn name(a, b) = value`, or `fn name(a, b) { … }` for lines that draw

Write `fn name(a, b) = value`, or `fn name(a, b) { … }` around lines that draw.

<h4 id="E742">E742 · a function's value that is not a name</h4>

> a function's values are names: `fn name(a, b) = …`

Name each value the function takes: `fn ring(cx, r) = …`.

<h4 id="E743">E743 · a function with no body</h4>

> after a function's values: `= value`, or `{ … }` for lines that draw

Put `= value` after the brackets, or `{ … }` around lines that draw.

<h4 id="E744">E744 · an empty function</h4>

> function `…` has nothing in it

Write what the function works out, or the lines it draws.

<h4 id="E745">E745 · a function calling itself</h4>

> `…` calls itself: a function can't (there are no loops of calls)

Use a `for` loop in draw, or write the steps out.

<h4 id="E746">E746 · a drawing function used as a value</h4>

> `…` draws: call it on a line of its own inside `draw { }`

Call it on its own line in `draw { }`: `palm(30, 31, 1)`.

<h4 id="E747">E747 · more than one value on a function's last line</h4>

> function `…`'s last line is one value: `fn …(…) = value`

Write one value after `=`, or use `{ … }` for lines that draw.

<h4 id="E748">E748 · a value function called as lines</h4>

> `…` is a value, not lines that draw: use it in one, `v = …(…)`

Use it in a value, `v = name(…)`, or in a call's values.

<h4 id="E749">E749 · too many calls</h4>

> this formula calls its functions too many times over — simplify it

Call a function fewer times, or set its value once as a name and use the name.

<h4 id="E750">E750 · a part</h4>

> there are no parts: write a function, `fn p(x, y) { … }`, and call it, `p(x*2, y)`

Write `fn p(x, y) { … }` with the part's lines; draw it with `p(x, y)` as the last line.

<h4 id="E751">E751 · a value before a function's last line</h4>

> only a function's last line is its value: set a name here (`v = …`), or move it last

Set a name on each line but the last: `v = …`.

<h4 id="E752">E752 · a whole-colour function inside a value</h4>

> `…(…)` is a whole colour: use it alone on the colour line (only `#…` and `rgb(r, g, b)` go into sums)

Use it alone as the last line, or make its last line `#…` or `rgb(…)` so it can be mixed.

### Frame rate (E8xx)

<h4 id="E801">E801 · a bad <code>fps</code></h4>

> `fps` is 25, 50 or 100 frames a second

The panel shows 25, 50 or 100 frames a second; nothing in between.

Write `fps 25`, `fps 50` or `fps 100`.

Before:

```psi
fps 60
gray(x)
```

After:

```psi
fps 50
gray(x)
```

<h4 id="E802">E802 · <code>fps</code> inside a layer</h4>

> `fps` is the whole formula's: put it on a line of its own, outside the layers

Move the `fps` line out of the layer.

<h4 id="E803">E803 · <code>fps</code> set twice</h4>

> `fps` is already set

Keep one `fps` line.

### Growing values, sprites, palettes, slow layers and declarations (E9xx)

<h4 id="E901">E901 · a value that grows without end</h4>

> `t` grows without end in `loop forever`, and this can't take it: use frac(t\*k), t % n, sin(t\*k), noise(t\*k, y, z), a comparison, min or max

In a `loop forever` formula `t` grows without end, and most arithmetic on it would lose its precision or overflow. Wrap it first: `frac`, `%`, `sin` and `noise` keep it in range.

Use frac(t\*k), t % n, sin(t\*k) or similar in its place.

Before:

```psi
loop forever
gray(t*x)
```

After:

```psi
loop forever
gray(frac(t/4 + x))
```

<h4 id="E902">E902 · too many noises of growing values</h4>

> noise of values that grow without end: four different ones at most — share one, as noise(u + k, …)

Share one noise between the places that need it.

<h4 id="E903">E903 · <code>seed</code> outside <code>loop forever</code></h4>

> `seed` is for formulas that go on without repeating (`loop forever`): a new one each time it plays

Add `loop forever`, or leave `seed` out.

<h4 id="E904">E904 · too many growing values</h4>

> too many values that grow without end (more than …): share one, as frac(u\*k) of one `u`

Use fewer values that grow without end.

<h4 id="E905">E905 · a colour fed a growing value</h4>

> a colour can't take a value that grows without end and changes across the panel: use frac(…) of it, or min(t/k, 1) (or: …, as `v` (line …) does: use frac(v) or min(v/k, 1))

A colour has to be worked out from values that stay in range; `t` in a `loop forever` formula does not.

Wrap it in frac(…), or limit it with min(t/k, 1).

Before:

```psi
loop forever
v = t + x
gray(v)
```

After:

```psi
loop forever
v = frac(t/8 + x)
gray(v)
```

<h4 id="E906">E906 · drawing at a growing value</h4>

> a value that grows without end can't be drawn at: use frac(t\*k)\*128, t % n or min(t, k)

Use frac(t\*k)\*128, t % n or min(t, k) as the place.

<h4 id="E907">E907 · a growing value with a per-pixel part</h4>

> a growing value plus a per-pixel or per-row part only goes into frac, the waves, noise, % or a multiply: as frac(t\*k) + x or sin(t\*k + x)

Add the pixel's part inside frac, a wave or noise: frac(t\*k) + x, sin(t\*k + x).

<h4 id="E910">E910 · a sprite's key too long</h4>

> a sprite's key has 16 characters at most

Use at most 16 characters in the key.

<h4 id="E911">E911 · a sprite inside a layer</h4>

> a sprite goes at the top of the formula, not inside a layer

Move the sprite out of the layer.

<h4 id="E912">E912 · a sprite without its key</h4>

> a sprite starts with its key: `key . 0, R 1, D 2` (each character, then the number it reads as)

Start the sprite with a `key` line.

<h4 id="E913">E913 · a bad key entry</h4>

> a key entry is a character (not ; { } , - / or a space) and its number, `R 1`, after a comma

Write each entry as a character, a space and its number, with commas between.

<h4 id="E914">E914 · a character not in the key</h4>

> `…` is not in the sprite's key

Add the character to the key, or change the row.

<h4 id="E915">E915 · sprite rows of different widths</h4>

> a sprite's rows are as wide as its first (… characters), at most 64

Make every row as wide as the first, at most 64.

<h4 id="E916">E916 · a sprite with no rows or too many</h4>

> a sprite has 1 to 32 rows of characters, after its key

Use 1 to 32 rows.

<h4 id="E917">E917 · a sprite without <code>[u, v]</code></h4>

> `…` is a sprite: read its pixels, `…[u, v]`

Read the sprite at a column and a row, `NAME[u, v]`.

<h4 id="E918">E918 · a sprite read wrongly</h4>

> a sprite is read at a column and a row: `…[u, v]`

Read a sprite with `NAME[u, v]`.

<h4 id="E919">E919 · a sprite never closed</h4>

> this `sprite {` is never closed — add `}` on a line of its own

Add a `}` on a line of its own.

<h4 id="E920">E920 · a character twice in the key</h4>

> `…` is in the key twice

Keep one entry for each character.

<h4 id="E921">E921 · a palette inside a layer</h4>

> a palette goes at the top of the formula, not inside a layer

Move the palette out of the layer.

<h4 id="E922">E922 · a palette of the wrong size</h4>

> a palette is 1 to 32 colours: `palette sea = #003, #06c, #fff`

Write 1 to 32 colours, each as #f80.

<h4 id="E923">E923 · too many palettes</h4>

> a formula can have … palettes of its own at most

Use fewer palettes (a palette in a tween's half takes half of one).

<h4 id="E924">E924 · too many numbers in tables and sprites</h4>

> tables and sprites hold 256 numbers in all — this would make …

Make the tables and sprites smaller.

<h4 id="E925">E925 · a palette's colour named twice</h4>

> `…` already names a colour of `…`: give this one another name

Give each named colour of a palette its own name.

<h4 id="E926">E926 · a palette's colour named <code>n</code> or <code>step</code></h4>

> `n` and `step` are every palette's own (`….n`, `….step`): name this colour another way

Pick another name for the colour: `P.n` is the palette's count and `P.step` the step between colours.

<h4 id="E927">E927 · an unknown colour of a palette</h4>

> palette `…` has no colour named `…`: name it in its line, as `… #f80`

Name the colour on the palette's line (`palette road = black #000, kerb #806`), or use another of its names.

<h4 id="E931">E931 · <code>cycle</code> turned by a value that changes along a row</h4>

> `cycle` turns a whole row at once (by t, a frame value or py), but `…` (line …) is per pixel: it reads `…`

`cycle` turns a whole row's palette at once, so how far it turns can change with the time or the row, not along the row.

Turn it by t, a value of t, or py.

Before:

```psi
cycle(fire, x, x)
```

After:

```psi
loop 4
cycle(fire, x, t/4 + y)
```

<h4 id="E932">E932 · <code>cycle</code> or <code>tween</code> of gray</h4>

> `cycle` turns a palette of colours, and gray is not one: write your own, `palette g = #000, #fff`

Write your own palette.

<h4 id="E933">E933 · too many palettes with <code>cycle</code></h4>

> a formula holds … palettes that `pal` and `cycle` read: its own (… here) and one more for each turn `cycle` gives (… here)

Read fewer palettes with `pal` and `cycle` (tweens may read more), or turn fewer by different values.

<h4 id="E934">E934 · <code>cycle</code> without its turn</h4>

> expected `,` and the turn (as in `cycle(fire, v, t/4)`) here, found …

Write the turn after the picture: `cycle(palette, v, turn)`.

<h4 id="E935">E935 · the wrong values for <code>pal</code> or <code>cycle</code></h4>

> `…` takes a palette (and how much of the pixel it covers, 0 to 1, if you like); found …

Give it a palette and a value (`cycle` also takes the picture and the turn).

<h4 id="E936">E936 · a tween mixed by a value that changes across the panel</h4>

> `tween` mixes a palette once a frame (by t or a frame value), but `…` (line …) is per pixel: it reads `…`

Mix it by t, a value of t, a slider or another once-a-frame value.

<h4 id="E937">E937 · a palette's half of the wrong palettes</h4>

> a palette's half is a tween of palettes of your own with 16 colours at most: `…` is not one

In a half, tween palettes of your own of 16 colours or fewer (tweens of them inside are fine).

<h4 id="E938">E938 · a tween painted with</h4>

> `…` is a tween, worked out every frame: paint takes a palette of colours (`palette p = #000, #fff`)

Paint with a palette of colours, or with `pal(P, v)` of one of the tween's palettes.

<h4 id="E939">E939 · tweens nested too deeply</h4>

> the tweens need more than … palettes worked out at once: nest them less deeply, or tween fewer palettes that only tweens read

Nest tweens less deeply (a tween as the first palette costs nothing), or read the palettes they make with `pal`.

<h4 id="E941">E941 · <code>rate</code> above the bottom</h4>

> `rate` goes on the bottom layers only: the first layer, or the first few one after another

Put `rate` on the bottom layers only.

<h4 id="E942">E942 · a bad <code>rate</code></h4>

> `rate` takes a whole number of frames, 2 to …: `layer rate 4 { … }` is redrawn every 4th frame

Write a whole number from 2 to the limit.

<h4 id="E943">E943 · different <code>rate</code>s</h4>

> the slow layers share one `rate`: give each the same number

Give the slow layers the same `rate`.

<h4 id="E944">E944 · a <code>rate</code> layer reading the picture</h4>

> a `rate` layer is drawn on its own: it can't read `ink`, `ur`, `ug`, `ub` or `ul`

Do not read `ink`, `ur`, `ug`, `ub` or `ul` in a `rate` formula.

<h4 id="E946">E946 · <code>rate</code> before the blend</h4>

> write the blend first: `layer add rate 4 { … }`

Write the blend, then `rate`.

<h4 id="E947">E947 · only <code>rate</code> layers</h4>

> a `rate` layer is the slow one under a layer that moves: add a layer above it, or leave `rate` out

Add a layer above, or leave `rate` out.

<h4 id="E948">E948 · a bottom <code>rate</code> layer that does not cover</h4>

> the bottom `rate` layer must cover the whole panel: `layer rate N { … }`, no blend, no coverage

Leave the blend and the coverage off the bottom `rate` layer.

<h4 id="E951">E951 · a declaration below other lines</h4>

> `…` goes at the top of the formula, before its other lines

Declarations (`name`, `about`, `control`, `look`) say what the formula is, so they come first, before anything it works out.

Move it to the top.

Before:

```psi
loop 4
name "Late"
gray(x)
```

After:

```psi
name "Late"
loop 4
gray(x)
```

<h4 id="E952">E952 · a declaration twice</h4>

> `…` is already set on line …

Keep one line.

<h4 id="E953">E953 · text in quotes not closed</h4>

> text in quotes ends with `"` on the same line

Close the quotes on the same line.

<h4 id="E954">E954 · a character that can't be in quotes</h4>

> text in quotes can't have `<`, `>`, `\` or a control character (a tab too) in it

Take it out.

<h4 id="E955">E955 · text too long</h4>

> too long: this text can be … characters at most

Shorten the text.

<h4 id="E956">E956 · text missing</h4>

> this takes its text in quotes, e.g. `name "Sunrise"`

Write the text in quotes.

<h4 id="E957">E957 · a bad <code>control</code> line</h4>

> a control is `control NAME lo..hi = default "Label"` or `control NAME = word, word, …`

A control is a slider (`control NAME lo..hi = default "Label"`) or a row of buttons (`control NAME = word, word`).

Write it as shown.

Before:

```psi
control k 1 = 2
gray(k)
```

After:

```psi
control k 0..2 = 1 "Level"
gray(k / 2)
```

<h4 id="E958">E958 · too many controls</h4>

> a formula has three controls at most (a, b and c)

Keep three controls at most.

<h4 id="E959">E959 · a range that does not go up</h4>

> a range goes from the lower number to the higher, e.g. `0..1`

Write the lower number first.

<h4 id="E960">E960 · a default outside the range</h4>

> the default is outside the control's range

Choose a default inside the range.

<h4 id="E961">E961 · a choice of the wrong size</h4>

> a choice has 2 to 6 words

Give 2 to 6 words.

<h4 id="E962">E962 · a word twice in a choice</h4>

> `…` is a choice already

Use each word once.

<h4 id="E963">E963 · a control out of its slot</h4>

> `…` is the … control's letter: give this one another name (controls take a, b, c in order)

Name it something else, or put it in its slot.

<h4 id="E964">E964 · a bad <code>look</code> line</h4>

> `look` takes a blend (over, screen, add, multiply, max or min) and an amount from 0 to 100

Write `look`, a blend and an amount from 0 to 100.

<h4 id="E965">E965 · a blend on a formula with layers</h4>

> a formula with layers goes over the GIF as a look: write `look` or `look over N`

Write `look` or `look over N`.

<h4 id="E966">E966 · a choice word that is not a name</h4>

> a choice is words (names) between commas: `control NAME = word, word, …`

Write names between commas.

<h4 id="E967">E967 · more than a declaration takes</h4>

> expected the end of the `…` line, found … (one declaration a line)

Put one declaration on a line.

<h4 id="E968">E968 · an unknown blend in a look</h4>

> a look's blend is one of over, screen, add, multiply, max, min

Use one of the listed blends.

<h4 id="E969">E969 · text that hides or reorders</h4>

> text in quotes can't have characters that hide or reorder text (zero-width, direction marks, line separators)

Take them out.

<!-- psi-errors end -->
