#!/usr/bin/env bash
# Push the built site (make site) to the Pages repo, opendmd.github.io.
#
#   tools/site-publish.sh <site-dir> <repo-url>      (normally via `make site-publish`)
#
# Keeps a clone in target/site-repo (SITE_REPO_WORK overrides it) and
# replaces everything in it with the site, except:
#
# - .git;
# - gallery/: the formula gallery, which takes contributions by pull request
#   in the public repo. What is merged there is never touched here; only its
#   README.md (how to contribute) is ours, from site/public-repo/gallery/;
# - .github/: replaced as a whole with what the public repo runs, from
#   site/public-repo/.github/ (the gallery's pull-request check and the
#   Pages workflow) plus, under .github/gallery-check/, the checker's source,
#   vendored from opendmd-psi/ and opendmd-gallery/ (both workflows build it
#   offline from main's copy), and tools/gallery_page.py (the gallery page's
#   generator, which the Pages workflow runs on every push to main).
#
# Commits as "Site <release>" only when something changed.
# tools/site-publish-test.sh runs it against a local bare repository
# (SITE_PUBLISH_ALLOW_DIRTY=1 is for that test only).
set -euo pipefail
site=$1 repo=$2 work=${SITE_REPO_WORK:-target/site-repo}
version=$(git describe --always --dirty --tags)
case $version in *-dirty) [ "${SITE_PUBLISH_ALLOW_DIRTY:-}" = 1 ] || { echo "refusing: the working tree is dirty ($version); commit first" >&2; exit 1; };; esac
for kept in gallery .github; do
  [ ! -e "$site/$kept" ] || { echo "refusing: the site has a $kept/, which the Pages repo keeps for itself" >&2; exit 1; }
done
# The Pages repo is public: name the release the site carries, not a
# development describe ("Site v1.0.0", or "Site v1.0.0, docs of 2026-10-21"
# when the docs moved on since the tag).
release=v$(sed -n 's/.*"version": *"\([^"]*\)".*/\1/p' "$site/install/release.json")
label=$release
[ "$version" = "$release" ] || label="$release, docs of $(date -u +%F)"

# What the public repo keeps besides the site: .github/ and the gallery's README.
stage=$(mktemp -d)
trap 'rm -rf "$stage"' EXIT
cp -a site/public-repo/. "$stage"/
mkdir -p "$stage/.github/gallery-check"
# Only what git tracks (as committed: the tree is clean), and nothing new beside it.
vendored=(opendmd-psi opendmd-gallery tools/gallery_page.py)
untracked=$(git ls-files --others --exclude-standard -- "${vendored[@]}")
[ -z "$untracked" ] || { echo "refusing: untracked files in the checker's source: $untracked" >&2; exit 1; }
git ls-files -z --cached -- "${vendored[@]}" |
  xargs -0 cp --parents -t "$stage/.github/gallery-check"

if [ -d "$work/.git" ]; then
  git -C "$work" fetch -q origin && git -C "$work" reset -q --hard origin/main 2>/dev/null || true
else
  git clone -q "$repo" "$work" 2>/dev/null || { mkdir -p "$work" && git -C "$work" init -q -b main && git -C "$work" remote add origin "$repo"; }
fi
# Commit as whoever commits here (this repo's identity may be local to it).
git -C "$work" config user.name "$(git config user.name)"
git -C "$work" config user.email "$(git config user.email)"
# (rsync never deletes what it excludes.)
# --checksum: a file checked out and rewritten in the same second at the same size
# would pass rsync's size+mtime test and keep its old content.
rsync -a --checksum --delete --exclude /.git --exclude /gallery/ --exclude /.github/ "$site"/ "$work"/
rsync -a --checksum --delete "$stage/.github/" "$work/.github/"
mkdir -p "$work/gallery"
cp "$stage/gallery/README.md" "$work/gallery/README.md"
git -C "$work" add -A
if git -C "$work" diff --cached --quiet; then echo "site: no changes"; exit 0; fi
git -C "$work" commit -q -m "Site $label"
git -C "$work" push -q origin HEAD:main
echo "site: pushed $label ($version) to $repo"
