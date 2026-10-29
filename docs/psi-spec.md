# Psi language reference

**Psi** is the language OpenDMD's **Formulas** are written in: a few lines
of arithmetic, worked out for every pixel of the 128 × 32 panel, every
frame, ending in a colour. This page is the reference: what the language
is, every name it knows, its limits and its error codes. To learn it, start
with [the guide's Formulas page](guide/formulas.md); to try it, open the
[demo](/demo/)'s Formulas.

The tables of names, the limits and the error codes on this page are
generated from the compiler's own tables, and the tests fail when they
drift, so what this page says is what the panel does.

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
          | "frame" "{" { assign sep } "}"    -- values once a frame (no x, y, px, py, i, ur…)
          | NAME "=" expr                     -- each name set once in its scope
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


The declarations (`decl`) come first, before every other line, each once
(`control` up to three times). The colour line is the last line of the
formula, of a part and of a layer; with layers, every colour is inside one.

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
  frame (or in `frame { }`), values of `y`, `py`, `cy` and `ty` once a row,
  the rest once a pixel. This changes only the cost, never the result.
- **Layers** (`layer [blend] [rate N] { … }`) are drawn bottom to top, each
  with its own names and colour, onto what is below with its blend (`over`,
  `add`, `multiply`, `screen`, `max`, `min`); a colour's last value says how
  much of the pixel it covers.
- **Parts** (`part p { … }`) are formulas inside a formula: `p` is its
  colour, `p.v` a value it sets, `p(x, y)` its colour drawn at another place.
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
| values about the middle | `cx`, `cy`, `ty`, `r`, `angle` |
| constants | `w`, `h` |
| functions (how many values) | `sin` (1), `cos` (1), `tri` (1), `saw` (1), `sqr` (1), `abs` (1), `floor` (1), `frac` (1), `fract` (1), `sqrt` (1), `min` (2), `max` (2), `step` (2), `hash` (2), `len` (2), `atan2` (2), `clamp` (3), `mix` (3), `smoothstep` (3), `noise` (3), `select` (3), `bit` (2), `digit(…)` (3 values and a font), `glyph(…)` (3 values and a font), `pick(k, a, b, …)` |
| colours (how many values, before the coverage) | `rgb` (3), `hsv` (3), `pal` (2), `gray` (1), `cycle` (3) |
| palettes | `fire`, `ice`, `ocean`, `chrono`, `amber`, `rainbow`, `gray` |
| fonts | `normal`, `small`, `bold` |
| blends | `over`, `add`, `multiply`, `screen`, `max`, `min` |
| keywords | `loop`, `frame`, `layer`, `part`, `use`, `draw`, `for`, `table` |
| drawing | `line`, `dot` |
| declarations | `psi`, `name`, `about`, `control`, `look` |
| words that are keywords in one place only | `fps`, `forever`, `sprite`, `palette`, `key`, `rate` |

A formula may set a name added after people had saved formulas, and then it is the formula's own: `cx`, `cy`, `ty`, `r`, `angle`, `bit`, `digit`, `glyph`, `pick`, `cycle` and `seed`, `ink`, `line`, `dot`, `forever`.
<!-- psi-builtins end -->

## What each does

These are the editor's own words (its cheat sheet), so the editor and this
page say the same thing.

<!-- psi-builtins: reference (generated from opendmd-psi/src/builtins.rs: PSI_BLESS=1 cargo test --features docs --test builtins) -->

#### Values

| write | what it does |
| --- | --- |
| `x, y` | where the pixel is: 0 to 1 across, and up from the bottom |
| `px, py` | the pixel’s column 0–127 and row 0–31 from the bottom |
| `t` | seconds into the loop |
| `hour, minute, second` | the time of day |
| `a, b, c` | the controls, in the order the control lines declare them (0 to 1 when undeclared) |
| `w, h` | 128 and 32 |
| `i` | the pixel’s number, 0–4095 |
| `cx, cy` | from the middle: −2 to 2 across, −0.5 to 0.5 up |
| `r, angle` | distance and angle (turns) from the middle |
| `ty` | row 0–31 from the top |
| `ur, ug, ub` | the colour under the formula: the GIF it is drawn over, 0 to 1 |
| `ul` | how bright that is |

#### Functions and statements

| write | what it does |
| --- | --- |
| `sin(v) cos(v)` | a wave: one turn per 1, between −1 and 1 |
| `tri(v) saw(v) sqr(v)` | triangle, sawtooth and square waves |
| `abs floor frac sqrt` | size, whole part, fraction, square root |
| `min max clamp(v, lo, hi)` | limits |
| `mix(a, b, k)` | from a to b as k goes 0 to 1 |
| `step(edge, v)` | 1 past the edge, else 0 |
| `smoothstep(e0, e1, v)` | a soft step |
| `len(dx, dy)` | distance |
| `atan2(dy, dx)` | angle, in turns |
| `noise(x, y, z)` | smooth random hills, 0 to 1 |
| `hash(a, b)` | random 0 to 1 |
| `select(c, a, b)` | a when c is true (not 0), else b; or c ? a : b |
| `pick(k, a, b, …)` | a at k = 0, b at 1, …; else 0 |
| `digit(n, col, row)` | the panel’s font: 1 where digit n has a pixel; col 0–4, row 0–6 from the top; add , small or , bold (chunky digits) for another font |
| `glyph('A', col, row)` | the same for a character |
| `bit(n, i)` | bit i of a whole number: 1 or 0 |
| `< <= > >= == !=` | compare: 1 or 0; and, or, not |
| `%` | remainder: v % 1 is the fraction |
| `;` | a new line, on the same line |
| `loop 8` | the loop’s length in seconds (first line) |
| `loop forever` | goes on without repeating: t grows from when it starts (use frac, %, sin, noise of it); seed is 0 to 1, new each time it plays |
| `fps 100` | frames a second: 25, 50 (else) or 100, for fast motion; a light formula only |
| `frame { … }` | values worked out once a frame |
| `layer add { … }` | a layer with its own names and colour, on what is below: over, add, multiply, screen, max or min |
| `layer rate 4 { … }` | a slow bottom layer: redrawn every 4th frame, a band of rows a frame (or when the clock or a control it reads changes); the layers above draw over it every frame |
| `use Plasma as p` | puts in one of your formulas or an example, as part p { … } |
| `p.v` | a value part p sets |
| `p(x, y, k)` | part p as a colour, drawn at another place; k: how much it covers |

#### Drawing

| write | what it does |
| --- | --- |
| `draw { … }` | once a frame, before the pixels: values, loops, lines and dots into ink |
| `for k < 12 { … }` | inside draw: k = 0, 1, … 11; two deep at most, 2048 turns a frame |
| `line(x0, y0, x1, y1, v0, v1)` | a line, in pixels up from the bottom left; v0, v1 its ink at each end, 0 to 1 (else 1) |
| `dot(x, y, v)` | one pixel of ink v |
| `ink` | what draw put at the pixel, 0 to 1: pal(ice, ink), #40c0ff * ink |
| `table T = 1, -2, 0.5` | numbers, 256 in all |
| `T[k]` | number k of table T, from 0 (0 outside it) |
| `sprite s { … }` | a small picture in characters: first its key, key . 0, X 1 (each character and the number it reads as, up to 16), then rows up to 64 wide and 32 tall |
| `s[u, v]` | sprite s at column u, row v from its top left: its key’s number (0 outside) |

#### Colours

| write | what it does |
| --- | --- |
| `hsv(hue, sat, val)` | colour: hue in turns, 0 to 1 each |
| `rgb(r, g, b)` | red, green, blue: 0 to 1 |
| `pal(fire, v)` | a palette: fire, ice, ocean, chrono, amber, rainbow, gray |
| `palette sea = #024, #08c, #fff` | your own palette, up to 32 colours (3 palettes): pal(sea, 0) the first, pal(sea, 1) the last, pal(sea, 0.5) the middle one |
| `pal(fire, v) * k` | a palette colour, dimmed (or lit) by k |
| `cycle(fire, v, s)` | palette cycling: the palette turned by s (one turn per 1: t/4 turns it every 4 s), at v; wraps where pal stops; s may change by row; cheap when v is the same every frame |
| `gray(v)` | white, 0 to 1 |
| `#ff8800` | a colour: alone, or #f80 * v, mix(#000, #f80, v) |
| `rgb(r, g, b, k)` | a last value: how much it covers, 0 to 1 (what is under shows through); rgb(#f80, k) |

#### Declarations

| write | what it does |
| --- | --- |
| `name "Sunrise"` | the formula’s name (60 characters at most) |
| `about "…"` | one line about it (120 at most) |
| `control speed 0..1 = 0.5 "Speed"` | a slider: its name in the formula, range, default and label (three controls at most) |
| `control mode = calm, wild` | buttons: the value is the choice’s number, 0 for the first (pick(mode, …)); two words make a switch |
| `look screen 40` | it can be the panel’s look: drawn over the GIF with a blend and an amount (look alone: over, 100) |
<!-- psi-builtins end -->

## Limits

<!-- psi-builtins: limits (generated from opendmd-psi/src/builtins.rs: PSI_BLESS=1 cargo test --features docs --test builtins) -->
| limit | at most |
| --- | --- |
| a formula's text, comments included | 16,384 characters |
| steps (per frame, per row, per pixel, and one per layer) | 256 |
| values held a pixel at once | 12 |
| constants and per-frame and per-row values held at once | 128 |
| nesting of brackets and operators (a part counts 3) | 18 |
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

- **A share link** carries the text: `https://opendmd.dev/f/#formula=…`
  (and the name and control values). The panel's built-in Formulas travel by
  name: `#bundled=<name>`.
- **On the panel** a playlist line or the look names a built-in Formula
  (`@formula:<name>`) or one saved on the panel; a formula sent to show it
  at once goes as its text on one line, comments left out
  (`@formula?src=…`, 6,144 bytes at most; 2,048 for a look set that way).
- **In the gallery** a formula is a file of two comment lines (`--
  author: …` and `-- licence: CC0-1.0`) and then the formula, 4,096 bytes
  at most.

## Tools

The `psi` command checks and formats formulas on a computer
(`cargo install --path opendmd-psi-cli` from the source, or `make psi`):

```text
$ psi check storm-front.psi --out -
ok: 41 of 256 steps (per frame 11, per row 5, per pixel 25), 2 layers, 3 of 12 registers, 40 of 128 values
name: Storm front
never repeats (loop forever) at 50 fps; reads t
panel: ~6.8 ms a frame: medium (light ≤ 5 ms, medium ≤ 8 ms; GIF looks take ≤ 12 ms)
compiled: ~5.6 ms a frame with the panel's JIT (the VM's ~6.8 ms is what the limits count)
compile: 130 tokens, ~16.0 KB of heap; the panel asks for ~34.5 KB free first (about 63 KB is free between items)
length: 541 B as written (16,384 at most), 364 B as the panel keeps it saved (8,192 at most), 397 B as a path (@formula?src=…: 6,144 to show it, 2,048 for an inline look)

$ psi check broken.psi
error[E203]: line 3, column 1: `v` is already set on line 2 — each name can be set once
  v = 2
  ^
```

`psi check` also writes a PNG contact sheet of frames across one loop
(`--out`, `--frames`, `--a`/`--b`/`--c`, `--clock`, `--hash`, `--listing`).
`psi fmt FILE` prints the formula in the one canonical layout (`--write`
rewrites the file, `--check` names the ones that aren't): one statement a
line, two spaces inside `{ }`, spaces round `=`, comparisons and `+ -`, and
`*`, `/`, `%` close up where they bind tighter (`x*3 + t/8`); comments kept
where they are. It changes only the spaces between the formula's words, so
the formula is the same program, and it says so before it writes.
`psi builtins --json` and `psi errors --json` give this page's tables as
data.

## Error codes

Every error the compiler gives has a code, for tools and links (each
entry's address is this page with `#` and the code, as `#E203`). The
editor shows the sentence only.


<!-- psi-errors (generated from opendmd-psi/src/errors.rs: PSI_BLESS=1 cargo test --features docs --test diagnostics) -->
### Characters and numbers (E0xx)

<h4 id="E001">E001 · a character Psi does not know</h4>

> I don't understand `…`

Take the character out. Text in quotes only goes on `name`, `about` and `control` lines; for a character's code write it in single quotes, 'A'.

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

Put a `*` between the number and the name.

<h4 id="E006">E006 · a bad character</h4>

> a character goes in quotes, one at a time: 'A', '7' or ':' (there are no strings)

Write one character between single quotes, as 'A'.

### Syntax (E1xx)

<h4 id="E101">E101 · something else was expected</h4>

> expected … here, found …

Write what the message says is expected at that place.

<h4 id="E102">E102 · a ( never closed</h4>

> this `(` is never closed — add `)`

Add the `)` that closes it.

<h4 id="E103">E103 · a block never closed</h4>

> this `layer {` is never closed — add `}`

Add the `}` that closes the block.

<h4 id="E104">E104 · the colour is not last</h4>

> the colour has to be the last line (move this to the end)

Move the colour line to the end of the formula (or the layer or part).

<h4 id="E105">E105 · <code>loop</code> inside a layer</h4>

> `loop` is the whole formula's: put it on a line of its own, outside the layers

Move the `loop` line out of the layer.

<h4 id="E106">E106 · a value is missing</h4>

> a value is missing here

Write the value that goes there.

<h4 id="E107">E107 · a [ never closed</h4>

> this `[` is never closed — add `]`

Add the `]` that closes it.

<h4 id="E108">E108 · a bare value that is not the colour</h4>

> only the last line can be a bare value (the colour) — did you mean `name = …`?

Give the value a name with `name = …`, or make it the last line.

<h4 id="E109">E109 · <code>loop</code> set twice</h4>

> `loop` is already set

Keep one `loop` line.

<h4 id="E110">E110 · <code>loop</code> without a time</h4>

> `loop` takes a number of seconds, e.g. `loop 8`, or `forever` for one that goes on without repeating

Write `loop` and a number of seconds, or `loop forever`.

<h4 id="E111">E111 · <code>use</code> left in a formula</h4>

> `use` puts a formula in by its name, as `part p { … }`, when you leave the line

Finish the line or write the part out as `part p { … }`.

<h4 id="E112">E112 · an empty formula</h4>

> empty: write a colour, e.g. `hsv(x + t/4, 1, 1)`

Write a colour as the last line.

<h4 id="E113">E113 · no colour at the end</h4>

> the last line should be the colour, e.g. `rgb(r, g, b)`, `hsv(h, s, v)` or `pal(fire, k)`

End the formula with a colour line.

<h4 id="E114">E114 · the wrong number of values</h4>

> `…` takes … values, but got …

Give the function the number of values it takes.

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

Set the name on an earlier line, or fix the spelling.

<h4 id="E202">E202 · an unknown function</h4>

> unknown function `…` (did you mean `…`?)

Fix the spelling, or use a function from the cheat sheet.

<h4 id="E203">E203 · a name set twice</h4>

> `…` is already set on line … — each name can be set once

Use a different name for the second one.

<h4 id="E204">E204 · a name that is built in</h4>

> `…` is built in; pick another name

Pick another name.

<h4 id="E205">E205 · a function used as a value</h4>

> `…` is a function: write `…(…)`

Call it with brackets: `name(…)`.

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

Use one of the listed palettes, or write your own with `palette name = #…, #…`.

<h4 id="E212">E212 · a palette expected</h4>

> the first value of `…` is a palette: fire, ocean, ice, chrono, amber, rainbow or gray

Start with a palette name.

<h4 id="E213">E213 · an unknown blend</h4>

> unknown blend `…` (a layer goes over, add, multiply, screen, max or min)

Use one of the listed blends.

### Placement and levels (E3xx)

<h4 id="E301">E301 · a per-pixel value in <code>frame { }</code></h4>

> inside frame { } only values that are the same for every pixel can be used, but this uses `…` (…)

Use only values that are the same everywhere (such as `t`) inside `frame { }`.

<h4 id="E302">E302 · a colour used as a value</h4>

> a colour can't be a value: use it in the colour itself (the last line), as `#f80`, `#f80 * k`, `mix(#000, #f80, k)` or `rgb(#f80, k)`

Use the colour in the colour line, not in a number's place.

<h4 id="E303">E303 · a colour outside a layer</h4>

> with layers, every colour goes inside one: put this line in `layer { … }`

Put the colour line inside a layer.

<h4 id="E304">E304 · a colour function in a sum of colours</h4>

> in a sum with colours only `#…` and `rgb(r, g, b)` are colours: `…(…)` can only be a whole line of its own

Use `#…` or `rgb(…)` in the sum, or give the function its own line.

<h4 id="E305">E305 · a colour that can't be summed</h4>

> `…(…)` can't go into a sum: it can only be scaled, `… * k` (`#…` and `rgb(r, g, b)` colours can: `#fff * k`, `mix(#000, #f80, k)`)

Scale it on its own line, or use `#…` or `rgb(…)` colours in the sum.

<h4 id="E306">E306 · a colour function that is not the whole line</h4>

> `…(…)` makes the colour, so it can only be the whole last line (of the formula, or of a layer)

Make it the whole last line.

<h4 id="E307">E307 · a colour inside <code>rgb(r, g, b)</code></h4>

> inside `rgb(r, g, b)` the three are values, not colours: write the colour on its own (`#f80`, `#f80 * k`)

Give `rgb(r, g, b)` three numbers, or write the colour on its own.

<h4 id="E308">E308 · a second <code>frame { }</code> block</h4>

> there can be one `frame { }` block

Merge the `frame { }` blocks into one.

<h4 id="E309">E309 · a line that does not belong in <code>frame { }</code></h4>

> only `name = value` lines can go inside frame { }

Put only `name = value` lines inside `frame { }`.

### Limits (E4xx)

<h4 id="E401">E401 · a formula that is too long to read</h4>

> too long: a formula can be … characters at most

Shorten the formula.

<h4 id="E402">E402 · too many steps</h4>

> this formula is too long for the panel (… steps; the limit is …) — simplify it

Simplify the formula: fewer or simpler steps.

<h4 id="E403">E403 · too many live values</h4>

> this formula keeps too many values at once (more than …) — simplify it

Set fewer names, or use each value sooner.

<h4 id="E404">E404 · too many numbers</h4>

> this formula has too many numbers and steps (more than … values) — simplify it

Use fewer different numbers and steps.

<h4 id="E405">E405 · nested too deeply</h4>

> this is nested too deeply — split it into named steps

Split it into named steps, `name = …`.

<h4 id="E406">E406 · a loop time out of range</h4>

> the loop must be between 0.1 and 3600 seconds

Use a loop between 0.1 and 3600 seconds.

<h4 id="E407">E407 · a layout fault</h4>

> this formula could not be laid out (a bug in Psi) — please report it

Report it with the formula.

<h4 id="E408">E408 · not enough memory</h4>

> not enough memory for this Formula right now

Try again, or make the Formula smaller.

### Parts and layers (E5xx)

<h4 id="E501">E501 · a part without a colour</h4>

> part `…` has no colour, only values: `….v`

Use one of its values, as `p.v`.

<h4 id="E502">E502 · a part without that value</h4>

> part `…` sets no value `…` (did you mean `…`?)

Use a name the part sets.

<h4 id="E503">E503 · parts too deep</h4>

> parts go too deep here — take one out of another

Take one part out of another.

<h4 id="E504">E504 · a part with layers</h4>

> this part has layers of its own: it can only go over, as in `layer { p }`

Draw it as `layer { p }`.

<h4 id="E505">E505 · too many places</h4>

> this formula draws its parts in too many places — simplify it

Draw fewer parts, or each part in fewer places.

<h4 id="E506">E506 · a part without a name</h4>

> a part needs a name, as in `part p { … }`

Name the part.

<h4 id="E507">E507 · a part used as a value</h4>

> `…` is a part: draw it as a whole last line (of the formula, or of a layer), or use one of its values, as in `….v`

Draw it as the last line, or use `p.v`.

<h4 id="E508">E508 · a part inside a layer</h4>

> a part goes outside the layers (draw it in one with `layer { p }`)

Move the part out of the layer.

<h4 id="E509">E509 · a layer inside a layer</h4>

> a layer can't go inside another — close this one with `}` first

Close the first layer before starting another.

<h4 id="E510">E510 · a layer without a colour</h4>

> this layer has no colour: end it with `rgb(…)`, `hsv(…)`, `pal(…)` or `gray(…)`

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

### The draw stage (E7xx)

<h4 id="E701">E701 · <code>draw { }</code> or a table inside a layer</h4>

> `draw { }` goes at the top of the formula (or a part), not inside a layer

Move it out of the layer, to the top of the formula or the part.

<h4 id="E702">E702 · a second <code>draw { }</code> block</h4>

> there can be one `draw { }` block (the formula's or one part's: they share one canvas)

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

> `…(…)` draws: it goes on a line of its own inside `draw { }` (also: `ink` needs a `draw { }` block to read from; `for` goes inside `draw { }`)

Put the line inside `draw { }`, and write a `draw { }` block to read `ink`.

<h4 id="E707">E707 · a table that is not numbers</h4>

> a table holds numbers only, up to 256 in all, as in `table T = 1, -2, 0.5`

Write only numbers in the table.

<h4 id="E708">E708 · a per-pixel value in <code>draw { }</code></h4>

> inside draw { } only values that are the same for every pixel can be used (no x, y, px, py, i, ur, ug, ub, ul or ink), but this uses `…` (…)

Use only values that are the same everywhere inside `draw { }`.

<h4 id="E709">E709 · a loop counter set twice</h4>

> `…` is a loop counter: it can't be set (or: `…` is already a loop counter here)

Use another name.

<h4 id="E710">E710 · a table read wrongly</h4>

> `[ ]` reads a table: `NAME[k]` — `…` is not a table (or: `…` is a table: read one of its numbers with `…[k]`)

Read a table with `NAME[k]`.

<h4 id="E711">E711 · a table without a name</h4>

> a table needs a name, as in `table T = 1, 2, 3`

Name the table.

<h4 id="E712">E712 · the wrong values for <code>line</code> or <code>dot</code></h4>

> `line` takes 4 values (x0, y0, x1, y1: pixels, y up from the bottom) and, if you like, the ink at each end (0 to 1), but got … (`dot` takes 2 values)

Give `line` four values and `dot` two.

<h4 id="E713">E713 · a line that does not belong in <code>draw { }</code></h4>

> inside draw { } go `name = value`, `for k < N { … }`, `line(…)` and `dot(…)` lines only

Use only those lines inside `draw { }`.

### Frame rate (E8xx)

<h4 id="E801">E801 · a bad <code>fps</code></h4>

> `fps` is 25, 50 or 100 frames a second

Write `fps 25`, `fps 50` or `fps 100`.

<h4 id="E802">E802 · <code>fps</code> inside a layer</h4>

> `fps` is the whole formula's: put it on a line of its own, outside the layers

Move the `fps` line out of the layer.

<h4 id="E803">E803 · <code>fps</code> set twice</h4>

> `fps` is already set

Keep one `fps` line.

### Growing values, sprites, palettes, slow layers and declarations (E9xx)

<h4 id="E901">E901 · a value that grows without end</h4>

> a value that grows without end (`t` in `loop forever`) can't go in here: use frac(t\*k), t % n, sin(t\*k), noise(t\*k, y, z), a comparison, min, max or smoothstep

Use frac(t\*k), t % n, sin(t\*k) or similar in its place.

<h4 id="E902">E902 · too many noises of growing values</h4>

> noise of values that grow without end: four different ones at most — share one, as noise(u + k, …)

Share one noise between the places that need it.

<h4 id="E903">E903 · <code>seed</code> outside <code>loop forever</code></h4>

> `seed` is for formulas that go on without repeating (`loop forever`): a new one each time it plays

Add `loop forever`, or leave `seed` out.

<h4 id="E904">E904 · too many growing values</h4>

> this formula has too many values that grow without end (more than …) — simplify it

Use fewer values that grow without end.

<h4 id="E905">E905 · a colour fed a growing value</h4>

> a colour can't take a value that grows without end and changes across the panel (`t` in a `loop forever` formula): use frac(…) of it, or min(t/k, 1)

Wrap it in frac(…), or limit it with min(t/k, 1).

<h4 id="E906">E906 · drawing at a growing value</h4>

> a value that grows without end can't be drawn at: use frac(t\*k)\*128, t % n or min(t, k)

Use frac(t\*k)\*128, t % n or min(t, k) as the place.

<h4 id="E910">E910 · a sprite's key too long</h4>

> a sprite's key has 16 characters at most

Use at most 16 characters in the key.

<h4 id="E911">E911 · a sprite inside a layer</h4>

> a sprite goes at the top of the formula (or a part), not inside a layer

Move the sprite out of the layer.

<h4 id="E912">E912 · a sprite without its key</h4>

> a sprite starts with its key: `key . 0, R 1, D 2` (each character, then the number it reads as)

Start the sprite with a `key` line.

<h4 id="E913">E913 · a bad key entry</h4>

> a key is a character and its number, as `R 1` or `. -0.5`, separated by commas (not ; { } , - / or a space)

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

<h4 id="E918">E918 · a sprite read wrongly</h4>

> a sprite is read at a column and a row: `NAME[u, v]` (or: `…` is a sprite: read its pixels with `NAME[u, v]`)

Read a sprite with `NAME[u, v]`.

<h4 id="E919">E919 · a sprite never closed</h4>

> this `sprite {` is never closed — add `}` on a line of its own

Add a `}` on a line of its own.

<h4 id="E920">E920 · a character twice in the key</h4>

> `…` is in the key twice

Keep one entry for each character.

<h4 id="E921">E921 · a palette inside a layer</h4>

> a palette goes at the top of the formula (or a part), not inside a layer

Move the palette out of the layer.

<h4 id="E922">E922 · a palette of the wrong size</h4>

> a palette is 1 to 32 colours: `palette sea = #003, #06c, #fff` (pal(sea, 0) is the first, pal(sea, 1) the last)

Write 1 to 32 colours, each as #f80.

<h4 id="E923">E923 · too many palettes</h4>

> a formula can have … palettes of its own at most

Use fewer palettes.

<h4 id="E924">E924 · too many numbers in tables, sprites and palettes</h4>

> tables, sprites and palettes hold 256 numbers in all — this would make …

Make the tables, sprites and palettes smaller.

<h4 id="E931">E931 · <code>cycle</code> turned by a value that changes along a row</h4>

> `cycle`'s turn is the same along a row: a value of t, a frame or the row (py), not of x or px

Turn it by t, a `frame { }` value or py.

<h4 id="E932">E932 · <code>cycle</code> of gray</h4>

> `cycle` turns a palette of colours, and gray is not one: write your own, `palette g = #000, #fff`

Write your own palette.

<h4 id="E933">E933 · too many palettes with <code>cycle</code></h4>

> a formula has room for … palettes: its own (… here) and each palette `cycle` turns by another value (… here)

Use fewer palettes, or turn fewer by different values.

<h4 id="E934">E934 · <code>cycle</code> without its turn</h4>

> expected `,` and the turn (as in `cycle(fire, v, t/4)`) here, found …

Write the turn after the picture: `cycle(palette, v, turn)`.

<h4 id="E935">E935 · the wrong values for <code>pal</code> or <code>cycle</code></h4>

> `…` takes a palette (and how much of the pixel it covers, 0 to 1, if you like); found …

Give it a palette and a value (`cycle` also takes the picture and the turn).

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

<h4 id="E945">E945 · <code>rate</code> on a part</h4>

> `rate` goes on the formula's own layers, not on a part's

Put `rate` on the formula's own layers.

<h4 id="E946">E946 · <code>rate</code> before the blend</h4>

> write the blend first: `layer add rate 4 { … }`

Write the blend, then `rate`.

<h4 id="E947">E947 · only <code>rate</code> layers</h4>

> a `rate` layer is the slow one under a layer that moves: add a layer above it, or leave `rate` out

Add a layer above, or leave `rate` out.

<h4 id="E948">E948 · a bottom <code>rate</code> layer that does not cover</h4>

> the bottom `rate` layer draws the whole panel: make it cover everything (`layer rate N { … }` with no blend and no coverage)

Leave the blend and the coverage off the bottom `rate` layer.

<h4 id="E951">E951 · a declaration below other lines</h4>

> `…` goes at the top of the formula, before its other lines

Move it to the top.

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

Write it as shown.

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

> `…` is control …'s letter: name this one something else (controls take a, b and c in order)

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
