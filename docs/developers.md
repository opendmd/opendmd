# For developers

OpenDMD is Rust throughout: one core shared by the ESP32 firmware, the
desktop simulator and the WebAssembly web app. This page is the starting
point for building it yourself. Everyday users don't need any of it: the
[installer](guide/install.md) puts a release on the board from the browser.

## Building from source

The core, renderer, simulator and card tools build with stable Rust on any
desktop. The web app also needs the `wasm32-unknown-unknown` target and
`wasm-pack`. The firmware needs the Espressif xtensa toolchain:

```sh
cargo install --locked espup ldproxy espflash
espup install --targets esp32 --export-file ~/.espup-env.sh
uv tool install esptool
git submodule update --init          # the HUB75 panel driver
```

Common targets:

```sh
make test                    # unit tests for every host crate
make clippy                  # clippy -D warnings, every crate
make sim ARGS="--root /path/to/card"
make web && make web-mock ROOT=/path/to/card   # the web app against a fake device on :8080
make web-smoke               # headless Chromium through every view
make fw-build                # the firmware (the first build fetches ESP-IDF)
make fw-flash PORT=/dev/ttyUSB0   # bench build over USB, then the serial monitor
```

## Releases and updates

```sh
make fw-release              # dist/: firmware, web app pack, merged 4 MB image, signed update bundle
make flasher                 # the browser installer with dist/ on http://localhost:8765
make improv-check            # the real Improv Serial host library against the firmware's framing (node, npm)
make fw-ota HOST=opendmd.local     # firmware over WiFi (signed bundle)
make www-push HOST=opendmd.local   # web app only, over WiFi
make sim-dist                # the simulator's download archive for this platform (release.yml builds all three)
make site                    # this site, with dist/'s release under install/ (and target/sim-dist/'s simulators)
```

A release build has no WiFi credentials in it: the installer gives it
your WiFi over USB (Improv Serial), or the player starts its
`OpenDMD-Setup` network instead. Firmware updates over WiFi are signed
bundles: the player checks the signature before installing, and a small
updater program copies the new firmware into place from the SD card.

The repository's [deploying notes](deploying.md),
[firmware notes](firmware.md) and [OTA notes](ota.md) cover the flash
layout, recovery and the update flow in detail.

## The crates

| Crate | What it is |
| --- | --- |
| `opendmd-core` | Manifests, tag parsing, filtering, scheduling, GIF playback and the playlist engine. No I/O. |
| `opendmd-render` | The LED-panel renderer used by the simulator and the web app |
| `opendmd-sim` | The desktop simulator |
| `opendmd-web` | The web app: wasm module plus hand-written HTML, CSS and JS in `www/` |
| `opendmd-art-web` | Create Art's image processing and GIF writer, loaded on demand |
| `opendmd-pack` | Prepares a card's GIF copies offline |
| `firmware` | The ESP32 firmware |

The crates are deliberately not a Cargo workspace.

## Reference

- [HTTP API](api.md): every route the player answers, with `curl`
  examples. Anything the web app does, a script can do too.
- [The simulator](simulator.md): the downloads, its options and how it
  draws LEDs.
- [Writing an effect](writing-effects.md): how the panel's effects work,
  their limits on the ESP32, and adding a new one, with a worked example.
- [Hardware pinout](../hardware/PINOUT.md): the DMDoS carrier board's
  wiring.
