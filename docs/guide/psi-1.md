# Make a Formula, 1: change one

A Formula is a short piece of text that the panel turns into a picture,
pixel by pixel, 50 times a second. You don't need to have programmed
before. On this page you write one line, change it a few times, and end
with a Formula of your own on the panel.

Each example plays as you read it. Press **Open in editor** under one to
change it yourself. In your web app, open **Formulas** and press **+ New
formula**, then type over the text that is there.

## One colour

```psi
name "Step 1"
#f80
```

The whole panel turns orange. `#f80` is a colour written the way web
pages write them, with one digit each for red, green and blue, from `0`
to `f`. `#f00` is red, `#0f0` green and `#fff` white.

In the editor, click in a colour and a swatch appears at the end of its
line. Click the swatch to open a colour picker. The colour you pick takes
the old one's place.

The last line of a Formula is a colour, and it is what the panel shows.

## A fade

```psi
name "Step 2"
#f80 * x
```

Now the orange fades in from the left. `x` is where the pixel is across
the panel, 0 at the left edge and 1 at the right. Multiplying a colour by
a number between 0 and 1 dims it, so the left is black and the right is
full orange.

Try `y` instead of `x`. It is 0 at the top and 1 at the bottom, so the
fade now comes in from the top. To fade in from the bottom, write
`#f80 * (1 - y)`.

The panel works that one line out for each of its 4,096 pixels, and each
pixel has its own `x` and `y`.

## Two colours

```psi
name "Step 3"
mix(#102040, #f80, x)
```

`mix` slides from the first colour to the second as the third number goes
from 0 to 1. Here that number is `x`, so the panel is dark blue on the
left and orange on the right.

Click in `#102040` and pick a green from its swatch. Then replace `x` with
`y`, or with `x * y`.

## A dot

```psi
name "Step 4"
#f80 * (r < 0.3)
```

`r` is how far the pixel is from the middle of the panel. `r < 0.3` is a
question, and the panel answers it with a number. It is 1 where the answer
is yes and 0 where it is no. Multiplied by the colour, that gives an
orange dot on black.

Drag the `0.3` left and right to change the size. On a phone, tap the
number and drag the bar that appears under the code. Change `<` to `>`
for a hole instead of a dot.

## Put the dot somewhere else

```psi
name "Step 5"
#f80 * (len(cx - 1, cy) < 0.3)
```

`cx` and `cy` say where the pixel is, measured from the middle. `cx` runs
from −2 at the left edge to 2 at the right, and `cy` from −0.5 at the top
to 0.5 at the bottom. `len` gives the distance to a point, and here the
point is 1 to the right of the middle. Drag the `1`, and the dot goes with
it.

## A slider

```psi
name "Step 6"
control size 0.1..0.5 = 0.3 "Size"
#f80 * (len(cx - 1, cy) < size)
```

A `control` line at the top makes a slider under the preview. `size` is
its name in the Formula, `0.1..0.5` its range, `0.3` where it starts and
`"Size"` its label. Move the slider and the dot grows.

A Formula can have up to three controls. They go with it to the panel, to
your playlists and to the gallery.

## A dot on a sky

```psi
name "Step 7"
control size 0.1..0.5 = 0.3 "Size"
dot = len(cx - 1, cy) < size
mix(#102040, #f80, dot)
```

You can give something a name on a line of its own and use it further
down. `dot` is 1 inside the disc and 0 outside, so `mix` picks orange
inside and dark blue outside. Names make a Formula easier to read and to
change.

## Make it yours

Change the colours, the place and the size until you like it. Then change
`name "Step 7"` to a name of your own. It is what the panel, your
playlists and the gallery call it.

In your web app's editor, press **Save** to keep it on your panel under
**Your formulas**. Press **Show on panel** (Ctrl+Enter) to play it there.

In the editor on this site, press **Open on my panel** instead. It saves
the Formula on your panel and opens it in your web app's editor.

Next, [make it move](psi-2.md). The dot swings from side to side, and you
meet `t`, the time.
