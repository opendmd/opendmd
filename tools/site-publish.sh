#!/usr/bin/env bash
# Push the built site (make site) to the Pages repo, opendmd.github.io.
#
#   tools/site-publish.sh <site-dir> <repo-url>      (normally via `make site-publish`)
#
# Keeps a clone in target/site-repo, replaces everything in it but .git with
# the site, and commits as "Site <release>" only when something changed.
set -euo pipefail
site=$1 repo=$2 work=target/site-repo
version=$(git describe --always --dirty --tags)
case $version in *-dirty) echo "refusing: the working tree is dirty ($version); commit first" >&2; exit 1;; esac
# The Pages repo is public: name the release the site carries, not a
# development describe ("Site v1.0.0", or "Site v1.0.0, docs of 2026-10-21"
# when the docs moved on since the tag).
release=v$(sed -n 's/.*"version": *"\([^"]*\)".*/\1/p' "$site/install/release.json")
label=$release
[ "$version" = "$release" ] || label="$release, docs of $(date -u +%F)"
if [ -d "$work/.git" ]; then
  git -C "$work" fetch -q origin && git -C "$work" reset -q --hard origin/main 2>/dev/null || true
else
  git clone -q "$repo" "$work" 2>/dev/null || { mkdir -p "$work" && git -C "$work" init -q -b main && git -C "$work" remote add origin "$repo"; }
fi
# Commit as whoever commits here (this repo's identity may be local to it).
git -C "$work" config user.name "$(git config user.name)"
git -C "$work" config user.email "$(git config user.email)"
rsync -a --delete --exclude .git "$site"/ "$work"/
git -C "$work" add -A
if git -C "$work" diff --cached --quiet; then echo "site: no changes"; exit 0; fi
git -C "$work" commit -q -m "Site $label"
git -C "$work" push -q origin HEAD:main
echo "site: pushed $label ($version) to $repo"
