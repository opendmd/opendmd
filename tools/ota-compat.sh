#!/usr/bin/env bash
# Firmware updates across releases and flash layouts (docs/ota.md "Two flash
# layouts", "The key trailer"; docs/www-pack.md "Signature"). An older
# release's own opendmd-core, checked out at its tag, and this tree's run
# tools/fixtures/ota-compat.rs (a 4mb player's checks, in the firmware's
# order) against files signed now:
#
#   the older release (TAG, a DevKit on partition table v2, 2,304 KB factory)
#     refuses a 4mb bundle for table v3 at its header, before the signature
#       (its image is bigger than the v2 factory partition: what must never
#       reach a v2 board's updater),
#     takes a 4mb bundle signed now for table v2 (so the refusal is the
#       layout's, not the signing's), and takes the www pack;
#   this tree (table v3, 2,560 KB factory)
#     takes the table v3 bundle, refuses the table v2 one at its header, and
#     takes the www pack.
#
#   tools/ota-compat.sh [TAG] [BUNDLE PUB WWW]
#
# TAG defaults to v1.0.0-beta.15, the last release on table v2. The bundles
# above are fake 4mb app images signed with a throwaway key (tools/fw-sign.py
# sign --layout), the pack this tree's www pack signed with it. Given BUNDLE
# PUB WWW (a release build's 4mb bundle, the public key, its pack: `make
# ota-compat` passes dist/'s when it has them), those go through both too:
# the bundle refused by TAG and taken here, the pack taken by both. Prints
# "OTA_COMPAT_OK <tag>" when every case comes out as it should.
set -euo pipefail
root=$(cd "$(dirname "$0")/.." && pwd)
tag=${1:-v1.0.0-beta.15}
work=${OTA_COMPAT_DIR:-$root/target/ota-compat}
mkdir -p "$work"
tree="$work/$tag"
if [ ! -d "$tree/opendmd-core" ]; then
  git -C "$root" worktree add --detach "$tree" "$tag" >/dev/null
fi
# The factory partitions: the older release's 4mb table (its own
# partitions.csv) and this tree's.
factory_of() { printf '%d' "$(awk -F, '$1 ~ /^factory/ { gsub(/ /, "", $5); print $5 }' "$1")"; }
old_factory=$(factory_of "$tree/firmware/partitions.csv")
new_factory=$(factory_of "$root/firmware/partitions.csv")
[ "$old_factory" = 2359296 ] || { echo "$tag's factory partition is $old_factory B, not table v2's 2359296" >&2; exit 2; }
[ "$new_factory" = 2621440 ] || { echo "this tree's factory partition is $new_factory B, not table v3's 2621440" >&2; exit 2; }

sign() { uv run --quiet "$root/tools/fw-sign.py" "$@"; }
tmp=$(mktemp -d)
cleanup() { rm -rf "$tmp"; rm -f "$tree/opendmd-core/tests/ota_compat.rs" "$root/opendmd-core/tests/ota_compat.rs"; }
trap cleanup EXIT
sign keygen --key "$tmp/k" --pub "$tmp/p" >/dev/null
fake() {  # fake <bytes> <out>: an app image, 0xE9 first, a 4mb app descriptor
  uv run --quiet python -I -c '
import os, struct, sys
img = bytearray(b"\xE9" + os.urandom(int(sys.argv[1]) - 1))
struct.pack_into("<I", img, 0x20, 0xABCD5432)
img[0x30:0x50] = b"9.9.9-compat 4mb".ljust(32, b"\0")
open(sys.argv[2], "wb").write(img)' "$1" "$2"
}
# Table v3: bigger than v2's factory partition (2,359,296 B), within v3's.
fake 2400000 "$tmp/big.bin"
sign sign "$tmp/big.bin" --version 9.9.9-compat --layout 3 --out "$tmp/v3.odfw" --key "$tmp/k" >/dev/null
fake 200000 "$tmp/small.bin"
sign sign "$tmp/small.bin" --version 9.9.9-compat --layout 2 --out "$tmp/v2.odfw" --key "$tmp/k" >/dev/null
[ -f "$root/opendmd-web/www.pack" ] || { echo "no opendmd-web/www.pack: make web first" >&2; exit 2; }
sign sign-www "$root/opendmd-web/www.pack" --out "$tmp/www.pack" --key "$tmp/k" >/dev/null

cp "$root/tools/fixtures/ota-compat.rs" "$tree/opendmd-core/tests/ota_compat.rs"
cp "$root/tools/fixtures/ota-compat.rs" "$root/opendmd-core/tests/ota_compat.rs"
n=0
run() {  # run <old|new> <take|refuse> <bundle> <pub> [<www>]
  local dir factory what
  if [ "$1" = old ]; then dir=$tree factory=$old_factory what="$tag (table v2)"; else dir=$root factory=$new_factory what="this tree (table v3)"; fi
  echo "==> $what: $(basename "$3") should be: $2${5:+; $(basename "$5"): take}"
  (cd "$dir/opendmd-core" && OTA_COMPAT_BUNDLE=$3 OTA_COMPAT_PUB=$4 OTA_COMPAT_WWW=${5:-} OTA_COMPAT_EXPECT=$2 \
    OTA_COMPAT_FACTORY=$factory RUSTUP_TOOLCHAIN=stable cargo test -q --features ota --test ota_compat -- --nocapture --test-threads 1)
  n=$((n + 1))
}
run old refuse "$tmp/v3.odfw" "$tmp/p" "$tmp/www.pack"
run old take "$tmp/v2.odfw" "$tmp/p"
run new take "$tmp/v3.odfw" "$tmp/p" "$tmp/www.pack"
run new refuse "$tmp/v2.odfw" "$tmp/p"
if [ $# -ge 4 ]; then
  bundle=$(realpath "$2") pub=$(realpath "$3") www=$(realpath "$4")
  run old refuse "$bundle" "$pub" "$www"
  run new take "$bundle" "$pub" "$www"
fi
echo "OTA_COMPAT_OK $tag ($n runs, each as it should be)"
