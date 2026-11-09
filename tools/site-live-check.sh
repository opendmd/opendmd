#!/usr/bin/env bash
# After a deploy: does opendmd.dev serve the release it should?
#
#   tools/site-live-check.sh BETA_VERSION RELEASE_VERSION [SUMS_RELEASE [SUMS_BETA]]
#
# install/beta.json must name BETA_VERSION and install/release.json
# RELEASE_VERSION (the same version unless a beta is newer than the final;
# docs/deploying.md "Betas"), and the update bundle and www pack each feed
# names must be served. With SUMS_* (a release's SHA256SUMS), each of those
# files must also be byte for byte the release's asset.
#
# It waits up to SITE_LIVE_WAIT seconds (default 180) for the deploy to show:
# with SITE_LIVE_VERSION (the deployed tree's `git describe`, which the
# Makefile gives the Worker as SITE_VERSION), until /api/health says that
# version, so a check right after a deploy reads the new one, not the one
# before; and in any case until a file the Worker does not count (the
# feed's bundle) is served. Then it reads
# each feed once: a GET of release.json or beta.json counts as an update
# check on /stats (site/worker/src/counts.ts), so it is not polled.
# Used by release.yml, site.yml and tools/release.sh (PUBLISH=local).
set -euo pipefail
export LC_ALL=C
beta=$1 rel=$2 sums_rel=${3:-} sums_beta=${4:-}
base=${SITE_URL:-https://opendmd.dev}
wait=${SITE_LIVE_WAIT:-180}
ua='opendmd-site-test/1 (+https://opendmd.dev)'
fail() { echo "FAIL  $*" >&2; exit 1; }
get() { curl -fsS -A "$ua" "$base/$1?live-check=$(date +%s%N)"; }

# The newest of the two (what beta.json names) lives under firmware/beta/
# when it is a beta newer than the final, else under firmware/.
dir=firmware; [ "$beta" = "$rel" ] || dir=firmware/beta
probe="install/$dir/opendmd-$beta-4mb.odfw"
deadline=$(( $(date +%s) + wait ))
if [ -n "${SITE_LIVE_VERSION:-}" ]; then
  until [ "$(curl -fsS -A "$ua" "$base/api/health?live-check=$(date +%s%N)" 2>/dev/null | jq -r .version 2>/dev/null)" = "$SITE_LIVE_VERSION" ]; do
    [ "$(date +%s)" -lt "$deadline" ] || fail "$base/api/health does not say version $SITE_LIVE_VERSION after ${wait}s (the deploy is not live)"
    sleep 5
  done
  echo "PASS  $base/api/health: version $SITE_LIVE_VERSION (this deploy)"
fi
until curl -fsS -o /dev/null -I -A "$ua" "$base/$probe?live-check=$(date +%s%N)" 2>/dev/null; do
  [ "$(date +%s)" -lt "$deadline" ] || fail "$base/$probe is not served after ${wait}s"
  sleep 5
done

tmp=$(mktemp -d); trap 'rm -rf "$tmp"' EXIT
check() {  # FEED VERSION DIR SUMS
  local feed=$1 ver=$2 d=$3 sums=$4 got f name want have
  get "install/$feed" > "$tmp/$feed" || fail "cannot fetch $base/install/$feed"
  got=$(jq -r .version "$tmp/$feed")
  [ "$got" = "$ver" ] || fail "$base/install/$feed says '$got', want $ver"
  for f in "$(jq -r .firmware "$tmp/$feed")" "$(jq -r .www "$tmp/$feed")"; do
    [ "${f%/*}" = "$d" ] || fail "$feed names $f, not a file under install/$d/"
    get "install/$f" > "$tmp/file" || fail "$base/install/$f (named by $feed) is not served"
    if [ -n "$sums" ]; then
      name=${f##*/}
      want=$(awk -v n="$name" '$2 == n || $2 == "*" n { print $1 }' "$sums")
      [ -n "$want" ] || fail "$sums has no $name"
      have=$(sha256sum "$tmp/file" | cut -d' ' -f1)
      [ "$have" = "$want" ] || fail "$base/install/$f is not the release's $name ($have, want $want)"
    fi
  done
  echo "PASS  $base/install/$feed: $ver, its bundle and www pack served$( [ -z "$sums" ] || echo ', byte for byte the release assets')"
}
check release.json "$rel" firmware "$sums_rel"
if [ "$beta" = "$rel" ]; then sums_beta=$sums_rel; fi
check beta.json "$beta" "$dir" "$sums_beta"

# The offline install (the install page's "Play from the card only"): the
# manifest and its NVS image beside the release's, when the release has
# them (releases before the box have neither: nothing to check).
offline() {  # VERSION DIR
  local ver=$1 d=$2 got nvs size word
  if ! get "install/$d/manifest-4mb-offline.json" > "$tmp/offline.json" 2>/dev/null; then
    echo "SKIP  $base/install/$d/manifest-4mb-offline.json: none ($ver is from before the offline install)"
    return
  fi
  got=$(jq -r .version "$tmp/offline.json")
  [ "$got" = "$ver" ] || fail "$base/install/$d/manifest-4mb-offline.json says '$got', want $ver"
  nvs=$(jq -r '.builds[0].parts[] | select(.offset == 36864) | .path' "$tmp/offline.json")
  [ "$nvs" = "nvs-offline-4mb.bin" ] || fail "$base/install/$d/manifest-4mb-offline.json has no nvs-offline-4mb.bin at 0x9000 ('$nvs')"
  [ "$(jq '.builds[0].parts | length' "$tmp/offline.json")" = 7 ] || fail "$base/install/$d/manifest-4mb-offline.json does not have seven parts"
  get "install/$d/$nvs" > "$tmp/nvs.bin" || fail "$base/install/$d/$nvs is not served"
  size=$(stat -c %s "$tmp/nvs.bin")
  [ "$size" = 24576 ] || fail "$base/install/$d/$nvs is $size B, not 24,576"
  word=$(head -c 4 "$tmp/nvs.bin" | od -An -tx1 | tr -d ' \n')
  [ "$word" = feffffff ] || fail "$base/install/$d/$nvs does not start with an active NVS page (fffffffe; got bytes $word)"
  echo "PASS  $base/install/$d/manifest-4mb-offline.json: $ver, seven parts, $nvs 24,576 B, an active NVS page"
}
offline "$rel" firmware
[ "$beta" = "$rel" ] || offline "$beta" "$dir"
