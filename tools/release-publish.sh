#!/usr/bin/env bash
# The GitHub release of a tag and its assets, made so that a rerun is safe
# (release.yml, and tools/release.sh PUBLISH=local):
#
#   tools/release-publish.sh TAG ASSETS_DIR NOTES_FILE     (GH_REPO, default opendmd/opendmd)
#
# - No release for TAG: one is created as a draft (title "OpenDMD <version>",
#   NOTES_FILE then GitHub's generated notes; a pre-release for a "-"
#   version), every file in ASSETS_DIR is uploaded to it, and only then is it
#   published (marked latest for a final, never for a pre-release). Nobody
#   sees a release whose assets are still arriving.
# - A draft (a run that stopped before publishing it): its assets are
#   brought to ASSETS_DIR's (a missing one uploaded, one with another digest
#   replaced: a draft has been seen by no one), then it is published.
# - A published release: an asset with ASSETS_DIR's digest is skipped and a
#   missing one uploaded; one with another digest stops it, as does an asset
#   in a broken state. A published asset is never replaced or deleted
#   (release.yml takes a published release's assets as the build instead,
#   so a rerun finds them all the same).
# A release drafted by hand for the tag keeps its notes.
#
# PUBLISHER (ci, default, or local): who publishes. A release whose notes
# name the other publisher ("Publisher: local" or "Publisher: ci", the
# lines tools/release-notes.sh ends them with) is never touched: CI never
# completes or adds to a draft the dev box started, nor the other way round.
# GUARD_CI_RUNS=1 (tools/release.sh PUBLISH=local): just before creating the
# release and just before publishing the draft, release.yml must have no
# run for the tag; one that started meanwhile stops this (it publishes).
# RELEASE_LATEST: true or false, whether a final is marked latest (default:
# a final is, a pre-release is not); release.yml passes false for a final
# that is no longer the newest.
#
# Prints PASS/SKIP lines; the last line is "published: <what changed>" or
# "published: nothing" (tools/release.sh records it).
set -euo pipefail
export LC_ALL=C
tag=$1 assets=$2 notes=$3
repo=${GH_REPO:-opendmd/opendmd}
version=${tag#v}
fail() { echo "FAIL  $*" >&2; exit 1; }
[ -d "$assets" ] && [ -f "$assets/SHA256SUMS" ] || fail "$assets has no SHA256SUMS"
(cd "$assets" && sha256sum --quiet -c SHA256SUMS) || fail "$assets does not match its SHA256SUMS"
[ -f "$notes" ] || fail "no notes file $notes"
prerelease=false; [[ $version == *-* ]] && prerelease=true
publisher=${PUBLISHER:-ci}
case $publisher in ci) other=local ;; local) other=ci ;; *) fail "PUBLISHER is '$publisher', not ci or local" ;; esac
latest=${RELEASE_LATEST:-}
[ -n "$latest" ] || { [ "$prerelease" = true ] && latest=false || latest=true; }
[ "$prerelease" = false ] || latest=false
changed=()

guard() {  # no release.yml run for the tag (GUARD_CI_RUNS=1)
  [ "${GUARD_CI_RUNS:-}" = 1 ] || return 0
  local runs
  runs=$(gh run list -R "$repo" --workflow release.yml --branch "$tag" --limit 5 --json databaseId --jq '.[].databaseId') ||
    fail "cannot list release.yml's runs for $tag"
  [ -z "$runs" ] || fail "release.yml has started for $tag (run $(head -1 <<<"$runs")) while this ran: CI publishes it; stopping before $1"
}

# gh release view finds a draft by its tag too. Anything but "not found" is a failure.
if state=$(gh release view "$tag" -R "$repo" --json isDraft,isPrerelease,body --jq '[.isDraft, .isPrerelease, (.body | test("(^|\\n)Publisher: '"$other"'"))] | @tsv' 2>"$notes.err"); then
  draft=$(cut -f1 <<<"$state")
  [ "$(cut -f3 <<<"$state")" = false ] ||
    fail "the release $tag says \"Publisher: $other\": it is the other publisher's to finish, not this one's"
  [ "$(cut -f2 <<<"$state")" = "$prerelease" ] ||
    fail "the release $tag has isPrerelease=$(cut -f2 <<<"$state"), want $prerelease: fix it by hand on GitHub"
  echo "SKIP  the release $tag exists ($([ "$draft" = true ] && echo draft || echo published)); its notes are kept"
elif grep -qi 'release not found' "$notes.err"; then
  guard "creating the release"
  flags=(--draft --verify-tag --title "OpenDMD $version" --notes-file "$notes" --generate-notes)
  [ "$prerelease" = false ] || flags+=(--prerelease)
  gh release create "$tag" -R "$repo" "${flags[@]}" >/dev/null || fail "gh release create $tag failed"
  draft=true
  changed+=("draft release $tag created")
  echo "PASS  draft release $tag created"
else
  fail "cannot read the release $tag: $(cat "$notes.err")"
fi
rm -f "$notes.err"

remote=$(gh release view "$tag" -R "$repo" --json assets --jq '.assets[] | [.name, .state, (.digest // "")] | @tsv') ||
  fail "cannot read $tag's assets"
uploaded=0 same=0
for f in "$assets"/*; do
  n=${f##*/}
  want="sha256:$(sha256sum "$f" | cut -d' ' -f1)"
  line=$(awk -F'\t' -v n="$n" '$1 == n' <<<"$remote")
  if [ -n "$line" ]; then
    st=$(cut -f2 <<<"$line") digest=$(cut -f3 <<<"$line")
    if [ "$st" = uploaded ] && [ -z "$digest" ]; then
      digest="sha256:$(gh release download "$tag" -R "$repo" -p "$n" -O - | sha256sum | cut -d' ' -f1)"
    fi
    if [ "$st" = uploaded ] && [ "$digest" = "$want" ]; then same=$((same + 1)); continue; fi
    [ "$draft" = true ] ||
      fail "the published release's $n ($st, $digest) is not this build's ($want): a published asset is never replaced"
    # A draft: nobody has it yet.
    gh release upload "$tag" -R "$repo" --clobber "$f" >/dev/null || fail "uploading $n failed"
    echo "      replaced $n on the draft"
  else
    gh release upload "$tag" -R "$repo" "$f" >/dev/null || fail "uploading $n failed"
    echo "      uploaded $n"
  fi
  uploaded=$((uploaded + 1))
done
total=$(find "$assets" -maxdepth 1 -type f | wc -l)
if [ "$uploaded" -gt 0 ]; then
  changed+=("$uploaded of $total assets uploaded")
  echo "PASS  $uploaded of $total assets uploaded ($same already there with this build's digest)"
else
  echo "SKIP  all $total assets are on $tag with this build's digests"
fi

if [ "$draft" = true ]; then
  guard "publishing the draft"
  gh release edit "$tag" -R "$repo" --draft=false "--latest=$latest" >/dev/null || fail "publishing the draft $tag failed"
  changed+=("release $tag published")
  how='final, marked latest'
  [ "$latest" = true ] || how='final, not marked latest: a newer final exists'
  [ "$prerelease" = false ] || how=pre-release
  echo "PASS  release $tag published ($how)"
fi

# What anyone now downloads: every asset, uploaded, with this build's digest.
remote=$(gh release view "$tag" -R "$repo" --json isDraft,assets --jq '(.isDraft | tostring), (.assets[] | [.name, .state, (.digest // "")] | @tsv)')
[ "$(head -1 <<<"$remote")" = false ] || fail "$tag is still a draft"
for f in "$assets"/*; do
  n=${f##*/}
  line=$(tail -n +2 <<<"$remote" | awk -F'\t' -v n="$n" '$1 == n')
  [ "$(cut -f2 <<<"$line")" = uploaded ] || fail "$tag's $n is not uploaded"
  d=$(cut -f3 <<<"$line")
  [ -z "$d" ] || [ "$d" = "sha256:$(sha256sum "$f" | cut -d' ' -f1)" ] || fail "$tag's $n has another digest"
done
echo "PASS  $tag: published, all $total assets with this build's digests"
if [ ${#changed[@]} -gt 0 ]; then
  printf -v joined '%s; ' "${changed[@]}"
  echo "published: ${joined%; }"
else
  echo "published: nothing"
fi
