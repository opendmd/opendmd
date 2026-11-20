#!/usr/bin/env bash
# No test-only firmware feature in a release's app image: the emulator lanes'
# hooks (nocard-test, emu-ballast, emu-census, emu-oom-test) each log a line
# no other build has, so its text in the image means the feature is in it.
# The Makefile refuses them in a build into dist/ (FW_TEST_ONLY_GUARD) and
# tools/release.sh clears FW_RELEASE_FEATURES; this checks the image itself.
# Run by release.yml (on whichever dist/ it checks) and tools/release.sh
# (every build it makes).
#
#   tools/release-test-features-check.sh <app.bin>
#   tools/release-test-features-check.sh --selftest
#
# Exit 0 with one line per check passed; 1 with the reason. GitHub
# annotations (::error::) when GITHUB_ACTIONS is set.
set -euo pipefail

# feature, then the text only it puts in the image.
MARKERS=(
  "emu-oom-test" "oom-test: "
  "nocard-test" "nocard-test: "
  "emu-census" "census: "
  "emu-ballast" "ballast: "
)

fail() {
  if [ "${GITHUB_ACTIONS:-}" = true ]; then echo "::error::$1"; else echo "FAIL: $1" >&2; fi
  exit 1
}

check() {
  local app=$1 i n
  [ -f "$app" ] || { echo "release-test-features-check: no $app" >&2; exit 2; }
  for ((i = 0; i < ${#MARKERS[@]}; i += 2)); do
    n=$(grep -ac -- "${MARKERS[i + 1]}" "$app" || true)
    [ "$n" -eq 0 ] || fail "$app carries the test-only feature ${MARKERS[i]} ('${MARKERS[i + 1]}'): never in a release"
  done
  echo "pass: $app: no test-only feature ($((${#MARKERS[@]} / 2)) markers: $(for ((i = 0; i < ${#MARKERS[@]}; i += 2)); do printf '%s ' "${MARKERS[i]}"; done | sed 's/ $//'))"
}

if [ "${1:-}" = --selftest ]; then
  d=$(mktemp -d)
  trap 'rm -r -- "$d"' EXIT
  printf 'OpenDMD firmware\0heartbeat: heap\0' > "$d/clean.bin"
  check "$d/clean.bin" > /dev/null || { echo "selftest: a clean image failed" >&2; exit 1; }
  for ((i = 0; i < ${#MARKERS[@]}; i += 2)); do
    printf 'x\0%sdrop 1\0' "${MARKERS[i + 1]}" > "$d/dirty.bin"
    if (check "$d/dirty.bin") > /dev/null 2>&1; then
      echo "selftest: an image with ${MARKERS[i]}'s marker passed" >&2; exit 1
    fi
  done
  echo "pass: release-test-features-check selftest: a clean image passes, each of $((${#MARKERS[@]} / 2)) markers fails"
  exit 0
fi

check "${1:?usage: tools/release-test-features-check.sh <app.bin> | --selftest}"
