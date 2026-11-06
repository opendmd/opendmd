# The playback cache: prepared copies

A GIF's **prepared copy** is the file the player plays in its place: a
standard GIF89a the device decodes with a small, capped-LZW decoder
(`opendmd_core::gifslim::GifSlim`) in **~16 KB of heap including the 12 KB
canvas**, against ~27 KB for the original through the same decoder at its
full 12-bit table and ~54 KB through the `gif` crate. Over the stock card
the copies are **0.61x the originals' size** (960 MB for 1,575 MB), so every
frame also reads ~40% fewer bytes off a card the web server shares.

Users never see a copy: playlists name GIFs, and the firmware finds each
GIF's copy by itself (below).

## The file

An ordinary GIF89a — Pillow, ffmpeg, ImageMagick and every browser play it —
written by `gifslim::recode_gif` / `recode_gif_streaming` with
`gifslim::PREPARED` (`cap: 9, delta: true, hint: true, keep_runs: true`):

- **Frames are what playback shows.** The source is decoded with gifslim at
  cap 12 (the `gif` crate for what gifslim does not decode: interlaced
  frames, `Previous` disposal), so disposal, transparency and the
  local-over-global palette rule are already applied: each frame of the copy
  is a composited logical screen, before any centre-crop (a 135x32 stray
  stays 135x32).
- **One global palette**: the animation's distinct colours, first seen
  scanning each composited frame in raster order, then a transparent slot;
  the table is padded to a power of two (≥ 4 entries) and the minimum code
  size is its log2. **Past 256 colours there is no copy** (below).
  **Exactly 256 colours** (114 files) keeps all 256 with no
  transparent slot: each frame's transparent index is any index its changed
  pixels do not use (a frame that used every one would be sent opaque; none
  on the card does).
- **Delta frames**: frame 0 whole and opaque (a browser shows what frame 0
  leaves uncovered as the page behind it); every later frame is the minimal
  rectangle of pixels whose palette index changed, disposal **Keep**, the
  unchanged pixels inside it transparent — except that an unchanged pixel
  whose index equals the pixel before it in the rectangle repeats that index
  (`keep_runs`: same picture, longer LZW runs). A frame that changes nothing
  is one transparent pixel at (0, 0) — its delay still counts. Delays are the
  source's raw hundredths (0 and 1 included: players apply their own rules)
  and the NETSCAPE loop count is copied.
- **LZW capped at 9 bits**: the encoder emits a CLEAR as the code the decoder
  reads when its next free entry is `2^9 - 1 = 511` — the last point where a
  code (the CLEAR itself) still fits in 9 bits — so a decoder never defines
  an entry past 510 and never reads a code wider than 9 bits. Code widths
  follow the rule every GIF decoder uses (giflib, `weezl`): a code is read
  with as many bits as the decoder's next free entry needs. The dictionary is
  a 2^10-slot hash emptied at each CLEAR.
- **The hint**: an application extension right after the global palette,
  before the NETSCAPE block — 19 bytes:

  ```text
  21 FF 0B "OPENDMD1LZW" 03 01 <cap> <flags> 00      flags bit 0 = delta frames
  ```

  `GifSlim::new` reads it before sizing its tables (`prefix: u16[2^cap]`,
  `suffix: u8[2^cap]`: 1.5 KB at cap 9, 12 KB at 12), matching the id
  case-insensitively (ImageMagick lowercases it). It is only a promise: a
  code wider than the cap is `NeedsFullDecoder`, and the player then plays
  the file with the `gif` crate (below). A tool that rewrites the file may
  keep the block and break the cap (ImageMagick does); the `gif` crate and
  Pillow drop it.
- **Container**: byte for byte what the `gif` crate's encoder writes for the
  same frames (header, padded global palette, the hint, NETSCAPE, a graphic
  control extension before every frame, 255-byte data sub-blocks, the
  trailer) — a unit test holds the hand-written streaming writer to it.

## Exact or absent: GIFs past 256 colours

A copy is the GIF's frames exactly, or there is none. A GIF with more than
256 distinct colours on its composited frames (38 on the stock card) could
only get a quantised copy (`NeuQuant` made them until 2026-10-23: worst
channel error 151), while the player plays the original exactly through
gifslim at cap 12 (~27 KB of heap). So all
three encoders refuse it (`gifslim::TooManyColours`, nothing written) and it
**plays as it is**:

- Whoever meets it records it in `cache/.failed` as
  `/gifs/…/X.gif<TAB>more than 256 colours: plays as it is`
  (`gifslim::ORIGINAL_REASON`; `gifslim::plays_original` also accepts the
  bare `more than 256 colours` earlier firmware wrote): the device's
  builder, the browser (`POST /api/cache/failed`), `opendmd-pack convert`
  (once per path).
- It **counts as done**: `GET /api/cache`'s `prepared` counts the playlist
  entries with a copy *or* such a line, and the builder's `failed` leaves
  them out. Nothing tries it again (deleting `cache/.failed` does, and it
  is recorded again) until the GIF is written anew: a `PUT` of a GIF drops
  its line.
- **Smooth motion** never applies: the tweened frames have the GIF's
  colours, so their copy would be lossy too. The Library does not offer the
  switch (`Smoother.fits`, `tween::Smooth::fits_palette`).
- **Old lossy copies** are removed where they can be recognised. A
  quantised copy always has a 256-entry global table (255 colours and the
  transparent slot), so `gifslim::copy_may_be_quantised(header)` rules out
  every copy with a smaller table, and `gifslim::fits_palette(GIF)` (the
  colour pass alone, stopping at the 257th) decides the rest:
  `opendmd-pack convert` checks every up-to-date copy that way and removes
  the 38 on the stock card (19 s with `--verify` on 16 jobs). The browser
  has no GIFs to count without downloading them all, so "Prepare remaining
  files" re-checks only a GIF `cache/.failed` already says plays as it is
  that has a copy all the same (made again: past 256 colours the copy goes,
  else — a GIF replaced since — the exact copy stays). A card whose lossy copies were made by an older browser or pack
  and that the device's builder never met keeps them until
  `opendmd-pack convert` runs on it.

**One encoder, three callers, the same bytes.** `recode_gif_streaming` is two
passes over a seekable source — the first collects colours, frame count, size
and loop count, the second writes the GIF a frame at a time with the LZW
streamed out as it is made — in bounded memory (below). `recode_gif`, the
in-memory entry point, *is* that function over a `Cursor`. `opendmd-pack convert`, the browser (`prepareGif`, in the art wasm module
that `www/js/artmod.js` loads on first use, for "Prepare remaining files",
Add GIFs and the Studio) and
the device's cache builder therefore write identical files: checked over the
whole stock card (`encodeprobe`: 11,001 byte-identical, the 38 past 256
colours refused by both, 8 undecodable by both, 0 mismatches; and the
tree it wrote equals `opendmd-pack convert`'s file for file), in the
art-web crate's wasm tests and web-smoke (every copy the browser makes
equals `opendmd-pack convert`'s), and on a player (copies the
builder made, downloaded, equal `opendmd-pack`'s for the same sources).

## Smooth motion: a smoothed copy

A GIF can be marked for **Smooth motion** (Library, per GIF; or
`opendmd-pack smooth analyse --write`): its copy is then made from the
GIF's frames with **in-between frames added** where the motion moves in
steps of 2 px or more — the integer-shift tweener,
`opendmd_core::tween`
(the `tween` cargo feature: built into `opendmd-pack` and the art wasm
module only, never the firmware). Every in-between pixel is a pixel of one
end moved by whole pixels; the total duration is unchanged. The GIF itself
is never touched.

- **The copy is `recode_gif(tween(GIF))`**: the tweened frames are written
  as a plain GIF (every frame whole, 12-bit LZW, the source's loop count;
  one global palette of the source's colours when it has ≤ 256, else one
  per frame) and that goes through the recoder in `PREPARED` like any GIF,
  so everything above holds (cap 9, delta frames, the hint; past 256 colours
  no copy at all). The browser
  (the art module's `Smoother` / `prepareSmoothGif`) and `opendmd-pack
  convert` write byte-identical files (web-smoke compares them; the art
  crate's tests pin a fingerprint natively and in wasm). The device never
  makes one: its builder only prepares GIFs that have no copy, so it leaves
  a smoothed copy alone — and after the cache is cleared it makes a plain
  one, which the browser then replaces (below).
- **The marker**: a second application extension, right after the hint —
  21 bytes:

  ```text
  21 FF 0B "OPENDMD1TWN" 05 01 <pairs u16 LE> <added u16 LE> 00
  ```

  identifier `OPENDMD1`, authentication code `TWN`; one data sub-block:
  version 1, the frame pairs tweened and the frames added (each saturating
  at 65,535). `GifSlim` and the `gif` crate skip it like any unknown
  application extension (a unit test plays a marked copy through both, frame
  for frame equal to the unmarked one); `tween::marker` finds it in the
  file's first bytes (≤ ~830 B: header, palette, the blocks before frame 0).
  Delays on a smoothed copy are the split ones (a 50 ms frame of a 2 px
  scroll becomes 30 + 20 ms on the GIF's 10 ms grid); frames that were not
  split keep their raw delay.
- **The list**: `/sets/.smooth` (`tween::LIST_PATH`), one card-absolute GIF
  path a line (`/gifs/Other/OTHER_SCROLL_Xmen01.gif`), UTF-8, LF, compared
  case-insensitively. Beside the playlists (and hidden, like `/sets/.ctx`)
  and **outside `cache/`**, so `DELETE /api/files/cache?recursive=1` keeps
  the choice. Not `.txt` (it would be listed as a playlist) and not `.meta`
  (a playlist's overrides); the firmware needs nothing new: `GET
  /sets/.smooth` is served like any file under `/sets/`, and `PUT
  /api/files/sets/.smooth` goes through the manifest install every file
  directly in `sets/` gets (no `.idx` to remove, never the playlist in play:
  just the temp + rename on the main task). There is no API for it; the
  browser writes it whole.
- **Reconciliation, not enforcement.** Settings → Device → Playback cache's "Prepare
  remaining files" reads the list and checks each listed GIF's copy (a GET
  of its first KB, the stream then cancelled — neither the firmware nor the
  mock serves ranges): missing, or without the marker (the builder made a
  plain one after a wipe), it is made smooth again in the same run. Turning
  Smooth motion off makes the plain copy again (it is not just deleted) and
  then drops the line.
- **`opendmd-pack convert`** honours the list when the card has one: a
  listed GIF's copy is smooth, every other copy plain, and a copy that
  disagrees is re-encoded whatever its mtime. With no list on the card a
  marked copy is kept smooth. `--verify` checks a smoothed copy is exactly
  `recode_gif(tween(GIF))` byte for byte, then plays it through the same
  three decoders against the tweened frames instead of the GIF's.

## Where copies live

`cache/<Category>/<xx>/<name>.gif`:

- `<Category>` is the GIF's directory relative to `gifs/`, verbatim (spaces
  and underscores included: `NEW 2021`, `Pinball_Short`). A GIF nested deeper
  keeps its whole directory path (`gifs/A/B/x.gif` -> `cache/A/B/<xx>/x.gif`);
  a GIF directly in `gifs/` gets no category directory (`cache/<xx>/x.gif`).
- `<name>.gif` is the GIF's own file name.
- `<xx>` is two lowercase hex digits: the low byte of **32-bit FNV-1a over
  the UTF-8 bytes of the file name** (extension included, no directories).
  FNV-1a: `h = 0x811c9dc5`; for each byte `h = (h ^ byte) * 0x01000193 mod
  2^32`. So 256 buckets per category.

Worked example: `gifs/Consoles/MD_Sonic3_RattenJager.gif` -> file name
`MD_Sonic3_RattenJager.gif` -> FNV-1a `0x52903b38` -> bucket `38` ->
`cache/Consoles/38/MD_Sonic3_RattenJager.gif`. (`SNES_Plok_08.gif` hashes to
`0x10b6d368`, bucket `68`.) The one implementation is
`opendmd-core/src/layout.rs` (`fnv1a32`, `bucket`, `cache_rel`,
`cache_path`), shared by `opendmd-pack`, the web app (`bucketFor`,
`cachePathFor` in wasm) and the firmware; its unit test pins these values —
a change is a card-format break, not a refactor.

**Why buckets.** FAT has no directory index: opening a file scans its
directory linearly, ~0.65 ms per 512-byte directory sector on the ESP32.
`gifs/Consoles` holds 3,781 files, so an open there costs ~330 ms, a visible
stall between animations; split 256 ways it is ~15 files a bucket and an open
costs a few milliseconds.

**Tags.** `Entry::from_path` takes the category from the first directory
under `gifs/` *or* `cache/`; under `cache/` the directory holding the file is
the bucket, never the category, so `/cache/Consoles/38/X.gif` gets exactly the
tags of `/gifs/Consoles/X.gif`.

`cache/.failed` is the list of GIFs with no copy: those the cache builder or
the browser could not prepare, and those that play as they are (past 256
colours, above) (`path<TAB>reason` a line; `GET/POST/DELETE
/api/cache/failed`).

**The retired format.** Cards prepared before this change have an `odm/`
tree of `.odm` files. The firmware never reads it (it logs once at boot that
it is there) and `DELETE /api/files/odm?recursive=1` removes it, a few
seconds' work per request (`"done":false` means ask again).

## Playback

- **Resolution** (firmware `sd::SdSource::open`): for a `/gifs/…/X.gif`
  path, open `layout::cache_path` first; if it opens and starts with `GIF8`,
  play it, else play the GIF (a miss is one failed lookup in a small
  directory, ~5-6 ms). A playlist line that already names a
  `/cache/` path is opened as it is. The engine, `/api/status` and the web
  app only ever see the GIF's path; the log says `<- prepared` or
  `<- original`.
- **Decoding** (`player::FrameSource`): a `GIF8` file opens with `GifSlim` —
  at the hint's cap (a copy: 9), else at 12. What gifslim does not decode —
  interlaced frames or `Previous` disposal (`Unsupported`, 5 of the stock
  card's originals), or a code wider than the hinted cap (`NeedsFullDecoder`,
  a copy some tool re-encoded) — falls back to the `gif` crate: the reader
  is rewound, gifslim's tables are dropped before the crate's decoder is
  built, and the frames already shown are decoded again onto the canvas, so
  the file carries on from the frame that failed. The engine logs that once
  (info) and loops the file with the crate directly.
- **Heap** (`heapprobe`, canvas included; steady == peak): a copy at cap 9
  16.1-16.8 KB; an original through gifslim at cap 12 26.8-27.5 KB; the `gif`
  crate 53.6-54.4 KB.

## The device's encoder

The cache builder (`firmware/src/cache_builder.rs`) runs
`recode_gif_streaming` over the card file, writing `<copy>.tmp` through a
2 KB buffer, syncing it and renaming it into place. Its heap, for a 128x32
GIF: gifslim at cap 12 (~15 KB: the 2 KB read buffer, 12 KB of tables, a row,
the source's palette), a canvas (12 KB), the previous frame's indices (4 KB),
the changed bits (512 B), the colour table (~2.8 KB), the LZW dictionary
(6 KB at cap 9) and the write buffer (2 KB). Measured with a counting
allocator over the whole card (`encodeprobe --heap`). A file gifslim cannot decode needs the `gif` crate
(~26 KB more); the builder uses it only if that much is free and otherwise
records the file as "too large to prepare here", which the browser then
prepares. More than 256 colours is recorded as playing as it is (above).

## Tools

- `opendmd-pack convert <card-root> [--jobs N] [--verify] [--force]` —
  writes every GIF's copy under `cache/`, copying the GIF's mtime onto it (a
  copy with the GIF's mtime is up to date and skipped unless `--force`).
  `--verify` plays every copy against its GIF through three decoders at once
  (original/`GifSource`, copy/`GifSource`, copy/`GifSlim` at cap 9): every
  canvas and delay identical. GIFs past 256 colours get none ("N play as
  they are", any old lossy copy removed, `cache/.failed` lines added) and
  count as done. Stock card, 16 jobs, loaded host: 11,001 copies
  identical, 38 playing as they are (`done: 11039 of 11047`); 8 GIFs
  neither decoder can play get no copy.
- `cargo run --release --example encodeprobe -- [--heap] [--out <root>]
  <gifs-dir>` (in `opendmd-core`) — the in-memory and streaming encoders
  against each other over every file, and the streaming one's peak heap.
- `opendmd-pack smooth analyse <card-root> [--min-percent 10] [--jobs N]
  [--write]` — which GIFs Smooth motion would change (≥ 10% of their frame
  pairs, the Library's threshold), with the frames each would gain; `--write`
  adds them to `/sets/.smooth` (lines already there are kept). The stock
  card: 1,002 files at ≥ 10% (+53,879 frames), 48,523 of 1,791,893 pairs
  (2.71%) tweenable card-wide, in 68.5 s on 16 jobs.
- `make test-card CARD_ROOT=<card>` — includes `prepared_copies_on_the_real_card`:
  twenty copies equal `recode_gif`'s bytes, open as gifslim at cap 9 and play
  every frame exactly as their GIFs do; then 50 entries through the engine
  with the firmware's resolution rule.
