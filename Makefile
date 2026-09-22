# OpenDMD. Host crates (core + simulator) build on any host with stable Rust;
# the ESP32 firmware in firmware/ needs the xtensa toolchain.

CORE   := opendmd-core
RENDER := opendmd-render
SIM    := opendmd-sim
WEB    := opendmd-web
FW     := firmware

# Serial port of the DevKit. Override: make fw-flash PORT=/dev/ttyUSB1
PORT ?= /dev/ttyUSB0
FW_BINARY := $(FW)/target/xtensa-esp32-espidf/debug/opendmd-firmware

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

.PHONY: test test-card test-wasm sim sim-build check check-wasm web web-serve web-smoke fmt clippy clean \
	fw-build fw-flash fw-monitor fw-capture fw-erase fw-clean fw-clippy fw-fmt \
	card-sync api-load

test:
	@echo "==> Testing $(CORE)"
	cd $(CORE) && RUSTUP_TOOLCHAIN=stable cargo test
	@echo "==> Testing $(RENDER)"
	cd $(RENDER) && RUSTUP_TOOLCHAIN=stable cargo test
	@echo "==> Testing $(SIM)"
	cd $(SIM) && RUSTUP_TOOLCHAIN=stable cargo test
	@echo "==> Testing $(WEB)"
	cd $(WEB) && RUSTUP_TOOLCHAIN=stable cargo test

# The same tests again, compiled to wasm and run under node — the only check
# on this box that the browser path really works. Needs node on PATH.
test-wasm:
	cd $(WEB) && RUSTUP_TOOLCHAIN=stable wasm-pack test --node

# Needs a real card dump; see CLAUDE.md.
test-card:
	@echo "==> Testing $(CORE) against $(CARD_ROOT)"
	cd $(CORE) && RUSTUP_TOOLCHAIN=stable DMD_CARD_ROOT=$(CARD_ROOT) \
		cargo test --release -- --ignored --nocapture

sim:
	cd $(SIM) && RUSTUP_TOOLCHAIN=stable cargo run --release -- $(ARGS)

sim-build:
	cd $(SIM) && RUSTUP_TOOLCHAIN=stable cargo build --release

check:
	@echo "==> Checking $(CORE)"
	cd $(CORE) && RUSTUP_TOOLCHAIN=stable cargo check --all-targets
	@echo "==> Checking $(RENDER)"
	cd $(RENDER) && RUSTUP_TOOLCHAIN=stable cargo check --all-targets
	@echo "==> Checking $(SIM)"
	cd $(SIM) && RUSTUP_TOOLCHAIN=stable cargo check --all-targets

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

# wasm-pack: `cargo install --locked wasm-pack`. Output is gitignored.
web:
	cd $(WEB) && RUSTUP_TOOLCHAIN=stable wasm-pack build --target web --release --out-dir www/pkg
	@ls -l $(WEB)/www/pkg/*.wasm

# Static server for the page; uv, never bare python. Then open
# http://localhost:8080 — the file/directory picker does the rest.
web-serve:
	uv run python -m http.server 8080 -d $(WEB)/www

# Headless Chromium drive of the page: DMD_CARD_ROOT=... make web-smoke
web-smoke: web
	uv run tools/web-smoke.py

fmt:
	cd $(CORE) && RUSTUP_TOOLCHAIN=stable cargo fmt
	cd $(RENDER) && RUSTUP_TOOLCHAIN=stable cargo fmt
	cd $(SIM) && RUSTUP_TOOLCHAIN=stable cargo fmt
	cd $(WEB) && RUSTUP_TOOLCHAIN=stable cargo fmt

clippy:
	@echo "==> Clippy $(CORE)"
	cd $(CORE) && RUSTUP_TOOLCHAIN=stable cargo clippy --all-targets -- -D warnings
	@echo "==> Clippy $(RENDER)"
	cd $(RENDER) && RUSTUP_TOOLCHAIN=stable cargo clippy --all-targets -- -D warnings
	@echo "==> Clippy $(SIM)"
	cd $(SIM) && RUSTUP_TOOLCHAIN=stable cargo clippy --all-targets -- -D warnings
	@echo "==> Clippy $(WEB)"
	cd $(WEB) && RUSTUP_TOOLCHAIN=stable cargo clippy --all-targets --target $(WASM_TARGET) -- -D warnings

clean:
	cd $(CORE) && cargo clean
	cd $(RENDER) && cargo clean
	cd $(SIM) && cargo clean
	cd $(WEB) && cargo clean
	rm -rf $(WEB)/www/pkg ./target

# --- ESP32 firmware ---------------------------------------------------------
# First build clones ESP-IDF v5.3 into ~/.espressif (shared between projects,
# unlike stripe-dashboard's in-project .embuild) and takes 5-10 minutes.

fw-build:
	@mkdir -p $(FW)/target
	@echo 'CONFIG_PARTITION_TABLE_CUSTOM_FILENAME="$(CURDIR)/$(FW)/partitions.csv"' > $(FW)/target/sdkconfig.partitions
	cd $(FW) && bash -c '$(FW_ENV) . $(ESPUP_ENV) && RUSTUP_TOOLCHAIN=esp ESP_IDF_SDKCONFIG_DEFAULTS="$(FW_SDKCONFIG)" cargo build'
	@ls -l $(FW_BINARY)

# 460800 baud gives frequent verify_failed errors on these CH340 boards.
fw-flash: fw-build
	bash -c '. $(ESPUP_ENV) && espflash flash --port $(PORT) --baud 115200 --partition-table $(FW)/partitions.csv --flash-size 4mb --monitor $(FW_BINARY)'

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

fw-clippy:
	cd $(FW) && bash -c '$(FW_ENV) . $(ESPUP_ENV) && RUSTUP_TOOLCHAIN=esp ESP_IDF_SDKCONFIG_DEFAULTS="$(FW_SDKCONFIG)" cargo clippy --all-targets -- -D warnings'

fw-fmt:
	cd $(FW) && RUSTUP_TOOLCHAIN=stable cargo fmt

fw-clean:
	cd $(FW) && bash -c '. $(ESPUP_ENV) && RUSTUP_TOOLCHAIN=esp cargo clean'
	rm -rf $(FW)/target

# --- Device tools -----------------------------------------------------------
# Both take HOST=<ip-or-hostname>, e.g. HOST=opendmd.local.

# Push opendmd-web/www (index.html, app.js and the wasm-pack output) onto the
# card over the API, which is how the browser app gets there in the first place.
card-sync:
	@test -n "$(HOST)" || { echo "usage: make card-sync HOST=<ip>"; exit 2; }
	uv run tools/card-sync.py $(HOST) $(WEB)/www

# Hammer the file server while watching the display metrics. This is the
# timing stress test for the networking phase, not a benchmark.
SECONDS_ ?= 120
WORKERS ?= 3
api-load:
	@test -n "$(HOST)" || { echo "usage: make api-load HOST=<ip> [SECONDS_=120] [WORKERS=3]"; exit 2; }
	uv run tools/api-load.py $(HOST) $(SECONDS_) --workers $(WORKERS)
