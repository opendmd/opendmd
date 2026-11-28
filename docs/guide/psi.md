# How a Formula is run

This page describes what the display does with a Formula's text: what it works out for each pixel and how often, how time is measured, how layers and drawing fit together, and what makes a Formula slow. It follows on from the three [Make a Formula](psi-1.md) pages. Every name and function is listed in [the language reference](../psi-spec.md).

## One pixel at a time

A Formula says what colour one pixel is. The display runs it for each of the panel's 4,096 pixels, 128 across and 32 down, and that makes one frame. Then it runs it again for the next frame. Nothing is carried over from one frame to the next.

A Formula gives different pixels different colours by reading where the pixel is (`x`, `y`, `px`, `py`), and gives different frames different pictures by reading the time (`t`).

The pixel's colour comes from the colour line: a line that is a colour, such as `hsv(x, 1, 1)` or `#f80 * y`, with no `name =` in front. It has to be the last line. The lines above it give names to values for it to use.

```psi-bad
name "Oops"
gray(x)
v = 1
```

Here the editor says "the colour has to be the last line (move this to the end)". If the last line is a number, as in `x` alone, the pixel is that shade of grey.

Two kinds of Formula are laid out differently. One with [layers](#layers) has a colour line at the end of each layer. One that only paints in a [`draw` block](#drawing) needs no colour line.

## How often each line is worked out

The display does not work out every line for every pixel. It looks at what each value reads and works it out only as often as that can change:

- a value made only of numbers is worked out once, when the Formula is compiled;
- a value that reads the time, the clock or a control is worked out once a frame;
- a value that reads `y`, `py` or `cy` is worked out once a row, 32 times a frame;
- a value that reads `x`, `px` or the picture under the Formula is worked out for every pixel, 4,096 times a frame.

A value that uses another value is worked out as often as that one. The comments here say how often each line runs.

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

A value that reads the pixel's position but nothing that changes with time, such as `sin(x*3) + sin(y*2)`, is the same in every frame. The display works it out once for each pixel and keeps the results, as long as it has the memory to.

The picture is the same as if every line ran for every pixel. Only the time a frame takes changes, and [What makes a Formula heavy](#what-makes-a-formula-heavy) uses that.

## Angles are in turns

`sin`, `cos`, `tri`, `saw` and `sqr` repeat each time their value goes up by 1. A full circle is 1, not 360 and not 6.28. The hue in `hsv` is measured the same way, and `atan2` and `angle` give directions in turns. So `sin(0.25)` is 1, and `sin(x)` is one wave across the panel. `sin(x*6.28)` is six waves and a bit.

In a `loop 4` Formula, `t/4` goes from 0 to 1 over the loop. A wave or a hue that reads it is back where it began when the loop ends, so the loop has no jump.

```psi
name "Turns"
loop 4
gray(sin(x + t/4)/2 + 0.5)
```

## Numbers

A number in Psi is between −32,768 and 32,767.99998, in steps of 1/65,536 (about 0.000015). A result past either end is held at that end. It does not wrap round to the other.

Dividing by zero is not an error. The result is the largest number, or the most negative one when the number divided was negative, and `0/0` is 0.

A comparison such as `x < 0.5` gives 1 when it is true and 0 when it is false. Any number but 0 counts as true.

## Colours and palettes

`rgb`, `hsv` and `gray` take values from 0 to 1. A value below 0 is treated as 0 and a value above 1 as 1.

A colour such as `#f80` is not a number, and a name can't hold one.

```psi-bad
name "Oops"
v = #f80
gray(v)
```

A colour can only be used on the colour line. There it can be dimmed (`#f80 * k`) or mixed with another (`mix(#000, #f80, k)`).

`pal(P, v)` picks one colour from palette `P`. It does not blend between neighbours, so a palette you write with three colours gives three bands:

```psi
name "Three bands"
palette sea = #024, #08c, #fff
pal(sea, y)
```

For a fade from one colour to another, use `mix`:

```psi
name "Smooth"
mix(#024, #fff, y)
```

The built-in palettes `fire`, `ice`, `chrono`, `amber` and `rainbow` have 32 colours each, which is close to a fade on a panel 32 rows high. `gray` is the exception: `pal(gray, v)` is the same as `gray(v)`, with no bands.

A `table` is a list of numbers. `level[0]` is the first, and reading past the end gives 0.

```psi
name "Cells"
table level = 0.2, 0.6, 1, 0.6
gray(level[floor(x*4)])
```

## Time and loops

`t` is the time of the frame being drawn, in seconds since the loop began. When the loop ends, `t` starts again from 0. The loop is 8 seconds long unless a `loop` line says otherwise (`loop 4`, from 0.1 to 3,600 seconds).

`phase` is `t` divided by the loop's length, so it goes from 0 to 1 over the loop whatever the length is. `sin(phase)` goes round once a loop.

With `loop forever` there is no loop and `t` keeps growing. A number that grows without end can't go into a colour directly, and the compiler refuses a Formula that does that. Put it through `frac`, `%`, `sin`, `noise`, `min`, `max`, `smoothstep` or a comparison first. `phase` doesn't exist in `loop forever`.

A Formula can't count frames, and the number of frames drawn in a second varies (see the next section). Write where a thing is at time `t`, such as `64 + 40*sin(t/4)`. Don't write how far it moves each frame.

`hour`, `minute` and `second` are the display's clock. They are whole numbers, and all three read 0 until the clock has been set.

In a playlist, a Formula plays whole loops, as a GIF does. The playlist's **Each GIF or Formula stays** setting gives a shortest and a longest time, 1 and 30 seconds unless you change them. A Formula stops at the end of the first loop that takes it past the shortest time, so a `loop 4` Formula plays once and a `loop 0.4` Formula three times. A Formula with no `loop` line has an 8-second loop and plays once. One whose loop is longer than the longest time is cut off there, partway through the loop. A `loop forever` Formula has no loop to finish, so it stays for the longest time. **Play longer or loop…** on it in the playlist changes this: twice as long (twice its usual number of loops, so a `loop 4` Formula plays twice, 8 seconds), or a number of seconds, for any Formula; a number of times, for one with a loop (the menu turns it off for a `loop forever` one). A playlist of one `loop forever` Formula and nothing else plays it on without a break.

A Formula you show on the panel yourself (Play now, **Show on panel**, Remote control) isn't timed: it loops until you skip it, play something else or go back to the playlist. So does a GIF you play yourself.

**Save as GIF…** saves one loop. A GIF's frames are a fixed length, 0.02 seconds here, or 0.04 when the loop is longer than about 5 seconds. A loop that isn't a whole number of frames is up to one frame short in the GIF.

## How often a frame is drawn

The display redraws the panel about 100 to 125 times a second, depending on the panel. Each of these is called a refresh. A Formula is drawn on every 2nd refresh. With a `smooth` line at the top it is drawn on every refresh.

The display measures how long each frame takes. If frames take too long it draws on every 4th or every 8th refresh, and goes back when they get quicker. It never draws more often than the Formula asked for.

A `smooth` Formula is drawn on every refresh only if it is light for the panel (see [What makes a Formula heavy](#what-makes-a-formula-heavy)) and **Phosphor glow** (Settings › Panel) is off. Otherwise it is drawn on every 2nd.

Things move at the same speed at any of these rates, because `t` is the real time of each frame. The preview in the browser draws at about the same rate as the display.

`smooth` and `phase` need OpenDMD 1.1 or later. Older Formulas ask for every refresh with the line `fps 100`, which still works.

## Layers

A Formula can be made of several layers, each written as `layer { … }`. Each layer has its own names and ends in its own colour line. The first layer in the text is at the bottom. Once a Formula has one layer, every colour line has to be inside a layer.

A word after `layer` says how the layer is combined with what is under it: `over` (used when there is no word), `add`, `multiply`, `screen`, `max` or `min`.

A colour can take one more value at the end, from 0 to 1: how much of the pixel it covers. `rgb(1, 0.5, 0, 0.25)` is orange covering a quarter, so three quarters of what is under it shows.

Under the bottom layer is the GIF the Formula is drawn over, or black when it plays alone. On this page, and in the editor until you pick a GIF, it is a sample picture of sky, sun and hills. This Formula is a single `multiply` layer that darkens every other row of that picture:

```psi
name "Rows"
layer multiply {
  gray(py % 2 ? 0.4 : 1)
}
```

If an `over` layer covers every pixel completely, nothing below it can be seen. The compiler drops the layers below it, and they take no time.

### Turning a palette

`cycle(P, v, s)` reads palette `P` at `v` moved on by `s`, going round from the last colour to the first. Changing `s` over time moves the colours through a picture that otherwise stays the same. `s` can read the time or the row. It can't read `x`.

```psi
name "Copper"
loop 6
palette sunset = #102, #315, #a24, #f62, #fb4, #fe8, #fb4, #f62, #a24, #315
cycle(sunset, 0, t/6 + py/40)
```

### A layer drawn less often

`layer rate N { … }` is for a bottom layer that is slow to work out. The display redraws part of it each frame, a band of rows at a time, and gets through all the rows every N frames. The layers above it are drawn in full every frame. Here the noise is redrawn every 4th frame and the orange line every frame:

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

A `draw { … }` block runs once a frame, before the pixels. Inside it you can set names, repeat lines with `for k < N { … }` (one `for` inside another at most), choose with `if c { … } else { … }`, and call the drawing functions. Positions are in pixels: column 0 to 127 from the left, row 0 to 31 from the top.

A `draw` block can't read `x`, `y`, `px`, `py` or anything else that differs between pixels, because it runs before them.

There are two kinds of drawing.

`line` and `dot` with a brightness from 0 to 1 (or none, which means 1) draw into the ink. The ink has no colour. The colour line reads it as `ink`, 0 where nothing was drawn, and decides the colour. Where two marks cross, the brighter one is kept.

`cls`, `pset`, `rect`, `rectfill`, `circ`, `circfill`, `trifill`, `spr` and `print` paint in colour, and so do `line` and `dot` when their last value is a colour. Paint goes on top of every layer, and later paint covers earlier paint. A colour to paint with is `#…`, `pal(…)` or `gray(…)`. `rgb(…)` and `hsv(…)` can't be used there.

```psi
name "Hello"
draw {
  cls(#102)
  print("HELLO", 4, 4, #ff0)
  print(minute, 90, 20, #0ff, small, 2)
}
```

The second `print` shows a number, `minute`, in the small font with at least 2 digits.

## Functions

`fn` defines a function. Its value is worked out from the values it is called with.

```psi
name "Ring"
loop 4
fn ring(cx, r) = abs(len(px - cx, py - 15.5) - r) < 1
gray(ring(64 + sin(t/4)*30, 8) + ring(64, 12))
```

Inside `ring`, `cx` and `r` are the two values of each call. They replace the built-in `cx` and `r` there.

A function written with `{ … }` in place of `= …` can hold several lines. If its last line is a colour, a call to it can be the colour line. If its lines draw, it is called on a line of its own inside `draw`.

When a function's values are named `x` and `y`, the other position names inside it (`px`, `py`, `cx`, `cy`, `r`, `angle`) are worked out from them. Calling it with changed values draws its picture moved, stretched or mirrored:

```psi
name "Mirror"
loop 8
fn p(x, y) {
  wave = sin(x*2 + t/8) + sin(y*1.5 - t/4)
  hsv(wave/4 + t/8, 0.9, r < 0.6)
}
p(abs(x - 0.5)*2, y)
```

The compiler copies a function's lines into each place it is called, so every call adds that much work. A function can't call itself.

In the editor, typing `use Plasma as p` and leaving the line replaces it with the Formula called Plasma, written out as a function `p(x, y)`. It works for your own Formulas and the ones that come with the web app, if they have one colour line and no layers, `draw` block, tables, sprites, palettes or functions.

## Controls

A `control` line at the top gives the Formula a slider or a row of buttons, shown in the editor and on the Formula's card in the web app.

- `control size 0..1 = 0.5 "Size"` is a slider. `size` is its name in the Formula, `0..1` its range, `0.5` where it starts and `"Size"` its label.
- `control mode = calm, wild` is a row of buttons. `mode` reads 0 when the first is chosen, 1 for the second, and so on.

A Formula can have up to three controls. The first can also be read as `a`, the second as `b` and the third as `c`. Reading `a`, `b` or `c` without a `control` line gives a slider from 0 to 1 that starts at 0.

The examples on these pages play with each control where it starts.

## Over a GIF

A Formula can read the picture under it. `ur`, `ug` and `ub` are the red, green and blue of that picture at this pixel, from 0 to 1, and `ul` is its brightness. `under(px + 2, py)` reads it at another pixel. A position outside the panel reads the nearest pixel on it.

The picture under a Formula is the GIF it is drawn over. When a Formula plays on its own there is none, and all of these read 0. On these pages, and in the editor until you pick a GIF, they read the sample picture.

```psi
name "Sepia"
look
mix(rgb(ur, ug, ub), rgb(ul*1.1 + 0.08, ul*0.92 + 0.04, ul*0.65), 0.8)
```

The `look` line says the Formula can be used as the panel's look, which is drawn over every GIF. Its card in the web app then has **Set as the panel’s look**. `look screen 40` would start it with the `screen` blend at 40 %. A Formula with layers can only say `look` or `look over N`.

Three things are different for a look:

- The display won't use a look that covers the GIF completely.
- A look is drawn on every 2nd refresh, about 50 times a second, or on every 4th when it takes more than 8 ms a frame. A `smooth` line has no effect. The GIF under it plays at its own speed.
- A look estimated at more than 12 ms a frame isn't used. A Formula playing alone is allowed 40 ms.

## Values from the display

A Formula can read these as well:

- `daylight` is 0 at night and 1 by day, with twilight between. It needs the display's location (Settings) and reads 1 without it.
- `night` is 1 while night mode is dimming the panel, otherwise 0.
- `bright` is the panel's brightness, from 0 to 1.
- `replay.playing` is 1 while a game is playing on a RePlayOS box the display follows. `replay.game` is a number from 0 to 1 that is the same each time the same game plays.

```psi
name "Dusk"
mix(#081030, #70b0ff, daylight) * (1 - y*0.5)
```

The reference lists the rest: `sun.alt`, `moon.phase`, `season`, `yearday` and `replay.platform`.

## What makes a Formula heavy

The editor shows an estimate of how long the display takes to draw one frame, and a rating under the picture:

| rating | estimate | drawn |
| --- | --- | --- |
| light for the panel | 5 ms or less | every refresh if it asks with `smooth`, otherwise every 2nd |
| medium for the panel | 8 ms or less | every 2nd refresh |
| heavy for the panel | more than 8 ms | every 4th or 8th refresh |

A Formula estimated at more than 40 ms a frame is not played at all. A Formula can have 768 steps at most, and one with more does not compile.

Most of the time goes on values worked out for every pixel. A step there runs 4,096 times a frame. The same step in a value worked out once a row runs 32 times, and once a frame, once. To make a Formula lighter, find the parts of a per-pixel line that read only `t`, or only `y`, and give them names of their own on earlier lines. The editor's warning lists the steps that take longest: "noise, len, sqrt, atan2 and dividing by a changing value".

`c ? a : b` gives `a` when `c` is not 0 and `b` otherwise, but both are worked out every time. To skip work, put it in a `scene`. The lines of a scene are worked out only while its condition is not 0. While it is 0, the names set in it read 0.

```psi
name "Blink"
loop 4
on = frac(t/4) > 0.5
scene s when on {
  w = sin(x*9 + t*3)*0.5 + 0.5
}
gray(w)
```

For half of each loop `on` is 0, `w` reads 0 and the sine is not worked out. The condition of a `scene` can read the time, a control or the row. It can't read `x`.
