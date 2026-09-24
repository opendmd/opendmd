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
# profile's image had grown to 99.9% of the OTA slot. The bench/release
# distinction is the dev-credentials feature, not the profile. Override with
# `make fw-build FW_PROFILE=dev` when you want debug assertions.
FW_PROFILE ?= release
FW_PROFILE_FLAG := $(if $(filter release,$(FW_PROFILE)),--release,)
FW_BINARY := $(FW)/target/xtensa-esp32-espidf/$(FW_PROFILE)/opendmd-firmware
# The project's own second-stage bootloader, which esp-idf-sys copies next to
# the ELF. It has to be flashed explicitly: without --bootloader, espflash
# writes its *bundled* bootloader, which has CONFIG_BOOTLOADER_APP_ROLLBACK
# compiled out, so an OTA'd image that fails to boot would never be reverted.
FW_BOOTLOADER := $(FW)/target/xtensa-esp32-espidf/$(FW_PROFILE)/bootloader.bin

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
FW_SDKCONFIG := $(CURDIR)/$(FW)/sdkconfig.defaults;$(CURDIR)/$(FW)/target/sdkconfig.partitions

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
	fw-build fw-flash fw-monitor fw-capture fw-erase fw-clean fw-clippy fw-fmt \
	fw-release fw-release-flash ota card-sync api-load

test:
	@echo "==> Testing $(CORE)"
	cd $(CORE) && RUSTUP_TOOLCHAIN=stable cargo test
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

# Needs a real card dump; see CLAUDE.md. ODM_ROOT is tools/odm-proto.py's output
# (the cross-implementation check); the test skips itself if it is missing.
ODM_ROOT ?= ~/scratch/odm-proto
test-card:
	@echo "==> Testing $(CORE) against $(CARD_ROOT) and $(ODM_ROOT)"
	cd $(CORE) && RUSTUP_TOOLCHAIN=stable DMD_CARD_ROOT=$(CARD_ROOT) DMD_ODM_ROOT=$(ODM_ROOT) \
		cargo test --release -- --ignored --nocapture

sim:
	cd $(SIM) && RUSTUP_TOOLCHAIN=stable cargo run --release -- $(ARGS)

sim-build:
	cd $(SIM) && RUSTUP_TOOLCHAIN=stable cargo build --release

# GIF -> ODM conversion for a card: make pack ARGS="convert ~/scratch/dmdos-card --verify"
# or ARGS="manifest <card-root>". See docs/odm-format.md.
pack:
	@test -n "$(PACK_ARGS)" || { echo "usage: make pack ARGS=\"convert <card-root> [--jobs N] [--verify] [--force]\""; exit 2; }
	cd $(PACK) && RUSTUP_TOOLCHAIN=stable cargo run --release -- $(PACK_ARGS)

check:
	@echo "==> Checking $(CORE)"
	cd $(CORE) && RUSTUP_TOOLCHAIN=stable cargo check --all-targets
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
	@echo "==> Checking $(RENDER) for $(WASM_TARGET)"
	cd $(RENDER) && RUSTUP_TOOLCHAIN=stable cargo check --target $(WASM_TARGET)
	@echo "==> Checking $(WEB) for $(WASM_TARGET)"
	cd $(WEB) && RUSTUP_TOOLCHAIN=stable cargo check --target $(WASM_TARGET)
	@echo "==> Checking $(ART) for $(WASM_TARGET)"
	cd $(ART) && RUSTUP_TOOLCHAIN=stable cargo check --target $(WASM_TARGET)

# wasm-pack: `cargo install --locked wasm-pack`. Output is gitignored.
# Then every file the device serves gets a `<file>.gz` beside it: the firmware
# sends `<file>.gz` under /www in preference to `<file>`, unconditionally
# (see "Serving gzip" in CLAUDE.md). wasm-pack's package.json, .d.ts files and
# README are deleted so `make card-sync` does not push them to the card.
# The Studio's own wasm (opendmd-art-web, "Make art") goes to www/pkg-art,
# loaded only when the Studio opens. The budgets (gzipped) are enforced by
# tools/web-size.py. www/version.txt (gitignored) is `git describe` of this
# build; the firmware reports it as `www_version` in /api/status once it is
# on the card.
WEB_WWW := $(WEB)/www
web:
	cd $(WEB) && RUSTUP_TOOLCHAIN=stable wasm-pack build --target web --release --out-dir www/pkg
	cd $(ART) && RUSTUP_TOOLCHAIN=stable wasm-pack build --target web --release --out-dir ../$(WEB_WWW)/pkg-art
	rm -f $(WEB_WWW)/pkg/package.json $(WEB_WWW)/pkg/*.d.ts $(WEB_WWW)/pkg/README.md
	rm -f $(WEB_WWW)/pkg-art/package.json $(WEB_WWW)/pkg-art/*.d.ts $(WEB_WWW)/pkg-art/README.md $(WEB_WWW)/pkg-art/.gitignore
	git describe --always --dirty --tags > $(WEB_WWW)/version.txt
	find $(WEB_WWW) -name '*.gz' -delete
	find $(WEB_WWW) -type f \( -name '*.html' -o -name '*.js' -o -name '*.css' -o -name '*.wasm' -o -name '*.svg' -o -name '*.json' \) \
		-exec gzip -9 -n -k -f {} \;
	@uv run tools/web-size.py $(WEB_WWW)

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
	rm -rf $(WEB)/www/pkg $(WEB)/www/pkg-art ./target

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

fw-build:
	@mkdir -p $(FW)/target
	@echo 'CONFIG_PARTITION_TABLE_CUSTOM_FILENAME="$(CURDIR)/$(FW)/partitions.csv"' > $(FW)/target/sdkconfig.partitions
	cd $(FW) && bash -c '$(FW_ENV) . $(ESPUP_ENV) && RUSTUP_TOOLCHAIN=esp ESP_IDF_SDKCONFIG_DEFAULTS="$(FW_SDKCONFIG)" cargo build $(FW_PROFILE_FLAG) $(FW_CARGO_FEATURES)'
	@ls -l $(FW_BINARY)

# 460800 baud gives frequent verify_failed errors on these CH340 boards.
# otadata is erased so the board boots the ota_0 image just written even if it
# had been OTA'd into ota_1 (espflash always writes the first app slot).
fw-flash: fw-build
	bash -c '. $(ESPUP_ENV) && espflash flash --port $(PORT) --baud 115200 --bootloader $(FW_BOOTLOADER) --partition-table $(FW)/partitions.csv --flash-size 4mb --erase-parts otadata --monitor $(FW_BINARY)'

# NOTE: do NOT capture serial with `cat $(PORT)` or stty tricks — they toggle
# DTR/RTS and the DevKit V1's auto-reset circuit reboots (or holds) the chip,
# and the reads silently fail on permissions. For a logged, non-disturbing
# capture use tools/serial-capture.py (a uv inline-metadata script), which
# holds DTR/RTS inactive and timestamps every line:
#
#   uv run tools/serial-capture.py $(PORT) 120 capture.log [--reset]
#
# `--reset` pulses EN cleanly so the boot banner is in the log. Opening and
# closing the port at all reboots the board, so take one long capture rather
# than several short ones.
fw-monitor:
	bash -c '. $(ESPUP_ENV) && espflash monitor --port $(PORT)'

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
fw-clippy:
	@mkdir -p $(FW)/target
	@echo 'CONFIG_PARTITION_TABLE_CUSTOM_FILENAME="$(CURDIR)/$(FW)/partitions.csv"' > $(FW)/target/sdkconfig.partitions
	@echo "==> Clippy firmware, --features $(FW_DEV_FEATURES)"
	cd $(FW) && bash -c '$(FW_ENV) . $(ESPUP_ENV) && RUSTUP_TOOLCHAIN=esp ESP_IDF_SDKCONFIG_DEFAULTS="$(FW_SDKCONFIG)" cargo clippy --all-targets $(FW_CARGO_FEATURES) -- -D warnings'
	@echo "==> Clippy firmware, release feature set (none)"
	cd $(FW) && env -u OPENDMD_WIFI_SSID -u OPENDMD_WIFI_PASS bash -c '. $(ESPUP_ENV) && RUSTUP_TOOLCHAIN=esp ESP_IDF_SDKCONFIG_DEFAULTS="$(FW_SDKCONFIG)" cargo clippy --all-targets -- -D warnings'

fw-fmt:
	cd $(FW) && RUSTUP_TOOLCHAIN=stable cargo fmt

fw-clean:
	cd $(FW) && bash -c '. $(ESPUP_ENV) && RUSTUP_TOOLCHAIN=esp cargo clean'
	rm -rf $(FW)/target

# --- Release -----------------------------------------------------------------
# `make fw-release` is what goes into the cabinet: a --release build with no
# features (so no firmware/.env credentials — the build does not even source
# it, and the two variables are scrubbed from the environment), saved as an
# OTA image in dist/. The version comes from firmware/Cargo.toml and is also
# written into the app descriptor as "<version> 4mb", which is what
# ota.rs's variant check reads at offset 0x30 of an uploaded image.
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
FW_IMAGE := $(DIST)/opendmd-$(FW_VERSION)-$(FW_VARIANT).bin
# Each ota_N slot in firmware/partitions.csv is 0x1F0000 bytes.
OTA_SLOT_BYTES := 2031616
OTA_SLOT_MAX_PCT := 95

fw-release:
	@test -n "$(FW_VERSION)" || { echo "cannot read the version from $(FW)/Cargo.toml"; exit 2; }
	@mkdir -p $(FW)/target $(DIST)
	@{ echo 'CONFIG_PARTITION_TABLE_CUSTOM_FILENAME="$(CURDIR)/$(FW)/partitions.csv"'; \
	   echo 'CONFIG_APP_PROJECT_VER_FROM_CONFIG=y'; \
	   echo 'CONFIG_APP_PROJECT_VER="$(FW_VERSION) $(FW_VARIANT)"'; } > $(FW_RELEASE_SDKCONFIG_FRAG)
	cd $(FW) && env -u OPENDMD_WIFI_SSID -u OPENDMD_WIFI_PASS bash -c '. $(ESPUP_ENV) && RUSTUP_TOOLCHAIN=esp CARGO_WORKSPACE_DIR="$(CURDIR)/$(FW)" OPENDMD_FLASH_VARIANT=$(FW_VARIANT) ESP_IDF_SDKCONFIG_DEFAULTS="$(FW_RELEASE_SDKCONFIG)" cargo build --release --target-dir "$(FW_RELEASE_TARGET_DIR)"'
	bash -c '. $(ESPUP_ENV) && espflash save-image --chip esp32 --flash-size $(FW_VARIANT) $(FW_RELEASE_BINARY) $(FW_IMAGE)'
	@desc=$$(dd if=$(FW_IMAGE) bs=1 skip=48 count=32 2>/dev/null | tr -d '\000'); \
	 echo "app descriptor version: '$$desc'"; \
	 test "$$desc" = "$(FW_VERSION) $(FW_VARIANT)" || { echo "expected '$(FW_VERSION) $(FW_VARIANT)' at 0x30"; exit 1; }
	@size=$$(stat -c %s $(FW_IMAGE)); \
	 limit=$$(( $(OTA_SLOT_BYTES) * $(OTA_SLOT_MAX_PCT) / 100 )); \
	 pct=$$(awk "BEGIN { printf \"%.1f\", $$size * 100 / $(OTA_SLOT_BYTES) }"); \
	 echo "$(FW_IMAGE): $$size B = $$pct% of the $(OTA_SLOT_BYTES) B OTA slot (limit $(OTA_SLOT_MAX_PCT)% = $$limit B)"; \
	 test $$size -le $$limit || { echo "image exceeds $(OTA_SLOT_MAX_PCT)% of the OTA slot"; exit 1; }

# First install on a fresh board (or one coming from DMDoS/other firmware:
# `make fw-erase` first). Flashes the release ELF — the same build the dist/
# image came from — plus bootloader and partition table, and erases otadata so
# the board boots this image even if it had been OTA'd into ota_1 before.
fw-release-flash: fw-release
	bash -c '. $(ESPUP_ENV) && espflash flash --port $(PORT) --baud 115200 --bootloader $(FW_RELEASE_BOOTLOADER) --partition-table $(FW)/partitions.csv --flash-size $(FW_VARIANT) --erase-parts otadata --monitor $(FW_RELEASE_BINARY)'

# Push an image over the air: make ota HOST=opendmd.local [BIN=dist/...]
# Posts it to /api/ota, prints the reply, then polls /api/status until the
# board is back up (uptime reset) reporting the image's version.
# OTA_DRY_RUN=1 prints the commands instead of running them.
BIN ?= $(FW_IMAGE)
OTA_TIMEOUT ?= 120
ota:
	@test -n "$(HOST)" || { echo "usage: make ota HOST=<ip-or-name> [BIN=$(FW_IMAGE)] [OTA_TIMEOUT=120] [OTA_DRY_RUN=1]"; exit 2; }
	@OTA_DRY_RUN=$(OTA_DRY_RUN) OTA_TIMEOUT=$(OTA_TIMEOUT) tools/ota.sh $(HOST) $(BIN)

# --- Device tools -----------------------------------------------------------
# Both take HOST=<ip-or-hostname>, e.g. HOST=opendmd.local.

# Push opendmd-web/www (index.html, app.js and the wasm-pack output) onto the
# card over the API, which is how the browser app gets there in the first place.
card-sync:
	@test -n "$(HOST)" || { echo "usage: make card-sync HOST=<ip>"; exit 2; }
	uv run tools/card-sync.py $(HOST) $(WEB)/www

# Hammer the file server while watching the display metrics. This is the
# timing stress test for the networking phase, not a benchmark. PATHS=<file>
# fetches a fixed list of card-absolute paths instead of picking GIFs.
SECONDS_ ?= 120
WORKERS ?= 3
api-load:
	@test -n "$(HOST)" || { echo "usage: make api-load HOST=<ip> [SECONDS_=120] [WORKERS=3] [PATHS=file]"; exit 2; }
	uv run tools/api-load.py $(HOST) $(SECONDS_) --workers $(WORKERS) $(if $(PATHS),--paths $(PATHS))
