# ODM v1 — pre-rendered frames for OpenDMD

An `.odm` file is a GIF animation that has already been played: every frame
fully composited (disposal and transparency applied), indexed into one palette
for the whole file, and stored as runs relative to the previous frame. Playing
one needs the palette (at most 768 B), a 2 KB read buffer and the 128x32
canvas the player owns anyway — about **14.4-15.1 KB** of heap against
**~54 KB** for the same GIF, because there is no LZW decoder (28,784 B) and no
`gif` crate read buffer (~8.5 KB).

The reference implementation is the Python prototype
[`tools/odm-proto.py`](../tools/odm-proto.py), which fixed the format; the
Rust encoder and decoder in `opendmd-core/src/odm.rs` read and write it
unchanged. When the two disagree about the *layout*, the Python is right.

## Byte layout

Everything is little-endian. The Python `struct` format for the header is
`"<4sBHHIH17s"`.

### Header (32 bytes)

| offset | size | field | notes |
|---|---|---|---|
| 0 | 4 | `magic` | `b"ODM1"` |
| 4 | 1 | `version` | `1` |
| 5 | 2 | `width` | `u16`, non-zero |
| 7 | 2 | `height` | `u16`, non-zero |
| 9 | 4 | `frame_count` | `u32` |
| 13 | 2 | `palette_len` | `u16`, `1..=256` |
| 15 | 17 | reserved | written as zero; readers ignore it |

### Palette

`palette_len` entries of 3 bytes, `R G B`.

### Frames

`frame_count` times:

| size | field | notes |
|---|---|---|
| 2 | `delay_ms` | `u16`, how long to show this frame |
| 4 | `data_len` | `u32`, bytes of ops that follow |
| `data_len` | ops | cover exactly `width * height` pixels in raster order |

Frames are always complete: there is no disposal method and no transparency in
the file, because the encoder has already applied them.

### Ops

One op byte `tag << 6 | len6`. If `len6 == 63` the real length is the `u16`
that follows the op byte; otherwise it is `len6` (so 1..=62 inline, 63..=65535
escaped).

| tag | name | payload | meaning |
|---|---|---|---|
| 0 | KEEP | — | the next `len` pixels are unchanged from the previous frame |
| 1 | FILL | 1 byte `index` | the next `len` pixels are `palette[index]` |
| 2 | LIT | `len` bytes | one palette index per pixel |
| 3 | — | — | reserved; a decoder rejects it |

**Frame 0's "previous frame" is all black RGB `(0,0,0)`**, not palette index 0
(which need not be black). KEEP in frame 0 therefore means black, and "is this
pixel unchanged?" is always decided on RGB, never on the index.

### What a decoder must reject

`OdmSource` returns `InvalidData` — never panics — for: bad magic; a version
other than 1; a zero or implausible (> 2048x2048) dimension; `palette_len`
outside 1..=256; a palette index `>= palette_len`; a run that overruns the
frame; tag 3; ops that do not consume exactly `data_len` bytes; and a file
that ends anywhere inside a structure. After `frame_count` frames it reports
end of stream; anything after that is ignored.

### Delays

`delay_ms` is final. The Rust encoder writes the delay `GifSource` returns,
i.e. after the player's rules — a GIF delay of 0 becomes 100 ms, anything else
is floored at 20 ms — so the decoder applies no rules. (The prototype wrote
Pillow's raw `duration` instead; across the whole card that differs from the
Rust value only where the rules apply, and the cross-check compares the two
after applying them.)

## The encoder

`opendmd_core::odm::encode_gif(reader, writer) -> EncodeStats`:

1. Frames come from our own `GifSource`, so disposal, transparency, the
   local-over-global palette rule and the delays are exactly what playback
   does. A GIF whose logical screen is not 128x32 is encoded at its own size;
   the decoder centre-crops it at playback, as the GIF path does.
2. **Palette.** If the animation has at most 256 distinct colours, they are the
   palette, in first-seen order (raster order, frame by frame). Otherwise
   `color_quant::NeuQuant` (sampling factor 10) builds 256 colours over every
   pixel of every frame, and `EncodeStats::quantised` is set.
3. **Ops, greedy.** Per frame, compute for each position `i` the length of the
   run of equal indices starting there (`fill[i]`) and of unchanged pixels
   starting there (`keep[i]`, 0 if pixel `i` changed). At each position: if
   `fill[p] >= 4` or `keep[p] >= 4`, emit the longer of KEEP and FILL — KEEP
   on a tie — and skip past it; otherwise emit one LIT up to the next position
   where such a run becomes available (or the end of the frame). This is the
   prototype's `encode_frame`, and it is deterministic. Runs longer than a
   `u16` are split (the prototype never needed to).

The encoder is desktop/wasm code: it reads the GIF into memory and decodes it
two or three times rather than keeping frames, so memory is the file plus a
few `w*h` buffers. The ESP32 never runs it.

## Measured on the card

`opendmd-pack convert ~/scratch/dmdos-card --verify` (16 jobs, 81.5 s) next
to the prototype (16 workers, 1,545 s):

| | prototype (Python) | `opendmd-pack` (Rust) |
|---|---|---|
| files converted / errors | 11,040 / 7 | 11,039 / 8 |
| total GIF bytes | 1,574,866,352 | 1,574,828,446 (less the one PNG, below) |
| total ODM bytes | 1,483,964,968 | 1,483,695,371 |
| ratio (overall / median / p95) | 0.9423 / 0.9313 / 1.8960 | 0.9421 / 0.9311 / 1.8960 |
| worst file | Gilligans_Island02, 5.658x | same, 5.658x (same top 10) |
| KEEP / FILL / LIT pixel share | 61.00% / 25.77% / 13.22% | 61.01% / 25.77% / 13.22% |
| quantised (>256 colours) | 38 | 38 |
| palette bytes max / mean | 768 / 166.2 | 768 / 166.2 |
| verified | 11,040 vs Pillow's frames | 11,001 identical to `GifSource` + 38 quantised (frame count and delays identical), 0 failures, 1,791,896 frames |

File by file, **11,000 of the 11,039 are the same size and decode to identical
frames** from both implementations. The bytes differ only in palette order
(the prototype sorts colours with `np.unique`, the Rust encoder keeps first-seen
order) and in the delay field (see above). The 39 that differ:

- **38 quantised files** — NeuQuant against Pillow's median cut. Both are
  lossy; the Rust output's worst per-channel error against the GIF is 150 (on
  dithered content), and every one of them keeps the GIF's frame count and
  delays.
- **`NINTENDO_DS_Wario_WareDIY_03.gif` (135x32)** — its frames never cover
  source columns 128-130. `GifSource` leaves never-drawn pixels black, as
  playback always has; Pillow starts the screen at the GIF's background colour
  `(107,41,57)`. The Rust file follows playback (40,086 B vs 40,302 B).

The extra error is `Consoles/NES_Unknow_01_Invert.gif`, which is a PNG with a
`.gif` name: Pillow opens it, the `gif` crate (and therefore the player) does
not. It is not in `filelist.txt`.

### The 38 quantised files

These have more than 256 distinct colours after compositing, so their `.odm`
is **not** pixel-identical to the GIF. That is the one lossy case in the
format; `opendmd-pack --verify` reports them separately (frames and delays must
still match exactly). If a quantised file looks wrong on the panel, play the
`.gif` for that entry — a manifest may freely mix `.gif` and `.odm` paths.

### Heap per playing file

`heapprobe` (`cargo run --release --example heapprobe -- -v <file>...`), live
heap after the first frame, for the seven files the GIF budget is tracked on:

| file | GIF steady / peak | ODM steady / peak |
|---|---|---|
| PINBALL_SHORT_Corvette_01 | 53,644 / 53,740 | 14,394 / 14,394 |
| PINBALL_SHORT_WWF_40 | 53,668 / 53,734 | 14,379 / 14,379 |
| SNES_Inspector_Gadget_02 | 53,716 / 53,782 | 14,388 / 14,388 |
| ARCADE_Arkanoid_03 | 53,716 / 53,782 | 14,418 / 14,418 |
| ARCADE_NEOGEO_MetalSlugFire05_Shabazz | 54,388 / 54,454 | 14,829 / 14,829 |
| SNES_Donkey_Kong_Country_04 | 53,668 / 53,734 | 14,388 / 14,388 |
| SNES_Yoshi_Island_01_01_Wolfsoft | 54,388 / 54,454 | 15,108 / 15,108 |

ODM is `12,288` canvas + `4` boxed `File` + `2,048` `BufReader` + the palette
(`3 * palette_len`, 768 max) + a handful of bytes; nothing is allocated per
frame, so peak == steady. The two strays own a centre-cropped screen as well
(27.3 KB and 27.8 KB).

### On the device

Same firmware, WiFi up, `/api/status` polled every 30 s: `filelist.txt` (GIFs)
ran at 59.2-60.2 KB free with a sampled floor of 55,044 B; a 200-entry `.odm`
set (all seven files above among them, every category) ran at 90.6-91.6 KB free
with a floor of 84,932 B over 7.5 minutes, with no decode warnings. Switching
back live: 91,264 B free before, 58,080 B with the first GIF decoding. The
firmware image grew by 9,568 B (1,903,552 -> 1,913,120; the OTA slot is
2,031,616).

## Card layout

`opendmd-pack` can put the `.odm` files in one of two places. Either way
`gifs/` and `filelist.txt` are left untouched, so the card still works with
stock DMDoS.

- **beside** (the default, `--layout beside`): `gifs/<Category>/<name>.odm`
  next to `gifs/<Category>/<name>.gif`; the manifest lists
  `/gifs/<Category>/<name>.odm`.
- **bucketed** (`--layout bucketed`): `odm/<Category>/<bb>/<name>.odm`; the
  manifest lists `/odm/<Category>/<bb>/<name>.odm`.
  - `<Category>` is the GIF's directory relative to `gifs/`, verbatim (spaces
    and underscores included: `NEW 2021`, `Pinball_Short`). A GIF nested
    deeper keeps its whole directory path (`gifs/A/B/x.gif` ->
    `odm/A/B/<bb>/x.odm`); a GIF directly in `gifs/` gets no category
    directory (`odm/<bb>/x.odm`). The stock card has neither.
  - `<name>.odm` is the GIF's file name with its final `.gif` (any case)
    replaced by `.odm` — the same name the beside layout uses, stray dots
    included (`X_01..gif` -> `X_01..odm`).
  - `<bb>` is two lowercase hex digits: the low byte of **32-bit FNV-1a over
    the UTF-8 bytes of the `.odm` file name** (extension included, no
    directories). FNV-1a: `h = 0x811c9dc5`; for each byte `h = (h ^ byte) *
    0x01000193 mod 2^32`. So 256 buckets per category.

  Worked example: `gifs/Consoles/MD_Sonic3_RattenJager.gif` -> file name
  `MD_Sonic3_RattenJager.odm` -> FNV-1a `0xd01ef790` -> bucket `90` ->
  `odm/Consoles/90/MD_Sonic3_RattenJager.odm`. (`SNES_Plok_08.odm` hashes to
  `0x4ee48ec0`, bucket `c0`.) The one implementation is
  `opendmd-core/src/layout.rs` (`fnv1a32`, `bucket`, `odm_rel`,
  `bucketed_path`), shared by `opendmd-pack` and the web app's converter
  (`bucketFor`/`odmPathFor` in wasm); its unit test pins these values.

**Why.** FAT has no directory index: opening a file scans its directory
linearly, and on the ESP32 that costs ~0.65 ms per 512-byte directory sector
(16 entries — fewer with long file names). `gifs/Consoles` holds 3,781 files,
so every open there costs ~330 ms, which is a visible stall between
animations. Split 256 ways, Consoles averages ~15 files per bucket (6-26
over the stock card's 3,779 converted files) and an open costs a few
milliseconds: measured on the rig, **5.7 ms p50 / 6.7 ms max** for the
bucketed copy of the 200-file `.odm` set against 343 / 532 ms for the same
set's files in `gifs/Consoles`, and zero frame-deadline misses in 5 minutes
of play (the beside set: 9, worst 490 ms). The web server's opens are the
same scans, so serving from the bucketed tree barely disturbs the display
(2 misses of 3 ms in 120 s of back-to-back downloads). See "Frame time" in
`CLAUDE.md`.

**Tags.** `Entry::from_path` takes the category from the first directory under
`gifs/` *or* `odm/`; under `odm/` the directory holding the file is the hash
bucket, never the category, so `/odm/Consoles/90/X.odm` gets exactly the tags
of `/gifs/Consoles/X.gif`. (A two-lowercase-hex-digit directory directly under
`odm/` with the file straight inside it is read as the bucket of an
uncategorised file.)

## Tools

- `opendmd-pack convert <card-root> [--layout beside|bucketed] [--jobs N]
  [--verify] [--force]` — writes `<name>.odm` for every `<name>.gif` under
  `gifs/`, where the layout says (see "Card layout"), copying the GIF's mtime
  onto it; an `.odm` with the GIF's mtime is up to date and skipped unless
  `--force`. In the bucketed layout a current beside `.odm` is copied instead
  of re-encoded (the encoder is deterministic, and `--verify` checks the copy
  like any other). Prints the report above.
- `opendmd-pack manifest <card-root> [--layout beside|bucketed] [--manifest
  filelist.txt] [--out filelist.odm.txt]` — the same manifest with every
  `/gifs/...gif` path switched to that layout's `.odm` path where the file
  exists, order and line endings kept.
- `tools/card-sync.py <host> --from <card copy> --paths <list>` — uploads a
  list of card paths over `PUT /api/files/…`.
- `make test-card` includes the cross-implementation check: 20 of the
  prototype's own `.odm` files, decoded by the Rust `OdmSource`, compared with
  `GifSource` on the source GIF frame by frame (`DMD_ODM_ROOT`).
