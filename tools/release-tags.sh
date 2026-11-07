#!/usr/bin/env bash
# Questions about the v* tags and GitHub releases, asked the same way by
# release.yml and tools/release.sh:
#
#   tools/release-tags.sh newest [VERSION]        the newest version among the v* tags (and VERSION)
#   tools/release-tags.sh newest-final [EXCEPT]   the newest final X.Y.Z among the v* tags, 0.x and EXCEPT left out; empty if none
#   tools/release-tags.sh state TAG               none, draft, published (the GitHub release of TAG), and with
#                                                 "local" after it when its notes say "Publisher: local"
#   tools/release-tags.sh tag-object TAG          the "Tag object:" its notes record (tools/release-notes.sh); empty if none
#
# Versions without the "v". Only tags that are versions this project
# releases count (release.sh's valid_version; a stray "v-test" tag does
# not). Semver order: "1.0.0-beta.2" < "1.0.0". `state` fails on anything
# but a clear answer: a failed request is never read as "no release".
# GH_REPO (default opendmd/opendmd).
set -euo pipefail
export LC_ALL=C
repo=${GH_REPO:-opendmd/opendmd}
VALID='^(0|[1-9][0-9]*)\.(0|[1-9][0-9]*)\.(0|[1-9][0-9]*)(-[0-9A-Za-z]+(\.[0-9A-Za-z]+)*)?$'

versions() { git tag -l 'v*' | sed 's/^v//' | grep -E "$VALID" || true; }
newest() { sed 's/-/~/' | sort -V | tail -1 | sed 's/~/-/'; }

case ${1:-} in
  newest)
    { versions; [ -z "${2:-}" ] || echo "$2"; } | grep . | newest ;;
  newest-final)
    versions | grep -E '^[0-9]+\.[0-9]+\.[0-9]+$' | grep -v '^0\.' | grep -vxF "${2:-}" | newest || true ;;
  state)
    tag=$2
    if out=$(gh release view "$tag" -R "$repo" --json isDraft,body 2>&1); then
      st=$(jq -r 'if .isDraft then "draft" else "published" end' <<<"$out")
      if jq -r .body <<<"$out" | grep -q '^Publisher: local'; then echo "$st local"; else echo "$st"; fi
    elif grep -qi 'release not found' <<<"$out"; then
      echo none
    else
      echo "release-tags.sh: cannot read the release $tag: $out" >&2; exit 1
    fi ;;
  tag-object)
    gh release view "$2" -R "$repo" --json body --jq .body | sed -n 's/^Tag object: *\([0-9a-f]\{40\}\).*/\1/p' | head -1 ;;
  *)
    sed -n '2,13p' "$0" | sed 's/^# \{0,1\}//' >&2; exit 2 ;;
esac
