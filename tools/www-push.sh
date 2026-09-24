#!/usr/bin/env bash
# Write a web app pack into a running player's flash, over WiFi.
#
#   tools/www-push.sh <host> <www.pack>        (normally via `make www-push HOST=...`)
#
# PUTs the pack (`make web` builds opendmd-web/www.pack; `make fw-release`
# copies it to dist/) to /api/www, prints the reply, then reads GET /api/www
# back and checks the device now serves a verified pack with the version the
# pack's header carries (bytes 20.. of the header, length at byte 18; see
# docs/www-pack.md). No reboot: the device remaps the new pack in place.
# While it is written (a few seconds to ~20 s) the API keeps answering and
# page loads get a 503; the UI already open in a browser keeps working.
set -euo pipefail

host=${1:?usage: tools/www-push.sh <host> <www.pack>}
pack=${2:?usage: tools/www-push.sh <host> <www.pack>}
[ -f "$pack" ] || { echo "no such pack: $pack (run make web first)" >&2; exit 2; }
[ "$(head -c 4 "$pack")" = "ODWP" ] || { echo "$pack is not a www pack" >&2; exit 2; }

vlen=$(od -An -tu1 -j18 -N1 "$pack" | tr -d ' ')
want=$(dd if="$pack" bs=1 skip=20 count="$vlen" 2>/dev/null)
size=$(stat -c %s "$pack")
echo "pushing $pack ($size B, version '$want') to http://$host/api/www"

t0=$(date +%s.%N)
reply=$(curl -sS --max-time 180 -w '\n%{http_code}' -X PUT \
    -H 'Content-Type: application/octet-stream' --data-binary @"$pack" \
    "http://$host/api/www")
code=${reply##*$'\n'}
body=${reply%$'\n'*}
t1=$(date +%s.%N)
awk -v c="$code" -v a="$t0" -v b="$t1" -v r="$body" 'BEGIN { printf "HTTP %s in %.1f s: %s\n", c, b - a, r }'
[ "$code" = 200 ] || exit 1

info=$(curl -sS --max-time 10 "http://$host/api/www")
echo "GET /api/www: $info"
case "$info" in
    *'"ok":true'*"\"version\":\"$want\""* | *"\"version\":\"$want\""*'"ok":true'*) echo "ok: the device serves '$want'" ;;
    *) echo "the device does not report '$want' as a verified pack" >&2; exit 1 ;;
esac
