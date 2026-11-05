#!/usr/bin/env bash
# The older firmware takes what is signed now (docs/ota.md "The key
# trailer", docs/www-pack.md "Signature"): an older release's own
# opendmd-core, checked out at its tag, runs tools/fixtures/ota-compat.rs
# against a bundle and a www pack.
#
#   tools/ota-compat.sh [TAG] [BUNDLE PUB WWW]
#
# TAG defaults to v1.0.0-beta.14. Without files: a fake 4mb app image and
# this tree's www pack, both signed now with a throwaway key (tools/fw-sign.py
# sign / sign-www). Prints "OTA_COMPAT_OK <tag>" when both pass.
set -euo pipefail
root=$(cd "$(dirname "$0")/.." && pwd)
tag=${1:-v1.0.0-beta.14}
work=${OTA_COMPAT_DIR:-$root/target/ota-compat}
mkdir -p "$work"
tree="$work/$tag"
if [ ! -d "$tree/opendmd-core" ]; then
  git -C "$root" worktree add --detach "$tree" "$tag" >/dev/null
fi
sign() { uv run --quiet "$root/tools/fw-sign.py" "$@"; }
if [ $# -ge 4 ]; then
  bundle=$2 pub=$3 www=$4
else
  tmp=$(mktemp -d)
  trap 'rm -rf "$tmp"' EXIT
  sign keygen --key "$tmp/k" --pub "$tmp/p" >/dev/null
  uv run --quiet python -I -c '
import os, struct, sys
img = bytearray(b"\xE9" + os.urandom(200_000))
struct.pack_into("<I", img, 0x20, 0xABCD5432)
img[0x30:0x50] = b"9.9.9-compat 4mb".ljust(32, b"\0")
open(sys.argv[1], "wb").write(img)' "$tmp/app.bin"
  sign sign "$tmp/app.bin" --version 9.9.9-compat --out "$tmp/b.odfw" --key "$tmp/k" >/dev/null
  [ -f "$root/opendmd-web/www.pack" ] || { echo "no opendmd-web/www.pack: make web first" >&2; exit 2; }
  sign sign-www "$root/opendmd-web/www.pack" --out "$tmp/www.pack" --key "$tmp/k" >/dev/null
  bundle=$tmp/b.odfw pub=$tmp/p www=$tmp/www.pack
fi
bundle=$(realpath "$bundle") pub=$(realpath "$pub") www=$(realpath "$www")
cp "$root/tools/fixtures/ota-compat.rs" "$tree/opendmd-core/tests/ota_compat.rs"
(cd "$tree/opendmd-core" && OTA_COMPAT_BUNDLE=$bundle OTA_COMPAT_PUB=$pub OTA_COMPAT_WWW=$www \
  RUSTUP_TOOLCHAIN=stable cargo test -q --features ota --test ota_compat -- --nocapture)
rm -f "$tree/opendmd-core/tests/ota_compat.rs"
echo "OTA_COMPAT_OK $tag"
