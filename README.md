# OpenDMD

An open-source replacement for **DMDoS**, the closed ESP32 firmware that plays
128x32 animated GIFs from a microSD card onto two chained 64x32 HUB75 LED
panels (a "DMD" — the dot-matrix display out of a pinball machine).

This repository currently holds the platform-independent core and a desktop
simulator. The ESP32 firmware crate comes later and will live at the repository
root.

## Layout

Two standalone crates, deliberately **not** a Cargo workspace. The ESP32
firmware will be a third crate under `firmware/` with its own xtensa build
config; these two build and test with plain stable Rust on any desktop.

| Path | What it is |
|------|------------|
| `opendmd-core/` | Library. Canvas, manifest/tag parsing, filtering, scheduling, streaming GIF playback, playlist engine. No device, window or filesystem dependencies. |
| `opendmd-sim/` | Binary. A `minifb` window that plays a card through `opendmd-core`. Glue and rendering only. |
| `hardware/` | Measured HUB75 + SD SPI pinout and photos. See `hardware/PINOUT.md`. |

## Build and run

Stable Rust, no other toolchain needed yet.

```bash
make test                       # unit tests (opendmd-core)
make check                      # cargo check both crates
make clippy                     # clippy -D warnings, both crates
make fmt
make sim ARGS="--root /path/to/card --platforms MD,SNES"
```

`make sim` needs a display; the simulator opens a `128 * scale` by
`32 * scale` window.

```
--root <dir>              card root, holding gifs/ and filelist.txt (required)
--manifest <path>         default <root>/filelist.txt
--platforms MD,SNES       comma-separated platform prefixes
--title <substr>          repeatable, matched against the normalized title
--exclude-flag INVERT     repeatable
--category / --exclude-category   repeatable, matched against gifs/<Category>
--shuffle --seed <u64>    reproducible weighted shuffle
--scale <u32>             pixels per LED, default 8
--min-dwell / --max-dwell seconds, default 3 and 30
```

Keys: `Space` next, `P` pause/resume, `Esc` quit. Each new GIF prints
`[n/total] /gifs/Consoles/SNES_Plok_08.gif`; startup prints the filter summary.

## The card

A DMDoS card root looks like:

```
gifs/<Category>/<file>.gif    18 categories: Consoles, Arcade, Pinball_Short,
                              Halloween, XMAS, Other, "NEW 2021", ...
logos/
filelist.txt
filelist.hall                 seasonal subsets, same format
filelist.xmas
```

`filelist.txt` is one path per line: LF-separated, leading slash, no BOM, in
insertion order rather than sorted. The stock firmware reads only
`/filelist.txt` and plays it sequentially. OpenDMD is tolerant on input (BOM,
CRLF, blank lines, trailing whitespace) and canonical on output (one path per
line, LF-terminated — byte-identical to the shipped card file).

Every GIF in the archive is 128x32 except two strays (132x32 and 135x32).
Mismatched sizes are centred and cropped, never resampled — it is pixel art and
any scaling filter ruins it.

## Tag grammar

Filenames encode tags as `PLATFORM_Title_Words_NN[_FLAG].gif`. It is a
convention rather than a format, so parsing is best-effort and derived from the
real ~10k-file card. Tokens are split on `_`:

| Filename | Platform | Title | Variant | Flags |
|---|---|---|---|---|
| `SNES_Donkey_Kong_Country_05.gif` | SNES | Donkey Kong Country | 5 | |
| `MD_Sonic3_RattenJager.gif` | MD | Sonic3 | | RattenJager |
| `SNES_Asterix01.gif` | SNES | Asterix | 1 | |
| `NINTENDO_DS_Mario_Kart_DS_01..gif` | NDS | Mario Kart DS | 1 | |
| `NES_Unknow_01_Invert.gif` | NES | Unknow | 1 | Invert |
| `HALLOWEEN_ARCADE_GoldenAxe16.gif` | ARCADE | GoldenAxe | 16 | HALLOWEEN |
| `MD_LOGO_Sega31.gif` | MD | Sega | 31 | LOGO |
| `Nes_Gradius.gif` | — | Nes Gradius | | |

Rules, in order:

1. A leading `HALLOWEEN` / `XMAS` becomes a flag only when a known platform
   prefix follows it; otherwise it *is* the platform. The directory category is
   the authoritative source of seasonality either way.
2. Multi-token prefixes (`NINTENDO_DS` -> `NDS`, `SEGA_MEGACD`, `VIRTUAL_BOY`,
   `SEGA_PICO`), then the known single-token list, then any unknown token that
   *looks* like a platform: `A-Z`, `0-9`, `-`, at least one letter — so `PICO-8`
   works and `Nes` doesn't. Anything else leaves the platform unset and the
   whole stem becomes the title.
3. Keyword flags (`INVERT`, `INTRO`, `LOGO`, `SLIDE`, `STORY`, ...) directly
   after the platform.
4. Trailing tags right-to-left: keyword flags, a standalone all-digit token as
   the variant, and author handles.
5. Failing that, trailing digits glued to the last word are the variant — but
   only two or more of them, because a single glued digit is almost always a
   sequel number (`Sonic3` stays whole; `Asterix01` becomes Asterix variant 1).

Author handles can't be told from title words in general (`StarWars01`,
`SuperMario` and `RattenJager` are all trailing CamelCase), so a token counts as
an author only if it starts with a lowercase letter (`sephirot68`, `clivefrog`)
or is in a small known list, and only when dropping it leaves a non-empty
title. Flags are stored verbatim; every flag comparison is case-insensitive.
`normalize_title` (lowercase, alphanumerics only) is the hook for fuzzy
matching a user's own collection list.

Against the real card this parses 10,162 entries with a non-empty title each,
and 3,784 of 3,786 `gifs/Consoles` entries get a platform (99.95%; the two
misses are `Nes_Gradius.gif` and `NEs_Captain_America...gif`).

## License

Licensed under either of

- Apache License, Version 2.0 ([LICENSE-APACHE](LICENSE-APACHE))
- MIT license ([LICENSE-MIT](LICENSE-MIT))

at your option.

Unless you explicitly state otherwise, any contribution intentionally submitted
for inclusion in this work by you, as defined in the Apache-2.0 license, shall
be dual licensed as above, without any additional terms or conditions.

**The DMDoS GIF pack is not covered by this and is not part of this
repository.** The ~11,000 animations on a DMDoS card are third-party,
unlicensed content: they are not ours to relicense or redistribute, and they
must never be committed here. `.gitignore` carries guards against doing it by
accident. Every test fixture in this repo is a GIF generated in code at test
time.
