#!/usr/bin/env bash
# Update a running player's firmware over WiFi (docs/ota.md).
#
#   tools/fw-ota.sh <host> <bundle.odfw>        (normally via `make fw-ota HOST=...`)
#
# PUTs the signed bundle (`make fw-release` writes dist/opendmd-<v>-<variant>.odfw)
# to /api/firmware and prints the reply. A bundle for another flash layout
# than the device's (its magic: "ODFW44mb" 4mb table v4, "ODFW34mb" table
# v3, "ODFW1" + NULs table v2, any other "ODFW" magic another layout; the device's `variant` and `layout` in GET
# /api/firmware, no `layout` meaning v2) is refused here, before anything is
# sent; the device would refuse it too. The device stages it on its card,
# checks the signature, restarts into the updater, which installs it and
# restarts into the new firmware (~45 s from the end of the upload to
# playback, the panel dark). This script then polls GET /api/firmware until
# its `version` (the app descriptor's, which the build's FW_VERSION sets and
# the bundle's header carries) is the one in the bundle's header (bytes
# 8..40, NUL-padded), and prints the device's answer with the end of its
# update log. A bundle the device refuses (another key: at its header) is
# answered at once, not after the upload. OPENDMD_API_KEY=<key> when the
# player has an API key set: the
# first GET /api/firmware reports a missing or wrong key in words (the PUT
# is refused before its body is read, which curl reports only as
# "Connection reset by peer").
set -euo pipefail

host=${1:?usage: tools/fw-ota.sh <host> <bundle.odfw>}
bundle=${2:?usage: tools/fw-ota.sh <host> <bundle.odfw>}
[ -f "$bundle" ] || { echo "no such bundle: $bundle (run make fw-release first)" >&2; exit 2; }
# The bundle's flash layout and partition table, from its magic.
case "$(head -c 8 "$bundle" | od -An -tx1 | tr -d ' \n')" in
    4f44465734346d62) bundle_variant=4mb bundle_layout=4 ;;
    4f44465733346d62) bundle_variant=4mb bundle_layout=3 ;;
    4f44465731000000) bundle_variant=4mb bundle_layout=2 ;;
    4f444657*) echo "$bundle is a firmware bundle for a flash layout this script does not know" >&2; exit 2 ;;
    *) echo "$bundle is not a firmware bundle" >&2; exit 2 ;;
esac
board() { case "$1" in 4mb) echo "DevKit (4 MB)" ;; *) echo "$1" ;; esac; }
auth=()
[ -n "${OPENDMD_API_KEY:-}" ] && auth=(-H "Authorization: Bearer $OPENDMD_API_KEY")

want=$(dd if="$bundle" bs=1 skip=8 count=32 2>/dev/null | tr -d '\000')
size=$(stat -c %s "$bundle")
pre=$(curl -sS --max-time 10 -w '\n%{http_code}' "${auth[@]}" "http://$host/api/firmware") \
    || { echo "cannot reach http://$host/api/firmware" >&2; exit 1; }
before=${pre%$'\n'*}
case "${pre##*$'\n'}" in
    200) ;;
    401) if [ -n "${OPENDMD_API_KEY:-}" ]; then
             echo "http://$host refused the API key in OPENDMD_API_KEY (401): check it in Settings > Security" >&2
         else
             echo "http://$host has an API key set (401): run it again with OPENDMD_API_KEY=<key>" >&2
         fi
         exit 1 ;;
    *) echo "GET http://$host/api/firmware: HTTP ${pre##*$'\n'}: $before" >&2; exit 1 ;;
esac
echo "device: $before"
# Every firmware with an updater reports its layout; none means 4mb.
device_variant=$(printf '%s' "$before" | sed -n 's/.*"variant":"\([^"]*\)".*/\1/p')
device_variant=${device_variant:-4mb}
if [ "$bundle_variant" != "$device_variant" ]; then
    echo "$bundle is for the $(board "$bundle_variant") ($bundle_variant), but http://$host runs the" >&2
    echo "$(board "$device_variant") firmware ($device_variant): a different flash layout, which only a USB install changes." >&2
    echo "Use its bundle instead: make fw-ota HOST=$host VARIANT=$device_variant (dist/opendmd-<version>-$device_variant.odfw)" >&2
    exit 2
fi
# The partition table: firmware from before `layout` (1.0.0-beta.15) is v2.
device_layout=$(printf '%s' "$before" | sed -n 's/.*"layout":\([0-9]*\).*/\1/p')
device_layout=${device_layout:-2}
if [ "$bundle_layout" != "$device_layout" ]; then
    echo "$bundle is for flash layout v$bundle_layout of the $(board "$bundle_variant"), but http://$host has layout" >&2
    echo "v$device_layout: an update over WiFi cannot change the partition table. Install it once over USB" >&2
    echo "(make fw-release-flash PORT=..., or the installer at https://opendmd.dev/install/; docs/deploying.md)" >&2
    exit 2
fi
# The bundle path defaults to dist/opendmd-<Cargo.toml version>.odfw, which
# after a `make fw-release FW_VERSION=…` can be an old bundle; the device
# installs an older one without asking (docs/ota.md), so ask here.
have=$(printf '%s' "$before" | sed -n 's/^{"version":"\([^"]*\)".*/\1/p')
if [ -n "$have" ] && [ "${OPENDMD_ALLOW_OLDER:-}" != 1 ] \
   && [ "$(printf '%s\n%s\n' "$want" "$have" | tr - '~' | sort -V | tail -1)" = "$(printf %s "$have" | tr - '~')" ]; then
    echo "$bundle is '$want', not newer than the device's '$have': pass BUNDLE= or FW_VERSION=," >&2
    echo "or OPENDMD_ALLOW_OLDER=1 to install it anyway" >&2
    exit 2
fi
echo "pushing $bundle ($size B, version '$want', $bundle_variant layout v$bundle_layout) to http://$host/api/firmware"

t0=$(date +%s.%N)
reply=$(curl -sS --max-time 300 -w '\n%{http_code}' -X PUT "${auth[@]}" \
    -H 'Content-Type: application/octet-stream' --data-binary @"$bundle" \
    "http://$host/api/firmware")
code=${reply##*$'\n'}
body=${reply%$'\n'*}
t1=$(date +%s.%N)
awk -v c="$code" -v a="$t0" -v b="$t1" -v r="$body" 'BEGIN { printf "HTTP %s in %.1f s: %s\n", c, b - a, r }'
[ "$code" = 200 ] || exit 1

echo "installing: waiting for the device to answer with '$want' (~45 s, up to 180 s)"
for _ in $(seq 1 60); do
    sleep 3
    info=$(curl -sS --max-time 3 "${auth[@]}" "http://$host/api/firmware" 2>/dev/null || true)
    case "$info" in
        *"\"version\":\"$want\""*)
            t2=$(date +%s.%N)
            awk -v a="$t1" -v b="$t2" 'BEGIN { printf "back after %.0f s\n", b - a }'
            echo "GET /api/firmware: $info"
            echo "ok: the device runs '$want'"
            exit 0 ;;
        '') printf '.' ;;
        *) printf '+' ;;
    esac
done
echo
echo "the device did not come back with '$want' in 180 s; last answer: ${info:-none}" >&2
echo "(the updater logs every step to /update/log.txt on the card and to the console at 921600)" >&2
exit 1
