# gifslim — can standard GIFs replace ODM as the prepared format?

> **Adopted (2026-09-24).** The prepared copy is now a GIF in exactly this
> recoder's cap-9 delta form with the hint, at `cache/<Category>/<xx>/<name>.gif`
> (not under `odm/` with the `.odm` name suggested below); ODM is retired, and
> originals play through `GifSlim` at cap 12 with the `gif` crate as the
> fallback. The format, the layout and the one encoder the desktop, the
> browser and the device share are in [`cache-format.md`](cache-format.md).
> What follows is the experiment as it was run, kept as the record.

An experiment, measured on the host only (no rig), against the card copy
`~/scratch/dmdos-card` (11,047 GIFs, 1,575 MB; ODM copies 1,484 MB). The
code is in `opendmd-core/src/gifslim.rs` (decoder, capped LZW encoder,
recoder), `opendmd-pack/src/recode.rs` (`recode`, `gifscan`) and the examples
`heapprobe --slim`, `gifbench` and `recodeexp`. Nothing here is on a playback
path yet.

**Answer: yes.** A GIF we encode ourselves — LZW codes capped at N bits,
frames as delta rectangles with a transparent index and disposal Keep, one
global palette — plays through a 3 KB-table decoder at **16.1-16.8 KB of heap
including the 12 KB canvas (cap 9)** against ODM's 14.4-15.1 KB and the `gif`
crate's 53.6-54.4 KB; it is **35-39% smaller than ODM** (cap 9: 960 MB, cap
10: 913 MB, against 1,484 MB) and 39-42% smaller than the originals; it
decodes at about the `gif` crate's cost per pixel but has half the pixels to
decode, **~3.3 ms a frame on the device (est.) against ODM's ~1.7 ms and the
original GIF's ~5.4 ms**; and every copy is an ordinary GIF89a (Pillow,
ffmpeg and ImageMagick play them; Pillow's frames equal the originals').
Every frame of every file was checked three ways, all 8 variants: **0
mismatches, 0 `NeedsFullDecoder`**.

## What was built

### The encoder: `gifslim::recode_gif` and `opendmd-pack recode`

`opendmd-pack recode <card-root> --cap N --delta on|off [--jobs J] [--verify]
[--verify-only] [--out-suffix S] [--no-hint] [--no-keep-runs] [--tsv F]`
writes `<name>.cap<N>[d].gif` beside each GIF (a separate file; the originals
are never touched; every walker in `opendmd-pack`, `convert` included, skips
names ending `.cap<digits>[d].gif`).

1. **Frames come from `GifSource`**, so disposal, transparency and the
   local-over-global palette rule are exactly what playback shows. The raw
   delays (hundredths, 0 and 1 included) and the NETSCAPE loop count are
   copied from the source through the `gif` crate's metadata, so browsers
   time the copy as they timed the original.
2. **One global palette**: the distinct colours in first-seen order, plus a
   transparent slot in delta mode. Past 255 colours (256 without deltas)
   `NeuQuant` (sample factor 10) builds 255 (256) — the same **38 files** ODM
   quantises; worst channel error 151. **Exactly 256 colours with deltas
   (114 files)**: no slot to spare, so each frame's transparent index is any
   index its changed pixels do not use (no frame on the card used all 256;
   a frame that did would be sent as an opaque rectangle).
3. **Deltas** (`--delta on`): the minimal rectangle of pixels whose colour
   changed, unchanged pixels inside it transparent, disposal Keep. Two
   refinements: **frame 0 is whole and opaque** (our players start from
   black; a browser shows what frame 0 leaves uncovered as the page behind
   it), and **`keep_runs`** — an unchanged pixel that has the same colour as
   the pixel before it in the rectangle repeats that index instead of
   switching to the transparent one (same output, longer runs: −0.7% bytes
   on a 1/8 sample; `--no-keep-runs` is the textbook rule). An unchanged
   frame is one transparent pixel (its delay still counts).
   `--delta off`: every frame whole, disposal unspecified.
4. **Our own LZW** (`lzw_encode_capped`): the `gif` crate's encoder cannot cap
   the width — it calls `weezl`, whose `MAX_CODESIZE` is a hard-coded 12 and
   which clears only when the table passes 4,096 entries
   (`weezl-0.1.12/src/encode.rs`, `EncodeState::advance`). So the recoder
   writes its own code stream and hands it to `gif::Encoder::
   write_lzw_pre_encoded_frame` (container, sub-blocks, graphic control) —
   the crate is still the file writer. Code widths follow the rule every GIF
   decoder uses (giflib, `weezl`): a code is read with as many bits as the
   decoder's next free entry needs. **CLEAR is emitted as the code the
   decoder reads when its next free entry is `2^N − 1`** — the last point
   where a code (the CLEAR itself) still fits in N bits; one entry later the
   next code would need N+1. So a decoder never defines an entry past
   `2^N − 2` and never reads a code wider than N. Minimum code size is
   `log2` of the palette table (≥ 2). Hash dictionary with `2^(N+1)` slots,
   emptied at each CLEAR.
5. **The hint**: an application extension `OPENDMD1LZW` (8-byte id +
   3-byte auth code) with data `[1, cap, flags]` (bit 0 = delta), right after
   the global palette — 19 bytes a file (`--no-hint` leaves it out).

### The decoder: `gifslim::GifSlim`

`new(reader)` (cap from the hint, else `DEFAULT_CAP` = 12),
`with_cap(reader, cap)` (3..=12), `next_frame(&mut Canvas) ->
io::Result<Option<Duration>>`, `logical_size`, `frames_decoded`, plus `cap`,
`hint`, `stats`, `heap_bytes`, `into_inner`. Same contract as `OdmSource`
and `GifSource`: `Ok(None)` at the end leaves the canvas alone, at the panel
size the caller's canvas *is* the logical screen (cleared before frame 0;
the caller must not write it between frames), strays are centre-cropped
through an owned screen exactly as `GifSource` does, the dirty map is marked
exactly as `GifSource` marks it (unit-tested pixel for pixel).

- **Subset**: GIF87a/89a header, global and local palettes, graphic control
  (delay, transparent index, disposal Any/Keep/Background), frame
  sub-rectangles (clipped like the player), NETSCAPE / comment /
  plain-text / application extensions skipped. Interlaced frames and
  `Previous` disposal are `Err(Unsupported)`. The `gif` crate's error
  behaviour is mirrored (unknown extension labels and blocks, a control
  extension of the wrong length, min code size outside 1..=11, "image
  truncated", invalid codes, EOF without a trailer).
- **Capped LZW**: tables `prefix: u16[2^cap]`, `suffix: u8[2^cap]`,
  allocated once at open. Before reading each code: **if the code would be
  wider than the cap, `Err(NeedsFullDecoder)`** (in an `io::Error` of kind
  `Unsupported`; `is_needs_full_decoder`). "A code wider than `cap` bits" is
  the precise form of "a code ≥ 2^cap": the width grows exactly when the
  next entry would be `2^cap`. The frame in progress is partly drawn; nothing
  before it is disturbed. Once a frame's pixels are complete the rest of its
  data is skipped unread, so a wide code (or EOI) after the last pixel
  never trips it. With cap 12 it is a complete GIF LZW decoder (deferred
  CLEAR at 4,096 entries included).
- **No frame buffer and no heap output stack.** The brief allowed a 4 KB
  indexed frame buffer for sub-rectangles and a `2^cap` output stack; neither
  is needed. Each code's string is walked once into a **96-byte buffer on the
  call stack** (filled from its end, so it reads forwards) and copied into a
  **one-row index buffer** (`width` bytes, 128), which is composited when the
  row completes — through a per-frame 256-entry colour table and skip mask
  (transparent index and indices past the palette), marking dirty spans like
  `player::Compositor::row`.
  **Why 96 is enough**: a string of length L needs entries of every length
  below it, which only codes of lengths 1..L−1 since the last CLEAR can have
  defined, so a frame of P pixels holds no string longer than L with
  L(L+1)/2 ≤ P — **90 for 128x32**. The measured maximum over every file and
  variant is exactly 90 (a one-colour frame). A longer string (possible only
  in frames over 4,656 pixels) is walked a second time and painted backwards
  from its end position; over the whole card that path ran 0 times with the
  96-byte buffer (0.07-0.28% of codes with the 64-byte one first tried).
- **No allocation per frame**: `heapprobe` peak == steady on every file.

### Tools

- `opendmd-pack recode … --verify`: for every file, **three decoders in
  lockstep**: the original through `GifSource`, the copy through `GifSource`
  (the `gif` crate — "it is a standard GIF") and the copy through
  `GifSlim::with_cap(cap)`. Every canvas (all 12,288 bytes) and every delay
  must be identical between the two copy decoders, and equal to the
  original's (or, for the 38 quantised files, the same frame count and
  delays with the colour error reported). `--verify-only` re-checks copies a
  previous run wrote. `--tsv` writes the per-file table.
- `opendmd-pack gifscan <card-root>`: every *original* through `GifSlim` at
  caps 9, 10, 11 and 12 against `GifSource`, frame by frame.
- `cargo run --release --example gifbench -- <base>...` (decode CPU; thread
  CPU time, median of 5), `heapprobe --slim <cap|auto> <file>...`,
  `recodeexp [--every K] <dir>` (variants and encode time in memory).

## Correctness

All 8 variants, 11,039 files (the other 8 are the known undecodable ones: 6
invalid LZW codes, 1 truncated, the PNG named `.gif`), **1,791,896 frames**
each, final binaries:

| variant | identical to the original (all 3 decoders) | quantised (copy decoders agree; frames + delays match) | `NeedsFullDecoder` | mismatch |
|---|---|---|---|---|
| cap12, cap11, cap10, cap9 (delta off) | 11,001 | 38 (worst channel error 150) | 0 | 0 |
| cap12d, cap11d, cap10d, cap9d | 11,001 | 38 (worst 151) | 0 | 0 |

Widest code read in each variant's copies = its cap, exactly. External
viewers: 300 random `cap10d`/`cap9d` copies and 150 `cap10` copies open in
Pillow with the right frame count and **every frame pixel-identical to
Pillow's rendering of the original**; 100 `cap10d` copies decode in ffmpeg
with the right frame count (3 print the same "non monotonically increasing
dts" warnings as their originals — zero delays); ImageMagick reads them.

`gifscan` also validates `GifSlim` as a general GIF decoder: at cap 12 it
plays **11,034 of the 11,047 originals identically to `GifSource`**; the
rest are 4 files with `Previous` disposal and 1 interlaced (Unsupported) and
the 8 undecodable (both decoders fail on the same frame). 0 mismatches.

## Measurements

The host is an i5-12600H (16 threads), shared: **load average 22-60
throughout** from other work, so wall-clock times are pessimistic and noisy;
CPU figures use thread CPU time.

### Size

Totals over the 11,039 files that convert (originals 1,574.8 MB, ODM
1,483.7 MB). Ratios per file are copy / original and copy / ODM.

| variant | total MB | vs originals | vs ODM | median / p95 per file (vs original) | median / p95 per file (vs ODM) | CLEARs forced by the cap |
|---|---|---|---|---|---|---|
| cap12 (plain re-encode) | 1,533.8 | 0.974 | 1.034 | 0.994 / 1.000 | 1.023 / 4.72 | 0 |
| cap11 | 1,534.1 | 0.974 | 1.034 | 0.994 / 1.000 | 1.023 / 4.72 | 25,860 |
| cap10 | 1,550.1 | 0.984 | 1.045 | 0.995 / 1.025 | 1.026 / 4.73 | 578,131 |
| cap9 | 1,637.6 | 1.040 | 1.104 | 1.004 / 1.135 | 1.062 / 4.92 | 3,204,015 |
| cap12d | 905.2 | 0.575 | 0.610 | 0.547 / 0.993 | 0.586 / 0.864 | 0 |
| cap11d | 905.3 | 0.575 | 0.610 | 0.547 / 0.993 | 0.586 / 0.864 | 15,384 |
| **cap10d** | **913.4** | **0.580** | **0.616** | 0.548 / 1.000 | 0.587 / 0.866 | 275,759 |
| **cap9d** | **960.2** | **0.610** | **0.647** | 0.558 / 1.041 | 0.596 / 0.899 | 1,751,175 |

- **Deltas are the whole win.** A plain re-encode is ODM-sized (1.03x); the
  worst plain files are the ones whose authors already used small frame
  rectangles (`NEW 2021/automn4.gif`: 5.2 KB -> 143 KB whole-frame, 27.5x).
- **The cap is nearly free down to 10**: cap 11 costs nothing, cap 10 +0.9%
  over cap 12, cap 9 +6.1% (256-colour palettes start at 9 bits and get only
  253 dictionary entries per CLEAR).
- Copies larger than their ODM: 148 (cap10d) / 209 (cap9d) of 11,039 — tiny
  files where the GIF header, palette, NETSCAPE block and hint dominate
  (`Test_Suite/32p_Fast_Test.gif`: ODM 84 B, copy 444 B), and a few
  whole-screen scrollers (`ATARI_ST_Logo`, the Pac-Man 25th set: ~1.9-2x
  ODM, still 0.26-0.48x the original). Larger than the original: 551
  (cap10d) / 879 (cap9d).

Worst 10 for **cap9d** (the recommended variant), copy / original:

| file | original B | ODM B | cap9d B | frames | vs original | vs ODM |
|---|---|---|---|---|---|---|
| Arcade/ARCADE_LastMission.gif | 90,088 | 218,381 | 122,591 | 102 | 1.361 | 0.561 |
| Arcade/ARCADE_Galaxian.gif | 94,505 | 198,046 | 123,889 | 198 | 1.311 | 0.626 |
| XMAS/XMAS_ATARI_ST_MEGADEMOS_01.gif | 174,687 | 300,756 | 224,938 | 280 | 1.288 | 0.748 |
| Other/OTHER_NeoArcadia02_RattenJager.gif | 206,668 | 398,480 | 262,085 | 156 | 1.268 | 0.658 |
| Arcade/ARCADE_Raiden.gif | 106,427 | 228,296 | 134,014 | 68 | 1.259 | 0.587 |
| Arcade/ARCADE_StarWars.gif | 172,915 | 311,639 | 216,873 | 394 | 1.254 | 0.696 |
| Arcade/ARCADE_Beast_Busters_01.gif | 175,221 | 334,967 | 218,747 | 108 | 1.248 | 0.653 |
| Consoles/SNES_Rock_N_Roll_Racing_05_sephirot68.gif | 1,598,197 | 2,840,957 | 1,993,639 | 993 | 1.247 | 0.702 |
| NEW 2021/CarrierAirWing004.gif | 337,880 | 551,330 | 420,164 | 203 | 1.244 | 0.762 |
| Consoles/GBC_NascarRacers_RattenJager.gif | 363,704 | 779,782 | 452,122 | 418 | 1.243 | 0.580 |

(For cap10d the worst is 1.19x the original, `ARCADE_LastMission` again.)

### Decode CPU

`gifbench` over the seven heap-budget files, from memory, release build,
thread CPU time, median of 5 runs of ≥40 ms each; µs per 128x32 frame (all
seven are 128x32). **Device estimate x100**: the one on-device figure we
have, `GifSource` on `filelist.txt` at 240 MHz, is 5.05 ms p50 per frame
(CLAUDE.md, "Frame time"), and `GifSource` here is 49-54 µs a frame, so the
LZW decoders scale by ~95-100x; ODM's ratio is unmeasured.

| decoder, file | ns/pixel | µs/frame (host) | device est. (x50 .. **x100**) |
|---|---|---|---|
| `gif` crate, original | 13.2 | 54.2 | 2.7 .. **5.4 ms** ⚠ |
| `OdmSource`, `.odm` | 4.1 | 16.9 | 0.8 .. **1.7 ms** |
| `GifSlim` cap 12, original | 13.6 | 55.5 | 2.8 .. **5.6 ms** ⚠ |
| `GifSlim` cap 9, cap9d | 8.0 | 32.8 | 1.6 .. **3.3 ms** |
| `GifSlim` cap 10, cap10d | 7.7 | 31.7 | 1.6 .. **3.2 ms** |
| `GifSlim` cap 11, cap11d | 7.7 | 31.5 | 1.6 .. **3.2 ms** |
| `GifSlim` cap 12, cap12d | 7.7 | 31.4 | 1.6 .. **3.1 ms** |
| `GifSlim` cap 9 / 10 / 11 / 12, whole-frame copies | 14.4 / 13.5 / 13.4 / 13.3 | 59.0 / 55.3 / 54.8 / 54.6 | **5.5-5.9 ms** ⚠ |
| `gif` crate, cap10d | 7.2 | 29.6 | 1.5 .. **3.0 ms** |

⚠ = over ~5 ms a frame at x100. Per file (µs/frame, host):

| file | frames | `gif`, original | ODM | slim 12, original | slim 9, cap9d | slim 10, cap10d | slim 11, cap11d | `gif`, cap10d |
|---|---|---|---|---|---|---|---|---|
| PINBALL_SHORT_Corvette_01 | 22 | 42.9 | 8.3 | 44.8 | 29.8 | 29.9 | 29.8 | 26.7 |
| PINBALL_SHORT_WWF_40 | 50 | 48.7 | 36.2 | 52.3 | 51.1 | 49.7 | 50.2 | 45.6 |
| SNES_Inspector_Gadget_02 | 99 | 37.5 | 12.5 | 41.6 | 25.9 | 25.2 | 24.8 | 25.8 |
| ARCADE_Arkanoid_03 | 282 | 51.3 | 12.7 | 51.3 | 22.4 | 22.0 | 22.0 | 20.9 |
| ARCADE_NEOGEO_MetalSlugFire05_Shabazz | 1680 | 80.5 | 19.4 | 81.2 | 43.0 | 40.7 | 39.8 | 39.1 |
| SNES_Donkey_Kong_Country_04 | 60 | 49.2 | 17.8 | 48.7 | 34.2 | 33.3 | 34.2 | 29.8 |
| SNES_Yoshi_Island_01_01_Wolfsoft | 2350 | 68.9 | 11.6 | 68.7 | 23.3 | 20.7 | 20.1 | 19.2 |

- Per pixel decoded, `GifSlim` costs what the `gif` crate (weezl) costs
  (within ~5-10%, both ways across runs); the delta copies are ~40% cheaper
  than the originals because half the pixels are never decoded (frame
  rectangles cover 51.5% of all frame pixels). ODM stays ~2x cheaper still:
  its runs need no dictionary.
- The cap does not change the speed.
- One file of the seven, `PINBALL_SHORT_WWF_40` (full-screen changes every
  frame), is ~5 ms at x100 as a delta copy — as its original is, and ODM is
  3.6 ms there. That is today's GIF path, which the device already plays.

### Heap per decoder instance

`heapprobe`, live heap once playing (canvas + boxed `File` + decoder),
steady == peak for every ODM and gifslim file:

| file | `gif` crate (original) | ODM | gifslim cap 9 (cap9d) | cap 10 (cap10d) | cap 11 (cap11d) | cap 12 (cap12d / original) |
|---|---|---|---|---|---|---|
| PINBALL_SHORT_Corvette_01 | 53,644 | 14,394 | 16,100 | 17,636 | 20,708 | 26,852 / 26,780 |
| PINBALL_SHORT_WWF_40 | 53,668 | 14,379 | 16,052 | 17,588 | 20,660 | 26,804 / 26,804 |
| SNES_Inspector_Gadget_02 | 53,716 | 14,388 | 16,100 | 17,636 | 20,708 | 26,852 / 26,852 |
| ARCADE_Arkanoid_03 | 53,716 | 14,418 | 16,100 | 17,636 | 20,708 | 26,852 / 26,852 |
| ARCADE_NEOGEO_MetalSlugFire05_Shabazz | 54,388 | 14,829 | 16,772 | 18,308 | 21,380 | 27,524 / 27,524 |
| SNES_Donkey_Kong_Country_04 | 53,668 | 14,388 | 16,100 | 17,636 | 20,708 | 26,852 / 26,804 |
| SNES_Yoshi_Island_01_01_Wolfsoft | 54,388 | 15,108 | 16,772 | 18,308 | 21,380 | 27,524 / 27,524 |

### The decoder's memory budget

| | cap 9 | cap 10 | cap 11 | cap 12 |
|---|---|---|---|---|
| `Canvas` (the caller's) | 12,288 | 12,288 | 12,288 | 12,288 |
| boxed `File` | 4 | 4 | 4 | 4 |
| read buffer (`BufReader`, as `OdmSource`) | 2,048 | 2,048 | 2,048 | 2,048 |
| `prefix: u16[2^cap]` | 1,024 | 2,048 | 4,096 | 8,192 |
| `suffix: u8[2^cap]` | 512 | 1,024 | 2,048 | 4,096 |
| row of indices (`width`) | 128 | 128 | 128 | 128 |
| global palette as declared (`3 * 2^k`) | 12-768 | 12-768 | 12-768 | 12-768 |
| **total** | **16,016-16,772** | **17,552-18,308** | **20,624-21,380** | **26,768-27,524** |
| on the call stack per `next_frame` | ~2 KB: local palette 768, colour table 768, skip mask 256, string buffer 96 | | | |

Against ODM (`14,388` for the same file) cap 9 is +1.7 KB: +1.5 KB of
tables, +128 B row, and the palette rounded up to a power of two. Knobs if
that matters: a 1 KB read buffer (−1 KB, for ODM too), compositing per
string instead of per row (−128 B), tables sized `2^cap − first` for a
known minimum code size (the hint could carry it; −774 B for a 256-colour
palette at cap 9, little for small palettes).

### Originals the small decoder plays with no preparation

`gifscan`, all 11,047 originals, every frame compared with `GifSource`:

| cap | plays identically | `NeedsFullDecoder` | Unsupported | undecodable (both fail) | mismatch |
|---|---|---|---|---|---|
| 9 | 1,504 (13.6%) | 9,534 | 3 | 6 | 0 |
| 10 | 6,702 (60.7%) | 4,334 | 5 | 6 | 0 |
| 11 | 10,615 (96.1%) | 419 | 5 | 8 | 0 |
| 12 | 11,034 (99.9%) | 0 | 5 (4 `Previous`, 1 interlaced) | 8 | 0 |

Widest code the originals need (files gifslim plays at 12): 8 bits 80, 9
bits 1,424, 10 bits 5,198, 11 bits 3,913, 12 bits 419. Longest string in
any original: 90. (Whether a file plays at cap N is exactly "widest code ≤
N": checked at all four caps, 0 exceptions.)

### Encode time

| | wall clock, 16 jobs |
|---|---|
| `opendmd-pack convert --verify` (ODM, from `docs/odm-format.md`, idle host) | 81.5 s |
| `recode --cap 9 --delta on` (no verify; load 22-50) | 122.7 s |
| `recode` with `--verify`, delta variants (load 22-60) | 180-240 s |
| `recode --verify-only` (three decoders over 1.79 M frames) | 58-78 s |
| in memory, same load, back to back: `odm::encode_gif` / `recode_gif` cap10d | 133.6 s / 174.5 s (1.31x) |

The recoder decodes each GIF three times (metadata, colours, frames) where
ODM's decodes it twice, and adds LZW; merging the metadata pass into the
colour pass would take most of the difference back.

## Recommendation

**Replace ODM with hinted, delta, cap-9 GIFs as the prepared format**, and
independently **play unprepared originals with `GifSlim` at cap 12**,
keeping the `gif` crate only as the fallback.

Why replace:

- **Heap**: 16.1-16.8 KB against ODM's 14.4-15.1 KB — +1.7 KB, within the
  "ODM class", and 37 KB under the `gif` crate. (Cap 10 is +3.2 KB for 5%
  fewer bytes; on a board where the drip cache builder is "marginal by
  arithmetic" at 76-80 KB free, the 1.5 KB is worth more than 47 MB of card.)
- **Size**: 960 MB against 1,484 MB (−35%): a prepared card grows by 0.61x
  its GIFs instead of 0.94x, and every frame reads ~35% fewer bytes off a
  card the web server shares.
- **CPU**: ~3.3 ms a frame (est.) against ODM's ~1.7 ms — slower than ODM,
  ~40% faster than today's GIF path, well inside the 20 ms floor.
- **One format**: the prepared copy is a GIF that any viewer, the browser
  included, shows correctly; lossless for 11,001 of 11,039 files. ODM's two
  encoders and its decoder go; the cache could even *be* the pack (a card
  holding only the copies is 0.61x the size, at the cost of the 38 lossy
  files and any unknown extensions).
- **Robust by construction**: the hint is only a promise — a tool that
  rewrites the file can keep the hint and break the cap (ImageMagick does:
  it keeps the block, lowercases the id, re-encodes the LZW; gifslim then
  stops at frame 28 with `NeedsFullDecoder`, as designed). A prepared copy
  that fails that way is just played by the `gif` crate.

Why also cap 12 for originals: identical output on 99.9% of the pack at
26.8-27.5 KB instead of 53.6-54.4 KB, same CPU. The unprepared path is
exactly where the heap is short today (`gif on screen` pauses the builder).

What would keep ODM instead: if 1.7 KB of heap or ~1.6 ms a frame turn out
to matter more on the rig than card space and a standard format. Neither
has been measured on the device; the first rig session should.

## If adopted: what changes

**Prepared cache.** Files stay GIFs; **layout and index unchanged**: the
same `layout::bucketed_path` (`/odm/<Category>/<bb>/<name>.odm` — keeping
the `.odm` name keeps every bucket, `prepared_key` and `count_prepared`
exactly as they are; renaming to `.gif` would move every bucket, a
card-format break for no functional gain), the same resolution rule, the
same `/api/cache*`. Only the content changes: a GIF89a with the
`OPENDMD1LZW` hint instead of `ODM1`. Existing ODM copies keep playing
during the transition because `FrameSource` sniffs content (`ODM1` vs
`GIF8`), and are replaced as the builder and the browser re-prepare them.

**Core.**
- `FrameSource` gains a `Slim(GifSlim<…>)` variant; `open` on `GIF8`
  opens `GifSlim::new` (hint → its cap; no hint → 12).
- `Unsupported` at open, or `NeedsFullDecoder`/`Unsupported` mid-file, needs
  a fallback: the engine already owns a `Box<dyn ReadSeek>`, so
  `into_inner`, seek 0, reopen with `GifSource` and restart the file from
  frame 0 (a visible restart, for 5 unhinted originals and for hinted files
  a tool has broken). Add `Previous` disposal (lazy 12 KB snapshot, as the
  player does) and interlacing (the row order is in `player::RowOrder`) to
  gifslim if even that is unwanted.
- A **streaming recoder** for the device (the one missing piece): two
  passes over the GIF — pass 1 the colour set (the ODM streaming encoder's
  `Colours` table carries over), pass 2 decode, index, diff against the
  previous frame's indices, emit the rectangle's rows straight into
  `lzw_encode_capped` and a 2 KB write buffer. Estimate: source decoder
  (gif crate ~37 KB, or gifslim cap 12 ~15 KB + a 12 KB screen) + 4 KB
  previous indices + 512 B changed bits + 6 KB LZW hash at cap 9 + 2 KB
  write buffer ≈ 50-60 KB, the ODM streaming encoder's class. No quantiser
  on the device, as today (`TooManyColours`: 38 files, left for the
  browser/pack).
- `odm.rs` becomes read-only legacy, then goes.

**Firmware.** `sd::SdSource::open`'s "starts with `ODM1`" check becomes
"`GIF8` and `GifSlim::new` finds the hint" (the hint is in the first
~800 bytes, inside the first read). `cache_builder.rs` calls the streaming
recoder; `BUILDER_MIN_HEAP` and the drip arithmetic are re-measured (the
player side is +1.7 KB; the builder's own encoder is the bigger unknown).
The heartbeat/log words "prepared" / "original" are unchanged. The heap
budget table in CLAUDE.md gets a gifslim column.

**Web.** The prepare job calls `recode_gif` (the same core function; builds
for wasm — `make check-wasm` passes) instead of `encode_gif`; it stays the
path for >256-colour files. The Library can show a prepared copy directly
as an `<img>`. `web-smoke`'s "never show odm" rule is unaffected.

**pack.** `convert` writes recoded GIFs (`recode`'s code path) at the
bucketed paths; `--verify` becomes the three-way check.

## The application-extension hint

- **Format**: `21 FF 0B "OPENDMD1LZW" 03 01 <cap> <flags> 00`, after the
  global palette and before the NETSCAPE block. `GifSlim` reads it in `new`
  (before sizing its tables) — it matches the id case-insensitively because
  ImageMagick lowercases it — and `new` then allocates exactly `2^cap`
  entries. A file without it opens at cap 12 (`DEFAULT_CAP`), where only
  interlacing and `Previous` disposal stop it.
- **The `gif` crate** (0.13.3 as vendored) skips unknown application
  extensions when decoding — every one of the 88,312 hinted copies decoded
  identically — but does **not preserve** them: the high-level `Decoder`
  never exposes their bytes (only the low-level `StreamingDecoder::last_ext`
  does) and nothing carries them from a decoded frame to the `Encoder`; it
  can *write* one (`Encoder::write_raw_extension`, which the recoder uses).
  So a decode → encode round trip through the crate drops the hint (and
  re-encodes with 12-bit codes, which is the right outcome).
- **Other tools**: Pillow drops it on save; ImageMagick keeps it (as profile
  `gif:opendmd1lzw`, id lowercased) but re-encodes the LZW, which is why the
  hint must stay a hint and the cap check stays in the decoder.

## Reproducing

```bash
cd opendmd-pack && cargo build --release
for cap in 9 10 11 12; do for d in off on; do
  ./target/release/opendmd-pack recode ~/scratch/dmdos-card --cap $cap --delta $d --verify --tsv /tmp/cap$cap$d.tsv
done; done
./target/release/opendmd-pack gifscan ~/scratch/dmdos-card
cd ../opendmd-core && cargo build --release --examples
./target/release/examples/gifbench <base-path-without-.gif>...        # needs the .odm and .cap*.gif beside
./target/release/examples/heapprobe --slim 9 <file.cap9d.gif>...
./target/release/examples/recodeexp --every 8 ~/scratch/dmdos-card/gifs
```

The copies are ~12 GB for all eight variants on the scratch card:
`find ~/scratch/dmdos-card/gifs -regex '.*\.cap[0-9]+d?\.gif' -delete`
removes them.
