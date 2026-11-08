# Make a Formula, 2: make it move

On [page 1](psi-1.md) you made a still picture. This page sets it moving
and ends with a plasma, the classic of the demo scene. Press **Open in
editor** under any step to change it.

## The dot swings

```psi
name "Swing"
control size 0.1..0.5 = 0.3 "Size"
loop 4
#f80 * (len(cx - sin(t/4)*1.5, cy) < size)
```

This is the dot from page 1 with one change. Where it said `cx - 1`, it
now says `cx - sin(t/4)*1.5`.

`t` is the time, in seconds since the loop began. `loop 4` makes the loop
four seconds long, so `t` goes from 0 to 4 and starts again. Without a
`loop` line the loop is eight seconds.

`sin` makes a wave between −1 and 1. As the number you give it goes from 0
to 1, the wave goes once up, down and back, and Psi calls that one
*turn*. `t/4` goes from 0 to 1 in the four seconds, so the dot makes one
swing a loop. `*1.5` makes the swing wider.

## A ramp that slides

```psi
name "Slide"
loop 4
gray(frac(x - t/4))
```

`gray` turns a number into a shade, 0 black and 1 white. `frac` keeps the
part of a number after the point, so it climbs from 0 to almost 1 and
drops back to 0. The ramp slides to the right, once across in a loop.

Try `frac(x*4 - t/4)` for four ramps, or `frac(x + t/4)` to slide the
other way.

## A wave

```psi
name "Wave"
loop 8
gray(sin(x*2 + t/8)*0.5 + 0.5)
```

`sin(x*2)` makes two waves across the panel, because `x*2` goes from 0
to 2. `*0.5 + 0.5` turns the wave's −1 to 1 into 0 to 1, which is what
`gray` wants. Adding `t/8` moves the wave along one turn in the eight
seconds of the loop.

Drag the `2` to change how many waves there are.

## Loops that don't jump

```psi
name "Two speeds"
loop 8
top = sin(x*2 + t/8)
bottom = sin(x*2 + t*3/8)
gray((y < 0.5 ? top : bottom)*0.5 + 0.5)
```

When the loop starts again, `t` goes back to 0. A wave that has come
round a whole number of turns is back where it started, so you can't see
the join. `t/8` is one turn in a loop of 8, and `t*3/8` is three. Change
the `3` to `2.5` and watch the bottom half jump each time the loop starts
again.

`y < 0.5 ? top : bottom` picks `top` where the question is yes and
`bottom` where it is no.

## Two waves

```psi
name "Two waves"
loop 8
wave = sin(x*2 + t/8) + sin(y*1.5 - t/4)
gray(wave/4 + 0.5)
```

The second wave goes down the panel and moves the other way, twice as
fast. Added together the two make a number from −2 to 2, so `wave/4 +
0.5` brings it back to 0 to 1.

## Every colour

```psi
name "Colours"
loop 8
wave = sin(x*2 + t/8) + sin(y*1.5 - t/4)
hsv(wave/4 + t/8, 1, 1)
```

`hsv` makes a colour from three numbers. The first is the hue, in turns
round the rainbow. 0 is red, 1/3 green, 2/3 blue, and 1 is red again. The
second is how strong the colour is, from 0 for gray to 1 for full, and the
third how bright, from 0 for black to 1.

Here the wave picks the hue, and `+ t/8` turns the whole rainbow once a
loop.

## The plasma

```psi
name "My plasma"
loop 8
wave = sin(x*2 + t/8) + sin(y*1.5 - t/4) + sin(x*1.5 + y + t/8)
hsv(wave/6 + t/8, 0.9, 1)
```

Add a third wave on a slant and you have, very nearly, the gallery's
**Plasma**. Three waves make a number from −3 to 3, so the hue takes
`wave/6`.

Now drag the numbers. Each `*2` and `*1.5` changes the size of a wave,
each `t/8` and `t/4` its speed, and `0.9` how strong the colours are. If a
change makes the loop jump, check that each speed is a whole number of
turns.

Next, [add a shape](psi-3.md). You put a ring over a sky and paint a little
picture of your own.
