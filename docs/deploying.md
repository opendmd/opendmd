# Deploying OpenDMD to the cabinet

This is the procedure for a player that lives in the cabinet rather than on
the bench: a card prepared on a laptop, a release firmware image with no
development credentials in it, and over-the-air updates afterwards.

The short version:

```bash
# once per card, on a laptop with the card in a reader (mounted at $CARD)
cp -r <DMDoS pack>/gifs <DMDoS pack>/filelist.txt "$CARD"/
make pack ARGS="convert $CARD --layout bucketed --verify"   # prepare every GIF (the playback cache)
make web && cp -r opendmd-web/www "$CARD"/www
sync && umount "$CARD"

# once per board, over USB
make fw-release-flash PORT=/dev/ttyUSB0    # after `make fw-erase` if it ran DMDoS
# join the OpenDMD-Setup WiFi, enter the network, then http://opendmd.local/

# every update thereafter
make fw-release && make ota HOST=opendmd.local
```

## 1. The card

### Filesystem: FAT32, as shipped — do not reformat

The firmware mounts the card with ESP-IDF's FATFS over SPI, which reads FAT32
(FAT12/16 too) but **not exFAT**. A 32 GB (SDHC) card comes from the factory
as FAT32, and that factory layout is aligned to the card controller's erase
blocks; a generic `mkfs.vfat` with default options can misalign it and make
the card slower and wear it harder. So use a card of 32 GB or less and leave
its filesystem alone. (Cards of 64 GB and up are SDXC and ship as exFAT, which
the player cannot read; don't use one.)

The rig's card is a 32 GB card that was full-surface scanned with zero errors
and no slow regions. A card that stalls on a bad sector hangs the player —
that is how the original 128 GB DMDoS card failed — so a new card is worth a
contiguous read scan before it goes into the cabinet.

### The DMDoS pack

Copy the pack's `gifs/` tree and its `filelist.txt` to the card root, exactly
as they ship. This keeps the card working with stock DMDoS as well, and
`filelist.txt` is **curated**: it is not "every GIF on the card" (the
Halloween and XMAS folders are deliberately left to `filelist.hall` /
`filelist.xmas`), so never regenerate it with a directory scan. Copy the two
seasonal lists too if you want them.

```bash
CARD=/media/$USER/<card>            # wherever the reader mounted it
cp -r /path/to/pack/gifs /path/to/pack/filelist.txt "$CARD"/
```

### Prepare the card (the playback cache)

Users see GIFs and playlists; a **prepared copy** of a GIF (an `.odm` file,
`docs/odm-format.md`) is a cache. The player looks for each GIF's copy at
`odm/<Category>/<bb>/<name>.odm` and plays it when it is there, otherwise the
GIF: playlists keep naming GIFs and nothing has to be configured. A copy
needs ~14.5 KB of heap against ~54 KB for a GIF, and lives in a directory of
~15 entries, so the ESP32 never scans the 3,781-file `gifs/Consoles` for it
(each open there costs ~330 ms and shows as a late frame). Both are measured
in CLAUDE.md ("Heap ceiling", "Frame time").

```bash
# A copy of every GIF; --verify plays both and compares every frame.
# Re-running skips files already prepared. gifs/ and filelist.txt are untouched.
make pack ARGS="convert $CARD --layout bucketed --verify"
```

`make pack` is `opendmd-pack` built in release; the same arguments work on
the binary directly. That is the whole step: no manifest to write. (`opendmd-pack
manifest` still exists for tooling and tests; the firmware ignores a
`filelist.odm.txt` and logs so once at boot.) A GIF with no copy simply
plays as a GIF. Files added later from the web app (Library, **Add GIFs**)
are prepared by the browser as they are uploaded, and **Settings → Playback
cache** shows how many of the playlist's files are prepared and can prepare
the rest in the background.

The default playlist is `/filelist.txt` ("All files" in the app); `{"set":""}`
selects it, `{"set":"filelist.hall"}` a root manifest, `{"set":"<name>"}`
`/sets/<name>.txt`.

### The web app

```bash
make web                                # wasm-pack -> opendmd-web/www/pkg
cp -r opendmd-web/www "$CARD"/www       # everything, including pkg/
```

Copy the whole directory, including any `*.gz` siblings the build leaves next
to its files: under `/www/` the server looks for `<file>.gz` first and sends
it with `Content-Encoding: gzip`, which is most of what makes the page load
quickly over the ESP32's small TCP windows. Without the app on the card the
player serves its embedded setup page at `/` instead, which still works (and
can upload the app one file at a time), so this step can also be done later
with `make card-sync HOST=opendmd.local`.

### Eject

```bash
sync && umount "$CARD"
```

Then pull the card. A plain unmount is enough; `udisksctl power-off` has
killed a USB reader on this machine before.

## 2. The firmware

### Build a release image

```bash
make fw-release
```

This is a `cargo build --release` **without** the `dev-credentials` feature:
`firmware/.env` is not read, the two WiFi variables are scrubbed from the
build environment, and the code that would compile them in does not exist in
this build. The image is saved to `dist/opendmd-<version>-4mb.bin` (version
from `firmware/Cargo.toml`), its app descriptor carries `"<version> 4mb"` —
the string the OTA handler's variant check reads — and the target fails if
the image is over 95% of the 2,031,616-byte OTA slot. 0.3.0 is 1,910,032 B,
94.0% (20 KB under the limit; 0.2.0 was 1,883,264 B, 92.7%).

The release build needs ~1.5 GB of scratch; `FW_RELEASE_TARGET_DIR=/roomier/dir
make fw-release` puts it elsewhere.

### First install on a fresh board (USB)

```bash
make fw-erase PORT=/dev/ttyUSB0           # only if it ran DMDoS or anything else
make fw-release-flash PORT=/dev/ttyUSB0   # bootloader, partition table, app; then monitor
```

Flash at 115200 (the default here): faster rates give `verify_failed` on
these CH340 boards. `fw-release-flash` also erases `otadata`, so a board that
had been OTA'd into its second slot boots the image just flashed.

Put the card in and power up. The boot banner reads
`OpenDMD firmware v0.3.0 (4mb, app desc '0.3.0 4mb')`, and on first boot
the player builds the default manifest's index on the card
(`filelist.txt.idx`; ~6 s for the full card) before it starts playing. Once the first frame is up the
log says `rollback mark: ota_0 was ..., now marked valid after the first
frame`.

`fw-release-flash` writes **the project's own bootloader** (`--bootloader`),
not espflash's bundled one: only ours has OTA rollback compiled in (section
4). A board that was last USB-flashed with a plain `espflash flash` — or
before this change — has the bundled bootloader and no rollback until it is
USB-flashed once with `make fw-release-flash` (or `make fw-flash` on the
bench).

### Getting on the network

A release image has no WiFi credentials, so a fresh board comes up as an
**open access point, `OpenDMD-Setup`**:

1. Join `OpenDMD-Setup` from a phone or laptop. The setup page opens as a
   "sign in to network" sheet; if it doesn't, browse to the address in the
   boot log (<http://192.168.71.1/>).
2. Enter the network name and password, **Save & reboot**. (The AP reboots
   the board after ten minutes regardless, so don't dawdle.)
3. The player joins the network as **<http://opendmd.local/>**. The app is
   there if `www/` is on the card; `/setup` is always there for recovery.
4. Nothing to select for the prepared copies: the player finds them per
   file (section 1). A playlist is selected from the app, or with
   `curl -X PUT -d '{"set":"<name>"}' http://opendmd.local/api/config`; a
   name that does not exist on the card is refused with a 404 and nothing is
   saved. The first selection of a manifest builds its `.idx` (~6 s for the
   full card, the display holds its frame meanwhile).

Credentials live in NVS from then on and survive every OTA update.

### Updating

```bash
# bump `version` in firmware/Cargo.toml first
make fw-release
make ota HOST=opendmd.local                 # BIN=dist/... for a specific image
OTA_DRY_RUN=1 make ota HOST=opendmd.local   # print the commands only
```

`make ota` POSTs the image to `/api/ota`, prints the reply
(`{"ok":true,"rebooting":true}`), then polls `/api/status` until the board
has rebooted and reports the version the image carries (`OTA_TIMEOUT`, default
120 s). The upload is rejected before anything is erased if the image was
built for another flash size, and one that fails validation at the end leaves
the running firmware as the boot image. The whole image must arrive within
120 s, or the update is abandoned (`esp_ota_abort`, one warn line: `POST
/api/ota: upload aborted at <bytes> of <total> B …`) and the board keeps
running what it had; over WiFi it takes ~10 s. The cache builder is stopped
cleanly before the restart (its file finished or abandoned), so an OTA may be
started at any time, the night window included.

This is what the first update of a deployed build looked like (0.3.0 over a
0.3.0 bench build in `ota_0`, 2026-09-24, a playlist playing):

```
$ make ota HOST=192.168.1.69
image: dist/opendmd-0.3.0-4mb.bin (1910032 B), app descriptor '0.3.0 4mb', expecting version '0.3.0'
uploading to http://192.168.1.69/api/ota ...
{"ok":true,"rebooting":true}
back after 23s: version 0.3.0, uptime 10s
```

and on the serial console:

```
ota: OTA: receiving 1910032 B
ota: OTA: variant check passed (4mb)
ota: OTA: 1910032 B written, rebooting                     (9.4 s after the first line)
rst:0xc (SW_CPU_RESET)
boot: Loaded app from partition at offset 0x210000         (ota_1)
OpenDMD firmware v0.3.0 (4mb, app desc '0.3.0 4mb')
reset reason: software
microSD mounted at /sdcard (31.27 GB) on attempt 1
manifest indexed: /sdcard/filelist.txt, 10133 entries, index loaded in 8 ms
rollback mark: ota_1 was pending-verify, now marked valid after the first frame
[1] /gifs/Arcade/ARCADE_NEOGEO_Metal_Slug_4_01_INVERT.gif <- prepared
wifi: scanning for '<your network>'                        (from NVS: a release image has no credentials)
WiFi connected, IP 192.168.1.69                            (11 s after the reset)
[net] ready: http://192.168.1.69/ and http://opendmd.local/
time: NTP synced
heartbeat: … 0 deadline misses …, builder: paused playing
```

What to check afterwards: `version` in `/api/status` is the new one,
`reset_reason` is `software` (not `panic` or `task watchdog`), `rollback
mark: … now marked valid` is in the log if you have one, and the panel is
playing. If the board comes back on the old version, the bootloader rolled
the new one back (section 4).

**Over USB, pause the cache builder first.** An OTA and `POST /api/reboot`
stop the builder before they restart; a USB flash (`make fw-flash`,
`fw-release-flash`, the bench scripts) resets the chip wherever it is, and a
reset in the middle of a card write can leave a card that times out at the
next mount. So check `curl http://opendmd.local/api/cache`: flash only while
`auto.state` is `paused`, `idle` or `done` — it is `paused` (`reason
"playing"`) whenever the panel is playing, with `auto_prepare` at its
default — never while it is `running` (night mode "off", or the player
paused). If it is running, resume playback (`POST /api/pause` toggles) or
wait for the night window to end, and check again.

## 3. What a healthy install looks like

`curl http://opendmd.local/api/status` — numbers measured on the rig
(CLAUDE.md, "Heap ceiling" and "Frame time"):

| field | `filelist.txt`, few files prepared (mostly GIFs) | every file played from a prepared copy |
|---|---|---|
| `version` / `variant` | `0.3.0` / `4mb` | same |
| `manifest` | `/sdcard/filelist.txt` | `/sdcard/filelist.txt` (the copies are found per file) |
| `index_state` | `ready` | `ready` |
| `www_version` | the `git describe` of the `make web` that built the card's app, `null` if the card's `www/` predates it | same |
| `heap.free` | ~57-60 KB | ~85-91 KB |
| `heap.floor` (sampled low-water) | ~55-58 KB idle; ~38 KB while serving one client, ~34 KB under three | ~85 KB idle |
| `refresh_hz` | 100 | 100 |
| `deadline_misses` | a handful per few minutes idle (19 in 5 min, worst ~520 ms: opening a file in a big directory) | 0 idle; 2 (worst 3 ms) in 2 min of back-to-back downloads of the same set |
| `worst_lateness_ms` | up to ~550 | 0 idle, single digits under load |
| `sd.mounted` | `true` | `true` |

The left column was re-measured with per-file copy resolution (200 of the
10,133 entries prepared, 5 min idle): free 56.8-57.6 KB, floor 53.3 KB, 20
misses (worst 443 ms); each GIF open first pays a ~5-6 ms miss looking for
its copy, and a file that has one plays with ~87-94 KB free. The right
column was measured with a 200-entry set of copies.

A floor that keeps falling towards 5 KB (`config::HEAP_RESERVE`, where the
server starts answering 503) or misses by the hundred while nobody is using
the web app are the signs something is wrong. `uptime_secs` resetting on its
own means the board is rebooting: capture the serial log
(`make fw-capture PORT=... SECS=600`) before anything else.

## 4. Rolling back

**The bootloader reverts an image that does not boot.**
`CONFIG_BOOTLOADER_APP_ROLLBACK_ENABLE=y` is in `firmware/sdkconfig.defaults`
(with the project's bootloader on the board — see "First install" above).
The partition table already has what it needs: `otadata` at 0xf000 and two
1.94 MB slots, `ota_0` and `ota_1`. Rollback costs no app-slot space; the
code is in the bootloader (26,832 B, under the 28,672 B that fit below the
partition table at 0x8000).

What happens on an update:

1. `POST /api/ota` writes the other slot and marks it `new` in otadata.
   Before anything is erased, an image for the wrong flash size is refused;
   an upload that is short, fails to write or fails `esp_ota_end`'s image
   validation is aborted, and the board keeps booting the running firmware.
2. The bootloader boots the new slot as `pending-verify`.
3. The new firmware brings the panel up, mounts the card, opens a playlist
   and blits its **first frame**, and only then calls
   `esp_ota_mark_app_valid_cancel_rollback()` (`rollback mark:` in the log).
   From then on the image is `valid` and stays.
4. If it resets **before** that — a panic, a watchdog, a crash loop, a hang
   the watchdog catches — the bootloader finds it still `pending-verify`,
   marks it invalid and boots the previous slot, which is untouched. The
   board comes back on the old version by itself; `make ota`'s poll then
   fails with "board rebooted but reports version '<old>'".

What rollback does not catch, and what to do instead:

- **It boots, plays and misbehaves afterwards** (network broken, UI broken,
  slow): the image was already marked valid. If the network works, post the
  previous image back — `make ota HOST=opendmd.local
  BIN=dist/opendmd-<previous>-4mb.bin`. Keep the last good image in `dist/`
  (it is gitignored, so copy it somewhere safe too).
- **It hangs before the first frame without resetting** (no watchdog fires):
  nothing reverts it until the board is power-cycled — which then does roll
  back, since it never got marked. So a power cycle is the first thing to
  try.
- **It never gets a first frame for an ordinary reason** — no card, a card
  with no manifest — and is then reset (a power cycle, the setup AP's
  10-minute reboot): it is rolled back too, because it never proved itself.
  Harmless (the old image is fine) but surprising; put the card back and
  update again.
- **Neither slot boots, or the bootloader is espflash's**: USB. Take the
  board out, `make fw-release-flash PORT=...` with a known-good tree. NVS —
  WiFi and settings — survives a USB flash that does not erase the chip.
