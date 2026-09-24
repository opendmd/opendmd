#!/usr/bin/env bash
# Push a firmware image to a running player and wait for it to come back.
#
#   tools/ota.sh <host> <image.bin>        (normally via `make ota HOST=...`)
#
# POSTs the raw image to /api/ota (curl --data-binary, the endpoint takes the
# body as-is), prints the JSON reply, then polls /api/status until the board
# answers with an uptime that started after the upload and the version the
# image carries. The expected version is the first word of the image's
# esp_app_desc_t.version at offset 0x30 ("0.3.0 4mb" for `make fw-release`),
# so rolling back to an older image waits for the older version.
#
# Env: OTA_TIMEOUT (seconds, default 120), OTA_DRY_RUN=1 (print, don't run).
set -euo pipefail

host=${1:?usage: tools/ota.sh <host> <image.bin>}
bin=${2:?usage: tools/ota.sh <host> <image.bin>}
timeout=${OTA_TIMEOUT:-120}
dry=${OTA_DRY_RUN:-}

if [ ! -f "$bin" ]; then
    if [ -n "$dry" ]; then
        echo "(dry run: $bin does not exist yet; run make fw-release first)"
    else
        echo "no such image: $bin (run make fw-release first)" >&2
        exit 2
    fi
    desc=""
else
    desc=$(dd if="$bin" bs=1 skip=48 count=32 2>/dev/null | tr -d '\000')
fi
# A release image says "<version> <variant>"; anything else has no marker.
want=""
case "$desc" in
    *" 4mb" | *" 8mb") want=${desc%% *} ;;
esac
size=$( [ -f "$bin" ] && stat -c %s "$bin" || echo "?")
echo "image: $bin ($size B), app descriptor '${desc:-?}', expecting version '${want:-any}'"

post=(curl -sS --fail-with-body --max-time 180
      -H 'Content-Type: application/octet-stream'
      --data-binary "@$bin" "http://$host/api/ota")
if [ -n "$dry" ]; then
    printf 'would run:'; printf ' %q' "${post[@]}"; echo
    echo "then poll: curl -sS --max-time 3 http://$host/api/status every 2 s for up to ${timeout}s until uptime_secs restarts and version == '${want:-any}'"
    exit 0
fi

uptime_of() { sed -n 's/.*"uptime_secs": *\([0-9]*\).*/\1/p'; }
# The board's uptime before the upload: after the reboot it must be lower
# than this plus the time since, which tells a reboot from a board that
# merely happened to boot recently.
base=$(curl -sS --max-time 3 "http://$host/api/status" 2>/dev/null | uptime_of || true)
base=${base:-0}
start=$(date +%s)
echo "uploading to http://$host/api/ota ..."
reply=$("${post[@]}") || { echo "upload failed: $reply" >&2; exit 1; }
echo "$reply"

# The handler replies, sleeps a second and restarts; give it a moment to go.
sleep 3
while :; do
    now=$(date +%s)
    elapsed=$((now - start))
    if [ "$elapsed" -ge "$timeout" ]; then
        echo "timed out after ${timeout}s waiting for the board to come back" >&2
        exit 1
    fi
    if status=$(curl -sS --max-time 3 "http://$host/api/status" 2>/dev/null); then
        version=$(printf '%s' "$status" | sed -n 's/.*"version": *"\([^"]*\)".*/\1/p')
        uptime=$(printf '%s' "$status" | uptime_of)
        rebooted=""
        if [ -n "$uptime" ] && [ "$uptime" -le "$elapsed" ] \
            && [ "$uptime" -lt $((base + elapsed - 2)) ]; then
            rebooted=1
        fi
        if [ -n "$rebooted" ] && { [ -z "$want" ] || [ "$version" = "$want" ]; }; then
            echo "back after ${elapsed}s: version $version, uptime ${uptime}s"
            exit 0
        fi
        if [ -n "$rebooted" ]; then
            echo "board rebooted but reports version '$version', not '$want'" >&2
            exit 1
        fi
    fi
    sleep 2
done
