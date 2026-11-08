# How Psi thinks

Psi is the language Formulas are written in. This page explains its ideas
one at a time, each with a small example that plays as you read it. To
learn by doing first, start with [Make a Formula](psi-1.md). Every name is
listed in the [language reference](../psi-spec.md).

## One rule for every pixel

A Formula is a rule for one pixel. The panel works it out for each of its
4,096 pixels, 50 times a second, and each time the pixel's place and the
time are different. The rule ends in a colour, and that colour is what the
pixel shows.

A Formula has no memory. Each frame is worked out fresh from the pixel's
place, the time, the clock and the sliders, and from the GIF under it when
it draws over one.

```psi
name "Corners"
rgb(x, y, 0.5)
```

## Where the pixel is

The pixel's place comes in several forms. Use whichever makes the sum
simplest.

| name | what it is |
| --- | --- |
| `x`, `y` | 0 to 1 across from the left, and 0 to 1 down from the top |
| `px`, `py` | the column, 0 to 127 from the left, and the row, 0 to 31 from the top |
| `cx`, `cy` | from the middle, −2 to 2 across and −0.5 to 0.5 down |
| `r`, `angle` | how far from the middle, and the angle round it in turns, from −0.5 to 0.5, 0 on the right |

`y` counts down, as on a screen. For something that rises from the
bottom, use `1 - y`.

The panel is four times wider than it is tall. `cx`, `cy` and `r` are
measured in panel heights, so `r < 0.4` is a round disc and not a long
oval.

```psi
name "Grid and disc"
gray((px % 8 == 0 or py % 8 == 0) + (r < 0.4)*0.5)
```

## Time

`t` is the time in seconds since the loop began. `loop 4` at the top
makes the loop four seconds long, and the picture starts again after it.
Without a `loop` line it is eight seconds.

`loop forever` gives a Formula with no loop. Then `t` counts the seconds
since it started playing and never goes back. Each time it plays it also
gets a new `seed`, a number from 0 to 1, so it can start somewhere new.

```psi
name "Rainbow road"
loop forever
hsv(frac(x + t/10 + seed), 1, 1 - y*0.6)
```

A Formula plays at 50 frames a second. `fps 100` asks for 100, for things
that move fast, and `fps 25` for 25. The panel plays 100 only for a
Formula that is light enough.

`hour`, `minute` and `second` are the time of day, so a Formula can be a
clock.

## Turns

Waves and angles are measured in *turns*. One turn is once round a
circle, and `sin` repeats every 1. `sin(0.25)` is 1, a quarter of the way
round.

Turns make loops easy. In a loop of 8 seconds, `t/8` goes once round, so
`sin(x*2 + t/8)` comes back to where it started just as the loop does.
Keep every speed a whole number of turns a loop and the loop never jumps.

```psi
name "Waves"
loop 4
gray(abs(y - 0.5 - 0.4*sin(x*2 + t/4)) < 0.06)
```

## Everything is a number

Every value in Psi is a number. A question such as `x < 0.5` is a number
too, 1 for yes and 0 for no, so you can multiply by it. `and`, `or` and
`not` join questions. `q ? a : b` is `a` where `q` is yes and `b` where it
is no.

A character in single quotes, such as `'A'`, is the number of that
character.

There is no power sign. Write `x * x` for x squared.

Numbers go from −32,768 to 32,767. A sum that goes past the end stays at
the end. Dividing by zero gives the biggest number, with the sign of what
you divided.

```psi
name "Checks"
gray((px % 16 < 8) != (py % 16 < 8))
```

## Names

A line such as `wave = sin(x*2)` gives a value a name, and the lines below
can use it. Set a name again and the lines after that read the new value.

`--` starts a comment, which the panel skips. A `;` counts as a new line,
so `u = x; v = y` is two lines in one.

## Colours

The last line is the colour. Write it as `#f80`, as `rgb(r, g, b)` or
`hsv(hue, strength, brightness)` with numbers from 0 to 1, or as a colour
from a palette such as `pal(fire, v)`. A plain number is a gray. A colour
can be dimmed with a number, `#f80 * 0.5`, or mixed with another,
`mix(#102040, #f80, x)`. It can't be given a name, so write it in the
last line, where it is used.

`palette sea = #024, #08c, #fff` near the top makes a palette from your
colours, read with `pal(sea, v)`. `cycle(sea, v, t/8)` turns the palette
round once in eight seconds while the picture stays still, the way old
games made water and fire move.

```psi
name "Sea"
loop 8
palette sea = #024, #08c, #4ce, #fff, #4ce, #08c
cycle(sea, sin(x*2) + sin(y*1.5), t/8)
```

## Layers

A Formula can be a stack of layers. Each `layer { }` has its own names
and a colour line, and the panel draws them from the first, at the
back, to the last, in front.

The word after `layer` says how it goes onto what is behind it. `over`,
the default, covers it. `add` and `screen` make it lighter, `multiply`
makes it darker, and `max` and `min` keep the lighter or the darker of the
two.

Any colour can take one more number, how much of the pixel it covers,
from 0 to 1. Where it covers less than all, what is behind shows through.

```psi
name "Bars on a wave"
loop 4
layer {
  wave = sin(x*2 + t/4) + sin(y*1.5 - t/2)
  hsv(wave/4 + t/4, 0.9, 1)
}
layer multiply { gray(py % 2 ? 1 : 0.6) }
```

## Functions

A piece you use more than once can be a function. `fn ring(k) = …` is a
function on one line, and with `{ }` it can have several lines. Call it
with values, as in `ring(0.5)`.

A function whose values are called `x` and `y` draws somewhere else.
Inside it, `px`, `r` and the rest follow the `x` and `y` it is given, so
`p(1 - x, y)` draws `p` mirrored.

```psi
name "Mirror"
loop 8
fn p(x, y) {
  wave = sin(x*2 + t/8) + sin(y*1.5 - t/4)
  hsv(wave/4 + t/8, 0.9, r < 0.6)
}
p(abs(x - 0.5)*2, y)
```

In the editor, a line such as `use Plasma as p` puts the Formula called
Plasma into yours as the function `p`. It works for one of your Formulas
or a bundled one that has a single colour line and no layers, drawing,
tables, sprites or palettes.

## Drawing

Some pictures are easier to draw than to work out one pixel at a time,
such as stars or a wireframe. A `draw { }` block runs once a frame, before
the pixels. Its `line` and `dot` draw *ink*, from 0 to 1, in pixels
counted from the top left, and the colour line reads it back as `ink`.
`for k < 64 { }` runs the lines inside it 64 times, with `k` from 0 to 63.
Inside `draw` only values that are the same for every pixel can be used,
such as `t`, the clock, the sliders and `k`.

```psi
name "Stars"
loop 4
draw {
  for k < 64 {
    dot(frac(hash(k, 1) - t/4)*128, hash(k, 2)*32, hash(k, 3))
  }
}
gray(ink)
```

## Painting

`draw { }` can also paint in colours. `cls` paints the whole panel one
colour, and `pset`, `rect`, `rectfill`, `circ`, `circfill`, `trifill`,
`print` and `spr` paint a pixel, a box, a circle, a triangle, text and a
sprite. Paint goes on top of the colour line, and the last thing painted
is in front, as in an old games console. Paint the back first.

A `sprite` is a little picture written as text, one character a pixel,
with a `key` that says what each character is. In the editor, **Edit the
picture** paints it on a grid.

```psi
name "Hello"
draw {
  cls(#102)
  circfill(110, 16, 9, #f80)
  print("HELLO", 4, 4, #ff0)
}
```

This one has no colour line. Then the paint goes straight onto what is
under the Formula, a GIF or black, and here `cls` covers it first.

## Over a GIF

A Formula can draw over the GIF that plays instead of covering it. Four
values say what is under it at the pixel. `ur`, `ug` and `ub` are its red,
green and blue, and `ul` how bright it is, each from 0 to 1.
`under(px + 2, py)` is the colour two pixels along, so a Formula can move
the GIF too.

The GIF lies under the first layer. It shows wherever a colour covers
less than all of a pixel, and through a first layer with a blend such as
`multiply`. A colour made from `ul`, `ur`, `ug` or `ub` shows the GIF
changed. A `look` line says the Formula can be the panel's
look, drawn over every GIF.

```psi
name "Pixelate"
look
under(floor(px/4)*4 + 2, floor(py/4)*4 + 2)
```

## The world around the panel

A few values come from the display itself, the same for every pixel.
`daylight` is how light it is outside, from 0 at night to 1 by day,
`night` is 1 while night mode dims the panel, and `replay.game` changes
when RePlayOS starts a game. The [reference](../psi-spec.md) lists them
all. In the editor's preview each reads a default, and **Try it out**
under the preview changes them.

```psi
name "Sky"
mix(#081030, #70b0ff, daylight) * (1 - y*0.5)
```

## Keeping it fast

The panel's small processor works out every pixel 50 times a second. The
editor rates each Formula **light**, **medium** or **heavy** for the
panel. In the editor on this site and in the demo, **What costs most**
shows which lines take the most work.

- Adding, multiplying, the waves, comparisons and `pal` are cheap.
- `noise`, `len`, `sqrt`, `atan2`, `smoothstep`, `hash` and dividing by a
  value that changes from pixel to pixel cost the most. One of them is
  fine, and several add up. `r` is a `len` and `angle` an `atan2`.
- Anything that doesn't depend on the pixel, such as `sin(t/8)`, is worked
  out once a frame.
- Anything that depends only on the column and the time is worked out
  once a column.
- Drawing costs by the line, not by the pixel. Painted shapes are the
  exception, so leave big areas of colour to the colour line.
- For a rainbow, `pal(rainbow, h)` is cheaper than `hsv`.

The [reference](../psi-spec.md) has the limits, such as the 32 KB of text
a Formula can have.
