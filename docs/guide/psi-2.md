# Make a Formula, 2: make it move

On the first page every picture stood still. To make one move, the colour line reads the time.

## Time and the loop

`t` is the number of seconds since the loop began. A `loop 4` line near the top makes the loop 4 seconds long, so `t` counts from 0 to 4 and then starts again. Without a `loop` line the loop is 8 seconds. It can be anything from 0.1 to 3,600 seconds.

```psi
name "Slide"
loop 4
hsv(x + t/4, 1, 1)
```

As `t` goes from 0 to 4, `t/4` goes from 0 to 1. That moves the hue once round the whole colour wheel, so when the loop starts again the colours are exactly where they began, and there is no jump.

## Waves

`sin(v)` makes a smooth wave between −1 and 1. Psi measures it in turns: the wave goes round once as `v` goes from 0 to 1. There is no pi to multiply by.

```psi
name "Breathe"
loop 2
gray(0.5 + 0.5*sin(t/2))
```

`0.5 + 0.5*sin(…)` turns the wave's −1 to 1 into 0 to 1, the range `gray` wants. `t/2` goes from 0 to 1 over the two-second loop, so the panel brightens and dims once each loop. `cos`, `tri` (a triangle wave), `saw` (a sawtooth) and `sqr` (a square wave) keep the same timing.

## Giving a value a name

A line like `wave = …` works out a value and gives it a name. Later lines can use the name.

```psi
name "Swell"
loop 4
wave = sin(x*2 - t/4)
hsv(0.55, 1, wave/2 + 0.5)
```

With `x` and `t` both in the wave, it rolls across the panel. `x*2` puts two waves side by side, and `- t/4` moves them to the right, one wave further each loop. The name `wave` only exists in this Formula, so pick any word that isn't one of Psi's own (`loop`, `layer`, `draw`, `fn` and a few others).

A wave can also bend a line:

```psi
name "Ribbon"
loop 4
gray(abs(y - 0.5 - 0.4*sin(x*2 + t/4)) < 0.06)
```

`0.5 + 0.4*sin(…)` is the height of a wavy line at each column. `abs(y - …)` is how far the pixel is from it, and `< 0.06` lights only the pixels close to it. Make 0.06 bigger for a thicker ribbon.

## The clock

`hour` (0 to 23), `minute` and `second` read the display's clock. On these pages they read your computer's clock.

```psi
name "Seconds bar"
gray(x < second/60)
```

The bar grows across the panel during each minute. Until the display's clock has been set, all three read 0.

## A slider

A `control` line at the top makes a slider. You set it in the editor, or on the Formula's card in the web app, without touching the code.

```psi
name "Speed dial"
control speed 0..2 = 1 "Speed"
loop 4
gray(frac(x - t*speed/4))
```

This reads as a control called `speed`, going from 0 to 2, starting at 1, with the label "Speed". `frac` keeps only the part after the decimal point, so the grey climbs from 0 to 1 and drops back, and the ramp slides along as `t` grows. The examples on these pages play with each slider at its starting value. Open one in the editor to move the slider. A Formula can have three controls.

## Frames a second

The display shows a Formula at 50 frames a second unless it asks for another rate. `fps 25`, `fps 50` and `fps 100` are the choices.

```psi
name "Fast sweep"
fps 100
loop 1
gray(abs(x - frac(t)) < 0.02)
```

At 100 frames a second a fast line looks smoother. The display only keeps that rate for a Formula that is light for the panel, and with **Phosphor glow** on (Settings › Look) the display holds Formulas to 50.

The editor rates each Formula under the picture as "light for the panel", "medium for the panel" or "heavy for the panel". A heavy one moves less smoothly on the display, because the display shows fewer frames. A Formula that is far too heavy isn't played at all. The page [How Psi thinks](psi.md) explains what makes a Formula heavy.

## Never the same twice

`loop forever` makes a Formula that doesn't repeat. `t` keeps growing for as long as it plays.

```psi
name "Drift"
loop forever
hsv(frac(x + t/10), 1, 1)
```

A number that grows without end can't go straight into a colour, so Psi asks you to wrap it first. `frac`, `%`, `sin`, `noise` and a comparison all do that.

```psi-bad
name "Oops"
loop forever
gray(t*x)
```

Here the editor says "`t` grows without end in `loop forever`, and this can't take it", and lists the ways to wrap it. A `loop forever` Formula can also read `seed`, a number from 0 to 1 that is new each time it starts playing.

Next: [Make a Formula, 3: add a shape](psi-3.md).
