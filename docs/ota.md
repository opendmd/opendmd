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
`"<version> <variant>"` (`4mb` or `8mb`: the flash layout, below), and the
release signs the bundle with the same `FW_VERSION` in its header. The firmware reports the descriptor's
version (`state::firmware_version`, `opendmd_core::ota::version_from_app_desc`)
as `version` in `/api/status` and `GET /api/firmware`, in the boot banner
and on the setup page, and compares it with the `done-<version>.bin` the
updater leaves after an update (a mismatch is a warning in the log).
`make fw-ota` waits for exactly that string.

**Older bundles.** A validly signed bundle is installed whatever its
version: going back is a feature (a release that misbehaves). A firmware
can be built with a **rollback floor** instead, off by default:
`make fw-release FW_OTA_FLOOR=1.0.0` (`OPENDMD_OTA_FLOOR`, read at compile
time) refuses any bundle older than that version with a 422 at its header
("firmware X is older than 1.0.0, the oldest this player installs";
`opendmd_core::ota::check_floor`, the same order as `older()`). The web
app's Firmware card reads the chosen file's header and, when its version
is older than the running one (semver-ish: dot-separated numbers, a
pre-release before its release; `older()` in `js/firmware.js`), asks
"This is older than the firmware on the panel (X). Install it anyway?"
first. `make fw-ota` does not ask.

**Timing**, on a 64 GB card at 20 MHz: the 2.09 MB bundle uploads in ~10-16 s; the updater
takes ~19 s (SHA-256 on the card 2.9 s, the write 13.7 s, the image check
1.4 s); `GET /api/firmware` answers with the new version ~33 s after the
upload ends, playback ~45 s. A bundle signed with another key is refused
at its header: curl has the 422 after 1.06 s (its `Expect: 100-continue`
wait: the device never sends `100 Continue`; 0.07 s with `-H 'Expect:'`),
having sent 64 KB of the 2 MB; Chromium (the Firmware card's XHR) keeps
sending until the device closes, ~2 s later, and shows the reason after
~4.6 s.

## Two flash layouts

There are two firmware images per release, one per flash layout (the
firmware's `variant`, in `/api/status`, `GET /api/firmware` and the app
descriptor):

| variant | board | flash | table |
|---|---|---|---|
| `4mb` | ESP32 DevKit V1 ("DevKit (4 MB)") | 4 MB | `firmware/partitions.csv` |
| `8mb` | an 8 MB board in development ("8 MB build") | 8 MB | `firmware/partitions-8mb.csv` |

The code is the same; the build differs in the partition table, the flash
size in the bootloader's and the app's image headers, the variant compiled
in (`OPENDMD_FLASH_VARIANT`), and the name the Improv answer gives
(`OpenDMD`, `OpenDMD 8 MB`: the installer only offers Update to a board
running the same one). The layout is fixed by a USB install: an update over
WiFi replaces the firmware only, never the table or the bootloader. So a
bundle carries its layout, and every place that installs one checks it:

- **The bundle's magic** names the layout (below). The signature covers it.
- **The firmware** refuses a bundle for the other layout at its header
  (`Header::check_variant` against `FLASH_VARIANT`, before the signature
  and before anything reaches the card): 422 "this update is for the
  8 MB build and this player runs the DevKit (4 MB) firmware:
  use the DevKit (4 MB) update file". After the image, it checks the
  image's app descriptor is `"<version> <variant>"` for the bundle's
  layout (`check_app_desc`), so an image signed under the wrong layout by
  mistake is refused too.
- **One updater for both.** `updater.bin` is built once, with a 4 MB
  image header, and goes into the 8 MB layout too. That is harmless: an
  app's own header size is not used at run time (IDF takes the chip's size
  from the bootloader's header, 8 MB on an 8mb board, and does not override
  it from the app's), and neither selecting nor verifying the updater
  looks at it.
- **The updater** accepts both magics and installs a bundle only if its
  layout is the board's (the flash size the installed bootloader set: 4 MB
  is `4mb`, 8 MB is `8mb`; the 8 MB table runs only with the 8mb
  bootloader, which checks the table against it) and the image's app
  descriptor agrees; otherwise it installs nothing and boots the firmware
  that is there.
- **Players from before the 8 MB layout** (their firmware and their
  updater) know only the 4mb magic, so they refuse an 8mb bundle as "not an
  OpenDMD firmware bundle (bad magic)". 4mb bundles keep that magic, so
  they take 4mb bundles as before.
- **The web app** reads the chosen file's magic and refuses the other
  board's in words; Check for updates takes the release's file for the
  player's variant (`release.json` `variants`). `tools/fw-ota.sh` (`make
  fw-ota`) refuses a mismatch before sending; `tools/fw-sign.py sign`
  refuses to sign an image whose descriptor is for the other layout.

A 4mb image runs on an 8 MB board (with the 4 MB layout; IDF uses the size
in the bootloader's header), so the DevKit build is the safe choice when in
doubt; `GET /api/firmware`'s `flash_size` (the chip's real size) lets the
web app say when a board has room for the 8mb build. An 8mb image does not
run on a 4 MB chip (in QEMU, IDF stops at boot with "Detected size(4096k)
smaller than the size in the binary image header(8192k)"). What the
installer's 8 MB button does to a DevKit exactly is not tested on a board;
either way the firmware won't start, and installing again with the DevKit
button and Erase ticked recovers it.

## If a board can't run its flash at 80 MHz

Builds run the flash at 80 MHz by default; the app sets the clock from its
own image header, the updater and bootloader stay at 40. A board whose
flash or wiring cannot do 80 MHz would boot-loop after an update (the
updater cannot notice, there is no rollback and no earlier image, and the
updater is no longer reachable), and only a USB install of a 40 MHz build
(`FW_FLASH_FREQ=40 make fw-release`) recovers it; BOOT only lets the
installer connect. Details, the clocks of each stage and the proposals to
make it safer: [`notes/flash-clock.md`](notes/flash-clock.md).

## Why not two app slots

The release image is ~2.3 MB and the web app pack has its own 1.25 MB
partition (`docs/www-pack.md`), so two full app slots do not fit in 4 MB.
(They would in 8 MB, but one scheme for both layouts keeps one update path
and one updater.)
The device cannot overwrite the partition it is running from either. So the
update is staged on the card and written by a small second program.

## Flash layout (v2)

4mb (`firmware/partitions.csv`):

```
nvs       0x9000   0x6000
otadata   0xF000   0x2000      which app boots next
phy_init  0x11000  0x1000
factory   0x20000  0x240000    the firmware (2.25 MB)
ota_0     0x260000 0x60000     the updater (384 KB, C on ESP-IDF)
www       0x2C0000 0x140000    the web app pack (unchanged offset and size)
```

The www partition keeps its offset and size, so a relayout flash keeps the
pack it holds. The factory slot shrank from 2.75 to 2.25 MB; the size line
in `make fw-release` insists on 64 KB to spare.

8mb (`firmware/partitions-8mb.csv`), the same scheme, bigger:

```
nvs       0x9000   0x6000      as 4mb
otadata   0xF000   0x2000      as 4mb
phy_init  0x11000  0x1000      as 4mb
factory   0x20000  0x300000    the firmware (3 MB)
ota_0     0x320000 0x60000     the updater (384 KB, the same updater.bin)
www       0x380000 0x200000    the web app pack (2 MB)
spare     0x580000 0x280000    data, unused (2.5 MB)
```

- `factory` 3 MB: the image is ~2.3 MB, ~1.8 MB of it code. More would be
  of no use: the ESP32 maps at most 3.19 MB of code (IROM 0x400D0000 to
  0x40400000). The 64 KB-spare rule applies to each layout's own slot.
- `www` 2 MB: the pack is one for both layouts, built to fit the 4 MB
  table's 1,280 KB; the rest is room for later. The pack is
  memory-mapped into the 4 MB data window it shares with the app's
  rodata (~0.36 MB).
- `spare` is declared now so a later firmware can use it after an update
  over WiFi: a new partition needs a USB install, as any table change does.
- NVS, otadata and phy_init are where they are in the 4 MB table, so a USB
  install that changes layout (unticked Erase) keeps the settings.

## The signed bundle

`opendmd-<version>-<variant>.odfw` = header + app image.

```
magic     "ODFW1" + layout: "\0\0\0" (4mb) or "8mb"   8 B
version   32 B, NUL-padded (e.g. "0.4.0")
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
unchanged since beta.14) and `make ota-compat`, which runs the beta.14
tag's own `opendmd-core` against a bundle and a www pack signed now.

## Keys

The firmware trusts the keys in `ota::KEYS` (id, public key); today only
key 0. **Rotating** later: make a new key, add it as id 1 to `KEYS` in a
release signed with key 0 (every player takes it), then sign later
releases with key 1 (`KEY_ID` in `tools/fw-sign.py`); retire key 0 from
`KEYS` once no player needs it. A leak of key 0 before that means a USB
reinstall for every player that never got the release with key 1.

**Local-only signing.** Releases are signed on the owner's machine
(`make release`, `tools/release.sh`): the private key exists only there,
in `~/.config/opendmd/opendmd.key`, read by `tools/fw-sign.py` (pinned
`pynacl==1.5.0`) and never copied into a build directory, a CI runner or
the repo. CI builds and tests with a throwaway key (`FW_SIGN_KEY`,
`FW_SIGN_PUB`); its release workflow is not the way releases are made.

The web app pack is signed with the same key and key ids
(`docs/www-pack.md` "Signature").

## The flow

1. **Browser or curl**: `PUT /api/firmware` with the bundle (Settings →
   Device → Firmware: "Update from file…", or "Check for updates", below).
   The device streams
   the body to `/update/opendmd.bin` on the card, hashing as it goes (the
   night window's heap rules apply: one transfer at a time, 4 KB buffers).
2. **Verify on the device**: header magic, the bundle's layout is the
   player's, image fits the factory partition, ESP image magic `0xE9` at
   offset 0, SHA-256 matches, Ed25519 signature valid, the image's app
   descriptor is `"<version> <variant>"` for that layout. Any failure: delete the file, 422 with a reason, nothing else
   changes. Success: write `/update/pending.txt` (version), set the boot
   partition to `ota_0` (`esp_ota_set_boot_partition`), reply
   `{"ok":true,"version":…,"rebooting":true}`, blank the panel, restart.
3. **Updater** (`ota/updater`, C): blanks the panel first (same early blank
   as `hub75_shim`), mounts the card on SPI3 (18/19/23/5), opens
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
5. **After boot**: the firmware sees `/update/done-*.bin`, logs the version
   change in the log ring, deletes the file, and the web app's Firmware card
   shows the new version. `GET /api/firmware` reports `{version, build,
   variant, flash_size, pending, last, log}`: `log` is the end of `/update/log.txt`
   (≤ 1 KB of whole lines; the Firmware card's "Update log"), so the
   updater's account can be read without taking the card out.

## Check for updates

Settings → Device → Firmware → **Check for updates** (`js/update.js`)
updates the firmware and the web app from the published release, with the
browser doing the fetching: the player itself never goes to the internet.

- **The release.** The project site publishes
  `https://opendmd.dev/install/release.json`:
  `{"version": "1.0.0", "firmware": "firmware/opendmd-1.0.0-4mb.odfw",
  "variants": {"4mb": {"firmware": "firmware/opendmd-1.0.0-4mb.odfw"},
  "8mb": {"firmware": "firmware/opendmd-1.0.0-8mb.odfw"}}, "www":
  "firmware/opendmd-1.0.0-www.pack", "www_version": "v1.0.0"}`, the paths
  relative to the JSON's own address, `www_version` the pack header's
  version (what `GET /api/www` reports for it). The page takes
  `variants.<the player's variant>.firmware`; `firmware` is the 4mb bundle,
  for the web apps from before there were two. The page is plain HTTP
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
  the download, its version `www_version`, and at most 1,310,720 B (the
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
- In the web demo (no player) the Settings view has no Firmware card, so
  there is nothing to check.

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

- Host: `tools/fw-sign.py` round-trips (sign, verify, tamper → fail, a
  bundle relabelled to the other layout → fail, a 4mb image in an 8mb
  bundle → fail); the firmware's verifier is in `opendmd-core::ota` with
  tests on the header, a known-answer signature for each layout and the
  app descriptor check.
- Browser: `make web-smoke` installs a bundle through "Update from
  file…" and runs "Check for updates" against a locally served
  `release.json` and files, through `tools/mock-device.py` (its simulated
  restart, and `PUT /api/www` taking on the pack's version): up to date,
  each failure, and an update applied and the page reloaded.
- Device: a spare DevKit with any FAT32 card (no panel needed): flash the v2
  layout once over USB, then `make fw-ota HOST=…` twice (a version bump
  between), then the fault drills: a tampered bundle (422), a cut-off upload
  (408, nothing staged), card pulled during the updater's write (recovers on
  reinsertion). The serial capture watches the updater's log.
- Then the real player: one USB flash for the layout, then every
  further firmware update over WiFi.
