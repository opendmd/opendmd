# OpenDMD

Open-source replacement for DMDoS, the closed ESP32 firmware that plays 128x32
animated GIFs from microSD onto two chained 64x32 HUB75 panels: a
platform-independent Rust core, a desktop simulator, the web app the player
serves, the ESP32 firmware in `firmware/`, and the opendmd.dev site.

This file is the short version for anyone working on the repository (people
and coding assistants alike). Contributor docs: `docs/developers.md`,
`docs/firmware.md`, `docs/web-app.md`, `docs/api.md`.

## Product model

Users see **GIFs, Formulas and playlists**, nothing else.

- A GIF's *prepared copy* (`docs/cache-format.md`) is a hidden cache: a
  standard GIF89a with its LZW capped at 9 bits and delta frames, which the
  player decodes in far less heap than the original. For a
  `/gifs/<Category>/<name>.gif` path the player first tries
  `/cache/<Category>/<xx>/<name>.gif` (`layout::cache_path`), else plays
  the GIF itself. Copies are made by the device, `opendmd-pack convert` or
  the browser, byte-identical. **Copies are exact or absent**: a GIF past
  256 colours gets no copy and plays as it is.
- Playlists (`filelist.txt`, `/sets/*.txt`) name GIF paths, and Formulas
  by reference (`@formula:<stem>`, `/formulas/<stem>.psi`).
- **Tags and stars** are the user's own (`/sets/.tags`, `opendmd_core::tags`);
  what a name says (platform, folder, Logo…, author) is a GIF's *details*,
  never called a tag. A **smart playlist** is an ordinary `/sets/<name>.txt`
  the web app writes again from its search (`/sets/.smart`,
  `opendmd_core::query`); the player never reads a search. A dot-file in
  `/sets/` is a plain file, never a playlist.
- Formulas are user effects written in **Psi** (`docs/psi-spec.md`,
  `docs/psi-for-models.md`), compiled and run on the player.
- **The UI never shows** "odm", "ODM", "bucket", paths, `@` items,
  "virtual", "weight", `.meta`, "index", "CORS" or "prepared copy"
  (`make web-smoke` checks every view for "odm" and "bucket").

## Build and test

Stable Rust (the host crates), the `wasm32-unknown-unknown` target and
`wasm-pack` (the web app), the Espressif xtensa toolchain via `espup` (the
firmware). Python tools run with `uv run` or `uvx`, never `python` or `pip`
directly. After a clone: `git submodule update --init` (the HUB75 driver).

```bash
make test        # cargo test for every host crate
make test-wasm   # the web crates' tests compiled to wasm, under node
make clippy      # clippy -D warnings, all host crates
make fmt
make check       # cargo check the host crates
make web         # the web app: wasm-pack builds, minified www-dist, the www pack, size budget
make web-mock ROOT=<card copy>    # a fake device on :8080 (tools/mock-device.py)
make web-smoke   # headless Chromium through every view against the mock
make sim ARGS="--root <card copy>"   # the desktop simulator; `make sim-build` on a headless box
make pack ARGS="convert <card copy> --verify"   # prepare a card's cache/
make psi-check F=my.psi           # check a Formula: errors, cost estimates, a contact sheet
make blocklist-check              # no route, asset or site URL may look like a tracker to ad blockers
make fw-build    # ESP32 firmware (bench build)
make fw-clippy   # clippy -D warnings, firmware feature sets
make fw-flash PORT=/dev/ttyUSB0   # bench build + web app + flash + monitor
make fw-release  # release images into dist/ (the 4mb layout), signed with FW_SIGN_KEY
make notices-check                # third-party notices up to date
make emu-build emu-smoke          # the firmware in an emulator (esp32sim): correctness only, never timing
make emu-s3-nocard emu-s3-probe   # the ESP32-S3 preview (VARIANT=s3, not released) in the emulator's S3 machine
```

- Machine-local settings go in a gitignored `local.mk` beside the
  Makefile: `CARD_ROOT` (a card copy, default `card/`), `SCRATCH` (big
  build output outside the tree instead of `target/`), anything assigned
  with `?=`.
- **Not a Cargo workspace.** Standalone crates, each with its own
  `Cargo.toml` and `Cargo.lock`, each repeating the `[patch.crates-io] gif`
  stanza (the vendored `third_party/gif/`). The host crates carry no
  `.cargo/config.toml` and build unchanged on Linux and macOS; never put a
  `[build] target` in a root `.cargo/config.toml`. Every Makefile target
  sets `RUSTUP_TOOLCHAIN=stable` explicitly.
- Serial tools use 921600 baud (flashing 115200); never read the port with
  `cat` (it resets the board): `tools/serial-capture.py`.

## Repository layout

- **`opendmd-core/`**: the library (deps: `gif`, `log`): manifest and tag
  parsing, filters, the scheduler, the `Engine`, the decoders (`gifslim`,
  the `gif` crate as fallback, virtual files), the compositor, looks,
  playlists, the cache layout rule (`layout`: a change is a card-format
  break) and the recoder.
- **`opendmd-render/`**: the LED renderer shared by the simulator and the
  web app; constants measured off `hardware/IMG_0495.jpeg`.
- **`opendmd-pack/`**: CLI to prepare a card. **`opendmd-sim/`**: the
  desktop simulator (no logic of its own: behaviour belongs in the core).
- **`opendmd-web/`**: the web app the device serves: a wasm module plus
  hand-written HTML/CSS/ES modules in `www/`, no framework, no bundler.
  **`opendmd-art-web/`**: the second wasm module (Create Art, Smooth motion).
  **`opendmd-studio-web/`**: the site's Studio's GIF writers (site-only wasm).
- **`opendmd-psi/`, `opendmd-psi-web/`, `opendmd-psi-cli/`**: Psi, the
  Formula language: compiler, VM and ESP32 JIT, its wasm module, the `psi`
  command. **`opendmd-gallery/`**: the Formula gallery checker.
  **`opendmd-webmin/`**: `make web`'s minifier.
- **`firmware/`**: the ESP32 crate (esp-idf std Rust, ESP-IDF v5.3, classic
  ESP32 DevKit V1, 4 MB flash). **`ota/updater/`**: the
  C updater app. **`flasher/`**: the browser installer.
- **`site/`**: the opendmd.dev site and its Cloudflare Worker.
- **`tools/`**: build, test and release scripts. **`hardware/`**: the
  measured pin map (`hardware/PINOUT.md`, the source of truth) and photos.
- **`docs/`**: the docs (`docs/guide/`, the user guide; `api.md`,
  `ota.md`, `firmware.md`, `web-app.md`, `psi-spec.md`, ...). Comments
  that cite `CLAUDE.md "<section>"` or `opendmd-notes` point at the
  maintainers' engineering notes, kept in a separate repository.

## Rules that hold everywhere

- **The engine never sleeps and never reads the clock**: the caller passes
  `now` into `tick` as a `Duration` since an epoch it picks (never an
  `Instant`: it panics on `wasm32-unknown-unknown`).
- **Card logic (tags, paths, filters, encoding) lives in Rust**, not in
  JS. No `serde` and no `f64::clamp` in the web module.
- **The web app has gzipped size budgets** (`tools/web-size.py`, enforced
  by `make web`). Raising one needs a maintainer's OK; record the reason in
  `tools/web-size.py`.
- **The firmware image must leave ≥ 64 KB of the `factory` partition**
  (`fw-release` fails otherwise). Measure size changes on the release build.
- **Heap is the device's scarce resource** (no PSRAM): no per-frame
  allocation on the display path, large buffers taken fallibly, transfers
  one at a time.
- **The display loop (CPU1) never touches WiFi or a socket**; handlers
  talk to it only through `Arc<Mutex<Status>>` and `chan::Sender<Command>`.
  No `std::sync::mpsc` in the firmware, and no local aligned to more than
  16 bytes (an Xtensa stack-realign bug: `tools/check-stack-realign.py`
  runs on every build).
- **Never add a `stat` or `exists` in front of an open on the card**: every
  FAT lookup is a linear directory scan under the lock the player waits on.
- **Planned restarts go through `crate::restart(Reason)`**; **panel
  brightness only through `Panel::set_brightness`**.
- Release builds never carry WiFi credentials: `firmware/.env`
  (`firmware/.env.example`) is for bench builds only and never committed.

## Gotchas, by headline

- `gif` delays are hundredths of a second; 0 means 100 ms, otherwise
  floored at 20 ms. Never call the `gif` crate's `read_next_frame` (it
  allocates per frame).
- Centre-and-crop, never resample. A loop rewinds, it does not re-open.
- `filelist.txt` is LF-terminated; category names contain spaces and
  underscores.
- The `ESP32-HUB75-MatrixPanel-I2S-DMA` submodule's directory name is
  load-bearing; `CONFIG_ESP32_HUB75_USE_GFX=n` is required.
- On httpd: never retry a failed send; close second (TIME_WAIT pcbs are heap).

## Content licensing

- The code is MIT OR Apache-2.0. The DMDoS GIF pack is third-party and
  unlicensed: **never commit it**, nor anything made from third-party art
  (contact sheets from card GIFs, ROM screenshots, console logos, title
  cards) or lists of the pack's files. Test fixtures are generated in code;
  example names in docs use made-up author handles.
- The only bundled third-party assets are two fonts, both OFL: Silkscreen
  (the web app and the site) and Fraunces (the site's headings,
  `site/fonts/`). The home page's scenes (`site/theme/den*.webp`) are our own,
  drawn by `tools/site-scene-alt.py` and `tools/site-scene.py`
  (`site/theme/SCENES.md`).
  Third-party notices are generated (`make notices`, checked by
  `fw-release` and CI); after a dependency or IDF bump, regenerate and read
  the diff.
- The site is public: its demo card holds only our own recordings, the CC0
  Kenney sprites and Formulas. Published docs describe how things are, not
  how they got there.
- **One exception, for screenshots only** (owner, 2026-11-09): web app
  screenshots (`docs/images/`, and the site pages that show them) may show
  panel GIFs that Create Art made from libretro-thumbnails **title
  screens**, credited ("Game title screens from libretro-thumbnails, made
  into panel GIFs with Create Art"). `tools/game-art.py` makes them into
  gitignored `target/game-art/` for `make screenshots GAME_ART=1`; the
  screenshots published now show our own Formulas instead, and carry no
  such credit. The GIFs themselves
  are never committed, hosted or put on a published card; no box art or
  logos beyond what a title screen contains; never pack or card GIFs, and
  nothing from GifCities.

## Commits

Commits made with an AI coding assistant end with a `Co-Authored-By:`
trailer naming it.

## Provenance

The crate split, the Makefile's shape and the `gif` compositing approach
come from stripe-dashboard, an earlier Rust ESP32 project by the same author.
