#!/usr/bin/env bash
# Update the public gallery repo, github.com/opendmd/opendmd.github.io (the
# name is from when it was the GitHub Pages site; the site is opendmd.dev
# now, on Cloudflare, and this repo publishes nothing).
#
#   tools/gallery-repo-publish.sh <site-dir> <repo-url>   (normally via `make gallery-repo-publish`)
#
# Keeps a clone in target/gallery-repo (GALLERY_REPO_WORK overrides it) and
# makes the repo exactly:
#
# - gallery/: the formula gallery's contributions, by pull request. What is
#   merged there is never touched here; only its README.md (how to
#   contribute) is ours, from site/public-repo/gallery/;
# - .github/: replaced as a whole with site/public-repo/.github/ (the
#   gallery's pull-request check, its threat model, CODEOWNERS) plus, under
#   .github/gallery-check/, the checker's source, vendored from opendmd-psi/
#   and opendmd-gallery/ (the check builds it offline from main's copy);
# - dev/api/: the HTTP API page as each release described it
#   (v<version>.html, tools/site.py), the archive release.yml fetches for
#   the next site build. Nothing there is ever removed or replaced, with one
#   exception: a release's own page, published from that release's tag (git
#   describe, or GALLERY_REPO_DESCRIBE, is exactly v<version>), may replace
#   an earlier publish of the same version. A publish from anywhere else
#   adds no page for its version; the earlier releases' pages the site
#   carries are added only where missing. A site that lacks one the repo
#   has is refused: its dev/api.html would not list it (build it with
#   API_ARCHIVE, see the refusal);
# - README.md: what the repo is, from site/public-repo/README.md.
#
# Anything else (an old copy of the site) is removed. Commits as
# "OpenDMD <release>" only when something changed.
# tools/gallery-repo-publish-test.sh runs it against a local bare repository
# (GALLERY_REPO_ALLOW_DIRTY=1 is for that test only).
set -euo pipefail
site=$1 repo=$2 work=${GALLERY_REPO_WORK:-target/gallery-repo}
# GALLERY_REPO_DESCRIBE stands in for git describe: release.yml sets it to
# the tag it releases (after checking the tree is clean), so another tag on
# the same commit cannot change what is published; and the test.
version=${GALLERY_REPO_DESCRIBE:-$(git describe --always --dirty --tags --match 'v*')}
case $version in *-dirty) [ "${GALLERY_REPO_ALLOW_DIRTY:-}" = 1 ] || { echo "refusing: the working tree is dirty ($version); commit first" >&2; exit 1; };; esac
# The repo is public: name the release the site carries, not a development
# describe ("OpenDMD v1.0.0", or "OpenDMD v1.0.0, docs of 2026-10-21" when
# the docs moved on since the tag).
release=v$(sed -n 's/.*"version": *"\([^"]*\)".*/\1/p' "$site/install/release.json")
label=$release
[ "$version" = "$release" ] || label="$release, docs of $(date -u +%F)"
# A release's site carries its own API page (tools/site.py freezes it when
# built from the tag); without it the release would have none.
[ "$version" != "$release" ] || [ -f "$site/dev/api/$release.html" ] ||
  { echo "refusing: the site of $release has no dev/api/$release.html (make site freezes it from the tag)" >&2; exit 1; }

# The repo as it will be, gallery/'s contributions aside.
stage=$(mktemp -d)
trap 'rm -rf "$stage"' EXIT
cp -a site/public-repo/. "$stage"/
mkdir -p "$stage/.github/gallery-check"
# Only what git tracks (as committed: the tree is clean), and nothing new beside it.
# opendmd-core/src/fonts.rs: the panel's fonts, which opendmd-psi include!s
# (its `digit` and `glyph`).
vendored=(opendmd-psi opendmd-core/src/fonts.rs opendmd-gallery)
untracked=$(git ls-files --others --exclude-standard -- "${vendored[@]}")
[ -z "$untracked" ] || { echo "refusing: untracked files in the checker's source: $untracked" >&2; exit 1; }
git ls-files -z --cached -- "${vendored[@]}" |
  xargs -0 cp --parents -t "$stage/.github/gallery-check"

if [ -d "$work/.git" ]; then
  git -C "$work" fetch -q origin && git -C "$work" reset -q --hard origin/main 2>/dev/null || true
else
  git clone -q "$repo" "$work" 2>/dev/null || { mkdir -p "$work" && git -C "$work" init -q -b main && git -C "$work" remote add origin "$repo"; }
fi
# Every earlier API page here must be in the site, or dev/api.html would not link it.
missing=$(for f in "$work"/dev/api/v*.html; do [ ! -f "$f" ] || [ -f "$site/dev/api/${f##*/}" ] || printf ' dev/api/%s' "${f##*/}"; done)
[ -z "$missing" ] || { echo "refusing: the site lacks the published API pages$missing, so its dev/api.html would not list them;" \
  "fetch them (git clone --depth 1 --filter=blob:none --no-checkout $repo D && git -C D sparse-checkout set --no-cone '/dev/api/v*.html'" \
  "&& git -C D checkout) and build with make site API_ARCHIVE=D/dev/api" >&2; exit 1; }
api=dev/api
mkdir -p "$stage/$api"
# The pages already published, as they are.
[ ! -d "$work/$api" ] || find "$work/$api" -maxdepth 1 -type f -name 'v*.html' -exec cp -t "$stage/$api" {} +
if [ -d "$site/$api" ]; then
  # Earlier releases' pages: added if missing, never replaced or removed.
  find "$site/$api" -maxdepth 1 -type f -name 'v*.html' ! -name "$release.html" -exec cp --update=none -t "$stage/$api" {} +
  # This release's own page: only from its tag (it may replace an earlier publish of it).
  if [ "$version" = "$release" ] && [ -f "$site/$api/$release.html" ]; then
    cp "$site/$api/$release.html" "$stage/$api/$release.html"
  fi
fi
# Commit as whoever commits here (this repo's identity may be local to it).
git -C "$work" config user.name "$(git config user.name)"
git -C "$work" config user.email "$(git config user.email)"
# --checksum: a file checked out and rewritten in the same second at the same size
# would pass rsync's size+mtime test and keep its old content.
rsync -a --checksum --delete --exclude /.git --exclude /gallery/ "$stage"/ "$work"/
mkdir -p "$work/gallery"
cp "$stage/gallery/README.md" "$work/gallery/README.md"
git -C "$work" add -A
if git -C "$work" diff --cached --quiet; then echo "gallery repo: no changes"; exit 0; fi
git -C "$work" commit -q -m "OpenDMD $label"
git -C "$work" push -q origin HEAD:main
echo "gallery repo: pushed $label ($version) to $repo"
