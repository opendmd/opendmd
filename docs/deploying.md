# Deploying OpenDMD to the cabinet

This is the procedure for a player that lives in the cabinet rather than on
the bench: a card prepared on a laptop, a release firmware image with no
development credentials in it, and over-the-air updates afterwards.

The short version:

```bash
# once per card, on a laptop with the card in a reader (mounted at $CARD)
cp -r <DMDoS pack>/gifs <DMDoS pack>/filelist.txt "$CARD"/
make pack ARGS="convert $CARD --layout bucketed --verify"
mkdir -p "$CARD/sets"
make pack ARGS="manifest $CARD --layout bucketed --out sets/odm.txt"
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

### Pre-rendered `.odm` files, bucketed

The player needs ~14.5 KB of heap for an `.odm` against ~54 KB for a GIF,
and the bucketed layout keeps every file in a directory of ~15 entries so
the ESP32 never scans the 3,781-file `gifs/Consoles` (each file open there
costs ~330 ms and shows as a late frame). Both are measured in CLAUDE.md
("Heap ceiling", "Frame time"). Neither step touches `gifs/` or
`filelist.txt`.

```bash
# odm/<Category>/<bb>/<name>.odm for every GIF; --verify plays both and
# compares every frame. Re-running skips files already converted.
make pack ARGS="convert $CARD --layout bucketed --verify"

# The same playlist with .odm paths, written as a set the player can select.
# (Without --out it writes $CARD/filelist.odm.txt; the firmware only reads
# /filelist.txt and /sets/<name>.txt, so a set is the useful place for it.)
mkdir -p "$CARD/sets"
make pack ARGS="manifest $CARD --layout bucketed --out sets/odm.txt"
```

`make pack` is `opendmd-pack` built in release; the same arguments work on
the binary directly. The manifest step prints how many entries switched to
`.odm`; any that stay `.gif` (no `.odm` beside them) still play.

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
the image is over 95% of the 2,031,616-byte OTA slot. 0.2.0 is 1,575,552 B,
77.6%.

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
`OpenDMD firmware v0.2.0 (4mb, app desc '0.2.0 4mb')`, and on first boot
the player builds `filelist.txt.idx` on the card (~6 s) before it starts
playing.

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
4. To play the `.odm` set, select it as the active set (from the app, or
   `curl -X PUT -d '{"set":"odm"}' http://opendmd.local/api/config`). The
   first selection builds `sets/odm.txt.idx` (~6 s, the display holds its
   frame meanwhile).

Credentials live in NVS from then on and survive every OTA update.

### Updates (OTA)

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
the running firmware as the boot image.

## 3. What a healthy install looks like

`curl http://opendmd.local/api/status` — numbers measured on the rig
(CLAUDE.md, "Heap ceiling" and "Frame time"):

| field | playing `filelist.txt` (GIFs) | playing the bucketed `odm` set |
|---|---|---|
| `version` / `variant` | `0.2.0` / `4mb` | same |
| `heap.free` | ~57-60 KB | ~85-91 KB |
| `heap.floor` (sampled low-water) | ~55-58 KB idle; ~38 KB while serving one client, ~34 KB under three | ~85 KB idle |
| `refresh_hz` | 100 | 100 |
| `deadline_misses` | a handful per few minutes idle (19 in 5 min, worst ~520 ms: opening a file in a big directory) | 0 idle; 2 (worst 3 ms) in 2 min of back-to-back downloads of the same set |
| `worst_lateness_ms` | up to ~550 | 0 idle, single digits under load |
| `sd.mounted` | `true` | `true` |

A floor that keeps falling towards 5 KB (`config::HEAP_RESERVE`, where the
server starts answering 503) or misses by the hundred while nobody is using
the web app are the signs something is wrong. `uptime_secs` resetting on its
own means the board is rebooting: capture the serial log
(`make fw-capture PORT=... SECS=600`) before anything else.

## 4. Rolling back

**There is no automatic bootloader rollback on this board.** The 4 MB layout
follows stripe-dashboard's 4 MB build, which ships with
`CONFIG_BOOTLOADER_APP_ROLLBACK_ENABLE` off; ours never turned it on. What
protects an update instead:

- An image for the wrong flash size is refused before any flash is erased.
- An upload that is short, fails to write or fails `esp_ota_end`'s image
  validation is aborted, and the board keeps booting the running firmware.
- The previous firmware stays intact in the other OTA slot until the *next*
  update overwrites it.

So:

- **New firmware boots but misbehaves, and the network works:** post the
  previous image back — `make ota HOST=opendmd.local
  BIN=dist/opendmd-<previous>-4mb.bin`. Keep the last good image in `dist/`
  (it is gitignored, so copy it somewhere safe too).
- **New firmware does not boot, crash-loops, or never gets on the network:**
  USB. Take the board out, `make fw-release-flash PORT=...` with a known-good
  tree (or `espflash flash` the previous release ELF). NVS — WiFi and
  settings — survives a USB flash that does not erase the chip.

If the cabinet board is ever hard to reach, turning on bootloader rollback
(`CONFIG_BOOTLOADER_APP_ROLLBACK_ENABLE=y`, plus marking the app valid once
it has booted and joined the network) is the change to make; it only takes
effect after a USB flash of the new bootloader.
