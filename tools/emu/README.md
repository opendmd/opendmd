# tools/emu: the emulator correctness lane

Boots OpenDMD's real classic-ESP32 4mb full flash image in
[esp32sim](https://github.com/mikroverk/esp32sim) (MIT; our fork: https://github.com/opendmd/esp32sim, branch `opendmd`) with an SD card.
Correctness only, never timing; agents may run it freely.

```
make emu-build     # fetch esp32sim at the pinned commit of our fork (the SD-SPI board is in it), cargo build --release (cached)
make emu-smoke     # normal 4mb image + card: mount, panel up, httpd, www pack, GIFs start -> EMU_SMOKE_OK
make emu-probe     # psi-probe image (8mb layout: the full probe outgrew the 4mb partition; EMU_PROBE_VARIANT=4mb for the DevKit one; run.py gives esp32sim --flash-mb from the image size): wait for `psi-probe: done`, no `differ`, golden line -> EMU_PROBE_OK
make emu-rig       # smoke image + the rig-scale card (mkcard.py --rig): the FAT index build finishes, heap floor reported -> EMU_RIG_OK
make emu-rig-tight # the same with the spare's playing heap (an emu-ballast image, ~46 KB free) -> EMU_RIG_OK
make emu-leak      # an emu-census image plays 50 GIFs: the heap's blocks and bytes stay flat after 10 -> EMU_LEAK_OK
make emu-api       # the emu-census image on WiFi, its HTTP API driven from the host over two boots -> EMU_API_OK
make emu-smoke FW_IMAGE=/path/to/x-4mb-full.bin   # boot a given image instead of building one
make emu-probe EMU_ARGS="--seconds 600 --wall 5400"
```

Files:

- `build-emulator.sh`: clone/fetch https://github.com/opendmd/esp32sim, check out `ESP32SIM_COMMIT`
  (pinned in the script, a7ccd1b on the fork's `opendmd` branch), build. Stamp = repo + commit.
  Refuses a dir outside `~/scratch`.
- The fork: our emulator changes live on its `opendmd` branch (upstream 9fdeb32, merged 2026-11-03, plus the
  SD-SPI board); upstream PRs maybe later if a change is general enough. To bump, commit on the fork and change the pin.
  The board: a byte-stream SD-over-SPI card on VSPI (SCLK 18, MOSI 23, MISO 19, CS 5) as
  board `esp32dev-sdspi`. Env: `ESP32SIM_SD_IMG` (raw FAT image, copy-on-write in RAM),
  `ESP32SIM_SD_OUT` (write back if modified; the runner leaves it unset), `ESP32SIM_SD_LOG`.
  Spike quality, not upstream quality.
- `mkcard.py`: `tools/ci-card.py`'s synthetic card (with prepared copies) into a 256 MB FAT32 image with
  `mformat`/`mcopy`, no mount, no sudo. Needs `mtools`; says so if missing (it installs nothing).
- `api_lane.py` (run by `run.py api`): the API lane, below. `fatcheck.py CARD.img`: a read-only FAT32
  check (cross-linked or lost clusters, chains the wrong length, FAT copies that differ), no mount.
- `run.py smoke|probe --image FULL.bin`: runs `esp32sim --chip esp32 --board esp32dev-sdspi --boot rom
  --rom esp32_rev300_rom.elf --flash-image ... --console uart0 --net none`, logs the console, judges it.
  Fails at once on a panic, `Guru Meditation`, stack overflow, `abort()` or a second reset.

`EMU_DIR` (default `~/scratch/opendmd-emu-sim`) holds `src/`, `card/`, `logs/`, and the image builds
(`smoke/`, `probe/`). The emulator runs about 0.55 emulated seconds per wall second; the probe is ~5 min wall.

## The API lane (`make emu-api`)

The leak lane's `emu-census` image (the same build) with WiFi credentials seeded into a copy's NVS
(`api_lane.seeded_image`: namespace `opendmd`, `wifi_ssid`/`wifi_pass`, IDF v5.3's `nvs_partition_gen`
from `~/.espressif/python_env/idf5.3_py3*_env`, at the table's `nvs` offset; cached under `$EMU_DIR/api/`),
run with `--wifi ssid=demo,chan=6,psk=demo-password --net nat --hostfwd tcp:127.0.0.1:8080-:80`.
`run.py` runs itself again under `unshare --user --map-root-user --net`: a private network namespace with
only a loopback, so the emulator's NAT reaches nothing (mDNS, SNTP, DNS and RePlayOS fail quietly; no real
device ever sees a packet), and port 8080 never collides with another run. Needs unprivileged user
namespaces (on here) and `mtools`.

Boot A (`--no-reboot`, so the reboot at its end stops the emulator and the card is written out):

- the first FAT index build; `GET /api/status`, `/api/config`; `PUT /api/config` (`dwell_max`) read back,
  a bad value 400 and nothing changed;
- uploads (`PUT /api/files/...`, a 300 KB one, `?new=1` and its 409 over an existing file, a replace),
  each read back byte for byte; listings (`?sizes=1`, a walk, and `?names=1`) exactly the files written;
  a move to a new folder (and onto an existing name 409, from nowhere 404); a delete (404 after);
- `/api/play` of an uploaded GIF (`<- original`), a Formula as `@formula?psb=` (`opendmd-psi-cli build
  --base64`, `EMU_API_FORMULA`, default plasma), the same saved to the card (`.psi` + `.psb`) by its path;
  a missing file 404, a relative path 400; skip; pause and resume;
- `PUT /api/www` with `$(WWW_PACK)`; `GET /` serves it;
- three load rounds of 30 s: status polls throughout, and on a second connection in turn a 64 KB upload
  read back and deleted, a settings save, a save into `/gifs/Consoles` and its delete, a listing; each
  followed by 30 s quiet (~100 emulated s, five census lines): the heap census of the third quiet spell against the first's (as emu-leak:
  24 blocks, 2 KB);
- a recursive `DELETE /api/files/cache/Consoles?recursive=1` (rounds until `done`) while the FAT index
  builds: the build is dropped (no `built` line before the delete), rebuilt, not stale, fewer records, no
  `fat index:` warning, `/cache/Consoles` 404, one of its GIFs plays from the original;
- `GET /api/firmware`; the signed bundle with a flipped signature bit, then with a flipped image bit: 422,
  nothing pending;
- every file read back again, the index current, `POST /api/reboot`.

Then the card boot A wrote: `fatcheck.py`, `/opendmd.json` holds the last `dwell_max` saved, the files the
bytes uploaded, the deleted and moved paths gone, no `.part`/`.tmp` left. Boot B (from that card, reboots
allowed): the setting and the files survived; `PUT /api/firmware` with the signed bundle (the build's
`.odfw`, `EMU_API_BUNDLE`): signature ok, staged, the updater installs it and the firmware is back with
its version (three resets: power-on, the updater, the firmware). Heap and stack high-water (heartbeat
floor, main, log drain, httpd, stream task, network worker) are printed. About 6 minutes of wall time (the image builds aside).

What it found (2026-11-03), see the comments in `api_lane.py`:

- **RX_BUFFERS**: concurrent connections stall the one being served (an upload `aborted ... no data
  for 5 s`, a download `response cut short`) and can wedge the network for good (no SYN answered,
  the player playing on); two upload bodies at once do it in seconds. Most likely the WiFi driver's
  6 dynamic RX buffers held by frames lwIP keeps for connections not read yet: `sdkconfig.defaults`'
  `CONFIG_LWIP_TCP_WND_DEFAULT=1440` (and `SND_BUF`) is below Kconfig's range (2440..) and silently
  becomes 5760, so one queued upload holds four. 16 RX buffers pass two uploads at once but the five
  loads still wedged in their third round. The lane's load keeps to two connections until that
  changes; `EMU_ARGS=--parallel` runs the five loads on five connections and shows it.
- **EMPTY_FOLDER**: deleting an emptied `/gifs` folder never reaches `fatidx::dir_changed()`
  (FatFs's unlink removes an empty directory, so the `is_dir` branch is dead): the index is not
  marked stale for a folder that went.
- The httpd task's stack: 736-1024 B of 6656 B unused under the lane.

Not covered: timing (emulated cycles are not trustworthy; the virtual AP delivers frames with no air
time), panel output (I2S TX is not modelled), radio conditions (a lossy or slow link, roaming, the AP
going away), mDNS and SNTP (nothing answers in the namespace), Follow RePlayOS, the captive portal / AP
mode setup flow, a browser's own concurrency (the lane's clients are sequential per connection), the
8mb layout's OTA, the rig-scale card under the API (`EMU_ARGS="--card $EMU_DIR/card/rig.img"` boots it,
not part of the gates).
