# Firmware updates over the LAN (OTA), design

Status: design 2026-10-02, implementation in progress. Replaces the "no
OTA" statement in `deploying.md` once it lands.

## Why not two app slots

The release image is 1.98 MB and the web app pack has its own 1.25 MB
partition (`docs/www-pack.md`), so two full app slots do not fit in 4 MB.
The device cannot overwrite the partition it is running from either. So the
update is staged on the card and written by a small second program.

## Flash layout (v2)

```
nvs       0x9000   0x6000
otadata   0xF000   0x2000      which app boots next
phy_init  0x11000  0x1000
factory   0x20000  0x240000    the firmware (2.25 MB; image 1.98 MB)
ota_0     0x260000 0x60000     the updater (384 KB, C on ESP-IDF)
www       0x2C0000 0x140000    the web app pack (unchanged offset and size)
```

The www partition keeps its offset and size, so a relayout flash keeps the
pack it holds. The factory slot shrinks from 2.75 to 2.25 MB (270 KB of
headroom over today's image; `tools/check-stack-realign.py` and the size
line in `make fw-release` watch it).

## The signed bundle

`opendmd-<version>.odfw` = header + app image.

```
magic     "ODFW1\0\0\0"  8 B
version   32 B, NUL-padded (e.g. "0.4.0")
length    u32 LE, image bytes
sha256    32 B, of the image
sig       64 B, Ed25519 over (magic|version|length|sha256)
```

Header 140 B, then the image. The public key is compiled into the firmware
(`firmware/keys/opendmd.pub`, 32 B, committed); the private key never enters
the repo (`~/.config/opendmd/opendmd.key`, created by `tools/fw-sign.py
keygen`). `make fw-release` signs; a bundle that fails to verify is refused
by the device with a 422 and nothing on the card changes.

## The flow

1. **Browser or curl**: `PUT /api/firmware` with the bundle (Settings →
   Advanced → Firmware: "Update from file…", later "Check for updates" via
   GitHub releases fetched by the browser over HTTPS). The device streams
   the body to `/update/opendmd.bin` on the card, hashing as it goes (the
   night window's heap rules apply: one transfer at a time, 4 KB buffers).
2. **Verify on the device**: header magic, image fits the factory partition,
   ESP image magic `0xE9` at offset 0, SHA-256 matches, Ed25519 signature
   valid. Any failure: delete the file, 422 with a reason, nothing else
   changes. Success: write `/update/pending.txt` (version), set the boot
   partition to `ota_0` (`esp_ota_set_boot_partition`), reply
   `{"ok":true,"version":…,"rebooting":true}`, blank the panel, restart.
3. **Updater** (`ota/updater`, C): blanks the panel first (same early blank
   as `hub75_shim`), mounts the card on SPI3 (18/19/23/5), opens
   `/update/opendmd.bin`, re-verifies the SHA-256 (the signature was
   checked by the firmware; the updater has no Ed25519 to keep it small),
   erases and writes the factory partition in 4 KB pieces, runs
   `esp_image_verify` on the result, sets the boot partition back to
   factory, renames the file to `/update/done-<version>.bin`, writes
   `/update/log.txt`, restarts. The LED (GPIO 2) blinks while writing.
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
   pending, last}`.

## Hardening that ships with it (same cable session)

- `PUT /api/www` keeps serving the old pack until the new one has verified:
  write to the partition, verify CRC, then flip the header's valid flag
  last (the partition's first page is written last).
- Mutating `/api` requests reject a foreign `Origin` header and require
  `Content-Type: application/json` where a JSON body is expected.
- Optional API key (`api_key` setting; `Authorization: Bearer`).

## Testing

- Host: `tools/fw-sign.py` round-trips (sign, verify, tamper → fail);
  the firmware's verifier is in `opendmd-core::ota` with tests on the
  header and a known-answer signature.
- Device: a spare DevKit with any FAT32 card, no panel: flash the v2
  layout once over USB, then `make fw-ota HOST=…` twice (a version bump
  between), then the fault drills: a tampered bundle (422), a cut-off upload
  (408, nothing staged), card pulled during the updater's write (recovers on
  reinsertion). The serial capture watches the updater's log.
- Rig: once the spare passes, one USB flash for the layout, then every
  further firmware update over WiFi.
