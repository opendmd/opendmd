# Writing an effect

The panel's effects (Tunnel, Rotozoom, Starfield, Wireframe) are pictures
computed every frame rather than GIFs read from the card. A playlist names
one as a line such as `@fx:tunnel?palette=fire&texture=rings`, and the same
Rust code draws it on the ESP32, in the desktop simulator and in every
browser preview. This page is for programmers who want to add one: what an
effect is, the rules it has to keep on the ESP32, every place a new effect
is registered, and a small worked example, a plasma.

Everything here lives in `opendmd-core/src/fx.rs`.

## What an effect is

An effect implements one trait:

```rust
pub trait Effect {
    /// The effect's name in its path (`tunnel`, ...).
    fn name(&self) -> &'static str;
    /// Frames in one loop.
    fn loop_frames(&self) -> u32;
    /// Draw the frame at `t` into `canvas`, marking what it changes,
    /// and return how long the frame holds.
    fn frame(&mut self, t: Duration, canvas: &mut Canvas) -> Duration;
}
```

- `t` is a multiple of the frame delay, less than one loop.
- The canvas still holds the frame before (the one at `t` minus one delay,
  or the loop's last frame at `t = 0`), except on the first call after the
  effect starts.
- The returned delay is treated like a GIF frame's delay: 10, 20 or 40 ms,
  for 100, 50 or 25 frames a second.

The effects themselves are variants of one enum, `Fx`, held inline with no
`Box`: an effect's whole state is its options and perhaps a flag, a few
bytes inside the player's decoder slot. `Fx::parse(path)` turns an `@fx:`
path into one, and the player treats it as a virtual file. That means
transitions, dwell, the now-playing status, the looks (tint and glow) and
the browser preview all come for free.

## Frames are pure functions of time

Every effect is periodic. Frame `n` of a loop of `N` frames is computed
from its phase, `p = n × 65,536 / N`, and nothing else. Everything that
moves completes a whole number of cycles per loop: a whole turn, a whole
texture length, a whole trip round the palette. So frame `N` would equal
frame 0 and the loop is seamless. This buys a lot:

- The player loops an effect in place, with no seam and no cost.
- The browser preview follows the panel by computing the frames in
  between, and gets the same pictures.
- A sparse effect can erase the previous frame by drawing it again in
  black, because it can recompute it, instead of remembering it.
- **Save as GIF** in the web app can draw any frame on its own.

The options every effect shares are in `Common`: `speed` (1 to 4, default
2), the frame delay (`fps=100|50|25`), `palette` and `cycle` (one turn of
the palette per loop). Its helpers do the bookkeeping:

| helper | what it does |
| --- | --- |
| `common.frames(base)` | frames in a loop whose length at speed 1 and 50 fps is `base` frames. Make `base` a multiple of 24 so that every speed and frame rate divides it evenly |
| `common.index(t, n)` | the frame index at `t` in a loop of `n`, and the one before it |
| `phase(i, n)` | the phase of frame `i`: 0 to 65,536 over the loop |
| `common.colour(pal, v, cyc)` | the palette entry for a value `v` of 0 to 255, plus the cycling offset |
| `common.delay()` | the frame delay to return |

### The shared tables

The frame path is integer and fixed-point. These tables are shared, so
use them before adding your own:

- `SIN`: 256 entries of the sine in Q14 (±16,384). `sin16(p)` and
  `cos16(p)` interpolate it to 65,536 steps a turn, which is smooth at any
  speed.
- The palettes: five of 32 colours each (fire, ice, chrono, amber,
  rainbow), from `palette.table()`. They are stepped, with a black band at
  entry 0, on purpose. The panel shows smooth, dim gradients as bands
  anyway, and hard steps look better when the palette cycles.
- The textures: checker, rings and noise, 64 × 32 bytes each, in the
  `TABLES` struct. Texel values are palette indices × 8.
- `hash(x)`: a small integer hash, for anything that should look random
  but be the same every time (the stars' positions, the noise).

Every table is a `static` built at compile time by a `const fn` (the
maths uses `f64` in const evaluation: a Taylor-series sine, Newton square
roots), so on the ESP32 it sits in flash and costs no RAM. All of them
together are `TABLE_BYTES`, asserted to stay at or under 24 KB. A table
that is read for every pixel belongs inside the one `TABLES` struct.
Separate statics fight each other for the same lines of the ESP32's flash
cache, and one texture then costs half as much again as another, or
more.

## The canvas

An effect draws into a `Canvas` of 128 × 32 `Rgb` pixels, which carries a
dirty map: the panel only sends the pixels that changed.

| call | marks the dirty map? |
| --- | --- |
| `set(x, y, rgb)`, `get(x, y)` | `set` marks the pixel; both ignore coordinates off the panel |
| `fill(rgb)`, `clear()` | marks everything |
| `pixels_and_dirty_mut()`, `bytes_and_dirty_mut()` | no: you mark what you write, through the map they hand back |
| `dirty_mut().mark(x, y)`, `mark_span(y, x0, x1)`, `mark_all()` | the marking itself |

There are two good patterns:

- **Full-frame effects** (tunnel, rotozoom) write every pixel through
  `pixels_and_dirty_mut()` and call `mark_all()`. The panel then sends 16
  whole row pairs, its cheapest path.
- **Sparse effects** (starfield, wireframe) draw the previous frame again in
  black, then the new one, setting a pixel only if it differs. Only the
  pixels they touch are marked and nothing is left behind. On the first
  frame after starting they clear the canvas instead: that is the `painted`
  flag in `Starfield` and `Wire`.

The tests check both: a full-frame effect must mark every pixel, and a
sparse one must, on every frame of a loop, equal the same frame drawn on a
fresh canvas, with every changed pixel marked.

## Options

`Fx::parse` reads `@fx:<name>?key=value&key=value`:

1. The name picks the variant and its defaults (including the default
   palette and whether it cycles).
2. Each `key=value` pair is matched as bytes against `(key, variant)`.
   `small(v, lo, hi)` reads a single digit in a range.
3. **Unknown keys are ignored and bad values keep the default**, as for
   the clock and the scrolling text. A playlist line from a newer version
   still plays on an older one, and a typo never stops the panel.

The parser works on bytes and allocates nothing. That is deliberate: the
same code is in the web app's wasm module, where string splitting and
number parsing from the standard library cost kilobytes.

## The ESP32's limits

An effect runs where a GIF's decoder would. The display loop (the panel,
the playlist engine, the frame loop) has CPU1 of the ESP32 to itself,
while WiFi, the web server and file transfers run on CPU0. An effect's
render happens inside the engine's tick on CPU1, and is counted as the
frame's decode time.

**The time budget.** A frame is render plus blit, and has to fit in the
frame's delay. A full-screen blit takes about 1.3-1.8 ms.

- An effect may take up to **8 ms (90th percentile) a frame at 50 fps**.
  That leaves room for a transition and the phosphor glow in the 20 ms
  frame.
- One that renders in **5 ms or less may run at 100 fps** (10 ms frames,
  the panel's refresh rate), and the effects default to that when it costs
  no more than a handful of extra late frames. The four effects render in
  1-3.5 ms.
- While the glow is on, effects are held to 50 fps whatever they ask for
  (`set_frame_floor`): the glow's blit plus a 100 fps render does not fit
  in 10 ms. The loop keeps its length, at half the frames.

Keep the per-pixel work to table reads, adds, multiplies and shifts: no
trigonometry and no division per pixel. Work that depends only on the row
or the column can be done once a frame, into a small array on the stack.

**Memory.** Nothing on the heap, ever:

- An effect's state is its options and at most a flag, inline in `Fx`
  (asserted to be at most 512 B; it is 10 B).
- No allocation per frame and none when the effect starts.
- Per-frame scratch goes on the stack: today's effects use up to about
  1.3 KB (the tunnel's lit palette, the wireframe's vertices). Keep yours
  that small or smaller, because the display loop's stack is shared with
  everything else on it.
- Tables are `static`, computed at compile time.

**Flash and the web.** Tables cost flash in the firmware image, and also
download size in the web app's main wasm module, which has a size budget
(`make web` checks it). Tables that compress well cost less there:
that's one reason texels are multiples of 8.

### Measuring it

- **On the desktop**, as a rough guide: add your paths to the ignored
  `fx_cost` test in `fx.rs`, then run
  `cargo test --release -- --ignored fx_cost --nocapture` in
  `opendmd-core`. Compare against the existing effects rather than reading
  the numbers as milliseconds on the ESP32: the default tunnel's 3.5 ms
  on the device is the yardstick.
- **On the device**: build the firmware with the timing probe
  (`make fw-build FW_FEATURES=probe`), play the effect with
  `POST /api/play` (see [the API](api.md)) and capture the serial log. The
  probe prints a per-30-second `decode` histogram, which is the effect's
  render time, and the blit's; `uv run tools/sdprobe-report.py LOG` turns
  a capture into tables. `GET /api/status` counts late frames
  (`misses.steady` and, by cause, `misses.by_cause.decode`): compare a
  couple of minutes of your effect with the same time on an idle
  playlist, with the glow off and on.
- **Heap**: the heartbeat in the serial log reports free heap. It should
  read the same with your effect playing as with any other effect.

## Previewing

In the **simulator**:

```sh
make sim ARGS="--root /path/to/card --play '@fx:plasma?palette=rainbow'"
```

On a machine with no display, take the raw 128 × 32 canvas at a moment of
the loop instead:

```sh
cd opendmd-sim && cargo run --release -- --root /path/to/card \
    --play '@fx:plasma?palette=rainbow' \
    --screenshot-canvas /tmp/plasma.png --screenshot-at 1500
```

The root only needs a `filelist.txt`; see [the simulator](simulator.md)
for the rest of its options.

In the **browser**, `make web` and then `make web-serve` (local mode) or
`make web-mock ROOT=/path/to/card` (against a fake player). The effect
shows in the Library's Effects tab once it's registered there (below),
drawn by the same core compiled to wasm. **Save as GIF…** there turns it
into an animation in Create Art, which is a quick way to step through its
frames one at a time.

## Registering a new effect

A new effect touches these places:

1. **`opendmd-core/src/fx.rs`**: the struct and its `impl Effect`; a
   variant in `Fx`; an arm in `Fx::effect`, `Fx::name`, `Fx::title` (what
   the Library and the status call it) and `Fx::delay`; in `Fx::parse`,
   its name with its defaults, its own keys in the options loop, and its
   arm where `common` is written back. Describe it in the module's doc
   comment.
2. **Tests** in the same file: add a path or two to
   `loops_are_whole_and_seamless`; to
   `full_frame_effects_mark_everything_and_fill_the_panel` or
   `sparse_effects_mark_exactly_what_they_leave`, whichever pattern it
   follows; its options to `parse_options`; and to `fx_cost`.
3. **The web app**: a row in the `FX` table in
   `opendmd-web/www/js/live.js`: the name, a short caption, and one entry
   per option, `label: [values, default, key]`. The label is what users
   read beside the select (capitalised for them), and each value can carry
   its own label (`[[1, 'shaded'], [0, 'flat']]`). If an option's value is a
   word that would read well in a GIF's title (a palette, a shape), add its
   key to `WORDS`, which is how **Save as GIF…** suggests "Tunnel Fire
   Rings".
4. **The mock player and the browser tests**: the `FX` table in
   `tools/mock-device.py` (the title, and `base`: the loop's length in
   frames at speed 1 and 50 fps), and `FX_ROWS` and the titles in
   `tools/web-smoke.py`'s Effects checks.
5. **The API reference**: the effect and its options in the "Live items"
   section of the [HTTP API reference](api.md) (`docs/api.md`). Keep it to
   a line or two: that page has a size budget of its own.
6. **The user guide**: a line on the guide's
   [effects page](guide/effects.md) (`docs/guide/effects.md`), in plain
   words.

Then `make test`, `make clippy`, `make web` (for the size budgets) and
`make web-smoke`.

## A worked example: a plasma

The classic demo-scene plasma: three sine waves, across, down and
diagonally, summed at every pixel, the sum picking a palette entry. It is
a full-frame effect, and the whole of its state is `Common`. It goes in
`fx.rs`, before `Fx`:

```rust
// --- the plasma -------------------------------------------------------------

/// `@fx:plasma[?speed&fps&palette&cycle]`: the demo-scene plasma. Three
/// sine waves (across, down and diagonally) are summed at every pixel and
/// the sum picks a palette entry. A loop (4.8 s at speed 2) is one, two
/// and three cycles of the waves and, with `cycle=1`, one turn of the
/// palette.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Plasma {
    pub common: Common,
}

impl Effect for Plasma {
    fn name(&self) -> &'static str {
        "plasma"
    }

    fn loop_frames(&self) -> u32 {
        self.common.frames(480)
    }

    fn frame(&mut self, t: Duration, canvas: &mut Canvas) -> Duration {
        let n = self.loop_frames();
        let p = phase(self.common.index(t, n).0, n);
        let cyc = if self.common.cycle { (p >> 8) as u8 } else { 0 };
        let pal = self.common.palette.table();
        // The wave across depends only on x: once a frame, on the stack.
        let mut across = [0i32; WIDTH];
        for (x, a) in across.iter_mut().enumerate() {
            *a = sin16((x as u32 * 1024).wrapping_add(p));
        }
        let (px, dirty) = canvas.pixels_and_dirty_mut();
        for y in 0..HEIGHT {
            let down = sin16((y as u32 * 2048).wrapping_add(2 * p));
            for x in 0..WIDTH {
                let diag = sin16(((x + 2 * y) as u32 * 512).wrapping_sub(3 * p));
                // -49,152..=49,152 -> 0..=255, without a division.
                let v = (((across[x] + down + diag + 49_152) * 171) >> 16).min(255);
                px[y * WIDTH + x] = self.common.colour(pal, v as u8, cyc);
            }
        }
        // Every pixel written: mark them all (the blit's cheapest path).
        dirty.mark_all();
        self.common.delay()
    }
}
```

What keeps it seamless: the phase `p` runs 0 to 65,536 over the loop and
`sin16` takes 65,536 steps a turn, so `p`, `2 * p` and `3 * p` are one, two
and three whole cycles. The spatial terms are whole too: `x × 1024` is two
cycles across the 128 columns and `y × 2048` one cycle down the 32 rows.
Per pixel there is one table lookup, three adds, a multiply and a shift.
On the desktop it renders in less than half the default tunnel's time.

Then register it in `Fx`:

```rust
pub enum Fx {
    // ...
    Wire(Wire),
    Plasma(Plasma),
}

// In Fx::effect, Fx::name, Fx::title and Fx::delay:
            Fx::Plasma(e) => e,
            Fx::Plasma(e) => e.name(),
            Fx::Plasma(_) => "Plasma",
            Fx::Plasma(e) => e.common.delay(),

// In Fx::parse, the variants by name (it takes the default palette,
// chrono, cycling):
            "plasma" => Fx::Plasma(Plasma { common }),

// ...and where `common` is written back after the options:
            Fx::Plasma(e) => e.common = common,
```

It has no options of its own, so the options loop needs nothing new:
`speed`, `fps`, `palette` and `cycle` are handled for every effect. Add
`"@fx:plasma?palette=rainbow"` to `loops_are_whole_and_seamless` and
`"@fx:plasma"` to `full_frame_effects_mark_everything_and_fill_the_panel`
and `fx_cost`, and it is tested like the others. In the web app, its row in
`live.js` would be:

```js
['plasma', 'the demo-scene classic', { palette: [PAL, 'chrono'], speed: [SPEED, 2], 'colour cycling': [ONOFF, 1, 'cycle'], 'frames/s': [FPS, 100, 'fps'] }],
```

and `make sim ARGS="--root /path/to/card --play '@fx:plasma?palette=rainbow'"`
shows it.
