# OpenDMD. Host crates (core + simulator) build on any host with stable Rust;
# the ESP32 firmware in firmware/ needs the xtensa toolchain.

CORE := opendmd-core
SIM  := opendmd-sim
FW   := firmware

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

export PATH := $(HOME)/.cargo/bin:$(PATH)

# Card root for `make sim` without ARGS.
CARD_ROOT ?= ~/scratch/dmdos-card
ARGS ?= --root $(CARD_ROOT)

.PHONY: test test-card sim sim-build check fmt clippy clean \
	fw-build fw-flash fw-monitor fw-capture fw-erase fw-clean fw-clippy fw-fmt

test:
	@echo "==> Testing $(CORE)"
	cd $(CORE) && RUSTUP_TOOLCHAIN=stable cargo test

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
	@echo "==> Checking $(SIM)"
	cd $(SIM) && RUSTUP_TOOLCHAIN=stable cargo check --all-targets

fmt:
	cd $(CORE) && RUSTUP_TOOLCHAIN=stable cargo fmt
	cd $(SIM) && RUSTUP_TOOLCHAIN=stable cargo fmt

clippy:
	@echo "==> Clippy $(CORE)"
	cd $(CORE) && RUSTUP_TOOLCHAIN=stable cargo clippy --all-targets -- -D warnings
	@echo "==> Clippy $(SIM)"
	cd $(SIM) && RUSTUP_TOOLCHAIN=stable cargo clippy --all-targets -- -D warnings

clean:
	cd $(CORE) && cargo clean
	cd $(SIM) && cargo clean
	rm -rf ./target

# --- ESP32 firmware ---------------------------------------------------------
# First build clones ESP-IDF v5.3 into ~/.espressif (shared between projects,
# unlike stripe-dashboard's in-project .embuild) and takes 5-10 minutes.

fw-build:
	@mkdir -p $(FW)/target
	@echo 'CONFIG_PARTITION_TABLE_CUSTOM_FILENAME="$(CURDIR)/$(FW)/partitions.csv"' > $(FW)/target/sdkconfig.partitions
	cd $(FW) && bash -c '. $(ESPUP_ENV) && RUSTUP_TOOLCHAIN=esp ESP_IDF_SDKCONFIG_DEFAULTS="$(FW_SDKCONFIG)" cargo build'
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
	cd $(FW) && bash -c '. $(ESPUP_ENV) && RUSTUP_TOOLCHAIN=esp ESP_IDF_SDKCONFIG_DEFAULTS="$(FW_SDKCONFIG)" cargo clippy --all-targets -- -D warnings'

fw-fmt:
	cd $(FW) && RUSTUP_TOOLCHAIN=stable cargo fmt

fw-clean:
	cd $(FW) && bash -c '. $(ESPUP_ENV) && RUSTUP_TOOLCHAIN=esp cargo clean'
	rm -rf $(FW)/target
