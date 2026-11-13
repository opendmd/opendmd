# Firmware updates over WiFi (OTA)

How an update over WiFi works. [`deploying.md`](deploying.md) has the procedure.

Where things are: the bundle format and its verifier
`opendmd-core/src/ota.rs`; the signer `tools/fw-sign.py`; the device side
`firmware/src/ota.rs` (`PUT`/`GET /api/firmware`, the boot-time tidy-up);
the updater `ota/updater/` (`make updater`); `make fw-sign`, `make fw-ota
HOST=`. The firmware checks the
signature as soon as the 140-byte header is in, on the network worker
(Ed25519 needs ~8 KB of stack; the stream task has 6 KB), before anything
is written to the card, and a bundle that fails it is answered 422 there
and then, without waiting for the rest of the body (the device reads off
and drops what is still in flight for up to 2 s, until the client stops,
so the close does not reset the connection under the reply; curl stops
sending as soon as an error status arrives); and the updater hashes the
whole file on the card once *before* it erases anything (a damaged file
then boots the old firmware, untouched), then again as it writes.

**Versions.** One version string, from the build: `make fw-build` /
`fw-release` write `FW_VERSION` (default `firmware/Cargo.toml`'s; `make
fw-build FW_VERSION=x.y.z` overrides it) into the app descriptor as
`"<version> <variant>"` (`4mb`: the flash layout, below), and the
release signs the bundle with the same `FW_VERSION` in its header. The firmware reports the descriptor's
version (`state::firmware_version`, `opendmd_core::ota::version_from_app_desc`)
as `version` in `/api/status` and `GET /api/firmware`, in the boot banner
and on the setup page, and compares it with the `done-<version>.bin` the
updater leaves after an update (a mismatch is a warning in the log).
`make fw-ota` waits for exactly that string.

**Older bundles.** A validly signed bundle is installed whatever its
version: going back is a feature (a release that misbehaves). A firmware
can be built with a **rollback floor** instead, off by default:
`OPENDMD_OTA_FLOOR=1.0.0` in the environment of the firmware build (read at
compile time; no Makefile variable sets it) refuses any bundle older than that version with a 422 at its header
("firmware X is older than 1.0.0, the oldest this display installs";
`opendmd_core::ota::check_floor`, the same order as `older()`). The web
app's Firmware card reads the chosen file's header and, when its version
is older than the running one (semver-ish: dot-separated numbers, a
pre-release before its release; `older()` in `js/firmware.js`), asks
"This is older than the firmware on the panel (X). Install it anyway?"
first. `make fw-ota` does not ask.

**Timing**, on a 64 GB card at 20 MHz: the 2.3 MB bundle uploads in ~10-16 s; the updater
takes ~19 s (SHA-256 on the card 2.9 s, the write 13.7 s, the image check
1.4 s); `GET /api/firmware` answers with the new version ~33 s after the
upload ends, playback ~45 s. A bundle signed with another key is refused
at its header: curl has the 422 after 1.06 s (its `Expect: 100-continue`
wait: the device never sends `100 Continue`; 0.07 s with `-H 'Expect:'`),
having sent 64 KB of the 2 MB; Chromium (the Firmware card's XHR) keeps
sending until the device closes, ~2 s later, and shows the reason after
~4.6 s.

## The flash layout

One firmware image per release, for one flash layout (the firmware's
`variant`, in `/api/status`, `GET /api/firmware` and the app descriptor):

| variant | board | flash | table |
|---|---|---|---|
| `4mb` | ESP32 DevKit V1 ("DevKit (4 MB)") | 4 MB | `firmware/partitions.csv` |

The layout is fixed by a USB install: an update over WiFi replaces the
firmware only, never the table or the bootloader. So a bundle carries its
layout, and every place that installs one checks it.

**A layout is a variant and the version of its partition table**
(`opendmd_core::ota::LAYOUTS`; the CSV's first line says `v4`, and
`opendmd-core`'s test `the_partition_tables_are_the_layouts` holds the two
together). The DevKit's table is **v4** (the releases after 1.0.1: a 2,816 KB
`factory` and a 768 KB `www`, below); 1.0.0-beta.16 to 1.0.1 ran **v3**,
1.0.0-beta.15 and earlier **v2**. `/api/status` and `GET /api/firmware`
report it as `layout` (firmware without the field is on v2), the boot
banner as `4mb layout v4`.

- **The bundle's magic** names the variant and the table (below). The
  signature covers it.
- **The firmware** refuses a bundle for another layout or another table at
  its header, before the signature and before anything reaches the card,
  with a 422 in words. Any other magic that starts `ODFW` (such as
  `ODFW18mb`, the 8mb layout that releases up to 1.0.0-beta.16 also
  carried): "this update is for another flash layout than this display's:
  use the DevKit (4 MB) update file (opendmd.dev/install has the latest)"
  (`OtaError::OtherLayout`); on a v4 board, a v3 (or v2) bundle: "this
  update is for the older flash layout v3 and this DevKit (4 MB) has layout
  v4: use a release made for layout v4 (opendmd.dev/install has the
  latest)"; and (for a later table, on a firmware that knows both) "this
  update needs flash layout v5 … : reinstall it once over USB from
  opendmd.dev/install".
  After the image, it checks the image's app descriptor is `"<version>
  <variant>"` for the bundle's layout (`check_app_desc`), so an image
  signed under the wrong layout by mistake is refused too.
- **The updater** knows the two magics and installs a bundle only if its
  layout is the board's: the variant from the flash size the installed
  bootloader set (4 MB is `4mb`), the table from the size of the board's
  `factory` partition (2,816 KB is 4mb v4, 2,560 KB 4mb v3, 2,304 KB 4mb
  v2), and the
  image's app descriptor must agree; otherwise it installs nothing and
  boots the firmware that is there.
- **Players from before a layout** (their firmware and their updater) do
  not know its magic, so they refuse its bundles as "not an OpenDMD
  firmware bundle (bad magic)" at the header, or (a firmware that knows
  the `ODFW` prefix) as another flash layout's: **a DevKit on table v3
  (1.0.0-beta.16 to 1.0.1) refuses every v4 bundle** (`ODFW44mb`: the
  firmware's `Header::parse` answers `OtherLayout` from 1.0.0, beta.16's
  `BadMagic`, "bad magic"; its updater's `memcmp`
  against the magics it knows fails, "bad magic", and its web app's
  `otherLayout` says "another flash layout"), so a v4 image, which may be
  bigger than v3's 2,560 KB `factory`, can never reach a v3 board's
  updater; and a DevKit on table v2 (1.0.0-beta.15 and earlier) refuses
  every v3 and v4 bundle as "bad magic". `make ota-compat` proves the v3
  case with 1.0.1's own code, `make emu-api`'s boot C with 1.0.0-beta.16's
  updater in a v4 board's `ota_0` (below).
- **The web app** reads the chosen file's magic and refuses another
  layout's or another table's in words (another layout: "This file is for
  another flash layout than this display's. Use the DevKit (4 MB) update
  file"; a newer table: "This firmware needs a one-time USB reinstall on
  this display: use the installer at opendmd.dev/install (Update OpenDMD
  keeps your WiFi and settings)"; an older one: "This file is for an older
  flash layout than this display has: get the latest from
  opendmd.dev/install"). Check for updates takes the release's file for
  the player's variant (`release.json` `variants`), and does not offer one
  whose `layout` is not the player's (it says the same words, after
  "Firmware X is out."); the downloaded file's magic is checked for it
  too (a v3 board's web app, 1.0.0 to 1.0.1, does the same with a v4
  release: "Firmware X is out. This firmware needs a one-time USB
  reinstall…", and its **Update from file…** calls a v4 file "for another
  flash layout than this display's"). A v2 board still running beta.15's
  web app shows that web app's words instead: "The firmware download is not the release it should be.
  Nothing was installed" (Check for updates), "This is not an OpenDMD
  firmware file (.odfw)" (Update from file…). `tools/fw-ota.sh` (`make
  fw-ota`) refuses a mismatch before sending; `tools/fw-sign.py sign`
  refuses to sign an image whose descriptor is for another variant, and
  signs for the variant's current table unless `--layout` says otherwise
  (test bundles).

The DevKit image also runs on a board with more flash (with the 4 MB
layout; IDF uses the size in the bootloader's header).

## If a board can't run its flash at 80 MHz

Builds run the flash at 80 MHz by default; the app sets the clock from its
own image header, the updater and bootloader stay at 40. A board whose
flash or wiring cannot do 80 MHz would boot-loop after an update (the
updater cannot notice, there is no rollback and no earlier image, and the
updater is no longer reachable), and only a USB install of a 40 MHz build
(`FW_FLASH_FREQ=40 make fw-release`) recovers it; BOOT only lets the
installer connect.

## Why not two app slots

The release image is ~2.5 MB and the web app pack has its own 768 KB
partition (`docs/www-pack.md`), so two full app slots do not fit in 4 MB.
The device cannot overwrite the partition it is running from either. So the
update is staged on the card and written by a small second program.

## Flash layout

4mb, table v4 (`firmware/partitions.csv`):

```
nvs       0x9000   0x6000
otadata   0xF000   0x2000      which app boots next
phy_init  0x11000  0x1000
factory   0x20000  0x2C0000    the firmware (2.75 MB)
ota_0     0x2E0000 0x60000     the updater (384 KB, C on ESP-IDF)
www       0x340000 0xC0000     the web app pack (768 KB)
```

v4 took another 256 KB from `www` (1,024 KB in v3; the pack is ~0.55 MB)
for `factory` (2,560 KB in v3): the firmware with the Formula compiler had
come down to ~67 KB to spare of v3's, against the 64 KB rule; v4 gives it
~320 KB. The size line in `make fw-release` insists on 64 KB to spare of
`factory`, and on 64 KB of `www` for the signed pack (`make web` checks the
same). Both app partitions start on a 64 KB boundary (the MMU page), `www`
too (it is memory-mapped), and the table ends at 0x400000. NVS, otadata
and phy_init did not move, so a USB install of v4 that does not erase the
chip keeps the settings (`docs/deploying.md` "Moving a board to table
v4").

v3 (1.0.0-beta.16 to 1.0.1), for the record:

```
factory   0x20000  0x280000    the firmware (2.5 MB)
ota_0     0x2A0000 0x60000     the updater
www       0x300000 0x100000    the web app pack (1 MB)
```

v2 (1.0.0-beta.15 and earlier):

```
factory   0x20000  0x240000    the firmware (2.25 MB)
ota_0     0x260000 0x60000     the updater
www       0x2C0000 0x140000    the web app pack (1.25 MB; v1's offset and size)
```

## The signed bundle

`opendmd-<version>-<variant>.odfw` = header + app image.

```
magic     the layout: "ODFW44mb" (4mb, table v4);      8 B
          "ODFW34mb" was 4mb's table v3, "ODFW1\0\0\0" its v2
version   32 B, NUL-padded (e.g. "1.0.0")
length    u32 LE, image bytes
sha256    32 B, of the image
sig       64 B, Ed25519 over (magic|version|length|sha256)
```

Header 140 B, then the image. The public key is compiled into the firmware
(`firmware/keys/opendmd.pub`, 32 B, committed); the private key never enters
the repo (`~/.config/opendmd/opendmd.key`, mode 0600, created by
`tools/fw-sign.py keygen`; the tool refuses a key file others can read).
`make fw-release` signs; a bundle that fails to verify is refused by the
device with a 422 and nothing on the card changes.

**The key trailer** (1.0.0-beta.15 on). `fw-sign.py sign` appends 16
bytes to the app image before it hashes and signs it:

```
magic     "ODFWKEY1"                                  8 B
key id    u8: 0 is the first release key (opendmd.pub)
reserved  7 zero bytes
```

They are part of the image as the header counts it (`length`, `sha256`),
so a firmware from before (beta.14 and earlier) verifies such a bundle
exactly as it did any: to it they are image bytes. The updater writes
them into the factory partition after the app image, where
`esp_image_verify` (which sizes the image from its own segment headers)
never reads. A firmware that knows the trailer checks the id against the
key that verified the header (`opendmd_core::ota::check_key_id`); a bundle
without one, from an earlier release, counts as key 0. The proof that old
firmware takes the new format: `opendmd_core::ota`'s test
`a_keyed_bundle_passes_the_beta14_checks` (the verification functions are
unchanged since beta.14), and `make ota-compat` ran the beta.14 tag's own
`opendmd-core` against a bundle and a www pack signed then. (It now runs
1.0.1's against table v4 and v3 bundles: below.)

## Keys

The firmware trusts the keys in `ota::KEYS` (id, public key); today only
key 0. **Rotating** later: make a new key, add it as id 1 to `KEYS` in a
release signed with key 0 (every player takes it), then sign later
releases with key 1 (`KEY_ID` in `tools/fw-sign.py`); retire key 0 from
`KEYS` once no player needs it. A leak of key 0 before that means a USB
reinstall for every player that never got the release with key 1.

**Where the key is.** The private key exists in two places: the
maintainer's machine (`~/.config/opendmd/opendmd.key`, read by
`tools/fw-sign.py`, pinned `pynacl==1.5.0`, never copied into a build
directory or the repo) and the repository's `OTA_SIGNING_KEY` Actions
secret. Releases are published by `.github/workflows/release.yml`, which
signs them with the secret (written to the runner's temporary directory
for the build and removed after); the maintainer's copy signs the check
builds `make release` makes before pushing a tag, and a release published
from that machine while CI is down (`PUBLISH=local`; `docs/deploying.md`
"Releasing"). Either way every bundle and pack is checked against the
committed `firmware/keys/opendmd.pub` before it is published. CI's builds
and tests otherwise use a throwaway key (`FW_SIGN_KEY`, `FW_SIGN_PUB`).

The web app pack is signed with the same key and key ids
(`docs/www-pack.md` "Signature").

## The flow

1. **Browser or curl**: `PUT /api/firmware` with the bundle (Settings →
   Device → Firmware: "Update from file…", or "Check for updates", below).
   The device streams
   the body to `/update/opendmd.bin` on the card, hashing as it goes (the
   night window's heap rules apply: one transfer at a time, 4 KB buffers).
   Once the header's layout and signature check out the panel shows a still
   message instead of what plays ("Updating", "to v<version>", "Don't
   unplug"); the file on screen is let go. Any failure takes the message
   down and playback resumes.
2. **Verify on the device**: header magic, the bundle's layout is the
   player's, image fits the factory partition, ESP image magic `0xE9` at
   offset 0, SHA-256 matches, Ed25519 signature valid, the image's app
   descriptor is `"<version> <variant>"` for that layout. Any failure: delete the file, 422 with a reason, nothing else
   changes. Success: write `/update/pending.txt` (version), set the boot
   partition to `ota_0` (`esp_ota_set_boot_partition`), reply
   `{"ok":true,"version":…,"rebooting":true}` (the panel says "Installing"
   over three dots, "Don't unplug"; "Installing", "Dark while it installs"
   if the updater in flash predates its picture), blank the panel, restart.
3. **Updater** (`ota/updater`, C): blanks the panel first (same early blank
   as `hub75_shim`), puts the same picture back up (see "The updater's
   panel" below), mounts the card on SPI3 (18/19/23/5), opens
   `/update/opendmd.bin`, checks the bundle's layout against the board's
   and the image's app descriptor, re-verifies the SHA-256 (the signature was
   checked by the firmware; the updater has no Ed25519 to keep it small),
   erases and writes the factory partition in 4 KB pieces, runs
   `esp_image_verify` on the result, sets the boot partition back to
   factory, renames the file to `/update/done-<version>.bin` (logged),
   writes `/update/log.txt`, restarts. The LED (GPIO 2) blinks while
   writing.
4. **Failure in the updater**: card missing or file unreadable → if the
   factory image verifies, boot it (nothing was written); else stay, blink
   fast, retry every 10 s (the card may be inserted). Write failed
   half-way → factory does not verify → the updater keeps trying from the
   file; if the file itself is bad, the device needs the USB flasher. The
   firmware never sets `ota_0` as boot without a verified file on the card,
   so this path needs a card fault mid-write.
5. **After boot**: the start-up show says "Updated to v<version>" for 3 s
   (the first boot of a version other than the one NVS says booted last,
   so after a USB install that keeps NVS too). The firmware sees
   `/update/done-*.bin`, logs the version
   change in the log ring, deletes the file, and the web app's Firmware card
   shows the new version. `GET /api/firmware` reports `{version, build,
   variant, layout, features, flash_size, flash{mhz,mode}, updater{version,panel}, pending, last, log}`: `log` is the end of `/update/log.txt`
   (≤ 1 KB of whole lines; the Firmware card's "Update log"), so the
   updater's account can be read without taking the card out.

## The updater's panel

The updater shows the firmware's "Installing / Don't unplug" picture while
it installs, so the panel does not look dead (`ota/updater/main/panel.cpp`):

- **The picture is made at build time.** `opendmd_core::updater_picture`
  packs `boot::installing` with no dot lit into
  `ota/updater/main/picture.h` (two bits a pixel, four colours, 1 KB), so
  the updater has no font code; `make updater-picture` writes it again and
  `make test` fails when the committed copy is stale. The firmware's own
  "Installing" screen is the same picture with its three dots lit, so the
  panel looks the same before and after the restart into the updater
  (when the updater has the picture: see "Which boards show it" below).
- **The driver is the firmware's**: mrcodetastic's
  ESP32-HUB75-MatrixPanel-I2S-DMA (the submodule under
  `firmware/components/`), single-buffered, with the firmware's pin map and
  the panel timing it saved (NVS `opendmd` / `panel`, read only; anything
  missing or damaged is the defaults, as the firmware's boot takes them;
  the parser is checked against the same vectors as `PanelTiming::from_nvs`,
  `ota/updater/test/`). The brightness is the firmware's start-up level,
  96, never above the saved `brightness` or `max_brightness`.
- **The DMA keeps it lit.** The picture is drawn into the driver's buffer
  once; the I2S DMA then loops over it through a closed ring of
  descriptors with no interrupt and no CPU work. Its buffers and
  descriptors are in internal DMA-capable RAM, which a flash erase or write
  (the cache off, the other CPU parked) does not touch, so the refresh goes
  on through them. A task on CPU1 lights one of the three dots at a time
  (0.4 s each; none while the updater waits to retry); it never runs during
  a flash operation and the copy never waits on it.
- **It cannot stop an install.** The picture goes up once, before the
  first attempt, and only after a restart or a power-on:
  after a panic, a watchdog or a brownout reset the updater stays dark, so
  a panel that crashed it, or drew too much from the supply, leaves the
  next boot exactly as an updater without a panel. A driver that does not
  start (no memory, a refused config) puts OE back high and the install
  goes on dark. The panel is blanked (OE high, the DMA stopped) before the
  restart into the new firmware.
- **The log** says `panel up in N ms: <timing> (...), brightness B, R Hz`
  or `panel skipped: <why>; it stays dark`, and after the write pass a
  `panel refresh:` line: a pulse counter counts the D row-address line's
  rising edges at its pad (one a refresh, in hardware, whatever the CPUs
  are doing), so it compares the refresh rate while the image is written
  with the rate while it was only being checked, and says how many
  milliseconds of refreshes the write pass, and its longest flash
  operation, lacked. A board with no panel counts the same.

**Which boards show it.** The updater in `ota_0` is written only by a USB
install (the installer's manifest writes `updater.bin`; an update over WiFi
replaces the firmware, never the updater). So the lit install needs the
updater from a USB install of 1.0.0 or later; a board updated only over
WiFi since an older USB install stays dark during the install until its
next USB install. The firmware tells them apart before it says anything:
it reads the `ota_0` image's app descriptor once
(`esp_ota_get_partition_description`, a flash read; `ota::updater_version`)
and takes `"updater <n>"` with `n` ≥ 2 as one that lights the panel
(`opendmd_core::boot::updater_lights_panel`; the updater's version is
`CONFIG_APP_PROJECT_VER` in `ota/updater/sdkconfig.defaults`, and a core
test checks the one this tree builds passes). An older or unreadable one
gets the old screen, "Installing / Dark while it installs / Don't unplug".
`GET /api/firmware` reports it as `updater: {version, panel}`. The
emulator's API lane runs both: this tree's updater installs (boot B), and
`OLD_UPDATER_TAG`'s, v1.0.0-beta.16 by default (boot C, `make
emu-old-updater`), gets the old screen and then refuses the bundle, since
it knows no table after v3 (on a real board the USB move to v4 always
writes the v4 updater with it). A USB install itself does not run the updater: the panel
stays dark while the installer writes.

## Check for updates

Settings → Device → Firmware → **Check for updates** (`js/update.js`)
updates the firmware and the web app from the published release, with the
browser doing the fetching: the player itself never goes to the internet.

- **The release.** The project site publishes
  `https://opendmd.dev/install/release.json`:
  `{"version": "1.0.0", "firmware": "firmware/opendmd-1.0.0-4mb.odfw",
  "variants": {"4mb": {"firmware": "firmware/opendmd-1.0.0-4mb.odfw", "layout": 3}}, "www":
  "firmware/opendmd-1.0.0-www.pack", "www_version": "v1.0.0"}`, the paths
  relative to the JSON's own address, `www_version` the pack header's
  version (what `GET /api/www` reports for it), `layout` the partition
  table each bundle is for (`tools/site.py` reads it from the bundle's
  magic). The page takes `variants.<the player's variant>.firmware`, and
  only when its `layout` is the player's (another one: the USB reinstall
  message above, nothing offered); `firmware` is the 4mb bundle too, for
  the web apps from before `variants`. The page is plain HTTP
  from the player and fetches this over HTTPS; the site sends
  `Access-Control-Allow-Origin: *`, and the request carries no credentials
  or custom headers, so there is no preflight. The player's
  Content-Security-Policy allows `connect-src *`. For testing, and in a
  **development build of the web app only** (its version, as `GET /api/www`
  reports it, is a `git describe` past a tag or `-dirty`), `?updates=<url>`
  in the page's address, or localStorage `odmd.updates` (a JSON string),
  names another `release.json`, and the "is out" line names that feed's
  host. A release's web app reads opendmd.dev's alone: no link can point
  its updates elsewhere.
- **Betas.** The site also publishes `install/beta.json`, the same shape:
  the newest release of all, beta or final, where `release.json` names
  the newest final release (before the first final, both name the newest
  beta). A beta newer than the final release has its files under
  `install/firmware/beta/`, which `beta.json`'s paths name. With the
  player's `beta_updates` setting on (Settings → Device → Firmware → **Get
  beta updates**, off by default; `GET/PUT /api/config`, kept in NVS and
  the card's `/opendmd.json`), the page reads `beta.json` instead, asked
  the same way (`?v=&layout=`). Turning it off installs nothing: a player
  on a beta is up to date until a final release newer than it comes out
  (`older()`: 1.0.0-beta.16 < 1.0.0 < 1.0.1-beta.1 < 1.0.1).
- **What counts as newer.** The firmware: the release's `version` against
  `GET /api/firmware`'s, with `older()` (1.0.0 is newer than 0.3.13). The web
  app: `www_version` against `GET /api/www`'s `version`, `git describe`
  strings: a release reads `v1.0.0`, a development build
  `v1.0.0-3-gabc1234-dirty` (or a bare commit hash). Any difference is an
  update, except when the player's firmware is newer than the release, or
  the web app is a development build of the release or later (installing
  the release would be a step back). Versions are shown without the `v`,
  and a development build as "1.0.0, development build". Both current:
  "Up to date (1.0.0)".
- **Update.** Everything needed is downloaded into memory first, with
  progress, and checked: the firmware's header must carry the release's
  version and the player's layout, the pack must start `ODWP` with its header's length equal to
  the download, its version `www_version`, and at most 786,432 B (the
  player then checks its signature). A
  failure there installs nothing. Then the firmware, if it is newer:
  `PUT /api/firmware` and the wait for the restart, exactly as "Update from
  file…" does (`/api/status` polled every 2 s until it reports the new
  version, up to 3 minutes). Then the web app, if it differs: `PUT
  /api/www` with the pack (`Content-Type: application/octet-stream`, the
  length set by the browser, as `make www-push` sends it), a `503` retried
  up to four times 3 s apart, the reply checked for `ok` and the new
  version. Then the page reloads itself, since its code is the old web
  app's. Requests go one at a time and carry the API key as every other
  request does (`Authorization: Bearer`); the page is the player's own
  origin, so the `Origin` check passes.
- **Failures** are said in words and never reported as an update: offline,
  the site unreachable (a missing CORS header looks the same to the page),
  a `404` for the JSON or a file, a JSON that does not parse, a download
  cut short, a firmware the player refuses (`422`, with its reason; the web
  app is then not sent), and each `PUT /api/www` reply of
  [`www-pack.md`](www-pack.md) (`400`, `408`, `413`, `422`, `500`), after
  which the page reads `GET /api/www` and says whether the old web app is
  still in place. If the firmware went in and the web app did not, the
  message says both.
- In the web demo the Firmware card says the demo can't install firmware,
  so there is nothing to check.

## Related protections

- `PUT /api/www` takes only a pack signed with the release key, and keeps
  serving the old pack until the new one has verified on the card (staged,
  then read back whole, size, CRC and signature, every card read retried):
  only then is the partition erased and written from the card, header
  last (`docs/www-pack.md`). Without a usable card it is refused. If the copy still fails after
  the erase, the checked pack stays on the card and the next boot writes
  it.
- Every request must name the panel in its `Host` (421 otherwise: DNS
  rebinding); mutating `/api` requests reject a foreign `Origin` header
  and require `Content-Type: application/json` where a JSON body is
  expected (`docs/api.md` "Security").
- Optional API key (`api_key` setting; `Authorization: Bearer`).

## Testing

- Host: `tools/fw-sign.py selftest` (in `make test`) round-trips (sign,
  verify, tamper → fail, a bundle relabelled to the other table → fail, one
  relabelled `ODFW18mb` → refused as another layout, an 8mb image in a 4mb
  bundle → fail); the firmware's verifier is in `opendmd-core::ota` with
  tests on the header, a known-answer signature for each table (4mb v4,
  v3, v2), an old 8mb bundle refused as another layout
  (`a_bundle_for_another_layout_is_refused`), the layout check, the
  partition CSV against `LAYOUTS` and the app descriptor check.
- Across layouts: `make ota-compat` (`tools/ota-compat.sh`, OTA_COMPAT_OK)
  checks out v1.0.1 (the last DevKit release on table v3; `OTA_COMPAT_TAG`)
  and runs `tools/fixtures/ota-compat.rs`, a 4mb player's checks in the
  firmware's order, with that release's own `opendmd-core` and with this
  tree's, against bundles signed now: 1.0.1 must refuse a table v4 bundle
  whose image (2,700,000 B) is bigger than its 2,621,440 B `factory`, at the
  header ("another flash layout"), and take a table v3 one and the www
  pack; this tree must take the v4 bundle and refuse the v3 one at the
  header ("the older flash layout v3"). With `DIST=` holding a release
  build, its 4mb bundle (the release key) goes through both too: refused by
  1.0.1, taken here. The emulator's API lane (`make emu-api`) sends a
  v3-labelled copy of the real bundle to the v4 firmware (422 at the
  header, the words above) before the real update, and in boot C puts
  1.0.0-beta.16's updater (tables v3 and v2 only) in `ota_0`: it refuses
  the v4 bundle as "bad magic", writes nothing and boots the firmware,
  which says the update "was not installed".
- Browser: `make web-smoke` installs a bundle through "Update from
  file…" and runs "Check for updates" against a locally served
  `release.json` and files, through `tools/mock-device.py` (its simulated
  restart, and `PUT /api/www` taking on the pack's version): up to date,
  each failure, and an update applied and the page reloaded.
- Device: a spare DevKit with any FAT32 card (no panel needed): flash
  table v4 once over USB, then `make fw-ota HOST=…` twice (a version bump
  between), then the fault drills: a tampered bundle (422), a cut-off upload
  (408, nothing staged), card pulled during the updater's write (recovers on
  reinsertion). The serial capture watches the updater's log.
- Then the real player: one USB flash for the layout, then every
  further firmware update over WiFi.
