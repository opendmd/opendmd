# Psi error codes

Every error the compiler gives has a code, for tools and links (each
entry's address is this page with `#` and the code, as `#E203`). The
editor shows the sentence with its code, linked to its entry here. The
common errors have a word on why they happen and a before and after; the
codes marked internal are never a formula's fault. What each name does is
in the [Psi language reference](psi-spec.md).

<!-- psi-errors (generated from opendmd-psi/src/errors.rs: PSI_BLESS=1 cargo test --features docs --test diagnostics) -->
## Characters and numbers (E0xx)

<h3 id="E001">E001 · a character Psi does not know</h3>

> I don't understand `…`

Psi has numbers, names, a few operators and brackets; anything else, such as `^`, `$` or a double quote outside a declaration or `print(…)`, is not part of it.

Take the character out. Text in quotes only goes on `name`, `about` and `control` lines and in `print(…)`; for a character's code write it in single quotes, 'A'.

Before:

```psi-bad
gray(x ^ 2)
```

After:

```psi
gray(x * x)
```

<h3 id="E002">E002 · a bad number</h3>

> `…` is not a number

Write the number with digits and at most one point, as 12 or 0.5.

<h3 id="E003">E003 · a bad colour</h3>

> a colour is # and 3 or 6 hex digits (0-9, a-f), as #f80 or #ff8800

Write # and then 3 or 6 hex digits.

<h3 id="E004">E004 · a number out of range</h3>

> numbers must be between -32768 and 32767

Use a smaller number, or scale it down with a multiply.

<h3 id="E005">E005 · a number run into a name</h3>

> `…` is not a number or a name (to multiply, write `… * …`)

Psi never multiplies by putting things side by side, as maths on paper does.

Put a `*` between the number and the name.

Before:

```psi-bad
gray(2x)
```

After:

```psi
gray(2 * x)
```

<h3 id="E006">E006 · a bad character</h3>

> a character goes in quotes, one at a time: 'A', '7' or ':' (there are no strings)

Write one character between single quotes, as 'A'.

## Syntax (E1xx)

<h3 id="E101">E101 · something else was expected</h3>

> expected … here, found …

Two values side by side with nothing between them: usually an operator or a comma is missing.

Write what the message says is expected at that place.

Before:

```psi-bad
v = x y
gray(v)
```

After:

```psi
v = x * y
gray(v)
```

<h3 id="E102">E102 · a ( never closed</h3>

> this `(` is never closed — add `)`

Every `(` needs its `)`: the message points at the one left open.

Add the `)` that closes it.

Before:

```psi-bad
v = (x + 1
gray(v / 2)
```

After:

```psi
v = (x + 1)
gray(v / 2)
```

<h3 id="E103">E103 · a block never closed</h3>

> this `layer {` is never closed — add `}`

A block (`layer`, `draw`, `fn`, `for`, `if`, `sprite`) runs to its `}`; the formula ended first.

Add the `}` that closes the block.

Before:

```psi-bad
layer {
  gray(x)
```

After:

```psi
layer {
  gray(x)
}
```

<h3 id="E104">E104 · the colour is not last</h3>

> the colour has to be the last line (move this to the end)

The last line is the colour, and only the last: lines after it would never be used.

Move the colour line to the end of the formula (or the layer or function).

Before:

```psi-bad
gray(x)
v = 1
```

After:

```psi
v = 1
gray(x * v)
```

<h3 id="E105">E105 · <code>loop</code> inside a layer</h3>

> `loop` is the whole formula's: put it on a line of its own, outside the layers

Move the `loop` line out of the layer.

<h3 id="E106">E106 · a value is missing</h3>

> a value is missing here

An operator, a `=` or a `,` with nothing after it.

Write the value that goes there.

Before:

```psi-bad
v =
gray(v)
```

After:

```psi
v = x
gray(v)
```

<h3 id="E107">E107 · a [ never closed</h3>

> this `[` is never closed — add `]`

Add the `]` that closes it.

<h3 id="E108">E108 · a bare value that is not the colour</h3>

> only the last line can be a bare value (the colour) — did you mean `name = …`?

A line that is only a value, not the last line: it would be worked out and thrown away, so it is most likely a name left out.

Give the value a name with `name = …`, or make it the last line.

Before:

```psi-bad
x + 1
gray(x)
```

After:

```psi
v = x + 1
gray(v / 2)
```

<h3 id="E109">E109 · <code>loop</code> set twice</h3>

> `loop` is already set

Keep one `loop` line.

<h3 id="E110">E110 · <code>loop</code> without a time</h3>

> `loop` takes a number of seconds, e.g. `loop 8`, or `forever` for one that goes on without repeating

Write `loop` and a number of seconds, or `loop forever`.

<h3 id="E111">E111 · <code>use</code> left in a formula</h3>

> `use` puts a formula in by its name, as a function `fn p(x, y) { … }`, when you leave the line

Finish the line (the editor puts the formula in), or write the function out.

<h3 id="E112">E112 · an empty formula</h3>

> empty: write a colour, e.g. `hsv(x + t/4, 1, 1)`

Write a colour as the last line.

<h3 id="E113">E113 · no colour at the end</h3>

> the last line should be the colour, e.g. `rgb(r, g, b)`, `hsv(h, s, v)` or `pal(fire, k)`

End the formula with a colour line.

<h3 id="E114">E114 · the wrong number of values</h3>

> `…` takes … values, but got …

Each function takes a fixed number of values (the reference's tables say how many); colours may take one more, how much of the pixel they cover.

Give the function the number of values it takes.

Before:

```psi-bad
gray(x, 2, 3)
```

After:

```psi
gray(x, 0.5)
```

<h3 id="E115">E115 · the wrong values for a character</h3>

> `…` takes 3 values (…, a column and a row; and a font, normal, small or bold, if you like), but got …

Give it the value, a column and a row, and a font if you like.

<h3 id="E116">E116 · the wrong values for <code>pick</code></h3>

> `pick` takes a number, then the values to pick from: `pick(k, a, b, …)`

Write `pick(k, a, b, …)`: a number, then the values.

<h3 id="E117">E117 · the bold font has digits only</h3>

> the bold font has digits only: use `digit(n, x, y, bold)`

Use `digit(…)` for the bold font.

<h3 id="E118">E118 · not a font</h3>

> the last value of `…` is a font: normal (the default), small or bold, found …

Write normal, small or bold, or leave the font out.

<h3 id="E119">E119 · the wrong values for <code>rgb</code> of a colour</h3>

> `rgb` of a colour takes the colour and, if you like, how much of the pixel it covers (0 to 1)

Write `rgb(colour)` or `rgb(colour, cover)`.

<h3 id="E120">E120 · coverage on an inner colour</h3>

> how much a colour covers goes on the whole line: `rgb(mix(…), k)`, not on an `rgb(…)` inside it

Move the coverage to the outer `rgb(…)`.

## Names (E2xx)

<h3 id="E201">E201 · an unknown name</h3>

> unknown name `…` (did you mean `…`?)

A name is used before it is set, or is spelled differently where it is set.

Set the name on an earlier line, or fix the spelling.

Before:

```psi-bad
gray(level)
```

After:

```psi
level = x
gray(level)
```

<h3 id="E202">E202 · an unknown function</h3>

> unknown function `…` (did you mean `…`?)

Only the built-in functions can be called; the message suggests the nearest one.

Fix the spelling, or use a function from the cheat sheet.

Before:

```psi-bad
gray(sine(x))
```

After:

```psi
gray(sin(x))
```

<h3 id="E203">E203 · a name that is already a function, table, sprite, palette or control</h3>

> `…` is already the name of a … (line …) — pick another name

A value may be set again (`v = v * 2` reads the `v` before it), but functions, tables, sprites, palettes and controls keep their names: neither a value nor another of them can take one. Give it a name of its own.

Give it a name of its own. (A value may be set again, `v = v*2`, and a layer or function may set a name from outside: only functions, tables, sprites, palettes and controls keep theirs, and none of them takes a value's name.)

Before:

```psi-bad
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

<h3 id="E204">E204 · a word of the language as a name</h3>

> `…` is a word of the language; pick another name

The language's own words (`loop`, `layer`, `part`, `use`, `draw`, `for`, `table`, `fn`) can't be names; any built-in value or function's name can (it is then yours).

Pick another name: `loop`, `layer`, `part`, `use`, `draw`, `for`, `table` and `fn` are the language's own words (any built-in value's name may be yours, and a built-in function's as a value; not a palette's or `sun`, `moon` or `replay`).

Before:

```psi-bad
layer = 1
gray(x * layer)
```

After:

```psi
level = 1
gray(x * level)
```

<h3 id="E205">E205 · a function used as a value</h3>

> `…` is a function: write `…(…)`

A function's name alone is not a value: it needs its brackets and values.

Call it with brackets: `name(…)`.

Before:

```psi-bad
gray(sin)
```

After:

```psi
gray(sin(x))
```

<h3 id="E206">E206 · a palette used as a value</h3>

> `…` is a palette: use it as `pal(…, k)` on the last line

Use it as `pal(name, k)` on the last line.

<h3 id="E207">E207 · a font used as a value</h3>

> `…` is a font: use it as `digit(n, x, y, …)`

Use it as the font of `digit(…)`.

<h3 id="E208">E208 · a name that is a palette's</h3>

> `…` is a palette's name; pick another name

Pick another name.

<h3 id="E209">E209 · a value called like a function</h3>

> `…` is a value, not a function

Leave the brackets off.

<h3 id="E210">E210 · a keyword inside a line</h3>

> `…` can only start a line

Move it to the start of a line.

<h3 id="E211">E211 · an unknown palette</h3>

> unknown palette `…` (there are fire, ocean, ice, chrono, amber, rainbow and gray…)

The palettes are fire, ice, ocean, chrono, amber, rainbow and gray, and the ones the formula makes with `palette`.

Use one of the listed palettes, or write your own with `palette name = #…, #…`.

Before:

```psi-bad
pal(lava, x)
```

After:

```psi
pal(fire, x)
```

<h3 id="E212">E212 · a palette expected</h3>

> the first value of `…` is a palette: fire, ocean, ice, chrono, amber, rainbow or gray

Start with a palette name.

<h3 id="E213">E213 · an unknown blend</h3>

> unknown blend `…` (a layer goes over, add, multiply, screen, max or min)

Use one of the listed blends.

<h3 id="E214">E214 · <code>ink(n)</code> without a whole number</h3>

> `ink(n)` takes a whole number from 1 to 255: `ink(16)` is the canvas in 16 levels

Write the number of levels, as `ink(16)`, and draw with inks `k/16`.

<h3 id="E215">E215 · a function named as a built-in one</h3>

> `…(…)` is a built-in function: a `fn` takes a name of its own

Name the function something else, as `fn wave(a) = …`. (A value may take a built-in function's name: `sin = 0.5`, and `sin(…)` is still the function.)

<h3 id="E216">E216 · a name that is the dotted inputs' group</h3>

> `…` is the built-in inputs' name (…); pick another name

Pick another name: `sun`, `moon` and `replay` are the start of the built-in inputs `sun.alt`, `moon.phase`, `replay.game` and the rest.

<h3 id="E217">E217 · <code>since(…)</code> or <code>was(…)</code> of the formula's own value</h3>

> `…` is this formula's own value here: `…(…)` reads a built-in input, so give yours another name

Rename your value (`light = …`): the formula's own names win everywhere, so `since(daylight)` would otherwise mean two things on one line.

<h3 id="E218">E218 · a chained comparison</h3>

> comparisons don't chain: write `… and …` (`…` would compare `…`, 1 or 0, with …)

Join two comparisons with `and`: `0 < x and x < 0.5` (a comparison gives 1 or 0, so `0 < x < 0.5` would compare that with 0.5).

## Placement and levels (E3xx)

<h3 id="E301">E301 · a <code>frame { }</code> block</h3>

> there are no `frame { }` blocks: a value is worked out once a frame by itself when it reads only the time, the clock and the controls; write its lines without the block

There are no `frame { }` blocks: a value that reads only the time, the clock and the controls is worked out once a frame by itself.

Take the lines out of the block, as they are.

Before:

```psi-bad
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

<h3 id="E302">E302 · a colour used as a value</h3>

> `#…` is a colour, not a number: use it on the last line, as `#f80 * k` or `mix(#000, #f80, k)`

A colour such as `#f80` is not a number: it goes in the colour line, where it can be dimmed or mixed.

Use the colour in the colour line, not in a number's place.

Before:

```psi-bad
v = #f80
gray(v)
```

After:

```psi
v = x
#f80 * v
```

<h3 id="E303">E303 · a colour outside a layer</h3>

> with layers, every colour goes inside one: put this line in `layer { … }`

Once a formula has a layer, each colour belongs to one: a colour line outside them would have nothing to go on.

Put the colour line inside a layer.

Before:

```psi-bad
layer { gray(x) }
gray(y)
```

After:

```psi
layer { gray(x) }
layer add { gray(y) }
```

<h3 id="E304">E304 · a colour function in a sum of colours</h3>

> `…(…)` is a whole colour: it can't be combined with others by `+`, `*`, `? :` or a function (only `#…` and `rgb(r, g, b)` can, as `c ? #f80 : #000`)

Use `#…` or `rgb(…)` in the sum, or give the function its own line.

<h3 id="E305">E305 · a colour that can't be summed</h3>

> `…(…)` is a whole colour: it can't go into a sum, only be scaled, `… * k` (`#…` and `rgb(r, g, b)` colours can: `#fff * k`, `mix(#000, #f80, k)`)

Scale it on its own line, or use `#…` or `rgb(…)` colours in the sum.

<h3 id="E306">E306 · a colour function that is not the whole line</h3>

> `…(…)` is a whole colour: it can only be the last line (of the formula or a layer)

Make it the whole last line.

<h3 id="E307">E307 · a colour inside <code>rgb(r, g, b)</code></h3>

> `rgb(r, g, b)` takes three numbers, not colours: write a colour alone, as `#f80 * k`

Give `rgb(r, g, b)` three numbers, or write the colour on its own.

## Limits (E4xx)

<h3 id="E401">E401 · a formula that is too long to read</h3>

> too long: a formula can be … bytes at most

Shorten the formula.

<h3 id="E402">E402 · too many steps</h3>

> this formula is too long for the panel: … steps (per frame …, draw …, per row …, per pixel …), the limit is … (or: … — simplify it)

The panel works a formula out for 4,096 pixels at every frame, so the steps a formula takes are limited. Values that change less often (once a row, once a frame) cost much less: move what you can out of the pixel.

Simplify the formula: fewer or simpler steps.

<h3 id="E403">E403 · too many live values</h3>

> this formula holds more than … values at once (the panel's registers): use each sooner

Set fewer names, or use each value sooner.

<h3 id="E404">E404 · too many numbers</h3>

> this formula needs … numbers and once-a-frame values, more than the … it can hold (steps: per frame …, draw …, per row …, per pixel …): use fewer

Use fewer different numbers and steps: the section with the most steps is the place to look.

<h3 id="E405">E405 · nested too deeply</h3>

> this is nested too deeply — split it into named steps

Split it into named steps, `name = …`.

<h3 id="E406">E406 · a loop time out of range</h3>

> the loop must be between 0.1 and 3600 seconds

Use a loop between 0.1 and 3600 seconds.

<h3 id="E407">E407 · a layout fault <span class="badge">internal</span></h3>

> this formula could not be laid out (a bug in Psi) — please report it

**Internal:** a bug in Psi: please report it, with the formula. It is never the formula's fault.

Report it with the formula.

<h3 id="E408">E408 · not enough memory</h3>

> not enough memory for this Formula right now

The panel didn't have the memory to compile this Formula just then (a GIF, a look or a transition was holding it): not the formula's fault. It plays when it is next due; a shorter formula needs less.

Try again, or make the Formula smaller.

## Layers (E5xx)

<h3 id="E509">E509 · a layer inside a layer</h3>

> a layer can't go inside another — close this one with `}` first

Close the first layer before starting another.

<h3 id="E510">E510 · a layer without a colour</h3>

> this layer ends without a colour: its last line is `rgb(…)`, `hsv(…)`, `pal(…)` or `gray(…)`

End the layer with a colour.

## Version (E6xx)

<h3 id="E601">E601 · a newer Psi</h3>

> this formula needs a newer OpenDMD: it is written for Psi …, this one has Psi …

Update OpenDMD, or write the formula for an older Psi.

<h3 id="E603">E603 · a bad <code>psi</code> version</h3>

> `psi` takes the language's version, a whole number: `psi 1`

Write `psi 1`.

<h3 id="E604">E604 · <code>psi</code> not on the first line</h3>

> `psi 1` goes on the formula's first line

Move `psi 1` to the first line.

## The draw stage, paint and functions (E7xx)

<h3 id="E701">E701 · <code>draw { }</code> or a table inside a layer</h3>

> `draw { }` goes at the top of the formula, not inside a layer

Move it out of the layer, to the top of the formula.

<h3 id="E702">E702 · a second <code>draw { }</code> block</h3>

> there can be one `draw { }` block (functions that draw are called inside it)

Merge the `draw { }` blocks into one.

<h3 id="E703">E703 · a bad <code>for</code> line</h3>

> `for` takes a name and a count, e.g. `for k < 12 { … }` (a whole number from 1 to 1024)

Write `for k < 12 { … }` with a whole count from 1 to 1024.

<h3 id="E704">E704 · too many loop turns</h3>

> this formula draws too much a frame (… loop turns in all; the limit is …) — fewer or shorter loops

Use fewer or shorter loops.

<h3 id="E705">E705 · <code>for</code> nested too deep</h3>

> `for` loops go two deep at most

Use at most two `for` loops, one inside the other.

<h3 id="E706">E706 · drawing outside <code>draw { }</code></h3>

> `…(…)` draws: it goes on a line of its own inside `draw { }`

`line` and `dot` draw into the frame's ink once a frame, so they live in `draw { }`; the pixels read what they drew as `ink`.

Put the line inside `draw { }`.

Before:

```psi-bad
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

<h3 id="E707">E707 · a table that is not numbers</h3>

> a table holds numbers only, as in `table T = 1, -2, 0.5`

Write only numbers in the table.

<h3 id="E708">E708 · a per-pixel value in <code>draw { }</code></h3>

> `draw { }` runs once a frame, before any pixel, but `…` (line …) is per pixel: it reads `…` (or: but this uses `…` (…))

`draw { }` runs once a frame, before the pixels: there is no pixel position there. Work in pixels, with numbers and the time.

Use only values that are the same everywhere inside `draw { }`.

Before:

```psi-bad
draw { dot(x, 1) }
gray(ink)
```

After:

```psi
draw { dot(t * 16, 1) }
gray(ink)
```

<h3 id="E709">E709 · a loop counter set twice</h3>

> `…` is a loop counter: it can't be set (or: `…` is already a loop counter here)

Use another name.

<h3 id="E710">E710 · a table read wrongly</h3>

> `…[…]` reads a table, and `…` is not one

Read a table with `NAME[k]`.

<h3 id="E711">E711 · a table without a name</h3>

> a table needs a name, as in `table T = 1, 2, 3`

Name the table.

<h3 id="E712">E712 · the wrong values for <code>line</code> or <code>dot</code></h3>

> `line` takes x0, y0, x1, y1 (pixels from the top left) and, if you like, the ink at each end (0 to 1): … given (`dot`: x, y and, if you like, its ink)

Give `line` four values and `dot` two.

<h3 id="E713">E713 · a line that does not belong in <code>draw { }</code></h3>

> inside draw { } go `name = value`, `for k < N { … }`, `if c { … }` and drawing: `line`, `dot`, `pset`, `rect`, `rectfill`, `circ`, `circfill`, `trifill`, `cls`, `spr`, `print`

Use only those lines inside `draw { }`; the colour line goes after it.

<h3 id="E714">E714 · <code>ink</code> without <code>draw { }</code></h3>

> `ink` needs a `draw { }` block to read from

Write a `draw { }` block that draws what `ink` reads.

<h3 id="E715">E715 · <code>for</code> outside <code>draw { }</code></h3>

> `for` goes inside `draw { }`: `draw { for k < 12 { … } }`

Put the loop inside `draw { }`.

<h3 id="E716">E716 · a table without <code>[k]</code></h3>

> `…` is a table: read one of its numbers, `…[k]` (k from 0)

Read a number of the table with `NAME[k]`.

<h3 id="E720">E720 · too many colours to paint with</h3>

> a formula paints with … colours at most (each of a palette's 32 counts)

Paint with fewer colours: a palette used in `pal(P, v)` takes 32 of the 127.

<h3 id="E721">E721 · not a colour to paint with</h3>

> a colour to paint with is `#…` (as #f80), `pal(palette, v)` or `gray(v)`

End the call with a colour: `#f80`, `pal(fire, v)` or `gray(v)` (for brightness into ink, `line` and `dot` take a number).

<h3 id="E722">E722 · <code>spr</code> of something that is not a sprite</h3>

> `spr` draws a sprite: `spr(name, x, y)`, its name a `sprite name { … }`'s

Name a sprite block, as `spr(ship, x, y)`.

<h3 id="E723">E723 · <code>spr</code> without its place</h3>

> … takes a sprite and its place

Give the sprite's place: `spr(ship, x, y)`, and maybe a flip (1 across, 2 down, 3 both).

<h3 id="E724">E724 · a text too long to print</h3>

> `print` writes 32 characters at most

Split the text into two `print`s.

<h3 id="E725">E725 · no room for a text</h3>

> this text doesn't fit: tables, sprites and texts hold 1024 numbers in all

Use shorter texts, or fewer tables and sprites.

<h3 id="E726">E726 · a font <code>print</code> doesn't have</h3>

> `print`'s font is `normal`, `small` or (for a value) `bold`

Write `normal`, `small` or, for a number, `bold`.

<h3 id="E727">E727 · something after a paint call's values</h3>

> …: expected `)` here

Close the call after its colour (and, for `print`, its font and digits).

<h3 id="E740">E740 · a function written wrongly</h3>

> a function is `fn name(a, b) = value`, or `fn name(a, b) { … }` for lines that draw

Write `fn name(a, b) = value`, or `fn name(a, b) { … }` around lines that draw.

<h3 id="E742">E742 · a function's value that is not a name</h3>

> a function's values are names: `fn name(a, b) = …`

Name each value the function takes: `fn ring(cx, r) = …`.

<h3 id="E743">E743 · a function with no body</h3>

> after a function's values: `= value`, or `{ … }` for lines that draw

Put `= value` after the brackets, or `{ … }` around lines that draw.

<h3 id="E744">E744 · an empty function</h3>

> function `…` has nothing in it

Write what the function works out, or the lines it draws.

<h3 id="E745">E745 · a function calling itself</h3>

> `…` calls itself: a function can't (there are no loops of calls)

Use a `for` loop in draw, or write the steps out.

<h3 id="E746">E746 · a drawing function used as a value</h3>

> `…` draws: call it on a line of its own inside `draw { }`

Call it on its own line in `draw { }`: `palm(30, 31, 1)`.

<h3 id="E747">E747 · more than one value on a function's last line</h3>

> function `…`'s last line is one value: `fn …(…) = value`

Write one value after `=`, or use `{ … }` for lines that draw.

<h3 id="E748">E748 · a value function called as lines</h3>

> `…` is a value, not lines that draw: use it in one, `v = …(…)`

Use it in a value, `v = name(…)`, or in a call's values.

<h3 id="E749">E749 · too many calls</h3>

> this formula calls its functions too many times over — simplify it

Call a function fewer times, or set its value once as a name and use the name.

<h3 id="E750">E750 · a part</h3>

> there are no parts: write a function, `fn p(x, y) { … }`, and call it, `p(x*2, y)`

Write `fn p(x, y) { … }` with the part's lines; draw it with `p(x, y)` as the last line.

<h3 id="E751">E751 · a value before a function's last line</h3>

> only a function's last line is its value: set a name here (`v = …`), or move it last

Set a name on each line but the last: `v = …`.

<h3 id="E752">E752 · a whole-colour function inside a value</h3>

> `…(…)` is a whole colour: use it alone on the colour line (only `#…` and `rgb(r, g, b)` go into sums)

Use it alone as the last line, or make its last line `#…` or `rgb(…)` so it can be mixed.

## Frame rate (E8xx)

<h3 id="E801">E801 · a bad <code>fps</code></h3>

> `fps` is 25, 50 or 100 frames a second

The panel draws a Formula every refresh of its scan (`fps 100`), every 2nd (`fps 50`, the default) or every 4th (`fps 25`); nothing in between.

Write `fps 25`, `fps 50` or `fps 100`.

Before:

```psi-bad
fps 60
gray(x)
```

After:

```psi
fps 50
gray(x)
```

<h3 id="E802">E802 · <code>fps</code> inside a layer</h3>

> `fps` is the whole formula's: put it on a line of its own, outside the layers

Move the `fps` line out of the layer.

<h3 id="E803">E803 · <code>fps</code> set twice</h3>

> `fps` is already set

Keep one `fps` line.

## Growing values, sprites, palettes, slow layers and declarations (E9xx)

<h3 id="E901">E901 · a value that grows without end</h3>

> `t` grows without end in `loop forever`, and this can't take it: use frac(t\*k), t % n, sin(t\*k), noise(t\*k, y, z), a comparison, min, max or smoothstep

In a `loop forever` formula `t` grows without end, and most arithmetic on it would lose its precision or overflow. Wrap it first: `frac`, `%`, `sin` and `noise` keep it in range.

Use frac(t\*k), t % n, sin(t\*k) or similar in its place.

Before:

```psi-bad
loop forever
gray(t*x)
```

After:

```psi
loop forever
gray(frac(t/4 + x))
```

<h3 id="E902">E902 · too many noises of growing values</h3>

> noise of values that grow without end: four different ones at most — share one, as noise(u + k, …)

Share one noise between the places that need it.

<h3 id="E903">E903 · <code>seed</code> outside <code>loop forever</code></h3>

> `seed` is for formulas that go on without repeating (`loop forever`): a new one each time it plays

Add `loop forever`, or leave `seed` out.

<h3 id="E904">E904 · too many growing values</h3>

> this formula has too many values that grow without end (more than …): share one, as frac(u\*k) of one `u`

Use fewer values that grow without end.

<h3 id="E905">E905 · a colour fed a growing value</h3>

> a colour can't take a value that grows without end and changes across the panel (`t` in a `loop forever` formula): use frac(…) of it, or min(t/k, 1) (or: …, as `v` (line …) does: use frac(v) or min(v/k, 1))

A colour has to be worked out from values that stay in range; `t` in a `loop forever` formula does not.

Wrap it in frac(…), or limit it with min(t/k, 1).

Before:

```psi-bad
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

<h3 id="E906">E906 · drawing at a growing value</h3>

> a value that grows without end can't be drawn at: use frac(t\*k)\*128, t % n or min(t, k)

Use frac(t\*k)\*128, t % n or min(t, k) as the place.

<h3 id="E907">E907 · a growing value with a per-pixel part</h3>

> a value that grows without end plus a part per pixel or row only goes into frac, sin, cos, tri, saw, sqr, noise, % or a multiply: add the pixel's part there, as frac(t\*k) + x or sin(t\*k + x)

Add the pixel's part inside frac, a wave or noise: frac(t\*k) + x, sin(t\*k + x).

<h3 id="E910">E910 · a sprite's key too long</h3>

> a sprite's key has 16 characters at most

Use at most 16 characters in the key.

<h3 id="E911">E911 · a sprite inside a layer</h3>

> a sprite goes at the top of the formula, not inside a layer

Move the sprite out of the layer.

<h3 id="E912">E912 · a sprite without its key</h3>

> a sprite starts with its key: `key . 0, R 1, D 2` (each character, then the number it reads as)

Start the sprite with a `key` line.

<h3 id="E913">E913 · a bad key entry</h3>

> a key entry is a character (not ; { } , - / or a space) and its number, as `R 1` or `. -0.5`, separated by commas

Write each entry as a character, a space and its number, with commas between.

<h3 id="E914">E914 · a character not in the key</h3>

> `…` is not in the sprite's key

Add the character to the key, or change the row.

<h3 id="E915">E915 · sprite rows of different widths</h3>

> a sprite's rows are as wide as its first (… characters), at most 64

Make every row as wide as the first, at most 64.

<h3 id="E916">E916 · a sprite with no rows or too many</h3>

> a sprite has 1 to 32 rows of characters, after its key

Use 1 to 32 rows.

<h3 id="E917">E917 · a sprite without <code>[u, v]</code></h3>

> `…` is a sprite: read its pixels, `…[u, v]` (column, row from its top left)

Read the sprite at a column and a row, `NAME[u, v]`.

<h3 id="E918">E918 · a sprite read wrongly</h3>

> a sprite is read at a column and a row: `…[u, v]`

Read a sprite with `NAME[u, v]`.

<h3 id="E919">E919 · a sprite never closed</h3>

> this `sprite {` is never closed — add `}` on a line of its own

Add a `}` on a line of its own.

<h3 id="E920">E920 · a character twice in the key</h3>

> `…` is in the key twice

Keep one entry for each character.

<h3 id="E921">E921 · a palette inside a layer</h3>

> a palette goes at the top of the formula, not inside a layer

Move the palette out of the layer.

<h3 id="E922">E922 · a palette of the wrong size</h3>

> a palette is 1 to 32 colours: `palette sea = #003, #06c, #fff` (pal(sea, 0) is the first, pal(sea, 1) the last)

Write 1 to 32 colours, each as #f80.

<h3 id="E923">E923 · too many palettes</h3>

> a formula can have … palettes of its own at most

Use fewer palettes (a palette in a tween's half takes half of one).

<h3 id="E924">E924 · too many numbers in tables and sprites</h3>

> tables and sprites hold 1024 numbers in all — this would make …

Make the tables and sprites smaller.

<h3 id="E925">E925 · a palette's colour named twice</h3>

> `…` already names a colour of `…`: give this one another name

Give each named colour of a palette its own name.

<h3 id="E926">E926 · a palette's colour named <code>n</code> or <code>step</code></h3>

> `n` and `step` are every palette's own (`….n`, `….step`): name this colour another way

Pick another name for the colour: `P.n` is the palette's count and `P.step` the step between colours.

<h3 id="E927">E927 · an unknown colour of a palette</h3>

> palette `…` has no colour named `…`: name it in its line, as `… #f80`

Name the colour on the palette's line (`palette road = black #000, kerb #806`), or use another of its names.

<h3 id="E931">E931 · <code>cycle</code> turned by a value that changes along a row</h3>

> `cycle` turns a whole row at once (by t, a frame value or py), but `…` (line …) is per pixel: it reads `…`

`cycle` turns a whole row's palette at once, so how far it turns can change with the time or the row, not along the row.

Turn it by t, a value of t, or py.

Before:

```psi-bad
cycle(fire, x, x)
```

After:

```psi
loop 4
cycle(fire, x, t/4 + y)
```

<h3 id="E932">E932 · <code>cycle</code> or <code>tween</code> of gray</h3>

> `cycle` turns a palette of colours, and gray is not one: write your own, `palette g = #000, #fff`

Write your own palette.

<h3 id="E933">E933 · too many palettes with <code>cycle</code></h3>

> a formula has room for … palettes that `pal` and `cycle` read: its own (… here) and each palette `cycle` turns by another value (… here)

Read fewer palettes with `pal` and `cycle` (tweens may read more), or turn fewer by different values.

<h3 id="E934">E934 · <code>cycle</code> without its turn</h3>

> expected `,` and the turn (as in `cycle(fire, v, t/4)`) here, found …

Write the turn after the picture: `cycle(palette, v, turn)`.

<h3 id="E935">E935 · the wrong values for <code>pal</code> or <code>cycle</code></h3>

> `…` takes a palette (and how much of the pixel it covers, 0 to 1, if you like); found …

Give it a palette and a value (`cycle` also takes the picture and the turn).

<h3 id="E936">E936 · a tween mixed by a value that changes across the panel</h3>

> `tween` mixes a palette once a frame (by t or a frame value), but `…` (line …) is per pixel: it reads `…`

Mix it by t, a value of t, a slider or another once-a-frame value.

<h3 id="E937">E937 · a palette's half of the wrong palettes</h3>

> a palette's half is a tween of palettes of your own with 16 colours at most: `…` is not one

In a half, tween palettes of your own of 16 colours or fewer (tweens of them inside are fine).

<h3 id="E938">E938 · a tween painted with</h3>

> `…` is a tween, worked out every frame: paint takes a palette of colours (`palette p = #000, #fff`)

Paint with a palette of colours, or with `pal(P, v)` of one of the tween's palettes.

<h3 id="E939">E939 · tweens nested too deeply</h3>

> the tweens need more than … palettes worked out at once: nest them less deeply, or tween fewer palettes that only tweens read

Nest tweens less deeply (a tween as the first palette costs nothing), or read the palettes they make with `pal`.

<h3 id="E941">E941 · <code>rate</code> above the bottom</h3>

> `rate` goes on the bottom layers only: the first layer, or the first few one after another

Put `rate` on the bottom layers only.

<h3 id="E942">E942 · a bad <code>rate</code></h3>

> `rate` takes a whole number of frames, 2 to …: `layer rate 4 { … }` is redrawn every 4th frame

Write a whole number from 2 to the limit.

<h3 id="E943">E943 · different <code>rate</code>s</h3>

> the slow layers share one `rate`: give each the same number

Give the slow layers the same `rate`.

<h3 id="E944">E944 · a <code>rate</code> layer reading the picture</h3>

> a `rate` layer is drawn on its own: it can't read `ink`, `ur`, `ug`, `ub` or `ul`

Do not read `ink`, `ur`, `ug`, `ub` or `ul` in a `rate` formula.

<h3 id="E946">E946 · <code>rate</code> before the blend</h3>

> write the blend first: `layer add rate 4 { … }`

Write the blend, then `rate`.

<h3 id="E947">E947 · only <code>rate</code> layers</h3>

> a `rate` layer is the slow one under a layer that moves: add a layer above it, or leave `rate` out

Add a layer above, or leave `rate` out.

<h3 id="E948">E948 · a bottom <code>rate</code> layer that does not cover</h3>

> the bottom `rate` layer draws the whole panel: make it cover everything (`layer rate N { … }` with no blend and no coverage)

Leave the blend and the coverage off the bottom `rate` layer.

<h3 id="E951">E951 · a declaration below other lines</h3>

> `…` goes at the top of the formula, before its other lines

Declarations (`name`, `about`, `control`, `look`) say what the formula is, so they come first, before anything it works out.

Move it to the top.

Before:

```psi-bad
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

<h3 id="E952">E952 · a declaration twice</h3>

> `…` is already set on line …

Keep one line.

<h3 id="E953">E953 · text in quotes not closed</h3>

> text in quotes ends with `"` on the same line

Close the quotes on the same line.

<h3 id="E954">E954 · a character that can't be in quotes</h3>

> text in quotes can't have `<`, `>`, `\` or a control character (a tab too) in it

Take it out.

<h3 id="E955">E955 · text too long</h3>

> too long: this text can be … characters at most

Shorten the text.

<h3 id="E956">E956 · text missing</h3>

> this takes its text in quotes, e.g. `name "Sunrise"`

Write the text in quotes.

<h3 id="E957">E957 · a bad <code>control</code> line</h3>

> a control is `control NAME lo..hi = default "Label"` or `control NAME = word, word, …`

A control is a slider (`control NAME lo..hi = default "Label"`) or a row of buttons (`control NAME = word, word`).

Write it as shown.

Before:

```psi-bad
control k 1 = 2
gray(k)
```

After:

```psi
control k 0..2 = 1 "Level"
gray(k / 2)
```

<h3 id="E958">E958 · too many controls</h3>

> a formula has three controls at most (a, b and c)

Keep three controls at most.

<h3 id="E959">E959 · a range that does not go up</h3>

> a range goes from the lower number to the higher, e.g. `0..1`

Write the lower number first.

<h3 id="E960">E960 · a default outside the range</h3>

> the default is outside the control's range

Choose a default inside the range.

<h3 id="E961">E961 · a choice of the wrong size</h3>

> a choice has 2 to 6 words

Give 2 to 6 words.

<h3 id="E962">E962 · a word twice in a choice</h3>

> `…` is a choice already

Use each word once.

<h3 id="E963">E963 · a control out of its slot</h3>

> `…` is the … control's letter: give this one another name (controls take a, b, c in order)

Name it something else, or put it in its slot.

<h3 id="E964">E964 · a bad <code>look</code> line</h3>

> `look` takes a blend (over, screen, add, multiply, max or min) and an amount from 0 to 100

Write `look`, a blend and an amount from 0 to 100.

<h3 id="E965">E965 · a blend on a formula with layers</h3>

> a formula with layers goes over the GIF as a look: write `look` or `look over N`

Write `look` or `look over N`.

<h3 id="E966">E966 · a choice word that is not a name</h3>

> a choice is words (names) between commas: `control NAME = word, word, …`

Write names between commas.

<h3 id="E967">E967 · more than a declaration takes</h3>

> expected the end of the `…` line, found … (one declaration a line)

Put one declaration on a line.

<h3 id="E968">E968 · an unknown blend in a look</h3>

> a look's blend is one of over, screen, add, multiply, max, min

Use one of the listed blends.

<h3 id="E969">E969 · text that hides or reorders</h3>

> text in quotes can't have characters that hide or reorder text (zero-width, direction marks, line separators)

Take them out.

<h3 id="E970">E970 · a scene's <code>when</code> that changes along a row</h3>

> a scene's `when` is worked out once a frame or once a row (by t, a frame value or py), but `…` (line …) is per pixel: it reads `…`

Make `when` a value of t, the clock, the sliders or the row (py, y); choose per pixel inside the scene instead.

<h3 id="E971">E971 · something other than values in a scene</h3>

> a scene holds values: `name = …` lines (its colours, drawing and palettes go outside it)

Keep `name = …` lines in the scene; put colours, `draw`, `layer`, `palette` and another scene outside it.

<h3 id="E972">E972 · too many scenes</h3>

> a formula can have … scenes at most

Join scenes that share a condition, or pick inside one.

<h3 id="E973">E973 · a <code>rate</code> layer reading a scene</h3>

> a `rate` layer can't read a scene's per-pixel values: work them out outside the scene, or leave `rate` out

Work the value out outside the scene, or leave `rate` out.

<h3 id="E974">E974 · a table too long</h3>

> a table holds 256 numbers at most — this one has …: split it in two

Split the table in two and read the second half from the other.

<h3 id="E975">E975 · a built-in input that doesn't exist</h3>

> `….…` is not a built-in input: `…` has …

Use one of the names listed, as `sun.alt` or `replay.game`.

<h3 id="E976">E976 · <code>since</code> or <code>was</code> of something that isn't a built-in input</h3>

> `…(…)` takes a built-in input's name, as in `…(replay.game)`: …

Give it one of the built-in inputs listed (`since` and `was` follow the panel's own values, not the formula's).

<h3 id="E977">E977 · too many built-in inputs</h3>

> a formula can read … built-in inputs at most (each `since` and `was` one more) — this one reads …

Read fewer built-in inputs, or fewer `since` and `was` of them.

<!-- psi-errors end -->
