# OpenDMD. Host crates (core + simulator) build on any host with stable Rust;
# the ESP32 firmware in firmware/ needs the xtensa toolchain.

CORE   := opendmd-core
RENDER := opendmd-render
SIM    := opendmd-sim
WEB    := opendmd-web
ART    := opendmd-art-web
PACK   := opendmd-pack
PSI    := opendmd-psi
PSIWEB := opendmd-psi-web
PSICLI := opendmd-psi-cli
GAL    := opendmd-gallery
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

# The flash layouts, one firmware image each (docs/ota.md): `4mb`, layout v2
# for the ESP32 DevKit V1 (firmware/partitions.csv), and `8mb` for the
# 8 MB board in development (firmware/partitions-8mb.csv). Both:
# otadata, the firmware in factory, the updater in ota_0, the web app's www
# pack; the 8 MB one bigger. VARIANT=4mb|8mb picks one for the targets that
# build, flash or update a board (default 4mb); `make fw-release` builds
# both unless given one. Keep in step with the CSVs.
FW_VARIANTS := 4mb 8mb
VARIANT ?=
FW_VARIANT := $(or $(VARIANT),4mb)
ifeq ($(filter $(FW_VARIANT),$(FW_VARIANTS)),)
$(error VARIANT=$(VARIANT): the flash layouts are $(FW_VARIANTS))
endif
FW_PARTITIONS_4mb := $(FW)/partitions.csv
FW_PARTITIONS_8mb := $(FW)/partitions-8mb.csv
FW_FLASH_BYTES_4mb := 4194304
FW_FLASH_BYTES_8mb := 8388608
FW_APP_BYTES_4mb := 2359296
FW_APP_BYTES_8mb := 3145728
UPDATER_OFFSET_4mb := 0x260000
UPDATER_OFFSET_8mb := 0x320000
WWW_OFFSET_4mb := 0x2C0000
WWW_OFFSET_8mb := 0x380000
WWW_BYTES_4mb := 1310720
WWW_BYTES_8mb := 2097152
# What ESP Web Tools' manifest calls the firmware: it must equal the Improv
# answer's name (firmware/src/improv.rs FIRMWARE_NAME), per layout.
FW_MANIFEST_NAME_4mb := OpenDMD
FW_MANIFEST_NAME_8mb := OpenDMD 8 MB
FW_PARTITIONS := $(FW_PARTITIONS_$(FW_VARIANT))
FW_FLASH_BYTES := $(FW_FLASH_BYTES_$(FW_VARIANT))
OTADATA_OFFSET := 0xF000
FW_APP_OFFSET := 0x20000
FW_APP_BYTES := $(FW_APP_BYTES_$(FW_VARIANT))
UPDATER_OFFSET := $(UPDATER_OFFSET_$(FW_VARIANT))
WWW_OFFSET := $(WWW_OFFSET_$(FW_VARIANT))
WWW_BYTES := $(WWW_BYTES_$(FW_VARIANT))
# One www pack for both layouts: built to fit the smaller www partition.
WWW_PACK_BYTES := $(WWW_BYTES_4mb)
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

.PHONY: test test-card test-wasm sim sim-build psi-bench psi-check psi check check-wasm web font web-serve web-mock web-smoke fmt clippy clean pack \
	notices notices-check notices-check-web emu-build emu-card emu-smoke emu-probe emu-image-smoke emu-image-probe \
	fw-build fw-sdkconfig-flavour fw-flash fw-monitor fw-capture fw-erase fw-clean fw-clippy fw-fmt \
	fw-release fw-release-variant fw-release-flash fw-sign fw-ota updater www-push flasher flasher-smoke improv-check card-sync api-load screenshots \
	site site-serve gallery-repo-publish site-smoke gallery-tool \
	site-worker-deps site-dev site-worker-test site-dev-smoke site-deploy-check site-deploy site-deploy-built site-d1-migrate release

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
	@echo "==> Testing $(PSI) (Psi formulas; + core's and the simulator's psi feature)"
	cd $(PSI) && RUSTUP_TOOLCHAIN=stable cargo test --features docs,syntax
	cd $(PSI) && RUSTUP_TOOLCHAIN=stable CARGO_PROFILE_RELEASE_OPT_LEVEL=2 cargo test --release --test stack
	@# The compiler's heap as the device compiles (no `syntax`): every bundled
	@# Formula under its budget, a short heap refused, never an abort.
	cd $(PSI) && RUSTUP_TOOLCHAIN=stable cargo test --release --test compile_heap
	cd $(CORE) && RUSTUP_TOOLCHAIN=stable cargo test --features psi
	cd $(SIM) && RUSTUP_TOOLCHAIN=stable cargo test --features psi
	@echo "==> Testing $(PSIWEB) (the web app's Formulas module)"
	cd $(PSIWEB) && RUSTUP_TOOLCHAIN=stable cargo test
	@echo "==> Testing $(PSICLI) (the psi command)"
	cd $(PSICLI) && RUSTUP_TOOLCHAIN=stable cargo test
	@echo "==> Testing $(GAL) (the formula gallery's checker)"
	cd $(GAL) && RUSTUP_TOOLCHAIN=stable cargo test
	tools/gallery-repo-publish-test.sh
	uv run tools/gallery-check-test.py
	uv run tools/gallery-pages-test.py

# The same tests again, compiled to wasm and run under node — the only check
# on this box that the browser path really works. Needs node on PATH.
test-wasm:
	cd $(WEB) && RUSTUP_TOOLCHAIN=stable wasm-pack test --node
	cd $(ART) && RUSTUP_TOOLCHAIN=stable wasm-pack test --node
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
	cd $(CORE) && RUSTUP_TOOLCHAIN=stable cargo check --target $(WASM_TARGET) --features tween,names
	@echo "==> Checking $(RENDER) for $(WASM_TARGET)"
	cd $(RENDER) && RUSTUP_TOOLCHAIN=stable cargo check --target $(WASM_TARGET)
	@echo "==> Checking $(WEB) for $(WASM_TARGET)"
	cd $(WEB) && RUSTUP_TOOLCHAIN=stable cargo check --target $(WASM_TARGET)
	@echo "==> Checking $(ART) for $(WASM_TARGET)"
	cd $(ART) && RUSTUP_TOOLCHAIN=stable cargo check --target $(WASM_TARGET)
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
# only when the Library's Formulas tab opens. The budgets
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
# Settings -> Device -> About links it.
WEB_WWW := $(WEB)/www
web:
	cd $(WEB) && RUSTUP_TOOLCHAIN=stable wasm-pack build --target web --release --out-dir www/pkg
	cd $(ART) && RUSTUP_TOOLCHAIN=stable wasm-pack build --target web --release --out-dir ../$(WEB_WWW)/pkg-art
	cd $(PSIWEB) && RUSTUP_TOOLCHAIN=stable wasm-pack build --target web --release --out-dir ../$(WEB_WWW)/pkg-psi -- --features bare,bump
	rm -f $(WEB_WWW)/pkg/package.json $(WEB_WWW)/pkg/*.d.ts $(WEB_WWW)/pkg/README.md
	rm -f $(WEB_WWW)/pkg-art/package.json $(WEB_WWW)/pkg-art/*.d.ts $(WEB_WWW)/pkg-art/README.md $(WEB_WWW)/pkg-art/.gitignore
	rm -f $(WEB_WWW)/pkg-psi/package.json $(WEB_WWW)/pkg-psi/*.d.ts $(WEB_WWW)/pkg-psi/README.md $(WEB_WWW)/pkg-psi/.gitignore
	@# The bundled Formulas (the -- bundle files in $(WEB)/formulas/; the site's gallery shows them all).
	uv run tools/formulas-json.py $(WEB)/formulas $(WEB_WWW)/formulas.json
	git describe --always --dirty --tags --match 'v*' > $(WEB_WWW)/version.txt
	uv run tools/api-doc-check.py
	rm -f $(WEB_WWW)/api.html $(WEB_WWW)/api.html.gz
	{ cat $(NOTICES_FW_TXT); printf '\n\n'; cat $(NOTICES_WEB_TXT); } > $(WEB_WWW)/licenses.txt
	find $(WEB_WWW) -name '*.gz' -delete
	find $(WEB_WWW) -type f \( -name '*.html' -o -name '*.js' -o -name '*.css' -o -name '*.wasm' -o -name '*.svg' -o -name '*.json' -o -name licenses.txt \) \
		-exec gzip -9 -n -k -f {} \;
	cd $(PACK) && RUSTUP_TOOLCHAIN=stable cargo run --release --quiet -- \
		www ../$(WEB_WWW) --out ../$(WWW_PACK) --version-file ../$(WEB_WWW)/version.txt --max-bytes $(WWW_PACK_BYTES)
	@uv run tools/web-size.py $(WEB_WWW) --pack $(WWW_PACK) --pack-limit $(WWW_PACK_BYTES) --share $(WEB)/share

# The headings' pixel font (Silkscreen, OFL 1.1): fetched from google/fonts at
# a pinned commit, checked, subset and written as committed files —
# www/fonts/silkscreen.woff2 + OFL.txt (packed, and published with the site)
# and the ASCII copy inlined in firmware/web/setup.html. Only needed to
# change the subset; `make web` uses the committed files.
font:
	uv run tools/font-subset.py

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
NOTICES_FW = bash -c 'build=$$(ls -t $(FW_RELEASE_VTARGET)/xtensa-esp32-espidf/release/build/esp-idf-sys-*/out/build/project_description.json 2>/dev/null | head -1) && \
	uv run --quiet tools/notices.py firmware $(NOTICES_ABOUT) --elf $(FW_RELEASE_BINARY) --idf-build "$$(dirname "$${build:-none/x}")" \
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
# does not build one (fw-release needs the xtensa toolchain).
gallery-tool:
	cd $(GAL) && RUSTUP_TOOLCHAIN=stable cargo build --release --locked

site: web sim-build gallery-tool
	@test -f $(DIST)/manifest-4mb.json -a -f $(DIST)/manifest-8mb.json || { echo "no $(DIST)/manifest-{4mb,8mb}.json: the site publishes the release's installer, run make fw-release first"; exit 2; }
	uv run tools/demo-card.py target/demo-card
	uv run tools/site-art.py target/site-art
	uv run tools/site.py $(SITE) --www $(WEB_WWW) --card target/demo-card --art target/site-art --dist $(DIST) \
		--gallery-tool $(GALLERY_TOOL) $(if $(GALLERY),--gallery $(GALLERY)) $(if $(API_ARCHIVE),--api-archive $(API_ARCHIVE)) \
		--api-freeze $(API_FREEZE)

# The site's pages in Chromium (the formula page, the gallery, the API pages and
# the install page), after make site.
site-smoke:
	uv run tools/site-smoke.py $(SITE) --dist $(DIST) --art target/site-art $(if $(SITE_BASE),--base $(SITE_BASE))

site-serve: site
	uv run python -m http.server 8081 -d $(SITE)

# The site on Cloudflare (docs/research/site-cloudflare.md): the opendmd-site
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

site-worker-test: site-worker-deps gallery-tool
	uv run tools/site-worker-test.py $(SITE) $(if $(SITE_BASE),--base $(SITE_BASE))

site-dev-smoke: site-worker-deps gallery-tool
	uv run tools/site-worker-test.py $(SITE) $(if $(SITE_BASE),--base $(SITE_BASE)) --smoke

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
	unset SITE_TEST_GALLERY_SOURCE; $(CF) deploy --message "site $$(git describe --always --tags --match 'v*')"
	@# The gallery store: this build's examples and the public repo's gallery/ now, not at the next cron.
	@curl -fsS -X POST -A 'opendmd-site-deploy/1 (+https://opendmd.dev)' --retry 3 --retry-delay 11 --retry-all-errors \
		https://opendmd.dev/api/gallery/sync -o /dev/null -w 'gallery sync: %{http_code}\n' || echo "gallery sync: not now (the cron runs it within 5 minutes)"

# The public gallery repo's own files (tools/gallery-repo-publish.sh): the
# gallery check and its vendored checker, the READMEs, and dev/api/ (this
# release's frozen API page, from $(SITE), added to the archive). release.yml
# runs it for a tag; by hand only with the owner.
gallery-repo-publish: site
	tools/gallery-repo-publish.sh $(SITE) $(GALLERY_REPO)

# A release from this machine (tools/release.sh: release.yml's steps while
# Actions is unavailable). make release VERSION=1.0.0-beta.12 is a dry run:
# plan, bump (a detached commit + local tag, undone after), build both
# layouts signed with the real key, every check, the assets and the site;
# nothing pushed. PUBLISH=1 then publishes (main, GitHub and origin pushes,
# the GitHub release, the gallery repo, the Cloudflare deploy); RESUME=1
# with it continues a publish that stopped part way.
release:
	@test -n "$(VERSION)" || { echo "make release VERSION=X.Y.Z[-beta.N] [PUBLISH=1 [RESUME=1]]"; exit 2; }
	tools/release.sh $(VERSION) $(if $(PUBLISH),--publish) $(if $(RESUME),--resume)

fmt:
	cd $(CORE) && RUSTUP_TOOLCHAIN=stable cargo fmt
	cd $(RENDER) && RUSTUP_TOOLCHAIN=stable cargo fmt
	cd $(SIM) && RUSTUP_TOOLCHAIN=stable cargo fmt
	cd $(WEB) && RUSTUP_TOOLCHAIN=stable cargo fmt
	cd $(ART) && RUSTUP_TOOLCHAIN=stable cargo fmt
	cd $(PACK) && RUSTUP_TOOLCHAIN=stable cargo fmt
	cd $(PSI) && RUSTUP_TOOLCHAIN=stable cargo fmt
	cd $(PSIWEB) && RUSTUP_TOOLCHAIN=stable cargo fmt
	cd $(GAL) && RUSTUP_TOOLCHAIN=stable cargo fmt

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
	@echo "==> Clippy $(PSI) (+ core's and the simulator's psi feature)"
	cd $(PSI) && RUSTUP_TOOLCHAIN=stable cargo clippy --all-targets -- -D warnings
	cd $(PSI) && RUSTUP_TOOLCHAIN=stable cargo clippy --all-targets --features docs,syntax -- -D warnings
	cd $(PSI) && RUSTUP_TOOLCHAIN=stable cargo clippy --lib --no-default-features -- -D warnings
	cd $(PSI) && RUSTUP_TOOLCHAIN=stable cargo clippy --lib --target $(WASM_TARGET) --no-default-features -- -D warnings
	cd $(CORE) && RUSTUP_TOOLCHAIN=stable cargo clippy --all-targets --features psi -- -D warnings
	cd $(SIM) && RUSTUP_TOOLCHAIN=stable cargo clippy --all-targets --features psi -- -D warnings
	@echo "==> Clippy $(PSIWEB)"
	cd $(PSIWEB) && RUSTUP_TOOLCHAIN=stable cargo clippy --all-targets -- -D warnings
	cd $(PSIWEB) && RUSTUP_TOOLCHAIN=stable cargo clippy --lib --target $(WASM_TARGET) --features bare,bump -- -D warnings
	@echo "==> Clippy $(GAL)"
	cd $(GAL) && RUSTUP_TOOLCHAIN=stable cargo clippy --all-targets -- -D warnings
	@echo "==> Clippy $(PSICLI)"
	cd $(PSICLI) && RUSTUP_TOOLCHAIN=stable cargo clippy --all-targets -- -D warnings

clean:
	cd $(CORE) && cargo clean
	cd $(RENDER) && cargo clean
	cd $(SIM) && cargo clean
	cd $(WEB) && cargo clean
	cd $(ART) && cargo clean
	cd $(PACK) && cargo clean
	cd $(PSI) && cargo clean
	cd $(PSIWEB) && cargo clean
	cd $(GAL) && cargo clean
	cd $(PSICLI) && cargo clean
	rm -rf $(WEB)/www/pkg $(WEB)/www/pkg-art $(WEB)/www/pkg-psi $(WWW_PACK) ./target

# --- ESP32 firmware ---------------------------------------------------------
# First build clones ESP-IDF v5.3 into ~/.espressif (shared between projects,
# unlike stripe-dashboard's in-project .embuild) and takes 5-10 minutes.

# FW_FEATURES=probe (or probe,probe-http) builds the timing instrumentation in
# firmware/src/probe.rs; the default build has none of it. The bench targets
# (fw-build, fw-flash) always add `dev-credentials`, which compiles the
# firmware/.env WiFi seed in; fw-release never does.
FW_FEATURES ?=
FW_DEV_FEATURES = dev-credentials,bench$(if $(FW_FEATURES),$(comma)$(FW_FEATURES))
comma := ,
FW_CARGO_FEATURES = --features $(FW_DEV_FEATURES)

# Every build ends with tools/check-stack-realign.py: a function that moves
# its stack pointer without MOVSP (an over-aligned local) corrupts its
# caller's registers now and then (opendmd-core/src/chan.rs). The diag build's
# std-channel torture test keeps one such function on purpose.
#
# A layout's sdkconfig fragment: its partition table's absolute path, its
# flash size (sdkconfig.defaults says 4 MB; the 8mb fragment overrides it,
# which also sets the bootloader's and the app's image headers), and the app
# descriptor's version as the release sets it ("<version> <variant>",
# `GET /api/firmware`'s `build`), not IDF's default (`git describe`, or "1").
FW_FRAGMENT = { echo 'CONFIG_PARTITION_TABLE_CUSTOM_FILENAME="$(CURDIR)/$(FW_PARTITIONS)"'; \
	$(if $(filter 8mb,$(FW_VARIANT)),echo 'CONFIG_ESPTOOLPY_FLASHSIZE_4MB=n'; echo 'CONFIG_ESPTOOLPY_FLASHSIZE_8MB=y';) \
	echo 'CONFIG_APP_PROJECT_VER_FROM_CONFIG=y'; \
	echo 'CONFIG_APP_PROJECT_VER="$(FW_VERSION) $(FW_VARIANT)"'; }
# The bench builds' (fw-build, fw-flash, fw-clippy; VARIANT=8mb for the
# 8 MB layout, which shares the bench target dir: switching rebuilds IDF).
FW_BENCH_FRAGMENT = $(FW_FRAGMENT) > $(FW)/target/sdkconfig.partitions
# Before a build: any generated sdkconfig under $(1) (a glob) that does not
# carry this build's flash size, app version and partition table (left by
# the other layout, another version, or another checkout sharing the dir) is
# removed, so IDF makes a new one from the fragment instead of keeping its
# old values over it.
FW_SDKCONFIG_GUARD = for cfg in $(1); do \
	  [ -f "$$cfg" ] || continue; \
	  grep -qx 'CONFIG_ESPTOOLPY_FLASHSIZE="$(shell echo $(FW_VARIANT) | tr a-z A-Z)"' "$$cfg" \
	  && grep -qx 'CONFIG_APP_PROJECT_VER="$(FW_VERSION) $(FW_VARIANT)"' "$$cfg" \
	  && grep -qx 'CONFIG_PARTITION_TABLE_CUSTOM_FILENAME="$(CURDIR)/$(FW_PARTITIONS)"' "$$cfg" \
	  || { echo "==> $$cfg is not this build's ($(FW_VARIANT), $(FW_VERSION), $(FW_PARTITIONS)): removing it"; rm -f "$$cfg"; }; \
	done

# Delete the generated sdkconfig when the bench flavour (normal/diag) or the
# version changes: it keeps the values it already has (the descriptor's
# version among them) over a changed fragment.
fw-sdkconfig-flavour:
	@mkdir -p $(FW)/target
	@want="$(if $(FW_DIAG),diag,normal) $(FW_VERSION) $(FW_VARIANT) $$(cat $(FW)/sdkconfig.defaults $(if $(FW_DIAG),$(FW)/sdkconfig.diag) | sha256sum | cut -c1-16)"; have=$$(cat $(FW)/target/sdkconfig.flavour 2>/dev/null); \
	 if [ "$$want" != "$$have" ]; then \
	   echo "==> sdkconfig flavour $${have:-unknown} -> $$want: removing the generated sdkconfig"; \
	   rm -f $(FW)/target/xtensa-esp32-espidf/*/build/esp-idf-sys-*/out/sdkconfig; \
	   echo $$want > $(FW)/target/sdkconfig.flavour; \
	 fi
	@$(call FW_SDKCONFIG_GUARD,$(FW)/target/xtensa-esp32-espidf/*/build/esp-idf-sys-*/out/sdkconfig)

fw-build: fw-sdkconfig-flavour
	@mkdir -p $(FW)/target
	@$(FW_BENCH_FRAGMENT)
	cd $(FW) && bash -c '$(FW_ENV) . $(ESPUP_ENV) && RUSTUP_TOOLCHAIN=esp OPENDMD_FLASH_VARIANT=$(FW_VARIANT) OPENDMD_FW_VERSION="$(FW_VERSION)" ESP_IDF_SDKCONFIG_DEFAULTS="$(FW_SDKCONFIG)" cargo build $(FW_PROFILE_FLAG) $(FW_CARGO_FEATURES)'
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
# docs/deploying.md. VARIANT=8mb for an 8 MB board (an 8 MB board in development).
fw-flash: fw-build web updater
	bash -c '. $(ESPUP_ENV) && espflash write-bin --port $(PORT) --baud 115200 $(WWW_OFFSET) $(WWW_PACK) && espflash write-bin --port $(PORT) --baud 115200 $(UPDATER_OFFSET) $(UPDATER_BIN) && espflash flash --port $(PORT) --baud 115200 --bootloader $(FW_BOOTLOADER) --partition-table $(FW_PARTITIONS) --erase-parts otadata --flash-size $(FW_VARIANT) --monitor --monitor-baud $(MONITOR_BAUD) $(FW_BINARY)'

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
	cd $(FW) && bash -c '$(FW_ENV) . $(ESPUP_ENV) && RUSTUP_TOOLCHAIN=esp OPENDMD_FLASH_VARIANT=$(FW_VARIANT) OPENDMD_FW_VERSION="$(FW_VERSION)" ESP_IDF_SDKCONFIG_DEFAULTS="$(FW_SDKCONFIG)" cargo clippy --all-targets $(FW_CARGO_FEATURES) -- -D warnings'
	@echo "==> Clippy firmware, release feature set (the defaults: psi, jit)"
	cd $(FW) && env -u OPENDMD_WIFI_SSID -u OPENDMD_WIFI_PASS bash -c '. $(ESPUP_ENV) && RUSTUP_TOOLCHAIN=esp OPENDMD_FLASH_VARIANT=$(FW_VARIANT) OPENDMD_FW_VERSION="$(FW_VERSION)" ESP_IDF_SDKCONFIG_DEFAULTS="$(FW_SDKCONFIG)" cargo clippy --all-targets -- -D warnings'
	@echo "==> Clippy firmware, no features (without psi)"
	cd $(FW) && env -u OPENDMD_WIFI_SSID -u OPENDMD_WIFI_PASS bash -c '. $(ESPUP_ENV) && RUSTUP_TOOLCHAIN=esp OPENDMD_FLASH_VARIANT=$(FW_VARIANT) OPENDMD_FW_VERSION="$(FW_VERSION)" ESP_IDF_SDKCONFIG_DEFAULTS="$(FW_SDKCONFIG)" cargo clippy --all-targets --no-default-features -- -D warnings'

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
# Tools) or espflash. It builds both flash layouts (FW_VARIANTS: 4mb, the
# DevKit V1; 8mb, an 8 MB board in development), each with its own sdkconfig and target
# dir; `make fw-release VARIANT=8mb` builds one. Per layout <var>:
#
#   opendmd-<ver>-<var>-app.bin    the app image, flashed at 0x20000 (factory)
#   bootloader-<var>.bin           at 0x1000 (its header carries the flash size)
#   partition-table-<var>.bin      at 0x8000
#   opendmd-<ver>-<var>-full.bin   all six parts merged into one image the size
#                                  of the flash (offset 0)
#   opendmd-<ver>-<var>.odfw       the signed bundle for PUT /api/firmware (fw-sign);
#                                  its magic names the layout
#   manifest-<var>.json            ESP Web Tools: the six parts and offsets; `name`
#                                  (FW_MANIFEST_NAME_<var>) and `version` are what the
#                                  firmware's Improv Serial answer says
#                                  (firmware/src/improv.rs: FIRMWARE_NAME,
#                                  state::firmware_version), which is how the
#                                  installer tells Update from Install; it waits
#                                  `new_install_improv_wait_time` (s) after an install
#                                  for the new firmware to answer, then asks for WiFi
#
# and once, the same for both:
#
#   updater.bin                    the updater (ota/updater), at ota_0 (0x260000 / 0x320000);
#                                  its image header says 4 MB in both: harmless, IDF
#                                  sizes the chip from the bootloader's header
#   ota_data_initial.bin           otadata erased (8 KB of 0xFF) at 0xF000: boots factory
#   opendmd-<ver>-www.pack         the web app, at www (0x2C0000 / 0x380000); built to
#                                  fit the smaller www partition
#   THIRD-PARTY-LICENSES-firmware.txt, THIRD-PARTY-LICENSES-web.txt
#                                  the third-party notices (licenses/, `make notices`),
#                                  checked against each build first
#
# The version comes from firmware/Cargo.toml and is also written into the app
# descriptor as "<version> <var>" (checked below at offset 0x30 of the image).
# See docs/deploying.md.
FW_VERSION := $(shell sed -n 's/^version *= *"\(.*\)"/\1/p' $(FW)/Cargo.toml | head -1)
# The release build wants ~1.5 GB of its own per layout (host build scripts
# plus a second ESP-IDF build); FW_RELEASE_TARGET_DIR=/somewhere/roomier
# moves it (the 8mb build goes in its v8mb/: a layout is a different
# sdkconfig, and sharing one dir would rebuild ESP-IDF at every switch).
# CARGO_WORKSPACE_DIR is set below because embuild otherwise takes the crate
# root to be the directory above the target dir, and with the target dir
# elsewhere it would lose the extra_components (hub75, mdns) in Cargo.toml.
FW_RELEASE_TARGET_DIR ?= $(CURDIR)/$(FW)/target
FW_RELEASE_VTARGET := $(FW_RELEASE_TARGET_DIR)$(if $(filter 4mb,$(FW_VARIANT)),,/v$(FW_VARIANT))
FW_RELEASE_BINARY := $(FW_RELEASE_VTARGET)/xtensa-esp32-espidf/release/opendmd-firmware
FW_RELEASE_BOOTLOADER := $(FW_RELEASE_VTARGET)/xtensa-esp32-espidf/release/bootloader.bin
# The fragment and its stamp live in the layout's own target dir: with a
# private FW_RELEASE_TARGET_DIR nothing a release build writes or removes
# (the generated sdkconfig among it) touches another checkout's build.
FW_RELEASE_SDKCONFIG_FRAG := $(FW_RELEASE_VTARGET)/sdkconfig.release
FW_RELEASE_SDKCONFIG_STAMP := $(FW_RELEASE_VTARGET)/sdkconfig.release.stamp
FW_RELEASE_SDKCONFIG := $(CURDIR)/$(FW)/sdkconfig.defaults;$(FW_RELEASE_SDKCONFIG_FRAG)
# Measurement builds only, never a public release: the release set built
# with these cargo features (e.g. FW_RELEASE_FEATURES=probe,psi-probe
# DIST=/somewhere/else), signed like any other so it can go to the rig over
# the LAN. Empty (the release feature set: none) by default.
FW_RELEASE_FEATURES ?=
DIST := dist
FW_APP_IMAGE := $(DIST)/opendmd-$(FW_VERSION)-$(FW_VARIANT)-app.bin
FW_WWW_IMAGE := $(DIST)/opendmd-$(FW_VERSION)-www.pack
FW_FULL_IMAGE := $(DIST)/opendmd-$(FW_VERSION)-$(FW_VARIANT)-full.bin
FW_DIST_BOOTLOADER := $(DIST)/bootloader-$(FW_VARIANT).bin
FW_DIST_TABLE := $(DIST)/partition-table-$(FW_VARIANT).bin
FW_MANIFEST := $(DIST)/manifest-$(FW_VARIANT).json
# Panic locations name source files: the release names them from neutral
# roots instead of this machine's paths (cargo joins these to the config's
# rustflags).
FW_RELEASE_REMAP := --remap-path-prefix=$(CURDIR)=/opendmd --remap-path-prefix=$(HOME)/.cargo=/cargo --remap-path-prefix=$(HOME)/.rustup=/rustup
# The app image must leave at least this much of the layout's factory
# partition free (each layout against its own: 2,359,296 B or 3,145,728 B).
FW_APP_SPARE := 65536

# Both layouts (or VARIANT=), each through fw-release-variant, after one
# `make web`: the one pack goes with both.
fw-release: web
	@for v in $(or $(VARIANT),$(FW_VARIANTS)); do \
	  $(MAKE) --no-print-directory fw-release-variant VARIANT=$$v || exit 1; \
	done
	@echo "dist/:"; ls -l $(foreach v,$(or $(VARIANT),$(FW_VARIANTS)),$(DIST)/opendmd-$(FW_VERSION)-$(v)-app.bin $(DIST)/opendmd-$(FW_VERSION)-$(v).odfw \
	  $(DIST)/opendmd-$(FW_VERSION)-$(v)-full.bin $(DIST)/bootloader-$(v).bin $(DIST)/partition-table-$(v).bin $(DIST)/manifest-$(v).json) \
	  $(FW_WWW_IMAGE) $(DIST)/updater.bin $(DIST)/ota_data_initial.bin $(DIST)/THIRD-PARTY-LICENSES-firmware.txt $(DIST)/THIRD-PARTY-LICENSES-web.txt

# One layout's release set (FW_VARIANT): needs `make web` done (fw-release).
fw-release-variant:
	@test -n "$(FW_VERSION)" || { echo "cannot read the version from $(FW)/Cargo.toml"; exit 2; }
	@test -f $(WWW_PACK) || { echo "no $(WWW_PACK): run make fw-release (it runs make web first)"; exit 2; }
	@echo "==> Release build, $(FW_VARIANT) ($(FW_PARTITIONS), target $(FW_RELEASE_VTARGET))"
	@mkdir -p $(FW_RELEASE_VTARGET) $(DIST)
	@$(FW_FRAGMENT) > $(FW_RELEASE_SDKCONFIG_FRAG)
	@# The generated sdkconfig keeps the values it has over changed defaults
	@# (a new sdkconfig.defaults line or version would silently not apply):
	@# remove it whenever the inputs change.
	@want=$$(cat $(FW)/sdkconfig.defaults $(FW_RELEASE_SDKCONFIG_FRAG) | sha256sum | cut -c1-16); \
	 have=$$(cat $(FW_RELEASE_SDKCONFIG_STAMP) 2>/dev/null); \
	 if [ "$$want" != "$$have" ]; then \
	   echo "==> release sdkconfig inputs changed ($(FW_VARIANT)): removing the generated sdkconfig"; \
	   rm -f $(FW_RELEASE_VTARGET)/xtensa-esp32-espidf/release/build/esp-idf-sys-*/out/sdkconfig; \
	   echo $$want > $(FW_RELEASE_SDKCONFIG_STAMP); \
	 fi
	@$(call FW_SDKCONFIG_GUARD,$(FW_RELEASE_VTARGET)/xtensa-esp32-espidf/release/build/esp-idf-sys-*/out/sdkconfig)
	cd $(FW) && env -u OPENDMD_WIFI_SSID -u OPENDMD_WIFI_PASS bash -c '. $(ESPUP_ENV) && RUSTUP_TOOLCHAIN=esp CARGO_WORKSPACE_DIR="$(CURDIR)/$(FW)" OPENDMD_FLASH_VARIANT=$(FW_VARIANT) OPENDMD_FW_VERSION="$(FW_VERSION)" ESP_IDF_SDKCONFIG_DEFAULTS="$(FW_RELEASE_SDKCONFIG)" CARGO_TARGET_XTENSA_ESP32_ESPIDF_RUSTFLAGS="$(FW_RELEASE_REMAP)" cargo build --release --target-dir "$(FW_RELEASE_VTARGET)" $(if $(FW_RELEASE_FEATURES),--features $(FW_RELEASE_FEATURES),)'
	@# The generated sdkconfig must carry the layout's flash size: a fragment
	@# that failed to apply would build a 4 MB image under an 8mb name.
	@cfg=$$(ls -t $(FW_RELEASE_VTARGET)/xtensa-esp32-espidf/release/build/esp-idf-sys-*/out/sdkconfig | head -1); \
	 want=$$(echo $(FW_VARIANT) | tr a-z A-Z); \
	 grep -q "^CONFIG_ESPTOOLPY_FLASHSIZE=\"$$want\"" "$$cfg" || { echo "$$cfg: CONFIG_ESPTOOLPY_FLASHSIZE is not \"$$want\""; exit 1; }; \
	 echo "sdkconfig: flash size $$want ($$cfg)"
	@# The updater after the firmware: on a fresh machine (CI) it is the
	@# firmware's build that installs the ESP-IDF checkout idf.py needs.
	$(MAKE) --no-print-directory updater
	bash -c '. $(ESPUP_ENV) && uv run --quiet tools/check-stack-realign.py $(FW_RELEASE_BINARY)'
	@# The third-party notices: the committed ones must be what this build links.
	$(MAKE) --no-print-directory notices-check
	bash -c '. $(ESPUP_ENV) && espflash save-image --chip esp32 --flash-size $(FW_VARIANT) $(FW_RELEASE_BINARY) $(FW_APP_IMAGE)'
	@desc=$$(dd if=$(FW_APP_IMAGE) bs=1 skip=48 count=32 2>/dev/null | tr -d '\000'); \
	 echo "app descriptor version: '$$desc'"; \
	 test "$$desc" = "$(FW_VERSION) $(FW_VARIANT)" || { echo "expected '$(FW_VERSION) $(FW_VARIANT)' at 0x30"; exit 1; }
	@size=$$(stat -c %s $(FW_APP_IMAGE)); \
	 spare=$$(( $(FW_APP_BYTES) - size )); \
	 pct=$$(awk "BEGIN { printf \"%.1f\", $$size * 100 / $(FW_APP_BYTES) }"); \
	 echo "$(FW_APP_IMAGE): $$size B = $$pct% of the $(FW_APP_BYTES) B $(FW_VARIANT) factory partition, $$spare B spare (need $(FW_APP_SPARE))"; \
	 test $$spare -ge $(FW_APP_SPARE) || { echo "the app image leaves less than $(FW_APP_SPARE) B of the $(FW_VARIANT) factory partition"; exit 1; }
	@size=$$(stat -c %s $(WWW_PACK)); \
	 echo "$(WWW_PACK): $$size B of the $(WWW_BYTES) B $(FW_VARIANT) www partition"; \
	 test $$size -le $(WWW_BYTES) || { echo "the www pack does not fit the $(FW_VARIANT) www partition"; exit 1; }
	$(MAKE) --no-print-directory fw-sign
	cp $(WWW_PACK) $(FW_WWW_IMAGE)
	cp $(FW_RELEASE_BOOTLOADER) $(FW_DIST_BOOTLOADER)
	cp $(UPDATER_BIN) $(DIST)/updater.bin
	cp $(UPDATER_BUILD)/build/ota_data_initial.bin $(DIST)/ota_data_initial.bin
	cp $(NOTICES_FW_TXT) $(NOTICES_WEB_TXT) $(DIST)/
	@! strings $(FW_APP_IMAGE) $(DIST)/updater.bin $(FW_DIST_BOOTLOADER) | grep -q '$(HOME)' || { echo "the release names $(HOME): a build path got in"; exit 1; }
	bash -c '. $(ESPUP_ENV) && espflash partition-table --to-binary -o $(FW_DIST_TABLE) $(FW_PARTITIONS)'
	@# The flash size in the image headers (byte 3's high nibble: 2 = 4 MB,
	@# 3 = 8 MB): the bootloader's is what the board runs with.
	@want=$$(case $(FW_VARIANT) in 4mb) echo 2;; 8mb) echo 3;; esac); \
	 for f in $(FW_DIST_BOOTLOADER) $(FW_APP_IMAGE); do \
	   got=$$(dd if=$$f bs=1 skip=3 count=1 status=none | od -An -tu1 | tr -d ' '); \
	   test $$(( got >> 4 )) -eq $$want || { echo "$$f: flash size code $$(( got >> 4 )) in its header, want $$want ($(FW_VARIANT))"; exit 1; }; \
	 done; echo "image headers: flash size $(FW_VARIANT) (bootloader, app)"
	@# The merged image: espflash lays out bootloader, table and app padded to
	@# the flash size with 0xFF (so otadata is erased: the bootloader boots
	@# factory); the updater and the pack go in at their offsets on top.
	bash -c '. $(ESPUP_ENV) && espflash save-image --chip esp32 --flash-size $(FW_VARIANT) --merge --bootloader $(FW_RELEASE_BOOTLOADER) --partition-table $(FW_PARTITIONS) $(FW_RELEASE_BINARY) $(FW_FULL_IMAGE)'
	dd if=$(DIST)/updater.bin of=$(FW_FULL_IMAGE) bs=4096 seek=$$(( $(UPDATER_OFFSET) / 4096 )) conv=notrunc status=none
	dd if=$(FW_WWW_IMAGE) of=$(FW_FULL_IMAGE) bs=4096 seek=$$(( $(WWW_OFFSET) / 4096 )) conv=notrunc status=none
	@test "$$(dd if=$(FW_FULL_IMAGE) bs=4096 skip=$$(( $(FW_APP_OFFSET) / 4096 )) count=1 status=none | head -c 1 | od -An -tx1 | tr -d ' ')" = e9 || { echo "no app image at $(FW_APP_OFFSET) in $(FW_FULL_IMAGE)"; exit 1; }
	@test "$$(dd if=$(FW_FULL_IMAGE) bs=4096 skip=$$(( $(OTADATA_OFFSET) / 4096 )) count=2 status=none | tr -d '\377' | wc -c)" = 0 || { echo "otadata is not erased in $(FW_FULL_IMAGE)"; exit 1; }
	@test $$(stat -c %s $(FW_FULL_IMAGE)) -eq $(FW_FLASH_BYTES) || { echo "$(FW_FULL_IMAGE) is not $(FW_FLASH_BYTES) B"; exit 1; }
	@# The parts in the order ESP Web Tools writes them. 8mb: the updater and
	@# the pack (the parts that reach past 4 MB) first, then the bootloader,
	@# table, otadata and app. On an 8 MB chip the order changes nothing. On a
	@# 4 MB DevKit given this manifest by mistake, the OpenDMD firmware does
	@# not start whatever happens to the writes past 4 MB (the stub may
	@# refuse them, or the chip may wrap them round to the start, which the
	@# later parts would then write over); installing again with the DevKit
	@# manifest and Erase ticked recovers the board. Not tested on a board.
	@boot='        { "path": "bootloader-$(FW_VARIANT).bin", "offset": 4096 }'; \
	 table='        { "path": "partition-table-$(FW_VARIANT).bin", "offset": 32768 }'; \
	 otadata='        { "path": "ota_data_initial.bin", "offset": '$$(( $(OTADATA_OFFSET) ))' }'; \
	 app='        { "path": "opendmd-$(FW_VERSION)-$(FW_VARIANT)-app.bin", "offset": '$$(( $(FW_APP_OFFSET) ))' }'; \
	 upd='        { "path": "updater.bin", "offset": '$$(( $(UPDATER_OFFSET) ))' }'; \
	 www='        { "path": "opendmd-$(FW_VERSION)-www.pack", "offset": '$$(( $(WWW_OFFSET) ))' }'; \
	 { printf '%s\n' '{' \
	  '  "name": "$(FW_MANIFEST_NAME_$(FW_VARIANT))",' \
	  '  "version": "$(FW_VERSION)",' \
	  '  "new_install_prompt_erase": true,' \
	  '  "new_install_improv_wait_time": 15,' \
	  '  "builds": [' \
	  '    {' \
	  '      "chipFamily": "ESP32",' \
	  '      "parts": ['; \
	  sep=; for part in $(if $(filter 8mb,$(FW_VARIANT)),www upd boot table otadata app,boot table otadata app upd www); do \
	    eval "line=\$$$$part"; [ -z "$$sep" ] || printf ',\n'; printf '%s' "$$line"; sep=1; done; echo; \
	  printf '%s\n' '      ]' \
	  '    }' \
	  '  ]' \
	  '}'; } > $(FW_MANIFEST)
	@uv run --quiet python -I -c 'import json, sys; m = json.load(open(sys.argv[1])); assert len(m["builds"][0]["parts"]) == 6' $(FW_MANIFEST) \
	  || { echo "$(FW_MANIFEST) is not the manifest it should be"; exit 1; }
	@echo "==> $(FW_VARIANT) done: $(FW_MANIFEST)"

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
# VARIANT (default 4mb, the DevKit; VARIANT=8mb for an 8 MB board in development). From
# DMDoS, other firmware or an older table: `make fw-erase` first.
fw-release-flash:
	$(MAKE) --no-print-directory fw-release VARIANT=$(FW_VARIANT)
	bash -c '. $(ESPUP_ENV) && espflash write-bin --port $(PORT) --baud 115200 $(WWW_OFFSET) $(FW_WWW_IMAGE) && espflash write-bin --port $(PORT) --baud 115200 $(UPDATER_OFFSET) $(DIST)/updater.bin && espflash flash --port $(PORT) --baud 115200 --bootloader $(FW_RELEASE_BOOTLOADER) --partition-table $(FW_PARTITIONS) --erase-parts otadata --flash-size $(FW_VARIANT) --monitor --monitor-baud $(MONITOR_BAUD) $(FW_RELEASE_BINARY)'

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
# IDF v5.3's GCC (also what tools/notices.py reads the licences and readelf from).
XTENSA_TOOLCHAIN := $(ESPRESSIF)/tools/xtensa-esp-elf/esp-13.2.0_20240530/xtensa-esp-elf
UPDATER_ENV := IDF_PATH=$(UPDATER_IDF) IDF_PYTHON_ENV_PATH=$(UPDATER_PYENV) \
	ESP_ROM_ELF_DIR=$(ESPRESSIF)/tools/esp-rom-elfs/20240305/ \
	PATH=$(UPDATER_PYENV)/bin:$(XTENSA_TOOLCHAIN)/bin:$(ESPRESSIF)/tools/cmake/3.24.0/bin:$(ESPRESSIF)/tools/ninja/1.11.1:$(UPDATER_IDF)/tools:$$PATH
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
# make fw-ota HOST=opendmd.local [VARIANT=8mb | BUNDLE=dist/opendmd-<v>-<var>.odfw];
# with an API key set on the device, OPENDMD_API_KEY=<key> in the
# environment. A bundle that is not newer than the device's firmware is
# refused here unless OPENDMD_ALLOW_OLDER=1 (pass the FW_VERSION you
# released, or BUNDLE=); one for the other flash layout always is.
BUNDLE ?= $(FW_BUNDLE)
fw-ota:
	@test -n "$(HOST)" || { echo "usage: make fw-ota HOST=<ip-or-name> [VARIANT=8mb] [BUNDLE=$(FW_BUNDLE)]"; exit 2; }
	tools/fw-ota.sh $(HOST) $(BUNDLE)

# The browser flasher (flasher/index.html, ESP Web Tools) with the dist/ set
# from `make fw-release` (both layouts: the page offers one Install per
# board), on http://localhost:8765: Web Serial needs a secure context, and
# localhost is one. Chrome or Edge.
FLASHER_PORT ?= 8765
FLASHER_SITE := target/flasher-site
flasher:
	@for v in $(FW_VARIANTS); do test -f $(DIST)/manifest-$$v.json || { echo "no $(DIST)/manifest-$$v.json: run make fw-release first"; exit 2; }; done
	rm -rf $(FLASHER_SITE) && mkdir -p $(FLASHER_SITE)/firmware
	cp flasher/index.html $(FLASHER_SITE)/
	cp $(DIST)/updater.bin $(DIST)/ota_data_initial.bin $(FW_WWW_IMAGE) $(DIST)/THIRD-PARTY-LICENSES-firmware.txt $(DIST)/THIRD-PARTY-LICENSES-web.txt $(FLASHER_SITE)/firmware/
	for v in $(FW_VARIANTS); do cp $(DIST)/manifest-$$v.json $(DIST)/bootloader-$$v.bin $(DIST)/partition-table-$$v.bin \
	  $(DIST)/opendmd-$(FW_VERSION)-$$v-app.bin $(DIST)/opendmd-$(FW_VERSION)-$$v-full.bin $(FLASHER_SITE)/firmware/ || exit 1; done
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

# The emulator lane (tools/emu/README.md): esp32sim at a pinned commit under
# ~/scratch, a rootless FAT32 card, a boot of the real full flash image.
# Correctness only, never timing. Agents may run it freely.
EMU_DIR ?= ~/scratch/opendmd-emu-sim
# The updater's idf.py build is tied to one checkout: one build dir per checkout (a hash of its path).
EMU_UPDATER ?= $(EMU_DIR)/updater-$(shell printf '%s' '$(CURDIR)' | md5sum | cut -c1-8)
export EMU_DIR
emu-build:
	tools/emu/build-emulator.sh

emu-card:
	uv run --quiet tools/emu/mkcard.py

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

# A psi-probe image (the psi-probe feature alone, never `probe`) to $(EMU_DIR)/probe/.
# The 8mb layout by default: the emulator does not care about the DevKit's 4 MB,
# and the full probe does not fit the 4mb factory partition with margin (same
# firmware code, classic ESP32; only the partition table and flash size differ).
# EMU_PROBE_VARIANT=4mb builds the DevKit layout (then EMU_PROBE_SPARE, 8 KB,
# is the rule). FW_IMAGE=... skips the build.
EMU_PROBE_VARIANT ?= 8mb
EMU_PROBE_SPARE ?= 8192
EMU_PROBE_IMAGE = $(EMU_DIR)/probe/dist/opendmd-$(FW_VERSION)-$(EMU_PROBE_VARIANT)-full.bin
emu-image-probe:
	@test -f $(WWW_PACK) || $(MAKE) --no-print-directory web
	rm -f $(EMU_DIR)/probe/dist/opendmd-*-full.bin $(EMU_DIR)/probe/dist/opendmd-*-app.bin
	$(MAKE) --no-print-directory fw-release-variant VARIANT=$(EMU_PROBE_VARIANT) FW_RELEASE_FEATURES=psi-probe FW_APP_SPARE=$(EMU_PROBE_SPARE) FW_RELEASE_TARGET_DIR=$(EMU_DIR)/probe/target DIST=$(EMU_DIR)/probe/dist UPDATER_BUILD=$(EMU_UPDATER)
emu-probe: emu-build emu-card
	@start=$$(date +%s) && $(if $(FW_IMAGE),,$(MAKE) --no-print-directory emu-image-probe &&) \
	uv run --quiet tools/emu/run.py probe --image $(or $(FW_IMAGE),$(EMU_PROBE_IMAGE)) $(if $(FW_IMAGE),,--built-after $$start) $(EMU_ARGS)
