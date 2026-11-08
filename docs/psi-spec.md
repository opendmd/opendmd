# Psi language reference

**Psi** is the language OpenDMD's **Formulas** are written in: a few lines
of arithmetic, worked out for every pixel of the 128 × 32 panel, every
frame, ending in a colour. This page is the reference: every name the
language knows and what it does, how a formula runs, its limits, some
recipes, and then the language in full. The error codes have
[a page of their own](psi-errors.md). To learn it, start with
[Make a Formula](guide/psi-1.md) in the guide; to try it, open the
[demo](/demo/)'s Formulas.

The tables of names and the limits on this page are generated from the
compiler's own tables, and the tests fail when they drift, so what this
page says is what the panel does. On the site, the search box filters the
built-ins (press `/`), and **Cheat sheet** folds the page down to the
tables.

## Every name

<!-- psi-builtins: names (generated from opendmd-psi/src/builtins.rs: PSI_BLESS=1 cargo test --features docs --test builtins) -->
| kind | names |
| --- | --- |
| values | `x`, `y`, `px`, `py`, `i`, `t`, `hour`, `minute`, `second`, `a`, `b`, `c`, `ur`, `ug`, `ub`, `ul`, `ink`, `seed` |
| values about the middle | `cx`, `cy`, `r`, `angle` |
| built-in inputs (the panel's own values) | `daylight`, `sun.alt`, `moon.phase`, `season`, `yearday`, `night`, `bright`, `replay.playing`, `replay.platform`, `replay.game` |
| their events | `since(KEY)`, `was(KEY)` |
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
| words of the draw stage and scenes (names elsewhere) | `if`, `else`, `when`, `scene`, `tween` |
| the picture under | `under`, `under(x, y)` |
| the built-in inputs' groups (never a name) | `sun`, `moon`, `replay` |

**The name rule.** A formula's own names win. Any built-in value's name, and any built-in function's or drawing call's, may be set as a value of the formula's (`t = 0.5`, `sin = 0.25`, `line = 3`): from the line that sets it, the name reads as the formula's value wherever a value is read, and a call `name(…)` is still the built-in function (`since`, `was`, `under` and `ink(n)` as `sin`). Not names: the keywords (E204), the palettes (E208) and `sun`, `moon` and `replay` (E216). A `fn`, table, sprite or palette of the formula's takes a name of its own: no built-in function's (E215) and no value's it has set (E203). `since(k)` and `was(k)` read the built-in inputs only: of a name the formula set they are refused (E217), never read two ways on one line. A built-in added later is read only where a formula has not set its name, so a formula saved before it means what it meant.
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
| `daylight` | how light it is outside where the panel is, 0 at night to 1 by day (twilight between); the panel's location in Settings, else 1 | <code class="psi-ex">mix(&#35;081030, &#35;70b0ff, daylight) &#42; (1 - y&#42;0.5)</code> |
| `sun.alt` | the sun's height in turns: 0 on the horizon, 0.25 straight up, below 0 at night; 0.1 without a location | <code class="psi-ex">s = 16 - sun.alt&#42;120; mix(&#35;102040, &#35;ffd060, len(px - 64, py - s) &lt; 4)</code> |
| `moon.phase, season, yearday` | the moon (0 new, 0.5 full), the time of year (0 midwinter, 0.5 midsummer) and the day of the year (0 on 1 January) | <code class="psi-ex">hsv(season + 0.3, 0.6, (abs(px - 64) &lt; 6) &#42; (abs(py - 16) &lt; 6) &#42; step(x, moon.phase + 0.4))</code> |
| `night, bright` | 1 while night mode dims the panel; the panel's brightness, 0 to 1 | <code class="psi-ex">gray(mix(1, 0.3, night) &#42; y)</code> |
| `replay.playing, replay.platform, replay.game` | RePlayOS: 1 while a game plays; its platform as a number (0 none); a number 0 to 1 for the game, the same each time it plays | <code class="psi-ex">hsv(replay.game + x&#42;0.2, replay.playing, 0.3 + 0.7&#42;y)</code> |

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
| `since(replay.game)` | seconds since a built-in input last changed (32767 when it never has): a flash is max(0, 1 - since(replay.game)) | <code class="psi-ex">k = max(0, 1 - since(replay.game)); hsv(replay.game, 1 - k, max(k, y))</code> |
| `was(replay.game)` | a built-in input's value before its last change: mix(was(k), k, min(1, since(k))) fades from the old to the new | <code class="psi-ex">hsv(mix(was(replay.game), replay.game, min(1, since(replay.game))), 1, y)</code> |
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
| `table T = 1, -2, 0.5` | numbers, 256 a table and 1024 in all (tables and sprites) | <code class="psi-ex">table T = 0.2, 0.6, 1, 0.6; gray(T&#91;floor(x&#42;4)&#93;)</code> |
| `T[k]` | number k of table T, from 0 (0 outside it) | <code class="psi-ex">table hue = 0, 0.1, 0.3, 0.6; hsv(hue&#91;px % 4&#93;, 1, 1)</code> |
| `sprite s { … }` | a small picture in characters: first its key, key . 0, X 1 (each character and the number it reads as, up to 16), then rows up to 64 wide and 32 tall | <code class="psi-ex">sprite s {<br>  key . 0, X 1<br>  .X.<br>  XXX<br>  .X.<br>}<br>gray(s&#91;floor(px/4) - 14, floor(py/4) - 2&#93;)</code> |
| `s[u, v]` | sprite s at column u, row v from its top left: its key’s number (0 outside) | <code class="psi-ex">sprite heart {<br>  key . 0, X 1<br>  .X.X.<br>  XXXXX<br>  .XXX.<br>  ..X..<br>}<br>rgb(1, 0.2, 0.3) &#42; heart&#91;px % 6, py % 5&#93;</code> |

#### Colours

| write | what it does | example |
| --- | --- | --- |
| `hsv(hue, sat, val)` | colour: hue in turns, 0 to 1 each | <code class="psi-ex">hsv(x, 1, y)</code> |
| `rgb(r, g, b)` | red, green, blue: 0 to 1 | <code class="psi-ex">rgb(x, y, 1 - x)</code> |
| `pal(fire, v)` | a palette: fire, ice, ocean, chrono, amber, rainbow, gray | <code class="psi-ex">pal(fire, y)</code> |
| `palette sea = #024, #08c, #fff` | your own palette, up to 32 colours (8 palettes; pal and cycle read 3): pal(sea, 0) the first, pal(sea, 1) the last, pal(sea, 0.5) the middle one | <code class="psi-ex">palette sea = &#35;024, &#35;08c, &#35;fff; pal(sea, y)</code> |
| `scene s when c { v = … }` | values worked out only while c (once a frame or a row) is not 0; after it its names are 0 while it is off; the estimate counts two scenes at once | <code class="psi-ex">loop 4; on = frac(t/4) &gt; 0.5; scene s when on { w = sin(x&#42;9 + t&#42;3)&#42;0.5 + 0.5 }; gray(w)</code> |
| `palette sky = tween(day, night, w)` | a palette mixed once a frame, day towards night by w (0 to 1, of t or sliders); tweens nest; tween(a, b, w), tween(c, d, v) fills the low and high 16 colours | <code class="psi-ex">loop 8; palette day = &#35;48c, &#35;adf, &#35;ffc; palette night = &#35;002, &#35;124, &#35;346; palette sky = tween(day, night, 0.5 + 0.5&#42;sin(t&#42;0.785)); pal(sky, y)</code> |
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
- **The panel's own values** (`daylight`, `sun.alt`, `moon.phase`,
  `season`, `yearday`, `night`, `bright`, `replay.playing`,
  `replay.platform`, `replay.game`) are read by name and are the same for
  every pixel of a frame, each held to its range, with a default when the
  panel has no value for it (previews and the gallery always draw at the
  defaults). `since(KEY)` and `was(KEY)` say when one last changed and what
  it was before ("Numbers" has their ends): a formula stays a function of
  its inputs, with no memory of its own.

## Limits

<!-- psi-builtins: limits (generated from opendmd-psi/src/builtins.rs: PSI_BLESS=1 cargo test --features docs --test builtins) -->
| limit | at most |
| --- | --- |
| a formula's text, comments included | 32,768 characters |
| steps (per frame, draw, per row, per pixel, and one per layer) | 768 |
| values held a pixel at once | 12 |
| constants and per-frame and per-row values held at once | 241 |
| nesting of brackets, operators and function calls | 48 |
| `table` and `sprite` numbers, in all | 1,024 |
| numbers in one `table` | 256 |
| built-in inputs read (each `since` and `was` one more) | 24 |
| palettes of your own (of colours) | 8 |
| palettes `pal` and `cycle` read | 3 |
| loop turns a frame in `draw { }` | 2,048 |
| lines and dots a frame | 2,048 |
| line steps a frame | 16,384 |
| `layer rate N` | N from 2 to 200 |
| `name` / `about` / a control's label | 60 / 120 / 24 characters |
<!-- psi-builtins end -->

How heavy a formula is for the panel is an estimate in milliseconds a frame:
**light** is 5 ms or less, **medium** 8 ms or less, past that **heavy**. The
panel refuses a formula past 40 ms a frame (12 ms for one drawn over every
GIF as a look); the gallery takes one of 10 ms or less.

<a id="cookbook"></a>

## Recipes

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
wave = sin(x*2) + sin((1 - y) * 1.5) + sin(x*1.5 + (1 - y))
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

## The language in full

For implementers, tools and the gallery's checker: the version promise,
the text and grammar, the numbers, what stays the same from version to
version, how a formula travels, and the `psi` command.

### Versions

This page describes **Psi 1**. A formula may say which version it is
written for with `psi 1` on its first line; it never needs to, because a
formula without the line is version 1. (The editor does not write the
line: it will once there is a version 2.)

**The promise starts at OpenDMD 1.0.0.** Psi 1 is the language as OpenDMD
1.0.0 ships it; before that, through the betas, it changed freely (`y`
turned to count down, `part` and `frame { }` went, `/` became exact at
every level) without a new version. From 1.0.0:

- Within version 1, new built-in names may be added, under the name rule
  ("Every name"): a name a formula sets is that formula's own, even where
  a later version made it a built-in, so a formula saved before a name
  existed means what it meant.
- Anything that would change the pixels of a formula that compiled under
  version 1, or refuse one, is a new version ("What stays the same" lists
  what that covers). The tests hold every example and gallery formula to
  the same frames.
- **The compiled form is another axis.** A compiled Formula (`.psb`) carries
  a program format number, the *ABI* (3 at 1.0.0): the steps the player
  runs and what each means. It is not the language: it may change
  at any release, and a panel that meets another ABI's bytes says so and the
  app prepares the Formula again from its text. A new ABI is never a new
  Psi version, and a new Psi version need not be a new ABI.
- A formula written for a later version than the panel has (`psi 2` on a
  version 1 panel) is refused with a clear sentence (`E601`) rather than
  failing somewhere in the middle.

### Text

- A formula is text, at most 32,768 characters, comments included.
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

### Grammar

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
          | "palette" NAME "=" [ NAME ] COLOUR { "," [ NAME ] COLOUR }   -- 1..32 colours (section 6)
          | "palette" NAME "=" tween [ "," tween ]   -- mixed once a frame; two: the halves (section 6)
          | "scene" NAME "when" e "{" { NAME "=" expr sep } "}"   -- e once a frame or a row; its names 0 while e is 0 (section 13)
tween     = "tween" "(" psrc "," psrc "," e ")"   -- e once a frame, held to 0..1
psrc      = PALETTE | tween                    -- yours, a built-in (not gray), or another tween
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
cmp       = sum [ ("<" | "<=" | ">" | ">=" | "==" | "!=") sum ]   -- one: E218
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

### Numbers

Every value is a fixed-point number, from −32,768 to 32,767.99998 in steps
of 1/65,536 (Q16.16). Arithmetic saturates rather than wrapping. `a / b` is
the exact quotient truncated towards zero to a step (`-1/65536 / 2` is 0),
held to the ends; `a / 0` is the largest number of `a`'s sign (0 for
`0 / 0`); `a % 0` is 0; `%` is floored (`-1 % 3` is 2). Comparisons and
`and`, `or`, `not` give 1 or 0; any number but 0 is true, and comparisons
don't chain: `0 < x < 0.5` is refused (`E218`: write `0 < x and x < 0.5`).
Both sides of `? :` are worked out. A division per pixel by a constant or
a frame's or row's value gives the same bits as the same division once a
frame (the panel divides a row of pixels by one divisor with multiplies,
exactly).

`since(KEY)` is the seconds since a built-in input last changed, held at
32766 (about 9 hours), and 32767 when it never has, so `since(k) < 32767`
means "it has changed"; `was(KEY)` is its value before that change. A
built-in input with no value on the panel (no location set, the clock not
set, RePlayOS not followed) reads its default.

Where a function is defined by a table (`sin`, `noise`, `hash`, `atan2`,
the palettes, the fonts) the table is the definition. The normative
statement is that **a formula gives the same bytes on the panel, in the
browser and on a computer**; golden frames in the tests hold every platform
to it.

### What stays the same

From OpenDMD 1.0.0 everything below is observable in a formula's pixels or
in whether it compiles, so changing any of it is a Psi 2 (see "Versions").
Machinery that gives the same bytes is not on the list and may change at
any release: the `.psb` ABI, the steps and their narrowed and fused forms,
the JIT, keeping values that never change from frame to frame, the
column run, the cost model and its tiers
(a formula refused as too heavy is a product limit, not the language),
and the error sentences (the codes are only ever added; a retired code is
never used again).

- **Syntax and acceptance.** The keywords, the palette names and the
  dotted inputs' groups (`sun`, `moon`, `replay`) are not names; the name
  rule ("Every name"); declarations first; `psi N` on the first line only;
  `{` on the line that opens it; `;` as a separator; `--` and `//`
  comments; a line going on after an operator or `,`; literal counts for
  `for`, `ink(n)` and `look`; comparisons don't chain; the colour grammar
  (what is a channel expression, what is a whole colour, `pal(P, k) * e` the
  one scaled whole colour, coverage on the outermost colour only). Grammar
  that is refused today may be accepted later (`else if`).
- **Limits only grow**: a formula that fits keeps fitting (the table
  below).
- **Numbers.** Q16.16 with `+ - *`, negation and `abs` saturating; `*`
  rounding down; `/` truncating towards zero, exact at every level; `/ 0`,
  `0 / 0`, `% 0`; floored `%`; `floor` and `frac` of negatives; `sqrt` of 0
  and below is 0; `abs` of the lowest number is the highest.
- **Tables are definitions**: `sin`/`cos` (256 entries, interpolated),
  `sqrt`, `atan2`, `noise`, `hash`, the three fonts, the hsv conversion,
  `ul`'s weights, the blends and coverage in 8-bit channels, `cycle`'s
  turn, tweens' rounding, and the palettes as they are:
  - `pal(P, k)` reads one of the palette's **32 entries, the nearest** to
    `k`, with no blending between them (so a three-colour palette bands
    rather than ramps; an interpolated read would be a new name);
  - `rainbow` is **black at 0** and dark at 1, its hues between;
  - `ocean` is **the same table as `ice`** (one palette, two names).
- **Inputs.** `x` and `y` across and down, `cx cy r angle` as defined,
  `angle` clockwise with a quarter down, `t` a whole number of frames,
  `loop` 8 and `fps` 50 when left out (and 25, 50 or 100 only), the
  controls' ranges and defaults, each built-in input's range and default,
  `since`'s 32766 for a change and 32767 for never, `hour minute second`
  whole.
- **Reads outside their range give 0**: `pick` past its choices, `T[k]`
  past a table, `S[u, v]` outside a sprite, `digit` and `glyph` outside the
  cell.
- **The draw stage**: ink keeps the brighter value, paint is painter's
  order over the colour line, `line` and `dot` with no ink draw 1, the caps
  skip the rest the same way on every panel.
- **Scenes, rates and layers**: a scene's names read 0 while it is off, the
  `rate` bands' schedule, layers blended bottom to top, a look ignored
  when the formula plays alone; `loop forever`'s growing `t`, its `seed`
  per play, and the growing-value forms it accepts (more may be accepted
  later, never fewer).

### How a formula travels

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
  author: …` and `-- licence: CC0-1.0`) and then the formula: at most
  32,768 characters, 4,500 bytes once compiled, 36 KB for the file.

### Tools

The `psi` command checks and formats formulas on a computer
(`cargo install --path opendmd-psi-cli` from the source, or `make psi`):

```text
$ psi check storm-front.psi --out -
ok: 41 of 768 steps (per frame 11, draw 0, per row 5, per pixel 25), 2 layers, 3 of 12 registers, 38 of 241 values
name: Storm front
never repeats (loop forever) at 50 fps; reads t
panel: ~6.8 ms a frame: medium (light ≤ 5 ms, medium ≤ 8 ms; GIF looks take ≤ 12 ms)
JIT: yes, ~5.6 ms a frame compiled (the VM's ~6.8 ms is what the limits count)
length: 559 B as written (32,768 at most), 360 B compiled (493 B as a path, @formula?psb=…: 6,144 to show it, 2,048 for an inline look)
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
(`--out`, `--frames`, `--a`/`--b`/`--c`, `--clock`, `--set KEY=V`, `--since KEY=S` and `--was KEY=V` for the built-in inputs, `--hash`, `--listing`).
It says whether the panel's JIT takes the formula (and why not), and it
prints the compiler's **notes**, each with its line: a `pick` worked out
per pixel because one arm is, a sum or product that brackets better
(`px - 64 - X` works `- X` out for every pixel; `px - (64 + X)` once a
frame), which values are worked out once and kept and which once a
column, the heaviest value that could not be and the value that kept it out, and a draw loop run as
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
