#!/bin/bash
# Fetch our esp32sim fork (branch `opendmd`: upstream 2e36224 + the SD-SPI board) at a pinned commit, build it.
# Everything lands in $EMU_DIR (default ~/scratch/opendmd-emu-sim); cached by the pin.
set -euo pipefail
HERE=$(cd "$(dirname "$0")" && pwd)
EMU_DIR=${EMU_DIR:-~/scratch/opendmd-emu-sim}
ESP32SIM_REPO=${ESP32SIM_REPO:-https://github.com/opendmd/esp32sim}
ESP32SIM_COMMIT=${ESP32SIM_COMMIT:-551417f49561f48e2167e65e8fe6b2ac3e218825}
SRC=$EMU_DIR/src
BIN=$SRC/target/release/esp32sim
STAMP=$EMU_DIR/built.stamp
want="$ESP32SIM_REPO $ESP32SIM_COMMIT"

case "$EMU_DIR" in ~/scratch/*) ;; *) [ -n "${EMU_ALLOW_ANYWHERE:-}" ] || { echo "EMU_DIR=$EMU_DIR: third-party code builds only under ~/scratch (EMU_ALLOW_ANYWHERE=1 overrides)"; exit 2; };; esac

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
