# OpenDMD

An open-source replacement for **DMDoS**, the closed ESP32 firmware that plays
128x32 animated GIFs from a microSD card onto two chained 64x32 HUB75 LED
panels (a "DMD" — the dot-matrix display out of a pinball machine).

This repository holds the platform-independent core, a desktop simulator, and
the ESP32 firmware.

## Layout

Six standalone crates, deliberately **not** a Cargo workspace. The core, the
renderer and the simulator build and test with plain stable Rust on any
desktop; the web app needs the `wasm32-unknown-unknown` target; the firmware
has its own xtensa build config under `firmware/`.

| Path | What it is |
|------|------------|
| `opendmd-core/` | Library. Canvas, manifest/tag parsing, filtering, scheduling, streaming GIF playback, playlist engine. No device, window or filesystem dependencies. |
| `opendmd-render/` | Library. The LED-panel renderer, shared by the simulator and the web app. Depends only on `opendmd-core`. |
| `opendmd-sim/` | Binary. A `minifb` window that plays a card through `opendmd-core`, drawing it as a simulated LED panel. Glue only. |
| `opendmd-web/` | WebAssembly front end: the same engine and renderer in a browser page. See "The web app". |
| `opendmd-pack/` | Binary. Prepares a card: a pre-rendered `.odm` copy of every GIF, which the player plays in its place. See "Prepared copies". |
| `firmware/` | ESP32 firmware. microSD + HUB75 + the core engine. See "Firmware". |
| `hardware/` | Measured HUB75 + SD SPI pinout and photos. See `hardware/PINOUT.md`. |

## Build and run

Stable Rust for the core and the simulator; the firmware needs the Espressif
xtensa toolchain as well.

```bash
make test                       # unit tests (core, render, sim, web, pack)
make check                      # cargo check the host crates
make clippy                     # clippy -D warnings, every crate
make fmt
make sim ARGS="--root /path/to/card --platforms MD,SNES"

rustup target add wasm32-unknown-unknown
cargo install --locked wasm-pack
make check-wasm                 # core + render + web for wasm32
make test-wasm                  # the web crate's tests under node
make web                        # wasm-pack build -> opendmd-web/www/pkg
make web-serve                  # static server on http://localhost:8080
make web-smoke                  # headless Chromium drive of the page (tools/web-smoke.py)
```

`make sim` needs a display; the simulator opens a `128 * scale` by
`32 * scale` window — 1280x320 at the default scale of 10.

### What it draws

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

## The web app

The management UI is a **WebAssembly page the player serves off its own SD
card** at `http://opendmd.local/`. All the heavy work happens in the browser —
parsing and filtering manifests, previewing files with the panel look,
preparing GIFs for playback — with the same `opendmd-core` and `opendmd-render`
Rust the firmware and the simulator run. The ESP32 is a file server plus the
small JSON API under "Firmware". No framework and no bundler: hand-written
HTML, CSS and ES modules in `opendmd-web/www/`, plus the `opendmd-web` wasm
module.

```bash
make web                                   # wasm-pack build + a .gz beside every file + the size budget
make web-mock ROOT=~/scratch/dmdos-card # a fake device on :8080 (OVERLAY=dir keeps the card copy untouched)
make web-mock PROXY=http://192.168.1.69    # this build, same-origin against a real device
make web-serve                             # plain static server: local mode
make web-smoke                             # headless Chromium through every view (DEVICE=http://… adds a real-device pass)
make card-sync HOST=opendmd.local          # put the app on the card
```

**Device mode or local mode.** The page asks `GET /api/status`; if a player
answers, it is in device mode and everything below talks to that device. If
not (a laptop running `make web-serve`), it offers a folder picker: point it
at an extracted copy of a card and the same views work over the picked files,
read one at a time as they are needed. Writes in local mode go to a folder you
grant (Chromium's File System Access API) or become downloads.

**Users see GIFs and playlists.** A GIF's *prepared copy* (a pre-rendered
`.odm`, "Prepared copies" below) is a cache: the player plays it when it
exists, previews fetch it first and fall back to the GIF, and the UI never
names the format or shows a copy's path.

**The views** — a left rail on a laptop, a bottom bar on a phone:

- **Now** — what the panel is playing (title, platform and category parsed
  from the path), polled every 2 s, with skip, pause, a brightness slider and
  **Send to panel** (any card path, `POST /api/play`: it plays now and the
  schedule carries on), and the device's health: uptime, address, free heap and its sampled floor,
  refresh rate, deadline misses and worst lateness. A **panel-look preview**
  of the same file plays alongside — fetched once when the file changes and
  played in the browser, so it is *not frame-synced* with the panel. It
  fetches the file's prepared copy first (cheap for the device; one 404 when
  there is none) and can be switched off. The LED-look controls (gain,
  bloom, gamma, flat) are here too and are remembered per browser. In local
  mode this view is the simulator: a manifest or the whole folder, a platform
  filter, shuffle and a seed, with the firmware's 3-30 s dwell.
- **Library** — the card's GIFs by category (`GET /api/files`), thousands
  of entries in a virtualised list, each name run through the core's tag
  parser. Search titles (normalised the way the core matches them), filter by
  platform chips, click to preview (and, on a device, **Send to panel**).
  Select entries to add them to a playlist. **Add GIFs** (device) uploads
  files into the category shown, prepares each in the browser (GIF, then its
  copy, one file at a time) and offers to add them to the playlist in play.
- **Playlists** (`#sets`) — the card's own (`filelist.txt` as **All files**,
  the default; `filelist.hall` **Halloween**; `filelist.xmas` **Christmas**;
  any other `filelist.*` by file name; an obsolete `filelist.odm.*` never)
  and `/sets/*.txt`. Open one to edit it; build a new one with the core's
  `Filter` (platforms, categories, title words, dropped flags, shuffle with a
  seed, a limit) over any playlist, or from the Library selection; reorder,
  remove, save (`PUT /api/sets/<name>`), activate (`PUT /api/config {"set":
  …}` — `""` for All files, a root manifest's file name, or a set's name —
  which rebuilds the device's index in a few seconds) and delete. Deleting
  the active playlist switches the player back to All files first. Switching
  is disabled while status says an index is `building`. Entries are always
  GIF paths.
- **Settings** — WiFi (with a scan) and hostname, which need a reboot;
  brightness, dwell, shuffle, platform filter and playlist, which apply
  live (not while an index is building); the **Playback cache** card ("9,826
  of 10,133 files prepared", from `GET /api/cache`) with **Prepare remaining
  files**, which works out what the playlist in play is missing
  (`GET /api/cache/list`) and encodes and uploads it in the background, one
  file at a time, with progress, an ETA and a stop button (starting again
  carries on); rebuild the index, reboot, and OTA firmware upload with
  progress; the firmware version, flash variant, panel facts, `index_state`
  and the web app's own version (`www_version`, from `www/version.txt`).

**Size budget**, checked by `make web` (`tools/web-size.py`) on the gzipped
sizes, which are what crosses the WiFi: the wasm module plus its generated JS
glue at most 100 KB, the hand-written JS and CSS at most 40 KB. Today that is
about 189 KB of wasm + 27 KB of glue (90.5 KB gzipped) and 104 KB of JS and
CSS (38.8 KB gzipped). `make web` writes a `<file>.gz` beside each file, and
the player sends those in preference.

**Favourites, hiding and schedules.** Right-click a GIF in the Library or a
playlist (long-press on a phone, or its "…" button) to make it a favourite,
play it less, hide it, play it longer or loop it — per playlist, so a GIF can
be a favourite in one and hidden in another. Settings → Time sets the zone
(the player keeps time over NTP), and Settings → Schedule picks a playlist
and/or brightness for parts of the week ("Weekdays 18:00–23:00 → Consoles")
and a night mode that turns the panel off or dims it; Now shows when either
is in force and can wake the panel for an hour. The saved playlist and
brightness come back when a rule ends.

**The mock device.** `tools/mock-device.py` (a `uv` script, standard library
only) serves the built app plus a fake device over an extracted card copy:
`/api/status` walking the active manifest (the default rule included),
config, sets, play, stat, the playback cache (`/api/cache`, `/list`,
`/forget`), file uploads and deletes, reindex, a canned WiFi
scan, and the card's files — with the firmware's JSON shapes and quirks.
`--overlay dir` sends every write there so the card copy is never modified;
`--strict` drops the mock's few deliberate deviations and adds the
firmware's directory-scan cost to stat and play; `--proxy URL` forwards
everything but the app to a real device.

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

## Prepared copies (`.odm` files)

An `.odm` file is a GIF that has already been played: every frame fully
composited, one palette for the whole file, and each frame stored as KEEP /
FILL / LIT runs against the previous one. The player needs about **14.5 KB**
of heap for one instead of **~54 KB** for the GIF — no LZW decoder, no GIF
read buffer — which is headroom the ESP32 spends on WiFi and serving. Across
the stock card they are 0.94x the size of the GIFs (median 0.93x; dithered
content can be several times bigger), and all but 38 of 11,039 files decode to
exactly the frames the GIF player shows. Those 38 have more than 256 colours
and are quantised; play the `.gif` for any that look wrong. The byte-level
spec is [`docs/odm-format.md`](docs/odm-format.md); `tools/odm-proto.py` is
the Python reference it came from.

**Users see GIFs and playlists; prepared copies are a cache.** Playlists
only ever name GIFs. For every `/gifs/<Category>/<name>.gif` it opens, the
firmware first tries the copy at `/odm/<Category>/<bb>/<name>.odm` (`<bb>` =
low byte of FNV-1a-32 of the `.odm` name, 256 buckets per category, so a
lookup scans ~15 entries, never the 3,781-file `gifs/Consoles`) and plays it
if it opens and is an ODM file; otherwise the GIF. A miss costs ~5-6 ms (one
failed open in a small directory, measured on the rig). A manifest line that
already names an `/odm/...` path is opened as it is, so old manifests keep
working, but nothing writes them any more and `filelist.odm.txt` is ignored.
To prepare a card copy:

```bash
# odm/<Category>/<bb>/<name>.odm for every GIF; skips files already
# prepared; --verify plays both and compares every frame
make pack ARGS="convert /path/to/card --layout bucketed --verify"

# onto a running player instead: the files (or use the web app's
# Settings -> Playback cache, which prepares them in the browser)
uv run tools/card-sync.py <host> --from /path/to/card --paths list.txt
```

**The player also prepares them itself.** While the panel is dark for the
night, paused, or has no playlist, it walks the playlist in play and writes
the missing copies with a one-pass encoder that needs ~57 KB of heap —
byte-identical to `opendmd-pack` and the browser for every file of 256
colours or fewer (11,001 of the stock card's 11,047; the 38 with more
colours, and 8 GIFs that will not decode, are left for the browser, which
has the quantiser). `auto_prepare` in `PUT /api/config` is `"idle"` (the
default: only then), `"always"` (also slowly while a prepared file plays) or
`"off"`; Settings → Playback cache shows it as one sentence ("Preparing
files automatically when the panel is idle · 4,210 of 10,133 · about 3
nights left") with the option "also while playing (slower)". On the rig
the stock card goes at ~1.7 s a file at full speed, so a card with none
prepared needs about 5 hours of night.

That is all: no manifest to write, nothing to select. (`opendmd-pack
manifest` still rewrites a manifest to `.odm` paths, for tooling and tests.)
`GET /api/cache` reports how many of the active playlist's entries have a
copy. The copies leave `gifs/` and `filelist.txt` alone, so the card still
works with stock DMDoS; the layout and its hash are described under "Card
layout" in [`docs/odm-format.md`](docs/odm-format.md).

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

It is also on the network: WiFi, a plain-HTTP server on port 80 that serves
the web app and the card's files, a small JSON API, and OTA firmware updates.
See "Getting on WiFi" and "Putting the web app on the card" below.

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

make card-sync HOST=opendmd.local    # push the web app onto the card
make api-load  HOST=opendmd.local    # stress the file server for 120 s (PATHS=list.txt: fetch those paths)

make fw-build FW_FEATURES=probe      # with the timing probe (off, and compiled out, by default)
uv run tools/sdprobe-report.py serial.log --boot -1   # open latency, miss causes, blit times
```

ESP-IDF is installed once into `~/.espressif` and shared between projects.
Everything tunable — pin map, panel driver chip and timing, brightness,
platform filter, dwell and retry intervals — is a constant in
`firmware/src/config.rs`. If the panels stay dark, the first thing to change
is `config::DRIVER` from `SHIFTREG` to `FM6126A`.

### Getting on WiFi the first time

With no credentials saved, the player raises an **open access point called
`OpenDMD-Setup`**. Join it from a phone or laptop and the setup page appears by
itself as a "sign in to network" sheet; if it does not, open
<http://192.168.71.1/> (the log prints the actual address). Enter your network
and password, press **Save & reboot**, and the player joins your WiFi and
announces itself as **`opendmd.local`**.

The AP is open, so it does not stay up indefinitely: after ten minutes the
board reboots, which retries your network if credentials were saved and
otherwise brings the AP back with a fresh timer.

`http://<ip>/setup` is always there afterwards — status, WiFi, playback
settings, a file uploader, OTA and a reboot button — whatever else is on the
card.

For a bench board you can skip the dance: copy `firmware/.env.example` to
`firmware/.env`, fill in `OPENDMD_WIFI_SSID` and `OPENDMD_WIFI_PASS`, and
rebuild. Those get compiled in and seed NVS on a device that has no
credentials of its own; anything saved through the setup page still wins.
`.env` is gitignored and the password is never logged or returned by the API.

### Putting the web app on the card

The browser runs the UI and the ESP32 just serves it, so the app has to be on
the card. Build it and push it over the API:

```bash
make web                             # wasm-pack -> opendmd-web/www/pkg
make card-sync HOST=opendmd.local    # PUTs every file to /www/ on the card
```

That uploads `index.html`, `app.css`, `app.js`, `js/` and `pkg/` (the JS
glue and the ~175 KB wasm module), each with its `.gz`, in a few seconds. `http://<ip>/` then serves the real app;
before that it serves the setup page instead. The setup page can also upload
one file at a time to any path, which is how a completely blank card gets
started.

```bash
make api-load HOST=opendmd.local     # stress the file server, diff the display metrics
```

### What the network costs the display

A classic ESP32 with 4 MB of flash and no PSRAM is tight with this feature
set. With the playlist index on the card (below), free heap while playing
bottoms out around 57 KB idle, 38 KB while serving one client back-to-back and
34 KB with three, and two minutes of continuous downloads leave the board up
(before the index moved it sat at 17-19 KB and aborted within two minutes).
What sustained serving still costs is display *timing*: the file server and
the player share one SPI bus and FAT's lock, and opening a file in a big
directory is a linear scan (~330 ms in the 3,781-file `gifs/Consoles`), so
frames run late while GIFs are being pulled out of it — 124 misses, worst
522 ms, in two minutes of back-to-back downloads. Played and served from the
bucketed `.odm` layout (above) the same load costs 2 misses of 3 ms, and an
idle player misses nothing. The player always gets priority on heap (requests are refused
with 503 and transfers pause when the heap falls into the player's reserve).
The arithmetic is in "Heap ceiling" in `CLAUDE.md`. Try it with
`make api-load HOST=… WORKERS=1`.

Both `/api/status` and the serial heartbeat report the numbers this is judged
on — frame-deadline misses, the worst lateness seen, and the sampled heap
floor:

### Serial output

```
I opendmd_firmware: OpenDMD firmware v0.2.0 (4mb, app desc '0.2.0 4mb') — 128x32 HUB75 GIF player
I opendmd_firmware: panel up: 128x32 (2 x 64x32), driver=0, double_buff=false, hub75_refresh_rate=100 Hz
I opendmd_firmware: microSD mounted at /sdcard (31.27 GB), heap 175600 B free
I opendmd_firmware: manifest indexed: /sdcard/filelist.txt, 10133 entries, index loaded in 9 ms (40532 B on card), heap 160664 B free
I opendmd_firmware::httpd: httpd started on port 80, pinned to CPU0, 10240 B stack
I opendmd_firmware::wifi: WiFi connected, IP 192.168.1.69
I opendmd_firmware::net_worker: [net] ready: http://192.168.1.69/ and http://opendmd.local/
I opendmd_firmware: [1] /gifs/Consoles/SMS_Fire_and_Forget_II_02_bd.azerty.p.gif (heap 66748 B free, largest block 65536 B)
I opendmd_firmware: heartbeat: heap 60076 B free (floor 58124 B, idf-min 54252 B), refresh 100 Hz, 5 GIFs started, 5 deadline misses (worst 357 ms), wifi sta 192.168.1.69
```

A missing card is not fatal: the mount is retried every 5 seconds, forever,
and so is an unreadable `filelist.txt`. Neither is a missing network: the
player plays regardless, and the HTTP server starts even if WiFi never
associates.

### The playlist is indexed, not loaded

A classic ESP32 has ~260 KB of usable DRAM, ~175 KB of it left once the HUB75
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

On the device even those 40 KB are too many to keep in RAM, so the firmware
writes the offsets to a file beside the manifest instead — `filelist.txt.idx`
(or `sets/<name>.txt.idx`) — and seeks into it: 4 bytes read per GIF started.
The file records the manifest's length and a fingerprint of the platform
filter it was built for; at boot a current one is simply reopened (~10 ms), and
a missing or stale one is rebuilt in one pass (~6 s over SPI) via a temp file
and a rename. `index_state` in `/api/status` is `building` while that runs. `POST /api/reindex`, a filter or set change and a manifest upload
all rebuild it in place, with no reboot. Anything that replaces a manifest
deletes its `.idx` first. If the card is read-only the firmware falls back to
the in-RAM index and keeps playing. `.idx` files are derived data: delete them
whenever you like.

**A set that does not exist is never saved.** `PUT /api/config` with a `set`
whose manifest does not open is answered `404 {"error":"set not found"}` and
changes nothing. And if the saved set has gone missing anyway (deleted with the
card in a reader, a different card), the firmware logs it at boot, plays the
default manifest and clears the set from NVS, rather than retrying a missing
file forever.

The simulator uses the same streaming index (the in-RAM `FileCatalog`), so
`make sim` exercises the same manifest path the firmware does.

## Deploying

A player for the cabinet is built with `make fw-release` — a release image in
`dist/opendmd-<version>-4mb.bin` with no bench WiFi credentials in it —
installed once over USB (`make fw-release-flash`, which also writes the
project's bootloader) and updated over the air after that (`make ota
HOST=opendmd.local`). An OTA image that resets before it has put its first
frame on the panel is rolled back to the previous one by the bootloader. Preparing a card from scratch,
first boot, what `/api/status` should read on a healthy install, and how to
roll back are in [`docs/deploying.md`](docs/deploying.md).

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
