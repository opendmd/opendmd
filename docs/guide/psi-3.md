# Make a Formula, 3: add a shape

On [page 2](psi-2.md) the whole panel moved. This page puts shapes on it,
one in front of another, then a little picture you paint yourself, and
ends with a Formula that draws over your GIFs.

## A ring

```psi
name "Ring"
loop 4
hsv(angle + t/4, 1, abs(r - 0.35) < 0.05)
```

`r` is the distance from the middle, as on page 1. `abs(r - 0.35)` is how
far the pixel is from a circle of size 0.35, so `< 0.05` keeps a thin ring.

`angle` is where the pixel is round the middle, in turns. It is 0 on the
right, 0.25 at the bottom, 0.5 or −0.5 on the left and −0.25 at the top.
A hue goes round again after each whole turn, so used as the hue, `angle`
puts the whole rainbow round the ring. `+ t/4` turns it once a loop.

## A ring over a sky

```psi
name "Ring over a sky"
loop 4
layer { mix(#102040, #f84, y) }
layer { hsv(angle + t/4, 1, 1, abs(r - 0.35) < 0.05) }
```

A Formula can be a stack of layers. Each `layer { }` has its own colour,
and the panel draws them from the first, at the back, to the last, in
front.

The ring's colour has a fourth number now. It says how much of the pixel
the colour covers, from 0 to 1. On the ring it is 1, so the ring covers
the sky. Everywhere else it is 0, and the sky shows through.

## Lines

```psi
name "Rays"
loop 4
draw {
  for k < 12 {
    an = k/12 + t/4
    line(64, 16, 64 + cos(an)*40, 16 + sin(an)*14, 0.2, 1)
  }
}
hsv(x, 0.6, ink)
```

Some pictures are easier to draw than to work out pixel by pixel. A
`draw { }` block runs once a frame, before the pixels, and draws lines and
dots.

`line` takes the two ends in pixels, counted from the top left. The panel
is 128 pixels across and 32 down, so `64, 16` is the middle. The last two
numbers are how bright the line is at each end, here 0.2 at the middle and
1 at the tip. `cos` is the same wave as `sin`, a quarter of a turn
earlier, so `cos(an)*40` and `sin(an)*14` go round an oval together as
`an` goes round a turn. `for k < 12` runs the lines inside it twelve times, with `k`
from 0 to 11, so `k/12` spreads the twelve rays round a turn.

The last line reads what was drawn as `ink`, from 0 where nothing was to 1.
Here it is the brightness of a colour that changes across the panel.

## A little picture

```psi
name "Drive"
loop 4
sprite car {
  key . 0, R #d22, W #fff, K #222
  ..RRRR..
  .RWWWWR.
  RRRRRRRR
  .K....K.
}
draw {
  circfill(100, 8, 6, #fc4)
  rectfill(0, 27, 127, 31, #333)
  spr(car, frac(t/4)*136 - 8, 23)
}
mix(#124, #f84, y)
```

A `sprite` is a picture written as text, one character a pixel. Its `key`
line says what each character is. `.` is `0`, which shows nothing, and
`R #d22` makes `R` red.

In the editor, put the cursor in the sprite and press **Edit the
picture**. Pick a character from the key, then click or drag on the grid
to paint with it. **+ Colour** adds a colour to the key, and **+ Column**
and **+ Row** make the picture bigger.

Inside `draw { }`, `circfill` paints a filled circle for the sun,
`rectfill` a filled box for the road, and `spr` the car, with its top left
corner where you say. Paint goes on top of the last line's colour, here
the sky. What is painted last is in front, so paint the things at the back
first.

## Over a GIF

```psi
name "Warm"
look
rgb(ul*1.1 + 0.08, ul*0.92 + 0.04, ul*0.65)
```

`ul` is how bright the GIF under the Formula is at this pixel, from 0 to
1. A Formula that reads it draws over a GIF instead of covering it. This
one turns the GIF into warm browns, like an old photo.

In the editor it is drawn over a sample picture. Press **Pick a GIF…** to
try it over one of your GIFs.

The `look` line says the Formula can be the panel's look. In your web app,
**Set as the panel's look** then keeps it over every GIF the panel plays.

## Where next

- [How Psi thinks](psi.md) explains the ideas behind everything here, a
  few lines each.
- [The editor](editor.md) has every button and key.
- The [recipes](../psi-spec.md#cookbook) are complete Formulas to open and
  change, and the [language reference](../psi-spec.md) lists every name.
- The [formula gallery](https://opendmd.dev/formulas/) has many more. Open
  any one to see how it is made.
