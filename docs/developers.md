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
make font                    # the headings' pixel font (Silkscreen): subset it again
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
make site                    # this site, with dist/'s release under install/
make notices                 # rewrite the third-party licence notices from the last fw-release (needs cargo-about)
```

The third-party notices in `licenses/` are generated from what each build
links: the firmware's from the release ELFs (every C and C++ source file
compiled in, mapped to its package by `licenses/components.toml`) and
`cargo-about` over its crates, the web app's from `cargo-about` over the
WebAssembly crates. `make fw-release` and CI fail when they are out of date
or a dependency's licence is not one the project accepts.

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

**Live items.** Paths that begin with `@` are drawn by the core, not read
from the card (`opendmd-core/src/synth.rs` parses them): `@clock`, `@text`,
`@boot`, `@card:` and `@formula?src=…`, a [Psi](psi-for-models.md) Formula
that `opendmd-psi` compiles and the player runs on the same code in the
firmware, the simulator and the browser. The built-in effects (`@fx:tunnel`
and the others) are gone: they are Formulas now. Options and limits are in
the [HTTP API](api.md#live-items); to see a live item without a board,
`make sim ARGS="--play '@clock?date=1'"`.

## Reference

- [HTTP API](api.md): every route the player answers, with `curl`
  examples. Anything the web app does, a script can do too. The page
  follows the latest docs; it links a copy for each release, as that
  release described it, and the web app's Settings → Device → About opens
  the one for the player's firmware.
- [The simulator](simulator.md): running it from source, its keys and screenshots,
  and where the LED model lives.
- [Hardware pinout](../hardware/PINOUT.md): the DMDoS carrier board's
  wiring.
