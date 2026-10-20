# The simulator

`opendmd-sim` plays a card in a desktop window through the same
`opendmd-core` engine the firmware runs.

```bash
make sim ARGS="--root /path/to/card --platforms MD,SNES"
```

`make sim` needs a display; the simulator opens a `128 * scale` by
`32 * scale` window — 1280x320 at the default scale of 10.

## What it draws

By default the simulator doesn't draw pixels, it draws **LEDs**, because a P4
HUB75 panel looks nothing like a grid of flat squares. The model in
`opendmd-render/src/lib.rs` is measured off a close-up photo of the real panel
(`hardware/IMG_0495.jpeg`), in units of the 4 mm LED pitch:

- a lit emitter is an anti-aliased disc of ~0.42 pitch across (measured FWHM
  0.40 horizontally, 0.44 vertically — the package makes it slightly tall);
- a soft bloom around it falls to the mask level by ~0.75 pitch, so bright
  neighbours' halos overlap, exactly as they do in the photo;
- the mask between LEDs is near black (5/255), and an unlit LED is a
  barely-there square at 10/255 over half a pitch;
- the panel is driven linearly (BCM/PWM), so canvas bytes are decoded to
  linear light, accumulated there, and re-encoded on the way out. That is why
  mid greys look brighter here than a naive blit makes them.

Everything that depends only on the scale — the per-LED kernel, the backdrop,
the gamma tables — is built once, and the float accumulator is reused, so a
frame is a fixed ~1.6 M multiply-adds at scale 10 with no allocation.

## Options

```
--root <dir>              card root, holding gifs/ and filelist.txt (required)
--manifest <path>         default <root>/filelist.txt
--platforms MD,SNES       comma-separated platform prefixes
--title <substr>          repeatable, matched against the normalized title
--exclude-flag INVERT     repeatable
--category / --exclude-category   repeatable, matched against gifs/<Category>
--shuffle --seed <u64>    reproducible weighted shuffle
--scale <u32>             pixels per LED, default 10
--min-dwell / --max-dwell seconds, default 3 and 30
--transition kind[:ms]    how files come in: dissolve (default, 400 ms),
                          wipe_left, wipe_right, wipe_down, wipe_up, blinds,
                          push, fade or none; ms 20..2000
--flat                    flat squares with a 1 px gutter, the old renderer
--gamma <f32>             canvas byte -> linear light, default 2.2
--led-gain <f32>          scales LED intensity, default 1.0 — match the panel
                          brightness by eye
--bloom <f32>             halo strength around each lit LED, default 0.22
                          (fitted by eye to the photo — the knob to tune)
--screenshot <out.png>    render the first frame of the first scheduled GIF
                          and exit, without opening a window
--screenshot-canvas <out.png>   the same frame as the raw 128x32 canvas
                          (after --look)
--look <name>             colour (default), amber, red, green, blue or white
--glow off|short|long     phosphor glow, default off
--play <path>             play just this: a card path or a live item,
                          e.g. '@clock?date=1', '@text?msg=Hello',
                          '@fx:tunnel?palette=chrono&cycle=1'
--screenshot-at <ms>      take the screenshot that far into the file (an
                          effect mid-loop), default 0
```

Below `--scale 6` there are too few window pixels per LED for a dot and a
bloom, so the flat renderer is used automatically and says so in the log.

Keys: `Space` next, `P` pause/resume, `L` toggle LED/flat (the title bar shows
which), `Esc` quit. Each new GIF prints
`[n/total] /gifs/Consoles/SNES_Plok_08.gif`; startup prints the filter summary.

The two `--screenshot*` flags are the only way to look at the renderer on a
headless box, where `minifb` cannot open a window:

```bash
cd opendmd-sim && cargo run --release -- --root /path/to/card \
    --platforms SNES,MD --seed 7 --screenshot /tmp/led.png \
    --screenshot-canvas /tmp/canvas.png
```
