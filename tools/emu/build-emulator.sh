#!/bin/bash
# Fetch our esp32sim fork (branch `opendmd`: upstream a57c5e0a, merged 2026-11-12, + the SD card in SPI mode
# via --sd/--sd-spi) at a pinned commit, build it.
# Everything lands in $EMU_DIR (the Makefile's default: $SCRATCH/opendmd-emu-sim when SCRATCH is set,
# else target/emu-sim); cached by the pin.
set -euo pipefail
HERE=$(cd "$(dirname "$0")" && pwd)
EMU_DIR=${EMU_DIR:-${SCRATCH:+$SCRATCH/opendmd-emu-sim}}
EMU_DIR=${EMU_DIR:-$(cd "$HERE/../.." && pwd)/target/emu-sim}
ESP32SIM_REPO=${ESP32SIM_REPO:-https://github.com/opendmd/esp32sim}
ESP32SIM_COMMIT=${ESP32SIM_COMMIT:-a0a15f0758cab12eb005e4204d87b52ebaaf1bfb}
SRC=$EMU_DIR/src
BIN=$SRC/target/release/esp32sim
STAMP=$EMU_DIR/built.stamp
want="$ESP32SIM_REPO $ESP32SIM_COMMIT"

# With SCRATCH set (local.mk), third-party code builds only under it.
if [ -n "${SCRATCH:-}" ]; then
  case "$EMU_DIR" in "${SCRATCH%/}"/*) ;; *) [ -n "${EMU_ALLOW_ANYWHERE:-}" ] || { echo "EMU_DIR=$EMU_DIR: third-party code builds only under SCRATCH=$SCRATCH (EMU_ALLOW_ANYWHERE=1 overrides)"; exit 2; };; esac
fi

if [ -x "$BIN" ] && [ "$(cat "$STAMP" 2>/dev/null)" = "$want" ]; then
  echo "emulator up to date: $BIN ($want)"; exit 0
fi
mkdir -p "$EMU_DIR"
if [ ! -d "$SRC/.git" ]; then
  git clone --quiet "$ESP32SIM_REPO" "$SRC"
fi
git -C "$SRC" remote set-url origin "$ESP32SIM_REPO"
git -C "$SRC" fetch --quiet origin "$ESP32SIM_COMMIT" 2>/dev/null || git -C "$SRC" fetch --quiet origin opendmd
git -C "$SRC" checkout --quiet --force "$ESP32SIM_COMMIT"
git -C "$SRC" clean -fdq -e target
(cd "$SRC" && RUSTUP_TOOLCHAIN=stable cargo build --release --bin esp32sim) 2>&1 | tee "$EMU_DIR/build.log" | tail -5
test -x "$BIN"
echo "$want" > "$STAMP"
echo "emulator built: $BIN"
