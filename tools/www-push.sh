#!/usr/bin/env bash
# Write a web app pack into a running player's flash, over WiFi.
#
#   tools/www-push.sh <host> <www.pack>        (normally via `make www-push HOST=...`)
#
# PUTs the pack (`make web` builds opendmd-web/www.pack, signed here first;
# `make fw-release` signs a copy into dist/) to /api/www, prints the reply, then reads GET /api/www
# back and checks the device now serves a verified pack with the version the
# pack's header carries (bytes 20.. of the header, length at byte 18; see
# docs/www-pack.md). No reboot: the device remaps the new pack in place.
# While it is written (a few seconds to ~20 s) the API keeps answering and
# page loads get a 503; the UI already open in a browser keeps working.
# OPENDMD_API_KEY=<key> when the player has an API key set: a cheap GET
# /api/www goes first so a missing or wrong key is reported in words (the
# PUT is refused before its body is read, which curl reports only as
# "Connection reset by peer"). Without a usable card the player answers 503:
# a pack is staged on the card or not at all (docs/www-pack.md).
set -euo pipefail

host=${1:?usage: tools/www-push.sh <host> <www.pack>}
pack=${2:?usage: tools/www-push.sh <host> <www.pack>}
[ -f "$pack" ] || { echo "no such pack: $pack (run make web first)" >&2; exit 2; }
[ "$(head -c 4 "$pack")" = "ODWP" ] || { echo "$pack is not a www pack" >&2; exit 2; }
# A player takes only a signed pack (docs/www-pack.md "Signature"): a local
# build (`make web`'s, unsigned) is signed here with FW_SIGN_KEY (default
# ~/.config/opendmd/opendmd.key) into a temporary copy.
if [ $(( $(od -An -tu1 -j19 -N1 "$pack" | tr -d ' ') & 1 )) -eq 0 ]; then
    key=${FW_SIGN_KEY:-$HOME/.config/opendmd/opendmd.key}
    [ -f "$key" ] || { echo "$pack is not signed and there is no key at $key to sign it (FW_SIGN_KEY=...)" >&2; exit 2; }
    signed=$(mktemp --suffix=.pack)
    trap 'rm -f "$signed"' EXIT
    uv run --quiet "$(dirname "$0")/fw-sign.py" sign-www "$pack" --out "$signed" --key "$key"
    pack=$signed
fi
auth=()
[ -n "${OPENDMD_API_KEY:-}" ] && auth=(-H "Authorization: Bearer $OPENDMD_API_KEY")

vlen=$(od -An -tu1 -j18 -N1 "$pack" | tr -d ' ')
want=$(dd if="$pack" bs=1 skip=20 count="$vlen" 2>/dev/null)
size=$(stat -c %s "$pack")

pre=$(curl -sS --max-time 10 -w '\n%{http_code}' "${auth[@]}" "http://$host/api/www") \
    || { echo "cannot reach http://$host/api/www" >&2; exit 1; }
case "${pre##*$'\n'}" in
    200) ;;
    401) if [ -n "${OPENDMD_API_KEY:-}" ]; then
             echo "http://$host refused the API key in OPENDMD_API_KEY (401): check it in Settings > Security" >&2
         else
             echo "http://$host has an API key set (401): run it again with OPENDMD_API_KEY=<key>" >&2
         fi
         exit 1 ;;
    *) echo "GET http://$host/api/www: HTTP ${pre##*$'\n'}: ${pre%$'\n'*}" >&2; exit 1 ;;
esac

echo "pushing $pack ($size B, version '$want') to http://$host/api/www"

t0=$(date +%s.%N)
reply=$(curl -sS --max-time 180 -w '\n%{http_code}' -X PUT "${auth[@]}" \
    -H 'Content-Type: application/octet-stream' --data-binary @"$pack" \
    "http://$host/api/www")
code=${reply##*$'\n'}
body=${reply%$'\n'*}
t1=$(date +%s.%N)
awk -v c="$code" -v a="$t0" -v b="$t1" -v r="$body" 'BEGIN { printf "HTTP %s in %.1f s: %s\n", c, b - a, r }'
[ "$code" = 200 ] || exit 1

info=$(curl -sS --max-time 10 "${auth[@]}" "http://$host/api/www")
echo "GET /api/www: $info"
case "$info" in
    *'"ok":true'*"\"version\":\"$want\""* | *"\"version\":\"$want\""*'"ok":true'*) echo "ok: the device serves '$want'" ;;
    *) echo "the device does not report '$want' as a verified pack" >&2; exit 1 ;;
esac
