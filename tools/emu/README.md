# tools/emu: the emulator correctness lane

Boots OpenDMD's real classic-ESP32 4mb full flash image in
[esp32sim](https://github.com/mikroverk/esp32sim) (MIT; our fork: https://github.com/opendmd/esp32sim, branch `opendmd`) with an SD card.
Correctness only, never timing; agents may run it freely.

```
make emu-build     # fetch esp32sim at the pinned commit of our fork (the SD-SPI board is in it), cargo build --release (cached)
make emu-smoke     # normal 4mb image + card: mount, panel up, httpd, www pack, GIFs start -> EMU_SMOKE_OK
make emu-probe     # psi-probe image: wait for `psi-probe: done`, no `differ`, golden line -> EMU_PROBE_OK
make emu-smoke FW_IMAGE=/path/to/x-4mb-full.bin   # boot a given image instead of building one
make emu-probe EMU_ARGS="--seconds 600 --wall 5400"
```

Files:

- `build-emulator.sh`: clone/fetch https://github.com/opendmd/esp32sim, check out `ESP32SIM_COMMIT`
  (pinned in the script, 551417f on the fork's `opendmd` branch), build. Stamp = repo + commit.
  Refuses a dir outside `~/scratch`.
- The fork: our emulator changes live on its `opendmd` branch (upstream 2e36224 plus one commit, the SD-SPI
  board); upstream PRs maybe later if a change is general enough. To bump, commit on the fork and change the pin.
  The board: a byte-stream SD-over-SPI card on VSPI (SCLK 18, MOSI 23, MISO 19, CS 5) as
  board `esp32dev-sdspi`. Env: `ESP32SIM_SD_IMG` (raw FAT image, copy-on-write in RAM),
  `ESP32SIM_SD_OUT` (write back if modified; the runner leaves it unset), `ESP32SIM_SD_LOG`.
  Spike quality, not upstream quality.
- `mkcard.py`: `tools/ci-card.py`'s synthetic card (with prepared copies) into a 256 MB FAT32 image with
  `mformat`/`mcopy`, no mount, no sudo. Needs `mtools`; says so if missing (it installs nothing).
- `run.py smoke|probe --image FULL.bin`: runs `esp32sim --chip esp32 --board esp32dev-sdspi --boot rom
  --rom esp32_rev300_rom.elf --flash-image ... --console uart0 --net none`, logs the console, judges it.
  Fails at once on a panic, `Guru Meditation`, stack overflow, `abort()` or a second reset.

`EMU_DIR` (default `~/scratch/opendmd-emu-sim`) holds `src/`, `card/`, `logs/`, and the image builds
(`smoke/`, `probe/`). The emulator runs about 0.55 emulated seconds per wall second; the probe is ~5 min wall.

Not covered: timing (emulated cycles are not trustworthy), panel output (I2S TX is not modelled),
inbound HTTP (no port forwarding), WiFi (`--net none`).
