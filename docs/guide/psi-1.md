# Make a Formula, 1: change one

A Formula is a few lines of text that make a moving picture on the panel. The language they are written in is called Psi. This tour takes three short pages. You change numbers first, then make things move, then draw shapes.

## Where to type

Every example on these pages plays as you read. Under each one, **Open in editor** opens it in the editor on this site, where you can change it and watch the result. When you like what you have, press **Open on my panel**. The first time, it asks for your display's address. It then saves the Formula on your display, under Your formulas, and opens it there.

If you already have the web app open on your display, you can type into its own editor instead: **Formulas**, then **+ New formula**. **Save** keeps the Formula on the display (in this browser, on a display with no card), and **Show on panel** plays it there. [The editor](editor.md) has the details.

The [gallery](/formulas/) has finished Formulas to open in the editor and change.

In both places the picture beside the code is drawn in your browser by the same rules the display uses, and it changes as you type.

## One colour everywhere

```psi
name "First light"
hsv(0.6, 1, 1)
```

The first line gives the Formula its name. The last line is the colour, and a Formula always ends with one. The display works it out for all 4,096 pixels of the panel, 128 across and 32 down, and does that again for every frame.

`hsv` takes three numbers, each from 0 to 1:

- the hue, the colour's place round the colour wheel;
- the saturation, from grey to full colour;
- the value, from black to full brightness.

Drag the 0.6 down towards 0 in the editor and the blue turns cyan, then green, yellow and red. A hue of 0 and a hue of 1 are the same red, because the wheel comes round to where it started.

## A colour that changes across the panel

The colour line can read where the pixel is. `x` runs from 0 at the left edge to 1 at the right, and `y` from 0 at the top to 1 at the bottom.

```psi
name "Fade"
gray(x)
```

`gray` takes one number, 0 for black and 1 for white. With `x` in it, every column gets its own shade.

```psi
name "Corners"
rgb(x, y, 0.5)
```

`rgb` mixes red, green and blue, each from 0 to 1. Here red grows to the right, green grows downwards and blue stays at half. Values past 1 stay at full, and values under 0 stay at none.

```psi
name "Rainbow"
hsv(x, 1, 1)
```

Put `x` where the hue goes and the whole colour wheel spreads across the panel once. Try `x*2` to see it twice.

## Palettes

A palette is a ready-made run of colours. `pal(fire, v)` runs from black through red, orange and yellow to white as `v` goes from 0 to 1.

```psi
name "Embers"
pal(fire, 1 - y)
```

`1 - y` is 1 at the top and 0 at the bottom, so the heat is at the top. Change it to `y` to turn it over. The other palettes are `ice` (also called `ocean`), `chrono`, `amber`, `rainbow` and `gray`.

## Whole pixels

`px` is the pixel's column, from 0 to 127, and `py` is its row, from 0 to 31. They count whole pixels, so they suit sharp patterns.

```psi
name "Grid"
gray(px % 8 == 0 or py % 8 == 0)
```

`%` gives the remainder, so `px % 8` counts 0 to 7 over and over. A comparison such as `==` or `<` gives 1 when it is true and 0 when it is false. `or` joins two of them. So the grey is 1 on every eighth column and every eighth row, and 0 everywhere else. Change both 8s to 16 for a wider grid.

## When it goes wrong

When a line doesn't make sense to Psi, the editor shows the problem above the code, as **Line N:** and a message. The picture keeps showing the last version that worked. Sometimes there is a **Fix it** button that makes the change for you.

```psi-bad
name "Oops"
gray(2x)
```

On paper, `2x` means two times x. Psi needs the `*`: `gray(2 * x)`. The message here says so: "`2x` is not a number or a name (to multiply, write `2 * x`)".

## Naming and keeping it

The `name "…"` line is the Formula's name, up to 60 characters. When you save in the web app, it fills that line in from the name box at the top. An `about "…"` line under it can hold one line of description. Both go at the top, before the other lines. Lines that start with `--` are comments, notes for people that the display ignores.

Next: [Make a Formula, 2: make it move](psi-2.md).
