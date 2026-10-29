#!/bin/sh
# Assemble the spike's kernels into raw, position-independent bytes
# (`include_bytes!` in psi_probe.rs). Needs ESP-IDF's GNU binutils.
set -e
B=${XTENSA_BIN:-$HOME/.espressif/tools/xtensa-esp-elf/esp-13.2.0_20240530/xtensa-esp-elf/bin}
cd "$(dirname "$0")"
for k in tiny call floor; do
  "$B/xtensa-esp32-elf-as" -o "$k.o" "$k.s"
  "$B/xtensa-esp32-elf-objcopy" -O binary -j .text "$k.o" "$k.bin"
  "$B/xtensa-esp32-elf-objdump" -d "$k.o" | sed -n '/>:/,$p'
  rm -f "$k.o"
done
ls -l ./*.bin
