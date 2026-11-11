# How Psi thinks

The three Make a Formula pages show what to type. This page explains the ideas underneath, one at a time. Every name and function is listed in [the language reference](../psi-spec.md).

## A Formula draws a whole frame

A Formula is a short program for one frame of the panel. Its last line is the colour. The display works that colour out at all 4,096 pixels, 128 across and 32 down, and then does the whole frame again for the next one.

The lines above the colour set up values for it to use. The colour comes last, and only the last line can be the colour.

```psi-bad
name "Oops"
gray(x)
v = 1
```

Here the editor says "the colour has to be the last line". The last line can also be a plain number, as in `x` alone. Then it is drawn as a grey.

## Once, once a frame, once a row, every pixel

The lines of a Formula don't all run as often as each other. Psi looks at what each value reads, and works it out only as often as that changes:

- a value made only of numbers costs next to nothing;
- one that reads the time, the clock or a control is worked out **once a frame**;
- one that reads `y` or `py`, the row, is worked out **once a row**, 32 times a frame;
- one that reads `x`, `px` or the picture under the Formula is worked out **for every pixel**, 4,096 times a frame.

A value that uses another value moves up to that one's level. The comments here say where each line lands.

```psi
name "Levels"
loop 4
-- once a frame: reads only t
glow = 0.5 + 0.5*sin(t/4)
-- once a row: reads y
band = floor(y*4)/4
-- every pixel: reads x
hsv(band + x/4, 1, glow)
```

Almost all of a Formula's work is the work it does for every pixel, so this is the idea that matters most. The section on [what costs](#what-costs) comes back to it.

## Turns, not degrees

`sin`, `cos`, `tri`, `saw` and `sqr` go round once as their value goes from 0 to 1. The hue in `hsv` works the same way, and so do `atan2` and `angle`, which give directions in turns. So `sin(0.25)` is 1, and `sin(x)` is one wave across the panel. Writing 6.28 to make a full circle gives six of them instead.

Turns also make loops join up. In a `loop 4` Formula, `t/4` goes from 0 to 1 over the loop. A wave or a hue driven by it ends exactly where it began.

```psi
name "Turns"
loop 4
gray(sin(x + t/4)/2 + 0.5)
```

## Numbers and colours

Psi's numbers run from about −32,768 to 32,767. A result past either end stays at the end rather than wrapping round. Dividing by zero gives no error. A colour channel takes 0 to 1, and anything outside that is held at the nearest end.

A colour such as `#f80` is not a number, so it can't be stored in a name.

```psi-bad
name "Oops"
v = #f80
gray(v)
```

It belongs on the colour line, where it can be dimmed (`#f80 * k`) or mixed (`mix(#000, #f80, k)`).

A palette steps from one colour to the next. It does not blend between them. A palette you write yourself with three colours gives three bands:

```psi
name "Three bands"
palette sea = #024, #08c, #fff
pal(sea, y)
```

For a smooth fade, mix the colours instead:

```psi
name "Smooth"
mix(#024, #fff, y)
```

The built-in palettes have 32 steps each, which is close to smooth. A `table` is a list of numbers, read from 0.

```psi
name "Cells"
table level = 0.2, 0.6, 1, 0.6
gray(level[floor(x*4)])
```

## Time and loops

`t` counts seconds from the start of the loop, and starts again from 0 when the loop ends. The loop is 8 seconds unless a `loop` line near the top says otherwise. `phase` is where the frame is in the loop, from 0 to 1: exactly `t` divided by the loop's length, so `sin(phase)` goes round once a loop whatever its length. In `loop forever`, `t` goes on growing, and Psi asks you to wrap it with `frac`, `%`, `sin`, `noise` or a comparison before it reaches a colour. `hour`, `minute` and `second` come from the display's clock.

`t` is the time of the frame being drawn. Which moments get drawn depends on the display, so a Formula says where things are at a time, never how far they move in a frame. The loop plays as whole frames: a loop that isn't a whole number of 0.04 seconds can come out up to 0.04 seconds short. The loop is also the least time a Formula plays for in a playlist, and the length of a GIF saved from it.

## How often it is drawn

The display draws its picture each time its scan comes round, a *refresh*: about 100 to 125 times a second, depending on the panel. A Formula is drawn every 2nd refresh, or every refresh with a `smooth` line at the top (Formulas written before it say `fps 100`, which means the same). The display measures how long each frame takes. When frames take too long, it draws every 4th or 8th refresh instead, and when they get faster again it speeds back up, never past what the Formula asked for. Every refresh only holds for a light Formula, and only with **Phosphor glow** off. The browser's preview draws at about the same moments.

## Layers

A Formula can have several colour lines, each in its own `layer { … }`. Each layer has its own names and ends in its own colour. The first is at the bottom.

A layer can name how it goes onto what is under it: `over` (the default), `add`, `multiply`, `screen`, `max` or `min`. A colour may also take a last value, how much of the pixel it covers, from 0 to 1. Once a Formula has a layer, every colour line has to be inside one.

If the bottom layer doesn't cover everything, the picture under the Formula shows through. That picture is the GIF it is drawn over, or black when the Formula plays alone. On this page, and in the editor until a GIF is picked, it is a sample picture of sky, sun and hills. A multiply layer on its own darkens what is under it:

```psi
name "Rows"
layer multiply {
  gray(py % 2 ? 0.4 : 1)
}
```

A layer that is `over` and covers every pixel hides everything below it. Psi leaves the hidden layers out, so they cost nothing.

`cycle(P, v, s)` reads palette `P` at `v`, turned round by `s`. It is the old palette-cycling trick. The turn can change with the time or the row, but not along a row.

```psi
name "Copper"
loop 6
palette sunset = #102, #315, #a24, #f62, #fb4, #fe8, #fb4, #f62, #a24, #315
cycle(sunset, 0, t/6 + py/40)
```

`layer rate N { … }` makes a slow bottom layer. It is redrawn a band of rows at a time, so every row is redrawn every Nth frame. The layers above it are drawn every frame.

```psi
name "Slow clouds"
loop 4
layer rate 4 {
  gray(noise(x*6, y*2, t))
}
layer {
  rgb(1, 0.5, 0, abs(x - frac(t/4)) < 0.01)
}
```

## Drawing

A `draw { … }` block runs once a frame, before any pixel. It can set names, repeat with `for k < N { … }` (two deep at most), choose with `if c { … } else { … }`, and draw. Because it runs before the pixels, it can't read `x`, `y` or anything that changes from pixel to pixel. Positions in it are pixels from the top left.

Drawing comes in two kinds:

- **Ink.** `line` and `dot` with a brightness put marks into the ink. The colour line reads it as `ink`, from 0 to 1, and gives it colour.
- **Paint.** `cls`, `pset`, `rect`, `rectfill`, `circ`, `circfill`, `trifill`, `spr` and `print`, and `line` and `dot` with a colour, paint in colour directly. Paint lies over every layer, and the last thing painted wins. A paint colour is a `#…` colour, `pal(…)` or `gray(…)`.

```psi
name "Hello"
draw {
  cls(#102)
  print("HELLO", 4, 4, #ff0)
  print(minute, 90, 20, #0ff, small, 2)
}
```

## Functions

`fn` makes a function: a value worked out from the values you give it.

```psi
name "Ring"
loop 4
fn ring(cx, r) = abs(len(px - cx, py - 15.5) - r) < 1
gray(ring(64 + sin(t/4)*30, 8) + ring(64, 12))
```

Inside the function, `cx` and `r` are the values it was called with. A function written with `{ … }` instead of `= …` holds several lines. If those lines draw, you call the function on a line of its own inside `draw`. If they end in a colour, the function can be the colour line.

When a function's values are called `x` and `y`, the built-in positions inside it follow them. So it can be drawn anywhere, stretched or mirrored:

```psi
name "Mirror"
loop 8
fn p(x, y) {
  wave = sin(x*2 + t/8) + sin(y*1.5 - t/4)
  hsv(wave/4 + t/8, 0.9, r < 0.6)
}
p(abs(x - 0.5)*2, y)
```

Psi copies a function's lines in wherever it is called, so each call adds its own work. A function can't call itself.

In the editor, a line `use Plasma as p` puts in one of your own Formulas, or one that comes with the web app, as a function `p(x, y)`. It works with Formulas that have one colour line and no layers, drawing, tables, sprites, palettes or functions of their own.

## Controls

A `control` line at the top gives the Formula a slider or a row of buttons:

- `control size 0..1 = 0.5 "Size"` is a slider: its name, its range, where it starts and its label;
- `control mode = calm, wild` is a row of buttons, and `mode` reads 0 for the first, 1 for the next.

A Formula has three controls at most. They also answer to `a`, `b` and `c`, in the order they are written. `a` read without a `control` line is a slider from 0 to 1 that starts at 0, so it reads 0 on the display unless a link sets it. The examples on these pages play at the controls' starting values.

## Over a GIF

A Formula can draw over the GIF that is playing. `ur`, `ug` and `ub` read the red, green and blue of the picture under the Formula at this pixel, from 0 to 1, and `ul` reads how bright it is. `under(px + 2, py)` reads another pixel, the nearest one on the panel. When a Formula plays on its own there is no picture under it, and all of these read 0. On these pages, and in the editor until you pick a GIF, they read a sample picture of sky, sun and hills.

```psi
name "Sepia"
look
mix(rgb(ur, ug, ub), rgb(ul*1.1 + 0.08, ul*0.92 + 0.04, ul*0.65), 0.8)
```

The `look` line says the Formula can be the panel's look, drawn over every GIF. Its card in the web app then offers **Set as the panel’s look**. `look screen 40` would put it on with the screen blend, covering 40 %. A Formula with layers takes `look` or `look over N` only.

The display won't use a look that hides the GIF completely. Over a GIF, a look is drawn with the GIF's frames, not at the rate its `smooth` line asks, and the limit on how heavy it can be is stricter than for a Formula playing alone.

## The world around it

A Formula can read a few things the display knows:

- `daylight` is 0 at night and 1 by day. It needs the display's location in Settings, and reads 1 without it.
- `night` is 1 while night mode has the panel dimmed.
- `bright` is the panel's brightness.
- `replay.playing` and `replay.game` follow RePlayOS.

```psi
name "Dusk"
mix(#081030, #70b0ff, daylight) * (1 - y*0.5)
```

## What costs

The editor rates a Formula under its picture as "light for the panel" (under 5 ms a frame), "medium for the panel" (under 8 ms) or "heavy for the panel". Light Formulas can be drawn every refresh, medium ones every 2nd, and a heavy one less often. A Formula that is far too heavy isn't played at all, and one with more than 768 steps doesn't compile.

Nearly all the cost is in the work done for every pixel. A step there runs 4,096 times a frame, a step once a row runs 32 times, and a step once a frame runs once. So the biggest saving is to move work out of the pixel. Work out what you can from `t` alone, or from `y` alone, and give it a name. The editor's warning names the costliest steps: "noise, len, sqrt, atan2 and dividing by a changing value".

`c ? a : b` picks one of two values, but every pixel still works out both. To skip work while something is off, use a `scene`. Its lines are only worked out while its condition is not 0, and its names read 0 while it is off.

```psi
name "Blink"
loop 4
on = frac(t/4) > 0.5
scene s when on {
  w = sin(x*9 + t*3)*0.5 + 0.5
}
gray(w)
```

The condition of a `scene` has to be worked out once a frame or once a row, never per pixel.
