# For developers

OpenDMD is Rust throughout: one core shared by the ESP32 firmware, the
desktop simulator and the WebAssembly web app. This page is the starting
point for building it yourself. Everyday users don't need any of it; the
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
make blocklist-check         # no API path, asset or site URL may match EasyPrivacy/EasyList (ad blockers); lists cached in target/blocklists
make font                    # the headings' pixel font (Silkscreen): subset it again
make site-font site-scene    # the site's heading face (Fraunces) and the home page's pixel-art scene, made again
make fw-build                # the firmware (the first build fetches ESP-IDF)
make fw-flash PORT=/dev/ttyUSB0   # bench build over USB, then the serial monitor
```

Put settings for your own machine in a `local.mk` beside the Makefile. It
is gitignored, and the Makefile reads it first. `CARD_ROOT = /path/to/card`
is the card that `make sim`, `make web-mock` and `make web-smoke` use
(`card/` in the repo by default); `export SCRATCH = /path/outside/the/tree`
moves the updater, emulator and release builds out of `target/`.

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
links. The firmware's come from the release ELFs, with every C and C++
source file compiled in mapped to its package by
`licenses/components.toml`, and from `cargo-about` over its crates. The
web app's come from `cargo-about` over the WebAssembly crates. `make fw-release` and CI fail when they are out of date
or a dependency's licence is not one the project accepts.

A release build has no WiFi credentials in it. The installer gives it
your WiFi over USB with Improv Serial, or the firmware starts its
`OpenDMD-Setup` network instead. Firmware updates over WiFi are signed
bundles; the firmware checks the signature before installing, and a small
updater program puts the new firmware in place from the SD card.

The repository's deploying notes (`docs/deploying.md`),
firmware notes (`docs/firmware.md`) and OTA notes (`docs/ota.md`) cover the flash
layout, recovery and the update flow in detail.

## The crates

| Crate | What it is |
| --- | --- |
| `opendmd-core` | Manifests, tag parsing, filtering, scheduling, GIF playback and the playlist engine. No I/O. |
| `opendmd-render` | The LED-panel renderer used by the simulator and the web app |
| `opendmd-sim` | The desktop simulator |
| `opendmd-web` | The web app: wasm module plus hand-written HTML, CSS and JS in `www/` |
| `opendmd-art-web` | Create Art's image processing and GIF writer, loaded on demand |
| `opendmd-pack` | Prepares a card's GIFs for playback, offline |
| `opendmd-psi` | Psi, the Formula language: its compiler and VM (`no_std`), shared by the firmware, the simulator and the browser |
| `opendmd-psi-cli` | `psi`, Formulas from the command line: `psi check` and `psi fmt` (`make psi` installs it) |
| `firmware` | The ESP32 firmware |

The crates are deliberately not a Cargo workspace.

### Virtual files

Paths that begin with `@` are drawn by the core instead of read from the
card. `opendmd-core/src/synth.rs` parses them.

- `@boot` is the start-up show (`formula=` puts a Formula in the
  wordmark's place, as the `boot_formula` setting does).
- `@power-on` is the power-on effect.
- `@card:<title>|<subtitle>` is a title card.
- `@formula?psb=…` is a compiled Psi Formula (`docs/psi-for-models.md`).
  `opendmd-psi` compiles it in the browser, or `psi build` does; the same
  code runs it in the firmware, the simulator and the browser. The firmware
  compiles only its card's `.psi` files that lack a current `.psb`
  (`firmware/src/card_formulas.rs`). Formulas draw everything, the clock
  included.

Their options and limits are in the [HTTP API](api.md#virtual-files). To
see one without a board, run `make sim ARGS="--play '@boot?len=full'"`.

## Reference

- [HTTP API](api.md): every route the display answers, with `curl`
  examples. The web app uses nothing else, so a script can do what it
  does. The page follows the latest docs and links the page for each
  release, as that release described it; the web app's Settings › Device ›
  About opens the one for the display's firmware version.
- [Psi language reference](psi-spec.md): the Formula language, every
  name it knows, its limits; the `psi` command. Its
  [error codes](psi-errors.md) have a page of their own.
- [The simulator](simulator.md): running it from source, its keys and screenshots,
  and where the LED model lives.
- [Hardware pinout](../hardware/PINOUT.md): the DMDoS carrier board's
  wiring.
