# Make a Formula, 3: add a shape

There are two ways to put a shape on the panel. You can describe it by distance, as a rule every pixel checks. You can also draw it, with lines and circles, the way you would on paper. This page does both.

## A shape by distance

`len(dx, dy)` is the distance across `dx` and down `dy`. Measured from a point in pixels, it gives a circle.

```psi
name "Disc"
gray(len(px - 64, py - 16) < 10)
```

Every pixel works out how far it is from column 64, row 16. The ones closer than 10 pixels are lit. Change the 10 to change the size, and the 64 to move it.

A hard edge is either on or off. `smoothstep` gives a soft one.

```psi
name "Soft disc"
d = len(px - 64, py - 16)
#ffa040 * smoothstep(12, 8, d)
```

`smoothstep(12, 8, d)` is 0 when `d` is 12 or more, 1 when it is 8 or less, and eases between the two. `#ffa040` is a colour written as red, green and blue in hex, the same as in web pages, and `#f80` is a short way to write `#ff8800`. A colour times a number is that colour dimmed.

## Moving it

```psi
name "Orbit"
loop 4
bx = 64 + 40*sin(t/4)
d = len(px - bx, py - 16)
pal(fire, 1 - d/12)
```

`bx` swings between 24 and 104 and back, once a loop. Notice that `bx` reads only the time, so it is the same for every pixel. Psi works it out once a frame. `d` reads `px` and `py`, so it is worked out for each of the 4,096 pixels. That difference matters when a Formula gets bigger, and [How Psi thinks](psi.md) explains it.

## Drawing instead

A `draw { … }` block works the way a pen does. It runs once a frame, before any pixel is worked out, and puts marks on the panel at positions in pixels, counted from the top left.

```psi
name "Ball"
loop 4
draw {
  circfill(64 + 40*sin(t/4), 16, 8, #f80)
}
```

`circfill(x, y, r, colour)` paints a filled circle. Painting is drawn on top of everything else, so this Formula needs no colour line. Under the paint you see whatever is under the Formula: on this page a sample picture of sky, sun and hills, and on the display black, or the GIF it is drawn over. There are also `circ` for a ring, `rect` and `rectfill` for boxes, `trifill` for triangles, `pset` for one pixel, `print` for text and `cls` to fill the whole panel.

Inside `draw`, there is no "this pixel". A line such as `dot(x, 1)` gets an error, because `x` changes from pixel to pixel and `draw` runs before any of them.

```psi-bad
name "Oops"
draw {
  dot(x, 1)
}
gray(ink)
```

## Ink

`line` and `dot` can also draw with a brightness instead of a colour. That puts marks into a picture called the ink, and the colour line reads it as `ink`, from 0 to 1. Where two marks cross, the brighter one wins.

```psi
name "Sweep"
loop 4
draw {
  line(64, 16, 64 + 40*cos(t/4), 16 + 15*sin(t/4))
}
pal(fire, ink)
```

The line goes from the middle to a point that goes round it. `pal(fire, ink)` then colours the ink. Change `fire` to `ice` and the same line is drawn in blue.

## Many at once

`for k < 64 { … }` repeats its lines 64 times, with `k` counting 0, 1, 2 and on up to 63.

```psi
name "Snow"
loop 4
draw {
  for k < 64 {
    dot(hash(k, 1)*128, frac(hash(k, 2) + t/4)*32)
  }
}
gray(ink)
```

`hash(a, b)` gives a number between 0 and 1 that looks random but is always the same for the same `a` and `b`. Each flake gets its own place across and its own start down. `frac(… + t/4)` makes each one fall the whole height of the panel once a loop and start again at the top. A frame can run 2,048 turns of `for` loops in all.

## A small picture

A sprite is a little picture drawn in characters. The `key` line says what each character stands for: here `.` is nothing and `R` is a red.

```psi
name "Heart"
sprite heart {
  key . 0, R #e24
  .R.R.
  RRRRR
  .RRR.
  ..R..
}
draw {
  spr(heart, 61, 14)
}
```

`spr(heart, 61, 14)` paints it with its top left corner at column 61, row 14. Add a fourth value of 1 to flip it left to right, 2 to flip it upside down, or 3 for both.

To start from a picture instead of typing one, press **Picture…** under the code: see [Make a sprite from a picture](editor.md).

## Layers

A Formula can have several colour lines, each in its own `layer { … }`. Layers stack, the first at the bottom.

```psi
name "Moon over sea"
layer {
  pal(ocean, y)
}
layer {
  rgb(1, 0.9, 0.6, r < 0.3)
}
```

The first layer fills the panel with the `ocean` palette, another name for `ice`: black at the top, through blue, to white at the bottom. The second has a fourth value, how much of the pixel it covers, from 0 to 1. `r` is the distance from the middle of the panel, so `r < 0.3` is 1 inside a disc and 0 outside it. Where it is 0, the sea below shows through.

That's the tour. The [gallery](/formulas/) has finished Formulas to open in the editor and change. [How Psi thinks](psi.md) takes the ideas behind it one at a time. When you want every name and function, [the language reference](../psi-spec.md) lists them all.
