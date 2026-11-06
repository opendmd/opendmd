# Psi drawing: a frame-time drawing stage, so Formulas replace the built-in effects (design)

The design as written before it was built; the "As built" notes below say
where the build differs. Labels: [M] measured in the repo or on the device,
[D] an estimate (the basis is given each time).

The brief, in two parts. First: extend Psi so the panel's four built-in
effects — Tunnel, Rotozoom, Starfield and Wireframe (then in
`opendmd-core/src/fx.rs`) — can be written as Formulas. Second: the
built-ins should go. Users should see **Formulas**, not a separate
"effects" idea. Nothing was released yet, so no compatibility was owed:
once the Formula versions pass an eye test on a panel, the native code, the
`@fx:<name>` paths and the Effects UI are deleted.

## 0. Summary

- **Tunnel and Rotozoom are per-pixel already** and compile in today's Psi
  (sketches in §4). They cost 3-5× the natives (7-11 ms a frame against
  2.2-3.5 ms [M]) because `len`, `1/v` and `atan2` are 530-690 ns a pixel
  each where the natives read a table. They run at 25-50 fps under the
  governor, not 100. A **still cache** (§3.6: values that depend on the
  pixel's position only, computed once at open into a lane of 4,096 i32,
  16 KB each, identical bytes with or without it) brings the tunnel to
  ~6.5 ms. It is phase 2.
- **Starfield and Wireframe do not fit "each pixel decides for itself"**: a
  cube as twelve per-pixel segment distances is ~40 ms, an icosahedron does
  not fit 256 steps. The recommendation is **(b), a frame-time drawing
  stage**: a `draw { }` block of ordinary Psi lines run once a frame, with
  bounded `for k < N { }` loops over constant `table`s, drawing `line` and
  `dot` primitives into a **4 KB u8 ink canvas** that the pixel stage reads
  as `ink` (0..1, as `ur ug ub ul` read the GIF). Starfield ≈ 4 ms,
  cube/icosahedron ≈ 1.5-2 ms, morph/torus ≈ 5.5 ms (3.3 with phase 2's
  writable `buf`), all [D] from the device's per-step and native line
  figures. Pixel-exact integer Bresenham, Q16.16 scalar maths through
  `fixed`: bit-identical on the ESP32, the host and wasm by construction,
  checked by goldens as everything else is.
- Firmware: the drawing stage is **+5-7 KB** [D]; deleting the native
  effects (24.5 KB of `fx` symbols [M], 13.3 KB of them tables, less the
  sine and palettes Psi borrows) is **−24 KB**, net **−17 to −19 KB**.
  Heap: +4 KB per open formula that draws (PsiBox 10 → 14 KB; the look's
  buffers 21 → 25 KB).
- **Removal (§7):** no compatibility layer — nothing is released. The four
  natives are deleted one by one as their Formulas pass the eye
  test (order: Wireframe, Starfield, Rotozoom, Tunnel), and `@fx:<name>`
  goes from the API, the playlist parsing, the web app and the docs. A
  stale playlist entry is skipped with a log line (the engine does that
  today for any entry that fails to open); a stale `attract` value is made
  harmless at boot. `@clock`, `@text`, `@boot`, `@power-on` and `@card`
  stay native (§7.5); the natives' knobs become each example's lines and
  sliders — no new parameter mechanism, a slider "picker" comment at most.

**As built (phase 1).** Where the build
differs from this design: (1) the canvas is passed to the VM
(`render_rows_ink(…, Some(&mut ink), …)`) and is its own 4 KB allocation,
not a bigger `PsiBox`/`Scratch` (the web module's bump allocator can't hold
a long-lived box: it uses a static); (2) one `draw { }` in the whole
formula (the formula's or one part's), since there is one canvas; (3)
**loop turns count every run of a loop's body, outer loops included** (a
12 × 6 grid is 84 turns, not 72; 32 × 64 is 2,080 and refused): the same
count the VM caps at run time; (4) the 16,384 frame cap counts line steps
walked, not pixels lit, and a line wholly off one side costs nothing; (5)
the draw stage runs on full renders only (a look's partial redraws reuse
the last canvas); (6) E7xx are sentences only (`Error.code` waits for the
tooling plan's P0); (7) the cost constants were measured on a test panel: a
step run 420 ns, a dot 400, a line 1,100 + 320 a pixel (§3.7's 300 /
400 / 1,500 + 200); (8) firmware +15 KB and pkg-psi +6.3 KB gz, against
§3.8's +5-7 KB and +3-4 KB. The sketches below use `x`, `y`, `h` and `i`
as names, which are built-ins: the shipped examples rename them.

**As built (phase 2: everything for replacing the natives, the removal
itself left for after the eye test).** Where it differs from §3.6, §4 and
§7.3:

1. **The still cache** is as §3.6 (a lane of 4,096 i32 per value, up to
   two, chosen greedily by ns saved, each ≥ 400 ns a pixel; identical bytes
   with or without it), laid out as one run after a `STILL` marker that the
   VM skips once the values are kept. It is **filled by the first full
   frame** (that frame runs every step and copies the values out), not at
   open, and taken in the effect's `frame`, not at open: the HTTP server's
   open check never allocates it. A fingerprint of the steps it was filled
   for guards against a live edit drawing from another formula's values.
   Firmware gate: none while a formula look is set; taken only if 32 KB stay
   free (48 in p2probe-a, which kept Tunnel's 32 KB out with ~80-90 KB free)
   and a 16 KB block exists; given back under 24 KB. Looks never have one.
2. **Added beyond the design** (needed to get within the frame budgets): a
   **column run** (values of `x`/`px` and per-frame values, worked out on a
   frame's first row and kept in pinned registers: the rotozoom's column
   wobble, `sin(x*3 + t/8)`); **`pal(P, v) * k`** as one colour step (§3.4's
   "pal() in colour expressions", only this form: the tunnel's light and the
   floor's fog without a multiply layer); **`noise` at a whole z** from four
   corners (same bytes). `compile_plain` lays out without either run (the
   step table is measured and tested that way).
3. **Tunnel** (§4.1): the two still values are the depth and the place
   round the walls *with the twist folded in* (so the twist is a number in
   the source, `twist = 1`, not a slider: a slider would make it move); one
   layer (the native's noise second layer is left out, its average folded
   into the checker's two palette positions, so the colours match frame for
   frame); no bevel; the light is the native's eight steps with the light
   going round. 7.5 ms estimated with the cache (50 fps), 17.6 without.
4. **Rotozoom** (§4.2): flat, two layers (checker + `noise(u, v, 0)`, its
   levels raised to the native's mean brightness: Psi's noise is smoother
   than the baked table), a picker `-- a: wobble = gentle, strong, off`,
   6.0 ms (50 fps); the floor is its own example, **Rotozoom floor** (4.3 ms,
   `fps 100`): one formula with a `view = flat, floor` picker measured 10.6 ms
   on a test panel (the picker made every column part per row). No bevel.
5. **Starfield** rewritten for fewer steps a star (6.0 → 5.0 ms, 100 fps
   by the estimate), the same picture (0.06 % of pixels differ by a
   rounding).
6. **Pickers** (§7.3) are built (web only) and **Add to playlist** for
   Formulas (playlist lines up to 2 KB, `&name=` on the path for the
   name).

**As built (removal).** The removal of §7 was done in one cut, not by the
staged gates of §7.6: nothing was formally released, so there is no
compatibility layer. The four natives
(`@fx:tunnel`, `rotozoom`, `starfield`, `wire`) are gone from the core, the
firmware, the web app, the tools and the docs; they live only as Formulas.
**`@fx:psi` was renamed `@formula`** (`@formula?src=…[&a=&b=&c=][&fps=][&name=]`,
the same query, ≤ 2,048 B). Any other `@fx:` path is retired: skipped with a
log line in a playlist, `415` from `POST /api/play`. **`attract` has no
Formula form** (§7.3's recommendation taken): `fx:` is a `400`, a stored one
reads as `""` with a log line at boot, and a playlist of one Formula chosen as
`attract_set` does the same. The psi probe never compared a Formula against
the natives on the device, so there was no probe code to remove.

## 1. Options compared

The cost figures use the device's measured per-step costs (`psi-probe` and
`Program::esp32_us`): a
per-pixel step is 50-110 ns for arithmetic, 529 `atan2`, 539 `sqrt`, 642
`len`, 685 `/` or `1/v`, 855 `noise`; 45 ns of loop a pixel; a colour 120;
×1.15 for the real paths; 4,096 pixels. A per-row step is counted at ~0.3 µs
(the estimate's `row × 10 µs` over 32 rows); a scalar step run once a
frame is the same `eval` and I use **300 ns** for one [D] (the per-frame
`compile`/`eval` path is the same code as the per-row one). The natives'
own line drawing on the device [M]: starfield 1.35 ms for 192 streaks drawn
twice (erase + draw: ~384 `line` calls of 3-10 px with the hash,
projection and palette each), the wire morph 2.25 ms for 144 edges drawn
twice (288 lines of 8-25 px with projection) — **~3.5-8 µs a line
including everything, ~0.25-0.4 µs a line pixel** through the dirty-map
`get`/`set` path.

### (a) Per-pixel distance built-ins: `seg(x0, y0, x1, y1)`, `dot(x, y)`

A segment's distance is a `len` and a clamp with ~8 arithmetic steps:
**~1.1 µs a pixel an edge**. The endpoints are per-frame (hoisted). Then:

| shape | edges | per-pixel steps | ms a frame |
|---|---|---|---|
| one hand / one streak | 1 | ~10 | ~5.5 (with the colour) |
| cube, octahedron | 12 | ~120 (+ 11 `min`) | **~62** |
| icosahedron | 30 | ~300 | does not fit `MAX_OPS` 256 |
| 192 stars | 192 | — | does not fit |

So (a) is not a route to the wireframes or the stars. It *is* a good
primitive in its own right (a smooth, antialiased hand, a laser, a soft
streak: things the per-pixel model does better than a rasteriser), cheap to
add (one ternary-plus op, ~0.3 KB), and worth adding **later** as `seg`
once the drawing stage exists — but it answers a different question, so it
is not in phase 1.

### (b) A frame-time drawing stage with a canvas (recommended)

Once a frame, before the rows: run a block of scalar Psi with bounded loops
over small constant tables, which rasterises **points and lines** into a
canvas; the pixel stage reads the canvas at its pixel as `ink`. The cost is
*per primitive*, not per primitive per pixel:

| what | ns | basis |
|---|---|---|
| a scalar step inside a loop | 300 | the per-row figure [M→D] |
| a `line` | 1,500 + 200 × its pixels | native `line` without the dirty-map compare, no shade table [D] |
| a `dot` | 400 | [D] |
| the pixel stage reading `ink` | 70 (as `ur`) + the colour | [M] |

The starfield is ~192 × (25 steps + a 6-px line) ≈ 2 ms + ~1 ms of pixel
stage; a cube ~0.3 + 1 ms. The whole of §4 fits under the 8 ms effect
budget at 50 fps and most of it under 5 ms. The canvas costs 4 KB of heap
per open drawing formula; the compiler and VM grow by ~5-7 KB of firmware.
Deterministic by construction (§3.5). It composes with everything Psi has:
the canvas is just another per-pixel input, so layers, coverage, parts,
looks over a GIF, `loop`/`t` all work unchanged.

### (c) Others looked at

- **Per-pixel with a compile-time edge list and per-pixel `min` folding**
  — (a) again; the arithmetic is the cost, not the steps.
- **A persistent canvas** (not cleared each frame: trails, decay, cellular
  rules). Tempting for particles and the time-warp feel, but a frame is no
  longer a pure function of `t`: the governor's held frames, `rewind`, the
  browser's scrub bar and the GIF export (frames drawn in any order) all
  break, and bit-identity becomes path-dependent. Rejected for the language;
  the glow does trails already.
- **A native "shape library"** (`@fx:wire` kept, parameterised harder).
  The direction is the opposite (one concept, Formulas); and every knob
  added natively is a knob the Formula language can't offer.
- **Running the drawing stage per row** (scanline rasteriser in the VM):
  elegant for filled shapes but every primitive is visited 32 times; a
  rasteriser into a canvas is simpler and cheaper.
- **An RGB canvas** (12 KB) so each primitive carries its colour. Three
  times the heap for a feature the pixel stage gives anyway (colour from
  `ink`, `pal(ice, ink)`, `#40c0ff * ink`), and a second u8 canvas (§3.2,
  phase 2) covers the per-particle-hue case for 4 KB. Rejected.

**Recommendation: (b)**, phase 1 as the smallest useful slice (§5), with
(a)'s `seg` as a later per-pixel addition and the still cache (§3.6) as the
thing that makes the per-pixel tunnel fast.

## 2. The language

House rules kept: plain words, no new punctuation beyond `[ ]`, each name
set once, the colour last, errors as sentences with a byte range (plus the
E-codes the tooling plan gives every error).

### 2.1 Grammar additions

```text
stmt      += "draw" "{" { dstmt sep } "}"           -- once a frame, before the rows; one per formula or part
          |  "table" NAME "=" NUMBER { "," NUMBER }  -- constants only; NAME[k] reads entry floor(k)
dstmt      = NAME "=" expr                            -- a value (no x y px py i ur ug ub ul ink in here)
          | "for" NAME "<" NUMBER "{" { dstmt sep } "}"   -- NAME = 0, 1, … NUMBER−1; NUMBER a whole 1..1024
          | "line" "(" e "," e "," e "," e [ "," e [ "," e ] ] ")"   -- x0, y0, x1, y1 [, v0 [, v1]]
          | "dot" "(" e "," e [ "," e ] ")"                            -- x, y [, v]
atom      += NAME "[" expr "]"                        -- a table's entry, 0 outside it
value     += "ink"                                    -- what the draw block put at this pixel, 0..1
```

- `draw { }` goes where `frame { }` goes: at the top level of a formula or
  a part (not inside a layer: the canvas is the formula's, every layer may
  read it). One per formula or part. It runs once a frame after the
  per-frame values and before the first row, on a canvas cleared to 0.
- Inside it, values are *scalar*: `t`, the clock, the sliders, tables and
  the loop counters; the pixel inputs are errors (E709). Names set inside
  a `for` are that loop's own (they may repeat in another loop); names set
  in `draw` outside loops are visible after it, as `frame { }` values are.
- `for k < N` counts `k` from 0 to N−1 as whole numbers; `N` is a literal
  number (so the compiler knows the cost). Loops nest two deep; the product
  of all iterations in a frame is at most **2,048** (E704).
- **Coordinates are pixels, `y` up from the bottom** (as `px`, `py`): `line(0,
  0, 127, 31)` is the diagonal from bottom-left to top-right. Fractions are
  rounded to the nearest pixel centre (§3.5).
- **Ink** `v` is 0..1 (clamped; default 1). A line's ink runs from `v0` at
  its start to `v1` at its end, in 1/255 steps (the natives' eight shades
  were a panel-banding choice; `floor(v*8)/8` gets them back). Primitives
  **lighten**: a pixel keeps the brighter of what was there and what is
  drawn, so crossing lines don't darken and the near edges win over the far
  without sorting (the natives sort for exactly this).
- `ink` is the canvas at the pixel, 0..1, per pixel, like `ur`. A formula
  with `ink` and no `draw { }` is an error (E706: it is always a mistake, unlike
  `ur` alone). Inside a part drawn elsewhere (`p(x', y')`), `ink` is still
  the *real* pixel's: the canvas is not relocated (phase 2's `ink(x', y')`
  would be).
- `table NAME = …` at the top level of a formula or part: whole or decimal
  constants, up to **256 entries in all** per formula (E707). `NAME[k]`
  reads entry `floor(k)`, 0 outside the table, at any level (per pixel it is
  one step, ~100 ns like `pick`: a user's own bitmap or lookup, the "pack
  rows into a number" idiom made proper). `table` and `draw`, `for`, `line`,
  `dot`, `ink` join the reserved words; a formula saved with `line = …`
  keeps working through the `LATER` rule for `line`, `dot` and `ink` (a
  name a formula sets is that name), but `draw`, `for` and `table` are
  keywords (`psi 2`? No: a formula saved today never starts a line with
  them — they are new statements, and the corpus test proves nothing
  compiled before changes).

### 2.2 Examples of the shape

A streak field, the smallest drawing formula:

```psi
loop 2.4
draw {
  for k < 192 {
    h = hash(k, 7); z = frac(hash(k, 3) - t/2.4)      -- a star's place and its depth, 0 near
    s = 1/(z*16 + 1)                                  -- perspective
    x = 64 + (h - 0.5)*2048*s/64; y = 16 + (hash(k, 11) - 0.5)*1024*s/64
    line(x, y, 64 + (x - 64)*0.8, 16 + (y - 16)*0.8, 1 - z, (1 - z)*0.2)
  }
}
pal(ice, ink)
```

A cube: tables for the model, two loops, projection written once as a part's
values would be — but parts have no loops, so the projection is written out
per endpoint (see §4.4 for the full sketch and §3.4 for `buf`, the phase 2
answer).

Over a GIF (a Formula as a look): the same block, then `layer screen {
gray(ink) }` — the stars screened onto the GIF. `look::compose` wraps a
drawing formula exactly as any other (the part keeps its `draw`), so "Use
as the look" needs nothing new.

### 2.3 Errors (E7xx, after the tooling plan's E0xx-E6xx)

| code | sentence (house style) |
|---|---|
| E701 | `draw { }` goes at the top of the formula (or a part), not inside a layer |
| E702 | there can be one `draw { }` block |
| E703 | `for` takes a name and a count, e.g. `for k < 12 { … }` (a whole number from 1 to 1024) |
| E704 | this formula draws too much a frame (N loop turns in all; the limit is 2048) — fewer or shorter loops |
| E705 | `for` loops go two deep at most |
| E706 | `line` and `dot` go inside `draw { }` / `ink` needs a `draw { }` block to read from |
| E707 | a table holds numbers only, up to 256 in all |
| E708 | inside `draw { }` only values that are the same for every pixel (no `x`, `y`, `px`, `py`, `i`, `ur ug ub ul`, `ink`) |
| E709 | `k` is a loop counter: it can't be set (or: `k` is already a loop counter here) |
| E710 | `[ ]` reads a table: `NAME[k]` — `v` is not a table |

(E708's numbering follows the pattern of E301 "per-pixel value in `frame {}`".)

## 3. VM and compiler changes

### 3.1 The program

`Program` gains a fourth section and a table area:

```text
ops[..frame_end]              per frame (as today)
ops[frame_end..draw_end]      the draw stage: scalar steps, FOR/END, LINE_FROM/LINE_TO, DOT     NEW
ops[draw_end..row_end]        per row
ops[row_end..pixel_end]       per pixel, colours included
tables: [i32; 256]            `table` constants, Q16.16                                        NEW (+1,024 B)
```

Still plain data (an all-zero value is valid), still one `Box` at open;
`size_of::<Program>()` 1,824 → ~2,860 B (the retune's temporary box too).
`MAX_OPS` stays 256 for everything together; the draw steps count once each
(a loop body is written once), their *cost* counts per iteration. The
sketches in §4 use 60-110 steps.

New op codes (all fit the `Op { code, d, a, b, c }` shape; operands are
scalar slots in the draw stage):

| op | meaning |
|---|---|
| `FOR d, a, b, c` | `d` = the counter slot (set to 0); `a` = the slot holding the count; `(b, c)` = the END's index (lo, hi) |
| `END a, b, c` | counter slot `a` += 1; if < count slot `b`, jump to index `c` (the FOR + 1) — the FOR's `d` and `a` repeated so each is self-contained |
| `TAB d, a, b, c` | `d` = `tables[b + floor(a)]` if `0 ≤ floor(a) < c` else 0 — any level; per pixel a `run1` loop like `PICK` |
| `LINE_FROM a, b, c` | the pending line's start x, y and ink v0 |
| `LINE_TO a, b, c` | its end x, y, v1: rasterise |
| `DOT a, b, c` | x, y, v |
| `INK d` | per pixel, like `U_R`: `d` = canvas byte at the pixel, `fixed::byte` |

The draw stage interpreter is a small loop with a program counter over
`ops[frame_end..draw_end]` (today's three stages have none: they run
straight through). Iteration is bounded by the compile-time check (E704)
and, belt and braces, a run-time cap of 2,048 END jumps a frame.

### 3.2 The canvas

**Format:** 128 × 32 **u8**, 4,096 B, row-major from the top as the frame
is; 0 = nothing drawn. `INK` reads it with `fixed::byte` (exactly 0..1,
`chan` round-trips), so `gray(ink)` draws the byte back. Write rule: `max`.
Cleared with `fill(0)` at the start of every frame that has a draw stage
(a 4 KB memset: ~10 µs). An RGB canvas was rejected (§1c); a **second u8
canvas** (`ink2`, phase 2: a per-primitive hue or depth) is the same shape
again, +4 KB, only for formulas that name it.

**Where it lives:** in `Scratch<N>` as `Option`-like: `Scratch` is built in
place in the PsiBox allocation; a formula without `draw` gets a box the
size it has today (10,000 B); with `draw`, the box is 4,096 B bigger
(`PsiBox::new(prog.draws())` after compiling — the program is compiled into
a temporary first, which `retune_from` already does). The look
(`FormulaLook`'s `LookBuffers`, 20,480 B) likewise takes 24,576 B when the
look draws; its `TAKE_HEAP` gate (48 KB free) stays. On the ESP32 that is
within what the device has when a formula plays (~67-77 KB free [M]) and
within the look's measured minimums (43-45 KB free with the look held [M]);
the still cache (§3.6) is the one that needs a gate of its own.

**Not on the stack:** nothing of this touches the main task's stack (the
draw stage's working values are scalar slots in `Scratch.sc`; the
rasteriser's locals are a dozen i32).

### 3.3 Limits

| limit | value | why |
|---|---|---|
| `for` count | 1..1024, a literal | the cost must be known at compile time |
| iterations a frame (product of nested counts, summed over loops) | 2,048 | 2,048 × a 25-step body = 15 ms: already beyond any budget; a guard, not a target |
| loop depth | 2 | a grid (12 × 6) is the real need; three deep is a cost explosion |
| table entries | 256 in all | 1 KB of Program; the icosahedron is 36 + 60 |
| line length | 1,024 Bresenham steps (as the native) after clamping endpoints to −512..639 × −512..543 | bounds the loop; a line clipped to the panel is ≤ 160 steps anyway |
| line pixels a frame | 16,384 plotted (the canvas four times over) | after it, primitives are skipped for the rest of the frame — deterministically, the same everywhere |
| primitives a frame | 2,048 (one per iteration is the natural shape) | with the above, worst case ~16,384 × 200 ns + 2,048 × 1.5 µs ≈ 6.4 ms of drawing |
| draw steps | within `MAX_OPS` 256 | one program |

### 3.4 Phase 2 additions already shaped

- **`buf NAME[N]`**: a writable per-frame array (N ≤ 64, 256 entries in all
  with the tables, in `Scratch`: +1 KB), `NAME[k] = expr` inside `draw`,
  read anywhere after. Cleared each frame (pure function of `t` still).
  Projected vertices once, edges read them: the torus halves (§4.4).
- **`disc(x, y, radius, v)`**: filled, scanline, deterministic (integer
  radius test on pixel centres). Particles with size, planets, the
  bouncing-ball idiom for 50 balls.
- **`ink2`** (second canvas) and **`ink(x', y')`** (sampling elsewhere: a
  part drawn at another place, mirror effects).
- **`seg(x0, y0, x1, y1)`** per pixel (§1a).
- **`pal(P, k)` inside a colour expression** (not drawing, but §4.1 hits
  it: `pal(chrono, v) * light` is an error today, so the tunnel's light is
  a `multiply` layer at +365 ns a pixel).

### 3.5 Determinism and bit-identity

The rule today: every value is Q16.16 through `fixed`, every table is a
`static` built by a `const fn`, the same crate runs everywhere; the goldens
(host, wasm, `psi-probe` on the device: 41/41 [M]) are the test. The drawing
stage keeps to it:

- **Endpoints:** `px = (x + ONE/2) >> 16` (floor of x + ½: the nearest
  pixel centre, ties up), then clamped to the range above. Integer from
  here on.
- **Bresenham** exactly as `fx::line` (integer `err`, `dx`, `dy`; `steps =
  max(dx, −dy, 1)`), including its order of the two `if`s; ink interpolated
  as `k += dk` with `dk = ((v1 − v0) << 16) / steps` in i32 — integer
  division, truncating, the same on every target. The plotted byte is
  `chan(k)`.
- **Scalar maths** in the loops is `eval`, the per-row `eval`: `fixed::div`
  for `/`, saturating adds. No new arithmetic.
- **No floats, no `f32` anywhere** (the ESP32's FPU and wasm's would not
  round alike; the codebase has none in Psi today and the rasteriser adds
  none).
- **Order of writes** is the program's order; `max` is commutative anyway.
- **The caps** (16,384 pixels, 2,048 iterations) are counted identically
  everywhere, so a formula that hits one draws the same partial picture on
  every target. The estimate refuses before the caps matter.
- **Goldens:** new frames for lines in all eight octants and both
  directions, ties at .5, clipping on every edge, a line longer than the
  cap, the `max` rule, `dot` at fractional coordinates, a table read at
  every level, a `for` of 1 and of 1,024, two loops nested, `ink` through a
  part and a look. Host + wasm (`make test-wasm`) + `psi-probe`.

### 3.6 The still cache (phase 2, for the per-pixel effects)

A new level between constant and per-row: **still** — a value whose inputs
are only `x y px py i` (and constants): the same every frame. Today it is
computed per pixel every frame. Cached, it is a lane of 4,096 i32 (16 KB)
filled once when the formula opens or is retuned, read per pixel like `x`
or `px` (free: a lane operand). The compiler marks still nodes; `emit` lays
the first `S` of them (by saved ns) out as cache lanes when `S` is allowed
(the box is built for `S` lanes: 0 on a device under its heap gate, up to 2
on the host and wasm — **the bytes are identical either way**, only the
time differs, so it is an optimisation the device may decline). `esp32_us`
reports both figures; the device's governor sees the real one. Tunnel's
`1/len(cx, cy*2)` and `atan2(cy, cx)` are the two it needs (−1,856 ns a
pixel: 7.5 → ~3.5 µs... see §4.1), `r`/`angle`-based rings, radar and
kaleidoscopes gain the same. Heap on the device: 16-32 KB taken like the
look's buffers (`TAKE_HEAP`-style gate, yielded for the glow), so a formula
that opens without them simply runs slower, as today.

### 3.7 Cost model additions

`esp32_us` adds, per `FOR` loop with count `n` (nested: the product):

```text
draw_ns = Σ_loops n × ( Σ body scalar steps × 300 + Σ LINE_TO × (1,500 + 200 × L̂) + Σ DOT × 400 )
          + 10,000 (the canvas clear)
```

with `L̂`, the guessed line length, **12 px** unless both endpoints'
difference is a constant (then its length). The per-pixel side adds `INK`
at 70 ns (as `U_R`) and `TAB` at 100 (as `PICK`). The refusal thresholds
(`MAX_EST_US` 40 ms, the governor's slowest step, `LOOK_MAX_EST_US` 12 ms)
and the editor's light/medium/heavy need no change. `psi-probe` gains:
`step tab`, `step ink`, `draw loop step` (an empty 1,024-turn loop with one
add: the 300 ns), `draw line 12`, `draw line 100`, `draw dot` — and the
constants above are replaced by what it measures, as the layer costs were
(`tests/measured.rs`, `tests/estimate.rs` hold the model
within ±15 %).

### 3.8 Compiler

- Parser: `draw`, `for`, `table` in `body()` beside `frame`/`layer`/`part`;
  `line`/`dot` as statements inside `draw` only (not expressions); `[`/`]`
  tokens; `ink` as an `Input` (level pixel, uses bit `INKED`).
- Levels: `LV_FRAME` < **`LV_LOOP1` < `LV_LOOP2`** < `LV_ROW` < `LV_PIXEL`
  (the still level slots in before `LV_FRAME` in phase 2). A value that
  does not depend on the counter is hoisted out of the loop by the existing
  rule — `sin(t/4.8)` written inside the loop runs once a frame, as it
  should. Loop counters are `K_INPUT`-like nodes bound to a slot.
- `emit`: a fourth pass lays the draw stage out in source order (loops must
  keep their order and their bodies contiguous; the DAG's sharing still
  applies within a body), FOR/END with the END's index patched; a
  primitive's six operands become two ops.
- Stack: the parser's recursion gains one level for `draw` and one per
  `for`; the house limit (`MAX_DEPTH` 18, 12 KB compile thread, ~2.2 KB
  untouched at the worst [M]) is checked again with `worst::draw` in
  `psi-probe` and `tests/stack.rs`. Nothing in the rasteriser recurses.
- Firmware size [D]: parser and errors ~3-4 KB (the E7xx sentences are
  ~700 B of them), VM loop + rasteriser ~1.5 KB, `esp32_us` ~0.3 KB,
  Program +1 KB of `tables` is heap not flash. **+5-7 KB**; wasm `pkg-psi`
  +3-4 KB gz (budget 45 → 49 KB, to approve).

## 4. The four built-ins as Formulas

Each: the sketch, its estimated steps and ms a frame on the device, and
what differs from the native. The natives' maths is in `fx.rs` and the
"Live effects" table of CLAUDE.md; where a sketch departs from it the
reason is given. Costs: per-pixel from `esp32_us`'s constants; draw-stage
from §3.7. None was compiled (the `draw` syntax does not exist yet); the
per-pixel ones are today's language and will be run through `psi-check`
when the branch is taken up.

### 4.1 Tunnel (per pixel, today's language)

```psi
loop 4.8
-- Tunnel: the time warp. One turn, one texture length of travel and one
-- palette turn a loop (speed 2). a: twist (0..3)
frame { tw = floor(a*3.99) }
-- an ellipse twice as tall as wide: cy is -0.5..0.5, cx -2..2
d = len(cx, cy*2)                     -- distance from the vanishing point, panel heights
depth = 1/d                           -- quarter texels / 128: 1 at d = 1
-- the texture goes round twice; travels one length a loop; twists with depth
u = frac(angle*2 + t/4.8 + depth*tw/32)
v = frac(depth/4 + t/4.8)             -- 32 texels deep: a length a loop
cell = (floor(u*8) + floor(v*4)) % 2  -- the checker: 8 x 4 cells of 8 texels
tex = mix(0.28, 0.78, cell)           -- the palette indices 72 and 200, of 256
-- the second layer: rings under it, four times round, half as deep, the other way
u2 = frac(angle*4 - t/4.8); v2 = frac(depth/8 - t/4.8)
tex2 = frac(len(u2 - 0.5, v2 - 0.5)*2)
val = tex*0.75 + tex2*0.25
-- lit by depth (full past d = 0.67, the square of a ramp to black at 0.25)
-- and by a side light going round the other way (0.6..1), in 8 steps
ramp = clamp((d - 0.25)*2.4, 0, 1)
light = floor(ramp*ramp*(0.8 + 0.2*cos(angle + t/4.8))*8)/8
layer { pal(chrono, frac(val + t/4.8)) }
layer multiply { gray(light) }
```

Per pixel (`cx` 2, `len` 642, `1/d` 685, `atan2` 529, ~22 cheap ops ≈
1,250, the second `len` 642, `cos` 110, `pal` 120+5, the multiply layer
245+120): **≈ 4.4 µs → ~21 ms** [D]. Without the second layer (`layers=1`,
−~900 ns): ~16 ms. With the still cache for `depth` and `angle` (§3.6,
−1,856): **~12 ms with two layers, ~8 with one**; and with `tex2` as a
`table` ring strip instead of a `len` (one `TAB` + 3 ops: −500) and
`pal(…) * light` allowed (−365): **~6.5 ms**. ~70 steps.

**What differs:** the native is 3.45 ms at 100 fps [M]; this is 25 fps
today, 50 with phase 2 — on a 4.8 s loop the motion is still smooth, but it
is an eye test on the panel that decides (§7.6). The textures: the native's checker
has a one-step bevel, its rings are 1/16-texel bands, its noise is a baked
64 × 32 table; the Formula's checker is flat, its rings are `len` rings,
and `texture=noise` becomes `noise(u*8, v*4, 0)` (+855 ns) — the same
*kind* of picture, not the same bytes. The fog's four levels become a
ramp; `drift=1` is a `frame { }` offset of `cx`/`cy` (two adds, free).
`cycle=0` drops `+ t/4.8` from the palette index.

### 4.2 Rotozoom (per pixel, today's language)

```psi
loop 9.6
-- Rotozoom: Mode 7. One turn a loop, the zoom breathing 0.5x..1.5x twice,
-- the centre panning two textures across and one down; a second texture
-- turning the other way twice as fast at 3/4 the scale. a: wobble (0..3)
frame {
  s = sin(t/9.6); c = cos(t/9.6)
  sc = 1/((1 + sin(t/4.8)*0.5)*2)     -- texels a pixel at zoom 2
  cu = t/9.6*128; cv = t/9.6*64
  s2 = -sin(t/4.8); c2 = cos(t/4.8); sc2 = sc*0.75
  amp = floor(a*3.99)*1.5
}
-- the row's wobble (sideways) and the column's (up and down): waves that travel
wob = sin(py/32 + t*3/9.6)*amp
col = sin(px*1536/65536 - t*2/9.6)*amp
dx = px - 64; dy = py - 16
u = cu + (c*dx - s*dy)*sc + wob
v = cv + (s*dx + c*dy)*sc + col
tex = (floor(u/8) + floor(v/8)) % 2
u2 = cv + (c2*dx - s2*dy)*sc2 + wob
v2 = cu + (s2*dx + c2*dy)*sc2 + col
tex2 = noise(u2/8, v2/8, 0)
pal(fire, (mix(0.28, 0.78, tex) + tex2)/2)
```

Per pixel: `col` (`sin` 110 + 2), `u`/`v` (6 mul 516, 6 add), two
`floor`, `%`, `mix`, the second layer's 6 mul + 6 add and `noise` 855,
`pal` 125: **≈ 2.9 µs → ~14 ms** [D]; with `tex2` a checker too (no
`noise`): ~9 ms; one layer (`layers=1`): **~5 ms**, 100 fps-class. ~45
steps. `floor=1` is a per-row `sc = 8/((ty + 2)*zoom)` and a fog by `ty`
(free: per row), one layer: ~5 ms.

**What differs:** native 2.15 ms [M]. The second-layer noise texture is
`noise()` (smooth, not the baked 8-texel lattice). The native's u/v are
16.16 wrapped to 64 × 32 texels; here `floor(u/8) % 2` wraps implicitly.
The checker's bevel is gone. Otherwise the same picture; the two-layer
default is the heavy case.

### 4.3 Starfield (the draw stage)

```psi
loop 2.4
-- Starfield: 192 stars from a hash, flying at the viewer through 4,096
-- depth units a loop; each a streak from where it was, fading along it.
-- a: density (96, 192, 288) b: streaks (0: dots)
frame { n = pick(floor(a*2.99), 96, 192, 288); back = 48 + 48*2 }
draw {
  for k < 288 {
    on = k < n
    h = hash(k, 7)
    sx = (h - 0.5)*4096; sy = (hash(k, 3) - 0.5)*1024          -- ±2,048, ±512
    z = frac(hash(k, 11) - t/2.4)*4096                           -- 0 near, 4,096 far
    zz = z/16 + 4
    x = 64 + sx*2/zz; y = 16 + sy*2/zz
    -- its tail: where it was `back` depth units ago (longer the faster)
    zt = min(z + back, 4095)/16 + 4
    xt = 64 + sx*2/zt; yt = 16 + sy*2/zt
    bright = (1 - z/4096)*on                                     -- fades in from black
    line(mix(x, xt, b > 0), mix(y, yt, b > 0), x, y, bright*0.19, bright)
  }
}
pal(ice, ink)
```

Draw stage: 288 × (~24 scalar steps: 3 `hash`, 4 `/`, the rest cheap →
~24 × 300 = 7.2 µs, plus a 6-px line 2.7 µs) ≈ **2.9 ms**; the 96 unlit
stars still cost their body (a `for` count is a literal: `n` can't be
one) — a `for k < 192` without the density slider is 1.9 ms. Pixel stage:
`INK` 70 + `pal` 125 + 45 = 240 × 4,096 = 1.0 ms. **≈ 3.9 ms (4.5 at
density 3)** [D], ~40 steps. Native: 1.35 ms (2.2 at density 3, speed 4)
[M]. 100 fps-class at the default density.

**What differs:** the native colours a star `palette[(255 − z/16)/8]` and
scales that by the streak's ramp; here the ramp goes into `ink` and the
palette is read once (`pal(ice, ink)`): a streak's tail is the palette's
*darker entries*, not the head's colour dimmed — on `ice` that is navy
instead of dim blue, which reads as more depth, and is the one visible
difference. The hash differs (`fixed::hash` vs `fx::hash`), so the stars
are *a* field, not *the* field. Off-panel heads are clipped by the
rasteriser instead of skipped (same picture). The native's `speed=` sets
the loop and the streak length; here `loop` and `back`.

### 4.4 Wireframe (the draw stage): cube, octahedron, icosahedron, tesseract, torus, morph

```psi
loop 4.8
-- Wireframe: a cube turning once about y and once about x (a sixth of a
-- turn behind) a loop while it sweeps across (±40 px); each edge lit by
-- its depth. a: shape (cube, octahedron, icosahedron)
table CUBE_V = -34, -34, -34,  34, -34, -34,  34, 34, -34,  -34, 34, -34,
               -34, -34, 34,   34, -34, 34,   34, 34, 34,   -34, 34, 34
table CUBE_E = 0, 1, 1, 2, 2, 3, 3, 0, 4, 5, 5, 6, 6, 7, 7, 4, 0, 4, 1, 5, 2, 6, 3, 7
frame {
  sa = sin(t/4.8); ca = cos(t/4.8)
  sb = sin(t/4.8 + 1/6); cb = cos(t/4.8 + 1/6)
  cx0 = 64 + sa*40
}
draw {
  for e < 12 {
    -- the two ends: vertex numbers, then model coordinates
    i0 = CUBE_E[e*2]*3; i1 = CUBE_E[e*2 + 1]*3
    x0 = CUBE_V[i0]; y0 = CUBE_V[i0 + 1]; z0 = CUBE_V[i0 + 2]
    x1 = CUBE_V[i1]; y1 = CUBE_V[i1 + 1]; z1 = CUBE_V[i1 + 2]
    -- turn about y, then x; w = z + 128; screen = centre + (x, y) * 36 / w
    xa = x0*ca + z0*sa; za = z0*ca - x0*sa
    ya = y0*cb - za*sb; zb = y0*sb + za*cb
    xc = x1*ca + z1*sa; zc = z1*ca - x1*sa
    yc = y1*cb - zc*sb; zd = y1*sb + zc*cb
    line(cx0 + xa*36/(zb + 128), 16 + ya*36/(zb + 128),
         cx0 + xc*36/(zd + 128), 16 + yc*36/(zd + 128),
         clamp(0.6 - zb/90, 0.16, 1), clamp(0.6 - zd/90, 0.16, 1))
  }
}
#40c0ff * ink
```

Per edge: 8 `TAB`, 2 `/`, 16 `mul`, ~20 cheap, 2 `clamp` ≈ 48 steps ×
300 ns = 14 µs + a ~14-px line 4.3 µs ≈ **18 µs**. Cube/octahedron 12 edges
**0.22 ms**, icosahedron (12 vertices as 36 numbers, 30 edges as 60: 96
table entries) 30 × 18 = **0.55 ms**, tesseract (16 vertices × 4 numbers,
32 edges, plus the x-w turn and the w-projection: ~70 steps an edge) 32 ×
25 = **0.8 ms**. Pixel stage: `INK` 70 + the colour expression (`#40c0ff *
ink`: two `mul`, a channel of 1 free, 172) + `rgb` 120 + 45 = 407 ×
4,096 = **1.7 ms** (`gray(ink)` would be 1.0: the colour costs more than
the drawing). Totals: **cube ≈ 2.2 ms, icosahedron ≈ 2.6, tesseract ≈
2.9** [D], 100 fps-class. ~70-80 steps.

The **torus and the morph** are a 12 × 6 grid with a torus's topology, 72
vertices computed from (section, ring) with `sin`/`cos` (~16 steps a
vertex) and 144 edges. Written as above, each edge computes both ends:
144 × (2 × 16 + 48 + line) × … ≈ 144 × 33 µs = **4.8 ms + 1.7 = 6.5 ms**
(medium, 50 fps). With phase 2's `buf`:

```psi
buf PX[72]; buf PY[72]; buf PV[72]
draw {
  for s < 12 { for r < 6 {
    k = s*6 + r; th = s/12; ph = r/6 + 1/12
    tube = small*(1 + sin(th*4)*petal)          -- the flower: petals on the tube
    rad = big + tube*cos(ph)
    x = rad*cos(th); y = tube*sin(ph); z = rad*sin(th)
    … the turn and the projection as above …
    PX[k] = sx; PY[k] = sy; PV[k] = lit
  } }
  for k < 72 {
    s = floor(k/6); r = k % 6
    n1 = s*6 + (r + 1) % 6; n2 = (s + 1) % 12*6 + r   -- around the tube; along to the next section
    line(PX[k], PY[k], PX[n1], PY[n1], PV[k], PV[n1])
    line(PX[k], PY[k], PX[n2], PY[n2], PV[k], PV[n2])
  } }
```

72 × 35 steps + 144 × (8 + a line) ≈ 0.76 + 0.3 + 0.6 = **1.7 ms + 1.7 =
3.4 ms** [D], ~110 steps. The morph's (R, r, petal) by thirds of the loop
is a `frame { }` of a few `pick`s and a `smoothstep` ease. Native: morph
2.25-2.75 ms, torus 2.25, icosahedron 0.95, tesseract 1.05 [M].

**What differs:** the native lights a line pixel by its interpolated depth
in eight shades and draws the far half of the edges first; here the ink
ramps in 255 steps (or `floor(v*8)/8` for the bands) and `max` resolves
crossings — a near edge over a far one looks the same, two edges of equal
depth crossing are indistinguishable from the native. `shade=0` is `line(…,
1, 1)`. `color=` is the last line. The six shapes are six example Formulas
(or one with a `pick`ed table, phase 2's `buf` makes that clean), not one
with a `shape=` option (§7.3).

### 4.5 In one table

| effect | Psi today / phase | est. ms a frame [D] | native ms [M] | fps on the device | steps | heap |
|---|---|---|---|---|---|---|
| Tunnel, two layers | today | ~21 (12 with the still cache; 6.5 with cache + table rings + `pal` × light) | 3.45 | 12.5 → 25 → 50 | ~70 | 10 KB (+32 KB cache) |
| Tunnel, one layer | today | ~16 (8 cached) | 3.15 | 25 → 50 | ~55 | " |
| Rotozoom, two layers (noise) | today | ~14 | 2.15 | 25 | ~45 | 10 KB |
| Rotozoom, one layer / floor | today | ~5 | 1.65 | 100 | ~30 | 10 KB |
| Starfield 192 / 288 | phase 1 | ~3.9 / 4.5 | 1.35 / 2.2 | 100 | ~40 | 14 KB |
| Wire cube, octahedron | phase 1 | ~2.2 | ~0.7 | 100 | ~75 | 14 KB |
| Wire icosahedron | phase 1 | ~2.6 | 0.95 | 100 | ~80 | 14 KB |
| Wire tesseract | phase 1 | ~2.9 | 1.05 | 100 | ~90 | 14 KB |
| Wire torus, morph | phase 1 / phase 2 (`buf`) | ~6.5 / 3.4 | 2.25-2.75 | 50 / 100 | ~110 | 14 / 15 KB |

Every figure here is to be replaced by `psi-probe` on a device before any
native effect is removed (§7.6).

## 5. Phased plan

### Phase 1 — the smallest shippable slice

1. **Language and compiler:** `draw { }`, `for k < N { }` (two deep),
   `table`, `NAME[k]`, `line`, `dot`, `ink`; the E7xx errors; levels
   `LV_LOOP1/2`; the fourth `emit` pass; `Program.tables`, `draw_end`; the
   `LATER` rule for `line dot ink`.
2. **VM:** the draw-stage interpreter, `TAB`, `INK`, the rasteriser
   (`line`, `dot`, the caps), the canvas in `Scratch` (sized at open), the
   per-frame clear.
3. **Core/firmware:** `PsiBox::new(draws)` and `retune_from` resizing (a
   retune that starts drawing re-boxes: a 14 KB allocation between frames,
   as an open does), `LookBuffers` + 4 KB when the look draws, `esp32_us`
   additions, `psi-probe` steps (`tab`, `ink`, loop step, line 12/100,
   dot), `psi_stack`'s worst case `draw`.
4. **Tests:** goldens (§3.5) host + wasm + device; `tests/estimate.rs` and
   `measured.rs` extended; fuzz (random `draw` blocks: no panic, the caps
   hold, both lane widths); `tests/stack.rs`; `models.rs` compiles every
   new ```psi block; `every_example_as_a_look` with the drawing examples.
5. **Examples:** Starfield and Wireframe (cube; the morph when `buf` lands)
   in `formulas.json`, with sliders named; Wireframe uses `#40c0ff * ink`
   so the colour is one edit.
6. **Web:** `pkg-psi` rebuild (budget 45 → 49 KB gz [D]); the editor's
   cheat sheet (`draw`, `for`, `table`, `line`, `dot`, `ink`), highlighter
   classes for the new words, `info()` bit for "draws" (so the Formulas
   view can say "draws N lines a frame"); Formulas-page budget 22 → 23 KB.
7. **Docs:** `docs/psi-for-models.md` §2 grammar, §4 (`ink`), §10 limits,
   §11 cost table (the draw formula), §12 idioms ("Particles", "A
   wireframe"), two worked examples with goldens; `docs/guide/formulas.md`
   "Drawing"; `docs/api.md` unchanged (the path is the same `@fx:psi`).
8. **Firmware size:** measured before/after with `make fw-build`; expected +5-7 KB against
   the 4 MB layout's ~271 KB spare (floor 64 KB) — no squeeze needed, and
   §7.4's removal pays it back twice over.

**Device probe steps** (on a test panel): a `psi-probe` build; the new golden frames
bit-identical; the new step costs; Starfield and Wireframe cube as effects
for 120 s each at 100 fps (misses, `frame_us`, heap before/after); as looks
over the demo card for 5 min with Dissolve 400 (transitions cut, misses);
a live-edit burst that toggles `draw` on and off (the re-boxing path).

### Phase 2

`buf`, `disc`, `ink2`, the still cache with its heap gate, `pal()` in
colour expressions, `seg`; Tunnel and Rotozoom examples; Wireframe morph,
torus, tesseract, icosahedron, octahedron examples; the eye tests and the
removal (§7.6). Measured on test panels.

### Phase 3 (if wanted)

`ink(x', y')`, filled triangles, a `for` over a table's length, the
tooling plan's hover/cost-per-line for draw loops ("× 192").

## 6. Risks and open questions (at design time)

1. **Speed of the per-pixel pair.** Formula Tunnel and Rotozoom run at
   25 fps now (two layers) and ~50 with phase 2's still cache; the natives
   run at 100. Is 50 fps acceptable for removing them — and is the still
   cache's 16-32 KB of heap (gated, yielded for the glow like the look's)
   acceptable on a device with ~67 KB free while a formula plays?
2. **One ink canvas (4 KB, value only; colour from the pixel stage) in
   phase 1** — or `ink2` from the start (8 KB) so a particle can carry its
   own hue? Recommendation: one, and see what the gallery asks for.
3. **Lighten (`max`) as the only write rule**, no antialiasing, no
   persistent canvas. Recommended: it is what the panel and the natives do,
   and it keeps frames pure functions of `t`. A `min`/`add` rule is a
   one-word addition later if wanted.
4. **Syntax:** `draw { for k < 12 { … } }`, `table NAME = …`, `NAME[k]`,
   `line(x0, y0, x1, y1, v0, v1)` in pixels with y up. Agree the words
   before the parser is written (they are hard to change after the first
   share link).
5. **Removal pace (§7.6):** delete each native the moment its Formula
   passes the eye test (four small deletions), or all four at once after
   the last passes (one clean cut, the Effects tab useful as the comparison
   tool until then)? Recommended: one cut, after the Tunnel.
6. **Sliders as pickers (§7.3):** a comment convention `-- a: shape = cube,
   octahedron, icosahedron` the web app renders as a segmented control.
   Cheap (web only). Yes? And is a fourth slider `d` wanted, or do the
   examples live within three?
9. **The Formula path's name:** keep `@fx:psi?src=` or rename it to
   `@formula?src=` when `fx` means nothing else (cosmetic; touches every
   saved look and share link, of which there were none yet).
7. **Risk — the estimate.** Loop-step and line costs are [D] until the
   probe runs; if a scalar step is 500 ns rather than 300 the starfield is
   ~5 ms, the morph without `buf` ~9. The thresholds and governor absorb
   it; the eye test might not.
8. **Risk — compile stack.** `for` bodies add recursion; measured on the
   device before merge (the 12 KB thread has ~2.2 KB of headroom today).

## 7. Removing the built-in effects

The direction: one concept, **Formulas**, and no backwards
compatibility — nothing was released and the only users were test
devices. So: no alias table, no `@fx:<name>` rewriting, no query options kept
alive. Each native effect is **deleted** once its Formula passes the eye
test, and `@fx:` goes from the API, the playlist parsing, the web app and
the docs. `@fx:psi?src=…` (the Formula path) stays as it is — renaming it
(`@formula?src=`) is a separate, cosmetic decision (§7.6).

### 7.1 What goes, where

| place | today | after |
|---|---|---|
| `opendmd-core/src/fx.rs` | `Tunnel`, `Rotozoom`, `Starfield`, `Wire`, their tables, `line`, `Fx::parse` of the four names | `Effect`, `Fx::Psi` only; `SIN`, `PALETTES`, `set_frame_floor`/`floor`, `glow_floor` kept (Psi and the glow use them); `TABLES`, `DEPTH_LIGHT`, the meshes, `Texture`, `Shape`, `Palette::from_name` deleted (`Palette` stays as the five tables' names) |
| `synth::parse` | `@fx:<name>` → `Spec::Fx` | only `@fx:psi` (`is_formula`); any other `@fx:` is an unknown live item: `InvalidData`, as `@fx:nonsense` is today |
| `firmware/src/nvs_config.rs` | `attract` = `"fx:<effect>[?options]"` accepted (`valid_attract`, ≤ 205 B) | `"fx:"` dropped; see §7.3 for a Formula as Follow's attract item |
| `firmware/src/http.rs`, `docs/api.md` | `@fx:<effect>` paths in `/api/play`, `attract` values | the paragraph and the value form removed; `@fx:psi` documented as the Formula path |
| web app | Library › Effects tab (`js/live.js` `effects()` and its option tables, ~3 KB), `now.js`'s `@fx:` branch, `state.js`'s toast | the Effects tab removed; the Live tab keeps the clock and text; Formulas gains Add to playlist (§7.3) |
| `opendmd-art-web` | `anim::effect_plan` (Save as GIF of an effect) | deleted; Formulas have Save as GIF through `fromFrames` |
| `opendmd-web/src/player.rs` | `fx::set_frame_floor` for the glow | kept (formulas honour the floor through `fx::floor`) |
| tools | `tools/demo-card.py` (`Starfield_Warp` as `@fx:starfield?…`), `tools/site-art.py` (seven `@fx:` strips and tiles), `tools/screenshots.py`, `tools/web-smoke.py`, `tools/crash-stress.py`, `tools/mock-device.py` | the paths become the Formula examples' `@fx:psi?src=…` (or `--formula FILE.psi` for the simulator); the demo card's Starfield is rebuilt from the Formula |
| docs | `docs/guide/effects.md`, `images/effects.png`, `docs/web-app.md` "live items and effects", `docs/writing-effects.md`, CLAUDE.md "Live effects" | `effects.md` folds into `formulas.md` ("The built-in Formulas"); `writing-effects.md` becomes a note that effects are Formulas now (its Rust guidance applies to `@clock`/`@text` only); CLAUDE.md's section is kept as history with a line at its top |
| tests | `fx.rs`'s `loops_are_whole_and_seamless`, `sparse_effects_mark_exactly_what_they_leave`, `lit_pixels_are_plausible`, `fx_cost`; `engine.rs`'s `@fx:starfield?fps=100` at 2793; synth's `title("@fx:tunnel…")` | deleted or moved to the Formula examples (`loops_are_whole_and_seamless` becomes "each built-in Formula's frame at `loop` equals frame 0", a golden); the engine test uses a Formula path |

### 7.2 What breaks on existing devices and saved playlists, and what to do

- **Playlist entries** (`filelist.txt` lines, the Library's playlists) that
  name `@fx:tunnel` etc. After the removal `synth::parse` returns `None`
  for them, the open fails, and the engine does what it does for any entry
  that fails to open: **skips it and moves on** (`engine.rs`, `Failed`: "that
  entry is skipped, as a failed open at the switch would be"), with the
  existing warning line. Nothing hangs; the playlist plays its GIFs. On a
  test card: `grep '@fx:' filelist.txt` before the update, replace the
  lines with the Formula paths (or drop them) — a one-off, by hand.
- **NVS `attract` = `fx:…`** (Follow's menu item). `nvs_config::attract`'s
  reader (`attract_path`, line ~259) would hand the engine a path that no
  longer parses: treat a stored `fx:` value as `""` (the playlist) with one
  log line at boot (`attract: "fx:starfield…" is no longer a live item;
  showing the playlist`), and `valid_attract` refuses new ones with a 400.
  Check `/api/config` on a device before updating.
- **The web app's open tab** on an older build talking to a newer device,
  or the reverse, for the minutes around the update: the Effects tab's
  Send to panel gets a `415` and a toast ("a live item this firmware does
  not know"), which the API already defines. Fine.
- **`now_playing.path`** in `/api/status` for a skipped entry: never shown
  (it is skipped).
- **The demo card and site art** are regenerated by the tools (§7.1).
- **Saved Formula looks and `psi.mine`** are untouched: they are `@fx:psi`
  paths.

So the only breakage is a log line per stale playlist entry and a stale
`attract` value made harmless; both are a `grep` away.

### 7.3 What the web app shows, and the knobs

- The Library's **Effects tab goes**. The **Formulas view** gains the
  four as bundled examples after Big clock (Tunnel, Rotozoom, Starfield,
  Wireframe — the wireframe's six shapes as six examples until phase 2's
  `buf` lets one example `pick` its tables), each with named sliders, Save
  as GIF, Show on panel, Use as the look, and **Add to playlist**, which
  Formulas lack today ("Formulas in playlists … are coming",
  `docs/guide/formulas.md`) and which is what makes the Effects tab
  removable: the `@fx:psi?src=…` path (≤ 2,048 B) goes in the playlist. The
  filelist line and the Library's playlist store must take a 2 KB line
  (check `MAX_READ`/the filelist parser's line cap).
- **Follow's attract item**: today `""`, `clock`, `text:…`, `fx:…`. With
  `fx:` gone, a Formula there is `formula:<@fx:psi path>` — but `ATTRACT_MAX`
  is 205 B and NVS strings are small; store it as the look is stored (its
  own NVS key, ≤ 2,048 B) or leave Follow's attract at clock/text/playlist
  for now (a playlist of one Formula does the same). Recommendation: leave
  it; one fewer NVS key.
- **Knobs.** The natives' options become each example's own lines: `loop`
  for speed, `pal(NAME, …)` for the palette, the texture lines, `#RRGGBB *
  ink` for the colour, and `frame { }` constants for twist/wobble/zoom/
  density/drift/floor/warp/shade — the ones worth a slider are bound to
  `a b c` by the example (`-- a: twist`). **Psi needs no new parameter
  mechanism**: the three sliders cover every number-valued knob, and the
  word-valued ones (palette, texture, shape) are source edits by nature.
  Two small things recommended because they fall out: (1) **pickers** — a
  naming comment `-- a: shape = cube, octahedron, icosahedron` the web app
  renders as a segmented control (the compiler ignores it; the formula
  reads `pick(floor(a*2.99), …)`; ~40 lines of JS); (2) a fourth slider
  `d` *only if* an example runs out of letters (the morph wants shape,
  speed, hue, shade). Not recommended: named numbers in the path.
- Status: `now_playing` says "Formula" and, when the path carries one, its
  name (`&name=` on the path, ≤ 60 B, as `look_formula_name`: small and
  wanted anyway so the Now card can say "Starfield").

### 7.4 Flash saved against what the drawing stage costs

In a release ELF (`xtensa-esp32-elf-nm -S` on `opendmd-firmware`) [M]:

| | bytes |
|---|---|
| all `opendmd_core::fx` symbols | 24,483 |
| of which data: `TABLES` 11,904, `DEPTH_LIGHT` 256, meshes 186, `SIN` 512, `PALETTES` 480 | 13,338 |
| of which code: `Tunnel::frame` 1,466, `Rotozoom::frame` 1,757, `Wire::draw` 2,407, `Fx::parse` 2,415, `line` 741, `Starfield::draw` 386, `Shape/Palette/Texture::from_name` 1,100, the rest | ~11,145 |
| stays (Psi lends them): `SIN`, `PALETTES`; the floor | −1,100 |
| `nvs_config`'s `fx:` validation, synth's dispatch, the engine test | −~500 [D] |
| **removable** | **~23,900** |
| the drawing stage (§3.8) | +5,000-7,000 [D] |
| **net** | **−17,000 to −19,000** |

Plus the main wasm's share of `fx` (the browser preview draws effects
natively today: ~8 KB gz of the 118 KB main wasm [D]), `opendmd-art-web`'s
`effect_plan`, and ~3 KB of `live.js`. The `TABLES` flash-cache finding
("one struct smaller than a way") stops mattering: Psi's textures are
arithmetic or 1 KB `table`s.

### 7.5 The other live items

| item | what it is | Formula? |
|---|---|---|
| `@clock` (`style=big/small`, `seconds`, `date`, `h12`, `color`) | the time, drawn only when the string changes; `--:--` until set | **Stays native for now.** Big clock already does the time in Psi (6 ms a frame, redrawn every frame; the native costs nothing between seconds). `date=1` needs weekday and month names: `glyph` has the font but Psi has no `day`, `month`, `weekday` inputs — three new inputs and it becomes a ~200-step formula. Revisit after 1.0; it is the next candidate. |
| `@text?msg=…` (fonts, speed, colour, scroll) | a user's message, up to 200 chars, UTF-8 | **Stays native.** Data-driven: a `pick` of 200 character codes exceeds the 128 scalar slots, `glyph` covers ASCII 32..126 only, and the message is the user's, not the author's. Psi could grow a `text` statement later; not for 1.0. |
| `@boot`, `@power-on`, `@card:` | the start-up show, Follow's power-on effect and title cards | **Stay.** System items with their own timing and the compositor's seam; nothing a user authors. |

So: the four `@fx:` effects go; `@clock` and `@text` stay as the "Live"
items; a later stage may make `@clock` a Formula once `day/month/weekday`
exist and the still cache makes a per-frame clock cheap.

### 7.6 The order, and how close is close enough

What must exist in Psi before each native can go, and the gate. Until the
last step the natives stay in the build (nothing is dual-shipped for
users; they are there for the side-by-side), and the Library's Effects tab
is the comparison tool.

| step | ships | gate to delete the native |
|---|---|---|
| 1 | Phase 1 (`draw`, `for`, `table`, `line`, `dot`, `ink`); Starfield and Wireframe cube/octahedron/icosahedron/tesseract as examples; pickers; Add to playlist | — |
| 2 | Phase 2 `buf` → torus and morph | **Wireframe, Starfield:** an eye test on a test panel side by side (the native from the Effects tab, the Formula from Formulas, the same options) at the default and two variants each; 0 steady misses at the Formula's governed rate for 10 min; the probe's `frame_us` within §4.5 ±25 % |
| 3 | Rotozoom example (one and two layers); `pal()` × value in colour expressions | **Rotozoom:** the same, accepting 25 fps for two layers unless the still cache ships first |
| 4 | the still cache; Tunnel example | **Tunnel:** the same; the eye test decides whether 50 fps against 100 passes |
| 5 | §7.1's deletions, the devices' playlists cleaned (§7.2), the flash measured; a release note: "The built-in effects are Formulas now: open any of them in Formulas to see how it is written and change it. `@fx:tunnel` and the others are gone." | — |

**As built:** step 5 was done in one cut with all of §7.1: `@fx:psi` is now
`@formula`, `attract` has no Formula form, the probe had nothing to remove.
The staged gates above were not used.

"Close enough" is an eye on the panel, not a pixel diff (the hashes and
textures differ by design: §4). The comparison fixture — the same options
both ways, screenshots from the simulator for the record (`--screenshot-at`
at four phases of the loop), a test panel for the eye — is the only honest
test. If one effect never passes, its native stays as the one exception;
the design loses nothing. Renaming the Formula path from `@fx:psi?src=` to
`@formula?src=` once `fx` means nothing else is cosmetic (one string in
`synth`, the web app, the API doc and every saved look): do it in step 5
or never.
