# OpenDMD. Host crates (core + simulator) build on any host with stable Rust;
# the ESP32 firmware in firmware/ needs the xtensa toolchain.

CORE   := opendmd-core
RENDER := opendmd-render
SIM    := opendmd-sim
WEB    := opendmd-web
ART    := opendmd-art-web
STUDIO := opendmd-studio-web
PACK   := opendmd-pack
PSI    := opendmd-psi
PSIWEB := opendmd-psi-web
PSICLI := opendmd-psi-cli
GAL    := opendmd-gallery
WEBMIN := opendmd-webmin
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
FW_BINARY = $(FW)/target/$(FW_TRIPLE)/$(FW_PROFILE)/opendmd-firmware
# The project's own second-stage bootloader, which esp-idf-sys copies next to
# the ELF, flashed explicitly (--bootloader) so the board runs the one built
# with this sdkconfig rather than espflash's bundled one.
FW_BOOTLOADER = $(FW)/target/$(FW_TRIPLE)/$(FW_PROFILE)/bootloader.bin

# The flash layout (docs/ota.md): `4mb`, table v4, for the ESP32 DevKit V1
# (firmware/partitions.csv): otadata, the firmware in factory, the updater in
# ota_0, the web app's www pack. The one layout this tree builds; the
# variant name stays in file names, the app descriptor and /api/status.
# Keep in step with the CSV.
FW_VARIANTS := 4mb
VARIANT ?=
FW_VARIANT := $(or $(VARIANT),4mb)
# EMULATOR ONLY, never released: `emu8mb` (firmware/partitions-emu.csv), the
# psi-probe image's layout for esp32sim, an 8 MB flash with a 3 MB factory
# (the probe outgrew the 4mb one). Not in FW_VARIANTS, so fw-release never
# builds it; only the emu-image targets' fw-release-variant (and the
# updater and notices-check it runs) take it, into $(EMU_DIR), never dist/;
# it gets no signed bundle (fw-sign.py does not know it) and no manifest.
FW_EMU_VARIANT := emu8mb
FW_S3_VARIANT := s3
ifeq ($(FW_VARIANT),$(FW_EMU_VARIANT))
ifneq ($(or $(filter-out fw-release-variant updater notices-check,$(MAKECMDGOALS)),$(if $(MAKECMDGOALS),,default)),)
$(error VARIANT=$(FW_EMU_VARIANT) is the emulator-only layout: make emu-probe builds it)
endif
ifeq ($(abspath $(or $(DIST),dist)),$(abspath dist))
$(error VARIANT=$(FW_EMU_VARIANT) is the emulator-only layout: never into dist/)
endif
else ifeq ($(FW_VARIANT),$(FW_S3_VARIANT))
ifneq ($(filter fw-flash fw-release-flash fw-erase fw-ota fw-sign fw-release flasher,$(MAKECMDGOALS)),)
$(error VARIANT=$(FW_S3_VARIANT) is the ESP32-S3 preview: built and run in the emulator only (make fw-build, emu-s3-nocard, emu-s3-probe); no install, update or release yet)
endif
ifeq ($(abspath $(or $(DIST),dist)),$(abspath dist))
ifneq ($(filter fw-release-variant,$(MAKECMDGOALS)),)
$(error VARIANT=$(FW_S3_VARIANT) is the ESP32-S3 preview, not released: give it a DIST of its own, never dist/)
endif
endif
else ifeq ($(filter $(FW_VARIANT),$(FW_VARIANTS)),)
$(error VARIANT=$(VARIANT): the flash layouts are $(FW_VARIANTS) (and $(FW_S3_VARIANT), the ESP32-S3 preview))
endif
# The ESP32-S3 PREVIEW, not released (S3 port phase 0): `s3`
# (firmware/partitions-s3.csv), an 8 MB layout for the ESP32-S3-DevKitC-1
# N8R8 and N16R8 (one image for both), the chip's own sdkconfig
# (firmware/sdkconfig.s3), target xtensa-esp32s3-espidf. Not in
# FW_VARIANTS, so fw-release never builds it; `make fw-build VARIANT=s3`
# and the emu-s3-* lanes do. fw-release-variant builds it into a DIST of its
# own with no updater, signed bundle, notices check or manifest (the release
# phase's work), and nothing flashes, updates or releases it.
# Variants that never reach dist/: no signed bundle or manifest.
FW_UNRELEASED := $(filter $(FW_EMU_VARIANT) $(FW_S3_VARIANT),$(FW_VARIANT))
# No updater (ota/updater is built for the classic ESP32): the S3 preview.
FW_NO_UPDATER := $(filter $(FW_S3_VARIANT),$(FW_VARIANT))
# The chip: espflash's --chip, esp-idf-sys's MCU, the Rust target.
FW_CHIP_4mb := esp32
FW_CHIP_emu8mb := esp32
FW_CHIP_s3 := esp32s3
FW_CHIP := $(FW_CHIP_$(FW_VARIANT))
FW_TRIPLE := xtensa-$(FW_CHIP)-espidf
# Its name in cargo's CARGO_TARGET_<triple>_RUSTFLAGS.
FW_TRIPLE_ENV := $(subst -,_,$(shell echo $(FW_TRIPLE) | tr a-z A-Z))
# The classic's builds name neither (firmware/.cargo/config.toml's defaults);
# the S3's pass both.
FW_TARGET_FLAG := $(if $(filter esp32,$(FW_CHIP)),, --target $(FW_TRIPLE))
FW_MCU_ENV := $(if $(filter esp32,$(FW_CHIP)),,MCU=$(FW_CHIP) )
# The chip's own sdkconfig, layered after sdkconfig.defaults (the S3's only).
FW_CHIP_SDKCONFIG := $(if $(filter esp32,$(FW_CHIP)),,$(CURDIR)/$(FW)/sdkconfig.s3)
FW_PARTITIONS_4mb := $(FW)/partitions.csv
FW_PARTITIONS_emu8mb := $(FW)/partitions-emu.csv
FW_PARTITIONS_s3 := $(FW)/partitions-s3.csv
# The flash size espflash, the sdkconfig and the image headers say.
FW_FLASH_SIZE_4mb := 4mb
FW_FLASH_SIZE_emu8mb := 8mb
FW_FLASH_SIZE_s3 := 8mb
FW_FLASH_SIZE := $(FW_FLASH_SIZE_$(FW_VARIANT))
FW_FLASH_BYTES_4mb := 4194304
FW_FLASH_BYTES_emu8mb := 8388608
FW_FLASH_BYTES_s3 := 8388608
FW_APP_BYTES_4mb := 2883584
FW_APP_BYTES_emu8mb := 3145728
FW_APP_BYTES_s3 := 3670016
UPDATER_OFFSET_4mb := 0x2E0000
UPDATER_OFFSET_emu8mb := 0x320000
UPDATER_OFFSET_s3 := 0x3A0000
WWW_OFFSET_4mb := 0x340000
WWW_OFFSET_emu8mb := 0x380000
WWW_OFFSET_s3 := 0x400000
WWW_BYTES_4mb := 786432
WWW_BYTES_emu8mb := 786432
WWW_BYTES_s3 := 1048576
# What ESP Web Tools' manifest calls the firmware: it must equal the Improv
# answer's name (firmware/src/improv.rs FIRMWARE_NAME).
FW_MANIFEST_NAME_4mb := OpenDMD
FW_PARTITIONS := $(FW_PARTITIONS_$(FW_VARIANT))
FW_FLASH_BYTES := $(FW_FLASH_BYTES_$(FW_VARIANT))
OTADATA_OFFSET := 0xF000
FW_APP_OFFSET := 0x20000
FW_APP_BYTES := $(FW_APP_BYTES_$(FW_VARIANT))
UPDATER_OFFSET := $(UPDATER_OFFSET_$(FW_VARIANT))
WWW_OFFSET := $(WWW_OFFSET_$(FW_VARIANT))
WWW_BYTES := $(WWW_BYTES_$(FW_VARIANT))
# The www pack: built to fit the www partition, and (tools/web-size.py)
# signed it must leave WWW_PACK_SPARE of it, the web app's headroom.
WWW_PACK_BYTES := $(WWW_BYTES_4mb)
WWW_PACK_SPARE := 65536
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
FW_SDKCONFIG = $(CURDIR)/$(FW)/sdkconfig.defaults;$(if $(FW_CHIP_SDKCONFIG),$(FW_CHIP_SDKCONFIG);)$(CURDIR)/$(FW)/target/sdkconfig.partitions$(if $(FW_DIAG),;$(CURDIR)/$(FW)/sdkconfig.diag)

# Optional bench WiFi credentials. `firmware/.env` is gitignored (template in
# firmware/.env.example); when it exists, OPENDMD_WIFI_SSID/PASS are exported
# into the build and seed NVS on a device that has none. Never committed, never
# logged. `set -a` exports every assignment the file makes.
FW_ENV := set -a; [ -f .env ] && . ./.env; set +a;

export PATH := $(HOME)/.cargo/bin:$(PATH)

# This machine's own settings (gitignored, optional): a local.mk beside this
# file is read first, so it can set anything below that is assigned with ?=.
# SCRATCH, when set (e.g. `export SCRATCH = /data/scratch`), moves the big
# build outputs that otherwise go under target/ (the updater's idf.py build,
# the emulator, tools/release.sh's builds) to a directory outside the tree;
# CARD_ROOT is a copy of a card. Command-line settings still win.
-include local.mk
SCRATCH ?=

# Card root (a copy of a card) for `make sim` without ARGS, `make test-card`,
# `make web-mock` and `make web-smoke`.
CARD_ROOT ?= $(CURDIR)/card
ARGS ?= --root $(CARD_ROOT)
# `make pack` uses ARGS too, but only when given: the sim's default is no use to it.
PACK_ARGS = $(if $(filter command line environment,$(origin ARGS)),$(ARGS),)

.PHONY: blocklist-check test test-card test-wasm sim sim-build psi-bench psi-check psi check check-wasm web webmin font site-font site-scene logo web-serve web-mock web-smoke fmt clippy clean pack \
	notices notices-check notices-check-web emu-build emu-card emu-smoke emu-probe emu-image-smoke emu-image-probe emu-image-paint emu-paint-probe emu-rig emu-rig-card emu-rig-tight emu-image-rigtight emu-leak emu-image-leak emu-api emu-api-psb \
	emu-image-s3 emu-image-s3-nocard emu-image-s3-probe emu-s3-nocard emu-s3-probe \
	fw-build fw-sdkconfig-flavour fw-flash fw-monitor fw-capture fw-erase fw-clean fw-clippy fw-fmt \
	fw-release fw-release-variant fw-release-flash fw-sign fw-ota ota-compat updater updater-picture emu-old-updater emu-offline emu-offline-psb www-push flasher flasher-smoke improv-check card-sync api-load screenshots \
	site site-serve gallery-repo-publish site-smoke gallery-tool \
	site-worker-deps site-dev site-worker-test site-dev-smoke site-deploy-check site-deploy site-deploy-built site-d1-migrate site-stats release studio-starter

# The Studio starter pack (studio-starter/README.md): check every record in
# studio-starter/src/ and write studio-starter/starter.dmdpack.
# make studio-starter ARGS="--sheets <dir>" also draws the contact sheet;
# ARGS=--check only verifies that the committed pack is current.
studio-starter:
	uv run studio-starter/build.py $(ARGS)

# No request URL the web app or the site makes may match EasyPrivacy or EasyList
# (uBlock Origin / Brave blocked /api/stat?). Fetches the lists once a day into
# target/blocklists/ and works offline from that cache (BLOCKLIST_ARGS=--offline);
# not part of `make test`, which needs no network.
blocklist-check:
	uv run tools/blocklist-check.py $(BLOCKLIST_ARGS)

test:
	@echo "==> Testing $(CORE) (+ the tween feature: Smooth motion; names: the GIF name builder; ota: firmware bundles; tags: tags, ratings and the search)"
	cd $(CORE) && RUSTUP_TOOLCHAIN=stable cargo test --features tween,names,ota,tags
	@echo "==> Testing the updater's panel timing parser on the host (ota/updater/test, the vectors from_nvs is held to)"
	@mkdir -p target
	$${CC:-cc} -std=c11 -Wall -Wextra -Werror -Iota/updater/main -o target/updater-panel-timing-test \
	  ota/updater/test/panel_timing_test.c ota/updater/main/panel_timing.c
	target/updater-panel-timing-test ota/updater/test/panel_timing_vectors.txt
	@echo "==> Testing tools/fw-sign.py (the bundle signer: layouts, tampering, www packs)"
	uv run --quiet tools/fw-sign.py selftest
	@echo "==> Testing $(CORE) as a firmware without Formulas builds it (player: a playlist's Formula lines skipped)"
	cd $(CORE) && RUSTUP_TOOLCHAIN=stable cargo test --features player,ota --lib a_player_without_formulas
	@echo "==> Testing $(RENDER)"
	cd $(RENDER) && RUSTUP_TOOLCHAIN=stable cargo test
	@echo "==> Testing $(SIM)"
	cd $(SIM) && RUSTUP_TOOLCHAIN=stable cargo test
	@echo "==> Testing $(WEB)"
	cd $(WEB) && RUSTUP_TOOLCHAIN=stable cargo test
	@echo "==> Testing $(ART)"
	cd $(ART) && RUSTUP_TOOLCHAIN=stable cargo test
	@echo "==> Testing $(STUDIO) (the site's Studio: its GIF writers)"
	cd $(STUDIO) && RUSTUP_TOOLCHAIN=stable cargo test
	@echo "==> Testing $(PACK)"
	cd $(PACK) && RUSTUP_TOOLCHAIN=stable cargo test
	@echo "==> Testing $(PSI) (Psi formulas; + core's and the simulator's psi feature)"
	cd $(PSI) && RUSTUP_TOOLCHAIN=stable cargo test --features docs,syntax
	cd $(CORE) && RUSTUP_TOOLCHAIN=stable cargo test --features psi
	cd $(SIM) && RUSTUP_TOOLCHAIN=stable cargo test --features psi
	@echo "==> Testing $(PSIWEB) (the web app's Formulas module)"
	cd $(PSIWEB) && RUSTUP_TOOLCHAIN=stable cargo test
	cd $(PSIWEB) && RUSTUP_TOOLCHAIN=stable cargo test --features lang lang
	@echo "==> Testing $(PSICLI) (the psi command)"
	cd $(PSICLI) && RUSTUP_TOOLCHAIN=stable cargo test
	@# The bundled Formulas (and the rest of the gallery's examples) stay as psi fmt lays them out: it names any that don't.
	cd $(PSICLI) && RUSTUP_TOOLCHAIN=stable cargo run --release --quiet -- fmt --check $(abspath $(wildcard $(WEB)/formulas/*.psi))
	@echo "==> Testing $(GAL) (the formula gallery's checker)"
	cd $(GAL) && RUSTUP_TOOLCHAIN=stable cargo test
	@echo "==> Testing $(WEBMIN) (make web's minifier)"
	cd $(WEBMIN) && RUSTUP_TOOLCHAIN=stable cargo test
	tools/gallery-repo-publish-test.sh
	uv run tools/gallery-check-test.py
	uv run tools/gallery-pages-test.py

# The same tests again, compiled to wasm and run under node — the only check
# on this box that the browser path really works. Needs node on PATH.
test-wasm:
	cd $(WEB) && RUSTUP_TOOLCHAIN=stable wasm-pack test --node
	cd $(ART) && RUSTUP_TOOLCHAIN=stable wasm-pack test --node
	cd $(STUDIO) && RUSTUP_TOOLCHAIN=stable wasm-pack test --node
	cd $(PSI) && RUSTUP_TOOLCHAIN=stable wasm-pack test --node
	cd $(PSIWEB) && RUSTUP_TOOLCHAIN=stable wasm-pack test --node

# Needs a real card dump; see CLAUDE.md. The prepared-copy test needs the
# card's cache/ (make pack ARGS="convert <card-root> --verify").
test-card:
	@echo "==> Testing $(CORE) against $(CARD_ROOT) (its cache/ from opendmd-pack convert)"
	cd $(CORE) && RUSTUP_TOOLCHAIN=stable DMD_CARD_ROOT=$(CARD_ROOT) \
		cargo test --release --features tween,names -- --ignored --nocapture

# SIM_FEATURES=psi: Psi formulas (make sim SIM_FEATURES=psi ARGS="--formula opendmd-psi/formulas/fire.psi").
SIM_FEATURES ?=
SIM_FEATURE_FLAG = $(if $(SIM_FEATURES),--features $(SIM_FEATURES),)
sim:
	cd $(SIM) && RUSTUP_TOOLCHAIN=stable cargo run --release $(SIM_FEATURE_FLAG) -- $(ARGS)

sim-build:
	cd $(SIM) && RUSTUP_TOOLCHAIN=stable cargo build --release $(SIM_FEATURE_FLAG)

# Psi's host timings: ns per frame of each sketch at lane widths 64 and 128
# (ARGS="--list" for the step listings, "FILE.psi" for more formulas).
psi-bench:
	cd $(PSI) && RUSTUP_TOOLCHAIN=stable cargo run --release --example bench -- $(if $(filter command line,$(origin ARGS)),$(ARGS),)

# Check a candidate formula (docs/psi-for-models.md): `psi check` — errors with
# their code, line and carets, steps, the panel's estimates (VM and JIT), the
# compile's heap, the lengths, and a PNG contact sheet of its frames.
# make psi-check F=my.psi [ARGS="--out sheet.png --frames 8 --a 0.5 --hash"]
psi-check:
	@test -n "$(F)" || { echo 'usage: make psi-check F=formula.psi [ARGS="--out sheet.png"]'; exit 2; }
	cd $(PSICLI) && RUSTUP_TOOLCHAIN=stable cargo run -q --release -- check $(abspath $(F)) $(if $(filter command line,$(origin ARGS)),$(ARGS),)

# The `psi` command (check, fmt, builtins/errors --json) into ~/.cargo/bin.
psi:
	RUSTUP_TOOLCHAIN=stable cargo install --locked --path $(PSICLI)

CARGO_ABOUT ?= cargo-about
CARGO_ABOUT_VERSION := 0.9.2

# Prepare a card (every GIF's copy under cache/): make pack ARGS="convert <card copy> --verify".
# See docs/cache-format.md.
pack:
	@test -n "$(PACK_ARGS)" || { echo "usage: make pack ARGS=\"convert <card-root> [--jobs N] [--verify] [--force]\""; exit 2; }
	cd $(PACK) && RUSTUP_TOOLCHAIN=stable cargo run --release -- $(PACK_ARGS)

check:
	@echo "==> Checking $(CORE)"
	cd $(CORE) && RUSTUP_TOOLCHAIN=stable cargo check --all-targets --features tween,names,ota,tags
	@echo "==> Checking $(RENDER)"
	cd $(RENDER) && RUSTUP_TOOLCHAIN=stable cargo check --all-targets
	@echo "==> Checking $(SIM)"
	cd $(SIM) && RUSTUP_TOOLCHAIN=stable cargo check --all-targets
	@echo "==> Checking $(PACK)"
	cd $(PACK) && RUSTUP_TOOLCHAIN=stable cargo check --all-targets
	@echo "==> Checking $(PSI)"
	cd $(PSI) && RUSTUP_TOOLCHAIN=stable cargo check --all-targets
	cd $(PSIWEB) && RUSTUP_TOOLCHAIN=stable cargo check --all-targets

# The UI is heading for the browser, so the core and the renderer have to stay
# free of anything wasm can't do — `std::time::Instant` above all, which has no
# clock there. One `rustup target add wasm32-unknown-unknown` is the only setup.
WASM_TARGET := wasm32-unknown-unknown
check-wasm:
	@echo "==> Checking $(CORE) for $(WASM_TARGET)"
	cd $(CORE) && RUSTUP_TOOLCHAIN=stable cargo check --target $(WASM_TARGET)
	cd $(CORE) && RUSTUP_TOOLCHAIN=stable cargo check --target $(WASM_TARGET) --features tween,names,tags
	@echo "==> Checking $(RENDER) for $(WASM_TARGET)"
	cd $(RENDER) && RUSTUP_TOOLCHAIN=stable cargo check --target $(WASM_TARGET)
	@echo "==> Checking $(WEB) for $(WASM_TARGET)"
	cd $(WEB) && RUSTUP_TOOLCHAIN=stable cargo check --target $(WASM_TARGET)
	@echo "==> Checking $(ART) for $(WASM_TARGET)"
	cd $(ART) && RUSTUP_TOOLCHAIN=stable cargo check --target $(WASM_TARGET)
	@echo "==> Checking $(STUDIO) for $(WASM_TARGET)"
	cd $(STUDIO) && RUSTUP_TOOLCHAIN=stable cargo check --target $(WASM_TARGET)
	@echo "==> Checking $(PSI) (and core's psi feature) for $(WASM_TARGET)"
	cd $(PSI) && RUSTUP_TOOLCHAIN=stable cargo check --target $(WASM_TARGET) --no-default-features
	cd $(CORE) && RUSTUP_TOOLCHAIN=stable cargo check --target $(WASM_TARGET) --features psi
	cd $(PSIWEB) && RUSTUP_TOOLCHAIN=stable cargo check --target $(WASM_TARGET) --features bare,bump

# wasm-pack: `cargo install --locked wasm-pack`. Output is gitignored.
# Then every file the device serves gets a `<file>.gz` beside it, and the
# whole app is packed into $(WWW_PACK) (`opendmd-pack www`, docs/www-pack.md):
# each file stored as its `.gz` where there is one, served from flash with
# `Content-Encoding: gzip`, unconditionally (see "Serving gzip" in
# CLAUDE.md). wasm-pack's package.json, .d.ts files and README are deleted
# so they are not packed. The Studio's own wasm (opendmd-art-web, "Make
# art") goes to www/pkg-art, loaded only when the Studio opens; the Formulas'
# (opendmd-psi-web: Psi, no std, its own allocator) to www/pkg-psi, loaded
# only when the Library's Formulas tab opens; the site's Studio's GIF writers
# (opendmd-studio-web) to www/pkg-studio, site-only like pkg-psi-lang (the
# packer leaves both out: opendmd-pack www.rs SITE_ONLY). The budgets
# (gzipped) are enforced by tools/web-size.py; the packer refuses a pack over
# the www partition. www/version.txt (gitignored) is `git describe` of this
# build: the pack's header carries it and the firmware reports it as
# `www_version` in /api/status (the file itself is not packed). The API
# reference is not in the pack: it is docs/api.md, published on the site
# (dev/api.html, and dev/api/v<version>.html per release, which Settings ->
# About links); tools/api-doc-check.py checks the doc's routes against
# firmware/src/http.rs both ways on every build. www/licenses.txt
# (gitignored) is the two committed third-party notices, firmware's then the
# web app's (licenses/, `make notices`), served as text from the pack:
# Settings -> Device -> About links it. www/js/build.js (gitignored) is
# the build id (tools/www-build-id.py: a hash of pkg/, pkg-art/, pkg-psi/
# and the site's pkg-psi-lang/ and pkg-studio/) that js/wasm.js puts on every wasm module's address, so a
# browser never runs an earlier build's module (opendmd-notes docs/notes/web-app.md
# "Module addresses").
#
# What ships is not www/ itself but $(WEB_DIST) (gitignored): a copy of it
# with every script and stylesheet minified one file at a time, no bundling
# (tools/web-minify.py running opendmd-webmin: oxc for JS, lightningcss for
# CSS, pinned in its Cargo.lock), so the sources keep their comments. The
# .gz files, the pack, the size lines, the mock (web-mock, web-smoke) and
# the site's /demo/ all read $(WEB_DIST). WEB_MINIFY=0 stages the files as
# they are, for a debugger (the size lines are then printed, not enforced):
# opendmd-notes docs/notes/web-app.md "Minified build".
WEB_WWW := $(WEB)/www
WEB_DIST := $(WEB)/www-dist
WEB_MINIFY ?= 1
webmin:
	cd $(WEBMIN) && RUSTUP_TOOLCHAIN=stable cargo build --release --locked --quiet
web: $(if $(filter 0,$(WEB_MINIFY)),,webmin)
	cd $(WEB) && RUSTUP_TOOLCHAIN=stable wasm-pack build --target web --release --out-dir www/pkg
	cd $(ART) && RUSTUP_TOOLCHAIN=stable wasm-pack build --target web --release --out-dir ../$(WEB_WWW)/pkg-art
	cd $(PSIWEB) && RUSTUP_TOOLCHAIN=stable wasm-pack build --target web --release --out-dir ../$(WEB_WWW)/pkg-psi -- --features bare,bump
	@# The site's language service (What costs most, Tidy up: js/fcost.js): the same crate with only observe() and
	@# format() (`lang`), never packed (opendmd-pack www.rs SITE_ONLY).
	cd $(PSIWEB) && RUSTUP_TOOLCHAIN=stable wasm-pack build --target web --release --out-dir ../$(WEB_WWW)/pkg-psi-lang -- --features bare,bump,lang
	@# The site's Studio's GIF writers (opendmd.dev/studio/, opendmd-studio-web), never packed (opendmd-pack www.rs SITE_ONLY).
	cd $(STUDIO) && RUSTUP_TOOLCHAIN=stable wasm-pack build --target web --release --out-dir ../$(WEB_WWW)/pkg-studio
	rm -f $(WEB_WWW)/pkg/package.json $(WEB_WWW)/pkg/*.d.ts $(WEB_WWW)/pkg/README.md
	rm -f $(WEB_WWW)/pkg-art/package.json $(WEB_WWW)/pkg-art/*.d.ts $(WEB_WWW)/pkg-art/README.md $(WEB_WWW)/pkg-art/.gitignore
	rm -f $(WEB_WWW)/pkg-psi/package.json $(WEB_WWW)/pkg-psi/*.d.ts $(WEB_WWW)/pkg-psi/README.md $(WEB_WWW)/pkg-psi/.gitignore
	rm -f $(WEB_WWW)/pkg-psi-lang/package.json $(WEB_WWW)/pkg-psi-lang/*.d.ts $(WEB_WWW)/pkg-psi-lang/README.md $(WEB_WWW)/pkg-psi-lang/.gitignore
	rm -f $(WEB_WWW)/pkg-studio/package.json $(WEB_WWW)/pkg-studio/*.d.ts $(WEB_WWW)/pkg-studio/README.md $(WEB_WWW)/pkg-studio/.gitignore
	@# js/build.js: the build id every wasm module's address carries (?v=, js/wasm.js), a hash of the five folders.
	uv run tools/www-build-id.py $(WEB_WWW)
	@# The bundled Formulas (the -- bundle files in $(WEB)/formulas/; the site's gallery shows them all).
	uv run tools/formulas-json.py $(WEB)/formulas $(WEB_WWW)/formulas.json
	git describe --always --dirty --tags --match 'v*' > $(WEB_WWW)/version.txt
	uv run tools/api-doc-check.py
	rm -f $(WEB_WWW)/api.html $(WEB_WWW)/api.html.gz
	{ cat $(NOTICES_FW_TXT); printf '\n\n'; cat $(NOTICES_WEB_TXT); } > $(WEB_WWW)/licenses.txt
	@# Earlier builds wrote the .gz copies beside the sources: none belong there now.
	find $(WEB_WWW) -name '*.gz' -delete
	uv run tools/web-minify.py $(WEB_WWW) $(WEB_DIST) $(if $(filter 0,$(WEB_MINIFY)),--no-minify)
	find $(WEB_DIST) -type f \( -name '*.html' -o -name '*.js' -o -name '*.css' -o -name '*.wasm' -o -name '*.svg' -o -name '*.json' -o -name licenses.txt \) \
		-exec gzip -9 -n -k -f {} \;
	cd $(PACK) && RUSTUP_TOOLCHAIN=stable cargo run --release --quiet -- \
		www ../$(WEB_DIST) --out ../$(WWW_PACK) --version-file ../$(WEB_DIST)/version.txt --max-bytes $(WWW_PACK_BYTES)
	@uv run tools/web-size.py $(WEB_DIST) --pack $(WWW_PACK) --pack-limit $(WWW_PACK_BYTES) --pack-spare $(WWW_PACK_SPARE) --share $(WEB)/share $(if $(filter 0,$(WEB_MINIFY)),--unminified)

# The headings' pixel font (Silkscreen, OFL 1.1): fetched from google/fonts at
# a pinned commit, checked, subset and written as committed files —
# www/fonts/silkscreen.woff2 + OFL.txt (packed, and published with the site)
# and the ASCII copy inlined in firmware/web/setup.html. Only needed to
# change the subset; `make web` uses the committed files.
font:
	uv run tools/font-subset.py

# The site's own pieces, committed like the font: its heading face (Fraunces,
# OFL 1.1, from google/fonts at a pinned commit: site/fonts/) and the home
# page's scenes (original pixel art drawn by the scripts: site/theme/den*.webp,
# everyone's and the ones a viewer can ask for: site/theme/SCENES.md).
# Only needed to change them; `make site` uses the committed files.
site-font:
	uv run tools/site-font.py
site-scene:
	uv run tools/site-scene-alt.py
	uv run tools/site-scene.py

# The OpenDMD mark and what is made from it (tools/logo.py): docs/images/
# logo.*, avatar-*, social-preview.png and Create Art's app icons, all
# committed. Only needed to change the mark.
logo:
	uv run tools/logo.py

# The third-party licence notices (tools/notices.py; licenses/ holds the
# config and the two committed files). Both are generated from what is
# built: the firmware's from the last `make fw-release` (the three ELFs'
# compile units, the IDF build's component list, cargo-about over the
# firmware's crates), the web app's from cargo-about over the wasm crates.
# `make notices` rewrites both (commit them); fw-release checks both and CI
# the web one: a stale file or a licence not accepted fails. Needs
# CARGO_ABOUT at CARGO_ABOUT_VERSION (above) and, for the firmware, the
# toolchain's readelf.
NOTICES_FW_TXT := licenses/THIRD-PARTY-LICENSES-firmware.txt
NOTICES_WEB_TXT := licenses/THIRD-PARTY-LICENSES-web.txt
NOTICES_ABOUT = --about $(CARGO_ABOUT) --about-version $(CARGO_ABOUT_VERSION)
NOTICES_FW = bash -c 'out=$$($(FW_RELEASE_IDF_OUT)) && \
	uv run --quiet tools/notices.py firmware $(NOTICES_ABOUT) --elf $(FW_RELEASE_BINARY) --idf-build "$$out/build" \
	--updater-build $(UPDATER_BUILD)/build --toolchain $(XTENSA_TOOLCHAIN) --readelf $(XTENSA_TOOLCHAIN)/bin/xtensa-esp32-elf-readelf $(1)'
NOTICES_WEB = uv run --quiet tools/notices.py web $(NOTICES_ABOUT) $(1)
notices:
	$(call NOTICES_FW,)
	$(call NOTICES_WEB,)
notices-check:
	$(call NOTICES_FW,--check)
	$(call NOTICES_WEB,--check)
notices-check-web:
	$(call NOTICES_WEB,--check)

# Static server for the page; uv, never bare python. Then open
# http://localhost:8080 — the file/directory picker does the rest.
web-serve:
	uv run python -m http.server 8080 -d $(WEB_DIST)

# A fake device (JSON API + card files + the built app) for working on the UI
# without a device: make web-mock ROOT=<card copy>, then open
# http://localhost:8080. OVERLAY=dir keeps the card copy untouched (writes go
# there); PROXY=http://<device> serves the local build against a real device.
ROOT ?= $(CARD_ROOT)
MOCK_PORT ?= 8080
web-mock: web
	uv run tools/mock-device.py --port $(MOCK_PORT) \
		$(if $(PROXY),--proxy $(PROXY),--root $(ROOT) $(if $(OVERLAY),--overlay $(OVERLAY)))

# Headless Chromium drive of the app against the mock (and, with DEVICE=,
# a read-mostly pass against a real device), over the card copy CARD_ROOT.
# CARD_ROOT on make's command line wins; else DMD_CARD_ROOT from the
# environment (tools/ci-card.py's usage line), else CARD_ROOT's default.
WEB_SMOKE_CARD = $(if $(filter command line,$(origin CARD_ROOT)),$(CARD_ROOT),$(or $(DMD_CARD_ROOT),$(CARD_ROOT)))
web-smoke: web
	DMD_CARD_ROOT="$(WEB_SMOKE_CARD)" uv run tools/web-smoke.py $(if $(DEVICE),--device $(DEVICE))

# The README's and the site's screenshots (docs/images/): the web app in
# headless Chromium against the mock over the demo card (the CC0 Kenney
# sprites and recordings of our own Formulas, with more of both put beside
# them by tools/screenshots.py), and docs/images/panels.json, where the
# panel is in the shots the home page plays a Formula over. GAME_ART=1 casts
# game title screens made into panel GIFs by Create Art instead
# (tools/game-art.py, cached in target/game-art, never committed or
# published; shots of them need their credit line: CLAUDE.md "Content
# licensing").
GAME_ART ?=
screenshots: web sim-build
	uv run tools/demo-card.py target/demo-card
	$(if $(GAME_ART),uv run tools/game-art.py --card target/demo-card --out target/game-art)
	uv run tools/screenshots.py --card target/demo-card --out docs/images $(if $(GAME_ART),--game-art target/game-art)

# The public site (https://opendmd.dev/): the README, the docs, and the web app
# with a demo card of the panel's own effects and CC0 sprites (tools/site.py,
# tools/demo-card.py, and tools/site-art.py for the site's own animations).
# site-serve to look at it; site-deploy (below) puts it on opendmd.dev. The
# formula gallery (formulas/) is the web app's examples plus the
# contributions in the public gallery repo's gallery/: GALLERY=<a checkout's
# gallery/> (release.yml fetches it; without it, the examples only). Each
# file is checked again by opendmd-gallery (make gallery-tool) before it is
# published.
SITE ?= target/site
GALLERY ?=
# The public gallery repo's dev/api/ (the HTTP API page as each earlier release
# described it; release.yml fetches it): API_ARCHIVE=<a checkout's dev/api/>.
# This release's own copy is frozen when API_FREEZE says so: auto (only
# when `git describe --match 'v*'` is exactly v<version>), yes (release.yml
# for a tag), no.
API_ARCHIVE ?=
API_FREEZE ?= auto
GALLERY_TOOL := $(GAL)/target/release/opendmd-gallery
# The public gallery repo (gallery/ takes contributions by pull request; the
# name is from its GitHub Pages days, it serves nothing now).
GALLERY_REPO ?= git@github.com:opendmd/opendmd.github.io.git
# The installer under install/ publishes whatever release dist/ holds; site
# does not build one (fw-release needs the xtensa toolchain). DIST is the
# release install/release.json and the Install button name (the newest
# final; before the first final, the newest beta); BETA_DIST=<a dist> a
# beta newer than it (a beta after a final: tools/release.sh fetches the
# final's files for DIST), which install/beta.json then names. Without it
# beta.json is release.json (docs/deploying.md "Betas").
BETA_DIST ?=
gallery-tool:
	cd $(GAL) && RUSTUP_TOOLCHAIN=stable cargo build --release --locked

site: web sim-build gallery-tool
	@test -f $(DIST)/manifest-4mb.json || { echo "no $(DIST)/manifest-4mb.json: the site publishes the release's installer, run make fw-release first"; exit 2; }
	uv run tools/demo-card.py target/demo-card
	uv run tools/site-art.py target/site-art
	@test -z "$(BETA_DIST)" || test -f $(BETA_DIST)/manifest-4mb.json || { echo "no $(BETA_DIST)/manifest-4mb.json (BETA_DIST)"; exit 2; }
	uv run tools/site.py $(SITE) --www $(WEB_DIST) --card target/demo-card --art target/site-art --dist $(DIST) $(if $(BETA_DIST),--beta-dist $(BETA_DIST)) \
		--gallery-tool $(GALLERY_TOOL) $(if $(filter 0,$(WEB_MINIFY)),,--webmin $(WEBMIN)/target/release/opendmd-webmin) $(if $(GALLERY),--gallery $(GALLERY)) $(if $(API_ARCHIVE),--api-archive $(API_ARCHIVE)) \
		--api-freeze $(API_FREEZE)

# The site's pages in Chromium (the formula page, the gallery, the API pages and
# the install page), after make site.
site-smoke:
	uv run tools/site-smoke.py $(SITE) --dist $(DIST) $(if $(BETA_DIST),--beta-dist $(BETA_DIST)) --art target/site-art $(if $(SITE_BASE),--base $(SITE_BASE))

site-serve: site
	uv run python -m http.server 8081 -d $(SITE)

# The site on Cloudflare (opendmd-notes docs/research/site-cloudflare.md): the opendmd-site
# Worker in site/worker/ (cf's cloudflare.config.ts + wrangler.config.ts; cf
# and wrangler pinned in its package.json/package-lock.json, installed by
# site-worker-deps) serves $(SITE) as static assets plus /f/* and /api/*.
# None of these rebuilds the site except site-deploy; make site first.
#   site-dev           cf dev, the Worker and the assets on http://127.0.0.1:$(SITE_DEV_PORT), with
#                      a local D1 (site/worker/.wrangler/state; the migrations applied first)
#   site-worker-test   tools/site-worker-test.py: starts cf dev itself (or SITE_BASE=<url>),
#                      checks the Worker's routes, short links, _headers, _redirects and the CSP in Chromium
#   site-d1-migrate    cf d1 migrations apply: site/worker/migrations/ to the opendmd-site D1
#                      (remote; site-deploy-built runs it before every deploy)
#   site-stats         the usage counts (/stats) in the terminal: SELECTs only, through
#                      cf d1 query on the opendmd-site D1 (remote, read-only; tools/site-stats.py)
#   site-dev-smoke     site-worker-test, then site-smoke against the same cf dev
#   site-deploy-check  cf deploy --dry-run: builds and checks, uploads nothing
#   site-deploy        make site, then site-d1-migrate and cf deploy: uploads a new version of the
#                      opendmd-site Worker and makes it live on opendmd.dev (auth:
#                      cf's OAuth profile, or CLOUDFLARE_API_TOKEN in the environment)
#   site-deploy-built  the same without rebuilding $(SITE); then POST /api/gallery/sync, so the
#                      gallery store (S6) holds this build's examples at once
# The Worker's cron syncs the public gallery repo's gallery/ into the store every 5 minutes
# (site/worker/src/gallery.ts): a merged contribution needs no deploy.
# site-smoke takes SITE_BASE=<url> too (the files it reads are still $(SITE)).
SITE_WORKER := site/worker
SITE_DEV_PORT ?= 8787
SITE_BASE ?=
CF = cd $(SITE_WORKER) && SITE_DIR="$(abspath $(SITE))" SITE_DEV_PORT=$(SITE_DEV_PORT) \
	SITE_VERSION="$$(git describe --always --dirty --tags --match 'v*')" npx --no-install cf
# The DB binding's database (cloudflare.config.ts, the one place its id is written).
SITE_D1 = $(shell sed -n 's/.*DB: {.* id: "\([0-9a-f-]*\)".*/\1/p' $(SITE_WORKER)/cloudflare.config.ts)
$(SITE_WORKER)/node_modules/.package-lock.json: $(SITE_WORKER)/package-lock.json $(SITE_WORKER)/package.json
	cd $(SITE_WORKER) && npm ci --no-audit --no-fund
site-worker-deps: $(SITE_WORKER)/node_modules/.package-lock.json

site-dev: site-worker-deps
	@test -f $(SITE)/_headers || { echo "no $(SITE)/_headers: make site first"; exit 2; }
	$(CF) d1 migrations apply $(SITE_D1) --local --persist-to .wrangler/state </dev/null
	$(CF) dev

site-d1-migrate: site-worker-deps
	@test -f $(SITE)/_headers || { echo "no $(SITE)/_headers: make site first"; exit 2; }
	$(CF) d1 migrations apply $(SITE_D1) </dev/null

site-stats: site-worker-deps
	uv run tools/site-stats.py

site-worker-test: site-worker-deps gallery-tool
	uv run tools/site-worker-test.py $(SITE) --dist $(DIST) $(if $(BETA_DIST),--beta-dist $(BETA_DIST)) $(if $(SITE_BASE),--base $(SITE_BASE))

site-dev-smoke: site-worker-deps gallery-tool
	uv run tools/site-worker-test.py $(SITE) --dist $(DIST) $(if $(BETA_DIST),--beta-dist $(BETA_DIST)) $(if $(SITE_BASE),--base $(SITE_BASE)) --smoke

site-deploy-check: site-worker-deps
	@test -f $(SITE)/_headers || { echo "no $(SITE)/_headers: make site first"; exit 2; }
	$(CF) deploy --dry-run

# Refuses a dirty tree (the version says what is live).
# site-deploy-built deploys $(SITE) as it is (release.yml and site.yml, after
# their own make site).
site-deploy: site
	$(MAKE) site-deploy-built
site-deploy-built: site-worker-deps
	@test -f $(SITE)/_headers || { echo "no $(SITE)/_headers: make site first"; exit 2; }
	@case "$$(git describe --always --dirty --tags --match 'v*')" in *-dirty) echo "refusing: the working tree is dirty; commit first"; exit 1;; esac
	$(MAKE) site-d1-migrate
	unset SITE_TEST_GALLERY_SOURCE SITE_TEST_RELAY_UPSTREAM; $(CF) deploy --message "site $$(git describe --always --tags --match 'v*')"
	@# The gallery store: this build's examples and the public repo's gallery/ now, not at the next cron.
	@curl -fsS -X POST -A 'opendmd-site-deploy/1 (+https://opendmd.dev)' --retry 3 --retry-delay 11 --retry-all-errors \
		https://opendmd.dev/api/gallery/sync -o /dev/null -w 'gallery sync: %{http_code}\n' || echo "gallery sync: not now (the cron runs it within 5 minutes)"

# The public gallery repo's own files (tools/gallery-repo-publish.sh): the
# gallery check and its vendored checker, the READMEs, and dev/api/ (this
# release's frozen API page, from $(SITE), added to the archive). release.yml
# runs it for a tag; by hand only by the maintainer.
gallery-repo-publish: site
	tools/gallery-repo-publish.sh $(SITE) $(GALLERY_REPO)

# A release (tools/release.sh; docs/deploying.md "Releasing"). make release
# VERSION=1.0.1 is a dry run: plan, bump (a detached commit + local tag,
# undone after), build signed with the real key, every check, the assets and
# the site; nothing pushed. PUBLISH=1 then pushes main and the tag, and
# .github/workflows/release.yml publishes (the GitHub release, the gallery
# repo, the Cloudflare deploy); it waits for that run and checks the result.
# PUBLISH=local is the fallback while CI is down: it publishes everything
# from here (and refuses once CI has run for the tag). RESUME=1 with either
# continues one that stopped part way.
release:
	@test -n "$(VERSION)" || { echo "make release VERSION=X.Y.Z[-beta.N] [PUBLISH=1|PUBLISH=local [RESUME=1]]"; exit 2; }
	tools/release.sh $(VERSION) $(if $(filter local,$(PUBLISH)),--publish-local,$(if $(PUBLISH),--publish)) $(if $(RESUME),--resume)

fmt:
	cd $(CORE) && RUSTUP_TOOLCHAIN=stable cargo fmt
	cd $(RENDER) && RUSTUP_TOOLCHAIN=stable cargo fmt
	cd $(SIM) && RUSTUP_TOOLCHAIN=stable cargo fmt
	cd $(WEB) && RUSTUP_TOOLCHAIN=stable cargo fmt
	cd $(ART) && RUSTUP_TOOLCHAIN=stable cargo fmt
	cd $(STUDIO) && RUSTUP_TOOLCHAIN=stable cargo fmt
	cd $(PACK) && RUSTUP_TOOLCHAIN=stable cargo fmt
	cd $(PSI) && RUSTUP_TOOLCHAIN=stable cargo fmt
	cd $(PSIWEB) && RUSTUP_TOOLCHAIN=stable cargo fmt
	cd $(GAL) && RUSTUP_TOOLCHAIN=stable cargo fmt
	cd $(WEBMIN) && RUSTUP_TOOLCHAIN=stable cargo fmt

clippy:
	@echo "==> Clippy $(CORE)"
	cd $(CORE) && RUSTUP_TOOLCHAIN=stable cargo clippy --all-targets -- -D warnings
	cd $(CORE) && RUSTUP_TOOLCHAIN=stable cargo clippy --all-targets --features tween,names,ota,tags -- -D warnings
	cd $(CORE) && RUSTUP_TOOLCHAIN=stable cargo clippy --all-targets --features player,ota -- -D warnings
	@echo "==> Clippy $(RENDER)"
	cd $(RENDER) && RUSTUP_TOOLCHAIN=stable cargo clippy --all-targets -- -D warnings
	@echo "==> Clippy $(SIM)"
	cd $(SIM) && RUSTUP_TOOLCHAIN=stable cargo clippy --all-targets -- -D warnings
	@echo "==> Clippy $(WEB)"
	cd $(WEB) && RUSTUP_TOOLCHAIN=stable cargo clippy --all-targets --target $(WASM_TARGET) -- -D warnings
	@echo "==> Clippy $(ART)"
	cd $(ART) && RUSTUP_TOOLCHAIN=stable cargo clippy --all-targets -- -D warnings
	cd $(ART) && RUSTUP_TOOLCHAIN=stable cargo clippy --all-targets --target $(WASM_TARGET) -- -D warnings
	@echo "==> Clippy $(STUDIO)"
	cd $(STUDIO) && RUSTUP_TOOLCHAIN=stable cargo clippy --all-targets -- -D warnings
	cd $(STUDIO) && RUSTUP_TOOLCHAIN=stable cargo clippy --all-targets --target $(WASM_TARGET) -- -D warnings
	@echo "==> Clippy $(PACK)"
	cd $(PACK) && RUSTUP_TOOLCHAIN=stable cargo clippy --all-targets -- -D warnings
	@echo "==> Clippy $(PSI) (+ core's and the simulator's psi feature)"
	cd $(PSI) && RUSTUP_TOOLCHAIN=stable cargo clippy --all-targets -- -D warnings
	cd $(PSI) && RUSTUP_TOOLCHAIN=stable cargo clippy --all-targets --features docs,syntax -- -D warnings
	cd $(PSI) && RUSTUP_TOOLCHAIN=stable cargo clippy --lib --no-default-features -- -D warnings
	cd $(PSI) && RUSTUP_TOOLCHAIN=stable cargo clippy --lib --target $(WASM_TARGET) --no-default-features -- -D warnings
	cd $(CORE) && RUSTUP_TOOLCHAIN=stable cargo clippy --all-targets --features psi -- -D warnings
	cd $(SIM) && RUSTUP_TOOLCHAIN=stable cargo clippy --all-targets --features psi -- -D warnings
	@echo "==> Clippy $(PSIWEB)"
	cd $(PSIWEB) && RUSTUP_TOOLCHAIN=stable cargo clippy --all-targets -- -D warnings
	cd $(PSIWEB) && RUSTUP_TOOLCHAIN=stable cargo clippy --all-targets --features lang -- -D warnings
	cd $(PSIWEB) && RUSTUP_TOOLCHAIN=stable cargo clippy --lib --target $(WASM_TARGET) --features bare,bump -- -D warnings
	@echo "==> Clippy $(GAL)"
	cd $(GAL) && RUSTUP_TOOLCHAIN=stable cargo clippy --all-targets -- -D warnings
	@echo "==> Clippy $(WEBMIN)"
	cd $(WEBMIN) && RUSTUP_TOOLCHAIN=stable cargo clippy --all-targets -- -D warnings
	@echo "==> Clippy $(PSICLI)"
	cd $(PSICLI) && RUSTUP_TOOLCHAIN=stable cargo clippy --all-targets -- -D warnings

clean:
	cd $(CORE) && cargo clean
	cd $(RENDER) && cargo clean
	cd $(SIM) && cargo clean
	cd $(WEB) && cargo clean
	cd $(ART) && cargo clean
	cd $(STUDIO) && cargo clean
	cd $(PACK) && cargo clean
	cd $(PSI) && cargo clean
	cd $(PSIWEB) && cargo clean
	cd $(GAL) && cargo clean
	cd $(WEBMIN) && cargo clean
	cd $(PSICLI) && cargo clean
	rm -rf $(WEB)/www/pkg $(WEB)/www/pkg-art $(WEB)/www/pkg-psi $(WEB)/www/pkg-psi-lang $(WEB)/www/pkg-studio $(WEB_DIST) $(WWW_PACK) ./target

# --- ESP32 firmware ---------------------------------------------------------
# First build clones ESP-IDF v5.3 into ~/.espressif (shared between projects,
# unlike stripe-dashboard's in-project .embuild) and takes 5-10 minutes.

# FW_FEATURES=probe (or probe,probe-http) builds the timing instrumentation in
# firmware/src/probe.rs; the default build has none of it. The bench targets
# (fw-build, fw-flash) always add `dev-credentials`, which compiles the
# firmware/.env WiFi seed in; fw-release never does.
FW_FEATURES ?=
FW_DEV_FEATURES = dev-credentials,bench$(if $(strip $(FW_DISABLE)),$(if $(strip $(FW_ENABLED)),$(comma)$(subst $(eval) ,$(comma),$(strip $(FW_ENABLED)))))$(if $(FW_FEATURES),$(comma)$(FW_FEATURES))
comma := ,
FW_CARGO_FEATURES = $(FW_NO_DEFAULT)--features $(FW_DEV_FEATURES)

# The components a build can leave out (firmware/Cargo.toml's default
# features; opendmd-notes docs/notes/firmware-build.md "Feature flags"): all on unless
# FW_DISABLE names them, e.g. `make fw-release FW_DISABLE=formulas`
# (`formulas` off takes `jit` with it). Such a build is
# `--no-default-features --features <the rest>`, in every firmware target
# (fw-build, fw-flash, fw-release, fw-release-flash, the emu images);
# `/api/status` `features` says what it has. Give a release build with
# components off its own FW_RELEASE_TARGET_DIR and DIST: its files have
# the release's names.
FW_OPTIONAL := formulas jit
FW_DISABLE ?=
FW_DISABLED = $(subst $(comma), ,$(FW_DISABLE))
FW_ENABLED = $(filter-out $(FW_DISABLED) $(if $(filter formulas,$(FW_DISABLED)),jit),$(FW_OPTIONAL))
FW_NO_DEFAULT = $(if $(FW_DISABLE),--no-default-features )
ifneq ($(filter-out $(FW_OPTIONAL),$(subst $(comma), ,$(FW_DISABLE))),)
$(error FW_DISABLE=$(FW_DISABLE): the components a build can leave out are $(FW_OPTIONAL))
endif

# Every build ends with tools/check-iram.py (the hot loops and the raced
# pair writer must be linked into IRAM: from flash their cost depends on the
# image's layout) and tools/check-stack-realign.py: a function that moves
# its stack pointer without MOVSP (an over-aligned local) corrupts its
# caller's registers now and then (opendmd-core/src/chan.rs). The diag build's
# std-channel torture test keeps one such function on purpose.
#
# A layout's sdkconfig fragment: its partition table's absolute path, its
# flash size (sdkconfig.defaults says 4 MB, which also sets the bootloader's
# and the app's image headers), and the app
# descriptor's version as the release sets it ("<version> <variant>",
# `GET /api/firmware`'s `build`), not IDF's default (`git describe`, or "1").
# The flash clock (FW_FLASH_FREQ=80|40, MHz, 80 by default; DIO
# either way; FW_FLASH_FREQ=40 for a board whose flash cannot run at 80 MHz,
# opendmd-notes docs/notes/flash-clock.md). The app sets
# SPI0's clock itself at start-up from its own image header
# (esp_system/port/cpu_start.c: bootloader_flash_clock_config(&app header),
# ESP32 without PSRAM), so 80 reaches a board by OTA with its bootloader
# left at 40: the app image's header (espflash --flash-freq) and the IDF's
# own idea of it (CONFIG_ESPTOOLPY_FLASHFREQ_*, which the flash driver's
# SPI1 uses) both follow this. `/api/firmware` `flash` reads the clock back.
FW_FLASH_FREQ ?= 80
# SRAM1 as IRAM (FW_SRAM1_IRAM=1): IDF's ESP_SYSTEM_ESP32_SRAM1_REGION_AS_IRAM,
# up to 40 KB more IRAM (0x400A0000..), each byte used taken from the top of
# the D/IRAM heap (heap/port/esp32/memory_layout.c reserves _heap_end..
# 0x40000000), and the firmware's `sram1-iram` feature, which places the next
# hottest functions there. The v5.3 bootloader's own data ends at 0x3FFF6000
# (bootloader.ld), below the region, and its load-address check takes SRAM1's
# IRAM alias as IRAM (SOC_IRAM_HIGH 0x400AA000): the bootloader OpenDMD's
# installs write (v5.3, the same code with or without the option) boots it,
# so it can go by OTA; a pre-5.1 bootloader would refuse the image.
FW_SRAM1_IRAM ?= 0
# lwIP copies each received frame out of the WiFi driver's buffer
# (FW_L2_COPY=1: CONFIG_LWIP_L2_TO_L3_COPY, off by default; a test
# switch, net-stall): the driver's eight receive buffers are then free again
# at once, and what lwIP holds is heap of the frames' size instead, bounded
# only by the windows (5760 B a connection with data unread). See "The
# receive side" in firmware/sdkconfig.defaults.
FW_L2_COPY ?= 0
# heapmap builds only (src/heapmap.rs): FreeRTOS's trace facility, for the
# task table (uxTaskGetSystemState).
# psi-probe builds only: the main task's stack 2 KB over the release's 16 KB
# (sdkconfig.defaults). The probe's checks run on the main task on top of
# the frame loop's own depth, and left 268 B of the 16 KB; a
# release build never runs them. (From the heap: the probe's heap figures
# are 2 KB under a release build's.)
FW_FRAGMENT = { echo 'CONFIG_PARTITION_TABLE_CUSTOM_FILENAME="$(CURDIR)/$(FW_PARTITIONS)"'; \
	$(if $(filter 80,$(FW_FLASH_FREQ)),echo 'CONFIG_ESPTOOLPY_FLASHFREQ_40M=n'; echo 'CONFIG_ESPTOOLPY_FLASHFREQ_80M=y';) \
	$(if $(filter 8mb,$(FW_FLASH_SIZE)),echo 'CONFIG_ESPTOOLPY_FLASHSIZE_4MB=n'; echo 'CONFIG_ESPTOOLPY_FLASHSIZE_8MB=y';) \
	$(if $(filter 1,$(FW_SRAM1_IRAM)),echo 'CONFIG_ESP_SYSTEM_ESP32_SRAM1_REGION_AS_IRAM=y';) \
	$(if $(filter 1,$(FW_L2_COPY)),echo 'CONFIG_LWIP_L2_TO_L3_COPY=y';) \
	$(if $(filter psi-probe paint-probe,$(subst $(FW_COMMA), ,$(FW_RELEASE_FEATURES) $(FW_FEATURES))),echo 'CONFIG_ESP_MAIN_TASK_STACK_SIZE=18432';) \
	$(if $(filter heapmap,$(subst $(FW_COMMA), ,$(FW_RELEASE_FEATURES) $(FW_FEATURES))),echo 'CONFIG_FREERTOS_USE_TRACE_FACILITY=y';) \
	echo 'CONFIG_APP_PROJECT_VER_FROM_CONFIG=y'; \
	echo 'CONFIG_APP_PROJECT_VER="$(FW_VERSION) $(FW_VARIANT)"'; }
# The bench builds' (fw-build, fw-flash, fw-clippy).
FW_BENCH_FRAGMENT = $(FW_FRAGMENT) > $(FW)/target/sdkconfig.partitions
# Before a build: any generated sdkconfig under $(1) (a glob) that does not
# carry this build's flash size, app version and partition table (left by
# the other layout, another version, or another checkout sharing the dir) is
# removed, so IDF makes a new one from the fragment instead of keeping its
# old values over it.
FW_SDKCONFIG_GUARD = for cfg in $(1); do \
	  [ -f "$$cfg" ] || continue; \
	  grep -qx 'CONFIG_ESPTOOLPY_FLASHSIZE="$(shell echo $(FW_FLASH_SIZE) | tr a-z A-Z)"' "$$cfg" \
	  && grep -qx 'CONFIG_APP_PROJECT_VER="$(FW_VERSION) $(FW_VARIANT)"' "$$cfg" \
	  && grep -qx 'CONFIG_PARTITION_TABLE_CUSTOM_FILENAME="$(CURDIR)/$(FW_PARTITIONS)"' "$$cfg" \
	  || { echo "==> $$cfg is not this build's ($(FW_VARIANT), $(FW_VERSION), $(FW_PARTITIONS)): removing it"; rm -f "$$cfg"; }; \
	done

# Delete the generated sdkconfig when the bench flavour (normal/diag) or the
# version changes: it keeps the values it already has (the descriptor's
# version among them) over a changed fragment.
fw-sdkconfig-flavour:
	@mkdir -p $(FW)/target
	@want="$(if $(FW_DIAG),diag,normal) $(FW_VERSION) $(FW_VARIANT) $$(cat $(FW)/sdkconfig.defaults $(if $(FW_DIAG),$(FW)/sdkconfig.diag)$(if $(FW_CHIP_SDKCONFIG), $(FW_CHIP_SDKCONFIG)) | sha256sum | cut -c1-16)"; have=$$(cat $(FW)/target/sdkconfig.flavour 2>/dev/null); \
	 if [ "$$want" != "$$have" ]; then \
	   echo "==> sdkconfig flavour $${have:-unknown} -> $$want: removing the generated sdkconfig"; \
	   rm -f $(FW)/target/$(FW_TRIPLE)/*/build/esp-idf-sys-*/out/sdkconfig; \
	   echo $$want > $(FW)/target/sdkconfig.flavour; \
	 fi
	@$(call FW_SDKCONFIG_GUARD,$(FW)/target/$(FW_TRIPLE)/*/build/esp-idf-sys-*/out/sdkconfig)

fw-build: fw-sdkconfig-flavour
	@mkdir -p $(FW)/target
	@$(FW_BENCH_FRAGMENT)
	cd $(FW) && bash -c '$(FW_ENV) . $(ESPUP_ENV) && RUSTUP_TOOLCHAIN=esp $(FW_MCU_ENV)OPENDMD_FLASH_VARIANT=$(FW_VARIANT) OPENDMD_FW_VERSION="$(FW_VERSION)" ESP_IDF_SDKCONFIG_DEFAULTS="$(FW_SDKCONFIG)" cargo build$(FW_TARGET_FLAG) $(FW_PROFILE_FLAG) $(FW_CARGO_FEATURES)'
	@ls -l $(FW_BINARY)
	bash -c '. $(ESPUP_ENV) && uv run --quiet tools/check-stack-realign.py $(FW_BINARY) $(if $(FW_DIAG),--allow "mpmc7channel.*9PlayError")'
	bash -c '. $(ESPUP_ENV) && uv run --quiet tools/check-iram.py $(FW_BINARY)'

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
	bash -c '. $(ESPUP_ENV) && espflash write-bin --port $(PORT) --baud 115200 $(WWW_OFFSET) $(WWW_PACK) && espflash write-bin --port $(PORT) --baud 115200 $(UPDATER_OFFSET) $(UPDATER_BIN) && espflash flash --port $(PORT) --baud 115200 --bootloader $(FW_BOOTLOADER) --partition-table $(FW_PARTITIONS) --erase-parts otadata --flash-size $(FW_VARIANT) --flash-freq $(FW_FLASH_FREQ)mhz --monitor --monitor-baud $(MONITOR_BAUD) $(FW_BINARY)'

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

# The feature sets: the bench build (dev-credentials + FW_FEATURES), the
# release build (the defaults, no .env) and none at all (no psi), so no cfg
# branch rots.
fw-clippy: fw-sdkconfig-flavour
	@mkdir -p $(FW)/target
	@$(FW_BENCH_FRAGMENT)
	@echo "==> Clippy firmware, --features $(FW_DEV_FEATURES)"
	cd $(FW) && bash -c '$(FW_ENV) . $(ESPUP_ENV) && RUSTUP_TOOLCHAIN=esp $(FW_MCU_ENV)OPENDMD_FLASH_VARIANT=$(FW_VARIANT) OPENDMD_FW_VERSION="$(FW_VERSION)" ESP_IDF_SDKCONFIG_DEFAULTS="$(FW_SDKCONFIG)" cargo clippy$(FW_TARGET_FLAG) --all-targets $(FW_CARGO_FEATURES) -- -D warnings'
	@echo "==> Clippy firmware, release feature set (the defaults: formulas, jit)"
	cd $(FW) && env -u OPENDMD_WIFI_SSID -u OPENDMD_WIFI_PASS bash -c '. $(ESPUP_ENV) && RUSTUP_TOOLCHAIN=esp $(FW_MCU_ENV)OPENDMD_FLASH_VARIANT=$(FW_VARIANT) OPENDMD_FW_VERSION="$(FW_VERSION)" ESP_IDF_SDKCONFIG_DEFAULTS="$(FW_SDKCONFIG)" cargo clippy$(FW_TARGET_FLAG) --all-targets -- -D warnings'
	@echo "==> Clippy firmware, no features (FW_DISABLE=formulas: without Formulas or the JIT)"
	cd $(FW) && env -u OPENDMD_WIFI_SSID -u OPENDMD_WIFI_PASS bash -c '. $(ESPUP_ENV) && RUSTUP_TOOLCHAIN=esp $(FW_MCU_ENV)OPENDMD_FLASH_VARIANT=$(FW_VARIANT) OPENDMD_FW_VERSION="$(FW_VERSION)" ESP_IDF_SDKCONFIG_DEFAULTS="$(FW_SDKCONFIG)" cargo clippy$(FW_TARGET_FLAG) --all-targets --no-default-features -- -D warnings'

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
# Tools) or espflash, for the flash layout (FW_VARIANTS: 4mb, the DevKit
# V1). Its files, <var> = 4mb:
#
#   opendmd-<ver>-<var>-app.bin    the app image, flashed at 0x20000 (factory)
#   bootloader-<var>.bin           at 0x1000 (its header carries the flash size)
#   partition-table-<var>.bin      at 0x8000
#   opendmd-<ver>-<var>-full.bin   all six parts merged into one image the size
#                                  of the flash (offset 0)
#   opendmd-<ver>-<var>.odfw       the signed bundle for PUT /api/firmware (fw-sign);
#                                  its magic names the layout and its table version
#   manifest-<var>.json            ESP Web Tools: the six parts and offsets; `name`
#                                  (FW_MANIFEST_NAME_<var>) and `version` are what the
#                                  firmware's Improv Serial answer says
#                                  (firmware/src/improv.rs: FIRMWARE_NAME,
#                                  state::firmware_version), which is how the
#                                  installer tells Update from Install; it waits
#                                  `new_install_improv_wait_time` (s) after an install
#                                  for the new firmware to answer, then asks for WiFi
#
# and:
#
#   updater.bin                    the updater (ota/updater), at ota_0 (0x2E0000)
#   ota_data_initial.bin           otadata erased (8 KB of 0xFF) at 0xF000: boots factory
#   opendmd-<ver>-www.pack         the web app, at www (0x340000)
#   THIRD-PARTY-LICENSES-firmware.txt, THIRD-PARTY-LICENSES-web.txt
#                                  the third-party notices (licenses/, `make notices`),
#                                  checked against each build first
#
# The version comes from firmware/Cargo.toml and is also written into the app
# descriptor as "<version> <var>" (checked below at offset 0x30 of the image).
# See docs/deploying.md.
FW_VERSION := $(shell sed -n 's/^version *= *"\(.*\)"/\1/p' $(FW)/Cargo.toml | head -1)
# The release build wants ~1.5 GB of its own (host build scripts plus a
# second ESP-IDF build); FW_RELEASE_TARGET_DIR=/somewhere/roomier moves it.
# CARGO_WORKSPACE_DIR is set below because embuild otherwise takes the crate
# root to be the directory above the target dir, and with the target dir
# elsewhere it would lose the extra_components (hub75, mdns) in Cargo.toml.
FW_RELEASE_TARGET_DIR ?= $(CURDIR)/$(FW)/target
FW_RELEASE_VTARGET := $(FW_RELEASE_TARGET_DIR)
FW_RELEASE_BINARY := $(FW_RELEASE_VTARGET)/$(FW_TRIPLE)/release/opendmd-firmware
FW_RELEASE_BOOTLOADER := $(FW_RELEASE_VTARGET)/$(FW_TRIPLE)/release/bootloader.bin
# The ESP-IDF build the release ELF was linked from (esp-idf-sys's out/), on
# stdout: the ELF names its esp-idf-sys-<hash> dir (the IDF objects' compile
# directories). A target dir can hold more than one (another esp-idf-sys, or
# its build script's env changed), and the newest is not always the one
# linked: notices.py then finds the ELF's IDF sources in no component. A shell
# command (no single quotes: NOTICES_FW runs it inside bash -c '...').
FW_RELEASE_IDF_OUT = h=$$(grep -aoE "esp-idf-sys-[0-9a-f]{16}" $(FW_RELEASE_BINARY) 2>/dev/null | sort -u); \
	d=$(FW_RELEASE_VTARGET)/$(FW_TRIPLE)/release/build/$$h/out; \
	if [ -n "$$h" ] && [ "$$(echo "$$h" | wc -l)" = 1 ] && [ -d "$$d" ]; then echo "$$d"; \
	else echo "$(FW_RELEASE_BINARY): names no one esp-idf-sys build dir in this target ($$(echo $${h:-none})): run make fw-release" >&2; exit 1; fi
# The fragment and its stamp live in the layout's own target dir: with a
# private FW_RELEASE_TARGET_DIR nothing a release build writes or removes
# (the generated sdkconfig among it) touches another checkout's build.
# (The S3's have names of their own, so one target dir holds both chips.)
FW_RELEASE_SDKCONFIG_FRAG := $(FW_RELEASE_VTARGET)/sdkconfig.release$(if $(FW_CHIP_SDKCONFIG),-$(FW_CHIP))
FW_RELEASE_SDKCONFIG_STAMP := $(FW_RELEASE_VTARGET)/sdkconfig.release$(if $(FW_CHIP_SDKCONFIG),-$(FW_CHIP)).stamp
FW_RELEASE_SDKCONFIG := $(CURDIR)/$(FW)/sdkconfig.defaults;$(if $(FW_CHIP_SDKCONFIG),$(FW_CHIP_SDKCONFIG);)$(FW_RELEASE_SDKCONFIG_FRAG)
# Measurement builds only, never a public release: the release set built
# with these cargo features (e.g. FW_RELEASE_FEATURES=probe,psi-probe
# DIST=/somewhere/else), signed like any other so it can go to a test panel
# over the LAN. Empty (the release feature set: none) by default.
FW_RELEASE_FEATURES ?=
FW_COMMA := ,
FW_RELEASE_CARGO_FEATURES = $(subst $(eval) ,$(FW_COMMA),$(strip $(subst $(FW_COMMA), ,$(FW_RELEASE_FEATURES)) $(if $(filter 1,$(FW_SRAM1_IRAM)),sram1-iram) $(if $(FW_DISABLE),$(FW_ENABLED))))
DIST := dist
FW_APP_IMAGE := $(DIST)/opendmd-$(FW_VERSION)-$(FW_VARIANT)-app.bin
FW_WWW_IMAGE := $(DIST)/opendmd-$(FW_VERSION)-www.pack
FW_FULL_IMAGE := $(DIST)/opendmd-$(FW_VERSION)-$(FW_VARIANT)-full.bin
FW_DIST_BOOTLOADER := $(DIST)/bootloader-$(FW_VARIANT).bin
FW_DIST_TABLE := $(DIST)/partition-table-$(FW_VARIANT).bin
FW_MANIFEST := $(DIST)/manifest-$(FW_VARIANT).json
# Offline mode (docs/deploying.md "Offline installs"): the installer's "Play
# from the card only, no WiFi" box swaps the button to this manifest, the six
# parts plus an image of the whole `nvs` partition that holds one setting,
# namespace `opendmd`, string `network` = `off`. The partition's offset and
# size come from the layout's table.
FW_MANIFEST_OFFLINE := $(DIST)/manifest-$(FW_VARIANT)-offline.json
FW_NVS_OFFLINE := $(DIST)/nvs-offline-$(FW_VARIANT).bin
FW_NVS_ROW = $(shell awk -F'[[:space:]]*,[[:space:]]*' '$$1 == "nvs" && $$3 == "nvs" { print $$4, $$5 }' $(FW_PARTITIONS))
FW_NVS_OFFSET = $(word 1,$(FW_NVS_ROW))
FW_NVS_SIZE = $(word 2,$(FW_NVS_ROW))
# Panic locations name source files: the release names them from neutral
# roots instead of this machine's paths (cargo joins these to the config's
# rustflags).
FW_RELEASE_REMAP := --remap-path-prefix=$(CURDIR)=/opendmd --remap-path-prefix=$(HOME)/.cargo=/cargo --remap-path-prefix=$(HOME)/.rustup=/rustup
# The app image must leave at least this much of the layout's factory
# partition free (2,883,584 B in table v4).
FW_APP_SPARE := 65536

# The layout (FW_VARIANTS) through fw-release-variant, after `make web`.
fw-release: web
	@for v in $(or $(VARIANT),$(FW_VARIANTS)); do \
	  $(MAKE) --no-print-directory fw-release-variant VARIANT=$$v || exit 1; \
	done
	@echo "dist/:"; ls -l $(foreach v,$(or $(VARIANT),$(FW_VARIANTS)),$(DIST)/opendmd-$(FW_VERSION)-$(v)-app.bin $(DIST)/opendmd-$(FW_VERSION)-$(v).odfw \
	  $(DIST)/opendmd-$(FW_VERSION)-$(v)-full.bin $(DIST)/bootloader-$(v).bin $(DIST)/partition-table-$(v).bin $(DIST)/manifest-$(v).json \
	  $(DIST)/manifest-$(v)-offline.json $(DIST)/nvs-offline-$(v).bin) \
	  $(FW_WWW_IMAGE) $(DIST)/updater.bin $(DIST)/ota_data_initial.bin $(DIST)/THIRD-PARTY-LICENSES-firmware.txt $(DIST)/THIRD-PARTY-LICENSES-web.txt

# One layout's release set (FW_VARIANT): needs `make web` done (fw-release).
fw-release-variant:
	@test -n "$(FW_VERSION)" || { echo "cannot read the version from $(FW)/Cargo.toml"; exit 2; }
	@test -f $(WWW_PACK) || { echo "no $(WWW_PACK): run make fw-release (it runs make web first)"; exit 2; }
	@echo "==> Release build, $(FW_VARIANT)$(if $(FW_DISABLE), without $(FW_DISABLE)) ($(FW_PARTITIONS), target $(FW_RELEASE_VTARGET))"
	@mkdir -p $(FW_RELEASE_VTARGET) $(DIST)
	@$(FW_FRAGMENT) > $(FW_RELEASE_SDKCONFIG_FRAG)
	@# The generated sdkconfig keeps the values it has over changed defaults
	@# (a new sdkconfig.defaults line or version would silently not apply):
	@# remove it whenever the inputs change.
	@want=$$(cat $(FW)/sdkconfig.defaults $(FW_CHIP_SDKCONFIG) $(FW_RELEASE_SDKCONFIG_FRAG) | sha256sum | cut -c1-16); \
	 have=$$(cat $(FW_RELEASE_SDKCONFIG_STAMP) 2>/dev/null); \
	 if [ "$$want" != "$$have" ]; then \
	   echo "==> release sdkconfig inputs changed ($(FW_VARIANT)): removing the generated sdkconfig"; \
	   rm -f $(FW_RELEASE_VTARGET)/$(FW_TRIPLE)/release/build/esp-idf-sys-*/out/sdkconfig; \
	   echo $$want > $(FW_RELEASE_SDKCONFIG_STAMP); \
	 fi
	@$(call FW_SDKCONFIG_GUARD,$(FW_RELEASE_VTARGET)/$(FW_TRIPLE)/release/build/esp-idf-sys-*/out/sdkconfig)
	cd $(FW) && env -u OPENDMD_WIFI_SSID -u OPENDMD_WIFI_PASS bash -c '. $(ESPUP_ENV) && RUSTUP_TOOLCHAIN=esp CARGO_WORKSPACE_DIR="$(CURDIR)/$(FW)" $(FW_MCU_ENV)OPENDMD_FLASH_VARIANT=$(FW_VARIANT) OPENDMD_FW_VERSION="$(FW_VERSION)" ESP_IDF_SDKCONFIG_DEFAULTS="$(FW_RELEASE_SDKCONFIG)" CARGO_TARGET_$(FW_TRIPLE_ENV)_RUSTFLAGS="$(FW_RELEASE_REMAP)" cargo build$(FW_TARGET_FLAG) --release --target-dir "$(FW_RELEASE_VTARGET)" $(FW_NO_DEFAULT)$(if $(FW_RELEASE_CARGO_FEATURES),--features $(FW_RELEASE_CARGO_FEATURES),)'
	@# The generated sdkconfig must carry the layout's flash size.
	@out=$$($(FW_RELEASE_IDF_OUT)) || exit 1; cfg=$$out/sdkconfig; \
	 want=$$(echo $(FW_FLASH_SIZE) | tr a-z A-Z); \
	 grep -q "^CONFIG_ESPTOOLPY_FLASHSIZE=\"$$want\"" "$$cfg" || { echo "$$cfg: CONFIG_ESPTOOLPY_FLASHSIZE is not \"$$want\""; exit 1; }; \
	 echo "sdkconfig: flash size $$want ($$cfg)"
	@# The updater after the firmware: on a fresh machine (CI) it is the
	@# firmware's build that installs the ESP-IDF checkout idf.py needs.
	@# (The S3 preview has none yet: its ota_0 stays erased.)
	$(if $(FW_NO_UPDATER),@echo "$(FW_VARIANT): no updater yet (the classic's is built for the ESP32)",$(MAKE) --no-print-directory updater)
	bash -c '. $(ESPUP_ENV) && uv run --quiet tools/check-stack-realign.py $(FW_RELEASE_BINARY)'
	bash -c '. $(ESPUP_ENV) && uv run --quiet tools/check-iram.py $(FW_RELEASE_BINARY)'
	@# The third-party notices: the committed ones must be what this build links
	@# (a build with components off links a subset of the default build's
	@# crates: the default build's check covers it). The S3 preview links IDF
	@# components the classic does not (esp_lcd, the PSRAM driver): its
	@# notices are the release phase's.
	$(if $(FW_NO_UPDATER),@echo "notices: not checked for the $(FW_VARIANT) preview",$(if $(FW_DISABLE),@echo "notices: not checked with FW_DISABLE=$(FW_DISABLE) (a subset of the default build's crates)",$(MAKE) --no-print-directory notices-check))
	bash -c '. $(ESPUP_ENV) && espflash save-image --chip $(FW_CHIP) --flash-size $(FW_FLASH_SIZE) --flash-freq $(FW_FLASH_FREQ)mhz $(FW_RELEASE_BINARY) $(FW_APP_IMAGE)'
	@desc=$$(dd if=$(FW_APP_IMAGE) bs=1 skip=48 count=32 2>/dev/null | tr -d '\000'); \
	 echo "app descriptor version: '$$desc'"; \
	 test "$$desc" = "$(FW_VERSION) $(FW_VARIANT)" || { echo "expected '$(FW_VERSION) $(FW_VARIANT)' at 0x30"; exit 1; }
	@size=$$(stat -c %s $(FW_APP_IMAGE)); \
	 spare=$$(( $(FW_APP_BYTES) - size )); \
	 pct=$$(awk "BEGIN { printf \"%.1f\", $$size * 100 / $(FW_APP_BYTES) }"); \
	 echo "$(FW_APP_IMAGE): $$size B = $$pct% of the $(FW_APP_BYTES) B $(FW_VARIANT) factory partition, $$spare B spare (need $(FW_APP_SPARE))"; \
	 test $$spare -ge $(FW_APP_SPARE) || { echo "the app image leaves less than $(FW_APP_SPARE) B of the $(FW_VARIANT) factory partition"; exit 1; }
	@size=$$(( $$(stat -c %s $(WWW_PACK)) + 72 )); \
	 echo "$(WWW_PACK): $$size B signed, of the $(WWW_BYTES) B $(FW_VARIANT) www partition, $$(( $(WWW_BYTES) - size )) B spare (need $(WWW_PACK_SPARE))"; \
	 test $$size -le $(WWW_BYTES) || { echo "the www pack does not fit the $(FW_VARIANT) www partition"; exit 1; }; \
	 test $$(( $(WWW_BYTES) - size )) -ge $(WWW_PACK_SPARE) || { echo "the www pack leaves less than $(WWW_PACK_SPARE) B of the $(FW_VARIANT) www partition"; exit 1; }
ifneq ($(FW_UNRELEASED),)
	@echo "$(FW_VARIANT) is not released: no signed bundle"
else
	$(MAKE) --no-print-directory fw-sign
endif
	@# The www pack, signed with the same key (docs/www-pack.md "Signature"):
	@# from 1.0.0-beta.15 on a player takes only a signed one over PUT /api/www.
	uv run --quiet tools/fw-sign.py sign-www $(WWW_PACK) --out $(FW_WWW_IMAGE) --key $(FW_SIGN_KEY)
	uv run --quiet tools/fw-sign.py verify-www $(FW_WWW_IMAGE) --pub $(FW_SIGN_PUB)
	cp $(FW_RELEASE_BOOTLOADER) $(FW_DIST_BOOTLOADER)
ifeq ($(FW_NO_UPDATER),)
	cp $(UPDATER_BIN) $(DIST)/updater.bin
	cp $(UPDATER_BUILD)/build/ota_data_initial.bin $(DIST)/ota_data_initial.bin
	cp $(NOTICES_FW_TXT) $(NOTICES_WEB_TXT) $(DIST)/
endif
	@! strings $(FW_APP_IMAGE) $(if $(FW_NO_UPDATER),,$(DIST)/updater.bin )$(FW_DIST_BOOTLOADER) | grep -q '$(HOME)' || { echo "the release names $(HOME): a build path got in"; exit 1; }
	bash -c '. $(ESPUP_ENV) && espflash partition-table --to-binary -o $(FW_DIST_TABLE) $(FW_PARTITIONS)'
	@# The flash size in the image headers (byte 3's high nibble: 2 = 4 MB):
	@# the bootloader's is what the board runs with.
	@want=$$(case $(FW_FLASH_SIZE) in 4mb) echo 2;; 8mb) echo 3;; esac); \
	 for f in $(FW_DIST_BOOTLOADER) $(FW_APP_IMAGE); do \
	   got=$$(dd if=$$f bs=1 skip=3 count=1 status=none | od -An -tu1 | tr -d ' '); \
	   test $$(( got >> 4 )) -eq $$want || { echo "$$f: flash size code $$(( got >> 4 )) in its header, want $$want ($(FW_VARIANT))"; exit 1; }; \
	 done; echo "image headers: flash size $(FW_VARIANT) (bootloader, app)"
	@# The app's flash clock (byte 3's low nibble: 0 = 40 MHz, 15 = 80 MHz),
	@# which the app applies at start-up whatever the bootloader runs at.
	@want=$$(case $(FW_FLASH_FREQ) in 40) echo 0;; 80) echo 15;; *) echo x;; esac); \
	 got=$$(dd if=$(FW_APP_IMAGE) bs=1 skip=3 count=1 status=none | od -An -tu1 | tr -d ' '); \
	 test $$(( got & 15 )) -eq $$want || { echo "$(FW_APP_IMAGE): flash clock code $$(( got & 15 )) in its header, want $$want ($(FW_FLASH_FREQ) MHz)"; exit 1; }; \
	 echo "app header: flash clock $(FW_FLASH_FREQ) MHz"
	@# The merged image: espflash lays out bootloader, table and app padded to
	@# the flash size with 0xFF (so otadata is erased: the bootloader boots
	@# factory); the updater and the pack go in at their offsets on top.
	bash -c '. $(ESPUP_ENV) && espflash save-image --chip $(FW_CHIP) --flash-size $(FW_FLASH_SIZE) --flash-freq $(FW_FLASH_FREQ)mhz --merge --bootloader $(FW_RELEASE_BOOTLOADER) --partition-table $(FW_PARTITIONS) $(FW_RELEASE_BINARY) $(FW_FULL_IMAGE)'
	$(if $(FW_NO_UPDATER),@echo "$(FW_VARIANT): ota_0 left erased (no updater)",dd if=$(DIST)/updater.bin of=$(FW_FULL_IMAGE) bs=4096 seek=$$(( $(UPDATER_OFFSET) / 4096 )) conv=notrunc status=none)
	dd if=$(FW_WWW_IMAGE) of=$(FW_FULL_IMAGE) bs=4096 seek=$$(( $(WWW_OFFSET) / 4096 )) conv=notrunc status=none
	@test "$$(dd if=$(FW_FULL_IMAGE) bs=4096 skip=$$(( $(FW_APP_OFFSET) / 4096 )) count=1 status=none | head -c 1 | od -An -tx1 | tr -d ' ')" = e9 || { echo "no app image at $(FW_APP_OFFSET) in $(FW_FULL_IMAGE)"; exit 1; }
	@test "$$(dd if=$(FW_FULL_IMAGE) bs=4096 skip=$$(( $(OTADATA_OFFSET) / 4096 )) count=2 status=none | tr -d '\377' | wc -c)" = 0 || { echo "otadata is not erased in $(FW_FULL_IMAGE)"; exit 1; }
	@test $$(stat -c %s $(FW_FULL_IMAGE)) -eq $(FW_FLASH_BYTES) || { echo "$(FW_FULL_IMAGE) is not $(FW_FLASH_BYTES) B"; exit 1; }
ifneq ($(FW_UNRELEASED),)
	@echo "==> $(FW_VARIANT) (not released) done: no manifest"
else
	@# The offline install's NVS image (FW_NVS_OFFLINE): the whole `nvs`
	@# partition, written by IDF v5.3's nvs_partition_gen from the IDF
	@# Python env the firmware's build set up (found now, not when make
	@# started: on a fresh machine that build is what makes it).
	@test -n "$(FW_NVS_OFFSET)" && test -n "$(FW_NVS_SIZE)" || { echo "no nvs partition in $(FW_PARTITIONS)"; exit 1; }
	@py=$$(ls -d $(ESPRESSIF)/python_env/idf5.3_py3*_env/bin/python 2>/dev/null | sort | tail -1); \
	 test -n "$$py" || { echo "no ESP-IDF v5.3 Python env under $(ESPRESSIF)/python_env (its nvs_partition_gen makes $(FW_NVS_OFFLINE))"; exit 1; }; \
	 printf '%s\n' key,type,encoding,value opendmd,namespace,, network,data,string,off > $(FW_RELEASE_VTARGET)/nvs-offline.csv; \
	 "$$py" -m esp_idf_nvs_partition_gen generate $(FW_RELEASE_VTARGET)/nvs-offline.csv $(FW_NVS_OFFLINE) $(FW_NVS_SIZE) >/dev/null \
	   || { echo "nvs_partition_gen failed for $(FW_NVS_OFFLINE)"; exit 1; }; \
	 echo "$(FW_NVS_OFFLINE): $$(stat -c %s $(FW_NVS_OFFLINE)) B for the nvs partition at $(FW_NVS_OFFSET) ($(FW_NVS_SIZE)), network = off"
	@# The two manifests, the parts in the order ESP Web Tools writes them
	@# (flash order): the normal one, and the offline one with the NVS image
	@# and no Improv wait (no WiFi form after the install; nor is a board
	@# recognised through it).
	@boot='        { "path": "bootloader-$(FW_VARIANT).bin", "offset": 4096 }'; \
	 table='        { "path": "partition-table-$(FW_VARIANT).bin", "offset": 32768 }'; \
	 nvs='        { "path": "nvs-offline-$(FW_VARIANT).bin", "offset": '$$(( $(FW_NVS_OFFSET) ))' }'; \
	 otadata='        { "path": "ota_data_initial.bin", "offset": '$$(( $(OTADATA_OFFSET) ))' }'; \
	 app='        { "path": "opendmd-$(FW_VERSION)-$(FW_VARIANT)-app.bin", "offset": '$$(( $(FW_APP_OFFSET) ))' }'; \
	 upd='        { "path": "updater.bin", "offset": '$$(( $(UPDATER_OFFSET) ))' }'; \
	 www='        { "path": "opendmd-$(FW_VERSION)-www.pack", "offset": '$$(( $(WWW_OFFSET) ))' }'; \
	 manifest() { \
	  wait=$$1; shift; \
	  printf '%s\n' '{' \
	  '  "name": "$(FW_MANIFEST_NAME_$(FW_VARIANT))",' \
	  '  "version": "$(FW_VERSION)",' \
	  '  "new_install_prompt_erase": true,' \
	  '  "new_install_improv_wait_time": '$$wait',' \
	  '  "builds": [' \
	  '    {' \
	  '      "chipFamily": "ESP32",' \
	  '      "parts": ['; \
	  sep=; for part in "$$@"; do \
	    eval "line=\$$$$part"; [ -z "$$sep" ] || printf ',\n'; printf '%s' "$$line"; sep=1; done; echo; \
	  printf '%s\n' '      ]' \
	  '    }' \
	  '  ]' \
	  '}'; }; \
	 manifest 15 boot table otadata app upd www > $(FW_MANIFEST); \
	 manifest 0 boot table nvs otadata app upd www > $(FW_MANIFEST_OFFLINE)
	@uv run --quiet python -I -c 'import json, os, struct, sys; \
	  m, o = (json.load(open(f)) for f in sys.argv[1:3]); nvs, off, size = sys.argv[3], int(sys.argv[4], 0), int(sys.argv[5], 0); \
	  p, q = m["builds"][0]["parts"], o["builds"][0]["parts"]; \
	  assert len(m["builds"]) == len(o["builds"]) == 1 and len(p) == 6 and len(q) == 7, "part counts"; \
	  assert m["new_install_improv_wait_time"] == 15 and o["new_install_improv_wait_time"] == 0, "Improv wait"; \
	  assert m["new_install_prompt_erase"] is True and o["new_install_prompt_erase"] is True, "erase prompt"; \
	  assert (o["name"], o["version"]) == (m["name"], m["version"]), "name and version"; \
	  assert q[:2] + q[3:] == p and q[2] == {"path": os.path.basename(nvs), "offset": off}, "the offline parts"; \
	  assert [x["offset"] for x in q] == sorted(x["offset"] for x in q), "flash order"; \
	  b = open(nvs, "rb").read(); assert len(b) == size, "the nvs image is not the partition size"; \
	  assert struct.unpack_from("<I", b)[0] == 0xFFFFFFFE and b"opendmd" in b[64:4096] and b"network" in b[64:4096], "the nvs image holds no opendmd/network"' \
	  $(FW_MANIFEST) $(FW_MANIFEST_OFFLINE) $(FW_NVS_OFFLINE) $(FW_NVS_OFFSET) $(FW_NVS_SIZE) \
	  || { echo "$(FW_MANIFEST) or $(FW_MANIFEST_OFFLINE) is not the manifest it should be"; exit 1; }
	@echo "==> $(FW_VARIANT) offline install: $(FW_MANIFEST_OFFLINE) (seven parts, $(notdir $(FW_NVS_OFFLINE)) at $(FW_NVS_OFFSET))"
	@echo "==> $(FW_VARIANT) done: $(FW_MANIFEST)"
endif

# The signed bundle for a LAN update (docs/ota.md): the release app image
# behind a 140-byte header signed with the private key, which never enters
# the repo (`uv run tools/fw-sign.py keygen` made it; FW_SIGN_KEY moves it).
# Checked against the committed public key, firmware/keys/opendmd.pub, the
# one the firmware compiles in. Run by fw-release for each layout; on its
# own it signs whatever app image dist/ holds for this version and VARIANT
# (the bundle's magic names the layout, and fw-sign.py refuses an image
# whose app descriptor is for the other one).
FW_SIGN_KEY ?= $(HOME)/.config/opendmd/opendmd.key
# The public key the bundle is checked against. Only a dry run (CI, a
# throwaway key) points it elsewhere: a device trusts the committed one alone.
FW_SIGN_PUB ?= $(FW)/keys/opendmd.pub
FW_BUNDLE := $(DIST)/opendmd-$(FW_VERSION)-$(FW_VARIANT).odfw
fw-sign:
	@test -f $(FW_APP_IMAGE) || { echo "no $(FW_APP_IMAGE): run make fw-release first"; exit 2; }
	@test -f $(FW_SIGN_KEY) || { echo "no signing key at $(FW_SIGN_KEY): uv run tools/fw-sign.py keygen (once), or FW_SIGN_KEY=..."; exit 2; }
	uv run --quiet tools/fw-sign.py sign $(FW_APP_IMAGE) --version $(FW_VERSION) --variant $(FW_VARIANT) --out $(FW_BUNDLE) --key $(FW_SIGN_KEY)
	uv run --quiet tools/fw-sign.py verify $(FW_BUNDLE) --pub $(FW_SIGN_PUB)

# First install on a board over USB, from the release build: the same parts
# as dist/ (bootloader, table, otadata erased, app, updater, pack) for
# the DevKit (4mb), at the table's offsets; NVS is not written, so a board
# on an older OpenDMD table (v3: docs/deploying.md "Moving a board to table
# v4") keeps its WiFi and settings. From DMDoS or other firmware: `make
# fw-erase` first.
fw-release-flash:
	$(MAKE) --no-print-directory fw-release VARIANT=$(FW_VARIANT)
	bash -c '. $(ESPUP_ENV) && espflash write-bin --port $(PORT) --baud 115200 $(WWW_OFFSET) $(FW_WWW_IMAGE) && espflash write-bin --port $(PORT) --baud 115200 $(UPDATER_OFFSET) $(DIST)/updater.bin && espflash flash --port $(PORT) --baud 115200 --bootloader $(FW_RELEASE_BOOTLOADER) --partition-table $(FW_PARTITIONS) --erase-parts otadata --flash-size $(FW_VARIANT) --flash-freq $(FW_FLASH_FREQ)mhz --monitor --monitor-baud $(MONITOR_BAUD) $(FW_RELEASE_BINARY)'

# Updates across releases and flash layouts (tools/ota-compat.sh): an older
# release's own opendmd-core (OTA_COMPAT_TAG, the last DevKit table v3 one,
# a detached worktree under target/) must refuse a 4mb bundle signed now
# (table v4) and take a v3 one and the www pack; this tree's must take the
# v4 bundle and refuse the v3 one. Throwaway-key files, or
# OTA_COMPAT_FILES="<v4 bundle> <pub> <www>". OTA_COMPAT_OK.
OTA_COMPAT_TAG ?= v1.0.1
# DIST's 4mb release bundle and pack too, when it has them (signed with the
# release key: checked against FW_SIGN_PUB).
OTA_COMPAT_DIST_BUNDLE = $(DIST)/opendmd-$(FW_VERSION)-4mb.odfw
OTA_COMPAT_FILES ?= $(if $(wildcard $(OTA_COMPAT_DIST_BUNDLE)),$(OTA_COMPAT_DIST_BUNDLE) $(FW_SIGN_PUB) $(FW_WWW_IMAGE))
ota-compat:
	tools/ota-compat.sh $(OTA_COMPAT_TAG) $(OTA_COMPAT_FILES)

# The web app over WiFi: PUT the pack to /api/www, then check the version
# the device reports. make www-push HOST=opendmd.local [WWW_PACK=dist/...]
www-push:
	@test -n "$(HOST)" || { echo "usage: make www-push HOST=<ip-or-name> [WWW_PACK=$(WWW_PACK)]"; exit 2; }
	FW_SIGN_KEY=$(FW_SIGN_KEY) tools/www-push.sh $(HOST) $(WWW_PACK)

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
UPDATER_BUILD ?= $(if $(SCRATCH),$(SCRATCH)/opendmd-target/updater,$(CURDIR)/target/updater)
UPDATER_BIN := $(UPDATER_BUILD)/build/opendmd-updater.bin
UPDATER_SLOT := 393216
UPDATER_SPARE := 32768
# IDF v5.3's GCC (also what tools/notices.py reads the licences and readelf from).
XTENSA_TOOLCHAIN := $(ESPRESSIF)/tools/xtensa-esp-elf/esp-13.2.0_20240530/xtensa-esp-elf
UPDATER_ENV := IDF_PATH=$(UPDATER_IDF) IDF_PYTHON_ENV_PATH=$(UPDATER_PYENV) \
	ESP_ROM_ELF_DIR=$(ESPRESSIF)/tools/esp-rom-elfs/20240305/ \
	PATH=$(UPDATER_PYENV)/bin:$(XTENSA_TOOLCHAIN)/bin:$(ESPRESSIF)/tools/cmake/3.24.0/bin:$(ESPRESSIF)/tools/ninja/1.11.1:$(UPDATER_IDF)/tools:$$PATH
updater:
	@test -f firmware/components/ESP32-HUB75-MatrixPanel-I2S-DMA/CMakeLists.txt || { echo "the HUB75 driver is missing: git submodule update --init"; exit 2; }
	@mkdir -p $(UPDATER_BUILD)
	@# idf.py only reads sdkconfig.defaults into a new sdkconfig.
	@if [ ota/updater/sdkconfig.defaults -nt $(UPDATER_BUILD)/sdkconfig ]; then rm -f $(UPDATER_BUILD)/sdkconfig; fi
	cd ota/updater && env $(UPDATER_ENV) python $(UPDATER_IDF)/tools/idf.py \
		-B $(UPDATER_BUILD)/build -DSDKCONFIG=$(UPDATER_BUILD)/sdkconfig build
	@size=$$(stat -c %s $(UPDATER_BIN)); \
	 echo "$(UPDATER_BIN): $$size B of the $(UPDATER_SLOT) B ota_0 slot, $$(( $(UPDATER_SLOT) - size )) B spare (need $(UPDATER_SPARE))"; \
	 test $$(( size + $(UPDATER_SPARE) )) -le $(UPDATER_SLOT) || { echo "the updater does not fit ota_0 with $(UPDATER_SPARE) B spare"; exit 1; }

# The updater's picture (ota/updater/main/picture.h, committed): the
# firmware's "Installing / Don't unplug" with no dot lit, packed by
# opendmd-core (src/updater_picture.rs); `make test` fails when it is stale.
updater-picture:
	cd $(CORE) && RUSTUP_TOOLCHAIN=stable cargo run -q --example updater_picture $(CURDIR)/ota/updater/main/picture.h

# A firmware update over WiFi (docs/ota.md): PUT the signed bundle to
# /api/firmware, then wait for the device to come back running it.
# make fw-ota HOST=opendmd.local [BUNDLE=dist/opendmd-<v>-4mb.odfw];
# with an API key set on the device, OPENDMD_API_KEY=<key> in the
# environment. A bundle that is not newer than the device's firmware is
# refused here unless OPENDMD_ALLOW_OLDER=1 (pass the FW_VERSION you
# released, or BUNDLE=); one for another flash layout always is.
BUNDLE ?= $(FW_BUNDLE)
fw-ota:
	@test -n "$(HOST)" || { echo "usage: make fw-ota HOST=<ip-or-name> [BUNDLE=$(FW_BUNDLE)]"; exit 2; }
	tools/fw-ota.sh $(HOST) $(BUNDLE)

# The browser flasher (flasher/index.html, ESP Web Tools) with the dist/ set
# from `make fw-release`, on http://localhost:8765: Web Serial needs a secure context, and
# localhost is one. Chrome or Edge.
FLASHER_PORT ?= 8765
FLASHER_SITE := target/flasher-site
flasher:
	@for v in $(FW_VARIANTS); do for f in manifest-$$v.json manifest-$$v-offline.json; do test -f $(DIST)/$$f || { echo "no $(DIST)/$$f: run make fw-release first"; exit 2; }; done; done
	rm -rf $(FLASHER_SITE) && mkdir -p $(FLASHER_SITE)/firmware
	cp flasher/index.html flasher/offline.js $(FLASHER_SITE)/
	cp -r flasher/esp-web-tools $(FLASHER_SITE)/ && cp licenses/THIRD-PARTY-LICENSES-installer.txt $(FLASHER_SITE)/esp-web-tools/
	cp $(DIST)/updater.bin $(DIST)/ota_data_initial.bin $(FW_WWW_IMAGE) $(DIST)/THIRD-PARTY-LICENSES-firmware.txt $(DIST)/THIRD-PARTY-LICENSES-web.txt $(FLASHER_SITE)/firmware/
	for v in $(FW_VARIANTS); do cp $(DIST)/manifest-$$v.json $(DIST)/manifest-$$v-offline.json $(DIST)/nvs-offline-$$v.bin $(DIST)/bootloader-$$v.bin $(DIST)/partition-table-$$v.bin \
	  $(DIST)/opendmd-$(FW_VERSION)-$$v-app.bin $(DIST)/opendmd-$(FW_VERSION)-$$v-full.bin $(FLASHER_SITE)/firmware/ || exit 1; done
	@echo "open http://localhost:$(FLASHER_PORT)/ in Chrome or Edge"
	uv run python -m http.server $(FLASHER_PORT) --bind 127.0.0.1 -d $(FLASHER_SITE)

# Headless Chromium: the flasher page loads, ESP Web Tools renders its
# button, both manifests' parts are served, the offline box swaps the
# button's manifest (the flash itself needs a person). DIST=<a dist>.
flasher-smoke:
	DIST=$(DIST) uv run tools/flasher-smoke.py

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

# The emulator lane (tools/emu/README.md): esp32sim at a pinned commit under
# EMU_DIR, a rootless FAT32 card, a boot of the real full flash image.
# Correctness only, never timing. Safe to run any time: it never touches a device.
EMU_DIR ?= $(if $(SCRATCH),$(SCRATCH)/opendmd-emu-sim,$(CURDIR)/target/emu-sim)
# The updater's idf.py build is tied to one checkout: one build dir per checkout (a hash of its path).
EMU_UPDATER ?= $(EMU_DIR)/updater-$(shell printf '%s' '$(CURDIR)' | md5sum | cut -c1-8)
export EMU_DIR SCRATCH
emu-build:
	tools/emu/build-emulator.sh

emu-card:
	uv run --quiet tools/emu/mkcard.py

# A test panel's scale (~11,000 GIFs in 41 folders, ~11,000 copies in ~4,900 buckets):
# the FAT index's build must finish with its heap floor reported, the panel
# playing. Boots the smoke image (or FW_IMAGE). EMU_RIG_OK.
emu-rig-card:
	uv run --quiet tools/emu/mkcard.py --rig
emu-rig: emu-build emu-rig-card
	@start=$$(date +%s) && $(if $(FW_IMAGE),,$(MAKE) --no-print-directory emu-image-smoke &&) \
	uv run --quiet tools/emu/run.py rig --image $(or $(FW_IMAGE),$(EMU_SMOKE_IMAGE)) $(if $(FW_IMAGE),,--built-after $$start) $(EMU_ARGS)
# The leak lane: an `emu-census` image (the heap's census every 20 s) plays
# 70 GIFs (on to 150 while the halves differ); blocks in use and bytes
# allocated stay flat after the first 10.
# EMU_LEAK_OK.
EMU_LEAK_IMAGE = $(EMU_DIR)/leak/dist/opendmd-$(FW_VERSION)-4mb-full.bin
emu-image-leak:
	@test -f $(WWW_PACK) || $(MAKE) --no-print-directory web
	rm -f $(EMU_DIR)/leak/dist/opendmd-*-4mb-full.bin $(EMU_DIR)/leak/dist/opendmd-*-4mb-app.bin
	@# (A diagnostic image, never a release: the census is ~7.7 KB of it, so
	@# it is held to 32 KB of spare, not the release's 64; the probes' are 8.)
	$(MAKE) --no-print-directory fw-release-variant VARIANT=4mb FW_RELEASE_FEATURES=emu-census FW_APP_SPARE=32768 FW_RELEASE_TARGET_DIR=$(EMU_DIR)/leak/target DIST=$(EMU_DIR)/leak/dist UPDATER_BUILD=$(EMU_UPDATER)
emu-leak: emu-build emu-card
	@start=$$(date +%s) && $(if $(FW_IMAGE),,$(MAKE) --no-print-directory emu-image-leak &&) \
	uv run --quiet tools/emu/run.py leak --image $(or $(FW_IMAGE),$(EMU_LEAK_IMAGE)) $(if $(FW_IMAGE),,--built-after $$start) $(EMU_ARGS)
# The API lane: the leak lane's `emu-census` image with WiFi credentials
# seeded into a copy's NVS, on the emulator's virtual AP with port 80 on
# 127.0.0.1, inside a private network namespace (nothing reaches a real
# network); the HTTP API driven from the host over two boots: uploads,
# listings, moves, deletes, settings, play, skip/pause, a www pack, bursts of
# simultaneous uploads (a test board's wedge recipe), five-connection load
# rounds with the heap census flat, a recursive /cache delete during the FAT
# index build, OTA refusals, the card checked after (fatcheck, the settings
# file, the bytes), then the signed bundle through the updater. EMU_API_OK.
# FW_IMAGE=<full.bin> EMU_API_BUNDLE=<.odfw> boot a given image.
EMU_API_BUNDLE ?= $(EMU_DIR)/leak/dist/opendmd-$(FW_VERSION)-4mb.odfw
EMU_API_FORMULA ?= $(PSI)/formulas/plasma.psi
EMU_API_PSB = $(EMU_DIR)/api/formula.psb.b64
# A long Formula (22 KB of text, past any path): Show on panel's bytes.
EMU_API_LONG ?= $(WEB)/experiments/road-v2/road-v2.psi
EMU_API_LONG_PSB = $(EMU_DIR)/api/long.psb
# A Formula that can be a look (`look` line, lets the GIF through): the
# look's rebuild when its .psb is written, and its end when the .psi goes.
EMU_API_LOOK ?= $(WEB)/formulas/vignette.psi
EMU_API_LOOK_PSB = $(EMU_DIR)/api/look.psb
# The governor step's Formulas (heavy and light `smooth`), base64url: Formula over Formula.
EMU_API_HEAVY_PSB = $(EMU_DIR)/api/gov-heavy.psb.b64
EMU_API_LIGHT_PSB = $(EMU_DIR)/api/gov-light.psb.b64
emu-api-psb:
	@mkdir -p $(EMU_DIR)/api
	cd $(PSICLI) && RUSTUP_TOOLCHAIN=stable cargo run -q --release -- build $(abspath $(EMU_API_FORMULA)) --base64 > $(EMU_API_PSB).$$$$ \
	  && mv $(EMU_API_PSB).$$$$ $(EMU_API_PSB)
	cd $(PSICLI) && RUSTUP_TOOLCHAIN=stable cargo run -q --release -- build $(abspath $(EMU_API_LONG)) -o $(EMU_API_LONG_PSB)
	cd $(PSICLI) && RUSTUP_TOOLCHAIN=stable cargo run -q --release -- build $(abspath $(EMU_API_LOOK)) -o $(EMU_API_LOOK_PSB)
	cd $(PSICLI) && RUSTUP_TOOLCHAIN=stable cargo run -q --release -- build $(abspath tools/emu/gov-heavy.psi) --base64 > $(EMU_API_HEAVY_PSB).$$$$ \
	  && mv $(EMU_API_HEAVY_PSB).$$$$ $(EMU_API_HEAVY_PSB)
	cd $(PSICLI) && RUSTUP_TOOLCHAIN=stable cargo run -q --release -- build $(abspath tools/emu/gov-light.psi) --base64 > $(EMU_API_LIGHT_PSB).$$$$ \
	  && mv $(EMU_API_LIGHT_PSB).$$$$ $(EMU_API_LIGHT_PSB)
# The pack it pushes is this build's, signed (PUT /api/www takes no other).
EMU_API_WWW = $(EMU_DIR)/api/www.pack
# Boot C's updater: one from before the panel and table v4 (OLD_UPDATER_TAG's
# ota/updater, "updater 1", tables v3 and v2 only), built once from `git
# archive` into $(EMU_DIR)/old-updater, never the release's: in a v4 image's
# ota_0 it must refuse the v4 bundle at its magic and boot the firmware.
OLD_UPDATER_TAG ?= v1.0.0-beta.16
EMU_OLD_UPDATER = $(EMU_DIR)/old-updater/$(OLD_UPDATER_TAG)/build/opendmd-updater.bin
emu-old-updater:
	@if [ ! -f $(EMU_OLD_UPDATER) ]; then \
	  d=$(EMU_DIR)/old-updater/$(OLD_UPDATER_TAG); rm -rf $$d && mkdir -p $$d/src && \
	  git archive $(OLD_UPDATER_TAG) ota/updater firmware/partitions.csv | tar -x -C $$d/src && \
	  cd $$d/src/ota/updater && env $(UPDATER_ENV) python $(UPDATER_IDF)/tools/idf.py \
	    -B $$d/build -DSDKCONFIG=$$d/sdkconfig build > $$d/build.log 2>&1 || { tail -20 $$d/build.log; exit 1; }; \
	fi
	@echo "old updater ($(OLD_UPDATER_TAG)): $(EMU_OLD_UPDATER)"
emu-api: emu-build emu-card emu-api-psb emu-old-updater
	@test -f $(WWW_PACK) || $(MAKE) --no-print-directory web
	uv run --quiet tools/fw-sign.py sign-www $(WWW_PACK) --out $(EMU_API_WWW) --key $(FW_SIGN_KEY)
	@start=$$(date +%s) && $(if $(FW_IMAGE),,$(MAKE) --no-print-directory emu-image-leak &&) \
	uv run --quiet tools/emu/run.py api --image $(or $(FW_IMAGE),$(EMU_LEAK_IMAGE)) $(if $(FW_IMAGE),,--built-after $$start) \
	  --www-pack $(EMU_API_WWW) --bundle $(EMU_API_BUNDLE) --psb $(EMU_API_PSB) --long-psb $(EMU_API_LONG_PSB) --look $(EMU_API_LOOK) --look-psb $(EMU_API_LOOK_PSB) --heavy-psb $(EMU_API_HEAVY_PSB) --light-psb $(EMU_API_LIGHT_PSB) --formula $(EMU_API_FORMULA) --old-updater $(EMU_OLD_UPDATER) $(EMU_ARGS)
# The same with a test board's heap: an `emu-ballast` image keeps ~45 KB free
# while playing (a test board, 2026-11-03). EMU_RIG_OK.
EMU_RIGTIGHT_IMAGE = $(EMU_DIR)/rigtight/dist/opendmd-$(FW_VERSION)-4mb-full.bin
emu-image-rigtight:
	@test -f $(WWW_PACK) || $(MAKE) --no-print-directory web
	rm -f $(EMU_DIR)/rigtight/dist/opendmd-*-4mb-full.bin $(EMU_DIR)/rigtight/dist/opendmd-*-4mb-app.bin
	$(MAKE) --no-print-directory fw-release-variant VARIANT=4mb FW_RELEASE_FEATURES=emu-ballast FW_RELEASE_TARGET_DIR=$(EMU_DIR)/rigtight/target DIST=$(EMU_DIR)/rigtight/dist UPDATER_BUILD=$(EMU_UPDATER)
emu-rig-tight: emu-build emu-rig-card
	@start=$$(date +%s) && $(if $(FW_IMAGE),,$(MAKE) --no-print-directory emu-image-rigtight &&) \
	uv run --quiet tools/emu/run.py rig --image $(or $(FW_IMAGE),$(EMU_RIGTIGHT_IMAGE)) $(if $(FW_IMAGE),,--built-after $$start) $(EMU_ARGS)

# FW_IMAGE=<full.bin> boots that image; otherwise a normal 4mb release build
# (incremental) goes to $(EMU_DIR)/smoke/.
EMU_SMOKE_IMAGE = $(EMU_DIR)/smoke/dist/opendmd-$(FW_VERSION)-4mb-full.bin
emu-image-smoke:
	@test -f $(WWW_PACK) || $(MAKE) --no-print-directory web
	rm -f $(EMU_DIR)/smoke/dist/opendmd-*-4mb-full.bin $(EMU_DIR)/smoke/dist/opendmd-*-4mb-app.bin
	$(MAKE) --no-print-directory fw-release-variant VARIANT=4mb FW_RELEASE_TARGET_DIR=$(EMU_DIR)/smoke/target DIST=$(EMU_DIR)/smoke/dist UPDATER_BUILD=$(EMU_UPDATER)
emu-smoke: emu-build emu-card
	@start=$$(date +%s) && $(if $(FW_IMAGE),,$(MAKE) --no-print-directory emu-image-smoke &&) \
	uv run --quiet tools/emu/run.py smoke --image $(or $(FW_IMAGE),$(EMU_SMOKE_IMAGE)) $(if $(FW_IMAGE),,--built-after $$start) $(EMU_ARGS)

# The offline lane (tools/emu/offline_lane.py): the emu-census image (the API
# lane's; with WiFi off it takes one card Formula as short at first, so a
# retry compiles on the network worker) with WiFi off (NVS, then
# the card's one-line opendmd.json), the card's Formulas compiled at mount, no WiFi or
# httpd for 700 emulated s, then Connect to Wi-Fi over Improv joining and restarting
# online. EMU_OFFLINE_OK.
EMU_OFFLINE_HOST = $(EMU_DIR)/offline/host-psb
emu-offline-psb:
	@mkdir -p $(EMU_OFFLINE_HOST)
	cd $(PSICLI) && for f in $(abspath tools/emu/offline)/*.psi; do \
	  RUSTUP_TOOLCHAIN=stable cargo run -q --release -- build $$f -o $(EMU_OFFLINE_HOST)/$$(basename $$f .psi).psb 2>/dev/null || true; done
emu-offline: emu-build emu-card emu-offline-psb
	@start=$$(date +%s) && $(if $(FW_IMAGE),,$(MAKE) --no-print-directory emu-image-leak &&) \
	uv run --quiet tools/emu/run.py offline --image $(or $(FW_IMAGE),$(EMU_LEAK_IMAGE)) $(if $(FW_IMAGE),,--built-after $$start) --host-psb $(EMU_OFFLINE_HOST) $(EMU_ARGS)

# The no-card lane: an image with the `nocard-test` feature (src/nocard_test.rs)
# booted with no card (an empty card image): it must boot, run its panel, keep
# its settings in NVS and keep a change across a restart. EMU_NOCARD_OK.
EMU_NOCARD_IMAGE = $(EMU_DIR)/nocard/dist/opendmd-$(FW_VERSION)-4mb-full.bin
emu-image-nocard:
	@test -f $(WWW_PACK) || $(MAKE) --no-print-directory web
	rm -f $(EMU_DIR)/nocard/dist/opendmd-*-4mb-full.bin $(EMU_DIR)/nocard/dist/opendmd-*-4mb-app.bin
	@# (A test image, never a release: the no-card hooks add a few KB, so it
	@# is held to 32 KB of spare like the census image, not the release's 64.)
	$(MAKE) --no-print-directory fw-release-variant VARIANT=4mb FW_RELEASE_FEATURES=nocard-test FW_APP_SPARE=32768 FW_RELEASE_TARGET_DIR=$(EMU_DIR)/nocard/target DIST=$(EMU_DIR)/nocard/dist UPDATER_BUILD=$(EMU_UPDATER)
emu-nocard: emu-build
	@start=$$(date +%s) && $(if $(FW_IMAGE),,$(MAKE) --no-print-directory emu-image-nocard &&) \
	uv run --quiet tools/emu/run.py nocard --image $(or $(FW_IMAGE),$(EMU_NOCARD_IMAGE)) $(if $(FW_IMAGE),,--built-after $$start) $(EMU_ARGS)

# The ESP32-S3 preview (VARIANT=s3, S3 port phase 0) in esp32sim's S3 machine
# (`--chip s3 --board none`, 8 MB flash, 8 MB octal PSRAM): correctness only,
# like every emulator lane. The machine has no SD card yet (the SD-over-SPI
# card is a classic ESP32 board), so both lanes run with none:
# emu-s3-nocard: the nocard-test image boots, its panel driver comes up (the
#   LCD_CAM backend; its output goes nowhere), its settings live in NVS and
#   a change survives a restart. EMU_NOCARD_OK.
# emu-s3-probe: the psi-probe image runs the probe's card-free sections
#   (psi_probe::run_nocard): every bundled Formula, the JIT spike's
#   hand-written kernels, the golden frames through the VM and through the
#   JIT's kernels, bit-identical. EMU_PROBE_OK.
# emu-image-s3: the plain s3 build (the release feature set), for its size.
# Each image has a target dir of its own (a feature set's sdkconfig differs).
EMU_S3_IMAGE = $(EMU_DIR)/s3/dist/opendmd-$(FW_VERSION)-s3-full.bin
EMU_S3_NOCARD_IMAGE = $(EMU_DIR)/s3nocard/dist/opendmd-$(FW_VERSION)-s3-full.bin
EMU_S3_PROBE_IMAGE = $(EMU_DIR)/s3probe/dist/opendmd-$(FW_VERSION)-s3-full.bin
# $(1): the image's dir under EMU_DIR; $(2): its features; $(3): its spare.
EMU_S3_BUILD = $(MAKE) --no-print-directory fw-release-variant VARIANT=s3 $(if $(2),FW_RELEASE_FEATURES=$(2) )FW_APP_SPARE=$(3) FW_RELEASE_TARGET_DIR=$(EMU_DIR)/$(1)/target DIST=$(EMU_DIR)/$(1)/dist
emu-image-s3:
	@test -f $(WWW_PACK) || $(MAKE) --no-print-directory web
	rm -f $(EMU_DIR)/s3/dist/opendmd-*-s3-full.bin $(EMU_DIR)/s3/dist/opendmd-*-s3-app.bin
	$(call EMU_S3_BUILD,s3,,$(FW_APP_SPARE))
emu-image-s3-nocard:
	@test -f $(WWW_PACK) || $(MAKE) --no-print-directory web
	rm -f $(EMU_DIR)/s3nocard/dist/opendmd-*-s3-full.bin $(EMU_DIR)/s3nocard/dist/opendmd-*-s3-app.bin
	$(call EMU_S3_BUILD,s3nocard,nocard-test,32768)
emu-image-s3-probe:
	@test -f $(WWW_PACK) || $(MAKE) --no-print-directory web
	rm -f $(EMU_DIR)/s3probe/dist/opendmd-*-s3-full.bin $(EMU_DIR)/s3probe/dist/opendmd-*-s3-app.bin
	$(call EMU_S3_BUILD,s3probe,psi-probe,$(EMU_PROBE_SPARE))
emu-s3-nocard: emu-build
	@start=$$(date +%s) && $(if $(FW_IMAGE),,$(MAKE) --no-print-directory emu-image-s3-nocard &&) \
	uv run --quiet tools/emu/run.py nocard --chip s3 --image $(or $(FW_IMAGE),$(EMU_S3_NOCARD_IMAGE)) $(if $(FW_IMAGE),,--built-after $$start) $(EMU_ARGS)
emu-s3-probe: emu-build
	@start=$$(date +%s) && $(if $(FW_IMAGE),,$(MAKE) --no-print-directory emu-image-s3-probe &&) \
	uv run --quiet tools/emu/run.py probe --chip s3 --image $(or $(FW_IMAGE),$(EMU_S3_PROBE_IMAGE)) $(if $(FW_IMAGE),,--built-after $$start) --log $(EMU_DIR)/logs/s3-probe-last.log $(EMU_ARGS) && \
	uv run --quiet tools/emu/probe_red.py --chip s3 --image $(or $(FW_IMAGE),$(EMU_S3_PROBE_IMAGE)) --log $(EMU_DIR)/logs/s3-probe-last.log

# A psi-probe image (the psi-probe feature alone, never `probe`) to $(EMU_DIR)/probe/.
# The emulator-only emu8mb layout (firmware/partitions-emu.csv): the probe
# outgrew table v3's 4mb factory partition (2,664,032 B of 2,621,440,
# 2026-11-06; v4's is 2,883,584).
# EMU_PROBE_SPARE (64 KB, as for a release) of its 3 MB factory.
# FW_IMAGE=... skips the build.
EMU_PROBE_VARIANT ?= $(FW_EMU_VARIANT)
EMU_PROBE_SPARE ?= 65536
EMU_PROBE_IMAGE = $(EMU_DIR)/probe/dist/opendmd-$(FW_VERSION)-$(EMU_PROBE_VARIANT)-full.bin
emu-image-probe:
	@test -f $(WWW_PACK) || $(MAKE) --no-print-directory web
	rm -f $(EMU_DIR)/probe/dist/opendmd-*-full.bin $(EMU_DIR)/probe/dist/opendmd-*-app.bin
	$(MAKE) --no-print-directory fw-release-variant VARIANT=$(EMU_PROBE_VARIANT) FW_RELEASE_FEATURES=psi-probe FW_APP_SPARE=$(EMU_PROBE_SPARE) FW_RELEASE_TARGET_DIR=$(EMU_DIR)/probe/target DIST=$(EMU_DIR)/probe/dist UPDATER_BUILD=$(EMU_UPDATER)
emu-probe: emu-build emu-card
	@start=$$(date +%s) && $(if $(FW_IMAGE),,$(MAKE) --no-print-directory emu-image-probe &&) \
	uv run --quiet tools/emu/run.py probe --image $(or $(FW_IMAGE),$(EMU_PROBE_IMAGE)) $(if $(FW_IMAGE),,--built-after $$start) $(EMU_ARGS)
# The paint probe (firmware/src/paint_probe.rs, the paint-probe feature: the paint steps,
# the overlay, `under(x, y)`, the Formulas that use them, each beside the cost
# model's estimate) in a test board's 4mb layout: its .odfw in $(EMU_DIR)/paint/dist
# goes to a test board by OTA; `make emu-paint-probe` boots it in the emulator
# (its paint line: each case's kernel frames against the VM's; correctness only).
# (Its version says what it is: $(FW_VERSION)-paint-<commit>, -paint80- for a
# FW_FLASH_FREQ=80 build, the devices' clock; the descriptor holds 31
# characters with " 4mb", so 5 of the commit's.)
EMU_PAINT_VERSION ?= $(FW_VERSION)-paint$(filter 80,$(FW_FLASH_FREQ))-$(shell git rev-parse --short=5 HEAD)
EMU_PAINT_IMAGE = $(EMU_DIR)/paint/dist/opendmd-$(EMU_PAINT_VERSION)-4mb-full.bin
emu-image-paint:
	@test -f $(WWW_PACK) || $(MAKE) --no-print-directory web
	rm -f $(EMU_DIR)/paint/dist/opendmd-*-4mb-full.bin $(EMU_DIR)/paint/dist/opendmd-*-4mb-app.bin
	$(MAKE) --no-print-directory fw-release-variant FW_VERSION=$(EMU_PAINT_VERSION) VARIANT=4mb FW_RELEASE_FEATURES=paint-probe FW_APP_SPARE=8192 FW_RELEASE_TARGET_DIR=$(EMU_DIR)/paint/target DIST=$(EMU_DIR)/paint/dist UPDATER_BUILD=$(EMU_UPDATER)
emu-paint-probe: emu-build emu-card
	@start=$$(date +%s) && $(if $(FW_IMAGE),,$(MAKE) --no-print-directory emu-image-paint &&) \
	uv run --quiet tools/emu/run.py probe --image $(or $(FW_IMAGE),$(EMU_PAINT_IMAGE)) $(if $(FW_IMAGE),,--built-after $$start) $(EMU_ARGS)
