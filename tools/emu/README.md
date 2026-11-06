# tools/emu: the emulator correctness lane

Boots OpenDMD's real classic-ESP32 4mb full flash image in
[esp32sim](https://github.com/mikroverk/esp32sim) (MIT; our fork: https://github.com/opendmd/esp32sim, branch `opendmd`) with an SD card.
Correctness only, never timing; safe to run any time: it never touches a device.

```
make emu-build     # fetch esp32sim at the pinned commit of our fork (the SD-SPI board is in it), cargo build --release (cached)
make emu-smoke     # normal 4mb image + card: mount, panel up, httpd, www pack, GIFs start -> EMU_SMOKE_OK
make emu-probe     # psi-probe image (emu8mb, firmware/partitions-emu.csv: an emulator-only 8 MB layout with a 3 MB factory, never released; 64 KB spare; run.py gives esp32sim --flash-mb from the image size): wait for `psi-probe: done`, no `differ`, golden line -> EMU_PROBE_OK
make emu-paint-probe  # paint-probe image (4mb, a test board's; 80 MHz flash like every build, FW_FLASH_FREQ=40 for the old clock; its .odfw in $EMU_DIR/paint/dist goes to a test board by OTA): `psi-probe: paint: N cases, 0 differ` (each case's kernel frames against the VM's) -> EMU_PROBE_OK
make emu-rig       # smoke image + the full-scale card (mkcard.py --rig): the FAT index build finishes, heap floor reported -> EMU_RIG_OK
make emu-rig-tight # the same with a test board's playing heap (an emu-ballast image, ~46 KB free) -> EMU_RIG_OK
make emu-leak      # an emu-census image plays 70 GIFs (on to 150 while the halves differ): the heap's blocks and bytes stay flat after 10 -> EMU_LEAK_OK
make emu-api       # the emu-census image on WiFi, its HTTP API driven from the host over two boots -> EMU_API_OK
make emu-smoke FW_IMAGE=/path/to/x-4mb-full.bin   # boot a given image instead of building one
make emu-probe EMU_ARGS="--seconds 600 --wall 5400"
```

Files:

- `build-emulator.sh`: clone/fetch https://github.com/opendmd/esp32sim, check out `ESP32SIM_COMMIT`
  (pinned in the script, a7ccd1b on the fork's `opendmd` branch), build. Stamp = repo + commit.
  With `SCRATCH` set (e.g. in `local.mk`), refuses a dir outside it.
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

`EMU_DIR` (default `target/emu-sim`, or `$SCRATCH/opendmd-emu-sim` when `SCRATCH` is set) holds `src/`, `card/`, `logs/`, and the image builds
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
- bursts (a test board's wedge recipe, `wedge.sh`): rounds of 2, 2, 2, 5, 5, 5 uploads of 300 KB at once
  (`curl -T`) beside a status read every second: each answered 200, or 503 + `Retry-After` and 200 when
  asked again (as the web app does), never a 408 or no answer; the network answering after every round;
  and 25 s after, `netdiag: sum` (below) shows no received frame held;
  and the least free heap of each round (the `emu-census` image's low watch: a task reading the free
  heap every ms, `lowwatch:` every 5 s with the Rust bytes, what the httpd and stream tasks were
  doing and lwIP's received frames and unacknowledged bytes at the low) never under
  `BURST_HEAP_MIN` (32 KB here, ~26 KB above that test board; see BURST_HEAP in `api_lane.py`);
- three load rounds of 30 s, display-stress's five loads each on its own connection at once by default
  (`--connections 5`): a 64 KB upload read back and deleted, status reads, settings saves, a save into
  `/gifs/Consoles` and its delete, a listing; each followed by 60 s quiet (about eight census lines): the
  heap census of the third quiet spell against the first's (as emu-leak: 24 blocks, 2 KB).
  `EMU_ARGS="--connections 2"` runs the status polls beside the rest in turn (no bursts),
  `--connections 1` everything in turn;
- a recursive `DELETE /api/files/cache/Consoles?recursive=1` (rounds until `done`) while the FAT index
  builds: the build is dropped (no `built` line before the delete), rebuilt, not stale, fewer records, no
  `fat index:` warning, `/cache/Consoles` 404, one of its GIFs plays from the original;
- `GET /api/firmware`; the signed bundle with a flipped signature bit, then with a flipped image bit: 422,
  nothing pending;
- every file read back again, the index current; an emptied `/gifs` folder deleted: the index stale at
  once and rebuilt current (EMPTY_FOLDER); `POST /api/reboot`.

Then the card boot A wrote: `fatcheck.py`, `/opendmd.json` holds the last `dwell_max` saved, the files the
bytes uploaded, the deleted and moved paths gone, no `.part`/`.tmp` left. Boot B (from that card, reboots
allowed): the setting and the files survived; `PUT /api/firmware` with the signed bundle (the build's
`.odfw`, `EMU_API_BUNDLE`): signature ok, staged, the updater installs it and the firmware is back with
its version (three resets: power-on, the updater, the firmware). Heap and stack high-water (heartbeat
floor, main, log drain, httpd, stream task, network worker) are printed. About 9 minutes of wall time (the image builds aside).

`EMU_API_ONLY=bursts` (a diagnostic shortcut) boots, runs the bursts alone and stops (~2 min wall).

What it found (2026-11-03), see the comments in `api_lane.py`:

- **RX_BUFFERS** (fixed in net-wedge, the same day): two uploads at once got the one being received a 408
  after 5 s of nothing, and a few rounds wedged the network for good (no TCP, ping or ARP answered; the
  spare too). The `emu-census` image's `netdiag:` lines (`components/hub75_shim/netdiag.c`, every 5 s on
  the network worker: lwIP's pcbs and sockets walked on its own thread, the frames each holds; every one
  is a WiFi driver receive buffer, six in all) showed it: the upload being received held four
  out-of-order segments after a lost one and the waiting upload the other two, so its retransmission
  had no buffer; then the 408'd uploads, closed into FIN_WAIT_1 with no socket, kept their out-of-order
  segments (4 + 2), the ACK of their FIN could never come in, and the queue's expiry (6 RTOs) receded as
  the RTO backed off: nothing received again. Fixed: the expiry is one RTO (the silence always passes it
  between two retransmissions), and one upload at a time, taken only with the stream task idle, a
  second one refused at once (503 + `Retry-After: 2`, keeping its place in line; a waiting upload held
  five buffers for 2.6 KB of small segments, and starved a download ahead of it as well as an upload;
  the slot is free once the reply is out). The windows stay 5760,
  now asked for (the 1440 asked for was under Kconfig's minimum and silently 5760; 2880 made uploads
  ~3.5x slower here and the 2.2 MB firmware bundle hit the 120 s body cap; no out-of-order queue at all
  did the same to the www pack). See "The receive side" in `firmware/sdkconfig.defaults` and
  `stream.rs`. Concurrency is in the gate now: the bursts and the five-connection load.
- **EMPTY_FOLDER** (fixed in net-wedge): deleting an emptied `/gifs` folder never reached
  `fatidx::dir_changed()` (FatFs's unlink removes an empty directory, so the `is_dir` branch was
  dead). The DELETE route tells a directory apart before removing anything now.
- **BURST_HEAP** (net-wedge2): a test board's heap reached 8 KB under five uploads at once and the
  WiFi driver, which takes its frame buffers from it, stopped delivering (refusals 10.5 s late,
  status reads timing out). The low watch put every low of the bursts in a status read during an
  upload: the whole answer and all its parts at once (~5 KB of strings), its ~3.7 KB of segments
  and the upload's received frames (up to six buffers, ~10 KB). The answer is written in pieces
  now, refused under 10 KB free, and a client that gave up has its reply aborted at close.
- **STALLED_BODY** (net-stall, 2026-11-05): a JSON body read on the httpd task that stopped part
  way held the task, and every other client, for 3 x 10 s of receive timeouts and 10 s more of
  httpd's purge (a test board: live edits timing out at 20 s beside simultaneous uploads, status
  polls' SYNs dropped while nothing was accepted). One 2 s timeout ends it now: a 503 +
  `Retry-After`, the connection dropped, `net.body_stalled`. The lane sends such a body and a
  status read beside it. (Connect times are not seen here: the emulator's NAT completes the
  host's handshake itself, so a SYN the firmware drops never shows.)
- **LONG_REQUEST** (net-stall): the requests answered by another task (a rebuild up to 60 s, the
  WiFi scan 12 s, the RePlayOS test 20 s) were waited out on the httpd task, with every other
  client held. Past 0.3 s they are a 202 + `X-Job` now, answered through `GET /api/job`; the lane
  checks status reads answering while the scan is pending, and a second long request refused.
- The httpd task's stack: 736-1024 B of 6656 B unused under the lane.

Not covered: timing (emulated cycles are not trustworthy; the virtual AP delivers frames with no air
time), panel output (I2S TX is not modelled), radio conditions (a lossy or slow link, roaming, the AP
going away), mDNS and SNTP (nothing answers in the namespace), Follow RePlayOS, the captive portal / AP
mode setup flow, a browser's own concurrency (the lane's clients are sequential per connection), the full-scale card under the API (`EMU_ARGS="--card $EMU_DIR/card/rig.img"` boots it,
not part of the gates).
