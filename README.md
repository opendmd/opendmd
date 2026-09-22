# OpenDMD

An open-source replacement for **DMDoS**, the closed ESP32 firmware that plays
128x32 animated GIFs from a microSD card onto two chained 64x32 HUB75 LED
panels (a "DMD" — the dot-matrix display out of a pinball machine).

This repository holds the platform-independent core, a desktop simulator, and
the ESP32 firmware.

## Layout

Three standalone crates, deliberately **not** a Cargo workspace. The core and
the simulator build and test with plain stable Rust on any desktop; the
firmware has its own xtensa build config under `firmware/`.

| Path | What it is |
|------|------------|
| `opendmd-core/` | Library. Canvas, manifest/tag parsing, filtering, scheduling, streaming GIF playback, playlist engine. No device, window or filesystem dependencies. |
| `opendmd-sim/` | Binary. A `minifb` window that plays a card through `opendmd-core`. Glue and rendering only. |
| `firmware/` | ESP32 firmware. microSD + HUB75 + the core engine. See "Firmware". |
| `hardware/` | Measured HUB75 + SD SPI pinout and photos. See `hardware/PINOUT.md`. |

## Build and run

Stable Rust for the core and the simulator; the firmware needs the Espressif
xtensa toolchain as well.

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

## Firmware

`firmware/` plays a card on the real hardware: an ESP32 DevKit V1 (classic
ESP32, 4 MB flash) on a DMDos carrier, a microSD over SPI, and two chained
64x32 HUB75 P4 panels driven by
[mrcodetastic/ESP32-HUB75-MatrixPanel-DMA](https://github.com/mrcodetastic/ESP32-HUB75-MatrixPanel-DMA)
3.0.14. Boot mounts the card, reads `/sdcard/filelist.txt`, and runs the same
`opendmd-core` engine the simulator does, one frame at a time, straight into
the panels' DMA buffer.

This phase is a standalone player: no WiFi, no web portal, no OTA.

### Toolchain

```bash
cargo install --locked espup ldproxy espflash
espup install --targets esp32 --export-file ~/.espup-env.sh
uv tool install esptool
git submodule update --init          # the HUB75 driver
```

### Build and flash

```bash
make fw-build                        # first build clones ESP-IDF v5.3 (~5-10 min)
make fw-erase   PORT=/dev/ttyUSB0    # once, if the board ran other firmware
make fw-flash   PORT=/dev/ttyUSB0    # build + flash at 115200 + monitor
make fw-monitor PORT=/dev/ttyUSB0
make fw-clean
```

ESP-IDF is installed once into `~/.espressif` and shared between projects.
Everything tunable — pin map, panel driver chip and timing, brightness,
platform filter, dwell and retry intervals — is a constant in
`firmware/src/config.rs`. If the panels stay dark, the first thing to change
is `config::DRIVER` from `SHIFTREG` to `FM6126A`.

### Serial output

```
I opendmd_firmware: OpenDMD firmware v0.1.0 — 128x32 HUB75 GIF player
I opendmd_firmware: microSD mounted at /sdcard (31.27 GB), heap 259516 B free
I opendmd_firmware: panel up: 128x32 (2 x 64x32), driver=0, double_buff=true, hub75_refresh_rate=100 Hz, brightness=96
I opendmd_firmware: manifest indexed: <n> entries, <4n> index bytes, heap <free> B free
I opendmd_firmware: [1] /gifs/Consoles/NES_Super_Mario_Bros_16.gif
I opendmd_firmware: heartbeat: heap <free> B free (min <min> B), refresh 100 Hz, 5 GIFs started
```

A missing card is not fatal: the mount is retried every 5 seconds, forever,
and so is an unreadable `filelist.txt`.

### The playlist is indexed, not loaded

A classic ESP32 has ~260 KB of usable DRAM, ~120 KB of it left once the HUB75
DMA buffers are allocated. The real card's `filelist.txt` is 450 KB and 10,162
lines; `std::fs::read_to_string` on it aborts with "out of memory" outright,
and `parse_manifest` would then want roughly 6 MB of `Entry` values (~590
bytes each, measured on this part).

So the firmware never holds the manifest. It opens the file once and hands it
to `opendmd_core::engine::FileCatalog`, which makes a **single streaming pass**
and keeps one `u32` byte offset per line that passes the filter — **40,648
bytes for the whole card**, 6,748 for a `MD,SNES` playlist. The file handle
stays open and the engine seeks back into it for one `Entry` at a time, only
when it is about to open the next GIF, so the transient cost is one line and
one entry. The whole card is playable in one session, and running out of
openable files reshuffles the existing index (`Engine::reset`) rather than
re-reading 450 KB over SPI.

The simulator uses exactly the same path, so `make sim` exercises what the
firmware does.

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
