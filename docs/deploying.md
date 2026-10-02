# Deploying OpenDMD to the cabinet

This is the procedure for a player that lives in the cabinet rather than on
the bench: a card prepared on a laptop, a release firmware image with no
development credentials in it, installed over USB from a browser (or
espflash), and firmware and web app updates over WiFi afterwards. The
firmware is updated over the LAN through the card: a signed bundle is
staged on it and a small updater program copies it into place
(`docs/ota.md`; section 2, "The flash" and "Updating the firmware").

The short version:

```bash
# once per card, on a laptop with the card in a reader (mounted at $CARD)
cp -r <DMDoS pack>/gifs <DMDoS pack>/filelist.txt "$CARD"/
make pack ARGS="convert $CARD --verify"   # prepare every GIF (the playback cache, under cache/)
sync && umount "$CARD"

# once per board (and to move a board to the v2 flash layout): over USB
make fw-release                            # dist/: app, updater, web app pack, bootloader, table, otadata, 4 MB image, manifest, signed bundle
make flasher                               # http://localhost:8765 in Chrome/Edge -> Install
#   or: make fw-release-flash PORT=/dev/ttyUSB0   (after `make fw-erase` if it ran anything else)
# join the OpenDMD-Setup WiFi, enter the network, then http://opendmd.local/

# a firmware update over WiFi (the board must already have the v2 layout)
make fw-release && make fw-ota HOST=opendmd.local

# a web app update on its own, over WiFi
make web && make www-push HOST=opendmd.local
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

If a card starts failing in service, the player notices (three card read
errors within a minute), restarts itself, and from then on runs **that
card** at 20 MHz instead of 40; the Settings page says "Card running at
reduced speed after errors". The slow clock is remembered by the card's
serial number, so it survives power cuts and only goes away when a
different card is inserted. After three such restarts within 15 minutes it
stops restarting, stays on the network without the card and tries it again
every minute ("Card errors: N, retrying"). Either message means: replace
the card (and scan the old one before trusting it again).

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

Users see GIFs and playlists; a **prepared copy** of a GIF
(`docs/cache-format.md`) is a cache: a standard GIF with its LZW capped at 9
bits and delta frames. The player looks for each GIF's copy at
`cache/<Category>/<xx>/<name>.gif` and plays it when it is there, otherwise
the GIF: playlists keep naming GIFs and nothing has to be configured. A copy
needs ~16 KB of heap (canvas included) against ~27 KB for the original
through the same decoder, is 0.61x its size on average, and lives in a
directory of ~15 entries, so the ESP32 never scans the 3,781-file
`gifs/Consoles` for it (each open there costs ~330 ms and shows as a late
frame). Both are measured in CLAUDE.md ("GIF heap budget", "Frame time").

```bash
# A copy of every GIF; --verify plays copy and original and compares every
# frame. Re-running skips files already prepared (the copy carries the GIF's
# mtime). gifs/ and filelist.txt are untouched.
make pack ARGS="convert $CARD --verify"
```

`make pack` is `opendmd-pack` built in release; the same arguments work on
the binary directly (`opendmd-pack convert --verify <card>`). That is the
whole step: no manifest to write. A GIF with no copy simply plays as a GIF.
Files added later from the web app (Library → **Add GIFs**, **Make art**) are
prepared by the browser as they are uploaded, the player prepares the rest
itself at night, and **Settings → Playback cache** shows how many of the
playlist's files are prepared and can prepare the rest in the background.

**Smooth motion (optional).** To switch it on for every GIF where it
helps (≥ 10% of the frame pairs move in steps of 2 px or more: ~1,000 of
the stock card's 11,047) before preparing, list them and convert — the
list is `/sets/.smooth`, which `convert` honours whenever it is on the
card, and which the web app's Library switch edits later:

```bash
make pack ARGS="smooth analyse $CARD --write"   # prints them, adds them to /sets/.smooth (68 s on 16 jobs)
make pack ARGS="convert $CARD --verify"         # their copies smooth, the rest plain
```

**A card prepared by an earlier build** has an `odm/` tree of the retired
pre-rendered `.odm` copies (and perhaps a `filelist.odm.txt`). The player
ignores both and logs each once at boot. Remove the tree on a laptop (`rm
-r "$CARD"/odm`) or over the network, a few seconds of work per request
until the reply says `"done":true`:

```bash
until curl -s -X DELETE "http://opendmd.local/api/files/odm?recursive=1" | grep -q '"done":true'; do :; done
curl -X DELETE http://opendmd.local/api/cache/failed   # the old failure list went with odm/; start clean
```

On the rig the stock card's tree (7,970 copies, 9,662 entries with their
directories) took 62 requests, 250 s, while the panel played (the deletes
hold the card, so expect late frames meanwhile).

The default playlist is `/filelist.txt` ("All files" in the app); `{"set":""}`
selects it, `{"set":"filelist.hall"}` a root manifest, `{"set":"<name>"}`
`/sets/<name>.txt`.

### Eject

```bash
sync && umount "$CARD"
```

Then pull the card. A plain unmount is enough; `udisksctl power-off` has
killed a USB reader on this machine before.

## 2. The firmware and the web app

### The flash

| partition | offset | size | what |
|---|---|---|---|
| (bootloader) | 0x1000 | | the project's second-stage bootloader |
| (partition table) | 0x8000 | | `firmware/partitions.csv` |
| `nvs` | 0x9000 | 24 KB | settings, WiFi, the schedule, Follow RePlayOS |
| `otadata` | 0xF000 | 8 KB | which app boots next; erased = `factory` |
| `phy_init` | 0x11000 | 4 KB | |
| `factory` | 0x20000 | 2,304 KB | the firmware |
| `ota_0` | 0x260000 | 384 KB | the updater (`ota/updater`), run only to install a firmware update |
| `www` | 0x2C0000 | 1,280 KB | the web app, one **www pack** (`docs/www-pack.md`) |

This is layout **v2** (`docs/ota.md`). v1 had one 2,752 KB app at 0x10000
and no `otadata`; moving a board from v1 takes one USB flash of a v2
release (the flasher, or `make fw-release-flash`), after which firmware
updates go over WiFi. The `www` partition did not move, so neither does the
web app in it.

The web app is not on the card. `make web` packs it (every file gzipped as
`make web` always did) into `opendmd-web/www.pack` — ~0.8 MB — and the
firmware maps that partition at boot, checks its CRC and serves the app
straight out of flash: the UI and the setup page work with no card in the
slot. A `www/` directory left on a card by an older firmware is ignored and
can be deleted.

### Build a release

```bash
make fw-release
```

This is a `cargo build --release` **without** the `dev-credentials` feature:
`firmware/.env` is not read, the two WiFi variables are scrubbed from the
build environment, and the code that would compile them in does not exist in
this build. It runs `make web` first, then writes to `dist/`:

| file | flashed at | |
|---|---|---|
| `opendmd-<version>-app.bin` | 0x20000 | the app image; its descriptor carries `"<version> 4mb"` (checked) |
| `updater.bin` | 0x260000 | the updater (`make updater`, `ota/updater`) |
| `ota_data_initial.bin` | 0xF000 | `otadata` erased (8 KB of 0xFF): the bootloader boots `factory` |
| `opendmd-<version>-www.pack` | 0x2C0000 | the web app |
| `bootloader.bin` | 0x1000 | |
| `partition-table.bin` | 0x8000 | |
| `opendmd-<version>-4mb-full.bin` | 0x0 | all six merged and padded to 4 MB: `esptool write-flash 0 <it>` or any "flash one image" tool |
| `manifest.json` | | ESP Web Tools: the six parts and their offsets |
| `opendmd-<version>.odfw` | | the signed bundle for an update over WiFi (`make fw-ota`; `make fw-sign`) |

The target fails if the app image leaves less than 64 KB of the 2,359,296 B
`factory` partition, if the updater leaves less than 32 KB of its 393,216 B
`ota_0`, and the packer refuses a web app over the 1,310,720 B `www`
partition. Signing needs the private key, `~/.config/opendmd/opendmd.key`
(`uv run tools/fw-sign.py keygen` made it, once; it is never in the repo;
`FW_SIGN_KEY=` points elsewhere); the device checks bundles against
`firmware/keys/opendmd.pub`. The release build needs ~1.5 GB of scratch;
`FW_RELEASE_TARGET_DIR=/roomier/dir make fw-release` puts it elsewhere.

### Installing from a browser (the flasher)

`flasher/index.html` is an install page built on
[ESP Web Tools](https://esphome.github.io/esp-web-tools/) (10.4.0, from
jsDelivr). It needs **Chrome or Edge** on a desktop (Web Serial; not
Firefox or Safari) and a secure context, which `localhost` is:

```bash
make fw-release    # once, for dist/
make flasher       # serves the page and dist/ on http://localhost:8765
```

Plug the DevKit in by USB, open <http://localhost:8765/>, click **Install**,
pick the serial port (a CH340, "USB Serial" / `ttyUSB0`), and the page asks
about **Erase device**, a box that starts unticked (the manifest sets
`new_install_prompt_erase`; `new_install_improv_wait_time` 0, since the
firmware does not speak Improv Serial):

- **Erase ticked — a first install, or a board coming from DMDoS or any
  other firmware.** The whole chip is erased, then the bootloader, the
  partition table, the app and the web app are written. *Everything* on the
  board goes, NVS included: WiFi credentials, brightness and its maximum, playlist, the
  schedule and night window, Follow RePlayOS. The board comes up as the
  `OpenDMD-Setup` access point (below). The card is not touched.
- **Erase unticked — a firmware update that keeps the settings.** ESP Web
  Tools writes only the six parts (the sectors they cover); NVS at 0x9000
  is not among them, so the WiFi, the schedule and every setting survive.
  This also moves a board from layout v1 (or the older dual-OTA one) to v2:
  NVS is at 0x9000 in every table, the new table replaces the old one, and
  `otadata` is written erased (`ota_data_initial.bin`), so whatever an older
  layout left at 0xF000-0x11000 cannot steer the bootloader: it boots
  `factory`.

The page cannot be tested end to end without a person at a real board (Web
Serial asks for a port by hand); `tools/web-smoke.py`-style checks cover
that it loads, the button renders and the manifest parses. When the repo is
public, GitHub Pages will host the same page with the release files beside
it (`flasher/index.html` plus a `firmware/` directory holding `dist/`'s
files), so installing needs no toolchain at all.

### Installing with espflash (USB)

```bash
make fw-erase PORT=/dev/ttyUSB0           # a first install, or from any other firmware or the old layout
make fw-release-flash PORT=/dev/ttyUSB0   # web app pack, updater, then bootloader, table, app (otadata erased); then monitor
```

Flash at 115200 (the default here): faster rates give `verify_failed` on
these CH340 boards. The firmware's console (its log) is a different rate,
**921600 baud**: `make fw-monitor` and `tools/serial-capture.py` use it; any
other serial monitor must be set to 921600 (the browser flasher's "Logs &
console" opens at 115200 and shows garbage). Without `fw-erase` this is the keep-settings path, like
the flasher's. On the bench, `make fw-flash` does the same with the bench
build (`dev-credentials`).

Put the card in and power up. The boot banner reads
`OpenDMD firmware v0.3.0 (4mb, app desc '0.3.0 4mb')`, then `www: pack
'<version>' verified, 37 entries, … B, … CRC pass N ms`, and on first boot
the player builds the default manifest's index on the card
(`filelist.txt.idx`; ~6 s for the full card) before it starts playing.

### Getting on the network

A release image has no WiFi credentials, so a fresh (or erased) board comes
up as an **open access point, `OpenDMD-Setup`**:

1. Join `OpenDMD-Setup` from a phone or laptop. The app opens as a "sign in
   to network" sheet; if it doesn't, browse to the address in the boot log
   (<http://192.168.71.1/>). `/setup`, the small embedded page, is always
   there too (and is what `/` shows in AP mode if the web app in flash is
   missing or damaged).
2. Enter the network name and password, **Save & reboot**. (The AP reboots
   the board after ten minutes regardless, so don't dawdle.)
3. The player joins the network as **<http://opendmd.local/>**.
4. Nothing to select for the prepared copies: the player finds them per
   file (section 1). A playlist is selected from the app, or with
   `curl -X PUT --json '{"set":"<name>"}' http://opendmd.local/api/config`; a
   name that does not exist on the card is refused with a 404 and nothing is
   saved. The first selection of a manifest builds its `.idx` (~6 s for the
   full card, the display holds its frame meanwhile).

Credentials live in NVS from then on and survive every update that does not
erase the chip.

### Updating the firmware

**Over WiFi** (a board already on layout v2, with a card in): `make
fw-release`, then `make fw-ota HOST=opendmd.local`, or Settings → Advanced →
Firmware → "Update from file…" with `dist/opendmd-<version>.odfw`. The
device receives the bundle onto the card (`/update/opendmd.bin`), checks
it (its header, that it fits, the ESP image magic, the SHA-256 and the
Ed25519 signature against the key compiled into the running firmware;
anything wrong is a 422 and nothing changes), restarts into the updater,
which copies it into `factory` and restarts into the new firmware: ~20-40 s
with the panel dark. `GET /api/firmware` then reports the new version;
`/update/log.txt` on the card has every step. If the updater cannot finish
(the card pulled mid-copy), it keeps retrying from the card every 10 s with
the LED blinking fast; put the card back. `docs/ota.md` has the details.
The web app is separate: push it too when the API changed
(`make www-push HOST=…`).

Over USB, keeping the settings: bump `version` in `firmware/Cargo.toml`,
`make fw-release`, then either the flasher with **Erase** unticked, or
`make fw-release-flash PORT=...` (no `fw-erase`). The web app pack is
written too, so firmware and app stay in step.

**Pause the cache builder first.** A USB flash resets the chip wherever it
is, and a reset in the middle of a card write can leave a card that times
out at the next mount. So check `curl http://opendmd.local/api/cache`: flash
only while `auto.state` is `paused`, `idle` or `done` — it is `paused`
(`reason "playing"`) whenever the panel is playing, with `auto_prepare` at
its default — never while it is `running` (night mode "off", or the player
paused). If it is running, resume playback (`POST /api/pause` toggles) or
wait for the night window to end, and check again.

### Updating the web app (WiFi)

```bash
make web
make www-push HOST=opendmd.local          # WWW_PACK=dist/opendmd-<v>-www.pack for a release's
```

`make www-push` (`tools/www-push.sh`) PUTs the pack to `/api/www`, then reads
`GET /api/www` back and checks the device serves a verified pack with the
version in the pack's header. No reboot. The device checks the header before
it erases anything (a file that is not a pack, or whose length disagrees with
its header, is a 400 and the app in flash is untouched); with a card in, it
receives the pack onto the card and checks its CRC there before touching
flash (a cut-short or damaged upload then leaves the old app working), then
writes the partition, header last, and verifies the CRC before serving it. While
it writes (~7 s for today's 782 KB; it is received like any upload, so the
same 120 s limit applies; the panel keeps playing, about one late frame per
push) the API answers as usual, a browser that already
has the app open keeps working, and a page load waits for the write and then
loads the new app. Without a card, a cut-short
or damaged upload leaves the API up and every page answering **503** with the
reason in plain text (`/api/status` `www.error` says it too) until a good
pack is pushed; the setup page (`/setup`) can also write one (**Web app** →
Write to flash).

The web app and the firmware share the JSON API, so after a firmware update
that changed it, push the web app from the same tree (the flasher and
`fw-release-flash` already do).

## 3. What a healthy install looks like

`curl http://opendmd.local/api/status` — numbers measured on the rig
(CLAUDE.md, "Heap ceiling" and "Frame time"):

| field | `filelist.txt`, few files prepared (mostly GIFs) | every file played from a prepared copy |
|---|---|---|
| `version` / `variant` | `0.3.0` / `4mb` | same |
| `manifest` | `/sdcard/filelist.txt` | `/sdcard/filelist.txt` (the copies are found per file) |
| `index_state` | `ready` | `ready` |
| `www_version` / `www` | the `git describe` of the `make web` that built the pack in flash; `www.ok` `true`, `www.error` `null` | same |
| `heap.free` | ~62-64 KB (originals through gifslim at cap 12; 0.3.0 with the `gif` crate: ~49-60 KB) | ~76 KB (copies through gifslim at cap 9; 75.9-76.2 KB over 90 s idle on a 645-copy set) |
| `heap.floor` (sampled low-water) | 24,980 B over the first 10 min after boot, which included deleting the old `odm/` tree (0.3.0: 12-18 KB; the older ~55-58 KB figures predate transitions and Follow) | 38,204 B over the first 2.5 min after boot (boot to idle on the copy set), 27,032 B after 120 s of `api-load WORKERS=3` |
| `refresh_hz` | 100 | 100 |
| `deadline_misses` | a handful per few minutes idle (19 in 5 min, worst ~520 ms: opening a file in a big directory) | 0 idle; 2 (worst 3 ms) in 2 min of back-to-back downloads of the same set |
| `worst_lateness_ms` | up to ~550 | 0 idle, single digits under load |
| `sd.mounted` | `true` | `true` |

The heap rows were measured on the rig on 2026-09-24 with no copies on the
card (5 min idle on `filelist.txt`, Dissolve, Follow RePlayOS in attract);
the other rows are older measurements (per-file copy resolution with 200 of
the 10,133 entries prepared as the retired `.odm` copies: 20 misses in 5
min, worst 443 ms; each GIF open first pays a ~5-6 ms miss looking for its
copy). The right column's misses were measured with a 200-entry set of
`.odm` copies.

A floor that keeps falling towards 5 KB (`config::HEAP_RESERVE`, where the
server starts answering 503) or misses by the hundred while nobody is using
the web app are the signs something is wrong. `uptime_secs` resetting on its
own means the board is rebooting: capture the serial log
(`make fw-capture PORT=... SECS=600`) before anything else.

## 4. When an update goes wrong

There is no bootloader rollback and no second app slot any more: the 4 MB
of flash hold one app and the web app. What to do instead:

- **The firmware does not boot, or misbehaves.** USB: flash the previous
  release (keep `dist/` somewhere safe; it is gitignored) with the flasher
  (Erase unticked keeps the settings) or `make fw-release-flash`. A board
  that is stuck in a boot loop is still reachable over USB: the ROM
  bootloader takes the flash whatever the app does.
- **The web app is broken or missing** (`/api/status` `www.ok` false, pages
  answer 503 with the reason): `make www-push HOST=...` a good pack, or
  write one from `/setup` in a browser. The API, the player and the setup
  page do not depend on it.
- **Nothing answers on the network**: USB, as above; `make fw-capture` for
  the serial log first if you want to know why.
