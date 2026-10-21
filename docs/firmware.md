# Firmware

`firmware/` plays a card on the real hardware: an ESP32 DevKit V1 (classic
ESP32, 4 MB flash) on a DMDos carrier, a microSD over SPI, and two chained
64x32 HUB75 P4 panels driven by
[mrcodetastic/ESP32-HUB75-MatrixPanel-DMA](https://github.com/mrcodetastic/ESP32-HUB75-MatrixPanel-DMA)
3.0.14. Boot mounts the card, reads `/sdcard/filelist.txt`, and runs the same
`opendmd-core` engine the simulator does, one frame at a time, straight into
the panels' DMA buffer.

It is also on the network: WiFi, a plain-HTTP server on port 80 that serves
the web app (from flash) and the card's files, and a small JSON API. See
"Getting on WiFi" and "The web app in flash" below.

## Toolchain

```bash
cargo install --locked espup ldproxy espflash
espup install --targets esp32 --export-file ~/.espup-env.sh
uv tool install esptool
git submodule update --init          # the HUB75 driver
```

## Build and flash

```bash
make fw-build                        # first build clones ESP-IDF v5.3 (~5-10 min)
make fw-erase   PORT=/dev/ttyUSB0    # once, if the board ran other firmware
make fw-flash   PORT=/dev/ttyUSB0    # build + flash (bootloader, table, app, web app pack) at 115200 + monitor (console: 921600 baud)
make fw-monitor PORT=/dev/ttyUSB0
make fw-clean

make www-push HOST=opendmd.local     # a new web app over WiFi (PUT /api/www)
make api-load HOST=opendmd.local    # stress the file server for 120 s (PATHS=list.txt: fetch those paths)

make fw-build FW_FEATURES=probe      # with the timing probe (off, and compiled out, by default)
uv run tools/sdprobe-report.py serial.log --boot -1   # open latency, miss causes, blit times
```

ESP-IDF is installed once into `~/.espressif` and shared between projects.
Everything tunable — pin map, brightness, platform filter, dwell and retry
intervals — is a constant in `firmware/src/config.rs`, except the panel
timing, which is a setting (below). If the panels stay dark, the first thing
to try is the `fm6126a` driver chip.

## Panel timing

The HUB75 driver's five timing options are settings, so a fussy panel can
be tuned without a reflash: `PUT /api/config` `panel` (docs/api.md),
Settings → Advanced → Panel timing in the web app. The defaults are what
the firmware always ran.

| key | default | what it is |
|---|---|---|
| `clk_phase` | `true` | the clock edge the data is sampled on: true rising, false falling (the library's `clkphase`) |
| `latch_blanking` | 1 | clocks of OE blanking either side of LAT, 1..=4 |
| `driver` | `shiftreg` | the panel's shift-register chip: `shiftreg`, `fm6126a`, `fm6124`, `icn2038s`, `mbi5124`, `dp3246`; the FM and ICN chips get an init sequence at start-up, `mbi5124` and `dp3246` are always clocked on the rising edge |
| `i2s_speed` | 8 | the data clock, MHz: 8 (the slowest the library has), 16 or 20 |
| `min_refresh` | 60 | Hz; the library trades colour-depth timing (its LSB/MSB transition bit) for at least this refresh rate |

Colour depth is not a setting: the frame path's fast span writer is built
for the library's 8-bit planes and falls back to its per-pixel path
(~13.6 ms a frame) at any other depth.

The model is `opendmd_core::panel_timing` (host-tested), executed by the
main task:

- **Live: `clk_phase` and `latch_blanking`.** Neither touches the DMA
  buffers. The clock edge is only the CLK pin's inversion in the GPIO matrix
  (`hub75_set_clk_phase`); the blanking re-renders the OE bits at the
  current brightness (`hub75_set_latch_blanking`: `setLatBlanking`, then
  `setBrightness8`, which rewrites every column's OE bit — the buffer ends
  as a fresh `begin()` would leave it). Both run on the main task (CPU1)
  between frames. At the brightness ceiling (144 by default) the OE window
  is well inside the row, so blanking only matters near full brightness.
- **At boot: `driver`, `i2s_speed`, `min_refresh`.** They need the driver
  rebuilt: an init sequence bit-banged on the data pins, the I2S clock, and
  the DMA descriptor chain (one contiguous block whose size follows
  `min_refresh`). Rebuilding at runtime would free ~32 KB of row buffers and
  that block and allocate them again while the network tasks on CPU0
  allocate too; the panel is started first at boot for exactly this reason
  (a churned heap is what produced `!KABOOM! HUB75 memory allocation
  failed`). A failed reallocation would leave a dark panel until a restart
  anyway, so a restart is what applies them, on a whole heap.

**The trial.** A `panel` object is tried, not saved. Live values apply at
once and revert after 60 s (`panel_timing::TRIAL`) unless
`"panel_confirm":true` keeps them; nothing is written to NVS until then, so
a restart reverts too. Boot values are staged in NVS (`panel_try`) and
`reboot_required`/`panel_next` say so; at the next boot the main task reads
`panel` and `panel_try`, **removes `panel_try` before the panel starts**,
and runs the staged values on trial for this one boot: reverted by a
restart 60 s after the later of boot and the network coming up, unless
kept. Whatever the staged values do — a dark panel, a crash, a power cut —
the boot after runs the saved ones. `"panel_confirm":false` undoes a trial
at once (a boot trial by restarting); `"panel":null` saves the defaults.
WiFi does not depend on the panel, so the web app and the API stay
reachable throughout. The FM and ICN init sequences write registers in the
panel's chips that a restart does not clear (it resets the ESP32, not the
panel): after trying one, power the display off and on.

**The test** (`POST /api/panel/test`, `opendmd_core::panel_test`, the web
app's Test panel timing). The main task draws four full-screen 1 px
patterns that exposed a real panel's bad clock edge (magenta/green and
(83,72,108)/(172,183,147) checkerboards, white/black columns, a
`v`/`255 - v` grey checkerboard with `v = row × 8`), each under A then B
for 2 s, a 5 × 7 "A"/"B" label in the bottom-right corner, and applies A's
or B's live values through the same `set_live_timing` a trial uses. The
engine is not ticked (the file on screen is let go and the next one opens
afterwards; nothing reaches Recently played), and the look and glow are
off. It is refused during a trial or night mode's dark panel; it ends on
`{"stop":true}`, on any `PUT /api/config` (before it applies) and after
120 s, putting back the timing that ran before (the saved values), the
look, and a black canvas. It never writes NVS: the web app saves the pick
with `"panel_confirm":true`, no trial, since both values were just seen.

To recover from saved settings that leave the panel unusable:

```sh
curl -s -X PUT http://opendmd.local/api/config --json '{"panel":null}'
curl -s -X POST http://opendmd.local/api/reboot   # if panel_next is not null
```

## Getting on WiFi the first time

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
settings, a file uploader, a web app (pack) uploader and a reboot button —
whatever state the web app in flash is in.

For a bench board you can skip the dance: copy `firmware/.env.example` to
`firmware/.env`, fill in `OPENDMD_WIFI_SSID` and `OPENDMD_WIFI_PASS`, and
rebuild. Those get compiled in and seed NVS on a device that has no
credentials of its own; anything saved through the setup page still wins.
`.env` is gitignored and the password is never logged or returned by the API.

## The web app in flash

The browser runs the UI and the ESP32 just serves it — out of its own flash,
not the card. `make web` packs the built app into one file,
`opendmd-web/www.pack` (~0.8 MB, every file gzipped; the format is
[`docs/www-pack.md`](www-pack.md)); `make fw-flash` writes it into the
1,280 KB `www` partition along with the firmware, and the player maps it at
boot, checks its CRC and serves each file as a slice of flash. So the app
works with no card in the slot. To update just the app over WiFi:

```bash
make web
make www-push HOST=opendmd.local     # PUT /api/www, then check the version it reports
```

```bash
make api-load HOST=opendmd.local     # stress the file server, diff the display metrics
```

## What the network costs the display

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
bucketed copy layout ([`card.md`](card.md#prepared-copies)) the same load cost 2 misses of 3 ms (measured
with the retired `.odm` copies; with today's GIF copies: 82 misses, worst
497 ms, in the same two minutes of `/gifs/Consoles` downloads, against 140
on originals — the misses are the Consoles directory scans the downloads
make, not the copies), and an
idle player misses nothing. The player always gets priority on heap (requests are refused
with 503 and transfers pause when the heap falls into the player's reserve).
Downloads and uploads both run on one transfer task, one at a time, so a
slow or stalled client never holds up the API: an upload that sends nothing
for 5 s, or takes over 120 s, is abandoned (`408`) and its partial file
removed.
Try it with `make api-load HOST=… WORKERS=1`.

Both `/api/status` and the serial heartbeat report the numbers this is judged
on — frame-deadline misses, the worst lateness seen, and the sampled heap
floor:

## Serial output

```
I opendmd_firmware: OpenDMD firmware v0.3.0 (4mb, app desc '0.3.0 4mb') — 128x32 HUB75 GIF player
I opendmd_firmware: panel up: 128x32 (2 x 64x32), timing 1,1,shiftreg,8,60 (the defaults), double_buff=false, hub75_refresh_rate=100 Hz
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

## The playlist is indexed, not loaded

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
