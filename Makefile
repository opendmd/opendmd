# OpenDMD. Host crates (core + simulator) build on any host; the ESP32 firmware
# will live in firmware/ with its own .cargo/config.toml and Makefile targets.

CORE := opendmd-core
SIM  := opendmd-sim

export PATH := $(HOME)/.cargo/bin:$(PATH)

# Card root for `make sim` without ARGS.
CARD_ROOT ?= ~/scratch/dmdos-card
ARGS ?= --root $(CARD_ROOT)

.PHONY: test test-card sim sim-build check fmt clippy clean

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
