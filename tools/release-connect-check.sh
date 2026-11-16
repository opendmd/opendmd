#!/usr/bin/env bash
# Remote control (OpenDMD Connect) in a release's app image: exactly as the
# channel says (the Makefile's FW_BETA_FEATURES). A beta ("-" version) must
# carry the shipping client (the image names connect.opendmd.dev); a final
# must not; and no release ever carries the device tests' measuring half
# (connect-measure: its telemetry's `tls_hs_peak` field is in no other
# build). Run by release.yml (on whichever dist/ it checks) and
# tools/release.sh (every build it makes, --publish-local's included).
#
#   tools/release-connect-check.sh <app.bin> <version>
#
# Exit 0 with one line per check passed; 1 with the reason. GitHub
# annotations (::error::) when GITHUB_ACTIONS is set.
set -euo pipefail

app=${1:?usage: tools/release-connect-check.sh <app.bin> <version>}
version=${2:?usage: tools/release-connect-check.sh <app.bin> <version>}
[ -f "$app" ] || { echo "release-connect-check: no $app" >&2; exit 2; }

fail() {
  if [ "${GITHUB_ACTIONS:-}" = true ]; then echo "::error::$1"; else echo "FAIL: $1" >&2; fi
  exit 1
}

count() { grep -ac "$1" "$app" || true; }

server=$(count 'connect\.opendmd\.dev')
measure=$(count 'tls_hs_peak')

[ "$measure" -eq 0 ] || fail "$app carries Remote control's measuring half (connect-measure): never in a release"
echo "pass: $app: no measuring half"
if [[ "$version" == *-* ]]; then
  [ "$server" -gt 0 ] || fail "$app is the beta $version without Remote control (the connect feature: the Makefile's FW_BETA_FEATURES)"
  echo "pass: $app: the beta $version carries Remote control"
else
  [ "$server" -eq 0 ] || fail "$app is the final $version with Remote control in it (finals have none until P1's acceptance)"
  echo "pass: $app: the final $version has no Remote control"
fi
