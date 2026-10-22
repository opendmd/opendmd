# OpenDMD. Host crates (core + simulator) build on any host with stable Rust;
# the ESP32 firmware in firmware/ needs the xtensa toolchain.

CORE   := opendmd-core
RENDER := opendmd-render
SIM    := opendmd-sim
WEB    := opendmd-web
ART    := opendmd-art-web
PACK   := opendmd-pack
FW     := firmware

# Serial port of the DevKit. Override: make fw-flash PORT=/dev/ttyUSB1
PORT ?= /dev/ttyUSB0
# Bench builds use the release *profile* (same opt-level as dev; drops debug
# assertions / overflow checks and uses fewer codegen units), because the dev
# profile's image had grown to 99.9% of the old OTA slot. The bench/release
# distinction is the dev-credentials feature, not the profile. Override with
# `make fw-build FW_PROFILE=dev` when you want debug assertions.
FW_PROFILE ?= release
FW_PROFILE_FLAG := $(if $(filter release,$(FW_PROFILE)),--release,)
FW_BINARY := $(FW)/target/xtensa-esp32-espidf/$(FW_PROFILE)/opendmd-firmware
# The project's own second-stage bootloader, which esp-idf-sys copies next to
# the ELF, flashed explicitly (--bootloader) so the board runs the one built
# with this sdkconfig rather than espflash's bundled one.
FW_BOOTLOADER := $(FW)/target/xtensa-esp32-espidf/$(FW_PROFILE)/bootloader.bin

# The flash layout v2 (firmware/partitions.csv, docs/ota.md): otadata, the
# firmware in factory, the updater in ota_0, the web app's www pack. Keep in
# step with the CSV.
OTADATA_OFFSET := 0xF000
FW_APP_OFFSET := 0x20000
FW_APP_BYTES := 2359296
UPDATER_OFFSET := 0x260000
WWW_OFFSET := 0x2C0000
WWW_BYTES := 1310720
# `make web` writes the pack here (gitignored); fw-flash writes it at WWW_OFFSET.
WWW_PACK := $(WEB)/www.pack

# The xtensa toolchain env is written by `espup install --export-file
# ~/.espup-env.sh`. Sourcing that file beats hardcoding the versioned
# ~/.rustup/toolchains/esp/... paths: those change with every espup release
# and differ between Linux and macOS.
ESPUP_ENV := $$HOME/.espup-env.sh

# ESP-IDF's cmake resolves CONFIG_PARTITION_TABLE_CUSTOM_FILENAME relative to
# its own CMakeLists inside the esp-idf-sys build output dir, not the crate
# root, so a relative path in sdkconfig.defaults silently fails to find the
# table. Generate a sdkconfig fragment with an absolute path and layer it on
# via ESP_IDF_SDKCONFIG_DEFAULTS. (Same workaround as ~/projects/stripe-dashboard.)
#
# A `FW_FEATURES=diag-heap` build (src/diag.rs) also layers on
# firmware/sdkconfig.diag (heap poisoning, the end-of-stack watchpoint); the
# normal and release builds never see it. Switching between the two deletes
# the generated sdkconfig (see `fw-sdkconfig-flavour`), which would otherwise
# keep the other flavour's values for the symbols it already has.
FW_DIAG = $(findstring diag-heap,$(FW_FEATURES))
FW_SDKCONFIG = $(CURDIR)/$(FW)/sdkconfig.defaults;$(CURDIR)/$(FW)/target/sdkconfig.partitions$(if $(FW_DIAG),;$(CURDIR)/$(FW)/sdkconfig.diag)

# Optional bench-rig credentials. `firmware/.env` is gitignored (template in
# firmware/.env.example); when it exists, OPENDMD_WIFI_SSID/PASS are exported
# into the build and seed NVS on a device that has none. Never committed, never
# logged. `set -a` exports every assignment the file makes.
FW_ENV := set -a; [ -f .env ] && . ./.env; set +a;

export PATH := $(HOME)/.cargo/bin:$(PATH)

# Card root for `make sim` without ARGS.
CARD_ROOT ?= ~/scratch/dmdos-card
ARGS ?= --root $(CARD_ROOT)
# `make pack` uses ARGS too, but only when given: the sim's default is no use to it.
PACK_ARGS = $(if $(filter command line environment,$(origin ARGS)),$(ARGS),)

.PHONY: test test-card test-wasm sim sim-build check check-wasm web web-serve web-mock web-smoke fmt clippy clean pack \
	fw-build fw-sdkconfig-flavour fw-flash fw-monitor fw-capture fw-erase fw-clean fw-clippy fw-fmt \
	fw-release fw-release-flash fw-sign fw-ota updater www-push flasher flasher-smoke improv-check card-sync api-load screenshots \
	site site-serve site-publish

test:
	@echo "==> Testing $(CORE) (+ the tween feature: Smooth motion; names: the GIF name builder; ota: firmware bundles)"
	cd $(CORE) && RUSTUP_TOOLCHAIN=stable cargo test --features tween,names,ota
	@echo "==> Testing $(RENDER)"
	cd $(RENDER) && RUSTUP_TOOLCHAIN=stable cargo test
	@echo "==> Testing $(SIM)"
	cd $(SIM) && RUSTUP_TOOLCHAIN=stable cargo test
	@echo "==> Testing $(WEB)"
	cd $(WEB) && RUSTUP_TOOLCHAIN=stable cargo test
	@echo "==> Testing $(ART)"
	cd $(ART) && RUSTUP_TOOLCHAIN=stable cargo test
	@echo "==> Testing $(PACK)"
	cd $(PACK) && RUSTUP_TOOLCHAIN=stable cargo test

# The same tests again, compiled to wasm and run under node — the only check
# on this box that the browser path really works. Needs node on PATH.
test-wasm:
	cd $(WEB) && RUSTUP_TOOLCHAIN=stable wasm-pack test --node
	cd $(ART) && RUSTUP_TOOLCHAIN=stable wasm-pack test --node

# Needs a real card dump; see CLAUDE.md. The prepared-copy test needs the
# card's cache/ (make pack ARGS="convert <card-root> --verify").
test-card:
	@echo "==> Testing $(CORE) against $(CARD_ROOT) (its cache/ from opendmd-pack convert)"
	cd $(CORE) && RUSTUP_TOOLCHAIN=stable DMD_CARD_ROOT=$(CARD_ROOT) \
		cargo test --release --features tween,names -- --ignored --nocapture

sim:
	cd $(SIM) && RUSTUP_TOOLCHAIN=stable cargo run --release -- $(ARGS)

sim-build:
	cd $(SIM) && RUSTUP_TOOLCHAIN=stable cargo build --release

# Prepare a card (every GIF's copy under cache/): make pack ARGS="convert ~/scratch/dmdos-card --verify".
# See docs/cache-format.md.
pack:
	@test -n "$(PACK_ARGS)" || { echo "usage: make pack ARGS=\"convert <card-root> [--jobs N] [--verify] [--force]\""; exit 2; }
	cd $(PACK) && RUSTUP_TOOLCHAIN=stable cargo run --release -- $(PACK_ARGS)

check:
	@echo "==> Checking $(CORE)"
	cd $(CORE) && RUSTUP_TOOLCHAIN=stable cargo check --all-targets --features tween,names,ota
	@echo "==> Checking $(RENDER)"
	cd $(RENDER) && RUSTUP_TOOLCHAIN=stable cargo check --all-targets
	@echo "==> Checking $(SIM)"
	cd $(SIM) && RUSTUP_TOOLCHAIN=stable cargo check --all-targets
	@echo "==> Checking $(PACK)"
	cd $(PACK) && RUSTUP_TOOLCHAIN=stable cargo check --all-targets

# The UI is heading for the browser, so the core and the renderer have to stay
# free of anything wasm can't do — `std::time::Instant` above all, which has no
# clock there. One `rustup target add wasm32-unknown-unknown` is the only setup.
WASM_TARGET := wasm32-unknown-unknown
check-wasm:
	@echo "==> Checking $(CORE) for $(WASM_TARGET)"
	cd $(CORE) && RUSTUP_TOOLCHAIN=stable cargo check --target $(WASM_TARGET)
	cd $(CORE) && RUSTUP_TOOLCHAIN=stable cargo check --target $(WASM_TARGET) --features tween,names
	@echo "==> Checking $(RENDER) for $(WASM_TARGET)"
	cd $(RENDER) && RUSTUP_TOOLCHAIN=stable cargo check --target $(WASM_TARGET)
	@echo "==> Checking $(WEB) for $(WASM_TARGET)"
	cd $(WEB) && RUSTUP_TOOLCHAIN=stable cargo check --target $(WASM_TARGET)
	@echo "==> Checking $(ART) for $(WASM_TARGET)"
	cd $(ART) && RUSTUP_TOOLCHAIN=stable cargo check --target $(WASM_TARGET)

# wasm-pack: `cargo install --locked wasm-pack`. Output is gitignored.
# Then every file the device serves gets a `<file>.gz` beside it, and the
# whole app is packed into $(WWW_PACK) (`opendmd-pack www`, docs/www-pack.md):
# each file stored as its `.gz` where there is one, served from flash with
# `Content-Encoding: gzip`, unconditionally (see "Serving gzip" in
# CLAUDE.md). wasm-pack's package.json, .d.ts files and README are deleted
# so they are not packed. The Studio's own wasm (opendmd-art-web, "Make
# art") goes to www/pkg-art, loaded only when the Studio opens. The budgets
# (gzipped) are enforced by tools/web-size.py; the packer refuses a pack over
# the www partition. www/version.txt (gitignored) is `git describe` of this
# build: the pack's header carries it and the firmware reports it as
# `www_version` in /api/status (the file itself is not packed). The API
# reference page, www/api.html (gitignored), is rendered from docs/api.md on
# every build (tools/api-page.py), after tools/api-doc-check.py has checked
# the doc's routes against firmware/src/http.rs both ways.
WEB_WWW := $(WEB)/www
web:
	cd $(WEB) && RUSTUP_TOOLCHAIN=stable wasm-pack build --target web --release --out-dir www/pkg
	cd $(ART) && RUSTUP_TOOLCHAIN=stable wasm-pack build --target web --release --out-dir ../$(WEB_WWW)/pkg-art
	rm -f $(WEB_WWW)/pkg/package.json $(WEB_WWW)/pkg/*.d.ts $(WEB_WWW)/pkg/README.md
	rm -f $(WEB_WWW)/pkg-art/package.json $(WEB_WWW)/pkg-art/*.d.ts $(WEB_WWW)/pkg-art/README.md $(WEB_WWW)/pkg-art/.gitignore
	git describe --always --dirty --tags > $(WEB_WWW)/version.txt
	uv run tools/api-doc-check.py
	uv run tools/api-page.py docs/api.md $(WEB_WWW)/api.html
	find $(WEB_WWW) -name '*.gz' -delete
	find $(WEB_WWW) -type f \( -name '*.html' -o -name '*.js' -o -name '*.css' -o -name '*.wasm' -o -name '*.svg' -o -name '*.json' \) \
		-exec gzip -9 -n -k -f {} \;
	cd $(PACK) && RUSTUP_TOOLCHAIN=stable cargo run --release --quiet -- \
		www ../$(WEB_WWW) --out ../$(WWW_PACK) --version-file ../$(WEB_WWW)/version.txt --max-bytes $(WWW_BYTES)
	@uv run tools/web-size.py $(WEB_WWW) --pack $(WWW_PACK) --pack-limit $(WWW_BYTES)

# Static server for the page; uv, never bare python. Then open
# http://localhost:8080 — the file/directory picker does the rest.
web-serve:
	uv run python -m http.server 8080 -d $(WEB)/www

# A fake device (JSON API + card files + the built app) for working on the UI
# without the rig: make web-mock ROOT=~/scratch/dmdos-card, then open
# http://localhost:8080. OVERLAY=dir keeps the card copy untouched (writes go
# there); PROXY=http://<device> serves the local build against a real device.
ROOT ?= $(CARD_ROOT)
MOCK_PORT ?= 8080
web-mock: web
	uv run tools/mock-device.py --port $(MOCK_PORT) \
		$(if $(PROXY),--proxy $(PROXY),--root $(ROOT) $(if $(OVERLAY),--overlay $(OVERLAY)))

# Headless Chromium drive of the app against the mock (and, with DEVICE=,
# a read-mostly pass against a real device): DMD_CARD_ROOT=... make web-smoke
web-smoke: web
	uv run tools/web-smoke.py $(if $(DEVICE),--device $(DEVICE))

# The README's and the site's screenshots (docs/images/): the web app in
# headless Chromium against the mock over the demo card (only the panel's
# own effects and the CC0 Kenney sprites: they are published).
screenshots: web sim-build
	uv run tools/demo-card.py target/demo-card
	uv run tools/screenshots.py --card target/demo-card --out docs/images

# The public site (opendmd.github.io): the README, the docs, and the web app
# with a demo card of the panel's own effects and CC0 sprites (tools/site.py,
# tools/demo-card.py, and tools/site-art.py for the site's own animations).
# site-serve to look at it; site-publish pushes it.
SITE ?= target/site
SITE_REPO ?= git@github.com:opendmd/opendmd.github.io.git
# The installer under install/ publishes whatever release dist/ holds; site
# does not build one (fw-release needs the xtensa toolchain).
site: web sim-build
	@test -f $(DIST)/manifest.json || { echo "no $(DIST)/manifest.json: the site publishes the release's installer, run make fw-release first"; exit 2; }
	uv run tools/demo-card.py target/demo-card
	uv run tools/site-art.py target/site-art
	uv run tools/site.py $(SITE) --www $(WEB_WWW) --card target/demo-card --art target/site-art --dist $(DIST)

site-serve: site
	uv run python -m http.server 8081 -d $(SITE)

site-publish: site
	tools/site-publish.sh $(SITE) $(SITE_REPO)

fmt:
	cd $(CORE) && RUSTUP_TOOLCHAIN=stable cargo fmt
	cd $(RENDER) && RUSTUP_TOOLCHAIN=stable cargo fmt
	cd $(SIM) && RUSTUP_TOOLCHAIN=stable cargo fmt
	cd $(WEB) && RUSTUP_TOOLCHAIN=stable cargo fmt
	cd $(ART) && RUSTUP_TOOLCHAIN=stable cargo fmt
	cd $(PACK) && RUSTUP_TOOLCHAIN=stable cargo fmt

clippy:
	@echo "==> Clippy $(CORE)"
	cd $(CORE) && RUSTUP_TOOLCHAIN=stable cargo clippy --all-targets -- -D warnings
	cd $(CORE) && RUSTUP_TOOLCHAIN=stable cargo clippy --all-targets --features tween,names,ota -- -D warnings
	@echo "==> Clippy $(RENDER)"
	cd $(RENDER) && RUSTUP_TOOLCHAIN=stable cargo clippy --all-targets -- -D warnings
	@echo "==> Clippy $(SIM)"
	cd $(SIM) && RUSTUP_TOOLCHAIN=stable cargo clippy --all-targets -- -D warnings
	@echo "==> Clippy $(WEB)"
	cd $(WEB) && RUSTUP_TOOLCHAIN=stable cargo clippy --all-targets --target $(WASM_TARGET) -- -D warnings
	@echo "==> Clippy $(ART)"
	cd $(ART) && RUSTUP_TOOLCHAIN=stable cargo clippy --all-targets -- -D warnings
	cd $(ART) && RUSTUP_TOOLCHAIN=stable cargo clippy --all-targets --target $(WASM_TARGET) -- -D warnings
	@echo "==> Clippy $(PACK)"
	cd $(PACK) && RUSTUP_TOOLCHAIN=stable cargo clippy --all-targets -- -D warnings

clean:
	cd $(CORE) && cargo clean
	cd $(RENDER) && cargo clean
	cd $(SIM) && cargo clean
	cd $(WEB) && cargo clean
	cd $(ART) && cargo clean
	cd $(PACK) && cargo clean
	rm -rf $(WEB)/www/pkg $(WEB)/www/pkg-art $(WWW_PACK) ./target

# --- ESP32 firmware ---------------------------------------------------------
# First build clones ESP-IDF v5.3 into ~/.espressif (shared between projects,
# unlike stripe-dashboard's in-project .embuild) and takes 5-10 minutes.

# FW_FEATURES=probe (or probe,probe-http) builds the timing instrumentation in
# firmware/src/probe.rs; the default build has none of it. The bench targets
# (fw-build, fw-flash) always add `dev-credentials`, which compiles the
# firmware/.env WiFi seed in; fw-release never does.
FW_FEATURES ?=
FW_DEV_FEATURES = dev-credentials$(if $(FW_FEATURES),$(comma)$(FW_FEATURES))
comma := ,
FW_CARGO_FEATURES = --features $(FW_DEV_FEATURES)

# Every build ends with tools/check-stack-realign.py: a function that moves
# its stack pointer without MOVSP (an over-aligned local) corrupts its
# caller's registers now and then (opendmd-core/src/chan.rs). The diag build's
# std-channel torture test keeps one such function on purpose.
#
# The bench builds' sdkconfig fragment: the partition table's absolute path,
# and the app descriptor's version as the release sets it ("<version> 4mb",
# `GET /api/firmware`'s `build`), not IDF's default (`git describe`, or "1").
FW_BENCH_FRAGMENT = { echo 'CONFIG_PARTITION_TABLE_CUSTOM_FILENAME="$(CURDIR)/$(FW)/partitions.csv"'; \
	echo 'CONFIG_APP_PROJECT_VER_FROM_CONFIG=y'; \
	echo 'CONFIG_APP_PROJECT_VER="$(FW_VERSION) $(FW_VARIANT)"'; } > $(FW)/target/sdkconfig.partitions

# Delete the generated sdkconfig when the bench flavour (normal/diag) or the
# version changes: it keeps the values it already has (the descriptor's
# version among them) over a changed fragment.
fw-sdkconfig-flavour:
	@mkdir -p $(FW)/target
	@want="$(if $(FW_DIAG),diag,normal) $(FW_VERSION)"; have=$$(cat $(FW)/target/sdkconfig.flavour 2>/dev/null); \
	 if [ "$$want" != "$$have" ]; then \
	   echo "==> sdkconfig flavour $${have:-unknown} -> $$want: removing the generated sdkconfig"; \
	   rm -f $(FW)/target/xtensa-esp32-espidf/*/build/esp-idf-sys-*/out/sdkconfig; \
	   echo $$want > $(FW)/target/sdkconfig.flavour; \
	 fi

fw-build: fw-sdkconfig-flavour
	@mkdir -p $(FW)/target
	@$(FW_BENCH_FRAGMENT)
	cd $(FW) && bash -c '$(FW_ENV) . $(ESPUP_ENV) && RUSTUP_TOOLCHAIN=esp OPENDMD_FW_VERSION="$(FW_VERSION)" ESP_IDF_SDKCONFIG_DEFAULTS="$(FW_SDKCONFIG)" cargo build $(FW_PROFILE_FLAG) $(FW_CARGO_FEATURES)'
	@ls -l $(FW_BINARY)
	bash -c '. $(ESPUP_ENV) && uv run --quiet tools/check-stack-realign.py $(FW_BINARY) $(if $(FW_DIAG),--allow "mpmc7channel.*9PlayError")'

# The console (the firmware's log) runs at 921600 baud (sdkconfig.defaults,
# CONFIG_ESP_CONSOLE_UART_BAUDRATE): every monitor and capture must match.
# Flashing is a separate rate: 460800 gives frequent verify_failed errors on
# these CH340 boards, so it stays at 115200.
MONITOR_BAUD := 921600

# Bootloader + partition table + app (into factory, otadata erased so it
# boots), then the web app's pack at WWW_OFFSET and the updater at
# UPDATER_OFFSET (`write-bin`s: `flash` takes one app image), then the
# monitor. A board on an older table (v1: one app at 0x10000; or the
# dual-OTA one before it) wants `make fw-erase` first (NVS goes too); see
# docs/deploying.md.
fw-flash: fw-build web updater
	bash -c '. $(ESPUP_ENV) && espflash write-bin --port $(PORT) --baud 115200 $(WWW_OFFSET) $(WWW_PACK) && espflash write-bin --port $(PORT) --baud 115200 $(UPDATER_OFFSET) $(UPDATER_BIN) && espflash flash --port $(PORT) --baud 115200 --bootloader $(FW_BOOTLOADER) --partition-table $(FW)/partitions.csv --erase-parts otadata --flash-size 4mb --monitor --monitor-baud $(MONITOR_BAUD) $(FW_BINARY)'

# NOTE: do NOT capture serial with `cat $(PORT)` or stty tricks — they toggle
# DTR/RTS and the DevKit V1's auto-reset circuit reboots (or holds) the chip,
# and the reads silently fail on permissions. For a logged, non-disturbing
# capture use tools/serial-capture.py (a uv inline-metadata script), which
# holds DTR/RTS inactive and timestamps every line:
#
#   uv run tools/serial-capture.py $(PORT) 120 capture.log [--reset] [--baud=N]
#
# (921600 by default, the console's rate; --baud=115200 for older firmware.)
#
# `--reset` pulses EN cleanly so the boot banner is in the log. Opening and
# closing the port at all reboots the board, so take one long capture rather
# than several short ones.
fw-monitor:
	bash -c '. $(ESPUP_ENV) && espflash monitor --port $(PORT) --monitor-baud $(MONITOR_BAUD)'

# Logged capture that does not disturb the board. SECS=120 by default.
SECS ?= 120
LOG  ?= serial.log
fw-capture:
	uv run tools/serial-capture.py $(PORT) $(SECS) $(LOG) $(CAPTURE_ARGS)

# esptool rather than `espflash erase-flash`, which is unreliable here.
# Installed with `uv tool install esptool` -> ~/.local/bin/esptool.
fw-erase:
	$(HOME)/.local/bin/esptool --chip esp32 --port $(PORT) erase-flash

# Both feature sets: the bench build (dev-credentials + FW_FEATURES) and the
# release build (no features, no .env), so neither cfg branch rots.
fw-clippy: fw-sdkconfig-flavour
	@mkdir -p $(FW)/target
	@$(FW_BENCH_FRAGMENT)
	@echo "==> Clippy firmware, --features $(FW_DEV_FEATURES)"
	cd $(FW) && bash -c '$(FW_ENV) . $(ESPUP_ENV) && RUSTUP_TOOLCHAIN=esp OPENDMD_FW_VERSION="$(FW_VERSION)" ESP_IDF_SDKCONFIG_DEFAULTS="$(FW_SDKCONFIG)" cargo clippy --all-targets $(FW_CARGO_FEATURES) -- -D warnings'
	@echo "==> Clippy firmware, release feature set (none)"
	cd $(FW) && env -u OPENDMD_WIFI_SSID -u OPENDMD_WIFI_PASS bash -c '. $(ESPUP_ENV) && RUSTUP_TOOLCHAIN=esp OPENDMD_FW_VERSION="$(FW_VERSION)" ESP_IDF_SDKCONFIG_DEFAULTS="$(FW_SDKCONFIG)" cargo clippy --all-targets -- -D warnings'

fw-fmt:
	cd $(FW) && RUSTUP_TOOLCHAIN=stable cargo fmt

fw-clean:
	cd $(FW) && bash -c '. $(ESPUP_ENV) && RUSTUP_TOOLCHAIN=esp cargo clean'
	rm -rf $(FW)/target

# --- Release -----------------------------------------------------------------
# `make fw-release` is what goes into the cabinet: a --release build with no
# features (so no firmware/.env credentials — the build does not even source
# it, and the two variables are scrubbed from the environment), plus the web
# app, as a set of files in dist/ for the browser flasher (flasher/, ESP Web
# Tools) or espflash:
#
#   opendmd-<ver>-app.bin          the app image, flashed at 0x20000 (factory)
#   updater.bin                    the updater (ota/updater), at 0x260000 (ota_0)
#   ota_data_initial.bin           otadata erased (8 KB of 0xFF) at 0xF000: boots factory
#   opendmd-<ver>-www.pack         the web app, flashed at 0x2C0000
#   bootloader.bin                 at 0x1000
#   partition-table.bin            at 0x8000
#   opendmd-<ver>-4mb-full.bin     all six merged into one 4 MB image (offset 0)
#   manifest.json                  ESP Web Tools: the six parts and offsets; `name`
#                                  "OpenDMD" and `version` are what the firmware's
#                                  Improv Serial answer says (firmware/src/improv.rs:
#                                  FIRMWARE_NAME, state::firmware_version), which is how
#                                  the installer tells Update from Install; it waits
#                                  `new_install_improv_wait_time` (s) after an install
#                                  for the new firmware to answer, then asks for WiFi
#   opendmd-<ver>.odfw             the signed bundle for PUT /api/firmware (fw-sign)
#
# The version comes from firmware/Cargo.toml and is also written into the app
# descriptor as "<version> 4mb" (checked below at offset 0x30 of the image).
# See docs/deploying.md.
FW_VERSION := $(shell sed -n 's/^version *= *"\(.*\)"/\1/p' $(FW)/Cargo.toml | head -1)
FW_VARIANT := 4mb
# The release build wants ~1.5 GB of its own (host build scripts plus a second
# ESP-IDF build); FW_RELEASE_TARGET_DIR=/somewhere/roomier moves it.
# CARGO_WORKSPACE_DIR is set below because embuild otherwise takes the crate
# root to be the directory above the target dir, and with the target dir
# elsewhere it would lose the extra_components (hub75, mdns) in Cargo.toml.
FW_RELEASE_TARGET_DIR ?= $(CURDIR)/$(FW)/target
FW_RELEASE_BINARY := $(FW_RELEASE_TARGET_DIR)/xtensa-esp32-espidf/release/opendmd-firmware
FW_RELEASE_BOOTLOADER := $(FW_RELEASE_TARGET_DIR)/xtensa-esp32-espidf/release/bootloader.bin
FW_RELEASE_SDKCONFIG_FRAG := $(CURDIR)/$(FW)/target/sdkconfig.release
FW_RELEASE_SDKCONFIG := $(CURDIR)/$(FW)/sdkconfig.defaults;$(FW_RELEASE_SDKCONFIG_FRAG)
DIST := dist
FW_APP_IMAGE := $(DIST)/opendmd-$(FW_VERSION)-app.bin
FW_WWW_IMAGE := $(DIST)/opendmd-$(FW_VERSION)-www.pack
FW_FULL_IMAGE := $(DIST)/opendmd-$(FW_VERSION)-$(FW_VARIANT)-full.bin
# Panic locations name source files: the release names them from neutral
# roots instead of this machine's paths (cargo joins these to the config's
# rustflags).
FW_RELEASE_REMAP := --remap-path-prefix=$(CURDIR)=/opendmd --remap-path-prefix=$(HOME)/.cargo=/cargo --remap-path-prefix=$(HOME)/.rustup=/rustup
# The app image must leave at least this much of the factory partition free.
FW_APP_SPARE := 65536

fw-release: web
	@test -n "$(FW_VERSION)" || { echo "cannot read the version from $(FW)/Cargo.toml"; exit 2; }
	@mkdir -p $(FW)/target $(DIST)
	@{ echo 'CONFIG_PARTITION_TABLE_CUSTOM_FILENAME="$(CURDIR)/$(FW)/partitions.csv"'; \
	   echo 'CONFIG_APP_PROJECT_VER_FROM_CONFIG=y'; \
	   echo 'CONFIG_APP_PROJECT_VER="$(FW_VERSION) $(FW_VARIANT)"'; } > $(FW_RELEASE_SDKCONFIG_FRAG)
	cd $(FW) && env -u OPENDMD_WIFI_SSID -u OPENDMD_WIFI_PASS bash -c '. $(ESPUP_ENV) && RUSTUP_TOOLCHAIN=esp CARGO_WORKSPACE_DIR="$(CURDIR)/$(FW)" OPENDMD_FLASH_VARIANT=$(FW_VARIANT) OPENDMD_FW_VERSION="$(FW_VERSION)" ESP_IDF_SDKCONFIG_DEFAULTS="$(FW_RELEASE_SDKCONFIG)" CARGO_TARGET_XTENSA_ESP32_ESPIDF_RUSTFLAGS="$(FW_RELEASE_REMAP)" cargo build --release --target-dir "$(FW_RELEASE_TARGET_DIR)"'
	@# The updater after the firmware: on a fresh machine (CI) it is the
	@# firmware's build that installs the ESP-IDF checkout idf.py needs.
	$(MAKE) --no-print-directory updater
	bash -c '. $(ESPUP_ENV) && uv run --quiet tools/check-stack-realign.py $(FW_RELEASE_BINARY)'
	bash -c '. $(ESPUP_ENV) && espflash save-image --chip esp32 --flash-size $(FW_VARIANT) $(FW_RELEASE_BINARY) $(FW_APP_IMAGE)'
	@desc=$$(dd if=$(FW_APP_IMAGE) bs=1 skip=48 count=32 2>/dev/null | tr -d '\000'); \
	 echo "app descriptor version: '$$desc'"; \
	 test "$$desc" = "$(FW_VERSION) $(FW_VARIANT)" || { echo "expected '$(FW_VERSION) $(FW_VARIANT)' at 0x30"; exit 1; }
	@size=$$(stat -c %s $(FW_APP_IMAGE)); \
	 spare=$$(( $(FW_APP_BYTES) - size )); \
	 pct=$$(awk "BEGIN { printf \"%.1f\", $$size * 100 / $(FW_APP_BYTES) }"); \
	 echo "$(FW_APP_IMAGE): $$size B = $$pct% of the $(FW_APP_BYTES) B factory partition, $$spare B spare (need $(FW_APP_SPARE))"; \
	 test $$spare -ge $(FW_APP_SPARE) || { echo "the app image leaves less than $(FW_APP_SPARE) B of the factory partition"; exit 1; }
	$(MAKE) --no-print-directory fw-sign
	cp $(WWW_PACK) $(FW_WWW_IMAGE)
	cp $(FW_RELEASE_BOOTLOADER) $(DIST)/bootloader.bin
	cp $(UPDATER_BIN) $(DIST)/updater.bin
	cp $(UPDATER_BUILD)/build/ota_data_initial.bin $(DIST)/ota_data_initial.bin
	@! strings $(FW_APP_IMAGE) $(DIST)/updater.bin $(DIST)/bootloader.bin | grep -q '$(HOME)' || { echo "the release names $(HOME): a build path got in"; exit 1; }
	bash -c '. $(ESPUP_ENV) && espflash partition-table --to-binary -o $(DIST)/partition-table.bin $(FW)/partitions.csv'
	@# The merged image: espflash lays out bootloader, table and app padded to
	@# 4 MB with 0xFF (so otadata is erased: the bootloader boots factory);
	@# the updater and the pack go in at their offsets on top.
	bash -c '. $(ESPUP_ENV) && espflash save-image --chip esp32 --flash-size $(FW_VARIANT) --merge --bootloader $(FW_RELEASE_BOOTLOADER) --partition-table $(FW)/partitions.csv $(FW_RELEASE_BINARY) $(FW_FULL_IMAGE)'
	dd if=$(DIST)/updater.bin of=$(FW_FULL_IMAGE) bs=4096 seek=$$(( $(UPDATER_OFFSET) / 4096 )) conv=notrunc status=none
	dd if=$(FW_WWW_IMAGE) of=$(FW_FULL_IMAGE) bs=4096 seek=$$(( $(WWW_OFFSET) / 4096 )) conv=notrunc status=none
	@test "$$(dd if=$(FW_FULL_IMAGE) bs=4096 skip=$$(( $(FW_APP_OFFSET) / 4096 )) count=1 status=none | head -c 1 | od -An -tx1 | tr -d ' ')" = e9 || { echo "no app image at $(FW_APP_OFFSET) in $(FW_FULL_IMAGE)"; exit 1; }
	@test "$$(dd if=$(FW_FULL_IMAGE) bs=4096 skip=$$(( $(OTADATA_OFFSET) / 4096 )) count=2 status=none | tr -d '\377' | wc -c)" = 0 || { echo "otadata is not erased in $(FW_FULL_IMAGE)"; exit 1; }
	@test $$(stat -c %s $(FW_FULL_IMAGE)) -eq 4194304 || { echo "$(FW_FULL_IMAGE) is not 4 MB"; exit 1; }
	@printf '%s\n' '{' \
	  '  "name": "OpenDMD",' \
	  '  "version": "$(FW_VERSION)",' \
	  '  "new_install_prompt_erase": true,' \
	  '  "new_install_improv_wait_time": 15,' \
	  '  "builds": [' \
	  '    {' \
	  '      "chipFamily": "ESP32",' \
	  '      "parts": [' \
	  '        { "path": "bootloader.bin", "offset": 4096 },' \
	  '        { "path": "partition-table.bin", "offset": 32768 },' \
	  '        { "path": "ota_data_initial.bin", "offset": '$$(( $(OTADATA_OFFSET) ))' },' \
	  '        { "path": "opendmd-$(FW_VERSION)-app.bin", "offset": '$$(( $(FW_APP_OFFSET) ))' },' \
	  '        { "path": "updater.bin", "offset": '$$(( $(UPDATER_OFFSET) ))' },' \
	  '        { "path": "opendmd-$(FW_VERSION)-www.pack", "offset": '$$(( $(WWW_OFFSET) ))' }' \
	  '      ]' \
	  '    }' \
	  '  ]' \
	  '}' > $(DIST)/manifest.json
	@echo "dist/:"; ls -l $(FW_APP_IMAGE) $(FW_WWW_IMAGE) $(FW_BUNDLE) $(DIST)/updater.bin $(DIST)/ota_data_initial.bin $(DIST)/bootloader.bin $(DIST)/partition-table.bin $(FW_FULL_IMAGE) $(DIST)/manifest.json

# The signed bundle for a LAN update (docs/ota.md): the release app image
# behind a 140-byte header signed with the private key, which never enters
# the repo (`uv run tools/fw-sign.py keygen` made it; FW_SIGN_KEY moves it).
# Checked against the committed public key, firmware/keys/opendmd.pub, the
# one the firmware compiles in. Run by fw-release; on its own it signs
# whatever app image dist/ holds for this version.
FW_SIGN_KEY ?= $(HOME)/.config/opendmd/opendmd.key
# The public key the bundle is checked against. Only a dry run (CI, a
# throwaway key) points it elsewhere: a device trusts the committed one alone.
FW_SIGN_PUB ?= $(FW)/keys/opendmd.pub
FW_BUNDLE := $(DIST)/opendmd-$(FW_VERSION).odfw
fw-sign:
	@test -f $(FW_APP_IMAGE) || { echo "no $(FW_APP_IMAGE): run make fw-release first"; exit 2; }
	@test -f $(FW_SIGN_KEY) || { echo "no signing key at $(FW_SIGN_KEY): uv run tools/fw-sign.py keygen (once), or FW_SIGN_KEY=..."; exit 2; }
	uv run --quiet tools/fw-sign.py sign $(FW_APP_IMAGE) --version $(FW_VERSION) --out $(FW_BUNDLE) --key $(FW_SIGN_KEY)
	uv run --quiet tools/fw-sign.py verify $(FW_BUNDLE) --pub $(FW_SIGN_PUB)

# First install on a board over USB, from the release build: the same parts
# as dist/ (bootloader, table, otadata erased, app, updater, pack). From
# DMDoS, other firmware or an older table: `make fw-erase` first.
fw-release-flash: fw-release
	bash -c '. $(ESPUP_ENV) && espflash write-bin --port $(PORT) --baud 115200 $(WWW_OFFSET) $(FW_WWW_IMAGE) && espflash write-bin --port $(PORT) --baud 115200 $(UPDATER_OFFSET) $(DIST)/updater.bin && espflash flash --port $(PORT) --baud 115200 --bootloader $(FW_RELEASE_BOOTLOADER) --partition-table $(FW)/partitions.csv --erase-parts otadata --flash-size $(FW_VARIANT) --monitor --monitor-baud $(MONITOR_BAUD) $(FW_RELEASE_BINARY)'

# The web app over WiFi: PUT the pack to /api/www, then check the version
# the device reports. make www-push HOST=opendmd.local [WWW_PACK=dist/...]
www-push:
	@test -n "$(HOST)" || { echo "usage: make www-push HOST=<ip-or-name> [WWW_PACK=$(WWW_PACK)]"; exit 2; }
	tools/www-push.sh $(HOST) $(WWW_PACK)

# The updater (ota/updater, docs/ota.md): a small C ESP-IDF app for the
# ota_0 partition that copies a staged firmware image from the card into
# factory. Built with idf.py against the same ESP-IDF v5.3 checkout that
# esp-idf-sys uses (~/.espressif, ESP_IDF_TOOLS_INSTALL_DIR = "global").
# Not through export.sh: embuild installed only the tools a build needs, and
# export.sh refuses to run without the debuggers; the environment below is
# what it would set for those. The build dir and sdkconfig live under
# UPDATER_BUILD, outside the tree. Must fit ota_0 (0x60000) with 32 KB spare.
ESPRESSIF := $(HOME)/.espressif
UPDATER_IDF := $(ESPRESSIF)/esp-idf/v5.3
# embuild names the venv after the Python that made it (py3.13 here, py3.12
# on the Ubuntu CI runner).
UPDATER_PYENV := $(firstword $(wildcard $(ESPRESSIF)/python_env/idf5.3_py3*_env) $(ESPRESSIF)/python_env/idf5.3_py3.13_env)
UPDATER_BUILD ?= ~/scratch/opendmd-target/updater
UPDATER_BIN := $(UPDATER_BUILD)/build/opendmd-updater.bin
UPDATER_SLOT := 393216
UPDATER_SPARE := 32768
UPDATER_ENV := IDF_PATH=$(UPDATER_IDF) IDF_PYTHON_ENV_PATH=$(UPDATER_PYENV) \
	ESP_ROM_ELF_DIR=$(ESPRESSIF)/tools/esp-rom-elfs/20240305/ \
	PATH=$(UPDATER_PYENV)/bin:$(ESPRESSIF)/tools/xtensa-esp-elf/esp-13.2.0_20240530/xtensa-esp-elf/bin:$(ESPRESSIF)/tools/cmake/3.24.0/bin:$(ESPRESSIF)/tools/ninja/1.11.1:$(UPDATER_IDF)/tools:$$PATH
updater:
	@mkdir -p $(UPDATER_BUILD)
	@# idf.py only reads sdkconfig.defaults into a new sdkconfig.
	@if [ ota/updater/sdkconfig.defaults -nt $(UPDATER_BUILD)/sdkconfig ]; then rm -f $(UPDATER_BUILD)/sdkconfig; fi
	cd ota/updater && env $(UPDATER_ENV) python $(UPDATER_IDF)/tools/idf.py \
		-B $(UPDATER_BUILD)/build -DSDKCONFIG=$(UPDATER_BUILD)/sdkconfig build
	@size=$$(stat -c %s $(UPDATER_BIN)); \
	 echo "$(UPDATER_BIN): $$size B of the $(UPDATER_SLOT) B ota_0 slot, $$(( $(UPDATER_SLOT) - size )) B spare (need $(UPDATER_SPARE))"; \
	 test $$(( size + $(UPDATER_SPARE) )) -le $(UPDATER_SLOT) || { echo "the updater does not fit ota_0 with $(UPDATER_SPARE) B spare"; exit 1; }

# A firmware update over WiFi (docs/ota.md): PUT the signed bundle to
# /api/firmware, then wait for the device to come back running it.
# make fw-ota HOST=opendmd.local [BUNDLE=dist/opendmd-<v>.odfw]; with an API
# key set on the device, OPENDMD_API_KEY=<key> in the environment. A bundle
# that is not newer than the device's firmware is refused here unless
# OPENDMD_ALLOW_OLDER=1 (pass the FW_VERSION you released, or BUNDLE=).
BUNDLE ?= $(FW_BUNDLE)
fw-ota:
	@test -n "$(HOST)" || { echo "usage: make fw-ota HOST=<ip-or-name> [BUNDLE=$(FW_BUNDLE)]"; exit 2; }
	tools/fw-ota.sh $(HOST) $(BUNDLE)

# The browser flasher (flasher/index.html, ESP Web Tools) with the dist/ set
# from `make fw-release`, on http://localhost:8765: Web Serial needs a secure
# context, and localhost is one. Chrome or Edge.
FLASHER_PORT ?= 8765
FLASHER_SITE := target/flasher-site
flasher:
	@test -f $(DIST)/manifest.json || { echo "no $(DIST)/manifest.json: run make fw-release first"; exit 2; }
	rm -rf $(FLASHER_SITE) && mkdir -p $(FLASHER_SITE)/firmware
	cp flasher/index.html $(FLASHER_SITE)/
	cp $(DIST)/manifest.json $(DIST)/bootloader.bin $(DIST)/partition-table.bin $(DIST)/updater.bin $(DIST)/ota_data_initial.bin $(FLASHER_SITE)/firmware/
	cp $(FW_APP_IMAGE) $(FW_WWW_IMAGE) $(FW_FULL_IMAGE) $(FLASHER_SITE)/firmware/
	@echo "open http://localhost:$(FLASHER_PORT)/ in Chrome or Edge"
	uv run python -m http.server $(FLASHER_PORT) --bind 127.0.0.1 -d $(FLASHER_SITE)

# Headless Chromium: the flasher page loads, ESP Web Tools renders its
# button, the manifest's parts are served (the flash itself needs a person).
flasher-smoke:
	uv run tools/flasher-smoke.py

# The real Improv Serial host library (improv-wifi-serial-sdk 2.8.0, what ESP
# Web Tools 10.4.0 runs) against opendmd-core's framing, under node: the
# protocol half of what the installer does with a board (the UART half needs
# a person at a real one). Fetches the SDK with npm once, into target/.
improv-check:
	tools/improv-sdk-check.sh

# --- Device tools -----------------------------------------------------------
# Both take HOST=<ip-or-hostname>, e.g. HOST=opendmd.local.

# Push files from a local copy of the card onto the device's card over the
# API: PATHS is a list of card-absolute paths (a manifest will do), each read
# from the same path under FROM — how `opendmd-pack convert`'s prepared copies
# get onto a card without taking it out. (The web app is not on the card: it
# is in flash, `make www-push`.)
card-sync:
	@test -n "$(HOST)" -a -n "$(FROM)" -a -n "$(PATHS)" || { echo "usage: make card-sync HOST=<ip> FROM=<card copy> PATHS=<list of card paths>"; exit 2; }
	uv run tools/card-sync.py $(HOST) --from $(FROM) --paths $(PATHS)

# Hammer the file server while watching the display metrics. This is the
# timing stress test for the networking phase, not a benchmark. PATHS=<file>
# fetches a fixed list of card-absolute paths instead of picking GIFs.
SECONDS_ ?= 120
WORKERS ?= 3
api-load:
	@test -n "$(HOST)" || { echo "usage: make api-load HOST=<ip> [SECONDS_=120] [WORKERS=3] [PATHS=file]"; exit 2; }
	uv run tools/api-load.py $(HOST) $(SECONDS_) --workers $(WORKERS) $(if $(PATHS),--paths $(PATHS))
