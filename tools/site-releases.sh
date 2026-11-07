#!/usr/bin/env bash
# The releases the site's installer and update feeds name, from the
# published (non-draft) GitHub releases of opendmd/opendmd, for a site build
# between releases (.github/workflows/site.yml; docs/deploying.md "Betas"):
#
#   tools/site-releases.sh [REPO]     prints release=vX.Y.Z and beta=vX.Y.Z-pre.N (or beta=)
#
# release: the newest final release (0.x finals predate the channels and do
# not count); before any final exists, the newest release of all.
# beta: the newest release of all when it is a beta newer than `release`,
# else empty (beta.json is then release.json).
#
# Only published releases count, never a bare tag: a tag whose release is
# still being built and published (release.yml) has no files to serve yet,
# so a site built from this can never name a firmware newer than the newest
# published release. Needs gh (GH_TOKEN in CI).
set -euo pipefail
export LC_ALL=C
repo=${1:-opendmd/opendmd}

valid() { [[ $1 =~ ^v(0|[1-9][0-9]*)\.(0|[1-9][0-9]*)\.(0|[1-9][0-9]*)(-[0-9A-Za-z]+(\.[0-9A-Za-z]+)*)?$ ]]; }
# Newest first by semver: "-" sorts before the release ("1.0.0~beta.1" < "1.0.0").
newest() { sed 's/-/~/' | sort -V | tail -1 | sed 's/~/-/'; }

tags=$(gh release list -R "$repo" --exclude-drafts --limit 1000 --json tagName --jq '.[].tagName') ||
  { echo "site-releases.sh: cannot list $repo's releases" >&2; exit 1; }
all=$(while read -r t; do [ -n "$t" ] && valid "$t" && echo "$t"; done <<<"$tags" || true)
[ -n "$all" ] || { echo "site-releases.sh: $repo has no published release" >&2; exit 1; }
newest_all=$(newest <<<"$all")
final=$(grep -E '^v[0-9]+\.[0-9]+\.[0-9]+$' <<<"$all" | grep -v '^v0\.' | newest || true)
if [ -z "$final" ]; then
  echo "release=$newest_all"; echo "beta="
elif [ "$newest_all" != "$final" ] && [[ $newest_all == *-* ]]; then
  echo "release=$final"; echo "beta=$newest_all"
else
  echo "release=$final"; echo "beta="
fi
